//! Record checks after a run stopped half way (O18-R: the service is
//! dropped and started again on the same database; this is not a real
//! process crash, see #21). Synthetic temporary files only.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::*;
use crate::contracts::{
    ActionView, AppError, BuildOutcome, ErrorCode, FileView, PlanRequest, PlanTarget, PlanView,
    StorageStatus,
};
use crate::execute::Checkpoint;
use crate::observe::sha256_hex;
use crate::test_fixture::{Fixture, edit_keeping_size_and_mtime, quick_read, steps};
use crate::test_snapshot as snapshot;

/// Runs the plan until `point` and stops it there: no failure handling runs
/// and nothing more is written. The process goes on, so this is O18-R; the
/// real process kill is O18-C (#21). Returns the action.
fn stop_at(f: &Fixture, plan_id: &str, point: Checkpoint) -> String {
    let stopped = catch_unwind(AssertUnwindSafe(|| {
        f.apply_with(plan_id, &mut |now| {
            if now == point {
                // Unwinds without printing a panic message.
                resume_unwind(Box::new("stopped by the test"));
            }
        })
    }));
    assert!(stopped.is_err(), "the run should stop at {point:?}");
    f.service().get_plan(plan_id).unwrap().action_id.unwrap()
}

fn action(f: &Fixture, action_id: &str) -> ActionView {
    f.service().get_action(action_id).unwrap()
}

fn outcome(view: &ActionView) -> (ActionStatus, Option<ErrorCode>) {
    (view.status, view.problem.as_ref().map(|p| p.code))
}

/// The latest record check of the action.
fn last_check_of(f: &Fixture, action_id: &str) -> Reconciliation {
    let rows = f.service().store().unwrap().journal(action_id).unwrap();
    last_check(&rows).cloned().expect("a record check")
}

fn found(f: &Fixture, action_id: &str) -> (Found, Found) {
    let check = last_check_of(f, action_id);
    (check.source_now, check.target_now)
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

fn recorded_path(f: &Fixture, file_id: &str) -> String {
    f.db()
        .query_row(
            "SELECT relative_path FROM files WHERE file_id = ?1",
            [file_id],
            |r| r.get(0),
        )
        .unwrap()
}

/// Appends a journal entry directly, as a damaged or edited database might.
fn add_to_journal(f: &Fixture, action_id: &str, step: &str, phase: &str, detail: &str) {
    f.db()
        .execute(
            "INSERT INTO action_events (action_id, sequence, step, phase, detail_json, recorded_at_ms)
             SELECT ?1, MAX(sequence) + 1, ?2, ?3, ?4, 0 FROM action_events WHERE action_id = ?1",
            (action_id, step, phase, detail),
        )
        .unwrap();
}

// ---------------------------------------------------------------------------
// The three O18-R checkpoints
// ---------------------------------------------------------------------------

#[test]
fn stopped_after_mkdir_before_its_receipt_keeps_the_folder_and_the_file() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMkdir(0));
    let before = snapshot::snapshot(f.root.path());

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(
        outcome(&view),
        (ActionStatus::FailedNoMove, Some(ErrorCode::RunInterrupted))
    );
    assert_eq!(last_check_of(&f, &action_id).basis, Basis::NoMoveIntent);
    // No receipt: the folder is reported, never claimed or removed.
    assert_eq!(view.folders_without_receipt, ["Notes/Archive"]);
    assert!(view.folders_created.is_empty() && view.folders_removed.is_empty());
    assert_eq!(
        snapshot::snapshot(f.root.path()),
        before,
        "the check changed nothing on disk"
    );
    assert_eq!(
        f.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("reconcile", "result"),
        ])
    );
}

#[test]
fn stopped_with_receipts_before_the_move_intent_removes_its_empty_folders() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive", "Q1"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::FailedNoMove);
    assert_eq!(last_check_of(&f, &action_id).basis, Basis::NoMoveIntent);
    assert_eq!(view.folders_created, ["Notes/Archive", "Notes/Archive/Q1"]);
    assert_eq!(
        view.folders_removed,
        ["Notes/Archive/Q1", "Notes/Archive"],
        "innermost first"
    );
    assert!(!f.path("Notes/Archive").exists());
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        file.sha256
    );
    assert_eq!(
        f.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("reconcile", "result"),
            ("cleanup", "intent"),
            ("cleanup", "result"),
            ("cleanup", "intent"),
            ("cleanup", "result"),
        ])
    );
}

#[test]
fn a_folder_is_not_removed_when_its_cleanup_intent_cannot_be_saved() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_refuse BEFORE INSERT ON action_events
             WHEN NEW.step = 'cleanup'
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;",
        )
        .unwrap();

    f.restart();

    assert!(matches!(
        f.service().get_app_state().unwrap().storage,
        StorageStatus::Blocked { .. }
    ));
    assert!(f.path("Notes/Archive").is_dir(), "no intent, no removal");
    f.db().execute_batch("DROP TRIGGER test_refuse;").unwrap();
    f.restart();
    // The check itself was saved before the cleanup; the folder just stays.
    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::FailedNoMove);
    assert!(view.folders_removed.is_empty());
    assert!(f.path("Notes/Archive").is_dir());
}

