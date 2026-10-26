use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::candidates::tests::{enable, only, replace, start_record};
use crate::capture::CheckOutcome;
use crate::family::{MemberBasis, RecordState, RelationStatus};
use crate::family_decide::{
    CandidateChoice, CandidateDecision, CandidateDecisionRequest, CurrentObjectChoice,
    PendingCandidatesView,
};
use crate::memory_contracts::{PairBasis, PairEvidence};
use crate::test_fixture::{Fixture, quick_read};
use crate::test_snapshot as tree;

fn preview(
    fixture: &Fixture,
    chain_id: &str,
    relative: &str,
) -> Result<RelocationPreview, AppError> {
    fixture
        .service()
        .relocation_preview(chain_id, &fixture.root_id, &fixture.path(relative))
}

fn request(preview: &RelocationPreview) -> RelocationRequest {
    RelocationRequest {
        chain_id: preview.chain_id.clone(),
        root_id: preview.new_place.root_id.clone(),
        relative_path: preview.new_place.relative_path.clone(),
        page_token: preview.page_token.clone(),
    }
}

/// Confirms `request`, counting how many times a file was read.
fn confirm_counting(
    fixture: &Fixture,
    request: &RelocationRequest,
) -> (Result<RelocationOutcome, AppError>, usize) {
    let mut reads = 0;
    let outcome = fixture
        .service()
        .relocate_record_with(request, &mut |root, parent, name| {
            reads += 1;
            quick_read(parent, name, root.id().dev)
        });
    (outcome, reads)
}

fn confirm(fixture: &Fixture, preview: &RelocationPreview) -> RelocationOutcome {
    let (outcome, reads) = confirm_counting(fixture, &request(preview));
    assert_eq!(reads, 1, "the chosen file is read once");
    outcome.unwrap()
}

fn code(result: Result<impl std::fmt::Debug, AppError>) -> ErrorCode {
    result.expect_err("expected a refusal").code
}

/// (kind, problem, end reason) of the chain's gaps, oldest first.
fn gaps(fixture: &Fixture, chain_id: &str) -> Vec<(String, Option<String>, Option<String>)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT kind, problem, end_reason FROM memory_gaps WHERE chain_id = ?1
             ORDER BY rowid",
        )
        .unwrap();
    stmt.query_map([chain_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn links(fixture: &Fixture, chain_id: &str) -> Vec<String> {
    let db = fixture.db();
    let mut stmt = db
        .prepare("SELECT reason FROM memory_chain_watches WHERE chain_id = ?1 ORDER BY rowid")
        .unwrap();
    stmt.query_map([chain_id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// What a relocation writes, to check nothing was written.
fn written(fixture: &Fixture) -> (i64, i64, i64, i64, i64) {
    (
        fixture.count("files"),
        fixture.count("snapshots"),
        fixture.count("memory_decisions"),
        fixture.count("memory_chain_watches"),
        fixture.count("memory_members"),
    )
}

fn missing_after_a_pass(fixture: &Fixture) {
    let outcome = only(fixture.check_memory());
    assert!(
        matches!(&outcome, CheckOutcome::Problem { error } if error.code == ErrorCode::FileNotFound),
        "{outcome:?}"
    );
}

fn page(fixture: &Fixture, chain_id: &str) -> PendingCandidatesView {
    fixture.service().pending_candidates(chain_id).unwrap()
}

#[test]
fn a_file_moved_outside_traeva_goes_on_at_the_place_the_user_chooses() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let first = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap()
        .pop()
        .unwrap();
    fixture.advance(1_000);
    fs::rename(
        fixture.path("todo.txt"),
        fixture.path("Taxes/todo-2026.txt"),
    )
    .unwrap();
    missing_after_a_pass(&fixture);
    assert!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .waiting_for_relocation
    );

    fixture.advance(1_000);
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo-2026.txt").unwrap();
    assert_eq!(shown.old_place.relative_path, "todo.txt");
    assert_eq!(shown.old_place_holds, PlaceHolds::Empty);
    assert_eq!(shown.new_place.relative_path, "Taxes/todo-2026.txt");
    assert_eq!(shown.byte_length, "buy milk\n".len() as u64);
    assert_eq!(shown.waiting_candidates, 0);
    assert_eq!(shown.candidate_capture, CandidateCapture::Off);

    let outcome = confirm(&fixture, &shown);
    let view = &outcome.record;
    assert_eq!(view.state, RecordState::Active);
    assert_eq!(view.place.relative_path, "Taxes/todo-2026.txt");
    assert!(!view.waiting_for_relocation);
    assert!(view.open_gaps.is_empty(), "{:?}", view.open_gaps);
    assert_eq!(view.chain_id, record.chain_id, "the same record task");
    assert_eq!(links(&fixture, &record.chain_id), ["started", "relocated"]);
    // The gap the watcher recorded ends here; no second one is written.
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        [(
            "capture_failed".to_string(),
            Some("file_not_found".to_string()),
            Some("relocated".to_string())
        )]
    );
    // A new file record (Organize's rules), the same bytes.
    let moved = &outcome.snapshot;
    assert_ne!(moved.file_id, record.origin_file_id);
    assert_eq!(moved.sha256, first.sha256);
    assert_eq!(view.watch.file_id, moved.file_id);

    let family = fixture.service().memory_family(&record.family_id).unwrap();
    let member = family
        .members
        .iter()
        .find(|m| m.file_id == moved.file_id)
        .unwrap();
    assert_eq!(member.status, RelationStatus::ConfirmedSystem);
    assert_eq!(member.basis, MemberBasis::Relocation);
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    let pair = &pairs[0];
    assert_eq!(pair.before.snapshot_id, first.snapshot_id);
    assert_eq!(pair.after.snapshot_id, moved.snapshot_id);
    assert_eq!(pair.basis, PairBasis::Continuation);
    assert_eq!(pair.evidence, PairEvidence::System);
    assert!(pair.gap_kinds.contains(&GapKind::CaptureFailed), "{pair:?}");
    assert_eq!(
        pair.permission_file_ids,
        std::slice::from_ref(&record.origin_file_id)
    );

    // Recording goes on at the new place.
    fixture.advance(1_000);
    fs::write(fixture.path("Taxes/todo-2026.txt"), "buy milk\nand eggs\n").unwrap();
    let outcome = only(fixture.check_memory());
    assert!(
        matches!(&outcome, CheckOutcome::Saved { snapshot } if snapshot.file_id == moved.file_id),
        "{outcome:?}"
    );
}

#[test]
fn a_pair_across_the_move_shows_a_gap_even_before_the_watcher_looked() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    fs::write(fixture.path("Taxes/todo.txt"), "edited after the move\n").unwrap();
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    confirm(&fixture, &shown);
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        [(
            "capture_failed".to_string(),
            Some("file_not_found".to_string()),
            Some("relocated".to_string())
        )]
    );
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert!(pairs[0].gap_kinds.contains(&GapKind::CaptureFailed));
}

