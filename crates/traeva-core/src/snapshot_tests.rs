use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::*;
use crate::contracts::ErrorStage;
use crate::memory_contracts::TextStatus;
use crate::test_fixture::{Fixture, edit_keeping_size_and_mtime, quick_read};
use crate::test_snapshot as tree;

fn snapshots_dir(fixture: &Fixture) -> PathBuf {
    fixture.app.path().join("Traeva").join(SNAPSHOT_DIR)
}

fn stored_copy(fixture: &Fixture, sha256: &str) -> PathBuf {
    snapshots_dir(fixture).join(&sha256[..2]).join(sha256)
}

#[test]
fn the_first_capture_saves_the_exact_bytes() {
    let fixture = Fixture::new();
    let file = fixture.register("Notes/2026/plan.md");
    let capture = fixture.capture(&file.file_id).unwrap();

    assert!(capture.created);
    let snapshot = capture.snapshot;
    assert_eq!(snapshot.file_id, file.file_id);
    assert_eq!(snapshot.sequence, 1);
    assert_eq!(snapshot.sha256, file.sha256);
    assert_eq!(snapshot.byte_length, file.byte_length);
    assert_eq!(snapshot.file_revision, file.file_revision);
    assert_eq!(snapshot.root_id, fixture.root_id);
    assert_eq!(snapshot.relative_path, "Notes/2026/plan.md");

    let copy = stored_copy(&fixture, &snapshot.sha256);
    assert_eq!(fs::read(&copy).unwrap(), b"# Plan\n\n- ship M1\n");
    let (read_back, bytes) = fixture
        .service()
        .read_snapshot(&snapshot.snapshot_id)
        .unwrap();
    assert_eq!(read_back, snapshot);
    assert_eq!(bytes, b"# Plan\n\n- ship M1\n");
}

#[test]
fn capturing_reads_only_and_writes_nothing_inside_the_root() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let before = tree::snapshot(fixture.root.path());
    fixture.capture(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy oat milk\n").unwrap();
    let edited = tree::snapshot(fixture.root.path());
    fixture.capture(&file.file_id).unwrap();
    assert_eq!(tree::snapshot(fixture.root.path()), edited);
    assert_ne!(before, edited);
}

#[test]
fn the_same_bytes_again_save_nothing_new() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let first = fixture.capture(&file.file_id).unwrap();
    let again = fixture.capture(&file.file_id).unwrap();
    assert!(!again.created);
    assert_eq!(again.snapshot, first.snapshot);
    assert_eq!(fixture.count("snapshots"), 1);
}

#[test]
fn a_save_without_changes_saves_nothing_new_but_is_observed() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.capture(&file.file_id).unwrap();
    // An editor saves the same text again: the modification time moves.
    fs::write(fixture.path("todo.txt"), "buy milk\n").unwrap();
    let handle = fs::File::options()
        .write(true)
        .open(fixture.path("todo.txt"))
        .unwrap();
    handle
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();

    let again = fixture.capture(&file.file_id).unwrap();
    assert!(!again.created);
    assert_eq!(fixture.count("snapshots"), 1);
    let (row, _) = fixture
        .service()
        .store()
        .unwrap()
        .get_file(&file.file_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.record_revision, file.file_revision + 1);
}

#[test]
fn each_changed_save_becomes_the_next_snapshot_and_old_ones_keep_their_bytes() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let texts = ["buy milk\n", "buy milk\nand bread\n", "buy bread\n"];
    for (index, text) in texts.iter().enumerate() {
        fs::write(fixture.path("todo.txt"), text).unwrap();
        let capture = fixture.capture(&file.file_id).unwrap();
        assert!(capture.created);
        assert_eq!(capture.snapshot.sequence, index as i64 + 1);
        // The revision the capture itself recorded, not the one before it.
        let (row, _) = fixture
            .service()
            .store()
            .unwrap()
            .get_file(&file.file_id)
            .unwrap()
            .unwrap();
        assert_eq!(capture.snapshot.file_revision, row.record_revision);
        assert_eq!(row.record_revision, file.file_revision + index as i64 + 1);
    }
    let list = fixture.service().list_snapshots(&file.file_id).unwrap();
    assert_eq!(list.len(), 3);
    for (snapshot, text) in list.iter().zip(texts) {
        let (_, bytes) = fixture
            .service()
            .read_snapshot(&snapshot.snapshot_id)
            .unwrap();
        assert_eq!(bytes, text.as_bytes());
        assert_eq!(snapshot.file_id, file.file_id);
    }
    assert!(list.windows(2).all(|pair| pair[0].sha256 != pair[1].sha256));
}

