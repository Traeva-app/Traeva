//! Tests for export.rs (#35), all offline, synthetic files only.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use super::*;
use crate::changes::tests::file_with_versions;
use crate::snapshot::SNAPSHOT_DIR;
use crate::test_fixture::Fixture;

const V1: &[u8] = b"Smith 2019 says A.\n";
const V2: &[u8] = b"Smith (2019, p. 4) says A.\n";

fn opened_dir(f: &Fixture) -> PathBuf {
    f.app.path().join("Traeva").join(OPENED_DIR)
}

/// Folders left in `opened`: a staged copy that was not cleaned up.
fn staged(f: &Fixture) -> usize {
    fs::read_dir(opened_dir(f)).map_or(0, |entries| entries.count())
}

fn versions(f: &Fixture, name: &str) -> Vec<SnapshotRef> {
    let file = file_with_versions(f, name, &[V1, V2]);
    f.service().list_snapshots(&file).unwrap()
}

/// A request the window could send without a preview: its token is well
/// formed but stands for nothing, so any check before the token decides.
fn request(f: &Fixture, snapshot: &SnapshotRef, folder: &str, name: &str) -> ExportRequest {
    ExportRequest {
        snapshot_id: snapshot.snapshot_id.clone(),
        root_id: f.root_id.clone(),
        folder: folder.to_string(),
        name: name.to_string(),
        page_token: "a".repeat(64),
    }
}

/// The request a preview of `relative` (inside the root) gives.
fn previewed(f: &Fixture, snapshot: &SnapshotRef, relative: &str) -> ExportRequest {
    let preview = f
        .service()
        .preview_export(&snapshot.snapshot_id, &f.root_id, &f.path(relative))
        .unwrap();
    ExportRequest {
        snapshot_id: snapshot.snapshot_id.clone(),
        root_id: preview.root_id,
        folder: preview.folder,
        name: preview.name,
        page_token: preview.page_token,
    }
}

fn code(result: Result<ExportedVersion, AppError>) -> ErrorCode {
    result.unwrap_err().code
}

#[test]
fn a_preview_only_looks_and_the_export_writes_the_exact_version_as_a_new_file() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let picked = f.path("Taxes/essay (version 1).md");

    let preview = f
        .service()
        .preview_export(&first.snapshot_id, &f.root_id, &picked)
        .unwrap();
    assert_eq!(preview.folder, "Taxes");
    assert_eq!(preview.name, "essay (version 1).md");
    assert!(
        preview
            .display_path
            .ends_with("/Taxes/essay (version 1).md")
    );
    assert_eq!(preview.byte_length, V1.len() as i64);
    assert!(!picked.exists(), "a preview writes nothing");
    assert_eq!(staged(&f), 0);

    let done = f
        .service()
        .export_version(&previewed(&f, &first, "Taxes/essay (version 1).md"))
        .unwrap();
    assert_eq!(done.display_path, preview.display_path);
    assert_eq!(done.sha256, first.sha256);
    assert_eq!(fs::read(&picked).unwrap(), V1);
    // An ordinary new file: the mode any new file gets here (the umask).
    fs::write(f.path("Taxes/probe.txt"), b"").unwrap();
    let mode = |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&picked), mode(&f.path("Taxes/probe.txt")));
    assert_eq!(staged(&f), 0, "nothing is left in app data");
    // The saved version is untouched, and the user's own file too.
    assert_eq!(f.service().read_snapshot(&first.snapshot_id).unwrap().1, V1);
    assert_eq!(fs::read(f.path("essay.md")).unwrap(), V2);
}

#[test]
fn the_root_itself_can_be_the_folder() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let preview = f
        .service()
        .preview_export(&first.snapshot_id, &f.root_id, &f.path("essay v1.md"))
        .unwrap();
    assert_eq!(preview.folder, "");
    f.service()
        .export_version(&previewed(&f, &first, "essay v1.md"))
        .unwrap();
    assert_eq!(fs::read(f.path("essay v1.md")).unwrap(), V1);
}

#[test]
fn a_file_is_never_replaced_whatever_its_case() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    fs::write(f.path("Taxes/Mine.md"), b"mine\n").unwrap();
    for name in ["Mine.md", "mine.md", "MINE.md"] {
        let shown = f.service().preview_export(
            &first.snapshot_id,
            &f.root_id,
            &f.path(&format!("Taxes/{name}")),
        );
        assert_eq!(shown.unwrap_err().code, ErrorCode::TargetConflict, "{name}");
        // Taken after a preview: the export checks again, and the move
        // itself never replaces.
        let refused = f
            .service()
            .export_version(&request(&f, &first, "Taxes", name));
        assert_eq!(code(refused), ErrorCode::TargetConflict, "{name}");
    }
    assert_eq!(fs::read(f.path("Taxes/Mine.md")).unwrap(), b"mine\n");
    assert_eq!(staged(&f), 0);
}