#[test]
fn a_copy_or_a_file_with_other_links_is_not_the_records_file() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    // A copy, the original removed (as a move to another volume does).
    fs::copy(fixture.path("todo.txt"), fixture.path("Taxes/copy.txt")).unwrap();
    // Two more names for the original object, then its old name removed.
    fs::hard_link(fixture.path("todo.txt"), fixture.path("Taxes/a.txt")).unwrap();
    fs::hard_link(fixture.path("todo.txt"), fixture.path("Taxes/b.txt")).unwrap();
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    missing_after_a_pass(&fixture);
    let before = written(&fixture);
    // A copy, a name with other links, another file, a symbolic link, a
    // folder.
    for name in [
        "Taxes/copy.txt",
        "Taxes/a.txt",
        "Notes/2026/plan.md",
        "plan-link.md",
        "Notes",
    ] {
        assert_eq!(
            code(preview(&fixture, &record.chain_id, name)),
            ErrorCode::NotSameObject,
            "{name}"
        );
    }
    // A confirmation that names the copy reads nothing.
    let (outcome, reads) = confirm_counting(
        &fixture,
        &RelocationRequest {
            chain_id: record.chain_id.clone(),
            root_id: fixture.root_id.clone(),
            relative_path: "Taxes/copy.txt".to_string(),
            page_token: "0".repeat(64),
        },
    );
    assert_eq!(code(outcome), ErrorCode::NotSameObject);
    assert_eq!(reads, 0);
    assert_eq!(written(&fixture), before, "nothing was written");
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert!(view.waiting_for_relocation, "the record keeps waiting");
}

#[test]
fn the_object_now_at_the_old_place_never_joins_the_work() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    fs::rename(fixture.path("todo.txt"), fixture.path("Notes/todo.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "a different file\n").unwrap();
    fixture.check_memory();
    let shown = preview(&fixture, &record.chain_id, "Notes/todo.txt").unwrap();
    assert_eq!(shown.old_place_holds, PlaceHolds::UnreadObject);
    let outcome = confirm(&fixture, &shown);
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        [(
            "replaced_uncaptured".to_string(),
            None,
            Some("relocated".to_string())
        )]
    );
    let family = fixture.service().memory_family(&record.family_id).unwrap();
    let mut files: Vec<_> = family.members.iter().map(|m| m.file_id.clone()).collect();
    files.sort();
    let mut expected = vec![
        record.origin_file_id.clone(),
        outcome.snapshot.file_id.clone(),
    ];
    expected.sort();
    assert_eq!(files, expected, "only the record's own files");
    // Only the new place is watched: the old one is not looked at or read.
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "changed again\n").unwrap();
    let checks = fixture.check_memory();
    assert_eq!(checks.len(), 1);
    assert!(matches!(checks[0].outcome, CheckOutcome::Unchanged));
    assert_eq!(checks[0].file_id, outcome.snapshot.file_id);
}

