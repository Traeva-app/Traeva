//! On-device text recognition (OCR) for scanned PDFs, the parent's side
//! (organize-spec.md §14.8, v0.7, #186).
//!
//! Only Organize uses it, and only for a PDF whose text layer the PDF
//! reader found empty because the pages are images. Memory and Find never
//! do. Recognition runs in a child: Traeva's own executable started with
//! `OCR_FLAG`, the PDF bytes on stdin, one JSON answer on stdout (the same
//! shape as the PDF child's). Two budgets, kept apart: before recognizing,
//! the child itself keeps to at most 3 pages, 200 dpi and 8 million pixels
//! a page; while it runs, the parent kills it after `OCR_TIME_LIMIT`, or
//! when the memory probe reads more than `OCR_MEMORY_LIMIT`, and reads the
//! probe once more after the child ended, before reaping it. That stop is
//! a soft budget: the parent looks every 5 ms, so a child can pass it for a
//! moment before it is stopped. (The PDF text child's 512 MiB counting
//! heap is a different rule and stays as it is.) Any stop or failure: the
//! text is dropped, the file goes to a manual pick and nothing retries; the
//! same bytes are never recognized again in this launch. One OCR child
//! runs at a time; reading other files meanwhile is not held up, only the
//! next OCR child waits.
//!
//! Before the first recognition, a preparation child (`OCR_PREPARE_FLAG`)
//! runs once per launch on a synthetic image, in the background. Until it
//! says it is ready, a scanned PDF goes to the manual pick as before; a
//! preparation that fails, times out or is stopped is not tried again
//! until the next launch. Ready holds for this launch only, and only until
//! a recognition is stopped at its time or memory limit: then recognition
//! is off (`Failed`) until the next launch.
//!
//! On by default (`OCR_ENABLED`, the student's go on #186; R4's production
//! check: #211), on macOS 13 or later (`ocr_supported_here`). The child's
//! side (rendering the pages, Apple's Vision, the memory monitor)
//! is G-OCR's `pdf_ocr` module; this module starts it and watches it.
//!
//! The same child, the same limits and the same one-child slot also read
//! one PNG or JPEG image (`image_extraction`, #37; the student's decision of
//! 2026-10-14): the parent adds `--input image` to the child's arguments,
//! and the child answers with the recognized lines, without page markers.
//! Only that text is used; recognition never sends the image.
//!
//! The same child, limits and slot also make pictures for the model
//! (`images_for_model`, tier 2; the student's decision of 2026-10-14, sent
//! only with the Settings opt-in): the parent adds `--output jpeg`, and the
//! child answers with downscaled JPEGs drawn afresh, without any metadata,
//! which the parent checks again. Nothing of this mode is kept.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::contracts::ExtractionStatus;
use crate::observe::{self, Extraction};
use crate::pdf_child::{
    Answer, AnswerStatus, Ended, MAX_ANSWER_BYTES, MAX_NOTE_CHARS, Watch, run_child,
};
use crate::pdf_ocr::{
    MODEL_IMAGE_MAX_BYTES, MODEL_IMAGE_MAX_COUNT, MODEL_IMAGE_MAX_SIDE, base64_decode, is_bare_jpeg,
};
use crate::reader::MAX_MEMORY_TEXT_CHARS;

/// The switch, read by the app at start: false turns text recognition off
/// entirely (no preparation, no recognition; a scanned PDF goes to the
/// manual pick as in v0.5). On (organize-spec 14.8, v0.8). It is the only
/// switch: no environment variable turns recognition on or off, and the
/// window cannot either.
pub const OCR_ENABLED: bool = true;
/// The first argument that makes Traeva's executable recognize the text of
/// one scanned PDF from stdin, instead of opening the window.
pub const OCR_FLAG: &str = "--traeva-pdf-ocr";
/// The first argument that makes Traeva's executable prepare recognition
/// on a synthetic image and say whether it is ready.
pub const OCR_PREPARE_FLAG: &str = "--traeva-ocr-prepare";
/// At most this many pages without a text layer are recognized: the first.
pub const OCR_MAX_PAGES: usize = 3;
/// Pages are drawn at this resolution for recognition.
pub const OCR_DPI: u32 = 200;
/// At most this many pixels per drawn page.
pub const OCR_MAX_PIXELS: u64 = 8_000_000;
/// How long one recognition may take, once prepared.
pub const OCR_TIME_LIMIT: Duration = Duration::from_secs(15);
/// How long the preparation may take.
pub const OCR_PREPARE_TIME_LIMIT: Duration = Duration::from_secs(90);
/// The most memory a recognition or preparation child may use (the larger
/// of its footprint and its lifetime peak): killed above it, its answer
/// dropped. A soft budget (see the module docs).
pub const OCR_MEMORY_LIMIT: u64 = 384 << 20;
/// How often the parent reads the child's footprint.
pub const OCR_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Recognitions kept in memory, by the bytes' SHA-256.
const KEPT_RECOGNITIONS: usize = 8;
/// Longest preparation answer read.
const MAX_PREPARE_ANSWER_BYTES: usize = 1024;
/// How long `stop_ocr` waits for a running child to be stopped.
const STOP_WAIT: Duration = Duration::from_secs(1);

