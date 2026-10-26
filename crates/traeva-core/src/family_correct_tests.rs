use std::fs;

use super::*;
use crate::candidates::tests::{enable, only, replace, start_record};
use crate::capture::CheckOutcome;
use crate::family::{CandidateCapture, HoldReason, MemoryRecordView, RecordState};
use crate::family_decide::{CandidateChoice, CandidateDecision, CandidateDecisionRequest};
use crate::lineage::{ChainLine, LinePart};
use crate::memory_contracts::{PairCheck, PairRefusal};
use crate::test_fixture::{Fixture, quick_read};

fn code(result: Result<impl std::fmt::Debug, AppError>) -> ErrorCode {
    result.expect_err("expected a refusal").code
}

fn request(shown: &RelationPreview, action: RelationAction) -> RelationRequest {
    RelationRequest {
        edge_id: shown.relation.edge_id.clone(),
        action,
        page_token: shown.page_token.clone(),
    }
}

/// Every Memory row: a preview or a refusal changes none.
fn rows(fixture: &Fixture) -> Vec<(String, String, String)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT 'member', member_id, status FROM memory_members
             UNION ALL SELECT 'edge', edge_id, status FROM memory_edges
             UNION ALL SELECT 'watch', watch_id, status FROM memory_watches
             UNION ALL SELECT 'hold', hold_id, coalesce(release_reason, '-')
                       FROM memory_record_holds
             UNION ALL SELECT 'decision', work_decision_id, kind FROM memory_work_decisions
             UNION ALL SELECT 'change', change_id, to_status FROM memory_work_changes
             ORDER BY 1, 2",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// todo.txt recorded with candidate capture on; an app saves N1 over it,
/// and the user confirms N1 continues the work: the record now watches N1.
/// Returns the record, N1's file and the continuation.
fn continued_record(fixture: &Fixture) -> (MemoryRecordView, String, String) {
    let todo = start_record(fixture, "todo.txt");
    enable(fixture, &todo.chain_id).unwrap();
    fixture.advance(1_000);
    replace(fixture, "todo.txt", "buy milk\nand eggs\n");
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    let page = fixture
        .service()
        .pending_candidates(&todo.chain_id)
        .unwrap();
    let item = &page.items[0];
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: todo.chain_id.clone(),
            page_token: page.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: item.candidate_id.clone(),
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: item.continue_from.clone().unwrap().snapshot_id,
                },
            }],
            current_object: None,
        })
        .unwrap();
    let record = fixture.service().memory_record(&todo.chain_id).unwrap();
    assert_eq!(record.watch.file_id, item.file_id);
    let edge: String = fixture
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE kind = 'continuation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    (record, item.file_id.clone(), edge)
}

