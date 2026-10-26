//! Tests for auto_memory.rs (#201), all offline: synthetic files in the
//! fixture's root, reads without the pause.

use std::fs;

use super::*;
use crate::observe::ReadError;
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::test_fixture::{Fixture, quick_read};
use crate::test_snapshot as snapshot;

fn quick(f: &Fixture, action_id: &str, limit: usize) -> AutoMemoryOutcome {
    f.service()
        .keep_versions_after_with(action_id, limit, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        })
        .unwrap()
}

fn keep(f: &Fixture, action_id: &str) -> AutoMemoryOutcome {
    quick(f, action_id, AUTO_MEMORY_LIMIT)
}

fn model(f: &Fixture) {
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
}

fn switch(f: &Fixture, enable: bool) -> AutoMemoryView {
    f.service()
        .set_auto_memory(&AutoMemoryRequest {
            enable,
            notice_version: AUTO_MEMORY_NOTICE_VERSION,
        })
        .unwrap()
}

/// Writes `name` in the root, organizes it into Taxes and returns the
/// file ID and the finished action's ID.
fn organized(f: &Fixture, name: &str) -> (String, String) {
    fs::write(f.path(name), "Receipts for March.\n").unwrap();
    let file = f.register(name);
    let plan = f.plan(&file, "Taxes", &[]);
    let action = f.apply(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    (file.file_id, action.action_id)
}

fn agreement_counts(f: &Fixture, file_id: &str) -> bool {
    f.service().counting_consent(file_id).unwrap().is_some()
}

#[test]
fn the_switch_is_off_until_turned_on() {
    let f = Fixture::new();
    model(&f);
    let view = f.service().auto_memory().unwrap();
    assert!(!view.enabled && !view.outdated);
    assert_eq!(view.limit, 50);
    assert!(view.notice.contains("Memory's own agreement"));

    let (file_id, action_id) = organized(&f, "march.txt");
    assert_eq!(keep(&f, &action_id), AutoMemoryOutcome::Off);
    assert!(f.service().memory_watches().unwrap().is_empty());
    assert!(!agreement_counts(&f, &file_id));
}

#[test]
fn on_a_moved_file_starts_recording_with_memory_s_agreement() {
    let f = Fixture::new();
    model(&f);
    assert!(switch(&f, true).enabled);
    let (file_id, action_id) = organized(&f, "march.txt");
    let tree = snapshot::snapshot(f.root.path());

    let outcome = keep(&f, &action_id);

    let AutoMemoryOutcome::Started {
        chain_id: Some(chain_id),
        agreement_given: true,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    let watches = f.service().memory_watches().unwrap();
    assert_eq!(watches.len(), 1);
    assert_eq!(watches[0].file_id, file_id);
    assert_eq!(
        watches[0].relative_path, "Taxes/march.txt",
        "recorded at its new place"
    );
    assert!(
        agreement_counts(&f, &file_id),
        "Memory may describe its changes"
    );
    let view = f.service().auto_memory().unwrap();
    assert_eq!(view.started_chain_ids, vec![chain_id]);
    assert_eq!(view.recording, 1);
    assert_eq!(
        snapshot::snapshot(f.root.path()),
        tree,
        "nothing written in the folder"
    );

    assert_eq!(keep(&f, &action_id), AutoMemoryOutcome::AlreadyHandled);
}

#[test]
fn files_memory_cannot_read_and_unfinished_moves_are_left_alone() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    for name in ["scan.pdf", "photo.png"] {
        let (_, action_id) = organized(&f, name);
        assert_eq!(
            keep(&f, &action_id),
            AutoMemoryOutcome::NotReadable,
            "{name}"
        );
    }

    fs::write(f.path("draft.md"), "draft\n").unwrap();
    let file = f.register("draft.md");
    let plan = f.plan(&file, "Taxes", &[]);
    let prepared = f.insert_action(&plan.plan_id, "prepared");
    assert_eq!(keep(&f, &prepared), AutoMemoryOutcome::NotAMove);
    assert!(f.service().memory_watches().unwrap().is_empty());
}

#[test]
fn a_file_memory_already_records_is_not_touched() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    fs::write(f.path("essay.txt"), "first\n").unwrap();
    let file = f.register("essay.txt");
    f.start_memory(&file.file_id).unwrap();
    let plan = f.plan(&file, "Taxes", &[]);
    let action = f.apply(&plan.plan_id).unwrap();

    assert_eq!(
        keep(&f, &action.action_id),
        AutoMemoryOutcome::AlreadyRecorded
    );
    assert!(
        !agreement_counts(&f, &file.file_id),
        "no agreement given for it"
    );
}

