use crate::{prototype::*, tests::package};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
use traeva_core::{ErrorCode, OrganizeService, SnapshotRef};

fn capture(source: &Path, data: &Path) -> (OrganizeService, SnapshotRef) {
    let service = OrganizeService::start(data.to_path_buf());
    let root = service.authorize_root(source.parent().unwrap()).unwrap();
    let file = service.register_file(&root.root_id, source).unwrap().file;
    service.start_memory(&file.file_id).unwrap();
    service.stop_memory(&file.file_id).unwrap();
    let snapshot = service.list_snapshots(&file.file_id).unwrap()[0].clone();
    (service, snapshot)
}
fn document() -> Vec<u8> {
    package(
        "<w:p><w:r><w:t>immutable historical body</w:t></w:r></w:p>",
        &[],
    )
}

fn isolated() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("traeva-word-research-")
        .tempdir_in("/private/tmp")
        .unwrap()
}

#[test]
fn docx_copy_is_independent_readonly_and_production_open_stays_false() {
    let root = isolated();
    let data = isolated();
    let source = root.path().join("history.docx");
    let bytes = document();
    fs::write(&source, &bytes).unwrap();
    let before = fs::metadata(&source).unwrap();
    let (service, snapshot) = capture(&source, data.path());
    let one = service.copy_for_opening(&snapshot.snapshot_id).unwrap();
    assert!(!one.can_open);
    assert!(Path::new(&one.path).extension().is_none());
    let meta = fs::metadata(&one.path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o444);
    assert_eq!(meta.nlink(), 1);
    assert_ne!(meta.ino(), before.ino());
    assert_eq!(fs::read(&one.path).unwrap(), bytes);
    fs::set_permissions(&one.path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&one.path, b"synthetic viewer changed only its copy").unwrap();
    let two = service.copy_for_opening(&snapshot.snapshot_id).unwrap();
    assert_ne!(one.path, two.path);
    assert_eq!(fs::read(&two.path).unwrap(), bytes);
    assert_eq!(
        service.read_snapshot(&snapshot.snapshot_id).unwrap().1,
        bytes
    );
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::metadata(&source).unwrap().ino(), before.ino());
    assert_eq!(
        fs::metadata(&source).unwrap().permissions(),
        before.permissions()
    );
}

#[test]
fn historical_reader_uses_snapshot_after_source_changes_or_disappears() {
    let root = isolated();
    let data = isolated();
    let source = root.path().join("history.docx");
    let bytes = document();
    fs::write(&source, &bytes).unwrap();
    let (service, snapshot) = capture(&source, data.path());
    fs::write(&source, b"unrelated newer synthetic bytes").unwrap();
    assert_eq!(
        read_docx(&service.read_snapshot(&snapshot.snapshot_id).unwrap().1).text,
        "immutable historical body"
    );
    fs::rename(&source, root.path().join("moved-synthetic.docx")).unwrap();
    drop(service);
    let restarted = OrganizeService::start(data.path().to_path_buf());
    let copy = restarted.copy_for_opening(&snapshot.snapshot_id).unwrap();
    assert_eq!(fs::read(copy.path).unwrap(), bytes);
    assert!(!copy.can_open);
}

#[test]
fn missing_snapshot_refuses_copy_even_with_valid_source_present() {
    let root = isolated();
    let data = isolated();
    let source = root.path().join("history.docx");
    fs::write(&source, document()).unwrap();
    let (service, snapshot) = capture(&source, data.path());
    let blob = data
        .path()
        .join("snapshots")
        .join(&snapshot.sha256[..2])
        .join(&snapshot.sha256);
    fs::rename(blob, data.path().join("moved-synthetic-blob")).unwrap();
    assert_eq!(
        service
            .copy_for_opening(&snapshot.snapshot_id)
            .unwrap_err()
            .code,
        ErrorCode::SnapshotUnavailable
    );
    assert!(!data.path().join("opened").exists());
}
