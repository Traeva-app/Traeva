//! Bounded, bytes-only extraction of the main WordprocessingML story (#66).
//! No ZIP entry is written to disk and no relationship target is opened.

use crate::{MAX_MEMORY_TEXT_CHARS, TextExtraction, TextStatus, TextSummary};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io::{Cursor, Read},
};
use zip::{CompressionMethod, ZipArchive, read::HasZipMetadata};

#[path = "docx_cached.rs"]
mod cached;
#[path = "docx_preview.rs"]
mod preview;
pub use cached::{DOCX_CACHED_READER, read_docx_cached};
pub use preview::preview_docx_snapshot;

pub const DOCX_READER: &str = "docx-2";
pub const MAX_DOCX_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_ZIP_ENTRIES: usize = 512;
pub const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_UNPACKED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_COMPRESSION_RATIO: u64 = 200;
pub const MAX_XML_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_XML_DEPTH: usize = 64;
pub const MAX_XML_EVENTS: usize = 500_000;
pub const MAX_NAMESPACE_WORK: usize = 1_000_000;
const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT_WORD: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const MAIN_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

type Result<T> = std::result::Result<T, Problem>;
#[derive(Debug)]
struct Problem(TextStatus, &'static str);
fn damaged() -> Problem {
    Problem(
        TextStatus::Failed,
        "The DOCX package or XML is damaged or inconsistent.",
    )
}
fn unsupported(note: &'static str) -> Problem {
    Problem(TextStatus::Unsupported, note)
}

/// Reads a bounded DOCX package. Failure discards all text, including any prefix.
pub fn read_docx(bytes: &[u8]) -> TextExtraction {
    summarize(extract(bytes), DOCX_READER)
}

fn summarize(outcome: Result<BodyExtraction>, reader: &str) -> TextExtraction {
    let mut total_chars = 0;
    let mut extraction = match outcome {
        Ok((text, omissions, _)) if text.is_empty() && !omissions.is_empty() => TextExtraction {
            summary: TextSummary {
                reader: reader.into(),
                status: TextStatus::Unsupported,
                chars: 0,
                covered: vec![],
                left_out: vec![],
                note: Some(format!(
                    "The document has no text in the supported main-body subset. Omitted content: {}.",
                    omissions.into_iter().collect::<Vec<_>>().join(", ")
                )),
            },
            text,
        },
        Ok((text, omissions, total)) => {
            total_chars = total;
            TextExtraction {
                summary: TextSummary {
                    reader: reader.into(),
                    status: if omissions.is_empty() {
                        TextStatus::Complete
                    } else {
                        TextStatus::Partial
                    },
                    chars: text.chars().count(),
                    covered: vec![
                        "main body paragraphs, runs, tabs and explicit line breaks".into(),
                    ],
                    left_out: omissions.into_iter().map(String::from).collect(),
                    note: None,
                },
                text,
            }
        }
        Err(Problem(status, note)) => TextExtraction {
            summary: TextSummary {
                reader: reader.into(),
                status,
                chars: 0,
                covered: vec![],
                left_out: vec![],
                note: Some(note.into()),
            },
            text: String::new(),
        },
    };
    if extraction.summary.status == TextStatus::Partial {
        if total_chars > MAX_MEMORY_TEXT_CHARS {
            extraction.summary.covered = vec![format!(
                "the first {MAX_MEMORY_TEXT_CHARS} characters of supported main-body text"
            )];
            extraction
                .summary
                .left_out
                .retain(|s| s != "text beyond the 1,000,000-character limit");
            extraction.summary.left_out.push(format!(
                "the last {} characters of supported main-body text",
                total_chars - MAX_MEMORY_TEXT_CHARS
            ));
            extraction.summary.note = Some(format!(
                "The supported main-body text has {total_chars} characters; Traeva keeps at most {MAX_MEMORY_TEXT_CHARS}."
            ));
        } else {
            extraction.summary.note = Some("Only supported main-body text was retained; see left_out. Formatting and layout are not interpreted.".into());
        }
    }
    extraction
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(damaged)?
            .try_into()
            .map_err(|_| damaged())?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(damaged)?
            .try_into()
            .map_err(|_| damaged())?,
    ))
}
fn signature(bytes: &[u8], at: usize, expected: &[u8]) -> Result<()> {
    if bytes.get(at..at + expected.len()) == Some(expected) {
        Ok(())
    } else {
        Err(damaged())
    }
}

