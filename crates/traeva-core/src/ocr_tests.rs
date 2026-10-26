//! Tests for ocr.rs (#186, organize-spec.md §14.8): the parent's side of
//! on-device text recognition. The PDF reader is a stand-in that reports
//! the real reader's "no text layer" note; the recognition and preparation
//! children are small shell programs that answer, sleep, crash or print
//! nonsense on purpose, and the memory probe is a stand-in. Offline: the
//! local test server plays OpenAI. Synthetic files and temporary folders
//! only.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::contracts::OperationStage;
use crate::memory_contracts::{TextExtraction, TextStatus, TextSummary};
use crate::model::tests::{Planned, TestServer, completed};
use crate::pdf_child::PdfHelper;
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::test_fixture::{Fixture, quick_read};

/// What the real reader says about a scanned PDF (tests/fixtures/pdf,
/// expected.json, p08).
const SCANNED_NOTE: &str = "No page has a text layer that this reader can use. 2 of 2 pages \
                            are mostly images (likely scans); reading them needs OCR, which \
                            this reader does not do.";

/// The recognition child's answer for two scanned pages.
const RECOGNIZED: &str = "{\"status\":\"partial\",\"text\":\"[Page 1]\\nTrip to Kyoto in May: \
                          train tickets.\\n[Page 2]\\nBudget about 400 USD for the week.\",\
                          \"note\":null}";

fn scanned(_bytes: &[u8]) -> TextExtraction {
    TextExtraction {
        summary: TextSummary {
            reader: "pdf-1".to_string(),
            status: TextStatus::Unsupported,
            chars: 0,
            covered: Vec::new(),
            left_out: Vec::new(),
            note: Some(SCANNED_NOTE.to_string()),
        },
        text: String::new(),
    }
}

fn memory(footprint: u64, lifetime_peak: u64) -> Option<ProcessMemory> {
    Some(ProcessMemory {
        footprint_bytes: footprint,
        lifetime_peak_bytes: lifetime_peak,
        kernel_lifetime_peak_bytes: lifetime_peak,
    })
}

fn small(_pid: u32) -> Option<ProcessMemory> {
    memory(1 << 20, 2 << 20)
}

/// Its footprint is small; its lifetime peak passed the budget.
fn huge(_pid: u32) -> Option<ProcessMemory> {
    memory(1 << 20, OCR_MEMORY_LIMIT + 1)
}

fn unreadable(_pid: u32) -> Option<ProcessMemory> {
    None
}

/// Set once the preparation is over, by the one test that uses the probes
/// below: they read a small footprint until then, and afterwards count
/// their looks at a recognition child.
static RECOGNIZING: AtomicBool = AtomicBool::new(false);
static LOOKS: AtomicUsize = AtomicUsize::new(0);

fn while_recognizing(pid: u32, probe: MemoryProbe) -> Option<ProcessMemory> {
    if RECOGNIZING.load(Ordering::SeqCst) {
        LOOKS.fetch_add(1, Ordering::SeqCst);
        probe(pid)
    } else {
        small(pid)
    }
}

fn huge_when_recognizing(pid: u32) -> Option<ProcessMemory> {
    while_recognizing(pid, huge)
}

fn unreadable_when_recognizing(pid: u32) -> Option<ProcessMemory> {
    while_recognizing(pid, unreadable)
}

/// Small while the child runs; over the budget at the last read, after it
/// ended and before it is reaped.
fn huge_at_the_end(pid: u32) -> Option<ProcessMemory> {
    let ended = crate::pdf_child::ended(pid) == Some(true);
    while_recognizing(pid, if ended { huge } else { small })
}

/// Small while the child runs; unreadable at the last read.
fn unreadable_at_the_end(pid: u32) -> Option<ProcessMemory> {
    let ended = crate::pdf_child::ended(pid) == Some(true);
    while_recognizing(pid, if ended { unreadable } else { small })
}

fn sh(script: &str, args: &[&OsString]) -> (PathBuf, Vec<OsString>) {
    let mut all: Vec<OsString> = vec!["-c".into(), script.into()];
    all.extend(args.iter().map(|arg| (*arg).clone()));
    (PathBuf::from("/bin/sh"), all)
}

fn ready() -> (PathBuf, Vec<OsString>) {
    sh("printf '%s' '{\"status\":\"ready\"}'", &[])
}

struct Setup {
    f: Fixture,
    server: TestServer,
    /// Each recognition child adds a line here (outside the root).
    runs: tempfile::TempDir,
}

/// A root with Travel/ and scan.pdf, model settings and Organize's sending
/// agreement, the stand-in reader, and a server recommending Travel.
fn setup() -> Setup {
    let mut f = Fixture::new();
    fs::create_dir(f.path("Travel")).unwrap();
    fs::write(f.path("scan.pdf"), b"%PDF-1.7 scanned stand-in\n").unwrap();
    let ids: HashMap<String, String> = f
        .service()
        .list_folders(&f.root_id)
        .unwrap()
        .folders
        .into_iter()
        .map(|folder| (folder.relative_path, folder.folder_id))
        .collect();
    let answer = json!({
        "disposition": "recommend",
        "summary": "Notes for a trip.",
        "candidates": [{ "id": "C1", "folder_id": ids["Travel"], "new_segments": [],
                         "reason": "Fits the trip.", "evidence_ids": ["E1"] }],
        "preferred_id": "C1",
        "confidence": 0.9,
        "uncertain_field": null,
        "clarification": null
    });
    let server = TestServer::start(Planned::answer(200, completed(&answer)));
    f.service_mut().model.endpoint = server.url.clone();
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    f.service().set_analysis_consent(true).unwrap();
    f.service()
        .set_pdf_helper(Some(PdfHelper::in_process(scanned)));
    let runs = tempfile::Builder::new()
        .prefix("traeva-ocr-runs-")
        .tempdir()
        .unwrap();
    Setup { f, server, runs }
}

