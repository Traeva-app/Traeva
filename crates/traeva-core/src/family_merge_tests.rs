use std::fs;

use super::*;
use crate::candidates::tests::{enable, only, replace, start_record};
use crate::capture::CheckOutcome;
use crate::family::{MemoryRecordView, RecordState};
use crate::family_decide::{CandidateChoice, CandidateDecision, CandidateDecisionRequest};
use crate::memory_contracts::PairEvidence;
use crate::test_fixture::Fixture;

/// One watcher pass in which some record saved a candidate.
fn candidate_saved(fixture: &Fixture) {
    let checks = fixture.check_memory();
    assert!(
        checks
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::CandidateSaved { .. })),
        "{checks:?}"
    );
}

fn merge(fixture: &Fixture, stays: &str, joins: &str) -> WorkDecisionOutcome {
    let shown = fixture.service().merge_preview(stays, joins).unwrap();
    fixture.service().merge_works(&request(&shown)).unwrap()
}

fn request(shown: &MergePreview) -> MergeRequest {
    MergeRequest {
        family_id: shown.family_id.clone(),
        other_family_id: shown.other_family_id.clone(),
        page_token: shown.page_token.clone(),
    }
}

fn code(result: Result<impl std::fmt::Debug, AppError>) -> ErrorCode {
    result.expect_err("expected a refusal").code
}

/// Rows a merge must never copy or change.
fn rows(fixture: &Fixture) -> Vec<(String, String, String)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT 'member', member_id, family_id || ':' || status FROM memory_members
             UNION ALL SELECT 'edge', edge_id, family_id || ':' || status FROM memory_edges
             UNION ALL SELECT 'chain', chain_id, family_id FROM memory_chains
             ORDER BY 1, 2",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Two works: todo.txt and Notes/2026/plan.md, each turned on separately,
/// each with a second version saved in place.
fn two_works(fixture: &Fixture) -> (MemoryRecordView, MemoryRecordView) {
    let a = start_record(fixture, "todo.txt");
    let b = start_record(fixture, "Notes/2026/plan.md");
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\n").unwrap();
    fs::write(fixture.path("Notes/2026/plan.md"), "# Plan\n\n- ship M2\n").unwrap();
    fixture.check_memory();
    (a, b)
}

#[test]
fn a_merge_keeps_both_ids_writes_one_merge_record_and_copies_no_rows() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    let before = rows(&fixture);
    let shown = fixture
        .service()
        .merge_preview(&a.family_id, &b.family_id)
        .unwrap();
    assert_eq!(shown.family_id, a.family_id);
    assert_eq!(shown.other_family_id, b.family_id);
    assert_eq!(shown.stays.members.len(), 1);
    assert_eq!(shown.joins.members.len(), 1);
    assert!(shown.overrides.is_empty());
    assert_eq!(rows(&fixture), before, "the preview writes nothing");
    let outcome = fixture.service().merge_works(&request(&shown)).unwrap();
    assert_eq!(outcome.kind, WorkDecisionKind::Merge);
    assert_eq!(
        rows(&fixture),
        before,
        "no membership, relation or record is copied"
    );
    assert_eq!(fixture.count("memory_work_decisions"), 1);
    assert_eq!(fixture.count("memory_family_merges"), 1);
    assert_eq!(fixture.count("memory_work_changes"), 0);
    // Both IDs open the one work.
    let merged = &outcome.family;
    assert_eq!(merged.family_id, a.family_id);
    assert_eq!(merged.merged_family_ids, std::slice::from_ref(&b.family_id));
    assert_eq!(merged.members.len(), 2);
    assert_eq!(merged.records.len(), 2);
    let by_old = fixture.service().memory_family(&b.family_id).unwrap();
    assert_eq!(&by_old, merged);
    let joined = merged
        .members
        .iter()
        .find(|m| m.file_id == b.origin_file_id)
        .unwrap();
    assert_eq!(joined.merged_from.as_deref(), Some(b.family_id.as_str()));
    // Only live works are listed.
    let works = fixture.service().memory_works().unwrap();
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].family_id, a.family_id);
    assert_eq!(works[0].display_paths.len(), 2);
    assert_eq!(works[0].records, 2);
}