#[test]
fn candidates_at_the_old_place_stay_and_capture_moves_to_the_new_place() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fixture.advance(1_000);
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "another file\n").unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    assert_eq!(shown.old_place_holds, PlaceHolds::PendingCandidate);
    assert_eq!(shown.waiting_candidates, 1);
    assert_eq!(shown.candidate_capture, CandidateCapture::On);
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.candidate_capture, CandidateCapture::On);
    assert_eq!(outcome.record.pending.objects, 1, "the candidate stays");

    // The candidate is no longer at the place, and the version it was found
    // after already goes on at the new place: "continue" is not offered.
    let shown = page(&fixture, &record.chain_id);
    assert_eq!(shown.items.len(), 1);
    assert!(!shown.items[0].at_place);
    assert!(shown.items[0].continue_from.is_none());
    assert!(shown.unread_occupant.is_none());
    let parent = shown.items[0].found_after.clone();
    let refused = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: shown.items[0].candidate_id.clone(),
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: parent.as_ref().unwrap().snapshot_id.clone(),
                },
            }],
            current_object: None,
        });
    assert_eq!(code(refused), ErrorCode::DecisionConflict);
    // Judging it unrelated does not lose the record: it is not at the place.
    let decided = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: shown.items[0].candidate_id.clone(),
                choice: CandidateChoice::Independent {},
            }],
            current_object: None,
        })
        .unwrap();
    assert_eq!(decided.record.state, RecordState::Active);

    // Capture now looks at the new place only.
    fixture.advance(1_000);
    replace(&fixture, "Taxes/todo.txt", "saved by an app\n");
    fs::write(fixture.path("todo.txt"), "the old place changes\n").unwrap();
    let outcome = only(fixture.check_memory());
    assert!(
        matches!(outcome, CheckOutcome::CandidateSaved { .. }),
        "{outcome:?}"
    );
    let shown = page(&fixture, &record.chain_id);
    assert_eq!(shown.items.len(), 1);
    assert!(shown.items[0].at_place);
}

#[test]
fn a_lost_record_goes_on_at_its_new_place() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/kept.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "unrelated\n").unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::CandidateSaved { .. }
    ));
    let shown = page(&fixture, &record.chain_id);
    let lost = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token,
            items: vec![CandidateDecision {
                candidate_id: shown.items[0].candidate_id.clone(),
                choice: CandidateChoice::Independent {},
            }],
            current_object: None,
        })
        .unwrap();
    assert_eq!(lost.record.state, RecordState::Lost);
    assert_eq!(lost.record.candidate_capture, CandidateCapture::Off);
    let shown = preview(&fixture, &record.chain_id, "Taxes/kept.txt").unwrap();
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.state, RecordState::Active);
    assert_eq!(
        outcome.record.candidate_capture,
        CandidateCapture::Off,
        "the switch the decision closed stays off"
    );
}

#[test]
fn a_lost_record_judged_without_reading_goes_on_at_its_new_place() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/kept.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "unrelated\n").unwrap();
    fixture.check_memory();
    let shown = page(&fixture, &record.chain_id);
    let lost = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token,
            items: Vec::new(),
            current_object: Some(CurrentObjectChoice::Independent {}),
        })
        .unwrap();
    assert_eq!(lost.record.state, RecordState::Lost);
    let shown = preview(&fixture, &record.chain_id, "Taxes/kept.txt").unwrap();
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.state, RecordState::Active);
    assert!(
        gaps(&fixture, &record.chain_id)
            .iter()
            .any(|(kind, _, end)| kind == "lost" && end.as_deref() == Some("relocated"))
    );
}

#[test]
fn a_stopped_record_or_a_file_still_at_its_place_is_not_relocated() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    assert_eq!(
        code(preview(&fixture, &record.chain_id, "todo.txt")),
        ErrorCode::DecisionConflict
    );
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    assert_eq!(
        code(preview(&fixture, &record.chain_id, "Taxes/todo.txt")),
        ErrorCode::RecordStopped
    );
    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        code(preview(&fixture, &unknown, "Taxes/todo.txt")),
        ErrorCode::UnknownRecord
    );
    assert_eq!(
        code(preview(&fixture, "not-an-id", "Taxes/todo.txt")),
        ErrorCode::InvalidInput
    );
}

