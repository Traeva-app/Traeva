use std::fs;

use super::*;
use crate::candidates::tests::{check_counting_reads, enable, only, replace, start_record};
use crate::capture::CheckOutcome;
use crate::family::MemoryRecordView;
use crate::memory_contracts::{PairBasis, PairEvidence};
use crate::test_fixture::{Fixture, quick_read};

fn page(fixture: &Fixture, chain_id: &str) -> PendingCandidatesView {
    fixture.service().pending_candidates(chain_id).unwrap()
}

fn decide(
    fixture: &Fixture,
    page: &PendingCandidatesView,
    items: Vec<(usize, CandidateChoice)>,
    current_object: Option<CurrentObjectChoice>,
) -> Result<CandidateDecisionOutcome, AppError> {
    fixture.service().decide_pending_candidates_with(
        &CandidateDecisionRequest {
            chain_id: page.chain_id.clone(),
            page_token: page.page_token.clone(),
            items: items
                .into_iter()
                .map(|(n, choice)| CandidateDecision {
                    candidate_id: page.items[n].candidate_id.clone(),
                    choice,
                })
                .collect(),
            current_object,
        },
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    )
}

/// "Continue" for item `n`, with the parent the page showed.
fn cont(page: &PendingCandidatesView, n: usize) -> (usize, CandidateChoice) {
    (
        n,
        CandidateChoice::Continue {
            parent_snapshot_id: page.items[n]
                .continue_from
                .as_ref()
                .unwrap()
                .snapshot_id
                .clone(),
        },
    )
}

/// A record with candidate capture on and `n` replacements saved as
/// candidates.
fn record_with_candidates(fixture: &Fixture, n: usize) -> MemoryRecordView {
    let record = start_record(fixture, "todo.txt");
    enable(fixture, &record.chain_id).unwrap();
    for k in 1..=n {
        fixture.advance(1_000);
        replace(fixture, "todo.txt", &format!("save {k}\n"));
        assert!(matches!(
            only(fixture.check_memory()),
            CheckOutcome::CandidateSaved { .. }
        ));
    }
    record
}

fn counts(fixture: &Fixture) -> Vec<i64> {
    [
        "memory_decisions",
        "memory_edges",
        "memory_watches",
        "memory_candidates",
        "memory_gaps",
        "files",
        "snapshots",
    ]
    .iter()
    .map(|table| fixture.count(table))
    .collect()
}

#[test]
fn confirming_the_list_links_the_shown_versions_and_moves_the_watch() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 3);
    let first = page(&fixture, &record.chain_id);
    assert_eq!(first.items.len(), 3);
    assert!(first.items[2].at_place);
    assert_eq!(
        first.items[1].continue_needs.as_deref(),
        Some(first.items[0].candidate_id.as_str())
    );
    let original_last = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first.items[0].continue_from.as_ref(), Some(&original_last));
    let outcome = decide(
        &fixture,
        &first,
        vec![cont(&first, 0), cont(&first, 1), cont(&first, 2)],
        None,
    )
    .unwrap();
    assert!(!outcome.history_only);
    let moved = outcome.continued_watch.unwrap();
    assert_eq!(moved.file_id, first.items[2].file_id);
    assert_eq!(outcome.record.watch.watch_id, moved.watch_id);
    assert_eq!(
        outcome.record.chain_id, record.chain_id,
        "the same record task"
    );
    assert!(outcome.page.items.is_empty());
    assert_eq!(
        fixture.count("memory_candidate_switches"),
        1,
        "the switch went along"
    );
    assert_eq!(
        outcome.record.candidate_capture,
        crate::CandidateCapture::On
    );
    // The new watch is not read again for what it already saved.
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Unchanged));
    assert_eq!(reads, 0);
    fs::write(
        fixture.path("todo.txt"),
        "saved in place after confirming\n",
    )
    .unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));

    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    let continuations: Vec<_> = pairs
        .iter()
        .filter(|p| p.basis == PairBasis::Continuation)
        .collect();
    assert_eq!(continuations.len(), 3);
    assert!(
        continuations
            .iter()
            .all(|p| p.evidence == PairEvidence::User
                && p.permission_file_ids == vec![record.origin_file_id.clone()]
                && p.chain_id.as_deref() == Some(record.chain_id.as_str()))
    );
    assert_eq!(continuations[0].before, original_last);
    let same_file: Vec<_> = pairs
        .iter()
        .filter(|p| p.basis == PairBasis::SameFile)
        .collect();
    assert!(
        same_file
            .iter()
            .any(|p| p.after.file_id == first.items[2].file_id && p.evidence == PairEvidence::User)
    );
}