#[test]
fn a_merge_derives_no_version_order_and_lists_every_pair_once() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    let before_a = fixture
        .service()
        .usable_pairs_of_family(&a.family_id)
        .unwrap();
    let before_b = fixture
        .service()
        .usable_pairs_of_family(&b.family_id)
        .unwrap();
    merge(&fixture, &a.family_id, &b.family_id);
    let after = fixture
        .service()
        .usable_pairs_of_family(&a.family_id)
        .unwrap();
    assert_eq!(after.len(), before_a.len() + before_b.len());
    // No pair joins a version of one work to a version of the other.
    assert!(after.iter().all(|p| p.before.file_id == p.after.file_id));
    assert!(after.iter().all(|p| p.family_id == a.family_id));
    assert_eq!(
        fixture
            .service()
            .usable_pairs_of_family(&b.family_id)
            .unwrap(),
        after,
        "the old ID gives the merged work's pairs"
    );
    let all = fixture.service().all_usable_pairs().unwrap();
    assert_eq!(all.len(), after.len(), "each pair once");
}

#[test]
fn a_merged_in_files_pairs_rest_on_the_users_merge() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    let pairs_b = fixture
        .service()
        .usable_pairs_of_family(&b.family_id)
        .unwrap();
    assert!(pairs_b.iter().all(|p| p.evidence == PairEvidence::System));
    merge(&fixture, &a.family_id, &b.family_id);
    for pair in fixture
        .service()
        .usable_pairs_of_family(&a.family_id)
        .unwrap()
    {
        let expected = if pair.after.file_id == b.origin_file_id {
            PairEvidence::User
        } else {
            PairEvidence::System
        };
        assert_eq!(pair.evidence, expected, "{pair:?}");
    }
}

#[test]
fn the_merged_in_record_keeps_recording_and_new_rows_go_to_the_live_work() {
    let fixture = Fixture::new().without_auto_link();
    let (a, b) = two_works(&fixture);
    enable(&fixture, &b.chain_id).unwrap();
    fixture.advance(1_000);
    replace(&fixture, "Notes/2026/plan.md", "saved by an app\n");
    candidate_saved(&fixture);
    merge(&fixture, &a.family_id, &b.family_id);
    let record = fixture.service().memory_record(&b.chain_id).unwrap();
    assert_eq!(
        record.family_id, a.family_id,
        "the record is in the live work"
    );
    assert_eq!(record.state, RecordState::Active);
    assert_eq!(record.candidate_capture, crate::CandidateCapture::On);
    assert_eq!(record.pending.objects, 1, "its candidate still waits");
    // A later capture and the decision write into the live work.
    fixture.advance(1_000);
    replace(&fixture, "Notes/2026/plan.md", "saved again\n");
    candidate_saved(&fixture);
    let page = fixture.service().pending_candidates(&b.chain_id).unwrap();
    assert_eq!(page.family_id, a.family_id);
    let choices = page
        .items
        .iter()
        .map(|item| CandidateDecision {
            candidate_id: item.candidate_id.clone(),
            choice: CandidateChoice::Continue {
                parent_snapshot_id: item.continue_from.clone().unwrap().snapshot_id,
            },
        })
        .collect();
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: b.chain_id.clone(),
            page_token: page.page_token,
            items: choices,
            current_object: None,
        })
        .unwrap();
    let db = fixture.db();
    let edge_works: Vec<String> = db
        .prepare("SELECT DISTINCT family_id FROM memory_edges")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(edge_works, std::slice::from_ref(&a.family_id));
    let confirmed_late: String = db
        .query_row(
            "SELECT family_id FROM memory_members WHERE status = 'confirmed_user'
             ORDER BY created_at_ms DESC, rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        confirmed_late, a.family_id,
        "the later candidate was written in the live work"
    );
    let merged = fixture.service().memory_family(&a.family_id).unwrap();
    assert_eq!(merged.members.len(), 4);
}

