//! Plan tests that need the clock or the file reads under control: expiry,
//! status rules, restart, and every pre-execution check (①–⑦).
//! Synthetic temporary files only.

use std::fs;

use super::*;
use crate::contracts::{AppError, ErrorCode, PlanLifecycle};
use crate::observe::{self, ReadLimits, read_stable_with};
use crate::test_fixture::{Fixture, START_MS, edit_keeping_size_and_mtime};
use crate::test_snapshot as snapshot;

/// Everything except the lifecycle columns: must never change.
fn payload(mut row: PlanRow) -> PlanRow {
    row.lifecycle = PlanLifecycle::Open;
    row.lifecycle_changed_at_ms = None;
    row.lifecycle_reason = None;
    row.action_id = None;
    row
}

fn expect_refused(result: Result<Revalidation, AppError>) -> AppError {
    match result {
        Ok(other) => panic!("expected a refusal, got {other:?}"),
        Err(error) => error,
    }
}

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

#[test]
fn a_preview_lasts_15_minutes_and_a_backwards_clock_counts_as_expired() {
    let t = START_MS;
    assert!(!is_expired(t, t));
    assert!(!is_expired(t, t + PREVIEW_LIFETIME_MS - 1));
    assert!(is_expired(t, t + PREVIEW_LIFETIME_MS));
    assert!(is_expired(t, t - 1));
}

#[test]
fn status_is_worked_out_in_a_fixed_order() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan = f.stored(&f.plan(&file, "Taxes", &[]).plan_id);
    let now = plan.created_at_ms;
    assert_eq!(plan_status(&plan, true, now), PlanStatus::Confirmable);
    assert_eq!(
        plan_status(&plan, true, now + PREVIEW_LIFETIME_MS),
        PlanStatus::Expired
    );
    assert_eq!(plan_status(&plan, false, now), PlanStatus::RootRevoked);
    for (lifecycle, status) in [
        (PlanLifecycle::Cancelled, PlanStatus::Cancelled),
        (PlanLifecycle::Superseded, PlanStatus::Superseded),
        (PlanLifecycle::Invalidated, PlanStatus::Invalidated),
    ] {
        let closed = PlanRow {
            lifecycle,
            ..plan.clone()
        };
        assert_eq!(plan_status(&closed, true, now), status);
    }
    // A used plan is reported as used, whatever else is true.
    let used = PlanRow {
        action_id: Some("a".into()),
        lifecycle: PlanLifecycle::Cancelled,
        ..plan
    };
    assert_eq!(
        plan_status(&used, false, now + PREVIEW_LIFETIME_MS),
        PlanStatus::Consumed
    );
}

#[test]
fn new_folder_names_are_checked_never_changed() {
    let names = |list: &[&str]| list.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    assert_eq!(parse_new_folders(&names(&[])).unwrap().len(), 0);
    assert_eq!(
        parse_new_folders(&names(&["2026", "Q1", "Receipts"]))
            .unwrap()
            .len(),
        3
    );
    assert!(
        parse_new_folders(&names(&["a", "b", "c", "d"]))
            .unwrap_err()
            .contains("At most 3")
    );
    for bad in ["a/b", ".hidden", "x:y", "", " pad", "Tools.app", ".."] {
        assert!(parse_new_folders(&names(&[bad])).is_err(), "{bad:?}");
    }
}

// ---------------------------------------------------------------------------
// The pre-execution check
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_plan_passes_every_check_and_nothing_is_written() {
    let f = Fixture::new();
    let before = snapshot::snapshot(f.root.path());
    let file = f.register("todo.txt");
    let view = f.plan(&file, "Notes", &["2026 taxes", "Q1"]);
    f.service().get_plan(&view.plan_id).unwrap();

    let ready = match f.revalidate(&view.plan_id).unwrap() {
        Revalidation::Ready(ready) => ready,
        other => panic!("{other:?}"),
    };
    assert_eq!(ready.plan.plan_id, view.plan_id);
    assert_eq!(ready.source.sha256, file.sha256);
    assert_eq!(
        ready.source_root.id(),
        ready.target_root.id(),
        "one root here"
    );
    assert_eq!(
        ready
            .target_chain
            .iter()
            .map(|f| f.name())
            .collect::<Vec<_>>(),
        ["Notes"]
    );
    assert!(ready.source_chain.is_empty());
    assert_eq!(f.stored(&view.plan_id).lifecycle, PlanLifecycle::Open);
    drop(ready);

    // The production read (with its real 350 ms pause) agrees.
    assert!(matches!(
        f.service()
            .revalidate_plan_with(&view.plan_id, &mut observe::read_stable),
        Ok(Revalidation::Ready(_))
    ));
    assert_eq!(
        snapshot::snapshot(f.root.path()),
        before,
        "planning and checking write nothing"
    );
}

