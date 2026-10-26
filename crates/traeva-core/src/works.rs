//! Works across merges (#31 M2b; shared-contract.md §4.3a). Merging two
//! works never rewrites a row: the absorbed work's memberships, relations
//! and record chains keep its ID, and that ID now means the live work it was
//! merged into (`memory_live_families`, migration 0008). Every reader
//! resolves a work ID to its live work; every new row goes under a live ID
//! (database triggers refuse anything else).
//!
//! Decisions about works (merge, associate, revoke, confirm again; move
//! later) are
//! rows of `memory_work_decisions`; every membership or relation status they
//! change gets a `memory_work_changes` row first, so a reader can always say
//! which decision changed what (material §7: corrections keep their basis).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    EdgeKind, MemberBasis, RelationStatus, latest_watch_of_chain_in, pending_counts_in, to_text,
};
use crate::memory_consent::memory_error;
use crate::store::{Store, sqlite_error};

/// One live work, for choosing the other side of a merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkSummaryView {
    pub family_id: String,
    pub created_at_ms: i64,
    /// Works merged into this one (their IDs still open it).
    pub merged_family_ids: Vec<String>,
    /// Where its confirmed member files are, oldest membership first.
    pub display_paths: Vec<String>,
    pub records: usize,
}

impl OrganizeService {
    /// Every live work, oldest first. Reads no user file.
    pub fn memory_works(&self) -> Result<Vec<WorkSummaryView>, AppError> {
        let store = self.store()?;
        let mut works = Vec::new();
        for family_id in store.all_family_ids()? {
            let view = self.memory_family(&family_id)?;
            works.push(WorkSummaryView {
                display_paths: view
                    .members
                    .iter()
                    .filter(|m| {
                        matches!(
                            m.status,
                            RelationStatus::ConfirmedSystem | RelationStatus::ConfirmedUser
                        )
                    })
                    .map(|m| m.display_path.clone())
                    .collect(),
                records: view.records.len(),
                merged_family_ids: view.merged_family_ids,
                created_at_ms: view.created_at_ms,
                family_id: view.family_id,
            });
        }
        Ok(works)
    }
}

/// The live work work `family_id` means now; none for an unknown ID.
pub(crate) fn live_family_in(
    conn: &Connection,
    family_id: &str,
) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT live_family_id FROM memory_live_families WHERE family_id = ?1",
        [family_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(fail)
}

/// The works merged into live work `live`, in the order they were merged.
pub(crate) fn merged_family_ids_in(conn: &Connection, live: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT l.family_id FROM memory_live_families AS l
             LEFT JOIN memory_family_merges AS m ON m.absorbed_family_id = l.family_id
             WHERE l.live_family_id = ?1 AND l.family_id <> ?1
             ORDER BY m.merged_at_ms, m.rowid",
        )
        .map_err(fail)?;
    stmt.query_map([live], |r| r.get(0))
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)
}

/// The live work record chain `chain_id` belongs to now: the work of the
/// file it watches (its latest watch). Moving that file to another work
/// takes the record along (M2b-3b), while the chain row keeps the work it
/// was made in. None for an unknown chain.
pub(crate) fn live_family_of_chain_in(
    conn: &Connection,
    chain_id: &str,
) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT coalesce(
             (SELECT l.live_family_id FROM memory_members AS m
              JOIN memory_live_families AS l ON l.family_id = m.family_id
              WHERE m.status IN ('confirmed_system', 'confirmed_user', 'disputed')
                AND m.file_id = (SELECT w.file_id FROM memory_chain_watches AS link
                                 JOIN memory_watches AS w ON w.watch_id = link.watch_id
                                 WHERE link.chain_id = ?1 ORDER BY w.rowid DESC LIMIT 1)),
             (SELECT l.live_family_id FROM memory_chains AS c
              JOIN memory_live_families AS l ON l.family_id = c.family_id
              WHERE c.chain_id = ?1))",
        [chain_id],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
    .map(Option::flatten)
    .map_err(fail)
}

