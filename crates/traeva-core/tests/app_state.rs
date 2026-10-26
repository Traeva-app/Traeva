//! What the window reads first and after every refresh (organize-spec.md
//! §9 `get_app_state`): roots, recent actions and every action waiting for a
//! decision. Reading it runs nothing. Synthetic temporary files only.

mod common;

use std::fs;

use common::setup::Setup;
use traeva_core::{ActionStatus, OrganizeService, StorageStatus};

#[test]
fn the_app_state_lists_roots_recent_actions_and_decisions_needed() {
    let s = Setup::new();
    let todo = s.register("todo.txt");
    let first = s
        .service()
        .apply_plan(&s.plan(&todo, "Taxes", &[]).plan_id)
        .unwrap();
    // The second run cannot save its final record: it waits for a decision.
    let plan_md = s.register("Notes/2026/plan.md");
    let plan = s.plan(&plan_md, "Taxes", &[]);
    s.refuse("BEFORE UPDATE OF status ON actions WHEN NEW.status = 'committed'");
    let second = s.service().apply_plan(&plan.plan_id).unwrap();
    s.allow_again();
    assert_eq!(second.status, ActionStatus::NeedsUserDecision);
    let events = s.count("action_events");

    let state = s.service().get_app_state().unwrap();

    assert!(matches!(state.storage, StorageStatus::Ready(_)));
    assert_eq!(state.roots.len(), 1);
    assert_eq!(state.roots[0].root_id, s.root_id);
    let recent: Vec<_> = state
        .recent_actions
        .iter()
        .map(|a| a.action_id.as_str())
        .collect();
    assert_eq!(
        recent,
        [second.action_id.as_str(), first.action_id.as_str()]
    );
    assert_eq!(state.decisions_needed, std::slice::from_ref(&second));
    assert_eq!(s.count("action_events"), events, "reading changed nothing");
}

#[test]
fn a_blocked_storage_lists_nothing() {
    let s = Setup::new();
    // A second service on the same app data is blocked by the process lock.
    let second = OrganizeService::start(s.app.path().join("Traeva"));
    let state = second.get_app_state().unwrap();
    assert!(matches!(state.storage, StorageStatus::Blocked { .. }));
    assert!(state.roots.is_empty());
    assert!(state.recent_actions.is_empty() && state.decisions_needed.is_empty());
    assert!(fs::metadata(s.path("todo.txt")).is_ok());
}
