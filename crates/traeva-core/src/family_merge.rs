//! Merging two works, and the hint when a new file has exactly the content
//! of a version in another work (#31 material §2 source 3, §6; the
//! 2026-10-10 decisions on #31; shared-contract.md §4.3a).
//!
//! Merging: the user says two works are one. The page shows both works (the
//! merge's scope) and every earlier "unrelated" judgement the merge would
//! contradict. Confirming writes one work decision and one merge row; those
//! judgements stop counting (superseded, each with its change row). Nothing
//! else is written: no file record is merged, no membership is copied, no
//! relation is made, so no order between the two works' versions is ever
//! derived (material §6). Each record keeps recording where it is, with its
//! switch, quota and candidates; new rows go under the live work.
//!
//! The hint: when Memory is turned on for a file of no work, its first
//! version is compared, by SHA-256 only, with the versions other works saved
//! before it. Equal bytes are shown as "may be related", never as a relation
//! or a candidate (material §7: the same hash does not prove a parent).
//! Empty content is never hinted: every empty file has the same hash.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode};
use crate::family::{MemoryFamilyView, RelationStatus};
use crate::is_canonical_uuid;
use crate::memory_consent::memory_error;
use crate::memory_contracts::SnapshotRef;
use crate::observe;
use crate::snapshot::query_snapshots;
use crate::store::{Store, sqlite_error};
use crate::works::{
    WorkDecisionKind, WorkDecisionRow, insert_work_decision_in, live_family_in,
    set_member_status_in, unknown_family, work_token_lines,
};

/// A version of another work with exactly the bytes of this record's first
/// version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SameContentHint {
    /// The other (live) work.
    pub family_id: String,
    /// This record's first version.
    pub own_version: SnapshotRef,
    /// The newest version of that work with the same bytes, saved before it.
    pub same_bytes_as: SnapshotRef,
}

/// An earlier "unrelated" judgement a decision makes stop counting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverrideView {
    pub member_id: String,
    pub file_id: String,
    pub display_path: String,
    /// The work it was judged unrelated to.
    pub rejected_in_family_id: String,
}

/// What merging two works would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergePreview {
    /// The work that stays (live).
    pub family_id: String,
    /// The work that joins it (live); its ID will open the merged work.
    pub other_family_id: String,
    pub stays: MemoryFamilyView,
    pub joins: MemoryFamilyView,
    pub overrides: Vec<OverrideView>,
    /// Bring it back to confirm.
    pub page_token: String,
}

/// Confirming a merge page. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeRequest {
    pub family_id: String,
    pub other_family_id: String,
    pub page_token: String,
}

/// What a work decision did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkDecisionOutcome {
    pub work_decision_id: String,
    pub kind: WorkDecisionKind,
    /// The work as it is now.
    pub family: MemoryFamilyView,
    /// A move's target work (a new one, or the one chosen).
    pub other_family_id: Option<String>,
    /// Records the decision stopped: an association's "continue" stops the
    /// record whose line ended at the parent (material §5).
    pub stopped_records: Vec<String>,
    /// Records the decision paused: revoking the continuation a record went
    /// on by holds it until the user turns Memory on again or stops it.
    pub held_records: Vec<String>,
}

/// The page's facts: the two live works and what the token covers.
struct MergeFacts {
    stays: String,
    joins: String,
    overrides: Vec<(String, String, String)>,
    token: String,
}

impl OrganizeService {
    /// What merging work `other_family_id` into work `family_id` would do.
    /// Reads no user file and writes nothing.
    pub fn merge_preview(
        &self,
        family_id: &str,
        other_family_id: &str,
    ) -> Result<MergePreview, AppError> {
        let store = self.store()?;
        check_ids(&[family_id, other_family_id])?;
        let facts = {
            let conn = store.connection()?;
            merge_facts(&conn, family_id, other_family_id)?
        };
        let overrides = override_views(store, &facts.overrides)?;
        Ok(MergePreview {
            stays: self.memory_family(&facts.stays)?,
            joins: self.memory_family(&facts.joins)?,
            family_id: facts.stays,
            other_family_id: facts.joins,
            overrides,
            page_token: facts.token,
        })
    }