#[test]
fn a_page_that_changed_or_was_used_is_refused_and_nothing_changes() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let old = page(&fixture, &record.chain_id);
    replace(&fixture, "todo.txt", "another save\n");
    fixture.check_memory();
    let before = counts(&fixture);
    let refused = decide(&fixture, &old, vec![cont(&old, 0)], None).unwrap_err();
    assert_eq!(refused.code, ErrorCode::PageOutdated);
    assert_eq!(counts(&fixture), before);
    let fresh = page(&fixture, &record.chain_id);
    decide(&fixture, &fresh, vec![(0, CandidateChoice::Defer {})], None).unwrap();
    let again = decide(&fixture, &fresh, vec![(0, CandidateChoice::Defer {})], None).unwrap_err();
    assert_eq!(again.code, ErrorCode::PageOutdated, "a page is used once");
    assert!(page(&fixture, &record.chain_id).items[0].deferred);
}

#[test]
fn bad_requests_are_refused() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let p = page(&fixture, &record.chain_id);
    let mut request = CandidateDecisionRequest {
        chain_id: record.chain_id.clone(),
        page_token: p.page_token.clone(),
        items: vec![],
        current_object: None,
    };
    let service = fixture.service();
    assert_eq!(
        service
            .decide_pending_candidates(&request)
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    request.items = vec![
        CandidateDecision {
            candidate_id: p.items[0].candidate_id.clone(),
            choice: CandidateChoice::Defer {},
        },
        CandidateDecision {
            candidate_id: p.items[0].candidate_id.clone(),
            choice: CandidateChoice::Unknown {},
        },
    ];
    assert_eq!(
        service
            .decide_pending_candidates(&request)
            .unwrap_err()
            .code,
        ErrorCode::DecisionConflict
    );
    request.items.truncate(1);
    request.items[0].candidate_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        service
            .decide_pending_candidates(&request)
            .unwrap_err()
            .code,
        ErrorCode::PageOutdated
    );
    request.page_token = "x".repeat(64);
    assert_eq!(
        service
            .decide_pending_candidates(&request)
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        service.pending_candidates(&unknown).unwrap_err().code,
        ErrorCode::UnknownRecord
    );
    assert!(
        serde_json::from_value::<CandidateDecisionRequest>(serde_json::json!({
            "chain_id": record.chain_id, "page_token": p.page_token,
            "items": [{"candidate_id": p.items[0].candidate_id, "choice": {"kind": "continue",
                       "parent_snapshot_id": "x", "file": "/etc"}}]
        }))
        .is_err()
    );
    // A choice without fields refuses extra fields too, and still reads as
    // the plain {"kind": …} the window sends.
    for choice in ["unknown", "independent", "defer"] {
        assert!(
            serde_json::from_value::<CandidateChoice>(serde_json::json!({
                "kind": choice, "file": "/etc"
            }))
            .is_err(),
            "{choice}"
        );
        assert!(
            serde_json::from_value::<CandidateChoice>(serde_json::json!({"kind": choice})).is_ok()
        );
    }
    assert!(
        serde_json::from_value::<CurrentObjectChoice>(serde_json::json!({
            "kind": "independent", "file": "/etc"
        }))
        .is_err()
    );
    assert_eq!(
        serde_json::from_value::<CurrentObjectChoice>(serde_json::json!({"kind": "independent"}))
            .unwrap(),
        CurrentObjectChoice::Independent {}
    );
}

