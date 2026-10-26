//! Unregistered projection candidate; shared wiring and v23 grammar belong to A/B.

use super::{Error, ErrorKind, Limits, Value, Workbook, read_xlsx};
use crate::{MAX_MEMORY_TEXT_CHARS, SnapshotRef, TextExtraction, TextStatus, TextSummary};
use crate::{escape_source_line, structure_name};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub use crate::{XLSX_MEMORY_READER as XLSX_READER, XLSX_PREVIEW_READER};
pub const XLSX_PROJECTION_REVISION: &str = "f-xlsx-projection-candidate-3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionMode {
    Memory,
    Preview,
}

impl ProjectionMode {
    fn reader(self) -> &'static str {
        match self {
            Self::Memory => XLSX_READER,
            Self::Preview => XLSX_PREVIEW_READER,
        }
    }
}

pub fn read_xlsx_memory(bytes: &[u8]) -> TextExtraction {
    read_projection(bytes, ProjectionMode::Memory)
}

pub fn read_xlsx_preview(bytes: &[u8]) -> TextExtraction {
    read_projection(bytes, ProjectionMode::Preview)
}

/// Caller supplies core's already-verified copy; this check is not a snapshot store.
pub fn project_verified_snapshot(
    snapshot: &SnapshotRef,
    bytes: &[u8],
    mode: ProjectionMode,
) -> Result<(SnapshotRef, TextExtraction), &'static str> {
    if snapshot.byte_length != bytes.len() as u64 {
        return Err("snapshot_length_mismatch");
    }
    if bytes.len() > Limits::default().input_bytes {
        return Err("input_size_limit");
    }
    let hash = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if snapshot.sha256 != hash {
        return Err("snapshot_hash_mismatch");
    }
    Ok((snapshot.clone(), read_projection(bytes, mode)))
}

fn read_projection(bytes: &[u8], mode: ProjectionMode) -> TextExtraction {
    match read_xlsx(bytes, &Limits::default()) {
        Ok(workbook) => project_workbook(&workbook, mode, MAX_MEMORY_TEXT_CHARS),
        Err(error) => rejected(mode, error),
    }
}

fn without_text(mode: ProjectionMode, status: TextStatus, note: String) -> TextExtraction {
    TextExtraction {
        summary: TextSummary {
            reader: mode.reader().into(),
            status,
            chars: 0,
            covered: Vec::new(),
            left_out: Vec::new(),
            note: Some(note),
        },
        text: String::new(),
    }
}

fn rejected(mode: ProjectionMode, error: Error) -> TextExtraction {
    let status = match error.kind {
        ErrorKind::Failed => TextStatus::Failed,
        ErrorKind::Unsupported | ErrorKind::ResourceLimit => TextStatus::Unsupported,
    };
    without_text(
        mode,
        status,
        format!("XLSX rejected ({:?}: {}).", error.kind, error.code),
    )
}

fn value_text(value: &Value) -> (&'static str, String) {
    match value {
        Value::Number(s) => ("number", s.clone()),
        Value::Text(s) => ("text", s.clone()),
        Value::Boolean(b) => ("boolean", b.to_string()),
        Value::Error(s) => ("error", s.clone()),
        Value::IsoDate(s) => ("date", s.clone()),
        Value::Blank => ("blank", String::new()),
    }
}

struct Writer {
    text: String,
    chars: usize,
    source_chars: usize,
    limit: usize,
    truncated: bool,
}

impl Writer {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            chars: 0,
            source_chars: 0,
            limit,
            truncated: false,
        }
    }

    fn remaining(&self) -> usize {
        self.limit - self.chars
    }

    fn label(&mut self, label: &str) -> bool {
        let count = label.chars().count() + 1;
        if count > self.remaining() {
            self.truncated = true;
            return false;
        }
        self.text.push_str(label);
        self.text.push('\n');
        self.chars += count;
        true
    }

    fn source(&mut self, source: &str) -> bool {
        let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
        for (line_index, line) in normalized.split('\n').enumerate() {
            if line_index > 0 {
                self.source_chars += 1;
            }
            let written = escape_source_line(line);
            let escape = written.len() != line.len();
            // Never leave a lone generated escape without the first source scalar.
            if self.remaining() < if escape { 2 } else { 1 } {
                self.truncated = true;
                return false;
            }
            for (index, ch) in written.chars().enumerate() {
                if self.remaining() == 0 {
                    self.truncated = true;
                    return false;
                }
                self.text.push(ch);
                self.chars += 1;
                if !(escape && index == 0) {
                    self.source_chars += 1;
                }
            }
            if self.remaining() == 0 {
                self.truncated = true;
                return false;
            }
            self.text.push('\n');
            self.chars += 1;
        }
        true
    }

    fn field(&mut self, label: &str, source: Option<&str>) -> bool {
        self.label(label) && source.is_none_or(|s| self.source(s))
    }
}

