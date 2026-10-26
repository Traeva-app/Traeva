//! The experiment keeps a copy of the shared reader contract (src/contract.rs)
//! because it does not link traeva-core. This test fails when the core's
//! definitions change, so the copy cannot drift silently.

use std::path::Path;

fn core_file(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/traeva-core/src")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn copy_matches_core_definitions() {
    let contracts = squeeze(&core_file("memory_contracts.rs"));
    for expected in [
        "pub struct TextExtraction { pub summary: TextSummary,",
        "pub text: String, }",
        "#[derive(Debug, Clone, PartialEq, Eq, Serialize)] pub struct TextSummary {",
        "pub reader: String, pub status: TextStatus,",
        "pub chars: usize,",
        "pub covered: Vec<String>,",
        "pub left_out: Vec<String>,",
        "pub note: Option<String>, }",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = \"snake_case\")] pub enum TextStatus {",
        ".field(\"text\", &format_args!(\"<{} bytes>\", self.text.len()))",
    ] {
        assert!(
            contracts.contains(expected),
            "core no longer has: {expected}"
        );
    }
    for variant in ["Complete,", "Partial,", "Unsupported,", "Failed,"] {
        assert!(
            contracts.contains(variant),
            "core TextStatus lacks {variant}"
        );
    }
    let reader = squeeze(&core_file("reader.rs"));
    assert!(reader.contains("pub const MAX_MEMORY_TEXT_CHARS: usize = 1_000_000;"));
    assert_eq!(traeva_pdf_experiment::MAX_MEMORY_TEXT_CHARS, 1_000_000);
}