#[test]
fn excluding_a_middle_object_never_links_across_it_without_a_shown_relation() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 3);
    let p = page(&fixture, &record.chain_id);
    let n1_last = p.items[0].snapshots.last().unwrap().snapshot_id.clone();
    let n2_last = p.items[1].snapshots.last().unwrap().snapshot_id.clone();
    let before = counts(&fixture);
    // N2 unrelated and N3 continuing N2 in one go: N2 is not on the line.
    let across_excluded = decide(
        &fixture,
        &p,
        vec![
            (0, CandidateChoice::Defer {}),
            (1, CandidateChoice::Independent {}),
            (
                2,
                CandidateChoice::Continue {
                    parent_snapshot_id: n2_last.clone(),
                },
            ),
        ],
        None,
    )
    .unwrap_err();
    assert_eq!(across_excluded.code, ErrorCode::DecisionConflict);
    // N3 from N1 in one go: not the relation the page showed.
    let skipping = decide(
        &fixture,
        &p,
        vec![
            (1, CandidateChoice::Independent {}),
            (
                2,
                CandidateChoice::Continue {
                    parent_snapshot_id: n1_last.clone(),
                },
            ),
        ],
        None,
    )
    .unwrap_err();
    assert_eq!(skipping.code, ErrorCode::DecisionConflict);
    assert_eq!(counts(&fixture), before, "nothing was written");
    // Decide N2 on its own; the page then shows N3 from N1, skipping one.
    decide(
        &fixture,
        &p,
        vec![(1, CandidateChoice::Independent {})],
        None,
    )
    .unwrap();
    let p = page(&fixture, &record.chain_id);
    assert_eq!(p.items.len(), 2);
    let n3 = &p.items[1];
    assert_eq!(n3.continue_from.as_ref().unwrap().snapshot_id, n1_last);
    assert_eq!(n3.continue_skips, 1);
    decide(&fixture, &p, vec![cont(&p, 0), cont(&p, 1)], None).unwrap();
    let skipped: i64 = fixture
        .db()
        .query_row(
            "SELECT skipped_items FROM memory_edges WHERE parent_snapshot_id = ?1",
            [&n1_last],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(skipped, 1);
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    let across = pairs
        .iter()
        .find(|p| p.before.snapshot_id == n1_last)
        .unwrap();
    assert!(across.gap && across.gap_kinds.contains(&GapKind::Skipped));
    assert!(
        pairs
            .iter()
            .all(|p| p.after.snapshot_id != n2_last && p.before.snapshot_id != n2_last)
    );
}

#[test]
fn an_object_of_unknown_order_is_never_a_parent_and_its_pairs_are_unplaced() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 2);
    let p = page(&fixture, &record.chain_id);
    decide(&fixture, &p, vec![(0, CandidateChoice::Unknown {})], None).unwrap();
    let p = page(&fixture, &record.chain_id);
    let original_last = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(p.items[0].continue_from.as_ref(), Some(&original_last));
    assert_eq!(p.items[0].continue_skips, 1);
    // The unknown object: part of the work, order unknown.
    let family = fixture.service().memory_family(&record.family_id).unwrap();
    let unknown = family
        .members
        .iter()
        .find(|m| {
            m.basis == crate::MemberBasis::CandidateCapture
                && m.status == crate::RelationStatus::ConfirmedUser
        })
        .unwrap();
    assert!(!unknown.placed);
    assert_eq!(family.boundaries.len(), 1);
}

#[test]
fn a_deferred_earlier_object_must_be_decided_first() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 2);
    let p = page(&fixture, &record.chain_id);
    let refused = decide(
        &fixture,
        &p,
        vec![(0, CandidateChoice::Defer {}), cont(&p, 1)],
        None,
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::DecisionConflict);
    // Deferring does not free the quota.
    decide(&fixture, &p, vec![(0, CandidateChoice::Defer {})], None).unwrap();
    assert_eq!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .pending
            .objects,
        2
    );
}

