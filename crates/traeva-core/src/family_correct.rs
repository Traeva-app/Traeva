//! Correcting one relation (#31 material §7 "纠错", §6 statuses, §8 last
//! paragraph; the 2026-10-10 decisions on #31; shared-contract.md §4.3a).
//!
//! Revoke: the relation stops counting (superseded, with its change row),
//! and its child version goes back to "same work, order unknown" (a new
//! unknown relation, as M1b writes it). Membership does not change (§7:
//! family 归属不自动改变), and nothing is joined again by itself (§7: 不自动
//! 把被移除的关系重新接上): a new relation only comes from an association
//! page. A relation Traeva confirmed from physical evidence (relocation) may
//! be revoked too; the page says so (relation.status), and the change row
//! keeps `confirmed_system` (decision 1).
//!
//! Confirm again: a disputed relation (moving members disputes the
//! relations that cross works, M2b-3b) is confirmed by the user once both
//! ends are in one work again, as `confirmed_user` (decision 4).
//!
//! Pausing: a record whose recording rests on the revoked continuation (its
//! current part of the line would start at another file) is held in the
//! same transaction: its watch stops, candidate capture turns off, its open
//! gaps end (`corrected`), and a hold row says why. Turning Memory on again
//! for its file resumes the same record; stopping it ends it (§7 last
//! bullet). Revoking a branch, or a continuation the record is not on,
//! pauses nothing.
//!
//! The preview reads no user file and writes nothing; confirming looks at
//! everything again under the execution lock, in one transaction, and
//! changes nothing when the page is outdated.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::candidates::page_outdated;
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    EdgeKind, MemoryRecordView, RelationStatus, VersionRelationView, from_text,
    latest_watch_of_chain_in,
};
use crate::family_merge::{WorkDecisionOutcome, check_ids, check_token};
use crate::lineage::{chain_line_in, line_parts, part_of};
use crate::memory_consent::memory_error;
use crate::memory_contracts::SnapshotRef;
use crate::observe;
use crate::snapshot::query_snapshots;
use crate::store::sqlite_error;
use crate::works::{
    WorkDecisionKind, WorkDecisionRow, hold_in, insert_edge_in, insert_work_decision_in,
    live_family_in, set_edge_status_in, work_token_lines,
};

/// What the user does with one relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationAction {
    Revoke,
    ConfirmAgain,
}

/// What correcting one relation would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationPreview {
    /// The relation's work (live).
    pub family_id: String,
    pub relation: VersionRelationView,
    pub can_revoke: bool,
    /// Disputed, and both of its files are in one work again.
    pub can_confirm_again: bool,
    /// Revoking leaves this version "same work, order unknown".
    pub leaves_unplaced: SnapshotRef,
    /// Records that pause when it is revoked.
    pub records_held: Vec<MemoryRecordView>,
    /// Bring it back to confirm.
    pub page_token: String,
}

/// Confirming a relation page. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationRequest {
    pub edge_id: String,
    pub action: RelationAction,
    pub page_token: String,
}

/// One relation, as its row.
struct EdgeFacts {
    edge_id: String,
    status: RelationStatus,
    child: String,
}

/// What the page shows, worked out on one connection.
struct RelationFacts {
    live: String,
    edge: EdgeFacts,
    can_confirm_again: bool,
    /// (record, its watch that is on) paused by revoking.
    held: Vec<(String, String)>,
    token: String,
}

