use std::fs;

use super::*;
use crate::candidates::tests::{enable, replace, start_record};
use crate::capture::CheckOutcome;
use crate::family::{MemoryRecordView, RecordState};
use crate::memory_contracts::{GapKind, PairBasis, PairCheck, PairEvidence};
use crate::test_fixture::{Fixture, quick_read};

fn preview_file(fixture: &Fixture, family_id: &str, file_id: &str) -> AssociationPreview {
    fixture
        .service()
        .association_preview_for_file(family_id, file_id)
        .unwrap()
}

fn preview_picked(
    fixture: &Fixture,
    family_id: &str,
    relative: &str,
) -> Result<AssociationPreview, AppError> {
    fixture.service().association_preview_with(
        family_id,
        &fixture.root_id,
        &fixture.path(relative),
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    )
}

fn confirm(
    fixture: &Fixture,
    shown: &AssociationPreview,
    choice: AssociateChoice,
) -> Result<WorkDecisionOutcome, AppError> {
    let subject = shown.request_subject.clone();
    fixture.service().associate_file_with(
        &AssociationRequest {
            family_id: shown.family_id.clone(),
            subject,
            choice,
            page_token: shown.page_token.clone(),
        },
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    )
}

fn code(result: Result<impl std::fmt::Debug, AppError>) -> ErrorCode {
    result.expect_err("expected a refusal").code
}

/// Every Memory row and file record: a preview or a refusal changes none.
fn rows(fixture: &Fixture) -> Vec<(String, String, String)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT 'member', member_id, family_id || ':' || status FROM memory_members
             UNION ALL SELECT 'edge', edge_id, family_id || ':' || status FROM memory_edges
             UNION ALL SELECT 'chain', chain_id, family_id FROM memory_chains
             UNION ALL SELECT 'watch', watch_id, status FROM memory_watches
             UNION ALL SELECT 'file', file_id, relative_path || ':' || presence FROM files
             UNION ALL SELECT 'snapshot', snapshot_id, file_id FROM snapshots
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

/// todo.txt recorded, with a second version saved in place.
fn todo_with_two_versions(fixture: &Fixture) -> MemoryRecordView {
    let todo = start_record(fixture, "todo.txt");
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\n").unwrap();
    fixture.check_memory();
    fixture.advance(1_000);
    todo
}

fn versions(fixture: &Fixture, file_id: &str) -> Vec<SnapshotRef> {
    query_snapshots(
        &fixture.db(),
        "WHERE file_id = ?1 ORDER BY sequence",
        [file_id],
    )
    .unwrap()
}

fn record(fixture: &Fixture, chain_id: &str) -> MemoryRecordView {
    fixture.service().memory_record(chain_id).unwrap()
}