#[test]
fn a_change_after_the_preview_changes_nothing() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    missing_after_a_pass(&fixture);
    let before = written(&fixture);

    // The file was edited after the page showed it.
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    fs::write(fixture.path("Taxes/todo.txt"), "buy milk\nedited\n").unwrap();
    let (outcome, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(outcome), ErrorCode::PageOutdated);
    assert_eq!(reads, 0);

    // Another object appeared at the old place.
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    fs::write(fixture.path("todo.txt"), "something new\n").unwrap();
    let (outcome, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(outcome), ErrorCode::PageOutdated);
    assert_eq!(reads, 0);

    // The confirmation names another place than the page.
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    for path in [
        "Taxes/../Taxes/todo.txt",
        "/etc/hosts",
        "Taxes//todo.txt",
        "./todo.txt",
    ] {
        let mut other = request(&shown);
        other.relative_path = path.to_string();
        let (outcome, reads) = confirm_counting(&fixture, &other);
        assert_eq!(code(outcome), ErrorCode::InvalidInput, "{path}");
        assert_eq!(reads, 0);
    }

    assert_eq!(written(&fixture), before, "nothing was written");
    assert_eq!(links(&fixture, &record.chain_id), ["started"]);
}

#[test]
fn a_file_that_already_has_its_own_record_is_not_a_new_place() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    missing_after_a_pass(&fixture);
    // Memory turned on for the file at its new place, as a record of its own.
    let own = start_record(&fixture, "Taxes/todo.txt");
    assert_ne!(own.chain_id, record.chain_id);
    let before = written(&fixture);
    assert_eq!(
        code(preview(&fixture, &record.chain_id, "Taxes/todo.txt")),
        ErrorCode::DecisionConflict
    );
    assert_eq!(written(&fixture), before);
}

#[test]
fn a_record_stopped_while_the_relocation_waits_for_the_lock_reads_nothing() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    let reads = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let held = fixture.service().lock_execution();
        let relocation = scope.spawn(|| {
            fixture
                .service()
                .relocate_record_with(&request(&shown), &mut |root, parent, name| {
                    reads.fetch_add(1, Ordering::SeqCst);
                    quick_read(parent, name, root.id().dev)
                })
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        // Stopped meanwhile (as stop_memory_record would, once it has the lock).
        fixture
            .db()
            .execute(
                "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1",
                [],
            )
            .unwrap();
        drop(held);
        assert_eq!(code(relocation.join().unwrap()), ErrorCode::RecordStopped);
    });
    assert_eq!(reads.load(Ordering::SeqCst), 0, "nothing was read");
    assert_eq!(fixture.count("memory_decisions"), 0);
}

#[test]
fn an_unsettled_action_in_the_folder_stops_the_relocation_before_reading() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let plan_file = fixture.register("Notes/2026/plan.md");
    let plan = fixture.plan(&plan_file, "Taxes", &[]);
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    fixture.insert_action(&plan.plan_id, "running");
    let (outcome, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(outcome), ErrorCode::UnresolvedAction);
    assert_eq!(reads, 0);
}

#[test]
fn a_relocation_is_kept_after_a_restart_and_writes_nothing_in_the_folder() {
    let mut fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    missing_after_a_pass(&fixture);
    let before = tree::snapshot(fixture.root.path());
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    let outcome = confirm(&fixture, &shown);
    assert_eq!(tree::snapshot(fixture.root.path()), before);
    fixture.restart();
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.state, RecordState::Active);
    assert_eq!(view.place.relative_path, "Taxes/todo.txt");
    assert_eq!(view.watch.file_id, outcome.snapshot.file_id);
    assert_eq!(links(&fixture, &record.chain_id), ["started", "relocated"]);
    // The old file record is not watched again by turning Memory on for it.
    assert!(
        fixture
            .service()
            .start_memory_for_file(&record.origin_file_id)
            .is_err()
    );
    assert_eq!(links(&fixture, &record.chain_id), ["started", "relocated"]);
}

