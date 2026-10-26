//! The shared Memory contract (#65): types more than one development lane
//! uses, and what their fields and failures mean. Claude-A owns this file;
//! other lanes ask for changes. docs/development/shared-contract.md has the
//! rules around these types: who writes what, and how each lane refers to
//! files, versions and sources.
//!
//! Organize's DTOs in contracts.rs stay as they are; `AppError` there only
//! gains the snapshot error codes and `ErrorStage::Memory`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One saved version of a file (#32): the exact bytes of one stable read,
/// kept in Traeva's own app-data folder, never inside an authorized folder.
/// Immutable: once recorded, a snapshot is never edited, replaced, repaired
/// or deleted.
///
/// A snapshot only says the file stood still across two full reads with a
/// pause (organize-spec.md §4.1). It never says that the app writing the
/// file had finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotRef {
    pub snapshot_id: String,
    /// The FileRecord (`files.file_id`) the bytes were read from: one
    /// physical file. Not a family: which files are versions of the same
    /// work is decided by #31's rules, never by a consumer of this type.
    pub file_id: String,
    /// 1, 2, 3... in capture order among the snapshots of this `file_id`.
    /// Two snapshots of one file with sequence n and n + 1 are consecutive
    /// saved versions; nothing else makes two snapshots consecutive.
    pub sequence: i64,
    /// `files.record_revision` when the bytes were read.
    pub file_revision: i64,
    /// Where the file was when the bytes were read.
    pub root_id: String,
    pub relative_path: String,
    /// SHA-256 of the exact bytes, lowercase hex. Checked on every read.
    pub sha256: String,
    pub byte_length: u64,
    pub captured_at_ms: i64,
}

/// What a reader got out of one exact set of bytes, for Memory: the whole
/// text up to a limit, and exactly what it covers. A pure function of the
/// file name, the bytes and the reader's version, so the same snapshot
/// always gives the same text.
///
/// The text stays inside the core: this type is deliberately not
/// serializable, so it cannot be sent over IPC, and it is never written to
/// the database or to logs. Its `Debug` output shows the summary and the
/// text's length only. (Organize keeps its own `Extraction`: a head and a
/// tail for one model request.)
///
/// Every reader keeps these rules (tests check them for txt/md):
/// - `Complete`: `left_out` is empty.
/// - `Partial`: some text, and `left_out` names what these bytes hold that
///   was not read. Cutting at `MAX_MEMORY_TEXT_CHARS` is always `Partial`.
/// - `Unsupported` / `Failed`: no text, `chars` 0, `covered` and `left_out`
///   empty, and a `note` saying why.
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

/// A saved version's text, for the window's local preview only
/// (`OrganizeService::preview_version_text`, contract §4.7 v23; first for
/// .docx, #66). The one place a version's text crosses IPC: never stored,
/// logged or sent to a model, and its `Debug` output hides the text like
/// `TextExtraction`'s. The window shows it as plain text with the summary
/// (what was left out, hidden slides or sheets, cached values).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct VersionPreview {
    pub snapshot: SnapshotRef,
    pub extraction: PreviewText,
}

/// The name `preview_memory_docx` has always returned (#103).
pub type DocxPreview = VersionPreview;

/// The reader's summary and the text it read, at most
/// `MAX_MEMORY_TEXT_CHARS` characters.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct PreviewText {
    pub summary: TextSummary,
    pub text: String,
}

impl fmt::Debug for VersionPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VersionPreview")
            .field("snapshot", &self.snapshot)
            .field("summary", &self.extraction.summary)
            .field(
                "text",
                &format_args!("<{} bytes>", self.extraction.text.len()),
            )
            .finish()
    }
}