#[test]
fn a_place_outside_the_root_is_refused() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let elsewhere = tempfile::tempdir().unwrap();
    let refused = f.service().preview_export(
        &first.snapshot_id,
        &f.root_id,
        &elsewhere.path().join("essay.md"),
    );
    assert_eq!(refused.unwrap_err().code, ErrorCode::OutsideRoot);
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[test]
fn names_and_folders_that_are_not_plain_ones_are_refused() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let said = |name: &str| {
        f.service()
            .export_version(&request(&f, &first, "Taxes", name))
            .unwrap_err()
            .message
    };
    assert!(said(".hidden.md").contains("starts with a dot"));
    assert!(said("a:b.md").contains("holds a colon"));
    assert!(said("essay.txt").contains("must end in .md"));
    for name in [
        ".hidden.md",
        "a:b.md",
        "essay.txt",
        "essay",
        " essay.md",
        "",
    ] {
        let refused = f
            .service()
            .export_version(&request(&f, &first, "Taxes", name));
        assert_eq!(code(refused), ErrorCode::InvalidInput, "{name:?}");
    }
    for folder in ["../x", "Taxes/..", "/Taxes", "Taxes//x", "./Taxes"] {
        let refused = f
            .service()
            .export_version(&request(&f, &first, folder, "essay.md"));
        assert_eq!(code(refused), ErrorCode::InvalidInput, "{folder:?}");
    }
    // The original's extension in another case is the same extension.
    f.service()
        .export_version(&previewed(&f, &first, "Taxes/essay.MD"))
        .unwrap();
    let extra = serde_json::json!({
        "snapshot_id": first.snapshot_id, "root_id": f.root_id, "folder": "Taxes",
        "name": "x.md", "page_token": "a".repeat(64), "overwrite": true,
    });
    assert!(
        serde_json::from_value::<ExportRequest>(extra).is_err(),
        "a request with a field the core does not know is refused"
    );
    let bad = ExportRequest {
        page_token: "not a token".to_string(),
        ..request(&f, &first, "Taxes", "x.md")
    };
    assert_eq!(
        code(f.service().export_version(&bad)),
        ErrorCode::InvalidInput
    );
    assert_eq!(staged(&f), 0);
}

#[test]
fn a_link_a_hidden_folder_or_a_missing_folder_is_never_written_into() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    symlink(f.path("Taxes"), f.path("Linked")).unwrap();
    let refused = f
        .service()
        .export_version(&request(&f, &first, "Linked", "essay.md"));
    assert_eq!(code(refused), ErrorCode::UnsupportedLocation);
    fs::create_dir(f.path(".secret")).unwrap();
    let refused = f
        .service()
        .export_version(&request(&f, &first, ".secret", "essay.md"));
    assert_eq!(code(refused), ErrorCode::ProtectedLocation);
    let refused = f
        .service()
        .export_version(&request(&f, &first, "Gone", "essay.md"));
    assert_eq!(code(refused), ErrorCode::FileNotFound);
    assert!(!f.path("Taxes/essay.md").exists());
    assert!(!f.path(".secret/essay.md").exists());
    assert_eq!(staged(&f), 0);
}

#[test]
fn nothing_is_exported_while_an_organize_action_in_the_root_is_not_settled() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let plan_file = f.register("Notes/2026/plan.md");
    let plan = f.plan(&plan_file, "Taxes", &[]);
    f.insert_action(&plan.plan_id, "running");
    let refused = f
        .service()
        .export_version(&request(&f, &first, "Taxes", "essay.md"));
    assert_eq!(code(refused), ErrorCode::UnresolvedAction);
    assert!(!f.path("Taxes/essay.md").exists());
    assert_eq!(staged(&f), 0);
}

#[test]
fn a_removed_folder_is_refused() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    f.service().revoke_root(&f.root_id).unwrap();
    let refused = f
        .service()
        .export_version(&request(&f, &first, "Taxes", "essay.md"));
    assert_eq!(code(refused), ErrorCode::RootRevoked);
    assert!(!f.path("Taxes/essay.md").exists());
}

