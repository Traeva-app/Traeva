//! Moving members out of a work (#31 material §7 "纠错": family 归属也错误
//! 时，用户明确选择要移出的成员，移到独立或指定 family；成员调整后跨越不同
//! family 的旧关系停止使用，需要重新核对；不删除文件、快照或原证据，不自动
//! 把被移除的关系重新接上. The 2026-10-10 decisions 2 and 3 on #31;
//! shared-contract.md §4.3a).
//!
//! The user picks confirmed members of a work and a target: a new work of
//! their own, or another work. Confirming, in one transaction:
//! - each file's membership here is replaced (superseded) by one in the
//!   target, confirmed by the user, basis `correction`: from now on the
//!   membership rests on the user (material §6);
//! - a confirmed relation with both ends moving goes along (it counts in
//!   the target);
//! - a confirmed relation with exactly one end moving would cross two works:
//!   it becomes disputed (stop using, re-check), never revoked silently. The
//!   user revokes it, or confirms it again once both ends are in one work;
//! - a record goes with the file it watches, with its candidate switch,
//!   quota and gaps;
//! - a record whose recording rested on a relation that now crosses works
//!   is held, as after revoking (decision 2);
//! - earlier "unrelated" judgements of the moved files in the target stop
//!   counting, each with its change row (decision 3).
//!
//! Snapshots, file records and candidates are not touched. Moving every
//! member is refused: that would be a merge (or change nothing). The
//! preview reads no user file and writes nothing.

use std::collections::{BTreeSet, HashSet};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::candidates::page_outdated;
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    FamilyMemberView, MemberBasis, MemoryRecordView, RelationStatus, VersionRelationView,
    candidate_pending, family_facts_in, latest_watch_of_chain_in, pending_counts_in,
};
use crate::family_merge::{
    OverrideView, WorkDecisionOutcome, check_ids, check_token, override_views,
};
use crate::lineage::{chain_line_in, line_parts, part_of};
use crate::memory_consent::memory_error;
use crate::observe;
use crate::store::sqlite_error;
use crate::works::{
    GROUP, WorkDecisionKind, WorkDecisionRow, hold_in, insert_member_in, insert_work_decision_in,
    live_family_in, set_edge_status_in, set_member_status_in, unknown_family, work_token_lines,
};

/// Where the moved members go. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MoveTarget {
    /// A new work of their own.
    NewWork {},
    /// An existing work (any of its IDs).
    Work { family_id: String },
}

/// What to move. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveQuery {
    pub family_id: String,
    pub file_ids: Vec<String>,
    pub target: MoveTarget,
}

/// What moving the members would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MovePreview {
    /// The work they leave (live).
    pub family_id: String,
    /// The work they join (live); null for a new work.
    pub target_family_id: Option<String>,
    pub moving: Vec<FamilyMemberView>,
    /// Records that go along: they watch a file that moves.
    pub records_moving: Vec<MemoryRecordView>,
    /// Relations with both ends moving: they count in the target.
    pub relations_kept: Vec<VersionRelationView>,
    /// Relations with one end moving: they become disputed.
    pub relations_disputed: Vec<VersionRelationView>,
    /// Records that pause: their recording rested on a disputed relation.
    pub records_held: Vec<MemoryRecordView>,
    /// Earlier "unrelated" judgements of the moved files in the target.
    pub overrides: Vec<OverrideView>,
    /// Bring it back to confirm.
    pub page_token: String,
}

/// Confirming a move page. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    pub family_id: String,
    pub file_ids: Vec<String>,
    pub target: MoveTarget,
    pub page_token: String,
}

/// What the page shows, worked out on one connection.
struct MoveFacts {
    live: String,
    target: Option<String>,
    files: Vec<String>,
    /// (member, its status) of the memberships that move.
    members: Vec<(String, RelationStatus)>,
    kept: Vec<String>,
    /// (relation, its status) that become disputed.
    disputed: Vec<(String, RelationStatus)>,
    records_moving: Vec<String>,
    /// (record, its watch, the relation it rested on).
    held: Vec<(String, String, Option<String>)>,
    overrides: Vec<(String, String, String)>,
    token: String,
}

