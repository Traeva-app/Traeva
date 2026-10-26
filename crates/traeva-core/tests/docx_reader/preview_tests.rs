use crate::{
    cached_tests::package,
    docx::{DOCX_CACHED_READER, preview_docx_snapshot},
    *,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use traeva_core::{ErrorCode, ErrorStage};

struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    source: PathBuf,
    service: OrganizeService,
    snapshot: SnapshotRef,
}
fn fixture(name: &str, bytes: &[u8]) -> Fixture {
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-preview-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    let data = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    let source = root.join(name);
    fs::write(&source, bytes).unwrap();
    let service = OrganizeService::start(data.clone());
    let folder = service.authorize_root(&root).unwrap();
    let file = service
        .register_file(&folder.root_id, &source)
        .unwrap()
        .file;
    service.start_memory(&file.file_id).unwrap();
    service.stop_memory(&file.file_id).unwrap();
    let snapshot = service.list_snapshots(&file.file_id).unwrap()[0].clone();
    Fixture {
        _temp: temp,
        data,
        source,
        service,
        snapshot,
    }
}
fn document() -> Vec<u8> {
    package(
        r#"<w:p><w:r><w:t>old </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>ADDIN ZOTERO_ITEM SECRET</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>(Stored, 2000)</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        &[],
    )
}
fn blob(f: &Fixture) -> PathBuf {
    f.data
        .join("snapshots")
        .join(&f.snapshot.sha256[..2])
        .join(&f.snapshot.sha256)
}
fn state(path: &Path) -> (u64, u64, u32, i64, i64, i64, i64, Vec<u8>) {
    let m = fs::metadata(path).unwrap();
    (
        m.dev(),
        m.ino(),
        m.permissions().mode(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
        fs::read(path).unwrap(),
    )
}
fn tree(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            files.extend(tree(&p));
        } else {
            files.push((p.clone(), fs::read(p).unwrap()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}
#[test]
fn preview_has_no_writes_no_docx_open_and_keeps_cached_text_out_of_the_default_reader() {
    let f = fixture("history.docx", &document());
    let source = state(&f.source);
    let saved = state(&blob(&f));
    let app = tree(&f.data);
    let (reference, extraction) =
        preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id).unwrap();
    assert_eq!(reference, f.snapshot);
    assert_eq!(extraction.text, "old (Stored, 2000)");
    assert_eq!(extraction.summary.reader, DOCX_CACHED_READER);
    assert_eq!(extraction.summary.status, TextStatus::Partial);
    assert_eq!(state(&f.source), source);
    assert_eq!(state(&blob(&f)), saved);
    assert_eq!(tree(&f.data), app);
    assert!(!f.data.join("opened").exists());
    // Since #103 (the student's limited Go of 2026-10-11) the default reader
    // is docx-2, which leaves a citation field's cached text out.
    let default = f.service.snapshot_text(&f.snapshot.snapshot_id).unwrap().1;
    assert_eq!(default.summary.reader, traeva_core::DOCX_READER);
    assert!(!default.text.contains("Stored, 2000"));
    assert!(
        !f.service
            .copy_for_opening(&f.snapshot.snapshot_id)
            .unwrap()
            .can_open
    );
}
#[test]
fn old_snapshot_preview_survives_new_capture_source_removal_and_restart() {
    let f = fixture("history.DOCX", &document());
    let later = package("<w:p><w:r><w:t>new body</w:t></w:r></w:p>", &[]);
    fs::write(&f.source, later).unwrap();
    f.service.start_memory(&f.snapshot.file_id).unwrap();
    f.service.stop_memory(&f.snapshot.file_id).unwrap();
    let versions = f.service.list_snapshots(&f.snapshot.file_id).unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(
        preview_docx_snapshot(&f.service, &versions[1].snapshot_id)
            .unwrap()
            .1
            .text,
        "new body"
    );
    fs::remove_file(&f.source).unwrap();
    drop(f.service);
    let restarted = OrganizeService::start(f.data);
    let (reference, extraction) =
        preview_docx_snapshot(&restarted, &f.snapshot.snapshot_id).unwrap();
    assert_eq!(reference, f.snapshot);
    assert_eq!(extraction.text, "old (Stored, 2000)");
}
#[test]
fn missing_snapshot_has_memory_error_even_when_source_is_valid() {
    let f = fixture("history.docx", &document());
    fs::remove_file(blob(&f)).unwrap();
    let err = preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id).unwrap_err();
    assert_eq!(err.stage, ErrorStage::Memory);
    assert_eq!(err.code, ErrorCode::SnapshotUnavailable);
    assert_eq!(fs::read(&f.source).unwrap(), document());
}
#[test]
fn hash_mismatch_has_memory_error_and_never_uses_current_source() {
    let f = fixture("history.docx", &document());
    let p = blob(&f);
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    let mut damaged = fs::read(&p).unwrap();
    damaged[50] ^= 1;
    fs::write(p, damaged).unwrap();
    let err = preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id).unwrap_err();
    assert_eq!(err.stage, ErrorStage::Memory);
    assert_eq!(err.code, ErrorCode::SnapshotCorrupt);
}
#[test]
fn snapshot_symlink_is_not_followed() {
    let f = fixture("history.docx", &document());
    fs::remove_file(blob(&f)).unwrap();
    symlink(&f.source, blob(&f)).unwrap();
    assert!(preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id).is_err());
}
#[test]
fn invalid_unknown_and_wrong_format_ids_never_produce_docx_text() {
    let f = fixture("history.txt", &document());
    let (_, result) = preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id).unwrap();
    assert_eq!(result.summary.status, TextStatus::Unsupported);
    assert!(result.text.is_empty());
    for (id, code) in [
        ("../../source", ErrorCode::InvalidInput),
        (
            "00000000-0000-4000-8000-000000000001",
            ErrorCode::UnknownSnapshot,
        ),
    ] {
        let err = preview_docx_snapshot(&f.service, id).unwrap_err();
        assert_eq!(err.stage, ErrorStage::Memory);
        assert_eq!(err.code, code);
    }
}
#[test]
fn verified_snapshot_does_not_mean_supported_or_undamaged_docx() {
    for (bytes, status) in [
        (
            package("<x:fldChar xmlns:x=\"urn:foreign\"/>", &[]),
            TextStatus::Unsupported,
        ),
        (b"not a DOCX package".to_vec(), TextStatus::Failed),
        (
            package(
                r#"<w:p><w:r><w:t>prefix</w:t><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
                &[],
            ),
            TextStatus::Failed,
        ),
    ] {
        let f = fixture("history.docx", &bytes);
        let (_, result) = preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id).unwrap();
        assert_eq!(result.summary.status, status);
        assert!(result.text.is_empty());
        assert_eq!(result.summary.chars, 0);
        assert!(result.summary.covered.is_empty() && result.summary.left_out.is_empty());
    }
}

#[test]
fn extensionless_docx_cannot_preview_and_hidden_docx_cannot_be_captured() {
    let f = fixture("docx", &document());
    let result = preview_docx_snapshot(&f.service, &f.snapshot.snapshot_id)
        .unwrap()
        .1;
    assert_eq!(result.summary.status, TextStatus::Unsupported);
    assert!(result.text.is_empty());
    let hidden = f.source.with_file_name(".docx");
    fs::write(&hidden, document()).unwrap();
    let err = f
        .service
        .register_file(&f.snapshot.root_id, &hidden)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedFile);
    assert_eq!(err.stage, ErrorStage::Observation);
}
