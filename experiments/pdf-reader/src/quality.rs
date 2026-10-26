//! Quality diagnostics: what a reading of these bytes is known to leave
//! out, what may be wrong with its text, and why reading stopped. They
//! never change the text, its order or the page offsets.
//!
//! - `ConfirmedOmission`: something the file holds that is not in the
//!   text; exactly the `left_out` entries, on the same pages.
//! - `SuspectedRisk`: the text is there, but a measured sign says it may
//!   be wrong: columns read across (`InterleavedColumns`), or text from an
//!   invisible layer (`InvisibleText`).
//! - `Stopped`: why there is no text, with one reason code.
//!
//! No diagnostic, and no lack of one, says the reading order was checked:
//! `reading_order` is always `DrawingOrderUnverified`.
//!
//! Columns read across (README, "质量诊断"): on upright lines, a gap of at
//! least 0.95 font size with text at least 6 font sizes wide on both sides,
//! where the two sides are separate text operations drawn one right after
//! the other on the same baseline (in one line, or in two consecutive
//! lines). A page is flagged when 3 or more rows have such a gap at the
//! same place. On a page with a ruled grid, a gap crossed by a vertical
//! rule is a cell border and does not count.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::layout::{Line, Piece};
use crate::{PDF_READER, TextStatus};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Quality {
    /// The reader whose text these findings are about.
    pub reader: String,
    /// SHA-256 (hex) of the exact bytes read: binds the findings to one
    /// snapshot of the file.
    pub input_sha256: String,
    /// The text has content (the status is `Complete` or `Partial`).
    pub text_available: bool,
    pub reading_order: ReadingOrder,
    pub diagnostics: Vec<Diagnostic>,
    /// What this reader never does, for any file. Not findings about this
    /// file.
    pub limitations: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingOrder {
    /// The text is in the order the PDF draws it. Nothing checked that a
    /// person would read it in that order.
    DrawingOrderUnverified,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    pub level: Level,
    pub code: Code,
    /// 1-based pages it is about; empty when it is about the whole file.
    pub pages: Vec<usize>,
    /// What it means for the text, for people.
    pub message: &'static str,
    /// What was found or measured that led to it.
    pub basis: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    ConfirmedOmission,
    SuspectedRisk,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    // Confirmed omissions (one per kind of `left_out` entry).
    ImagePage,
    Images,
    UnmappedCharacters,
    UndecodedFonts,
    TableGrid,
    Drawings,
    MathLayout,
    Annotations,
    FormFields,
    XfaForm,
    Javascript,
    EmbeddedFiles,
    HiddenLayers,
    ExternalContent,
    OutsidePage,
    TruncatedText,
    // Suspected risks.
    InterleavedColumns,
    InvisibleText,
    // Why reading stopped.
    NotAPdf,
    Truncated,
    Damaged,
    Encrypted,
    ResourceLimit,
    UnsupportedFeature,
    UnsupportedStructure,
    InternalError,
    NoTextLayer,
    NoUnicode,
    UndecodableFonts,
    NoText,
}

/// One `left_out` entry and what its confirmed-omission diagnostic needs.
pub(crate) struct Finding {
    pub code: Code,
    /// The `left_out` entry.
    pub line: String,
    /// Its name in the note ("Left out: ...").
    pub kind: &'static str,
    pub pages: Vec<usize>,
    pub basis: String,
}

pub(crate) const LIMITATIONS: [&str; 5] = [
    "The text is in the order the PDF draws it. This reader does not check or rebuild the reading order, so a reading without warnings is not a verified reading order.",
    "Having all of a page's characters does not mean they are in reading order.",
    "Images are not read, and there is no OCR.",
    "Table rows and columns are not recovered.",
    "The column check is a heuristic: it finds columns drawn row by row across a gap, can miss other wrong orders, and can warn about tables or forms that read correctly.",
];

pub(crate) fn report(bytes: &[u8], status: TextStatus, diagnostics: Vec<Diagnostic>) -> Quality {
    let digest = Sha256::digest(bytes);
    Quality {
        reader: PDF_READER.to_string(),
        input_sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
        text_available: matches!(status, TextStatus::Complete | TextStatus::Partial),
        reading_order: ReadingOrder::DrawingOrderUnverified,
        diagnostics,
        limitations: LIMITATIONS.to_vec(),
    }
}

pub(crate) fn diagnostic(code: Code, pages: Vec<usize>, basis: String) -> Diagnostic {
    let (level, message) = describe(code);
    Diagnostic {
        level,
        code,
        pages,
        message,
        basis,
    }
}

fn describe(code: Code) -> (Level, &'static str) {
    use Code::*;
    use Level::*;
    match code {
        ImagePage => (
            ConfirmedOmission,
            "These pages have no usable text layer and are mostly images (likely scans). Their content is not in the text; OCR is not supported.",
        ),
        Images => (
            ConfirmedOmission,
            "Images were not read and there is no OCR: any text or meaning in them is not in the text.",
        ),
        UnmappedCharacters => (
            ConfirmedOmission,
            "Some characters have no Unicode mapping and are missing from the text.",
        ),
        UndecodedFonts => (
            ConfirmedOmission,
            "Some text is in fonts this reader could not decode and is missing from the text.",
        ),
        TableGrid => (
            ConfirmedOmission,
            "Table text was extracted in drawing order. Its rows and columns were not recovered, so check which values belong together.",
        ),
        Drawings => (
            ConfirmedOmission,
            "Charts, diagrams or shapes were not read; only text drawn in them is kept.",
        ),
        MathLayout => (
            ConfirmedOmission,
            "Formula symbols were kept as characters, but fractions, sub- and superscripts and radicals were not, so formulas may read wrongly.",
        ),
        Annotations => (
            ConfirmedOmission,
            "Comments, highlights, stamps or attachments were not read.",
        ),
        FormFields => (ConfirmedOmission, "Form field values were not read."),
        XfaForm => (
            ConfirmedOmission,
            "The XFA form's data and layout were not read.",
        ),
        Javascript => (
            ConfirmedOmission,
            "The PDF's JavaScript was neither run nor read.",
        ),
        EmbeddedFiles => (
            ConfirmedOmission,
            "Files embedded in the PDF were not opened.",
        ),
        HiddenLayers => (
            ConfirmedOmission,
            "Content in layers that are hidden by default was not read.",
        ),
        ExternalContent => (
            ConfirmedOmission,
            "Content stored in external files was not fetched.",
        ),
        OutsidePage => (
            ConfirmedOmission,
            "Characters drawn outside the visible page area were left out.",
        ),
        TruncatedText => (
            ConfirmedOmission,
            "The text passed the length limit and was cut; the rest is not in the text.",
        ),
        InterleavedColumns => (
            SuspectedRisk,
            "Text was extracted, but these pages may have columns read across, so lines from different columns may alternate. The order was kept as drawn.",
        ),
        InvisibleText => (
            SuspectedRisk,
            "Some text is invisible on the page (often an OCR layer over a scan). It was kept as is; its accuracy is not checked.",
        ),
        NotAPdf => (Stopped, "The file is not a PDF. Nothing was read."),
        Truncated => (
            Stopped,
            "The PDF is cut short or its end is missing. Nothing was read.",
        ),
        Damaged => (Stopped, "The PDF is damaged. Nothing was read."),
        Encrypted => (
            Stopped,
            "The PDF is encrypted, and this reader does not open encrypted PDFs. Nothing was read.",
        ),
        ResourceLimit => (
            Stopped,
            "The PDF passes one of this reader's safety limits. Nothing was read.",
        ),
        UnsupportedFeature => (
            Stopped,
            "The PDF uses a feature this reader does not support. Nothing was read.",
        ),
        UnsupportedStructure => (
            Stopped,
            "The PDF is built in a way this reader cannot check safely. Nothing was read.",
        ),
        InternalError => (
            Stopped,
            "The PDF library stopped on this file. Nothing was read.",
        ),
        NoTextLayer => (
            Stopped,
            "The file has no usable text layer, and OCR is not supported.",
        ),
        NoUnicode => (
            Stopped,
            "The file's characters have no Unicode mapping, so it has no reliable text.",
        ),
        UndecodableFonts => (
            Stopped,
            "The file's text is in fonts this reader could not decode.",
        ),
        NoText => (Stopped, "The file draws no text."),
    }
}

/// A vertical straight line drawn on the page (points, y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rule {
    pub x: f64,
    pub top: f64,
    pub bottom: f64,
}