#[test]
fn only_text_versions_are_exported() {
    let f = Fixture::new();
    let first = versions(&f, "setup.terminal").remove(0);
    let refused = f
        .service()
        .export_version(&request(&f, &first, "Taxes", "setup.terminal"));
    assert_eq!(code(refused), ErrorCode::UnsupportedFile);
    assert!(!f.path("Taxes/setup.terminal").exists());
}

#[test]
fn a_damaged_saved_version_is_not_exported() {
    let f = Fixture::new();
    let later = versions(&f, "essay.md").remove(1);
    let stored = f
        .app
        .path()
        .join("Traeva")
        .join(SNAPSHOT_DIR)
        .join(&later.sha256[..2])
        .join(&later.sha256);
    fs::set_permissions(&stored, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&stored, b"Smith (2019, p. 5) says A.\n").unwrap();
    let refused = f
        .service()
        .export_version(&request(&f, &later, "Taxes", "essay.md"));
    assert_eq!(code(refused), ErrorCode::SnapshotCorrupt);
    assert!(!f.path("Taxes/essay.md").exists());
    assert_eq!(staged(&f), 0);
}

// ---------------------------------------------------------------------------
// The place the preview showed, and the root while the export waits
// ---------------------------------------------------------------------------

#[test]
fn a_folder_replaced_since_the_preview_is_refused() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let shown = previewed(&f, &first, "Taxes/essay.md");
    fs::rename(f.path("Taxes"), f.path("Taxes old")).unwrap();
    fs::create_dir(f.path("Taxes")).unwrap();
    let refused = f.service().export_version(&shown);
    assert_eq!(code(refused), ErrorCode::PageOutdated);
    assert!(!f.path("Taxes/essay.md").exists());
    assert!(!f.path("Taxes old/essay.md").exists());
    assert_eq!(staged(&f), 0);
}

/// Runs `export` in another thread while this one holds the execution lock
/// (as Organize's apply or a Memory capture would), does `meanwhile`, then
/// lets the lock go and returns the export's result.
fn export_after_waiting(
    f: &Fixture,
    shown: &ExportRequest,
    meanwhile: impl FnOnce(),
) -> Result<ExportedVersion, AppError> {
    std::thread::scope(|scope| {
        let held = f.service().lock_execution();
        let waiting = scope.spawn(|| f.service().export_version(shown));
        // Long enough for the export to reach the lock.
        std::thread::sleep(std::time::Duration::from_millis(300));
        meanwhile();
        drop(held);
        waiting.join().unwrap()
    })
}

#[test]
fn a_root_removed_while_the_export_waits_is_never_written_into() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let shown = previewed(&f, &first, "Taxes/essay.md");
    let refused = export_after_waiting(&f, &shown, || {
        f.service().revoke_root(&f.root_id).unwrap();
    });
    assert_eq!(code(refused), ErrorCode::RootRevoked);
    assert!(!f.path("Taxes/essay.md").exists());
    assert_eq!(staged(&f), 0);
}

#[test]
fn a_root_replaced_while_the_export_waits_is_never_written_into() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let shown = previewed(&f, &first, "Taxes/essay.md");
    let root = f.path("");
    let moved = root.with_file_name(format!(
        "{}-moved",
        root.file_name().unwrap().to_string_lossy()
    ));
    let refused = export_after_waiting(&f, &shown, || {
        fs::rename(&root, &moved).unwrap();
        fs::create_dir_all(root.join("Taxes")).unwrap();
    });
    let error = refused.unwrap_err();
    assert_ne!(error.code, ErrorCode::ResultUnconfirmed, "{error:?}");
    assert!(!root.join("Taxes/essay.md").exists());
    assert!(
        !moved.join("Taxes/essay.md").exists(),
        "not into the folder moved away"
    );
    assert_eq!(staged(&f), 0);
    fs::remove_dir_all(&moved).unwrap();
}

#[test]
fn a_move_macos_refuses_leaves_nothing_behind() {
    let f = Fixture::new();
    let first = versions(&f, "essay.md").remove(0);
    let shown = previewed(&f, &first, "Taxes/essay.md");
    fs::set_permissions(f.path("Taxes"), fs::Permissions::from_mode(0o555)).unwrap();
    let refused = f.service().export_version(&shown);
    fs::set_permissions(f.path("Taxes"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code(refused), ErrorCode::OperationRefused);
    assert!(!f.path("Taxes/essay.md").exists());
    assert_eq!(
        staged(&f),
        0,
        "the copy and its folder are removed from app data"
    );
}
