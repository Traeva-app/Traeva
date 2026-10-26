use std::fs;

use super::*;
use crate::candidates::tests::{enable, only, replace, start_record};
use crate::capture::CheckOutcome;
use crate::family::{HoldReason, MemoryRecordView, RecordState};
use crate::family_correct::{RelationAction, RelationRequest};
use crate::family_decide::{CandidateChoice, CandidateDecision, CandidateDecisionRequest};
use crate::family_merge::MergeRequest;
use crate::memory_contracts::{PairCheck, PairRefusal};
use crate::test_fixture::{Fixture, quick_read};

fn code(result: Result<impl std::fmt::Debug, AppError>) -> ErrorCode {
    result.expect_err("expected a refusal").code
}

fn query(family_id: &str, files: &[&str], target: MoveTarget) -> MoveQuery {
    MoveQuery {
        family_id: family_id.to_string(),
        file_ids: files.iter().map(|f| f.to_string()).collect(),
        target,
    }
}

fn confirm(fixture: &Fixture, q: &MoveQuery, shown: &MovePreview) -> WorkDecisionOutcome {
    fixture
        .service()
        .move_members(&MoveRequest {
            family_id: q.family_id.clone(),
            file_ids: q.file_ids.clone(),
            target: q.target.clone(),
            page_token: shown.page_token.clone(),
        })
        .unwrap()
}

/// Every Memory row: a preview or a refusal changes none.
fn rows(fixture: &Fixture) -> Vec<(String, String, String)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT 'member', member_id, family_id || ':' || status FROM memory_members
             UNION ALL SELECT 'edge', edge_id, status FROM memory_edges
             UNION ALL SELECT 'family', family_id, '' FROM memory_families
             UNION ALL SELECT 'watch', watch_id, status FROM memory_watches
             UNION ALL SELECT 'decision', work_decision_id, kind FROM memory_work_decisions
             ORDER BY 1, 2",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn merge(fixture: &Fixture, stays: &str, joins: &str) {
    let shown = fixture.service().merge_preview(stays, joins).unwrap();
    fixture
        .service()
        .merge_works(&MergeRequest {
            family_id: shown.family_id,
            other_family_id: shown.other_family_id,
            page_token: shown.page_token,
        })
        .unwrap();
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

/// todo.txt recorded with capture on; an app saves N1 over it and the user
/// confirms N1 continues the work: one record, its line todo → N1.
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
    let edge: String = fixture
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE kind = 'continuation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let record = fixture.service().memory_record(&todo.chain_id).unwrap();
    (record, item.file_id.clone(), edge)
}

