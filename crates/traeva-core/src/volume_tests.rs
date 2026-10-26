//! A volume renumbered across a restart (#37, shared contract v46). After a
//! shutdown on 10/15 macOS gave the data volume another device number
//! (16777232 became 16777234), and every saved record looked like "another
//! object". Saved evidence is now compared by inode number, birth time and
//! the volume's UUID, never by `dev`. These tests simulate the renumbering on
//! a synthetic root by rewriting every saved `dev`
//! (`Fixture::renumber_saved_volume`). Synthetic temporary files only.

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use crate::capture::CheckOutcome;
use crate::contracts::{ActionStatus, ErrorCode, FileView, UndoPreview};
use crate::execute::Checkpoint;
use crate::observe::sha256_hex;
use crate::platform::macos;
use crate::store::{ChainLink, FileEvidence, RootEvidence};
use crate::test_fixture::{Fixture, quick_read};
use crate::{SourceDownloadEvent, SourceDownloadState, SourceRecordView, SourceRelation};

const VOLUME_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VOLUME_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn file(dev: i64, volume: Option<&str>, ino: u64, birthtime_ns: i64) -> FileEvidence {
    FileEvidence {
        dev,
        volume: volume.map(str::to_string),
        ino,
        birthtime_ns,
        mtime_ns: 10,
        ctime_ns: 20,
        link_count: 1,
    }
}

fn root(dev: i64, volume: Option<&str>, ino: u64, birthtime_ns: i64) -> RootEvidence {
    RootEvidence {
        dev,
        volume: volume.map(str::to_string),
        ino,
        birthtime_ns,
        fs_type: "apfs".to_string(),
    }
}

fn link(name: &str, dev: i64, volume: Option<&str>, ino: u64) -> ChainLink {
    ChainLink {
        name: name.to_string(),
        dev,
        volume: volume.map(str::to_string),
        ino,
    }
}

#[test]
fn saved_evidence_is_compared_by_inode_birth_time_and_volume_never_by_dev() {
    let saved = file(16_777_232, Some(VOLUME_A), 7, 9);
    // Only the device number differs: the same object.
    assert!(saved.same_object(&file(16_777_234, Some(VOLUME_A), 7, 9)));
    // A record without a volume UUID (saved before it existed): not compared.
    assert!(file(16_777_232, None, 7, 9).same_object(&file(16_777_234, Some(VOLUME_A), 7, 9)));
    assert!(saved.same_object(&file(16_777_234, None, 7, 9)));
    assert!(file(1, None, 7, 9).same_object(&file(2, None, 7, 9)));
    // Another volume, inode number or birth time: another object.
    assert!(!saved.same_object(&file(16_777_232, Some(VOLUME_B), 7, 9)));
    assert!(!saved.same_object(&file(16_777_232, Some(VOLUME_A), 8, 9)));
    assert!(!saved.same_object(&file(16_777_232, Some(VOLUME_A), 7, 10)));
    assert!(!file(1, None, 7, 9).same_object(&file(1, None, 8, 9)));
    assert!(!file(1, None, 7, 9).same_object(&file(1, None, 7, 10)));

    // "Nothing changed": the same object with the same times and link count;
    // where it is (dev, volume) does not count as a change.
    let after_restart = FileEvidence {
        dev: 16_777_234,
        volume: None,
        ..saved.clone()
    };
    assert!(saved.same_state(&after_restart));
    for changed in [
        FileEvidence {
            mtime_ns: 11,
            ..saved.clone()
        },
        FileEvidence {
            ctime_ns: 21,
            ..saved.clone()
        },
        FileEvidence {
            link_count: 2,
            ..saved.clone()
        },
        FileEvidence {
            ino: 8,
            ..saved.clone()
        },
    ] {
        assert!(!saved.same_state(&changed), "{changed:?}");
    }

    // Roots: the same rule.
    let authorized = root(16_777_232, Some(VOLUME_A), 7, 9);
    assert!(authorized.same_object(&root(16_777_234, Some(VOLUME_A), 7, 9)));
    assert!(root(16_777_232, None, 7, 9).same_object(&root(16_777_234, Some(VOLUME_A), 7, 9)));
    assert!(!authorized.same_object(&root(16_777_232, Some(VOLUME_B), 7, 9)));
    assert!(!authorized.same_object(&root(16_777_232, Some(VOLUME_A), 8, 9)));
    assert!(!authorized.same_object(&root(16_777_232, Some(VOLUME_A), 7, 10)));

    // Folders of a plan's chain: name, inode number and volume.
    let notes = link("Notes", 16_777_232, Some(VOLUME_A), 7);
    assert!(notes.same_folder(&link("Notes", 16_777_234, Some(VOLUME_A), 7)));
    assert!(link("Notes", 16_777_232, None, 7).same_folder(&notes));
    assert!(!notes.same_folder(&link("Notes", 16_777_232, Some(VOLUME_B), 7)));
    assert!(!notes.same_folder(&link("Notes", 16_777_232, Some(VOLUME_A), 8)));
    assert!(!notes.same_folder(&link("Taxes", 16_777_232, Some(VOLUME_A), 7)));
}

