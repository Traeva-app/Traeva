//! Opt-in: pictures for the model (tier 2; the student's decision of
//! 2026-10-14) from the real child, the core's `traeva-ocr` test
//! executable, started with the arguments the app's parent gives it
//! (`--traeva-pdf-ocr`, the resource flags, `--input image` for an image
//! and `--output jpeg`), the file's bytes on stdin. Synthetic images
//! (experiments/image-ocr/make_images.py; text-meta.jpg carries made-up
//! EXIF tags and GPS 0,0) and the scanned PDF p08 of the OCR tests. The
//! upright check reads the picture's text again with Apple's Vision, so it
//! is ignored by default. Run it with:
//!
//! ```sh
//! cargo test -p traeva-core --test model_image -- --ignored --nocapture
//! ```
//!
//! Nothing is sent to a model; the pictures stay in memory.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Deserialize;

const OCR_CHILD: &str = env!("CARGO_BIN_EXE_traeva-ocr");

/// What must never be in a picture: metadata segments and the made-up
/// values of text-meta.jpg.
const NEVER: [&str; 7] = [
    "Exif",
    "GPS",
    "FAKE",
    "XMP",
    "ICC_PROFILE",
    "Photoshop",
    "8BIM",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    status: String,
    text: String,
    note: Option<String>,
}

fn fixture(folder: &str, name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(folder)
            .join(name),
    )
    .unwrap()
}

/// Runs the child on `bytes` with the app's arguments and `extra`.
fn run(name: &str, bytes: &[u8], extra: &[&str]) -> Answer {
    let started = Instant::now();
    let mut child = Command::new(OCR_CHILD)
        .arg(traeva_core::OCR_FLAG)
        .args([
            "--max-pages",
            "3",
            "--dpi",
            "200",
            "--max-pixels",
            "8000000",
            "--parent-pid",
            &std::process::id().to_string(),
        ])
        .args(extra)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{name}: exit {:?}", out.status);
    let answer: Answer = serde_json::from_slice(&out.stdout).unwrap();
    eprintln!(
        "{name}: {} in {} ms, {} bytes of answer",
        answer.status,
        started.elapsed().as_millis(),
        out.stdout.len()
    );
    answer
}

/// The pictures of a `partial` answer, one standard base64 line each.
fn pictures(name: &str, answer: &Answer) -> Vec<Vec<u8>> {
    assert_eq!(answer.status, "partial", "{name}: {answer:?}");
    assert_eq!(answer.note, None, "{name}");
    answer.text.split('\n').map(base64).collect()
}

/// Standard base64 with padding; panics on anything else.
fn base64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    assert!(
        !text.is_empty() && text.len().is_multiple_of(4),
        "base64 length"
    );
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for &c in text.trim_end_matches('=').as_bytes() {
        let value = ALPHABET
            .iter()
            .position(|&a| a == c)
            .expect("base64 alphabet") as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    out
}

/// The JPEG's width and height from its frame header (SOF0 to SOF15, not
/// DHT, JPG or DAC), walking its segments.
fn jpeg_size(jpeg: &[u8]) -> (u32, u32) {
    let mut at = 2;
    loop {
        assert_eq!(jpeg[at], 0xFF, "a marker at {at}");
        let code = jpeg[at + 1];
        let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if (0xC0..=0xCF).contains(&code) && !matches!(code, 0xC4 | 0xC8 | 0xCC) {
            let height = u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]);
            let width = u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]);
            return (u32::from(width), u32::from(height));
        }
        assert_ne!(code, 0xDA, "no frame header before the scan");
        at += 2 + length;
    }
}

/// A JPEG, at most 1 MiB, its longest side at most 1568 pixels, with
/// nothing in it that `NEVER` lists: its width and height.
fn checked(name: &str, jpeg: &[u8]) -> (u32, u32) {
    assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]), "{name}: JPEG start");
    assert!(jpeg.ends_with(&[0xFF, 0xD9]), "{name}: JPEG end");
    assert!(jpeg.len() <= 1 << 20, "{name}: {} bytes", jpeg.len());
    for word in NEVER {
        assert!(
            !jpeg
                .windows(word.len())
                .any(|window| window == word.as_bytes()),
            "{name}: holds {word}"
        );
    }
    let (width, height) = jpeg_size(jpeg);
    assert!(width.max(height) <= 1568, "{name}: {width} x {height}");
    eprintln!("{name}: {width} x {height}, {} bytes", jpeg.len());
    (width, height)
}

/// Lower case, runs of white space as one space.
fn plain(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[test]
#[ignore = "runs Apple's Vision on this Mac; see the module docs"]
fn the_real_child_makes_pictures_without_metadata() {
    let image = ["--input", "image", "--output", "jpeg"];
    for name in ["text-meta.jpg", "text-alpha.png", "text-plain.png"] {
        let answer = run(name, &fixture("images", name), &image);
        let pictures = pictures(name, &answer);
        assert_eq!(pictures.len(), 1, "{name}");
        checked(name, &pictures[0]);
    }

    // Stored 320 x 900 (a quarter turn to the left); EXIF orientation 6
    // turns it: the picture is wider than high, and its text reads upright.
    let rotated = run(
        "text-exif6.jpg",
        &fixture("images", "text-exif6.jpg"),
        &image,
    );
    let rotated = pictures("text-exif6.jpg", &rotated).remove(0);
    let (width, height) = checked("text-exif6.jpg", &rotated);
    assert!(width > height, "upright: {width} x {height}");
    let read_again = run("text-exif6 picture", &rotated, &["--input", "image"]);
    assert_eq!(read_again.status, "partial", "{read_again:?}");
    let text = plain(&read_again.text);
    for words in ["rotated text sample", "orientation six"] {
        assert!(text.contains(words), "{words:?} not in {text:?}");
    }

    // A large image is scaled to the longest side.
    let large = run(
        "white-12mp.png",
        &fixture("images", "white-12mp.png"),
        &image,
    );
    let large = pictures("white-12mp.png", &large).remove(0);
    let (width, height) = checked("white-12mp.png", &large);
    assert_eq!(width.max(height), 1568);

    // A scanned PDF: one picture for each of its two scanned pages.
    let scan = run(
        "p08-scanned-images.pdf",
        &fixture("pdf", "p08-scanned-images.pdf"),
        &["--output", "jpeg"],
    );
    let pages = pictures("p08-scanned-images.pdf", &scan);
    assert_eq!(pages.len(), 2);
    for (index, page) in pages.iter().enumerate() {
        checked(&format!("p08 page {}", index + 1), page);
    }

    // No picture: a PDF with a text layer, a cut-off image.
    let layer = run(
        "p02-multi-en.pdf",
        &fixture("pdf", "p02-multi-en.pdf"),
        &["--output", "jpeg"],
    );
    assert_eq!(layer.status, "unsupported");
    assert!(layer.text.is_empty());
    let damaged = run("truncated.jpg", &fixture("images", "truncated.jpg"), &image);
    assert_eq!(damaged.status, "failed");
    assert!(damaged.text.is_empty());
    assert!(damaged.note.unwrap().contains("cut off"));
}
