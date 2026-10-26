//! Reading a PDF's text layer for Organize in a separate process
//! (organize-spec.md §14, v0.5, #186).
//!
//! The PDF reader has its own limits, but its resource safety is not
//! verified (R4), so it never runs in the app's process. The core hands the
//! child the exact bytes it read and hashed, through a pipe; the child gets
//! no path, writes no file and returns only the text and its status as one
//! JSON object. The child is Traeva's own executable started with
//! `PDF_TEXT_FLAG`: its allocator stops at the heap cap it is given, and
//! the parent kills it after the time limit. Too much memory, too much
//! time, a crash or an answer that does not parse: the file has no usable
//! text, so it goes to a manual pick and nothing is sent. Nothing retries.
//!
//! A PDF with no text layer at all can then be read by on-device text
//! recognition, in another child (ocr.rs, §14.8); `run_child` here starts
//! and watches both.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::contracts::ExtractionStatus;
use crate::memory_contracts::TextStatus;
use crate::observe::{self, Extraction, MAX_FILE_BYTES};
use crate::ocr::{self, MemoryProbe, OcrUse};
use crate::reader::MAX_MEMORY_TEXT_CHARS;

/// The first argument that makes Traeva's executable read one PDF from
/// stdin and print its text, instead of opening the window.
pub const PDF_TEXT_FLAG: &str = "--traeva-pdf-text";
/// The child's heap cap (the same as in R4's runs of the reader).
pub const PDF_HEAP_CAP: usize = 512 << 20;
/// How long the child may take before it is killed.
pub const PDF_TIME_LIMIT: Duration = Duration::from_secs(15);
/// Longest answer the parent reads: the longest text, JSON-escaped.
pub(crate) const MAX_ANSWER_BYTES: usize = 8 * MAX_MEMORY_TEXT_CHARS;
/// Longest note from the child that is shown.
pub(crate) const MAX_NOTE_CHARS: usize = 300;
/// How often the parent looks at the PDF child.
const PDF_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How Organize reads PDF files: by starting this program with these
/// arguments, the bytes on stdin and the answer on stdout.
#[derive(Debug, Clone)]
pub struct PdfHelper {
    kind: HelperKind,
    time_limit: Duration,
}

#[derive(Debug, Clone)]
enum HelperKind {
    Process {
        program: PathBuf,
        args: Vec<OsString>,
    },
    /// Tests: the reader's answer without a process.
    #[cfg(test)]
    InProcess(fn(&[u8]) -> crate::memory_contracts::TextExtraction),
}

impl PdfHelper {
    /// Traeva's own executable, started with `PDF_TEXT_FLAG` and the heap cap.
    pub fn traeva(executable: PathBuf) -> PdfHelper {
        PdfHelper::command(
            executable,
            vec![PDF_TEXT_FLAG.into(), PDF_HEAP_CAP.to_string().into()],
        )
    }

    /// A program that behaves like Traeva's executable after the PDF flag
    /// (the core's tests: `traeva-pdf-text`), with the heap cap.
    pub fn traeva_with(program: impl Into<PathBuf>) -> PdfHelper {
        PdfHelper::command(program.into(), vec![PDF_HEAP_CAP.to_string().into()])
    }

    /// Any program and arguments (the process tests use system tools).
    pub fn command(program: PathBuf, args: Vec<OsString>) -> PdfHelper {
        PdfHelper {
            kind: HelperKind::Process { program, args },
            time_limit: PDF_TIME_LIMIT,
        }
    }

    pub fn with_time_limit(mut self, time_limit: Duration) -> PdfHelper {
        self.time_limit = time_limit;
        self
    }

    #[cfg(test)]
    pub(crate) fn in_process(
        read: fn(&[u8]) -> crate::memory_contracts::TextExtraction,
    ) -> PdfHelper {
        PdfHelper {
            kind: HelperKind::InProcess(read),
            time_limit: PDF_TIME_LIMIT,
        }
    }
}

/// What the child prints: the reader's status, its text and why it read
/// less. Never the file's metadata. The text recognition child (ocr.rs)
/// answers in the same shape.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Answer {
    pub status: AnswerStatus,
    pub text: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnswerStatus {
    Complete,
    Partial,
    Unsupported,
    Failed,
}