/// Rows of one page whose text sits on both sides of the same gap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Columns {
    pub rows: usize,
    /// The part of the gap all those rows share, in points from the left
    /// edge of the page.
    pub from: f64,
    pub to: f64,
}

const MIN_GAP: f64 = 0.95;
const MIN_PIECE: f64 = 6.0;
const SAME_ROW: f64 = 0.5;
const MIN_ROWS: usize = 3;

/// A gap between two pieces of text on one row, drawn one right after the
/// other.
#[derive(Debug, Clone, Copy)]
struct Gap {
    row: f64,
    from: f64,
    to: f64,
    size: f64,
}

pub(crate) fn interleaved_columns(lines: &[Line], rules: &[Rule], grid: bool) -> Option<Columns> {
    let mut gaps = gaps(lines);
    if grid {
        gaps.retain(|gap| {
            !rules.iter().any(|rule| {
                (gap.from..=gap.to).contains(&rule.x)
                    && (rule.top - gap.size..=rule.bottom + gap.size).contains(&gap.row)
            })
        });
    }
    if gaps.len() < MIN_ROWS {
        return None;
    }
    let mut sizes: Vec<f64> = gaps.iter().map(|gap| gap.size).collect();
    sizes.sort_by(f64::total_cmp);
    let width = MIN_GAP * sizes[sizes.len() / 2];

    // A strip `width` wide starting at x lies in a gap when x is in
    // [from, to - width]. Rows: baselines within half a font size of the
    // previous one. Each row's spans are merged, so a row counts once.
    gaps.sort_by(|a, b| a.row.total_cmp(&b.row));
    let mut spans: Vec<(usize, f64, f64)> = Vec::new();
    let mut row = 0;
    for (i, gap) in gaps.iter().enumerate() {
        if i > 0 && gap.row - gaps[i - 1].row > SAME_ROW * gap.size {
            row += 1;
        }
        if gap.to - width >= gap.from {
            spans.push((row, gap.from, gap.to - width));
        }
    }
    spans.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut merged: Vec<(usize, f64, f64)> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if last.0 == span.0 && span.1 <= last.2 => last.2 = last.2.max(span.2),
            _ => merged.push(span),
        }
    }
    // Starts before ends at the same x: the spans are closed.
    let mut events: Vec<(f64, i32)> = merged
        .iter()
        .flat_map(|&(_, from, to)| [(from, 1), (to, -1)])
        .collect();
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
    let (mut count, mut best, mut at) = (0, 0, 0.0);
    for (x, step) in events {
        count += step;
        if count > best {
            best = count;
            at = x;
        }
    }
    let rows = usize::try_from(best).unwrap_or(0);
    if rows < MIN_ROWS {
        return None;
    }
    let shared = gaps
        .iter()
        .filter(|gap| gap.from <= at && at <= gap.to - width);
    Some(Columns {
        rows,
        from: shared.clone().map(|gap| gap.from).fold(f64::MIN, f64::max),
        to: shared.map(|gap| gap.to).fold(f64::MAX, f64::min),
    })
}