/// What a page about live work `live` shows that a decision depends on, as
/// lines for its token: the works it stands for, every membership and
/// relation with its status, each record (its watch, lost or held, waiting
/// candidates) and the latest work decision about it. Saving a file does
/// not change these (no version counts or tails), so a merge page stays
/// valid while Memory records.
pub(crate) fn work_token_lines(conn: &Connection, live: &str) -> Result<Vec<String>, AppError> {
    let group = |sql: &str| sql.replace("{GROUP}", GROUP);
    let mut ids = merged_family_ids_in(conn, live)?;
    ids.sort();
    let mut lines = vec![format!("work:{live}:{}", ids.join(","))];
    let mut stmt = conn
        .prepare(&group(
            "SELECT 'member:' || member_id || ':' || file_id || ':' || status
             FROM memory_members WHERE family_id IN {GROUP} AND status <> 'superseded'
             ORDER BY 1",
        ))
        .map_err(fail)?;
    let rows: Vec<String> = stmt
        .query_map([live], |r| r.get(0))
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?;
    lines.extend(rows);
    // The relations and records the work's facts gather (by membership).
    let facts = crate::family::family_facts_in(conn, live)?;
    let mut edges: Vec<String> = facts
        .edges
        .iter()
        .map(|e| format!("edge:{}:{}", e.edge_id, to_text(e.status)))
        .collect();
    edges.sort();
    lines.extend(edges);
    let mut chains: Vec<String> = facts.chains.iter().map(|c| c.chain_id.clone()).collect();
    chains.sort();
    for chain_id in chains {
        let watch = latest_watch_of_chain_in(conn, &chain_id)?;
        let flag = |sql: &str| -> Result<Option<String>, AppError> {
            conn.query_row(sql, [&chain_id], |r| r.get(0))
                .optional()
                .map_err(fail)
        };
        let lost = flag(
            "SELECT gap_id FROM memory_gaps WHERE chain_id = ?1 AND kind = 'lost'
             AND ended_at_ms IS NULL",
        )?;
        let hold = flag(
            "SELECT hold_id FROM memory_record_holds WHERE chain_id = ?1
             AND released_at_ms IS NULL",
        )?;
        let pending = pending_counts_in(conn, &chain_id)?;
        lines.push(format!(
            "record:{chain_id}:{}:{}:{}:{}:{}",
            watch.as_ref().map_or("-", |w| w.watch_id.as_str()),
            watch.as_ref().is_some_and(|w| w.active),
            lost.as_deref().unwrap_or("-"),
            hold.as_deref().unwrap_or("-"),
            pending.objects
        ));
    }
    let decision: Option<String> = conn
        .query_row(
            &group(
                "SELECT work_decision_id FROM memory_work_decisions
                 WHERE family_id IN {GROUP} OR other_family_id IN {GROUP}
                 ORDER BY decided_at_ms DESC, rowid DESC LIMIT 1",
            ),
            [live],
            |r| r.get(0),
        )
        .optional()
        .map_err(fail)?;
    lines.push(format!("decision:{}", decision.as_deref().unwrap_or("-")));
    Ok(lines)
}

/// The works a live work stands for, as a subquery on `?1`.
pub(crate) const GROUP: &str =
    "(SELECT family_id FROM memory_live_families WHERE live_family_id = ?1)";

/// The kinds of work decisions (`memory_work_decisions.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkDecisionKind {
    Merge,
    Revoke,
    ConfirmAgain,
    Move,
    Associate,
}

/// One work decision row, as `insert_work_decision_in` writes it. The
/// association columns are set only for `associate`.
pub(crate) struct WorkDecisionRow<'a> {
    pub(crate) work_decision_id: &'a str,
    pub(crate) kind: WorkDecisionKind,
    pub(crate) family_id: &'a str,
    pub(crate) other_family_id: Option<&'a str>,
    pub(crate) edge_id: Option<&'a str>,
    pub(crate) file_id: Option<&'a str>,
    pub(crate) choice: Option<crate::family_associate::AssociateChoiceKind>,
    pub(crate) parent_snapshot_id: Option<&'a str>,
    pub(crate) evidence: Option<&'a str>,
    pub(crate) page_token: &'a str,
}