impl From<TextStatus> for AnswerStatus {
    fn from(status: TextStatus) -> AnswerStatus {
        match status {
            TextStatus::Complete => AnswerStatus::Complete,
            TextStatus::Partial => AnswerStatus::Partial,
            TextStatus::Unsupported => AnswerStatus::Unsupported,
            TextStatus::Failed => AnswerStatus::Failed,
        }
    }
}

/// The child's side: reads at most `MAX_FILE_BYTES` from stdin, reads the
/// PDF's text layer and prints the answer. `args` are the ones after
/// `PDF_TEXT_FLAG`: the heap cap in bytes. Returns the exit code.
pub fn pdf_child_main(args: &[String]) -> i32 {
    let cap = args
        .first()
        .and_then(|cap| cap.parse::<usize>().ok())
        .unwrap_or(PDF_HEAP_CAP);
    set_heap_cap(cap);
    let mut bytes = Vec::new();
    let limit = MAX_FILE_BYTES + 1;
    if std::io::stdin()
        .take(limit)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 >= limit
    {
        return 2;
    }
    let read = crate::pdf::read_pdf(&bytes);
    drop(bytes);
    let answer = Answer {
        status: read.summary.status.into(),
        text: read.text,
        note: read.summary.note,
    };
    let mut out = std::io::stdout().lock();
    let written = serde_json::to_writer(&mut out, &answer).is_ok() && out.flush().is_ok();
    if written { 0 } else { 3 }
}

impl OrganizeService {
    /// Tells Organize how to read PDF files (the app: its own executable).
    /// Without it, a PDF has no text Traeva can read.
    pub fn set_pdf_helper(&self, helper: Option<PdfHelper>) {
        *self
            .pdf_helper
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = helper;
    }

    /// The text Organize reads from a file's bytes: a PDF's text layer in
    /// the child process; a Word, PowerPoint or Excel file with Memory's
    /// readers (`office_extraction`); any other file as §4.1 says. Memory never reads
    /// its text here (`reader::read_text` leaves PDF out). On-device text
    /// recognition runs only when `ocr` allows it (Organize's own reading of
    /// a chosen or dropped file and its reread before sending): for a PDF
    /// with no text layer at all (organize-spec.md §14.8) and for a JPEG or
    /// PNG image (§14.9). Memory's and Source's registration (`Never`) read
    /// an image like any other unsupported file: no text.
    pub(crate) fn organize_extraction(
        &self,
        file_name: &str,
        bytes: &[u8],
        ocr: OcrUse,
    ) -> Extraction {
        if observe::is_pdf_file_name(file_name) {
            self.pdf_extraction(bytes, ocr)
        } else if ocr == OcrUse::Allowed && observe::is_image_file_name(file_name) {
            let mut extraction = self.image_extraction(bytes);
            extraction.summary.image_input = crate::pdf_ocr::image_kind(bytes).is_some();
            extraction
        } else if observe::is_office_file_name(file_name) {
            office_extraction(file_name, bytes)
        } else {
            observe::extract_text(file_name, bytes)
        }
    }

    fn pdf_extraction(&self, bytes: &[u8], ocr: OcrUse) -> Extraction {
        let helper = self
            .pdf_helper
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(helper) = helper else {
            return observe::not_extracted(
                ExtractionStatus::Unsupported,
                "Traeva cannot read PDF files here.",
            );
        };
        let answer = match ask(&helper, bytes) {
            Ok(answer) => answer,
            Err(note) => return observe::not_extracted(ExtractionStatus::Failed, note),
        };
        let note = answer
            .note
            .map(|note| note.chars().take(MAX_NOTE_CHARS).collect::<String>());
        match answer.status {
            AnswerStatus::Complete | AnswerStatus::Partial => {
                if answer.text.chars().count() > MAX_MEMORY_TEXT_CHARS {
                    return observe::not_extracted(
                        ExtractionStatus::Failed,
                        "The PDF reader gave more text than Traeva reads.",
                    );
                }
                let mut extraction = observe::excerpt_text(answer.text);
                if answer.status == AnswerStatus::Partial {
                    // The reader left part of the file out (images, say):
                    // the text stays partial even when it is short enough
                    // to be used whole (R2-PDF-01, #196; organize-spec §14.2).
                    extraction.summary.status = ExtractionStatus::Partial;
                    extraction.summary.note = note.or_else(|| {
                        Some("Traeva read only part of this PDF's text layer.".to_string())
                    });
                }
                extraction
            }
            AnswerStatus::Unsupported => {
                let note = note
                    .as_deref()
                    .unwrap_or("This PDF has no text layer Traeva can read.");
                if ocr == OcrUse::Allowed && ocr::has_no_text_layer(note) {
                    // A scanned PDF: its first pages may also go to the
                    // model as pictures (§14.10).
                    let mut extraction = self.scanned_pdf_extraction(bytes, note);
                    extraction.summary.image_input = true;
                    extraction
                } else {
                    observe::not_extracted(ExtractionStatus::Unsupported, note)
                }
            }
            AnswerStatus::Failed => observe::not_extracted(
                ExtractionStatus::Failed,
                note.as_deref().unwrap_or("Traeva could not read this PDF."),
            ),
        }
    }
}

