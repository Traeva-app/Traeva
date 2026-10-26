//! The questions the raw-byte check (`preflight.rs`) asks about the bytes,
//! answered so that the whole check is linear in the size of the file.
//!
//! The check reads a dictionary at every `obj` keyword, also inside strings,
//! comments and other streams' data, so the same bytes are read from many
//! starting points. Scanning toward the end of the file once per question
//! makes that quadratic. Instead:
//!
//! - Tables built once, one entry per 64-byte block, answer "where do white
//!   space and comments end", "where is the next end of line, `>`, or
//!   keyword", reading at most one block per question.
//! - Where a literal string ends comes from the nesting of unescaped
//!   parentheses over the whole file and a tree over its per-block minima.
//!   Every string read starts at a `(` and agrees with that nesting from the
//!   next byte on, wherever it started.
//! - Walks of dictionaries and arrays that reach the same element in the
//!   same state continue identically, so a walk records what it found at its
//!   first element in each new 64-byte block (`Rest`), and a later walk that
//!   gets there takes the result instead of walking again. A dictionary read
//!   from its start whose walk joins another one gets the keys it needs from
//!   `KeyRecord`s.
//!
//! The answers are the same as reading the bytes directly (see the
//! `reference` tests, which compare with that on every position).

use std::collections::HashMap;
use std::rc::Rc;

use memchr::memmem;

use crate::MAX_DICT_DEPTH;

const BLOCK: usize = 64;
/// Walks deeper than this are stopped; any dictionary or array that deep is
/// past the nesting limit for every reader of it.
const WALK_LIMIT: usize = 2 * (MAX_DICT_DEPTH + 1);

pub(crate) fn is_white(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

pub(crate) fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_eol(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

/// Keywords whose next occurrence the check asks for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Word {
    Stream,
    Endstream,
    Endobj,
    ObjStm,
    XRef,
    First,
    /// A `#`, which starts an escape in a name.
    Hash,
}

impl Word {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Stream => b"stream",
            Self::Endstream => b"endstream",
            Self::Endobj => b"endobj",
            Self::ObjStm => b"ObjStm",
            Self::XRef => b"XRef",
            Self::First => b"First",
            Self::Hash => b"#",
        }
    }
}

/// For every block, the first position at or after its start where
/// something holds (the file's length if nowhere).
struct Next {
    first: Vec<u32>,
}

impl Next {
    /// From the positions where it holds, in increasing order.
    fn from_positions(len: usize, positions: impl Iterator<Item = usize>) -> Self {
        let blocks = len.div_ceil(BLOCK);
        let mut first = vec![len as u32; blocks + 1];
        let mut filled = 0;
        for at in positions {
            while filled <= at / BLOCK {
                first[filled] = at as u32;
                filled += 1;
            }
        }
        Self { first }
    }

    fn of_bytes(bytes: &[u8], holds: impl Fn(u8) -> bool) -> Self {
        Self::from_positions(
            bytes.len(),
            bytes
                .iter()
                .enumerate()
                .filter(|(_, b)| holds(**b))
                .map(|(i, _)| i),
        )
    }

    /// The first position at or after `i` where `holds` is true at that
    /// position; `len` if none.
    fn find(&self, i: usize, len: usize, holds: impl Fn(usize) -> bool) -> usize {
        if i >= len {
            return len;
        }
        let block_end = ((i / BLOCK + 1) * BLOCK).min(len);
        (i..block_end)
            .find(|&j| holds(j))
            .unwrap_or_else(|| self.first[i / BLOCK + 1] as usize)
    }
}

/// What the rest of a dictionary or array body holds, from one of its
/// elements on.
#[derive(Debug, Clone, Copy)]
struct Rest {
    /// Just after the closing `>>` or `]`; `None` when it cannot be read
    /// (bad syntax, or nested past the limit for every reader).
    end: Option<u32>,
    /// How deep the rest reaches: each element adds 1, plus the depth of a
    /// dictionary or array element itself.
    need: u8,
    /// Where it ended or stopped being readable.
    reach: u32,
    /// Which walk of a body recorded it (see `keys_from`).
    body: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Dict,
    Array,
}

enum Walked {
    Done(Rest),
    /// Stopped at `WALK_LIMIT`; the result depends on the reader's depth.
    TooDeep,
}

/// Where a value starts and ends (just after it) in the file.
pub(crate) type Span = (u32, u32);

/// The values of the keys the check reads in one dictionary (the last
/// occurrence of each, as in hayro), and the last `/Filter` whose value is
/// not a name, an array of names or a reference.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Keys {
    pub filter: Option<Span>,
    pub bad_filter: Option<Span>,
    pub subtype: Option<Span>,
    pub type_: Option<Span>,
    pub length: Option<Span>,
    pub external: Option<Span>,
    pub encrypt: Option<Span>,
    pub size: Option<Span>,
    pub prev: Option<Span>,
    pub xref_stm: Option<Span>,
    pub decode_parms: Option<Span>,
    pub dp: Option<Span>,
    pub n: Option<Span>,
    pub first: Option<Span>,
    pub w: Option<Span>,
    pub predictor: Option<Span>,
    pub colors: Option<Span>,
    pub bits_per_component: Option<Span>,
    pub columns: Option<Span>,
}

impl Keys {
    fn slots(&mut self) -> [&mut Option<Span>; 19] {
        [
            &mut self.filter,
            &mut self.bad_filter,
            &mut self.subtype,
            &mut self.type_,
            &mut self.length,
            &mut self.external,
            &mut self.encrypt,
            &mut self.size,
            &mut self.prev,
            &mut self.xref_stm,
            &mut self.decode_parms,
            &mut self.dp,
            &mut self.n,
            &mut self.first,
            &mut self.w,
            &mut self.predictor,
            &mut self.colors,
            &mut self.bits_per_component,
            &mut self.columns,
        ]
    }

    /// Keeps the later of the two for every key (`later` comes after
    /// `self` in the dictionary).
    fn then(mut self, mut later: Keys) -> Keys {
        for (mine, theirs) in self.slots().into_iter().zip(later.slots()) {
            if theirs.is_some() {
                *mine = *theirs;
            }
        }
        self
    }

    /// Only the keys at or after `from`.
    fn from(mut self, from: usize) -> Keys {
        for slot in self.slots() {
            if slot.is_some_and(|(at, _)| (at as usize) < from) {
                *slot = None;
            }
        }
        self
    }
}