#[test]
fn a_moved_file_takes_its_record_and_the_relation_that_would_cross_is_disputed() {
    let fixture = Fixture::new().without_auto_link();
    let (record, n1, edge) = continued_record(&fixture);
    let work = record.family_id.clone();
    let todo_last = snapshot_of(&fixture, &record.origin_file_id, 1);
    let n1_first = snapshot_of(&fixture, &n1, 1);
    let q = query(&work, &[&n1], MoveTarget::NewWork {});
    let before = rows(&fixture);
    let shown = fixture.service().move_preview(&q).unwrap();
    assert_eq!(rows(&fixture), before, "the preview writes nothing");
    assert_eq!(shown.family_id, work);
    assert_eq!(shown.target_family_id, None);
    assert_eq!(shown.moving.len(), 1);
    assert_eq!(shown.moving[0].file_id, n1);
    assert_eq!(shown.records_moving.len(), 1);
    assert_eq!(shown.relations_disputed.len(), 1);
    assert_eq!(shown.relations_disputed[0].edge_id, edge);
    assert!(shown.relations_kept.is_empty());
    // The record went on by that relation: it pauses (decision 2).
    assert_eq!(shown.records_held.len(), 1);
    let outcome = confirm(&fixture, &q, &shown);
    assert_eq!(outcome.kind, WorkDecisionKind::Move);
    assert_eq!(outcome.held_records, std::slice::from_ref(&record.chain_id));
    let target = outcome.other_family_id.clone().unwrap();
    // The source keeps todo; the relation is shown there as crossing.
    let source = &outcome.family;
    assert_eq!(source.members.len(), 1);
    assert_eq!(source.members[0].file_id, record.origin_file_id);
    assert!(source.records.is_empty(), "the record went with N1");
    let crossing = source.relations.iter().find(|r| r.edge_id == edge).unwrap();
    assert_eq!(crossing.status, RelationStatus::Disputed);
    assert!(crossing.crosses_works);
    // The new work holds N1, its record, and the same crossing relation.
    let moved = fixture.service().memory_family(&target).unwrap();
    assert_eq!(moved.members.len(), 1);
    assert_eq!(moved.members[0].file_id, n1);
    assert_eq!(moved.members[0].status, RelationStatus::ConfirmedUser);
    assert_eq!(moved.members[0].basis, MemberBasis::Correction);
    assert_eq!(moved.records.len(), 1);
    assert!(
        moved
            .relations
            .iter()
            .any(|r| r.edge_id == edge && r.crosses_works)
    );
    assert_eq!(fixture.service().memory_works().unwrap().len(), 2);
    // The pair across the works is not used, and says why.
    assert_eq!(
        fixture
            .service()
            .check_usable_pair(&todo_last, &n1_first)
            .unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::RelationDisputed
        }
    );
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
            (None, "confirmed_user".to_string()),
            (Some("confirmed_user".to_string()), "disputed".to_string()),
        ]
    );
    // Paused: turning Memory on again goes on in the new work, under N1's
    // own permission.
    let paused = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(paused.state, RecordState::Stopped);
    assert_eq!(paused.family_id, target);
    assert_eq!(paused.hold.unwrap().why, HoldReason::MovedApart);
    fixture
        .service()
        .start_memory_for_file_with(&n1, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    let resumed = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(resumed.state, RecordState::Active);
    assert_eq!(resumed.family_id, target);
    assert_eq!(resumed.permission_file_id, n1);
    // Once both ends are in one work again, the relation can be confirmed
    // again by the user.
    merge(&fixture, &work, &target);
    let relation = fixture.service().relation_preview(&edge).unwrap();
    assert!(relation.can_confirm_again);
    fixture
        .service()
        .correct_relation(&RelationRequest {
            edge_id: edge.clone(),
            action: RelationAction::ConfirmAgain,
            page_token: relation.page_token,
        })
        .unwrap();
    assert!(matches!(
        fixture
            .service()
            .check_usable_pair(&todo_last, &n1_first)
            .unwrap(),
        PairCheck::Usable { .. }
    ));
}

#[test]
fn moving_both_ends_keeps_their_relation_and_a_whole_line_holds_nothing() {
    let fixture = Fixture::new().without_auto_link();
    let (record, n1, edge) = continued_record(&fixture);
    let plan = start_record(&fixture, "Notes/2026/plan.md");
    merge(&fixture, &record.family_id, &plan.family_id);
    let work = record.family_id.clone();
    let files = [record.origin_file_id.as_str(), n1.as_str()];
    let q = query(&work, &files, MoveTarget::NewWork {});
    let shown = fixture.service().move_preview(&q).unwrap();
    assert_eq!(shown.relations_kept.len(), 1);
    assert_eq!(shown.relations_kept[0].edge_id, edge);
    assert!(shown.relations_disputed.is_empty());
    assert!(shown.records_held.is_empty());
    let outcome = confirm(&fixture, &q, &shown);
    assert!(outcome.held_records.is_empty());
    let target = outcome.other_family_id.unwrap();
    // plan.md stays with its record; todo's record and line moved.
    assert_eq!(outcome.family.members.len(), 1);
    assert_eq!(outcome.family.records.len(), 1);
    assert_eq!(outcome.family.records[0].chain_id, plan.chain_id);
    let moved = fixture.service().memory_family(&target).unwrap();
    assert_eq!(moved.records.len(), 1);
    assert_eq!(moved.records[0].chain_id, record.chain_id);
    assert_eq!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .state,
        RecordState::Active
    );
    // The relation still counts, now in the new work, on the user's word.
    let pairs = fixture.service().usable_pairs_of_family(&target).unwrap();
    let pair = pairs
        .iter()
        .find(|p| p.edge_id.as_deref() == Some(edge.as_str()))
        .unwrap();
    assert_eq!(pair.family_id, target);
    assert_eq!(
        pair.permission_file_ids,
        std::slice::from_ref(&record.origin_file_id)
    );
    // A new version is captured in the record's new work.
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\nand tea\n").unwrap();
    assert!(
        fixture
            .check_memory()
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::Saved { .. }))
    );
    assert!(
        fixture
            .service()
            .usable_pairs_of_family(&target)
            .unwrap()
            .len()
            > pairs.len()
    );
    // Renamed outside Traeva and relocated: the new file record joins the
    // record's work now, not the work its chain was made in.
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &record.chain_id,
            &fixture.root_id,
            &fixture.path("Taxes/todo.txt"),
        )
        .unwrap();
    let outcome = fixture
        .service()
        .relocate_record_with(
            &crate::relocate::RelocationRequest {
                chain_id: record.chain_id.clone(),
                root_id: shown.new_place.root_id.clone(),
                relative_path: shown.new_place.relative_path.clone(),
                page_token: shown.page_token.clone(),
            },
            &mut |root, parent, name| quick_read(parent, name, root.id().dev),
        )
        .unwrap();
    assert_eq!(outcome.record.family_id, target);
    let moved = fixture.service().memory_family(&target).unwrap();
    assert!(
        moved
            .members
            .iter()
            .any(|m| m.file_id == outcome.snapshot.file_id)
    );
    assert_eq!(
        fixture
            .service()
            .memory_family(&record.family_id)
            .unwrap()
            .members
            .len(),
        1
    );
}