#[test]
fn a_ds_store_keeps_the_folder_and_every_folder_around_it() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive", "Q1"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
    fs::write(f.path("Notes/Archive/Q1/.DS_Store"), "finder").unwrap();

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::FailedNoMove);
    assert!(view.folders_removed.is_empty());
    assert!(f.path("Notes/Archive/Q1/.DS_Store").exists());
    assert!(
        !f.journal(&action_id)
            .iter()
            .any(|(step, _)| step == "cleanup"),
        "nothing was even tried"
    );
}

#[test]
fn stopped_after_the_move_intent_before_the_move_is_failed_no_move() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMove);
    let before = snapshot::snapshot(f.root.path());

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(
        outcome(&view),
        (ActionStatus::FailedNoMove, Some(ErrorCode::RunInterrupted))
    );
    let check = last_check_of(&f, &action_id);
    assert_eq!(check.basis, Basis::StillAtSource);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::ExpectedFile, Found::Nothing)
    );
    assert_eq!(
        snapshot::snapshot(f.root.path()),
        before,
        "the remaining move was not done"
    );
    // Settled: the file can be planned again.
    assert!(f.plan(&file, "Notes", &[]).can_confirm);
}

#[test]
fn stopped_after_the_move_before_the_final_record_is_committed_once() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMove);
    let before = snapshot::snapshot(f.root.path());

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::Committed);
    assert!(view.problem.is_none());
    assert!(view.committed_at_ms.is_some() && view.recovered_at_ms.is_some());
    assert_eq!(last_check_of(&f, &action_id).basis, Basis::MovedAsPlanned);
    assert_eq!(recorded_path(&f, &file.file_id), "Taxes/todo.txt");
    assert_eq!(organize_entries(&f, &file.file_id), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before, "no second move");
    assert_eq!(
        f.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("move", "result"),
            ("reconcile", "result"),
        ])
    );

    // Starting again checks nothing more and records nothing twice.
    let events = f.count("action_events");
    f.restart();
    assert_eq!(action(&f, &action_id), view);
    assert_eq!((f.count("action_events"), f.count("actions")), (events, 1));
    assert_eq!(organize_entries(&f, &file.file_id), 1);
}

// ---------------------------------------------------------------------------
// Anything else needs a decision, and the check moves nothing
// ---------------------------------------------------------------------------

/// Stops a run at `point`, changes things with `meanwhile`, starts again,
/// and checks the action needs a decision without anything moved.
fn needs_a_decision_after(
    point: Checkpoint,
    meanwhile: impl FnOnce(&Fixture),
) -> (Fixture, String, (Found, Found)) {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, point);
    meanwhile(&f);
    let before = snapshot::snapshot(f.root.path());

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(
        outcome(&view),
        (
            ActionStatus::NeedsUserDecision,
            Some(ErrorCode::ResultUnconfirmed)
        )
    );
    assert_eq!(
        snapshot::snapshot(f.root.path()),
        before,
        "the check moved nothing"
    );
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    let seen = found(&f, &action_id);
    (f, action_id, seen)
}

#[test]
fn a_file_at_both_names_needs_a_decision() {
    let (_, _, seen) = needs_a_decision_after(Checkpoint::AfterMove, |f| {
        fs::write(f.path("todo.txt"), "a new list\n").unwrap();
    });
    assert_eq!(seen, (Found::SomethingElse, Found::ExpectedFile));
}

#[test]
fn a_file_at_neither_name_needs_a_decision() {
    let (_, _, seen) = needs_a_decision_after(Checkpoint::AfterMove, |f| {
        fs::remove_file(f.path("Taxes/todo.txt")).unwrap();
    });
    assert_eq!(seen, (Found::Nothing, Found::Nothing));
}

#[test]
fn the_same_inode_with_other_bytes_needs_a_decision() {
    let (_, _, seen) = needs_a_decision_after(Checkpoint::AfterMove, |f| {
        fs::write(f.path("Taxes/todo.txt"), "buy mild\n").unwrap(); // same length
    });
    assert_eq!(seen, (Found::Nothing, Found::ChangedFile));
}

#[test]
fn a_replaced_target_folder_needs_a_decision() {
    // After the move: the file sits in the folder that was moved away.
    let (f, _, seen) = needs_a_decision_after(Checkpoint::AfterMove, |f| {
        fs::rename(f.path("Taxes"), f.path("Taxes-moved")).unwrap();
        fs::create_dir(f.path("Taxes")).unwrap();
    });
    assert_eq!(seen, (Found::Nothing, Found::NotChecked));
    assert!(f.path("Taxes-moved/todo.txt").exists());

    // Before the move, with a move intent: the source is unchanged, but the
    // material asks for valid folder chains on both sides.
    let (_, _, seen) = needs_a_decision_after(Checkpoint::BeforeMove, |f| {
        fs::rename(f.path("Taxes"), f.path("Taxes-old")).unwrap();
        fs::create_dir(f.path("Taxes")).unwrap();
    });
    assert_eq!(seen, (Found::ExpectedFile, Found::NotChecked));
}

