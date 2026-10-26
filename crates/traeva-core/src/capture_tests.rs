use std::fs;
use std::time::{Duration, SystemTime};

use super::*;
use crate::contracts::ErrorCode;
use crate::test_fixture::{Fixture, edit_keeping_size_and_mtime, quick_read};
use crate::test_snapshot as tree;

/// One pass, counting how many times a file was read (not just looked at).
fn check_counting_reads(fixture: &Fixture) -> (CheckOutcome, usize) {
    let mut reads = 0;
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            reads += 1;
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    (only(checks), reads)
}

/// The one outcome of a pass that looked at exactly one watch.
fn only(checks: Vec<WatchCheck>) -> CheckOutcome {
    assert_eq!(checks.len(), 1, "{checks:?}");
    checks.into_iter().next().unwrap().outcome
}

fn problem_code(outcome: &CheckOutcome) -> Option<ErrorCode> {
    match outcome {
        CheckOutcome::Problem { error } => Some(error.code),
        _ => None,
    }
}

fn saved_bytes(fixture: &Fixture, file_id: &str) -> Vec<Vec<u8>> {
    fixture
        .service()
        .list_snapshots(file_id)
        .unwrap()
        .iter()
        .map(|s| fixture.service().read_snapshot(&s.snapshot_id).unwrap().1)
        .collect()
}

#[test]
fn turning_memory_on_saves_the_first_version_right_away() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let view = fixture.start_memory(&file.file_id).unwrap();
    assert_eq!(view.status, WatchStatus::Active);
    assert_eq!(view.file_id, file.file_id);
    assert_eq!(view.relative_path, "todo.txt");
    assert_eq!(view.versions, 1);
    assert_eq!(view.latest.as_ref().unwrap().sha256, file.sha256);
    assert!(view.checked_at_ms.is_some());
    assert_eq!(view.problem, None);
    assert_eq!(fixture.service().memory_watches().unwrap(), [view]);
}

#[test]
fn turning_it_on_twice_keeps_one_watch() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let first = fixture.start_memory(&file.file_id).unwrap();
    let again = fixture.start_memory(&file.file_id).unwrap();
    assert_eq!(again.watch_id, first.watch_id);
    assert_eq!(fixture.count("memory_watches"), 1);
    assert_eq!(again.versions, 1);
}

#[test]
fn each_save_becomes_a_version_and_an_untouched_file_is_not_read() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    let (outcome, reads) = check_counting_reads(&fixture);
    assert_eq!(outcome, CheckOutcome::Unchanged);
    assert_eq!(reads, 0, "an untouched file is only looked at, never read");

    fs::write(fixture.path("todo.txt"), "buy milk\nand bread\n").unwrap();
    let CheckOutcome::Saved { snapshot } = only(fixture.check_memory()) else {
        panic!("expected a new version");
    };
    assert_eq!(snapshot.sequence, 2);
    assert_eq!(only(fixture.check_memory()), CheckOutcome::Unchanged);

    fs::write(fixture.path("todo.txt"), "buy bread\n").unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
    assert_eq!(
        saved_bytes(&fixture, &file.file_id),
        [&b"buy milk\n"[..], b"buy milk\nand bread\n", b"buy bread\n"]
    );
}

#[test]
fn an_edit_that_keeps_size_and_modification_time_is_still_found() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    edit_keeping_size_and_mtime(&fixture.path("todo.txt"), "buy MILK\n");
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
}

#[test]
fn saving_the_same_text_again_adds_no_version() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy milk\n").unwrap();
    fs::File::options()
        .write(true)
        .open(fixture.path("todo.txt"))
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::SameBytes { .. }
    ));
    assert_eq!(fixture.count("snapshots"), 1);
    // Recorded: the next pass sees nothing new.
    assert_eq!(only(fixture.check_memory()), CheckOutcome::Unchanged);
}

#[test]
fn files_memory_is_not_on_for_are_never_captured() {
    let fixture = Fixture::new();
    let watched = fixture.register("todo.txt");
    let other = fixture.register("Notes/2026/plan.md");
    fixture.start_memory(&watched.file_id).unwrap();
    fs::write(fixture.path("Notes/2026/plan.md"), "# Plan\n\nchanged\n").unwrap();
    fs::write(fixture.path("new-note.txt"), "a new file\n").unwrap();
    fs::write(
        fixture.path("todo.txt.sb-1234-tmp"),
        "an editor's temp file\n",
    )
    .unwrap();

    let checks = fixture.check_memory();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].file_id, watched.file_id);
    assert_eq!(checks[0].outcome, CheckOutcome::Unchanged);
    assert!(
        fixture
            .service()
            .list_snapshots(&other.file_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.count("snapshots"), 1);
    assert_eq!(fixture.count("files"), 2, "no other file was registered");
}

