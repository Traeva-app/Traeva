//! OCR worker implementation for scanned PDF pages (#211) and for one PNG or
//! JPEG image (`image`, #37). Product callers use the child-process runtime;
//! detailed readings remain available for migration tests. Vision confidence
//! is diagnostic only. The same child also prepares pictures for the model
//! (`model_image`, `jpeg`; tier 2): downscaled JPEGs without metadata.

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

mod jpeg;
mod memory;
mod runtime;

#[cfg(target_os = "macos")]
mod image;
#[cfg(target_os = "macos")]
mod model_image;
#[cfg(target_os = "macos")]
mod render;
#[cfg(target_os = "macos")]
mod vision;
#[cfg(not(target_os = "macos"))]
mod without_ocr;
#[cfg(not(target_os = "macos"))]
use without_ocr::{render, vision};

use crate::pdf;
pub(crate) use jpeg::{
    MODEL_IMAGE_MAX_BYTES, MODEL_IMAGE_MAX_COUNT, MODEL_IMAGE_MAX_SIDE, base64_decode,
    base64_encode, frame_size, is_bare_jpeg,
};
pub use memory::{ProcessMemory, child_parent_watchdog, process_memory};
pub use runtime::ocr_prepare_main as ocr_prepare_child_main;
pub use runtime::{ocr_child_main, ocr_prepare_main};

use pdf::{Omission, PageState};
pub use vision::{Line, VisionInfo, prepare_ocr, system_supported, vision_info};

/// The image kinds text recognition reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageKind {
    Png,
    Jpeg,
}

/// PNG or JPEG, by the file's first bytes; None for anything else.
pub(crate) fn image_kind(bytes: &[u8]) -> Option<ImageKind> {
    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.starts_with(&PNG_SIGNATURE) {
        Some(ImageKind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageKind::Jpeg)
    } else {
        None
    }
}

/// A completed synthetic Vision request, not a promise about future caches.
#[derive(Debug, Clone, Serialize)]
pub struct Preparation {
    pub synthetic_fixture: &'static str,
    pub pixels: (usize, usize),
    pub ocr_ms: u128,
    pub recognized_chars: usize,
    pub lines: Vec<Line>,
}

/// OCR parameters; the child runtime uses the approved defaults.
#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    /// Rendering resolution, in dots per inch.
    pub dpi: f32,
    /// A rendered page is scaled down to at most this many pixels.
    pub max_pixels: usize,
    /// Pages read by OCR, at most; later image pages are left out.
    pub max_ocr_pages: usize,
    /// Non-white-space characters a page needs for its OCR text to count.
    pub min_page_chars: usize,
    /// Vision's recognition languages, in order of preference.
    pub languages: Vec<String>,
    pub language_correction: bool,
    /// Let Vision detect the language (revision 3), the list then only
    /// helps.
    pub auto_language: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dpi: 200.0,
            max_pixels: 8_000_000,
            max_ocr_pages: 3,
            min_page_chars: 10,
            languages: vec!["zh-Hans".to_string(), "en-US".to_string()],
            language_correction: true,
            auto_language: true,
        }
    }
}

