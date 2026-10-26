//! Confirming and running plans through the public service API, with the
//! real stable reads (organize-spec.md §2.3, §6; O09, O17, O19, O20): a real
//! move and its records, new folders, double clicks, a restart, and a
//! database that refuses a write at each step. Hashes, database rows and
//! the journal order are checked directly, not only the returned status.
//! Synthetic temporary files only.
//!
//! A refused write is real: a test trigger in the temporary database aborts
//! one kind of write, the way a full disk would. The app has no switch for it.

mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::thread;

use common::setup::{Setup, steps};
use common::*;
use sha2::{Digest, Sha256};
use traeva_core::{ActionStatus, BuildOutcome, ErrorCode, PlanStatus};

fn sha256_of(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One journal entry's detail, e.g. ("settle", "result").
fn detail(s: &Setup, action_id: &str, step: &str, phase: &str) -> Vec<serde_json::Value> {
    let db = s.db();
    let mut stmt = db
        .prepare(
            "SELECT detail_json FROM action_events
             WHERE action_id = ?1 AND step = ?2 AND phase = ?3 ORDER BY sequence",
        )
        .unwrap();
    stmt.query_map((action_id, step, phase), |r| r.get::<_, String>(0))
        .unwrap()
        .map(|json| serde_json::from_str(&json.unwrap()).unwrap())
        .collect()
}

/// Where the database says the file is: path, revision and presence.
fn record(s: &Setup, file_id: &str) -> (String, i64, String) {
    s.db()
        .query_row(
            "SELECT relative_path, record_revision, presence FROM files WHERE file_id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

fn organize_entries(s: &Setup, file_id: &str) -> i64 {
    s.db()
        .query_row(
            "SELECT count(*) FROM path_history WHERE file_id = ?1 AND reason = 'organize'",
            [file_id],
            |r| r.get(0),
        )
        .unwrap()
}

// ---------------------------------------------------------------------------
// Success, exactly once
// ---------------------------------------------------------------------------

#[test]
fn organizing_moves_the_file_and_records_it_exactly_once() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let inode = fs::metadata(s.path("todo.txt")).unwrap().ino();
    let plan = s.plan(&file, "Taxes", &[]);

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(
        (action.plan_id.as_str(), action.file_id.as_str()),
        (plan.plan_id.as_str(), file.file_id.as_str())
    );
    assert_eq!(
        (
            action.source.relative_path.as_str(),
            action.target.relative_path.as_str()
        ),
        ("todo.txt", "Taxes/todo.txt")
    );
    assert!(action.committed_at_ms.is_some() && action.problem.is_none());
    assert!(action.folders_created.is_empty());

    // The file: moved (same inode), same bytes, extended attribute kept.
    let moved = s.path("Taxes/todo.txt");
    assert!(!s.path("todo.txt").exists());
    assert_eq!(fs::metadata(&moved).unwrap().ino(), inode);
    assert_eq!(sha256_of(&moved), file.sha256);
    assert_eq!(
        xattr::get(&moved, "com.traeva.test").unwrap().as_deref(),
        Some(&b"synthetic"[..])
    );

    // The records: new location, one organize entry, the journal in order.
    assert_eq!(
        record(&s, &file.file_id),
        (
            "Taxes/todo.txt".into(),
            file.file_revision + 1,
            "present".into()
        )
    );
    let history: Vec<(String, Option<String>, String, Option<String>)> = s
        .db()
        .prepare(
            "SELECT reason, from_relative_path, to_relative_path, action_id FROM path_history
             WHERE file_id = ?1 ORDER BY reason",
        )
        .unwrap()
        .query_map([&file.file_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        history,
        [
            ("intake".into(), None, "todo.txt".into(), None),
            (
                "organize".into(),
                Some("todo.txt".into()),
                "Taxes/todo.txt".into(),
                Some(action.action_id.clone())
            ),
        ]
    );
    assert_eq!(
        s.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("move", "result"),
        ])
    );
    assert_eq!(
        s.service().get_plan(&plan.plan_id).unwrap().status,
        PlanStatus::Consumed
    );

    // Asking again (a double click, a retry after a timeout) returns the
    // same action and changes nothing.
    assert_eq!(s.service().apply_plan(&plan.plan_id).unwrap(), action);
    assert_eq!(s.service().get_action(&action.action_id).unwrap(), action);
    assert_eq!(
        (
            s.count("actions"),
            s.count("path_history"),
            s.count("action_events")
        ),
        (1, 2, 3)
    );
}

#[test]
fn new_folders_are_created_first_and_each_gets_a_receipt() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Notes", &["2026 taxes", "Q1"]);

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(
        action.folders_created,
        ["Notes/2026 taxes", "Notes/2026 taxes/Q1"]
    );
    assert_eq!(
        sha256_of(&s.path("Notes/2026 taxes/Q1/todo.txt")),
        file.sha256
    );
    assert_eq!(
        s.journal(&action.action_id),
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
    for receipt in detail(&s, &action.action_id, "mkdir", "result") {
        let on_disk = fs::metadata(s.path(receipt["path"].as_str().unwrap())).unwrap();
        assert_eq!(receipt["ino"].as_u64(), Some(on_disk.ino()));
        assert_eq!(receipt["dev"].as_i64(), Some(on_disk.dev() as i64));
    }
}

#[test]
fn a_double_click_runs_the_plan_only_once() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);

    let (first, second) = thread::scope(|scope| {
        let a = scope.spawn(|| s.service().apply_plan(&plan.plan_id));
        let b = scope.spawn(|| s.service().apply_plan(&plan.plan_id));
        (a.join().unwrap().unwrap(), b.join().unwrap().unwrap())
    });

    // The later click may see the action while it is still running.
    assert_eq!(first.action_id, second.action_id);
    let action = s.service().get_action(&first.action_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(
        (s.count("actions"), organize_entries(&s, &file.file_id)),
        (1, 1)
    );
    assert_eq!(sha256_of(&s.path("Taxes/todo.txt")), file.sha256);
}

