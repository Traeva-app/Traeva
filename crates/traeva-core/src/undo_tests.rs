//! Undo tests: the normal round trip, every conflict, repeats, cancelling,
//! an interrupted undo (O18-R: service restarted on the same database), and
//! cleaning up the organize action's folders. Synthetic temporary files only.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::*;
use crate::contracts::{ActionView, PlanStatus, PlanView, StorageStatus};
use crate::execute::Checkpoint;
use crate::observe::sha256_hex;
use crate::test_fixture::{Fixture, steps};
use crate::test_snapshot as snapshot;

/// Organizes `file` into `folder` (plus `new_folders`) and returns the action.
fn organize(f: &Fixture, file: &str, folder: &str, new_folders: &[&str]) -> ActionView {
    let view = f.register(file);
    let plan_id = f.plan(&view, folder, new_folders).plan_id;
    let action = f.apply(&plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    action
}

fn undo_plan(f: &Fixture, action_id: &str) -> PlanView {
    match f.service().preview_undo(action_id).unwrap() {
        UndoPreview::Planned { plan } => plan,
        other => panic!("expected an undo plan, got {other:?}"),
    }
}

fn conflict(f: &Fixture, action_id: &str) -> AppError {
    let error = f.service().preview_undo(action_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::UndoConflict, "{error:?}");
    error
}

/// Runs the plan until `point` and stops it there (O18-R: the process goes
/// on; the real process kill is O18-C, #21).
fn stop_at(f: &Fixture, plan_id: &str, point: Checkpoint) -> String {
    let stopped = catch_unwind(AssertUnwindSafe(|| {
        f.apply_with(plan_id, &mut |now| {
            if now == point {
                resume_unwind(Box::new("stopped by the test"));
            }
        })
    }));
    assert!(stopped.is_err(), "the run should stop at {point:?}");
    f.service().get_plan(plan_id).unwrap().action_id.unwrap()
}

fn history(f: &Fixture, file_id: &str) -> Vec<(String, String, Option<String>)> {
    let db = f.db();
    let mut stmt = db
        .prepare(
            "SELECT reason, to_relative_path, action_id FROM path_history
             WHERE file_id = ?1 ORDER BY recorded_at_ms, reason",
        )
        .unwrap();
    stmt.query_map([file_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn recorded(f: &Fixture, file_id: &str) -> (String, i64) {
    f.db()
        .query_row(
            "SELECT relative_path, record_revision FROM files WHERE file_id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

// ---------------------------------------------------------------------------
// The round trip
// ---------------------------------------------------------------------------

#[test]
fn an_organized_file_goes_back_exactly_and_its_new_folders_go_away() {
    let f = Fixture::new();
    let original_sha = sha256_hex(&fs::read(f.path("todo.txt")).unwrap());
    let inode = fs::metadata(f.path("todo.txt")).unwrap().ino();
    let organized = organize(&f, "todo.txt", "Notes", &["2026 taxes", "Q1"]);
    let revision = recorded(&f, &organized.file_id).1;
    assert_eq!(
        (organized.undo_status, organized.undo_action_id.clone()),
        (UndoStatus::Available, None)
    );

    // The preview: exactly where it goes, and nothing written yet.
    let before = snapshot::snapshot(f.root.path());
    let plan = undo_plan(&f, &organized.action_id);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
    assert_eq!(plan.kind, PlanKind::Undo);
    assert_eq!(
        plan.undo_of_action_id.as_deref(),
        Some(organized.action_id.as_str())
    );
    assert_eq!(plan.source.relative_path, "Notes/2026 taxes/Q1/todo.txt");
    assert_eq!(plan.target.relative_path, "todo.txt");
    assert!(plan.folders_to_create.is_empty());
    assert!(plan.can_confirm);

    let undo = f.apply(&plan.plan_id).unwrap();

    assert_eq!(undo.status, ActionStatus::Committed);
    assert_eq!(undo.kind, PlanKind::Undo);
    assert_eq!(
        undo.undo_of_action_id.as_deref(),
        Some(organized.action_id.as_str())
    );
    // The same file, the same bytes, back at its old name.
    assert!(!f.path("Notes/2026 taxes/Q1/todo.txt").exists());
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        original_sha
    );
    assert_eq!(fs::metadata(f.path("todo.txt")).unwrap().ino(), inode);
    // The records: the same file_id, the original undone, one undo entry.
    assert_eq!(
        recorded(&f, &organized.file_id),
        ("todo.txt".to_string(), revision + 1)
    );
    let original = f.service().get_action(&organized.action_id).unwrap();
    assert_eq!(
        (original.status, original.undo_status),
        (ActionStatus::Undone, UndoStatus::Undone)
    );
    assert_eq!(
        original.undo_action_id.as_deref(),
        Some(undo.action_id.as_str())
    );
    assert_eq!(
        history(&f, &organized.file_id),
        [
            ("intake".to_string(), "todo.txt".to_string(), None),
            (
                "organize".to_string(),
                "Notes/2026 taxes/Q1/todo.txt".to_string(),
                Some(organized.action_id.clone())
            ),
            (
                "undo".to_string(),
                "todo.txt".to_string(),
                Some(undo.action_id.clone())
            ),
        ]
    );
    // The folders the organize action created are gone again, innermost first.
    assert!(!f.path("Notes/2026 taxes").exists());
    assert_eq!(
        undo.folders_removed,
        ["Notes/2026 taxes/Q1", "Notes/2026 taxes"]
    );
    assert!(undo.folders_kept.is_empty(), "no cleanup warning");
    assert_eq!(
        f.journal(&undo.action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("move", "result"),
            ("cleanup", "intent"),
            ("cleanup", "result"),
            ("cleanup", "intent"),
            ("cleanup", "result"),
        ])
    );
}

#[test]
fn undo_works_after_a_restart() {
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    f.restart();

    let plan = undo_plan(&f, &organized.action_id);
    let undo = f.apply(&plan.plan_id).unwrap();

    assert_eq!(undo.status, ActionStatus::Committed);
    assert!(f.path("todo.txt").exists());
    assert!(
        f.path("Taxes").is_dir(),
        "a folder that existed before stays"
    );
}

// ---------------------------------------------------------------------------
// Conflicts: nothing moves, no plan is made
// ---------------------------------------------------------------------------

#[test]
fn an_edited_moved_or_replaced_file_cannot_be_undone() {
    // Edited: same object, same length, old modification time.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let moved = f.path("Taxes/todo.txt");
    let mtime = fs::metadata(&moved).unwrap().modified().unwrap();
    fs::write(&moved, "buy mild\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&moved)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert!(
        conflict(&f, &organized.action_id)
            .message
            .contains("edited")
    );

    // Moved away.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    fs::rename(f.path("Taxes/todo.txt"), f.path("Notes/todo.txt")).unwrap();
    conflict(&f, &organized.action_id);

    // Replaced by a copy with exactly the same bytes.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    fs::rename(f.path("Taxes/todo.txt"), f.path("Taxes/todo.bak")).unwrap();
    fs::copy(f.path("Taxes/todo.bak"), f.path("Taxes/todo.txt")).unwrap();
    let error = conflict(&f, &organized.action_id);
    assert!(error.message.contains("same content"));
    assert_eq!(f.count("plans"), 1, "no undo plan was made");
}

#[test]
fn a_taken_old_name_or_a_changed_old_folder_blocks_the_undo() {
    // A file or a folder now has the old name.
    for occupant in ["file", "folder"] {
        let f = Fixture::new();
        let organized = organize(&f, "todo.txt", "Taxes", &[]);
        if occupant == "file" {
            fs::write(f.path("todo.txt"), "a new list\n").unwrap();
        } else {
            fs::create_dir(f.path("todo.txt")).unwrap();
        }
        conflict(&f, &organized.action_id);
        assert_eq!(
            sha256_hex(&fs::read(f.path("Taxes/todo.txt")).unwrap()),
            organized_sha(&f, &organized)
        );
    }

    // The old folder was replaced by another folder of the same name.
    let f = Fixture::new();
    let organized = organize(&f, "Notes/2026/plan.md", "Taxes", &[]);
    fs::rename(f.path("Notes/2026"), f.path("Notes/2026-old")).unwrap();
    fs::create_dir(f.path("Notes/2026")).unwrap();
    conflict(&f, &organized.action_id);

    // The old folder was deleted: it is not re-created.
    let f = Fixture::new();
    let organized = organize(&f, "Notes/2026/plan.md", "Taxes", &[]);
    fs::remove_dir(f.path("Notes/2026")).unwrap();
    let error = conflict(&f, &organized.action_id);
    assert!(error.message.contains("does not re-create"));
    assert!(!f.path("Notes/2026").exists());
}

fn organized_sha(f: &Fixture, organized: &ActionView) -> String {
    f.service()
        .store()
        .unwrap()
        .get_file(&organized.file_id)
        .unwrap()
        .unwrap()
        .0
        .sha256
}

#[test]
fn a_revoked_folder_blocks_the_undo() {
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    f.service().revoke_root(&f.root_id).unwrap();
    conflict(&f, &organized.action_id);
    assert!(f.path("Taxes/todo.txt").exists());
}

#[test]
fn confirming_checks_everything_again() {
    // Edited between the preview and the confirmation.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    fs::write(f.path("Taxes/todo.txt"), "buy milk and eggs\n").unwrap();
    assert_eq!(
        f.apply(&plan.plan_id).unwrap_err().code,
        ErrorCode::FileChanged
    );
    assert_eq!(
        f.service().get_plan(&plan.plan_id).unwrap().status,
        PlanStatus::Invalidated
    );
    // The old name taken between the preview and the confirmation.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    fs::write(f.path("todo.txt"), "a new list\n").unwrap();
    assert_eq!(
        f.apply(&plan.plan_id).unwrap_err().code,
        ErrorCode::TargetConflict
    );
    // Either way: nothing moved, the organize action still committed.
    assert!(f.path("Taxes/todo.txt").exists());
    assert_eq!(
        f.service()
            .get_action(&organized.action_id)
            .unwrap()
            .undo_status,
        UndoStatus::Available
    );
    assert_eq!(f.count("actions"), 1);
}

#[test]
fn an_undo_plan_whose_organize_action_changed_meanwhile_never_runs() {
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    // However it happened, the organize action is no longer committed.
    f.db()
        .execute(
            "UPDATE actions SET status = 'undone' WHERE action_id = ?1",
            [&organized.action_id],
        )
        .unwrap();

    let error = f.apply(&plan.plan_id).unwrap_err();

    assert_eq!(error.code, ErrorCode::NothingToUndo);
    assert_eq!(
        f.service().get_plan(&plan.plan_id).unwrap().status,
        PlanStatus::Invalidated
    );
    assert!(f.path("Taxes/todo.txt").exists(), "nothing moved");
    assert_eq!(f.count("actions"), 1, "no undo action");
}

// ---------------------------------------------------------------------------
// Repeats, cancelling, other plans
// ---------------------------------------------------------------------------

#[test]
fn undoing_twice_moves_nothing_more() {
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    let undo = f.apply(&plan.plan_id).unwrap();
    let records = (f.count("actions"), f.count("path_history"));

    // Confirming the same undo plan again, and asking to undo again.
    assert_eq!(f.apply(&plan.plan_id).unwrap(), undo);
    match f.service().preview_undo(&organized.action_id).unwrap() {
        UndoPreview::AlreadyUndone { undo: found } => assert_eq!(found, undo),
        other => panic!("{other:?}"),
    }
    // The organize plan itself is never used again.
    let organize_plan = organized.plan_id.clone();
    assert_eq!(
        f.apply(&organize_plan).unwrap().action_id,
        organized.action_id
    );
    assert_eq!((f.count("actions"), f.count("path_history")), records);
    assert!(f.path("todo.txt").exists());
}

#[test]
fn only_an_organized_file_can_be_undone() {
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let undo = f
        .apply(&undo_plan(&f, &organized.action_id).plan_id)
        .unwrap();
    // An undo itself.
    assert_eq!(
        f.service().preview_undo(&undo.action_id).unwrap_err().code,
        ErrorCode::NothingToUndo
    );
    // An organize action that did not move the file.
    let file = f.register("Notes/2026/plan.md");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let failed = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::BeforeMove {
                fs::write(f.path("Taxes/plan.md"), "theirs").unwrap();
            }
        })
        .unwrap();
    assert_eq!(failed.status, ActionStatus::FailedNoMove);
    assert_eq!(failed.undo_status, UndoStatus::NotAvailable);
    assert_eq!(
        f.service()
            .preview_undo(&failed.action_id)
            .unwrap_err()
            .code,
        ErrorCode::NothingToUndo
    );
    // IDs.
    assert_eq!(
        f.service().preview_undo("nope").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        f.service()
            .preview_undo("0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b")
            .unwrap_err()
            .code,
        ErrorCode::UnknownAction
    );
}

#[test]
fn cancelling_or_replacing_an_undo_preview_leaves_the_organize_action() {
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);

    let first = undo_plan(&f, &organized.action_id);
    assert_eq!(
        f.service().cancel_plan(&first.plan_id).unwrap().status,
        PlanStatus::Cancelled
    );
    // Asking again supersedes nothing that runs; a normal plan for the file
    // then supersedes the open undo plan (one open plan per file).
    let second = undo_plan(&f, &organized.action_id);
    let file = f.register("Taxes/todo.txt");
    assert_eq!(file.file_id, organized.file_id);
    let normal = f.plan(&file, "Notes", &[]);
    assert_eq!(
        f.service().get_plan(&second.plan_id).unwrap().status,
        PlanStatus::Superseded
    );
    assert!(normal.can_confirm);
    let original = f.service().get_action(&organized.action_id).unwrap();
    assert_eq!(
        (original.status, original.undo_status),
        (ActionStatus::Committed, UndoStatus::Available)
    );
    assert!(f.path("Taxes/todo.txt").exists());
}

// ---------------------------------------------------------------------------
// An undo that stopped half way (O18-R)
// ---------------------------------------------------------------------------

#[test]
fn an_undo_stopped_after_its_move_is_committed_once_at_the_next_start() {
    let mut f = Fixture::new();
    let first_seen = (
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        fs::metadata(f.path("todo.txt")).unwrap().ino(),
    );
    let organized = organize(&f, "todo.txt", "Notes", &["Archive"]);
    let plan = undo_plan(&f, &organized.action_id);
    let undo_id = stop_at(&f, &plan.plan_id, Checkpoint::AfterMove);
    // Meanwhile the undo is in progress: no second undo, no other plan.
    match f.service().preview_undo(&organized.action_id).unwrap() {
        UndoPreview::UndoInProgress { undo } => assert_eq!(undo.action_id, undo_id),
        other => panic!("{other:?}"),
    }
    let file = f.register("todo.txt");
    assert_eq!(
        f.try_plan(&file, "Taxes", &[]).unwrap_err().code,
        ErrorCode::UnresolvedAction
    );

    f.restart();

    let undo = f.service().get_action(&undo_id).unwrap();
    assert_eq!(undo.status, ActionStatus::Committed);
    assert!(undo.recovered_at_ms.is_some());
    let original = f.service().get_action(&organized.action_id).unwrap();
    assert_eq!(original.status, ActionStatus::Undone);
    assert_eq!(recorded(&f, &organized.file_id).0, "todo.txt");
    // The recovered undo cleaned up too.
    assert!(!f.path("Notes/Archive").exists());
    assert_eq!(undo.folders_removed, ["Notes/Archive"]);
    // The same file object and bytes, back once.
    assert_eq!(
        (
            sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
            fs::metadata(f.path("todo.txt")).unwrap().ino(),
        ),
        first_seen
    );
    let undo_entries = history(&f, &organized.file_id)
        .iter()
        .filter(|(reason, _, _)| reason == "undo")
        .count();
    assert_eq!(undo_entries, 1);

    // Starting again adds nothing.
    let records = (
        f.count("actions"),
        f.count("path_history"),
        f.count("action_events"),
    );
    f.restart();
    assert_eq!(
        (
            f.count("actions"),
            f.count("path_history"),
            f.count("action_events")
        ),
        records
    );
}

#[test]
fn an_undo_stopped_before_its_move_ends_and_can_be_asked_for_again() {
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    let undo_id = stop_at(&f, &plan.plan_id, Checkpoint::BeforeMove);

    f.restart();

    assert_eq!(
        f.service().get_action(&undo_id).unwrap().status,
        ActionStatus::FailedNoMove
    );
    let original = f.service().get_action(&organized.action_id).unwrap();
    assert_eq!(
        (original.status, original.undo_status),
        (ActionStatus::Committed, UndoStatus::Available)
    );
    assert!(f.path("Taxes/todo.txt").exists(), "not moved automatically");
    // A new undo plan, a new confirmation.
    let again = f
        .apply(&undo_plan(&f, &organized.action_id).plan_id)
        .unwrap();
    assert_eq!(again.status, ActionStatus::Committed);
    assert_ne!(again.action_id, undo_id);
}

#[test]
fn an_uncertain_undo_waits_for_a_decision() {
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    let undo_id = stop_at(&f, &plan.plan_id, Checkpoint::AfterMove);
    // Something else appears at the name the file just left.
    fs::write(f.path("Taxes/todo.txt"), "someone else's\n").unwrap();

    f.restart();

    assert_eq!(
        f.service().get_action(&undo_id).unwrap().status,
        ActionStatus::NeedsUserDecision
    );
    let original = f.service().get_action(&organized.action_id).unwrap();
    assert_eq!(
        (original.status, original.undo_status),
        (ActionStatus::Committed, UndoStatus::InProgress)
    );
    assert!(matches!(
        f.service().preview_undo(&organized.action_id).unwrap(),
        UndoPreview::UndoInProgress { .. }
    ));
}

#[test]
fn a_failed_final_undo_record_is_completed_at_the_next_start() {
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_refuse BEFORE UPDATE OF status ON actions
             WHEN NEW.status = 'undone'
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;",
        )
        .unwrap();
    let undo = f.apply(&plan.plan_id).unwrap();
    assert_eq!(undo.status, ActionStatus::NeedsUserDecision);
    assert_eq!(
        f.service().get_action(&organized.action_id).unwrap().status,
        ActionStatus::Committed,
        "the undo and the undone mark go together or not at all"
    );
    f.db().execute_batch("DROP TRIGGER test_refuse;").unwrap();

    f.restart();

    assert_eq!(
        f.service().get_action(&undo.action_id).unwrap().status,
        ActionStatus::Committed
    );
    assert_eq!(
        f.service().get_action(&organized.action_id).unwrap().status,
        ActionStatus::Undone
    );
    let undo_entries = history(&f, &organized.file_id)
        .into_iter()
        .filter(|(reason, _, _)| reason == "undo")
        .count();
    assert_eq!(undo_entries, 1);
}

