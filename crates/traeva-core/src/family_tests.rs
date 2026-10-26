use std::fs;

use super::*;
use crate::capture::CheckOutcome;
use crate::contracts::ErrorCode;
use crate::test_fixture::{Fixture, quick_read};
use crate::test_snapshot as tree;

/// Registers `relative` and turns Memory on for it, with reads that do not
/// pause.
fn start_record(fixture: &Fixture, relative: &str) -> MemoryRecordView {
    let file = fixture.register(relative);
    fixture
        .service()
        .start_memory_for_file_with(&file.file_id, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        })
        .unwrap()
}

/// One pass, counting how many times a file was read (not just looked at).
fn check_counting_reads(fixture: &Fixture) -> (Vec<crate::WatchCheck>, usize) {
    let mut reads = 0;
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            reads += 1;
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    (checks, reads)
}

fn problem_code(outcome: &CheckOutcome) -> Option<ErrorCode> {
    match outcome {
        CheckOutcome::Problem { error } => Some(error.code),
        _ => None,
    }
}

/// How many apps save: write a new file, then rename it over the old name.
fn replace(fixture: &Fixture, relative: &str, text: &str) {
    let swap = format!("{relative}.swap");
    fs::write(fixture.path(&swap), text).unwrap();
    fs::rename(fixture.path(&swap), fixture.path(relative)).unwrap();
}

/// The chain's gaps as (kind, problem, open), oldest first.
fn gaps(fixture: &Fixture, chain_id: &str) -> Vec<(GapKind, Option<ErrorCode>, bool)> {
    fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(chain_id)
        .unwrap()
        .into_iter()
        .map(|gap| (gap.kind, gap.problem, gap.ended_at_ms.is_none()))
        .collect()
}

/// The watcher's runs as (started, last seen).
fn runs(fixture: &Fixture) -> Vec<(i64, i64)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare("SELECT started_at_ms, last_seen_at_ms FROM memory_runs ORDER BY rowid")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn turning_memory_on_starts_a_record_in_a_work_of_its_own() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    assert_eq!(record.state, RecordState::Active);
    assert_eq!(record.candidate_capture, CandidateCapture::Off);
    assert!(record.root_active);
    assert!(!record.waiting_for_relocation);
    assert_eq!(record.pending.objects, 0);
    assert_eq!(record.pending.limit, CANDIDATE_LIMIT);
    assert!(record.open_gaps.is_empty());
    assert_eq!(record.place.relative_path, "todo.txt");
    assert!(record.place.display_path.ends_with("/todo.txt"));
    assert_eq!(record.watch.versions, 1);
    assert_eq!(record.origin_file_id, record.watch.file_id);

    let family = fixture.service().memory_family(&record.family_id).unwrap();
    assert_eq!(family.members.len(), 1);
    let member = &family.members[0];
    assert_eq!(member.file_id, record.origin_file_id);
    assert_eq!(member.status, RelationStatus::ConfirmedSystem);
    assert_eq!(member.basis, MemberBasis::MemoryOn);
    assert!(member.placed);
    assert_eq!(member.snapshots.len(), 1);
    assert_eq!(family.records, vec![record.clone()]);
    assert!(family.relations.is_empty() && family.gaps.is_empty());

    // Turning it on again while it is on keeps the same record and watch.
    let again = fixture
        .service()
        .start_memory_for_file(&record.origin_file_id)
        .unwrap();
    assert_eq!(again.chain_id, record.chain_id);
    assert_eq!(again.watch.watch_id, record.watch.watch_id);
    assert_eq!(fixture.count("memory_chains"), 1);
    assert_eq!(fixture.count("memory_families"), 1);
}

