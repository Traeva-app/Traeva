//! Execution tests that need the timing under control: whatever happens at
//! the worst moment of a run (a file put at the target just before the move,
//! an edit after the confirmation, changes right after the move), plus the
//! order of intents, changes and results. Synthetic temporary files only.

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use super::*;
use crate::contracts::{ActionStatus, ErrorCode, PlanLifecycle, PlanStatus};
use crate::observe::sha256_hex;
use crate::plan::PREVIEW_LIFETIME_MS;
use crate::test_fixture::{Fixture, START_MS, edit_keeping_size_and_mtime, quick_read, steps};
use crate::test_snapshot as snapshot;

fn problem(action: &ActionView) -> ErrorCode {
    action.problem.as_ref().expect("a problem").code
}

/// Where the database says the file is, and its revision.
fn recorded(f: &Fixture, file_id: &str) -> (String, i64) {
    f.db()
        .query_row(
            "SELECT relative_path, record_revision FROM files WHERE file_id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

fn organize_entries(f: &Fixture, file_id: &str) -> i64 {
    f.db()
        .query_row(
            "SELECT count(*) FROM path_history WHERE file_id = ?1 AND reason = 'organize'",
            [file_id],
            |r| r.get(0),
        )
        .unwrap()
}

fn settlement(f: &Fixture, action_id: &str) -> Settlement {
    let json: String = f
        .db()
        .query_row(
            "SELECT detail_json FROM action_events WHERE action_id = ?1 AND step = 'settle'",
            [action_id],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&json).unwrap()
}

fn action_of(f: &Fixture, plan_id: &str) -> String {
    f.service().get_plan(plan_id).unwrap().action_id.unwrap()
}

fn not_moved() -> Vec<(String, String)> {
    steps(&[("confirm", "result"), ("settle", "result")])
}

fn move_refused() -> Vec<(String, String)> {
    steps(&[
        ("confirm", "result"),
        ("move", "intent"),
        ("settle", "result"),
    ])
}

// ---------------------------------------------------------------------------
// The order of a successful run
// ---------------------------------------------------------------------------

#[test]
fn every_change_has_a_saved_intent_before_it_and_a_result_after_it() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let inode = fs::metadata(f.path("todo.txt")).unwrap().ino();
    let plan_id = f.plan(&file, "Notes", &["2026 taxes", "Q1"]).plan_id;
    let new_folders = ["Notes/2026 taxes", "Notes/2026 taxes/Q1"];

    let mut reached = Vec::new();
    let action = f
        .apply_with(&plan_id, &mut |point| {
            let action_id = action_of(&f, &plan_id);
            let last = f.journal(&action_id).pop().unwrap();
            match point {
                Checkpoint::BeforeMkdir(n) => {
                    // The intent is saved and the folder does not exist yet.
                    assert_eq!(last, ("mkdir".into(), "intent".into()));
                    assert!(!f.path(new_folders[n]).exists());
                    let status = f.service().get_action(&action_id).unwrap().status;
                    assert_eq!(status, ActionStatus::Running, "saved before any change");
                }
                Checkpoint::AfterMkdir(n) => {
                    // Created, but its receipt waits for the check.
                    assert_eq!(last, ("mkdir".into(), "intent".into()));
                    assert!(f.path(new_folders[n]).is_dir());
                }
                Checkpoint::BeforeMove => {
                    assert_eq!(last, ("move".into(), "intent".into()));
                    assert!(f.path("todo.txt").exists());
                }
                Checkpoint::BeforeMoveCheck | Checkpoint::AfterMove => {}
            }
            reached.push(point);
        })
        .unwrap();

    assert_eq!(
        reached,
        [
            Checkpoint::BeforeMkdir(0),
            Checkpoint::AfterMkdir(0),
            Checkpoint::BeforeMkdir(1),
            Checkpoint::AfterMkdir(1),
            Checkpoint::BeforeMoveCheck,
            Checkpoint::BeforeMove,
            Checkpoint::AfterMove,
        ]
    );
    assert_eq!(action.status, ActionStatus::Committed);
    assert!(action.problem.is_none() && action.committed_at_ms.is_some());
    assert_eq!(action.folders_created, new_folders);
    assert_eq!(
        f.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("move", "intent"),
            ("move", "result"),
        ])
    );
    // Each receipt is the folder that is really there.
    let journal = f
        .service()
        .store()
        .unwrap()
        .journal(&action.action_id)
        .unwrap();
    let receipts: Vec<_> = journal
        .iter()
        .filter_map(|row| match &row.entry {
            Some(Entry::FolderReceipt(receipt)) => Some(receipt),
            _ => None,
        })
        .collect();
    assert_eq!(receipts.len(), 2);
    for receipt in receipts {
        let on_disk = fs::metadata(f.path(&receipt.path)).unwrap();
        assert_eq!(
            (receipt.dev, receipt.ino),
            (on_disk.dev() as i64, on_disk.ino())
        );
    }
    let moved = f.path("Notes/2026 taxes/Q1/todo.txt");
    assert_eq!(sha256_hex(&fs::read(&moved).unwrap()), file.sha256);
    assert_eq!(
        fs::metadata(&moved).unwrap().ino(),
        inode,
        "moved, not copied"
    );
    assert_eq!(
        recorded(&f, &file.file_id),
        (
            "Notes/2026 taxes/Q1/todo.txt".to_string(),
            file.file_revision + 1
        )
    );
    assert_eq!(organize_entries(&f, &file.file_id), 1);
}

