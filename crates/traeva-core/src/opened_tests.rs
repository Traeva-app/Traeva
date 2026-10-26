//! Tests for opened.rs (#35), all offline, synthetic files only.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use super::*;
use crate::changes::tests::file_with_versions;
use crate::snapshot::SNAPSHOT_DIR;
use crate::test_fixture::Fixture;

const V1: &[u8] = b"Smith 2019 says A.\n";
const V2: &[u8] = b"Smith (2019, p. 4) says A.\n";

fn app_dir(f: &Fixture) -> PathBuf {
    f.app.path().join("Traeva")
}

/// Every file under `dir`, as relative paths, sorted.
fn files_under(dir: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                found.push(path.strip_prefix(dir).unwrap().display().to_string());
            }
        }
    }
    found.sort();
    found
}

fn version(sequence: i64, relative_path: &str) -> SnapshotRef {
    SnapshotRef {
        snapshot_id: uuid::Uuid::new_v4().to_string(),
        file_id: uuid::Uuid::new_v4().to_string(),
        sequence,
        file_revision: 1,
        root_id: uuid::Uuid::new_v4().to_string(),
        relative_path: relative_path.to_string(),
        sha256: "0".repeat(64),
        byte_length: 0,
        captured_at_ms: 0,
    }
}

#[test]
fn a_copy_holds_the_exact_bytes_read_only_in_app_data() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    let first = f.service().list_snapshots(&file).unwrap()[0].clone();
    let root_before = files_under(f.root.path());

    let copy = f.service().copy_for_opening(&first.snapshot_id).unwrap();
    let path = PathBuf::from(&copy.path);
    assert!(path.starts_with(app_dir(&f).join(OPENED_DIR)), "{path:?}");
    assert_eq!(path.file_name().unwrap(), "essay (version 1).txt");
    assert_eq!(fs::read(&path).unwrap(), V1);
    assert_eq!(copy.sha256, first.sha256);
    assert_eq!(copy.byte_length, V1.len() as i64);
    assert_eq!(copy.snapshot, first);
    assert!(copy.can_open, "a text file Memory reads");
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o444, "read-only");
    assert_eq!(
        files_under(f.root.path()),
        root_before,
        "nothing in the root"
    );
}

#[test]
fn each_open_makes_a_new_copy_and_never_writes_over_one() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    let first = f.service().list_snapshots(&file).unwrap()[0].clone();
    let one = f.service().copy_for_opening(&first.snapshot_id).unwrap();
    // The user makes the copy writable and changes it in the app.
    fs::set_permissions(&one.path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&one.path, b"changed in the app\n").unwrap();

    let two = f.service().copy_for_opening(&first.snapshot_id).unwrap();
    assert_ne!(one.path, two.path);
    assert_eq!(fs::read(&two.path).unwrap(), V1, "a fresh copy");
    assert_eq!(
        fs::read(&one.path).unwrap(),
        b"changed in the app\n",
        "left alone"
    );
    // The saved version itself is unchanged.
    assert_eq!(f.service().read_snapshot(&first.snapshot_id).unwrap().1, V1);
}

#[test]
fn an_unknown_or_damaged_version_makes_no_copy() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    let service = f.service();
    assert_eq!(
        service.copy_for_opening("not-an-id").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        service
            .copy_for_opening(&uuid::Uuid::new_v4().to_string())
            .unwrap_err()
            .code,
        ErrorCode::UnknownSnapshot
    );

    let later = service.list_snapshots(&file).unwrap()[1].clone();
    let stored = app_dir(&f)
        .join(SNAPSHOT_DIR)
        .join(&later.sha256[..2])
        .join(&later.sha256);
    fs::set_permissions(&stored, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&stored, b"Smith (2019, p. 5) says A.\n").unwrap();
    assert_eq!(
        service
            .copy_for_opening(&later.snapshot_id)
            .unwrap_err()
            .code,
        ErrorCode::SnapshotCorrupt
    );
    assert!(files_under(&app_dir(&f).join(OPENED_DIR)).is_empty());
}