/// The manual pick's note while recognition is still being prepared.
pub const OCR_GETTING_READY_NOTE: &str =
    "Text recognition is getting ready (the first time can take about a minute).";

/// The extra arguments that make the recognition child read one image.
const IMAGE_INPUT: [&str; 2] = ["--input", "image"];
/// The fewest non-white-space characters an image's text needs (the
/// child's `min_page_chars`).
pub const OCR_MIN_IMAGE_CHARS: usize = 10;

/// Traeva's warning on text recognized from an image.
pub const IMAGE_RECOGNIZED_NOTE: &str =
    "Read by on-device text recognition from an image; it may contain mistakes.";
/// The manual pick's note for an image when recognition is off or not
/// working in this launch.
const IMAGE_NOT_READ_NOTE: &str = "Traeva reads an image's text only with on-device text \
                                   recognition, which is not available right now.";
const IMAGE_NOT_AN_IMAGE_NOTE: &str = "Only PNG and JPEG images are recognized.";
const IMAGE_FAILED_EARLIER_NOTE: &str = "Text recognition did not work on this image earlier \
                                         in this run; Traeva tries it again only after it \
                                         restarts.";
const IMAGE_RECOGNIZED_EARLIER_NOTE: &str = "This image's text was recognized earlier in this \
                                             run and is no longer kept; choose it again after \
                                             restarting Traeva.";
const IMAGE_NO_TEXT_NOTE: &str = "Text recognition could not read text in this image.";

/// The extra arguments that make the recognition child answer with
/// pictures for the model instead of text.
const JPEG_OUTPUT: [&str; 2] = ["--output", "jpeg"];

/// The reasons `images_for_model` gives for no pictures.
const PICTURE_NOT_AVAILABLE: &str = "Picture preparation is not available right now.";
const PICTURE_GETTING_READY: &str =
    "Picture preparation is getting ready (the first time can take about a minute).";
const PICTURE_NOT_AN_IMAGE: &str = "Only PNG and JPEG images can be sent as pictures.";
const PICTURE_UNUSABLE: &str = "Preparing the picture gave an answer Traeva cannot use.";
const PICTURE_NOT_PREPARED: &str = "The picture could not be prepared.";

/// What people are told when the OCR child gave no answer, by how it ended.
struct StopNotes {
    crashed: &'static str,
    not_started: &'static str,
    timed_out: &'static str,
    over_memory: &'static str,
    unmeasured: &'static str,
    lost: &'static str,
    stopped: &'static str,
}

const RECOGNITION_STOPS: StopNotes = StopNotes {
    crashed: "Text recognition stopped before it finished.",
    not_started: "Traeva could not start text recognition.",
    timed_out: "Text recognition took too long, so Traeva stopped it.",
    over_memory: "Text recognition needed more memory than Traeva allows, so Traeva stopped it.",
    unmeasured: "Traeva could not check how much memory text recognition used, so it stopped it.",
    lost: "Traeva lost track of text recognition, so it stopped it.",
    stopped: "Text recognition was stopped.",
};

const PICTURE_STOPS: StopNotes = StopNotes {
    crashed: "Preparing the picture stopped before it finished.",
    not_started: "Traeva could not start preparing the picture.",
    timed_out: "Preparing the picture took too long, so Traeva stopped it.",
    over_memory: "Preparing the picture needed more memory than Traeva allows, so Traeva stopped \
                  it.",
    unmeasured: "Traeva could not check how much memory preparing the picture used, so it \
                 stopped it.",
    lost: "Traeva lost track of preparing the picture, so it stopped it.",
    stopped: "Preparing the picture was stopped.",
};

/// The PDF reader's notes (pdf.rs `no_text_note`) for a PDF with no text
/// layer on any page whose pages are images: mostly images (likely scans),
/// or only images and drawings. Only these start a recognition; a PDF with
/// characters it cannot map, fonts it cannot decode, blank pages,
/// encryption or a broken file does not.
const NO_TEXT_LAYER_NOTES: [&str; 2] = [
    "No page has a text layer that this reader can use. ",
    "No page has a text layer; the PDF holds only images and drawings.",
];

/// The notes for bytes recognition already ran on in this launch, once
/// their result is no longer kept.
const FAILED_EARLIER_NOTE: &str = "Text recognition did not work on this PDF earlier in this \
                                   run; Traeva tries it again only after it restarts.";
const RECOGNIZED_EARLIER_NOTE: &str = "This PDF's text was recognized earlier in this run and \
                                       is no longer kept; choose it again after restarting \
                                       Traeva.";

