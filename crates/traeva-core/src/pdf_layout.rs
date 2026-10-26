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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    pub text: String,
    pub rotated: bool,
    pub invisible_chars: usize,
    pub wide_gaps: usize,
}

const SAME_DIRECTION: f64 = 0.985; // cos 10°
const UPRIGHT: f64 = 0.996; // cos 5°
const BASELINE: f64 = 0.6;
const BACKWARD: f64 = 0.6;
const WIDE_GAP: f64 = 2.0;
const WORD_GAP: f64 = 0.15;
const CJK_GAP: f64 = 0.5;

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
            },
            last: first.clone(),
            pending: Separator::None,
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
        self.pending = Separator::None;
        self.last = glyph.clone();
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