#[test]
fn after_turning_memory_off_nothing_new_is_saved() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let on = fixture.start_memory(&file.file_id).unwrap();
    let off = fixture
        .service()
        .stop_memory(&file.file_id)
        .unwrap()
        .unwrap();
    assert_eq!(off.watch_id, on.watch_id);
    assert_eq!(off.status, WatchStatus::Stopped);
    assert!(off.stopped_at_ms.is_some());
    assert_eq!(off.versions, 1, "the saved version stays");

    fs::write(fixture.path("todo.txt"), "changed after turning it off\n").unwrap();
    assert!(fixture.check_memory().is_empty());
    assert_eq!(fixture.count("snapshots"), 1);
    // Turning it off again changes nothing.
    assert_eq!(
        fixture.service().stop_memory(&file.file_id).unwrap(),
        Some(off)
    );

    // On again: a new watch, which saves the file as it is now.
    let again = fixture.start_memory(&file.file_id).unwrap();
    assert_ne!(again.watch_id, on.watch_id);
    assert_eq!(again.versions, 2);
}

#[test]
fn turning_memory_off_waits_for_a_capture_under_way_then_nothing_more_happens() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\n").unwrap();

    std::thread::scope(|scope| {
        let mut stopper = None;
        let checks = fixture
            .service()
            .check_memory_with(&mut |root, parent, name| {
                let read = quick_read(parent, name, root.id().dev);
                // The user turns Memory off while the file is being read.
                let handle = scope.spawn(|| fixture.service().stop_memory(&file.file_id));
                std::thread::sleep(Duration::from_millis(200));
                assert!(!handle.is_finished(), "turning off waits for the capture");
                stopper = Some(handle);
                read
            })
            .unwrap();
        assert!(matches!(only(checks), CheckOutcome::Saved { .. }));
        let off = stopper.unwrap().join().unwrap().unwrap().unwrap();
        assert_eq!(off.status, WatchStatus::Stopped);
        assert_eq!(off.versions, 2);
    });

    // Once it returned: nothing more is read or saved.
    fs::write(fixture.path("todo.txt"), "changed after turning it off\n").unwrap();
    assert!(fixture.check_memory().is_empty());
    assert_eq!(fixture.count("snapshots"), 2);
    // No stored copy without a snapshot.
    let copies: usize = fs::read_dir(fixture.app.path().join("Traeva").join(crate::SNAPSHOT_DIR))
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name() != "incoming")
        .map(|shard| fs::read_dir(shard.path()).unwrap().count())
        .sum();
    assert_eq!(copies, 2);
}

#[test]
fn a_watch_that_is_off_is_not_read_at_all() {
    // Turned off between a pass listing the watches and its capture.
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let on = fixture.start_memory(&file.file_id).unwrap();
    fixture.service().stop_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "changed\n").unwrap();
    let mut reads = 0;
    let captured = fixture
        .service()
        .capture_for(
            &file.file_id,
            Some(&on.watch_id),
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap();
    assert!(captured.is_none());
    assert_eq!(reads, 0);
}

#[test]
fn a_watch_turned_off_behind_the_capture_saves_nothing() {
    // The same transaction that would save checks the watch: here it is
    // turned off directly in the database while the file is read.
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\n").unwrap();
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            let read = quick_read(parent, name, root.id().dev);
            fixture
                .db()
                .execute(
                    "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1",
                    [],
                )
                .unwrap();
            read
        })
        .unwrap();
    assert_eq!(only(checks), CheckOutcome::Stopped);
    assert_eq!(fixture.count("snapshots"), 1);
    let (row, _) = fixture
        .service()
        .store()
        .unwrap()
        .get_file(&file.file_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        row.record_revision, file.file_revision,
        "nothing was recorded"
    );
}

#[test]
fn a_first_version_that_could_not_be_saved_is_saved_at_the_next_pass() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let mut writes = 0;
    let view = fixture
        .service()
        .start_memory_with(&file.file_id, &mut |root, parent, name| {
            crate::observe::read_stable_with(
                parent,
                name,
                root.id().dev,
                crate::observe::ReadLimits::SPEC,
                &mut |_| {
                    writes += 1;
                    fs::write(fixture.path("todo.txt"), format!("saving {writes}\n")).unwrap();
                },
            )
        })
        .unwrap();
    assert_eq!(view.versions, 0);
    assert_eq!(
        view.problem.as_ref().map(|p| p.code),
        Some(ErrorCode::FileChanging)
    );
    let (outcome, reads) = check_counting_reads(&fixture);
    assert!(matches!(outcome, CheckOutcome::Saved { .. }), "{outcome:?}");
    assert_eq!(reads, 1);
    assert_eq!(fixture.service().memory_watches().unwrap()[0].problem, None);
}

