//! Observing files and folders (organize-spec.md §4). Everything here only reads.
//!
//! - `read_stable`: read the whole file twice with a pause in between and keep
//!   the result only if nothing moved. The SHA-256 is of the exact bytes.
//! - `extract_text`: a pure function from those same bytes to the text Traeva
//!   may show and later send for analysis.
//! - `build_catalog`: the bounded list of folders inside a root.

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::contracts::{ExtractionStatus, ExtractionSummary};
use crate::platform::macos::{self, Folder, FsError, ObjectId, Observation, Protection};

/// Files above this size are never read or moved, not even by hand.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
pub const PAUSE_BETWEEN_READS: Duration = Duration::from_millis(350);
pub const MAX_READ_ATTEMPTS: u32 = 3;
pub const MAX_READ_TIME: Duration = Duration::from_secs(5);

/// Texts up to this many characters are used whole; longer ones as head + tail.
pub const MAX_TEXT_CHARS: usize = 8_000;
pub const HEAD_CHARS: usize = 6_000;
pub const TAIL_CHARS: usize = 2_000;
/// Longest piece of text that gets its own evidence ID.
pub const EVIDENCE_MAX_CHARS: usize = 800;
/// Saved with every model request (`analyses.extractor_version`). Raise it
/// when `extract_text` starts producing different text from the same bytes.
pub const EXTRACTOR_VERSION: &str = "txt-md-1";

pub const MAX_CATALOG_FOLDERS: usize = 500;
pub const MAX_CATALOG_DEPTH: usize = 6;
pub const MAX_MODEL_CANDIDATES: usize = 100;

// ---------------------------------------------------------------------------
// Stable reads
// ---------------------------------------------------------------------------

/// Budget for one stable read. Production uses `ReadLimits::SPEC`.
#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    pub max_attempts: u32,
    pub max_time: Duration,
    pub max_bytes: u64,
}

impl ReadLimits {
    pub const SPEC: ReadLimits = ReadLimits {
        max_attempts: MAX_READ_ATTEMPTS,
        max_time: MAX_READ_TIME,
        max_bytes: MAX_FILE_BYTES,
    };
}

/// Both full reads gave the same bytes and nothing about the file moved in
/// between. This does not prove an editor has finished saving; it only shows
/// the file stood still for one pause.
#[derive(Debug)]
pub struct StableRead {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub observed: Observation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The filesystem rules refuse the file, or it cannot be read.
    Fs(FsError),
    TooLarge,
    /// Still changing when the attempts or the time ran out (waiting_for_file).
    StillChanging {
        attempts: u32,
    },
}

/// `read_stable_with` with the spec's budget and a real 350 ms pause.
pub fn read_stable(parent: &Folder, name: &str, root_dev: i64) -> Result<StableRead, ReadError> {
    read_stable_with(parent, name, root_dev, ReadLimits::SPEC, &mut |_| {
        std::thread::sleep(PAUSE_BETWEEN_READS)
    })
}

/// Up to `max_attempts` attempts within `max_time`. `pause` runs between the
/// two reads of each attempt: production sleeps there, tests change the file.
pub fn read_stable_with(
    parent: &Folder,
    name: &str,
    root_dev: i64,
    limits: ReadLimits,
    pause: &mut dyn FnMut(u32),
) -> Result<StableRead, ReadError> {
    let started = Instant::now();
    let mut attempt = 0;
    loop {
        attempt += 1;
        if let Some(read) = read_once(parent, name, root_dev, limits.max_bytes, attempt, pause)? {
            return Ok(read);
        }
        if attempt >= limits.max_attempts || started.elapsed() >= limits.max_time {
            return Err(ReadError::StillChanging { attempts: attempt });
        }
    }
}