impl OrganizeService {
    /// A PDF's whole text layer for Find (#198): the same child process,
    /// with Find's shorter time limit. None when there is no usable text.
    pub(crate) fn pdf_find_text(&self, bytes: &[u8], time_limit: Duration) -> Option<String> {
        let helper = self
            .pdf_helper
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()?;
        let answer = ask(&helper.with_time_limit(time_limit), bytes).ok()?;
        match answer.status {
            AnswerStatus::Complete | AnswerStatus::Partial
                if answer.text.chars().count() <= MAX_MEMORY_TEXT_CHARS =>
            {
                Some(answer.text)
            }
            _ => None,
        }
    }
}

/// Runs the helper on the bytes; `Err` is the reason for people.
fn ask(helper: &PdfHelper, bytes: &[u8]) -> Result<Answer, &'static str> {
    let (program, args) = match &helper.kind {
        HelperKind::Process { program, args } => (program, args),
        #[cfg(test)]
        HelperKind::InProcess(read) => {
            let read = read(bytes);
            return Ok(Answer {
                status: read.summary.status.into(),
                text: read.text,
                note: read.summary.note,
            });
        }
    };
    let watch = Watch {
        time_limit: helper.time_limit,
        poll: PDF_POLL_INTERVAL,
        memory: None,
        stop: None,
    };
    let out = match run_child(program, args, Some(bytes), MAX_ANSWER_BYTES, &watch) {
        Ended::Exited { success: true, out } => out,
        Ended::Exited { success: false, .. } => {
            return Err("Traeva could not read this PDF within its limits.");
        }
        Ended::NotStarted => return Err("Traeva could not start its PDF reader."),
        Ended::Lost => return Err("Traeva could not read this PDF within its limits."),
        // No memory probe and no stop flag here: only the time limit.
        Ended::TimedOut | Ended::OverMemory | Ended::Unmeasured | Ended::Stopped => {
            return Err("Reading this PDF took too long, so Traeva stopped.");
        }
    };
    if out.len() > MAX_ANSWER_BYTES {
        return Err("The PDF reader gave more text than Traeva reads.");
    }
    serde_json::from_slice(&out).map_err(|_| "The PDF reader gave an answer Traeva cannot use.")
}

/// How the parent watches a child while it runs.
pub(crate) struct Watch<'a> {
    /// Killed when it runs longer.
    pub time_limit: Duration,
    /// The pause between two looks.
    pub poll: Duration,
    /// A probe that reads the child's memory by its process ID, and the
    /// most it may use (`ProcessMemory::counted`): killed above it, and read
    /// once more after it ended, before it is reaped. The check is only as
    /// good as the polling: between two looks the child can go over the
    /// limit for a moment before it is stopped.
    pub memory: Option<(MemoryProbe, u64)>,
    /// Set from another thread to stop the child (the app is quitting).
    pub stop: Option<&'a AtomicBool>,
}

/// How a child run ended.
#[derive(Debug)]
pub(crate) enum Ended {
    /// It exited by itself: whether it succeeded, and what it printed (at
    /// most `max_out` + 1 bytes, so the caller can tell it was too long).
    Exited {
        success: bool,
        out: Vec<u8>,
    },
    NotStarted,
    /// Killed after the time limit.
    TimedOut,
    /// Killed above its memory limit.
    OverMemory,
    /// Killed because its memory could not be read.
    Unmeasured,
    /// Killed because `stop` was set.
    Stopped,
    /// Whether it still runs could not be told: killed.
    Lost,
}

