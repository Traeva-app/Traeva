use std::fs;

use serde_json::json;

use super::*;
use crate::MemoryRecordView;
use crate::test_fixture::{Fixture, quick_read};

fn start_record(fixture: &Fixture, relative: &str) -> MemoryRecordView {
    let file = fixture.register(relative);
    fixture
        .service()
        .start_memory_for_file_with(&file.file_id, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        })
        .unwrap()
}

/// Saves `text` in place and lets the watcher capture it, one second later.
fn save(fixture: &Fixture, relative: &str, text: &str) {
    fixture.advance(1_000);
    fs::write(fixture.path(relative), text).unwrap();
    fixture.check_memory();
}

fn sequences(pairs: &[UsablePair]) -> Vec<(i64, i64)> {
    pairs
        .iter()
        .map(|pair| (pair.before.sequence, pair.after.sequence))
        .collect()
}

#[test]
fn saves_in_place_are_consecutive_pairs_of_one_file() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    save(&fixture, "todo.txt", "second\n");
    save(&fixture, "todo.txt", "third\n");
    let pairs = fixture
        .service()
        .usable_pairs_of_file(&record.origin_file_id)
        .unwrap();
    assert_eq!(sequences(&pairs), vec![(1, 2), (2, 3)]);
    for pair in &pairs {
        assert_eq!(pair.basis, PairBasis::SameFile);
        assert_eq!(pair.evidence, PairEvidence::System);
        assert!(!pair.gap, "{pair:?}");
        assert!(pair.gap_kinds.is_empty());
        assert!(!pair.unplaced);
        assert_eq!(pair.family_id, record.family_id);
        assert_eq!(pair.chain_id.as_deref(), Some(record.chain_id.as_str()));
        assert_eq!(
            pair.permission_file_ids,
            vec![record.origin_file_id.clone()]
        );
        assert_eq!(pair.edge_id, None);
    }
    assert_eq!(
        fixture
            .service()
            .usable_pairs_of_family(&record.family_id)
            .unwrap(),
        pairs
    );
    assert_eq!(fixture.service().all_usable_pairs().unwrap(), pairs);
}

#[test]
fn each_recorded_gap_marks_the_pair_that_starts_where_it_began() {
    let mut fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let file_id = record.origin_file_id.clone();
    // Memory off and on again: (1, 2) spans the time it was off.
    fixture.advance(1_000);
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "edited while off\n").unwrap();
    fixture.service().start_memory_for_file(&file_id).unwrap();
    // The file is away for a while: (2, 3) spans a failed capture.
    fixture.advance(1_000);
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    fixture.check_memory();
    fixture.advance(1_000);
    fs::rename(fixture.path("Taxes/todo.txt"), fixture.path("todo.txt")).unwrap();
    save(&fixture, "todo.txt", "back and edited\n");
    // Traeva quits and starts again: (3, 4) spans the time it was not running.
    fixture.advance(40_000);
    fixture.check_memory();
    fixture.restart();
    fixture.advance(60_000);
    fs::write(fixture.path("todo.txt"), "edited while closed\n").unwrap();
    fixture.check_memory();
    // An ordinary save: (4, 5) has no gap.
    save(&fixture, "todo.txt", "ordinary save\n");

    let pairs = fixture.service().usable_pairs_of_file(&file_id).unwrap();
    let kinds: Vec<((i64, i64), Vec<GapKind>)> = pairs
        .iter()
        .map(|pair| {
            (
                (pair.before.sequence, pair.after.sequence),
                pair.gap_kinds.clone(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ((1, 2), vec![GapKind::MemoryOff]),
            ((2, 3), vec![GapKind::CaptureFailed]),
            ((3, 4), vec![GapKind::WatcherStopped]),
            ((4, 5), vec![]),
        ]
    );
    assert!(pairs.iter().take(3).all(|pair| pair.gap));
    assert!(!pairs[3].gap);
}

#[test]
fn a_pair_is_checked_by_the_same_rules_that_list_it() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    save(&fixture, "todo.txt", "second\n");
    save(&fixture, "todo.txt", "third\n");
    let versions = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap();
    let id = |n: usize| versions[n].snapshot_id.as_str();
    let service = fixture.service();
    match service.check_usable_pair(id(0), id(1)).unwrap() {
        PairCheck::Usable { pair } => {
            assert_eq!((pair.before.sequence, pair.after.sequence), (1, 2))
        }
        other => panic!("expected usable, got {other:?}"),
    }
    assert_eq!(
        service.check_usable_pair(id(0), id(2)).unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::NotAdjacent
        },
        "a version is never skipped"
    );
    assert_eq!(
        service.check_usable_pair(id(1), id(0)).unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::NotAdjacent
        }
    );

    // A file Memory was never on for is in no work: its versions pair with
    // nothing, not even with a copy of the same bytes.
    let other = fixture.register("Notes/2026/plan.md");
    let copy = fixture.capture(&other.file_id).unwrap().snapshot;
    assert_eq!(
        service.check_usable_pair(id(2), &copy.snapshot_id).unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::NotAMember
        }
    );
    assert_eq!(
        service.check_usable_pair(&copy.snapshot_id, id(0)).unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::NotAMember
        }
    );
    assert!(
        service
            .usable_pairs_of_file(&other.file_id)
            .unwrap()
            .is_empty()
    );

    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        service.check_usable_pair(id(0), &unknown).unwrap_err().code,
        ErrorCode::UnknownSnapshot
    );
    assert_eq!(
        service.check_usable_pair("../x", id(0)).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        service.usable_pairs_of_file(&unknown).unwrap_err().code,
        ErrorCode::UnknownFile
    );
    assert_eq!(
        service.usable_pairs_of_family("bad").unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn a_copy_with_the_same_bytes_turned_on_is_a_work_of_its_own() {
    let fixture = Fixture::new();
    let first = start_record(&fixture, "todo.txt");
    fs::copy(
        fixture.path("todo.txt"),
        fixture.path("Taxes/todo copy.txt"),
    )
    .unwrap();
    let copy = start_record(&fixture, "Taxes/todo copy.txt");
    assert_ne!(copy.family_id, first.family_id);
    save(&fixture, "todo.txt", "edited original\n");
    save(&fixture, "Taxes/todo copy.txt", "edited copy\n");
    let pairs = fixture.service().all_usable_pairs().unwrap();
    assert_eq!(pairs.len(), 2);
    assert!(
        pairs
            .iter()
            .all(|pair| pair.before.file_id == pair.after.file_id)
    );
    let versions = |file_id: &str| fixture.service().list_snapshots(file_id).unwrap();
    assert_eq!(
        fixture
            .service()
            .check_usable_pair(
                &versions(&first.origin_file_id)[0].snapshot_id,
                &versions(&copy.origin_file_id)[1].snapshot_id
            )
            .unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::NoConfirmedRelation
        },
        "two works: never linked by bytes or name"
    );
}

