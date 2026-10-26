//! Glyphs → lines, in the order the PDF draws them.
//!
//! Rules (the README has the same list):
//! - Glyphs drawn one after another join one line while they keep the same
//!   direction (within about 10°), stay on the same baseline (within 0.6 of
//!   the larger font size) and do not jump back by more than 0.6 of it.
//!   Anything else starts a new line. Lines are never re-sorted by position.
//! - In a line, a gap wider than 2 font sizes becomes a TAB. Otherwise a
//!   space glyph, or a gap wider than 0.15 font size (0.5 between two CJK
//!   characters), becomes one space. Spaces never repeat; lines are trimmed.
//! - A line is "rotated" when its direction is more than 5° from the page's
//!   left-to-right direction, after the page's own /Rotate.
//! - For the quality checks only (never for the text): each line keeps its
//!   baseline, its font size and its "pieces", the stretches of text
//!   between gaps of at least 0.95 font size where one text operation ends
//!   and another begins.

use kurbo::{Point, Vec2};

/// One drawn glyph, in page coordinates (points, y down, after /Rotate).
#[derive(Debug, Clone)]
pub(crate) struct Glyph {
    /// Cleaned Unicode text; a single " " for any white space.
    pub text: String,
    pub origin: Point,
    /// Where the next glyph would start: origin + advance along the baseline.
    pub end: Point,
    /// Unit vector along the baseline.
    pub dir: Vec2,
    /// Font size in points (the em along the baseline).
    pub size: f64,
    /// Drawn with text render mode 3 (invisible), usually an OCR layer.
    pub invisible: bool,
    /// Which text operation (Tj, TJ, ' or ") of the page drew it.
    pub run: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    pub text: String,
    pub rotated: bool,
    pub invisible_chars: usize,
    pub wide_gaps: usize,
    /// The first glyph's baseline: its y on the page (points, y down).
    pub baseline: f64,
    /// The largest font size in the line.
    pub size: f64,
    /// Stretches of text along the line's direction, in drawing order.
    pub pieces: Vec<Piece>,
}

/// Where a stretch of a line's text starts and ends, measured along the
/// line's direction (for an upright line: x on the page).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Piece {
    pub start: f64,
    pub end: f64,
}

const SAME_DIRECTION: f64 = 0.985; // cos 10°
const UPRIGHT: f64 = 0.996; // cos 5°
const BASELINE: f64 = 0.6;
const BACKWARD: f64 = 0.6;
const WIDE_GAP: f64 = 2.0;
const WORD_GAP: f64 = 0.15;
const CJK_GAP: f64 = 0.5;
const PIECE_GAP: f64 = 0.95;

pub(crate) fn lines(glyphs: &[Glyph]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut current: Option<Builder> = None;
    for glyph in glyphs {
        match current.as_mut() {
            Some(builder) if builder.continues(glyph) => builder.push(glyph),
            _ => {
                if let Some(done) = current.take().and_then(Builder::finish) {
                    lines.push(done);
                }
                let mut builder = Builder::new(glyph);
                builder.push(glyph);
                current = Some(builder);
            }
        }
    }
    if let Some(done) = current.and_then(Builder::finish) {
        lines.push(done);
    }
    lines
}

struct Builder {
    line: Line,
    last: Glyph,
    pending: Separator,
    /// The line's direction, for measuring pieces.
    dir: Vec2,
    /// The last glyph that is not a space: its end and its text operation.
    ink: Option<(f64, usize)>,
}

/// What goes before the next non-space glyph. Ordered: a TAB wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Separator {
    None,
    Space,
    Tab,
}

impl Builder {
    fn new(first: &Glyph) -> Self {
        Self {
            line: Line {
                text: String::new(),
                rotated: first.dir.x < UPRIGHT,
                invisible_chars: 0,
                wide_gaps: 0,
                baseline: first.origin.y,
                size: first.size,
                pieces: Vec::new(),
            },
            last: first.clone(),
            pending: Separator::None,
            dir: first.dir,
            ink: None,
        }
    }