#[test]
fn turning_memory_on_again_goes_on_with_the_same_record_and_records_the_time_it_was_off() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::write(fixture.path("todo.txt"), "second\n").unwrap();
    fixture.check_memory();
    fixture.advance(1_000);
    let stopped = fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    assert_eq!(stopped.state, RecordState::Stopped);
    fixture.advance(5_000);
    let again = fixture
        .service()
        .start_memory_for_file(&record.origin_file_id)
        .unwrap();
    assert_eq!(again.chain_id, record.chain_id, "the same record task");
    assert_ne!(again.watch.watch_id, record.watch.watch_id, "a new watch");
    assert_eq!(again.state, RecordState::Active);
    assert_eq!(again.candidate_capture, CandidateCapture::Off);
    assert_eq!(fixture.count("memory_families"), 1);

    let gaps = fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(&record.chain_id)
        .unwrap();
    assert_eq!(gaps.len(), 1);
    let off = &gaps[0];
    assert_eq!(off.kind, GapKind::MemoryOff);
    let latest = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap();
    assert_eq!(
        off.after_snapshot_id.as_deref(),
        Some(latest[1].snapshot_id.as_str())
    );
    assert_eq!(off.started_at_ms, stopped.watch.stopped_at_ms.unwrap());
    assert_eq!(off.ended_at_ms, Some(again.watch.started_at_ms));
    assert_eq!(off.end_reason, Some(GapEnd::Resumed));
    assert_eq!(
        fixture
            .db()
            .query_row(
                "SELECT reason || ' ' || previous_watch_id FROM memory_chain_watches
             WHERE watch_id = ?1",
                [&again.watch.watch_id],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        format!("restarted {}", record.watch.watch_id)
    );
}

#[test]
fn a_replacement_opens_one_gap_and_reads_nothing() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "replaced by an editor\n");
    for _ in 0..3 {
        let (checks, reads) = check_counting_reads(&fixture);
        assert_eq!(
            problem_code(&checks[0].outcome),
            Some(ErrorCode::FileChanged)
        );
        assert_eq!(reads, 0, "an object Memory may not read is not read");
    }
    assert_eq!(fixture.count("files"), 1, "nothing was registered");
    assert_eq!(fixture.count("snapshots"), 1);
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::ReplacedUncaptured, None, true)]
    );
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.open_gaps.len(), 1);
    assert_eq!(
        view.open_gaps[0].after.as_ref().map(|s| s.sequence),
        Some(1),
        "the gap lies after the newest version"
    );
    // Stopping ends it.
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::ReplacedUncaptured, None, false)]
    );
}

#[test]
fn a_place_found_again_under_organize_is_reported_replaced() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "new object\n");
    // Organize chooses the file at that place again: the watched record
    // becomes missing, the new object gets its own record.
    fixture.register("todo.txt");
    let (checks, reads) = check_counting_reads(&fixture);
    assert_eq!(
        problem_code(&checks[0].outcome),
        Some(ErrorCode::FileChanged)
    );
    assert_eq!(reads, 0);
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::ReplacedUncaptured, None, true)]
    );
}

#[test]
fn a_place_that_cannot_be_captured_writes_one_start_and_one_end() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    for _ in 0..3 {
        let (checks, reads) = check_counting_reads(&fixture);
        assert_eq!(
            problem_code(&checks[0].outcome),
            Some(ErrorCode::FileNotFound)
        );
        assert_eq!(reads, 0);
    }
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::CaptureFailed, Some(ErrorCode::FileNotFound), true)]
    );
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert!(view.waiting_for_relocation);
    assert_eq!(view.state, RecordState::Active);

    // Put back, edited: captured, and the gap ends with that capture.
    fs::rename(fixture.path("Taxes/todo.txt"), fixture.path("todo.txt")).unwrap();
    fs::write(fixture.path("todo.txt"), "edited while away\n").unwrap();
    assert!(matches!(
        fixture.check_memory()[0].outcome,
        CheckOutcome::Saved { .. }
    ));
    let rows = fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(&record.chain_id)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].end_reason, Some(GapEnd::Captured));
    assert!(
        !fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .waiting_for_relocation
    );
}

#[test]
fn a_failure_that_changes_ends_its_gap_and_starts_another() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fixture.check_memory();
    fixture.service().revoke_root(&fixture.root_id).unwrap();
    for _ in 0..2 {
        let checks = fixture.check_memory();
        assert_eq!(
            problem_code(&checks[0].outcome),
            Some(ErrorCode::RootRevoked)
        );
    }
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![
            (GapKind::CaptureFailed, Some(ErrorCode::FileNotFound), false),
            (GapKind::CaptureFailed, Some(ErrorCode::RootRevoked), true),
        ]
    );
    let first = &fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(&record.chain_id)
        .unwrap()[0];
    assert_eq!(first.end_reason, Some(GapEnd::Changed), "not recovered");
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert!(!view.root_active);
    assert_eq!(view.candidate_capture, CandidateCapture::Off);
}