fn project_workbook(workbook: &Workbook, mode: ProjectionMode, limit: usize) -> TextExtraction {
    let mut omitted = BTreeMap::new();
    for finding in &workbook.omissions {
        *omitted.entry(finding.code.as_str()).or_insert(0usize) += 1;
    }
    let mut left_out: Vec<String> = omitted
        .into_iter()
        .map(|(code, n)| format!("Reader omission: {code} ({n} findings)."))
        .collect();
    let hidden = workbook
        .sheets
        .iter()
        .filter(|s| s.visibility != "visible")
        .count();
    if mode == ProjectionMode::Memory && hidden > 0 {
        left_out.push(format!(
            "Hidden/veryHidden sheets excluded from Memory ({hidden})."
        ));
    }
    let mut writer = Writer::new(limit);
    let mut merges = 0usize;
    let mut styles = 0usize;
    let mut formula_metadata = 0usize;
    'sheets: for (index, sheet) in workbook.sheets.iter().enumerate() {
        if mode == ProjectionMode::Memory && sheet.visibility != "visible" {
            continue;
        }
        merges += sheet.merged_ranges.len();
        styles += sheet
            .cells
            .iter()
            .filter(|c| c.style_index.is_some())
            .count();
        formula_metadata += sheet
            .cells
            .iter()
            .filter(|c| c.formula.as_ref().is_some_and(|f| !f.attributes.is_empty()))
            .count();
        let visibility = if sheet.visibility == "visible" {
            String::new()
        } else {
            format!(
                " · {}",
                if sheet.visibility == "veryHidden" {
                    "very hidden"
                } else {
                    &sheet.visibility
                }
            )
        };
        if !writer.label(&format!(
            "⟦sheet {} · id {} · name {}{}⟧",
            index + 1,
            sheet.sheet_id.parse::<u32>().expect("validated sheet ID"),
            structure_name(&sheet.name),
            visibility
        )) {
            break;
        }
        let mut cells: Vec<_> = sheet.cells.iter().collect();
        cells.sort_by_key(|c| (c.row, c.column));
        for cell in cells {
            if let Some(value) = &cell.literal
                && !matches!(value, Value::Blank)
            {
                let (kind, value) = value_text(value);
                if !writer.field(
                    &format!("⟦cell {} · literal · {kind}⟧", cell.address),
                    Some(&value),
                ) {
                    break 'sheets;
                }
            }
            if let Some(formula) = &cell.formula {
                if !writer.field(
                    &format!("⟦cell {} · formula⟧", cell.address),
                    formula.text.as_deref(),
                ) {
                    break 'sheets;
                }
                let cache = cell
                    .cached_result
                    .as_ref()
                    .and_then(|cache| cache.value.as_ref());
                let (kind, value) = cache.map(value_text).unwrap_or(("missing", String::new()));
                if !writer.field(
                    &format!("⟦cell {} · cache · {kind} · not verified⟧", cell.address),
                    cache
                        .filter(|v| !matches!(v, Value::Blank))
                        .map(|_| value.as_str()),
                ) {
                    break 'sheets;
                }
            }
        }
    }
    for (n, description) in [
        (merges, "Merged ranges"),
        (styles, "Stored style indices"),
        (
            formula_metadata,
            "Formula attributes not expanded or projected",
        ),
    ] {
        if n > 0 {
            left_out.push(format!("{description} ({n})."));
        }
    }
    if writer.truncated {
        left_out.push("Projection remainder beyond the Unicode scalar limit.".into());
    }
    if writer.source_chars == 0 {
        return without_text(
            mode,
            TextStatus::Unsupported,
            "XLSX has no text in the selected projection (no_visible_projection).".into(),
        );
    }
    let partial = !left_out.is_empty();
    TextExtraction {
        summary: TextSummary {
            reader: mode.reader().into(),
            status: if partial { TextStatus::Partial } else { TextStatus::Complete },
            chars: writer.chars,
            covered: vec![if mode == ProjectionMode::Memory { "Visible worksheets" }
                else { "Worksheets including marked hidden/veryHidden sheets" }.into(),
                "Generated sheet/A1/type labels; stored literals, formula text and separate saved caches (freshness unverified); part paths remain internal.".into()],
            left_out,
            note: partial.then(|| "Only the selected stored-cell projection was read; see left_out. Cache freshness is unverified. Formatting, logical row alignment and business meaning are not interpreted.".into()),
        },
        text: writer.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_never_cuts_a_label_or_exposes_a_source_label() {
        let mut writer = Writer::new(7);
        assert!(!writer.label("⟦sheet 1⟧"));
        assert!(writer.text.is_empty());
        let mut writer = Writer::new(4);
        assert!(!writer.source("⟦🙂more"));
        assert_eq!(writer.text, "\\⟦🙂m");
        assert_eq!(writer.chars, 4);
        assert!(!writer.text.starts_with('⟦'));
        let mut writer = Writer::new(1);
        assert!(!writer.source("\\source"));
        assert!(writer.text.is_empty());
    }
}
