use crate::pptx::{
    PPTX_MEMORY_READER, PPTX_PREVIEW_READER, read_pptx, read_pptx_memory, read_pptx_preview,
};
use std::fs;
use traeva_core::{CheckOutcome, ErrorCode, ErrorStage, OrganizeService};

const V1: &[u8] = include_bytes!("../fixtures/pptx/powerpoint-basic-v1.pptx");
const V2: &[u8] = include_bytes!("../fixtures/pptx/powerpoint-basic-v2.pptx");

#[test]
fn both_projections_bind_to_one_verified_snapshot_not_changed_source() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-e-projections-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let source = root.join("synthetic.pptx");
    fs::write(&source, V1).unwrap();
    let service = OrganizeService::start(temp.path().join("data"));
    let authorized = service.authorize_root(&root).unwrap();
    let file = service
        .register_file(&authorized.root_id, &source)
        .unwrap()
        .file;
    service.start_memory(&file.file_id).unwrap();
    let snapshot = service.list_snapshots(&file.file_id).unwrap().remove(0);
    let (bound, bytes) = service.read_snapshot(&snapshot.snapshot_id).unwrap();
    assert_eq!(bound, snapshot);
    assert_eq!(bytes, V1);
    let memory = read_pptx_memory(&bytes);
    let preview = read_pptx_preview(&bytes);
    assert_eq!(memory.summary.reader, PPTX_MEMORY_READER);
    assert_eq!(preview.summary.reader, PPTX_PREVIEW_READER);
    assert!(!memory.text.contains("Second title"));
    assert!(!memory.text.contains("Second speaker note"));
    assert!(memory.text.contains("First speaker note"));
    assert!(preview.text.contains(" · hidden⟧"));
    assert!(preview.text.contains("Second speaker note"));
    fs::write(&source, b"changed synthetic source, not the snapshot").unwrap();
    let (same, checked_bytes) = service.read_snapshot(&snapshot.snapshot_id).unwrap();
    assert_eq!(same, bound);
    assert_eq!(read_pptx_memory(&checked_bytes), memory);
    assert_eq!(read_pptx_preview(&checked_bytes), preview);
    // Since the #28 wiring, core's own reading of the snapshot is the Memory
    // projection of the same checked bytes.
    assert_eq!(
        service.snapshot_text(&snapshot.snapshot_id).unwrap().1,
        memory
    );
}

#[test]
fn verified_snapshot_bytes_survive_synthetic_replacement_rename_and_restart() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-e-pptx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    let data = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    let source = root.join("synthetic.pptx");
    fs::write(&source, V1).unwrap();
    let before = fs::metadata(&source).unwrap().modified().unwrap();
    let service = OrganizeService::start(data.clone());
    let authorized = service.authorize_root(&root).unwrap();
    let file = service
        .register_file(&authorized.root_id, &source)
        .unwrap()
        .file;
    service.start_memory(&file.file_id).unwrap();
    let snapshot = service.list_snapshots(&file.file_id).unwrap().remove(0);
    let (saved, bytes) = service.read_snapshot(&snapshot.snapshot_id).unwrap();
    assert_eq!(bytes, V1);
    assert_eq!(
        read_pptx(&bytes).text,
        include_str!("../fixtures/pptx/expected.txt")
    );
    assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    // Only this test writes its synthetic root; the Reader remains bytes-only.
    fs::write(root.join("replacement.pptx"), V2).unwrap();
    fs::rename(root.join("replacement.pptx"), &source).unwrap();
    assert!(service.check_memory().unwrap().iter().any(|c| matches!(&c.outcome, CheckOutcome::Problem {error} if error.code == ErrorCode::FileChanged)));
    assert_eq!(service.list_snapshots(&file.file_id).unwrap().len(), 1);
    fs::rename(&source, root.join("renamed.pptx")).unwrap();
    service.stop_memory(&file.file_id).unwrap();
    drop(service);
    let reopened = OrganizeService::start(data);
    let (old, bytes) = reopened.read_snapshot(&saved.snapshot_id).unwrap();
    assert_eq!(old, saved);
    assert_eq!(bytes, V1);
    assert_eq!(
        read_pptx(&bytes).text,
        include_str!("../fixtures/pptx/expected.txt")
    );
    // Since the #28 wiring, core reads a saved .pptx with its Memory projection.
    let dispatched = reopened.snapshot_text(&saved.snapshot_id).unwrap().1;
    assert_eq!(dispatched, read_pptx_memory(&bytes));
    assert_eq!(dispatched.summary.reader, PPTX_MEMORY_READER);
}

#[test]
fn damaged_missing_unknown_snapshot_is_a_memory_error_with_no_fallback() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-e-pptx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    let data = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    let source = root.join("synthetic.pptx");
    fs::write(&source, V1).unwrap();
    let service = OrganizeService::start(data.clone());
    let authorized = service.authorize_root(&root).unwrap();
    let file = service
        .register_file(&authorized.root_id, &source)
        .unwrap()
        .file;
    service.start_memory(&file.file_id).unwrap();
    let snapshot = service.list_snapshots(&file.file_id).unwrap().remove(0);
    // Fault injection into this test's own app-data; never a Reader path input.
    let blob = data
        .join("snapshots")
        .join(&snapshot.sha256[..2])
        .join(&snapshot.sha256);
    fs::write(&blob, b"corrupt test bytes").unwrap();
    let error = service.read_snapshot(&snapshot.snapshot_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotCorrupt);
    assert_eq!(error.stage, ErrorStage::Memory);
    assert_eq!(fs::read(&blob).unwrap(), b"corrupt test bytes");
    fs::remove_file(&blob).unwrap();
    assert_eq!(
        service
            .read_snapshot(&snapshot.snapshot_id)
            .unwrap_err()
            .code,
        ErrorCode::SnapshotUnavailable
    );
    assert_eq!(
        service
            .read_snapshot("00000000-0000-4000-8000-000000000001")
            .unwrap_err()
            .code,
        ErrorCode::UnknownSnapshot
    );
    assert!(!blob.exists());
    assert_eq!(fs::read(source).unwrap(), V1);
}