#[test]
fn going_back_to_an_earlier_text_is_a_new_version() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.capture(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy bread\n").unwrap();
    fixture.capture(&file.file_id).unwrap();
    fs::write(fixture.path("todo.txt"), "buy milk\n").unwrap();
    let back = fixture.capture(&file.file_id).unwrap();
    assert!(back.created);
    assert_eq!(back.snapshot.sequence, 3);
    let list = fixture.service().list_snapshots(&file.file_id).unwrap();
    assert_eq!(list[0].sha256, list[2].sha256);
}

#[test]
fn a_change_that_keeps_size_and_modification_time_is_still_found() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.capture(&file.file_id).unwrap();
    edit_keeping_size_and_mtime(&fixture.path("todo.txt"), "buy MILK\n");
    let capture = fixture.capture(&file.file_id).unwrap();
    assert!(capture.created);
    assert_eq!(capture.snapshot.sequence, 2);
}

#[test]
fn a_replaced_file_is_not_captured_and_nothing_is_recorded() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.capture(&file.file_id).unwrap();
    // Save by replacing: write a new file, then rename it over the old name.
    fs::write(fixture.path("todo.tmp"), "buy milk\nand eggs\n").unwrap();
    fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();
    let files_before = fixture.count("files");

    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanged);
    assert_eq!(error.file_id.as_deref(), Some(file.file_id.as_str()));
    assert_eq!(fixture.count("snapshots"), 1);
    assert_eq!(fixture.count("files"), files_before);
    let (row, present) = fixture
        .service()
        .store()
        .unwrap()
        .get_file(&file.file_id)
        .unwrap()
        .unwrap();
    assert!(present);
    assert_eq!(row.record_revision, file.file_revision);
}

#[test]
fn a_file_that_is_gone_is_reported() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fs::rename(fixture.path("todo.txt"), fixture.path("Taxes/todo.txt")).unwrap();
    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::FileNotFound);
    assert_eq!(fixture.count("snapshots"), 0);
}

#[test]
fn a_record_already_marked_missing_is_reported_as_gone() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    // Another file takes the name and is registered: the old record is missing.
    fs::write(fixture.path("todo.tmp"), "new list\n").unwrap();
    fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();
    let newer = fixture.register("todo.txt");
    assert_ne!(newer.file_id, file.file_id);

    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::FileNotFound);
    assert_eq!(fixture.count("snapshots"), 0);
}

#[test]
fn a_revoked_folder_is_not_read_any_more() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.capture(&file.file_id).unwrap();
    fixture.service().revoke_root(&fixture.root_id).unwrap();
    fs::write(fixture.path("todo.txt"), "changed after revoking\n").unwrap();
    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::RootRevoked);
    assert_eq!(fixture.count("snapshots"), 1);
}

#[test]
fn a_file_with_an_unsettled_action_is_not_captured() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let plan = fixture.plan(&file, "Taxes", &[]);
    fixture.insert_action(&plan.plan_id, "needs_user_decision");
    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnresolvedAction);
    assert_eq!(fixture.count("snapshots"), 0);
}

#[test]
fn bad_and_unknown_ids_are_refused() {
    let fixture = Fixture::new();
    let unknown = uuid::Uuid::new_v4().to_string();
    for (result, code) in [
        (fixture.capture("../x").err(), ErrorCode::InvalidInput),
        (fixture.capture(&unknown).err(), ErrorCode::UnknownFile),
        (
            fixture.service().list_snapshots(&unknown).err(),
            ErrorCode::UnknownFile,
        ),
        (
            fixture.service().read_snapshot("not-an-id").err(),
            ErrorCode::InvalidInput,
        ),
        (
            fixture.service().read_snapshot(&unknown).err(),
            ErrorCode::UnknownSnapshot,
        ),
    ] {
        assert_eq!(result.map(|error| error.code), Some(code));
    }
}