#[test]
fn a_file_still_being_saved_opens_no_gap() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::write(fixture.path("todo.txt"), "half saved\n").unwrap();
    let mut writes = 0;
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            crate::observe::read_stable_with(
                parent,
                name,
                root.id().dev,
                crate::observe::ReadLimits::SPEC,
                &mut |_| {
                    writes += 1;
                    fs::write(fixture.path("todo.txt"), format!("still saving {writes}\n"))
                        .unwrap();
                },
            )
        })
        .unwrap();
    assert_eq!(
        problem_code(&checks[0].outcome),
        Some(ErrorCode::FileChanging)
    );
    assert!(gaps(&fixture, &record.chain_id).is_empty());
}

#[test]
fn a_failure_seen_without_the_lock_opens_no_gap_when_the_locked_look_is_fine() {
    // Organize holds the execution lock for a whole move: a look without the
    // lock may see the place empty for a moment. Under the lock it is fine.
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.service().note_gaps(
        &record.watch.watch_id,
        &record.chain_id,
        &record.origin_file_id,
        GapSignal::Failed {
            code: ErrorCode::FileNotFound,
            unlocked: true,
        },
    );
    assert!(gaps(&fixture, &record.chain_id).is_empty());
    // A failure seen under the lock is written as it is.
    fixture.service().note_gaps(
        &record.watch.watch_id,
        &record.chain_id,
        &record.origin_file_id,
        GapSignal::Failed {
            code: ErrorCode::UnresolvedAction,
            unlocked: false,
        },
    );
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(
            GapKind::CaptureFailed,
            Some(ErrorCode::UnresolvedAction),
            true
        )]
    );
}

#[test]
fn a_gap_for_a_watch_that_is_no_longer_on_is_not_written() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    fixture.service().note_gaps(
        &record.watch.watch_id,
        &record.chain_id,
        &record.origin_file_id,
        GapSignal::Replaced,
    );
    assert!(gaps(&fixture, &record.chain_id).is_empty());
}

#[test]
fn a_gap_that_cannot_be_saved_is_not_claimed_and_is_written_later() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    fixture
        .db()
        .execute_batch(
            "CREATE TRIGGER refuse_gaps BEFORE INSERT ON memory_gaps
             BEGIN SELECT RAISE(ABORT, 'test: storage refuses gaps'); END;",
        )
        .unwrap();
    replace(&fixture, "todo.txt", "replaced\n");
    let checks = fixture.check_memory();
    assert_eq!(
        problem_code(&checks[0].outcome),
        Some(ErrorCode::FileChanged)
    );
    assert!(gaps(&fixture, &record.chain_id).is_empty());
    assert!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .open_gaps
            .is_empty()
    );
    fixture
        .db()
        .execute_batch("DROP TRIGGER refuse_gaps;")
        .unwrap();
    fixture.check_memory();
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::ReplacedUncaptured, None, true)]
    );
}

#[test]
fn turning_a_file_off_stops_its_record_and_ends_its_gaps() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "replaced\n");
    fixture.check_memory();
    fixture
        .service()
        .stop_memory(&record.origin_file_id)
        .unwrap();
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.state, RecordState::Stopped);
    assert!(view.open_gaps.is_empty());
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::ReplacedUncaptured, None, false)]
    );
    // Stopping again changes nothing.
    let again = fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    assert_eq!(again, view);
}

#[test]
fn a_file_now_at_another_records_place_waits_for_that_record() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "new object\n");
    let occupant = fixture.register("todo.txt");
    let refused = fixture
        .service()
        .start_memory_for_file(&occupant.file_id)
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::CandidatePending);
    assert_eq!(fixture.count("memory_chains"), 1);
    // Once that record is stopped, the new object can have its own.
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    let own = fixture
        .service()
        .start_memory_for_file(&occupant.file_id)
        .unwrap();
    assert_ne!(own.chain_id, record.chain_id);
    assert_ne!(own.family_id, record.family_id, "a work of its own");
}

#[test]
fn choosing_a_file_with_the_picker_registers_it_and_turns_memory_on() {
    let fixture = Fixture::new();
    let record = fixture
        .service()
        .start_memory_at_with(
            &fixture.root_id,
            &fixture.path("todo.txt"),
            &mut |root, parent, name| quick_read(parent, name, root.id().dev),
        )
        .unwrap();
    assert_eq!(record.place.relative_path, "todo.txt");
    assert_eq!(record.watch.versions, 1);
    let outside = fixture
        .service()
        .start_memory_at(&fixture.root_id, std::path::Path::new("/etc/hosts"))
        .unwrap_err();
    assert_eq!(outside.code, ErrorCode::OutsideRoot);
}

