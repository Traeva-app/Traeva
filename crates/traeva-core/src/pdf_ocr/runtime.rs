//! Child-only wire protocol for OCR fallback. The parent provides its PID
//! and supervises elapsed time and actual process memory.

use std::io::{Read, Write};

use serde::Serialize;

use crate::memory_contracts::TextStatus;
use crate::observe::MAX_FILE_BYTES;
use crate::reader::MAX_MEMORY_TEXT_CHARS;

use super::{
    OcrReading, Settings, Source, child_parent_watchdog, prepare_ocr, read_scanned_document,
};

struct ChildOptions {
    settings: Settings,
    parent_pid: u32,
    /// `--input image`: the bytes are one PNG or JPEG image, not a PDF.
    image: bool,
    /// `--output jpeg`: the answer is pictures for the model, not text.
    jpeg: bool,
}

fn parse_args(args: &[String], prepare: bool) -> Option<ChildOptions> {
    let settings = Settings::default();
    let mut parent_pid = None;
    let mut image = false;
    let mut jpeg = false;
    let mut seen = 0u8;
    let (pairs, remainder) = args.as_chunks::<2>();
    for pair in pairs {
        let (bit, valid) = match pair[0].as_str() {
            "--parent-pid" => {
                parent_pid = pair[1]
                    .parse::<u32>()
                    .ok()
                    .filter(|pid| *pid > 1 && *pid <= i32::MAX as u32);
                (1, parent_pid.is_some())
            }
            "--max-pages" if !prepare => (
                2,
                pair[1].parse::<usize>().ok() == Some(settings.max_ocr_pages),
            ),
            "--dpi" if !prepare => (4, pair[1].parse::<f32>().ok() == Some(settings.dpi)),
            "--max-pixels" if !prepare => (
                8,
                pair[1].parse::<usize>().ok() == Some(settings.max_pixels),
            ),
            "--input" if !prepare => {
                image = pair[1] == "image";
                (16, image)
            }
            "--output" if !prepare => {
                jpeg = pair[1] == "jpeg";
                (32, jpeg)
            }
            _ => return None,
        };
        if !valid || seen & bit != 0 {
            return None;
        }
        seen |= bit;
    }
    if !remainder.is_empty() {
        return None;
    }
    Some(ChildOptions {
        settings,
        parent_pid: parent_pid?,
        image,
        jpeg,
    })
}

#[derive(Debug, Serialize)]
struct Answer {
    status: TextStatus,
    text: String,
    note: Option<String>,
}

fn refusal(status: TextStatus, note: impl Into<String>) -> Answer {
    Answer {
        status,
        text: String::new(),
        note: Some(note.into()),
    }
}

fn read_input(source: impl Read) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    source
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(());
    }
    Ok(bytes)
}

fn page_ranges(pages: &[usize]) -> String {
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < pages.len() {
        let first = pages[index];
        let mut last = first;
        index += 1;
        while index < pages.len() && last.checked_add(1) == Some(pages[index]) {
            last = pages[index];
            index += 1;
        }
        ranges.push(if first == last {
            first.to_string()
        } else {
            format!("{first}-{last}")
        });
    }
    let all = ranges.join(", ");
    if all.chars().count() <= 48 {
        all
    } else {
        format!("{}...", all.chars().take(45).collect::<String>())
    }
}

fn success_note(reading: &OcrReading, ocr_pages: usize) -> String {
    let omitted: Vec<usize> = reading
        .pages
        .iter()
        .filter(|page| {
            matches!(
                page.source,
                Source::NotReadPageLimit | Source::OcrTooLittle | Source::None
            )
        })
        .map(|page| page.page)
        .collect();
    let mut note = format!(
        "OCR read {ocr_pages} scanned pages; may contain errors. {} pages omitted. Other reader omissions, if any, remain.",
        omitted.len()
    );
    if !omitted.is_empty() {
        note.push_str(&format!(" Omitted pages: {}.", page_ranges(&omitted)));
    }
    if let Some(reader_note) = &reading.reader.note {
        note.push_str(" Reader before OCR: ");
        note.push_str(reader_note);
    }
    if note.chars().count() > 300 {
        note = note.chars().take(297).collect();
        note.push_str("...");
    }
    note
}