#[test]
fn a_check_that_cannot_be_recorded_for_an_undo_blocks_storage() {
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let plan = undo_plan(&f, &organized.action_id);
    stop_at(&f, &plan.plan_id, Checkpoint::AfterMove);
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_refuse BEFORE UPDATE OF status ON actions
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;",
        )
        .unwrap();
    f.restart();
    assert!(matches!(
        f.service().get_app_state().unwrap().storage,
        StorageStatus::Blocked { .. }
    ));
}

// ---------------------------------------------------------------------------
// Cleaning up after an undo
// ---------------------------------------------------------------------------

#[test]
fn folders_that_are_not_empty_or_not_its_own_stay_with_a_warning() {
    // A .DS_Store in a folder the organize action created.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Notes", &["Archive", "Q1"]);
    fs::write(f.path("Notes/Archive/Q1/.DS_Store"), "finder").unwrap();
    let undo = f
        .apply(&undo_plan(&f, &organized.action_id).plan_id)
        .unwrap();
    assert_eq!(undo.status, ActionStatus::Committed, "the file is back");
    assert!(f.path("todo.txt").exists());
    assert!(f.path("Notes/Archive/Q1/.DS_Store").exists());
    assert_eq!(undo.folders_kept, ["Notes/Archive", "Notes/Archive/Q1"]);

    // Another file was put into the outer folder later.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Notes", &["Archive", "Q1"]);
    fs::write(f.path("Notes/Archive/mine.txt"), "mine").unwrap();
    let undo = f
        .apply(&undo_plan(&f, &organized.action_id).plan_id)
        .unwrap();
    assert_eq!(undo.folders_removed, ["Notes/Archive/Q1"]);
    assert_eq!(undo.folders_kept, ["Notes/Archive"]);
    assert_eq!(fs::read(f.path("Notes/Archive/mine.txt")).unwrap(), b"mine");

    // Into a folder that already existed: nothing is Traeva's to remove.
    let f = Fixture::new();
    let organized = organize(&f, "Notes/2026/plan.md", "Taxes", &[]);
    let undo = f
        .apply(&undo_plan(&f, &organized.action_id).plan_id)
        .unwrap();
    assert!(undo.folders_removed.is_empty() && undo.folders_kept.is_empty());
    assert!(f.path("Taxes").is_dir());
}

