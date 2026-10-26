//! The PDF reader for Organize (#186, organize-spec §14): the reviewed
//! experiment `experiments/pdf-reader` (PR #148, `81ddd80`) moved into the
//! core without changing its logic (`tests/fixtures/pdf/relocate.py`).
//! Organize reads PDF only through `pdf_child`, in a separate process;
//! Memory does not read PDF.
//!
//! `read_with_pages(bytes)` turns one exact set of PDF bytes into the
//! shared text contract (`memory_contracts`) plus a per-page table;
//! `read_pdf(bytes)` returns the text contract only.
//! It never opens a path, writes, runs JavaScript, fetches anything or
//! decompresses to disk. Steps:
//!
//! 1. `preflight`: raw-byte checks (size, header, end marker, encryption)
//!    and every stream's decoded size, measured with capped decoders
//!    before the PDF library parses anything.
//! 2. `document`: hayro opens the bytes; object, page and per-page content
//!    limits; each page is drawn into a collector of glyphs, images and
//!    line shapes.
//! 3. `layout`: glyphs → lines, in the order the PDF draws them.
//! 4. Here: each page's state, the document status, `covered`/`left_out`.

#[path = "pdf_document.rs"]
mod document;
#[path = "pdf_layout.rs"]
mod layout;
#[path = "pdf_preflight.rs"]
mod preflight;
#[path = "pdf_scan.rs"]
mod scan;

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};

use serde::Serialize;

use crate::memory_contracts::{TextExtraction, TextStatus, TextSummary};
use crate::reader::MAX_MEMORY_TEXT_CHARS;

/// Which reader, and which version of its rules, produced a text: raise
/// the number when the same bytes start producing different text. `pdf-1`
/// is `pdf-experiment-9` (PR #148, `81ddd80`; Codex-R4 round five, LIMITED
/// PASS) moved into the core unchanged.
pub const PDF_READER: &str = "pdf-1";

pub const MAX_PDF_BYTES: usize = 32 << 20;
pub const MAX_OBJECTS: usize = 200_000;
pub const MAX_PAGES: usize = 500;
/// The note of an encrypted PDF (the reader opens none, even one that needs
/// no password, §14); the drop queue says it in its own words.
pub const ENCRYPTED_NOTE: &str = "The PDF is encrypted. This reader does not open encrypted PDFs, even when no password is needed.";
pub const MAX_STREAMS: usize = 50_000;
pub const MAX_FILTERS: usize = 4;
/// One decoded stream that is not an image.
pub const MAX_STREAM_BYTES: usize = 16 << 20;
/// One image stream, decoded up to (not through) its image codec.
pub const MAX_IMAGE_STREAM_BYTES: usize = 64 << 20;
/// All non-image streams together.
pub const MAX_DECODED_BYTES: usize = 128 << 20;
/// All streams together, as decoded by the raw-byte check.
pub const MAX_DECODE_WORK: usize = 256 << 20;
/// One page's content streams together.
pub const MAX_PAGE_CONTENT_BYTES: usize = 32 << 20;
/// The document is reopened after this much drawn page content.
pub const MAX_CACHED_CONTENT_BYTES: usize = 64 << 20;
/// Different values one object can hold as an indirect `/Length` (each
/// definition of it in the file and in object streams; usually one or two):
/// a stream whose length object has more cannot be checked before reading.
pub const MAX_LENGTH_VALUES: usize = 64;
/// The offset table of one object stream, in bytes: hayro reads it again
/// for each member it reads (about 6,000 pairs of the usual size).
pub const MAX_OBJSTM_TABLE_BYTES: usize = 64 << 10;
/// Member reads one pass over all objects can make times the largest offset
/// table: the bytes of offset tables one pass may read.
pub const MAX_OBJSTM_SCAN: usize = 512 << 20;
/// Nesting of dictionaries and arrays in the raw-byte check (hayro has the
/// same object limit, and 50 nested forms, patterns or Type 3 glyphs).
pub const MAX_DICT_DEPTH: usize = 64;
pub const MAX_GLYPHS: usize = 2_000_000;
pub const MAX_DRAW_EVENTS: usize = 1_000_000;
/// A page with no text whose images cover at least this share is reported
/// as an image page (likely a scan).
pub const IMAGE_PAGE_COVER: f64 = 0.5;