#[test]
fn a_successful_move_call_is_not_yet_a_success() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let mut during = None;
    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                let view = f.service().get_action(&action_of(&f, &plan_id)).unwrap();
                during = Some((view, recorded(&f, &file.file_id)));
            }
        })
        .unwrap();

    let (view, record) = during.unwrap();
    assert_eq!(
        (view.status, view.committed_at_ms),
        (ActionStatus::Running, None)
    );
    assert_eq!(record.0, "todo.txt", "not recorded as moved yet");
    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(
        f.service().get_plan(&plan_id).unwrap().status,
        PlanStatus::Consumed
    );
}

// ---------------------------------------------------------------------------
// Before the move: enough evidence that nothing moved → failed_no_move
// ---------------------------------------------------------------------------

#[test]
fn a_file_put_at_the_target_just_before_the_move_is_never_replaced() {
    // The same name, another case, and another Unicode spelling (NFD).
    for (name, late) in [
        ("todo.txt", "todo.txt"),
        ("todo.txt", "TODO.TXT"),
        ("caf\u{e9}.txt", "cafe\u{301}.txt"),
    ] {
        let f = Fixture::new();
        fs::write(f.path("caf\u{e9}.txt"), "coffee\n").unwrap();
        let file = f.register(name);
        let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
        let late_path = f.path("Taxes").join(late);

        let action = f
            .apply_with(&plan_id, &mut |point| {
                if point == Checkpoint::BeforeMove {
                    fs::write(&late_path, "someone else's file").unwrap();
                }
            })
            .unwrap();

        if fs::read_dir(f.path("Taxes")).unwrap().count() == 2 {
            // A case- or normalization-sensitive volume: two different names.
            assert_eq!(action.status, ActionStatus::Committed, "{late}");
            continue;
        }
        assert_eq!(
            (action.status, problem(&action)),
            (ActionStatus::FailedNoMove, ErrorCode::TargetConflict),
            "{late}"
        );
        assert_eq!(fs::read(&late_path).unwrap(), b"someone else's file");
        assert_eq!(
            sha256_hex(&fs::read(f.path(name)).unwrap()),
            file.sha256,
            "the file stays where it was"
        );
        assert_eq!(f.journal(&action.action_id), move_refused());
        let found = settlement(&f, &action.action_id);
        assert_eq!(
            (found.source_now, found.target_now),
            (Seen::ExpectedFile, Seen::SomethingElse)
        );
        assert_eq!(recorded(&f, &file.file_id).0, name);
        // The plan is used: asking again returns the same failed action.
        assert_eq!(f.apply(&plan_id).unwrap(), action);
        assert_eq!(f.count("actions"), 1);
        // A failed_no_move action is settled, so a new plan can be made.
        assert!(f.plan(&file, "Notes", &[]).can_confirm);
    }
}

#[test]
fn a_folder_someone_else_creates_is_not_claimed() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive", "2026"]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMkdir(1) {
                fs::create_dir(f.path("Notes/Archive/2026")).unwrap();
                fs::write(f.path("Notes/Archive/2026/theirs.txt"), "theirs").unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::TargetConflict)
    );
    assert_eq!(
        action.folders_created,
        ["Notes/Archive"],
        "only its own folder"
    );
    assert_eq!(
        f.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("mkdir", "intent"),
            ("settle", "result"),
        ])
    );
    // Nothing is removed or moved: the new folder stays, the other one too.
    assert!(f.path("Notes/Archive").is_dir());
    assert_eq!(
        fs::read(f.path("Notes/Archive/2026/theirs.txt")).unwrap(),
        b"theirs"
    );
    assert!(f.path("todo.txt").exists());
}