/// The part of a `TextExtraction` that may be shown, sent to the window or
/// stored: everything except the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextSummary {
    /// Which reader produced the text, and which version of its rules
    /// ("txt-md-1"). Anything derived from the text (a change, a line
    /// range) is saved with it.
    pub reader: String,
    pub status: TextStatus,
    /// Unicode scalar values in `text`.
    pub chars: usize,
    /// What the text covers, in the reader's words ("the whole text").
    /// Empty when nothing was read.
    pub covered: Vec<String>,
    /// What these bytes hold that the reader did not read ("comments", "the
    /// last 5 characters"). Only things actually present in these bytes:
    /// never a list of what the reader can't read in general. Empty exactly
    /// when the status is not `Partial`.
    pub left_out: Vec<String>,
    /// Why the status is not `Complete`, for people. Never file content.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStatus {
    /// Everything the reader covers was read and nothing was left out.
    /// Empty or blank text is still `Complete`.
    Complete,
    /// Some text was read; `left_out` says what is missing.
    Partial,
    /// A file type, encoding or feature this reader does not handle (for
    /// example UTF-16, or an encrypted document), or bytes over a reader's
    /// resource limit (archive entries, unpacked size). No text.
    Unsupported,
    /// The reader handles this type, but the bytes are broken (invalid
    /// UTF-8, a damaged archive). No text.
    Failed,
}

/// Two saved versions that may be used as a known before and after (#31,
/// shared-contract.md §4.3). Only core lists them (lineage.rs); a consumer
/// never pairs snapshots by file, name, hash or time itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsablePair {
    pub before: SnapshotRef,
    pub after: SnapshotRef,
    pub basis: PairBasis,
    /// `user` when the pair rests on the user's confirmation (a relation, or
    /// the file's place in the work); never upgraded to `system`.
    pub evidence: PairEvidence,
    /// Memory may not have seen every save between the two: describe the
    /// pair as a difference, never as one save. `gap_kinds` says why.
    pub gap: bool,
    pub gap_kinds: Vec<GapKind>,
    /// The file's place in the work is unknown ("same work, order unknown"):
    /// the pair may be shown and compared, but never used to order versions
    /// or to find the first or last one.
    pub unplaced: bool,
    pub family_id: String,
    /// The record task (chain) whose line holds `after`; null for a file on
    /// no chain's line.
    pub chain_id: Option<String>,
    /// The files whose Memory send permission (#33, `memory_consents`) must
    /// all count before the pair's lines may be sent (D4). One file for a
    /// pair on one line; both sides for a branch relation.
    pub permission_file_ids: Vec<String>,
    /// The relation the pair rests on; null for two versions of one file.
    pub edge_id: Option<String>,
}

/// What makes two versions a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairBasis {
    /// Versions n and n + 1 of one file.
    SameFile,
    /// The last version of one object and the first of the object that the
    /// user confirmed goes on from it, or of the same object's new file
    /// record at the place the user chose after it was moved (relocation).
    Continuation,
    /// A fork point and the first version of a branch.
    Branch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairEvidence {
    System,
    User,
}

/// A reason Memory knows its history may be incomplete somewhere (D3).
/// Each is recorded evidence that Memory was not looking, or could not save;
/// none of them says that the file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    /// Memory was turned off for the record, then on again.
    MemoryOff,
    /// Candidate capture was paused at its 20/20 quota.
    Paused,
    /// The file's place could not be captured (the problem says why).
    CaptureFailed,
    /// The place held an object Memory did not read (it was replaced and
    /// candidate capture was off, or the object was recorded elsewhere).
    ReplacedUncaptured,
    /// The object at the place was judged unrelated; the work is lost until
    /// its new place is chosen.
    Lost,
    /// Traeva's watcher was not running (Traeva was quit or crashed).
    WatcherStopped,
    /// From before Traeva recorded when its watcher ran (data older than
    /// schema version 7).
    NotRecorded,
    /// The pair skips objects that are not on its line (judged unrelated,
    /// unknown or a branch), a branch whose start was not seen, or a
    /// relation the user made by associating a file with a work. Pairs only.
    Skipped,
}

/// Whether one pair may be used right now, by the rules that list pairs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairCheck {
    Usable { pair: UsablePair },
    NotUsable { reason: PairRefusal },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairRefusal {
    /// A version's file is not a confirmed member of a work.
    NotAMember,
    /// A version is a candidate's: nothing about it is confirmed.
    CandidateVersion,
    /// The two versions are not joined by a confirmed relation.
    NoConfirmedRelation,
    /// Two versions of one file that are not consecutive.
    NotAdjacent,
    /// The relation between the two is disputed (#31 M2b): it is not used
    /// until the user confirms it again or revokes it.
    RelationDisputed,
    /// The relation between the two was revoked or replaced by a later
    /// decision (#31 M2b): it no longer counts.
    RelationSuperseded,
}