/// Writes one work decision row. A page token used before (the same page
/// confirmed twice) is refused as `page_outdated`.
pub(crate) fn insert_work_decision_in(
    tx: &Connection,
    row: &WorkDecisionRow<'_>,
    now: i64,
) -> Result<(), AppError> {
    tx.execute(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, other_family_id,
             edge_id, file_id, choice, parent_snapshot_id, evidence, page_token, decided_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            row.work_decision_id,
            to_text(row.kind),
            row.family_id,
            row.other_family_id,
            row.edge_id,
            row.file_id,
            row.choice.map(to_text),
            row.parent_snapshot_id,
            row.evidence,
            row.page_token,
            now
        ],
    )
    .map_err(|err| match err.sqlite_error_code() {
        Some(rusqlite::ErrorCode::ConstraintViolation) => crate::candidates::page_outdated(
            "This page was already confirmed, or what it showed changed. Nothing was changed; \
             open it again.",
        ),
        _ => fail(err),
    })?;
    Ok(())
}

/// Changes membership `member_id` from `from` to `to` for work decision
/// `work_decision_id`: the change row first (the 0008 triggers ask for it),
/// then the status. Changed meanwhile: `page_outdated`.
pub(crate) fn set_member_status_in(
    tx: &Connection,
    work_decision_id: &str,
    member_id: &str,
    from: RelationStatus,
    to: RelationStatus,
    now: i64,
) -> Result<(), AppError> {
    insert_change_in(
        tx,
        work_decision_id,
        Some(member_id),
        None,
        Some(from),
        to,
        now,
    )?;
    let changed = tx
        .execute(
            "UPDATE memory_members SET status = ?3, updated_at_ms = ?4
             WHERE member_id = ?1 AND status = ?2",
            params![member_id, to_text(from), to_text(to), now],
        )
        .map_err(fail)?;
    if changed != 1 {
        return Err(crate::candidates::page_outdated(
            "A membership changed meanwhile. Nothing was changed; open the page again.",
        ));
    }
    Ok(())
}

/// Changes relation `edge_id` from `from` to `to` for work decision
/// `work_decision_id`, like `set_member_status_in`.
pub(crate) fn set_edge_status_in(
    tx: &Connection,
    work_decision_id: &str,
    edge_id: &str,
    from: RelationStatus,
    to: RelationStatus,
    now: i64,
) -> Result<(), AppError> {
    insert_change_in(
        tx,
        work_decision_id,
        None,
        Some(edge_id),
        Some(from),
        to,
        now,
    )?;
    let changed = tx
        .execute(
            "UPDATE memory_edges SET status = ?3, updated_at_ms = ?4
             WHERE edge_id = ?1 AND status = ?2",
            params![edge_id, to_text(from), to_text(to), now],
        )
        .map_err(fail)?;
    if changed != 1 {
        return Err(crate::candidates::page_outdated(
            "A relation changed meanwhile. Nothing was changed; open the page again.",
        ));
    }
    Ok(())
}

/// A new membership of file `file_id` in live work `family_id`, written by
/// work decision `work_decision_id` (its change row says so). Returns its
/// ID.
pub(crate) fn insert_member_in(
    tx: &Connection,
    work_decision_id: &str,
    family_id: &str,
    file_id: &str,
    status: RelationStatus,
    basis: MemberBasis,
    now: i64,
) -> Result<String, AppError> {
    let member_id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO memory_members (member_id, family_id, file_id, status, basis,
             created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            member_id,
            family_id,
            file_id,
            to_text(status),
            to_text(basis),
            now
        ],
    )
    .map_err(fail)?;
    insert_change_in(
        tx,
        work_decision_id,
        Some(&member_id),
        None,
        None,
        status,
        now,
    )?;
    Ok(member_id)
}