#[test]
fn a_new_folder_swapped_right_after_mkdir_is_not_claimed() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive"]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMkdir(0) {
                fs::rename(f.path("Notes/Archive"), f.path("Notes/Archive-moved")).unwrap();
                fs::create_dir(f.path("Notes/Archive")).unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::StalePlan)
    );
    assert!(
        action.folders_created.is_empty(),
        "no receipt without a passed check"
    );
    assert_eq!(
        f.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("settle", "result"),
        ])
    );
    assert!(f.path("Notes/Archive-moved").is_dir() && f.path("Notes/Archive").is_dir());
    assert!(f.path("todo.txt").exists());
}

#[test]
fn an_edit_after_the_confirmation_stops_the_move() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let path = f.path("todo.txt");
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMoveCheck {
                // Same length, and the old modification time put back.
                fs::write(&path, "buy mild\n").unwrap();
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(mtime)
                    .unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::FileChanged)
    );
    assert_eq!(f.journal(&action.action_id), not_moved());
    assert_eq!(fs::read(&path).unwrap(), b"buy mild\n");
    assert_eq!(fs::read_dir(f.path("Taxes")).unwrap().count(), 0);
}

#[test]
fn a_folder_replaced_after_the_confirmation_stops_the_move() {
    // The target folder.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMoveCheck {
                fs::rename(f.path("Taxes"), f.path("Taxes-old")).unwrap();
                fs::create_dir(f.path("Taxes")).unwrap();
            }
        })
        .unwrap();
    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::StalePlan)
    );
    assert!(f.path("todo.txt").exists());
    assert_eq!(fs::read_dir(f.path("Taxes-old")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(f.path("Taxes")).unwrap().count(), 0);

    // A folder above the file.
    let f = Fixture::new();
    let file = f.register("Notes/2026/plan.md");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMoveCheck {
                fs::rename(f.path("Notes"), f.path("Notes-old")).unwrap();
                fs::create_dir_all(f.path("Notes/2026")).unwrap();
                fs::copy(
                    f.path("Notes-old/2026/plan.md"),
                    f.path("Notes/2026/plan.md"),
                )
                .unwrap();
            }
        })
        .unwrap();
    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::FileChanged)
    );
    assert!(f.path("Notes-old/2026/plan.md").exists());
    assert_eq!(fs::read_dir(f.path("Taxes")).unwrap().count(), 0);
}

#[test]
fn a_move_the_system_refuses_is_failed_no_move_after_looking_at_both_names() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let taxes = f.path("Taxes");

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMove {
                // Nothing may be added to the target folder any more.
                fs::set_permissions(&taxes, fs::Permissions::from_mode(0o555)).unwrap();
            }
        })
        .unwrap();
    fs::set_permissions(&taxes, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::OperationRefused)
    );
    let found = settlement(&f, &action.action_id);
    assert_eq!(
        (found.source_now, found.target_now),
        (Seen::ExpectedFile, Seen::Nothing)
    );
    assert_eq!(f.journal(&action.action_id), move_refused());
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        file.sha256
    );
}

#[test]
fn a_failed_move_with_the_file_gone_from_its_old_name_needs_a_decision() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMove {
                fs::rename(f.path("todo.txt"), f.path("todo-elsewhere.txt")).unwrap();
            }
        })
        .unwrap();

    // The move call failed, but the file is not at its old name either:
    // not enough evidence for "not moved".
    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    let found = settlement(&f, &action.action_id);
    assert_eq!(
        (found.source_now, found.target_now),
        (Seen::Nothing, Seen::Nothing)
    );
    assert!(f.path("todo-elsewhere.txt").exists(), "left where it is");
}

#[test]
fn revoking_a_root_during_a_run_is_noticed_before_and_after_the_move() {
    for point in [Checkpoint::BeforeMoveCheck, Checkpoint::AfterMove] {
        let f = Fixture::new();
        let file = f.register("todo.txt");
        let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
        let action = f
            .apply_with(&plan_id, &mut |now| {
                if now == point {
                    f.service().revoke_root(&f.root_id).unwrap();
                }
            })
            .unwrap();
        if point == Checkpoint::BeforeMoveCheck {
            assert_eq!(
                (action.status, problem(&action)),
                (ActionStatus::FailedNoMove, ErrorCode::RootRevoked)
            );
            assert!(f.path("todo.txt").exists());
        } else {
            assert_eq!(
                (action.status, problem(&action)),
                (
                    ActionStatus::NeedsUserDecision,
                    ErrorCode::ResultUnconfirmed
                )
            );
            assert!(f.path("Taxes/todo.txt").exists(), "not moved back");
        }
    }
}

