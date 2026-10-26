use crate::docx::{self, read_docx};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
use traeva_core::{CheckOutcome, ErrorCode, ErrorStage, OrganizeService, TextStatus};
use zip::{ZipWriter, write::SimpleFileOptions};

fn synthetic(text: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let parts=[("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>".into()),
        ("_rels/.rels","<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"main\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".into()),
        ("word/document.xml",format!("<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"))];
    for (name, data) in parts {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn capture(service: &OrganizeService, root: &Path, bytes: &[u8]) -> (String, String) {
    fs::write(root.join("synthetic.docx"), bytes).unwrap();
    let folder = service.authorize_root(root).unwrap();
    let file = service
        .register_file(&folder.root_id, &root.join("synthetic.docx"))
        .unwrap()
        .file;
    service.start_memory(&file.file_id).unwrap();
    let versions = service.list_snapshots(&file.file_id).unwrap();
    assert_eq!(versions.len(), 1);
    (file.file_id.clone(), versions[0].snapshot_id.clone())
}
#[test]
fn docx_from_real_snapshot_survives_source_edit_removal_and_service_restart() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    let data = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    let first = synthetic("before 中国");
    let second = synthetic("after 中国");
    let service = OrganizeService::start(data.clone());
    let (file, id) = capture(&service, &root, &first);
    let (ref_before, bytes) = service.read_snapshot(&id).unwrap();
    assert_eq!(bytes, first);
    assert_eq!(read_docx(&bytes).text, "before 中国");
    // Test author writes only its synthetic source; the service has no writes in this root.
    fs::write(root.join("synthetic.docx"), &second).unwrap();
    let checks = service.check_memory().unwrap();
    assert!(
        checks
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::Saved { .. }))
    );
    let versions = service.list_snapshots(&file).unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(service.read_snapshot(&id).unwrap().1, first);
    assert_eq!(
        read_docx(&service.read_snapshot(&versions[1].snapshot_id).unwrap().1).text,
        "after 中国"
    );
    service.stop_memory(&file).unwrap();
    drop(service);
    fs::remove_file(root.join("synthetic.docx")).unwrap();
    let reopened = OrganizeService::start(data);
    let (saved, bytes) = reopened.read_snapshot(&id).unwrap();
    assert_eq!(saved, ref_before);
    assert_eq!(bytes, first);
    assert_eq!(read_docx(&bytes).summary.status, TextStatus::Complete);
    // Shared dispatch belongs to A; this test deliberately makes its current limit visible.
    let dispatched = reopened.snapshot_text(&id).unwrap().1;
    if dispatched.summary.reader == docx::DOCX_READER {
        assert_eq!(dispatched, read_docx(&bytes));
    } else {
        assert_eq!(dispatched.summary.reader, traeva_core::NO_READER);
        assert_eq!(dispatched.summary.status, TextStatus::Unsupported);
        assert!(dispatched.text.is_empty());
    }
}
#[test]
fn corrupt_and_missing_snapshots_never_fall_back_to_source_docx() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    let data = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    let service = OrganizeService::start(data.clone());
    let (_, id) = capture(&service, &root, &synthetic("valid original"));
    let snapshot = service.read_snapshot(&id).unwrap().0;
    let object = data
        .join("snapshots")
        .join(&snapshot.sha256[..2])
        .join(&snapshot.sha256);
    fs::write(&object, b"corrupt synthetic object").unwrap();
    let error = service.read_snapshot(&id).unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotCorrupt);
    assert_eq!(error.stage, ErrorStage::Memory);
    assert_eq!(fs::read(&object).unwrap(), b"corrupt synthetic object");
    fs::remove_file(&object).unwrap();
    assert_eq!(
        service.read_snapshot(&id).unwrap_err().code,
        ErrorCode::SnapshotUnavailable
    );
    assert!(!object.exists());
    assert!(root.join("synthetic.docx").exists());
}
#[test]
fn unchanged_docx_capture_is_deduplicated_without_source_writes() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let service = OrganizeService::start(temp.path().join("data"));
    let bytes = synthetic("same");
    let (file, _) = capture(&service, &root, &bytes);
    let before = fs::metadata(root.join("synthetic.docx")).unwrap();
    service.check_memory().unwrap();
    let after = fs::metadata(root.join("synthetic.docx")).unwrap();
    assert_eq!(fs::read(root.join("synthetic.docx")).unwrap(), bytes);
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(service.list_snapshots(&file).unwrap().len(), 1);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(docx::read_docx(&bytes).text, "same");
}