    /// Merges the two works the page showed (see the module note). Nothing
    /// changes when anything the page showed changed.
    pub fn merge_works(&self, request: &MergeRequest) -> Result<WorkDecisionOutcome, AppError> {
        let store = self.store()?;
        check_ids(&[&request.family_id, &request.other_family_id])?;
        check_token(&request.page_token)?;
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the merge", &err);
        let _no_capture = self.lock_execution();
        let work_decision_id = uuid::Uuid::new_v4().to_string();
        let stays = {
            let mut conn = store.connection()?;
            let tx = conn.transaction().map_err(fail)?;
            let facts = merge_facts(&tx, &request.family_id, &request.other_family_id)?;
            if facts.token != request.page_token {
                return Err(crate::candidates::page_outdated(
                    "The works changed since the page was opened. Nothing was changed; open \
                     it again.",
                ));
            }
            let now = self.now();
            insert_work_decision_in(
                &tx,
                &WorkDecisionRow {
                    work_decision_id: &work_decision_id,
                    kind: WorkDecisionKind::Merge,
                    family_id: &facts.stays,
                    other_family_id: Some(&facts.joins),
                    edge_id: None,
                    file_id: None,
                    choice: None,
                    parent_snapshot_id: None,
                    evidence: None,
                    page_token: &request.page_token,
                },
                now,
            )?;
            tx.execute(
                "INSERT INTO memory_family_merges (absorbed_family_id, into_family_id,
                     work_decision_id, merged_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![facts.joins, facts.stays, work_decision_id, now],
            )
            .map_err(fail)?;
            for (member_id, _, _) in &facts.overrides {
                set_member_status_in(
                    &tx,
                    &work_decision_id,
                    member_id,
                    RelationStatus::Rejected,
                    RelationStatus::Superseded,
                    now,
                )?;
            }
            tx.commit().map_err(fail)?;
            facts.stays
        };
        Ok(WorkDecisionOutcome {
            work_decision_id,
            kind: WorkDecisionKind::Merge,
            family: self.memory_family(&stays)?,
            other_family_id: None,
            stopped_records: Vec::new(),
            held_records: Vec::new(),
        })
    }
}

/// The page's view of earlier "unrelated" judgements (member, file, work).
pub(crate) fn override_views(
    store: &Store,
    overrides: &[(String, String, String)],
) -> Result<Vec<OverrideView>, AppError> {
    let mut views = Vec::new();
    for (member_id, file_id, rejected_in) in overrides {
        let display_path = store
            .get_file(file_id)?
            .map(|(file, _)| {
                let root = store.get_root(&file.root_id).ok().flatten();
                match root {
                    Some(root) => format!("{}/{}", root.display_path, file.relative_path),
                    None => file.relative_path,
                }
            })
            .unwrap_or_default();
        views.push(OverrideView {
            member_id: member_id.clone(),
            file_id: file_id.clone(),
            display_path,
            rejected_in_family_id: rejected_in.clone(),
        });
    }
    Ok(views)
}

fn merge_facts(conn: &Connection, family_id: &str, other: &str) -> Result<MergeFacts, AppError> {
    let stays = live_family_in(conn, family_id)?.ok_or_else(unknown_family)?;
    let joins = live_family_in(conn, other)?.ok_or_else(unknown_family)?;
    if stays == joins {
        return Err(memory_error(
            ErrorCode::DecisionConflict,
            "These are already one work.",
            false,
            None,
        ));
    }
    let overrides = overrides_in(conn, &stays, &joins)?;
    let mut lines = vec!["memory-merge-1".to_string()];
    lines.extend(
        work_token_lines(conn, &stays)?
            .into_iter()
            .map(|l| format!("stays:{l}")),
    );
    lines.extend(
        work_token_lines(conn, &joins)?
            .into_iter()
            .map(|l| format!("joins:{l}")),
    );
    lines.extend(overrides.iter().map(|(id, _, _)| format!("override:{id}")));
    Ok(MergeFacts {
        token: observe::sha256_hex(lines.join("\n").as_bytes()),
        stays,
        joins,
        overrides,
    })
}