#[test]
fn a_move_traeva_made_keeps_the_pairs_going_without_a_gap() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let file = fixture.register("todo.txt");
    let plan = fixture.plan(&file, "Taxes", &[]);
    fixture.apply(&plan.plan_id).unwrap();
    save(&fixture, "Taxes/todo.txt", "edited after the move\n");
    let pairs = fixture
        .service()
        .usable_pairs_of_file(&record.origin_file_id)
        .unwrap();
    assert_eq!(sequences(&pairs), vec![(1, 2)]);
    assert!(!pairs[0].gap, "{:?}", pairs[0].gap_kinds);
    assert_eq!(pairs[0].after.relative_path, "Taxes/todo.txt");
}

#[test]
fn pair_json_names_are_fixed() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    save(&fixture, "todo.txt", "second\n");
    let pair = fixture
        .service()
        .usable_pairs_of_file(&record.origin_file_id)
        .unwrap()
        .remove(0);
    let value = serde_json::to_value(&pair).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "after",
            "basis",
            "before",
            "chain_id",
            "edge_id",
            "evidence",
            "family_id",
            "gap",
            "gap_kinds",
            "permission_file_ids",
            "unplaced"
        ]
    );
    assert_eq!(value["basis"], json!("same_file"));
    assert_eq!(value["evidence"], json!("system"));
    assert_eq!(
        serde_json::to_value(PairCheck::NotUsable {
            reason: PairRefusal::CandidateVersion
        })
        .unwrap(),
        json!({"state": "not_usable", "reason": "candidate_version"})
    );
    let kinds = [
        GapKind::MemoryOff,
        GapKind::Paused,
        GapKind::CaptureFailed,
        GapKind::ReplacedUncaptured,
        GapKind::Lost,
        GapKind::WatcherStopped,
        GapKind::NotRecorded,
        GapKind::Skipped,
    ];
    assert_eq!(
        serde_json::to_value(kinds).unwrap(),
        json!([
            "memory_off",
            "paused",
            "capture_failed",
            "replaced_uncaptured",
            "lost",
            "watcher_stopped",
            "not_recorded",
            "skipped"
        ])
    );
    assert_eq!(
        serde_json::to_value([
            ErrorCode::UnknownRecord,
            ErrorCode::UnknownFamily,
            ErrorCode::PageOutdated,
            ErrorCode::DecisionConflict,
            ErrorCode::RecordStopped,
            ErrorCode::CandidatePending,
            ErrorCode::NotSameObject,
        ])
        .unwrap(),
        json!([
            "unknown_record",
            "unknown_family",
            "page_outdated",
            "decision_conflict",
            "record_stopped",
            "candidate_pending",
            "not_same_object"
        ])
    );
}

