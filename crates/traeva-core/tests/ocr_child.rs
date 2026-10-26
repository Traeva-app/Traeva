//! On-device text recognition for scanned PDFs (#186, organize-spec.md
//! §14.8), with the real PDF child and reader on G4's synthetic samples
//! (tests/fixtures/pdf): the reader's own note for a scan starts a
//! recognition (a small shell program stands in for the recognizer there);
//! G-OCR's real child refuses wrong arguments and a PDF with a text layer
//! before any Vision work; Find never recognizes. Recognition itself with
//! Vision runs in the opt-in tests/ocr_app.rs. Synthetic files and
//! temporary folders only.

mod common;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use traeva_core::{
    ExtractionStatus, OcrHelper, OcrState, OperationStage, OrganizeService, PdfHelper,
    ProcessMemory,
};

const PDF_CHILD: &str = env!("CARGO_BIN_EXE_traeva-pdf-text");
const OCR_CHILD: &str = env!("CARGO_BIN_EXE_traeva-ocr");

struct Setup {
    _app: tempfile::TempDir,
    root: tempfile::TempDir,
    runs: tempfile::TempDir,
    service: OrganizeService,
    root_id: String,
}

/// A root holding the sample `name` as doc.pdf, read by the real PDF child.
fn setup(name: &str) -> Setup {
    let app = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let root = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let runs = tempfile::Builder::new()
        .prefix("traeva-ocr-runs-")
        .tempdir()
        .unwrap();
    let sample = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name),
    )
    .unwrap();
    fs::write(root.path().join("doc.pdf"), sample).unwrap();
    let service = OrganizeService::start(common::app_data_dir(&app));
    let root_id = service.authorize_root(root.path()).unwrap().root_id;
    service.set_pdf_helper(Some(PdfHelper::traeva_with(PDF_CHILD)));
    Setup {
        _app: app,
        root,
        runs,
        service,
        root_id,
    }
}

fn small(_pid: u32) -> Option<ProcessMemory> {
    Some(ProcessMemory {
        footprint_bytes: 1 << 20,
        lifetime_peak_bytes: 1 << 20,
        kernel_lifetime_peak_bytes: 1 << 20,
    })
}

fn sh(script: &str, args: Vec<OsString>) -> (PathBuf, Vec<OsString>) {
    let mut all: Vec<OsString> = vec!["-c".into(), script.into()];
    all.extend(args);
    (PathBuf::from("/bin/sh"), all)
}

impl Setup {
    /// A stand-in recognizer that notes each run and recognizes one page.
    fn recognizer_ready(&self) {
        let answer = "{\"status\":\"partial\",\"text\":\"[Page 1]\\nInvoice 2026-10 for the \
                      studio rent.\",\"note\":null}";
        let recognize = sh(
            "echo run >> \"$0\"; /bin/cat > /dev/null; printf '%s' \"$1\"",
            vec![
                self.runs.path().join("runs").into_os_string(),
                answer.into(),
            ],
        );
        let prepare = sh("printf '%s' '{\"status\":\"ready\"}'", Vec::new());
        self.service
            .set_ocr_helper(Some(OcrHelper::commands(recognize, prepare, small)));
        assert_eq!(self.service.start_ocr_preparation(), OcrState::Ready);
    }

    fn runs(&self) -> usize {
        fs::read_to_string(self.runs.path().join("runs"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

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

#[test]
fn the_reader_s_note_for_a_real_scan_starts_a_recognition() {
    let s = setup("p08-scanned-images.pdf");
    s.recognizer_ready();
    let before = common::snapshot::snapshot(s.root.path());
    let view = s.choose();
    assert_eq!(view.stage, OperationStage::ReadyForAnalysis, "{view:?}");
    let extraction = view.file.unwrap().extraction;
    assert!(extraction.recognized);
    assert_eq!(extraction.status, ExtractionStatus::Partial);
    assert!(
        extraction
            .note
            .unwrap()
            .starts_with("Read by on-device text recognition from 1 scanned page;")
    );
    assert_eq!(s.runs(), 1);
    // Nothing was written in the folder.
    assert_eq!(common::snapshot::snapshot(s.root.path()), before);
}

#[test]
fn a_pdf_with_a_text_layer_or_an_encrypted_one_is_never_recognized() {
    for name in ["p02-multi-en.pdf", "e03a-libreoffice-open-password.pdf"] {
        let s = setup(name);
        s.recognizer_ready();
        let view = s.choose();
        assert!(!view.file.unwrap().extraction.recognized, "{name}");
        assert_eq!(s.runs(), 0, "{name}");
    }
}

/// G-OCR's child (what Traeva's executable does with the OCR flags)
/// refuses wrong arguments before any work: no parent pid, a pid that is
/// not its parent, a budget other than the approved one.
#[test]
fn the_real_children_refuse_wrong_arguments_before_any_work() {
    let me = std::process::id().to_string();
    for args in [
        vec![traeva_core::OCR_FLAG],
        vec![traeva_core::OCR_FLAG, "--parent-pid", "1"],
        vec![traeva_core::OCR_FLAG, "--parent-pid", &me, "--dpi", "300"],
        vec![
            traeva_core::OCR_FLAG,
            "--parent-pid",
            &me,
            "--max-pages",
            "4",
        ],
        // Pictures for the model: only `--output jpeg`, once (tier 2).
        vec![
            traeva_core::OCR_FLAG,
            "--parent-pid",
            &me,
            "--output",
            "png",
        ],
        vec![
            traeva_core::OCR_FLAG,
            "--parent-pid",
            &me,
            "--output",
            "jpeg",
            "--output",
            "jpeg",
        ],
        vec![traeva_core::OCR_PREPARE_FLAG],
        vec![traeva_core::OCR_PREPARE_FLAG, "--parent-pid", "1"],
        vec![
            traeva_core::OCR_PREPARE_FLAG,
            "--parent-pid",
            &me,
            "--dpi",
            "200",
        ],
        vec![
            traeva_core::OCR_PREPARE_FLAG,
            "--parent-pid",
            &me,
            "--output",
            "jpeg",
        ],
    ] {
        let out = std::process::Command::new(OCR_CHILD)
            .args(&args)
            .env_clear()
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

/// The real child refuses a PDF with a text layer (`unsupported`, no text)
/// before drawing any page or calling Vision; the parent never asks it for
/// one anyway (above).
#[test]
fn the_real_child_refuses_a_pdf_with_a_text_layer() {
    use std::io::Write;
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/p02-multi-en.pdf"),
    )
    .unwrap();
    let me = std::process::id().to_string();
    let mut child = std::process::Command::new(OCR_CHILD)
        .args([traeva_core::OCR_FLAG, "--parent-pid", &me])
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let answer: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(answer["status"], "unsupported", "{answer}");
    assert_eq!(answer["text"], "");
}

#[test]
fn find_never_recognizes() {
    let s = setup("p08-scanned-images.pdf");
    s.recognizer_ready();
    let started = s.service.start_find("doc").unwrap();
    let view = s.service.run_find(&started.find_id).unwrap();
    assert_eq!(view.coverage.unwrap().texts_unreadable, 1, "no text layer");
    assert_eq!(s.runs(), 0);
}