/// The keys of one walked dictionary body from `from` to its end.
struct KeyRecord {
    from: usize,
    keys: Keys,
    end: u32,
}

/// A walk that took another walk's result: where, and which walk.
#[derive(Clone, Copy)]
struct Join {
    at: u32,
    body: u32,
    end: u32,
}

pub(crate) struct Scan<'a> {
    pub bytes: &'a [u8],
    non_white: Next,
    /// For every block, the last position before it that is not white space.
    non_white_before: Vec<Option<u32>>,
    eol: Next,
    greater: Next,
    /// Where white space and comments end, from each block's start.
    trivia_at_block: Vec<u32>,
    /// The same, from the first end of line at or after each block's start.
    trivia_after_eol: Vec<u32>,
    words: [Next; 7],
    parens: Parens,
    /// Walk results by element position (`position << 1 | is array`).
    rests: HashMap<u32, Rest>,
    has_rest: Vec<u64>,
    /// Keys of walked dictionary bodies that a dictionary read from its
    /// start joined, by walk.
    key_records: HashMap<u32, KeyRecord>,
    /// Walks that kept samples and then took another walk's result.
    joins: HashMap<u32, Join>,
    next_body: u32,
    /// When to drop unreachable records again.
    forget_at: usize,
    /// Dictionary and array elements read so far (the work the tests bound).
    steps: usize,
    /// Numbers as hayro reads them, by position: each is read once.
    numbers: HashMap<u32, Option<(i64, u32)>>,
    /// `/Filter` values, by position: each is read once.
    filter_lists: HashMap<u32, Option<FilterList>>,
    /// Bytes of numbers and `/Filter` values read so far (each once).
    value_bytes: usize,
}

/// A `/Filter` value: the names in it, or a list stored elsewhere (a
/// reference, or an array with a member that is not a name). The names
/// are stored once per position and shared: many dictionaries can read
/// the same value, and each holds only a reference to it (R4-PDF-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilterList {
    Names(Rc<[Vec<u8>]>),
    Elsewhere,
}