/// Gaps inside upright lines (between pieces), and between two
/// consecutive upright lines on the same baseline that do not overlap.
fn gaps(lines: &[Line]) -> Vec<Gap> {
    let mut found = Vec::new();
    for line in lines.iter().filter(|line| !line.rotated) {
        for pair in line.pieces.windows(2) {
            push_gap(&mut found, line.baseline, line.size, pair[0], pair[1]);
        }
    }
    for pair in lines.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let size = a.size.max(b.size);
        if a.rotated || b.rotated || (a.baseline - b.baseline).abs() > SAME_ROW * size {
            continue;
        }
        let (Some((a_left, a_right)), Some((b_left, b_right))) = (ends(a), ends(b)) else {
            continue;
        };
        if b_left.start >= a_right.end {
            push_gap(&mut found, a.baseline, size, a_right, b_left);
        } else if a_left.start >= b_right.end {
            push_gap(&mut found, a.baseline, size, b_right, a_left);
        }
    }
    found
}

/// The line's leftmost and rightmost pieces.
fn ends(line: &Line) -> Option<(Piece, Piece)> {
    let left = line
        .pieces
        .iter()
        .min_by(|a, b| a.start.total_cmp(&b.start))?;
    let right = line.pieces.iter().max_by(|a, b| a.end.total_cmp(&b.end))?;
    Some((*left, *right))
}