#[test]
fn a_change_recorded_by_choosing_the_file_again_is_still_saved() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy milk\nand tea\n").unwrap();
    // Organize reads the file again: its record now has the new bytes.
    let again = fixture.register("todo.txt");
    assert_eq!(again.file_id, file.file_id);

    let CheckOutcome::Saved { snapshot } = only(fixture.check_memory()) else {
        panic!("the change must still become a version");
    };
    assert_eq!(snapshot.sha256, again.sha256);
    assert_eq!(only(fixture.check_memory()), CheckOutcome::Unchanged);
}

#[test]
fn a_folder_revoked_while_a_version_is_read_saves_nothing() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy milk\nand eggs\n").unwrap();
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            let read = quick_read(parent, name, root.id().dev);
            fixture.service().revoke_root(&fixture.root_id).unwrap();
            read
        })
        .unwrap();
    assert_eq!(problem_code(&only(checks)), Some(ErrorCode::RootRevoked));
    assert_eq!(fixture.count("snapshots"), 1);
}

#[test]
fn turning_memory_on_returns_the_watch_as_it_is_afterwards() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let view = fixture
        .service()
        .start_memory_with(&file.file_id, &mut |root, parent, name| {
            let read = quick_read(parent, name, root.id().dev);
            fixture
                .db()
                .execute(
                    "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1",
                    [],
                )
                .unwrap();
            read
        })
        .unwrap();
    assert_eq!(view.status, WatchStatus::Stopped);
    assert_eq!(view.versions, 0);
}

#[test]
fn a_file_saved_by_replacing_it_is_recorded_as_its_next_version() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    // How many apps save: write a new file, then rename it over the old name.
    fs::write(fixture.path("todo.tmp"), "buy milk\nand eggs\n").unwrap();
    fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();

    // The student's 10/14 decision: a new object at the exact place is the
    // next version (auto_link.rs has the details).
    let (outcome, reads) = check_counting_reads(&fixture);
    let CheckOutcome::LinkedAutomatically { snapshot } = outcome else {
        panic!("expected the next version, got {outcome:?}");
    };
    assert_eq!(reads, 1, "the new file is read once");
    assert_ne!(snapshot.file_id, file.file_id, "a new file record");
    assert_eq!(fixture.count("snapshots"), 2);
    let views = fixture.service().memory_watches().unwrap();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].status, WatchStatus::Active);
    assert_eq!(views[0].file_id, snapshot.file_id, "Memory goes on there");
    assert_eq!(views[1].status, WatchStatus::Stopped);
    assert_eq!(views[1].problem, None);
}

/// When Traeva cannot follow the new file by itself, it only reports it.
#[test]
fn a_replacement_that_is_not_linked_is_reported_and_not_read() {
    let fixture = Fixture::new().without_auto_link();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.tmp"), "buy milk\nand eggs\n").unwrap();
    fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();

    let (outcome, reads) = check_counting_reads(&fixture);
    assert_eq!(problem_code(&outcome), Some(ErrorCode::FileChanged));
    assert_eq!(reads, 0, "another file at that name is not read");
    assert_eq!(fixture.count("snapshots"), 1);
    assert_eq!(fixture.count("files"), 1, "the new file was not registered");
    let view = &fixture.service().memory_watches().unwrap()[0];
    assert_eq!(view.status, WatchStatus::Active);
    assert_eq!(
        view.problem.as_ref().map(|p| p.code),
        Some(ErrorCode::FileChanged)
    );
}

#[test]
fn a_file_renamed_or_moved_outside_traeva_is_reported_not_followed() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/list.txt")).unwrap();
    fs::write(fixture.path("Taxes/list.txt"), "edited after the move\n").unwrap();

    let (outcome, reads) = check_counting_reads(&fixture);
    assert_eq!(problem_code(&outcome), Some(ErrorCode::FileNotFound));
    assert_eq!(reads, 0);
    assert_eq!(fixture.count("snapshots"), 1);
    assert_eq!(fixture.count("files"), 1);

    // Put back where it was, it is the same file again and is captured.
    fs::rename(fixture.path("Taxes/list.txt"), fixture.path("todo.txt")).unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
    let view = &fixture.service().memory_watches().unwrap()[0];
    assert_eq!(view.problem, None);
}