#[test]
fn an_action_reads_the_same_after_a_restart() {
    let mut s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);
    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    s.restart();

    assert_eq!(s.service().get_action(&action.action_id).unwrap(), action);
    assert_eq!(s.service().apply_plan(&plan.plan_id).unwrap(), action);
    assert_eq!(
        s.service().get_plan(&plan.plan_id).unwrap().status,
        PlanStatus::Consumed,
        "a restart ends unused plans only"
    );
}

#[test]
fn a_record_of_a_file_gone_from_the_target_name_becomes_missing() {
    let s = Setup::new();
    fs::write(s.path("Taxes/todo.txt"), "an older list\n").unwrap();
    let older = s.register("Taxes/todo.txt");
    fs::remove_file(s.path("Taxes/todo.txt")).unwrap(); // still recorded as present
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(record(&s, &older.file_id).2, "missing");
    assert_eq!(
        record(&s, &file.file_id),
        (
            "Taxes/todo.txt".into(),
            file.file_revision + 1,
            "present".into()
        )
    );
}

#[test]
fn a_plan_into_another_root_records_both_roots() {
    let s = Setup::new();
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    fs::create_dir(other.path().join("Archive")).unwrap();
    let other_root = s.service().authorize_root(other.path()).unwrap().root_id;
    let file = s.register("todo.txt");
    let catalog = s.service().list_folders(&other_root).unwrap();
    let plan = match s
        .service()
        .build_plan(&s.request(&file, &catalog, "Archive", &[]))
        .unwrap()
    {
        BuildOutcome::Planned { plan } => plan,
        other => panic!("{other:?}"),
    };

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(
        sha256_of(&other.path().join("Archive/todo.txt")),
        file.sha256
    );
    let (from, to): (String, String) = s
        .db()
        .query_row(
            "SELECT from_root_id, to_root_id FROM path_history WHERE reason = 'organize'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((from, to), (s.root_id.clone(), other_root.clone()));
    let root_now: String = s
        .db()
        .query_row(
            "SELECT root_id FROM files WHERE file_id = ?1",
            [&file.file_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(root_now, other_root);
}

// ---------------------------------------------------------------------------
// The database refuses a write
// ---------------------------------------------------------------------------

#[test]
fn a_failed_confirmation_changes_nothing_and_the_plan_stays_usable() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Notes", &["Archive"]);
    let before = snapshot::snapshot(s.root_dir.path());
    s.refuse("BEFORE INSERT ON actions");

    let error = s.service().apply_plan(&plan.plan_id).unwrap_err();

    assert_eq!(error.code, ErrorCode::DatabaseUnavailable);
    assert_eq!((s.count("actions"), s.count("action_events")), (0, 0));
    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "no folder, no move"
    );
    assert!(s.service().get_plan(&plan.plan_id).unwrap().can_confirm);

    // Nothing used the plan, so once the database works it can still run.
    s.allow_again();
    let action = s.service().apply_plan(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
}

/// The other half of the confirmation transaction, its journal entry, cannot
/// be saved: the whole transaction rolls back (O17, O20).
#[test]
fn a_confirmation_whose_journal_entry_fails_changes_nothing() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Notes", &["Archive"]);
    let before = snapshot::snapshot(s.root_dir.path());
    s.refuse("BEFORE INSERT ON action_events WHEN NEW.step = 'confirm'");

    let error = s.service().apply_plan(&plan.plan_id).unwrap_err();

    assert_eq!(error.code, ErrorCode::DatabaseUnavailable);
    assert_eq!((s.count("actions"), s.count("action_events")), (0, 0));
    assert_eq!(snapshot::snapshot(s.root_dir.path()), before);
    assert!(s.service().get_plan(&plan.plan_id).unwrap().can_confirm);
}

#[test]
fn no_folder_is_created_without_its_saved_intent() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Notes", &["Archive"]);
    s.refuse("BEFORE INSERT ON action_events WHEN NEW.step = 'mkdir' AND NEW.phase = 'intent'");

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::FailedNoMove);
    assert_eq!(action.problem.unwrap().code, ErrorCode::DatabaseUnavailable);
    assert!(!s.path("Notes/Archive").exists());
    assert_eq!(sha256_of(&s.path("todo.txt")), file.sha256);
    assert_eq!(
        s.journal(&action.action_id),
        steps(&[("confirm", "result"), ("settle", "result")])
    );
}