impl OrganizeService {
    /// What moving files `query.file_ids` out of work `query.family_id`
    /// would do. Reads no user file and writes nothing.
    pub fn move_preview(&self, query: &MoveQuery) -> Result<MovePreview, AppError> {
        let store = self.store()?;
        let _no_capture = self.lock_execution();
        let facts = {
            let conn = store.connection()?;
            move_facts(&conn, &query.family_id, &query.file_ids, &query.target)?
        };
        let source = self.memory_family(&facts.live)?;
        let moving = source
            .members
            .iter()
            .filter(|m| facts.files.contains(&m.file_id))
            .cloned()
            .collect();
        let relation = |ids: &[String]| -> Vec<VersionRelationView> {
            source
                .relations
                .iter()
                .filter(|r| ids.contains(&r.edge_id))
                .cloned()
                .collect()
        };
        let disputed: Vec<String> = facts.disputed.iter().map(|(id, _)| id.clone()).collect();
        let records = |ids: Vec<&String>| -> Result<Vec<MemoryRecordView>, AppError> {
            ids.into_iter().map(|id| self.memory_record(id)).collect()
        };
        Ok(MovePreview {
            relations_kept: relation(&facts.kept),
            relations_disputed: relation(&disputed),
            records_moving: records(facts.records_moving.iter().collect())?,
            records_held: records(facts.held.iter().map(|(chain, _, _)| chain).collect())?,
            overrides: override_views(store, &facts.overrides)?,
            moving,
            family_id: facts.live,
            target_family_id: facts.target,
            page_token: facts.token,
        })
    }

    /// Moves the members the page showed (see the module note).
    pub fn move_members(&self, request: &MoveRequest) -> Result<WorkDecisionOutcome, AppError> {
        let store = self.store()?;
        check_token(&request.page_token)?;
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the move", &err);
        let _no_capture = self.lock_execution();
        let now = self.now();
        let work_decision_id = uuid::Uuid::new_v4().to_string();
        let (live, target, held) = {
            let mut conn = store.connection()?;
            let tx = conn.transaction().map_err(fail)?;
            let facts = move_facts(&tx, &request.family_id, &request.file_ids, &request.target)?;
            if facts.token != request.page_token {
                return Err(page_outdated(
                    "The works or their records changed since the page was opened. Nothing was \
                     changed; open it again.",
                ));
            }
            let target = match &facts.target {
                Some(target) => target.clone(),
                None => {
                    let family_id = uuid::Uuid::new_v4().to_string();
                    tx.execute(
                        "INSERT INTO memory_families (family_id, created_at_ms) VALUES (?1, ?2)",
                        params![family_id, now],
                    )
                    .map_err(fail)?;
                    family_id
                }
            };
            insert_work_decision_in(
                &tx,
                &WorkDecisionRow {
                    work_decision_id: &work_decision_id,
                    kind: WorkDecisionKind::Move,
                    family_id: &facts.live,
                    other_family_id: Some(&target),
                    edge_id: None,
                    file_id: None,
                    choice: None,
                    parent_snapshot_id: None,
                    evidence: None,
                    page_token: &request.page_token,
                },
                now,
            )?;
            // An earlier "unrelated" row of a file in the target goes first:
            // a file has one live row per work.
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
            // The old membership is replaced first: a file is a member of
            // one work at a time.
            for ((member_id, status), file_id) in facts.members.iter().zip(&facts.files) {
                set_member_status_in(
                    &tx,
                    &work_decision_id,
                    member_id,
                    *status,
                    RelationStatus::Superseded,
                    now,
                )?;
                insert_member_in(
                    &tx,
                    &work_decision_id,
                    &target,
                    file_id,
                    RelationStatus::ConfirmedUser,
                    MemberBasis::Correction,
                    now,
                )?;
            }
            for (edge_id, status) in &facts.disputed {
                set_edge_status_in(
                    &tx,
                    &work_decision_id,
                    edge_id,
                    *status,
                    RelationStatus::Disputed,
                    now,
                )?;
            }
            let mut held = Vec::new();
            for (chain_id, watch_id, edge_id) in &facts.held {
                hold_in(
                    &tx,
                    chain_id,
                    watch_id,
                    edge_id.as_deref(),
                    &work_decision_id,
                    now,
                )?;
                held.push(chain_id.clone());
            }
            tx.commit().map_err(fail)?;
            (facts.live, target, held)
        };
        if !held.is_empty() {
            // As in `stop_memory_record`: the watcher was running until now.
            self.note_run_seen(true);
        }
        Ok(WorkDecisionOutcome {
            work_decision_id,
            kind: WorkDecisionKind::Move,
            family: self.memory_family(&live)?,
            other_family_id: Some(target),
            stopped_records: Vec::new(),
            held_records: held,
        })
    }
}