impl Setup {
    fn service(&self) -> &OrganizeService {
        self.f.service()
    }

    fn count_file(&self) -> OsString {
        self.runs.path().join("runs").into_os_string()
    }

    /// A recognition child that notes its run, then runs `script` (the
    /// answer is `$1`).
    fn recognizer(&self, script: &str, answer: &str) -> (PathBuf, Vec<OsString>) {
        let counted = format!("echo run >> \"$0\"; {script}");
        sh(&counted, &[&self.count_file(), &OsString::from(answer)])
    }

    /// Sets the recognizer and prepares it (a stand-in that is ready at once).
    /// The time limit is generous so that a busy machine never turns a
    /// crash or an answer into a timeout (a crashing child can be held
    /// while macOS writes its crash report).
    fn ready_with(&self, recognize: (PathBuf, Vec<OsString>), probe: MemoryProbe) {
        self.ready_within(recognize, probe, Duration::from_secs(30));
    }

    fn ready_within(
        &self,
        recognize: (PathBuf, Vec<OsString>),
        probe: MemoryProbe,
        time_limit: Duration,
    ) {
        self.service().set_ocr_helper(Some(
            OcrHelper::commands(recognize, ready(), probe)
                .with_time_limits(time_limit, Duration::from_secs(10)),
        ));
        assert_eq!(self.service().start_ocr_preparation(), OcrState::Ready);
    }

    /// A recognizer that answers `answer` after reading the bytes.
    fn answering(&self, answer: &str) {
        let recognize = self.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", answer);
        self.ready_with(recognize, small);
    }

    fn runs(&self) -> usize {
        fs::read_to_string(self.runs.path().join("runs"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    /// Chooses `name` in a new operation and reads it.
    fn choose(&self, name: &str) -> String {
        let id = self
            .service()
            .start_operation(&self.f.root_id, &self.f.root_id)
            .unwrap()
            .operation_id;
        self.service()
            .run_operation_with(&id, &self.f.path(name), &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            });
        id
    }

    fn view(&self, id: &str) -> crate::contracts::OperationView {
        self.service().get_operation(id).unwrap()
    }

    fn analyze(&self, id: &str) {
        self.service().start_analysis(id).unwrap();
        self.service()
            .run_analysis_with(id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            });
    }

    /// The data part of the last request, parsed.
    fn sent_data(&self) -> Value {
        let input = self
            .server
            .last(|r| r.json()["input"].as_str().unwrap().to_string());
        let (_, data) = input.split_once('\n').unwrap();
        serde_json::from_str(data).unwrap()
    }

    fn request_meta(&self) -> Value {
        let meta: String = self
            .f
            .db()
            .query_row("SELECT request_meta_json FROM analyses", [], |r| r.get(0))
            .unwrap();
        serde_json::from_str(&meta).unwrap()
    }
}

/// The file went to a manual pick for the reason in `words`, and nothing
/// was sent.
fn manual_pick(s: &Setup, id: &str, words: &str) {
    let view = s.view(id);
    assert_eq!(view.stage, OperationStage::NeedsManualLocation, "{view:?}");
    let extraction = view.file.unwrap().extraction;
    assert!(!extraction.usable_text);
    assert!(!extraction.recognized);
    let note = extraction.note.unwrap_or_default();
    assert!(note.contains(words), "note {note:?} should say {words:?}");
    assert!(s.service().start_analysis(id).is_err());
    assert_eq!(s.server.count(), 0);
}

// ---------------------------------------------------------------------------
// Which PDFs are recognized
// ---------------------------------------------------------------------------

fn fixture_pdf(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name),
    )
    .unwrap()
}

#[test]
fn only_the_reader_s_notes_for_image_pages_start_a_recognition() {
    // The real reader on a real scan, and on an encrypted file.
    let scan = crate::pdf::read_pdf(&fixture_pdf("p08-scanned-images.pdf"));
    assert_eq!(scan.summary.status, TextStatus::Unsupported);
    assert!(has_no_text_layer(scan.summary.note.as_deref().unwrap()));
    let locked = crate::pdf::read_pdf(&fixture_pdf("e03a-libreoffice-open-password.pdf"));
    assert!(!has_no_text_layer(locked.summary.note.as_deref().unwrap()));
    // Both notes are the reader's own words (pdf.rs `no_text_note`), so a
    // change there shows here.
    let reader = include_str!("pdf.rs");
    for start in NO_TEXT_LAYER_NOTES {
        assert!(reader.contains(start), "pdf.rs no longer says {start:?}");
    }
    for other in [
        "The PDF's characters have no Unicode mapping, so it has no reliable text.",
        "The PDF's text is in fonts this reader could not decode.",
        "The PDF has no text to read: its pages draw no text.",
        "The PDF is encrypted. This reader does not open encrypted PDFs, even when no password is needed.",
    ] {
        assert!(!has_no_text_layer(other), "{other}");
    }
}