/// One attempt: look, open, read A, pause, read B, look again.
/// `None` means something moved and the attempt does not count.
fn read_once(
    parent: &Folder,
    name: &str,
    root_dev: i64,
    max_bytes: u64,
    attempt: u32,
    pause: &mut dyn FnMut(u32),
) -> Result<Option<StableRead>, ReadError> {
    // Look before opening, so a placeholder, link or special file is never opened.
    let looked = macos::observe(parent, name).map_err(ReadError::Fs)?;
    macos::check_movable_file(name, &looked, root_dev).map_err(ReadError::Fs)?;
    if looked.size > max_bytes {
        return Err(ReadError::TooLarge);
    }

    let mut file = macos::open_file(parent, name).map_err(ReadError::Fs)?;
    let opened = macos::observe_open(&file).map_err(ReadError::Fs)?;
    macos::check_movable_file(name, &opened, root_dev).map_err(ReadError::Fs)?;
    if opened.id != looked.id {
        return Ok(None); // replaced between looking and opening
    }

    let bytes_a = macos::read_whole_file(&mut file, max_bytes).map_err(ReadError::Fs)?;
    if bytes_a.len() as u64 > max_bytes {
        return Err(ReadError::TooLarge);
    }
    let after_a = macos::observe_open(&file).map_err(ReadError::Fs)?;
    pause(attempt);
    let bytes_b = macos::read_whole_file(&mut file, max_bytes).map_err(ReadError::Fs)?;
    let after_b = macos::observe_open(&file).map_err(ReadError::Fs)?;
    let at_name_now = macos::observe(parent, name).ok();

    let hash_a = sha256_hex(&bytes_a);
    let hash_b = sha256_hex(&bytes_b);
    if is_stable(
        &hash_a,
        &hash_b,
        &opened,
        &after_a,
        &after_b,
        at_name_now.as_ref(),
    ) {
        Ok(Some(StableRead {
            bytes: bytes_a,
            sha256: hash_a,
            observed: after_b,
        }))
    } else {
        Ok(None)
    }
}

/// Stable means: both full reads have the same hash (even if size and mtime
/// look the same, contents are compared), the open file's attributes did not
/// move, and the name still points at the very object that was read.
pub fn is_stable(
    hash_a: &str,
    hash_b: &str,
    opened: &Observation,
    after_a: &Observation,
    after_b: &Observation,
    at_name_now: Option<&Observation>,
) -> bool {
    hash_a == hash_b
        && same_state(opened, after_a)
        && same_state(after_a, after_b)
        && at_name_now.is_some_and(|now| same_state(now, after_b))
}

fn same_state(a: &Observation, b: &Observation) -> bool {
    a.id == b.id
        && a.kind == b.kind
        && a.size == b.size
        && a.mtime_ns == b.mtime_ns
        && a.ctime_ns == b.ctime_ns
        && a.link_count == b.link_count
        && a.flags == b.flags
}

/// SHA-256 of the exact bytes, as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Text extraction: a pure function of (file name, bytes)
// ---------------------------------------------------------------------------

/// The text Traeva may use: the whole text, or its head and tail with the
/// middle explicitly left out. Kept in memory only; never written to the
/// database or to logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub summary: ExtractionSummary,
    /// The excerpt as continuous text, line breaks kept: the whole text, or
    /// its head and its tail (never joined). The evidence pieces lie inside it.
    pub excerpt: Vec<String>,
    pub evidence: Vec<Evidence>,
    /// The left-out middle, when only head and tail are used.
    pub omitted: Option<Omission>,
}

/// One piece of text the model may cite by ID. Lines are 1-based and counted
/// in the whole text; chars are Unicode scalar offsets, `start..end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub id: String,
    pub first_line: usize,
    pub last_line: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
}

/// Where the left-out middle is: chars `start..end`, from line `first_line`
/// (where the head stops) to line `last_line` (where the tail starts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Omission {
    pub start_char: usize,
    pub end_char: usize,
    pub first_line: usize,
    pub last_line: usize,
}

const TEXT_EXTENSIONS: &[&str] = &["txt", "md", "markdown"];

/// Strict UTF-8 only (a UTF-8 BOM is allowed and dropped). UTF-16, NUL bytes
/// and invalid UTF-8 are never guessed at or decoded with replacements.
pub fn extract_text(file_name: &str, bytes: &[u8]) -> Extraction {
    match decode_text(file_name, bytes) {
        Ok(text) => excerpt_text(text),
        Err((status, note)) => not_extracted(status, note),
    }
}