#[test]
fn an_expired_plan_is_refused_and_its_lifecycle_is_not_changed() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    f.advance(PREVIEW_LIFETIME_MS - 1);
    assert!(f.service().get_plan(&plan_id).unwrap().can_confirm);
    f.advance(1);
    let view = f.service().get_plan(&plan_id).unwrap();
    assert_eq!(
        (view.status, view.can_confirm),
        (PlanStatus::Expired, false)
    );
    assert_eq!(
        expect_refused(f.revalidate(&plan_id)).code,
        ErrorCode::StalePlan
    );
    assert_eq!(
        f.stored(&plan_id).lifecycle,
        PlanLifecycle::Open,
        "expiry is worked out, not stored"
    );
}

#[test]
fn a_clock_set_back_makes_a_plan_expire() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    f.advance(-1);
    assert_eq!(
        f.service().get_plan(&plan_id).unwrap().status,
        PlanStatus::Expired
    );
}

#[test]
fn an_equal_length_edit_with_restored_mtime_invalidates_the_plan() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let before = f.stored(&plan_id);
    let mtime = fs::metadata(f.path("todo.txt"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(f.path("todo.txt"), "buy mild\n").unwrap(); // same length as "buy milk\n"
    fs::File::options()
        .write(true)
        .open(f.path("todo.txt"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();

    let error = expect_refused(f.revalidate(&plan_id));

    assert_eq!(error.code, ErrorCode::FileChanged);
    assert_eq!(error.plan_id.as_deref(), Some(plan_id.as_str()));
    let after = f.stored(&plan_id);
    assert_eq!(after.lifecycle, PlanLifecycle::Invalidated);
    assert_eq!(
        after.lifecycle_reason.as_deref(),
        Some("the file content changed")
    );
    assert_eq!(payload(after), payload(before), "the payload never changes");
}

#[test]
fn an_added_attribute_alone_still_plans_but_a_hidden_edit_does_not() {
    // macOS and other apps add attributes (com.apple.macl and the like): only
    // ctime moves and the bytes decide. An edit that restores mtime also moves
    // only ctime, and is refused.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    xattr::set(f.path("todo.txt"), "com.traeva.test", b"1").unwrap();
    let plan = f.plan(&file, "Taxes", &[]);
    assert!(plan.can_confirm);
    assert!(matches!(
        f.revalidate(&plan.plan_id),
        Ok(Revalidation::Ready(_))
    ));

    edit_keeping_size_and_mtime(&f.path("todo.txt"), "buy mild\n");
    let error = f.try_plan(&file, "Taxes", &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanged);
}

#[test]
fn a_moved_or_replaced_file_invalidates_the_plan() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let moved = f.plan(&file, "Taxes", &[]).plan_id;
    fs::rename(f.path("todo.txt"), f.path("done.txt")).unwrap();
    assert_eq!(
        expect_refused(f.revalidate(&moved)).code,
        ErrorCode::FileChanged
    );
    assert_eq!(f.stored(&moved).lifecycle, PlanLifecycle::Invalidated);

    // Same name, same bytes, but another object (an atomic save).
    fs::rename(f.path("done.txt"), f.path("todo.txt")).unwrap();
    let file = f.register("todo.txt");
    let replaced = f.plan(&file, "Taxes", &[]).plan_id;
    fs::write(f.path("todo.tmp"), "buy milk\n").unwrap();
    fs::rename(f.path("todo.tmp"), f.path("todo.txt")).unwrap();
    assert_eq!(
        expect_refused(f.revalidate(&replaced)).code,
        ErrorCode::FileChanged
    );
    assert_eq!(
        f.stored(&replaced).lifecycle_reason.as_deref(),
        Some("the file was replaced")
    );
}

#[test]
fn a_target_taken_after_the_preview_invalidates_the_plan() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    fs::write(f.path("Taxes/todo.txt"), "someone else's file").unwrap();

    assert_eq!(
        expect_refused(f.revalidate(&plan_id)).code,
        ErrorCode::TargetConflict
    );
    assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Invalidated);
    assert_eq!(
        fs::read(f.path("Taxes/todo.txt")).unwrap(),
        b"someone else's file"
    );
}