#[test]
fn a_moved_root_needs_a_decision_until_it_is_back() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMove);
    let root = f.root.path().to_path_buf();
    let moved = root.with_extension("moved");
    fs::rename(&root, &moved).unwrap();

    f.restart();

    assert_eq!(
        outcome(&action(&f, &action_id)),
        (
            ActionStatus::NeedsUserDecision,
            Some(ErrorCode::ResultUnconfirmed)
        )
    );
    assert_eq!(
        found(&f, &action_id),
        (Found::NotChecked, Found::NotChecked)
    );
    assert!(moved.join("Taxes/todo.txt").exists(), "nothing moved");

    // Once the folder is back, the next start finds the move done as planned.
    fs::rename(&moved, &root).unwrap();
    f.restart();
    assert_eq!(action(&f, &action_id).status, ActionStatus::Committed);
    assert_eq!(organize_entries(&f, &file.file_id), 1);
}

#[test]
fn checking_again_with_nothing_new_records_nothing() {
    let (mut f, action_id, _) = needs_a_decision_after(Checkpoint::AfterMove, |f| {
        fs::remove_file(f.path("Taxes/todo.txt")).unwrap();
    });
    let journal = f.journal(&action_id);
    f.restart();
    f.restart();
    assert_eq!(f.journal(&action_id), journal);
    assert_eq!(f.count("actions"), 1);
    assert_eq!(
        f.service().reconcile_action(&action_id).unwrap().status,
        ActionStatus::NeedsUserDecision
    );
    assert_eq!(f.journal(&action_id), journal);
}

// ---------------------------------------------------------------------------
// The material's supplement: no move intent
// ---------------------------------------------------------------------------

#[test]
fn without_a_move_intent_a_changed_or_missing_file_is_still_failed_no_move() {
    // The file was edited before the next start.
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
    fs::write(f.path("todo.txt"), "buy milk and eggs\n").unwrap();
    f.restart();
    assert_eq!(action(&f, &action_id).status, ActionStatus::FailedNoMove);
    assert_eq!(found(&f, &action_id), (Found::ChangedFile, Found::Nothing));
    // Not blocked: once chosen again, the file can be planned.
    let again = f.register("todo.txt");
    assert!(f.plan(&again, "Taxes", &[]).can_confirm);

    // The file was deleted before the next start.
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
    fs::remove_file(f.path("todo.txt")).unwrap();
    f.restart();
    assert_eq!(action(&f, &action_id).status, ActionStatus::FailedNoMove);
    assert_eq!(last_check_of(&f, &action_id).basis, Basis::NoMoveIntent);
    assert_eq!(found(&f, &action_id), (Found::Nothing, Found::Nothing));
}

#[test]
fn a_journal_that_does_not_match_its_plan_is_not_read_as_no_move_intent() {
    for (step, phase, detail) in [
        // A receipt for a folder this plan never creates.
        (
            "mkdir",
            "result",
            r#"{"path":"Elsewhere","dev":1,"ino":2,"birthtime_ns":3}"#,
        ),
        // A move intent that cannot be read.
        ("move", "intent", "{}"),
        // A readable move intent for another target (filled in below).
        ("move", "intent", "WRONG_TARGET"),
    ] {
        let mut f = Fixture::new();
        let file = f.register("todo.txt");
        let plan_id = f.plan(&file, "Notes", &["Archive"]).plan_id;
        let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
        let detail = if detail == "WRONG_TARGET" {
            let plan = f.stored(&plan_id);
            serde_json::to_string(&MoveIntent {
                from_root_id: plan.source_root_id.clone(),
                from_path: plan.source_relative_path.clone(),
                to_root_id: plan.target_root_id.clone(),
                to_path: "Elsewhere/todo.txt".to_string(),
                sha256: plan.expected_source.sha256.clone(),
                byte_length: plan.expected_source.byte_length,
                evidence: plan.expected_source.evidence.clone(),
            })
            .unwrap()
        } else {
            detail.to_string()
        };
        add_to_journal(&f, &action_id, step, phase, &detail);

        f.restart();

        let view = action(&f, &action_id);
        assert_eq!(
            view.status,
            ActionStatus::NeedsUserDecision,
            "{step}/{phase}"
        );
        let check = last_check_of(&f, &action_id);
        assert!(!check.journal_matches);
        assert_eq!(check.basis, Basis::Uncertain);
        // Receipts from such a journal are not trusted: nothing is removed.
        assert!(f.path("Notes/Archive").is_dir());
        assert!(f.path("todo.txt").exists());
    }
}

#[test]
fn a_moved_file_with_a_journal_that_does_not_match_is_not_committed() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMove);
    add_to_journal(
        &f,
        &action_id,
        "mkdir",
        "result",
        r#"{"path":"Elsewhere","dev":1,"ino":2,"birthtime_ns":3}"#,
    );

    f.restart();

    assert_eq!(
        action(&f, &action_id).status,
        ActionStatus::NeedsUserDecision
    );
    assert_eq!(found(&f, &action_id), (Found::Nothing, Found::ExpectedFile));
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(recorded_path(&f, &file.file_id), "todo.txt");
}