/// Where a page's text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The PDF's own text layer, as the core's reader read it.
    TextLayer,
    /// Recognized by Vision from the rendered page.
    Ocr,
    /// An image page past `max_ocr_pages`: not read.
    NotReadPageLimit,
    /// An image page that OCR read, but with fewer than `min_page_chars`.
    OcrTooLittle,
    /// Rendering or recognition failed on this page.
    Failed,
    /// No text layer and not an image page (blank, drawings): nothing read.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub page: usize,
    pub source: Source,
    /// Non-white-space characters of the page's text.
    pub chars: usize,
    /// OCR lines in Vision's order (empty for text-layer pages).
    pub lines: Vec<Line>,
    pub render_ms: Option<u128>,
    pub ocr_ms: Option<u128>,
    /// The rendered size, after any scaling down.
    pub pixels: Option<(usize, usize)>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReaderView {
    pub reader: String,
    pub status: crate::memory_contracts::TextStatus,
    pub note: Option<String>,
    pub pages: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Timing {
    pub reader_ms: u128,
    pub render_ms: u128,
    pub ocr_ms: u128,
    pub total_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrReading {
    /// text_layer, ocr, mixed, no_usable_text, reader_refused or failed.
    pub status: &'static str,
    /// Whether Organize could use the text (the first three statuses).
    pub usable_text: bool,
    pub text: String,
    pub note: Option<String>,
    /// Pages whose content is not in the text, and why.
    pub left_out: Vec<String>,
    pub reader: ReaderView,
    pub pages: Vec<Page>,
    pub timing: Timing,
    pub settings: Settings,
}

/// Worker diagnostic reading. It may retain good pages beside a failed page;
/// the runtime must discard all text when any page has `Source::Failed`.
pub fn read_with_ocr(bytes: &[u8], settings: &Settings) -> OcrReading {
    read_document(bytes, settings, false)
}

/// Product fallback accepts documents with no usable text-layer page.
/// Eligibility is checked after the first parse and before rendering or Vision.
fn read_scanned_document(bytes: &[u8], settings: &Settings) -> OcrReading {
    read_document(bytes, settings, true)
}

fn has_usable_text_layer(states: impl IntoIterator<Item = PageState>) -> bool {
    states
        .into_iter()
        .any(|state| matches!(state, PageState::Text | PageState::TextWithOmissions))
}

fn read_document(bytes: &[u8], settings: &Settings, scanned_only: bool) -> OcrReading {
    let started = Instant::now();
    let reading = pdf::read_with_pages(bytes);
    let summary = &reading.extraction.summary;
    let reader = ReaderView {
        reader: summary.reader.clone(),
        status: summary.status,
        note: summary.note.clone(),
        pages: reading.pages.len(),
    };
    let mut timing = Timing {
        reader_ms: started.elapsed().as_millis(),
        render_ms: 0,
        ocr_ms: 0,
        total_ms: 0,
    };
    if reading.pages.is_empty() {
        // The reader stopped before reading any page: nothing is rendered.
        timing.total_ms = started.elapsed().as_millis();
        return OcrReading {
            status: "reader_refused",
            usable_text: false,
            text: String::new(),
            note: summary.note.clone(),
            left_out: Vec::new(),
            reader,
            pages: Vec::new(),
            timing,
            settings: settings.clone(),
        };
    }
    if scanned_only && has_usable_text_layer(reading.pages.iter().map(|page| page.state)) {
        timing.total_ms = started.elapsed().as_millis();
        return OcrReading {
            status: "text_layer_refused",
            usable_text: false,
            text: String::new(),
            note: Some("OCR fallback requires a document with no usable text-layer pages.".into()),
            left_out: vec!["OCR was not attempted because a text-layer page is present.".into()],
            reader,
            pages: reading
                .pages
                .iter()
                .map(|page| {
                    empty_page(
                        page.page,
                        if matches!(page.state, PageState::Text | PageState::TextWithOmissions) {
                            Source::TextLayer
                        } else {
                            Source::None
                        },
                    )
                })
                .collect(),
            timing,
            settings: settings.clone(),
        };
    }

    let to_read = scanned_pages(&reading, settings.max_ocr_pages);

    let mut recognized = HashMap::new();
    render::for_each_rendered_page(
        bytes,
        &to_read,
        reading.pages.len(),
        settings,
        |number, rendered| {
            let page = match rendered {
                Ok(rendered) => {
                    timing.render_ms += rendered.millis;
                    let ocr_started = Instant::now();
                    let result = vision::recognize(&rendered.gray, settings);
                    let ocr_ms = ocr_started.elapsed().as_millis();
                    timing.ocr_ms += ocr_ms;
                    match result {
                        Ok(lines) => {
                            let chars = count_chars(lines.iter().map(|l| l.text.as_str()));
                            Page {
                                page: number,
                                source: if chars >= settings.min_page_chars {
                                    Source::Ocr
                                } else {
                                    Source::OcrTooLittle
                                },
                                chars,
                                lines,
                                render_ms: Some(rendered.millis),
                                ocr_ms: Some(ocr_ms),
                                pixels: Some((rendered.gray.width, rendered.gray.height)),
                                error: None,
                            }
                        }
                        Err(error) => {
                            failed_page(number, Some(rendered.millis), Some(ocr_ms), error)
                        }
                    }
                }
                Err(error) => failed_page(number, None, None, error),
            };
            recognized.insert(number, page);
        },
    );

    let text_layer = &reading.extraction.text;
    let mut text = String::new();
    let mut pages = Vec::with_capacity(reading.pages.len());
    let mut left_out = Vec::new();
    for page in &reading.pages {
        let number = page.page;
        text.push_str(&format!("[Page {number}]\n"));
        let entry = if let Some(read) = recognized.remove(&number) {
            match read.source {
                Source::Ocr => {
                    for line in &read.lines {
                        text.push_str(&line.text);
                        text.push('\n');
                    }
                }
                Source::OcrTooLittle => left_out.push(format!(
                    "page {number}: a scanned page where OCR recognized too little text"
                )),
                _ => left_out.push(format!("page {number}: a scanned page OCR could not read")),
            }
            read
        } else if is_image_page(page) {
            left_out.push(format!(
                "page {number}: a scanned page past the OCR page limit ({})",
                settings.max_ocr_pages
            ));
            empty_page(number, Source::NotReadPageLimit)
        } else if matches!(page.state, PageState::Text | PageState::TextWithOmissions) {
            let slice: String = text_layer
                .chars()
                .skip(page.start)
                .take(page.end - page.start)
                .collect();
            text.push_str(&slice);
            if !slice.is_empty() {
                text.push('\n');
            }
            Page {
                chars: count_chars([slice.as_str()]),
                ..empty_page(number, Source::TextLayer)
            }
        } else {
            if page.state == PageState::NotRead {
                left_out.push(format!(
                    "page {number}: past the text length limit, not read"
                ));
            }
            empty_page(number, Source::None)
        };
        pages.push(entry);
    }
    let text = text.trim_end_matches('\n').to_string();

    let count = |source: Source| pages.iter().filter(|p| p.source == source).count();
    let (ocr, layer, failed) = (
        count(Source::Ocr),
        count(Source::TextLayer),
        count(Source::Failed),
    );
    let status = match (ocr > 0, layer > 0) {
        (true, true) => "mixed",
        (true, false) => "ocr",
        (false, true) => "text_layer",
        (false, false) if failed > 0 && failed == to_read.len() => "failed",
        (false, false) => "no_usable_text",
    };
    let note = match status {
        "no_usable_text" if to_read.is_empty() => summary.note.clone(),
        "no_usable_text" => {
            Some("OCR recognized too little text on the scanned pages.".to_string())
        }
        "failed" => Some("The scanned pages could not be rendered or recognized.".to_string()),
        _ => None,
    };
    timing.total_ms = started.elapsed().as_millis();
    OcrReading {
        status,
        usable_text: matches!(status, "text_layer" | "ocr" | "mixed"),
        text,
        note,
        left_out,
        reader,
        pages,
        timing,
        settings: settings.clone(),
    }
}

/// A page with no text whose content is an image: a scanned page.
fn is_image_page(page: &pdf::PageReading) -> bool {
    page.state == PageState::NoText
        && page
            .omissions
            .iter()
            .any(|o| matches!(o, Omission::ImagePage { .. }))
}

/// The first `max` scanned pages, in page order: the pages text recognition
/// reads, and the pages sent to the model as pictures (`model_image`).
fn scanned_pages(reading: &pdf::PdfReading, max: usize) -> Vec<usize> {
    reading
        .pages
        .iter()
        .filter(|page| is_image_page(page))
        .map(|page| page.page)
        .take(max)
        .collect()
}

fn count_chars<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    texts
        .into_iter()
        .map(|t| t.chars().filter(|c| !c.is_whitespace()).count())
        .sum()
}

fn empty_page(page: usize, source: Source) -> Page {
    Page {
        page,
        source,
        chars: 0,
        lines: Vec::new(),
        render_ms: None,
        ocr_ms: None,
        pixels: None,
        error: None,
    }
}

fn failed_page(page: usize, render_ms: Option<u128>, ocr_ms: Option<u128>, error: String) -> Page {
    Page {
        render_ms,
        ocr_ms,
        error: Some(error),
        ..empty_page(page, Source::Failed)
    }
}

#[cfg(test)]
mod eligibility_tests {
    use super::*;

    #[test]
    fn any_usable_text_layer_rejects_a_document_before_ocr() {
        assert!(has_usable_text_layer([PageState::NoText, PageState::Text]));
        assert!(has_usable_text_layer([
            PageState::TextWithOmissions,
            PageState::NoText
        ]));
        assert!(!has_usable_text_layer([
            PageState::NoText,
            PageState::NotRead
        ]));
    }
}