#[test]
fn moving_into_a_work_that_judged_the_file_unrelated_overrides_that_judgement() {
    use crate::family_associate::{AssociateChoice, AssociationRequest};
    let fixture = Fixture::new();
    let todo = start_record(&fixture, "todo.txt");
    let plan = start_record(&fixture, "Notes/2026/plan.md");
    // plan.md judged unrelated to todo's work.
    let shown = fixture
        .service()
        .association_preview_for_file(&todo.family_id, &plan.origin_file_id)
        .unwrap();
    fixture
        .service()
        .associate_file(&AssociationRequest {
            family_id: shown.family_id,
            subject: shown.request_subject,
            choice: AssociateChoice::Independent {},
            page_token: shown.page_token,
        })
        .unwrap();
    // plan's work gets a second member (a file with no version yet).
    fs::write(fixture.path("draft.txt"), "draft\n").unwrap();
    let draft = fixture
        .service()
        .association_preview_with(
            &plan.family_id,
            &fixture.root_id,
            &fixture.path("draft.txt"),
            &mut |root, parent, name| quick_read(parent, name, root.id().dev),
        )
        .unwrap();
    fixture
        .service()
        .associate_file(&AssociationRequest {
            family_id: draft.family_id,
            subject: draft.request_subject,
            choice: AssociateChoice::Unknown {},
            page_token: draft.page_token,
        })
        .unwrap();
    let q = query(
        &plan.family_id,
        &[&plan.origin_file_id],
        MoveTarget::Work {
            family_id: todo.family_id.clone(),
        },
    );
    let shown = fixture.service().move_preview(&q).unwrap();
    assert_eq!(
        shown.target_family_id.as_deref(),
        Some(todo.family_id.as_str())
    );
    assert_eq!(shown.overrides.len(), 1);
    assert_eq!(shown.overrides[0].file_id, plan.origin_file_id);
    let outcome = confirm(&fixture, &q, &shown);
    assert_eq!(
        outcome.other_family_id.as_deref(),
        Some(todo.family_id.as_str())
    );
    let joined = fixture.service().memory_family(&todo.family_id).unwrap();
    let member = joined
        .members
        .iter()
        .find(|m| m.file_id == plan.origin_file_id)
        .unwrap();
    assert_eq!(member.status, RelationStatus::ConfirmedUser);
    assert_eq!(joined.records.len(), 2, "plan's record moved with it");
}