#[test]
fn records_are_listed_on_first_and_unknown_ids_are_refused() {
    let fixture = Fixture::new();
    let first = start_record(&fixture, "todo.txt");
    fixture.advance(10);
    let second = start_record(&fixture, "Notes/2026/plan.md");
    fixture
        .service()
        .stop_memory_record(&second.chain_id)
        .unwrap();
    let listed: Vec<(String, RecordState)> = fixture
        .service()
        .memory_records()
        .unwrap()
        .into_iter()
        .map(|record| (record.chain_id, record.state))
        .collect();
    assert_eq!(
        listed,
        vec![
            (first.chain_id.clone(), RecordState::Active),
            (second.chain_id.clone(), RecordState::Stopped),
        ]
    );
    let unknown = uuid::Uuid::new_v4().to_string();
    let service = fixture.service();
    assert_eq!(
        service.memory_record(&unknown).unwrap_err().code,
        ErrorCode::UnknownRecord
    );
    assert_eq!(
        service.stop_memory_record(&unknown).unwrap_err().code,
        ErrorCode::UnknownRecord
    );
    assert_eq!(
        service.memory_family(&unknown).unwrap_err().code,
        ErrorCode::UnknownFamily
    );
    for bad in ["", "../../etc", "not-an-id"] {
        assert_eq!(
            service.memory_record(bad).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            service.memory_family(bad).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            service.memory_watches_of(bad).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }
    assert_eq!(
        service.memory_watches_of(&unknown).unwrap_err().code,
        ErrorCode::UnknownFile
    );
}

#[test]
fn every_watch_of_a_file_is_listed_with_its_gaps() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let file_id = record.origin_file_id.clone();
    for _ in 0..3 {
        fixture.advance(1_000);
        fixture
            .service()
            .stop_memory_record(&record.chain_id)
            .unwrap();
        fixture.advance(1_000);
        fixture.service().start_memory_for_file(&file_id).unwrap();
    }
    let view = fixture.service().memory_watches_of(&file_id).unwrap();
    assert_eq!(view.file_id, file_id);
    assert_eq!(view.watches.len(), 4);
    assert!(
        view.watches
            .windows(2)
            .all(|pair| pair[0].started_at_ms < pair[1].started_at_ms),
        "oldest first"
    );
    let kinds: Vec<GapKind> = view.gaps.iter().map(|gap| gap.kind).collect();
    assert_eq!(kinds, vec![GapKind::MemoryOff; 3]);
    assert!(
        view.gaps
            .iter()
            .all(|gap| gap.chain_id.as_deref() == Some(record.chain_id.as_str()))
    );
    assert_eq!(
        view.current_run_started_at_ms,
        Some(crate::test_fixture::START_MS)
    );
}

#[test]
fn the_watcher_run_is_recorded_and_kept_up_to_date() {
    let fixture = Fixture::new();
    start_record(&fixture, "todo.txt");
    let start = crate::test_fixture::START_MS;
    assert_eq!(
        runs(&fixture),
        vec![(start, start)],
        "begun before the first capture"
    );
    fixture.advance(10_000);
    fixture.check_memory();
    assert_eq!(
        runs(&fixture),
        vec![(start, start)],
        "not written at every pass"
    );
    fixture.advance(25_000);
    fixture.check_memory();
    assert_eq!(
        runs(&fixture),
        vec![(start, start + 35_000)],
        "at least every 30 s"
    );
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "saved\n").unwrap();
    fixture.check_memory();
    assert_eq!(
        runs(&fixture),
        vec![(start, start + 36_000)],
        "and after every pass that saved a version"
    );
}