#[test]
fn no_new_record_once_the_limit_is_reached() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    fs::write(f.path("other.txt"), "x\n").unwrap();
    let other = f.register("other.txt");
    f.start_memory(&other.file_id).unwrap();
    let (file_id, action_id) = organized(&f, "march.txt");

    assert_eq!(quick(&f, &action_id, 1), AutoMemoryOutcome::LimitReached);
    assert!(!agreement_counts(&f, &file_id));
    assert_eq!(f.service().auto_memory().unwrap().recording, 1);
}

#[test]
fn turning_it_off_withdraws_only_the_agreements_it_gave() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    let (auto_file, action_id) = organized(&f, "march.txt");
    keep(&f, &action_id);
    // A file the user agreed for on their own, before the switch moved it:
    // the switch gives nothing, and turning it off does not withdraw it.
    fs::write(f.path("own.txt"), "mine\n").unwrap();
    let own = f.register("own.txt");
    f.service().set_memory_consent(&own.file_id, true).unwrap();
    let plan = f.plan(&own, "Taxes", &[]);
    let action = f.apply(&plan.plan_id).unwrap();
    assert!(matches!(
        keep(&f, &action.action_id),
        AutoMemoryOutcome::Started {
            agreement_given: false,
            ..
        }
    ));

    let off = switch(&f, false);

    assert!(!off.enabled);
    assert!(
        !agreement_counts(&f, &auto_file),
        "nothing more is sent about it"
    );
    assert!(
        agreement_counts(&f, &own.file_id),
        "the user's own agreement stays"
    );
    let watches = f.service().memory_watches().unwrap();
    assert!(
        watches
            .iter()
            .all(|w| w.status == crate::WatchStatus::Active),
        "the records keep saving versions"
    );
    let (_, later) = organized(&f, "april.txt");
    assert_eq!(keep(&f, &later), AutoMemoryOutcome::Off);
}

#[test]
fn another_model_ends_the_switch_and_switching_back_does_not_revive_it() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    let (auto_file, action_id) = organized(&f, "march.txt");
    keep(&f, &action_id);
    assert!(agreement_counts(&f, &auto_file));

    f.service()
        .save_model_settings(&openai("gpt-5.6-luna", None))
        .unwrap();
    let view = f.service().auto_memory().unwrap();
    assert!(!view.enabled && !view.outdated, "it is off, not waiting");
    let store = f.service().store().unwrap();
    assert!(
        store.open_consent(&auto_file).unwrap().is_none(),
        "the agreement it gave is withdrawn at once"
    );

    model(&f); // back to the first model
    assert!(
        !f.service().auto_memory().unwrap().enabled,
        "switching back does not revive it (G4 M1)"
    );
    assert!(!agreement_counts(&f, &auto_file));
    let (_, later) = organized(&f, "april.txt");
    assert_eq!(keep(&f, &later), AutoMemoryOutcome::Off);
    assert!(switch(&f, true).enabled, "agreeing again turns it on");
}

#[test]
fn a_record_the_user_stopped_is_not_started_again() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    let (file_id, action_id) = organized(&f, "march.txt");
    keep(&f, &action_id);
    f.service().stop_memory(&file_id).unwrap();

    // Organized once more, into another folder.
    let moved = f.register("Taxes/march.txt");
    assert_eq!(moved.file_id, file_id);
    let plan = f.plan(&moved, "Notes", &[]);
    let again = f.apply(&plan.plan_id).unwrap();

    assert_eq!(
        keep(&f, &again.action_id),
        AutoMemoryOutcome::AlreadyRecorded
    );
    let watches = f.service().memory_watches().unwrap();
    assert!(
        watches
            .iter()
            .all(|w| w.status == crate::WatchStatus::Stopped),
        "the user's stop stays (G4 M2)"
    );
}