/// Starts `program` with `args` and an empty environment, gives it `input`
/// on stdin (or nothing at all), reads its stdout and watches it as `watch`
/// says. Writing and reading happen on their own threads: a child that
/// stops reading or never answers cannot block the watching. stderr is
/// dropped.
///
/// Only a child that really ended counts as exited (`ended`): a paused one
/// (SIGSTOP) still meets the time limit, the stop and the memory check. A
/// child that ended by itself is not reaped at once: its memory is read
/// one last time, and the stop and the time limit are checked again; any
/// of them failing drops its answer. Every child is reaped (`wait`) before
/// this returns.
pub(crate) fn run_child(
    program: &Path,
    args: &[OsString],
    input: Option<&[u8]>,
    max_out: usize,
    watch: &Watch,
) -> Ended {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .env_clear()
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Ended::NotStarted;
    };
    let writer = match (input, child.stdin.take()) {
        (None, _) => None,
        (Some(bytes), Some(mut stdin)) => {
            let input = bytes.to_vec();
            Some(std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            }))
        }
        (Some(_), None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Ended::NotStarted;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Ended::NotStarted;
    };
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(max_out as u64 + 1).read_to_end(&mut out);
        out
    });
    let pid = child.id();
    let stopped = || watch.stop.is_some_and(|stop| stop.load(Ordering::SeqCst));
    let deadline = Instant::now() + watch.time_limit;
    // While it runs: None once it ended by itself, or why it is killed.
    let mut reason = loop {
        match ended(pid) {
            Some(true) => break None,
            Some(false) => {}
            None => break Some(Ended::Lost),
        }
        if stopped() {
            break Some(Ended::Stopped);
        }
        if let Some((probe, limit)) = watch.memory {
            match probe(pid) {
                Some(memory) if memory.counted() > limit => break Some(Ended::OverMemory),
                Some(_) => {}
                None => break Some(Ended::Unmeasured),
            }
        }
        if Instant::now() >= deadline {
            break Some(Ended::TimedOut);
        }
        std::thread::sleep(watch.poll);
    };
    // It ended by itself and is not reaped yet: the last checks.
    if reason.is_none() {
        if let Some((probe, limit)) = watch.memory {
            match probe(pid) {
                Some(memory) if memory.counted() > limit => reason = Some(Ended::OverMemory),
                Some(_) => {}
                None => reason = Some(Ended::Unmeasured),
            }
        }
        if reason.is_none() && stopped() {
            reason = Some(Ended::Stopped);
        }
        if reason.is_none() && Instant::now() > deadline {
            reason = Some(Ended::TimedOut);
        }
    } else {
        let _ = child.kill();
    }
    let status = child.wait();
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let out = reader.join().unwrap_or_default();
    match (reason, status) {
        (Some(reason), _) => reason,
        (None, Ok(status)) => Ended::Exited {
            success: status.success(),
            out,
        },
        (None, Err(_)) => Ended::Lost,
    }
}

/// Whether the child really ended (it exited, was killed or dumped core),
/// without reaping it, so its memory can still be read. A paused or
/// resumed child has not ended: macOS can report one here as well
/// (`CLD_STOPPED`, `CLD_CONTINUED`; R4 on the OCR experiment, #211), so
/// only the three codes for an ended child count. None: it cannot be told.
pub(crate) fn ended(pid: u32) -> Option<bool> {
    #[cfg(test)]
    if let Some(errno) = waitid_fault() {
        return after_waitid_error(std::io::Error::from_raw_os_error(errno));
    }
    // SAFETY: siginfo_t is plain data, and all zeros is a valid value.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a siginfo_t this call fills in; WNOWAIT leaves the
    // child as it is, to be reaped by `Child::wait`.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return after_waitid_error(std::io::Error::last_os_error());
    }
    Some(
        info.si_pid == pid as libc::pid_t
            && matches!(
                info.si_code,
                libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED
            ),
    )
}

/// What a failed `waitid` means. Interrupted by a signal (EINTR): the child
/// may still run, so it is looked at again at the next poll, still under
/// the time limit, the stop and the memory check. Any other error: whether
/// it still runs cannot be told, so `run_child` kills it, reaps it and
/// reports `Ended::Lost` (for OCR, Ready is revoked).
fn after_waitid_error(error: std::io::Error) -> Option<bool> {
    (error.kind() == std::io::ErrorKind::Interrupted).then_some(false)
}

