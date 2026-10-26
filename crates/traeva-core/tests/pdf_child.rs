//! Organize reads a PDF's text layer in a separate process (#186,
//! organize-spec.md §14.3). These tests run the real child program
//! (`traeva-pdf-text`, what Traeva's executable does with the PDF flag) and
//! system tools that misbehave on purpose: too slow, crashing, a wrong
//! answer, too much memory. Each time the file goes to a manual pick, the
//! service keeps working and the folder is not touched. Synthetic files
//! and temporary folders only.

mod common;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use traeva_core::{ExtractionStatus, MatchPlace, OperationStage, OrganizeService, PdfHelper};

const CHILD: &str = env!("CARGO_BIN_EXE_traeva-pdf-text");

struct Setup {
    _app: tempfile::TempDir,
    root: tempfile::TempDir,
    service: OrganizeService,
    root_id: String,
}

/// A root holding `doc.pdf` with these bytes (not a real PDF: the tests
/// here are about the process, not the reader).
fn setup(bytes: &[u8]) -> Setup {
    let app = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let root = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    fs::write(root.path().join("doc.pdf"), bytes).unwrap();
    let service = OrganizeService::start(common::app_data_dir(&app));
    let root_id = service.authorize_root(root.path()).unwrap().root_id;
    Setup {
        _app: app,
        root,
        service,
        root_id,
    }
}

impl Setup {
    /// Chooses doc.pdf in a new operation and reads it; the operation after.
    fn choose(&self) -> traeva_core::OperationView {
        let started = self
            .service
            .start_operation(&self.root_id, &self.root_id)
            .unwrap();
        self.service
            .run_operation(&started.operation_id, &self.root.path().join("doc.pdf"));
        self.service.get_operation(&started.operation_id).unwrap()
    }
}

fn manual_pick_because(view: &traeva_core::OperationView, words: &str) {
    assert_eq!(view.stage, OperationStage::NeedsManualLocation, "{view:?}");
    let extraction = &view.file.as_ref().expect("the file was read").extraction;
    assert!(!extraction.usable_text);
    let note = extraction.note.as_deref().unwrap_or_default();
    assert!(note.contains(words), "note {note:?} should say {words:?}");
}

fn helper(program: &str, args: &[&str]) -> PdfHelper {
    PdfHelper::command(
        PathBuf::from(program),
        args.iter().map(OsString::from).collect(),
    )
}

#[test]
fn without_a_helper_a_pdf_goes_to_a_manual_pick() {
    let s = setup(b"%PDF-1.7\n");
    manual_pick_because(&s.choose(), "cannot read PDF files here");
}

#[test]
fn the_child_reads_the_bytes_from_the_pipe_and_answers() {
    let s = setup(b"%PDF-1.7\n");
    s.service
        .set_pdf_helper(Some(helper(CHILD, &["536870912"])));
    let view = s.choose();
    // The real reader treats bytes that are only a header as damaged; what
    // matters here is that the answer came back through the child and was
    // used.
    assert_eq!(view.stage, OperationStage::NeedsManualLocation, "{view:?}");
    let extraction = &view.file.unwrap().extraction;
    assert_eq!(extraction.status, ExtractionStatus::Failed);
}

#[test]
fn a_child_that_takes_too_long_is_stopped() {
    let s = setup(b"%PDF-1.7\n");
    s.service.set_pdf_helper(Some(
        helper("/bin/sleep", &["30"]).with_time_limit(Duration::from_secs(1)),
    ));
    let started = Instant::now();
    manual_pick_because(&s.choose(), "took too long");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "waited too long"
    );
}

/// R4 on the OCR experiment (#211): a paused child is not an exited one.
/// The parent counts only a child that ended (exited, killed or dumped;
/// never CLD_STOPPED), so a child stopped with SIGSTOP still meets the
/// time limit: it is killed and reaped, and the next file is read as usual.
#[test]
fn a_paused_child_is_killed_at_the_time_limit_and_reaped() {
    let s = setup(b"%PDF-1.7\n");
    // The child notes its pid outside the root.
    let notes = tempfile::Builder::new()
        .prefix("traeva-paused-")
        .tempdir()
        .unwrap();
    let pid_file = notes.path().join("pid");
    let script = "echo $$ > \"$0\"; kill -STOP $$; exec /bin/sleep 30";
    s.service.set_pdf_helper(Some(
        PdfHelper::command(
            PathBuf::from("/bin/sh"),
            vec!["-c".into(), script.into(), pid_file.clone().into()],
        )
        .with_time_limit(Duration::from_secs(1)),
    ));
    let started = Instant::now();
    manual_pick_because(&s.choose(), "took too long");
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid: libc::pid_t = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 sends nothing; it only checks that the process
    // exists (a zombie that was not reaped would still be found).
    let found = unsafe { libc::kill(pid, 0) };
    let errno = std::io::Error::last_os_error().raw_os_error();
    assert_eq!(
        (found, errno),
        (-1, Some(libc::ESRCH)),
        "the paused child {pid} is left over"
    );
    // The next read goes through the child as usual.
    s.service
        .set_pdf_helper(Some(helper(CHILD, &["536870912"])));
    let view = s.choose();
    assert_eq!(
        view.file.unwrap().extraction.status,
        ExtractionStatus::Failed,
        "the real reader's answer for bytes that are only a header"
    );
}