#[test]
fn going_back_to_a_place_the_record_used_before_works() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let first = confirm(
        &fixture,
        &preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap(),
    );
    let db = fixture.db();
    let presence = |file_id: &str| -> String {
        db.query_row(
            "SELECT presence FROM files WHERE file_id = ?1",
            [file_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        presence(&record.origin_file_id),
        "missing",
        "its object was found elsewhere with one name"
    );
    // Moved back (Finder's Undo, say): a third file record, same object.
    fixture.advance(1_000);
    fs::rename(fixture.path("Taxes/todo.txt"), fixture.path("todo.txt")).unwrap();
    missing_after_a_pass(&fixture);
    let back = confirm(
        &fixture,
        &preview(&fixture, &record.chain_id, "todo.txt").unwrap(),
    );
    assert_ne!(back.snapshot.file_id, record.origin_file_id);
    assert_ne!(back.snapshot.file_id, first.snapshot.file_id);
    assert_eq!(back.record.place.relative_path, "todo.txt");
    assert_eq!(presence(&first.snapshot.file_id), "missing");
    assert_eq!(
        links(&fixture, &record.chain_id),
        ["started", "relocated", "relocated"]
    );
    let line = fixture
        .service()
        .usable_pairs_of_line(&back.snapshot.file_id)
        .unwrap();
    assert_eq!(line.len(), 2);
    assert_eq!(line[0].after.snapshot_id, first.snapshot.snapshot_id);
    assert_eq!(line[1].before.snapshot_id, first.snapshot.snapshot_id);
    assert_eq!(line[1].after.snapshot_id, back.snapshot.snapshot_id);
    assert_eq!(back.record.line_versions, 3);
}

#[test]
fn a_record_that_went_on_as_a_later_object_is_not_relocated() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    for text in ["first app save\n", "second app save\n"] {
        fixture.advance(1_000);
        replace(&fixture, "todo.txt", text);
        assert!(matches!(
            only(fixture.check_memory()),
            CheckOutcome::CandidateSaved { .. }
        ));
    }
    // Only the earlier object is confirmed to continue the work: the watch
    // stays, as it is not at the place.
    let shown = page(&fixture, &record.chain_id);
    let parent = shown.items[0].continue_from.clone().unwrap();
    let decided = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: shown.items[0].candidate_id.clone(),
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: parent.snapshot_id,
                },
            }],
            current_object: None,
        })
        .unwrap();
    assert!(decided.history_only);
    let before = written(&fixture);
    assert_eq!(
        code(preview(&fixture, &record.chain_id, "Taxes/todo.txt")),
        ErrorCode::DecisionConflict
    );
    assert_eq!(written(&fixture), before);
}

#[test]
fn no_candidate_from_the_old_place_is_offered_a_continue() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    for text in ["first app save\n", "second app save\n"] {
        fixture.advance(1_000);
        replace(&fixture, "todo.txt", text);
        only(fixture.check_memory());
    }
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    assert_eq!(shown.waiting_candidates, 2);
    let outcome = confirm(&fixture, &shown);
    // The pair across the move shows the record's file was not at its place.
    assert!(gaps(&fixture, &record.chain_id).contains(&(
        "capture_failed".to_string(),
        Some("file_not_found".to_string()),
        Some("relocated".to_string())
    )));
    let shown = page(&fixture, &record.chain_id);
    assert_eq!(shown.items.len(), 2);
    for item in &shown.items {
        assert!(item.continue_from.is_none(), "{item:?}");
        assert!(item.continue_needs.is_none(), "{item:?}");
    }
    // A save at the new place is a candidate found after the new version.
    fixture.advance(1_000);
    replace(&fixture, "Taxes/todo.txt", "saved at the new place\n");
    only(fixture.check_memory());
    let shown = page(&fixture, &record.chain_id);
    let newest = shown.items.last().unwrap();
    assert_eq!(newest.found_after.as_ref(), Some(&outcome.snapshot));
    assert_eq!(newest.continue_from.as_ref(), Some(&outcome.snapshot));
}

#[test]
fn a_continued_object_moved_away_goes_on_and_the_old_place_offers_no_continue() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved by an app\n");
    only(fixture.check_memory());
    let shown = page(&fixture, &record.chain_id);
    let parent = shown.items[0].continue_from.clone().unwrap();
    let decided = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: shown.items[0].candidate_id.clone(),
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: parent.snapshot_id,
                },
            }],
            current_object: None,
        })
        .unwrap();
    assert!(decided.continued_watch.is_some());
    // The continued object is moved away and another appears at the place.
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "another file\n").unwrap();
    only(fixture.check_memory());
    confirm(
        &fixture,
        &preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap(),
    );
    let shown = page(&fixture, &record.chain_id);
    assert_eq!(shown.items.len(), 1);
    assert!(shown.items[0].continue_from.is_none());
}