#[test]
fn time_traeva_was_not_running_is_a_gap_even_when_nothing_changed() {
    let mut fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let file_id = record.origin_file_id.clone();
    let start = crate::test_fixture::START_MS;
    fixture.advance(40_000);
    fixture.check_memory();
    // Traeva quits; an hour later it starts again.
    fixture.restart();
    fixture.advance(3_600_000);
    let before_first_pass = fixture.service().memory_watches_of(&file_id).unwrap();
    assert_eq!(before_first_pass.current_run_started_at_ms, None);
    let open: Vec<_> = before_first_pass
        .gaps
        .iter()
        .map(|gap| (gap.kind, gap.started_at_ms, gap.ended_at_ms))
        .collect();
    assert_eq!(
        open,
        vec![(GapKind::WatcherStopped, start + 40_000, None)],
        "not running yet: the stretch is still going on"
    );
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(checks[0].outcome, CheckOutcome::Unchanged));
    assert_eq!(reads, 0, "the same file is not read");
    let after = fixture.service().memory_watches_of(&file_id).unwrap();
    let restarted = start + 40_000 + 3_600_000;
    assert_eq!(after.current_run_started_at_ms, Some(restarted));
    let gaps: Vec<_> = after
        .gaps
        .iter()
        .map(|gap| (gap.kind, gap.started_at_ms, gap.ended_at_ms))
        .collect();
    assert_eq!(
        gaps,
        vec![(GapKind::WatcherStopped, start + 40_000, Some(restarted))]
    );
}

#[test]
fn run_history_works_out_the_stretches_between_runs() {
    let history = RunHistory {
        runs: vec![
            ("a".to_string(), 100, 200),
            ("b".to_string(), 300, 400),
            ("c".to_string(), 400, 500),
        ],
        current: Some("c".to_string()),
    };
    assert_eq!(history.downtimes(), vec![(200, Some(300))]);
    assert_eq!(
        history.kinds_between(150, 350),
        vec![GapKind::WatcherStopped]
    );
    assert!(history.kinds_between(310, 450).is_empty());
    assert_eq!(
        history.kinds_between(50, 150),
        vec![GapKind::NotRecorded],
        "before the first run, when the watcher ran is not known"
    );
    let not_running_now = RunHistory {
        current: None,
        ..history
    };
    assert_eq!(
        not_running_now.downtimes(),
        vec![(200, Some(300)), (500, None)]
    );
    assert_eq!(
        not_running_now.kinds_between(510, 520),
        vec![GapKind::WatcherStopped]
    );
    // The clock was set back between two runs: the stretch between the two
    // times still counts.
    let set_back = RunHistory {
        runs: vec![("a".to_string(), 100, 200), ("b".to_string(), 150, 300)],
        current: Some("b".to_string()),
    };
    assert_eq!(set_back.downtimes(), vec![(150, Some(200))]);
    assert_eq!(set_back.first_started(), Some(100));
    let never = RunHistory {
        runs: Vec::new(),
        current: None,
    };
    assert_eq!(never.kinds_between(1, 2), vec![GapKind::NotRecorded]);
}

#[test]
fn record_flows_write_nothing_inside_the_root() {
    let fixture = Fixture::new();
    let before = tree::snapshot(fixture.root.path());
    let record = start_record(&fixture, "todo.txt");
    fixture.check_memory();
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    fixture
        .service()
        .start_memory_for_file(&record.origin_file_id)
        .unwrap();
    fixture.service().memory_records().unwrap();
    fixture.service().memory_family(&record.family_id).unwrap();
    fixture
        .service()
        .memory_watches_of(&record.origin_file_id)
        .unwrap();
    assert_eq!(tree::snapshot(fixture.root.path()), before);
}

#[test]
fn an_empty_place_then_another_object_there_changes_the_gap() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fixture.check_memory();
    assert!(
        fixture
            .service()
            .memory_record(&record.chain_id)
            .unwrap()
            .waiting_for_relocation
    );
    // Another document saved at the same place.
    fs::write(fixture.path("todo.txt"), "a new document\n").unwrap();
    let (checks, reads) = check_counting_reads(&fixture);
    assert_eq!(
        problem_code(&checks[0].outcome),
        Some(ErrorCode::FileChanged)
    );
    assert_eq!(reads, 0);
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![
            (GapKind::CaptureFailed, Some(ErrorCode::FileNotFound), false),
            (GapKind::ReplacedUncaptured, None, true),
        ]
    );
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert!(
        !view.waiting_for_relocation,
        "the place is not empty any more"
    );
    assert_eq!(view.open_gaps.len(), 1);
    // And back: the new object goes away too.
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fixture.check_memory();
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![
            (GapKind::CaptureFailed, Some(ErrorCode::FileNotFound), false),
            (GapKind::ReplacedUncaptured, None, false),
            (GapKind::CaptureFailed, Some(ErrorCode::FileNotFound), true),
        ]
    );
}