#[test]
fn a_decision_keeps_every_folder() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMove);
    fs::remove_file(f.path("Notes/Archive/todo.txt")).unwrap(); // the folder is empty again

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    assert!(f.path("Notes/Archive").is_dir());
    assert!(view.folders_removed.is_empty());
}

// ---------------------------------------------------------------------------
// Unsettled results left by a run, and checking on request
// ---------------------------------------------------------------------------

/// A run whose final record cannot be saved ends in needs_user_decision.
fn final_record_refused(f: &Fixture) -> (String, String) {
    final_record_refused_for(f, "todo.txt")
}

/// The same for the file at `relative`, moved to Taxes.
fn final_record_refused_for(f: &Fixture, relative: &str) -> (String, String) {
    let file = f.register(relative);
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_refuse BEFORE UPDATE OF status ON actions
             WHEN NEW.status = 'committed'
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;",
        )
        .unwrap();
    let view = f.apply(&plan_id).unwrap();
    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    f.db().execute_batch("DROP TRIGGER test_refuse;").unwrap();
    (view.action_id, file.file_id)
}

#[test]
fn a_decision_left_by_a_run_is_checked_again_at_the_next_start() {
    let mut f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::Committed);
    assert!(view.recovered_at_ms.is_some());
    assert_eq!(recorded_path(&f, &file_id), "Taxes/todo.txt");
    assert_eq!(organize_entries(&f, &file_id), 1);
}

#[test]
fn reconcile_action_checks_one_action_on_request() {
    let f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);

    let view = f.service().reconcile_action(&action_id).unwrap();

    assert_eq!(view.status, ActionStatus::Committed);
    assert_eq!(organize_entries(&f, &file_id), 1);
    // A settled action is only reported.
    let events = f.count("action_events");
    assert_eq!(f.service().reconcile_action(&action_id).unwrap(), view);
    assert_eq!(f.count("action_events"), events);
    let unknown = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";
    assert_eq!(
        f.service().reconcile_action(unknown).unwrap_err().code,
        ErrorCode::UnknownAction
    );
    assert_eq!(
        f.service().reconcile_action("nope").unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

// ---------------------------------------------------------------------------
// Changes while a record check reads the file (#46)
// ---------------------------------------------------------------------------

/// `reconcile_action` with `meanwhile` run at the end of the check's first
/// full read: the bytes were read and were right, then something changes
/// before the result is recorded.
fn reconcile_changing_during_read(
    f: &Fixture,
    action_id: &str,
    meanwhile: &dyn Fn(),
) -> ActionView {
    let mut first = true;
    f.service()
        .reconcile_action_with(action_id, &mut |parent, name, dev| {
            let read = quick_read(parent, name, dev);
            if std::mem::take(&mut first) {
                meanwhile();
            }
            read
        })
        .unwrap()
}

fn outside() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap()
}

#[test]
fn a_check_does_not_commit_when_the_target_folder_leaves_the_root_while_it_reads() {
    let f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);
    let outside = outside();

    let view = reconcile_changing_during_read(&f, &action_id, &|| {
        fs::rename(f.path("Taxes"), outside.path().join("Taxes")).unwrap();
    });

    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    assert_eq!(view.recovered_at_ms, None);
    let check = last_check_of(&f, &action_id);
    assert_eq!(check.basis, Basis::Uncertain);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::Nothing, Found::NotChecked)
    );
    assert_eq!(organize_entries(&f, &file_id), 0);
    assert_eq!(recorded_path(&f, &file_id), "todo.txt");
    // Only looked: the file stays where the folder took it.
    assert!(!f.path("todo.txt").exists());
    assert_eq!(
        fs::read(outside.path().join("Taxes/todo.txt")).unwrap(),
        b"buy milk\n"
    );
}

#[test]
fn a_check_does_not_commit_when_the_old_name_is_taken_while_it_reads() {
    let f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);

    let view = reconcile_changing_during_read(&f, &action_id, &|| {
        fs::write(f.path("todo.txt"), "another file\n").unwrap();
    });

    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    let check = last_check_of(&f, &action_id);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::SomethingElse, Found::ExpectedFile)
    );
    assert_eq!(organize_entries(&f, &file_id), 0);
    assert_eq!(fs::read(f.path("todo.txt")).unwrap(), b"another file\n");
    assert_eq!(fs::read(f.path("Taxes/todo.txt")).unwrap(), b"buy milk\n");
}

#[test]
fn a_check_does_not_commit_when_the_moved_file_is_swapped_while_it_reads() {
    // Same name, another file object: by name alone it would look fine.
    let f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);

    let view = reconcile_changing_during_read(&f, &action_id, &|| {
        fs::rename(f.path("Taxes/todo.txt"), f.path("Taxes/todo.old")).unwrap();
        fs::write(f.path("Taxes/todo.txt"), "buy milk\n").unwrap();
    });

    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    let check = last_check_of(&f, &action_id);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::Nothing, Found::SomethingElse)
    );
    assert_eq!(organize_entries(&f, &file_id), 0);
}

