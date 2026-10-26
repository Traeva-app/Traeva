//! Operation tests: reading a picked file for the window, with the reads
//! under control (no pause, or a file that keeps changing). Synthetic
//! temporary files only.

use std::fs;

use super::*;
use crate::observe::{ReadLimits, read_stable_with};
use crate::test_fixture::{Fixture, quick_read};

fn quick(root: &Folder, parent: &Folder, name: &str) -> Result<StableRead, ReadError> {
    quick_read(parent, name, root.id().dev)
}

fn start(f: &Fixture) -> OperationView {
    f.service().start_operation(&f.root_id, &f.root_id).unwrap()
}

#[test]
fn a_read_file_with_text_is_ready_for_analysis() {
    let f = Fixture::new();
    let started = start(&f);
    assert_eq!(
        (started.stage, started.generation),
        (OperationStage::Observing, 1)
    );

    f.service()
        .run_operation_with(&started.operation_id, &f.path("todo.txt"), &mut quick);

    let done = f.service().get_operation(&started.operation_id).unwrap();
    assert_eq!(
        (done.stage, done.generation),
        (OperationStage::ReadyForAnalysis, 2)
    );
    let file = done.file.expect("the registered file");
    assert_eq!(file.relative_path, "todo.txt");
    assert!(done.error.is_none());
    // Reading again (a second run) changes nothing: the stage is final.
    f.service()
        .run_operation_with(&started.operation_id, &f.path("todo.txt"), &mut quick);
    assert_eq!(
        f.service()
            .get_operation(&started.operation_id)
            .unwrap()
            .generation,
        2
    );

    // No text the model may read (not .txt/.md): straight to a manual choice.
    fs::write(f.path("scan.pdf"), b"%PDF-1.7 synthetic").unwrap();
    let pdf = start(&f);
    f.service()
        .run_operation_with(&pdf.operation_id, &f.path("scan.pdf"), &mut quick);
    assert_eq!(
        f.service().get_operation(&pdf.operation_id).unwrap().stage,
        OperationStage::NeedsManualLocation
    );
}

#[test]
fn a_changing_file_waits_and_an_unsupported_one_is_blocked() {
    let f = Fixture::new();
    let path = f.path("todo.txt");
    let changing = start(&f);
    f.service()
        .run_operation_with(&changing.operation_id, &path, &mut |root, parent, name| {
            read_stable_with(parent, name, root.id().dev, ReadLimits::SPEC, &mut |n| {
                fs::write(&path, format!("draft {n}\n")).unwrap()
            })
        });
    let waiting = f.service().get_operation(&changing.operation_id).unwrap();
    assert_eq!(waiting.stage, OperationStage::WaitingForFile);
    assert_eq!(waiting.error.unwrap().code, ErrorCode::FileChanging);

    // A symbolic link is never read.
    let link = start(&f);
    f.service()
        .run_operation_with(&link.operation_id, &f.path("plan-link.md"), &mut quick);
    let blocked = f.service().get_operation(&link.operation_id).unwrap();
    assert_eq!(blocked.stage, OperationStage::Blocked);
    assert_eq!(blocked.error.unwrap().code, ErrorCode::UnsupportedFile);
    assert!(blocked.file.is_none());
}

/// A file over 20 MiB is blocked in the window too: it is not read, not
/// registered, and cannot be sent to a manual choice (O26).
#[test]
fn a_file_over_20_mib_is_blocked_and_cannot_be_chosen_by_hand() {
    let f = Fixture::new();
    fs::File::create(f.path("huge.txt"))
        .unwrap()
        .set_len(crate::observe::MAX_FILE_BYTES + 1)
        .unwrap();
    let started = start(&f);
    f.service()
        .run_operation_with(&started.operation_id, &f.path("huge.txt"), &mut quick);

    let blocked = f.service().get_operation(&started.operation_id).unwrap();
    assert_eq!(blocked.stage, OperationStage::Blocked);
    assert_eq!(blocked.error.unwrap().code, ErrorCode::FileTooLarge);
    assert!(blocked.file.is_none());
    assert_eq!(f.count("files"), 0, "not registered");
    assert_eq!(
        f.service()
            .choose_manual_location(&started.operation_id)
            .unwrap_err()
            .code,
        ErrorCode::WrongStage
    );
}

#[test]
fn a_cancelled_operation_drops_its_late_result() {
    let f = Fixture::new();
    let started = start(&f);
    let cancelled = f.service().cancel_operation(&started.operation_id).unwrap();
    assert_eq!(
        (cancelled.stage, cancelled.generation),
        (OperationStage::Cancelled, 2)
    );

    // The reading finishes after the cancel: its answer is dropped.
    f.service()
        .run_operation_with(&started.operation_id, &f.path("todo.txt"), &mut quick);
    assert_eq!(
        f.service().get_operation(&started.operation_id).unwrap(),
        cancelled
    );

    // A read file can still be cancelled before anything is confirmed.
    let read = start(&f);
    f.service()
        .run_operation_with(&read.operation_id, &f.path("todo.txt"), &mut quick);
    let cancelled = f.service().cancel_operation(&read.operation_id).unwrap();
    assert_eq!(
        (cancelled.stage, cancelled.generation),
        (OperationStage::Cancelled, 3)
    );

    // Cancelling a finished operation (here: blocked) leaves it as it is.
    let finished = start(&f);
    f.service()
        .run_operation_with(&finished.operation_id, &f.path("plan-link.md"), &mut quick);
    let before = f.service().get_operation(&finished.operation_id).unwrap();
    assert_eq!(before.stage, OperationStage::Blocked);
    assert_eq!(
        f.service()
            .cancel_operation(&finished.operation_id)
            .unwrap(),
        before
    );
}

#[test]
fn a_cancel_while_reading_wins_over_the_reading() {
    let f = Fixture::new();
    let started = start(&f);
    let id = started.operation_id.clone();
    // The user cancels while the file is being read (between the two reads).
    f.service()
        .run_operation_with(&id, &f.path("todo.txt"), &mut |root, parent, name| {
            read_stable_with(parent, name, root.id().dev, ReadLimits::SPEC, &mut |_| {
                f.service().cancel_operation(&id).unwrap();
            })
        });
    let after = f.service().get_operation(&id).unwrap();
    assert_eq!(after.stage, OperationStage::Cancelled);
    assert!(after.file.is_none(), "the late result was dropped");
}

#[test]
fn ids_are_checked_and_operations_end_with_a_restart() {
    let mut f = Fixture::new();
    assert_eq!(
        f.service().get_operation("nope").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        f.service().cancel_operation("nope").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    let started = start(&f);
    f.restart();
    assert_eq!(
        f.service()
            .get_operation(&started.operation_id)
            .unwrap_err()
            .code,
        ErrorCode::UnknownOperation
    );
}

#[test]
fn an_operation_needs_usable_roots() {
    let f = Fixture::new();
    let unknown = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";
    assert_eq!(
        f.service()
            .start_operation(&f.root_id, unknown)
            .unwrap_err()
            .code,
        ErrorCode::UnknownRoot
    );
    f.service().revoke_root(&f.root_id).unwrap();
    assert_eq!(
        f.service()
            .start_operation(&f.root_id, &f.root_id)
            .unwrap_err()
            .code,
        ErrorCode::RootRevoked
    );
}