#[test]
fn a_save_as_copy_is_hinted_then_associated_as_a_branch_from_the_version_it_copies() {
    let fixture = Fixture::new();
    let todo = todo_with_two_versions(&fixture);
    let todo_versions = versions(&fixture, &todo.origin_file_id);
    // Save As: the same bytes under a new name; Memory turned on for it.
    fs::write(fixture.path("todo-copy.txt"), "buy milk\nand eggs\n").unwrap();
    let copy = start_record(&fixture, "todo-copy.txt");
    let hint = &copy.same_content[0];
    assert_eq!(hint.family_id, todo.family_id);
    let before = rows(&fixture);
    let shown = preview_file(&fixture, &hint.family_id, &copy.origin_file_id);
    assert_eq!(rows(&fixture), before, "the preview writes nothing");
    assert_eq!(shown.standing, FileStanding::InOtherWork);
    assert_eq!(
        shown.other_family_id.as_deref(),
        Some(copy.family_id.as_str())
    );
    assert_eq!(shown.merge_scope.as_ref().unwrap().records.len(), 1);
    assert!(!shown.needs_recording);
    assert_eq!(
        shown.choices,
        [
            AssociateChoiceKind::Continue,
            AssociateChoiceKind::Branch,
            AssociateChoiceKind::Unknown,
            AssociateChoiceKind::Independent
        ]
    );
    assert_eq!(shown.evidence.same_bytes_as, [todo_versions[1].clone()]);
    assert!(shown.evidence.same_object_as.is_empty());
    assert_eq!(shown.fork_points, todo_versions);
    assert_eq!(shown.continue_from.len(), 1);
    assert_eq!(shown.continue_from[0].parent, todo_versions[1]);
    assert_eq!(
        shown.continue_from[0]
            .stops_record
            .as_ref()
            .map(|r| r.chain_id.as_str()),
        Some(todo.chain_id.as_str())
    );
    let outcome = confirm(
        &fixture,
        &shown,
        AssociateChoice::Branch {
            parent_snapshot_id: todo_versions[1].snapshot_id.clone(),
        },
    )
    .unwrap();
    assert_eq!(outcome.kind, WorkDecisionKind::Associate);
    assert!(outcome.stopped_records.is_empty());
    // The copy's whole work joined; both records go on recording.
    let work = &outcome.family;
    assert_eq!(work.family_id, todo.family_id);
    assert_eq!(
        work.merged_family_ids,
        std::slice::from_ref(&copy.family_id)
    );
    assert_eq!(record(&fixture, &todo.chain_id).state, RecordState::Active);
    assert_eq!(record(&fixture, &copy.chain_id).state, RecordState::Active);
    assert!(record(&fixture, &copy.chain_id).same_content.is_empty());
    let branch = work
        .relations
        .iter()
        .find(|r| r.child.file_id == copy.origin_file_id)
        .unwrap();
    assert_eq!(branch.kind, EdgeKind::Branch);
    assert_eq!(branch.status, RelationStatus::ConfirmedUser);
    assert_eq!(branch.parent.as_ref(), Some(&todo_versions[1]));
    assert!(branch.decided_at_ms.is_some());
    // The decision says what the page showed; one row was made.
    let (choice, evidence, other): (String, String, String) = fixture
        .db()
        .query_row(
            "SELECT choice, evidence, other_family_id FROM memory_work_decisions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (choice.as_str(), evidence.as_str(), other.as_str()),
        ("branch", "same_bytes", copy.family_id.as_str())
    );
    assert_eq!(fixture.count("memory_work_changes"), 1);
    assert_eq!(fixture.count("memory_family_merges"), 1);
    // Memory saw nothing between the two versions: the pair says so.
    let copy_first = &versions(&fixture, &copy.origin_file_id)[0];
    let pair = match fixture
        .service()
        .check_usable_pair(&todo_versions[1].snapshot_id, &copy_first.snapshot_id)
        .unwrap()
    {
        PairCheck::Usable { pair } => pair,
        other => panic!("expected a usable pair, got {other:?}"),
    };
    assert_eq!(pair.basis, PairBasis::Branch);
    assert_eq!(pair.evidence, PairEvidence::User);
    assert!(pair.gap);
    assert!(pair.gap_kinds.contains(&GapKind::Skipped));
}