#[test]
fn a_damaged_copy_is_reported_and_never_repaired() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    let copy = stored_copy(&fixture, &snapshot.sha256);
    // Same length, other bytes: only the SHA-256 shows the damage.
    fs::write(&copy, "buy MILK\n").unwrap();

    let error = fixture
        .service()
        .read_snapshot(&snapshot.snapshot_id)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotCorrupt);
    assert_eq!(error.stage, ErrorStage::Memory);
    assert_eq!(error.file_id.as_deref(), Some(file.file_id.as_str()));
    let text_error = fixture
        .service()
        .snapshot_text(&snapshot.snapshot_id)
        .unwrap_err();
    assert_eq!(text_error.code, ErrorCode::SnapshotCorrupt);
    assert_eq!(fs::read(&copy).unwrap(), b"buy MILK\n");
}

#[test]
fn a_missing_copy_is_reported_and_not_rebuilt_from_the_file() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    let copy = stored_copy(&fixture, &snapshot.sha256);
    fs::remove_file(&copy).unwrap();

    let error = fixture
        .service()
        .read_snapshot(&snapshot.snapshot_id)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotUnavailable);
    assert!(!copy.exists());
}

#[test]
fn a_copy_that_was_replaced_by_a_link_is_not_followed() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    let copy = stored_copy(&fixture, &snapshot.sha256);
    fs::remove_file(&copy).unwrap();
    std::os::unix::fs::symlink(fixture.path("todo.txt"), &copy).unwrap();
    let error = fixture
        .service()
        .read_snapshot(&snapshot.snapshot_id)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotUnavailable);
}

#[test]
fn a_damaged_existing_copy_is_never_overwritten_by_a_capture() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let copy = stored_copy(&fixture, &file.sha256);
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::write(&copy, "something else\n").unwrap();

    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotCorrupt);
    assert_eq!(fixture.count("snapshots"), 0);
    assert_eq!(fs::read(&copy).unwrap(), b"something else\n");
}

#[test]
fn files_with_the_same_bytes_share_one_copy_but_keep_their_own_snapshots() {
    let fixture = Fixture::new();
    fs::write(fixture.path("Taxes/copy.txt"), "buy milk\n").unwrap();
    let first = fixture.register("todo.txt");
    let second = fixture.register("Taxes/copy.txt");
    assert_ne!(first.file_id, second.file_id);
    let a = fixture.capture(&first.file_id).unwrap().snapshot;
    let b = fixture.capture(&second.file_id).unwrap().snapshot;
    assert_ne!(a.snapshot_id, b.snapshot_id);
    assert_eq!((a.sequence, b.sequence), (1, 1));
    assert_eq!(a.sha256, b.sha256);
    let copy = stored_copy(&fixture, &a.sha256);
    let shard = fs::read_dir(copy.parent().unwrap()).unwrap().count();
    assert_eq!(shard, 1);
    for id in [&a.snapshot_id, &b.snapshot_id] {
        assert_eq!(
            fixture.service().read_snapshot(id).unwrap().1,
            b"buy milk\n"
        );
    }
}

#[test]
fn snapshot_text_uses_the_name_the_file_had_when_captured() {
    let fixture = Fixture::new();
    let file = fixture.register("Notes/2026/plan.md");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    let (same, text) = fixture
        .service()
        .snapshot_text(&snapshot.snapshot_id)
        .unwrap();
    assert_eq!(same, snapshot);
    assert_eq!(text.summary.status, TextStatus::Complete);
    assert_eq!(text.text, "# Plan\n\n- ship M1\n");
}

#[test]
fn snapshots_survive_a_restart_and_unfinished_copies_are_cleared() {
    let mut fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    let incoming = fixture
        .app
        .path()
        .join("Traeva")
        .join(SNAPSHOT_DIR)
        .join(INCOMING_DIR);
    fs::create_dir_all(&incoming).unwrap();
    fs::write(incoming.join(uuid::Uuid::new_v4().to_string()), "half").unwrap();

    fixture.restart();
    assert_eq!(fs::read_dir(&incoming).unwrap().count(), 0);
    let list = fixture.service().list_snapshots(&file.file_id).unwrap();
    assert_eq!(list, std::slice::from_ref(&snapshot));
    let (_, bytes) = fixture
        .service()
        .read_snapshot(&snapshot.snapshot_id)
        .unwrap();
    assert_eq!(bytes, b"buy milk\n");
}

#[test]
fn a_stored_copy_is_flushed_and_has_one_link() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    let meta = fs::symlink_metadata(stored_copy(&fixture, &snapshot.sha256)).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.nlink(), 1);
    let incoming = fixture
        .app
        .path()
        .join("Traeva")
        .join(SNAPSHOT_DIR)
        .join(INCOMING_DIR);
    assert_eq!(fs::read_dir(incoming).unwrap().count(), 0);
}