/// A record whose line went on as an object an app saved (v1, v2, v3, then
/// N1), with N2 judged a branch from v2 that has two versions of its own.
fn line_with_a_branch(fixture: &Fixture) -> (MemoryRecordView, String, String) {
    use crate::candidates::tests::{enable, only, replace};
    use crate::family_decide::{CandidateChoice, CandidateDecision, CandidateDecisionRequest};
    let record = start_record(fixture, "todo.txt");
    save(fixture, "todo.txt", "second\n");
    save(fixture, "todo.txt", "third\n");
    enable(fixture, &record.chain_id).unwrap();
    let decide = |choice: &dyn Fn(&crate::PendingCandidate) -> CandidateChoice| {
        let page = fixture
            .service()
            .pending_candidates(&record.chain_id)
            .unwrap();
        let item = page.items.last().unwrap();
        fixture
            .service()
            .decide_pending_candidates(&CandidateDecisionRequest {
                chain_id: record.chain_id.clone(),
                page_token: page.page_token.clone(),
                items: vec![CandidateDecision {
                    candidate_id: item.candidate_id.clone(),
                    choice: choice(item),
                }],
                current_object: None,
            })
            .unwrap();
        item.file_id.clone()
    };
    fixture.advance(1_000);
    replace(fixture, "todo.txt", "saved by an app\n");
    only(fixture.check_memory());
    let continued = decide(&|item| CandidateChoice::Continue {
        parent_snapshot_id: item.continue_from.clone().unwrap().snapshot_id,
    });
    // Saved in place once more, with the clock set back a minute: the
    // continued file has two versions, and the line's order must come from
    // the order they were saved, not from the clock.
    fixture.advance(-60_000);
    save(fixture, "todo.txt", "saved by an app, edited in place\n");
    fixture.advance(60_000);
    fixture.advance(1_000);
    replace(fixture, "todo.txt", "another start\n");
    only(fixture.check_memory());
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "another start, edited\n").unwrap();
    only(fixture.check_memory());
    let v2 = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap()[1]
        .snapshot_id
        .clone();
    let branch = decide(&|_| CandidateChoice::Branch {
        parent_snapshot_id: Some(v2.clone()),
    });
    (record, continued, branch)
}

#[test]
fn every_file_on_a_line_gives_the_same_line_in_saved_order() {
    let fixture = Fixture::new().without_auto_link();
    let (record, continued, _) = line_with_a_branch(&fixture);
    let from_old = fixture
        .service()
        .usable_pairs_of_line(&record.origin_file_id)
        .unwrap();
    let from_new = fixture.service().usable_pairs_of_line(&continued).unwrap();
    assert_eq!(from_old, from_new);
    let files: Vec<(&str, &str)> = from_old
        .iter()
        .map(|p| (p.before.file_id.as_str(), p.after.file_id.as_str()))
        .collect();
    let origin = record.origin_file_id.as_str();
    // In saved order: across to the continued file before its own next
    // version.
    assert_eq!(
        files,
        [
            (origin, origin),
            (origin, origin),
            (origin, continued.as_str()),
            (continued.as_str(), continued.as_str())
        ]
    );
    assert_eq!(sequences(&from_old), [(1, 2), (2, 3), (3, 1), (1, 2)]);
    assert_eq!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .line_versions,
        5
    );
}

#[test]
fn a_branch_and_its_own_versions_are_not_on_the_line() {
    let fixture = Fixture::new().without_auto_link();
    let (record, _, branch) = line_with_a_branch(&fixture);
    let line = fixture
        .service()
        .usable_pairs_of_line(&record.origin_file_id)
        .unwrap();
    assert!(
        line.iter()
            .all(|p| p.before.file_id != branch && p.after.file_id != branch),
        "{line:?}"
    );
    // The branch file's own list: its versions and the branch it starts.
    let own = fixture.service().usable_pairs_of_line(&branch).unwrap();
    let bases: Vec<PairBasis> = own.iter().map(|p| p.basis).collect();
    assert!(bases.contains(&PairBasis::Branch), "{own:?}");
    assert!(bases.contains(&PairBasis::SameFile), "{own:?}");
    // The whole work still has every pair.
    let all = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    assert_eq!(all.len(), line.len() + own.len());
}