/// What moving `file_ids` out of work `family_id` to `target` would do,
/// and the page's token. Refusals are returned as errors.
fn move_facts(
    conn: &Connection,
    family_id: &str,
    file_ids: &[String],
    target: &MoveTarget,
) -> Result<MoveFacts, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    check_ids(&[family_id])?;
    let files: Vec<String> = file_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if files.is_empty() || files.len() != file_ids.len() {
        return Err(invalid("Choose each file to move once."));
    }
    if !files.iter().all(|id| crate::is_canonical_uuid(id)) {
        return Err(invalid("A file ID is not valid."));
    }
    let live = live_family_in(conn, family_id)?.ok_or_else(unknown_family)?;
    let target = match target {
        MoveTarget::NewWork {} => None,
        MoveTarget::Work { family_id } => {
            check_ids(&[family_id])?;
            let target = live_family_in(conn, family_id)?.ok_or_else(unknown_family)?;
            if target == live {
                return Err(conflict("The files are in this work already."));
            }
            Some(target)
        }
    };
    let facts = family_facts_in(conn, &live)?;
    let confirmed = |status: RelationStatus| {
        matches!(
            status,
            RelationStatus::ConfirmedSystem | RelationStatus::ConfirmedUser
        )
    };
    let mut members = Vec::new();
    for file_id in &files {
        let member = facts
            .members
            .iter()
            .find(|m| &m.file_id == file_id && confirmed(m.status))
            .ok_or_else(|| conflict("Only confirmed members of this work can be moved."))?;
        members.push((member.member_id.clone(), member.status));
    }
    let staying = facts
        .members
        .iter()
        .filter(|m| confirmed(m.status) && !files.contains(&m.file_id))
        .count();
    if staying == 0 {
        return Err(conflict(
            "Every member of this work would move. Merge the works instead, or keep them as \
             they are.",
        ));
    }
    // A record whose line or current file moves must not wait for a
    // decision: the waiting objects were found on that line.
    let moving: HashSet<&str> = files.iter().map(String::as_str).collect();
    let mut records_moving = Vec::new();
    for chain in &facts.chains {
        let watch = latest_watch_of_chain_in(conn, &chain.chain_id)?;
        let current_moves = watch
            .as_ref()
            .is_some_and(|w| moving.contains(w.file_id.as_str()));
        let line_moves = facts
            .lines
            .iter()
            .any(|(c, f)| c == &chain.chain_id && moving.contains(f.as_str()));
        if (current_moves || line_moves) && pending_counts_in(conn, &chain.chain_id)?.objects > 0 {
            let file = watch.map(|w| w.file_id).unwrap_or_default();
            return Err(candidate_pending(&file));
        }
        if current_moves {
            records_moving.push(chain.chain_id.clone());
        }
    }
    // Relations: both ends moving go along; one end moving is disputed.
    let file_of = |snapshot_id: &str| -> Result<String, AppError> {
        conn.query_row(
            "SELECT file_id FROM snapshots WHERE snapshot_id = ?1",
            [snapshot_id],
            |r| r.get(0),
        )
        .map_err(fail)
    };
    let mut kept = Vec::new();
    let mut disputed = Vec::new();
    for edge in &facts.edges {
        let Some(parent) = &edge.parent_snapshot_id else {
            continue;
        };
        if !confirmed(edge.status) {
            continue;
        }
        let parent_moves = moving.contains(file_of(parent)?.as_str());
        let child_moves = moving.contains(file_of(&edge.child_snapshot_id)?.as_str());
        match (parent_moves, child_moves) {
            (true, true) => kept.push(edge.edge_id.clone()),
            (true, false) | (false, true) => disputed.push((edge.edge_id.clone(), edge.status)),
            (false, false) => {}
        }
    }
    // Records whose current part would start at another file once the
    // disputed relations stop joining their line pause (decision 2).
    let stopped: HashSet<String> = disputed.iter().map(|(id, _)| id.clone()).collect();
    let mut held = Vec::new();
    for chain in &facts.chains {
        let Some(watch) = latest_watch_of_chain_in(conn, &chain.chain_id)?.filter(|w| w.active)
        else {
            continue;
        };
        let line = chain_line_in(conn, &chain.chain_id)?;
        let first = |stopped: &HashSet<String>| {
            part_of(&line_parts(&line, stopped), &watch.file_id)
                .map(|part| part.first_file_id.clone())
        };
        let after = first(&stopped);
        if first(&HashSet::new()) != after {
            let rested_on = line
                .joins
                .iter()
                .filter(|(edge, _, _)| stopped.contains(edge))
                .find(|(_, _, to)| Some(to) == after.as_ref())
                .or_else(|| {
                    line.joins
                        .iter()
                        .find(|(edge, _, _)| stopped.contains(edge))
                })
                .map(|(edge, _, _)| edge.clone());
            held.push((chain.chain_id.clone(), watch.watch_id, rested_on));
        }
    }
    // Earlier "unrelated" judgements of the moved files in the target.
    let mut overrides = Vec::new();
    if let Some(target) = &target {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT member_id, file_id, family_id FROM memory_members
                 WHERE family_id IN {GROUP} AND file_id = ?2 AND status = 'rejected'
                 ORDER BY member_id"
            ))
            .map_err(fail)?;
        for file_id in &files {
            let rows: Vec<(String, String, String)> = stmt
                .query_map(params![target, file_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(fail)?
                .collect::<Result<_, _>>()
                .map_err(fail)?;
            overrides.extend(rows);
        }
    }
    let mut lines = vec![
        "memory-move-1".to_string(),
        format!(
            "move:{live}:{}:{}",
            target.as_deref().unwrap_or("new"),
            files.join(",")
        ),
        format!("kept:{}", kept.join(",")),
        format!(
            "disputed:{}",
            disputed
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ),
    ];
    lines.extend(
        held.iter()
            .map(|(chain, watch, _)| format!("held:{chain}:{watch}")),
    );
    lines.extend(overrides.iter().map(|(id, _, _)| format!("override:{id}")));
    lines.extend(
        work_token_lines(conn, &live)?
            .into_iter()
            .map(|l| format!("work:{l}")),
    );
    if let Some(target) = &target {
        lines.extend(
            work_token_lines(conn, target)?
                .into_iter()
                .map(|l| format!("target:{l}")),
        );
    }
    Ok(MoveFacts {
        token: observe::sha256_hex(lines.join("\n").as_bytes()),
        live,
        target,
        files,
        members,
        kept,
        disputed,
        records_moving,
        held,
        overrides,
    })
}

fn invalid(message: &str) -> AppError {
    memory_error(ErrorCode::InvalidInput, message, false, None)
}

fn conflict(message: &str) -> AppError {
    memory_error(ErrorCode::DecisionConflict, message, false, None)
}

#[cfg(test)]
#[path = "family_move_tests.rs"]
mod tests;