#[test]
fn an_expired_plan_creates_no_action_and_changes_nothing() {
    let f = Fixture::new();
    let before = snapshot::snapshot(f.root.path());
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    f.advance(PREVIEW_LIFETIME_MS);

    assert_eq!(f.apply(&plan_id).unwrap_err().code, ErrorCode::StalePlan);
    assert_eq!(f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

// ---------------------------------------------------------------------------
// After the move: uncertain → needs_user_decision, nothing moves back
// ---------------------------------------------------------------------------

#[test]
fn a_change_right_after_the_move_needs_a_decision_and_nothing_moves_back() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                // Same object, same length, old modification time: only the
                // full hash shows the change.
                let moved = f.path("Taxes/todo.txt");
                let mtime = fs::metadata(&moved).unwrap().modified().unwrap();
                fs::write(&moved, "buy mild\n").unwrap();
                fs::File::options()
                    .write(true)
                    .open(&moved)
                    .unwrap()
                    .set_modified(mtime)
                    .unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    assert!(!f.path("todo.txt").exists(), "not moved back");
    assert_eq!(fs::read(f.path("Taxes/todo.txt")).unwrap(), b"buy mild\n");
    // Not recorded as moved: the record and path history are unchanged.
    assert_eq!(
        recorded(&f, &file.file_id),
        ("todo.txt".to_string(), file.file_revision)
    );
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(f.journal(&action.action_id), move_refused());
    // Asking again reports the same action, and the file is blocked.
    assert_eq!(f.apply(&plan_id).unwrap(), action);
    assert_eq!(f.count("actions"), 1);
    let error = f.try_plan(&file, "Notes", &[]).unwrap_err();
    assert_eq!(
        (error.code, error.action_id),
        (ErrorCode::UnresolvedAction, Some(action.action_id))
    );
}

#[test]
fn the_same_bytes_in_another_file_after_the_move_need_a_decision() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                // A copy with exactly the same bytes takes the moved file's name.
                fs::rename(f.path("Taxes/todo.txt"), f.path("Taxes/todo.bak")).unwrap();
                fs::copy(f.path("Taxes/todo.bak"), f.path("Taxes/todo.txt")).unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    let found = settlement(&f, &action.action_id);
    assert_eq!(
        found.target_now,
        Seen::SomethingElse,
        "same hash, other file"
    );
    assert_eq!(organize_entries(&f, &file.file_id), 0);
}

/// Applies `plan_id` and runs `meanwhile` at the end of the final read of
/// the moved file: the bytes were read and were right, then something
/// changes before the result is recorded.
fn apply_changing_during_final_read(
    f: &Fixture,
    plan_id: &str,
    meanwhile: &dyn Fn(),
) -> ActionView {
    let moved = Cell::new(false);
    f.service()
        .apply_plan_with(
            plan_id,
            &mut |parent, name, dev| {
                let read = quick_read(parent, name, dev);
                if moved.get() {
                    meanwhile();
                }
                read
            },
            &mut |point| {
                if point == Checkpoint::AfterMove {
                    moved.set(true);
                }
            },
        )
        .unwrap()
}

#[test]
fn a_folder_leaving_the_root_during_the_final_read_needs_a_decision() {
    // #46: the target folder is moved out of the root while the moved file
    // is read. The read still works (it holds the folder), but the file is
    // no longer where the record would say.
    let f = Fixture::new();
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = apply_changing_during_final_read(&f, &plan_id, &|| {
        fs::rename(f.path("Taxes"), outside.path().join("Taxes")).unwrap();
    });

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    // On disk: the file went with its folder; nothing was moved back.
    assert!(!f.path("todo.txt").exists());
    assert!(!f.path("Taxes").exists());
    assert_eq!(
        fs::read(outside.path().join("Taxes/todo.txt")).unwrap(),
        b"buy milk\n"
    );
    // In the database: no success of any kind.
    assert_eq!(
        recorded(&f, &file.file_id),
        ("todo.txt".to_string(), file.file_revision)
    );
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(f.journal(&action.action_id), move_refused());
    assert_eq!(action.committed_at_ms, None);
}