#[test]
fn continuing_stops_the_record_that_ended_at_the_parent_and_starts_no_watch() {
    let fixture = Fixture::new();
    let todo = todo_with_two_versions(&fixture);
    let last = versions(&fixture, &todo.origin_file_id).pop().unwrap();
    fs::write(
        fixture.path("todo-next.txt"),
        "buy milk\nand eggs\nand bread\n",
    )
    .unwrap();
    let next = start_record(&fixture, "todo-next.txt");
    assert!(next.same_content.is_empty());
    let watches = fixture.count("memory_watches");
    let shown = preview_file(&fixture, &todo.family_id, &next.origin_file_id);
    assert!(shown.evidence.same_bytes_as.is_empty());
    // The file saved again: its first version is still the relation's end.
    fixture.advance(1_000);
    fs::write(
        fixture.path("todo-next.txt"),
        "buy milk\nand eggs\nand jam\n",
    )
    .unwrap();
    fixture.check_memory();
    assert_eq!(versions(&fixture, &next.origin_file_id).len(), 2);
    let outcome = confirm(
        &fixture,
        &shown,
        AssociateChoice::Continue {
            parent_snapshot_id: last.snapshot_id.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        outcome.stopped_records,
        std::slice::from_ref(&todo.chain_id)
    );
    assert_eq!(fixture.count("memory_watches"), watches, "no watch starts");
    assert_eq!(record(&fixture, &todo.chain_id).state, RecordState::Stopped);
    assert_eq!(record(&fixture, &next.chain_id).state, RecordState::Active);
    let evidence: String = fixture
        .db()
        .query_row("SELECT evidence FROM memory_work_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(evidence, "none");
    // The pair holds lines of both files: both permissions count.
    let next_first = &versions(&fixture, &next.origin_file_id)[0];
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&todo.family_id)
        .unwrap();
    let pair = pairs
        .iter()
        .find(|p| p.after.snapshot_id == next_first.snapshot_id)
        .unwrap();
    assert_eq!(pair.basis, PairBasis::Continuation);
    assert_eq!(
        pair.permission_file_ids,
        [todo.origin_file_id.clone(), next.origin_file_id.clone()]
    );
    assert!(pair.gap_kinds.contains(&GapKind::Skipped));
    // The old file's line went on: it is not turned on again.
    let before = fixture.count("memory_watches");
    assert!(
        fixture
            .service()
            .start_memory_for_file_with(&todo.origin_file_id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            })
            .is_err()
    );
    assert_eq!(fixture.count("memory_watches"), before);
    // It went on once: no other file continues from that version.
    fs::write(fixture.path("other.txt"), "something else\n").unwrap();
    let other = start_record(&fixture, "other.txt");
    let shown = preview_file(&fixture, &todo.family_id, &other.origin_file_id);
    assert!(
        shown
            .continue_from
            .iter()
            .all(|c| c.parent.snapshot_id != last.snapshot_id)
    );
}

