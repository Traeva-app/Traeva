//! Opt-in: on-device text recognition through a built Traeva.app (#186,
//! organize-spec.md §14.8), for the app's own verification. The core's real
//! parent logic (preparation, the one-child slot, the time and memory
//! budget, the last check before reaping, the answer's form) starts the
//! app's executable as its PDF child and its OCR children, exactly as the
//! app does; no window is opened. Ignored by default; run it with:
//!
//! ```sh
//! TRAEVA_OCR_APP=/path/to/Traeva.app/Contents/MacOS/traeva-app \
//! TRAEVA_OCR_SAMPLES=/path/to/synthetic/samples \
//! cargo test -p traeva-core --test ocr_app -- --ignored --nocapture
//! ```
//!
//! `TRAEVA_OCR_SAMPLES` is optional: a folder of G-OCR's synthetic samples
//! with their `truth.json` (`experiments/pdf-ocr/samples` on the branch of
//! #215, e.g. `git archive f73a06f experiments/pdf-ocr/samples`). The
//! core's own fixtures always run; without the samples no scan with text
//! is recognized (p08's images hold no text). Synthetic files and temporary folders
//! only; nothing is sent to a model.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use traeva_core::{
    ExtractionStatus, OcrHelper, OcrState, OperationStage, OrganizeService, PdfHelper,
    ProcessMemory, child_footprint,
};

/// The highest memory the probe read for the current file, and every child
/// it looked at.
static PEAK: AtomicU64 = AtomicU64::new(0);
static SEEN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// The app's own monitor, noting what it read.
fn recording(pid: u32) -> Option<ProcessMemory> {
    let memory = child_footprint(pid);
    {
        let mut seen = SEEN.lock().unwrap();
        if !seen.contains(&pid) {
            seen.push(pid);
        }
    }
    if let Some(memory) = memory {
        PEAK.fetch_max(
            memory.footprint_bytes.max(memory.lifetime_peak_bytes),
            Ordering::SeqCst,
        );
    }
    memory
}

fn children_seen() -> usize {
    SEEN.lock().unwrap().len()
}

/// What a sample should give.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// A scan: recognized.
    Scan,
    /// A text layer, or a text layer on some pages: read from it, no OCR.
    TextLayer,
    /// Damaged: a manual pick, no OCR.
    Damaged,
    /// No text in the images, or not known: only reported.
    Other,
}

fn samples() -> Vec<(String, PathBuf, Kind)> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf");
    let mut list = vec![
        (
            "p08-scanned-images.pdf".to_string(),
            fixtures.join("p08-scanned-images.pdf"),
            Kind::Other,
        ),
        (
            "p02-multi-en.pdf".to_string(),
            fixtures.join("p02-multi-en.pdf"),
            Kind::TextLayer,
        ),
    ];
    let Some(folder) = std::env::var_os("TRAEVA_OCR_SAMPLES").map(PathBuf::from) else {
        return list;
    };
    let truth: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("truth.json")).unwrap()).unwrap();
    let mut names: Vec<&String> = truth["samples"].as_object().unwrap().keys().collect();
    names.sort();
    for name in names {
        let path = folder.join(name);
        if !path.exists() {
            continue;
        }
        let kind = match truth["samples"][name]["kind"].as_str() {
            Some("scan") => Kind::Scan,
            Some("text_layer") | Some("mixed") => Kind::TextLayer,
            Some("damaged") => Kind::Damaged,
            _ => Kind::Other,
        };
        list.push((name.clone(), path, kind));
    }
    list
}

/// The recognition child's own answer for these bytes: its text's
/// `[Page N]` sections (checked again here, apart from the parent) and its
/// note.
fn child_answer(app: &Path, bytes: &[u8]) -> (Vec<u32>, Option<String>) {
    use std::io::Write;
    let me = std::process::id().to_string();
    let mut child = std::process::Command::new(app)
        .args([
            traeva_core::OCR_FLAG,
            "--max-pages",
            "3",
            "--dpi",
            "200",
            "--max-pixels",
            "8000000",
            "--parent-pid",
            &me,
        ])
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let answer: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(answer["status"], "partial", "{answer}");
    let text = answer["text"].as_str().unwrap();
    let mut pages = Vec::new();
    let mut has_text = true;
    for line in text.lines() {
        if let Some(number) = line
            .strip_prefix("[Page ")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            assert!(has_text, "a page without text: {text:?}");
            pages.push(number.parse::<u32>().unwrap());
            has_text = false;
        } else {
            assert!(!pages.is_empty(), "text before the first page line");
            has_text |= !line.trim().is_empty();
        }
    }
    assert!(has_text, "the last page has no text");
    (pages, answer["note"].as_str().map(str::to_string))
}