#[test]
fn a_new_file_at_the_old_name_during_the_final_read_needs_a_decision() {
    // #46: something takes the old name while the moved file is read.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = apply_changing_during_final_read(&f, &plan_id, &|| {
        fs::write(f.path("todo.txt"), "another file\n").unwrap();
    });

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    assert_eq!(fs::read(f.path("todo.txt")).unwrap(), b"another file\n");
    assert_eq!(fs::read(f.path("Taxes/todo.txt")).unwrap(), b"buy milk\n");
    let found = settlement(&f, &action.action_id);
    assert_eq!(found.source_now, Seen::SomethingElse);
    assert_eq!(
        recorded(&f, &file.file_id),
        ("todo.txt".to_string(), file.file_revision)
    );
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(f.journal(&action.action_id), move_refused());
}

#[test]
fn a_root_moved_away_during_the_final_read_needs_a_decision() {
    // #46: the authorized folder itself is renamed while the file is read.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let renamed = f.root.path().with_extension("renamed");

    let action = apply_changing_during_final_read(&f, &plan_id, &|| {
        fs::rename(f.root.path(), &renamed).unwrap();
    });
    // Put the root back before the fixture cleans up.
    fs::rename(&renamed, f.root.path()).unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(
        recorded(&f, &file.file_id),
        ("todo.txt".to_string(), file.file_revision)
    );
}

#[test]
fn an_edit_right_after_the_final_read_needs_a_decision() {
    // #54: the moved file was read and its bytes were right, then it is
    // edited in place before the result is recorded. Same file object, same
    // place, same size and modification time: only its change time tells,
    // and the hash just read is stale.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = apply_changing_during_final_read(&f, &plan_id, &|| {
        edit_keeping_size_and_mtime(&f.path("Taxes/todo.txt"), "pay rent\n");
    });

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    let message = &action.problem.as_ref().unwrap().message;
    assert!(
        message.contains("changed after Traeva read it"),
        "{message}"
    );
    // On disk: the edited file stays at the target; nothing was moved back.
    assert!(!f.path("todo.txt").exists());
    assert_eq!(fs::read(f.path("Taxes/todo.txt")).unwrap(), b"pay rent\n");
    // In the database: no success of any kind.
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(
        recorded(&f, &file.file_id),
        ("todo.txt".to_string(), file.file_revision)
    );
}

#[test]
fn a_new_file_at_the_old_name_after_the_move_needs_a_decision() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                fs::write(f.path("todo.txt"), "a new list\n").unwrap();
            }
        })
        .unwrap();

    assert_eq!(action.status, ActionStatus::NeedsUserDecision);
    let found = settlement(&f, &action.action_id);
    assert_eq!(
        (found.source_now, found.target_now),
        (Seen::SomethingElse, Seen::ExpectedFile),
        "what was found at both names is recorded"
    );
    assert_eq!(fs::read(f.path("todo.txt")).unwrap(), b"a new list\n");
    assert_eq!(
        sha256_hex(&fs::read(f.path("Taxes/todo.txt")).unwrap()),
        file.sha256
    );
}

#[test]
fn a_folder_moved_during_the_move_needs_a_decision() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                fs::rename(f.path("Taxes"), f.path("Taxes-moved")).unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    assert!(f.path("Taxes-moved/todo.txt").exists(), "left where it is");
    assert!(!f.path("todo.txt").exists());
}

// ---------------------------------------------------------------------------
// A file with an unsettled action
// ---------------------------------------------------------------------------

#[test]
fn an_unsettled_action_blocks_new_plans_and_runs_for_its_file() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let older = f.plan(&file, "Notes", &[]).plan_id;
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id; // supersedes `older`
    for status in ["prepared", "running", "needs_user_decision"] {
        f.db()
            .execute("DELETE FROM actions WHERE plan_id = ?1", [&older])
            .unwrap();
        let busy = f.insert_action(&older, status);

        let error = f.apply(&plan_id).unwrap_err();
        assert_eq!(
            (error.code, error.action_id.as_deref()),
            (ErrorCode::UnresolvedAction, Some(busy.as_str())),
            "{status}"
        );
        assert_eq!(
            f.try_plan(&file, "Taxes", &[]).unwrap_err().code,
            ErrorCode::UnresolvedAction
        );
        assert_eq!(f.count("actions"), 1);
        assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Open);
        assert!(f.path("todo.txt").exists());
    }

    // The unsettled action is the reason given, even when the file changed
    // too, and the plan is left as it is.
    fs::write(f.path("todo.txt"), "buy milk and eggs\n").unwrap();
    assert_eq!(
        f.apply(&plan_id).unwrap_err().code,
        ErrorCode::UnresolvedAction
    );
    assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Open);
    fs::write(f.path("todo.txt"), "buy milk\n").unwrap();

    // Once that action is settled, the file can move again.
    f.db()
        .execute(
            "UPDATE actions SET status = 'failed_no_move' WHERE plan_id = ?1",
            [&older],
        )
        .unwrap();
    assert_eq!(f.apply(&plan_id).unwrap().status, ActionStatus::Committed);
}