/// The excerpt of a whole text, as §4.1 says: all of it, or its head and
/// tail with the middle marked as left out. Text from a PDF's text layer
/// takes the same way (organize-spec.md §14.2).
pub(crate) fn excerpt_text(text: String) -> Extraction {
    let total_chars = text.chars().count();
    let blank = text.trim().is_empty();
    let truncated = total_chars > MAX_TEXT_CHARS;
    let mut next_id = 1;
    let (excerpt, evidence, omitted) = if blank {
        (Vec::new(), Vec::new(), None)
    } else if !truncated {
        let evidence = split_evidence(&text, 0, 1, &mut next_id);
        (vec![text.clone()], evidence, None)
    } else {
        let tail_start_char = total_chars - TAIL_CHARS;
        let head = &text[..byte_index(&text, HEAD_CHARS)];
        let tail_start = byte_index(&text, tail_start_char);
        let tail = &text[tail_start..];
        let head_end_line = 1 + head.matches('\n').count();
        let tail_first_line = 1 + text[..tail_start].matches('\n').count();
        let mut evidence = split_evidence(head, 0, 1, &mut next_id);
        evidence.extend(split_evidence(
            tail,
            tail_start_char,
            tail_first_line,
            &mut next_id,
        ));
        let omitted = Omission {
            start_char: HEAD_CHARS,
            end_char: tail_start_char,
            first_line: head_end_line,
            last_line: tail_first_line,
        };
        (
            vec![head.to_string(), tail.to_string()],
            evidence,
            Some(omitted),
        )
    };

    Extraction {
        summary: ExtractionSummary {
            status: if truncated {
                ExtractionStatus::Partial
            } else {
                ExtractionStatus::Ok
            },
            total_chars,
            truncated,
            blank,
            usable_text: !evidence.is_empty(),
            note: None,
            recognized: false,
            image_input: false,
        },
        excerpt,
        evidence,
        omitted,
    }
}

/// The whole text of a .txt or .md file, with CRLF and CR turned into LF;
/// or why there is none (`Unsupported` or `Failed`, and a note for people).
/// Organize's `extract_text` and Memory's `reader::read_text` both decode
/// through here, so the two read the same bytes as the same text.
pub fn decode_text(
    file_name: &str,
    bytes: &[u8],
) -> Result<String, (ExtractionStatus, &'static str)> {
    if !is_text_file_name(file_name) {
        return Err((
            ExtractionStatus::Unsupported,
            "Only .txt and .md files are read.",
        ));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err((
            ExtractionStatus::Unsupported,
            "UTF-16 text is not supported.",
        ));
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if body.contains(&0) {
        return Err((
            ExtractionStatus::Unsupported,
            "The file contains NUL bytes, so it is not plain text.",
        ));
    }
    let Ok(raw) = std::str::from_utf8(body) else {
        return Err((ExtractionStatus::Failed, "The file is not valid UTF-8."));
    };
    Ok(raw.replace("\r\n", "\n").replace('\r', "\n"))
}

pub(crate) fn not_extracted(status: ExtractionStatus, note: &str) -> Extraction {
    Extraction {
        summary: ExtractionSummary {
            status,
            total_chars: 0,
            truncated: false,
            blank: false,
            usable_text: false,
            note: Some(note.to_string()),
            recognized: false,
            image_input: false,
        },
        excerpt: Vec::new(),
        evidence: Vec::new(),
        omitted: None,
    }
}

/// A `.pdf` name (with a stem): Organize reads its text layer in a separate
/// process (organize-spec.md §14). Memory never reads it.
pub fn is_pdf_file_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, extension)) => !stem.is_empty() && extension.eq_ignore_ascii_case("pdf"),
        None => false,
    }
}

/// A `.jpg`, `.jpeg` or `.png` name (with a stem): Organize recognizes the
/// image's text on this Mac (organize-spec.md §14.9). Memory, Find and
/// Source never read it.
pub fn is_image_file_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, extension)) => {
            !stem.is_empty()
                && ["jpg", "jpeg", "png"]
                    .iter()
                    .any(|kind| extension.eq_ignore_ascii_case(kind))
        }
        None => false,
    }
}

/// A `.docx`, `.pptx` or `.xlsx` name (with a stem): Organize reads its
/// text with Memory's readers (the student's decision 10/17).
pub fn is_office_file_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, extension)) => {
            !stem.is_empty()
                && ["docx", "pptx", "xlsx"]
                    .iter()
                    .any(|kind| extension.eq_ignore_ascii_case(kind))
        }
        None => false,
    }
}

pub fn is_text_file_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => TEXT_EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension)),
        _ => false,
    }
}

/// Byte position of character number `chars` (or the end of the text).
fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(index, _)| index)
}