#[test]
fn branches_need_a_known_fork_point_and_need_both_permissions() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let p = page(&fixture, &record.chain_id);
    let own = p.items[0].snapshots[0].snapshot_id.clone();
    let refused = decide(
        &fixture,
        &p,
        vec![(
            0,
            CandidateChoice::Branch {
                parent_snapshot_id: Some(own),
            },
        )],
        None,
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::DecisionConflict);
    let fork = p.fork_points[0].snapshot_id.clone();
    decide(
        &fixture,
        &p,
        vec![(
            0,
            CandidateChoice::Branch {
                parent_snapshot_id: Some(fork.clone()),
            },
        )],
        None,
    )
    .unwrap();
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    let branch = pairs.iter().find(|p| p.basis == PairBasis::Branch).unwrap();
    assert_eq!(branch.before.snapshot_id, fork);
    assert_eq!(branch.permission_file_ids.len(), 2, "lines of both sides");
    assert!(branch.permission_file_ids.contains(&record.origin_file_id));
}

#[test]
fn judging_the_object_at_the_place_unrelated_leaves_the_record_lost() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 2);
    let p = page(&fixture, &record.chain_id);
    // An earlier object judged unrelated keeps capture on.
    decide(
        &fixture,
        &p,
        vec![(0, CandidateChoice::Independent {})],
        None,
    )
    .unwrap();
    assert_eq!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .state,
        crate::RecordState::Active
    );
    let p = page(&fixture, &record.chain_id);
    assert!(p.items[0].at_place);
    let outcome = decide(
        &fixture,
        &p,
        vec![(0, CandidateChoice::Independent {})],
        None,
    )
    .unwrap();
    assert_eq!(outcome.record.state, crate::RecordState::Lost);
    assert!(outcome.record.waiting_for_relocation);
    assert_eq!(
        outcome.record.candidate_capture,
        crate::CandidateCapture::Off
    );
    // The unrelated file can have a record of its own, in a work of its own;
    // the lost record stays as it is.
    let own = fixture
        .service()
        .start_memory_for_file(&p.items[0].file_id)
        .unwrap();
    assert_ne!(own.chain_id, record.chain_id);
    assert_ne!(own.family_id, record.family_id);
    assert_eq!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .state,
        crate::RecordState::Lost
    );
    fixture.service().stop_memory_record(&own.chain_id).unwrap();
    replace(&fixture, "todo.txt", "yet another\n");
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Problem { .. }));
    assert_eq!(reads, 0, "a lost record looks at nothing");
    assert_eq!(
        enable(&fixture, &record.chain_id).unwrap_err().code,
        ErrorCode::RecordStopped
    );
}

#[test]
fn with_capture_off_continuing_reads_the_object_at_the_place_once() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "saved by replacing\n");
    fixture.check_memory();
    let p = page(&fixture, &record.chain_id);
    let unread = p.unread_occupant.as_ref().unwrap();
    let parent = unread.continue_from.as_ref().unwrap().snapshot_id.clone();
    let mut reads = 0;
    let outcome = fixture
        .service()
        .decide_pending_candidates_with(
            &CandidateDecisionRequest {
                chain_id: record.chain_id.clone(),
                page_token: p.page_token.clone(),
                items: vec![],
                current_object: Some(CurrentObjectChoice::Continue {
                    parent_snapshot_id: parent.clone(),
                }),
            },
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap();
    assert_eq!(reads, 1);
    assert!(outcome.continued_watch.is_some());
    // The record went on by this decision: its link says so.
    let db = fixture.db();
    let reasons: Vec<String> = db
        .prepare("SELECT reason FROM memory_chain_watches WHERE chain_id = ?1 ORDER BY rowid")
        .unwrap()
        .query_map([&record.chain_id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(reasons, ["started", "continued"]);
    drop(db);
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Unchanged));
    assert_eq!(reads, 0);
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    let across = pairs
        .iter()
        .find(|p| p.before.snapshot_id == parent)
        .unwrap();
    assert_eq!(across.basis, PairBasis::Continuation);
    assert!(
        across.gap_kinds.contains(&GapKind::ReplacedUncaptured),
        "saves before it were not seen"
    );
}