#[test]
fn saved_evidence_without_a_volume_still_reads_and_a_new_one_is_written() {
    let legacy: FileEvidence = serde_json::from_str(
        r#"{"dev":16777232,"ino":7,"birthtime_ns":9,"mtime_ns":10,"ctime_ns":20,"link_count":1}"#,
    )
    .unwrap();
    assert_eq!(legacy, file(16_777_232, None, 7, 9));
    assert!(
        !serde_json::to_string(&legacy).unwrap().contains("volume"),
        "a record without a volume is written as before"
    );
    let text = serde_json::to_string(&file(16_777_234, Some(VOLUME_A), 7, 9)).unwrap();
    assert!(
        text.contains(&format!(r#""volume":"{VOLUME_A}""#)),
        "{text}"
    );
    let legacy_root: RootEvidence =
        serde_json::from_str(r#"{"dev":16777232,"ino":7,"birthtime_ns":9,"fs_type":"apfs"}"#)
            .unwrap();
    assert_eq!(legacy_root.volume, None);
}

fn saved_root(f: &Fixture) -> RootEvidence {
    f.service()
        .store()
        .unwrap()
        .get_root(&f.root_id)
        .unwrap()
        .unwrap()
        .evidence
}

#[test]
fn a_root_saved_before_a_restart_that_renumbered_its_volume_still_verifies() {
    for legacy in [true, false] {
        let mut f = Fixture::new();
        let before = saved_root(&f);
        assert_eq!(
            before.volume,
            macos::volume_uuid(before.dev),
            "the root's volume UUID is saved"
        );
        assert!(before.volume.is_some());

        f.renumber_saved_volume(legacy);
        f.restart();

        let after = saved_root(&f);
        assert_eq!(after.dev, before.dev + 2);
        assert_eq!(after.volume.is_none(), legacy);
        let verified = f.service().verified_root(&f.root_id);
        assert!(verified.is_ok(), "legacy {legacy}: {:?}", verified.err());
        assert_eq!(f.service().list_roots().unwrap()[0].problem, None);
    }
}

#[test]
fn a_root_with_another_inode_birth_time_or_volume_is_still_not_the_authorized_folder() {
    let changes = [
        (
            true,
            "json_set(evidence_json, '$.ino', json_extract(evidence_json, '$.ino') + 1)",
        ),
        (
            false,
            "json_set(evidence_json, '$.ino', json_extract(evidence_json, '$.ino') + 1)",
        ),
        (
            false,
            "json_set(evidence_json, '$.birthtime_ns', \
             json_extract(evidence_json, '$.birthtime_ns') + 1)",
        ),
        (
            false,
            "json_set(evidence_json, '$.volume', '00000000000000000000000000000001')",
        ),
    ];
    for (legacy, change) in changes {
        let mut f = Fixture::new();
        f.renumber_saved_volume(legacy);
        f.db()
            .execute(
                &format!("UPDATE authorized_roots SET evidence_json = {change}"),
                [],
            )
            .unwrap();
        f.restart();
        let error = f.service().verified_root(&f.root_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::RootChanged, "{change}");
    }
}

#[test]
fn authorizing_the_same_folder_again_after_the_restart_keeps_its_root() {
    for legacy in [true, false] {
        let mut f = Fixture::new();
        f.renumber_saved_volume(legacy);
        f.restart();
        let again = f.service().authorize_root(f.root.path()).unwrap();
        assert_eq!(again.root_id, f.root_id, "legacy {legacy}: the same root");
        assert_eq!(f.count("authorized_roots"), 1, "legacy {legacy}");
    }
}

#[test]
fn registering_again_after_the_restart_keeps_the_record_and_saves_the_volume() {
    let mut f = Fixture::new();
    let first = f.register("todo.txt");
    f.renumber_saved_volume(true);
    f.restart();

    let again = f.register("todo.txt");
    assert_eq!(again.file_id, first.file_id, "the same record");
    assert_eq!(
        again.file_revision, first.file_revision,
        "nothing about the file changed"
    );
    let (row, present) = f
        .service()
        .store()
        .unwrap()
        .get_file(&first.file_id)
        .unwrap()
        .unwrap();
    assert!(present);
    assert!(row.evidence.volume.is_some());
    assert_eq!(row.evidence.volume, macos::volume_uuid(row.evidence.dev));
}

#[test]
fn a_memory_watch_saved_before_the_restart_still_sees_its_file() {
    let mut f = Fixture::new().without_auto_link();
    let file = f.register("todo.txt");
    f.start_memory(&file.file_id).unwrap();
    f.renumber_saved_volume(true);
    f.restart();

    let mut reads = 0;
    let checks = f
        .service()
        .check_memory_with(&mut |root, parent, name| {
            reads += 1;
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(
        checks[0].outcome,
        CheckOutcome::Unchanged,
        "the same file, not a replacement"
    );
    assert_eq!(reads, 0, "nothing about it changed, so it is not read");

    // An edit is the next version of the same record.
    fs::write(f.path("todo.txt"), "edited after the restart\n").unwrap();
    let checks = f.check_memory();
    assert!(
        matches!(checks[0].outcome, CheckOutcome::Saved { .. }),
        "{checks:?}"
    );
    assert_eq!(f.count("files"), 1);
    assert_eq!(f.count("snapshots"), 2);
}

/// Organizes `relative` into `folder`; returns the file and the action.
fn organize(f: &Fixture, relative: &str, folder: &str) -> (FileView, String) {
    let file = f.register(relative);
    let plan_id = f.plan(&file, folder, &[]).plan_id;
    let action = f.apply(&plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    (file, action.action_id)
}

#[test]
fn undo_after_the_restart_puts_the_file_back() {
    let mut f = Fixture::new();
    let (file, action_id) = organize(&f, "todo.txt", "Taxes");
    f.renumber_saved_volume(true);
    f.restart();

    let UndoPreview::Planned { plan } = f.service().preview_undo(&action_id).unwrap() else {
        panic!("expected an undo plan");
    };
    assert!(plan.can_confirm);
    assert_eq!(
        f.apply(&plan.plan_id).unwrap().status,
        ActionStatus::Committed
    );
    assert!(!f.path("Taxes/todo.txt").exists());
    assert_eq!(
        sha256_hex(&fs::read(f.path("todo.txt")).unwrap()),
        file.sha256
    );
}

#[test]
fn undo_after_the_restart_still_refuses_another_object() {
    let mut f = Fixture::new();
    let (_, action_id) = organize(&f, "todo.txt", "Taxes");
    // A copy with the same bytes takes the moved file's place.
    let bytes = fs::read(f.path("Taxes/todo.txt")).unwrap();
    fs::write(f.path("Taxes/copy.tmp"), bytes).unwrap();
    fs::rename(f.path("Taxes/copy.tmp"), f.path("Taxes/todo.txt")).unwrap();
    f.renumber_saved_volume(true);
    f.restart();

    let error = f.service().preview_undo(&action_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::UndoConflict, "{error:?}");
    assert!(f.path("Taxes/todo.txt").exists(), "nothing moved");
}

/// Runs the plan until `point` and stops it there (O18-R).
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

#[test]
fn a_run_stopped_after_the_move_is_committed_after_a_restart_that_renumbered_the_volume() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Taxes", &[]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::AfterMove);
    f.renumber_saved_volume(true);
    f.restart();

    let view = f.service().get_action(&action_id).unwrap();
    assert_eq!(view.status, ActionStatus::Committed, "{view:?}");
    assert!(view.problem.is_none());
}

#[test]
fn a_run_stopped_before_the_move_removes_its_folders_after_a_restart_that_renumbered_the_volume() {
    let mut f = Fixture::new();
    let file = f.register("todo.txt");
    let plan_id = f.plan(&file, "Notes", &["Archive", "Q1"]).plan_id;
    let action_id = stop_at(&f, &plan_id, Checkpoint::BeforeMoveCheck);
    f.renumber_saved_volume(true);
    f.restart();

    let view = f.service().get_action(&action_id).unwrap();
    assert_eq!(view.status, ActionStatus::FailedNoMove);
    assert_eq!(
        view.folders_removed,
        ["Notes/Archive/Q1", "Notes/Archive"],
        "the receipts still name these folders"
    );
    assert!(!f.path("Notes/Archive").exists());
    assert!(f.path("todo.txt").exists());
}

/// A finished browser download of `relative` (synthetic URLs only).
fn download(f: &Fixture, id: &str, relative: &str) -> SourceDownloadEvent {
    let path = f.root.path().canonicalize().unwrap().join(relative);
    SourceDownloadEvent {
        profile_id: "77777777-7777-4777-8777-777777777777".to_string(),
        download_id: id.to_string(),
        started_at_ms: Some(1_700_000_000_000),
        completed_at_ms: Some(1_700_000_000_100),
        state: SourceDownloadState::Complete,
        file_size: Some(fs::metadata(&path).unwrap().len()),
        filename: path.display().to_string(),
        initial_url: Some("https://example.test/start".to_string()),
        final_url: Some("https://example.test/final".to_string()),
        referrer: None,
        incognito: false,
        explicit_history: true,
    }
}

/// Saves the download's source; returns its ID.
fn ingest(f: &Fixture, id: &str, relative: &str) -> String {
    let result = f
        .service()
        .ingest_source_event(download(f, id, relative))
        .unwrap();
    assert_eq!(result.outcome, "saved", "{:?}", result.reason);
    result.source_id.unwrap()
}

/// The source as the Sources page lists it (nothing is registered again).
fn listed(f: &Fixture, source_id: &str) -> SourceRecordView {
    f.service()
        .source_state()
        .unwrap()
        .records
        .into_iter()
        .find(|record| record.source_id == source_id)
        .unwrap()
}

#[test]
fn a_source_saved_before_the_restart_still_finds_its_file() {
    for legacy in [true, false] {
        let mut f = Fixture::new();
        f.service().set_source_enabled(true).unwrap();
        // One download the user confirmed, one still a suggestion.
        let confirmed = ingest(&f, "1", "todo.txt");
        let file_id = listed(&f, &confirmed).candidates[0].file_id.clone();
        f.service()
            .confirm_source(&confirmed, &file_id, SourceRelation::Download, "")
            .unwrap();
        let suggested = ingest(&f, "2", "Notes/2026/plan.md");
        f.renumber_saved_volume(legacy);
        f.restart();

        let view = listed(&f, &confirmed);
        assert!(
            view.association.unwrap().current_location_confirmed,
            "legacy {legacy}: {:?}",
            view.reason
        );
        let view = listed(&f, &suggested);
        assert!(view.candidates[0].can_confirm, "legacy {legacy}");
        let view = f
            .service()
            .confirm_source(
                &suggested,
                &view.candidates[0].file_id,
                SourceRelation::Download,
                "",
            )
            .unwrap();
        assert!(view.association.unwrap().current_location_confirmed);
    }
}