#[test]
fn a_link_in_place_of_the_copies_folder_is_not_followed() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    let first = f.service().list_snapshots(&file).unwrap()[0].clone();
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), app_dir(&f).join(OPENED_DIR)).unwrap();

    let error = f
        .service()
        .copy_for_opening(&first.snapshot_id)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AppDataUnavailable);
    assert!(
        files_under(elsewhere.path()).is_empty(),
        "nothing written there"
    );
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[test]
fn a_test_folder_with_copies_in_it_is_still_accepted_at_the_next_start() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    let first = f.service().list_snapshots(&file).unwrap()[0].clone();
    f.service().copy_for_opening(&first.snapshot_id).unwrap();
    let home = tempfile::tempdir().unwrap();
    let chosen = crate::app_data::choose(
        Some(app_dir(&f).into_os_string()),
        Some(home.path().as_os_str().to_os_string()),
    );
    assert!(
        matches!(chosen, Ok(crate::app_data::Chosen::Test(_))),
        "{chosen:?}"
    );
}

#[test]
fn only_text_files_memory_reads_keep_their_extension_and_may_be_opened() {
    let cases = [
        ("Drafts/essay.md", "essay (version 3).md", true),
        ("Notes.TXT", "Notes (version 3).TXT", true),
        ("plan.markdown", "plan (version 3).markdown", true),
        (".hidden.md", "version-3.md", true),
        // Anything else is never handed to the system: `open` picks the
        // app by the extension, and a copy has no quarantine mark.
        ("setup.terminal", "version-3", false),
        ("Desktop.fileloc", "version-3", false),
        ("installer.dmg", "version-3", false),
        ("report.docx", "version-3", false),
        ("archive.tar.gz", "version-3", false),
        ("notes", "version-3", false),
        (".profile", "version-3", false),
        ("Drafts/..", "version-3", false),
    ];
    for (relative_path, name, can_open) in cases {
        assert_eq!(
            copy_name(&version(3, relative_path), Some(3)),
            (name.to_string(), can_open),
            "{relative_path}"
        );
    }
    let long = format!("{}.txt", "x".repeat(MAX_NAME_BYTES));
    assert_eq!(
        copy_name(&version(3, &long), Some(3)),
        ("version-3.txt".to_string(), true)
    );
}

#[test]
fn a_copy_of_another_kind_of_file_is_made_but_may_not_be_opened() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "setup.terminal", &[V1, V2]);
    let first = f.service().list_snapshots(&file).unwrap()[0].clone();
    let copy = f.service().copy_for_opening(&first.snapshot_id).unwrap();
    assert!(!copy.can_open);
    assert_eq!(PathBuf::from(&copy.path).file_name().unwrap(), "version-1");
    assert_eq!(fs::read(&copy.path).unwrap(), V1);
}

/// The copy says the version's number the history and search show: its
/// place on the record line, not the file's own count, which starts at 1
/// again after an editor wrote a new file (R2-MEM-10).
#[test]
fn a_copy_is_named_by_the_versions_number_on_its_line() {
    let f = Fixture::new().without_auto_link();
    let (old, new) = crate::changes::tests::continued_record(&f);
    let first_of_new = f.service().list_snapshots(&new).unwrap()[0].clone();
    assert_eq!(first_of_new.sequence, 1);
    let copy = f
        .service()
        .copy_for_opening(&first_of_new.snapshot_id)
        .unwrap();
    assert_eq!(
        PathBuf::from(&copy.path).file_name().unwrap(),
        "todo (version 3).txt",
        "old v1, old v2, then the new file's first version"
    );
    assert_eq!(copy.number, Some(3), "the page says the number in the name");
    let first_of_old = f.service().list_snapshots(&old).unwrap()[0].clone();
    let copy = f
        .service()
        .copy_for_opening(&first_of_old.snapshot_id)
        .unwrap();
    assert_eq!(
        PathBuf::from(&copy.path).file_name().unwrap(),
        "todo (version 1).txt"
    );
    assert_eq!(
        copy_name(&first_of_old, None).0,
        "todo (an older version).txt",
        "a version the line does not list has no number"
    );
}