#[test]
fn snapshot_rows_cannot_be_changed_or_deleted() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    fixture.capture(&file.file_id).unwrap();
    let db = fixture.db();
    assert!(
        db.execute("UPDATE snapshots SET relative_path = 'x'", [])
            .is_err()
    );
    assert!(db.execute("DELETE FROM snapshots", []).is_err());
    assert_eq!(fixture.count("snapshots"), 1);
}

#[test]
fn copy_names_are_only_ever_a_lowercase_sha256() {
    assert!(is_valid_hash(&"ab".repeat(32)));
    for bad in [
        "AB".repeat(32),
        "ab".repeat(31),
        format!("../{}", "a".repeat(61)),
        format!("{}/x", "a".repeat(62)),
    ] {
        assert!(!is_valid_hash(&bad), "{bad}");
    }
}

#[test]
fn a_file_replaced_and_chosen_again_during_the_capture_records_nothing() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let mut newer = None;
    let error = fixture
        .service()
        .capture_snapshot_with(&file.file_id, &mut |root, parent, name| {
            let read = quick_read(parent, name, root.id().dev);
            // While Traeva reads: another app replaces the file, and the new
            // file is chosen in the window.
            fs::write(fixture.path("todo.tmp"), "new list\n").unwrap();
            fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();
            newer = Some(fixture.register("todo.txt"));
            read
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanging);
    assert_eq!(fixture.count("snapshots"), 0);

    // The new file's record is untouched and stays the present one.
    let newer = newer.unwrap();
    let store = fixture.service().store().unwrap();
    let (row, present) = store.get_file(&newer.file_id).unwrap().unwrap();
    assert!(present);
    assert_eq!(row.sha256, newer.sha256);
    assert!(!store.get_file(&file.file_id).unwrap().unwrap().1);
    assert_eq!(fixture.count("files"), 2);
}

#[test]
fn a_file_edited_and_chosen_again_during_the_capture_records_nothing() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let error = fixture
        .service()
        .capture_snapshot_with(&file.file_id, &mut |root, parent, name| {
            let read = quick_read(parent, name, root.id().dev);
            fs::write(fixture.path("todo.txt"), "buy milk\nand tea\n").unwrap();
            fixture.register("todo.txt");
            read
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanging);
    assert_eq!(fixture.count("snapshots"), 0);
    let (row, _) = fixture
        .service()
        .store()
        .unwrap()
        .get_file(&file.file_id)
        .unwrap()
        .unwrap();
    // The newer observation from choosing the file again is kept.
    assert_eq!(
        row.sha256,
        crate::observe::sha256_hex(b"buy milk\nand tea\n")
    );
}

#[test]
fn a_record_moved_elsewhere_during_the_capture_records_nothing() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let error = fixture
        .service()
        .capture_snapshot_with(&file.file_id, &mut |root, parent, name| {
            let read = quick_read(parent, name, root.id().dev);
            // Only a move changes where a record says the file is, and moves
            // wait for the capture; this stands in for one that did not.
            fixture
                .db()
                .execute(
                    "UPDATE files SET relative_path = 'Taxes/todo.txt' WHERE file_id = ?1",
                    [&file.file_id],
                )
                .unwrap();
            read
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanging);
    assert_eq!(fixture.count("snapshots"), 0);
}

/// A folder outside app data, holding a file that must never be touched.
fn outside_with_a_file() -> (tempfile::TempDir, PathBuf) {
    let outside = tempfile::tempdir().unwrap();
    let incoming = outside.path().join(INCOMING_DIR);
    fs::create_dir_all(&incoming).unwrap();
    let keep = incoming.join(uuid::Uuid::new_v4().to_string());
    fs::write(&keep, "not Traeva's").unwrap();
    (outside, keep)
}

#[test]
fn a_link_in_place_of_the_snapshot_folder_is_never_followed() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let (outside, keep) = outside_with_a_file();
    std::os::unix::fs::symlink(outside.path(), snapshots_dir(&fixture)).unwrap();
    let before = tree_of(outside.path());

    let error = fixture.capture(&file.file_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::AppDataUnavailable);
    assert_eq!(fixture.count("snapshots"), 0);
    clear_incoming(&fixture.app.path().join("Traeva"));
    assert_eq!(tree_of(outside.path()), before);
    assert!(keep.exists());
}