// Bound and validate the directory before ZipArchive allocates entries. Check
// local headers too: a ZIP parser's central-directory view alone is insufficient.
struct ZipEntry {
    name: String,
    central: usize,
    local: usize,
    data: usize,
    flags: u16,
    method: CompressionMethod,
    crc: u32,
    packed: u32,
    unpacked: u32,
}
fn preflight(bytes: &[u8]) -> Result<Vec<ZipEntry>> {
    if bytes.len() > MAX_DOCX_BYTES {
        return Err(unsupported("DOCX exceeds the 20 MiB input limit."));
    }
    if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        return Err(unsupported(
            "Encrypted Office containers and legacy binary Word files are unsupported.",
        ));
    }
    signature(bytes, 0, b"PK\x03\x04")?;
    let end = (bytes.len().saturating_sub(65_557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            bytes.get(i..i + 4) == Some(b"PK\x05\x06")
                && u16_at(bytes, i + 20).is_ok_and(|n| i + 22 + n as usize == bytes.len())
        })
        .ok_or_else(damaged)?;
    // zip's relaxed EOCD search accepts signatures inside a trailing comment.
    if u16_at(bytes, end + 20)? != 0 {
        return Err(unsupported("ZIP archive comments are unsupported."));
    }
    let count = u16_at(bytes, end + 10)? as usize;
    if count == u16::MAX as usize
        || u32_at(bytes, end + 12)? == u32::MAX
        || u32_at(bytes, end + 16)? == u32::MAX
    {
        return Err(unsupported("ZIP64 is unsupported."));
    }
    if count > MAX_ZIP_ENTRIES {
        return Err(unsupported("DOCX exceeds the 512 ZIP-entry limit."));
    }
    if u16_at(bytes, end + 4)? != 0
        || u16_at(bytes, end + 6)? != 0
        || u16_at(bytes, end + 8)? as usize != count
    {
        return Err(unsupported("Multi-volume ZIP packages are unsupported."));
    }
    let central = u32_at(bytes, end + 16)? as usize;
    if central + u32_at(bytes, end + 12)? as usize != end {
        return Err(damaged());
    }
    let mut at = central;
    let mut names = HashSet::new();
    let mut ranges = Vec::new();
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        signature(bytes, at, b"PK\x01\x02")?;
        let flags = u16_at(bytes, at + 8)?;
        let method = u16_at(bytes, at + 10)?;
        if flags & 0x2041 != 0 {
            return Err(unsupported("Encrypted ZIP entries are unsupported."));
        }
        if flags & !0x080e != 0 || !matches!(method, 0 | 8) {
            return Err(unsupported(
                "This ZIP compression method or flag is unsupported.",
            ));
        }
        let packed = u32_at(bytes, at + 20)?;
        let unpacked = u32_at(bytes, at + 24)?;
        let local = u32_at(bytes, at + 42)?;
        if [packed, unpacked, local].contains(&u32::MAX) {
            return Err(unsupported("ZIP64 is unsupported."));
        }
        if u16_at(bytes, at + 34)? != 0 {
            return Err(unsupported("Multi-volume ZIP packages are unsupported."));
        }
        let name_len = u16_at(bytes, at + 28)? as usize;
        let extra_len = u16_at(bytes, at + 30)? as usize;
        let comment_len = u16_at(bytes, at + 32)? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(damaged)?;
        let name = std::str::from_utf8(name).map_err(|_| damaged())?;
        let path = name.strip_suffix('/').unwrap_or(name);
        if name.is_empty()
            || name.len() > 240
            || !name.is_ascii()
            || name
                .bytes()
                .any(|b| b <= 32 || b == 127 || matches!(b, b'\\' | b':' | b'%'))
            || path.split('/').any(|p| matches!(p, "" | "." | ".."))
            || !names.insert(name.to_ascii_lowercase())
        {
            return Err(damaged());
        }
        check_extra(
            bytes
                .get(at + 46 + name_len..at + 46 + name_len + extra_len)
                .ok_or_else(damaged)?,
        )?;
        if method == 0 && packed != unpacked {
            return Err(damaged());
        }
        let local = local as usize;
        signature(bytes, local, b"PK\x03\x04")?;
        if u16_at(bytes, local + 6)? != flags
            || u16_at(bytes, local + 8)? != method
            || u16_at(bytes, local + 26)? as usize != name_len
        {
            return Err(damaged());
        }
        let local_extra = u16_at(bytes, local + 28)? as usize;
        if bytes.get(local + 30..local + 30 + name_len) != Some(name.as_bytes()) {
            return Err(damaged());
        }
        check_extra(
            bytes
                .get(local + 30 + name_len..local + 30 + name_len + local_extra)
                .ok_or_else(damaged)?,
        )?;
        let data = local + 30 + name_len + local_extra;
        let mut data_end = data + packed as usize;
        if flags & 8 == 0 {
            if u32_at(bytes, local + 14)? != u32_at(bytes, at + 16)?
                || u32_at(bytes, local + 18)? != packed
                || u32_at(bytes, local + 22)? != unpacked
            {
                return Err(damaged());
            }
        } else {
            // Both signed and unsigned 32-bit data descriptors are supported.
            let descriptor = data_end
                + usize::from(bytes.get(data_end..data_end + 4) == Some(b"PK\x07\x08")) * 4;
            if u32_at(bytes, descriptor)? != u32_at(bytes, at + 16)?
                || u32_at(bytes, descriptor + 4)? != packed
                || u32_at(bytes, descriptor + 8)? != unpacked
            {
                return Err(damaged());
            }
            data_end = descriptor + 12;
        }
        if data_end > central {
            return Err(damaged());
        }
        ranges.push((local, data_end));
        entries.push(ZipEntry {
            name: name.to_string(),
            central: at,
            local,
            data,
            flags,
            method: if method == 0 {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            },
            crc: u32_at(bytes, at + 16)?,
            packed,
            unpacked,
        });
        at += 46 + name_len + extra_len + comment_len;
        if at > end {
            return Err(damaged());
        }
    }
    if at != end {
        return Err(damaged());
    }
    ranges.sort_unstable();
    let mut previous_end = 0;
    for (start, finish) in ranges {
        if start != previous_end {
            return Err(damaged());
        }
        previous_end = finish;
    }
    if previous_end != central {
        return Err(damaged());
    }
    Ok(entries)
}
fn check_extra(mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        let kind = u16_at(bytes, 0)?;
        let size = u16_at(bytes, 2)? as usize;
        let payload = bytes.get(4..4 + size).ok_or_else(damaged)?;
        match kind {
            // Office's inert growth hint: signature, padding length, zero padding.
            0xa220
                if size >= 4
                    && u16_at(payload, 0)? == 0xa028
                    && u16_at(payload, 2)? as usize + 4 == size
                    && payload[4..].iter().all(|&b| b == 0) => {}
            1 => return Err(unsupported("ZIP64 is unsupported.")),
            0x7075 => return Err(unsupported("Unicode ZIP path aliases are unsupported.")),
            0x9901 => return Err(unsupported("AES ZIP metadata is unsupported.")),
            _ => return Err(unsupported("This ZIP extra field is unsupported.")),
        }
        bytes = bytes.get(4 + size..).ok_or_else(damaged)?;
    }
    Ok(())
}