/// What the memory monitor reads about a child, in bytes (`pdf_ocr`).
pub use crate::pdf_ocr::ProcessMemory;

impl ProcessMemory {
    /// What is held against `OCR_MEMORY_LIMIT`.
    pub(crate) fn counted(&self) -> u64 {
        self.footprint_bytes.max(self.lifetime_peak_bytes)
    }
}

/// Reads a child's memory by its process ID; None when it cannot be read.
/// A child whose memory cannot be read is stopped and its answer dropped.
pub type MemoryProbe = fn(u32) -> Option<ProcessMemory>;

/// The memory monitor the app uses: `pdf_ocr::process_memory`, on
/// `proc_pid_rusage` (current footprint and lifetime peak).
pub fn child_footprint(pid: u32) -> Option<ProcessMemory> {
    crate::pdf_ocr::process_memory(pid)
}

/// Whether this Mac can recognize text at all: macOS 13 or later (Apple's
/// Vision text recognition, revision 3). On an older system the app does
/// not set up recognition, so its state is `off` and scanned PDFs go to the
/// manual pick as in v0.5.
pub fn ocr_supported_here() -> bool {
    crate::pdf_ocr::system_supported()
}

/// What Traeva's executable does with `OCR_FLAG`: recognize one scanned
/// PDF from stdin (`pdf_ocr`). `args` are the ones after the flag.
pub fn ocr_child_main(args: &[String]) -> i32 {
    crate::pdf_ocr::ocr_child_main(args)
}

/// What Traeva's executable does with `OCR_PREPARE_FLAG`: prepare
/// recognition on a synthetic image (`pdf_ocr`).
pub fn ocr_prepare_main(args: &[String]) -> i32 {
    crate::pdf_ocr::ocr_prepare_main(args)
}

/// Where text recognition stands in this launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrState {
    /// Turned off, or no recognizer was set.
    #[default]
    Off,
    /// Set, and the preparation has not started.
    NotPrepared,
    Preparing,
    Ready,
    /// The preparation failed, or a recognition was stopped at its time or
    /// memory limit: no recognition until the next launch.
    Failed,
    /// The preparation passed its time limit.
    TimedOut,
    /// The preparation was stopped (the app is quitting).
    Cancelled,
}

/// Whether a reading may use text recognition: only Organize's reading of
/// a chosen or dropped file and its reread before sending. Memory's and
/// Source's registration of a file, and Find, never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OcrUse {
    Allowed,
    Never,
}

/// A program and its arguments.
#[derive(Debug, Clone)]
struct ChildCommand {
    program: PathBuf,
    args: Vec<OsString>,
}

/// How Organize recognizes text: the two children, their limits and the
/// memory probe.
#[derive(Debug, Clone)]
pub struct OcrHelper {
    recognize: ChildCommand,
    prepare: ChildCommand,
    memory: MemoryProbe,
    time_limit: Duration,
    prepare_time_limit: Duration,
}

impl OcrHelper {
    /// Traeva's own executable with the two flags, the limits and this
    /// process's ID (the child stops when its parent is gone), and the
    /// memory monitor.
    pub fn traeva(executable: PathBuf) -> OcrHelper {
        let parent: [OsString; 2] = ["--parent-pid".into(), std::process::id().to_string().into()];
        let mut recognize: Vec<OsString> = vec![
            OCR_FLAG.into(),
            "--max-pages".into(),
            OCR_MAX_PAGES.to_string().into(),
            "--dpi".into(),
            OCR_DPI.to_string().into(),
            "--max-pixels".into(),
            OCR_MAX_PIXELS.to_string().into(),
        ];
        recognize.extend(parent.clone());
        let mut prepare: Vec<OsString> = vec![OCR_PREPARE_FLAG.into()];
        prepare.extend(parent);
        OcrHelper::commands(
            (executable.clone(), recognize),
            (executable, prepare),
            child_footprint,
        )
    }

    /// The two children's arguments: recognition's, then preparation's.
    pub fn arguments(&self) -> (&[OsString], &[OsString]) {
        (&self.recognize.args, &self.prepare.args)
    }

    /// Any two programs that behave like Traeva's executable after the two
    /// flags, and any probe (the tests use stand-ins).
    pub fn commands(
        recognize: (PathBuf, Vec<OsString>),
        prepare: (PathBuf, Vec<OsString>),
        memory: MemoryProbe,
    ) -> OcrHelper {
        OcrHelper {
            recognize: ChildCommand {
                program: recognize.0,
                args: recognize.1,
            },
            prepare: ChildCommand {
                program: prepare.0,
                args: prepare.1,
            },
            memory,
            time_limit: OCR_TIME_LIMIT,
            prepare_time_limit: OCR_PREPARE_TIME_LIMIT,
        }
    }

    pub fn with_time_limits(mut self, recognize: Duration, prepare: Duration) -> OcrHelper {
        self.time_limit = recognize;
        self.prepare_time_limit = prepare;
        self
    }