impl<'a> Scan<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        let len = bytes.len();
        let blocks = len.div_ceil(BLOCK);
        // Where `skip_white` from each position ends, computed backwards and
        // kept at block starts: white space continues to the next position,
        // a comment continues at the end of line that ends it.
        let mut trivia_at_block = vec![len as u32; blocks + 1];
        let mut trivia_after_eol = vec![len as u32; blocks + 1];
        let mut after = len;
        let mut after_eol = len;
        for i in (0..len).rev() {
            let byte = bytes[i];
            let here = if is_white(byte) {
                after
            } else if byte == b'%' {
                after_eol
            } else {
                i
            };
            if is_eol(byte) {
                after_eol = here;
            }
            after = here;
            if i % BLOCK == 0 {
                trivia_at_block[i / BLOCK] = here as u32;
                trivia_after_eol[i / BLOCK] = after_eol as u32;
            }
        }
        let word = |w: Word| Next::from_positions(len, memmem::find_iter(bytes, w.bytes()));
        let mut non_white_before = Vec::with_capacity(blocks);
        let mut last = None;
        for (b, chunk) in bytes.chunks(BLOCK).enumerate() {
            non_white_before.push(last);
            if let Some(k) = chunk.iter().rposition(|&byte| !is_white(byte)) {
                last = Some((b * BLOCK + k) as u32);
            }
        }
        Self {
            bytes,
            non_white: Next::of_bytes(bytes, |b| !is_white(b)),
            non_white_before,
            eol: Next::of_bytes(bytes, is_eol),
            greater: Next::of_bytes(bytes, |b| b == b'>'),
            trivia_at_block,
            trivia_after_eol,
            words: [
                word(Word::Stream),
                word(Word::Endstream),
                word(Word::Endobj),
                word(Word::ObjStm),
                word(Word::XRef),
                word(Word::First),
                word(Word::Hash),
            ],
            parens: Parens::new(bytes),
            rests: HashMap::new(),
            has_rest: vec![0; len / 64 + 1],
            key_records: HashMap::new(),
            joins: HashMap::new(),
            next_body: 0,
            forget_at: 256,
            steps: 0,
            numbers: HashMap::new(),
            filter_lists: HashMap::new(),
            value_bytes: 0,
        }
    }

    /// The first position at or after `i` that is not white space or in a
    /// comment.
    pub(crate) fn skip_white(&self, mut i: usize) -> usize {
        let bytes = self.bytes;
        let len = bytes.len();
        if i >= len {
            return i;
        }
        loop {
            if i >= len {
                return len;
            }
            let boundary = (i / BLOCK + 1) * BLOCK;
            let j = self.non_white.find(i, len, |k| !is_white(bytes[k]));
            if j >= boundary && boundary < len {
                // White space runs over the block's start.
                return self.trivia_at_block[boundary / BLOCK] as usize;
            }
            if j >= len || bytes[j] != b'%' {
                return j;
            }
            let end = self.eol.find(j, len, |k| is_eol(bytes[k]));
            if end >= len {
                return len;
            }
            if end / BLOCK > j / BLOCK {
                // The comment runs into `end`'s block, so `end` is that
                // block's first end of line.
                return self.trivia_after_eol[end / BLOCK] as usize;
            }
            i = end;
        }
    }

    /// The first position at or after `i` that is not white space (comments
    /// are not skipped), or the file's length.
    pub(crate) fn next_non_white(&self, i: usize) -> usize {
        let bytes = self.bytes;
        self.non_white.find(i, bytes.len(), |k| !is_white(bytes[k]))
    }

    /// The last position before `end` that is not white space.
    pub(crate) fn last_non_white_before(&self, end: usize) -> Option<usize> {
        let end = end.min(self.bytes.len());
        if end == 0 {
            return None;
        }
        let block = (end - 1) / BLOCK;
        (block * BLOCK..end)
            .rev()
            .find(|&k| !is_white(self.bytes[k]))
            .or_else(|| self.non_white_before[block].map(|at| at as usize))
    }

    /// Dictionary and array elements read so far, and bytes of values
    /// read (the work the tests bound).
    pub(crate) fn steps(&self) -> usize {
        self.steps + self.value_bytes
    }

    /// The number hayro 0.8.0 reads at `at` (see `hayro_number`), as the
    /// integer `as_i64` gives, and the index after it. Read once per
    /// position, so many dictionaries that share a value do not read a long
    /// number again.
    pub(crate) fn hayro_integer(&mut self, at: usize) -> Option<(i64, usize)> {
        if let Some(&found) = self.numbers.get(&(at as u32)) {
            return found.map(|(n, end)| (n, end as usize));
        }
        let found = hayro_number(self.bytes, at);
        self.value_bytes += found.map_or(1, |(_, end)| end - at);
        self.numbers
            .insert(at as u32, found.map(|(n, end)| (n, end as u32)));
        found
    }

    /// The `/Filter` value from `at` to `end` (a value read while walking a
    /// dictionary), read once per position. `None` if it is neither names
    /// nor a list stored elsewhere. A value read again is the same shared
    /// list, not a copy.
    pub(crate) fn filter_list(&mut self, at: usize, end: usize) -> Option<FilterList> {
        if let Some(found) = self.filter_lists.get(&(at as u32)) {
            return found.clone();
        }
        let value = &self.bytes[at..end];
        self.value_bytes += value.len();
        let found = match value.first() {
            Some(b'/') => read_name(value, 0).map(|(name, _)| FilterList::Names(Rc::from([name]))),
            Some(b'[') => {
                let mut names = Vec::new();
                let mut i = skip_white_plain(value, 1);
                loop {
                    match value.get(i) {
                        None => break None,
                        Some(b']') => break Some(FilterList::Names(names.into())),
                        Some(b'/') => match read_name(value, i) {
                            Some((name, next)) => {
                                names.push(name);
                                i = skip_white_plain(value, next);
                            }
                            None => break None,
                        },
                        Some(_) => break Some(FilterList::Elsewhere),
                    }
                }
            }
            Some(b'0'..=b'9') => Some(FilterList::Elsewhere),
            _ => None,
        };
        self.filter_lists.insert(at as u32, found.clone());
        found
    }

    /// A reference as hayro reads one at `at` (hayro-syntax
    /// src/object/ref.rs): two numbers that fit an i32, then `R`, with only
    /// white space (no comments) between. The numbers and the index after
    /// `R`.
    pub(crate) fn hayro_reference(&mut self, at: usize) -> Option<(i64, i64, usize)> {
        let (number, after) = self.hayro_integer(at)?;
        i32::try_from(number).ok()?;
        let k = self.next_non_white(after);
        let (generation, after) = self.hayro_integer(k)?;
        i32::try_from(generation).ok()?;
        let r = self.next_non_white(after);
        (self.bytes.get(r) == Some(&b'R')).then_some((number, generation, r + 1))
    }

    /// The next occurrence of `word` at or after `i`, or the file's length.
    pub(crate) fn next_word(&self, word: Word, i: usize) -> usize {
        let bytes = self.bytes;
        let w = word.bytes();
        let table = &self.words[word as usize];
        table.find(i, bytes.len(), |k| bytes[k..].starts_with(w))
    }

    /// Just after the literal string that starts with the `(` at `i`.
    pub(crate) fn string_end(&self, i: usize) -> Option<usize> {
        self.parens.end(self.bytes, i)
    }

    /// Just after the hex string that starts with the `<` at `i`.
    fn hex_end(&self, i: usize) -> Option<usize> {
        let bytes = self.bytes;
        let at = self.greater.find(i, bytes.len(), |k| bytes[k] == b'>');
        (at < bytes.len()).then_some(at + 1)
    }

    /// The index just after the object starting at `i`, read at nesting
    /// `depth` (the limit is `MAX_DICT_DEPTH`).
    pub(crate) fn skip_object(&mut self, i: usize, depth: usize) -> Option<usize> {
        if depth > MAX_DICT_DEPTH {
            return None;
        }
        let (end, need) = self.element(i, 0)?;
        (depth + usize::from(need) <= MAX_DICT_DEPTH).then_some(end)
    }

    /// The `<< ... >>` at `start`: where the values of the keys the check
    /// uses start, and the index just after its `>>`. Values are skipped as
    /// whole PDF objects.
    pub(crate) fn read_dict(&mut self, start: usize) -> Option<(Keys, usize)> {
        if !self.bytes[start..].starts_with(b"<<") {
            return None;
        }
        let mut keys = Keys::default();
        match self.walk(start, Body::Dict, 0, Some(&mut keys)) {
            Walked::Done(Rest {
                end: Some(end),
                need,
                ..
            }) if usize::from(need) <= MAX_DICT_DEPTH => Some((keys, end as usize)),
            _ => None,
        }
    }

    /// One object at `i`: just after it, and how deep it reaches below it.
    fn element(&mut self, i: usize, level: usize) -> Option<(usize, u8)> {
        let bytes = self.bytes;
        match *bytes.get(i)? {
            b'/' => Some((read_name(bytes, i)?.1, 0)),
            b'(' => Some((self.string_end(i)?, 0)),
            b'<' if bytes.get(i + 1) == Some(&b'<') => self.composite(i, Body::Dict, level),
            b'<' => Some((self.hex_end(i)?, 0)),
            b'[' => self.composite(i, Body::Array, level),
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                let mut j = i + 1;
                while j < bytes.len() && matches!(bytes[j], b'0'..=b'9' | b'.') {
                    j += 1;
                }
                // `12 0 R`
                let k = self.skip_white(j);
                if let Some((_, after)) = read_number(bytes, k) {
                    let r = self.skip_white(after);
                    if bytes.get(r) == Some(&b'R')
                        && bytes
                            .get(r + 1)
                            .is_none_or(|&b| is_white(b) || is_delimiter(b))
                    {
                        return Some((r + 1, 0));
                    }
                }
                Some((j, 0))
            }
            b if b.is_ascii_alphabetic() => {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                Some((j, 0))
            }
            _ => None,
        }
    }

    fn composite(&mut self, open: usize, body: Body, level: usize) -> Option<(usize, u8)> {
        match self.walk(open, body, level + 1, None) {
            Walked::Done(Rest {
                end: Some(end),
                need,
                ..
            }) => Some((end as usize, need)),
            _ => None,
        }
    }

    /// Walks the body of the dictionary or array whose opening `<<` or `[`
    /// is at `open`. With `keys`, also records where the values of the keys
    /// the check reads start (a dictionary read from its start).
    fn walk(
        &mut self,
        open: usize,
        body: Body,
        level: usize,
        mut keys: Option<&mut Keys>,
    ) -> Walked {
        if level > WALK_LIMIT {
            return Walked::TooDeep;
        }
        let body_id = self.next_body;
        self.next_body += 1;
        let bytes = self.bytes;
        let len = bytes.len();
        let (close, mut j) = match body {
            Body::Dict => (&b">>"[..], open + 2),
            Body::Array => (&b"]"[..], open + 1),
        };
        let mut need = 0u8;
        // Elements where this walk's results are kept: the first element in
        // each block after the opening one, with the deepest reach of the
        // elements from there to the next one.
        let mut samples: Vec<(usize, u8)> = Vec::new();
        let mut last_block = open / BLOCK;
        let fail = |at: usize| Rest {
            end: None,
            need: 0,
            reach: at as u32,
            body: body_id,
        };
        let mut joined = None;
        let rest = loop {
            j = self.skip_white(j);
            if j >= len {
                break fail(len);
            }
            if let Some(found) = self.rest_at(j, body) {
                joined = Some((j, found));
                if let (Some(keys), Some(end)) = (keys.as_deref_mut(), found.end) {
                    *keys = keys.then(self.keys_from(found.body, j, end as usize));
                }
                break Rest {
                    end: found.end,
                    need: need.max(found.need),
                    reach: found.reach,
                    body: body_id,
                };
            }
            if bytes[j..].starts_with(close) {
                break Rest {
                    end: Some((j + close.len()) as u32),
                    need,
                    reach: (j + close.len()) as u32,
                    body: body_id,
                };
            }
            if j / BLOCK > last_block {
                last_block = j / BLOCK;
                samples.push((j, 0));
            }
            self.steps += 1;
            let (value, key) = match body {
                Body::Dict => {
                    if bytes[j] != b'/' {
                        break fail(j);
                    }
                    let Some((name, after)) = read_name(bytes, j) else {
                        break fail(j);
                    };
                    (self.skip_white(after), Some(name))
                }
                Body::Array => (j, None),
            };
            let (end, inner) = match bytes.get(value) {
                Some(b'<') if bytes.get(value + 1) == Some(&b'<') => {
                    match self.walk(value, Body::Dict, level + 1, None) {
                        Walked::Done(r) => match r.end {
                            Some(end) => (end as usize, r.need),
                            None => break fail(r.reach as usize),
                        },
                        Walked::TooDeep => return self.too_deep(body, body_id, level, &samples),
                    }
                }
                Some(b'[') => match self.walk(value, Body::Array, level + 1, None) {
                    Walked::Done(r) => match r.end {
                        Some(end) => (end as usize, r.need),
                        None => break fail(r.reach as usize),
                    },
                    Walked::TooDeep => return self.too_deep(body, body_id, level, &samples),
                },
                _ => match self.element(value, level) {
                    Some(found) => found,
                    None => break fail(value),
                },
            };
            let reach = inner.saturating_add(1).min(MAX_DICT_DEPTH as u8 + 1);
            need = need.max(reach);
            if let Some(last) = samples.last_mut() {
                last.1 = last.1.max(reach);
            }
            if let (Some(keys), Some(name)) = (keys.as_deref_mut(), key) {
                record_key(keys, bytes, &name, value, end);
            }
            j = end;
        };
        // Kept per sample before the body as a whole is judged: an element
        // past the nesting limit makes the body unreadable, but not the
        // rest after it.
        let raw = rest;
        let rest = if usize::from(rest.need) > MAX_DICT_DEPTH {
            Rest { end: None, ..rest }
        } else {
            rest
        };
        if let (Some((at, found)), false) = (joined, samples.is_empty()) {
            self.joins.insert(
                body_id,
                Join {
                    at: at as u32,
                    body: found.body,
                    end: found.reach,
                },
            );
        }
        self.keep(body, &samples, raw, joined.map_or(0, |(_, r)| r.need));
        Walked::Done(rest)
    }

    /// A walk below this one stopped at `WALK_LIMIT`. If that is more than
    /// the nesting limit below this body, the body is past the limit for
    /// every reader; otherwise it depends on the reader, so nothing is kept.
    fn too_deep(
        &mut self,
        body: Body,
        body_id: u32,
        level: usize,
        samples: &[(usize, u8)],
    ) -> Walked {
        if WALK_LIMIT + 1 - level > MAX_DICT_DEPTH {
            let rest = Rest {
                end: None,
                need: MAX_DICT_DEPTH as u8 + 1,
                reach: self.bytes.len() as u32,
                body: body_id,
            };
            self.keep(body, samples, rest, 0);
            Walked::Done(rest)
        } else {
            Walked::TooDeep
        }
    }

    /// Keeps the result of a finished walk at its samples that are at least
    /// a block before where it ended (nearer ones are cheap to walk again,
    /// and this keeps at most one sample per block). `rest` is the walk's
    /// result before its own depth was judged: from each sample on, the rest
    /// is unreadable only if it breaks, or if its own depth is past the
    /// nesting limit.
    fn keep(&mut self, body: Body, samples: &[(usize, u8)], rest: Rest, joined_need: u8) {
        let mut need = joined_need;
        for &(at, own) in samples.iter().rev() {
            need = need.max(own);
            if (rest.reach as usize).saturating_sub(at) < BLOCK {
                continue;
            }
            let key = (at as u32) << 1 | u32::from(body == Body::Array);
            let end = rest.end.filter(|_| usize::from(need) <= MAX_DICT_DEPTH);
            self.rests.insert(
                key,
                Rest {
                    end,
                    need: if end.is_some() {
                        need
                    } else {
                        rest.need.max(need)
                    },
                    ..rest
                },
            );
            self.has_rest[at / 64] |= 1 << (at % 64);
        }
    }

    fn rest_at(&self, at: usize, body: Body) -> Option<Rest> {
        if self.has_rest[at / 64] & (1 << (at % 64)) == 0 {
            return None;
        }
        let key = (at as u32) << 1 | u32::from(body == Body::Array);
        self.rests.get(&key).copied()
    }

    /// The keys of the dictionary body walked as `body`, from its element at
    /// `from` to its end at `end`. A walk that took another walk's result
    /// continues in that walk's body, so the keys are read along that chain,
    /// each walk's own part once; a later dictionary that joins earlier only
    /// reads the part before.
    fn keys_from(&mut self, body: u32, from: usize, end: usize) -> Keys {
        let mut parts = Vec::new();
        let (mut body, mut from) = (body, from);
        let base = loop {
            let record = self.key_records.get(&body).map(|r| (r.from, r.keys));
            if let Some((at, keys)) = record
                && at <= from
            {
                break keys.from(from);
            }
            let join = self.joins.get(&body).copied();
            let stop = match (record, join) {
                (Some((at, _)), _) => at,
                (None, Some(join)) => join.at as usize,
                (None, None) => end,
            };
            parts.push((body, from, self.read_keys(from, stop)));
            match (record, join) {
                (Some((_, keys)), _) => break keys,
                (None, Some(join)) => (body, from) = (join.body, join.at as usize),
                (None, None) => break Keys::default(),
            }
        };
        let mut keys = base;
        for (body, from, own) in parts.into_iter().rev() {
            keys = own.then(keys);
            self.key_records.insert(
                body,
                KeyRecord {
                    from,
                    keys,
                    end: end as u32,
                },
            );
        }
        keys
    }

    /// The keys of a dictionary body's elements from `from` up to the
    /// element at `stop` or the body's `>>`.
    fn read_keys(&mut self, from: usize, stop: usize) -> Keys {
        let mut found = Keys::default();
        let mut j = from;
        loop {
            j = self.skip_white(j);
            if j >= stop || j >= self.bytes.len() || self.bytes[j..].starts_with(b">>") {
                return found;
            }
            let Some((name, after)) = read_name(self.bytes, j) else {
                return found;
            };
            self.steps += 1;
            let value = self.skip_white(after);
            let Some((value_end, _)) = self.element(value, 0) else {
                return found;
            };
            record_key(&mut found, self.bytes, &name, value, value_end);
            j = value_end;
        }
    }

    /// Drops what can no longer be reached: a walk starting at or after
    /// `start` never visits earlier positions.
    pub(crate) fn forget_before(&mut self, start: usize) {
        if self.key_records.len() + self.joins.len() > self.forget_at {
            self.key_records.retain(|_, r| r.end as usize > start);
            self.joins.retain(|_, j| j.end as usize > start);
            self.forget_at = 256.max(2 * (self.key_records.len() + self.joins.len()));
        }
    }
}

