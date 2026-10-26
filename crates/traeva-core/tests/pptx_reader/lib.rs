pub use traeva_core::{MAX_MEMORY_TEXT_CHARS, TextExtraction, TextStatus, TextSummary};

#[path = "../../src/pptx.rs"]
pub mod pptx;

#[cfg(test)]
mod snapshot_tests;