#[test]
fn the_confirmation_checks_again_inside_its_transaction() {
    // The run lock and #8's checks stop these cases earlier. The transaction
    // checks once more, so the database alone also refuses them.
    let f = Fixture::new();
    let todo = f.register("todo.txt");
    let plan_md = f.register("Notes/2026/plan.md");
    let first = f.plan(&todo, "Taxes", &[]).plan_id;
    let saved = match f.revalidate(&first).unwrap() {
        Revalidation::Ready(ready) => preconditions(&ready),
        other => panic!("{other:?}"),
    };
    let store = f.service().store().unwrap();
    let confirm = |plan_id: &str, action_id: &str| {
        store
            .confirm_plan(&f.stored(plan_id), action_id, &saved, START_MS)
            .unwrap()
    };
    let new_id = || uuid::Uuid::new_v4().to_string();

    // A closed plan.
    store
        .close_plan(&first, PlanLifecycle::Cancelled, START_MS, "test")
        .unwrap();
    assert_eq!(confirm(&first, &new_id()), Confirmation::PlanClosed);

    // A file with an action that is not settled.
    let second = f.plan(&todo, "Notes", &[]).plan_id;
    let busy = f.insert_action(&first, "running");
    assert_eq!(
        confirm(&second, &new_id()),
        Confirmation::FileBusy(busy.clone())
    );

    // Settled: the action is created once, and only once.
    f.db()
        .execute(
            "UPDATE actions SET status = 'failed_no_move' WHERE action_id = ?1",
            [&busy],
        )
        .unwrap();
    let created = new_id();
    assert_eq!(confirm(&second, &created), Confirmation::Created);
    assert_eq!(
        confirm(&second, &new_id()),
        Confirmation::AlreadyUsed(created.clone())
    );

    // A revoked root.
    let third = f.plan(&plan_md, "Taxes", &[]).plan_id;
    f.service().revoke_root(&f.root_id).unwrap();
    assert_eq!(confirm(&third, &new_id()), Confirmation::RootRevoked);

    // Only the one created action wrote anything.
    assert_eq!(f.count("actions"), 2);
    assert_eq!(f.journal(&created), steps(&[("confirm", "result")]));
    assert_eq!(f.count("action_events"), 1);
}

// ---------------------------------------------------------------------------
// One run at a time
// ---------------------------------------------------------------------------

#[test]
fn a_second_click_during_a_run_returns_at_once_with_the_running_action() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let mut answer = None;
    // The scope ends after the run, so a click that did wait is collected
    // only once the run let go of the lock.
    let action = thread::scope(|scope| {
        f.apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                // This run holds the lock. Another click must not wait for it.
                let (tx, rx) = mpsc::channel();
                let service = f.service();
                let id = plan_id.as_str();
                scope.spawn(move || {
                    let _ = tx.send(service.apply_plan_with(id, &mut quick_read, &mut |_| {}));
                });
                answer = Some(rx.recv_timeout(Duration::from_secs(2)));
            }
        })
        .unwrap()
    });

    let during = answer
        .unwrap()
        .expect("answered at once")
        .expect("the action, not an error");
    assert_eq!(during.action_id, action.action_id);
    assert_eq!(during.status, ActionStatus::Running);
    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(f.count("actions"), 1);
}