/// Records where the value of a key the check reads is.
fn record_key(keys: &mut Keys, bytes: &[u8], name: &[u8], value: usize, end: usize) {
    let at = Some((value as u32, end as u32));
    match name {
        b"Filter" => {
            keys.filter = at;
            if !filter_value_ok(&bytes[value..end]) {
                keys.bad_filter = at;
            }
        }
        b"Subtype" => keys.subtype = at,
        b"Type" => keys.type_ = at,
        b"Length" => keys.length = at,
        b"F" => keys.external = at,
        b"Encrypt" => keys.encrypt = at,
        b"Size" => keys.size = at,
        b"Prev" => keys.prev = at,
        b"XRefStm" => keys.xref_stm = at,
        b"DecodeParms" => keys.decode_parms = at,
        b"DP" => keys.dp = at,
        b"N" => keys.n = at,
        b"First" => keys.first = at,
        b"W" => keys.w = at,
        b"Predictor" => keys.predictor = at,
        b"Colors" => keys.colors = at,
        b"BitsPerComponent" => keys.bits_per_component = at,
        b"Columns" => keys.columns = at,
        _ => {}
    }
}

/// A `/Filter` value the check can read: a name, an array whose members up
/// to the first one that is not a name are names, or a number (a reference).
fn filter_value_ok(value: &[u8]) -> bool {
    match value.first() {
        Some(b'/') => read_name(value, 0).is_some(),
        Some(b'[') => {
            let mut i = skip_white_plain(value, 1);
            loop {
                match value.get(i) {
                    None => return false,
                    Some(b']') => return true,
                    Some(b'/') => match read_name(value, i) {
                        Some((_, next)) => i = skip_white_plain(value, next),
                        None => return false,
                    },
                    Some(_) => return true,
                }
            }
        }
        Some(b'0'..=b'9') => true,
        _ => false,
    }
}