#[test]
fn bad_moves_are_refused_without_writes() {
    let fixture = Fixture::new().without_auto_link();
    let (record, n1, _) = continued_record(&fixture);
    let work = record.family_id.clone();
    let other = start_record(&fixture, "Notes/2026/plan.md");
    let before = rows(&fixture);
    let refused = |q: MoveQuery| code(fixture.service().move_preview(&q));
    assert_eq!(
        refused(query(&work, &[], MoveTarget::NewWork {})),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(query(&work, &[&n1, &n1], MoveTarget::NewWork {})),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(query(
            &work,
            &[&other.origin_file_id],
            MoveTarget::NewWork {}
        )),
        ErrorCode::DecisionConflict,
        "not a member of this work"
    );
    assert_eq!(
        refused(query(
            &work,
            &[&n1],
            MoveTarget::Work {
                family_id: work.clone()
            }
        )),
        ErrorCode::DecisionConflict
    );
    assert_eq!(
        refused(query(
            &work,
            &[&n1],
            MoveTarget::Work {
                family_id: uuid::Uuid::new_v4().to_string()
            }
        )),
        ErrorCode::UnknownFamily
    );
    assert_eq!(
        refused(query(
            &work,
            &[&n1, &record.origin_file_id],
            MoveTarget::NewWork {}
        )),
        ErrorCode::DecisionConflict,
        "every member would move"
    );
    assert_eq!(rows(&fixture), before);
    // A waiting candidate on the line keeps it in place.
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "another save\n");
    assert!(
        fixture
            .check_memory()
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::CandidateSaved { .. }))
    );
    assert_eq!(
        refused(query(&work, &[&n1], MoveTarget::NewWork {})),
        ErrorCode::CandidatePending
    );
    // A page whose works changed is outdated.
    let q = query(
        &other.family_id,
        &[&other.origin_file_id],
        MoveTarget::NewWork {},
    );
    assert_eq!(
        refused(q),
        ErrorCode::DecisionConflict,
        "plan.md is its work's only member"
    );
    let parsed: Result<MoveRequest, _> = serde_json::from_value(serde_json::json!({
        "family_id": work, "file_ids": [n1], "page_token": "a",
        "target": {"kind": "new_work", "name": "x"}
    }));
    assert!(parsed.is_err());
}

#[test]
fn an_outdated_or_reused_move_page_changes_nothing() {
    let fixture = Fixture::new().without_auto_link();
    let (record, n1, _) = continued_record(&fixture);
    let plan = start_record(&fixture, "Notes/2026/plan.md");
    merge(&fixture, &record.family_id, &plan.family_id);
    let q = query(
        &record.family_id,
        &[&plan.origin_file_id],
        MoveTarget::NewWork {},
    );
    let shown = fixture.service().move_preview(&q).unwrap();
    fixture
        .service()
        .stop_memory_record(&plan.chain_id)
        .unwrap();
    let before = rows(&fixture);
    let request = MoveRequest {
        family_id: q.family_id.clone(),
        file_ids: q.file_ids.clone(),
        target: q.target.clone(),
        page_token: shown.page_token.clone(),
    };
    assert_eq!(
        code(fixture.service().move_members(&request)),
        ErrorCode::PageOutdated
    );
    assert_eq!(rows(&fixture), before);
    let shown = fixture.service().move_preview(&q).unwrap();
    confirm(&fixture, &q, &shown);
    let again = MoveRequest {
        page_token: shown.page_token.clone(),
        ..request
    };
    assert!(fixture.service().move_members(&again).is_err());
    let _ = n1;
}

#[test]
fn move_json_names_are_fixed() {
    let fixture = Fixture::new().without_auto_link();
    let keys = |value: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let (record, n1, _) = continued_record(&fixture);
    let q = query(&record.family_id, &[&n1], MoveTarget::NewWork {});
    let shown = fixture.service().move_preview(&q).unwrap();
    let json = serde_json::to_value(&shown).unwrap();
    assert_eq!(
        keys(&json),
        [
            "family_id",
            "moving",
            "overrides",
            "page_token",
            "records_held",
            "records_moving",
            "relations_disputed",
            "relations_kept",
            "target_family_id"
        ]
    );
    let request: MoveRequest = serde_json::from_value(serde_json::json!({
        "family_id": record.family_id, "file_ids": [n1],
        "target": {"kind": "new_work"}, "page_token": shown.page_token
    }))
    .unwrap();
    let outcome = serde_json::to_value(fixture.service().move_members(&request).unwrap()).unwrap();
    assert_eq!(outcome["kind"], "move");
    assert!(outcome["other_family_id"].is_string());
    let paused =
        serde_json::to_value(fixture.service().memory_record(&record.chain_id).unwrap()).unwrap();
    assert_eq!(paused["hold"]["why"], "moved_apart");
    let work =
        serde_json::to_value(fixture.service().memory_family(&record.family_id).unwrap()).unwrap();
    assert_eq!(work["relations"][0]["crosses_works"], true);
    let target: MoveTarget =
        serde_json::from_value(serde_json::json!({"kind": "work", "family_id": "x"})).unwrap();
    assert_eq!(
        target,
        MoveTarget::Work {
            family_id: "x".to_string()
        }
    );
}