#[test]
fn no_move_happens_without_its_saved_intent() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Notes", &["Archive", "Q1"]);
    s.refuse("BEFORE INSERT ON action_events WHEN NEW.step = 'move' AND NEW.phase = 'intent'");

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::FailedNoMove);
    // The folders it created stay, each with its receipt; the file did not move.
    assert_eq!(
        action.folders_created,
        ["Notes/Archive", "Notes/Archive/Q1"]
    );
    assert_eq!(fs::read_dir(s.path("Notes/Archive/Q1")).unwrap().count(), 0);
    assert_eq!(sha256_of(&s.path("todo.txt")), file.sha256);
    assert_eq!(
        s.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("mkdir", "intent"),
            ("mkdir", "result"),
            ("settle", "result"),
        ])
    );
}

#[test]
fn a_folder_whose_receipt_cannot_be_saved_is_not_claimed() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Notes", &["Archive"]);
    s.refuse("BEFORE INSERT ON action_events WHEN NEW.step = 'mkdir' AND NEW.phase = 'result'");

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::FailedNoMove);
    // The folder was created, but without a saved receipt it is not Traeva's.
    assert!(s.path("Notes/Archive").is_dir());
    assert!(action.folders_created.is_empty());
    assert_eq!(
        s.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("settle", "result"),
        ])
    );
    assert_eq!(sha256_of(&s.path("todo.txt")), file.sha256);
}