#[test]
fn a_child_that_crashes_leaves_the_app_working() {
    let s = setup(b"%PDF-1.7\n");
    s.service
        .set_pdf_helper(Some(helper("/bin/sh", &["-c", "kill -ABRT $$"])));
    manual_pick_because(&s.choose(), "within its limits");
    // The service still works: the same file again, the same answer.
    manual_pick_because(&s.choose(), "within its limits");
}

#[test]
fn an_answer_that_does_not_parse_is_not_used() {
    let s = setup(b"%PDF-1.7\n");
    s.service
        .set_pdf_helper(Some(helper("/bin/echo", &["{\"status\":\"complete\"}"])));
    manual_pick_because(&s.choose(), "cannot use");
}

#[test]
fn a_child_over_its_heap_cap_stops_and_nothing_changes() {
    // Reading 2 MiB from the pipe passes a 64 KiB cap: the allocator
    // refuses, the child aborts.
    let s = setup(&vec![b'%'; 2 << 20]);
    let before = common::snapshot::snapshot(s.root.path());
    s.service.set_pdf_helper(Some(helper(CHILD, &["65536"])));
    manual_pick_because(&s.choose(), "within its limits");
    assert_eq!(common::snapshot::snapshot(s.root.path()), before);
}

// The real reader in the real child, on G4's synthetic samples
// (tests/fixtures/pdf, expected.json): §14 P01 and P02.

fn sample(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name),
    )
    .unwrap()
}

fn with_child(name: &str) -> Setup {
    let s = setup(&sample(name));
    s.service
        .set_pdf_helper(Some(PdfHelper::traeva_with(CHILD)));
    s
}

#[test]
fn a_pdf_with_a_text_layer_is_read_in_the_child_and_ready_for_analysis() {
    for (name, chars) in [("p02-multi-en.pdf", 150), ("p03-cjk-reportlab.pdf", 72)] {
        let s = with_child(name);
        let before = common::snapshot::snapshot(s.root.path());
        let view = s.choose();
        assert_eq!(
            view.stage,
            OperationStage::ReadyForAnalysis,
            "{name}: {view:?}"
        );
        let extraction = &view.file.as_ref().unwrap().extraction;
        assert_eq!(extraction.status, ExtractionStatus::Ok, "{name}");
        assert!(extraction.usable_text, "{name}");
        assert_eq!(extraction.total_chars, chars, "{name}");
        // Reading wrote nothing in the folder (§14.3, P06).
        assert_eq!(common::snapshot::snapshot(s.root.path()), before, "{name}");
    }
}

#[test]
fn a_scanned_or_encrypted_pdf_goes_to_a_manual_pick() {
    for (name, words) in [
        ("p08-scanned-images.pdf", "OCR"),
        ("e03a-libreoffice-open-password.pdf", "encrypted"),
    ] {
        let s = with_child(name);
        let view = s.choose();
        manual_pick_because(&view, words);
        assert_eq!(
            view.file.unwrap().extraction.status,
            ExtractionStatus::Unsupported,
            "{name}"
        );
    }
}

// Find (#198) reads a PDF's text layer in the same child, with its own
// shorter time limit, and writes nothing either.

#[test]
fn find_searches_a_pdf_s_text_layer_in_the_child() {
    let s = with_child("p03-cjk-reportlab.pdf");
    let before = common::snapshot::snapshot(s.root.path());
    let started = s.service.start_find("嵌入字体").unwrap();
    let view = s.service.run_find(&started.find_id).unwrap();
    assert_eq!(view.results.len(), 1, "{view:?}");
    assert_eq!(view.results[0].name, "doc.pdf");
    assert_eq!(view.results[0].matches[0].place, MatchPlace::Text);
    assert_eq!(view.coverage.unwrap().texts_searched, 1);
    assert_eq!(common::snapshot::snapshot(s.root.path()), before);

    let scanned = with_child("p08-scanned-images.pdf");
    let started = scanned.service.start_find("doc").unwrap();
    let view = scanned.service.run_find(&started.find_id).unwrap();
    assert_eq!(view.results[0].matches[0].place, MatchPlace::Name);
    assert_eq!(view.coverage.unwrap().texts_unreadable, 1, "no text layer");
}