#[test]
fn a_check_does_not_settle_as_not_moved_when_the_source_folder_leaves_while_it_reads() {
    // "Still at the source" also rests on a read: the file's folder leaves
    // the root during it. Not failed_no_move, and no folder is cleaned up.
    let f = Fixture::new();
    let file = f.register("Notes/2026/plan.md");
    let plan_id = f.plan(&file, "Taxes", &["2026"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMove);
    let outside = outside();

    let view = reconcile_changing_during_read(&f, &action_id, &|| {
        fs::rename(f.path("Notes"), outside.path().join("Notes")).unwrap();
    });

    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    let check = last_check_of(&f, &action_id);
    assert_eq!(check.basis, Basis::Uncertain);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::NotChecked, Found::Nothing)
    );
    assert!(
        f.path("Taxes/2026").is_dir(),
        "a check that cannot tell removes nothing"
    );
    assert!(outside.path().join("Notes/2026/plan.md").exists());
}

#[test]
fn a_check_does_not_commit_when_the_moved_file_is_edited_right_after_it_is_read() {
    // #54: the same file object at the same place, edited in place with its
    // size and modification time put back. Only its change time tells, and
    // the hash just read is stale.
    let f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);

    let view = reconcile_changing_during_read(&f, &action_id, &|| {
        edit_keeping_size_and_mtime(&f.path("Taxes/todo.txt"), "pay rent\n");
    });

    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    assert_eq!(view.recovered_at_ms, None);
    let check = last_check_of(&f, &action_id);
    assert_eq!(check.basis, Basis::Uncertain);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::Nothing, Found::ChangedFile)
    );
    assert_eq!(organize_entries(&f, &file_id), 0);
    assert_eq!(recorded_path(&f, &file_id), "todo.txt");
    // Only looked: the edited file stays where it is.
    assert_eq!(fs::read(f.path("Taxes/todo.txt")).unwrap(), b"pay rent\n");
}

#[test]
fn a_check_does_not_settle_as_not_moved_when_the_file_is_edited_right_after_it_is_read() {
    // #54: "still at the source" rests on a read as well. The file there is
    // edited in place right after it was read: not failed_no_move, and no
    // folder is cleaned up.
    let f = Fixture::new();
    let file = f.register("Notes/2026/plan.md");
    let plan_id = f.plan(&file, "Taxes", &["2026"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMove);

    let view = reconcile_changing_during_read(&f, &action_id, &|| {
        edit_keeping_size_and_mtime(&f.path("Notes/2026/plan.md"), "# Plan\n\n- ship M2\n");
    });

    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    let check = last_check_of(&f, &action_id);
    assert_eq!(check.basis, Basis::Uncertain);
    assert_eq!(
        (check.source_now, check.target_now),
        (Found::ChangedFile, Found::Nothing)
    );
    assert!(
        f.path("Taxes/2026").is_dir(),
        "a check that cannot tell removes nothing"
    );
    assert_eq!(
        fs::read(f.path("Notes/2026/plan.md")).unwrap(),
        b"# Plan\n\n- ship M2\n"
    );
}

#[test]
fn the_start_check_does_not_commit_when_the_target_folder_leaves_the_root_while_it_reads() {
    let mut f = Fixture::new();
    let (action_id, file_id) = final_record_refused(&f);
    let outside = outside();
    let (taxes, away) = (f.path("Taxes"), outside.path().join("Taxes"));
    // The check at the next start reads the file for at least 350 ms. The
    // folder leaves the root during that read, or on a slow machine just
    // before it: either way the move cannot be confirmed.
    let mover = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        fs::rename(taxes, away).unwrap();
    });
    f.restart();
    mover.join().unwrap();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::NeedsUserDecision);
    assert_eq!(view.recovered_at_ms, None);
    assert_eq!(organize_entries(&f, &file_id), 0);
    assert_eq!(recorded_path(&f, &file_id), "todo.txt");
    assert!(outside.path().join("Taxes/todo.txt").exists());
}

#[test]
fn a_check_that_cannot_be_recorded_blocks_storage_until_the_next_start() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMove);
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_refuse BEFORE UPDATE OF status ON actions
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;",
        )
        .unwrap();

    f.restart();

    match f.service().get_app_state().unwrap().storage {
        StorageStatus::Blocked { error } => {
            assert_eq!(error.code, ErrorCode::DatabaseUnavailable)
        }
        other => panic!("expected blocked storage, got {other:?}"),
    }
    f.db().execute_batch("DROP TRIGGER test_refuse;").unwrap();
    f.restart();
    assert_eq!(action(&f, &action_id).status, ActionStatus::FailedNoMove);
}

// ---------------------------------------------------------------------------
// What an unsettled action blocks
// ---------------------------------------------------------------------------