fn same_entry<R: Read + ?Sized>(file: &zip::read::ZipFile<'_, R>, entry: &ZipEntry) -> bool {
    file.name() == entry.name
        && file.name_raw() == entry.name.as_bytes()
        && file.header_start() == entry.local as u64
        && file.central_header_start() == entry.central as u64
        && file.data_start() == Some(entry.data as u64)
        && file.get_metadata().flags == entry.flags
        && file.compression() == entry.method
        && file.crc32() == entry.crc
        && file.compressed_size() == entry.packed as u64
        && file.size() == entry.unpacked as u64
}

type BodyExtraction = (String, BTreeSet<&'static str>, usize);
type BodyReader = fn(&[u8], &mut XmlBudget, &mut BTreeSet<&'static str>) -> Result<(String, usize)>;

fn extract(bytes: &[u8]) -> Result<BodyExtraction> {
    extract_with(bytes, body_text)
}

fn extract_with(bytes: &[u8], read_body: BodyReader) -> Result<BodyExtraction> {
    let entries = preflight(bytes)?;
    let count = entries.len();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| damaged())?;
    if archive.len() != count || archive.offset() != 0 {
        return Err(damaged());
    }
    let mut total = 0u64;
    for (i, entry) in entries.iter().enumerate() {
        let file = archive.by_index_raw(i).map_err(|_| damaged())?;
        if !same_entry(&file, entry) {
            return Err(damaged());
        }
        if file.is_symlink() {
            return Err(unsupported("ZIP symbolic links are unsupported."));
        }
        if file.encrypted() {
            return Err(unsupported("Encrypted ZIP entries are unsupported."));
        }
        if file.size() > MAX_ENTRY_BYTES as u64 {
            return Err(unsupported("A DOCX entry exceeds the 8 MiB limit."));
        }
        if file.size() > file.compressed_size().max(1) * MAX_COMPRESSION_RATIO {
            return Err(unsupported(
                "A DOCX entry exceeds the 200:1 compression-ratio limit. Highly repetitive long documents can exceed this limit; the document is Unsupported, not truncated Partial text.",
            ));
        }
        total += file.size();
        if total > MAX_UNPACKED_BYTES as u64 {
            return Err(unsupported(
                "DOCX exceeds the 32 MiB expanded-package limit.",
            ));
        }
    }
    let mut omissions = BTreeSet::new();
    let mut budget = XmlBudget::default();
    let mut parts = BTreeMap::new();
    let mut relationships = Vec::new();
    let mut types = false;
    let mut main_relation = false;
    for i in 0..count {
        let mut file = archive.by_index_raw(i).map_err(|_| damaged())?;
        let name = file.name().to_ascii_lowercase();
        if name.eq_ignore_ascii_case("word/vbaProject.bin") {
            return Err(unsupported("Macro-bearing Word packages are unsupported."));
        }
        let mut packed = Vec::new();
        (&mut file)
            .take(MAX_DOCX_BYTES as u64 + 1)
            .read_to_end(&mut packed)
            .map_err(|_| damaged())?;
        if packed.len() as u64 != file.compressed_size() {
            return Err(damaged());
        }
        let data = match file.compression() {
            CompressionMethod::Stored => packed,
            CompressionMethod::Deflated => {
                let mut decoder = flate2::Decompress::new(false);
                let mut data = Vec::new();
                loop {
                    let before_in = decoder.total_in();
                    let before_out = decoder.total_out();
                    let mut block = [0u8; 32 * 1024];
                    let status = decoder
                        .decompress(
                            &packed[before_in as usize..],
                            &mut block,
                            flate2::FlushDecompress::None,
                        )
                        .map_err(|_| damaged())?;
                    let written = (decoder.total_out() - before_out) as usize;
                    if data.len() + written > MAX_ENTRY_BYTES {
                        return Err(unsupported("Expanded ZIP entry exceeds the 8 MiB limit."));
                    }
                    data.extend_from_slice(&block[..written]);
                    if status == flate2::Status::StreamEnd {
                        if decoder.total_in() != packed.len() as u64 {
                            return Err(damaged());
                        }
                        break;
                    }
                    if decoder.total_in() == before_in && decoder.total_out() == before_out {
                        return Err(damaged());
                    }
                }
                data
            }
            _ => return Err(unsupported("This ZIP compression method is unsupported.")),
        };
        if data.len() as u64 != file.size() || crc32fast::hash(&data) != file.crc32() {
            return Err(damaged());
        }
        if name.ends_with(".xml") || name.ends_with(".rels") {
            if name != "word/document.xml" {
                inspect_part(
                    &name,
                    &data,
                    &mut budget,
                    &mut relationships,
                    &mut types,
                    &mut main_relation,
                )?;
            }
            parts.insert(name, data);
        }
    }
    if !types || !main_relation {
        return Err(damaged());
    }
    for relation in relationships {
        let Some((root, omission)) = story_kind(&relation.kind) else {
            continue;
        };
        if relation.external {
            omissions.insert("external story relationship targets (never accessed)");
            continue;
        }
        let target = relationship_target(&relation.owner, &relation.target)?;
        let part = parts.get(&target).ok_or_else(damaged)?;
        if story_has_content(part, root, &mut budget)? {
            omissions.insert(omission);
        }
    }
    let document = parts.get("word/document.xml").ok_or_else(damaged)?;
    let (text, chars) = read_body(document, &mut budget, &mut omissions)?;
    Ok((text, omissions, chars))
}

#[derive(Clone)]
struct Element {
    ns: String,
    name: String,
    attrs: Vec<(String, String)>,
}
impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn word(&self) -> bool {
        self.ns == WORD || self.ns == STRICT_WORD
    }
    fn word_attr(&self, name: &str) -> Option<&str> {
        self.attr(&format!("{WORD}|{name}"))
            .or_else(|| self.attr(&format!("{STRICT_WORD}|{name}")))
    }
}
#[derive(Default)]
struct XmlBudget {
    events: usize,
    namespace_work: usize,
}
impl XmlBudget {
    fn events(&mut self, amount: usize) -> Result<()> {
        self.events += amount;
        if self.events > MAX_XML_EVENTS {
            return Err(unsupported("DOCX exceeds the XML-event/attribute budget."));
        }
        Ok(())
    }
    fn resolve(&mut self, bindings: usize) -> Result<()> {
        self.namespace_work += bindings;
        if self.namespace_work > MAX_NAMESPACE_WORK {
            return Err(unsupported(
                "DOCX exceeds the namespace-resolution work budget.",
            ));
        }
        Ok(())
    }
}
enum XmlEvent {
    Start(Element),
    End,
    Text(String),
}
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r')
        || matches!(c as u32, 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

// XML 1.0 Fifth Edition NameStartChar/NameChar, excluding colon in NCNames.
fn name_start(c: char) -> bool {
    c == '_'
        || c.is_ascii_alphabetic()
        || matches!(c as u32,
        0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2ff | 0x370..=0x37d |
        0x37f..=0x1fff | 0x200c..=0x200d | 0x2070..=0x218f |
        0x2c00..=0x2fef | 0x3001..=0xd7ff | 0xf900..=0xfdcf |
        0xfdf0..=0xfffd | 0x10000..=0xeffff)
}
fn qname(name: &str) -> bool {
    let mut count = 0;
    for part in name.split(':') {
        count += 1;
        if count > 2 {
            return false;
        }
        let mut chars = part.chars();
        if !chars.next().is_some_and(name_start)
            || !chars.all(|c| {
                name_start(c)
                    || c.is_ascii_digit()
                    || matches!(c, '-' | '.' | '\u{b7}')
                    || matches!(c as u32, 0x300..=0x36f | 0x203f..=0x2040)
            })
        {
            return false;
        }
    }
    true
}

fn xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}
fn xml_whitespace(text: &str) -> bool {
    text.chars().all(xml_space)
}
// quick-xml deliberately accepts adjacent attributes; require XML S separators.
fn attribute_syntax(mut raw: &str) -> Result<()> {
    while !raw.is_empty() {
        let rest = raw.trim_start_matches(xml_space);
        if rest.len() == raw.len() {
            return Err(damaged());
        }
        if rest.is_empty() {
            return Ok(());
        }
        let end = rest
            .find(|c: char| c == '=' || xml_space(c))
            .ok_or_else(damaged)?;
        if !qname(&rest[..end]) {
            return Err(damaged());
        }
        let value = rest[end..]
            .trim_start_matches(xml_space)
            .strip_prefix('=')
            .ok_or_else(damaged)?
            .trim_start_matches(xml_space);
        let quote = value.as_bytes().first().copied().ok_or_else(damaged)?;
        if quote != b'\'' && quote != b'"' {
            return Err(damaged());
        }
        let end = value[1..].find(quote as char).ok_or_else(damaged)? + 1;
        raw = &value[end + 1..];
    }
    Ok(())
}
fn reference_syntax(raw: &str) -> bool {
    if let Some(hex) = raw.strip_prefix("#x") {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(decimal) = raw.strip_prefix('#') {
        return !decimal.is_empty() && decimal.bytes().all(|b| b.is_ascii_digit());
    }
    matches!(raw, "amp" | "lt" | "gt" | "apos" | "quot")
}
fn attribute_references(mut raw: &str) -> Result<()> {
    while let Some(start) = raw.find('&') {
        raw = &raw[start + 1..];
        let end = raw.find(';').ok_or_else(damaged)?;
        if !reference_syntax(&raw[..end]) {
            return Err(damaged());
        }
        raw = &raw[end + 1..];
    }
    Ok(())
}

fn xml(
    bytes: &[u8],
    budget: &mut XmlBudget,
    mut visit: impl FnMut(XmlEvent) -> Result<()>,
) -> Result<()> {
    if bytes.len() > MAX_XML_BYTES {
        return Err(unsupported("An XML part exceeds the 4 MiB limit."));
    }
    if bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
        || bytes.starts_with(&[b'<', 0])
        || bytes.starts_with(&[0, b'<'])
    {
        return Err(unsupported("Only UTF-8 DOCX XML is supported."));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| damaged())?;
    if !text.chars().all(xml_char) {
        return Err(damaged());
    }
    let mut reader = NsReader::from_str(text.strip_prefix('\u{feff}').unwrap_or(text));
    reader.config_mut().check_comments = true;
    reader.resolver_mut().set_max_namespace_bindings(256);
    let mut depth = 0;
    let mut roots = 0;
    let mut declaration = false;
    let mut scopes = Vec::new();
    let mut bindings = 2;
    loop {
        budget.events(1)?;
        let before_event = reader.buffer_position();
        let event = reader.read_event().map_err(|error| match error {
            quick_xml::Error::Namespace(quick_xml::name::NamespaceError::TooManyBindings(_)) => {
                unsupported("DOCX exceeds the 256 namespace-binding limit.")
            }
            _ => damaged(),
        })?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                if depth == 0 {
                    roots += 1;
                    if roots != 1 {
                        return Err(damaged());
                    }
                }
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err(unsupported("DOCX exceeds the XML-depth limit."));
                }
                attribute_syntax(start.attributes_raw())?;
                if !qname(start.name().as_ref()) {
                    return Err(damaged());
                }
                if start.name().as_ref().len() > 256 {
                    return Err(unsupported("XML names exceed the 256-byte limit."));
                }
                let mut added = 0;
                let mut count = 0;
                for attr in start.attributes().with_checks(false) {
                    let attr = attr.map_err(|_| damaged())?;
                    count += 1;
                    if count > 64 {
                        return Err(unsupported("XML element exceeds the attribute limit."));
                    }
                    if attr.key.as_ref() == "xmlns" || attr.key.as_ref().starts_with("xmlns:") {
                        added += 1;
                    }
                }
                budget.events(count)?;
                bindings += added;
                scopes.push(added);
                budget.resolve(bindings)?;
                let (ns, name) = reader.resolver().resolve_element(start.name());
                let ns = match ns {
                    ResolveResult::Bound(n) if n.as_ref().len() <= 1024 => n.as_ref().to_string(),
                    ResolveResult::Bound(_) => {
                        return Err(unsupported(
                            "XML namespace URI exceeds the 1024-byte limit.",
                        ));
                    }
                    ResolveResult::Unbound => String::new(),
                    ResolveResult::Unknown(_) => return Err(damaged()),
                };
                let name = name.as_ref().to_string();
                if name.is_empty() {
                    return Err(damaged());
                }
                let mut attrs = Vec::new();
                let mut expanded = HashSet::new();
                for attr in start.attributes() {
                    if attrs.len() == 64 {
                        return Err(unsupported("XML element exceeds the attribute limit."));
                    }
                    let attr = attr.map_err(|_| damaged())?;
                    attribute_references(&attr.value)?;
                    let key = attr.key.as_ref().to_string();
                    if !qname(&key) || attr.value.contains('<') {
                        return Err(damaged());
                    }
                    if key.len() > 256 {
                        return Err(unsupported("XML names exceed the 256-byte limit."));
                    }
                    let mut stored_key = key.clone();
                    if key != "xmlns" && !key.starts_with("xmlns:") {
                        if key.contains(':') {
                            budget.resolve(bindings)?;
                        }
                        let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                        let namespace = match ns {
                            ResolveResult::Bound(n) if n.as_ref().len() <= 1024 => {
                                n.as_ref().to_string()
                            }
                            ResolveResult::Bound(_) => {
                                return Err(unsupported(
                                    "XML namespace URI exceeds the 1024-byte limit.",
                                ));
                            }
                            ResolveResult::Unbound => String::new(),
                            ResolveResult::Unknown(_) => return Err(damaged()),
                        };
                        if !expanded.insert((namespace.to_string(), local.as_ref().to_string())) {
                            return Err(damaged());
                        }
                        if !namespace.is_empty() {
                            stored_key = format!("{namespace}|{}", local.as_ref());
                        }
                    }
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|_| damaged())?
                        .into_owned();
                    if !value.chars().all(xml_char) {
                        return Err(damaged());
                    }
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        if value.len() > 1024 {
                            return Err(unsupported(
                                "XML namespace URI exceeds the 1024-byte limit.",
                            ));
                        }
                        const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
                        const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";
                        if attr.value.contains('&') {
                            return Err(unsupported(
                                "Escaped namespace declarations are unsupported.",
                            ));
                        }
                        if (key.starts_with("xmlns:") && value.is_empty())
                            || key == "xmlns:xmlns"
                            || value == XMLNS_NS
                            || (key == "xmlns:xml") != (value == XML_NS)
                        {
                            return Err(damaged());
                        }
                    }
                    attrs.push((stored_key, value));
                }
                visit(XmlEvent::Start(Element { ns, name, attrs }))?;
                if empty {
                    visit(XmlEvent::End)?;
                    depth -= 1;
                    bindings -= scopes.pop().ok_or_else(damaged)?;
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    return Err(damaged());
                }
                visit(XmlEvent::End)?;
                depth -= 1;
                bindings -= scopes.pop().ok_or_else(damaged)?;
            }
            Event::Text(value) => {
                if value.as_ref().contains("]]>") {
                    return Err(damaged());
                }
                let value = value.xml10_content().into_owned();
                if depth == 0 && !xml_whitespace(&value) {
                    return Err(damaged());
                }
                visit(XmlEvent::Text(value))?;
            }
            Event::CData(value) => {
                if depth == 0 {
                    return Err(damaged());
                }
                visit(XmlEvent::Text(value.xml10_content().into_owned()))?;
            }
            Event::GeneralRef(reference) => {
                if depth == 0 {
                    return Err(damaged());
                }
                if !reference_syntax(reference.as_ref()) {
                    return Err(damaged());
                }
                let value = match reference.resolve_char_ref().map_err(|_| damaged())? {
                    Some(c) if xml_char(c) => c,
                    Some(_) => return Err(damaged()),
                    None => match reference.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "apos" => '\'',
                        "quot" => '"',
                        _ => return Err(damaged()),
                    },
                };
                visit(XmlEvent::Text(value.to_string()))?;
            }
            Event::Decl(decl) => {
                if before_event != 0
                    || declaration
                    || roots != 0
                    || decl.version().map_err(|_| damaged())?.as_ref() != "1.0"
                {
                    return Err(damaged());
                }
                declaration = true;
                let declaration_start =
                    quick_xml::events::BytesStart::from_content(decl.as_ref(), 3);
                attribute_syntax(declaration_start.attributes_raw())?;
                let mut last = 0;
                for attr in declaration_start.attributes() {
                    let attr = attr.map_err(|_| damaged())?;
                    let order = match attr.key.as_ref() {
                        "version" if attr.value == "1.0" => 1,
                        "encoding" => {
                            if !attr.value.eq_ignore_ascii_case("UTF-8") {
                                return Err(unsupported("Only UTF-8 DOCX XML is supported."));
                            }
                            2
                        }
                        "standalone" if matches!(attr.value.as_ref(), "yes" | "no") => 3,
                        _ => return Err(damaged()),
                    };
                    if order <= last || (last == 0 && order != 1) {
                        return Err(damaged());
                    }
                    last = order;
                }
            }
            Event::DocType(_) => {
                return Err(unsupported("DTD and entity declarations are unsupported."));
            }
            Event::PI(_) => {
                return Err(unsupported("XML processing instructions are unsupported."));
            }
            Event::Comment(_) => (),
            Event::Eof => break,
        }
    }
    if depth != 0 || roots != 1 {
        return Err(damaged());
    }
    Ok(())
}