#[test]
fn atomic_replacement_is_not_implicitly_adopted_and_old_snapshot_survives() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let data = temp.path().join("data");
    let service = OrganizeService::start(data.clone());
    let first = include_bytes!("../fixtures/docx/word-layout-v1.docx");
    let second = include_bytes!("../fixtures/docx/word-layout-v2.docx");
    let (file, id) = capture(&service, &root, first);
    fs::write(root.join("replacement.docx"), second).unwrap();
    fs::rename(root.join("replacement.docx"), root.join("synthetic.docx")).unwrap();
    let checks = service.check_memory().unwrap();
    assert!(checks.iter().any(|c| matches!(&c.outcome, CheckOutcome::Problem {error} if error.code == ErrorCode::FileChanged)));
    assert_eq!(service.list_snapshots(&file).unwrap().len(), 1);
    assert_eq!(service.read_snapshot(&id).unwrap().1, first);
    service.stop_memory(&file).unwrap();
    drop(service);
    fs::rename(root.join("synthetic.docx"), root.join("renamed.docx")).unwrap();
    let reopened = OrganizeService::start(data);
    assert_eq!(reopened.read_snapshot(&id).unwrap().1, first);
    assert_eq!(
        read_docx(&reopened.read_snapshot(&id).unwrap().1).text,
        include_str!("../fixtures/docx/word-basic-v1.txt")
    );
}

#[test]
fn same_bytes_rewrite_deduplicates_and_stable_incomplete_zip_is_failed() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let service = OrganizeService::start(temp.path().join("data"));
    let bytes = include_bytes!("../fixtures/docx/word-layout-v1.docx");
    let (file, id) = capture(&service, &root, bytes);
    fs::write(root.join("synthetic.docx"), bytes).unwrap();
    assert!(
        service
            .check_memory()
            .unwrap()
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::SameBytes { .. }))
    );
    assert_eq!(service.list_snapshots(&file).unwrap().len(), 1);
    // A writer paused on an incomplete ZIP is a stable byte state, not a complete Word save.
    let prefix = &bytes[..bytes.len() / 2];
    fs::write(root.join("synthetic.docx"), prefix).unwrap();
    assert!(
        service
            .check_memory()
            .unwrap()
            .iter()
            .any(|c| matches!(c.outcome, CheckOutcome::Saved { .. }))
    );
    let versions = service.list_snapshots(&file).unwrap();
    assert_eq!(versions.len(), 2);
    let stored = service.read_snapshot(&versions[1].snapshot_id).unwrap().1;
    assert_eq!(stored, prefix);
    let out = read_docx(&stored);
    assert_eq!(out.summary.status, TextStatus::Failed);
    assert!(out.text.is_empty());
    assert_eq!(service.read_snapshot(&id).unwrap().1, bytes);
}

#[test]
fn writes_during_capture_are_refused_without_mutating_previous_snapshot() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };
    let temp = tempfile::Builder::new()
        .prefix("traeva-docx-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let service = OrganizeService::start(temp.path().join("data"));
    let bytes = include_bytes!("../fixtures/docx/word-layout-v1.docx");
    let (file, id) = capture(&service, &root, bytes);
    let path = root.join("synthetic.docx");
    let running = Arc::new(AtomicBool::new(true));
    let writer_running = running.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let writer_count = writes.clone();
    let (ready, wait) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut first = true;
        let mut n = 100;
        while writer_running.load(Ordering::Acquire) {
            fs::write(&path, &bytes[..n]).unwrap();
            writer_count.fetch_add(1, Ordering::Release);
            if first {
                ready.send(()).unwrap();
                first = false;
            }
            n = if n == 100 { 200 } else { 100 };
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    wait.recv().unwrap();
    let writes_before = writes.load(Ordering::Acquire);
    let result = service.check_memory();
    let writes_after = writes.load(Ordering::Acquire);
    running.store(false, Ordering::Release);
    writer.join().unwrap();
    let checks = result.unwrap();
    assert!(
        writes_after > writes_before,
        "Writer did not overlap the capture attempt"
    );
    assert!(checks.iter().any(
        |c| matches!(&c.outcome, CheckOutcome::Problem {error} if error.code == ErrorCode::FileChanging)
    ));
    assert_eq!(service.list_snapshots(&file).unwrap().len(), 1);
    assert_eq!(service.read_snapshot(&id).unwrap().1, bytes);
}
