//! Opt-in: text recognition from synthetic PNG and JPEG images with the real
//! child and Apple's Vision (#37; the student's decision of 2026-10-14). The
//! child is the core's `traeva-ocr` test executable, started with the
//! arguments the app's parent gives it (`--traeva-pdf-ocr`, the resource
//! flags and `--input image`), the image bytes on stdin. Ignored by default:
//! the first Vision use on a Mac can take about half a minute. Run it with:
//!
//! ```sh
//! cargo test -p traeva-core --test ocr_image -- --ignored --nocapture
//! ```
//!
//! The images are drawn by experiments/image-ocr/make_images.py; text-meta.jpg
//! carries made-up EXIF tags and GPS 0,0, which must never come out.
//! Nothing is sent to a model.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Deserialize;

const OCR_CHILD: &str = env!("CARGO_BIN_EXE_traeva-ocr");

/// The made-up EXIF values of text-meta.jpg (make_images.py).
const FAKE_METADATA: [&str; 9] = [
    "FAKE-CAMERA-MAKE",
    "FAKE-CAMERA-MODEL",
    "FAKE-ARTIST-NAME",
    "FAKE-IMAGE-DESCRIPTION",
    "FAKE-COPYRIGHT-HOLDER",
    "FAKE-SOFTWARE-NAME",
    "FAKE-GPS-AREA",
    "FAKE",
    "GPS",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    status: String,
    text: String,
    note: Option<String>,
}

fn recognize(name: &str) -> Answer {
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/images")
            .join(name),
    )
    .unwrap();
    let started = Instant::now();
    let mut child = Command::new(OCR_CHILD)
        .args([
            traeva_core::OCR_FLAG,
            "--parent-pid",
            &std::process::id().to_string(),
            "--max-pages",
            "3",
            "--dpi",
            "200",
            "--max-pixels",
            "8000000",
            "--input",
            "image",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{name}: exit {:?}", out.status);
    let answer: Answer = serde_json::from_slice(&out.stdout).unwrap();
    eprintln!(
        "{name}: {} in {} ms, {} characters",
        answer.status,
        started.elapsed().as_millis(),
        answer.text.chars().count()
    );
    answer
}

/// Lower case, runs of white space as one space.
fn plain(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn assert_reads(name: &str, words: &[&str]) -> Answer {
    let answer = recognize(name);
    assert_eq!(answer.status, "partial", "{name}: {answer:?}");
    let text = plain(&answer.text);
    for expected in words {
        assert!(
            text.contains(expected),
            "{name}: {expected:?} not in {text:?}"
        );
    }
    assert!(!answer.text.contains("[Page"), "{name}: no page markers");
    answer
}

#[test]
#[ignore = "runs Apple's Vision on this Mac; see the module docs"]
fn synthetic_images_are_read_and_their_metadata_never_is() {
    assert_reads(
        "text-plain.png",
        &["invoice number 4471", "total due 128.50"],
    );
    assert_reads(
        "text-exif6.jpg",
        &["rotated text sample", "orientation six"],
    );
    assert_reads("text-alpha.png", &["transparent background"]);

    let meta = assert_reads(
        "text-meta.jpg",
        &["library card renewal", "due back on tuesday"],
    );
    let note = meta.note.clone().unwrap_or_default();
    for value in FAKE_METADATA {
        assert!(!meta.text.contains(value), "the text holds {value}");
        assert!(!note.contains(value), "the note holds {value}");
    }

    let white = recognize("white-12mp.png");
    assert_eq!(white.status, "unsupported");
    assert!(white.text.is_empty());
    assert!(white.note.unwrap().contains("No usable OCR text"));

    for damaged in ["truncated.png", "truncated.jpg"] {
        let answer = recognize(damaged);
        assert_eq!(answer.status, "failed", "{damaged}");
        assert!(answer.text.is_empty());
        assert!(answer.note.unwrap().contains("cut off"));
    }
}