/// Rejected memberships in one of the two works whose file is confirmed or
/// waiting as a candidate in the other: (member, file, the work it was
/// rejected in).
pub(crate) fn overrides_in(
    conn: &Connection,
    a: &str,
    b: &str,
) -> Result<Vec<(String, String, String)>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let mut stmt = conn
        .prepare(
            "SELECT r.member_id, r.file_id, r.family_id FROM memory_members AS r
             JOIN memory_live_families AS rl ON rl.family_id = r.family_id
             WHERE r.status = 'rejected' AND rl.live_family_id IN (?1, ?2)
               AND EXISTS (
                   SELECT 1 FROM memory_members AS o
                   JOIN memory_live_families AS ol ON ol.family_id = o.family_id
                   WHERE o.file_id = r.file_id
                     AND o.status IN ('confirmed_system', 'confirmed_user', 'candidate')
                     AND ol.live_family_id IN (?1, ?2)
                     AND ol.live_family_id <> rl.live_family_id)
             ORDER BY r.member_id",
        )
        .map_err(fail)?;
    stmt.query_map(params![a, b], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)
}

/// The hint for record chain `chain_id` in live work `live` (see the module
/// note): only for a record whose Memory was turned on for a file of no
/// work; at most one version per other work, at most ten works.
pub(crate) fn same_content_in(
    conn: &Connection,
    chain_id: &str,
    live: &str,
) -> Result<Vec<SameContentHint>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let started: Option<(String, String)> = conn
        .query_row(
            "SELECT c.origin_file_id,
                    (SELECT m.basis FROM memory_members AS m WHERE m.file_id = c.origin_file_id
                     ORDER BY m.created_at_ms, m.rowid LIMIT 1)
             FROM memory_chains AS c
             WHERE c.chain_id = ?1
               AND EXISTS (SELECT 1 FROM memory_chain_watches AS link
                           WHERE link.chain_id = c.chain_id AND link.reason = 'started')",
            [chain_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            },
        )
        .map(Some)
        .or_else(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            err => Err(fail(err)),
        })?;
    let Some((origin, basis)) = started else {
        return Ok(Vec::new());
    };
    if basis != "memory_on" {
        return Ok(Vec::new());
    }
    let Some(own) = query_snapshots(
        conn,
        "WHERE file_id = ?1 ORDER BY sequence LIMIT 1",
        [&origin],
    )?
    .pop() else {
        return Ok(Vec::new());
    };
    if own.byte_length == 0 {
        return Ok(Vec::new());
    }
    let found: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT l.live_family_id, s.snapshot_id FROM snapshots AS s
                 JOIN memory_members AS m ON m.file_id = s.file_id
                      AND m.status IN ('confirmed_system', 'confirmed_user')
                 JOIN memory_live_families AS l ON l.family_id = m.family_id
                 WHERE s.sha256 = ?1 AND s.byte_length > 0
                   AND s.rowid < (SELECT rowid FROM snapshots WHERE snapshot_id = ?2)
                   AND l.live_family_id <> ?3
                   AND NOT EXISTS (
                       SELECT 1 FROM memory_members AS r
                       JOIN memory_live_families AS rl ON rl.family_id = r.family_id
                       WHERE r.file_id = ?4 AND r.status = 'rejected'
                         AND rl.live_family_id = l.live_family_id)
                 ORDER BY s.rowid DESC",
            )
            .map_err(fail)?;
        stmt.query_map(params![own.sha256, own.snapshot_id, live, origin], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?
    };
    let mut hints: Vec<SameContentHint> = Vec::new();
    for (family_id, snapshot_id) in found {
        if hints.len() == 10 || hints.iter().any(|h| h.family_id == family_id) {
            continue;
        }
        if let Some(same) = query_snapshots(conn, "WHERE snapshot_id = ?1", [&snapshot_id])?.pop() {
            hints.push(SameContentHint {
                family_id,
                own_version: own.clone(),
                same_bytes_as: same,
            });
        }
    }
    Ok(hints)
}

pub(crate) fn check_ids(ids: &[&str]) -> Result<(), AppError> {
    if ids.iter().all(|id| is_canonical_uuid(id)) {
        Ok(())
    } else {
        Err(memory_error(
            ErrorCode::InvalidInput,
            "A work ID is not valid.",
            false,
            None,
        ))
    }
}

pub(crate) fn check_token(token: &str) -> Result<(), AppError> {
    if token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(memory_error(
            ErrorCode::InvalidInput,
            "The page token is not valid.",
            false,
            None,
        ))
    }
}

#[cfg(test)]
#[path = "family_merge_tests.rs"]
mod tests;