/// R4-OCR-04: only `partial` with 1–3 well-formed page sections is text;
/// everything else is a wrong answer, and a wrong answer is never used.
#[test]
fn a_recognized_answer_must_keep_its_form() {
    let answer = |json: &str| match answered(json.as_bytes()) {
        Recognized::Text { pages, .. } => Ok(pages),
        Recognized::NoText { note, .. } => Err(note),
        Recognized::ImageText { .. } => panic!("a PDF answer is never an image's text"),
    };
    let text = |text: &str| {
        let json = serde_json::json!({ "status": "partial", "text": text, "note": null });
        answer(&json.to_string())
    };
    assert_eq!(text("[Page 1]\nHello\n[Page 3]\nWorld"), Ok(2));
    assert_eq!(text("[Page 2]\n  Hello\n\n"), Ok(1));
    assert_eq!(text("[Page 1]\na\n[Page 2]\nb\n[Page 9]\nc"), Ok(3));
    for (wrong, why) in [
        ("", "no page"),
        ("Hello\n[Page 1]\nWorld", "text before the first page line"),
        ("[Page 1]", "a page line only"),
        ("[Page 1]\n   \n", "a page with only blank lines"),
        ("[Page 1]\n[Page 2]\nWorld", "the first page has no text"),
        ("[Page 1]\nHello\n[Page 2]", "the last page has no text"),
        ("[Page 0]\nHello", "page 0"),
        ("[Page 01]\nHello", "a leading zero"),
        ("[Page -1]\nHello", "a negative page"),
        ("[Page ]\nHello", "no number"),
        ("[Page 1a]\nHello", "not a number"),
        ("[Page 1] cover\nHello", "more after the page line"),
        ("[Page 99999999999]\nHello", "a number too large"),
        ("[Page 2]\na\n[Page 2]\nb", "the same page twice"),
        ("[Page 3]\na\n[Page 2]\nb", "out of order"),
        (
            "[Page 1]\na\n[Page 2]\nb\n[Page 3]\nc\n[Page 4]\nd",
            "four pages",
        ),
    ] {
        let note = text(wrong).expect_err(why);
        assert!(note.contains("cannot use"), "{why}: {note}");
    }
    // The answer itself: only `partial` is text; no other field, no other
    // type; `unsupported` and `failed` are answers without text.
    let ok_text = "[Page 1]\nHello";
    for (wrong, why) in [
        (
            serde_json::json!({ "status": "complete", "text": ok_text, "note": null }),
            "complete",
        ),
        (
            serde_json::json!({ "status": "partial", "text": ok_text, "note": null, "pages": 1 }),
            "an extra field",
        ),
        (
            serde_json::json!({ "status": "partial", "text": ["[Page 1]", "Hello"], "note": null }),
            "text is not a string",
        ),
        (
            serde_json::json!({ "status": "partial", "text": ok_text, "note": 3 }),
            "note is not a string",
        ),
        (
            serde_json::json!({ "status": "done", "text": ok_text, "note": null }),
            "an unknown status",
        ),
        (serde_json::json!({ "status": "partial" }), "no text"),
        (serde_json::json!([ok_text]), "not an object"),
    ] {
        let note = answer(&wrong.to_string()).expect_err(why);
        assert!(note.contains("cannot use"), "{why}: {note}");
    }
    let note = answer(r#"{"status":"failed","text":"","note":"Vision said no."}"#).unwrap_err();
    assert_eq!(note, "Vision said no.");
    let note = answer(r#"{"status":"unsupported","text":"","note":null}"#).unwrap_err();
    assert!(note.contains("could not read"), "{note}");
}

// ---------------------------------------------------------------------------
// Ready, preparing, off
// ---------------------------------------------------------------------------

#[test]
fn a_recognized_pdf_is_partial_marked_and_sent_like_any_text() {
    let s = setup();
    s.answering(RECOGNIZED);
    let id = s.choose("scan.pdf");
    let view = s.view(&id);
    assert_eq!(view.stage, OperationStage::ReadyForAnalysis, "{view:?}");
    let extraction = view.file.unwrap().extraction;
    assert!(extraction.usable_text);
    assert!(extraction.recognized);
    assert_eq!(extraction.status, ExtractionStatus::Partial);
    assert_eq!(
        extraction.note.as_deref(),
        Some("Read by on-device text recognition from 2 scanned pages; it may contain mistakes.")
    );

    s.analyze(&id);
    assert_eq!(s.view(&id).stage, OperationStage::RecommendationReady);
    // Sent like any text (§5 fields only): the excerpt is the recognized text.
    let data = s.sent_data();
    assert_eq!(data["file_name"], "scan.pdf");
    assert!(data.to_string().contains("Trip to Kyoto in May"));
    let keys: Vec<&String> = data.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "answered_questions",
            "current_folder",
            "file_name",
            "more_folders_not_listed",
            "target_folders",
            "text"
        ]
    );
    // The request's kept metadata says where the text came from.
    assert_eq!(s.request_meta()["context"]["excerpt"]["text_source"], "ocr");
    assert_eq!(s.server.count(), 1);
    // The reread before sending used the recognition already made.
    assert_eq!(s.runs(), 1);
}