/// A new relation in live work `family_id`, written by work decision
/// `work_decision_id` (its change row says so). Returns its ID.
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_edge_in(
    tx: &Connection,
    work_decision_id: &str,
    family_id: &str,
    kind: EdgeKind,
    parent_snapshot_id: Option<&str>,
    child_snapshot_id: &str,
    status: RelationStatus,
    now: i64,
) -> Result<String, AppError> {
    let edge_id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO memory_edges (edge_id, family_id, kind, parent_snapshot_id,
             child_snapshot_id, status, skipped_items, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
        params![
            edge_id,
            family_id,
            to_text(kind),
            parent_snapshot_id,
            child_snapshot_id,
            to_text(status),
            now
        ],
    )
    .map_err(fail)?;
    insert_change_in(
        tx,
        work_decision_id,
        None,
        Some(&edge_id),
        None,
        status,
        now,
    )?;
    Ok(edge_id)
}

/// One `memory_work_changes` row: a membership or relation the decision
/// created (`from` none) or whose status it changed.
fn insert_change_in(
    tx: &Connection,
    work_decision_id: &str,
    member_id: Option<&str>,
    edge_id: Option<&str>,
    from: Option<RelationStatus>,
    to: RelationStatus,
    now: i64,
) -> Result<(), AppError> {
    tx.execute(
        "INSERT INTO memory_work_changes (change_id, work_decision_id, member_id, edge_id,
             from_status, to_status, changed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            uuid::Uuid::new_v4().to_string(),
            work_decision_id,
            member_id,
            edge_id,
            from.map(to_text),
            to_text(to),
            now
        ],
    )
    .map_err(fail)?;
    Ok(())
}

/// Pauses record chain `chain_id` for work decision `work_decision_id`
/// (material §7 last bullet): its watch `watch_id` stops, candidate capture
/// turns off and its open gaps end (both `corrected`), and one hold row says
/// which continuation (`edge_id`) the recording rested on. Turning Memory on
/// again releases it (`family.rs` `release_hold_in`).
pub(crate) fn hold_in(
    tx: &Connection,
    chain_id: &str,
    watch_id: &str,
    edge_id: Option<&str>,
    work_decision_id: &str,
    now: i64,
) -> Result<(), AppError> {
    let stopped = tx
        .execute(
            "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = ?2
             WHERE watch_id = ?1 AND status = 'active'",
            params![watch_id, now],
        )
        .map_err(fail)?;
    if stopped != 1 {
        return Err(crate::candidates::page_outdated(
            "Memory was turned off meanwhile. Nothing was changed; open the page again.",
        ));
    }
    tx.execute(
        "UPDATE memory_candidate_switches SET disabled_at_ms = ?2, disabled_reason = 'corrected'
         WHERE chain_id = ?1 AND disabled_at_ms IS NULL",
        params![chain_id, now],
    )
    .map_err(fail)?;
    tx.execute(
        "UPDATE memory_gaps SET ended_at_ms = max(started_at_ms, ?2), end_reason = 'corrected'
         WHERE chain_id = ?1 AND ended_at_ms IS NULL",
        params![chain_id, now],
    )
    .map_err(fail)?;
    tx.execute(
        "INSERT INTO memory_record_holds (hold_id, chain_id, watch_id, edge_id, work_decision_id,
             held_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            uuid::Uuid::new_v4().to_string(),
            chain_id,
            watch_id,
            edge_id,
            work_decision_id,
            now
        ],
    )
    .map_err(fail)?;
    Ok(())
}

pub(crate) fn unknown_family() -> AppError {
    memory_error(
        ErrorCode::UnknownFamily,
        "There is no work with this ID.",
        false,
        None,
    )
}

impl Store {
    pub(crate) fn live_family(&self, family_id: &str) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        live_family_in(&conn, family_id)
    }
}

fn fail(err: rusqlite::Error) -> AppError {
    sqlite_error("Cannot read the Memory records", &err)
}