#[test]
fn a_lost_record_whose_file_is_back_resumes_at_its_place() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("todo-old.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "unrelated\n").unwrap();
    fixture.check_memory();
    let shown = page(&fixture, &record.chain_id);
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token,
            items: Vec::new(),
            current_object: Some(CurrentObjectChoice::Independent {}),
        })
        .unwrap();
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fs::rename(fixture.path("todo-old.txt"), fixture.path("todo.txt")).unwrap();
    let shown = preview(&fixture, &record.chain_id, "todo.txt").unwrap();
    assert_eq!(shown.old_place_holds, PlaceHolds::RecordedFile);
    assert_eq!(shown.new_place.relative_path, "todo.txt");
    let watches = fixture.count("memory_watches");
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.state, RecordState::Active);
    assert_eq!(outcome.snapshot.file_id, record.origin_file_id);
    assert_eq!(
        fixture.count("memory_watches"),
        watches,
        "the watch never stopped"
    );
    assert_eq!(links(&fixture, &record.chain_id), ["started"]);
    assert!(
        gaps(&fixture, &record.chain_id)
            .iter()
            .any(|(kind, _, end)| kind == "lost" && end.as_deref() == Some("relocated"))
    );
    // Recording goes on in place.
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nand bread\n").unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
}

#[test]
fn a_file_moved_into_another_folder_traeva_may_use_goes_on_there() {
    let fixture = Fixture::new();
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let other_root = fixture.service().authorize_root(other.path()).unwrap();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), other.path().join("todo.txt")).unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &record.chain_id,
            &other_root.root_id,
            &other.path().join("todo.txt"),
        )
        .unwrap();
    assert_eq!(shown.new_place.root_id, other_root.root_id);
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.place.root_id, other_root.root_id);
    assert_eq!(outcome.snapshot.file_id, outcome.record.watch.file_id);
    // An action that is not settled in the new folder stops a relocation
    // there before anything is read.
    fs::rename(
        other.path().join("todo.txt"),
        other.path().join("moved.txt"),
    )
    .unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &record.chain_id,
            &other_root.root_id,
            &other.path().join("moved.txt"),
        )
        .unwrap();
    // A file of the new folder being organized inside that folder only.
    fs::create_dir(other.path().join("Sub")).unwrap();
    fs::write(other.path().join("note.txt"), "a note\n").unwrap();
    let note = fixture
        .service()
        .register_file_with(
            &other_root.root_id,
            &other.path().join("note.txt"),
            &mut |_root, parent, name| quick_read(parent, name, parent.id().dev),
        )
        .unwrap()
        .file;
    let plan = plan_in(&fixture, &note, &other_root.root_id, "Sub");
    fixture.insert_action(&plan, "running");
    let (refused, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(refused), ErrorCode::UnresolvedAction);
    assert_eq!(reads, 0);
}

#[test]
fn a_folder_removed_after_the_preview_stops_the_relocation() {
    let fixture = Fixture::new();
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let other_root = fixture.service().authorize_root(other.path()).unwrap();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), other.path().join("todo.txt")).unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &record.chain_id,
            &other_root.root_id,
            &other.path().join("todo.txt"),
        )
        .unwrap();
    let before = written(&fixture);
    fixture.service().revoke_root(&other_root.root_id).unwrap();
    let (refused, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(refused), ErrorCode::RootRevoked);
    assert_eq!(reads, 0);
    assert_eq!(written(&fixture), before);
}

#[test]
fn a_read_that_fails_or_finds_another_object_changes_nothing() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    let before = written(&fixture);
    let changing = fixture
        .service()
        .relocate_record_with(&request(&shown), &mut |_, _, _| {
            Err(observe::ReadError::StillChanging { attempts: 3 })
        });
    assert_eq!(code(changing), ErrorCode::FileChanging);
    // The bytes read are another object's (replaced during the read).
    fs::write(fixture.path("Taxes/other.txt"), "another file\n").unwrap();
    let replaced = fixture
        .service()
        .relocate_record_with(&request(&shown), &mut |root, parent, _| {
            quick_read(parent, "other.txt", root.id().dev)
        });
    assert_eq!(code(replaced), ErrorCode::PageOutdated);
    // Not even its bytes are kept in Traeva's copies.
    let sha = observe::sha256_hex(b"another file\n");
    let copy = fixture
        .app
        .path()
        .join("Traeva/snapshots")
        .join(&sha[..2])
        .join(&sha);
    assert!(!copy.exists(), "another object's bytes were stored");
    assert_eq!(written(&fixture), before);
}

#[test]
fn confirming_twice_or_after_a_quota_pause_keeps_the_rules() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    for n in 1..=21 {
        replace(&fixture, "todo.txt", &format!("save {n}\n"));
        only(fixture.check_memory());
    }
    assert_eq!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .candidate_capture,
        CandidateCapture::Paused
    );
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.candidate_capture, CandidateCapture::Paused);
    assert!(
        outcome
            .record
            .open_gaps
            .iter()
            .any(|gap| gap.kind == GapKind::Paused),
        "the quota pause goes along"
    );
    // The same confirmation again: the file is at the record's place now.
    let (again, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(again), ErrorCode::DecisionConflict);
    assert_eq!(reads, 0);
}

