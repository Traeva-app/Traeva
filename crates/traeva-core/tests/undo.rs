//! Undo through the public service API, with the real stable reads
//! (organize-spec.md §8; O21, O22, O24, O18-R for an interrupted undo): the
//! same file goes back with the same bytes and file_id, conflicts are
//! refused, repeats change nothing. Synthetic temporary files only.

mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use common::setup::{Setup, steps};
use common::*;
use sha2::{Digest, Sha256};
use traeva_core::{ActionStatus, ActionView, ErrorCode, PlanKind, UndoPreview, UndoStatus};

fn sha256_of(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn organize(s: &Setup, file: &str, folder: &str, new: &[&str]) -> ActionView {
    let view = s.register(file);
    let plan = s.plan(&view, folder, new);
    let action = s.service().apply_plan(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    action
}

fn undo_plan_id(s: &Setup, action_id: &str) -> String {
    match s.service().preview_undo(action_id).unwrap() {
        UndoPreview::Planned { plan } => plan.plan_id,
        other => panic!("expected an undo plan, got {other:?}"),
    }
}

fn reasons(s: &Setup, file_id: &str) -> Vec<(String, String)> {
    let db = s.db();
    let mut stmt = db
        .prepare(
            "SELECT reason, to_relative_path FROM path_history
             WHERE file_id = ?1 ORDER BY recorded_at_ms, reason",
        )
        .unwrap();
    stmt.query_map([file_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn organize_then_undo_puts_the_same_file_back() {
    let s = Setup::new();
    let sha = sha256_of(&s.path("todo.txt"));
    let inode = fs::metadata(s.path("todo.txt")).unwrap().ino();
    let organized = organize(&s, "todo.txt", "Notes", &["Archive"]);
    let before = snapshot::snapshot(s.root_dir.path());

    let plan_id = undo_plan_id(&s, &organized.action_id);
    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "a preview writes nothing"
    );
    let undo = s.service().apply_plan(&plan_id).unwrap();

    assert_eq!(
        (undo.kind, undo.status),
        (PlanKind::Undo, ActionStatus::Committed)
    );
    assert_eq!(sha256_of(&s.path("todo.txt")), sha, "the same bytes");
    assert_eq!(
        fs::metadata(s.path("todo.txt")).unwrap().ino(),
        inode,
        "the same file"
    );
    assert!(
        !s.path("Notes/Archive").exists(),
        "its own empty folder is gone"
    );
    let original = s.service().get_action(&organized.action_id).unwrap();
    assert_eq!(
        (original.status, original.undo_status),
        (ActionStatus::Undone, UndoStatus::Undone)
    );
    let file = s.register("todo.txt");
    assert_eq!(file.file_id, organized.file_id, "the same file_id");
    assert_eq!(
        reasons(&s, &organized.file_id)
            .into_iter()
            .map(|(reason, _)| reason)
            .collect::<Vec<_>>(),
        ["intake", "organize", "undo"]
    );
    assert_eq!(
        s.journal(&undo.action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("move", "result"),
            ("cleanup", "intent"),
            ("cleanup", "result"),
        ])
    );
}

#[test]
fn after_a_restart_an_undo_works_once_and_then_only_reports() {
    let mut s = Setup::new();
    let bytes = sha256_of(&s.path("todo.txt"));
    let inode = fs::metadata(s.path("todo.txt")).unwrap().ino();
    let organized = organize(&s, "todo.txt", "Notes", &["Archive", "Q1"]);
    s.restart();

    let plan_id = undo_plan_id(&s, &organized.action_id);
    let undo = s.service().apply_plan(&plan_id).unwrap();
    assert_eq!(undo.status, ActionStatus::Committed);
    // The same object, the same bytes, the same file_id, and the two new
    // folders gone again (O21).
    assert_eq!(sha256_of(&s.path("todo.txt")), bytes);
    assert_eq!(fs::metadata(s.path("todo.txt")).unwrap().ino(), inode);
    assert_eq!(s.register("todo.txt").file_id, organized.file_id);
    assert!(!s.path("Notes/Archive").exists());
    assert_eq!(undo.folders_removed, ["Notes/Archive/Q1", "Notes/Archive"]);
    let records = (s.count("actions"), s.count("path_history"));

    match s.service().preview_undo(&organized.action_id).unwrap() {
        UndoPreview::AlreadyUndone { undo: found } => assert_eq!(found, undo),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.service().apply_plan(&plan_id).unwrap(), undo);
    assert_eq!((s.count("actions"), s.count("path_history")), records);
}

#[test]
fn an_edited_file_or_a_taken_old_name_is_an_undo_conflict() {
    let s = Setup::new();
    let organized = organize(&s, "todo.txt", "Taxes", &[]);
    fs::write(s.path("Taxes/todo.txt"), "buy milk and eggs\n").unwrap();
    let error = s.service().preview_undo(&organized.action_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::UndoConflict);
    assert_eq!(
        fs::read(s.path("Taxes/todo.txt")).unwrap(),
        b"buy milk and eggs\n",
        "no old content comes back"
    );

    let s = Setup::new();
    let organized = organize(&s, "todo.txt", "Taxes", &[]);
    fs::write(s.path("todo.txt"), "a new list\n").unwrap();
    assert_eq!(
        s.service()
            .preview_undo(&organized.action_id)
            .unwrap_err()
            .code,
        ErrorCode::UndoConflict
    );
    assert_eq!(fs::read(s.path("todo.txt")).unwrap(), b"a new list\n");
    assert_eq!(s.count("plans"), 1, "no undo plan was made");
}

#[test]
fn an_undo_whose_final_record_failed_is_completed_at_the_next_start() {
    let mut s = Setup::new();
    let organized = organize(&s, "todo.txt", "Taxes", &[]);
    let plan_id = undo_plan_id(&s, &organized.action_id);
    s.refuse("BEFORE UPDATE OF status ON actions WHEN NEW.status = 'undone'");
    let undo = s.service().apply_plan(&plan_id).unwrap();
    assert_eq!(undo.status, ActionStatus::NeedsUserDecision);
    s.allow_again();

    s.restart();

    assert_eq!(
        s.service().get_action(&undo.action_id).unwrap().status,
        ActionStatus::Committed
    );
    assert_eq!(
        s.service().get_action(&organized.action_id).unwrap().status,
        ActionStatus::Undone
    );
    assert_eq!(
        reasons(&s, &organized.file_id)
            .iter()
            .filter(|(reason, _)| reason == "undo")
            .count(),
        1
    );
    assert!(s.path("todo.txt").exists());
}