/// R4-OCR-05: the child's note on pages it left out (a 4-page scan, page 4
/// over the 3-page limit) stays with the text, after Traeva's own warning,
/// also when the kept recognition is used again.
#[test]
fn the_child_s_note_on_left_out_pages_is_kept_with_the_text() {
    let s = setup();
    let child_note = "OCR read 3 scanned pages; may contain errors. 1 pages omitted. Other \
                      reader omissions, if any, remain. Omitted pages: 4.";
    let answer = json!({
        "status": "partial",
        "text": "[Page 1]\nTrip to Kyoto\n[Page 2]\nTrain tickets\n[Page 3]\nBudget",
        "note": child_note,
    });
    s.answering(&answer.to_string());
    let expected = "Read by on-device text recognition from 3 scanned pages; it may contain \
                    mistakes. 1 pages omitted. Other reader omissions, if any, remain. Omitted \
                    pages: 4.";
    let id = s.choose("scan.pdf");
    let note = |id: &str| s.view(id).file.unwrap().extraction.note;
    assert_eq!(note(&id).as_deref(), Some(expected));
    // The reread before sending and a later choice use the kept recognition.
    s.analyze(&id);
    assert_eq!(s.view(&id).stage, OperationStage::RecommendationReady);
    assert_eq!(note(&id).as_deref(), Some(expected));
    let again = s.choose("scan.pdf");
    assert_eq!(note(&again).as_deref(), Some(expected));
    assert_eq!(s.runs(), 1);

    // Without a note, or with one in other words, nothing is lost.
    let warning = "Read by on-device text recognition from 1 scanned page; it may contain \
                   mistakes.";
    assert_eq!(recognized_note(1, None), warning);
    assert_eq!(recognized_note(1, Some("  ")), warning);
    assert_eq!(
        recognized_note(1, Some("Page 2 was blank.")),
        format!("{warning} Page 2 was blank.")
    );
    assert_eq!(
        recognized_note(1, Some("OCR read 2 scanned pages; may contain errors.")),
        format!("{warning} OCR read 2 scanned pages; may contain errors."),
        "a count that does not match is kept as the child wrote it"
    );
}

#[test]
fn before_the_recognizer_is_ready_a_scanned_pdf_goes_to_a_manual_pick() {
    let s = setup();
    // Off: as before §14.8.
    assert_eq!(s.service().ocr_state(), OcrState::Off);
    let id = s.choose("scan.pdf");
    manual_pick(&s, &id, "needs OCR");

    // Set, not prepared yet: still the reader's note.
    let slow = sh("exec /bin/sleep 30", &[]);
    let recognize = s.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", RECOGNIZED);
    s.service()
        .set_ocr_helper(Some(OcrHelper::commands(recognize, slow, small)));
    assert_eq!(s.service().ocr_state(), OcrState::NotPrepared);
    let id = s.choose("scan.pdf");
    manual_pick(&s, &id, "needs OCR");

    // Preparing: the note says recognition is getting ready.
    std::thread::scope(|scope| {
        let preparing = scope.spawn(|| s.service().start_ocr_preparation());
        let deadline = Instant::now() + Duration::from_secs(10);
        while s.service().ocr_state() != OcrState::Preparing && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let id = s.choose("scan.pdf");
        manual_pick(&s, &id, "Text recognition is getting ready");
        // Quitting stops it; it is not tried again.
        s.service().stop_ocr();
        assert_eq!(preparing.join().unwrap(), OcrState::Cancelled);
    });
    assert_eq!(s.service().start_ocr_preparation(), OcrState::Cancelled);
    assert_eq!(s.runs(), 0, "nothing was recognized");
}

#[test]
fn preparation_ends_ready_failed_or_timed_out_and_runs_once() {
    let s = setup();
    let recognize = || s.recognizer("printf '%s' \"$1\"", RECOGNIZED);
    for (prepare, expected) in [
        (ready(), OcrState::Ready),
        (sh("exit 1", &[]), OcrState::Failed),
        (
            sh("printf '%s' '{\"status\":\"done\"}'", &[]),
            OcrState::Failed,
        ),
        (
            sh("printf '%s' '{\"status\":\"ready\",\"x\":1}'", &[]),
            OcrState::Failed,
        ),
        (sh("kill -ABRT $$", &[]), OcrState::Failed),
        (sh("exec /bin/sleep 30", &[]), OcrState::TimedOut),
    ] {
        s.service().set_ocr_helper(Some(
            OcrHelper::commands(recognize(), prepare, small)
                .with_time_limits(Duration::from_secs(2), Duration::from_secs(1)),
        ));
        let started = Instant::now();
        assert_eq!(s.service().start_ocr_preparation(), expected);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(s.service().ocr_state(), expected);
        // Only once per launch: asking again changes nothing.
        assert_eq!(s.service().start_ocr_preparation(), expected);
    }
    // A preparation whose memory cannot be read, or is too high, fails.
    for probe in [huge as MemoryProbe, unreadable] {
        s.service().set_ocr_helper(Some(OcrHelper::commands(
            recognize(),
            sh("/bin/sleep 1; printf '%s' '{\"status\":\"ready\"}'", &[]),
            probe,
        )));
        assert_eq!(s.service().start_ocr_preparation(), OcrState::Failed);
    }
    s.service().set_ocr_helper(None);
    assert_eq!(s.service().ocr_state(), OcrState::Off);
    assert_eq!(s.service().start_ocr_preparation(), OcrState::Off);
}

// ---------------------------------------------------------------------------
// Stops and failures: manual pick, nothing sent, no retry
// ---------------------------------------------------------------------------