    fn watch<'a>(&self, time_limit: Duration, stop: &'a AtomicBool) -> Watch<'a> {
        Watch {
            time_limit,
            poll: OCR_POLL_INTERVAL,
            memory: Some((self.memory, OCR_MEMORY_LIMIT)),
            stop: Some(stop),
        }
    }
}

/// Text recognition in this launch: the recognizer, where its preparation
/// stands, and the recognitions kept. In memory only.
#[derive(Debug, Default)]
pub(crate) struct Ocr {
    helper: Option<OcrHelper>,
    state: OcrState,
    /// Raised when the recognizer is set again: a preparation that started
    /// before does not change the state.
    generation: u64,
    /// The last recognitions, newest last, by the bytes' SHA-256: the
    /// reread before sending and the request after an answer use the same
    /// text.
    kept: Vec<(String, Recognized)>,
    /// Every SHA-256 recognition ran on in this launch, and whether it gave
    /// text (hashes only, never text): such bytes are never recognized
    /// again, also after their result left `kept`.
    attempted: HashMap<String, bool>,
}

/// What a recognition (or a picture for the model) reads: a scanned PDF's
/// pages, or one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Input {
    ScannedPdf,
    Image,
}

impl Ocr {
    fn kept(&self, sha256: &str) -> Option<Recognized> {
        self.kept
            .iter()
            .find(|(key, _)| key == sha256)
            .map(|(_, recognized)| recognized.clone())
    }

    /// The result for bytes recognition already ran on: kept, or a note
    /// saying it ran earlier.
    fn earlier(&self, sha256: &str, input: Input) -> Option<Recognized> {
        if let Some(kept) = self.kept(sha256) {
            return Some(kept);
        }
        self.attempted.get(sha256).map(|&gave_text| {
            let (recognized_earlier, failed_earlier) = match input {
                Input::ScannedPdf => (RECOGNIZED_EARLIER_NOTE, FAILED_EARLIER_NOTE),
                Input::Image => (IMAGE_RECOGNIZED_EARLIER_NOTE, IMAGE_FAILED_EARLIER_NOTE),
            };
            if gave_text {
                Recognized::NoText {
                    status: ExtractionStatus::Unsupported,
                    note: recognized_earlier.to_string(),
                }
            } else {
                Recognized::failed(failed_earlier)
            }
        })
    }

    fn keep(&mut self, sha256: String, recognized: Recognized) {
        let gave_text = matches!(
            recognized,
            Recognized::Text { .. } | Recognized::ImageText { .. }
        );
        self.attempted.insert(sha256.clone(), gave_text);
        self.kept.retain(|(key, _)| *key != sha256);
        self.kept.push((sha256, recognized));
        if self.kept.len() > KEPT_RECOGNITIONS {
            self.kept.remove(0);
        }
    }
}

/// What one recognition gave.
#[derive(Debug, Clone)]
enum Recognized {
    /// Text from `pages` scanned pages, and the child's note on pages it
    /// left out and the reader's other omissions (at most 300 characters).
    Text {
        text: String,
        pages: usize,
        note: Option<String>,
    },
    /// Text recognized from one image.
    ImageText { text: String },
    /// No usable text, and why.
    NoText {
        status: ExtractionStatus,
        note: String,
    },
}

impl Recognized {
    fn failed(note: &str) -> Recognized {
        Recognized::NoText {
            status: ExtractionStatus::Failed,
            note: note.to_string(),
        }
    }

    /// Recognized text is excerpted like any text (§4.1), always partial,
    /// and marked as recognized.
    fn extraction(&self) -> Extraction {
        match self {
            Recognized::Text { text, pages, note } => {
                let mut extraction = observe::excerpt_text(text.clone());
                extraction.summary.status = ExtractionStatus::Partial;
                extraction.summary.recognized = true;
                extraction.summary.note = Some(recognized_note(*pages, note.as_deref()));
                extraction
            }
            Recognized::ImageText { text } => {
                let mut extraction = observe::excerpt_text(text.clone());
                extraction.summary.status = ExtractionStatus::Partial;
                extraction.summary.recognized = true;
                extraction.summary.note = Some(IMAGE_RECOGNIZED_NOTE.to_string());
                extraction
            }
            Recognized::NoText { status, note } => observe::not_extracted(*status, note),
        }
    }
}

/// The note for recognized text: Traeva's own warning, from the page
/// sections it counted, then the child's note on the pages it left out and
/// the reader's other omissions (R4-OCR-05), without the child's copy of
/// the warning.
fn recognized_note(pages: usize, child_note: Option<&str>) -> String {
    let warning = format!(
        "Read by on-device text recognition from {pages} scanned page{}; it may contain \
         mistakes.",
        if pages == 1 { "" } else { "s" }
    );
    let child_warning = format!("OCR read {pages} scanned pages; may contain errors.");
    let rest = child_note
        .map(|note| {
            note.strip_prefix(child_warning.as_str())
                .unwrap_or(note)
                .trim()
        })
        .unwrap_or_default();
    if rest.is_empty() {
        warning
    } else {
        format!("{warning} {rest}")
    }
}