/// The undo's cleanup fails: its intent cannot be saved, or macOS refuses to
/// remove the folder. The file is back all the same; the folder stays and is
/// shown as kept, also after a restart (O35).
#[test]
fn a_cleanup_that_fails_never_undoes_the_undo() {
    use std::os::unix::fs::PermissionsExt;
    // The cleanup intent cannot be saved.
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Notes", &["Archive"]);
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_refuse BEFORE INSERT ON action_events
             WHEN NEW.step = 'cleanup'
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;",
        )
        .unwrap();
    let undo = f
        .apply(&undo_plan(&f, &organized.action_id).plan_id)
        .unwrap();
    f.db().execute_batch("DROP TRIGGER test_refuse;").unwrap();
    assert_eq!(undo.status, ActionStatus::Committed, "the file is back");
    assert_eq!(undo.folders_kept, ["Notes/Archive"]);
    assert!(f.path("Notes/Archive").is_dir() && f.path("todo.txt").exists());
    f.restart();
    assert_eq!(
        f.service().get_action(&undo.action_id).unwrap().status,
        ActionStatus::Committed
    );
    assert_eq!(
        f.service().get_action(&organized.action_id).unwrap().status,
        ActionStatus::Undone
    );
    assert_eq!(recorded(&f, &organized.file_id).0, "todo.txt");

    // macOS refuses to remove the folder.
    let f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Notes", &["Archive"]);
    let notes = f.path("Notes");
    let plan_id = undo_plan(&f, &organized.action_id).plan_id;
    let undo = f
        .apply_with(&plan_id, &mut |point| {
            if point == Checkpoint::AfterMove {
                // The file is out of the folder; nothing may be removed from Notes.
                fs::set_permissions(&notes, fs::Permissions::from_mode(0o555)).unwrap();
            }
        })
        .unwrap();
    fs::set_permissions(&notes, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(undo.status, ActionStatus::Committed, "the file is back");
    assert_eq!(undo.folders_kept, ["Notes/Archive"]);
    assert!(f.path("Notes/Archive").is_dir() && f.path("todo.txt").exists());
    assert_eq!(
        f.service().get_action(&organized.action_id).unwrap().status,
        ActionStatus::Undone
    );
}