#[test]
fn turning_memory_on_again_after_a_replacement_reads_nothing() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    replace(
        &fixture,
        "todo.txt",
        "saved by an editor while Memory was off\n",
    );
    let mut reads = 0;
    let watch = fixture
        .service()
        .start_memory_with(&record.origin_file_id, &mut |root, parent, name| {
            reads += 1;
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    assert_eq!(reads, 0, "an object Memory may not read is not read");
    assert_eq!(
        watch.problem.as_ref().map(|problem| problem.code),
        Some(ErrorCode::FileChanged)
    );
    assert_eq!(fixture.count("snapshots"), 1);
}

#[test]
fn the_run_begins_before_the_first_capture_on_a_running_clock() {
    // Every reading of this clock is one millisecond later.
    let app = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let root = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    fs::write(root.path().join("todo.txt"), "first\n").unwrap();
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let clock = std::sync::Arc::clone(&ticks);
    let service = OrganizeService::start_with_clock(
        app.path().join("Traeva"),
        Box::new(move || {
            crate::test_fixture::START_MS + clock.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        }),
    );
    let root_id = service.authorize_root(root.path()).unwrap().root_id;
    let read = &mut |root: &crate::platform::macos::Folder,
                     parent: &crate::platform::macos::Folder,
                     name: &str| quick_read(parent, name, root.id().dev);
    let record = service
        .start_memory_at_with(&root_id, &root.path().join("todo.txt"), read)
        .unwrap();
    fs::write(root.path().join("todo.txt"), "second\n").unwrap();
    service.check_memory_with(read).unwrap();
    let versions = service.list_snapshots(&record.origin_file_id).unwrap();
    let run_started = service
        .memory_watches_of(&record.origin_file_id)
        .unwrap()
        .current_run_started_at_ms
        .unwrap();
    assert!(run_started < versions[0].captured_at_ms);
    let pairs = service
        .usable_pairs_of_file(&record.origin_file_id)
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert!(pairs[0].gap_kinds.is_empty(), "{:?}", pairs[0].gap_kinds);
}

#[test]
fn time_before_runs_were_recorded_is_not_recorded_and_only_touching_stretches_count() {
    let mut fixture = Fixture::new();
    let start = crate::test_fixture::START_MS;
    let record = start_record(&fixture, "todo.txt");
    let file_id = record.origin_file_id.clone();
    // As if this watch came from before schema version 7: the first run
    // began after it.
    fixture
        .db()
        .execute_batch(&format!(
            "DROP TRIGGER memory_runs_fixed_columns;
             UPDATE memory_runs SET started_at_ms = {late}, last_seen_at_ms = {late};",
            late = start + 10_000
        ))
        .unwrap();
    fixture.advance(20_000);
    fs::write(fixture.path("todo.txt"), "second\n").unwrap();
    fixture.check_memory();
    let view = fixture.service().memory_watches_of(&file_id).unwrap();
    let gaps: Vec<_> = view
        .gaps
        .iter()
        .map(|gap| (gap.kind, gap.started_at_ms, gap.ended_at_ms))
        .collect();
    assert_eq!(
        gaps,
        vec![(GapKind::NotRecorded, start, Some(start + 10_000))]
    );
    let pairs = fixture.service().usable_pairs_of_file(&file_id).unwrap();
    assert_eq!(pairs[0].gap_kinds, vec![GapKind::NotRecorded]);

    // Memory turned off, then Traeva quits later: that stretch does not
    // touch this file's watches.
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    fixture.advance(40_000);
    fixture.check_memory();
    fixture.restart();
    fixture.advance(60_000);
    fixture.check_memory();
    let view = fixture.service().memory_watches_of(&file_id).unwrap();
    assert!(
        view.gaps
            .iter()
            .all(|gap| gap.kind != GapKind::WatcherStopped),
        "{:?}",
        view.gaps
    );
}

#[test]
fn record_and_work_json_names_are_fixed() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "replaced\n");
    fixture.check_memory();
    let keys = |value: &serde_json::Value| -> Vec<String> {
        value.as_object().unwrap().keys().cloned().collect()
    };
    let view =
        serde_json::to_value(fixture.service().memory_record(&record.chain_id).unwrap()).unwrap();
    assert_eq!(
        keys(&view["open_gaps"][0]),
        vec![
            "after",
            "chain_id",
            "end_reason",
            "ended_at_ms",
            "gap_id",
            "kind",
            "problem",
            "started_at_ms"
        ]
    );
    assert_eq!(view["open_gaps"][0]["kind"], "replaced_uncaptured");
    assert_eq!(
        keys(&view["place"]),
        vec!["display_path", "relative_path", "root_id"]
    );
    assert_eq!(
        keys(&view["pending"]),
        vec!["limit", "objects", "snapshots"]
    );
    let family =
        serde_json::to_value(fixture.service().memory_family(&record.family_id).unwrap()).unwrap();
    assert_eq!(
        keys(&family),
        vec![
            "boundaries",
            "created_at_ms",
            "family_id",
            "gaps",
            "members",
            "merged_family_ids",
            "records",
            "relations"
        ]
    );
    assert_eq!(
        keys(&family["members"][0]),
        vec![
            "basis",
            "decided_at_ms",
            "display_path",
            "file_id",
            "merged_from",
            "placed",
            "present",
            "snapshots",
            "status"
        ]
    );
    let watches = serde_json::to_value(
        fixture
            .service()
            .memory_watches_of(&record.origin_file_id)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        keys(&watches),
        vec!["current_run_started_at_ms", "file_id", "gaps", "watches"]
    );
    assert_eq!(
        keys(&watches["gaps"][0]),
        vec![
            "chain_id",
            "ended_at_ms",
            "gap_id",
            "kind",
            "problem",
            "started_at_ms"
        ]
    );
}