#[test]
fn with_capture_off_an_object_other_than_shown_is_never_read() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "first\n");
    fixture.check_memory();
    let p = page(&fixture, &record.chain_id);
    replace(&fixture, "todo.txt", "replaced again since\n");
    let parent = p
        .unread_occupant
        .as_ref()
        .unwrap()
        .continue_from
        .as_ref()
        .unwrap()
        .snapshot_id
        .clone();
    let mut reads = 0;
    let refused = fixture
        .service()
        .decide_pending_candidates_with(
            &CandidateDecisionRequest {
                chain_id: record.chain_id.clone(),
                page_token: p.page_token.clone(),
                items: vec![],
                current_object: Some(CurrentObjectChoice::Continue {
                    parent_snapshot_id: parent,
                }),
            },
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::PageOutdated);
    assert_eq!(reads, 0);
    assert_eq!(fixture.count("memory_candidates"), 0);
}

#[test]
fn with_capture_off_judging_the_object_unrelated_reads_nothing() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "unrelated\n");
    fixture.check_memory();
    let p = page(&fixture, &record.chain_id);
    let mut reads = 0;
    let outcome = fixture
        .service()
        .decide_pending_candidates_with(
            &CandidateDecisionRequest {
                chain_id: record.chain_id.clone(),
                page_token: p.page_token,
                items: vec![],
                current_object: Some(CurrentObjectChoice::Independent {}),
            },
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap();
    assert_eq!(reads, 0);
    assert_eq!(outcome.record.state, crate::RecordState::Lost);
    let kinds: Vec<(GapKind, Option<GapEnd>)> = fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(&record.chain_id)
        .unwrap()
        .into_iter()
        .map(|g| (g.kind, g.end_reason))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (GapKind::ReplacedUncaptured, Some(GapEnd::Decided)),
            (GapKind::Lost, None)
        ]
    );
}

#[test]
fn a_late_decision_after_stopping_only_confirms_the_history() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let old = page(&fixture, &record.chain_id);
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    assert_eq!(
        decide(&fixture, &old, vec![cont(&old, 0)], None)
            .unwrap_err()
            .code,
        ErrorCode::PageOutdated
    );
    let watches = fixture.count("memory_watches");
    let p = page(&fixture, &record.chain_id);
    assert_eq!(p.state, crate::RecordState::Stopped);
    let outcome = decide(&fixture, &p, vec![cont(&p, 0)], None).unwrap();
    assert!(outcome.history_only);
    assert!(outcome.continued_watch.is_none());
    assert_eq!(
        fixture.count("memory_watches"),
        watches,
        "recording is not reopened"
    );
    // Turning Memory on for that object goes on with the same record.
    let again = fixture
        .service()
        .start_memory_for_file(&p.items[0].file_id)
        .unwrap();
    assert_eq!(again.chain_id, record.chain_id);
}

#[test]
fn a_restart_keeps_the_page_and_confirms_nothing_by_itself() {
    let mut fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 2);
    let before = page(&fixture, &record.chain_id);
    fixture.restart();
    let after = page(&fixture, &record.chain_id);
    assert_eq!(after.page_token, before.page_token);
    assert!(after.items.iter().all(|item| !item.deferred));
    assert_eq!(after.pending.objects, 2);
}

#[test]
fn the_quota_is_counted_per_record_not_per_group() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 3);
    let p = page(&fixture, &record.chain_id);
    // The first two go on; the third (at the place) stays waiting.
    decide(&fixture, &p, vec![cont(&p, 0), cont(&p, 1)], None).unwrap();
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.pending.objects, 1);
    replace(&fixture, "todo.txt", "after the decision\n");
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.chain_id, record.chain_id);
    assert_eq!(
        view.pending.objects, 2,
        "counted for the record, not per group"
    );
}

#[test]
fn decision_json_names_are_fixed() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let value = serde_json::to_value(page(&fixture, &record.chain_id)).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "candidate_capture",
            "chain_id",
            "family_id",
            "fork_points",
            "items",
            "page_token",
            "pending",
            "place",
            "place_holds",
            "place_problem",
            "root_active",
            "state",
            "tail",
            "unread_occupant"
        ]
    );
    let item: Vec<&str> = value["items"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        item,
        vec![
            "at_place",
            "candidate_id",
            "continue_from",
            "continue_needs",
            "continue_skips",
            "deferred",
            "display_path",
            "file_id",
            "found_after",
            "found_at_ms",
            "present",
            "reason",
            "seq",
            "snapshots"
        ]
    );
    assert_eq!(value["items"][0]["reason"], "replaced");
    let request: CandidateDecisionRequest = serde_json::from_value(serde_json::json!({
        "chain_id": record.chain_id, "page_token": value["page_token"],
        "items": [{"candidate_id": value["items"][0]["candidate_id"],
                   "choice": {"kind": "branch", "parent_snapshot_id": null}}],
        "current_object": {"kind": "independent"}
    }))
    .unwrap();
    assert_eq!(
        request.items[0].choice,
        CandidateChoice::Branch {
            parent_snapshot_id: None
        }
    );
}