#[test]
fn a_file_with_no_version_is_read_once_and_only_its_membership_is_confirmed() {
    let fixture = Fixture::new();
    let todo = todo_with_two_versions(&fixture);
    fs::write(fixture.path("draft.txt"), "buy milk\nand eggs\n").unwrap();
    let before = rows(&fixture);
    let mut reads = 0;
    let shown = fixture
        .service()
        .association_preview_with(
            &todo.family_id,
            &fixture.root_id,
            &fixture.path("draft.txt"),
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap();
    assert_eq!(reads, 1);
    assert_eq!(rows(&fixture), before, "nothing is registered or saved");
    assert_eq!(shown.standing, FileStanding::NotRecorded);
    assert_eq!(shown.subject.file_id, None);
    assert_eq!(shown.subject.place.relative_path, "draft.txt");
    assert!(shown.needs_recording);
    assert_eq!(
        shown.choices,
        [
            AssociateChoiceKind::Unknown,
            AssociateChoiceKind::Independent
        ]
    );
    // Its bytes are those of todo's second version: evidence, not a relation.
    let todo_versions = versions(&fixture, &todo.origin_file_id);
    assert_eq!(
        shown.subject.sha256.as_deref(),
        Some(todo_versions[1].sha256.as_str())
    );
    assert_eq!(shown.evidence.same_bytes_as, [todo_versions[1].clone()]);
    assert!(
        code(confirm(
            &fixture,
            &shown,
            AssociateChoice::Branch {
                parent_snapshot_id: todo_versions[0].snapshot_id.clone(),
            },
        )) == ErrorCode::DecisionConflict
    );
    let outcome = confirm(&fixture, &shown, AssociateChoice::Unknown {}).unwrap();
    let draft = outcome
        .family
        .members
        .iter()
        .find(|m| m.display_path.ends_with("/draft.txt"))
        .unwrap();
    assert_eq!(draft.status, RelationStatus::ConfirmedUser);
    assert_eq!(draft.basis, crate::family::MemberBasis::UserDecision);
    assert!(draft.snapshots.is_empty());
    assert!(!draft.placed);
    assert!(draft.decided_at_ms.is_some());
    assert_eq!(
        fixture.count("memory_watches"),
        1,
        "Memory is not turned on"
    );
    assert!(versions(&fixture, &draft.file_id).is_empty());
    assert!(
        outcome
            .family
            .relations
            .iter()
            .all(|r| r.child.file_id != draft.file_id)
    );
    // Picked again, it is this work's member with nothing to place yet.
    assert_eq!(
        code(preview_picked(&fixture, &todo.family_id, "draft.txt")),
        ErrorCode::DecisionConflict
    );
    // Turned on, it records in this work; then it can be placed.
    let watch = fixture.start_memory(&draft.file_id).unwrap();
    assert_eq!(watch.file_id, draft.file_id);
    let chain = fixture.service().memory_records().unwrap();
    let draft_record = chain
        .iter()
        .find(|r| r.origin_file_id == draft.file_id)
        .unwrap();
    assert_eq!(draft_record.family_id, todo.family_id);
    let shown = preview_file(&fixture, &todo.family_id, &draft.file_id);
    assert_eq!(shown.standing, FileStanding::MemberWithoutParent);
    assert_eq!(
        shown.choices,
        [AssociateChoiceKind::Continue, AssociateChoiceKind::Branch]
    );
    let outcome = confirm(
        &fixture,
        &shown,
        AssociateChoice::Branch {
            parent_snapshot_id: todo_versions[1].snapshot_id.clone(),
        },
    )
    .unwrap();
    let placed = outcome
        .family
        .members
        .iter()
        .find(|m| m.file_id == draft.file_id)
        .unwrap();
    assert!(placed.placed);
    // Placed now: picking it again is refused.
    assert_eq!(
        code(
            fixture
                .service()
                .association_preview_for_file(&todo.family_id, &draft.file_id)
        ),
        ErrorCode::DecisionConflict
    );
}

#[test]
fn independent_is_kept_and_a_later_positive_choice_supersedes_it_with_its_reason() {
    let fixture = Fixture::new();
    let todo = todo_with_two_versions(&fixture);
    fs::write(fixture.path("letter.txt"), "Dear Sam,\n").unwrap();
    let shown = preview_picked(&fixture, &todo.family_id, "letter.txt").unwrap();
    let outcome = confirm(&fixture, &shown, AssociateChoice::Independent {}).unwrap();
    let letter = outcome
        .family
        .members
        .iter()
        .find(|m| m.display_path.ends_with("/letter.txt"))
        .unwrap()
        .clone();
    assert_eq!(letter.status, RelationStatus::Rejected);
    // The same page again: already confirmed.
    assert_eq!(
        code(confirm(&fixture, &shown, AssociateChoice::Independent {})),
        ErrorCode::PageOutdated
    );
    // Independent is not offered again; a positive choice lists what it
    // overrides.
    let shown = preview_picked(&fixture, &todo.family_id, "letter.txt").unwrap();
    assert_eq!(
        shown.subject.file_id.as_deref(),
        Some(letter.file_id.as_str())
    );
    assert_eq!(shown.choices, [AssociateChoiceKind::Unknown]);
    assert_eq!(shown.overrides.len(), 1);
    assert_eq!(shown.overrides[0].file_id, letter.file_id);
    assert_eq!(
        code(confirm(&fixture, &shown, AssociateChoice::Independent {})),
        ErrorCode::DecisionConflict
    );
    let outcome = confirm(&fixture, &shown, AssociateChoice::Unknown {}).unwrap();
    let member = outcome
        .family
        .members
        .iter()
        .find(|m| m.file_id == letter.file_id)
        .unwrap();
    assert_eq!(member.status, RelationStatus::ConfirmedUser);
    let changes: Vec<(Option<String>, String)> = {
        let db = fixture.db();
        let mut stmt = db
            .prepare(
                "SELECT from_status, to_status FROM memory_work_changes
                 WHERE member_id IS NOT NULL ORDER BY rowid",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        changes,
        [
            (None, "rejected".to_string()),
            (Some("rejected".to_string()), "superseded".to_string()),
            (None, "confirmed_user".to_string()),
        ]
    );
}

#[test]
fn independent_from_another_work_leaves_that_work_as_it_was() {
    let fixture = Fixture::new();
    let todo = todo_with_two_versions(&fixture);
    let plan = start_record(&fixture, "Notes/2026/plan.md");
    let shown = preview_file(&fixture, &todo.family_id, &plan.origin_file_id);
    assert_eq!(shown.standing, FileStanding::InOtherWork);
    // Saved after todo's versions: it may continue or branch from them.
    assert!(shown.choices.contains(&AssociateChoiceKind::Continue));
    confirm(&fixture, &shown, AssociateChoice::Independent {}).unwrap();
    assert_eq!(fixture.count("memory_family_merges"), 0);
    assert_eq!(fixture.service().memory_works().unwrap().len(), 2);
    let own = fixture.service().memory_family(&plan.family_id).unwrap();
    assert_eq!(own.members[0].status, RelationStatus::ConfirmedSystem);
    // Its rejection in todo's work keeps a merge from proposing it silently.
    let merge = fixture
        .service()
        .merge_preview(&todo.family_id, &plan.family_id)
        .unwrap();
    assert_eq!(merge.overrides.len(), 1);
}

#[test]
fn waiting_candidates_moved_record_files_and_changed_pages_are_refused_without_writes() {
    let fixture = Fixture::new();
    let todo = todo_with_two_versions(&fixture);
    let plan = start_record(&fixture, "Notes/2026/plan.md");
    // The unread object at a record's place is decided in that record.
    replace(&fixture, "Notes/2026/plan.md", "# Plan\n\n- replaced\n");
    let before = rows(&fixture);
    assert_eq!(
        code(preview_picked(
            &fixture,
            &todo.family_id,
            "Notes/2026/plan.md"
        )),
        ErrorCode::CandidatePending
    );
    // A saved candidate waits for its decision there too.
    enable(&fixture, &plan.chain_id).unwrap();
    let candidate: String = fixture
        .db()
        .query_row(
            "SELECT file_id FROM memory_candidates WHERE state = 'pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let before_candidate = rows(&fixture);
    assert_eq!(
        code(
            fixture
                .service()
                .association_preview_for_file(&todo.family_id, &candidate)
        ),
        ErrorCode::CandidatePending
    );
    assert_eq!(rows(&fixture), before_candidate);
    assert_ne!(
        rows(&fixture),
        before,
        "the candidate was saved by enabling"
    );
    // A record's file moved outside Traeva: its record chooses the place.
    fs::rename(fixture.path("todo.txt"), fixture.path("moved.txt")).unwrap();
    let before = rows(&fixture);
    assert_eq!(
        code(preview_picked(&fixture, &plan.family_id, "moved.txt")),
        ErrorCode::DecisionConflict
    );
    assert_eq!(rows(&fixture), before);
    fs::rename(fixture.path("moved.txt"), fixture.path("todo.txt")).unwrap();
    // A page whose parent is no longer the last version is outdated.
    fs::write(fixture.path("later.txt"), "later\n").unwrap();
    let later = start_record(&fixture, "later.txt");
    let shown = preview_file(&fixture, &todo.family_id, &later.origin_file_id);
    let last = shown.continue_from[0].parent.snapshot_id.clone();
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\nand tea\n").unwrap();
    assert!(
        fixture
            .check_memory()
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::Saved { .. }))
    );
    let before = rows(&fixture);
    assert_eq!(
        code(confirm(
            &fixture,
            &shown,
            AssociateChoice::Continue {
                parent_snapshot_id: last
            }
        )),
        ErrorCode::PageOutdated
    );
    assert_eq!(rows(&fixture), before);
    // A version the page did not offer is refused.
    let shown = preview_file(&fixture, &todo.family_id, &later.origin_file_id);
    assert_eq!(
        code(confirm(
            &fixture,
            &shown,
            AssociateChoice::Continue {
                parent_snapshot_id: versions(&fixture, &todo.origin_file_id)[0]
                    .snapshot_id
                    .clone()
            }
        )),
        ErrorCode::DecisionConflict
    );
    assert_eq!(rows(&fixture), before);
}

#[test]
fn requests_refuse_unknown_fields_and_bad_ids() {
    let fixture = Fixture::new();
    let parsed: Result<AssociationRequest, _> = serde_json::from_value(serde_json::json!({
        "family_id": "x", "page_token": "y",
        "subject": {"kind": "file", "file_id": "z", "extra": 1},
        "choice": {"kind": "unknown"},
    }));
    assert!(parsed.is_err());
    let parsed: Result<AssociationRequest, _> = serde_json::from_value(serde_json::json!({
        "family_id": "x", "page_token": "y",
        "subject": {"kind": "picked", "root_id": "r", "relative_path": "a.txt"},
        "choice": {"kind": "continue", "parent_snapshot_id": "p"},
    }));
    assert!(parsed.is_ok());
    for choice in [
        serde_json::json!({"kind": "unknown", "why": 1}),
        serde_json::json!({"kind": "independent", "why": 1}),
    ] {
        let parsed: Result<AssociationRequest, _> = serde_json::from_value(serde_json::json!({
            "family_id": "x", "page_token": "y",
            "subject": {"kind": "file", "file_id": "z"}, "choice": choice,
        }));
        assert!(parsed.is_err(), "{parsed:?}");
    }
    let parsed: Result<AssociationRequest, _> = serde_json::from_value(serde_json::json!({
        "family_id": "x", "page_token": "y",
        "subject": {"kind": "file", "file_id": "z"}, "choice": {"kind": "unknown"},
    }));
    assert_eq!(parsed.unwrap().choice, AssociateChoice::Unknown {});
    assert_eq!(
        code(
            fixture
                .service()
                .association_preview_for_file("not-a-uuid", &uuid::Uuid::new_v4().to_string())
        ),
        ErrorCode::InvalidInput
    );
    let todo = start_record(&fixture, "todo.txt");
    assert_eq!(
        code(
            fixture.service().association_preview_for_file(
                &uuid::Uuid::new_v4().to_string(),
                &todo.origin_file_id
            )
        ),
        ErrorCode::UnknownFamily
    );
    // A picked path that is not the one the page showed.
    let shown = {
        fs::write(fixture.path("draft.txt"), "draft\n").unwrap();
        preview_picked(&fixture, &todo.family_id, "draft.txt").unwrap()
    };
    let result = fixture.service().associate_file_with(
        &AssociationRequest {
            family_id: shown.family_id.clone(),
            subject: AssociationSubject::Picked {
                root_id: fixture.root_id.clone(),
                relative_path: "../draft.txt".to_string(),
            },
            choice: AssociateChoice::Unknown {},
            page_token: shown.page_token.clone(),
        },
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    );
    assert_eq!(code(result), ErrorCode::InvalidInput);
}

#[test]
fn placing_a_member_of_unknown_order_supersedes_its_unknown_relation_first() {
    use crate::family_decide::{CandidateChoice, CandidateDecision, CandidateDecisionRequest};
    let fixture = Fixture::new().without_auto_link();
    let todo = start_record(&fixture, "todo.txt");
    enable(&fixture, &todo.chain_id).unwrap();
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "buy milk\nand eggs\n");
    assert!(
        fixture
            .check_memory()
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::CandidateSaved { .. }))
    );
    let page = fixture
        .service()
        .pending_candidates(&todo.chain_id)
        .unwrap();
    let candidate_file = page.items[0].file_id.clone();
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: todo.chain_id.clone(),
            page_token: page.page_token,
            items: vec![CandidateDecision {
                candidate_id: page.items[0].candidate_id.clone(),
                choice: CandidateChoice::Unknown {},
            }],
            current_object: None,
        })
        .unwrap();
    let unknown: String = fixture
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE status = 'unknown'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let shown = preview_file(&fixture, &todo.family_id, &candidate_file);
    assert_eq!(shown.standing, FileStanding::MemberWithoutParent);
    assert_eq!(
        shown.replaces_unknown_placement.as_deref(),
        Some(unknown.as_str())
    );
    let todo_first = versions(&fixture, &todo.origin_file_id)[0].clone();
    assert_eq!(shown.fork_points, std::slice::from_ref(&todo_first));
    confirm(
        &fixture,
        &shown,
        AssociateChoice::Branch {
            parent_snapshot_id: todo_first.snapshot_id.clone(),
        },
    )
    .unwrap();
    let edges: Vec<(String, String, Option<String>)> = {
        let db = fixture.db();
        let mut stmt = db
            .prepare("SELECT kind, status, parent_snapshot_id FROM memory_edges ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        edges,
        [
            ("unplaced".to_string(), "superseded".to_string(), None),
            (
                "branch".to_string(),
                "confirmed_user".to_string(),
                Some(todo_first.snapshot_id.clone())
            ),
        ]
    );
    let change: (Option<String>, String) = fixture
        .db()
        .query_row(
            "SELECT from_status, to_status FROM memory_work_changes WHERE edge_id = ?1",
            [&unknown],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        change,
        (Some("unknown".to_string()), "superseded".to_string())
    );
}