#[test]
fn a_replacement_found_only_by_the_capture_is_recorded_as_one() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fs::write(fixture.path("todo.txt"), "edited\n").unwrap();
    // The editor replaces the file just as Memory reads it.
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            replace(&fixture, "todo.txt", "saved by replacing\n");
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    assert_eq!(
        problem_code(&checks[0].outcome),
        Some(ErrorCode::FileChanged)
    );
    assert_eq!(fixture.count("snapshots"), 1, "nothing of it is saved");
    assert_eq!(
        gaps(&fixture, &record.chain_id),
        vec![(GapKind::ReplacedUncaptured, None, true)],
        "a replacement, not a failed capture"
    );
}

#[test]
fn quitting_soon_after_memory_was_turned_off_is_not_a_gap_of_that_watch() {
    let mut fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(40_000);
    fixture.check_memory();
    fixture.advance(5_000);
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    // Traeva quits a few seconds later, well within 30 s.
    fixture.advance(3_000);
    fixture.restart();
    fixture.advance(60_000);
    fixture.check_memory();
    let view = fixture
        .service()
        .memory_watches_of(&record.origin_file_id)
        .unwrap();
    assert!(
        view.gaps
            .iter()
            .all(|gap| gap.kind != GapKind::WatcherStopped),
        "{:?}",
        view.gaps
    );
}

#[test]
fn turning_the_watched_file_on_again_after_its_line_went_on_is_refused() {
    use crate::family_decide::{CandidateChoice, CandidateDecision, CandidateDecisionRequest};
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    crate::candidates::tests::enable(&fixture, &record.chain_id).unwrap();
    for text in ["first app save\n", "second app save\n"] {
        fixture.advance(1_000);
        replace(&fixture, "todo.txt", text);
        fixture.check_memory();
    }
    // Only the first object is confirmed to continue the work: only the
    // history changes, the watch stays on the original file.
    let page = fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    let decided = fixture
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: page.page_token.clone(),
            items: vec![CandidateDecision {
                candidate_id: page.items[0].candidate_id.clone(),
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: page.items[0].continue_from.clone().unwrap().snapshot_id,
                },
            }],
            current_object: None,
        })
        .unwrap();
    assert!(decided.history_only);
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    // The original file is no longer the newest on its line (here it is
    // not at its place either): nothing starts.
    let watches = fixture.count("memory_watches");
    assert!(
        fixture
            .service()
            .start_memory_for_file(&record.origin_file_id)
            .is_err()
    );
    assert_eq!(fixture.count("memory_watches"), watches);
    // The newest file on the line goes on in the same record.
    let front = page.items[0].file_id.clone();
    let again = fixture.service().start_memory_for_file(&front);
    if let Ok(again) = again {
        assert_eq!(again.chain_id, record.chain_id);
    }
}