fn answer_from_reading(reading: OcrReading) -> Answer {
    if reading.status == "reader_refused" || reading.pages.is_empty() {
        let status = if reading.reader.status == TextStatus::Failed {
            TextStatus::Failed
        } else {
            TextStatus::Unsupported
        };
        return refusal(
            status,
            reading
                .reader
                .note
                .unwrap_or_else(|| "The PDF reader refused this input.".into()),
        );
    }
    if reading.status == "failed"
        || reading
            .pages
            .iter()
            .any(|page| page.source == Source::Failed)
    {
        return refusal(
            TextStatus::Failed,
            "A scanned page could not be rendered or recognized; all OCR output was discarded.",
        );
    }
    if reading.status == "text_layer_refused"
        || reading
            .pages
            .iter()
            .any(|page| page.source == Source::TextLayer)
    {
        return refusal(
            TextStatus::Unsupported,
            "OCR fallback requires a document with no usable text-layer pages.",
        );
    }
    let ocr_pages = reading
        .pages
        .iter()
        .filter(|page| page.source == Source::Ocr)
        .count();
    if ocr_pages == 0 || ocr_pages > 3 || !reading.usable_text || reading.status != "ocr" {
        return refusal(
            TextStatus::Unsupported,
            "No scanned page produced usable OCR text. Text-layer reading uses the ordinary PDF reader.",
        );
    }
    let mut text = String::new();
    let mut previous_page = 0;
    for page in reading
        .pages
        .iter()
        .filter(|page| page.source == Source::Ocr)
    {
        if page.page <= previous_page {
            return refusal(
                TextStatus::Failed,
                "OCR page identifiers must be positive and strictly increasing; all output was discarded.",
            );
        }
        previous_page = page.page;
        if !page
            .lines
            .iter()
            .any(|line| line.text.chars().any(|c| !c.is_whitespace()))
        {
            return refusal(
                TextStatus::Failed,
                "An OCR page contained no usable lines; all output was discarded.",
            );
        }
        if page.lines.iter().any(|line| {
            line.text
                .split(['\n', '\r'])
                .any(|part| part.trim_start().starts_with("[Page "))
        }) {
            return refusal(
                TextStatus::Failed,
                "OCR text contains an ambiguous page marker; all output was discarded.",
            );
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("[Page {}]", page.page));
        for line in &page.lines {
            text.push('\n');
            text.push_str(&line.text);
        }
    }
    if text.chars().count() > MAX_MEMORY_TEXT_CHARS {
        return refusal(
            TextStatus::Unsupported,
            "OCR text exceeds the one-million-character limit; all output was discarded.",
        );
    }
    let note = success_note(&reading, ocr_pages);
    Answer {
        status: TextStatus::Partial,
        text,
        note: Some(note),
    }
}

fn write_answer(mut output: impl Write, answer: &Answer) -> i32 {
    if serde_json::to_writer(&mut output, answer).is_ok() && output.flush().is_ok() {
        0
    } else {
        3
    }
}

/// The answer for one PNG or JPEG image: partial with the recognized lines
/// (no page markers), or why there is no text.
#[cfg(target_os = "macos")]
fn image_answer(bytes: &[u8], settings: &Settings) -> Answer {
    match super::image::image_text(bytes, settings) {
        Ok(text) => Answer {
            status: TextStatus::Partial,
            text,
            note: Some("OCR read one image; may contain errors.".into()),
        },
        Err((status, note)) => refusal(status, note),
    }
}

/// The answer with pictures for the model (`--output jpeg`): partial, the
/// JPEGs as standard base64, one per line in page order, and no note; or
/// why there are none, with a fixed note.
#[cfg(target_os = "macos")]
fn jpeg_answer(bytes: &[u8], options: &ChildOptions) -> Answer {
    let pictures = if options.image {
        super::model_image::image_jpeg(bytes).map(|picture| vec![picture])
    } else {
        super::model_image::scanned_pdf_jpegs(bytes, &options.settings)
    };
    match pictures {
        Ok(pictures) => Answer {
            status: TextStatus::Partial,
            text: pictures
                .iter()
                .map(|picture| super::jpeg::base64_encode(picture))
                .collect::<Vec<_>>()
                .join("\n"),
            note: None,
        },
        Err((status, note)) => refusal(status, note),
    }
}