// ---------------------------------------------------------------------------
// #79: a folder removed from Traeva
// ---------------------------------------------------------------------------

/// Organizes `name` inside another authorized root into its "Archive".
fn organize_in(f: &Fixture, root: &std::path::Path, name: &str) -> (String, ActionView) {
    let root_id = f.service().authorize_root(root).unwrap().root_id;
    let file = f
        .service()
        .register_file(&root_id, &root.join(name))
        .unwrap()
        .file;
    let catalog = f.service().list_folders(&root_id).unwrap();
    let folder_id = catalog
        .folders
        .iter()
        .find(|folder| folder.relative_path == "Archive")
        .unwrap()
        .folder_id
        .clone();
    let outcome = f
        .service()
        .build_plan(&crate::contracts::PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: root_id.clone(),
            operation_id: None,
            target: crate::contracts::PlanTarget::Folder {
                catalog_id: catalog.catalog_id,
                folder_id,
                new_folders: Vec::new(),
            },
        })
        .unwrap();
    let crate::contracts::BuildOutcome::Planned { plan } = outcome else {
        panic!("expected a plan");
    };
    let action = f.apply(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    (root_id, action)
}

/// After its folder is removed from Traeva, an organized file can no longer
/// be undone, and the saved view says why: also after a restart and after
/// adding the folder again. Actions in other folders keep their Undo, the
/// history stays readable, and the core still refuses on its own.
#[test]
fn a_removed_folder_ends_the_undo_of_its_actions_and_says_so() {
    let mut f = Fixture::new();
    let organized = organize(&f, "todo.txt", "Taxes", &[]);
    let undone = organize(&f, "Notes/2026/plan.md", "Taxes", &[]);
    f.apply(&undo_plan(&f, &undone.action_id).plan_id).unwrap();
    // A preview made before the folder is removed.
    let early = organize(&f, "Taxes/todo.txt", "Notes", &[]);
    let early_plan = undo_plan(&f, &early.action_id);
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    fs::create_dir(other.path().join("Archive")).unwrap();
    fs::write(other.path().join("memo.txt"), "call the dentist\n").unwrap();
    let (other_root, elsewhere) = organize_in(&f, other.path(), "memo.txt");
    // From the other folder into this one: only its target root is removed.
    fs::write(other.path().join("list.txt"), "pick up the photos\n").unwrap();
    let crossing = {
        let file = f
            .service()
            .register_file(&other_root, &other.path().join("list.txt"))
            .unwrap()
            .file;
        let catalog = f.service().list_folders(&f.root_id).unwrap();
        let folder_id = catalog
            .folders
            .iter()
            .find(|folder| folder.relative_path == "Notes")
            .unwrap()
            .folder_id
            .clone();
        let outcome = f
            .service()
            .build_plan(&crate::contracts::PlanRequest {
                file_id: file.file_id,
                file_revision: file.file_revision,
                target_root_id: f.root_id.clone(),
                operation_id: None,
                target: crate::contracts::PlanTarget::Folder {
                    catalog_id: catalog.catalog_id,
                    folder_id,
                    new_folders: Vec::new(),
                },
            })
            .unwrap();
        let crate::contracts::BuildOutcome::Planned { plan } = outcome else {
            panic!("expected a plan");
        };
        f.apply(&plan.plan_id).unwrap()
    };
    assert_eq!(crossing.undo_status, UndoStatus::Available);
    let status = |f: &Fixture, id: &str| f.service().get_action(id).unwrap().undo_status;
    assert_eq!(status(&f, &organized.action_id), UndoStatus::Available);

    f.service().revoke_root(&f.root_id).unwrap();
    let before = snapshot::snapshot(f.root.path());

    assert_eq!(status(&f, &early.action_id), UndoStatus::RootRevoked);
    assert_eq!(
        serde_json::to_value(UndoStatus::RootRevoked).unwrap(),
        "root_revoked",
        "the name types.ts mirrors"
    );
    assert_eq!(
        status(&f, &undone.action_id),
        UndoStatus::Undone,
        "history kept"
    );
    assert_eq!(status(&f, &elsewhere.action_id), UndoStatus::Available);
    assert_eq!(
        status(&f, &crossing.action_id),
        UndoStatus::RootRevoked,
        "its target folder was removed"
    );
    let recent = f.service().get_app_state().unwrap().recent_actions;
    assert!(recent.iter().any(|a| a.action_id == organized.action_id));
    assert!(
        recent
            .iter()
            .filter(|a| a.source.root_id == f.root_id && a.kind == PlanKind::Organize)
            .all(|a| a.undo_status != UndoStatus::Available),
        "nothing in the removed folder offers Undo"
    );
    // The core refuses on its own: a new preview, and the preview from before.
    conflict(&f, &early.action_id);
    let refused = f.apply(&early_plan.plan_id).unwrap_err();
    assert_eq!(refused.code, ErrorCode::StalePlan, "{refused:?}");
    assert_eq!(snapshot::snapshot(f.root.path()), before, "nothing moved");

    // Neither a restart nor adding the folder again brings Undo back.
    f.restart();
    assert_eq!(status(&f, &early.action_id), UndoStatus::RootRevoked);
    f.service().authorize_root(f.root.path()).unwrap();
    assert_eq!(status(&f, &early.action_id), UndoStatus::RootRevoked);
    assert_eq!(status(&f, &elsewhere.action_id), UndoStatus::Available);
    conflict(&f, &early.action_id);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}