#[test]
fn a_link_in_place_of_incoming_or_a_shard_is_never_followed() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let (outside, keep) = outside_with_a_file();
    fs::create_dir_all(snapshots_dir(&fixture)).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join(INCOMING_DIR),
        snapshots_dir(&fixture).join(INCOMING_DIR),
    )
    .unwrap();
    let before = tree_of(outside.path());
    assert_eq!(
        fixture.capture(&file.file_id).unwrap_err().code,
        ErrorCode::AppDataUnavailable
    );
    clear_incoming(&fixture.app.path().join("Traeva"));
    assert_eq!(tree_of(outside.path()), before);
    assert!(keep.exists());

    // The same for the folder named after the first two hex digits.
    fs::remove_file(snapshots_dir(&fixture).join(INCOMING_DIR)).unwrap();
    // The attempt above already made the real (empty) folder; swap it for a link.
    fs::remove_dir(snapshots_dir(&fixture).join(&file.sha256[..2])).unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        snapshots_dir(&fixture).join(&file.sha256[..2]),
    )
    .unwrap();
    assert_eq!(
        fixture.capture(&file.file_id).unwrap_err().code,
        ErrorCode::AppDataUnavailable
    );
    assert_eq!(tree_of(outside.path()), before);
    assert_eq!(fixture.count("snapshots"), 0);
}

#[test]
fn only_files_named_like_traevas_own_are_cleared_at_start() {
    let mut fixture = Fixture::new();
    let incoming = snapshots_dir(&fixture).join(INCOMING_DIR);
    fs::create_dir_all(&incoming).unwrap();
    let unfinished = incoming.join(uuid::Uuid::new_v4().to_string());
    fs::write(&unfinished, "half").unwrap();
    fs::write(incoming.join("notes.txt"), "someone else's").unwrap();
    fs::create_dir(incoming.join(uuid::Uuid::new_v4().to_string())).unwrap();

    fixture.restart();
    assert!(!unfinished.exists());
    assert_eq!(fs::read_dir(&incoming).unwrap().count(), 2);
    assert!(incoming.join("notes.txt").exists());
}

fn tree_of(path: &Path) -> Vec<tree::Entry> {
    tree::snapshot(path)
}

// A saved .docx version's local preview (#66, the student's limited Go).

const WORD_V1: &[u8] = include_bytes!("../tests/fixtures/docx/word-basic-v1.docx");

#[test]
fn a_saved_word_version_is_previewed_from_its_snapshot_only() {
    let fixture = Fixture::new();
    fs::write(fixture.path("report.docx"), WORD_V1).unwrap();
    let file = fixture.register("report.docx");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    // The file changes afterwards: the preview still shows the saved version.
    fs::write(fixture.path("report.docx"), b"not a word document").unwrap();

    let preview = fixture
        .service()
        .preview_docx_version(&snapshot.snapshot_id)
        .unwrap();
    let expected = crate::docx::read_docx_cached(WORD_V1);
    assert_eq!(preview.snapshot, snapshot);
    assert_eq!(preview.extraction.summary, expected.summary);
    assert_eq!(preview.extraction.text, expected.text);
    assert_eq!(preview.extraction.summary.reader, crate::DOCX_CACHED_READER);
    assert!(preview.extraction.text.contains("Traeva DOCX validation"));
    // What changes are compared and sent on stays the default reader.
    let (_, default) = fixture
        .service()
        .snapshot_text(&snapshot.snapshot_id)
        .unwrap();
    assert_eq!(default.summary.reader, crate::DOCX_READER);

    // The text never shows in Debug output, and nothing is written.
    assert!(!format!("{preview:?}").contains("Traeva DOCX validation"));
    assert!(
        !fixture
            .app
            .path()
            .join("Traeva")
            .join(crate::OPENED_DIR)
            .exists()
    );

    let json = serde_json::to_value(&preview).unwrap();
    let keys = |value: &serde_json::Value| {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&json), ["extraction", "snapshot"]);
    assert_eq!(keys(&json["extraction"]), ["summary", "text"]);
    assert_eq!(
        keys(&json["extraction"]["summary"]),
        ["chars", "covered", "left_out", "note", "reader", "status"]
    );
}