struct Relationship {
    owner: String,
    target: String,
    kind: String,
    external: bool,
}
fn story_kind(kind: &str) -> Option<(&'static str, &'static str)> {
    let local = kind
        .strip_prefix("http://schemas.openxmlformats.org/officeDocument/2006/relationships/")
        .or_else(|| {
            kind.strip_prefix("http://purl.oclc.org/ooxml/officeDocument/relationships/")
        })?;
    match local {
        "header" => Some(("hdr", "headers and footers")),
        "footer" => Some(("ftr", "headers and footers")),
        "footnotes" => Some(("footnotes", "footnotes and endnotes")),
        "endnotes" => Some(("endnotes", "footnotes and endnotes")),
        "comments" => Some(("comments", "comments")),
        _ => None,
    }
}
fn relationship_target(owner: &str, target: &str) -> Result<String> {
    if target.len() > 240
        || !target.is_ascii()
        || target
            .bytes()
            .any(|b| b <= 32 || b == 127 || matches!(b, b'\\' | b':' | b'%' | b'?' | b'#'))
    {
        return Err(unsupported(
            "This internal relationship target is unsupported.",
        ));
    }
    let mut parts = if target.starts_with('/') {
        vec![]
    } else {
        owner
            .rsplit_once('/')
            .map_or(vec![], |(dir, _)| dir.split('/').collect())
    };
    for part in target.strip_prefix('/').unwrap_or(target).split('/') {
        match part {
            "." => (),
            ".." => {
                parts.pop().ok_or_else(damaged)?;
            }
            "" => return Err(damaged()),
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/").to_ascii_lowercase())
}
fn story_has_content(bytes: &[u8], root: &str, budget: &mut XmlBudget) -> Result<bool> {
    let mut stack = Vec::<(Element, bool)>::new();
    let mut content = false;
    xml(bytes, budget, |event| {
        match event {
            XmlEvent::Start(e) => {
                if stack.is_empty() && (!e.word() || e.name != root) {
                    return Err(damaged());
                }
                let skip = stack.last().is_some_and(|(_, skip)| *skip)
                    || (e.word()
                        && matches!(e.name.as_str(), "footnote" | "endnote")
                        && matches!(
                            e.word_attr("type"),
                            Some("separator" | "continuationSeparator" | "continuationNotice")
                        ));
                if !skip
                    && e.word()
                    && matches!(
                        e.name.as_str(),
                        "drawing"
                            | "pict"
                            | "object"
                            | "tbl"
                            | "fldSimple"
                            | "fldChar"
                            | "sym"
                            | "tab"
                            | "br"
                            | "cr"
                    )
                {
                    content = true;
                }
                stack.push((e, skip));
            }
            XmlEvent::End => {
                stack.pop().ok_or_else(damaged)?;
            }
            XmlEvent::Text(t) => {
                if stack.last().is_some_and(|(e, skip)| {
                    !skip
                        && ((!t.is_empty()
                            && e.word()
                            && matches!(e.name.as_str(), "t" | "delText" | "instrText"))
                            || !xml_whitespace(&t))
                }) {
                    content = true;
                }
            }
        }
        Ok(())
    })?;
    Ok(content)
}

fn inspect_part(
    name: &str,
    bytes: &[u8],
    budget: &mut XmlBudget,
    relationships: &mut Vec<Relationship>,
    types: &mut bool,
    main_relation: &mut bool,
) -> Result<()> {
    let mut stack = Vec::<Element>::new();
    let mut ids = HashSet::new();
    xml(bytes, budget, |event| {
        match event {
            XmlEvent::Start(e) => {
                if stack.is_empty()
                    && ((name == "[content_types].xml" && (e.ns != TYPES || e.name != "Types"))
                        || (name.ends_with(".rels") && (e.ns != RELS || e.name != "Relationships")))
                {
                    return Err(damaged());
                }
                if name == "[content_types].xml"
                    && !stack.is_empty()
                    && (stack.len() != 1
                        || e.ns != TYPES
                        || !matches!(e.name.as_str(), "Override" | "Default"))
                {
                    return Err(damaged());
                }
                if name == "[content_types].xml"
                    && e.ns == TYPES
                    && matches!(e.name.as_str(), "Override" | "Default")
                {
                    let kind = e.attr("ContentType").ok_or_else(damaged)?;
                    if kind.to_ascii_lowercase().contains("macroenabled")
                        || kind.to_ascii_lowercase().contains("vbaproject")
                    {
                        return Err(unsupported("Macro-bearing Word packages are unsupported."));
                    }
                    if e.attr("PartName")
                        .is_some_and(|p| p.eq_ignore_ascii_case("/word/document.xml"))
                    {
                        if *types {
                            return Err(damaged());
                        }
                        if kind != MAIN_TYPE {
                            return Err(unsupported(
                                "This OOXML main-part content type is unsupported.",
                            ));
                        }
                        *types = true;
                    }
                }
                if name.ends_with(".rels") && e.ns == RELS && e.name == "Relationship" {
                    if stack.len() != 1 {
                        return Err(damaged());
                    }
                    let id = e.attr("Id").ok_or_else(damaged)?;
                    if id.is_empty() || !ids.insert(id.to_string()) {
                        return Err(damaged());
                    }
                    let target = e.attr("Target").ok_or_else(damaged)?;
                    let kind = e.attr("Type").ok_or_else(damaged)?;
                    let external = match e.attr("TargetMode") {
                        Some("External") => true,
                        None | Some("Internal") => false,
                        _ => return Err(damaged()),
                    };
                    if name == "word/_rels/document.xml.rels" {
                        relationships.push(Relationship {
                            owner: "word/document.xml".into(),
                            target: target.into(),
                            kind: kind.into(),
                            external,
                        });
                    }
                    if name == "_rels/.rels"
                        && matches!(
                            kind,
                            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
                                | "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument"
                        )
                    {
                        if external {
                            return Err(unsupported(
                                "An external main-document relationship is unsupported.",
                            ));
                        }
                        if *main_relation
                            || !target
                                .strip_prefix('/')
                                .unwrap_or(target)
                                .eq_ignore_ascii_case("word/document.xml")
                        {
                            return Err(damaged());
                        }
                        *main_relation = true;
                    }
                }
                stack.push(e);
            }
            XmlEvent::End => {
                stack.pop().ok_or_else(damaged)?;
            }
            XmlEvent::Text(t) => {
                if (name.ends_with(".rels") || name == "[content_types].xml") && !xml_whitespace(&t)
                {
                    return Err(damaged());
                }
            }
        }
        Ok(())
    })
}

fn body_text(
    bytes: &[u8],
    budget: &mut XmlBudget,
    omissions: &mut BTreeSet<&'static str>,
) -> Result<(String, usize)> {
    let mut stack = Vec::<(Element, bool)>::new();
    let mut output = String::new();
    let mut chars = 0;
    let mut paragraphs = 0;
    let mut bodies = 0;
    let mut previous_cr = false;
    let mut fields = Vec::new();
    let mut pending_paragraph_break = false;
    let mut append = |text: &str, omissions: &mut BTreeSet<&'static str>, boundary: bool| {
        if boundary {
            previous_cr = false;
        }
        for c in text.chars() {
            if c == '\n' && previous_cr {
                previous_cr = false;
                continue;
            }
            previous_cr = c == '\r';
            if chars >= MAX_MEMORY_TEXT_CHARS {
                omissions.insert("text beyond the 1,000,000-character limit");
            } else {
                output.push(if c == '\r' { '\n' } else { c });
            }
            chars += 1;
        }
    };
    xml(bytes, budget, |event| {
        match event {
            XmlEvent::Start(e) => {
                if stack.is_empty() && (!e.word() || e.name != "document") {
                    return Err(damaged());
                }
                let mut skip = stack.last().is_some_and(|(_, skip)| *skip);
                if e.word()
                    && matches!(
                        e.name.as_str(),
                        "ins"
                            | "del"
                            | "moveFrom"
                            | "moveTo"
                            | "pPrChange"
                            | "rPrChange"
                            | "sectPrChange"
                            | "tblPrChange"
                            | "trPrChange"
                            | "tcPrChange"
                            | "numberingChange"
                    )
                {
                    omissions.insert("tracked changes (no revision acceptance policy applied)");
                }
                if e.word() && e.name == "fldChar" {
                    omissions.insert("fields and field instructions");
                    match e.word_attr("fldCharType") {
                        Some("begin") => fields.push(false),
                        Some("separate") => {
                            let separated = fields.last_mut().ok_or_else(damaged)?;
                            if *separated {
                                return Err(damaged());
                            }
                            *separated = true;
                        }
                        Some("end") => {
                            fields.pop().ok_or_else(damaged)?;
                        }
                        _ => return Err(damaged()),
                    }
                    skip = true;
                }
                if !skip && e.word() {
                    match e.name.as_str() {
                        "document" => {
                            if !stack.is_empty() {
                                return Err(damaged());
                            }
                        }
                        "body" => {
                            if stack.len() != 1 || stack[0].0.name != "document" {
                                return Err(damaged());
                            }
                            bodies += 1;
                            if bodies != 1 {
                                return Err(damaged());
                            }
                        }
                        "p" => {
                            if !stack
                                .last()
                                .is_some_and(|(p, _)| p.name == "body" && p.word())
                            {
                                return Err(damaged());
                            }
                            if paragraphs > 0 {
                                pending_paragraph_break = !fields.is_empty();
                                if !pending_paragraph_break {
                                    append("\n", omissions, true);
                                }
                            }
                            paragraphs += 1;
                        }
                        "r" => {
                            if !stack.last().is_some_and(|(p, _)| {
                                p.word() && matches!(p.name.as_str(), "p" | "hyperlink")
                            }) {
                                return Err(damaged());
                            }
                        }
                        "hyperlink" => {
                            if !stack.last().is_some_and(|(p, _)| p.word() && p.name == "p") {
                                return Err(damaged());
                            }
                        }
                        "t" => {
                            if !stack.last().is_some_and(|(p, _)| p.word() && p.name == "r") {
                                return Err(damaged());
                            }
                        }
                        "tab" | "br" | "cr" => {
                            if !stack.last().is_some_and(|(p, _)| p.word() && p.name == "r") {
                                return Err(damaged());
                            }
                            if fields.is_empty() {
                                if pending_paragraph_break {
                                    append("\n", omissions, true);
                                    pending_paragraph_break = false;
                                }
                                append(if e.name == "tab" { "\t" } else { "\n" }, omissions, true);
                            }
                        }
                        "pPr" | "rPr" | "sectPr" => skip = true,
                        "bookmarkStart" | "bookmarkEnd" | "proofErr" | "lastRenderedPageBreak" => {
                            skip = true
                        }
                        "tbl" => {
                            omissions.insert("tables");
                            skip = true;
                        }
                        "ins" | "del" | "moveFrom" | "moveTo" => {
                            omissions
                                .insert("tracked changes (no revision acceptance policy applied)");
                            skip = true;
                        }
                        "drawing" | "pict" | "object" | "txbxContent" => {
                            omissions.insert("images, embedded objects and text boxes");
                            skip = true;
                        }
                        "fldSimple" | "instrText" => {
                            omissions.insert("fields and field instructions");
                            skip = true;
                        }
                        "footnoteReference" | "endnoteReference" => {
                            omissions.insert("footnotes and endnotes");
                            skip = true;
                        }
                        "commentReference" | "commentRangeStart" | "commentRangeEnd" => {
                            omissions.insert("comments");
                            skip = true;
                        }
                        _ => {
                            omissions.insert("unsupported WordprocessingML elements");
                            skip = true;
                        }
                    }
                } else if !skip {
                    omissions.insert("foreign namespaces and alternate content");
                    skip = true;
                }
                stack.push((e, skip));
            }
            XmlEvent::End => {
                stack.pop().ok_or_else(damaged)?;
            }
            XmlEvent::Text(t) => {
                if stack
                    .last()
                    .is_some_and(|(e, skip)| !skip && e.word() && e.name == "t")
                    && fields.is_empty()
                {
                    if pending_paragraph_break {
                        append("\n", omissions, true);
                        pending_paragraph_break = false;
                    }
                    append(&t, omissions, false);
                } else if !stack.last().is_some_and(|(_, skip)| *skip)
                    && fields.is_empty()
                    && !xml_whitespace(&t)
                {
                    return Err(damaged());
                }
            }
        }
        Ok(())
    })?;
    if bodies != 1 || !fields.is_empty() {
        return Err(damaged());
    }
    Ok((output, chars))
}

#[cfg(test)]
#[path = "docx_tests.rs"]
mod tests;