#[test]
fn relocating_after_a_merge_writes_into_the_live_work() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    merge(&fixture, &a.family_id, &b.family_id);
    fs::rename(
        fixture.path("Notes/2026/plan.md"),
        fixture.path("Taxes/plan.md"),
    )
    .unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &b.chain_id,
            &fixture.root_id,
            &fixture.path("Taxes/plan.md"),
        )
        .unwrap();
    let outcome = fixture
        .service()
        .relocate_record_with(
            &crate::RelocationRequest {
                chain_id: shown.chain_id.clone(),
                root_id: shown.new_place.root_id.clone(),
                relative_path: shown.new_place.relative_path.clone(),
                page_token: shown.page_token.clone(),
            },
            &mut |root, parent, name| crate::test_fixture::quick_read(parent, name, root.id().dev),
        )
        .unwrap();
    assert_eq!(outcome.record.family_id, a.family_id);
    let work: String = fixture
        .db()
        .query_row(
            "SELECT family_id FROM memory_members WHERE file_id = ?1",
            [&outcome.snapshot.file_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(work, a.family_id);
}

#[test]
fn turning_memory_on_after_a_merge_starts_the_record_in_the_live_work() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    fixture.service().stop_memory_record(&b.chain_id).unwrap();
    merge(&fixture, &a.family_id, &b.family_id);
    // The stopped record goes on (same chain), in the merged work.
    let again = fixture
        .service()
        .start_memory_for_file(&b.origin_file_id)
        .unwrap();
    assert_eq!(again.chain_id, b.chain_id);
    assert_eq!(again.family_id, a.family_id);
}

#[test]
fn a_merge_over_an_earlier_rejection_lists_and_supersedes_it() {
    let fixture = Fixture::new().without_auto_link();
    let a = start_record(&fixture, "todo.txt");
    enable(&fixture, &a.chain_id).unwrap();
    // An app saves another object at the place; it is judged unrelated.
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "another file\n").unwrap();
    only(fixture.check_memory());
    let page = fixture.service().pending_candidates(&a.chain_id).unwrap();
    let other_file = page.items[0].file_id.clone();
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: a.chain_id.clone(),
            page_token: page.page_token,
            items: vec![CandidateDecision {
                candidate_id: page.items[0].candidate_id.clone(),
                choice: CandidateChoice::Independent {},
            }],
            current_object: None,
        })
        .unwrap();
    // That file later gets Memory of its own, a work of its own.
    let b = fixture
        .service()
        .start_memory_for_file(&other_file)
        .unwrap();
    assert_ne!(b.family_id, a.family_id);
    let shown = fixture
        .service()
        .merge_preview(&a.family_id, &b.family_id)
        .unwrap();
    assert_eq!(shown.overrides.len(), 1);
    assert_eq!(shown.overrides[0].file_id, other_file);
    assert_eq!(shown.overrides[0].rejected_in_family_id, a.family_id);
    let outcome = fixture.service().merge_works(&request(&shown)).unwrap();
    let (from, to): (String, String) = fixture
        .db()
        .query_row(
            "SELECT from_status, to_status FROM memory_work_changes WHERE work_decision_id = ?1",
            [&outcome.work_decision_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((from.as_str(), to.as_str()), ("rejected", "superseded"));
    let member = outcome
        .family
        .members
        .iter()
        .find(|m| m.file_id == other_file)
        .unwrap();
    assert_eq!(member.status, RelationStatus::ConfirmedSystem);
}

#[test]
fn merging_follows_merged_away_ids_and_refuses_one_work_twice() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    fs::write(fixture.path("Taxes/third.txt"), "a third work\n").unwrap();
    let c = start_record(&fixture, "Taxes/third.txt");
    merge(&fixture, &a.family_id, &b.family_id);
    assert_eq!(
        code(fixture.service().merge_preview(&a.family_id, &b.family_id)),
        ErrorCode::DecisionConflict
    );
    // Merging into the old ID merges into the live work.
    let shown = fixture
        .service()
        .merge_preview(&b.family_id, &c.family_id)
        .unwrap();
    assert_eq!(shown.family_id, a.family_id);
    let outcome = fixture.service().merge_works(&request(&shown)).unwrap();
    assert_eq!(outcome.family.merged_family_ids.len(), 2);
    let record = fixture.service().memory_record(&c.chain_id).unwrap();
    assert_eq!(record.family_id, a.family_id);
}