/// True when the PDF reader's note says the PDF has no text layer because
/// its pages are images (`NO_TEXT_LAYER_NOTES`).
pub(crate) fn has_no_text_layer(reader_note: &str) -> bool {
    NO_TEXT_LAYER_NOTES
        .iter()
        .any(|start| reader_note.starts_with(start))
}

impl OrganizeService {
    /// Sets how Organize recognizes text (the app: its own executable, at
    /// start, when `OCR_ENABLED`). None turns recognition off. Either way
    /// the preparation starts over and nothing recognized before is kept.
    pub fn set_ocr_helper(&self, helper: Option<OcrHelper>) {
        let mut ocr = self.ocr();
        ocr.state = if helper.is_some() {
            OcrState::NotPrepared
        } else {
            OcrState::Off
        };
        ocr.helper = helper;
        ocr.generation += 1;
        ocr.kept.clear();
        ocr.attempted.clear();
    }

    /// Where text recognition stands, for the window.
    pub fn ocr_state(&self) -> OcrState {
        self.ocr().state
    }

    /// Runs the preparation child once, and waits for it: the app calls
    /// this once at start, on a background thread. Only a recognizer that
    /// was not prepared yet is prepared; any other state is returned as it
    /// is, so a failed preparation is not tried again in this launch.
    pub fn start_ocr_preparation(&self) -> OcrState {
        let (helper, generation) = {
            let mut ocr = self.ocr();
            let Some(helper) = ocr.helper.clone() else {
                return ocr.state;
            };
            if ocr.state != OcrState::NotPrepared {
                return ocr.state;
            }
            if self.ocr_stop.load(Ordering::SeqCst) {
                ocr.state = OcrState::Cancelled;
                return ocr.state;
            }
            ocr.state = OcrState::Preparing;
            (helper, ocr.generation)
        };
        let outcome = {
            let _one_child = self.ocr_child();
            prepare(&helper, &self.ocr_stop)
        };
        let mut ocr = self.ocr();
        if ocr.generation == generation {
            ocr.state = outcome;
        }
        ocr.state
    }

    /// The app is quitting: stops a running preparation or recognition
    /// (waiting up to a second for its child to end), and none starts
    /// afterwards.
    pub fn stop_ocr(&self) {
        self.ocr_stop.store(true, Ordering::SeqCst);
        {
            let mut ocr = self.ocr();
            if ocr.state == OcrState::NotPrepared {
                ocr.state = OcrState::Cancelled;
            }
        }
        let deadline = Instant::now() + STOP_WAIT;
        while Instant::now() < deadline {
            if self.ocr_child.try_lock().is_ok() {
                return;
            }
            std::thread::sleep(OCR_POLL_INTERVAL);
        }
    }

    /// The text of a PDF with no text layer (`reader_note` is the PDF
    /// reader's note): recognized when the recognizer is ready, otherwise
    /// the manual pick (with a note saying so while it is being prepared).
    /// One OCR child at a time: a second scanned PDF waits for the first
    /// (at most its time limit), then is recognized if the recognizer is
    /// still ready; reading it (and any other file) before that is not held
    /// up. Bytes recognition already ran on in this launch are never
    /// recognized again: their kept result, or a note saying so.
    pub(crate) fn scanned_pdf_extraction(&self, bytes: &[u8], reader_note: &str) -> Extraction {
        self.recognized_extraction(bytes, Input::ScannedPdf, reader_note)
    }

    /// The text of one PNG or JPEG image (#37; the student's decision of
    /// 2026-10-14), recognized on this Mac exactly like a scanned PDF's
    /// pages: the same child, limits, one-child slot and preparation, and
    /// the same bytes never recognized twice in a launch. The text is always
    /// partial, marked as recognized, with `IMAGE_RECOGNIZED_NOTE`. Anything
    /// else (recognition off or not ready, not a PNG or JPEG, too large,
    /// damaged, no text, a stop or a limit) is a manual pick with a note.
    pub(crate) fn image_extraction(&self, bytes: &[u8]) -> Extraction {
        if crate::pdf_ocr::image_kind(bytes).is_none() {
            return observe::not_extracted(ExtractionStatus::Unsupported, IMAGE_NOT_AN_IMAGE_NOTE);
        }
        self.recognized_extraction(bytes, Input::Image, IMAGE_NOT_READ_NOTE)
    }