/// A record whose file was saved twice in place (v1, v2) and then replaced
/// once with candidate capture on.
fn two_versions_then_a_candidate(fixture: &Fixture) -> (MemoryRecordView, Vec<SnapshotRef>) {
    let record = start_record(fixture, "todo.txt");
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "second version\n").unwrap();
    fixture.check_memory();
    enable(fixture, &record.chain_id).unwrap();
    replace(fixture, "todo.txt", "replaced\n");
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    let versions = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap();
    (record, versions)
}

#[test]
fn continue_accepts_only_the_parent_the_page_showed() {
    let fixture = Fixture::new().without_auto_link();
    let (record, versions) = two_versions_then_a_candidate(&fixture);
    let p = page(&fixture, &record.chain_id);
    assert_eq!(p.items[0].continue_from.as_ref(), Some(&versions[1]));
    let refused = decide(
        &fixture,
        &p,
        vec![(
            0,
            CandidateChoice::Continue {
                parent_snapshot_id: versions[0].snapshot_id.clone(),
            },
        )],
        None,
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::DecisionConflict);
    assert_eq!(fixture.count("memory_edges"), 0);
}

#[test]
fn a_branch_from_an_earlier_version_than_where_it_was_found_is_a_gap() {
    for (fork, skipped) in [(0, true), (1, false)] {
        let fixture = Fixture::new().without_auto_link();
        let (record, versions) = two_versions_then_a_candidate(&fixture);
        let p = page(&fixture, &record.chain_id);
        let parent = versions[fork].snapshot_id.clone();
        decide(
            &fixture,
            &p,
            vec![(
                0,
                CandidateChoice::Branch {
                    parent_snapshot_id: Some(parent.clone()),
                },
            )],
            None,
        )
        .unwrap();
        let pairs = fixture
            .service()
            .usable_pairs_of_family(&record.family_id)
            .unwrap();
        let branch = pairs
            .iter()
            .find(|pair| pair.basis == PairBasis::Branch)
            .unwrap();
        assert_eq!(branch.before.snapshot_id, parent);
        assert_eq!(
            branch.gap_kinds.contains(&GapKind::Skipped),
            skipped,
            "fork {fork}"
        );
    }
}

#[test]
fn a_relation_that_skips_objects_carries_the_gaps_before_them() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "N1\n");
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    // Capture off for a while: the next save is not read, a gap after N1.
    fixture
        .service()
        .set_candidate_capture(&crate::CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: false,
            notice_version: None,
            occupant_token: None,
            place_token: None,
        })
        .unwrap();
    replace(&fixture, "todo.txt", "N2\n");
    fixture.check_memory();
    enable(&fixture, &record.chain_id).unwrap();
    let p = page(&fixture, &record.chain_id);
    decide(
        &fixture,
        &p,
        vec![(0, CandidateChoice::Independent {})],
        None,
    )
    .unwrap();
    let p = page(&fixture, &record.chain_id);
    assert_eq!(p.items[0].continue_skips, 1);
    decide(&fixture, &p, vec![cont(&p, 0)], None).unwrap();
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    let across = pairs
        .iter()
        .find(|pair| pair.basis == PairBasis::Continuation)
        .unwrap();
    assert!(across.gap_kinds.contains(&GapKind::Skipped));
    assert!(
        across.gap_kinds.contains(&GapKind::ReplacedUncaptured),
        "{:?}",
        across.gap_kinds
    );
}