#[test]
fn a_folder_to_create_made_by_someone_else_invalidates_the_plan() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive", "2026"]).plan_id;
    fs::create_dir(f.path("Notes/Archive")).unwrap();

    assert_eq!(
        expect_refused(f.revalidate(&plan_id)).code,
        ErrorCode::TargetConflict
    );
    assert_eq!(
        f.stored(&plan_id).lifecycle_reason.as_deref(),
        Some("the target name is taken")
    );
}

#[test]
fn a_replaced_target_folder_invalidates_the_plan() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    fs::rename(f.path("Taxes"), f.path("Taxes-old")).unwrap();
    fs::create_dir(f.path("Taxes")).unwrap();

    assert_eq!(
        expect_refused(f.revalidate(&plan_id)).code,
        ErrorCode::StalePlan
    );
    assert_eq!(
        f.stored(&plan_id).lifecycle_reason.as_deref(),
        Some("the target folder changed")
    );
}

#[test]
fn revoking_a_root_ends_its_plans_and_authorizing_again_does_not_revive_them() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;

    f.service().revoke_root(&f.root_id).unwrap();
    assert_eq!(
        f.service().get_plan(&plan_id).unwrap().status,
        PlanStatus::RootRevoked
    );
    assert_eq!(
        expect_refused(f.revalidate(&plan_id)).code,
        ErrorCode::StalePlan
    );
    assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Invalidated);

    let again = f.service().authorize_root(f.root.path()).unwrap();
    assert_ne!(again.root_id, f.root_id);
    let view = f.service().get_plan(&plan_id).unwrap();
    assert_eq!(
        (view.status, view.can_confirm),
        (PlanStatus::Invalidated, false)
    );
    assert_eq!(
        view.source.root_id, f.root_id,
        "the plan keeps its old root"
    );
    // Confirming it anyway creates no action and moves nothing (O06).
    let before = snapshot::snapshot(f.root.path());
    assert_eq!(f.apply(&plan_id).unwrap_err().code, ErrorCode::StalePlan);
    assert_eq!(f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

#[test]
fn a_used_plan_is_final() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    // A settled action, so the file itself is not blocked (#9).
    let action_id = f.insert_action(&plan_id, "failed_no_move");

    match f.revalidate(&plan_id).unwrap() {
        Revalidation::AlreadyConsumed { action_id: found } => assert_eq!(found, action_id),
        other => panic!("{other:?}"),
    }
    let cancelled = f.service().cancel_plan(&plan_id).unwrap();
    assert_eq!(cancelled.status, PlanStatus::Consumed);
    assert_eq!(cancelled.action_id.as_deref(), Some(action_id.as_str()));
    assert_eq!(
        f.stored(&plan_id).lifecycle,
        PlanLifecycle::Open,
        "a used plan is never closed"
    );

    // The database itself also refuses to close a used plan.
    let store = f.service().store().unwrap();
    assert!(
        !store
            .close_plan(&plan_id, PlanLifecycle::Cancelled, START_MS, "test")
            .unwrap()
    );
    assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Open);

    // A new plan for the same file leaves the used plan alone.
    let newer = f.plan(&file, "Notes", &[]);
    assert!(newer.can_confirm);
    assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Open);
}

#[test]
fn a_file_that_keeps_changing_does_not_end_the_plan() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let path = f.path("todo.txt");

    let error = expect_refused(f.service().revalidate_plan_with(
        &plan_id,
        &mut |parent, name, dev| {
            read_stable_with(parent, name, dev, ReadLimits::SPEC, &mut |attempt| {
                fs::write(&path, format!("draft {attempt}\n")).unwrap()
            })
        },
    ));

    assert_eq!(error.code, ErrorCode::FileChanging);
    assert!(error.retryable);
    assert_eq!(
        f.stored(&plan_id).lifecycle,
        PlanLifecycle::Open,
        "the hash only could not be checked"
    );
}

#[test]
fn a_restart_invalidates_open_plans_but_not_used_ones() {
    let mut f = Fixture::new();
    let todo = f.register("todo.txt");
    let open = f.plan(&todo, "Taxes", &[]).plan_id;
    let plan_md = f.register("Notes/2026/plan.md");
    let used = f.plan(&plan_md, "Taxes", &[]).plan_id;
    f.insert_action(&used, "prepared");
    let before = f.stored(&open);

    f.restart();

    let after = f.stored(&open);
    assert_eq!(after.lifecycle, PlanLifecycle::Invalidated);
    assert_eq!(after.lifecycle_reason.as_deref(), Some("Traeva restarted"));
    assert_eq!(payload(after), payload(before));
    assert_eq!(f.stored(&used).lifecycle, PlanLifecycle::Open);
    assert_eq!(
        f.service().get_plan(&used).unwrap().status,
        PlanStatus::Consumed
    );
}