    /// Recognizes `bytes` as `input`; `not_read_note` is the manual pick's
    /// note when recognition is off or no longer ready.
    fn recognized_extraction(&self, bytes: &[u8], input: Input, not_read_note: &str) -> Extraction {
        let not_read = || observe::not_extracted(ExtractionStatus::Unsupported, not_read_note);
        if self.ocr().helper.is_none() {
            return not_read();
        }
        let sha256 = observe::sha256_hex(bytes);
        {
            let ocr = self.ocr();
            if let Some(earlier) = ocr.earlier(&sha256, input) {
                return earlier.extraction();
            }
            match (&ocr.helper, ocr.state) {
                (Some(_), OcrState::Ready) => {}
                (Some(_), OcrState::Preparing) => {
                    return observe::not_extracted(
                        ExtractionStatus::Unsupported,
                        OCR_GETTING_READY_NOTE,
                    );
                }
                _ => return not_read(),
            }
        }
        let _one_child = self.ocr_child();
        // Looked at again after the wait: another reading may have
        // recognized the same bytes, or passed a limit meanwhile.
        if let Some(earlier) = self.ocr().earlier(&sha256, input) {
            return earlier.extraction();
        }
        let Some((helper, generation)) = self.ready_recognizer() else {
            return not_read();
        };
        let (recognized, revoke) = recognize(&helper, bytes, input, &self.ocr_stop);
        let mut ocr = self.ocr();
        // A recognizer set again meanwhile starts with nothing kept.
        if ocr.generation == generation {
            if revoke {
                // Ready held only until a recognition passed its time or
                // memory limit: no more recognition in this launch.
                ocr.state = OcrState::Failed;
            }
            ocr.keep(sha256, recognized.clone());
        }
        recognized.extraction()
    }

    /// One PNG or JPEG image (`Input::Image`) or a scanned PDF's first
    /// scanned pages (`Input::ScannedPdf`, at most 3) as pictures for the
    /// model (tier 2; the student's decision of 2026-10-14; the caller sends
    /// them only with the Settings opt-in): JPEGs drawn afresh on this Mac,
    /// at most 1568 pixels long, without any metadata, at most 1 MiB each,
    /// in page order. The same child as text recognition (`--output jpeg`),
    /// with the same limits and one-child slot, and only while it is Ready;
    /// a child stopped at its time or memory limit revokes Ready, as a
    /// recognition does. Nothing is kept: the same bytes asked again start
    /// the child again, and recognition's kept results are not touched.
    /// `Err` is a short reason for people.
    pub(crate) fn images_for_model(
        &self,
        bytes: &[u8],
        input: Input,
    ) -> Result<Vec<Vec<u8>>, String> {
        if input == Input::Image && crate::pdf_ocr::image_kind(bytes).is_none() {
            return Err(PICTURE_NOT_AN_IMAGE.to_string());
        }
        match self.ocr().state {
            OcrState::Ready => {}
            OcrState::Preparing => return Err(PICTURE_GETTING_READY.to_string()),
            _ => return Err(PICTURE_NOT_AVAILABLE.to_string()),
        }
        let _one_child = self.ocr_child();
        let Some((helper, generation)) = self.ready_recognizer() else {
            return Err(PICTURE_NOT_AVAILABLE.to_string());
        };
        let mut extra = match input {
            Input::ScannedPdf => Vec::new(),
            Input::Image => IMAGE_INPUT.to_vec(),
        };
        extra.extend(JPEG_OUTPUT);
        match run_recognizer(&helper, bytes, &extra, &self.ocr_stop, &PICTURE_STOPS) {
            Ok(out) => answered_pictures(&out, input),
            Err((note, revoke)) => {
                let mut ocr = self.ocr();
                // As for a recognition: Ready held only until a child
                // passed its time or memory limit.
                if revoke && ocr.generation == generation {
                    ocr.state = OcrState::Failed;
                }
                Err(note.to_string())
            }
        }
    }

    /// The recognizer and its generation, when it is Ready and the app is
    /// not quitting. Looked at again after waiting for the one-child slot:
    /// another reading may have passed a limit meanwhile.
    fn ready_recognizer(&self) -> Option<(OcrHelper, u64)> {
        let ocr = self.ocr();
        match (&ocr.helper, ocr.state) {
            (Some(helper), OcrState::Ready) if !self.ocr_stop.load(Ordering::SeqCst) => {
                Some((helper.clone(), ocr.generation))
            }
            _ => None,
        }
    }