fn snapshot_of(fixture: &Fixture, file_id: &str, sequence: i64) -> String {
    fixture
        .db()
        .query_row(
            "SELECT snapshot_id FROM snapshots WHERE file_id = ?1 AND sequence = ?2",
            rusqlite::params![file_id, sequence],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn revoking_the_continuation_a_record_went_on_by_pauses_it_until_the_user_decides() {
    let fixture = Fixture::new().without_auto_link();
    let (record, n1, edge) = continued_record(&fixture);
    assert_eq!(record.permission_file_id, record.origin_file_id);
    let todo_last = snapshot_of(&fixture, &record.origin_file_id, 1);
    let n1_first = snapshot_of(&fixture, &n1, 1);
    let before = rows(&fixture);
    let shown = fixture.service().relation_preview(&edge).unwrap();
    assert_eq!(rows(&fixture), before, "the preview writes nothing");
    assert_eq!(shown.family_id, record.family_id);
    assert_eq!(shown.relation.status, RelationStatus::ConfirmedUser);
    assert!(shown.can_revoke);
    assert!(!shown.can_confirm_again);
    assert_eq!(shown.leaves_unplaced.snapshot_id, n1_first);
    assert_eq!(shown.records_held.len(), 1);
    assert_eq!(shown.records_held[0].chain_id, record.chain_id);
    let outcome = fixture
        .service()
        .correct_relation(&request(&shown, RelationAction::Revoke))
        .unwrap();
    assert_eq!(outcome.kind, WorkDecisionKind::Revoke);
    assert_eq!(outcome.held_records, std::slice::from_ref(&record.chain_id));
    // The relation stops counting, with its reason; N1 is still a member,
    // its order unknown.
    let revoked = outcome
        .family
        .relations
        .iter()
        .find(|r| r.edge_id == edge)
        .unwrap();
    assert_eq!(revoked.status, RelationStatus::Superseded);
    let unknown = outcome
        .family
        .relations
        .iter()
        .find(|r| r.child.snapshot_id == n1_first && r.status == RelationStatus::Unknown)
        .unwrap();
    assert_eq!(unknown.kind, EdgeKind::Unplaced);
    assert!(unknown.parent.is_none());
    let member = outcome
        .family
        .members
        .iter()
        .find(|m| m.file_id == n1)
        .unwrap();
    assert_eq!(member.status, RelationStatus::ConfirmedUser);
    assert!(!member.placed);
    let changes: Vec<(Option<String>, String)> = {
        let db = fixture.db();
        let mut stmt = db
            .prepare("SELECT from_status, to_status FROM memory_work_changes ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        changes,
        [
            (Some("confirmed_user".to_string()), "superseded".to_string()),
            (None, "unknown".to_string()),
        ]
    );
    // The pair across it is refused, and says why.
    assert_eq!(
        fixture
            .service()
            .check_usable_pair(&todo_last, &n1_first)
            .unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::RelationSuperseded
        }
    );
    // The record is paused: nothing is looked at, capture is off, and N1's
    // versions need N1's own permission now.
    let paused = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(paused.state, RecordState::Stopped);
    assert_eq!(paused.candidate_capture, CandidateCapture::Off);
    let hold = paused.hold.clone().unwrap();
    assert_eq!(hold.why, HoldReason::RelationRevoked);
    assert_eq!(hold.edge_id.as_deref(), Some(edge.as_str()));
    assert_eq!(paused.permission_file_id, n1);
    assert_eq!(paused.line_versions, 1);
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\nand tea\n").unwrap();
    assert!(fixture.check_memory().is_empty());
    // Turning Memory on again resumes the same record, after a recorded gap.
    let watches = fixture.count("memory_watches");
    fixture
        .service()
        .start_memory_for_file_with(&n1, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    assert_eq!(fixture.count("memory_watches"), watches + 1);
    let resumed = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(resumed.state, RecordState::Active);
    assert!(resumed.hold.is_none());
    assert_eq!(resumed.candidate_capture, CandidateCapture::Off);
    assert_eq!(resumed.permission_file_id, n1);
    let released: String = fixture
        .db()
        .query_row("SELECT release_reason FROM memory_record_holds", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(released, "resumed");
    // The pairs of the record's line are now only N1's.
    let line = fixture.service().usable_pairs_of_line(&n1).unwrap();
    assert!(
        line.iter()
            .all(|p| p.before.file_id == n1 && p.after.file_id == n1)
    );
    assert!(line.iter().all(|p| p.permission_file_ids == [n1.clone()]));
    assert!(line.iter().all(|p| p.unplaced));
}

#[test]
fn stopping_a_paused_record_releases_its_hold_and_old_pages_are_outdated() {
    let fixture = Fixture::new().without_auto_link();
    let (record, _, edge) = continued_record(&fixture);
    let shown = fixture.service().relation_preview(&edge).unwrap();
    let candidates = fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    fixture
        .service()
        .correct_relation(&request(&shown, RelationAction::Revoke))
        .unwrap();
    // The same page again; the relation is not correctable any more.
    assert_eq!(
        code(
            fixture
                .service()
                .correct_relation(&request(&shown, RelationAction::Revoke))
        ),
        ErrorCode::DecisionConflict
    );
    // An older candidate page does not reopen recording.
    let before = rows(&fixture);
    assert!(
        fixture
            .service()
            .decide_pending_candidates(&CandidateDecisionRequest {
                chain_id: record.chain_id.clone(),
                page_token: candidates.page_token,
                items: vec![],
                current_object: None,
            })
            .is_err()
    );
    assert_eq!(rows(&fixture), before);
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    let stopped = fixture.service().memory_record(&record.chain_id).unwrap();
    assert!(stopped.hold.is_none());
    let released: String = fixture
        .db()
        .query_row("SELECT release_reason FROM memory_record_holds", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(released, "stopped");
}

#[test]
fn revoking_a_branch_pauses_nothing_and_its_child_can_be_placed_again_by_association() {
    use crate::family_associate::{AssociateChoice, AssociateChoiceKind, FileStanding};
    let fixture = Fixture::new();
    let todo = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    fs::write(fixture.path("todo-copy.txt"), "buy milk\n").unwrap();
    let copy = start_record(&fixture, "todo-copy.txt");
    let todo_first = snapshot_of(&fixture, &todo.origin_file_id, 1);
    let shown = fixture
        .service()
        .association_preview_for_file(&todo.family_id, &copy.origin_file_id)
        .unwrap();
    fixture
        .service()
        .associate_file(&crate::family_associate::AssociationRequest {
            family_id: shown.family_id.clone(),
            subject: shown.request_subject.clone(),
            choice: AssociateChoice::Branch {
                parent_snapshot_id: todo_first.clone(),
            },
            page_token: shown.page_token.clone(),
        })
        .unwrap();
    let edge: String = fixture
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE kind = 'branch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let shown = fixture.service().relation_preview(&edge).unwrap();
    assert!(shown.records_held.is_empty());
    let outcome = fixture
        .service()
        .correct_relation(&request(&shown, RelationAction::Revoke))
        .unwrap();
    assert!(outcome.held_records.is_empty());
    for chain_id in [&todo.chain_id, &copy.chain_id] {
        assert_eq!(
            fixture.service().memory_record(chain_id).unwrap().state,
            RecordState::Active
        );
    }
    let copy_first = snapshot_of(&fixture, &copy.origin_file_id, 1);
    assert_eq!(
        fixture
            .service()
            .check_usable_pair(&todo_first, &copy_first)
            .unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::RelationSuperseded
        }
    );
    // Nothing is joined again by itself: only an association page does it.
    let again = fixture
        .service()
        .association_preview_for_file(&todo.family_id, &copy.origin_file_id)
        .unwrap();
    assert_eq!(again.standing, FileStanding::MemberWithoutParent);
    assert!(again.choices.contains(&AssociateChoiceKind::Branch));
    assert!(again.replaces_unknown_placement.is_some());
}

#[test]
fn a_disputed_relation_is_refused_as_disputed_and_can_be_confirmed_again_by_the_user() {
    let fixture = Fixture::new().without_auto_link();
    let (record, n1, edge) = continued_record(&fixture);
    let todo_last = snapshot_of(&fixture, &record.origin_file_id, 1);
    let n1_first = snapshot_of(&fixture, &n1, 1);
    // Moving members (M2b-3b) disputes relations; here one is disputed
    // with its recorded reason, as 0008 asks.
    let db = fixture.db();
    db.execute(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, edge_id,
             page_token, decided_at_ms)
         VALUES ('11111111-1111-4111-8111-111111111111', 'revoke', ?1, ?2, ?3, 1)",
        rusqlite::params![record.family_id, edge, "0".repeat(64)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memory_work_changes (change_id, work_decision_id, edge_id, from_status,
             to_status, changed_at_ms)
         VALUES ('c', '11111111-1111-4111-8111-111111111111', ?1, 'confirmed_user',
                 'disputed', 1)",
        [&edge],
    )
    .unwrap();
    db.execute(
        "UPDATE memory_edges SET status = 'disputed' WHERE edge_id = ?1",
        [&edge],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        fixture
            .service()
            .check_usable_pair(&todo_last, &n1_first)
            .unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::RelationDisputed
        }
    );
    let shown = fixture.service().relation_preview(&edge).unwrap();
    assert!(shown.can_confirm_again);
    assert!(
        shown.records_held.is_empty(),
        "a disputed relation holds nothing"
    );
    let outcome = fixture
        .service()
        .correct_relation(&request(&shown, RelationAction::ConfirmAgain))
        .unwrap();
    assert_eq!(outcome.kind, WorkDecisionKind::ConfirmAgain);
    let relation = outcome
        .family
        .relations
        .iter()
        .find(|r| r.edge_id == edge)
        .unwrap();
    assert_eq!(relation.status, RelationStatus::ConfirmedUser);
    assert!(matches!(
        fixture
            .service()
            .check_usable_pair(&todo_last, &n1_first)
            .unwrap(),
        PairCheck::Usable { .. }
    ));
    // Not disputed any more: it cannot be confirmed again.
    let shown = fixture.service().relation_preview(&edge).unwrap();
    assert!(!shown.can_confirm_again);
    assert_eq!(
        code(
            fixture
                .service()
                .correct_relation(&request(&shown, RelationAction::ConfirmAgain))
        ),
        ErrorCode::DecisionConflict
    );
}

#[test]
fn unknown_unplaced_and_changed_relations_are_refused_without_writes() {
    let fixture = Fixture::new().without_auto_link();
    assert_eq!(
        code(fixture.service().relation_preview("not-a-uuid")),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code(
            fixture
                .service()
                .relation_preview(&uuid::Uuid::new_v4().to_string())
        ),
        ErrorCode::UnknownRelation
    );
    let (record, _, edge) = continued_record(&fixture);
    let shown = fixture.service().relation_preview(&edge).unwrap();
    // The record changed meanwhile: the page is outdated.
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    let before = rows(&fixture);
    assert_eq!(
        code(
            fixture
                .service()
                .correct_relation(&request(&shown, RelationAction::Revoke))
        ),
        ErrorCode::PageOutdated
    );
    assert_eq!(rows(&fixture), before);
    // An unknown relation ("same work, order unknown") is not corrected
    // here.
    let shown = fixture.service().relation_preview(&edge).unwrap();
    assert!(
        shown.records_held.is_empty(),
        "a stopped record is not paused"
    );
    fixture
        .service()
        .correct_relation(&request(&shown, RelationAction::Revoke))
        .unwrap();
    let unknown: String = fixture
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE status = 'unknown'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        code(fixture.service().relation_preview(&unknown)),
        ErrorCode::DecisionConflict
    );
    let parsed: Result<RelationRequest, _> = serde_json::from_value(serde_json::json!({
        "edge_id": edge, "action": "revoke", "page_token": "a", "why": 1
    }));
    assert!(parsed.is_err());
    let parsed: Result<RelationRequest, _> = serde_json::from_value(serde_json::json!({
        "edge_id": edge, "action": "delete", "page_token": "a"
    }));
    assert!(parsed.is_err());
}

#[test]
fn a_line_falls_into_parts_where_a_continuation_stops() {
    let line = ChainLine {
        chain_id: "c".to_string(),
        origin_file_id: "a".to_string(),
        files: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        joins: vec![
            ("ab".to_string(), "a".to_string(), "b".to_string()),
            ("bc".to_string(), "b".to_string(), "c".to_string()),
        ],
    };
    let part = |first: &str, files: &[&str]| LinePart {
        chain_id: "c".to_string(),
        first_file_id: first.to_string(),
        file_ids: files.iter().map(|f| f.to_string()).collect(),
    };
    assert_eq!(
        line_parts(&line, &HashSet::new()),
        [part("a", &["a", "b", "c"])]
    );
    assert_eq!(
        line_parts(&line, &HashSet::from(["bc".to_string()])),
        [part("a", &["a", "b"]), part("c", &["c"])]
    );
    assert_eq!(
        line_parts(&line, &HashSet::from(["ab".to_string()])),
        [part("a", &["a"]), part("b", &["b", "c"])]
    );
}

#[test]
fn relation_json_names_are_fixed() {
    let fixture = Fixture::new().without_auto_link();
    let keys = |value: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let (record, _, edge) = continued_record(&fixture);
    let shown = fixture.service().relation_preview(&edge).unwrap();
    let json = serde_json::to_value(&shown).unwrap();
    assert_eq!(
        keys(&json),
        [
            "can_confirm_again",
            "can_revoke",
            "family_id",
            "leaves_unplaced",
            "page_token",
            "records_held",
            "relation"
        ]
    );
    let request: RelationRequest = serde_json::from_value(serde_json::json!({
        "edge_id": edge, "action": "revoke", "page_token": shown.page_token
    }))
    .unwrap();
    let outcome =
        serde_json::to_value(fixture.service().correct_relation(&request).unwrap()).unwrap();
    assert_eq!(outcome["kind"], "revoke");
    assert_eq!(
        outcome["held_records"],
        serde_json::json!([record.chain_id])
    );
    let paused =
        serde_json::to_value(fixture.service().memory_record(&record.chain_id).unwrap()).unwrap();
    assert_eq!(
        keys(&paused["hold"]),
        ["edge_id", "held_at_ms", "why", "work_decision_id"]
    );
    assert_eq!(paused["hold"]["why"], "relation_revoked");
    assert_eq!(
        serde_json::to_value([
            PairRefusal::RelationDisputed,
            PairRefusal::RelationSuperseded
        ])
        .unwrap(),
        serde_json::json!(["relation_disputed", "relation_superseded"])
    );
    assert_eq!(
        serde_json::to_value(ErrorCode::UnknownRelation).unwrap(),
        "unknown_relation"
    );
}