#[test]
fn a_click_that_waited_for_the_same_plan_gets_its_action_whatever_the_result() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let (go, started) = mpsc::channel();

    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(|| {
            let mut first_read = true;
            f.service().apply_plan_with(
                &plan_id,
                &mut |parent, name, dev| {
                    if first_read {
                        // Inside the run, before the confirmation: the second
                        // click starts now and has to wait for this run.
                        first_read = false;
                        go.send(()).unwrap();
                        thread::sleep(Duration::from_millis(200));
                    }
                    quick_read(parent, name, dev)
                },
                &mut |point| {
                    if point == Checkpoint::AfterMove {
                        fs::write(f.path("Taxes/todo.txt"), "buy mild\n").unwrap();
                    }
                },
            )
        });
        started.recv().unwrap();
        let second = scope.spawn(|| f.apply(&plan_id));
        (first.join().unwrap().unwrap(), second.join().unwrap())
    });

    assert_eq!(first.status, ActionStatus::NeedsUserDecision);
    let second = second.expect("the same action, not an error");
    assert_eq!(second.action_id, first.action_id);
    assert_eq!(f.count("actions"), 1);
}

#[test]
fn runs_of_different_plans_never_overlap() {
    let f = Fixture::new();
    let todo = f.register("todo.txt");
    let plan_md = f.register("Notes/2026/plan.md");
    let first = f.plan(&todo, "Taxes", &[]).plan_id;
    let second = f.plan(&plan_md, "Taxes", &[]).plan_id;
    let log = Mutex::new(Vec::new());
    let (go, started) = mpsc::channel();

    thread::scope(|scope| {
        let a = scope.spawn(|| {
            f.apply_with(&first, &mut |point| {
                log.lock().unwrap().push("first");
                if point == Checkpoint::BeforeMoveCheck {
                    // The second run is asked for while this one is half done.
                    go.send(()).unwrap();
                    thread::sleep(Duration::from_millis(200));
                }
            })
        });
        started.recv().unwrap();
        let b = scope.spawn(|| f.apply_with(&second, &mut |_| log.lock().unwrap().push("second")));
        assert_eq!(a.join().unwrap().unwrap().status, ActionStatus::Committed);
        assert_eq!(b.join().unwrap().unwrap().status, ActionStatus::Committed);
    });

    // Every step of the first run comes before any step of the second.
    assert_eq!(
        log.into_inner().unwrap(),
        ["first", "first", "first", "second", "second", "second"]
    );
}

// ---------------------------------------------------------------------------
// #20: more changes at the worst moment
// ---------------------------------------------------------------------------

/// Changes after the preview that the check at confirmation must catch: no
/// action is created and nothing on disk changes (O07, O08).
#[test]
fn a_file_changed_after_the_preview_is_refused_at_the_confirmation() {
    type Change = fn(&Fixture);
    let changes: [(&str, Change, ErrorCode); 3] = [
        (
            "the same length, the old modification time",
            |f| edit_keeping_size_and_mtime(&f.path("todo.txt"), "buy mild\n"),
            ErrorCode::FileChanged,
        ),
        (
            "renamed",
            |f| fs::rename(f.path("todo.txt"), f.path("list.txt")).unwrap(),
            ErrorCode::FileChanged,
        ),
        (
            "replaced by a copy with the same bytes",
            |f| {
                fs::copy(f.path("todo.txt"), f.path("copy.tmp")).unwrap();
                fs::rename(f.path("copy.tmp"), f.path("todo.txt")).unwrap();
            },
            ErrorCode::FileChanged,
        ),
    ];
    for (what, change, code) in changes {
        let f = Fixture::new();
        let file = f.register("todo.txt");
        let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
        change(&f);
        let before = snapshot::snapshot(f.root.path());

        let error = f.apply(&plan_id).unwrap_err();

        assert_eq!(error.code, code, "{what}: {error:?}");
        assert_eq!(f.count("actions"), 0, "{what}");
        assert_eq!(snapshot::snapshot(f.root.path()), before, "{what}");
        assert_eq!(recorded(&f, &file.file_id).0, "todo.txt", "{what}");
    }
}

/// A file that grew past 20 MiB after the preview is not moved (O26). Its
/// size changed, so the check reports a changed file; registering it again
/// then says it is too large.
#[test]
fn a_file_that_grew_past_20_mib_after_the_preview_is_not_moved() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    // The same file object, now one byte over the limit.
    fs::File::options()
        .write(true)
        .open(f.path("todo.txt"))
        .unwrap()
        .set_len(crate::observe::MAX_FILE_BYTES + 1)
        .unwrap();
    let before = snapshot::snapshot(f.root.path());

    let error = f.apply(&plan_id).unwrap_err();

    assert_eq!(error.code, ErrorCode::FileChanged, "{error:?}");
    assert_eq!(f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(f.root.path()), before);

    // Growing after the confirmation is caught by the check before the move.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMoveCheck {
                fs::File::options()
                    .write(true)
                    .open(f.path("todo.txt"))
                    .unwrap()
                    .set_len(crate::observe::MAX_FILE_BYTES + 1)
                    .unwrap();
            }
        })
        .unwrap();
    assert_eq!(action.status, ActionStatus::FailedNoMove);
    assert_eq!(f.journal(&action.action_id), not_moved());
    assert!(f.path("todo.txt").exists());
    assert_eq!(fs::read_dir(f.path("Taxes")).unwrap().count(), 0);
}

