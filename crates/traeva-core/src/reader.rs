//! Memory readers (#65): one exact set of bytes → `TextExtraction`. Readers
//! are pure functions of (file name, bytes). They never open a file, so
//! they only ever see bytes that were already read stably and hashed.
//!
//! - .txt / .md / .markdown: here, decoded exactly like Organize
//!   (`observe::decode_text`).
//! - .docx: `docx.rs` (#66, Codex-D), the main body of a Word document, read
//!   from the bytes in memory with resource limits; what it leaves out is
//!   listed. Old Word versions are still not opened (`opened.rs`).
//!   Organize reads .docx, .pptx and .xlsx with these readers too, since the
//!   student's decision of 10/17 (`pdf_child.rs`, `office_extraction`).
//! - Anything else: `Unsupported`.
//!
//! Structure lines (contract §4.4, v23): the PPTX and XLSX readers put
//! lines of their own, like `⟦slide 3 · id 257⟧`, between the document's
//! text. `structure_line` tells them apart, for those readers only.

use crate::contracts::ExtractionStatus;
use crate::memory_contracts::{TextExtraction, TextStatus, TextSummary};
use crate::observe;

/// Longest text any reader keeps, in Unicode scalar values. Longer texts
/// are cut there and reported as `Partial`.
pub const MAX_MEMORY_TEXT_CHARS: usize = 1_000_000;

/// Raise the number when the plain-text reader starts producing different
/// text from the same bytes.
pub const PLAIN_TEXT_READER: &str = "txt-md-1";

/// The reader name when no reader handles the file type.
pub const NO_READER: &str = "none";

/// The readers whose output holds structure lines (contract §4.4, v23):
/// what Memory compares and sends, and what only the local preview shows.
pub const PPTX_MEMORY_READER: &str = "pptx-1";
pub const PPTX_PREVIEW_READER: &str = "pptx-preview-1";
pub const XLSX_MEMORY_READER: &str = "xlsx-1";
pub const XLSX_PREVIEW_READER: &str = "xlsx-preview-1";

