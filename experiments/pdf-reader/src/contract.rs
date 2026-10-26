//! A copy of the shared Memory reader contract (#65, shared-contract.md §4.4):
//! `TextExtraction`, `TextSummary`, `TextStatus` and `MAX_MEMORY_TEXT_CHARS`
//! from `crates/traeva-core/src/memory_contracts.rs` and `reader.rs`.
//!
//! This experiment does not link traeva-core (it does not build on Linux,
//! and the experiment must not add to the core's dependencies), so it keeps
//! an exact copy. `tests/contract.rs` fails if the core's definitions drift.
//! An approved integration would delete this file and use the core's types.

use std::fmt;

use serde::Serialize;

/// Longest text any reader keeps, in Unicode scalar values. Longer texts
/// are cut there and reported as `Partial`.
pub const MAX_MEMORY_TEXT_CHARS: usize = 1_000_000;

/// What a reader got out of one exact set of bytes. Not serializable, and
/// its `Debug` output shows the text's length only.
#[derive(Clone, PartialEq, Eq)]
pub struct TextExtraction {
    pub summary: TextSummary,
    /// The text, with '\n' as the only line break. Empty unless the status
    /// is `Complete` or `Partial`.
    pub text: String,
}

impl fmt::Debug for TextExtraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextExtraction")
            .field("summary", &self.summary)
            .field("text", &format_args!("<{} bytes>", self.text.len()))
            .finish()
    }
}

/// The part of a `TextExtraction` that may be shown or stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextSummary {
    pub reader: String,
    pub status: TextStatus,
    /// Unicode scalar values in `text`.
    pub chars: usize,
    /// What the text covers, in the reader's words. Empty when nothing was read.
    pub covered: Vec<String>,
    /// What these bytes hold that the reader did not read. Only things
    /// actually present. Empty exactly when the status is not `Partial`.
    pub left_out: Vec<String>,
    /// Why the status is not `Complete`, for people. Never file content.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStatus {
    Complete,
    Partial,
    Unsupported,
    Failed,
}