#[test]
fn turning_it_on_needs_the_notice_shown_and_a_model() {
    let f = Fixture::new();
    let error = f
        .service()
        .set_auto_memory(&AutoMemoryRequest {
            enable: true,
            notice_version: AUTO_MEMORY_NOTICE_VERSION,
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ModelNotConfigured);
    model(&f);
    let error = f
        .service()
        .set_auto_memory(&AutoMemoryRequest {
            enable: true,
            notice_version: AUTO_MEMORY_NOTICE_VERSION + 1,
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PageOutdated);
    assert!(!f.service().auto_memory().unwrap().enabled);
}

#[test]
fn a_first_version_that_cannot_be_read_yet_is_read_by_memory_s_next_pass() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    let (file_id, action_id) = organized(&f, "march.txt");

    let outcome = f
        .service()
        .keep_versions_after_with(&action_id, AUTO_MEMORY_LIMIT, &mut |_, _, _| {
            Err(ReadError::StillChanging { attempts: 3 })
        })
        .unwrap();

    // As when the user turns Memory on: the record is on, and the next
    // pass saves the first version.
    assert!(
        matches!(outcome, AutoMemoryOutcome::Started { .. }),
        "{outcome:?}"
    );
    f.check_memory();
    let watches = f.service().memory_watches().unwrap();
    assert_eq!(watches[0].file_id, file_id);
    assert_eq!(watches[0].versions, 1);
}

#[test]
fn a_start_memory_refuses_changes_nothing_and_is_not_retried() {
    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    let (file_id, action_id) = organized(&f, "march.txt");
    f.service().revoke_root(&f.root_id).unwrap();

    let outcome = keep(&f, &action_id);

    assert_eq!(
        outcome,
        AutoMemoryOutcome::Failed {
            code: ErrorCode::RootRevoked
        }
    );
    assert!(f.service().memory_watches().unwrap().is_empty());
    assert!(!agreement_counts(&f, &file_id));
}

#[test]
fn memory_describes_the_next_save_by_itself_until_the_switch_is_off() {
    use crate::changes::PassStep;
    use crate::changes::tests::{FakeModel, answer, change, pass};

    let f = Fixture::new();
    model(&f);
    switch(&f, true);
    let (_, action_id) = organized(&f, "march.txt");
    keep(&f, &action_id);
    fs::write(
        f.path("Taxes/march.txt"),
        "Receipts for March.\nAnd April.\n",
    )
    .unwrap();
    f.check_memory();
    let mut model = FakeModel::answering(vec![Ok(answer(
        vec![change(
            "added",
            "april",
            "Added April",
            &["H1"],
            None,
            Some("And April"),
        )],
        &[],
    ))]);

    let steps = pass(&f, &mut model);

    assert_eq!(model.sent.len(), 1, "the switch's agreement let it send");
    assert!(steps.iter().any(|step| matches!(
        step,
        PassStep::Described { status, .. } if status == "analyzed"
    )));

    switch(&f, false);
    fs::write(f.path("Taxes/march.txt"), "Receipts for March.\nAnd May.\n").unwrap();
    f.check_memory();
    let mut quiet = FakeModel::default();
    pass(&f, &mut quiet);
    assert!(
        quiet.sent.is_empty(),
        "nothing is sent after the switch is off"
    );
}

/// R2-AM-01 (#227): with 49 records on, two moves that finish together
/// while a capture holds the execution lock must not both start Memory.
#[test]
fn two_automatic_starts_at_once_never_pass_the_limit_together() {
    let f = Fixture::new();
    model(&f);
    for i in 0..49 {
        let name = format!("kept-{i}.txt");
        fs::write(f.path(&name), format!("record {i}\n")).unwrap();
        let file = f.register(&name);
        f.start_memory(&file.file_id).unwrap();
    }
    let (left_file, left) = organized(&f, "left.txt");
    let (right_file, right) = organized(&f, "right.txt");
    switch(&f, true);
    assert_eq!(f.service().auto_memory().unwrap().recording, 49);

    let service = f.service();
    let outcomes = {
        // A capture holds the execution lock while both moves finish.
        let held = service.execution.lock().unwrap();
        std::thread::scope(|scope| {
            let run = |action: String| {
                scope.spawn(move || {
                    service
                        .keep_versions_after_with(
                            &action,
                            AUTO_MEMORY_LIMIT,
                            &mut |root, parent, name| quick_read(parent, name, root.id().dev),
                        )
                        .unwrap()
                })
            };
            let a = run(left.clone());
            let b = run(right.clone());
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(held);
            vec![a.join().unwrap(), b.join().unwrap()]
        })
    };

    let started = outcomes
        .iter()
        .filter(|o| matches!(o, AutoMemoryOutcome::Started { .. }))
        .count();
    let limited = outcomes
        .iter()
        .filter(|o| **o == AutoMemoryOutcome::LimitReached)
        .count();
    assert_eq!((started, limited), (1, 1), "{outcomes:?}");
    assert_eq!(f.service().auto_memory().unwrap().recording, 50);
    let given = [left_file, right_file]
        .iter()
        .filter(|file| agreement_counts(&f, file))
        .count();
    assert_eq!(given, 1, "one agreement given, not two");
}