/// Reads at most 20 MiB from stdin. Success writes exactly one JSON object
/// with status/text/note; successful OCR is always partial. Only documents
/// without usable text-layer pages qualify, with at most three OCR sections;
/// with `--input image`, the bytes are one PNG or JPEG image instead. With
/// `--output jpeg`, the same input gives pictures for the model instead of
/// text (`jpeg_answer`).
/// Requires --parent-pid; optional resource flags must equal approved defaults.
pub fn ocr_child_main(args: &[String]) -> i32 {
    crate::pdf_child::set_heap_cap(512 << 20);
    let Some(options) = parse_args(args, false) else {
        return 2;
    };
    if child_parent_watchdog(options.parent_pid).is_err() {
        return 2;
    }
    let Ok(bytes) = read_input(std::io::stdin().lock()) else {
        return 2;
    };
    #[cfg(target_os = "macos")]
    let answer = if options.jpeg {
        jpeg_answer(&bytes, &options)
    } else if options.image {
        image_answer(&bytes, &options.settings)
    } else {
        answer_from_reading(read_scanned_document(&bytes, &options.settings))
    };
    #[cfg(not(target_os = "macos"))]
    let answer = refusal(
        TextStatus::Unsupported,
        "Local OCR is supported only on macOS.",
    );
    drop(bytes);
    write_answer(std::io::stdout().lock(), &answer)
}