/// The process is gone and reaped.
fn reaped(pid: u32) -> bool {
    // SAFETY: signal 0 sends nothing; it only checks that the process exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[test]
#[ignore = "needs a built Traeva.app: set TRAEVA_OCR_APP to its executable"]
fn a_built_app_recognizes_scanned_pdfs_through_the_core_s_parent() {
    let app = PathBuf::from(
        std::env::var_os("TRAEVA_OCR_APP").expect("TRAEVA_OCR_APP: the .app's executable"),
    );
    assert!(app.is_file(), "{}", app.display());
    let app_data = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let root = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let service = OrganizeService::start(common::app_data_dir(&app_data));
    let root_id = service.authorize_root(root.path()).unwrap().root_id;
    service.set_pdf_helper(Some(PdfHelper::traeva(app.clone())));
    let (recognize, prepare) = {
        let app_like = OcrHelper::traeva(app.clone());
        let (recognize, prepare) = app_like.arguments();
        (recognize.to_vec(), prepare.to_vec())
    };
    service.set_ocr_helper(Some(OcrHelper::commands(
        (app.clone(), recognize),
        (app.clone(), prepare),
        recording,
    )));

    PEAK.store(0, Ordering::SeqCst);
    let started = Instant::now();
    let state = service.start_ocr_preparation();
    println!(
        "preparation: {state:?} in {} ms, peak {:.1} MiB",
        started.elapsed().as_millis(),
        PEAK.load(Ordering::SeqCst) as f64 / 1048576.0
    );
    assert_eq!(state, OcrState::Ready);

    // A damaged copy of the scan: the first half of its bytes.
    let p08 = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/p08-scanned-images.pdf"),
    )
    .unwrap();
    let mut files = samples();
    let damaged = root.path().join("half-p08.pdf");
    fs::write(&damaged, &p08[..p08.len() / 2]).unwrap();
    files.push(("half-p08.pdf".to_string(), damaged, Kind::Damaged));

    for (name, source, kind) in files {
        let bytes = fs::read(&source).unwrap();
        let target = root.path().join(&name);
        if source != target {
            fs::write(&target, &bytes).unwrap();
        }
        let children_before = children_seen();
        PEAK.store(0, Ordering::SeqCst);
        let started = Instant::now();
        let operation = service.start_operation(&root_id, &root_id).unwrap();
        service.run_operation(&operation.operation_id, &target);
        let view = service.get_operation(&operation.operation_id).unwrap();
        let elapsed = started.elapsed();
        let extraction = view.file.as_ref().expect("read").extraction.clone();
        let ocr_children = children_seen() - children_before;
        println!(
            "{name}: {:?} {:?} recognized={} ocr_children={ocr_children} {} ms peak {:.1} MiB note={:?}",
            view.stage,
            extraction.status,
            extraction.recognized,
            elapsed.as_millis(),
            PEAK.load(Ordering::SeqCst) as f64 / 1048576.0,
            extraction.note
        );
        match kind {
            Kind::Scan => {
                assert_eq!(view.stage, OperationStage::ReadyForAnalysis, "{name}");
                assert_eq!(extraction.status, ExtractionStatus::Partial, "{name}");
                assert!(extraction.recognized, "{name}");
                let note = extraction.note.clone().unwrap();
                assert!(
                    note.starts_with("Read by on-device text recognition from "),
                    "{name}: {note}"
                );
                let (sections, child_note) = child_answer(&app, &bytes);
                assert!((1..=3).contains(&sections.len()), "{name}: {sections:?}");
                assert!(sections.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(sections[0] >= 1);
                println!("  sections {sections:?}");
                // R4-OCR-05: what the child said about pages it left out
                // and the reader's other omissions reaches the extraction.
                let child_note = child_note.unwrap_or_default();
                let warning = format!(
                    "OCR read {} scanned pages; may contain errors.",
                    sections.len()
                );
                let rest = child_note
                    .strip_prefix(warning.as_str())
                    .unwrap_or(&child_note);
                assert!(
                    note.ends_with(rest.trim()),
                    "{name}: {note:?} / {child_note:?}"
                );
                if child_note.contains("Omitted pages:") {
                    assert!(note.contains("Omitted pages:"), "{name}: {note}");
                }
            }
            Kind::TextLayer => {
                assert!(!extraction.recognized, "{name}");
                assert_eq!(ocr_children, 0, "{name}: no OCR child");
                assert!(extraction.usable_text, "{name}");
            }
            Kind::Damaged => {
                assert_eq!(view.stage, OperationStage::NeedsManualLocation, "{name}");
                assert!(!extraction.recognized, "{name}");
                assert_eq!(ocr_children, 0, "{name}: no OCR child");
            }
            Kind::Other => {}
        }
        assert!(elapsed < Duration::from_secs(40), "{name}");
    }
    assert_eq!(
        service.ocr_state(),
        OcrState::Ready,
        "no recognition passed a limit"
    );

    // Every child the probe looked at is gone and reaped.
    let seen = SEEN.lock().unwrap().clone();
    println!("children looked at: {}", seen.len());
    for pid in seen {
        assert!(reaped(pid), "child {pid} is left over");
    }
}