impl OrganizeService {
    /// What revoking or confirming again relation `edge_id` would do.
    /// Reads no user file and writes nothing.
    pub fn relation_preview(&self, edge_id: &str) -> Result<RelationPreview, AppError> {
        let store = self.store()?;
        check_edge_id(edge_id)?;
        let _no_capture = self.lock_execution();
        let facts = {
            let conn = store.connection()?;
            relation_facts(&conn, edge_id)?
        };
        let work = self.memory_family(&facts.live)?;
        let relation = work
            .relations
            .iter()
            .find(|r| r.edge_id == edge_id)
            .cloned()
            .ok_or_else(not_correctable)?;
        let records_held = facts
            .held
            .iter()
            .map(|(chain_id, _)| self.memory_record(chain_id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RelationPreview {
            family_id: facts.live,
            leaves_unplaced: relation.child.clone(),
            relation,
            can_revoke: true,
            can_confirm_again: facts.can_confirm_again,
            records_held,
            page_token: facts.token,
        })
    }

    /// Revokes or confirms again the relation the page showed (see the
    /// module note).
    pub fn correct_relation(
        &self,
        request: &RelationRequest,
    ) -> Result<WorkDecisionOutcome, AppError> {
        let store = self.store()?;
        check_edge_id(&request.edge_id)?;
        check_token(&request.page_token)?;
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the correction", &err);
        let _no_capture = self.lock_execution();
        let now = self.now();
        let work_decision_id = uuid::Uuid::new_v4().to_string();
        let (live, held) = {
            let mut conn = store.connection()?;
            let tx = conn.transaction().map_err(fail)?;
            let facts = relation_facts(&tx, &request.edge_id)?;
            if facts.token != request.page_token {
                return Err(page_outdated(
                    "The relation or its work changed since the page was opened. Nothing was \
                     changed; open it again.",
                ));
            }
            let kind = match request.action {
                RelationAction::Revoke => WorkDecisionKind::Revoke,
                RelationAction::ConfirmAgain if facts.can_confirm_again => {
                    WorkDecisionKind::ConfirmAgain
                }
                RelationAction::ConfirmAgain => {
                    return Err(conflict(
                        "Only a disputed relation whose two files are in one work again can be \
                         confirmed again. Nothing was changed.",
                    ));
                }
            };
            insert_work_decision_in(
                &tx,
                &WorkDecisionRow {
                    work_decision_id: &work_decision_id,
                    kind,
                    family_id: &facts.live,
                    other_family_id: None,
                    edge_id: Some(&facts.edge.edge_id),
                    file_id: None,
                    choice: None,
                    parent_snapshot_id: None,
                    evidence: None,
                    page_token: &request.page_token,
                },
                now,
            )?;
            let mut held = Vec::new();
            if kind == WorkDecisionKind::Revoke {
                set_edge_status_in(
                    &tx,
                    &work_decision_id,
                    &facts.edge.edge_id,
                    facts.edge.status,
                    RelationStatus::Superseded,
                    now,
                )?;
                // Same work, order unknown: the one live placement of the
                // version now (the old one was superseded first).
                insert_edge_in(
                    &tx,
                    &work_decision_id,
                    &facts.live,
                    EdgeKind::Unplaced,
                    None,
                    &facts.edge.child,
                    RelationStatus::Unknown,
                    now,
                )?;
                for (chain_id, watch_id) in &facts.held {
                    hold_in(
                        &tx,
                        chain_id,
                        watch_id,
                        Some(&facts.edge.edge_id),
                        &work_decision_id,
                        now,
                    )?;
                    held.push(chain_id.clone());
                }
            } else {
                // Never upgraded to system evidence (material §6).
                set_edge_status_in(
                    &tx,
                    &work_decision_id,
                    &facts.edge.edge_id,
                    RelationStatus::Disputed,
                    RelationStatus::ConfirmedUser,
                    now,
                )?;
            }
            tx.commit().map_err(fail)?;
            (facts.live, held)
        };
        if !held.is_empty() {
            // As in `stop_memory_record`: the watcher was running until now.
            self.note_run_seen(true);
        }
        Ok(WorkDecisionOutcome {
            work_decision_id,
            kind: match request.action {
                RelationAction::Revoke => WorkDecisionKind::Revoke,
                RelationAction::ConfirmAgain => WorkDecisionKind::ConfirmAgain,
            },
            family: self.memory_family(&live)?,
            other_family_id: None,
            stopped_records: Vec::new(),
            held_records: held,
        })
    }
}

/// What the page shows about relation `edge_id`, and its token.
fn relation_facts(conn: &Connection, edge_id: &str) -> Result<RelationFacts, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let row: Option<(String, String, Option<String>, String, String)> = conn
        .query_row(
            "SELECT family_id, kind, parent_snapshot_id, child_snapshot_id, status
             FROM memory_edges WHERE edge_id = ?1",
            [edge_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(fail)?;
    let Some((family_id, kind, parent, child, status)) = row else {
        return Err(memory_error(
            ErrorCode::UnknownRelation,
            "There is no relation with this ID.",
            false,
            None,
        ));
    };
    let kind: EdgeKind = from_text(&kind).ok_or_else(not_correctable)?;
    let status: RelationStatus = from_text(&status).ok_or_else(not_correctable)?;
    // Only a relation between two versions that counts (or is disputed).
    let correctable = matches!(kind, EdgeKind::Continuation | EdgeKind::Branch)
        && matches!(
            status,
            RelationStatus::ConfirmedSystem
                | RelationStatus::ConfirmedUser
                | RelationStatus::Disputed
        );
    let Some(parent) = parent.filter(|_| correctable) else {
        return Err(not_correctable());
    };
    let live = live_family_in(conn, &family_id)?.ok_or_else(not_correctable)?;
    // Confirm again: both files are confirmed members of one work again.
    let work_of = |snapshot_id: &str| -> Result<Option<String>, AppError> {
        conn.query_row(
            "SELECT l.live_family_id FROM snapshots AS s
             JOIN memory_members AS m ON m.file_id = s.file_id
                  AND m.status IN ('confirmed_system', 'confirmed_user')
             JOIN memory_live_families AS l ON l.family_id = m.family_id
             WHERE s.snapshot_id = ?1",
            [snapshot_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(fail)
    };
    let parent_work = work_of(&parent)?;
    let can_confirm_again = status == RelationStatus::Disputed
        && parent_work.is_some()
        && parent_work == work_of(&child)?;
    // Records whose current part would start at another file once the
    // continuation stops: they pause.
    let mut held = Vec::new();
    if kind == EdgeKind::Continuation && status != RelationStatus::Disputed {
        let chains: Vec<String> = crate::family::family_facts_in(conn, &live)?
            .chains
            .into_iter()
            .map(|c| c.chain_id)
            .collect();
        let stopped: HashSet<String> = HashSet::from([edge_id.to_string()]);
        for chain_id in chains {
            let Some(watch) = latest_watch_of_chain_in(conn, &chain_id)?.filter(|w| w.active)
            else {
                continue;
            };
            let line = chain_line_in(conn, &chain_id)?;
            let first = |stopped: &HashSet<String>| {
                part_of(&line_parts(&line, stopped), &watch.file_id)
                    .map(|part| part.first_file_id.clone())
            };
            if first(&HashSet::new()) != first(&stopped) {
                held.push((chain_id, watch.watch_id));
            }
        }
    }
    // The child must be a saved version (the page shows it).
    if query_snapshots(conn, "WHERE snapshot_id = ?1", [&child])?.is_empty() {
        return Err(not_correctable());
    }
    let mut lines = vec![
        "memory-relation-1".to_string(),
        format!(
            "edge:{edge_id}:{}:{parent}:{child}",
            crate::family::to_text(status)
        ),
        format!("confirm_again:{can_confirm_again}"),
    ];
    lines.extend(
        held.iter()
            .map(|(chain_id, watch_id)| format!("held:{chain_id}:{watch_id}")),
    );
    lines.extend(
        work_token_lines(conn, &live)?
            .into_iter()
            .map(|l| format!("work:{l}")),
    );
    Ok(RelationFacts {
        token: observe::sha256_hex(lines.join("\n").as_bytes()),
        live,
        edge: EdgeFacts {
            edge_id: edge_id.to_string(),
            status,
            child,
        },
        can_confirm_again,
        held,
    })
}

fn check_edge_id(edge_id: &str) -> Result<(), AppError> {
    check_ids(&[edge_id]).map_err(|_| {
        memory_error(
            ErrorCode::InvalidInput,
            "The relation ID is not valid.",
            false,
            None,
        )
    })
}

fn not_correctable() -> AppError {
    conflict(
        "Only a confirmed or disputed relation between two saved versions can be corrected. \
         Nothing was changed.",
    )
}

fn conflict(message: &str) -> AppError {
    memory_error(ErrorCode::DecisionConflict, message, false, None)
}

#[cfg(test)]
#[path = "family_correct_tests.rs"]
mod tests;