#[test]
fn a_record_organize_made_for_the_file_at_its_new_place_is_used() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let registered = fixture.register("Taxes/todo.txt");
    let outcome = confirm(
        &fixture,
        &preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap(),
    );
    assert_eq!(outcome.snapshot.file_id, registered.file_id);
}

#[test]
fn relocation_json_names_are_fixed() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    let json = serde_json::to_value(&shown).unwrap();
    let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "byte_length",
            "candidate_capture",
            "chain_id",
            "modified_at_ms",
            "new_place",
            "old_place",
            "old_place_holds",
            "page_token",
            "waiting_candidates"
        ]
    );
    assert_eq!(json["old_place_holds"], "empty");
    let request: RelocationRequest = serde_json::from_value(serde_json::json!({
        "chain_id": shown.chain_id, "root_id": shown.new_place.root_id,
        "relative_path": shown.new_place.relative_path, "page_token": shown.page_token
    }))
    .unwrap();
    let outcome =
        serde_json::to_value(confirm(&fixture, &preview_again(&fixture, &request))).unwrap();
    let mut keys: Vec<_> = outcome.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["record", "snapshot"]);
    assert!(outcome["record"]["line_versions"].is_i64());
    assert_eq!(
        serde_json::to_value(ErrorCode::NotSameObject).unwrap(),
        "not_same_object"
    );
}

fn preview_again(fixture: &Fixture, request: &RelocationRequest) -> RelocationPreview {
    fixture
        .service()
        .relocation_preview(
            &request.chain_id,
            &request.root_id,
            &fixture.path(&request.relative_path),
        )
        .unwrap()
}

/// A plan for `file` into folder `folder` of root `root_id`.
fn plan_in(fixture: &Fixture, file: &crate::FileView, root_id: &str, folder: &str) -> String {
    let catalog = fixture.service().list_folders(root_id).unwrap();
    let folder_id = catalog
        .folders
        .iter()
        .find(|f| f.relative_path == folder)
        .unwrap()
        .folder_id
        .clone();
    match fixture
        .service()
        .build_plan(&crate::PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: root_id.to_string(),
            operation_id: None,
            target: crate::PlanTarget::Folder {
                catalog_id: catalog.catalog_id,
                folder_id,
                new_folders: Vec::new(),
            },
        })
        .unwrap()
    {
        crate::BuildOutcome::Planned { plan } => plan.plan_id,
        other => panic!("expected a plan, got {other:?}"),
    }
}

#[test]
fn an_unsettled_action_in_the_old_folder_alone_stops_the_relocation() {
    let fixture = Fixture::new();
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let other_root = fixture.service().authorize_root(other.path()).unwrap();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), other.path().join("todo.txt")).unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &record.chain_id,
            &other_root.root_id,
            &other.path().join("todo.txt"),
        )
        .unwrap();
    // Organize is moving another file inside the old folder only.
    let plan_file = fixture.register("Notes/2026/plan.md");
    let plan = plan_in(&fixture, &plan_file, &fixture.root_id, "Taxes");
    fixture.insert_action(&plan, "running");
    let (refused, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(refused), ErrorCode::UnresolvedAction);
    assert_eq!(reads, 0);
}

#[test]
fn an_unsettled_action_in_the_new_folder_alone_stops_the_relocation() {
    let fixture = Fixture::new();
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let other_root = fixture.service().authorize_root(other.path()).unwrap();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), other.path().join("todo.txt")).unwrap();
    let shown = fixture
        .service()
        .relocation_preview(
            &record.chain_id,
            &other_root.root_id,
            &other.path().join("todo.txt"),
        )
        .unwrap();
    // Organize is moving another file inside the new folder only.
    fs::create_dir(other.path().join("Sub")).unwrap();
    fs::write(other.path().join("note.txt"), "a note\n").unwrap();
    let note = fixture
        .service()
        .register_file_with(
            &other_root.root_id,
            &other.path().join("note.txt"),
            &mut |_root, parent, name| quick_read(parent, name, parent.id().dev),
        )
        .unwrap()
        .file;
    let plan = plan_in(&fixture, &note, &other_root.root_id, "Sub");
    fixture.insert_action(&plan, "running");
    let (refused, reads) = confirm_counting(&fixture, &request(&shown));
    assert_eq!(code(refused), ErrorCode::UnresolvedAction);
    assert_eq!(reads, 0);
}