#[test]
fn a_recognition_that_takes_too_long_is_stopped_and_ready_is_revoked() {
    let s = setup();
    let slow = s.recognizer("exec /bin/sleep 30", "");
    s.ready_within(slow, small, Duration::from_secs(2));
    let started = Instant::now();
    let id = s.choose("scan.pdf");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "waited too long"
    );
    manual_pick(&s, &id, "took too long");
    assert_eq!(s.service().ocr_state(), OcrState::Failed);
    // No retry: neither this file again nor another scanned file.
    fs::write(s.f.path("scan-2.pdf"), b"%PDF-1.7 another scan\n").unwrap();
    let again = s.choose("scan.pdf");
    manual_pick(&s, &again, "took too long");
    let other = s.choose("scan-2.pdf");
    manual_pick(&s, &other, "needs OCR");
    assert_eq!(s.runs(), 1);
}

#[test]
fn a_recognition_over_its_memory_limit_is_stopped_and_ready_is_revoked() {
    // While it runs: stopped at the first look.
    for (probe, words) in [
        (huge_when_recognizing as MemoryProbe, "more memory"),
        (
            unreadable_when_recognizing,
            "could not check how much memory",
        ),
    ] {
        let s = setup();
        let recognize = s.recognizer("/bin/sleep 1; printf '%s' \"$1\"", RECOGNIZED);
        RECOGNIZING.store(false, Ordering::SeqCst);
        s.ready_with(recognize, probe);
        RECOGNIZING.store(true, Ordering::SeqCst);
        LOOKS.store(0, Ordering::SeqCst);
        let id = s.choose("scan.pdf");
        manual_pick(&s, &id, words);
        assert_eq!(s.service().ocr_state(), OcrState::Failed);
        // Stopped at the first look; no second child for the same file.
        assert_eq!(LOOKS.load(Ordering::SeqCst), 1);
        let again = s.choose("scan.pdf");
        manual_pick(&s, &again, words);
        assert_eq!(LOOKS.load(Ordering::SeqCst), 1);
        assert!(s.runs() <= 1);
    }
    // R4-OCR-03: after a normal exit, before it is reaped, the last read
    // of its memory decides. A peak over the budget, or memory that cannot
    // be read then, drops its well-formed answer and revokes Ready.
    for (probe, words) in [
        (huge_at_the_end as MemoryProbe, "more memory"),
        (unreadable_at_the_end, "could not check how much memory"),
    ] {
        let s = setup();
        let recognize = s.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", RECOGNIZED);
        RECOGNIZING.store(false, Ordering::SeqCst);
        s.ready_with(recognize, probe);
        RECOGNIZING.store(true, Ordering::SeqCst);
        let id = s.choose("scan.pdf");
        manual_pick(&s, &id, words);
        assert_eq!(s.service().ocr_state(), OcrState::Failed);
        assert_eq!(s.runs(), 1, "it ran to its end");
        let again = s.choose("scan.pdf");
        manual_pick(&s, &again, words);
        assert_eq!(s.runs(), 1);
    }
    RECOGNIZING.store(false, Ordering::SeqCst);
}

#[test]
fn a_crash_or_an_unusable_answer_goes_to_a_manual_pick_once() {
    for (script, answer, words) in [
        ("kill -ABRT $$", "", "stopped before it finished"),
        (
            "printf '%s' \"$1\"",
            "{\"status\":\"partial\"}",
            "cannot use",
        ),
        ("printf '%s' \"$1\"", "not json", "cannot use"),
        (
            "printf '%s' \"$1\"",
            "{\"status\":\"partial\",\"text\":\"[Page 1]\\nx\",\"note\":null,\"pages\":1}",
            "cannot use",
        ),
        (
            "printf '%s' \"$1\"",
            "{\"status\":\"complete\",\"text\":\"[Page 1]\\nx\",\"note\":null}",
            "cannot use",
        ),
        (
            "printf '%s' \"$1\"",
            "{\"status\":\"failed\",\"text\":\"\",\"note\":\"Vision said no.\"}",
            "Vision said no.",
        ),
        (
            "printf '%s' \"$1\"",
            "{\"status\":\"partial\",\"text\":\"[Page 1]\\n \",\"note\":null}",
            "cannot use",
        ),
    ] {
        let s = setup();
        let recognize = s.recognizer(&format!("/bin/cat > /dev/null; {script}"), answer);
        s.ready_with(recognize, small);
        let id = s.choose("scan.pdf");
        manual_pick(&s, &id, words);
        // The recognizer stays ready, and the same bytes are not
        // recognized again in this launch.
        assert_eq!(s.service().ocr_state(), OcrState::Ready, "{answer}");
        let again = s.choose("scan.pdf");
        manual_pick(&s, &again, words);
        assert_eq!(s.runs(), 1, "{answer}");
    }
}

/// The pid of the child `pause_first` paused (0: none yet). Only the
/// paused-child test uses it.
static PAUSED: AtomicU32 = AtomicU32::new(0);

/// Pauses the first child it looks at (SIGSTOP), right after it started,
/// and reads a small footprint for every child.
fn pause_first(pid: u32) -> Option<ProcessMemory> {
    if PAUSED
        .compare_exchange(0, pid, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // SAFETY: a signal to the child the service under test just started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) };
    }
    small(pid)
}