#[test]
fn a_move_traeva_made_is_followed() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    let plan = fixture.plan(&file, "Taxes", &[]);
    fixture.apply(&plan.plan_id).unwrap();

    fs::write(fixture.path("Taxes/todo.txt"), "edited in its new folder\n").unwrap();
    let CheckOutcome::Saved { snapshot } = only(fixture.check_memory()) else {
        panic!("expected a new version");
    };
    assert_eq!(snapshot.file_id, file.file_id);
    assert_eq!(snapshot.relative_path, "Taxes/todo.txt");
    assert_eq!(
        fixture.service().memory_watches().unwrap()[0].relative_path,
        "Taxes/todo.txt"
    );
}

#[test]
fn a_revoked_folder_is_not_read() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fixture.service().revoke_root(&fixture.root_id).unwrap();
    fs::write(fixture.path("todo.txt"), "changed after revoking\n").unwrap();
    let outcome = only(fixture.check_memory());
    assert_eq!(problem_code(&outcome), Some(ErrorCode::RootRevoked));
    assert_eq!(fixture.count("snapshots"), 1);
    assert_eq!(
        fixture.start_memory(&file.file_id).unwrap_err().code,
        ErrorCode::RootRevoked
    );
}

#[test]
fn edits_made_while_traeva_was_closed_are_found_at_the_next_start() {
    let mut fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let on = fixture.start_memory(&file.file_id).unwrap();
    fixture.restart();
    fs::write(fixture.path("todo.txt"), "edited while Traeva was closed\n").unwrap();

    let view = &fixture.service().memory_watches().unwrap()[0];
    assert_eq!(
        (view.watch_id.as_str(), view.status),
        (on.watch_id.as_str(), WatchStatus::Active)
    );
    assert_eq!(view.checked_at_ms, None, "not looked at yet in this run");
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
}

#[test]
fn a_file_that_keeps_changing_is_tried_again_at_the_next_pass() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.start_memory(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "half saved\n").unwrap();
    let mut writes = 0;
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            // Changed between the two reads of every attempt.
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
    assert_eq!(fixture.count("snapshots"), 1);
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
}

#[test]
fn watching_writes_nothing_inside_the_root() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let before = tree::snapshot(fixture.root.path());
    fixture.start_memory(&file.file_id).unwrap();
    fixture.check_memory();
    fs::write(fixture.path("todo.txt"), "changed\n").unwrap();
    let changed = tree::snapshot(fixture.root.path());
    fixture.check_memory();
    fixture.check_memory();
    fixture.service().stop_memory(&file.file_id).unwrap();
    assert_eq!(tree::snapshot(fixture.root.path()), changed);
    assert_ne!(before, changed);
}

#[test]
fn bad_and_unknown_files_are_refused() {
    let fixture = Fixture::new();
    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        fixture.start_memory("x").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        fixture.start_memory(&unknown).unwrap_err().code,
        ErrorCode::UnknownFile
    );
    assert_eq!(
        fixture.service().stop_memory(&unknown).unwrap_err().code,
        ErrorCode::UnknownFile
    );
    let file = fixture.register("todo.txt");
    assert_eq!(fixture.service().stop_memory(&file.file_id).unwrap(), None);
    assert_eq!(fixture.count("memory_watches"), 0);
}

#[test]
fn a_file_that_is_already_gone_cannot_be_turned_on() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fs::write(fixture.path("todo.tmp"), "other\n").unwrap();
    fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();
    fixture.register("todo.txt"); // the old record is now missing
    assert_eq!(
        fixture.start_memory(&file.file_id).unwrap_err().code,
        ErrorCode::FileNotFound
    );
    assert_eq!(fixture.count("memory_watches"), 0);
}

#[test]
fn the_database_keeps_one_watch_on_per_file_and_off_is_final() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let on = fixture.start_memory(&file.file_id).unwrap();
    let db = fixture.db();
    let second = db.execute(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
         VALUES ('00000000-0000-4000-8000-000000000001', ?1, 'active', 0)",
        [&file.file_id],
    );
    assert!(second.is_err(), "two watches on for one file");
    fixture.service().stop_memory(&file.file_id).unwrap();
    for change in [
        "UPDATE memory_watches SET status = 'active', stopped_at_ms = NULL",
        "UPDATE memory_watches SET stopped_at_ms = 5",
        "UPDATE memory_watches SET file_id = 'other'",
        "DELETE FROM memory_watches",
    ] {
        assert!(db.execute(change, []).is_err(), "{change}");
    }
    let status: String = db
        .query_row(
            "SELECT status FROM memory_watches WHERE watch_id = ?1",
            [&on.watch_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "stopped");
}

#[test]
fn the_watcher_reads_a_changed_file_once_per_pass() {
    // One attempt (two reads with the pause between them): a file still
    // being saved is tried again at the next pass instead of holding up a
    // plan's run for several attempts.
    assert_eq!(WATCH_READ.max_attempts, 1);
    assert_eq!(WATCH_READ.max_bytes, crate::observe::MAX_FILE_BYTES);
}