/// Splits one segment into evidence pieces: runs of non-blank lines (a blank
/// line ends a piece), at most `EVIDENCE_MAX_CHARS` each; a longer single
/// line is cut into pieces of its own. `start_char` and `first_line` say
/// where the segment sits in the whole text.
fn split_evidence(
    segment: &str,
    start_char: usize,
    first_line: usize,
    next_id: &mut usize,
) -> Vec<Evidence> {
    let chars: Vec<char> = segment.chars().collect();
    // (first line, last line, start, end), in chars from the segment start.
    let mut ranges: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut open: Option<(usize, usize, usize, usize)> = None;
    let mut offset = 0;
    for (index, line) in segment.split('\n').enumerate() {
        let line_number = first_line + index;
        let length = line.chars().count();
        if line.trim().is_empty() {
            ranges.extend(open.take());
        } else if length > EVIDENCE_MAX_CHARS {
            ranges.extend(open.take());
            let mut start = offset;
            while start < offset + length {
                let end = (start + EVIDENCE_MAX_CHARS).min(offset + length);
                ranges.push((line_number, line_number, start, end));
                start = end;
            }
        } else {
            match open {
                Some((first, _, start, _)) if offset + length - start <= EVIDENCE_MAX_CHARS => {
                    open = Some((first, line_number, start, offset + length));
                }
                _ => {
                    ranges.extend(open.take());
                    open = Some((line_number, line_number, offset, offset + length));
                }
            }
        }
        offset += length + 1; // the line plus its '\n'
    }
    ranges.extend(open);

    ranges
        .into_iter()
        .map(|(first, last, start, end)| {
            let piece = Evidence {
                id: format!("E{next_id}"),
                first_line: first,
                last_line: last,
                start_char: start_char + start,
                end_char: start_char + end,
                text: chars[start..end].iter().collect(),
            };
            *next_id += 1;
            piece
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Folder catalog
// ---------------------------------------------------------------------------

/// One folder in a catalog, with the evidence needed to check it again later.
/// `folder_id` is only meaningful within this one catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRef {
    pub folder_id: String,
    pub parent_id: Option<String>,
    pub segments: Vec<String>,
    pub id: ObjectId,
    pub protected: Option<Protection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCatalog {
    /// The root first (as "F0"), then breadth-first: shallow folders come first.
    pub folders: Vec<FolderRef>,
    /// True when the folder or depth limit left folders out.
    pub truncated: bool,
}

impl FolderCatalog {
    /// Folders the model may suggest: never protected ones, at most 100,
    /// shallowest first.
    pub fn model_candidates(&self) -> Vec<&FolderRef> {
        self.folders
            .iter()
            .filter(|folder| folder.protected.is_none())
            .take(MAX_MODEL_CANDIDATES)
            .collect()
    }
}

pub fn build_catalog(root: &Folder) -> Result<FolderCatalog, FsError> {
    build_catalog_with(root, MAX_CATALOG_FOLDERS, MAX_CATALOG_DEPTH)
}

/// Lists folders breadth-first from `root`, opening each one again from the
/// root handle (so at most one short chain of handles is open at a time).
/// Protected folders are listed but never entered.
pub fn build_catalog_with(
    root: &Folder,
    max_folders: usize,
    max_depth: usize,
) -> Result<FolderCatalog, FsError> {
    let mut folders = vec![FolderRef {
        folder_id: "F0".to_string(),
        parent_id: None,
        segments: Vec::new(),
        id: root.id(),
        protected: None,
    }];
    let mut truncated = false;
    let mut next = 0;
    'expand: while next < folders.len() {
        let current = folders[next].clone();
        next += 1;
        if current.protected.is_some() {
            continue;
        }
        let names: Vec<&str> = current.segments.iter().map(String::as_str).collect();
        let opened = if names.is_empty() {
            None
        } else {
            match macos::open_chain(root, &names) {
                Ok(mut chain) => chain.pop(),
                Err(_) => continue, // gone or changed since it was listed
            }
        };
        let folder = opened.as_ref().unwrap_or(root);
        if folder.id() != current.id {
            continue; // replaced since it was listed
        }
        let children = match macos::list_child_folders(folder, root.id().dev) {
            Ok(children) => children,
            Err(error) if names.is_empty() => return Err(error),
            Err(_) => continue,
        };
        for child in children {
            if current.segments.len() >= max_depth || folders.len() >= max_folders {
                truncated = true;
                if folders.len() >= max_folders {
                    break 'expand;
                }
                break;
            }
            let mut segments = current.segments.clone();
            segments.push(child.name);
            folders.push(FolderRef {
                folder_id: format!("F{}", folders.len()),
                parent_id: Some(current.folder_id.clone()),
                segments,
                id: child.id,
                protected: child.protected,
            });
        }
    }
    Ok(FolderCatalog { folders, truncated })
}

#[cfg(test)]
#[path = "observe_tests.rs"]
mod tests;