/// The process is gone and reaped: signal 0 finds no such process (a
/// zombie that was not reaped would still be found).
fn reaped(pid: u32) -> bool {
    // SAFETY: signal 0 sends nothing; it only checks that the process exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// R4 on the experiment (#211): a paused child is not an exited one. A
/// recognition or preparation child stopped with SIGSTOP right after it
/// started still meets the time limit (or the stop when the app quits): it
/// is killed and reaped, the one-child slot is free again, and the next
/// preparation and recognition run.
#[test]
fn a_paused_child_is_killed_reaped_and_the_slot_is_free_again() {
    let quick = |recognize, prepare| {
        OcrHelper::commands(recognize, prepare, pause_first)
            .with_time_limits(Duration::from_secs(1), Duration::from_secs(1))
    };
    // The next run: prepared, then a recognition.
    let next_runs = |s: &Setup| {
        let recognize = s.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", RECOGNIZED);
        s.ready_with(recognize, small);
        let id = s.choose("scan.pdf");
        assert!(s.view(&id).file.unwrap().extraction.recognized);
    };

    // A paused preparation child: timed out, killed, reaped.
    let s = setup();
    PAUSED.store(0, Ordering::SeqCst);
    let started = Instant::now();
    let slow_ready = sh("/bin/sleep 0.5; printf '%s' '{\"status\":\"ready\"}'", &[]);
    let recognize = s.recognizer("printf '%s' \"$1\"", RECOGNIZED);
    s.service()
        .set_ocr_helper(Some(quick(recognize, slow_ready)));
    assert_eq!(s.service().start_ocr_preparation(), OcrState::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid = PAUSED.load(Ordering::SeqCst);
    assert!(
        pid != 0 && reaped(pid),
        "the paused child {pid} is left over"
    );
    assert!(s.service().ocr_child.try_lock().is_ok(), "the slot is free");
    next_runs(&s);

    // A paused recognition child: timed out (Ready is revoked), killed,
    // reaped; a recognizer set again prepares and recognizes.
    let s = setup();
    PAUSED.store(1, Ordering::SeqCst); // the preparation is not paused
    let slow_answer = s.recognizer("/bin/sleep 0.5; printf '%s' \"$1\"", RECOGNIZED);
    s.service()
        .set_ocr_helper(Some(quick(slow_answer, ready())));
    assert_eq!(s.service().start_ocr_preparation(), OcrState::Ready);
    PAUSED.store(0, Ordering::SeqCst);
    let id = s.choose("scan.pdf");
    manual_pick(&s, &id, "took too long");
    assert_eq!(s.service().ocr_state(), OcrState::Failed);
    let pid = PAUSED.load(Ordering::SeqCst);
    assert!(
        pid != 0 && reaped(pid),
        "the paused child {pid} is left over"
    );
    assert!(s.service().ocr_child.try_lock().is_ok(), "the slot is free");
    next_runs(&s);

    // A paused preparation child when the app quits: stopped and reaped.
    let s = setup();
    PAUSED.store(0, Ordering::SeqCst);
    let slow = sh("exec /bin/sleep 30", &[]);
    let recognize = s.recognizer("printf '%s' \"$1\"", RECOGNIZED);
    s.service().set_ocr_helper(Some(
        OcrHelper::commands(recognize, slow, pause_first)
            .with_time_limits(Duration::from_secs(30), Duration::from_secs(30)),
    ));
    std::thread::scope(|scope| {
        let preparing = scope.spawn(|| s.service().start_ocr_preparation());
        let deadline = Instant::now() + Duration::from_secs(10);
        while PAUSED.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        s.service().stop_ocr();
        assert_eq!(preparing.join().unwrap(), OcrState::Cancelled);
    });
    let pid = PAUSED.load(Ordering::SeqCst);
    assert!(
        pid != 0 && reaped(pid),
        "the paused child {pid} is left over"
    );
    assert!(s.service().ocr_child.try_lock().is_ok(), "the slot is free");
}

#[test]
fn one_recognition_runs_at_a_time_and_the_next_one_waits() {
    let s = setup();
    fs::write(s.f.path("scan-2.pdf"), b"%PDF-1.7 another scan\n").unwrap();
    // Each run notes its start and end around a short sleep.
    let log = s.runs.path().join("log").into_os_string();
    let script = "echo start >> \"$0\"; /bin/cat > /dev/null; /bin/sleep 0.3; \
                  echo end >> \"$0\"; printf '%s' \"$1\"";
    s.ready_with(sh(script, &[&log, &OsString::from(RECOGNIZED)]), small);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| s.choose("scan.pdf"));
        let second = scope.spawn(|| s.choose("scan-2.pdf"));
        for id in [first.join().unwrap(), second.join().unwrap()] {
            let extraction = s.view(&id).file.unwrap().extraction;
            assert!(extraction.recognized, "{extraction:?}");
        }
    });
    let log = fs::read_to_string(s.runs.path().join("log")).unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        ["start", "end", "start", "end"]
    );
}

// ---------------------------------------------------------------------------
// Only Organize
// ---------------------------------------------------------------------------

#[test]
fn registering_a_file_for_memory_or_source_never_recognizes() {
    let s = setup();
    s.answering(RECOGNIZED);
    // Memory's and Source's way in (`register_file`, `start_memory_at`).
    let observed = s
        .service()
        .register_file(&s.f.root_id, &s.f.path("scan.pdf"))
        .unwrap();
    assert!(!observed.extraction.summary.recognized);
    assert!(!observed.extraction.summary.usable_text);
    assert_eq!(s.runs(), 0);
    // Memory reads no PDF at all.
    let read = crate::reader::read_text("scan.pdf", b"%PDF-1.7 scanned stand-in\n");
    assert_eq!(read.summary.status, TextStatus::Unsupported);
}