/// `skip_white` without tables, for short values already cut out.
pub(crate) fn skip_white_plain(bytes: &[u8], mut i: usize) -> usize {
    loop {
        while i < bytes.len() && is_white(bytes[i]) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'%') {
            return i;
        }
        while i < bytes.len() && !is_eol(bytes[i]) {
            i += 1;
        }
    }
}

/// A name at `bytes[i] == '/'`, with `#xx` escapes decoded.
pub(crate) fn read_name(bytes: &[u8], i: usize) -> Option<(Vec<u8>, usize)> {
    let mut name = Vec::new();
    let mut j = i + 1;
    while j < bytes.len() && !is_white(bytes[j]) && !is_delimiter(bytes[j]) {
        if bytes[j] == b'#' {
            let hex = std::str::from_utf8(bytes.get(j + 1..j + 3)?).ok()?;
            name.push(u8::from_str_radix(hex, 16).ok()?);
            j += 3;
        } else {
            name.push(bytes[j]);
            j += 1;
        }
    }
    Some((name, j))
}

/// A number as hayro 0.8.0 reads one at `i` (hayro-syntax
/// src/object/number.rs, `read_inner` and `as_i64`): an optional sign,
/// digits with at most one point, not followed by a regular character; a
/// minus after digits ends the number and the digits and minuses after it
/// are skipped; a lone sign or point is 0. A real is divided as hayro does in
/// f64 and cut toward zero; a number too long for 64 bits is parsed as text.
pub(crate) fn hayro_number(bytes: &[u8], i: usize) -> Option<(i64, usize)> {
    const POWERS_OF_10: [f64; 20] = [
        1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
        1e17, 1e18, 1e19,
    ];
    let mut j = i;
    let negative = match bytes.get(j) {
        Some(b'-') => {
            j += 1;
            true
        }
        Some(b'+') => {
            j += 1;
            false
        }
        _ => false,
    };
    let (mut mantissa, mut shift) = (0u64, 0u32);
    let (mut dot, mut digits, mut overflowed) = (false, false, false);
    let mut number_end = None;
    loop {
        match bytes.get(j) {
            Some(&d @ b'0'..=b'9') => {
                j += 1;
                if !overflowed {
                    match mantissa
                        .checked_mul(10)
                        .and_then(|v| v.checked_add(u64::from(d - b'0')))
                    {
                        Some(v) => mantissa = v,
                        None => overflowed = true,
                    }
                }
                digits = true;
                if dot {
                    shift += 1;
                }
            }
            Some(b'.') if !dot => {
                j += 1;
                dot = true;
            }
            Some(b'-') if digits => {
                number_end = Some(j);
                j += 1;
                while bytes
                    .get(j)
                    .is_some_and(|&b| b.is_ascii_digit() || b == b'-')
                {
                    j += 1;
                }
                break;
            }
            _ => break,
        }
    }
    if !digits {
        return (negative || dot).then_some((0, j));
    }
    if bytes
        .get(j)
        .is_some_and(|&b| !is_white(b) && !is_delimiter(b))
    {
        return None;
    }
    let value = if overflowed || (!dot && mantissa > i64::MAX as u64) {
        let text = std::str::from_utf8(&bytes[i..number_end.unwrap_or(j)]).ok()?;
        match text.parse::<i64>() {
            Ok(v) if !dot => v,
            _ => text.parse::<f64>().ok()? as i64,
        }
    } else if !dot {
        if negative {
            (mantissa as i64).wrapping_neg()
        } else {
            mantissa as i64
        }
    } else {
        let mut v = mantissa as f64;
        if shift > 0 {
            v /= match POWERS_OF_10.get(shift as usize) {
                Some(&power) => power,
                None => 10f64.powi(shift as i32),
            };
        }
        (if negative { -v } else { v }) as i64
    };
    Some((value, j))
}

