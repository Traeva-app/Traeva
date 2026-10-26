//! Regression tests for the defects Codex-R4 found: in 07ce381 (first
//! round, fixtures/defects/), in e2d836f (second round, R4-PDF-06 and
//! R4-PDF-07, fixtures/defects2/) and in c3a28cf (third round, R4-PDF-08,
//! fixtures/defects3/). The reproductions were written from R4's
//! descriptions; R4's own inputs were not seen. The expected status, note
//! and text come from each set's expected.json, frozen before the fix (in
//! the third set g14-g17 re-frozen before the fix with their basis, and
//! g25-g27 marked unverified: for those only the shared contract is
//! checked). Run each set's `generate.py` first.

use std::path::Path;

use serde_json::Value;
use traeva_pdf_experiment::{TextStatus, read_pdf};

fn here() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn status_name(status: TextStatus) -> &'static str {
    match status {
        TextStatus::Complete => "complete",
        TextStatus::Partial => "partial",
        TextStatus::Unsupported => "unsupported",
        TextStatus::Failed => "failed",
    }
}

#[test]
fn every_reproduction_reads_as_frozen() {
    reads_as_frozen("fixtures/defects/expected.json", 24);
}

#[test]
fn every_second_round_reproduction_reads_as_frozen() {
    reads_as_frozen("fixtures/defects2/expected.json", 24);
}

#[test]
fn every_third_round_reproduction_reads_as_frozen() {
    reads_as_frozen("fixtures/defects3/expected.json", 27);
}

fn reads_as_frozen(set: &str, count: usize) {
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(here().join(set)).unwrap()).unwrap();
    let cases = expected["fixtures"].as_object().unwrap();
    assert_eq!(cases.len(), count);
    for (name, case) in cases {
        let path = here().join("fixtures").join(case["path"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!("{}: {e}; run the generator of {set} first", path.display())
        });
        let reading = read_pdf(&bytes);
        let summary = &reading.extraction.summary;
        if case.get("unverified").is_some() {
            assert_eq!(
                summary.chars,
                reading.extraction.text.chars().count(),
                "{name}"
            );
            if matches!(summary.status, TextStatus::Unsupported | TextStatus::Failed) {
                assert!(
                    reading.extraction.text.is_empty() && summary.note.is_some(),
                    "{name}"
                );
            }
            continue;
        }
        assert_eq!(
            status_name(summary.status),
            case["status"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(summary.note.as_deref(), case["note"].as_str(), "{name}");
        assert_eq!(
            reading.extraction.text,
            case["text"].as_str().unwrap(),
            "{name}"
        );
        let left_out: Vec<&str> = case["left_out"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(summary.left_out, left_out, "{name}");
    }
}

#[test]
fn reproductions_keep_the_contract_under_byte_changes() {
    for name in [
        "defects/generated/d01-nested-form-string-bomb.pdf",
        "defects/generated/d05-strings-in-stream-dict.pdf",
        "defects/generated/d09-escaped-javascript-action.pdf",
        "defects/generated/d13-contents-missing-member.pdf",
        "defects/generated/d17-flate-wrong-adler.pdf",
        "defects/generated/d22-xref-stream.pdf",
        "defects/generated/d24-incremental-update.pdf",
        "defects2/generated/f01-listing-in-text.pdf",
        "defects2/generated/f06-lzw-listing-in-image-data.pdf",
        "defects2/generated/f08-endstream-in-text-indirect-length.pdf",
        "defects2/generated/f17-text-and-indirect-null-page.pdf",
        "defects2/generated/f22-contents-null-in-object-stream.pdf",
        "defects3/generated/g13-matching-length-endstream-comment.pdf",
        "defects3/generated/g14-length-too-short.pdf",
        "defects3/generated/g16-length-missing.pdf",
        "defects3/generated/g18-flate-bomb-wrong-length.pdf",
        "defects3/generated/g23-rebuilt-xref-control.pdf",
    ] {
        let bytes = std::fs::read(here().join("fixtures").join(name)).unwrap();
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        for n in 0..150 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let at = (state >> 33) as usize % bytes.len();
            let mut changed = bytes.clone();
            changed[at] ^= [0xff, 0x01, 0x20, 0x80][n % 4];
            let reading = read_pdf(&changed);
            let summary = &reading.extraction.summary;
            let text = &reading.extraction.text;
            let context = format!("{name}: byte {at} changed");
            assert_eq!(summary.chars, text.chars().count(), "{context}");
            match summary.status {
                TextStatus::Complete => {
                    assert!(
                        summary.left_out.is_empty() && summary.note.is_none(),
                        "{context}"
                    );
                    assert!(!text.trim().is_empty(), "{context}");
                }
                TextStatus::Partial => {
                    assert!(
                        !text.is_empty() && !summary.left_out.is_empty(),
                        "{context}"
                    );
                }
                TextStatus::Unsupported | TextStatus::Failed => {
                    assert!(text.is_empty() && summary.note.is_some(), "{context}");
                }
            }
        }
    }
}
