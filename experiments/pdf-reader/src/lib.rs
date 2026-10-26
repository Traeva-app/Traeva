//! TRAEVA PDF Reader experiment (#28, Claude-G). Not registered in
//! traeva-core, Memory, Organize or Tauri.
//!
//! `read_pdf(bytes)` turns one exact set of PDF bytes into the shared
//! Memory text contract (copied in `contract.rs`) plus a per-page table.
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
//! 5. `quality`: diagnostics about the reading (what is left out, what may
//!    be wrong, why it stopped). They never change the text.

mod contract;
mod document;
mod layout;
mod preflight;
mod quality;
mod scan;

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};

use serde::Serialize;

pub use contract::{MAX_MEMORY_TEXT_CHARS, TextExtraction, TextStatus, TextSummary};
use quality::Finding;
pub use quality::{Code, Diagnostic, Level, Quality, ReadingOrder};

/// Raise the number when the same bytes start producing different output
/// or the status means something else.
/// 3: the fixes for R4's findings on 07ce381 (stream location, structural
/// trailer, JavaScript and attachment checks, content lists, zlib
/// checksums). Number 2 is the layout experiment on another branch (PR
/// #149), which this version does not include.
/// 4: the quality report. The text, status, `covered`, `left_out`, note
/// and page table are the same as 3's.
/// 5 (PR #148, without the quality report): the fixes for R4's
/// second-round findings on e2d836f: text that looks like an object no
/// longer decides the status, and /Contents that refers to a null object
/// is a blank page.
/// 6: 5 with the quality report. The text, status, `covered`, `left_out`,
/// note and page table are the same as 5's.
/// 7 (PR #148, without the quality report): the fixes for R4's third-round
/// findings on c3a28cf: a stream without filters is held to the per-stream
/// limit, a stream's range follows hayro's rule and is checked again after
/// hayro reads it, the raw-byte check grows linearly with the file, and
/// limits on predictor rows and object-stream offset tables.
/// 8: 7 with the quality report. The text, status, `covered`, `left_out`,
/// note and page table are the same as 7's.
/// 9 (PR #148, without the quality report): the fixes for R4's
/// fourth-round finding on 8b0971b: a filter list many dictionaries share
/// is stored once, not copied for each; and a length object's possible
/// values are kept once each, at most MAX_LENGTH_VALUES.
/// 10: 9 with the quality report. The text, status, `covered`, `left_out`,
/// note and page table are the same as 9's.
pub const PDF_READER: &str = "pdf-experiment-10";

pub const MAX_PDF_BYTES: usize = 32 << 20;
pub const MAX_OBJECTS: usize = 200_000;
pub const MAX_PAGES: usize = 500;
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

/// The shared text result, plus where each page is in the text, what was
/// left out on it, and the quality report. The page table and the quality
/// report are this experiment's own: the shared contract has no field for
/// them.
#[derive(Debug, Clone)]
pub struct PdfReading {
    pub extraction: TextExtraction,
    pub pages: Vec<PageReading>,
    pub quality: Quality,
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
    reason: Code,
    note: String,
}

impl Stop {
    pub(crate) fn unsupported(reason: Code, note: impl Into<String>) -> Self {
        Self {
            status: TextStatus::Unsupported,
            reason,
            note: note.into(),
        }
    }

    pub(crate) fn failed(reason: Code, note: impl Into<String>) -> Self {
        Self {
            status: TextStatus::Failed,
            reason,
            note: note.into(),
        }
    }
}

/// Reads one exact set of PDF bytes. A pure function of the bytes and
/// `PDF_READER`.
pub fn read_pdf(bytes: &[u8]) -> PdfReading {
    let (extraction, pages, diagnostics) = match read(bytes) {
        Ok(read) => read,
        Err(stop) => {
            let stopped = quality::diagnostic(stop.reason, Vec::new(), stop.note.clone());
            (
                without_text(stop.status, stop.note),
                Vec::new(),
                vec![stopped],
            )
        }
    };
    let quality = quality::report(bytes, extraction.summary.status, diagnostics);
    PdfReading {
        extraction,
        pages,
        quality,
    }
}