#[test]
fn a_stale_or_repeated_merge_page_changes_nothing() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    let shown = fixture
        .service()
        .merge_preview(&a.family_id, &b.family_id)
        .unwrap();
    // A record changed after the page was shown.
    fixture.service().stop_memory_record(&b.chain_id).unwrap();
    assert_eq!(
        code(fixture.service().merge_works(&request(&shown))),
        ErrorCode::PageOutdated
    );
    assert_eq!(fixture.count("memory_work_decisions"), 0);
    // Saving a file does not make the page old.
    let shown = fixture
        .service()
        .merge_preview(&a.family_id, &b.family_id)
        .unwrap();
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\nand bread\n").unwrap();
    fixture.check_memory();
    fixture.service().merge_works(&request(&shown)).unwrap();
    // The same page again.
    assert!(fixture.service().merge_works(&request(&shown)).is_err());
    assert_eq!(fixture.count("memory_work_decisions"), 1);
}

#[test]
fn unknown_or_invalid_work_ids_are_refused() {
    let fixture = Fixture::new();
    let (a, _) = two_works(&fixture);
    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        code(fixture.service().merge_preview(&a.family_id, &unknown)),
        ErrorCode::UnknownFamily
    );
    assert_eq!(
        code(fixture.service().merge_preview(&a.family_id, "../x")),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code(fixture.service().merge_works(&MergeRequest {
            family_id: a.family_id.clone(),
            other_family_id: unknown,
            page_token: "nope".to_string(),
        })),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code(
            fixture
                .service()
                .memory_family(&uuid::Uuid::new_v4().to_string())
        ),
        ErrorCode::UnknownFamily
    );
}

#[test]
fn turning_memory_on_for_a_same_content_copy_only_hints() {
    let fixture = Fixture::new();
    let a = start_record(&fixture, "todo.txt");
    fs::copy(
        fixture.path("todo.txt"),
        fixture.path("Taxes/todo copy.txt"),
    )
    .unwrap();
    let before = rows(&fixture);
    let b = start_record(&fixture, "Taxes/todo copy.txt");
    assert_ne!(b.family_id, a.family_id);
    assert_eq!(b.same_content.len(), 1);
    let hint = &b.same_content[0];
    assert_eq!(hint.family_id, a.family_id);
    assert_eq!(hint.own_version.file_id, b.origin_file_id);
    assert_eq!(hint.same_bytes_as.file_id, a.origin_file_id);
    // Only the new record's own rows were added: no relation between them.
    let added: Vec<_> = rows(&fixture)
        .into_iter()
        .filter(|row| !before.contains(row))
        .collect();
    assert!(
        added
            .iter()
            .all(|(_, _, work)| work.starts_with(&b.family_id)),
        "{added:?}"
    );
    assert!(
        fixture
            .service()
            .all_usable_pairs()
            .unwrap()
            .iter()
            .all(|p| p.before.file_id == p.after.file_id)
    );
    // The earlier record gets no hint about the later copy.
    let a_now = fixture.service().memory_record(&a.chain_id).unwrap();
    assert!(a_now.same_content.is_empty());
    // After a merge they are one work: no hint.
    merge(&fixture, &a.family_id, &b.family_id);
    let b_now = fixture.service().memory_record(&b.chain_id).unwrap();
    assert!(b_now.same_content.is_empty());
}