#[test]
fn an_unsettled_action_blocks_plans_at_its_locations() {
    let f = Fixture::new();
    fs::write(f.path("Notes/todo.txt"), "another list\n").unwrap();
    fs::write(f.path("Notes/2026/TODO.TXT"), "a third list\n").unwrap();
    let todo = f.register("todo.txt");
    let other = f.register("Notes/todo.txt");
    // Planned before the conflict exists, run after it.
    let early = f.plan(&other, "Taxes", &[]).plan_id;
    let plan_id = f.plan(&todo, "Taxes", &[]).plan_id;
    let busy = stop_at(&f, &plan_id, Checkpoint::BeforeMove);

    // Into its target location, also spelled differently.
    let error = f.try_plan(&other, "Taxes", &[]).unwrap_err();
    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(busy.as_str()))
    );
    let shouting = f.register("Notes/2026/TODO.TXT");
    assert_eq!(
        f.try_plan(&shouting, "Taxes", &[]).unwrap_err().code,
        ErrorCode::UnresolvedAction
    );
    assert_eq!(
        f.apply(&early).unwrap_err().code,
        ErrorCode::UnresolvedAction
    );
    // Anything else goes ahead.
    let plan_md = f.register("Notes/2026/plan.md");
    assert!(f.plan(&plan_md, "Taxes", &[]).can_confirm);
}

#[test]
fn an_unsettled_action_blocks_taking_a_file_away_from_its_locations() {
    let f = Fixture::new();
    let todo = f.register("todo.txt");
    let plan_id = f.plan(&todo, "Taxes", &[]).plan_id;
    let busy = stop_at(&f, &plan_id, Checkpoint::AfterMove);

    // The moved file, chosen again where it now is, gets a new record; it
    // still may not be planned away from the action's target location.
    let moved = f.register("Taxes/todo.txt");
    assert_ne!(moved.file_id, todo.file_id);
    let error = f.try_plan(&moved, "Notes", &[]).unwrap_err();
    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(busy.as_str()))
    );
}

#[test]
fn removing_folders_never_breaks_another_unsettled_action() {
    let mut f = Fixture::new();
    let todo = f.register("todo.txt");
    let plan_md = f.register("Notes/2026/plan.md");
    // The first run creates Notes/Archive and stops before its move intent.
    let first_plan = f.plan(&todo, "Notes", &["Archive"]).plan_id;
    let first = stop_at(&f, &first_plan, Checkpoint::BeforeMoveCheck);
    f.advance(1); // so the first action is checked first
    // The second run targets that folder and stops right before its move.
    let second_plan = f.plan(&plan_md, "Notes/Archive", &[]).plan_id;
    let second = stop_at(&f, &second_plan, Checkpoint::BeforeMove);

    f.restart();

    // The first action's folder is empty, but the second relies on it.
    let first_view = action(&f, &first);
    assert_eq!(first_view.status, ActionStatus::FailedNoMove);
    assert!(first_view.folders_removed.is_empty());
    assert!(f.path("Notes/Archive").is_dir());
    let second_view = action(&f, &second);
    assert_eq!(second_view.status, ActionStatus::FailedNoMove);
    assert_eq!(last_check_of(&f, &second).basis, Basis::StillAtSource);
}

// ---------------------------------------------------------------------------
// The same file or place through another root (#48)
// ---------------------------------------------------------------------------

/// Registers `relative` (a path in the fixture's folder) through another
/// root, `root_id`, which lies inside the fixture's folder.
fn register_through(f: &Fixture, root_id: &str, relative: &str) -> FileView {
    f.service()
        .register_file_with(root_id, &f.path(relative), &mut |_root, parent, name| {
            quick_read(parent, name, parent.id().dev)
        })
        .unwrap()
        .file
}

/// A plan for `file` into `folder` (relative, "" for the root itself) of
/// root `root_id`.
fn plan_into(
    f: &Fixture,
    file: &FileView,
    root_id: &str,
    folder: &str,
) -> Result<PlanView, AppError> {
    let catalog = f.service().list_folders(root_id).unwrap();
    let folder_id = catalog
        .folders
        .iter()
        .find(|listed| listed.relative_path == folder)
        .unwrap()
        .folder_id
        .clone();
    let outcome = f.service().build_plan(&PlanRequest {
        file_id: file.file_id.clone(),
        file_revision: file.file_revision,
        target_root_id: root_id.to_string(),
        operation_id: None,
        target: PlanTarget::Folder {
            catalog_id: catalog.catalog_id,
            folder_id,
            new_folders: Vec::new(),
        },
    })?;
    match outcome {
        BuildOutcome::Planned { plan } => Ok(plan),
        other => panic!("expected a plan, got {other:?}"),
    }
}