    fn ocr(&self) -> MutexGuard<'_, Ocr> {
        self.ocr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ocr_child(&self) -> MutexGuard<'_, ()> {
        self.ocr_child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What the preparation child prints when it is ready.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    status: PreparedStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PreparedStatus {
    Ready,
}

/// Runs the preparation child: no input, `{"status":"ready"}` and exit 0
/// when prepared.
fn prepare(helper: &OcrHelper, stop: &AtomicBool) -> OcrState {
    let watch = helper.watch(helper.prepare_time_limit, stop);
    let ended = run_child(
        &helper.prepare.program,
        &helper.prepare.args,
        None,
        MAX_PREPARE_ANSWER_BYTES,
        &watch,
    );
    match ended {
        Ended::Exited { success: true, out }
            if out.len() <= MAX_PREPARE_ANSWER_BYTES
                && serde_json::from_slice::<Prepared>(&out)
                    .is_ok_and(|prepared| matches!(prepared.status, PreparedStatus::Ready)) =>
        {
            OcrState::Ready
        }
        Ended::TimedOut => OcrState::TimedOut,
        Ended::Stopped => OcrState::Cancelled,
        // Failed, a wrong answer, over its memory or unreadable, not
        // started, or lost track of.
        _ => OcrState::Failed,
    }
}

/// Runs the recognition child on the bytes. The flag is true when the child
/// was stopped at its time or memory limit, its memory could not be read,
/// or the parent lost track of it: then the recognizer is no longer taken
/// to be ready.
fn recognize(
    helper: &OcrHelper,
    bytes: &[u8],
    input: Input,
    stop: &AtomicBool,
) -> (Recognized, bool) {
    let extra: &[&str] = match input {
        Input::ScannedPdf => &[],
        Input::Image => &IMAGE_INPUT,
    };
    match run_recognizer(helper, bytes, extra, stop, &RECOGNITION_STOPS) {
        Ok(out) => match input {
            Input::ScannedPdf => (answered(&out), false),
            Input::Image => (answered_image(&out), false),
        },
        Err((note, revoke)) => (Recognized::failed(note), revoke),
    }
}

/// Runs the recognition child on the bytes, with `extra` arguments after
/// the app's own, under its time and memory limits: what it printed, or
/// the note (from `notes`) on why there is nothing, and whether that
/// revokes Ready: the child was stopped at its time or memory limit, its
/// memory could not be read, or the parent lost track of it.
fn run_recognizer(
    helper: &OcrHelper,
    bytes: &[u8],
    extra: &[&str],
    stop: &AtomicBool,
    notes: &StopNotes,
) -> Result<Vec<u8>, (&'static str, bool)> {
    let watch = helper.watch(helper.time_limit, stop);
    let mut args = helper.recognize.args.clone();
    args.extend(extra.iter().map(|arg| OsString::from(*arg)));
    let ended = run_child(
        &helper.recognize.program,
        &args,
        Some(bytes),
        MAX_ANSWER_BYTES,
        &watch,
    );
    match ended {
        Ended::Exited { success: true, out } => Ok(out),
        Ended::Exited { success: false, .. } => Err((notes.crashed, false)),
        Ended::NotStarted => Err((notes.not_started, false)),
        Ended::TimedOut => Err((notes.timed_out, true)),
        Ended::OverMemory => Err((notes.over_memory, true)),
        Ended::Unmeasured => Err((notes.unmeasured, true)),
        Ended::Lost => Err((notes.lost, true)),
        Ended::Stopped => Err((notes.stopped, false)),
    }
}

/// The pictures in the child's answer to `--output jpeg`: `partial`, no
/// note, and as its text one standard base64 JPEG per line, 1 to 3 (exactly
/// 1 for an image). Each must be a whole JPEG without any metadata
/// (`is_bare_jpeg`: it starts FF D8 FF, ends FF D9 and holds only what a
/// decoder needs) of at most 1 MiB. Otherwise why there are none: the
/// child's note, or that the answer cannot be used.
fn answered_pictures(out: &[u8], input: Input) -> Result<Vec<Vec<u8>>, String> {
    let unusable = || PICTURE_UNUSABLE.to_string();
    if out.len() > MAX_ANSWER_BYTES {
        return Err(unusable());
    }
    let answer = serde_json::from_slice::<Answer>(out).map_err(|_| unusable())?;
    let note = answer
        .note
        .map(|note| note.chars().take(MAX_NOTE_CHARS).collect::<String>());
    match (answer.status, note) {
        (AnswerStatus::Partial, None) => {}
        (AnswerStatus::Unsupported | AnswerStatus::Failed, note) => {
            return Err(note.unwrap_or_else(|| PICTURE_NOT_PREPARED.to_string()));
        }
        _ => return Err(unusable()),
    }
    let most = match input {
        Input::Image => 1,
        Input::ScannedPdf => MODEL_IMAGE_MAX_COUNT,
    };
    let lines: Vec<&str> = answer.text.split('\n').collect();
    if lines.len() > most {
        return Err(unusable());
    }
    lines
        .into_iter()
        .map(|line| {
            base64_decode(line)
                .filter(|jpeg| {
                    // The parent checks the size too (#37 review): never
                    // more than 1568 pixels on the longest side.
                    jpeg.len() <= MODEL_IMAGE_MAX_BYTES
                        && is_bare_jpeg(jpeg)
                        && crate::pdf_ocr::frame_size(jpeg).is_some_and(|(width, height)| {
                            width > 0
                                && height > 0
                                && usize::from(width.max(height)) <= MODEL_IMAGE_MAX_SIDE
                        })
                })
                .ok_or_else(unusable)
        })
        .collect()
}

/// What the recognition child's answer for an image gives: `partial` with
/// the recognized lines (at least `OCR_MIN_IMAGE_CHARS` non-white-space
/// characters, at most the shared text limit), or why there is no text.
/// The child's note on success is not used: the warning is Traeva's own.
fn answered_image(out: &[u8]) -> Recognized {
    let unusable = || Recognized::failed("Text recognition gave an answer Traeva cannot use.");
    if out.len() > MAX_ANSWER_BYTES {
        return Recognized::failed("Text recognition gave more text than Traeva reads.");
    }
    let Ok(answer) = serde_json::from_slice::<Answer>(out) else {
        return unusable();
    };
    let note = answer
        .note
        .map(|note| note.chars().take(MAX_NOTE_CHARS).collect::<String>());
    match answer.status {
        AnswerStatus::Partial => {
            let text = answer.text;
            if text.chars().count() > MAX_MEMORY_TEXT_CHARS {
                Recognized::failed("Text recognition gave more text than Traeva reads.")
            } else if text.chars().filter(|c| !c.is_whitespace()).count() < OCR_MIN_IMAGE_CHARS {
                unusable()
            } else {
                Recognized::ImageText { text }
            }
        }
        AnswerStatus::Complete => unusable(),
        AnswerStatus::Unsupported => Recognized::NoText {
            status: ExtractionStatus::Unsupported,
            note: note.unwrap_or_else(|| IMAGE_NO_TEXT_NOTE.to_string()),
        },
        AnswerStatus::Failed => Recognized::NoText {
            status: ExtractionStatus::Failed,
            note: note.unwrap_or_else(|| IMAGE_NO_TEXT_NOTE.to_string()),
        },
    }
}

/// What the recognition child's answer gives.
fn answered(out: &[u8]) -> Recognized {
    if out.len() > MAX_ANSWER_BYTES {
        return Recognized::failed("Text recognition gave more text than Traeva reads.");
    }
    let Ok(answer) = serde_json::from_slice::<Answer>(out) else {
        return Recognized::failed("Text recognition gave an answer Traeva cannot use.");
    };
    let note = answer
        .note
        .map(|note| note.chars().take(MAX_NOTE_CHARS).collect::<String>());
    match answer.status {
        // Recognized text is never complete (§14.8): `complete` is a wrong
        // answer.
        AnswerStatus::Partial => checked_text(answer.text, note),
        AnswerStatus::Complete => {
            Recognized::failed("Text recognition gave an answer Traeva cannot use.")
        }
        AnswerStatus::Unsupported => Recognized::NoText {
            status: ExtractionStatus::Unsupported,
            note: note.unwrap_or_else(|| {
                "Text recognition could not read this PDF's scanned pages.".to_string()
            }),
        },
        AnswerStatus::Failed => Recognized::NoText {
            status: ExtractionStatus::Failed,
            note: note.unwrap_or_else(|| {
                "Text recognition could not read this PDF's scanned pages.".to_string()
            }),
        },
    }
}

/// The recognized text, if it keeps to the answer's form and the shared
/// text limit, with the child's note.
fn checked_text(text: String, note: Option<String>) -> Recognized {
    if text.chars().count() > MAX_MEMORY_TEXT_CHARS {
        return Recognized::failed("Text recognition gave more text than Traeva reads.");
    }
    match page_sections(&text) {
        Some(pages) => Recognized::Text { text, pages, note },
        None => Recognized::failed("Text recognition gave an answer Traeva cannot use."),
    }
}

/// How many page sections the text has, when it keeps to the answer's
/// form: 1 to `OCR_MAX_PAGES` sections, each a `[Page N]` line (N a whole
/// number from 1, without leading zeros, larger than the one before) and
/// then at least one line of recognized text. None otherwise: text before
/// the first page line, a page line written wrong, a page with no text,
/// page 0, a page twice or out of order, more pages.
fn page_sections(text: &str) -> Option<usize> {
    let mut pages = 0;
    let mut last_page = 0;
    let mut page_has_text = false;
    for line in text.lines() {
        if line.starts_with("[Page ") {
            let number = page_number(line)?;
            if number <= last_page || (pages > 0 && !page_has_text) {
                return None;
            }
            pages += 1;
            last_page = number;
            page_has_text = false;
        } else if pages == 0 {
            return None;
        } else if !line.trim().is_empty() {
            page_has_text = true;
        }
    }
    ((1..=OCR_MAX_PAGES).contains(&pages) && page_has_text).then_some(pages)
}

/// N of a "[Page N]" line.
fn page_number(line: &str) -> Option<u32> {
    let digits = line.strip_prefix("[Page ")?.strip_suffix(']')?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
#[path = "ocr_tests.rs"]
mod tests;