#[test]
fn the_file_looked_for_is_the_newest_on_the_line() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/original.txt")).unwrap();
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "saved by an app\n").unwrap();
    only(fixture.check_memory());
    // The app's file is moved away too, then confirmed to continue the work:
    // it is not at the place, so only the history changes.
    fs::rename(fixture.path("todo.txt"), fixture.path("Notes/app.txt")).unwrap();
    missing_after_a_pass(&fixture);
    let shown = page(&fixture, &record.chain_id);
    let continued = shown.items[0].file_id.clone();
    let decided = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: shown.items[0].candidate_id.clone(),
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: shown.items[0].continue_from.clone().unwrap().snapshot_id,
                },
            }],
            current_object: None,
        })
        .unwrap();
    assert!(decided.history_only);
    // The watched file's object: its line went on, so not there.
    assert_eq!(
        code(preview(&fixture, &record.chain_id, "Taxes/original.txt")),
        ErrorCode::DecisionConflict
    );
    // The newest file's object: the record goes on there.
    let outcome = confirm(
        &fixture,
        &preview(&fixture, &record.chain_id, "Notes/app.txt").unwrap(),
    );
    assert_eq!(outcome.record.place.relative_path, "Notes/app.txt");
    let line = fixture
        .service()
        .usable_pairs_of_line(&outcome.snapshot.file_id)
        .unwrap();
    assert_eq!(line.len(), 2, "{line:?}");
    assert_eq!(line[1].before.file_id, continued);
    assert_eq!(line[1].after.snapshot_id, outcome.snapshot.snapshot_id);
    assert_eq!(line[1].evidence, PairEvidence::System);
}

/// A record judged lost while its own file was renamed away, then the file
/// put back under `back_name`.
fn lost_then_back(fixture: &Fixture, back_name: &str) -> crate::family::MemoryRecordView {
    let record = start_record(fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("todo-old.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "unrelated\n").unwrap();
    fixture.check_memory();
    let shown = page(fixture, &record.chain_id);
    fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: shown.page_token,
            items: Vec::new(),
            current_object: Some(CurrentObjectChoice::Independent {}),
        })
        .unwrap();
    // The watcher remembers the lost state for this watch.
    fixture.check_memory();
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fs::rename(fixture.path("todo-old.txt"), fixture.path(back_name)).unwrap();
    record
}

#[test]
fn a_lost_record_whose_file_comes_back_edited_saves_the_new_version() {
    let fixture = Fixture::new().without_auto_link();
    let record = lost_then_back(&fixture, "todo.txt");
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "buy milk\nedited while lost\n").unwrap();
    let first = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap()
        .pop()
        .unwrap();
    let outcome = confirm(
        &fixture,
        &preview(&fixture, &record.chain_id, "todo.txt").unwrap(),
    );
    assert_eq!(outcome.snapshot.file_id, record.origin_file_id);
    assert_eq!(
        outcome.snapshot.sequence,
        first.sequence + 1,
        "a new version"
    );
    assert!(
        outcome.record.watch.problem.is_none(),
        "{:?}",
        outcome.record.watch.problem
    );
    // The record's newest version is the new one: a later object is found
    // after it.
    enable(&fixture, &record.chain_id).unwrap();
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved by an app\n");
    only(fixture.check_memory());
    let shown = page(&fixture, &record.chain_id);
    assert_eq!(shown.items[0].found_after.as_ref(), Some(&outcome.snapshot));
}

#[test]
fn a_file_back_under_another_letter_case_resumes_at_its_recorded_place() {
    let fixture = Fixture::new().without_auto_link();
    let record = lost_then_back(&fixture, "TODO.txt");
    if !fixture.path("todo.txt").exists() {
        return; // A volume that tells letter case apart: nothing to check.
    }
    let shown = preview(&fixture, &record.chain_id, "TODO.txt").unwrap();
    assert_eq!(shown.old_place_holds, PlaceHolds::RecordedFile);
    assert_eq!(shown.new_place.relative_path, "todo.txt");
    let outcome = confirm(&fixture, &shown);
    assert_eq!(outcome.record.state, RecordState::Active);
    assert_eq!(outcome.snapshot.file_id, record.origin_file_id);
}

#[test]
fn an_unread_object_at_the_old_place_before_the_watcher_looked_marks_the_pair() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "something else\n").unwrap();
    let shown = preview(&fixture, &record.chain_id, "Taxes/todo.txt").unwrap();
    assert_eq!(shown.old_place_holds, PlaceHolds::UnreadObject);
    confirm(&fixture, &shown);
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        [(
            "replaced_uncaptured".to_string(),
            None,
            Some("relocated".to_string())
        )]
    );
    let pairs = fixture
        .service()
        .usable_pairs_of_family(&record.family_id)
        .unwrap();
    assert!(pairs[0].gap_kinds.contains(&GapKind::ReplacedUncaptured));
}
