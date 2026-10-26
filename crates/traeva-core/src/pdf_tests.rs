//! The PDF reader on the normal fixtures in `tests/fixtures/pdf/`: each
//! reads exactly as the reviewed experiment read it (`expected.json`), under
//! the core's reader name. The experiment keeps its other tests and inputs.

use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf")
}

/// (file name, bytes, expected reading) for every fixture.
fn cases() -> Vec<(String, Vec<u8>, Value)> {
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("expected.json")).unwrap())
            .unwrap();
    let files = expected["files"].as_object().unwrap();
    files
        .iter()
        .map(|(name, want)| {
            let bytes = std::fs::read(fixtures().join(name)).unwrap();
            let digest: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(digest, want["sha256"].as_str().unwrap(), "{name}");
            (name.clone(), bytes, want.clone())
        })
        .collect()
}

#[test]
fn normal_fixtures_read_as_the_reviewed_experiment() {
    let cases = cases();
    assert_eq!(cases.len(), 4);
    for (name, bytes, want) in cases {
        let got = read_pdf(&bytes);
        assert_eq!(
            serde_json::to_value(&got.summary).unwrap(),
            want["summary"],
            "{name}"
        );
        assert_eq!(got.text, want["text"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn text_layer_scan_and_encryption_give_the_expected_statuses() {
    let status = |name: &str| {
        let bytes = std::fs::read(fixtures().join(name)).unwrap();
        read_pdf(&bytes).summary.status
    };
    // Three pages with a text layer; two pages of CJK text.
    assert_eq!(status("p02-multi-en.pdf"), TextStatus::Complete);
    assert_eq!(status("p03-cjk-reportlab.pdf"), TextStatus::Complete);
    // Only images (a scan): no usable text.
    assert_eq!(status("p08-scanned-images.pdf"), TextStatus::Unsupported);
    // Encrypted with an open password: not opened.
    assert_eq!(
        status("e03a-libreoffice-open-password.pdf"),
        TextStatus::Unsupported
    );
}

#[test]
fn readings_are_a_pure_function_of_the_bytes() {
    for (name, bytes, _) in cases() {
        let first = read_pdf(&bytes);
        assert_eq!(first, read_pdf(&bytes), "{name}");
        assert_eq!(first, read_with_pages(&bytes).extraction, "{name}");
        assert_eq!(first.summary.reader, PDF_READER);
        assert!(first.summary.chars == first.text.chars().count());
    }
    assert_eq!(PDF_READER, "pdf-1");
}