#[test]
fn association_json_names_are_fixed() {
    let fixture = Fixture::new();
    let keys = |value: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let todo = todo_with_two_versions(&fixture);
    fs::write(fixture.path("todo-copy.txt"), "buy milk\nand eggs\n").unwrap();
    let copy = start_record(&fixture, "todo-copy.txt");
    let shown = preview_file(&fixture, &todo.family_id, &copy.origin_file_id);
    let json = serde_json::to_value(&shown).unwrap();
    assert_eq!(
        keys(&json),
        [
            "choices",
            "continue_from",
            "evidence",
            "family_id",
            "fork_points",
            "merge_scope",
            "needs_recording",
            "other_family_id",
            "overrides",
            "page_token",
            "replaces_unknown_placement",
            "request_subject",
            "standing",
            "subject"
        ]
    );
    assert_eq!(json["standing"], "in_other_work");
    assert_eq!(
        json["choices"],
        serde_json::json!(["continue", "branch", "unknown", "independent"])
    );
    assert_eq!(
        json["request_subject"],
        serde_json::json!({"kind": "file", "file_id": copy.origin_file_id})
    );
    assert_eq!(
        keys(&json["subject"]),
        ["file_id", "place", "sha256", "versions"]
    );
    assert_eq!(keys(&json["evidence"]), ["same_bytes_as", "same_object_as"]);
    assert_eq!(keys(&json["continue_from"][0]), ["parent", "stops_record"]);
    fs::write(fixture.path("draft.txt"), "draft\n").unwrap();
    let picked =
        serde_json::to_value(preview_picked(&fixture, &todo.family_id, "draft.txt").unwrap())
            .unwrap();
    assert_eq!(picked["standing"], "not_recorded");
    assert_eq!(
        picked["request_subject"],
        serde_json::json!({"kind": "picked", "root_id": fixture.root_id, "relative_path": "draft.txt"})
    );
    let request: AssociationRequest = serde_json::from_value(serde_json::json!({
        "family_id": shown.family_id,
        "subject": json["request_subject"],
        "choice": {"kind": "branch", "parent_snapshot_id": shown.fork_points[1].snapshot_id},
        "page_token": shown.page_token,
    }))
    .unwrap();
    let outcome =
        serde_json::to_value(fixture.service().associate_file(&request).unwrap()).unwrap();
    assert_eq!(outcome["kind"], "associate");
    assert_eq!(outcome["stopped_records"], serde_json::json!([]));
}