/// A second name (hard link) for the file, after the preview or after the
/// confirmation: never moved (O11).
#[test]
fn a_file_that_gets_a_second_name_is_not_moved() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    fs::hard_link(f.path("todo.txt"), f.path("Notes/todo-link.txt")).unwrap();
    let before = snapshot::snapshot(f.root.path());

    let error = f.apply(&plan_id).unwrap_err();

    // The link count is part of what was observed.
    assert_eq!(error.code, ErrorCode::FileChanged, "{error:?}");
    assert_eq!(f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(f.root.path()), before);

    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMoveCheck {
                fs::hard_link(f.path("todo.txt"), f.path("Notes/todo-link.txt")).unwrap();
            }
        })
        .unwrap();
    assert_eq!(action.status, ActionStatus::FailedNoMove);
    assert_eq!(f.journal(&action.action_id), not_moved());
    assert!(f.path("todo.txt").exists());
    assert_eq!(fs::read_dir(f.path("Taxes")).unwrap().count(), 0);
}

/// The target folder replaced by a symbolic link to a folder outside the
/// root after the confirmation: nothing goes outside (O10).
#[test]
fn a_target_folder_turned_into_a_link_to_outside_stops_the_move() {
    let outside = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMoveCheck {
                fs::rename(f.path("Taxes"), f.path("Taxes-real")).unwrap();
                std::os::unix::fs::symlink(outside.path(), f.path("Taxes")).unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::StalePlan)
    );
    assert_eq!(f.journal(&action.action_id), not_moved());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(f.path("Taxes-real")).unwrap().count(), 0);
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        file.sha256
    );
    assert_eq!(recorded(&f, &file.file_id).0, "todo.txt");
}

/// The target folder moved out of the root between the last check and the
/// move call. The move follows the folder's handle, so the file can end up
/// outside: organize-spec.md §2.1 accepts this race but requires that it is
/// noticed, never reported as a success and never moved back (O10, O19).
#[test]
fn a_target_folder_moved_out_of_the_root_just_before_the_move_needs_a_decision() {
    let outside = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMove {
                fs::rename(f.path("Taxes"), outside.path().join("Taxes")).unwrap();
            }
        })
        .unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (
            ActionStatus::NeedsUserDecision,
            ErrorCode::ResultUnconfirmed
        )
    );
    // Left where it is, and the records still show the old place.
    let outside_file = outside.path().join("Taxes/todo.txt");
    assert_eq!(sha256_hex(&fs::read(&outside_file).unwrap()), file.sha256);
    assert!(!f.path("todo.txt").exists());
    assert_eq!(recorded(&f, &file.file_id).0, "todo.txt");
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    // Asking again only reports.
    assert_eq!(f.apply(&plan_id).unwrap(), action);
    assert_eq!(f.count("actions"), 1);
}

/// macOS refuses the second new folder: the first one keeps its receipt and
/// stays, the file does not move (O17).
#[test]
fn a_new_folder_the_system_refuses_ends_the_run_without_a_move() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive", "Q1"]).plan_id;
    let archive = f.path("Notes/Archive");

    let action = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMkdir(1) {
                fs::set_permissions(&archive, fs::Permissions::from_mode(0o555)).unwrap();
            }
        })
        .unwrap();
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        (action.status, problem(&action)),
        (ActionStatus::FailedNoMove, ErrorCode::OperationRefused)
    );
    assert_eq!(action.folders_created, ["Notes/Archive"]);
    assert!(action.folders_removed.is_empty());
    // Its intent has no result, so it is listed as a folder without a
    // receipt, although macOS refused to create it.
    assert_eq!(action.folders_without_receipt, ["Notes/Archive/Q1"]);
    assert!(archive.is_dir() && !f.path("Notes/Archive/Q1").exists());
    assert_eq!(
        f.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("mkdir", "intent"),
            ("settle", "result"),
        ])
    );
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        file.sha256
    );
    assert_eq!(recorded(&f, &file.file_id).0, "todo.txt");
}