/// A decimal number of at most 10 digits at `i`, and the index after it.
pub(crate) fn read_number(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    let mut value = 0usize;
    while j < bytes.len() && bytes[j].is_ascii_digit() && j - i < 10 {
        value = value * 10 + usize::from(bytes[j] - b'0');
        j += 1;
    }
    (j > i && !bytes.get(j).is_some_and(u8::is_ascii_digit)).then_some((value, j))
}

/// The nesting of unescaped parentheses: `level(y)` counts `(` minus `)`
/// before `y`, where a byte right after an unescaped backslash is escaped.
/// A string read from the `(` at `q` agrees with this from `q + 1` on (it
/// counts `q` itself even when `q` is escaped here), so it ends just after
/// the first position `y > q + 1` where the level drops below `level(q + 1)`.
/// Kept per block: the level and escape state at the block's start, and the
/// lowest level inside it, with a tree over those minima.
struct Parens {
    start: Vec<i32>,
    escaped: Vec<bool>,
    tree: Vec<i32>,
    leaves: usize,
}

impl Parens {
    fn new(bytes: &[u8]) -> Self {
        let blocks = bytes.len().div_ceil(BLOCK);
        let leaves = blocks.next_power_of_two().max(1);
        let mut start = Vec::with_capacity(blocks);
        let mut escaped = Vec::with_capacity(blocks);
        let mut tree = vec![i32::MAX; 2 * leaves];
        let (mut level, mut escape) = (0i32, false);
        for (b, chunk) in bytes.chunks(BLOCK).enumerate() {
            start.push(level);
            escaped.push(escape);
            let mut lowest = i32::MAX;
            for &byte in chunk {
                step(byte, &mut level, &mut escape);
                lowest = lowest.min(level);
            }
            tree[leaves + b] = lowest;
        }
        for node in (1..leaves).rev() {
            tree[node] = tree[2 * node].min(tree[2 * node + 1]);
        }
        Self {
            start,
            escaped,
            tree,
            leaves,
        }
    }

    fn end(&self, bytes: &[u8], q: usize) -> Option<usize> {
        let block = q / BLOCK;
        let (mut level, mut escape) = (self.start[block], self.escaped[block]);
        for &byte in &bytes[block * BLOCK..=q] {
            step(byte, &mut level, &mut escape);
        }
        let target = level - 1;
        let block_end = ((block + 1) * BLOCK).min(bytes.len());
        for (y, &byte) in bytes.iter().enumerate().take(block_end).skip(q + 1) {
            step(byte, &mut level, &mut escape);
            if level == target {
                return Some(y + 1);
            }
        }
        let next = self.first_at_most(block + 1, target)?;
        let (mut level, mut escape) = (self.start[next], self.escaped[next]);
        let from = next * BLOCK;
        for (y, &byte) in bytes.iter().enumerate().skip(from).take(BLOCK) {
            step(byte, &mut level, &mut escape);
            if level == target {
                return Some(y + 1);
            }
        }
        None
    }

    /// The first block at or after `from` whose lowest level is at most
    /// `target`.
    fn first_at_most(&self, from: usize, target: i32) -> Option<usize> {
        if from >= self.start.len() {
            return None;
        }
        let mut node = self.leaves + from;
        loop {
            if self.tree[node] <= target {
                while node < self.leaves {
                    node = if self.tree[2 * node] <= target {
                        2 * node
                    } else {
                        2 * node + 1
                    };
                }
                return Some(node - self.leaves);
            }
            while node % 2 == 1 {
                node /= 2;
                if node <= 1 {
                    return None;
                }
            }
            node += 1;
        }
    }
}

fn step(byte: u8, level: &mut i32, escape: &mut bool) {
    if *escape {
        *escape = false;
    } else if byte == b'\\' {
        *escape = true;
    } else if byte == b'(' {
        *level += 1;
    } else if byte == b')' {
        *level -= 1;
    }
}

#[cfg(test)]
mod reference {
    //! The direct readings the tables replace (the code of
    //! `pdf-experiment-5`), compared with the tables on every position of
    //! generated inputs.

    use super::*;

    fn skip_white(bytes: &[u8], mut i: usize) -> usize {
        loop {
            while i < bytes.len() && is_white(bytes[i]) {
                i += 1;
            }
            if bytes.get(i) != Some(&b'%') {
                return i;
            }
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
        }
    }