fn push_gap(found: &mut Vec<Gap>, row: f64, size: f64, left: Piece, right: Piece) {
    let wide = |piece: Piece| piece.end - piece.start >= MIN_PIECE * size;
    if right.start - left.end >= MIN_GAP * size && wide(left) && wide(right) {
        found.push(Gap {
            row,
            from: left.end,
            to: right.start,
            size,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One upright line at `y` with pieces `(start, end)`, 10 pt.
    fn line(y: f64, pieces: &[(f64, f64)]) -> Line {
        Line {
            text: String::new(),
            rotated: false,
            invisible_chars: 0,
            wide_gaps: 0,
            baseline: y,
            size: 10.0,
            pieces: pieces
                .iter()
                .map(|&(start, end)| Piece { start, end })
                .collect(),
        }
    }

    #[test]
    fn rows_drawn_across_a_gutter_are_flagged() {
        // Each row: the left line, then the right line (two lines, or one).
        let lines = [
            line(100.0, &[(72.0, 282.0)]),
            line(100.0, &[(324.0, 534.0)]),
            line(114.0, &[(72.0, 280.0), (324.0, 530.0)]),
            line(128.0, &[(72.0, 270.0), (324.0, 534.0)]),
        ];
        let found = interleaved_columns(&lines, &[], false).unwrap();
        assert_eq!(found.rows, 3);
        assert_eq!((found.from, found.to), (282.0, 324.0));
    }

    #[test]
    fn columns_drawn_one_after_the_other_are_not() {
        let mut lines: Vec<Line> = (0..4)
            .map(|i| line(100.0 + 14.0 * f64::from(i), &[(72.0, 282.0)]))
            .collect();
        lines.extend((0..4).map(|i| line(100.0 + 14.0 * f64::from(i), &[(324.0, 534.0)])));
        assert_eq!(interleaved_columns(&lines, &[], false), None);
    }

    #[test]
    fn two_rows_short_pieces_and_narrow_gaps_are_not() {
        let two_rows = [
            line(100.0, &[(72.0, 282.0), (324.0, 534.0)]),
            line(114.0, &[(72.0, 282.0), (324.0, 534.0)]),
        ];
        assert_eq!(interleaved_columns(&two_rows, &[], false), None);
        let short: Vec<Line> = (0..5)
            .map(|i| {
                line(
                    100.0 + 14.0 * f64::from(i),
                    &[(72.0, 282.0), (324.0, 370.0)],
                )
            })
            .collect();
        assert_eq!(interleaved_columns(&short, &[], false), None);
        let narrow: Vec<Line> = (0..5)
            .map(|i| {
                line(
                    100.0 + 14.0 * f64::from(i),
                    &[(72.0, 282.0), (290.0, 534.0)],
                )
            })
            .collect();
        assert_eq!(interleaved_columns(&narrow, &[], false), None);
    }

    #[test]
    fn gaps_at_different_places_do_not_add_up() {
        let lines = [
            line(100.0, &[(72.0, 140.0), (160.0, 300.0)]),
            line(114.0, &[(72.0, 200.0), (220.0, 400.0)]),
            line(128.0, &[(72.0, 300.0), (320.0, 500.0)]),
        ];
        assert_eq!(interleaved_columns(&lines, &[], false), None);
    }

    #[test]
    fn a_rule_in_the_gap_of_a_grid_is_a_cell_border() {
        let lines: Vec<Line> = (0..4)
            .map(|i| {
                line(
                    100.0 + 20.0 * f64::from(i),
                    &[(75.0, 235.0), (305.0, 470.0)],
                )
            })
            .collect();
        let rule = Rule {
            x: 300.0,
            top: 92.0,
            bottom: 172.0,
        };
        assert_eq!(interleaved_columns(&lines, &[rule], true), None);
        // A gutter rule on a page without a grid does not hide columns.
        assert!(interleaved_columns(&lines, &[rule], false).is_some());
        // Nor does a grid rule elsewhere on the page.
        let away = Rule {
            top: 400.0,
            bottom: 500.0,
            ..rule
        };
        assert!(interleaved_columns(&lines, &[away], true).is_some());
    }

    #[test]
    fn a_row_counts_once() {
        // Three gaps on one baseline (rows split into many lines) are one row.
        let lines = [
            line(100.0, &[(72.0, 282.0)]),
            line(100.0, &[(324.0, 534.0)]),
            line(100.5, &[(72.0, 282.0)]),
            line(100.5, &[(324.0, 534.0)]),
            line(114.0, &[(72.0, 282.0), (324.0, 534.0)]),
        ];
        assert_eq!(interleaved_columns(&lines, &[], false), None);
    }
}