#[test]
fn no_hint_for_empty_content_or_a_file_already_in_a_work() {
    let fixture = Fixture::new().without_auto_link();
    fs::write(fixture.path("Taxes/empty-a.txt"), "").unwrap();
    fs::write(fixture.path("Taxes/empty-b.txt"), "").unwrap();
    start_record(&fixture, "Taxes/empty-a.txt");
    let empty = start_record(&fixture, "Taxes/empty-b.txt");
    assert!(
        empty.same_content.is_empty(),
        "every empty file has the same hash"
    );
    // Another work saved these bytes first.
    fs::write(fixture.path("Notes/elsewhere.txt"), "saved by an app\n").unwrap();
    start_record(&fixture, "Notes/elsewhere.txt");
    // A record captures the same bytes as a candidate, judged a branch: the
    // file is in that record's work already.
    let a = start_record(&fixture, "todo.txt");
    enable(&fixture, &a.chain_id).unwrap();
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved by an app\n");
    candidate_saved(&fixture);
    let page = fixture.service().pending_candidates(&a.chain_id).unwrap();
    let branch_file = page.items[0].file_id.clone();
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: a.chain_id.clone(),
            page_token: page.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: page.items[0].candidate_id.clone(),
                choice: CandidateChoice::Branch {
                    parent_snapshot_id: Some(
                        page.items[0].continue_from.clone().unwrap().snapshot_id,
                    ),
                },
            }],
            current_object: None,
        })
        .unwrap();
    fixture.service().stop_memory_record(&a.chain_id).unwrap();
    // Memory turned on for that file: a record of the same work, no hint.
    let branch = fixture
        .service()
        .start_memory_for_file(&branch_file)
        .unwrap();
    assert_eq!(branch.family_id, a.family_id);
    assert_ne!(branch.chain_id, a.chain_id);
    assert!(branch.same_content.is_empty(), "{:?}", branch.same_content);
}

#[test]
fn merge_json_names_are_fixed() {
    let fixture = Fixture::new();
    let (a, b) = two_works(&fixture);
    let keys = |value: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let shown = fixture
        .service()
        .merge_preview(&a.family_id, &b.family_id)
        .unwrap();
    let json = serde_json::to_value(&shown).unwrap();
    assert_eq!(
        keys(&json),
        [
            "family_id",
            "joins",
            "other_family_id",
            "overrides",
            "page_token",
            "stays"
        ]
    );
    let request: MergeRequest = serde_json::from_value(serde_json::json!({
        "family_id": shown.family_id, "other_family_id": shown.other_family_id,
        "page_token": shown.page_token
    }))
    .unwrap();
    assert!(
        serde_json::from_value::<MergeRequest>(serde_json::json!({
            "family_id": "x", "other_family_id": "y", "page_token": "z", "extra": 1
        }))
        .is_err()
    );
    let outcome = serde_json::to_value(fixture.service().merge_works(&request).unwrap()).unwrap();
    assert_eq!(
        keys(&outcome),
        [
            "family",
            "held_records",
            "kind",
            "other_family_id",
            "stopped_records",
            "work_decision_id"
        ]
    );
    assert_eq!(outcome["kind"], "merge");
    let works = serde_json::to_value(fixture.service().memory_works().unwrap()).unwrap();
    assert_eq!(
        keys(&works[0]),
        [
            "created_at_ms",
            "display_paths",
            "family_id",
            "merged_family_ids",
            "records"
        ]
    );
    fs::copy(fixture.path("todo.txt"), fixture.path("Taxes/copy.txt")).unwrap();
    let copy = start_record(&fixture, "Taxes/copy.txt");
    let hint = serde_json::to_value(&copy.same_content[0]).unwrap();
    assert_eq!(keys(&hint), ["family_id", "own_version", "same_bytes_as"]);
}