/// The shared text result, plus where each page is in the text and what
/// was left out on it. The page table is this experiment's own: the shared
/// contract has no field for it.
#[derive(Debug, Clone)]
pub struct PdfReading {
    pub extraction: TextExtraction,
    pub pages: Vec<PageReading>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageReading {
    /// 1-based position in the document (not the printed page label).
    pub page: usize,
    pub state: PageState,
    /// The page's lines in `extraction.text`, as Unicode scalar offsets
    /// (end exclusive), not counting its "[Page N]" line. Empty when the
    /// page has no lines in the text.
    pub start: usize,
    pub end: usize,
    pub lines: usize,
    pub glyphs: usize,
    pub rotated_lines: usize,
    pub invisible_chars: usize,
    pub wide_gaps: usize,
    pub omissions: Vec<Omission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageState {
    /// Text, and nothing on the page was left out.
    Text,
    /// Text, and something on the page was left out.
    TextWithOmissions,
    /// No usable text on the page.
    NoText,
    /// Past the text limit: not in the text.
    NotRead,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Omission {
    Images { count: usize, cover_percent: u32 },
    ImagePage { cover_percent: u32 },
    Unmapped { count: usize },
    UndecodedText,
    Grid,
    Drawings { count: usize },
    Math,
    Annotations { count: usize },
    FormFields { count: usize },
    Outside { count: usize },
    Truncated,
}

/// The work of the raw-byte check on one set of bytes, reported by
/// `pdf-probe` as evidence (no limit uses it).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ScanWork {
    /// Dictionary and array elements read.
    pub elements: usize,
    /// Bytes the decoders read and wrote, and unfiltered stream data.
    pub decode_bytes: usize,
}

/// The raw-byte check's work, or `None` when it stopped the reading.
pub fn scan_work(bytes: &[u8]) -> Option<ScanWork> {
    let checked = preflight::check(bytes).ok()?;
    Some(ScanWork {
        elements: checked.elements,
        decode_bytes: checked.decode_work,
    })
}

/// Why reading stopped without text.
#[derive(Debug, Clone)]
pub(crate) struct Stop {
    status: TextStatus,
    note: String,
}

impl Stop {
    pub(crate) fn unsupported(note: impl Into<String>) -> Self {
        Self {
            status: TextStatus::Unsupported,
            note: note.into(),
        }
    }

    pub(crate) fn failed(note: impl Into<String>) -> Self {
        Self {
            status: TextStatus::Failed,
            note: note.into(),
        }
    }
}

/// The text, status and note of one exact set of PDF bytes: what
/// Organize's child process asks for. A pure function of the bytes and
/// `PDF_READER`.
pub(crate) fn read_pdf(bytes: &[u8]) -> TextExtraction {
    read_with_pages(bytes).extraction
}

/// Reads one exact set of PDF bytes, with the per-page table. A pure
/// function of the bytes and `PDF_READER`.
pub fn read_with_pages(bytes: &[u8]) -> PdfReading {
    match read(bytes) {
        Ok(reading) => reading,
        Err(stop) => PdfReading {
            extraction: without_text(stop.status, stop.note),
            pages: Vec::new(),
        },
    }
}

fn read(bytes: &[u8]) -> Result<PdfReading, Stop> {
    let checked = preflight::check(bytes)?;
    let mut budget = document::Budget::default();
    let facts = match panic::catch_unwind(AssertUnwindSafe(|| {
        document::read(bytes, &checked, &mut budget)
    })) {
        Ok(result) => result?,
        Err(payload) => {
            return Err(match payload.downcast::<document::LimitHit>() {
                Ok(hit) => {
                    Stop::unsupported(format!("The PDF passes this reader's limit on {}.", hit.0))
                }
                Err(_) => Stop::failed("The PDF library stopped on this file (internal error)."),
            });
        }
    };
    Ok(summarize(&facts, &checked))
}

fn summarize(facts: &document::DocumentFacts, checked: &preflight::Preflight) -> PdfReading {
    let mut text = String::new();
    let mut chars = 0usize;
    let mut pages = Vec::with_capacity(facts.pages.len());
    for (index, page) in facts.pages.iter().enumerate() {
        let number = index + 1;
        let marker = format!("[Page {number}]");
        if index > 0 {
            text.push('\n');
            chars += 1;
        }
        text.push_str(&marker);
        chars += marker.chars().count();
        let start = chars + usize::from(!page.lines.is_empty());
        for line in &page.lines {
            text.push('\n');
            text.push_str(&line.text);
            chars += 1 + line.text.chars().count();
        }
        let has_text = page
            .lines
            .iter()
            .any(|line| line.text.chars().any(|c| !c.is_whitespace()));
        let omissions = page_omissions(page, has_text);
        let state = match (has_text, omissions.is_empty()) {
            (true, true) => PageState::Text,
            (true, false) => PageState::TextWithOmissions,
            (false, _) => PageState::NoText,
        };
        pages.push(PageReading {
            page: number,
            state,
            start: if has_text { start } else { chars },
            end: chars,
            lines: page.lines.len(),
            glyphs: page.glyphs,
            rotated_lines: page.lines.iter().filter(|line| line.rotated).count(),
            invisible_chars: page.lines.iter().map(|line| line.invisible_chars).sum(),
            wide_gaps: page.lines.iter().map(|line| line.wide_gaps).sum(),
            omissions,
        });
    }

    let text_pages: Vec<usize> = pages
        .iter()
        .filter(|page| page.state != PageState::NoText)
        .map(|page| page.page)
        .collect();
    if text_pages.is_empty() {
        for page in &mut pages {
            page.start = 0;
            page.end = 0;
        }
        return PdfReading {
            extraction: without_text(TextStatus::Unsupported, no_text_note(&pages)),
            pages,
        };
    }

    // Cut at the shared limit; pages after the cut are not read.
    let mut cut_from = None;
    if chars > MAX_MEMORY_TEXT_CHARS {
        let at = text
            .char_indices()
            .nth(MAX_MEMORY_TEXT_CHARS)
            .map_or(text.len(), |(i, _)| i);
        text.truncate(at);
        for page in &mut pages {
            if page.end > MAX_MEMORY_TEXT_CHARS {
                if page.start >= MAX_MEMORY_TEXT_CHARS {
                    page.state = PageState::NotRead;
                    page.start = MAX_MEMORY_TEXT_CHARS;
                } else {
                    page.state = PageState::TextWithOmissions;
                }
                page.end = MAX_MEMORY_TEXT_CHARS;
                page.omissions.push(Omission::Truncated);
                cut_from.get_or_insert(page.page);
            }
        }
    }
    let kept = text.chars().count();

    let mut covered = vec![format!(
        "the text layer of {} in the order the PDF draws it: one line per text line, each page after a \"[Page N]\" line",
        match (text_pages.len(), pages.len()) {
            (1, 1) => "the page".to_string(),
            (k, n) if k == n => format!("all {n} pages"),
            (k, n) => format!("{} ({k} of {n} pages)", on_pages(&text_pages)),
        }
    )];
    let invisible: Vec<usize> = pages
        .iter()
        .filter(|p| p.invisible_chars > 0)
        .map(|p| p.page)
        .collect();
    if !invisible.is_empty() {
        covered.push(format!(
            "invisible text on {} (often an OCR layer over a scan; kept as is, its accuracy is not checked)",
            on_pages(&invisible)
        ));
    }
    let rotated: Vec<usize> = pages
        .iter()
        .filter(|p| p.rotated_lines > 0)
        .map(|p| p.page)
        .collect();
    if !rotated.is_empty() {
        covered.push(format!(
            "rotated text on {}, each rotated line kept as its own line",
            on_pages(&rotated)
        ));
    }

    let (mut left_out, mut kinds) = page_left_out(&pages);
    let widgets: usize = facts.pages.iter().map(|page| page.widgets).sum();
    if facts.form_fields > 0 && widgets == 0 {
        left_out.push(format!(
            "a form with {} fields (their values are not read)",
            facts.form_fields
        ));
        kinds.push("form fields");
    }
    if facts.xfa {
        left_out.push("an XFA form (its data and layout are not read)".to_string());
        kinds.push("XFA form");
    }
    if facts.javascript {
        left_out.push("JavaScript (not run and not read)".to_string());
        kinds.push("JavaScript");
    }
    if facts.embedded_files {
        left_out.push("embedded files (not opened)".to_string());
        kinds.push("embedded files");
    }
    if facts.hidden_layers {
        left_out.push("content in optional layers that are hidden by default".to_string());
        kinds.push("hidden layers");
    }
    if checked.external_streams > 0 {
        left_out.push("content stored in external files (not fetched)".to_string());
        kinds.push("external content");
    }
    if let Some(page) = cut_from {
        left_out.push(format!(
            "the last {} characters of the text (from page {page} on)",
            chars - MAX_MEMORY_TEXT_CHARS
        ));
        kinds.push("text past the length limit");
    }

    let (status, note) = if left_out.is_empty() {
        (TextStatus::Complete, None)
    } else {
        (
            TextStatus::Partial,
            Some(format!(
                "Not all of this PDF is in the text. Left out: {}.",
                kinds.join(", ")
            )),
        )
    };
    PdfReading {
        extraction: TextExtraction {
            summary: TextSummary {
                reader: PDF_READER.to_string(),
                status,
                chars: kept,
                covered,
                left_out,
                note,
            },
            text,
        },
        pages,
    }
}

fn page_omissions(page: &document::PageFacts, has_text: bool) -> Vec<Omission> {
    let mut omissions = Vec::new();
    let cover_percent = (page.image_cover * 100.0).round() as u32;
    if !has_text && page.images > 0 && page.image_cover >= IMAGE_PAGE_COVER {
        omissions.push(Omission::ImagePage { cover_percent });
    } else if page.images > 0 {
        omissions.push(Omission::Images {
            count: page.images,
            cover_percent,
        });
    }
    if page.unmapped > 0 {
        omissions.push(Omission::Unmapped {
            count: page.unmapped,
        });
    }
    if page.undecoded_text {
        omissions.push(Omission::UndecodedText);
    }
    if page.grid {
        omissions.push(Omission::Grid);
    }
    // On a page with text, a few shapes are underlines and boxes; ten or
    // more curved or slanted paths are taken to be a chart or diagram.
    if page.drawings >= 10 || (!has_text && page.drawings > 0) {
        omissions.push(Omission::Drawings {
            count: page.drawings,
        });
    }
    if page.math {
        omissions.push(Omission::Math);
    }
    if page.annotations > 0 {
        omissions.push(Omission::Annotations {
            count: page.annotations,
        });
    }
    if page.widgets > 0 {
        omissions.push(Omission::FormFields {
            count: page.widgets,
        });
    }
    if page.outside > 0 {
        omissions.push(Omission::Outside {
            count: page.outside,
        });
    }
    omissions
}

/// One `left_out` line per kind of omission, with the pages it is on.
fn page_left_out(pages: &[PageReading]) -> (Vec<String>, Vec<&'static str>) {
    // kind order → (pages, summed count)
    let mut found: BTreeMap<u8, (Vec<usize>, usize)> = BTreeMap::new();
    for page in pages {
        for omission in &page.omissions {
            let (order, count) = match omission {
                Omission::ImagePage { .. } => (0, 1),
                Omission::Images { count, .. } => (1, *count),
                Omission::Unmapped { count } => (2, *count),
                Omission::UndecodedText => (3, 0),
                Omission::Grid => (4, 0),
                Omission::Drawings { count } => (5, *count),
                Omission::Math => (6, 0),
                Omission::Annotations { count } => (7, *count),
                Omission::FormFields { count } => (8, *count),
                Omission::Outside { count } => (9, *count),
                Omission::Truncated => continue,
            };
            let entry = found.entry(order).or_default();
            entry.0.push(page.page);
            entry.1 += count;
        }
    }
    let mut lines = Vec::new();
    let mut kinds = Vec::new();
    for (order, (on, count)) in found {
        let list = on_pages(&on);
        let (line, kind) = match order {
            0 => (
                format!(
                    "{list}: no text layer; mostly an image (likely a scan) that would need OCR, which this reader does not do"
                ),
                "scanned or image-only pages",
            ),
            1 => (
                format!("images on {list} ({count} in all; images are not read, no OCR)"),
                "images",
            ),
            2 => (
                format!("{count} characters with no Unicode mapping on {list}"),
                "characters without Unicode",
            ),
            3 => (
                format!("text in fonts this reader could not decode on {list}"),
                "undecoded text",
            ),
            4 => (
                format!(
                    "ruled lines forming a table or chart grid on {list}: its text is kept in drawing order, its rows and columns are not"
                ),
                "table structure",
            ),
            5 => (
                format!(
                    "vector drawings (charts, diagrams or shapes) on {list}: any text in them is kept, the shapes are not"
                ),
                "drawings",
            ),
            6 => (
                format!(
                    "mathematical notation on {list}: its symbols are kept as characters, its layout (fractions, sub- and superscripts, radicals) is not"
                ),
                "formula layout",
            ),
            7 => (
                format!(
                    "annotations on {list} ({count}: comments, highlights, stamps or attachments; not read)"
                ),
                "annotations",
            ),
            8 => (
                format!("form fields on {list} ({count}; their values are not read)"),
                "form fields",
            ),
            _ => (
                format!("{count} characters drawn outside the visible page area on {list}"),
                "text outside the page",
            ),
        };
        lines.push(line);
        kinds.push(kind);
    }
    (lines, kinds)
}

fn no_text_note(pages: &[PageReading]) -> String {
    let has = |wanted: fn(&Omission) -> bool| {
        pages
            .iter()
            .filter(|page| page.omissions.iter().any(wanted))
            .count()
    };
    let image_pages = has(|o| matches!(o, Omission::ImagePage { .. }));
    if image_pages > 0 {
        return format!(
            "No page has a text layer that this reader can use. {image_pages} of {} pages are mostly images (likely scans); reading them needs OCR, which this reader does not do.",
            pages.len()
        );
    }
    if has(|o| matches!(o, Omission::Unmapped { .. })) > 0 {
        return "The PDF's characters have no Unicode mapping, so it has no reliable text."
            .to_string();
    }
    if has(|o| matches!(o, Omission::UndecodedText)) > 0 {
        return "The PDF's text is in fonts this reader could not decode.".to_string();
    }
    if has(|o| matches!(o, Omission::Images { .. })) > 0 {
        return "No page has a text layer; the PDF holds only images and drawings.".to_string();
    }
    "The PDF has no text to read: its pages draw no text.".to_string()
}

/// "page 4", "pages 1-3, 5"
fn on_pages(pages: &[usize]) -> String {
    if pages.len() == 1 {
        format!("page {}", pages[0])
    } else {
        format!("pages {}", page_list(pages))
    }
}

/// "1-3, 5"
fn page_list(pages: &[usize]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let mut j = i;
        while j + 1 < pages.len() && pages[j + 1] == pages[j] + 1 {
            j += 1;
        }
        parts.push(if i == j {
            pages[i].to_string()
        } else {
            format!("{}-{}", pages[i], pages[j])
        });
        i = j + 1;
    }
    parts.join(", ")
}

fn without_text(status: TextStatus, note: impl Into<String>) -> TextExtraction {
    TextExtraction {
        summary: TextSummary {
            reader: PDF_READER.to_string(),
            status,
            chars: 0,
            covered: Vec::new(),
            left_out: Vec::new(),
            note: Some(note.into()),
        },
        text: String::new(),
    }
}

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod tests;