#[test]
fn a_failed_final_record_needs_a_decision_and_the_move_never_repeats() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);
    s.refuse("BEFORE UPDATE OF status ON actions WHEN NEW.status = 'committed'");

    let action = s.service().apply_plan(&plan.plan_id).unwrap();

    assert_eq!(action.status, ActionStatus::NeedsUserDecision);
    assert_eq!(
        action.problem.as_ref().unwrap().code,
        ErrorCode::ResultUnconfirmed
    );
    // The file did move, with the same bytes; nothing moved it back.
    assert!(!s.path("todo.txt").exists());
    assert_eq!(sha256_of(&s.path("Taxes/todo.txt")), file.sha256);
    // The final transaction left nothing behind: no new location, no
    // organize entry, no move result. What was found is in the journal.
    assert_eq!(record(&s, &file.file_id).0, "todo.txt");
    assert_eq!(organize_entries(&s, &file.file_id), 0);
    assert_eq!(
        s.journal(&action.action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("settle", "result"),
        ])
    );
    let found = &detail(&s, &action.action_id, "settle", "result")[0];
    assert_eq!(
        (found["source_now"].as_str(), found["target_now"].as_str()),
        (Some("nothing"), Some("expected_file"))
    );

    // Even with the database working again, asking again only reports.
    s.allow_again();
    assert_eq!(s.service().apply_plan(&plan.plan_id).unwrap(), action);
    assert_eq!(s.count("actions"), 1);
    assert_eq!(
        s.build(&file, "Notes", &[]).unwrap_err().code,
        ErrorCode::UnresolvedAction
    );
}

#[test]
fn when_even_the_failure_cannot_be_recorded_the_storage_error_is_returned() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);
    let before = snapshot::snapshot(s.root_dir.path());
    s.refuse("BEFORE UPDATE OF status ON actions");

    let error = s.service().apply_plan(&plan.plan_id).unwrap_err();

    assert_eq!(error.code, ErrorCode::DatabaseUnavailable);
    let action_id = error.action_id.expect("the action it could not finish");
    s.allow_again();
    // The database says what was really saved: confirmed, nothing after.
    let action = s.service().get_action(&action_id).unwrap();
    assert_eq!(action.status, ActionStatus::Prepared);
    assert_eq!(s.journal(&action_id), steps(&[("confirm", "result")]));
    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "nothing changed on disk"
    );
    // The plan is used and the file waits for the record check (#10).
    assert_eq!(s.service().apply_plan(&plan.plan_id).unwrap(), action);
    assert_eq!(
        s.build(&file, "Notes", &[]).unwrap_err().code,
        ErrorCode::UnresolvedAction
    );
}

#[test]
fn ids_are_checked() {
    let s = Setup::new();
    let unknown = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";
    assert_eq!(
        s.service().get_action("not-an-id").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        s.service().get_action(unknown).unwrap_err().code,
        ErrorCode::UnknownAction
    );
    assert_eq!(
        s.service().apply_plan("not-an-id").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        s.service().apply_plan(unknown).unwrap_err().code,
        ErrorCode::UnknownPlan
    );

    // A real ID of the wrong kind is unknown, never another object (O32).
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);
    let action = s.service().apply_plan(&plan.plan_id).unwrap();
    assert_eq!(
        s.service().apply_plan(&action.action_id).unwrap_err().code,
        ErrorCode::UnknownPlan
    );
    assert_eq!(
        s.service().get_plan(&action.action_id).unwrap_err().code,
        ErrorCode::UnknownPlan
    );
    assert_eq!(
        s.service().get_action(&plan.plan_id).unwrap_err().code,
        ErrorCode::UnknownAction
    );
    assert_eq!(
        s.service().get_action(&file.file_id).unwrap_err().code,
        ErrorCode::UnknownAction
    );
    assert_eq!(
        s.service().preview_undo(&plan.plan_id).unwrap_err().code,
        ErrorCode::UnknownAction
    );
    assert_eq!(
        s.service()
            .get_operation(&action.action_id)
            .unwrap_err()
            .code,
        ErrorCode::UnknownOperation
    );
    assert_eq!(s.count("actions"), 1);
}