/// The separate preparation entrypoint never reads stdin or user files.
/// Success writes only {"status":"ready"}; failure exits without a body.
/// This completion does not promise that OS caches survive another launch.
pub fn ocr_prepare_main(args: &[String]) -> i32 {
    crate::pdf_child::set_heap_cap(512 << 20);
    let Some(options) = parse_args(args, true) else {
        return 2;
    };
    if child_parent_watchdog(options.parent_pid).is_err() {
        return 2;
    }
    if prepare_ocr(&options.settings).is_err() {
        return 2;
    }
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(br#"{"status":"ready"}"#).is_ok() && stdout.flush().is_ok() {
        0
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Line, Page, ReaderView, Timing};
    use super::*;

    fn reading(sources: &[Source]) -> OcrReading {
        OcrReading {
            status: if sources.contains(&Source::TextLayer) {
                "mixed"
            } else {
                "ocr"
            },
            usable_text: true,
            text: "[Page 1]\nOriginal layer\n[Page 2]\nRecognized scan".into(),
            note: None,
            left_out: Vec::new(),
            reader: ReaderView {
                reader: "pdf-1".into(),
                status: TextStatus::Partial,
                note: None,
                pages: sources.len(),
            },
            pages: sources
                .iter()
                .enumerate()
                .map(|(index, source)| Page {
                    page: index + 1,
                    source: *source,
                    chars: 20,
                    lines: if *source == Source::Ocr {
                        vec![Line {
                            text: format!("Recognized scan {}", index + 1),
                            confidence: 1.0,
                            bbox: [0.0; 4],
                        }]
                    } else {
                        Vec::new()
                    },
                    render_ms: None,
                    ocr_ms: None,
                    pixels: None,
                    error: None,
                })
                .collect(),
            timing: Timing {
                reader_ms: 0,
                render_ms: 0,
                ocr_ms: 0,
                total_ms: 0,
            },
            settings: Settings::default(),
        }
    }

    #[test]
    fn mixed_document_is_refused_without_returning_layer_or_scan_text() {
        let answer = answer_from_reading(reading(&[Source::TextLayer, Source::Ocr]));
        assert_eq!(answer.status, TextStatus::Unsupported);
        assert!(answer.text.is_empty());
    }

    #[test]
    fn successful_wire_has_three_fields_and_only_real_ocr_page_sections() {
        let mut input = reading(&[
            Source::Ocr,
            Source::None,
            Source::Ocr,
            Source::NotReadPageLimit,
            Source::Ocr,
            Source::NotReadPageLimit,
        ]);
        input.text = "[Page 99]\nDiagnostic text must not enter the wire".into();
        let answer = answer_from_reading(input);
        assert_eq!(answer.status, TextStatus::Partial);
        assert_eq!(
            answer.text,
            "[Page 1]\nRecognized scan 1\n[Page 3]\nRecognized scan 3\n[Page 5]\nRecognized scan 5"
        );
        assert_eq!(answer.text.matches("[Page ").count(), 3);
        let value = serde_json::to_value(answer).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 3);
        assert!(
            object.contains_key("status")
                && object.contains_key("text")
                && object.contains_key("note")
        );
    }

    #[test]
    fn empty_or_excess_ocr_page_results_cannot_emit_empty_or_extra_sections() {
        let mut empty = reading(&[Source::Ocr]);
        empty.pages[0].lines.clear();
        let empty_answer = answer_from_reading(empty);
        assert_eq!(empty_answer.status, TextStatus::Failed);
        assert!(empty_answer.text.is_empty());
        let excess = answer_from_reading(reading(&[Source::Ocr; 4]));
        assert_eq!(excess.status, TextStatus::Unsupported);
        assert!(excess.text.is_empty());
    }

    #[test]
    fn zero_duplicate_or_descending_ocr_page_ids_discard_all_text() {
        for page_ids in [[0, 2, 3], [1, 1, 3], [1, 3, 2]] {
            let mut input = reading(&[Source::Ocr; 3]);
            for (page, id) in input.pages.iter_mut().zip(page_ids) {
                page.page = id;
            }
            let answer = answer_from_reading(input);
            assert_eq!(answer.status, TextStatus::Failed, "page IDs: {page_ids:?}");
            assert!(answer.text.is_empty());
            assert!(answer.note.unwrap().contains("strictly increasing"));
        }
    }

    #[test]
    fn standalone_or_nested_ocr_page_markers_discard_other_good_pages() {
        for text in [
            "[Page 2]",
            " \t[Page 2] OCR body",
            "Safe text\n[Page 2]\nNested body",
            "Safe text\r\n\t[Page 2]",
            "Safe text\r[Page 2]",
            "Safe text\n\u{2003}[Page ",
        ] {
            let mut input = reading(&[Source::Ocr; 2]);
            input.pages[1].lines[0].text = text.into();
            let answer = answer_from_reading(input);
            assert_eq!(answer.status, TextStatus::Failed, "OCR text: {text:?}");
            assert!(answer.text.is_empty());
            assert!(answer.note.unwrap().contains("ambiguous page marker"));
        }
    }

    #[test]
    fn inline_marker_mentions_preserve_the_original_ocr_text() {
        let mut input = reading(&[Source::Ocr]);
        input.pages[0].lines[0].text = "The literal [Page 2] is discussed here.\nNext line.".into();
        let answer = answer_from_reading(input);
        assert_eq!(answer.status, TextStatus::Partial);
        assert_eq!(
            answer.text,
            "[Page 1]\nThe literal [Page 2] is discussed here.\nNext line."
        );
    }

    #[test]
    fn a_single_failed_page_discards_good_ocr_and_layer_text() {
        let answer =
            answer_from_reading(reading(&[Source::TextLayer, Source::Ocr, Source::Failed]));
        assert_eq!(answer.status, TextStatus::Failed);
        assert!(answer.text.is_empty());
    }

    #[test]
    fn text_layer_only_or_too_little_ocr_cannot_claim_ocr_success() {
        for sources in [
            &[Source::TextLayer][..],
            &[Source::TextLayer, Source::OcrTooLittle][..],
        ] {
            let answer = answer_from_reading(reading(sources));
            assert_eq!(answer.status, TextStatus::Unsupported);
            assert!(answer.text.is_empty());
        }
    }

    #[test]
    fn text_limit_counts_unicode_scalars_and_refuses_instead_of_truncating() {
        let mut input = reading(&[Source::Ocr]);
        input.pages[0].lines[0].text =
            "文".repeat(MAX_MEMORY_TEXT_CHARS - "[Page 1]\n".chars().count());
        assert_eq!(
            answer_from_reading(input.clone()).status,
            TextStatus::Partial
        );
        input.pages[0].lines[0].text.push('文');
        let answer = answer_from_reading(input);
        assert_eq!(answer.status, TextStatus::Unsupported);
        assert!(answer.text.is_empty());
    }

    #[test]
    fn input_accepts_exactly_twenty_mib_and_rejects_the_next_byte() {
        assert_eq!(MAX_FILE_BYTES, 20 << 20);
        assert_eq!(
            read_input(std::io::repeat(0).take(MAX_FILE_BYTES))
                .unwrap()
                .len() as u64,
            MAX_FILE_BYTES
        );
        assert!(read_input(std::io::repeat(0).take(MAX_FILE_BYTES + 1)).is_err());
    }

    #[test]
    fn note_keeps_ocr_risk_omission_count_and_caveats_before_long_details() {
        let mut input = reading(&[Source::Ocr, Source::NotReadPageLimit, Source::OcrTooLittle]);
        input.reader.note = Some(format!(
            "Left out: form fields, annotations, unmapped text. {}",
            "Original omission detail ".repeat(100)
        ));
        let answer = answer_from_reading(input);
        let note = answer.note.unwrap();
        assert!(note.starts_with("OCR read 1 scanned pages; may contain errors. 2 pages omitted."));
        for detail in [
            "form fields",
            "annotations",
            "unmapped text",
            "Omitted pages: 2-3",
            "Reader before OCR",
            "Other reader omissions, if any, remain",
        ] {
            assert!(note.contains(detail));
        }
        assert_eq!(note.chars().count(), 300);
    }

    #[test]
    fn reader_refusal_preserves_failure_kind_without_text() {
        let mut input = reading(&[]);
        input.status = "reader_refused";
        input.reader.status = TextStatus::Failed;
        input.reader.note = Some("Damaged PDF.".into());
        let answer = answer_from_reading(input);
        assert_eq!(answer.status, TextStatus::Failed);
        assert_eq!(answer.note.as_deref(), Some("Damaged PDF."));
        assert!(answer.text.is_empty());
    }

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn the_image_input_is_one_flag_of_the_recognition_child_only() {
        let image = arguments(&[
            "--parent-pid",
            "123",
            "--input",
            "image",
            "--max-pages",
            "3",
        ]);
        assert!(parse_args(&image, false).is_some_and(|options| options.image));
        assert!(parse_args(&arguments(&["--parent-pid", "123"]), false).is_some_and(|o| !o.image));
        assert!(
            parse_args(
                &arguments(&["--parent-pid", "123", "--input", "image"]),
                true
            )
            .is_none()
        );
    }

    #[test]
    fn the_jpeg_output_combines_with_either_input_of_the_recognition_child_only() {
        let options = |values: &[&str], prepare: bool| parse_args(&arguments(values), prepare);
        let pdf = options(&["--parent-pid", "123", "--output", "jpeg"], false).unwrap();
        assert!(pdf.jpeg && !pdf.image, "a scanned PDF's pages");
        let image = options(
            &[
                "--parent-pid",
                "123",
                "--input",
                "image",
                "--output",
                "jpeg",
            ],
            false,
        )
        .unwrap();
        assert!(image.jpeg && image.image, "one image");
        // In any order, with the approved resource values.
        let with_budget = options(
            &[
                "--output",
                "jpeg",
                "--max-pages",
                "3",
                "--dpi",
                "200",
                "--max-pixels",
                "8000000",
                "--parent-pid",
                "123",
                "--input",
                "image",
            ],
            false,
        )
        .unwrap();
        assert!(with_budget.jpeg && with_budget.image);
        // Without it, text as before.
        assert!(!options(&["--parent-pid", "123"], false).unwrap().jpeg);
        for invalid in [
            &["--parent-pid", "123", "--output", "png"][..],
            &["--parent-pid", "123", "--output", "JPEG"],
            &["--parent-pid", "123", "--output", "text"],
            &["--parent-pid", "123", "--output", ""],
            &["--parent-pid", "123", "--output"],
            &[
                "--parent-pid",
                "123",
                "--output",
                "jpeg",
                "--output",
                "jpeg",
            ],
            &[
                "--parent-pid",
                "123",
                "--output",
                "jpeg",
                "--input",
                "image",
                "--output",
                "jpeg",
            ],
            &["--output", "jpeg"],
            &["--parent-pid", "123", "--output", "jpeg", "--dpi", "300"],
        ] {
            assert!(options(invalid, false).is_none(), "accepted {invalid:?}");
        }
        // The preparation child takes no output.
        assert!(options(&["--parent-pid", "123", "--output", "jpeg"], true).is_none());
    }

    #[test]
    fn child_arguments_require_parent_and_only_approved_resource_values() {
        let valid = arguments(&[
            "--parent-pid",
            "123",
            "--max-pages",
            "3",
            "--dpi",
            "200",
            "--max-pixels",
            "8000000",
        ]);
        assert!(parse_args(&valid, false).is_some());
        assert!(parse_args(&arguments(&["--parent-pid", "123"]), true).is_some());
        assert!(parse_args(&valid, true).is_none());
        for invalid in [
            vec![],
            vec!["--parent-pid"],
            vec!["--parent-pid", "1"],
            vec!["--parent-pid", "4294967295"],
            vec!["--parent-pid", "123", "--parent-pid", "123"],
            vec![
                "--parent-pid",
                "123",
                "--max-pages",
                "3",
                "--max-pages",
                "3",
            ],
            vec!["--parent-pid", "123", "--max-pages", "4"],
            vec!["--parent-pid", "123", "--max-pages", "2"],
            vec!["--parent-pid", "123", "--dpi", "NaN"],
            vec!["--parent-pid", "123", "--dpi", "300"],
            vec!["--parent-pid", "123", "--max-pixels", "8000001"],
            vec!["--parent-pid", "123", "--auto-language", "true"],
            vec!["--parent-pid", "123", "--input", "pdf"],
            vec![
                "--parent-pid",
                "123",
                "--input",
                "image",
                "--input",
                "image",
            ],
            vec!["--parent-pid", "123", "--input"],
        ] {
            assert!(
                parse_args(&arguments(&invalid), false).is_none(),
                "accepted {invalid:?}"
            );
        }
    }
}