/// The quality report for a reading stopped from outside the reader (the
/// probe's isolated process: a time or memory limit, or a crash).
pub fn stopped_quality(bytes: &[u8], reason: Code, note: &str) -> Quality {
    let stopped = quality::diagnostic(reason, Vec::new(), note.to_string());
    quality::report(bytes, TextStatus::Failed, vec![stopped])
}

type Read = (TextExtraction, Vec<PageReading>, Vec<Diagnostic>);

fn read(bytes: &[u8]) -> Result<Read, Stop> {
    let checked = preflight::check(bytes)?;
    let mut budget = document::Budget::default();
    let facts = match panic::catch_unwind(AssertUnwindSafe(|| {
        document::read(bytes, &checked, &mut budget)
    })) {
        Ok(result) => result?,
        Err(payload) => {
            return Err(match payload.downcast::<document::LimitHit>() {
                Ok(hit) => Stop::unsupported(
                    Code::ResourceLimit,
                    format!("The PDF passes this reader's limit on {}.", hit.0),
                ),
                Err(_) => Stop::failed(
                    Code::InternalError,
                    "The PDF library stopped on this file (internal error).",
                ),
            });
        }
    };
    Ok(summarize(&facts, &checked))
}

fn summarize(facts: &document::DocumentFacts, checked: &preflight::Preflight) -> Read {
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
        let (reason, note) = no_text_note(&pages);
        let every_page = pages.iter().map(|page| page.page).collect();
        let stopped = quality::diagnostic(reason, every_page, note.clone());
        return (
            without_text(TextStatus::Unsupported, note),
            pages,
            vec![stopped],
        );
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

    let mut findings = page_findings(&pages);
    let widgets: usize = facts.pages.iter().map(|page| page.widgets).sum();
    let whole_file = |code, line: String, kind, basis: String| Finding {
        code,
        line,
        kind,
        pages: Vec::new(),
        basis,
    };
    if facts.form_fields > 0 && widgets == 0 {
        findings.push(whole_file(
            Code::FormFields,
            format!(
                "a form with {} fields (their values are not read)",
                facts.form_fields
            ),
            "form fields",
            format!(
                "the document's /AcroForm lists {} fields, and no page has a widget for them",
                facts.form_fields
            ),
        ));
    }
    if facts.xfa {
        findings.push(whole_file(
            Code::XfaForm,
            "an XFA form (its data and layout are not read)".to_string(),
            "XFA form",
            "the document's /AcroForm has an /XFA entry".to_string(),
        ));
    }
    if facts.javascript {
        findings.push(whole_file(
            Code::Javascript,
            "JavaScript (not run and not read)".to_string(),
            "JavaScript",
            "a parsed dictionary has a /JS or /JavaScript key, or /S /JavaScript".to_string(),
        ));
    }
    if facts.embedded_files {
        findings.push(whole_file(
            Code::EmbeddedFiles,
            "embedded files (not opened)".to_string(),
            "embedded files",
            "a parsed dictionary has an /EmbeddedFiles or /EF key, or /Type /EmbeddedFile"
                .to_string(),
        ));
    }
    if facts.hidden_layers {
        findings.push(whole_file(
            Code::HiddenLayers,
            "content in optional layers that are hidden by default".to_string(),
            "hidden layers",
            "the default optional content configuration (/OCProperties /D) turns layers off"
                .to_string(),
        ));
    }
    if checked.external_streams > 0 {
        findings.push(whole_file(
            Code::ExternalContent,
            "content stored in external files (not fetched)".to_string(),
            "external content",
            format!(
                "streams that take their data from an external file (/F): {}",
                checked.external_streams
            ),
        ));
    }
    if let Some(page) = cut_from {
        let cut: Vec<usize> = pages
            .iter()
            .filter(|p| p.omissions.contains(&Omission::Truncated))
            .map(|p| p.page)
            .collect();
        findings.push(Finding {
            code: Code::TruncatedText,
            line: format!(
                "the last {} characters of the text (from page {page} on)",
                chars - MAX_MEMORY_TEXT_CHARS
            ),
            kind: "text past the length limit",
            pages: cut,
            basis: format!(
                "the text came to {chars} characters; the shared limit is {MAX_MEMORY_TEXT_CHARS}"
            ),
        });
    }

    let (status, note) = if findings.is_empty() {
        (TextStatus::Complete, None)
    } else {
        let kinds: Vec<&str> = findings.iter().map(|finding| finding.kind).collect();
        (
            TextStatus::Partial,
            Some(format!(
                "Not all of this PDF is in the text. Left out: {}.",
                kinds.join(", ")
            )),
        )
    };
    let left_out = findings
        .iter()
        .map(|finding| finding.line.clone())
        .collect();
    let mut diagnostics: Vec<Diagnostic> = findings
        .into_iter()
        .map(|finding| quality::diagnostic(finding.code, finding.pages, finding.basis))
        .collect();
    diagnostics.extend(risks(facts, &pages));
    (
        TextExtraction {
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
        diagnostics,
    )
}

/// Suspected risks, on pages whose text is in the result.
fn risks(facts: &document::DocumentFacts, pages: &[PageReading]) -> Vec<Diagnostic> {
    let in_text =
        |page: &PageReading| matches!(page.state, PageState::Text | PageState::TextWithOmissions);
    let mut risks = Vec::new();
    let columns: Vec<(usize, quality::Columns)> = pages
        .iter()
        .zip(&facts.pages)
        .filter(|(page, _)| in_text(page))
        .filter_map(|(page, page_facts)| page_facts.columns.map(|found| (page.page, found)))
        .collect();
    if !columns.is_empty() {
        let basis = columns
            .iter()
            .map(|(page, found)| {
                format!(
                    "page {page}: on {} rows, text on both sides of the same gap (x {:.0} to {:.0} pt) is drawn one side right after the other",
                    found.rows, found.from, found.to
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        risks.push(quality::diagnostic(
            Code::InterleavedColumns,
            columns.iter().map(|(page, _)| *page).collect(),
            basis,
        ));
    }
    let invisible: Vec<&PageReading> = pages
        .iter()
        .filter(|page| in_text(page) && page.invisible_chars > 0)
        .collect();
    if !invisible.is_empty() {
        let count: usize = invisible.iter().map(|page| page.invisible_chars).sum();
        risks.push(quality::diagnostic(
            Code::InvisibleText,
            invisible.iter().map(|page| page.page).collect(),
            format!("characters drawn in text render mode 3 (invisible): {count}"),
        ));
    }
    risks
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
fn page_findings(pages: &[PageReading]) -> Vec<Finding> {
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
    let mut findings = Vec::new();
    for (order, (on, count)) in found {
        let list = on_pages(&on);
        let (code, line, kind, basis) = match order {
            0 => (
                Code::ImagePage,
                format!(
                    "{list}: no text layer; mostly an image (likely a scan) that would need OCR, which this reader does not do"
                ),
                "scanned or image-only pages",
                format!(
                    "{list}: no usable text drawn, and images cover at least {}% of the page",
                    (IMAGE_PAGE_COVER * 100.0) as u32
                ),
            ),
            1 => (
                Code::Images,
                format!("images on {list} ({count} in all; images are not read, no OCR)"),
                "images",
                format!("images drawn on {list}: {count}"),
            ),
            2 => (
                Code::UnmappedCharacters,
                format!("{count} characters with no Unicode mapping on {list}"),
                "characters without Unicode",
                format!("glyphs drawn whose font gives no usable Unicode value: {count}"),
            ),
            3 => (
                Code::UndecodedFonts,
                format!("text in fonts this reader could not decode on {list}"),
                "undecoded text",
                "the content shows text but no glyph came out, or the PDF library reported a font it does not support".to_string(),
            ),
            4 => (
                Code::TableGrid,
                format!(
                    "ruled lines forming a table or chart grid on {list}: its text is kept in drawing order, its rows and columns are not"
                ),
                "table structure",
                format!(
                    "at least 3 separate horizontal and 3 separate vertical straight lines drawn on {list}"
                ),
            ),
            5 => (
                Code::Drawings,
                format!(
                    "vector drawings (charts, diagrams or shapes) on {list}: any text in them is kept, the shapes are not"
                ),
                "drawings",
                format!(
                    "curved or slanted paths drawn on {list}: {count} (10 or more on a page with text, any on a page without)"
                ),
            ),
            6 => (
                Code::MathLayout,
                format!(
                    "mathematical notation on {list}: its symbols are kept as characters, its layout (fractions, sub- and superscripts, radicals) is not"
                ),
                "formula layout",
                format!(
                    "2 or more mathematical symbols in the text, or a TeX or OpenType math font, on {list}"
                ),
            ),
            7 => (
                Code::Annotations,
                format!(
                    "annotations on {list} ({count}: comments, highlights, stamps or attachments; not read)"
                ),
                "annotations",
                format!("annotations other than links and pop-ups in the /Annots of {list}: {count}"),
            ),
            8 => (
                Code::FormFields,
                format!("form fields on {list} ({count}; their values are not read)"),
                "form fields",
                format!("form field widgets in the /Annots of {list}: {count}"),
            ),
            _ => (
                Code::OutsidePage,
                format!("{count} characters drawn outside the visible page area on {list}"),
                "text outside the page",
                format!("glyphs drawn outside the page's box on {list}: {count}"),
            ),
        };
        findings.push(Finding {
            code,
            line,
            kind,
            pages: on,
            basis,
        });
    }
    findings
}

/// Why no page has text, with its reason code.
fn no_text_note(pages: &[PageReading]) -> (Code, String) {
    let has = |wanted: fn(&Omission) -> bool| {
        pages
            .iter()
            .filter(|page| page.omissions.iter().any(wanted))
            .count()
    };
    let image_pages = has(|o| matches!(o, Omission::ImagePage { .. }));
    if image_pages > 0 {
        return (
            Code::NoTextLayer,
            format!(
                "No page has a text layer that this reader can use. {image_pages} of {} pages are mostly images (likely scans); reading them needs OCR, which this reader does not do.",
                pages.len()
            ),
        );
    }
    if has(|o| matches!(o, Omission::Unmapped { .. })) > 0 {
        return (
            Code::NoUnicode,
            "The PDF's characters have no Unicode mapping, so it has no reliable text.".to_string(),
        );
    }
    if has(|o| matches!(o, Omission::UndecodedText)) > 0 {
        return (
            Code::UndecodableFonts,
            "The PDF's text is in fonts this reader could not decode.".to_string(),
        );
    }
    if has(|o| matches!(o, Omission::Images { .. })) > 0 {
        return (
            Code::NoTextLayer,
            "No page has a text layer; the PDF holds only images and drawings.".to_string(),
        );
    }
    (
        Code::NoText,
        "The PDF has no text to read: its pages draw no text.".to_string(),
    )
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
mod tests {
    use super::*;

    #[test]
    fn page_lists() {
        assert_eq!(page_list(&[1, 2, 3, 5, 7, 8]), "1-3, 5, 7-8");
        assert_eq!(page_list(&[4]), "4");
    }

    #[test]
    fn not_a_pdf_fails_without_text() {
        let reading = read_pdf(b"hello");
        assert_eq!(reading.extraction.summary.status, TextStatus::Failed);
        assert!(reading.extraction.text.is_empty());
        assert!(reading.extraction.summary.note.is_some());
    }
}