    fn skip_object(bytes: &[u8], i: usize, depth: usize) -> Option<usize> {
        if depth > MAX_DICT_DEPTH {
            return None;
        }
        match *bytes.get(i)? {
            b'/' => Some(read_name(bytes, i)?.1),
            b'(' => {
                let mut nesting = 0usize;
                let mut j = i;
                loop {
                    match *bytes.get(j)? {
                        b'\\' => j += 1,
                        b'(' => nesting += 1,
                        b')' => {
                            nesting -= 1;
                            if nesting == 0 {
                                return Some(j + 1);
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
            b'<' if bytes.get(i + 1) == Some(&b'<') => {
                let mut j = skip_white(bytes, i + 2);
                while !bytes[j..].starts_with(b">>") {
                    if bytes.get(j)? != &b'/' {
                        return None;
                    }
                    j = skip_white(bytes, read_name(bytes, j)?.1);
                    j = skip_white(bytes, skip_object(bytes, j, depth + 1)?);
                }
                Some(j + 2)
            }
            b'<' => Some(i + memchr::memchr(b'>', &bytes[i..])? + 1),
            b'[' => {
                let mut j = skip_white(bytes, i + 1);
                while *bytes.get(j)? != b']' {
                    j = skip_white(bytes, skip_object(bytes, j, depth + 1)?);
                }
                Some(j + 1)
            }
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                let mut j = i + 1;
                while j < bytes.len() && matches!(bytes[j], b'0'..=b'9' | b'.') {
                    j += 1;
                }
                let k = skip_white(bytes, j);
                if let Some((_, after)) = read_number(bytes, k) {
                    let r = skip_white(bytes, after);
                    if bytes.get(r) == Some(&b'R')
                        && bytes
                            .get(r + 1)
                            .is_none_or(|&b| is_white(b) || is_delimiter(b))
                    {
                        return Some(r + 1);
                    }
                }
                Some(j)
            }
            b if b.is_ascii_alphabetic() => {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                Some(j)
            }
            _ => None,
        }
    }

    fn read_dict(bytes: &[u8], start: usize) -> Option<(Keys, usize)> {
        if !bytes[start..].starts_with(b"<<") {
            return None;
        }
        let mut keys = Keys::default();
        let mut i = start + 2;
        loop {
            i = skip_white(bytes, i);
            if bytes[i..].starts_with(b">>") {
                return Some((keys, i + 2));
            }
            if bytes.get(i) != Some(&b'/') {
                return None;
            }
            let (key, after_key) = read_name(bytes, i)?;
            let value_start = skip_white(bytes, after_key);
            let value_end = skip_object(bytes, value_start, 1)?;
            record_key(&mut keys, bytes, &key, value_start, value_end);
            i = value_end;
        }
    }

    /// Every position: the same answers as the direct readings. A bad
    /// `/Filter` value made the old dictionary reading fail as a whole; here
    /// it is kept as `bad_filter`, so that is compared too.
    /// How many kept results, joins and key records the in-order pass made.
    fn compare(bytes: &[u8]) -> (usize, usize, usize) {
        let mut scan = Scan::new(bytes);
        for i in 0..=bytes.len() {
            assert_eq!(scan.skip_white(i), skip_white(bytes, i), "skip_white {i}");
            let next = (i..bytes.len())
                .find(|&k| !is_white(bytes[k]))
                .unwrap_or(bytes.len());
            assert_eq!(scan.next_non_white(i), next, "next_non_white {i}");
            let last = (0..i).rev().find(|&k| !is_white(bytes[k]));
            assert_eq!(
                scan.last_non_white_before(i),
                last,
                "last_non_white_before {i}"
            );
            if i < bytes.len() && bytes[i] == b'(' {
                let direct = skip_object(bytes, i, 0);
                assert_eq!(scan.string_end(i), direct, "string {i}");
            }
            for depth in [0, 1, 30, MAX_DICT_DEPTH] {
                if i < bytes.len() {
                    assert_eq!(
                        scan.skip_object(i, depth),
                        skip_object(bytes, i, depth),
                        "skip_object {i} at {depth}"
                    );
                }
            }
            if i < bytes.len() {
                assert_eq!(scan.read_dict(i), read_dict(bytes, i), "read_dict {i}");
            }
        }
        // As the raw-byte check uses it: dictionaries read in file order,
        // dropping what can no longer be reached.
        let mut scan = Scan::new(bytes);
        let mut records = 0;
        for i in 0..bytes.len() {
            scan.forget_before(i);
            assert_eq!(
                scan.read_dict(i),
                read_dict(bytes, i),
                "in order, read_dict {i}"
            );
            records = records.max(scan.key_records.len());
        }
        for word in [
            Word::Stream,
            Word::Endstream,
            Word::Endobj,
            Word::ObjStm,
            Word::XRef,
        ] {
            for i in 0..=bytes.len() {
                let direct = memmem::find(&bytes[i.min(bytes.len())..], word.bytes())
                    .map_or(bytes.len(), |at| i + at);
                assert_eq!(scan.next_word(word, i), direct, "{word:?} {i}");
            }
        }
        (scan.rests.len(), scan.joins.len(), records)
    }

    /// Reads a dictionary at every `obj` keyword in file order, as the
    /// raw-byte check does, and returns the elements walked per byte.
    fn steps_per_byte(bytes: &[u8]) -> f64 {
        let mut scan = Scan::new(bytes);
        for at in memmem::find_iter(bytes, b"obj") {
            let start = scan.skip_white(at + 3);
            if bytes[start..].starts_with(b"<<") {
                scan.forget_before(start);
                let _ = scan.read_dict(start);
            }
        }
        scan.steps as f64 / bytes.len() as f64
    }

    /// A named way to build an input of about the given size.
    type Family = (&'static str, fn(usize) -> Vec<u8>);

    /// Readings from many starting points that join the same long
    /// dictionary or array: the work stays a small multiple of the size,
    /// at every size (it would grow with the size if each reading walked
    /// to the end again).
    #[test]
    fn joined_readings_are_linear() {
        let families: [Family; 4] = [
            ("comments", |n| {
                let mut out = b"1 0 obj << ".to_vec();
                while out.len() < n {
                    out.extend(b"% 2 0 obj <<\n/A 1 /B (x) ".iter());
                }
                out.extend(b">>".iter());
                out
            }),
            ("keys after joins", |n| {
                let mut out = b"1 0 obj << ".to_vec();
                let mut i = 0;
                while out.len() < n {
                    out.extend(
                        format!("% {i} 0 obj <<\n/Length {i} /Filter /AHx /Type /XRef ").bytes(),
                    );
                    i += 1;
                }
                out.extend(b">>".iter());
                out
            }),
            ("nested", |n| {
                let mut out = b"1 0 obj << /A [ ".to_vec();
                while out.len() < n {
                    out.extend(b"% 2 0 obj << /K [\n1 [2] << /C 3 >> ".iter());
                }
                out.extend(b"] >>".iter());
                out
            }),
            ("strings and hex", |n| {
                let mut out = b"1 0 obj << ".to_vec();
                while out.len() < n {
                    out.extend(b"/S (2 0 obj << /K % \\)\n) /H <3 0 obj << /L <> ".iter());
                }
                out.extend(b">>".iter());
                out
            }),
        ];
        for (name, family) in families {
            let small = steps_per_byte(&family(1 << 16));
            let large = steps_per_byte(&family(1 << 20));
            assert!(large < 2.0, "{name}: {large} elements per byte");
            assert!(
                large < small * 1.5 + 0.1,
                "{name}: {small} then {large} per byte"
            );
        }
    }

    /// Inputs made of PDF-syntax pieces, so that strings, comments,
    /// escapes, nesting and keys all meet in many ways.
    fn generated(seed: u64, pieces: usize) -> Vec<u8> {
        const PIECES: &[&[u8]] = &[
            b"<<",
            b">>",
            b"[",
            b"]",
            b"(",
            b")",
            b"\\",
            b"\\\\",
            b"%",
            b"\n",
            b"\r",
            b" ",
            b"/Filter",
            b"/Length",
            b"/Type",
            b"/ObjStm",
            b"/A",
            b"/F",
            b"/DecodeParms",
            b"/Columns",
            b"/Fl#61teDecode",
            b"#",
            b"1",
            b"12",
            b"0 R",
            b" R",
            b"obj",
            b"<4F>",
            b"<",
            b">",
            b"stream\n",
            b"endstream",
            b"endobj",
            b"XRef",
            b"{",
            b"x",
            b".5",
            b"-",
            b"\x00",
            b"\t",
        ];
        let mut state = seed;
        let mut out = Vec::new();
        for _ in 0..pieces {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            out.extend_from_slice(PIECES[(state >> 33) as usize % PIECES.len()]);
        }
        out
    }

    #[test]
    fn tables_answer_as_the_direct_readings() {
        for seed in 0..400 {
            compare(&generated(seed, 40 + (seed as usize % 7) * 60));
        }
        for seed in 400..440 {
            compare(&generated(seed, 3000));
        }
    }

    #[test]
    fn deep_and_long_inputs_answer_as_the_direct_readings() {
        let mut deep = Vec::new();
        for n in [10, 63, 64, 65, 66, 70, 140] {
            deep.extend(b"1 0 obj << /A ".iter());
            deep.extend(std::iter::repeat_n(b'[', n));
            deep.extend(b"1".iter());
            deep.extend(std::iter::repeat_n(b']', n));
            deep.extend(b" >>\n".iter());
            deep.extend(std::iter::repeat_n(b"<< /K ".as_slice(), n).flatten());
            deep.extend(std::iter::repeat_n(b">> ".as_slice(), n).flatten());
            deep.push(b'\n');
        }
        compare(&deep);
        // A member that takes the dictionary past the limit (64 levels of
        // its own, one more for the dictionary), then more members across
        // blocks that another reading joins: from there on the rest reads.
        let mut past = b"1 0 obj << /A ".to_vec();
        past.extend(std::iter::repeat_n(b'[', 64));
        past.push(b'1');
        past.extend(std::iter::repeat_n(b']', 64));
        for i in 0..40 {
            past.extend(format!(" /B{i} 1 % {i} 0 obj <<\n/C 2").bytes());
        }
        past.extend(b" >>\n".iter());
        compare(&past);
        // Depth that only shows after kept results: long arrays and
        // dictionaries whose deep members come late, read again from inside
        // at other depths.
        let mut late = Vec::new();
        for n in [20, 33, 34, 62, 63, 64] {
            late.extend(b"1 0 obj << /A [ ".iter());
            late.extend(b"1 ".repeat(80));
            late.extend(std::iter::repeat_n(b'[', n));
            late.extend(b"1".iter());
            late.extend(std::iter::repeat_n(b']', n));
            late.extend(b" ] /B << ".iter());
            late.extend(b"/K 1 ".repeat(30));
            late.extend(b"/D ".iter());
            late.extend(std::iter::repeat_n(b"<< /E ".as_slice(), n).flatten());
            late.extend(b"1 ".iter());
            late.extend(std::iter::repeat_n(b">> ".as_slice(), n).flatten());
            late.extend(b">> >>\n".iter());
        }
        compare(&late);
        // Long runs that cross many blocks: comments, white space, strings,
        // a long flat dictionary entered from many places, with keys the
        // check reads after the places where other readings join it.
        let mut long = b"1 0 obj\n<< ".to_vec();
        for i in 0..300 {
            long.extend(format!("% {i} 0 obj <<\n/A {i} ").bytes());
            long.extend(b"/S (a (b) \\) c) /H <414243> /R [1 2 3] ".iter());
            match i % 7 {
                0 => long.extend(format!("/Length {i} ").bytes()),
                1 => long.extend(b"/Filter /FlateDecode ".iter()),
                2 => long.extend(b"/Type /ObjStm /N 3 ".iter()),
                3 => long.extend(b"/Filter (bad) ".iter()),
                4 => long.extend(b"/T (% 9 0 obj << /Filter /AHx /L <) ".iter()),
                5 => long.extend(b"/DecodeParms << /Predictor 12 /Columns 4 >> ".iter()),
                _ => long.extend(format!("/S <{i}> % {i} 0 obj << /F 1\n").bytes()),
            }
        }
        long.extend(b">>\nstream\nx\nendstream\n".iter());
        let (rests, joins, records) = compare(&long);
        // Readings joined kept results and read keys along joined walks.
        assert!(
            rests > 100 && joins > 0 && records > 0,
            "{rests} {joins} {records}"
        );
        // The same, as array members and nested several levels down.
        let nested = [
            b"2 0 obj << /A [ << /B [ ".to_vec(),
            long[10..].to_vec(),
            b" ] >> ] >>".to_vec(),
        ]
        .concat();
        compare(&nested);
    }
}