#[cfg(test)]
thread_local! {
    /// Tests only: the next `count` looks at a child on this thread fail
    /// with `errno` instead of calling `waitid` (`fail_waitid`).
    static WAITID_FAULT: std::cell::Cell<(i32, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Tests only: makes the next `count` calls of `ended` on this thread fail
/// with `errno` (usize::MAX: every call), so the error paths above can be
/// run; a real `waitid` with WNOHANG almost never fails.
#[cfg(test)]
pub(crate) fn fail_waitid(errno: i32, count: usize) {
    WAITID_FAULT.set((errno, count));
}

#[cfg(test)]
fn waitid_fault() -> Option<i32> {
    let (errno, count) = WAITID_FAULT.get();
    if count == 0 {
        return None;
    }
    WAITID_FAULT.set((errno, count.saturating_sub(1)));
    Some(errno)
}

// The child's heap cap. The app installs `CappedAlloc` as its global
// allocator (src-tauri/src/main.rs); it counts every allocation and refuses
// one that would pass the cap, which aborts the process. The app itself
// never sets a cap, so for it this only counts.

static HEAP_CAP: AtomicUsize = AtomicUsize::new(usize::MAX);
static HEAP_USED: AtomicUsize = AtomicUsize::new(0);

/// Also for the OCR child's entry (#186 OCR): one allocator, one cap.
pub(crate) fn set_heap_cap(bytes: usize) {
    HEAP_CAP.store(bytes, Ordering::SeqCst);
}

/// Takes `size` more bytes if the cap allows it.
fn reserve(size: usize) -> bool {
    let cap = HEAP_CAP.load(Ordering::Relaxed);
    HEAP_USED
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(size).filter(|&total| total <= cap)
        })
        .is_ok()
}

fn release(size: usize) {
    HEAP_USED.fetch_sub(size, Ordering::Relaxed);
}

/// The system allocator with a heap cap (see above).
pub struct CappedAlloc;

// SAFETY: every call forwards to the system allocator with the same
// pointer and layout; the cap only refuses (returns null) before asking it.
unsafe impl GlobalAlloc for CappedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's layout is passed on unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            release(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: as in `alloc`.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if pointer.is_null() {
            release(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from this allocator with this layout.
        unsafe { System.dealloc(pointer, layout) };
        release(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        if new_size > old && !reserve(new_size - old) {
            return std::ptr::null_mut();
        }
        // SAFETY: as in `dealloc`; `new_size` is the caller's.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if moved.is_null() {
            if new_size > old {
                release(new_size - old);
            }
        } else if new_size < old {
            release(old - new_size);
        }
        moved
    }
}

/// A Word, PowerPoint or Excel file's text for Organize (the student's
/// decision 10/17; organize-spec v0.16): the readers Memory and Find use, in
/// this process, with their rules for what they leave out (contract §4.4),
/// then the same excerpt as a text file (§4.1). An Excel sheet's name is not
/// sent, as in Memory (`reader::without_sheet_names`). What a reader left
/// out is said in the note, so the preview shows the text is partial.
fn office_extraction(file_name: &str, bytes: &[u8]) -> Extraction {
    let read = crate::reader::read_text(file_name, bytes);
    let status = match read.summary.status {
        TextStatus::Complete => ExtractionStatus::Ok,
        TextStatus::Partial => ExtractionStatus::Partial,
        TextStatus::Unsupported => ExtractionStatus::Unsupported,
        TextStatus::Failed => ExtractionStatus::Failed,
    };
    if !matches!(status, ExtractionStatus::Ok | ExtractionStatus::Partial) {
        let note = read
            .summary
            .note
            .unwrap_or_else(|| "Traeva could not read this file's text.".to_string());
        return observe::not_extracted(status, &note);
    }
    let text = crate::reader::without_sheet_names(&read.summary.reader, &read.text);
    let mut extraction = observe::excerpt_text(text);
    if status == ExtractionStatus::Partial {
        extraction.summary.status = ExtractionStatus::Partial;
        // The readers' own notes point at their `left_out` list; people
        // see the list itself.
        extraction.summary.note = Some(if read.summary.left_out.is_empty() {
            "Traeva read only part of this file's text.".to_string()
        } else {
            format!(
                "Traeva read only part of this file's text. Left out: {}.",
                read.summary.left_out.join(", ")
            )
        });
    }
    extraction
}

#[cfg(test)]
#[path = "pdf_child_tests.rs"]
mod tests;