#[test]
fn a_branch_never_starts_from_a_later_version() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 2);
    let p = page(&fixture, &record.chain_id);
    let original_last = p.items[0]
        .continue_from
        .as_ref()
        .unwrap()
        .snapshot_id
        .clone();
    decide(
        &fixture,
        &p,
        vec![(
            1,
            CandidateChoice::Branch {
                parent_snapshot_id: Some(original_last),
            },
        )],
        None,
    )
    .unwrap();
    let p = page(&fixture, &record.chain_id);
    let later = fixture
        .service()
        .memory_family(&record.family_id)
        .unwrap()
        .relations[0]
        .child
        .snapshot_id
        .clone();
    assert!(p.fork_points.iter().any(|s| s.snapshot_id == later));
    let before = counts(&fixture);
    let refused = decide(
        &fixture,
        &p,
        vec![(
            0,
            CandidateChoice::Branch {
                parent_snapshot_id: Some(later),
            },
        )],
        None,
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::DecisionConflict);
    assert_eq!(counts(&fixture), before);
}

#[test]
fn a_decided_file_at_the_place_can_be_judged_unrelated_without_reading() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let p = page(&fixture, &record.chain_id);
    decide(
        &fixture,
        &p,
        vec![(
            0,
            CandidateChoice::Branch {
                parent_snapshot_id: None,
            },
        )],
        None,
    )
    .unwrap();
    let outcome = only(fixture.check_memory());
    assert!(
        matches!(&outcome, CheckOutcome::Problem { error } if error.code == ErrorCode::FileChanged)
    );
    // Turning Memory on for it is refused while this record looks there.
    let branch_file = p.items[0].file_id.clone();
    assert_eq!(
        fixture
            .service()
            .start_memory_for_file(&branch_file)
            .unwrap_err()
            .code,
        ErrorCode::CandidatePending
    );
    let p = page(&fixture, &record.chain_id);
    assert_eq!(p.place_holds, crate::PlaceHolds::DecidedCandidate);
    let outcome = decide(
        &fixture,
        &p,
        vec![],
        Some(CurrentObjectChoice::Independent {}),
    )
    .unwrap();
    assert_eq!(outcome.record.state, crate::RecordState::Lost);
    let own = fixture
        .service()
        .start_memory_for_file(&branch_file)
        .unwrap();
    assert_ne!(own.chain_id, record.chain_id);
    assert_eq!(own.family_id, record.family_id, "a branch of the same work");
}

#[test]
fn a_file_that_is_not_the_newest_on_the_line_cannot_be_turned_on_again() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    // Organize moves N1 away; a new object appears at the place: N2.
    let n1 = fixture.register("todo.txt");
    let plan = fixture.plan(&n1, "Taxes", &[]);
    fixture.apply(&plan.plan_id).unwrap();
    fs::write(fixture.path("todo.txt"), "N2\n").unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    let p = page(&fixture, &record.chain_id);
    decide(&fixture, &p, vec![cont(&p, 0), cont(&p, 1)], None).unwrap();
    let chains = fixture.count("memory_chains");
    assert_eq!(
        fixture
            .service()
            .start_memory_for_file(&n1.file_id)
            .unwrap_err()
            .code,
        ErrorCode::DecisionConflict
    );
    assert_eq!(fixture.count("memory_chains"), chains);
    let newest = fixture
        .service()
        .start_memory_for_file(&p.items[1].file_id)
        .unwrap();
    assert_eq!(newest.chain_id, record.chain_id);
}

#[test]
fn every_not_sure_yet_is_a_new_page() {
    let fixture = Fixture::new().without_auto_link();
    let record = record_with_candidates(&fixture, 1);
    let p = page(&fixture, &record.chain_id);
    decide(&fixture, &p, vec![(0, CandidateChoice::Defer {})], None).unwrap();
    let p = page(&fixture, &record.chain_id);
    decide(&fixture, &p, vec![(0, CandidateChoice::Defer {})], None).unwrap();
    assert_eq!(
        decide(&fixture, &p, vec![(0, CandidateChoice::Defer {})], None)
            .unwrap_err()
            .code,
        ErrorCode::PageOutdated,
        "a page is used once, even to say not sure again"
    );
}
