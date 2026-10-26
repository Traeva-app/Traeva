//! Sweeps over generated and exported fixtures: every truncation of a small
//! file, seeded single-byte mutations, and repeat reads. Run
//! `python3 fixtures/generate.py` first.

use std::path::Path;

use traeva_pdf_experiment::{PdfReading, TextStatus, read_pdf};

fn fixture(path: &str) -> Vec<u8> {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(path);
    std::fs::read(&full).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run `python3 fixtures/generate.py` first",
            full.display()
        )
    })
}

/// The shared contract's rules (shared-contract.md §4.4), plus this
/// experiment's own: never Complete with empty text.
fn assert_contract(reading: &PdfReading, context: &str) {
    let summary = &reading.extraction.summary;
    let text = &reading.extraction.text;
    assert_eq!(summary.chars, text.chars().count(), "{context}");
    assert!(summary.chars <= 1_000_000, "{context}");
    assert!(!text.contains('\r'), "{context}");
    match summary.status {
        TextStatus::Complete => {
            assert!(
                summary.left_out.is_empty() && summary.note.is_none(),
                "{context}"
            );
            assert!(
                !summary.covered.is_empty() && !text.trim().is_empty(),
                "{context}"
            );
        }
        TextStatus::Partial => {
            assert!(!text.is_empty() && !summary.covered.is_empty(), "{context}");
            assert!(
                !summary.left_out.is_empty() && summary.note.is_some(),
                "{context}"
            );
        }
        TextStatus::Unsupported | TextStatus::Failed => {
            assert!(text.is_empty() && summary.chars == 0, "{context}");
            assert!(
                summary.covered.is_empty() && summary.left_out.is_empty(),
                "{context}"
            );
            assert!(summary.note.is_some(), "{context}");
        }
    }
}

#[test]
fn every_truncation_of_a_small_file_fails_until_the_end_marker() {
    let bytes = fixture("generated/p01-single-en.pdf");
    let full = read_pdf(&bytes);
    assert_eq!(full.extraction.summary.status, TextStatus::Complete);
    let end_marker = bytes.windows(5).rposition(|w| w == b"%%EOF").unwrap() + 5;
    for cut in 0..bytes.len() {
        let reading = read_pdf(&bytes[..cut]);
        let context = format!("cut at {cut} of {}", bytes.len());
        assert_contract(&reading, &context);
        if cut < end_marker {
            assert_eq!(
                reading.extraction.summary.status,
                TextStatus::Failed,
                "{context}"
            );
        } else {
            assert_eq!(reading.extraction, full.extraction, "{context}");
        }
    }
}

/// A small fixed generator, so the mutated positions are the same each run.
fn positions(seed: u64, len: usize, count: usize) -> Vec<usize> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize % len
        })
        .collect()
}

fn mutate_and_check(path: &str, seed: u64, count: usize) {
    let bytes = fixture(path);
    for (n, at) in positions(seed, bytes.len(), count).into_iter().enumerate() {
        let mut changed = bytes.clone();
        changed[at] ^= [0xff, 0x01, 0x20, 0x80][n % 4];
        let reading = read_pdf(&changed);
        assert_contract(&reading, &format!("{path}: byte {at} changed"));
    }
}

#[test]
fn single_byte_changes_keep_the_contract() {
    mutate_and_check("generated/p02-multi-en.pdf", 1, 400);
    mutate_and_check("generated/p06-font-mapping.pdf", 2, 400);
    mutate_and_check("generated/p03-cjk-reportlab.pdf", 3, 300);
    mutate_and_check("exported/e01-libreoffice.pdf", 4, 300);
    mutate_and_check("exported/e02-chromium.pdf", 5, 300);
}

#[test]
fn the_same_bytes_read_the_same() {
    for path in [
        "generated/p03-cjk-reportlab.pdf",
        "generated/p09-mixed-pages.pdf",
        "exported/e01-libreoffice.pdf",
        "exported/e02-chromium.pdf",
    ] {
        let bytes = fixture(path);
        let first = read_pdf(&bytes);
        let second = read_pdf(&bytes);
        assert_eq!(first.extraction, second.extraction, "{path}");
        assert_eq!(first.pages, second.pages, "{path}");
    }
}