/// A line a PPTX or XLSX reader wrote itself, not the document's text. It
/// says where the lines under it come from; it is never evidence of what
/// the document says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureLine {
    /// `⟦slide 3 · id 257⟧`: the third slide in this reading, whose
    /// `sldId` is 257. `hidden` only in the preview.
    Slide { number: u32, id: u32, hidden: bool },
    /// `⟦title⟧`, `⟦text⟧`, `⟦notes⟧`: what the lines under it are on the
    /// slide above.
    Role(SlideRole),
    /// `⟦sheet 1 · id 1 · name "F Data"⟧`: the first sheet as declared,
    /// whose `sheetId` is 1. Hidden sheets only in the preview.
    Sheet {
        number: u32,
        id: u32,
        name: String,
        visibility: SheetVisibility,
    },
    /// `⟦cell C2 · literal · number⟧`, `⟦cell C2 · formula⟧`,
    /// `⟦cell C2 · cache · number · not verified⟧`: one field of a cell. A
    /// formula has no value type; a cache is always "not verified". A cache
    /// that is empty is `Blank`; a formula with no cache in the file at all
    /// (`cache · missing`) has none. Neither has a value line under it.
    Cell {
        address: String,
        field: CellField,
        value_type: Option<CellValueType>,
    },
    /// A line that starts with `⟦` in these readers' output but follows
    /// none of the forms above. Source lines starting with `⟦` are escaped,
    /// so it is still the reader's own line, never the document's text.
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideRole {
    Title,
    Text,
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetVisibility {
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellField {
    Literal,
    Formula,
    Cache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellValueType {
    Number,
    Text,
    Boolean,
    Error,
    Date,
    /// An empty cached value; never a literal (a cell with no value has
    /// no literal line).
    Blank,
}

/// Whether `reader`'s output holds structure lines (and escapes source
/// lines). TXT/MD and docx-2 do not: a `⟦…⟧` line there is the user's text.
pub fn has_structure_lines(reader: &str) -> bool {
    matches!(
        reader,
        PPTX_MEMORY_READER | PPTX_PREVIEW_READER | XLSX_MEMORY_READER | XLSX_PREVIEW_READER
    )
}

/// The structure line `line` is, in output of `reader`; None for the
/// document's own lines and for every other reader. The whole line is
/// taken as it is, without trimming.
/// `text` with each sheet line of an Excel reading written without the
/// sheet's name (`⟦sheet 2 · id 2⟧`): a sheet's name is the writer's text and
/// is never sent to a model (contract §4.4 v25). Other readers' text is
/// returned as it is.
pub(crate) fn without_sheet_names(reader: &str, text: &str) -> String {
    if reader != XLSX_MEMORY_READER {
        return text.to_string();
    }
    text.split('\n')
        .map(|line| match structure_line(reader, line) {
            Some(StructureLine::Sheet { number, id, .. }) => format!("⟦sheet {number} · id {id}⟧"),
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn structure_line(reader: &str, line: &str) -> Option<StructureLine> {
    if !has_structure_lines(reader) || !line.starts_with('⟦') {
        return None;
    }
    let preview = matches!(reader, PPTX_PREVIEW_READER | XLSX_PREVIEW_READER);
    let parsed = line
        .strip_prefix('⟦')
        .and_then(|rest| rest.strip_suffix('⟧'))
        .map(|inner| inner.split(" · ").collect::<Vec<_>>())
        .and_then(|fields| {
            if reader.starts_with("pptx") {
                slide_line(&fields, preview)
            } else {
                sheet_line(&fields, preview)
            }
        });
    Some(parsed.unwrap_or(StructureLine::Unrecognized))
}

fn slide_line(fields: &[&str], preview: bool) -> Option<StructureLine> {
    match fields {
        ["title"] => Some(StructureLine::Role(SlideRole::Title)),
        ["text"] => Some(StructureLine::Role(SlideRole::Text)),
        ["notes"] => Some(StructureLine::Role(SlideRole::Notes)),
        [slide, id, rest @ ..] => {
            let hidden = match rest {
                [] => false,
                ["hidden"] if preview => true,
                _ => return None,
            };
            Some(StructureLine::Slide {
                number: counted(slide.strip_prefix("slide ")?)?,
                id: counted(id.strip_prefix("id ")?)?,
                hidden,
            })
        }
        _ => None,
    }
}

fn sheet_line(fields: &[&str], preview: bool) -> Option<StructureLine> {
    match fields {
        [cell, field, rest @ ..] if cell.starts_with("cell ") => {
            let address = cell.strip_prefix("cell ")?;
            if !is_cell_address(address) {
                return None;
            }
            let (field, value_type) = match (*field, rest) {
                ("literal", [kind]) => (CellField::Literal, Some(value_type(kind)?)),
                ("formula", []) => (CellField::Formula, None),
                ("cache", ["missing", "not verified"]) => (CellField::Cache, None),
                ("cache", ["blank", "not verified"]) => {
                    (CellField::Cache, Some(CellValueType::Blank))
                }
                ("cache", [kind, "not verified"]) => (CellField::Cache, Some(value_type(kind)?)),
                _ => return None,
            };
            Some(StructureLine::Cell {
                address: address.to_string(),
                field,
                value_type,
            })
        }
        [sheet, id, name, rest @ ..] => {
            let visibility = match rest {
                [] => SheetVisibility::Visible,
                ["hidden"] if preview => SheetVisibility::Hidden,
                ["very hidden"] if preview => SheetVisibility::VeryHidden,
                _ => return None,
            };
            Some(StructureLine::Sheet {
                number: counted(sheet.strip_prefix("sheet ")?)?,
                id: counted(id.strip_prefix("id ")?)?,
                name: serde_json::from_str(name.strip_prefix("name ")?).ok()?,
                visibility,
            })
        }
        _ => None,
    }
}

/// A positive number written without sign or leading zeros.
fn counted(text: &str) -> Option<u32> {
    let plain =
        !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) && !text.starts_with('0');
    plain.then(|| text.parse().ok()).flatten()
}

/// `A1` to `XFD1048576` in shape: 1–3 capital letters, then a row number.
fn is_cell_address(text: &str) -> bool {
    let letters = text.bytes().take_while(u8::is_ascii_uppercase).count();
    (1..=3).contains(&letters) && counted(&text[letters..]).is_some()
}

fn value_type(text: &str) -> Option<CellValueType> {
    Some(match text {
        "number" => CellValueType::Number,
        "text" => CellValueType::Text,
        "boolean" => CellValueType::Boolean,
        "error" => CellValueType::Error,
        "date" => CellValueType::Date,
        _ => return None,
    })
}

/// A source line as these readers write it: one starting with `⟦` or `\`
/// gets a `\` in front, so only the reader's own lines start with `⟦`.
pub fn escape_source_line(line: &str) -> String {
    if line.starts_with('⟦') || line.starts_with('\\') {
        format!("\\{line}")
    } else {
        line.to_string()
    }
}

/// The source line as the user wrote it, for showing: the escape taken
/// off again. Positions (`QuoteAt`) still count the reader's output.
pub fn unescape_source_line<'a>(reader: &str, line: &'a str) -> &'a str {
    match line.strip_prefix('\\') {
        Some(rest)
            if has_structure_lines(reader) && (rest.starts_with('⟦') || rest.starts_with('\\')) =>
        {
            rest
        }
        _ => line,
    }
}

/// A sheet name inside a structure line: a JSON string in which `"`, `\`,
/// control characters, `⟦`, `⟧` and `·` are all `\uXXXX`, so neither the
/// field separator nor the closing `⟧` can turn up inside it.
pub fn structure_name(name: &str) -> String {
    let mut out = String::from("\"");
    for c in name.chars() {
        if c == '"' || c == '\\' || c.is_control() || matches!(c, '⟦' | '⟧' | '·') {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Picks the reader by the file name's extension.
pub fn read_text(file_name: &str, bytes: &[u8]) -> TextExtraction {
    if observe::is_text_file_name(file_name) {
        return read_plain_text(file_name, bytes);
    }
    if is_docx_file_name(file_name) {
        return crate::docx::read_docx(bytes);
    }
    if has_extension(file_name, "pptx") {
        return crate::pptx::read_pptx_memory(bytes);
    }
    if has_extension(file_name, "xlsx") {
        return crate::xlsx::read_xlsx_memory(bytes);
    }
    no_reader()
}

/// Memory has a reader for this file name: TXT, Markdown, Word,
/// PowerPoint or Excel (never PDF, which only Organize reads).
pub(crate) fn has_reader(file_name: &str) -> bool {
    observe::is_text_file_name(file_name)
        || is_docx_file_name(file_name)
        || has_extension(file_name, "pptx")
        || has_extension(file_name, "xlsx")
}

/// What any file type without a reader gets.
pub(crate) fn no_reader() -> TextExtraction {
    without_text(
        NO_READER,
        TextStatus::Unsupported,
        "Traeva has no reader for this file type.",
    )
}

/// `name.docx`, by the same rule as text names (any letter case, a name
/// before the dot).
fn is_docx_file_name(name: &str) -> bool {
    has_extension(name, "docx")
}

/// `name.<extension>` in any letter case, with a name before the dot.
fn has_extension(name: &str, extension: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, found)) if !stem.is_empty() => found.eq_ignore_ascii_case(extension),
        _ => false,
    }
}

fn read_plain_text(file_name: &str, bytes: &[u8]) -> TextExtraction {
    let text = match observe::decode_text(file_name, bytes) {
        Ok(text) => text,
        Err((status, note)) => {
            let status = match status {
                ExtractionStatus::Failed => TextStatus::Failed,
                ExtractionStatus::Unsupported => TextStatus::Unsupported,
                // decode_text never fails with these; if it ever did, call
                // it broken rather than guess.
                ExtractionStatus::Ok | ExtractionStatus::Partial => TextStatus::Failed,
            };
            return without_text(PLAIN_TEXT_READER, status, note);
        }
    };
    let total = text.chars().count();
    if total <= MAX_MEMORY_TEXT_CHARS {
        return TextExtraction {
            summary: TextSummary {
                reader: PLAIN_TEXT_READER.to_string(),
                status: TextStatus::Complete,
                chars: total,
                covered: vec!["the whole text".to_string()],
                left_out: Vec::new(),
                note: None,
            },
            text,
        };
    }
    let cut = text
        .char_indices()
        .nth(MAX_MEMORY_TEXT_CHARS)
        .map_or(text.len(), |(index, _)| index);
    TextExtraction {
        summary: TextSummary {
            reader: PLAIN_TEXT_READER.to_string(),
            status: TextStatus::Partial,
            chars: MAX_MEMORY_TEXT_CHARS,
            covered: vec![format!("the first {MAX_MEMORY_TEXT_CHARS} characters")],
            left_out: vec![format!(
                "the last {} characters",
                total - MAX_MEMORY_TEXT_CHARS
            )],
            note: Some(format!(
                "The text has {total} characters; Traeva keeps at most {MAX_MEMORY_TEXT_CHARS}."
            )),
        },
        text: text[..cut].to_string(),
    }
}

fn without_text(reader: &str, status: TextStatus, note: &str) -> TextExtraction {
    TextExtraction {
        summary: TextSummary {
            reader: reader.to_string(),
            status,
            chars: 0,
            covered: Vec::new(),
            left_out: Vec::new(),
            note: Some(note.to_string()),
        },
        text: String::new(),
    }
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