#[test]
fn a_nested_root_cannot_plan_the_file_of_an_unsettled_action() {
    // Codex's case: the file of an action that needs a decision, registered
    // again through a root nested in the first one, gets another file ID.
    let f = Fixture::new();
    let (action_id, _) = final_record_refused(&f);
    let nested = f
        .service()
        .authorize_root(&f.path("Taxes"))
        .unwrap()
        .root_id;
    let again = register_through(&f, &nested, "Taxes/todo.txt");
    let before = snapshot::snapshot(f.root.path());

    let error = plan_into(&f, &again, &f.root_id, "Notes").unwrap_err();

    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert_eq!(f.count("actions"), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
    // The original action is still checked as usual.
    assert_eq!(
        f.service().reconcile_action(&action_id).unwrap().status,
        ActionStatus::Committed
    );
}

#[test]
fn a_folder_authorized_again_cannot_plan_the_file_of_an_unsettled_action() {
    // The same folder under a new root ID, after a revocation.
    let f = Fixture::new();
    let (action_id, _) = final_record_refused(&f);
    f.service().revoke_root(&f.root_id).unwrap();
    let again_root = f.service().authorize_root(f.root.path()).unwrap().root_id;
    assert_ne!(again_root, f.root_id);
    let again = register_through(&f, &again_root, "Taxes/todo.txt");
    let before = snapshot::snapshot(f.root.path());

    let error = plan_into(&f, &again, &again_root, "Notes").unwrap_err();

    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert_eq!(f.count("actions"), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

#[test]
fn a_plan_aiming_at_an_unsettled_place_through_another_root_is_refused_at_confirmation() {
    // Built while the place was free, aiming at it through a nested root;
    // confirmed after another action left a file there that needs a decision.
    let f = Fixture::new();
    fs::write(f.path("Notes/todo.txt"), "another note\n").unwrap();
    let nested = f
        .service()
        .authorize_root(&f.path("Taxes"))
        .unwrap()
        .root_id;
    let other = f.register("Notes/todo.txt");
    let late = plan_into(&f, &other, &nested, "").unwrap();
    let (action_id, _) = final_record_refused(&f);
    let before = snapshot::snapshot(f.root.path());

    let error = f.apply(&late.plan_id).unwrap_err();

    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert_eq!(f.count("actions"), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

#[test]
fn cleanup_keeps_a_created_folder_that_became_the_root_of_another_unsettled_action() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_a = f.plan(&file, "Taxes", &["New"]).plan_id;
    let a = stop_at(&f, &plan_a, Checkpoint::BeforeMove);
    // Meanwhile the folder A created is authorized as a root of its own,
    // and another file's move into it stops half way too.
    let nested = f
        .service()
        .authorize_root(&f.path("Taxes/New"))
        .unwrap()
        .root_id;
    let other = f.register("Notes/2026/plan.md");
    let plan_b = plan_into(&f, &other, &nested, "").unwrap().plan_id;
    let b = stop_at(&f, &plan_b, Checkpoint::BeforeMove);

    let view = f.service().reconcile_action(&a).unwrap();

    assert_eq!(view.status, ActionStatus::FailedNoMove);
    assert!(
        f.path("Taxes/New").is_dir(),
        "the other action's root stays"
    );
    // The other action is still checked as usual.
    assert_eq!(
        f.service().reconcile_action(&b).unwrap().status,
        ActionStatus::FailedNoMove
    );
}

#[test]
fn the_file_of_an_unsettled_action_moved_by_hand_cannot_be_planned_again() {
    // Another place, so only the file object can tell: the same device,
    // inode and birth time as the file the undecided action is about.
    let f = Fixture::new();
    let (action_id, _) = final_record_refused(&f);
    fs::rename(f.path("Taxes/todo.txt"), f.path("Notes/todo.txt")).unwrap();
    let again = f.register("Notes/todo.txt");
    let before = snapshot::snapshot(f.root.path());

    // Into a folder that is neither end of the undecided action.
    let error = f.try_plan(&again, "Notes/2026", &[]).unwrap_err();

    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert_eq!(f.count("actions"), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

// ---------------------------------------------------------------------------
// The same place under a name APFS treats as the same (#55)
// ---------------------------------------------------------------------------

/// Whether this volume treats `a` and `b` as one name: a file made under
/// `a` in a scratch folder (outside every root) opens under `b`.
fn one_name_on_this_volume(a: &str, b: &str) -> bool {
    let scratch = tempfile::tempdir().unwrap();
    fs::write(scratch.path().join(a), "probe").unwrap();
    let made = fs::symlink_metadata(scratch.path().join(a)).unwrap().ino();
    fs::symlink_metadata(scratch.path().join(b)).is_ok_and(|found| found.ino() == made)
}

/// The undecided action moved the root's file, named `left`, to Taxes.
/// Another file, named `right`, waits in Incoming. Returns the action and
/// the other file.
fn unsettled_under(f: &Fixture, left: &str, right: &str) -> (String, FileView) {
    fs::rename(f.path("todo.txt"), f.path(left)).unwrap();
    let (action_id, _) = final_record_refused_for(f, left);
    fs::create_dir(f.path("Incoming")).unwrap();
    fs::write(f.path(&format!("Incoming/{right}")), "another file\n").unwrap();
    (action_id, f.register(&format!("Incoming/{right}")))
}

#[test]
fn another_file_cannot_take_the_place_of_an_unsettled_action_under_an_equivalent_name() {
    // Codex's case: the undecided action moved "café.txt" (é as one
    // character) out of the root. Another "café.txt" (e plus a combining
    // accent) must not be moved into that place: APFS treats both as one.
    let (nfc, nfd) = ("caf\u{e9}.txt", "cafe\u{301}.txt");
    assert!(one_name_on_this_volume(nfc, nfd));
    let f = Fixture::new();
    let (action_id, other) = unsettled_under(&f, nfc, nfd);
    let before = snapshot::snapshot(f.root.path());

    let error = plan_into(&f, &other, &f.root_id, "").unwrap_err();

    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert_eq!(f.count("actions"), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
    assert!(f.path(&format!("Incoming/{nfd}")).exists());
    // The original action is still checked as usual.
    assert_eq!(
        f.service().reconcile_action(&action_id).unwrap().status,
        ActionStatus::Committed
    );
}

#[test]
fn every_spelling_apfs_treats_as_the_same_name_keeps_the_place() {
    // Lower case alone would let each of these through.
    for (left, right) in [
        ("cafe\u{301}.txt", "caf\u{e9}.txt"), // the other way round
        ("stra\u{df}e.txt", "STRASSE.TXT"),   // ß and SS
        ("\u{fb01}le.txt", "file.txt"),       // the ligature ﬁ and fi
        ("\u{b5}m.txt", "\u{3bc}m.txt"),      // the micro sign and mu
        ("\u{3bf}\u{3c2}.txt", "\u{3bf}\u{3c3}.txt"), // final sigma and sigma
    ] {
        assert!(one_name_on_this_volume(left, right), "{left} / {right}");
        let f = Fixture::new();
        let (action_id, other) = unsettled_under(&f, left, right);

        let error = plan_into(&f, &other, &f.root_id, "").unwrap_err();

        assert_eq!(
            (error.code, error.action_id.as_deref()),
            (ErrorCode::UnresolvedAction, Some(action_id.as_str())),
            "{left} / {right}"
        );
        assert_eq!(f.count("actions"), 1);
    }
}

#[test]
fn a_name_apfs_treats_as_another_name_does_not_wait() {
    // The fullwidth "Ａ" is a different name on APFS: no reason to wait.
    let (left, right) = ("A.txt", "\u{ff21}.txt");
    assert!(!one_name_on_this_volume(left, right));
    let f = Fixture::new();
    let (_, other) = unsettled_under(&f, left, right);

    assert!(plan_into(&f, &other, &f.root_id, "").is_ok());
}

#[test]
fn a_plan_aiming_at_an_unsettled_place_under_an_equivalent_name_is_refused_at_confirmation() {
    // Built while Taxes had no "café.txt"; confirmed after an undecided
    // action left its "café.txt", spelled the other way, there.
    let f = Fixture::new();
    fs::create_dir(f.path("Incoming")).unwrap();
    fs::write(f.path("Incoming/cafe\u{301}.txt"), "another file\n").unwrap();
    let other = f.register("Incoming/cafe\u{301}.txt");
    let late = plan_into(&f, &other, &f.root_id, "Taxes").unwrap();
    fs::rename(f.path("todo.txt"), f.path("caf\u{e9}.txt")).unwrap();
    let (action_id, _) = final_record_refused_for(&f, "caf\u{e9}.txt");
    let before = snapshot::snapshot(f.root.path());

    let error = f.apply(&late.plan_id).unwrap_err();

    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert_eq!(f.count("actions"), 1);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

// ---------------------------------------------------------------------------
// #20: a revoked root and a swapped folder at the record check
// ---------------------------------------------------------------------------

/// The root was removed from Traeva while the action was unsettled: the
/// check cannot rely on it, so the action needs a decision and nothing is
/// moved or recorded as moved (O19).
#[test]
fn a_root_revoked_before_the_check_needs_a_decision() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMove);
    f.service().revoke_root(&f.root_id).unwrap();
    let before = snapshot::snapshot(f.root.path());

    f.restart();

    assert_eq!(
        outcome(&action(&f, &action_id)),
        (
            ActionStatus::NeedsUserDecision,
            Some(ErrorCode::ResultUnconfirmed)
        )
    );
    assert_eq!(
        found(&f, &action_id),
        (Found::NotChecked, Found::NotChecked)
    );
    assert_eq!(snapshot::snapshot(f.root.path()), before, "nothing moved");
    assert_eq!(organize_entries(&f, &file.file_id), 0);
    assert_eq!(recorded_path(&f, &file.file_id), "todo.txt");
}

/// A folder this action created (with a receipt) was swapped for another
/// empty folder of the same name: that one is not the action's, so it stays
/// (O23).
#[test]
fn a_swapped_folder_with_the_same_name_is_not_removed() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
    fs::rename(f.path("Notes/Archive"), f.path("Notes/Archive-old")).unwrap();
    fs::create_dir(f.path("Notes/Archive")).unwrap();

    f.restart();

    let view = action(&f, &action_id);
    assert_eq!(view.status, ActionStatus::FailedNoMove);
    assert_eq!(view.folders_created, ["Notes/Archive"]);
    assert!(view.folders_removed.is_empty());
    assert!(f.path("Notes/Archive").is_dir() && f.path("Notes/Archive-old").is_dir());
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        file.sha256
    );
}
