//! Record checks through the public service API, with the real stable reads
//! (organize-spec.md §7; O18-R, O19, O20, O35): actions a run could not
//! settle are checked again at the next start or on request, and a check
//! never moves anything. A refused database write is a test trigger, as in
//! tests/execute.rs. Synthetic temporary files only.

mod common;

use std::fs;

use common::setup::{Setup, steps};
use common::*;
use traeva_core::{ActionStatus, ErrorCode};

fn organize_entries(s: &Setup, file_id: &str) -> i64 {
    s.db()
        .query_row(
            "SELECT count(*) FROM path_history WHERE file_id = ?1 AND reason = 'organize'",
            [file_id],
            |r| r.get(0),
        )
        .unwrap()
}

fn recorded(s: &Setup, file_id: &str) -> (String, i64) {
    s.db()
        .query_row(
            "SELECT relative_path, record_revision FROM files WHERE file_id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

/// A run whose final record cannot be saved: the file moved, and the action
/// waits for a decision.
fn final_record_refused(s: &Setup) -> (String, String, i64) {
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);
    s.refuse("BEFORE UPDATE OF status ON actions WHEN NEW.status = 'committed'");
    let action = s.service().apply_plan(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::NeedsUserDecision);
    s.allow_again();
    (action.action_id, file.file_id, file.file_revision)
}

#[test]
fn a_run_that_could_not_record_its_result_is_settled_at_the_next_start() {
    let mut s = Setup::new();
    let file = s.register("todo.txt");
    let plan = s.plan(&file, "Taxes", &[]);
    s.refuse("BEFORE UPDATE OF status ON actions");
    let error = s.service().apply_plan(&plan.plan_id).unwrap_err();
    let action_id = error.action_id.expect("the action it could not finish");
    s.allow_again();
    let before = snapshot::snapshot(s.root_dir.path());

    s.restart();

    let action = s.service().get_action(&action_id).unwrap();
    assert_eq!(action.status, ActionStatus::FailedNoMove);
    assert_eq!(action.problem.unwrap().code, ErrorCode::RunInterrupted);
    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "the check changed nothing on disk"
    );
    assert_eq!(
        s.journal(&action_id),
        steps(&[("confirm", "result"), ("reconcile", "result")])
    );
    // Settled: the same file can be planned and moved again.
    let again = s.plan(&file, "Taxes", &[]);
    assert_eq!(
        s.service().apply_plan(&again.plan_id).unwrap().status,
        ActionStatus::Committed
    );
}

#[test]
fn a_move_whose_final_record_failed_is_committed_once_at_the_next_start() {
    let mut s = Setup::new();
    let (action_id, file_id, revision) = final_record_refused(&s);
    let before = snapshot::snapshot(s.root_dir.path());

    s.restart();

    let action = s.service().get_action(&action_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    assert!(action.recovered_at_ms.is_some() && action.problem.is_none());
    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "no second move"
    );
    assert_eq!(
        recorded(&s, &file_id),
        ("Taxes/todo.txt".to_string(), revision + 1)
    );
    assert_eq!(organize_entries(&s, &file_id), 1);
    assert_eq!(
        s.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("settle", "result"),
            ("move", "result"),
            ("reconcile", "result"),
        ])
    );

    // Starting again adds nothing.
    let events = s.count("action_events");
    s.restart();
    assert_eq!(s.service().get_action(&action_id).unwrap(), action);
    assert_eq!(
        (
            s.count("action_events"),
            s.count("actions"),
            organize_entries(&s, &file_id)
        ),
        (events, 1, 1)
    );
}

#[test]
fn reconcile_action_settles_one_action_on_request() {
    let s = Setup::new();
    let (action_id, file_id, _) = final_record_refused(&s);

    let action = s.service().reconcile_action(&action_id).unwrap();

    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(organize_entries(&s, &file_id), 1);
    let events = s.count("action_events");
    assert_eq!(s.service().reconcile_action(&action_id).unwrap(), action);
    assert_eq!(
        s.count("action_events"),
        events,
        "a settled action is only reported"
    );
}

#[test]
fn an_action_waiting_for_a_decision_blocks_plans_at_its_locations() {
    let s = Setup::new();
    fs::write(s.path("Notes/todo.txt"), "another list\n").unwrap();
    let other = s.register("Notes/todo.txt");
    let (action_id, _, _) = final_record_refused(&s);

    // todo.txt (the action's old location) is free on disk now, but it is
    // still part of an action whose result is not settled.
    let error = s.build(&other, "", &[]).unwrap_err();
    assert_eq!(
        (error.code, error.action_id.as_deref()),
        (ErrorCode::UnresolvedAction, Some(action_id.as_str()))
    );
    assert!(!s.path("todo.txt").exists());
}