#[test]
fn the_entries_and_the_monitor_are_g_ocr_s() {
    // The entries refuse missing arguments before any Vision work.
    assert_eq!(ocr_prepare_main(&[]), 2);
    assert_eq!(ocr_child_main(&["--parent-pid".to_string()]), 2);
    // The monitor reads real numbers for a live process.
    let me = child_footprint(std::process::id()).expect("a live process");
    assert!(me.footprint_bytes > 0 && me.counted() >= me.footprint_bytes);
}

/// On by default (#186, organize-spec 14.8 v0.8). The constant is the only
/// switch: the verification variable of v0.7 is gone, so nothing outside
/// the code turns recognition on or off.
#[test]
fn recognition_is_on_by_default() {
    const { assert!(OCR_ENABLED) };
}

/// The app sets up recognition only where macOS can recognize text (13 or
/// later; `pdf_ocr` tests the version rule). The tests run on such a Mac.
#[test]
fn this_mac_can_recognize_text() {
    assert!(ocr_supported_here());
}

/// G-OCR's interface: both children learn the parent's pid; recognition
/// also gets the pre-processing budget.
#[test]
fn the_children_get_the_parent_s_pid_and_the_budget() {
    let helper = OcrHelper::traeva(PathBuf::from(
        "/Applications/Traeva.app/Contents/MacOS/Traeva",
    ));
    let (recognize, prepare) = helper.arguments();
    let pid = std::process::id().to_string();
    let words = |args: &[OsString]| -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(
        words(recognize),
        [
            OCR_FLAG,
            "--max-pages",
            "3",
            "--dpi",
            "200",
            "--max-pixels",
            "8000000",
            "--parent-pid",
            &pid
        ]
    );
    assert_eq!(words(prepare), [OCR_PREPARE_FLAG, "--parent-pid", &pid]);
}

/// R4-OCR-02: bytes recognition ran on are never recognized again in this
/// launch, also after their result left the 8 kept: a failure stays a
/// failure, and a text no longer kept is not recognized again either.
#[test]
fn bytes_recognized_once_never_start_another_child_in_this_launch() {
    let s = setup();
    // Fails (crashes) on bytes holding FAIL; answers for any other.
    let recognize = s.recognizer(
        "input=$(/bin/cat); case \"$input\" in *FAIL*) kill -ABRT $$;; \
         *) printf '%s' \"$1\";; esac",
        RECOGNIZED,
    );
    s.ready_with(recognize, small);
    fs::write(s.f.path("fail.pdf"), b"%PDF-1.7 FAIL\n").unwrap();
    fs::write(s.f.path("first.pdf"), b"%PDF-1.7 first\n").unwrap();
    let failed = s.choose("fail.pdf");
    manual_pick(&s, &failed, "stopped before it finished");
    let first = s.choose("first.pdf");
    assert!(s.view(&first).file.unwrap().extraction.recognized);
    // 8 other scans push both out of the kept recognitions.
    for n in 1..=8 {
        let name = format!("other-{n}.pdf");
        fs::write(s.f.path(&name), format!("%PDF-1.7 other {n}\n")).unwrap();
        let id = s.choose(&name);
        assert!(s.view(&id).file.unwrap().extraction.recognized, "{name}");
    }
    assert_eq!(s.runs(), 10);
    assert_eq!(s.service().ocr_state(), OcrState::Ready);

    let again = s.choose("fail.pdf");
    manual_pick(&s, &again, "did not work on this PDF earlier in this run");
    let again = s.choose("first.pdf");
    manual_pick(&s, &again, "recognized earlier in this run");
    assert_eq!(s.runs(), 10, "no new child");
}

// ---------------------------------------------------------------------------
// waitid errors and answers that come too late (the coordinator's conditions
// before OCR is turned on, #211): `fail_waitid` makes the next looks at a
// child on this thread fail, since a real waitid with WNOHANG almost never
// does.
// ---------------------------------------------------------------------------