#[test]
fn choosing_the_place_the_file_is_in_ends_the_earlier_plan() {
    // #71: a plan moves todo.txt to Taxes; then the user picks the folder
    // the file is in now. Nothing to do, and the earlier plan cannot run.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let earlier = f.plan(&file, "Taxes", &[]);
    assert!(matches!(
        f.try_plan(&file, "", &[]).unwrap(),
        BuildOutcome::AlreadyInPlace { .. }
    ));
    let ended = f.service().get_plan(&earlier.plan_id).unwrap();
    assert_eq!(
        (ended.status, ended.can_confirm),
        (PlanStatus::Superseded, false)
    );
    assert_eq!(
        f.apply(&earlier.plan_id).unwrap_err().code,
        ErrorCode::StalePlan
    );
    assert!(f.path("todo.txt").exists());
    assert!(!f.path("Taxes/todo.txt").exists());
    assert_eq!(f.count("actions"), 0);

    // A plan an action already used stays as it was: confirming it again
    // still reports that one action.
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let used = f.plan(&file, "Taxes", &[]);
    let action = f.apply(&used.plan_id).unwrap();
    let moved = f.register("Taxes/todo.txt");
    assert!(matches!(
        f.try_plan(&moved, "Taxes", &[]).unwrap(),
        BuildOutcome::AlreadyInPlace { .. }
    ));
    assert_eq!(
        f.service().get_plan(&used.plan_id).unwrap().action_id,
        Some(action.action_id.clone())
    );
    let lifecycle: String = f
        .db()
        .query_row(
            "SELECT lifecycle FROM plans WHERE plan_id = ?1",
            [&used.plan_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lifecycle, "open", "a used plan is never superseded");
    assert_eq!(f.apply(&used.plan_id).unwrap().action_id, action.action_id);
    assert_eq!(f.count("actions"), 1);
}

// ---------------------------------------------------------------------------
// #20: more changes between the preview and the confirmation
// ---------------------------------------------------------------------------

/// The same file object, moved into a look-alike folder with the old names:
/// the folder above it is another object, so the plan ends (O10).
#[test]
fn a_file_moved_into_look_alike_folders_ends_the_plan() {
    let f = Fixture::new();
    let file = f.register("Notes/2026/plan.md");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    fs::rename(f.path("Notes"), f.path("Notes-old")).unwrap();
    fs::create_dir_all(f.path("Notes/2026")).unwrap();
    fs::rename(
        f.path("Notes-old/2026/plan.md"),
        f.path("Notes/2026/plan.md"),
    )
    .unwrap();

    assert_eq!(
        expect_refused(f.revalidate(&plan_id)).code,
        ErrorCode::FileChanged
    );
    assert_eq!(
        f.stored(&plan_id).lifecycle_reason.as_deref(),
        Some("a folder above the file changed")
    );
    let before = snapshot::snapshot(f.root.path());
    assert_eq!(f.apply(&plan_id).unwrap_err().code, ErrorCode::StalePlan);
    assert_eq!(f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}

/// A file that can no longer be read cannot be checked, so it is not moved.
/// The plan stays open: the problem may be temporary (O11).
#[test]
fn a_file_that_cannot_be_read_any_more_is_not_moved() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let path = f.path("todo.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let checked = f.revalidate(&plan_id);
    let applied = f.apply(&plan_id);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(expect_refused(checked).code, ErrorCode::FileUnreadable);
    assert_eq!(applied.unwrap_err().code, ErrorCode::FileUnreadable);
    assert_eq!(f.count("actions"), 0);
    assert_eq!(f.stored(&plan_id).lifecycle, PlanLifecycle::Open);
    assert!(path.exists() && !f.path("Taxes/todo.txt").exists());
}

/// A target folder that became a project folder after the preview (a
/// project marker appeared in it) is never used (O12).
#[test]
fn a_target_that_became_a_project_folder_is_not_used() {
    let f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    fs::write(f.path("Taxes/Cargo.toml"), "[package]\n").unwrap();
    let before = snapshot::snapshot(f.root.path());

    let error = f.apply(&plan_id).unwrap_err();

    // Opening the folder finds the marker; the plan reports it as a target
    // folder that changed.
    assert_eq!(error.code, ErrorCode::StalePlan, "{error:?}");
    assert_eq!(
        f.stored(&plan_id).lifecycle_reason.as_deref(),
        Some("the target folder changed")
    );
    assert_eq!(f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(f.root.path()), before);
}