#[test]
fn a_version_that_is_not_a_word_document_previews_as_unsupported() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;

    let preview = fixture
        .service()
        .preview_docx_version(&snapshot.snapshot_id)
        .unwrap();
    assert_eq!(preview.extraction.summary.status, TextStatus::Unsupported);
    assert_eq!(preview.extraction.summary.chars, 0);
    assert!(preview.extraction.text.is_empty());
    assert!(preview.extraction.summary.note.is_some());
}

#[test]
fn a_preview_of_a_damaged_or_unknown_version_is_refused() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let invalid = service.preview_docx_version("not-an-id").unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidInput);
    let unknown = service
        .preview_docx_version("00000000-0000-4000-8000-000000000000")
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::UnknownSnapshot);

    fs::write(fixture.path("report.docx"), WORD_V1).unwrap();
    let file = fixture.register("report.docx");
    let snapshot = fixture.capture(&file.file_id).unwrap().snapshot;
    // Same length, one byte changed: only the SHA-256 shows the damage. The
    // file itself is still the same Word document and is not read instead.
    let copy = stored_copy(&fixture, &snapshot.sha256);
    let mut bytes = fs::read(&copy).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    fs::write(&copy, &bytes).unwrap();
    let damaged = service
        .preview_docx_version(&snapshot.snapshot_id)
        .unwrap_err();
    assert_eq!(damaged.code, ErrorCode::SnapshotCorrupt);
    assert_eq!(damaged.stage, ErrorStage::Memory);
}

// The generic local preview (contract §4.7, v23).

#[test]
fn any_saved_version_is_previewed_by_the_name_it_was_captured_under() {
    let fixture = Fixture::new();
    let service = fixture.service();
    // TXT/MD: what Memory compares.
    let todo = fixture.register("todo.txt");
    let text = fixture.capture(&todo.file_id).unwrap().snapshot;
    let preview = service.preview_version_text(&text.snapshot_id).unwrap();
    let (_, read) = service.snapshot_text(&text.snapshot_id).unwrap();
    assert_eq!(preview.snapshot, text);
    assert_eq!(preview.extraction.summary, read.summary);
    assert_eq!(preview.extraction.text, read.text);

    // .docx: the reader that keeps citations' cached text.
    fs::write(fixture.path("Report.DOCX"), WORD_V1).unwrap();
    let report = fixture.register("Report.DOCX");
    let word = fixture.capture(&report.file_id).unwrap().snapshot;
    let preview = service.preview_version_text(&word.snapshot_id).unwrap();
    assert_eq!(preview.extraction.summary.reader, crate::DOCX_CACHED_READER);
    assert_eq!(
        preview.extraction.text,
        crate::docx::read_docx_cached(WORD_V1).text
    );
    // Changing the file afterwards changes nothing: the saved version is read.
    fs::write(fixture.path("Report.DOCX"), b"other").unwrap();
    assert_eq!(
        service.preview_version_text(&word.snapshot_id).unwrap(),
        preview
    );

    // A .pptx gets its preview projection, hidden slides marked.
    fs::write(
        fixture.path("Deck.PPTX"),
        include_bytes!("../tests/fixtures/pptx/basic-source.pptx"),
    )
    .unwrap();
    let deck = fixture.register("Deck.PPTX");
    let slides = fixture.capture(&deck.file_id).unwrap().snapshot;
    let preview = service.preview_version_text(&slides.snapshot_id).unwrap();
    assert_eq!(
        preview.extraction.summary.reader,
        crate::PPTX_PREVIEW_READER
    );
    assert!(!preview.extraction.text.is_empty());

    // A .xlsx gets its preview projection; bytes that are no workbook give
    // no text, with the reader's own failure.
    fs::write(fixture.path("book.XLSX"), b"PK not a workbook").unwrap();
    let book = fixture.register("book.XLSX");
    let saved = fixture.capture(&book.file_id).unwrap().snapshot;
    let preview = service.preview_version_text(&saved.snapshot_id).unwrap();
    assert_eq!(
        preview.extraction.summary.reader,
        crate::XLSX_PREVIEW_READER
    );
    assert!(matches!(
        preview.extraction.summary.status,
        TextStatus::Failed | TextStatus::Unsupported
    ));
    assert!(preview.extraction.text.is_empty());
    assert!(
        !fixture
            .app
            .path()
            .join("Traeva")
            .join(crate::OPENED_DIR)
            .exists()
    );

    let invalid = service.preview_version_text("not-an-id").unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidInput);
    let unknown = service
        .preview_version_text("00000000-0000-4000-8000-000000000000")
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::UnknownSnapshot);
}