thread_local! {
    /// The last child a probe below looked at, on this thread.
    static NOTED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Set once the recognizer is prepared: the probes are also called for
    /// the preparation child, which must not fail.
    static PREPARED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn noted() -> u32 {
    NOTED.get()
}

/// Notes the child and reads a small footprint.
fn noting(pid: u32) -> Option<ProcessMemory> {
    NOTED.set(pid);
    small(pid)
}

/// Notes the child, and (once prepared) the next look at it fails with
/// ECHILD, as if someone else had reaped it.
fn then_echild(pid: u32) -> Option<ProcessMemory> {
    NOTED.set(pid);
    if PREPARED.get() {
        crate::pdf_child::fail_waitid(libc::ECHILD, 1);
    }
    small(pid)
}

/// Notes the child, and (once prepared) the next look at it fails with
/// EINVAL.
fn then_einval(pid: u32) -> Option<ProcessMemory> {
    NOTED.set(pid);
    if PREPARED.get() {
        crate::pdf_child::fail_waitid(libc::EINVAL, 1);
    }
    small(pid)
}

/// Notes the child; once it has ended, takes longer than the 1 s time
/// limit before it lets the parent go on, so its answer comes too late.
fn slow_at_the_end(pid: u32) -> Option<ProcessMemory> {
    NOTED.set(pid);
    if crate::pdf_child::ended(pid) == Some(true) {
        std::thread::sleep(Duration::from_millis(1200));
    }
    small(pid)
}

/// Set by `stop_at_the_end` once the child has ended: the stop comes after
/// the answer, before the child is reaped.
static LATE_STOP: AtomicBool = AtomicBool::new(false);

fn stop_at_the_end(pid: u32) -> Option<ProcessMemory> {
    NOTED.set(pid);
    if crate::pdf_child::ended(pid) == Some(true) {
        LATE_STOP.store(true, Ordering::SeqCst);
    }
    small(pid)
}

/// The child is gone and reaped, and the one-child slot is free.
fn cleaned_up(s: &Setup) {
    let pid = noted();
    assert!(pid != 0 && reaped(pid), "the child {pid} is left over");
    assert!(s.service().ocr_child.try_lock().is_ok(), "the slot is free");
}

/// EINTR means a signal came in, not that the child ended: it is looked at
/// again, and a recognition that ends normally afterwards gives its text.
#[test]
fn a_look_interrupted_by_a_signal_is_made_again() {
    let s = setup();
    let recognize = s.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", RECOGNIZED);
    s.ready_with(recognize, noting);
    crate::pdf_child::fail_waitid(libc::EINTR, 3);
    let id = s.choose("scan.pdf");
    crate::pdf_child::fail_waitid(0, 0);
    let extraction = s.view(&id).file.unwrap().extraction;
    assert!(extraction.recognized, "{extraction:?}");
    assert_eq!(s.service().ocr_state(), OcrState::Ready);
    assert_eq!(s.runs(), 1);
    cleaned_up(&s);
}

/// A child whose every look is interrupted still meets the time limit: it
/// is killed and reaped, nothing is used, Ready is revoked, and it is not
/// tried again.
#[test]
fn looks_interrupted_every_time_still_meet_the_time_limit() {
    let s = setup();
    let recognize = s.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", RECOGNIZED);
    s.ready_within(recognize, noting, Duration::from_secs(1));
    crate::pdf_child::fail_waitid(libc::EINTR, usize::MAX);
    let started = Instant::now();
    let id = s.choose("scan.pdf");
    crate::pdf_child::fail_waitid(0, 0);
    assert!(started.elapsed() < Duration::from_secs(10));
    manual_pick(&s, &id, "took too long");
    assert_eq!(s.service().ocr_state(), OcrState::Failed);
    cleaned_up(&s);
    let again = s.choose("scan.pdf");
    manual_pick(&s, &again, "took too long");
    assert_eq!(s.runs(), 1);
}

/// Any other waitid error: whether the child still runs cannot be told. It
/// is killed and reaped, nothing is used (a manual pick, nothing sent),
/// Ready is revoked, and the same bytes are not tried again.
#[test]
fn another_waitid_error_stops_the_child_and_revokes_ready() {
    for probe in [then_echild as MemoryProbe, then_einval] {
        let s = setup();
        // Slow enough that the first look finds it running.
        let recognize = s.recognizer(
            "/bin/cat > /dev/null; /bin/sleep 0.3; printf '%s' \"$1\"",
            RECOGNIZED,
        );
        PREPARED.set(false);
        s.ready_with(recognize, probe);
        PREPARED.set(true);
        let id = s.choose("scan.pdf");
        crate::pdf_child::fail_waitid(0, 0);
        PREPARED.set(false);
        manual_pick(&s, &id, "lost track");
        assert_eq!(s.service().ocr_state(), OcrState::Failed);
        cleaned_up(&s);
        let again = s.choose("scan.pdf");
        manual_pick(&s, &again, "lost track");
        // Killed a few milliseconds after it started, the child may not
        // have noted its run yet; what counts is that no second one ran.
        assert!(s.runs() <= 1);
    }
}

/// An answer that is ready only after the time limit (the child ended, but
/// the parent got to it late) is dropped: Ready is revoked, nothing sent.
#[test]
fn an_answer_after_the_time_limit_is_dropped() {
    let s = setup();
    let recognize = s.recognizer("/bin/cat > /dev/null; printf '%s' \"$1\"", RECOGNIZED);
    s.ready_within(recognize, slow_at_the_end, Duration::from_secs(1));
    let id = s.choose("scan.pdf");
    manual_pick(&s, &id, "took too long");
    assert_eq!(s.service().ocr_state(), OcrState::Failed);
    assert_eq!(s.runs(), 1);
    cleaned_up(&s);
}

/// A stop that comes after the child answered but before it is reaped
/// drops the answer (`run_child`'s last checks), and the child is reaped.
#[test]
fn a_stop_after_the_answer_drops_it() {
    LATE_STOP.store(false, Ordering::SeqCst);
    let (program, args) = sh("printf '%s' \"$0\"", &[&OsString::from(RECOGNIZED)]);
    let watch = Watch {
        time_limit: Duration::from_secs(10),
        poll: Duration::from_millis(5),
        memory: Some((stop_at_the_end, u64::MAX)),
        stop: Some(&LATE_STOP),
    };
    let ended = run_child(&program, &args, None, MAX_ANSWER_BYTES, &watch);
    assert!(matches!(ended, Ended::Stopped), "{ended:?}");
    let pid = noted();
    assert!(pid != 0 && reaped(pid), "the child {pid} is left over");
}

#[path = "ocr_image_tests.rs"]
mod image;

#[path = "ocr_model_image_tests.rs"]
mod model_image;