    fn continues(&self, next: &Glyph) -> bool {
        let last = &self.last;
        let size = last.size.max(next.size);
        let step = next.origin - last.origin;
        last.dir.dot(next.dir) >= SAME_DIRECTION
            && last.dir.cross(step).abs() <= BASELINE * size
            && last.dir.dot(next.origin - last.end) >= -BACKWARD * size
    }

    fn push(&mut self, glyph: &Glyph) {
        let size = self.last.size.max(glyph.size);
        let gap = self.last.dir.dot(glyph.origin - self.last.end);
        // Every gap counts, also one between two space glyphs.
        if gap > WIDE_GAP * size {
            self.pending = Separator::Tab;
        }
        if glyph.text == " " {
            self.pending = self.pending.max(Separator::Space);
            self.last = glyph.clone();
            return;
        }
        if !self.line.text.is_empty() {
            let before = self.line.text.chars().last().unwrap_or(' ');
            let after = glyph.text.chars().next().unwrap_or(' ');
            let word_gap = if is_cjk(before) && is_cjk(after) {
                CJK_GAP
            } else {
                WORD_GAP
            };
            if gap > word_gap * size {
                self.pending = self.pending.max(Separator::Space);
            }
            match self.pending {
                Separator::Tab => {
                    self.line.text.push('\t');
                    self.line.wide_gaps += 1;
                }
                Separator::Space => self.line.text.push(' '),
                Separator::None => {}
            }
        }
        self.line.text.push_str(&glyph.text);
        if glyph.invisible {
            self.line.invisible_chars += glyph.text.chars().count();
        }
        self.measure(glyph, size);
        self.pending = Separator::None;
        self.last = glyph.clone();
    }

    /// Extends the last piece with a glyph that is not a space, or starts a
    /// new piece after a gap of at least `PIECE_GAP` where another text
    /// operation begins.
    fn measure(&mut self, glyph: &Glyph, size: f64) {
        let start = self.dir.dot(glyph.origin.to_vec2());
        let end = self.dir.dot(glyph.end.to_vec2());
        self.line.size = self.line.size.max(glyph.size);
        let joins = self
            .ink
            .is_some_and(|(last_end, run)| run == glyph.run || start - last_end < PIECE_GAP * size);
        match self.line.pieces.last_mut() {
            Some(piece) if joins => {
                piece.start = piece.start.min(start);
                piece.end = piece.end.max(end);
            }
            _ => self.line.pieces.push(Piece { start, end }),
        }
        self.ink = Some((end, glyph.run));
    }

    fn finish(mut self) -> Option<Line> {
        let kept = self.line.text.trim_end_matches([' ', '\t']).len();
        self.line.text.truncate(kept);
        (!self.line.text.is_empty()).then_some(self.line)
    }
}

/// Cleans the Unicode text of one glyph. `None` when the glyph has no
/// usable mapping: nothing, the replacement character, private-use code
/// points, or only control characters. Presentation-form ligatures are
/// spelled out ("ﬁ" → "fi"); any white space becomes one " ".
pub(crate) fn clean(raw: &str) -> Option<String> {
    let mut text = String::new();
    for c in raw.chars() {
        match c {
            '\u{FB00}' => text.push_str("ff"),
            '\u{FB01}' => text.push_str("fi"),
            '\u{FB02}' => text.push_str("fl"),
            '\u{FB03}' => text.push_str("ffi"),
            '\u{FB04}' => text.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => text.push_str("st"),
            '\u{FFFD}' => return None,
            c if is_private_use(c) => return None,
            c if c.is_whitespace() => {
                if !text.ends_with(' ') {
                    text.push(' ');
                }
            }
            c if c.is_control() => {}
            c => text.push(c),
        }
    }
    if text.is_empty() {
        return None;
    }
    if text.trim().is_empty() {
        return Some(" ".to_string());
    }
    Some(text)
}

fn is_private_use(c: char) -> bool {
    matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{10FFFF}')
}

pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{303F}' // CJK symbols and punctuation
        | '\u{3040}'..='\u{30FF}' // Hiragana, Katakana
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{AC00}'..='\u{D7AF}' // Hangul
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}' // full-width forms
        | '\u{20000}'..='\u{3FFFF}')
}

/// Mathematical operators and letters (not Greek: Greek is also prose).
pub(crate) fn is_math(c: char) -> bool {
    matches!(c,
        '\u{2200}'..='\u{22FF}'
        | '\u{27C0}'..='\u{27EF}'
        | '\u{2980}'..='\u{2AFF}'
        | '\u{1D400}'..='\u{1D7FF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(text: &str, x: f64, y: f64, size: f64) -> Glyph {
        let advance = size * 0.5 * text.chars().count() as f64;
        Glyph {
            text: text.to_string(),
            origin: Point::new(x, y),
            end: Point::new(x + advance, y),
            dir: Vec2::new(1.0, 0.0),
            size,
            invisible: false,
            run: 0,
        }
    }

    #[test]
    fn joins_words_and_breaks_lines() {
        let glyphs = [
            g("Hel", 10.0, 20.0, 10.0),
            g("lo", 25.0, 20.0, 10.0),
            g("world", 38.0, 20.0, 10.0),
            g("Next", 10.0, 32.0, 10.0),
        ];
        let lines = lines(&glyphs);
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Hello world", "Next"]);
    }

    #[test]
    fn wide_gaps_become_tabs_and_spaces_do_not_repeat() {
        let glyphs = [
            g("a", 10.0, 20.0, 10.0),
            g(" ", 15.0, 20.0, 10.0),
            g(" ", 20.0, 20.0, 10.0),
            g("b", 25.0, 20.0, 10.0),
            g("c", 60.0, 20.0, 10.0),
        ];
        let lines = lines(&glyphs);
        assert_eq!(lines[0].text, "a b\tc");
        assert_eq!(lines[0].wide_gaps, 1);
    }

    #[test]
    fn a_wide_gap_between_two_space_glyphs_is_still_a_tab() {
        // Chromium: "Before", a space, a jump over an inline box, a space, "after".
        let glyphs = [
            g("Before", 10.0, 20.0, 10.0),
            g(" ", 40.0, 20.0, 10.0),
            g(" ", 150.0, 20.0, 10.0),
            g("after", 155.0, 20.0, 10.0),
        ];
        let lines = lines(&glyphs);
        assert_eq!(lines[0].text, "Before\tafter");
        assert_eq!(lines[0].wide_gaps, 1);
    }

    #[test]
    fn superscripts_stay_on_the_line_and_jumps_back_break_it() {
        let glyphs = [
            g("E=mc", 10.0, 20.0, 10.0),
            g("2", 30.0, 16.0, 6.0),
            g("left", 10.0, 20.5, 10.0),
        ];
        let texts: Vec<_> = lines(&glyphs).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, ["E=mc2", "left"]);
    }

    #[test]
    fn cjk_needs_a_wider_gap_for_a_space() {
        // 版 ends at 15: an 8-point gap (0.8 em) is a space, 3 points is not.
        let glyphs = [g("版", 10.0, 20.0, 10.0), g("本", 23.0, 20.0, 10.0)];
        assert_eq!(lines(&glyphs)[0].text, "版 本");
        let tight = [g("版", 10.0, 20.0, 10.0), g("本", 18.0, 20.0, 10.0)];
        assert_eq!(lines(&tight)[0].text, "版本");
    }

    #[test]
    fn clean_rules() {
        assert_eq!(clean("ﬁ").as_deref(), Some("fi"));
        assert_eq!(clean("\u{a0}").as_deref(), Some(" "));
        assert_eq!(clean("\u{E001}"), None);
        assert_eq!(clean("\u{FFFD}"), None);
        assert_eq!(clean(""), None);
        assert_eq!(clean("\u{7}"), None);
        assert_eq!(clean("中").as_deref(), Some("中"));
    }
}
