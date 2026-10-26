//! Checks on the raw bytes, before the PDF library parses anything.
//!
//! hayro decodes streams without a size limit, so every stream is measured
//! here first with capped decoders, and the file is refused before hayro
//! sees it when a limit would be passed.
//!
//! Every stream is found after an `obj` keyword (`12 0 obj << ... >>
//! stream`), wherever that keyword is: each dictionary is read as PDF
//! syntax, so strings, comments and nested dictionaries are skipped whole and
//! a `(>>)` or a comment cannot hide a stream. Those are the candidates, and
//! every one is measured, so the size limits hold for anything the PDF
//! library could ever decode.
//!
//! Which candidates are the file's own objects is found by reading the file
//! from its header as PDF syntax, object after object, each stream's data
//! skipped by its `/Length` (`object_dicts`). Text inside a string, a hex
//! string, a comment or another stream's data that looks like an object is
//! not one. For the file's own streams every check stops the reading here;
//! for look-alikes only the resource limits do, and the other findings
//! (damage, a filter this check does not know or cannot measure) are kept
//! and apply only if the PDF library turns out to read them (see
//! `document.rs`).
//!
//! Where a stream's data ends follows hayro's own rule (`data_lengths`), and
//! a stream without filters is held to the same per-stream limit as a
//! decoded one. After hayro opens the file, `document.rs` checks that every
//! stream hayro reads is one measured here, by where its dictionary and its
//! data start and how long its data is.
//!
//! The trailer dictionaries are read by following `startxref`, `/Prev` and
//! `/XRefStm`, never by searching the file for words. This is a resource
//! check, not a PDF parser: it never decides what text a page holds. All
//! questions about the bytes go through `Scan`, so the whole check is linear
//! in the size of the file.
//!
//! Object and cross-reference streams are decoded while hayro opens the
//! file, before any later check. hayro gives a stream those roles by where
//! the file refers to it (a cross-reference section's offset, a member
//! entry), not by its `/Type`, so any stream at a section's offset or with
//! `/N` and `/First` is treated as one ("early"): every finding on it stops
//! the reading, and the data hayro could take for it is measured.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use flate2::{Decompress, FlushDecompress, Status};
use memchr::memmem;

use super::scan::{
    FilterList, Keys, Scan, Span, Word, hayro_number, is_delimiter, is_white, read_number,
    skip_white_plain,
};
use super::{
    MAX_DECODE_WORK, MAX_DECODED_BYTES, MAX_FILTERS, MAX_IMAGE_STREAM_BYTES, MAX_LENGTH_VALUES,
    MAX_OBJECTS, MAX_OBJSTM_SCAN, MAX_OBJSTM_TABLE_BYTES, MAX_PDF_BYTES, MAX_STREAM_BYTES,
    MAX_STREAMS, Stop,
};

/// What the raw-byte check learned, for every stream candidate.
#[derive(Debug, Default)]
pub(crate) struct Preflight {
    pub streams: HashMap<StreamAt, StreamCheck>,
    /// The file's own streams whose data is in another file (`/F`).
    pub external_streams: usize,
    /// Dictionary and array elements this check read, and bytes its
    /// decoders read and wrote (evidence only; no limit uses them).
    pub elements: usize,
    pub decode_work: usize,
}

/// Where a stream's dictionary (its `<<`) and its data start in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct StreamAt {
    pub dict: usize,
    pub data: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct StreamCheck {
    /// Bytes after all filters except a final image codec.
    pub decoded: usize,
    /// The compressed data breaks, ends before its own end, or (Flate) has
    /// no complete zlib wrapper or a wrong checksum.
    pub damaged: bool,
    /// The last filter is an image codec (JPEG and the like).
    pub image_codec: bool,
    /// Why this look-alike must not be read, if the PDF library reads it
    /// after all. Always `None` for the file's own streams: their findings
    /// stop the reading here.
    pub refusal: Option<Stop>,
    /// The data lengths hayro may take for this stream (`data_lengths`),
    /// sorted, each once; any other length was not measured here.
    pub lengths: Vec<usize>,
}

/// Cross-reference sections followed through `/Prev` and `/XRefStm`.
const MAX_XREF_SECTIONS: usize = 1_000;

pub(crate) fn check(bytes: &[u8]) -> Result<Preflight, Stop> {
    if bytes.len() > MAX_PDF_BYTES {
        return Err(Stop::unsupported(format!(
            "The PDF is larger than this reader's input limit ({} MiB).",
            MAX_PDF_BYTES >> 20
        )));
    }
    let Some(header) = memmem::find(&bytes[..bytes.len().min(1024)], b"%PDF-") else {
        return Err(Stop::failed("The file has no PDF header."));
    };
    let mut scan = Scan::new(bytes);
    let first = check_end(&scan, header)?;
    let (trailers, sections) = trailer_chain(&mut scan, header, first);
    if trailers.iter().any(|trailer| trailer.encrypt) {
        return Err(encrypted());
    }
    if trailers
        .iter()
        .any(|trailer| trailer.size.is_some_and(|size| size > MAX_OBJECTS))
    {
        return Err(Stop::unsupported(format!(
            "The PDF declares more objects than this reader's limit ({MAX_OBJECTS})."
        )));
    }
    scan_streams(&mut scan, header, &sections)
}

/// The last `startxref` must point at cross-reference data, and the file
/// must end with `%%EOF`. A truncated file fails here, before the PDF
/// library can quietly rebuild a shorter document from what is left.
/// Returns where that cross-reference data starts.
fn check_end(scan: &Scan, header: usize) -> Result<usize, Stop> {
    let bytes = scan.bytes;
    let truncated =
        || Stop::failed("The file ends before its end-of-file marker; it is truncated or damaged.");
    let damaged = || {
        Stop::failed("The PDF's cross-reference data is damaged (the offset at its end is wrong).")
    };
    let tail_start = bytes.len().saturating_sub(2048);
    let tail = &bytes[tail_start..];
    let eof = memmem::rfind(tail, b"%%EOF").ok_or_else(truncated)?;
    let at = memmem::rfind(&tail[..eof], b"startxref").ok_or_else(truncated)?;
    let mut i = tail_start + at + b"startxref".len();
    while i < bytes.len() && is_white(bytes[i]) {
        i += 1;
    }
    let offset = read_number(bytes, i).ok_or_else(damaged)?.0;
    section_start(scan, offset, header).ok_or_else(damaged)
}

/// Where a cross-reference section starts: at `offset`, or `header` bytes
/// later (some files carry bytes before the header, and readers then count
/// offsets from the header).
fn section_start(scan: &Scan, offset: usize, header: usize) -> Option<usize> {
    let bytes = scan.bytes;
    [offset, offset.saturating_add(header)]
        .into_iter()
        .filter(|&start| start < bytes.len())
        .find_map(|start| {
            let j = scan.next_non_white(start);
            (bytes[j..].starts_with(b"xref") || object_header(scan, j).is_some()).then_some(j)
        })
}

/// The trailer dictionaries the file's cross-reference data actually uses:
/// the section `startxref` points at, then every `/Prev` and `/XRefStm`.
/// A section that cannot be read ends that branch of the chain. Also where
/// the dictionary of every object at a section's offset starts: hayro reads
/// such an object as a cross-reference stream, whatever its `/Type`.
fn trailer_chain(scan: &mut Scan, header: usize, first: usize) -> (Vec<DictInfo>, HashSet<usize>) {
    let mut found = Vec::new();
    let mut sections = HashSet::new();
    let mut pending = vec![first];
    let mut seen = HashSet::new();
    while let Some(at) = pending.pop() {
        if seen.len() == MAX_XREF_SECTIONS || !seen.insert(at) {
            continue;
        }
        if let Some((_, after)) = object_header(scan, at) {
            sections.insert(scan.skip_white(after));
        }
        let Some(trailer) = section_trailer(scan, at) else {
            continue;
        };
        for offset in [trailer.prev, trailer.xref_stm].into_iter().flatten() {
            pending.extend(section_start(scan, offset, header));
        }
        found.push(trailer);
    }
    (found, sections)
}

/// The trailer of one cross-reference section: after a classic `xref`
/// table, or the dictionary of a cross-reference stream.
fn section_trailer(scan: &mut Scan, at: usize) -> Option<DictInfo> {
    let bytes = scan.bytes;
    if bytes[at..].starts_with(b"xref") {
        let mut i = at + b"xref".len();
        loop {
            i = scan.skip_white(i);
            if bytes[i..].starts_with(b"trailer") {
                let start = scan.skip_white(i + b"trailer".len());
                return read_info(scan, start).map(|(dict, _)| dict);
            }
            let (_, after) = read_number(bytes, i)?;
            let (count, after) = read_number(bytes, scan.skip_white(after))?;
            // An entry takes at least 18 bytes.
            if count > bytes.len() / 18 {
                return None;
            }
            i = after;
            for _ in 0..count {
                let (_, a) = read_number(bytes, scan.skip_white(i))?;
                let (_, b) = read_number(bytes, scan.skip_white(a))?;
                let k = scan.skip_white(b);
                if !matches!(bytes.get(k), Some(b'n' | b'f')) {
                    return None;
                }
                i = k + 1;
            }
        }
    }
    let (_, after) = object_header(scan, at)?;
    let start = scan.skip_white(after);
    let (dict, _) = read_info(scan, start)?;
    dict.xref.then_some(dict)
}

/// A candidate's dictionary, read.
struct Candidate {
    dict_at: usize,
    dict_end: usize,
    /// Where its `stream` keyword is, if one follows the dictionary.
    keyword: Option<usize>,
    dict: DictInfo,
    is_own: bool,
    /// hayro may decode it while opening the file: an object stream, a
    /// cross-reference stream, or a stream at a section's offset or with
    /// `/N` and `/First`.
    early: bool,
    /// At a cross-reference section's offset.
    section: bool,
}

/// Running totals of the decoding done here.
#[derive(Default)]
struct Totals {
    /// Bytes every filter read and wrote, and unfiltered data.
    work: usize,
    /// Output of all non-image streams.
    decoded: usize,
}

/// What hayro may spend reading object-stream members in one pass over all
/// objects: each member read reads its stream's offset table again (see
/// `object_stream`).
#[derive(Default)]
struct Members {
    /// The largest offset table, in bytes.
    table: usize,
    /// How many member reads a pass can make: the entries cross-reference
    /// streams can list, and the pairs hayro adds when it rebuilds
    /// cross-reference data from object streams.
    reads: usize,
}

fn scan_streams(
    scan: &mut Scan,
    header: usize,
    sections: &HashSet<usize>,
) -> Result<Preflight, Stop> {
    let bytes = scan.bytes;
    let mut lengths = length_objects(scan);
    let own = object_dicts(scan, header, &lengths)?;
    // First every candidate's dictionary, so an encrypted file is refused
    // before any of its (encrypted) streams is decoded.
    let mut candidates = Vec::new();
    let mut stream_count = 0;
    for dict_at in candidate_dicts(scan) {
        scan.forget_before(dict_at);
        let is_own = own.contains(&dict_at);
        let section = sections.contains(&dict_at);
        let Some((dict, dict_end)) = read_info(scan, dict_at) else {
            // A dictionary this check cannot read. A stream of the file's
            // own that this check cannot measure stops the reading. One
            // that may be decoded while hayro opens the file stops it too,
            // since hayro takes its data from the next `stream` anywhere.
            let early = section || names_decoded_early(scan, dict_at);
            if (is_own && stream_follows(scan, dict_at))
                || (early && scan.next_word(Word::Stream, dict_at) < bytes.len())
            {
                return Err(Stop::unsupported(
                    "A stream's dictionary could not be checked before reading.",
                ));
            }
            continue;
        };
        let keyword = scan.skip_white(dict_end);
        let keyword = is_stream_keyword(bytes, keyword).then_some(keyword);
        if keyword.is_some() {
            if is_own && dict.xref && dict.encrypt {
                return Err(encrypted());
            }
            if stream_count == MAX_STREAMS {
                return Err(Stop::unsupported(format!(
                    "The PDF has more streams than this reader's limit ({MAX_STREAMS})."
                )));
            }
            stream_count += 1;
        }
        let early = dict.object_stream || dict.xref || dict.has_members() || section;
        candidates.push(Candidate {
            dict_at,
            dict_end,
            keyword,
            dict,
            is_own,
            early,
            section,
        });
    }

    let mut found = Preflight::default();
    let mut totals = Totals::default();
    let mut members = Members::default();
    // Object streams first: an integer stored in one can be another
    // stream's /Length.
    let (object_streams, others): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| candidate.dict.has_members());
    for candidate in object_streams.iter().chain(&others) {
        if candidate.keyword.is_some() {
            measure_stream(
                scan,
                candidate,
                &mut lengths,
                &mut found,
                &mut totals,
                &mut members,
            )?;
        } else {
            measure_pseudo_stream(scan, candidate, &mut totals, &mut members)?;
        }
    }
    if members.reads.saturating_mul(members.table) > MAX_OBJSTM_SCAN {
        return Err(Stop::unsupported(
            "The PDF's object streams would be read more than this reader's limit allows.",
        ));
    }
    found.elements = scan.steps();
    found.decode_work = totals.work;
    Ok(found)
}

/// Measures one candidate followed by its `stream` keyword.
fn measure_stream(
    scan: &mut Scan,
    candidate: &Candidate,
    lengths: &mut Lengths,
    found: &mut Preflight,
    totals: &mut Totals,
    members: &mut Members,
) -> Result<(), Stop> {
    let bytes = scan.bytes;
    let (dict, is_own, early) = (&candidate.dict, candidate.is_own, candidate.early);
    let keyword = candidate.keyword.unwrap_or_default();
    if is_own && dict.external {
        found.external_streams += 1;
    }
    let at = StreamAt {
        dict: candidate.dict_at,
        data: data_start(bytes, keyword),
    };
    let record = |stop: Stop, lengths: &[usize], found: &mut Preflight| {
        found.streams.insert(
            at,
            StreamCheck {
                decoded: 0,
                damaged: false,
                image_codec: false,
                refusal: Some(stop),
                lengths: lengths.to_vec(),
            },
        );
    };
    let ranges = match data_lengths(scan, candidate.dict_end, keyword, dict.length, lengths) {
        Ok(ranges) => ranges,
        Err(stop) if is_own || early => return Err(stop),
        Err(stop) => {
            record(stop, &[], found);
            return Ok(());
        }
    };
    let cap = stream_cap(dict);
    let plan = match plan_filters(dict) {
        Ok(plan) => plan,
        Err(stop) if is_own || early => return Err(stop),
        Err(stop) => {
            record(stop, &ranges, found);
            return Ok(());
        }
    };
    if plan.unknown && is_own {
        return Err(unknown_filter());
    }
    match predictor_row(scan, dict) {
        Row::Fits(row) if row > cap => return Err(row_too_large(cap)),
        Row::Fits(_) => {}
        Row::Unknown if is_own || early => return Err(parameters_unknown()),
        Row::Unknown => {
            record(parameters_unknown(), &ranges, found);
            return Ok(());
        }
    }
    // An unfiltered stream decoded while hayro opens the file must have a
    // range this check knows: hayro may resolve an indirect /Length that
    // was not found here before any later check.
    if early
        && plan.filters.is_empty()
        && matches!(dict.length, Some(Length::Indirect(id)) if !lengths.contains_key(&id))
    {
        return Err(length_unchecked());
    }
    let size = ranges.iter().copied().max().unwrap_or(0);
    let measured = measure_data(bytes, &plan, at.data, size, cap, totals)?;
    let mut refusal = None;
    if !dict.image {
        totals.decoded += measured.size;
        if totals.decoded > MAX_DECODED_BYTES {
            return Err(Stop::unsupported(format!(
                "The PDF's non-image streams expand to more than this reader's limit ({} MiB).",
                MAX_DECODED_BYTES >> 20
            )));
        }
    }
    if measured.damaged && (!dict.image || early) {
        let damaged = Stop::failed("A compressed stream in the PDF is damaged.");
        if is_own || early {
            return Err(damaged);
        }
        refusal = Some(damaged);
    }
    roles(candidate, &measured.data, lengths, members)?;
    if early {
        // hayro's `parse_fallback` starts at the first `stream` after the
        // dictionary, which can come before the keyword (in a comment
        // between them): measure that range too.
        let first = scan.next_word(Word::Stream, candidate.dict_end);
        if first != keyword
            && let Some(data) = eol_after(bytes, first + b"stream".len())
        {
            let length = fallback_length(scan, data).unwrap_or(0);
            let other = measure_data(bytes, &plan, data, length, cap, totals)?;
            if other.damaged {
                return Err(Stop::failed("A compressed stream in the PDF is damaged."));
            }
            roles(candidate, &other.data, lengths, members)?;
        }
    }
    if plan.unknown {
        // A look-alike: hayro skips the name it does not know, and the
        // rest was measured above.
        refusal = Some(unknown_filter());
    }
    found.streams.insert(
        at,
        StreamCheck {
            decoded: measured.size,
            damaged: measured.damaged,
            image_codec: plan.image_codec,
            refusal,
            lengths: ranges,
        },
    );
    Ok(())
}

/// A dictionary with no `stream` keyword after it that names filters hayro
/// decodes, or that hayro may decode while opening the file ("early").
/// Asked for a stream there, hayro takes the data after the next `stream`
/// in the file (its `parse_fallback`) and decodes it with this dictionary's
/// filters: an early one while it opens the file, others when a page uses
/// them. The resource limits hold for that data too, and for an early one
/// every finding stops the reading. Others whose filters this check cannot
/// measure or whose parameters are stored elsewhere, and others without
/// filters, are not checked (README, residual risks); a dictionary naming
/// only filters hayro does not know (a signature's `/Filter
/// /Adobe.PPKLite`) is never decoded.
fn measure_pseudo_stream(
    scan: &mut Scan,
    candidate: &Candidate,
    totals: &mut Totals,
    members: &mut Members,
) -> Result<(), Stop> {
    let bytes = scan.bytes;
    let dict = &candidate.dict;
    let early = candidate.early;
    if matches!(dict.filters, Filters::None) && !early {
        return Ok(());
    }
    let keyword = scan.next_word(Word::Stream, candidate.dict_end);
    let Some(data) = eol_after(bytes, keyword + b"stream".len()) else {
        return Ok(());
    };
    let stop_if = |stop: Stop| {
        if candidate.is_own || early {
            Err(stop)
        } else {
            Ok(())
        }
    };
    let cap = stream_cap(dict);
    let plan = match plan_filters(dict) {
        Ok(plan) => plan,
        Err(stop) => return stop_if(stop),
    };
    if plan.filters.is_empty() && !early {
        return Ok(());
    }
    match predictor_row(scan, dict) {
        Row::Fits(row) if row > cap => return Err(row_too_large(cap)),
        Row::Fits(_) => {}
        Row::Unknown => return stop_if(parameters_unknown()),
    }
    let length = fallback_length(scan, data).unwrap_or(0);
    let measured = measure_data(bytes, &plan, data, length, cap, totals)?;
    if measured.damaged && early {
        return Err(Stop::failed("A compressed stream in the PDF is damaged."));
    }
    roles(candidate, &measured.data, &mut Lengths::new(), members)
}

/// The data hayro decodes from `data`, as measured here, and its size.
struct Measured<'b> {
    size: usize,
    damaged: bool,
    data: Cow<'b, [u8]>,
}

/// Measures the data at `data`: with filters, decoded from there to the end
/// of the file (each decoder stops at its own end, so a wrong `/Length`
/// cannot hide data); without, `length` bytes, held to the same per-stream
/// limit. Charges the work and checks its total.
fn measure_data<'b>(
    bytes: &'b [u8],
    plan: &Plan,
    data: usize,
    length: usize,
    cap: usize,
    totals: &mut Totals,
) -> Result<Measured<'b>, Stop> {
    let measured = if plan.filters.is_empty() {
        if length > cap {
            return Err(data_too_large(cap));
        }
        totals.work += length;
        Measured {
            size: length,
            damaged: false,
            data: Cow::Borrowed(&bytes[data..data + length]),
        }
    } else {
        let decoded = decode_chain(&bytes[data..], &plan.filters, cap)?;
        totals.work += decoded.work;
        Measured {
            size: decoded.data.len(),
            damaged: decoded.damaged,
            data: Cow::Owned(decoded.data),
        }
    };
    if totals.work > MAX_DECODE_WORK {
        return Err(Stop::unsupported(format!(
            "The PDF's streams expand to more than this reader's limit ({} MiB).",
            MAX_DECODE_WORK >> 20
        )));
    }
    Ok(measured)
}

/// What a stream's roles cost when hayro opens the file: as an object
/// stream (any stream with `/N` and `/First`), its offset table; as a
/// cross-reference stream (by `/Type` or at a section's offset), the entries
/// it can list, each of which may be a member read.
fn roles(
    candidate: &Candidate,
    data: &[u8],
    lengths: &mut Lengths,
    members: &mut Members,
) -> Result<(), Stop> {
    let dict = &candidate.dict;
    if dict.has_members() {
        object_stream(dict, data, lengths, members)?;
    }
    if dict.xref || candidate.section {
        let rows = match dict.width {
            Some(width) if width > 0 => data.len() / width,
            _ => data.len(),
        };
        members.reads = members.reads.saturating_add(rows);
    }
    Ok(())
}

fn stream_cap(dict: &DictInfo) -> usize {
    if dict.image {
        MAX_IMAGE_STREAM_BYTES
    } else {
        MAX_STREAM_BYTES
    }
}

fn data_too_large(cap: usize) -> Stop {
    Stop::unsupported(format!(
        "A stream's data is larger than this reader's limit of {} MiB.",
        cap >> 20
    ))
}

fn row_too_large(cap: usize) -> Stop {
    Stop::unsupported(format!(
        "A stream's decoding parameters ask for a row larger than this reader's limit of {} MiB.",
        cap >> 20
    ))
}

fn unknown_filter() -> Stop {
    Stop::unsupported("A stream uses a filter this reader does not know.")
}

fn parameters_unknown() -> Stop {
    Stop::unsupported("A stream's decoding parameters could not be checked before reading.")
}

fn length_unchecked() -> Stop {
    Stop::unsupported("A stream's length could not be checked before reading.")
}

/// The possible values of indirect `/Length` objects, by (number,
/// generation): each value once, in the order found, and at most one past
/// `MAX_LENGTH_VALUES` (`add_length`), so what a stream reads from it does
/// not grow with the file.
type Lengths = HashMap<(usize, usize), Vec<usize>>;

fn add_length(lengths: &mut Lengths, id: (usize, usize), value: usize) {
    let values = lengths.entry(id).or_default();
    if values.len() <= MAX_LENGTH_VALUES && !values.contains(&value) {
        values.push(value);
    }
}

/// The possible values of the length object `id`; an error when it has
/// more than this reader checks.
fn length_values(lengths: &Lengths, id: (usize, usize)) -> Result<&[usize], Stop> {
    let values = lengths.get(&id).map_or(&[][..], Vec::as_slice);
    if values.len() > MAX_LENGTH_VALUES {
        return Err(length_unchecked());
    }
    Ok(values)
}

/// The data lengths hayro 0.8.0 may take for a stream whose dictionary ends
/// at `dict_end` and whose `stream` keyword is at `keyword`
/// (hayro-syntax src/object/stream.rs). `parse_proper`: `/Length` bytes
/// after the keyword and exactly one end of line, if only white space and
/// then `endstream` follow. Otherwise `parse_fallback`: from the first
/// `stream` after the dictionary (here, only when that is this keyword; an
/// early stream's other range is measured apart) up to the first
/// `endstream`, trailing white space removed. A direct `/Length` that fits
/// is always used. An indirect one may resolve to any value found for that
/// object, or not at all (hayro reads cross-reference streams and rebuilt
/// cross-reference data without resolving it), so all of those count. Empty
/// when hayro would take no data here; an error when the length object has
/// more values than this reader checks. Sorted, each length once.
fn data_lengths(
    scan: &Scan,
    dict_end: usize,
    keyword: usize,
    length: Option<Length>,
    lengths: &Lengths,
) -> Result<Vec<usize>, Stop> {
    let bytes = scan.bytes;
    let Some(data) = eol_after(bytes, keyword + b"stream".len()) else {
        return Ok(Vec::new());
    };
    let proper = |value: usize| {
        let end = data.checked_add(value).filter(|&end| end <= bytes.len())?;
        bytes[scan.next_non_white(end)..]
            .starts_with(b"endstream")
            .then_some(value)
    };
    let mut found = Vec::new();
    match length {
        Some(Length::Direct(value)) => {
            if let Some(value) = proper(value) {
                return Ok(vec![value]);
            }
        }
        Some(Length::Indirect(id)) => {
            found.extend(
                length_values(lengths, id)?
                    .iter()
                    .filter_map(|&v| proper(v)),
            );
        }
        None => {}
    }
    if scan.next_word(Word::Stream, dict_end) == keyword {
        found.extend(fallback_length(scan, data));
    }
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

/// hayro's `parse_fallback` from the data at `data`: up to the first
/// `endstream`, trailing white space removed.
fn fallback_length(scan: &Scan, data: usize) -> Option<usize> {
    let end = scan.next_word(Word::Endstream, data);
    if end >= scan.bytes.len() {
        return None;
    }
    let last = scan
        .last_non_white_before(end)
        .filter(|&at| at >= data)
        .map_or(data, |at| at + 1);
    Some(last - data)
}

/// After `stream`, exactly one end of line (LF, CR LF or CR), as hayro
/// requires: where the data starts.
fn eol_after(bytes: &[u8], i: usize) -> Option<usize> {
    match (bytes.get(i), bytes.get(i + 1)) {
        (Some(b'\r'), Some(b'\n')) => Some(i + 2),
        (Some(b'\r' | b'\n'), _) => Some(i + 1),
        _ => None,
    }
}

/// What a predictor needs as its row buffer (hayro-syntax
/// src/filter/lzw_flate.rs, `PredictorParams` and `apply_predictor`).
enum Row {
    /// The largest row for any Flate or LZW filter of the stream (0 when
    /// none uses a predictor).
    Fits(usize),
    /// The parameters are stored elsewhere in the file, or cannot be read
    /// the way hayro reads them.
    Unknown,
}

/// hayro allocates a predictor's row from `/Columns`, `/Colors` and
/// `/BitsPerComponent` before any data is decoded, however short the data.
/// Its parameters for a single filter are `/DP` or else `/DecodeParms`, a
/// dictionary; for a filter array, the member at the same place in the
/// `/DP` or else `/DecodeParms` array.
fn predictor_row(scan: &mut Scan, dict: &DictInfo) -> Row {
    let Filters::Names(names) = &dict.filters else {
        return Row::Fits(0);
    };
    let predicted = |name: &Vec<u8>| {
        matches!(
            name.as_slice(),
            b"FlateDecode" | b"Fl" | b"LZWDecode" | b"LZW"
        )
    };
    if !names.iter().any(predicted) {
        return Row::Fits(0);
    }
    let bytes = scan.bytes;
    // The dictionary or array that holds the parameters.
    let mut holder = None;
    for (at, _) in dict.parms.into_iter().flatten() {
        let at = at as usize;
        let found = if dict.filter_array {
            bytes[at] == b'['
        } else {
            bytes[at..].starts_with(b"<<")
        };
        if found {
            holder = Some(at);
            break;
        }
        if scan.hayro_reference(at).is_some() {
            return Row::Unknown;
        }
    }
    let Some(holder) = holder else {
        return Row::Fits(0);
    };
    let mut largest = 0usize;
    for (index, name) in names.iter().enumerate() {
        if !predicted(name) {
            continue;
        }
        let at = if dict.filter_array {
            match array_member(scan, holder, index) {
                Member::Dict(at) => at,
                Member::Default => continue,
                Member::Unknown => return Row::Unknown,
            }
        } else {
            holder
        };
        let Some((keys, _)) = scan.read_dict(at) else {
            return Row::Unknown;
        };
        // A value as hayro reads it with `get`: out of range or not a
        // number gives the default; a reference cannot be checked.
        let mut read = |span: Option<Span>, max: i64, default: u128| -> Option<u128> {
            let Some((at, _)) = span else {
                return Some(default);
            };
            if scan.hayro_reference(at as usize).is_some() {
                return None;
            }
            Some(match scan.hayro_integer(at as usize) {
                Some((n, _)) if (0..=max).contains(&n) => n as u128,
                _ => default,
            })
        };
        let (Some(predictor), Some(colors), Some(bits), Some(columns)) = (
            read(keys.predictor, 255, 1),
            read(keys.colors, 255, 1),
            read(keys.bits_per_component, 255, 8),
            read(keys.columns, i64::MAX, 1),
        ) else {
            return Row::Unknown;
        };
        if predictor == 1 {
            continue;
        }
        let row = (columns * colors * bits).div_ceil(8);
        largest = largest.max(usize::try_from(row).unwrap_or(usize::MAX));
    }
    Row::Fits(largest)
}

/// A member of a `/DP` or `/DecodeParms` array, as hayro pairs it with a
/// filter.
enum Member {
    Dict(usize),
    /// `null`, or the array is shorter: the default parameters.
    Default,
    /// A reference, or a member this check does not read as hayro does.
    Unknown,
}

/// The `index`th member of the array at `at`. Only dictionaries, `null` and
/// references are read the same way here and by hayro, so any other member
/// before or at `index` makes it unknown.
fn array_member(scan: &mut Scan, at: usize, index: usize) -> Member {
    let bytes = scan.bytes;
    let mut j = scan.skip_white(at + 1);
    for k in 0..=index {
        if bytes.get(j).is_none_or(|&b| b == b']') {
            return Member::Default;
        }
        let (member, end) = if bytes[j..].starts_with(b"<<") {
            match scan.skip_object(j, 1) {
                Some(end) => (Member::Dict(j), end),
                None => return Member::Unknown,
            }
        } else if bytes[j..].starts_with(b"null")
            && bytes
                .get(j + 4)
                .is_none_or(|&b| is_white(b) || is_delimiter(b))
        {
            (Member::Default, j + 4)
        } else if let Some((_, _, end)) = scan.hayro_reference(j) {
            (Member::Unknown, end)
        } else {
            return Member::Unknown;
        };
        if k == index {
            return member;
        }
        j = scan.skip_white(end);
    }
    Member::Default
}

/// A stream hayro may read members from (decoded as `data`): any stream
/// with `/N` and `/First`, whatever its `/Type`. Each time hayro reads one
/// member it reads `/N` pairs of numbers from the start of the data again
/// (hayro-syntax src/xref.rs, `ObjectStream::new`), so the bytes of that
/// offset table must stay within `MAX_OBJSTM_TABLE_BYTES`, and a pass over
/// all objects reads it at most `members.reads` times. The integers it holds
/// can be other streams' `/Length`.
fn object_stream(
    dict: &DictInfo,
    data: &[u8],
    lengths: &mut Lengths,
    members: &mut Members,
) -> Result<(), Stop> {
    let (table, pairs, list) = match (dict.count, dict.first) {
        (Count::Missing, _) | (_, Count::Missing) => (0, 0, None),
        (Count::Unknown, _) => (data.len(), data.len() / 2 + 1, None),
        (Count::Value(n), _) => offset_table(data, n),
    };
    if table > MAX_OBJSTM_TABLE_BYTES {
        return Err(Stop::unsupported(format!(
            "An object stream's offset table is larger than this reader's limit ({} KiB).",
            MAX_OBJSTM_TABLE_BYTES >> 10
        )));
    }
    members.table = members.table.max(table);
    members.reads = members.reads.saturating_add(pairs);
    let (Some(pairs), Count::Value(first)) = (list, dict.first) else {
        return Ok(());
    };
    for (number, offset) in pairs {
        // A member that is an integer, with at most a little white space
        // before it (more is left unresolved: the stream then reads as
        // unchecked if hayro uses it as a /Length).
        let offset = first.saturating_add(offset);
        let Some(start) = data
            .get(offset..data.len().min(offset.saturating_add(64)))
            .and_then(|near| near.iter().position(|&b| !is_white(b)))
        else {
            continue;
        };
        if let Some((value, _)) = hayro_number(data, offset + start)
            && let Ok(value) = u32::try_from(value)
        {
            add_length(lengths, (number, 0), value as usize);
        }
    }
    Ok(())
}

/// The offset table hayro reads: `n` pairs of numbers, white space and
/// comments allowed before each. The bytes it reads (up to where it stops),
/// the pairs it reads, and the pairs if it reads them all. Stops once past
/// the table limit.
fn offset_table(data: &[u8], n: usize) -> (usize, usize, Option<Vec<(usize, usize)>>) {
    let mut pairs = Vec::new();
    let mut i = 0;
    for _ in 0..n {
        if i > MAX_OBJSTM_TABLE_BYTES {
            return (i, pairs.len(), None);
        }
        let k = skip_white_plain(data, i);
        let Some((number, after)) = hayro_number(data, k) else {
            return (k, pairs.len(), None);
        };
        let k = skip_white_plain(data, after);
        let Some((offset, after)) = hayro_number(data, k) else {
            return (k, pairs.len(), None);
        };
        let (Ok(number), Ok(offset)) = (u32::try_from(number), usize::try_from(offset)) else {
            return (after, pairs.len(), None);
        };
        pairs.push((number as usize, offset));
        i = after;
    }
    (i, pairs.len(), Some(pairs))
}

/// Where a dictionary starts after each `obj` keyword in the file (white
/// space and comments may come between), wherever the keyword is: a
/// superset of every stream the PDF library can read, also when it
/// rebuilds a broken cross-reference table. In file order.
fn candidate_dicts(scan: &Scan) -> Vec<usize> {
    let bytes = scan.bytes;
    let mut found: Vec<usize> = memmem::find_iter(bytes, b"obj")
        .map(|at| scan.skip_white(at + b"obj".len()))
        .filter(|&start| bytes[start..].starts_with(b"<<"))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// Where the dictionaries of the file's own objects start: the file read
/// from its header as PDF syntax, one object after another. A stream's data
/// is skipped by its `/Length` (direct, or an integer object found by
/// `length_objects`) when `endstream` follows there, else up to the first
/// `endstream`. Where something is not a whole object (damage, or bytes
/// between objects), reading goes on token by token, and an object header
/// met that way counts as the file's own: in damaged parts this check
/// errs towards checking more. A stream of the file's own whose length
/// object has more values than this reader checks stops the reading.
fn object_dicts(scan: &mut Scan, header: usize, lengths: &Lengths) -> Result<HashSet<usize>, Stop> {
    let bytes = scan.bytes;
    let mut found = HashSet::new();
    let mut i = header;
    loop {
        i = scan.skip_white(i);
        if i >= bytes.len() {
            return Ok(found);
        }
        if let Some((_, after)) = object_header(scan, i) {
            let start = scan.skip_white(after);
            if bytes[start..].starts_with(b"<<") {
                found.insert(start);
            }
            i = object_end(scan, start, lengths)?.unwrap_or(after);
            continue;
        }
        i = skip_token(bytes, i);
    }
}

/// Just after the object whose value starts at `start` (after its `obj`):
/// after `endobj`, or before the next object or cross-reference section
/// when `endobj` is missing. `None` when it is not a whole object.
fn object_end(scan: &mut Scan, start: usize, lengths: &Lengths) -> Result<Option<usize>, Stop> {
    let bytes = scan.bytes;
    let Some(end) = scan.skip_object(start, 0) else {
        return Ok(None);
    };
    let mut k = scan.skip_white(end);
    if bytes[start..].starts_with(b"<<") && is_stream_keyword(bytes, k) {
        let length = read_info(scan, start).and_then(|(dict, _)| dict.length);
        let Some(endstream) = payload_end(scan, data_start(bytes, k), length, lengths)? else {
            return Ok(None);
        };
        k = scan.skip_white(endstream + b"endstream".len());
    }
    if bytes[k..].starts_with(b"endobj") {
        return Ok(Some(k + b"endobj".len()));
    }
    let next = k >= bytes.len()
        || object_header(scan, k).is_some()
        || [&b"xref"[..], b"trailer", b"startxref"]
            .iter()
            .any(|word| bytes[k..].starts_with(word));
    Ok(next.then_some(k))
}

/// Where the `endstream` that ends the data starting at `data` is: after
/// `/Length` bytes (and white space) if one of its possible values ends
/// there, else the first `endstream`. The values are read where they are
/// kept, not copied; an error when the length object has more than this
/// reader checks.
fn payload_end(
    scan: &Scan,
    data: usize,
    length: Option<Length>,
    lengths: &Lengths,
) -> Result<Option<usize>, Stop> {
    let bytes = scan.bytes;
    let (direct, indirect) = match length {
        Some(Length::Direct(value)) => (Some(value), &[][..]),
        Some(Length::Indirect(id)) => (None, length_values(lengths, id)?),
        None => (None, &[][..]),
    };
    Ok(direct
        .iter()
        .chain(indirect)
        .find_map(|&value| {
            let end = data.checked_add(value).filter(|&end| end <= bytes.len())?;
            let end = scan.next_non_white(end);
            bytes[end..].starts_with(b"endstream").then_some(end)
        })
        .or_else(|| {
            let end = scan.next_word(Word::Endstream, data);
            (end < bytes.len()).then_some(end)
        }))
}

/// Every `N G obj <integer> endobj` in the file, by (number, generation):
/// the possible values of an indirect `/Length`. A look-alike can add a
/// wrong value; `payload_end` only takes a value that ends at `endstream`.
fn length_objects(scan: &Scan) -> Lengths {
    let bytes = scan.bytes;
    let mut found: Lengths = HashMap::new();
    for at in memmem::find_iter(bytes, b"obj") {
        let Some((generation, start)) = number_before(bytes, at) else {
            continue;
        };
        let Some((number, _)) = number_before(bytes, start) else {
            continue;
        };
        let value_at = scan.skip_white(at + b"obj".len());
        let Some((value, after)) = read_number(bytes, value_at) else {
            continue;
        };
        if bytes[scan.skip_white(after)..].starts_with(b"endobj") {
            add_length(&mut found, (number, generation), value);
        }
    }
    found
}

/// One token of bytes that are not a whole object: a delimiter, or a run of
/// other bytes up to the next white space or delimiter.
fn skip_token(bytes: &[u8], i: usize) -> usize {
    if is_delimiter(bytes[i]) {
        return i + 1;
    }
    let mut j = i + 1;
    while j < bytes.len() && !is_white(bytes[j]) && !is_delimiter(bytes[j]) {
        j += 1;
    }
    j
}

/// An unreadable dictionary that may be an object or cross-reference
/// stream: before its next `stream`, the bytes name one (`ObjStm`, `XRef`,
/// `First`), or a name has an escape (hayro decodes `#xx`, so a name this
/// check cannot match by its bytes).
fn names_decoded_early(scan: &Scan, start: usize) -> bool {
    let end = scan.next_word(Word::Stream, start);
    [Word::ObjStm, Word::XRef, Word::First, Word::Hash]
        .into_iter()
        .any(|word| scan.next_word(word, start) < end)
}

fn encrypted() -> Stop {
    Stop::unsupported(crate::pdf::ENCRYPTED_NOTE)
}

/// `12 0 obj` at `i`: its (number, generation) and the index after `obj`.
/// White space or comments separate the numbers, as hayro reads them.
fn object_header(scan: &Scan, i: usize) -> Option<((usize, usize), usize)> {
    let bytes = scan.bytes;
    let (number, j) = read_number(bytes, i)?;
    let k = scan.skip_white(j);
    if k == j {
        return None;
    }
    let (generation, l) = read_number(bytes, k)?;
    let m = scan.skip_white(l);
    if !bytes[m..].starts_with(b"obj") {
        return None;
    }
    let after = m + b"obj".len();
    if bytes
        .get(after)
        .is_some_and(|&b| !is_white(b) && !is_delimiter(b))
    {
        return None;
    }
    Some(((number, generation), after))
}

/// The number that ends just before the white space before `i`, and where
/// it starts.
fn number_before(bytes: &[u8], mut i: usize) -> Option<(usize, usize)> {
    let mut spaces = 0;
    while i > 0 && is_white(bytes[i - 1]) {
        i -= 1;
        spaces += 1;
    }
    if spaces == 0 {
        return None;
    }
    let end = i;
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == end {
        return None;
    }
    Some((read_number(bytes, i)?.0, i))
}

/// `stream` at `i`, followed by white space (the end of line before the data).
fn is_stream_keyword(bytes: &[u8], i: usize) -> bool {
    bytes[i..].starts_with(b"stream") && bytes.get(i + 6).is_some_and(|&b| is_white(b))
}

/// The data starts after the end-of-line that follows the keyword.
fn data_start(bytes: &[u8], keyword: usize) -> usize {
    let mut data = keyword + b"stream".len();
    while bytes.get(data) == Some(&b' ') {
        data += 1;
    }
    match bytes.get(data) {
        Some(b'\r') if bytes.get(data + 1) == Some(&b'\n') => data += 2,
        Some(b'\r' | b'\n') => data += 1,
        _ => {}
    }
    data.min(bytes.len())
}

/// An object whose dictionary cannot be read, and that has `stream` before
/// its `endobj`: a stream this check cannot measure.
fn stream_follows(scan: &Scan, start: usize) -> bool {
    scan.next_word(Word::Stream, start) < scan.next_word(Word::Endobj, start)
}

/// A count such as `/Size`: a number that is too long to read counts as
/// larger than any limit.
fn read_count(value: &[u8]) -> Option<usize> {
    let digits = value
        .iter()
        .take(11)
        .take_while(|b| b.is_ascii_digit())
        .count();
    match digits {
        0 => None,
        1..=10 => read_number(value, 0).map(|(n, _)| n),
        _ => Some(usize::MAX),
    }
}

#[derive(Debug, Default)]
struct DictInfo {
    filters: Filters,
    /// `/Filter` is an array (hayro then reads its parameters as an array).
    filter_array: bool,
    image: bool,
    external: bool,
    /// `/Type /XRef`: a cross-reference stream.
    xref: bool,
    /// `/Type /ObjStm`: an object stream.
    object_stream: bool,
    length: Option<Length>,
    encrypt: bool,
    size: Option<usize>,
    prev: Option<usize>,
    xref_stm: Option<usize>,
    /// The values of `/DP` and `/DecodeParms`, in the order hayro tries
    /// them.
    parms: [Option<Span>; 2],
    /// `/N` and `/First` (an object stream's).
    count: Count,
    first: Count,
    /// The sum of a cross-reference stream's `/W`, if it can be read here.
    width: Option<usize>,
}

impl DictInfo {
    /// hayro can read members from it: it has `/N` and `/First` (or says
    /// it is an object stream).
    fn has_members(&self) -> bool {
        self.object_stream
            || !(matches!(self.count, Count::Missing) || matches!(self.first, Count::Missing))
    }
}

/// A count hayro reads with `get::<usize>`.
#[derive(Debug, Default, Clone, Copy)]
enum Count {
    /// Absent or not a count: hayro reads no member.
    #[default]
    Missing,
    Value(usize),
    /// Stored elsewhere in the file.
    Unknown,
}

#[derive(Debug, Clone, Copy)]
enum Length {
    Direct(usize),
    /// `/Length 12 0 R`: (number, generation).
    Indirect((usize, usize)),
}

#[derive(Debug, Default)]
enum Filters {
    #[default]
    None,
    /// Shared with every dictionary that reads the same `/Filter` value
    /// (`Scan::filter_list`): a candidate holds a reference, not a copy.
    Names(Rc<[Vec<u8>]>),
    /// `/Filter 5 0 R`: the list lives elsewhere.
    Indirect,
}

/// The `<< ... >>` at `start`: the keys this check uses, and the index
/// just after its `>>`. `None` when it cannot be read, also when a
/// `/Filter` is neither names nor a reference.
fn read_info(scan: &mut Scan, start: usize) -> Option<(DictInfo, usize)> {
    let (keys, end) = scan.read_dict(start)?;
    if keys.bad_filter.is_some() {
        return None;
    }
    Some((dict_info(scan, &keys), end))
}

/// The keys' values, each read with work that does not grow with the value
/// (numbers and filter lists are read once per position in `Scan`), since
/// many dictionaries can share the same values.
fn dict_info(scan: &mut Scan, keys: &Keys) -> DictInfo {
    let bytes = scan.bytes;
    let slice = |span: Option<Span>| span.map(|(at, end)| &bytes[at as usize..end as usize]);
    let filters = match keys.filter {
        None => Filters::None,
        Some((at, end)) => match scan.filter_list(at as usize, end as usize) {
            Some(FilterList::Names(names)) => Filters::Names(names),
            Some(FilterList::Elsewhere) => Filters::Indirect,
            None => Filters::None,
        },
    };
    let mut count = |span: Option<Span>| match span {
        None => Count::Missing,
        Some((at, _)) if scan.hayro_reference(at as usize).is_some() => Count::Unknown,
        Some((at, _)) => scan
            .hayro_integer(at as usize)
            .and_then(|(n, _)| usize::try_from(n).ok())
            .map_or(Count::Missing, Count::Value),
    };
    let (n, first) = (count(keys.n), count(keys.first));
    DictInfo {
        filters,
        filter_array: keys
            .filter
            .is_some_and(|(at, _)| bytes[at as usize] == b'['),
        image: keys
            .subtype
            .is_some_and(|span| name_is(bytes, span, b"Image")),
        external: keys.external.is_some(),
        xref: keys.type_.is_some_and(|span| name_is(bytes, span, b"XRef")),
        object_stream: keys
            .type_
            .is_some_and(|span| name_is(bytes, span, b"ObjStm")),
        length: keys.length.and_then(|span| length_value(scan, span)),
        encrypt: keys.encrypt.is_some(),
        size: slice(keys.size).and_then(read_count),
        prev: slice(keys.prev).and_then(|v| read_number(v, 0).map(|(n, _)| n)),
        xref_stm: slice(keys.xref_stm).and_then(|v| read_number(v, 0).map(|(n, _)| n)),
        parms: [keys.dp, keys.decode_parms],
        count: n,
        first,
        width: keys.w.and_then(|span| width(scan, span)),
    }
}

/// A `/Length` as hayro reads it: a reference (`12 0 R`, numbers as hayro
/// reads them), or a number that fits a u32.
fn length_value(scan: &mut Scan, (at, _): Span) -> Option<Length> {
    let at = at as usize;
    if let Some((number, generation, _)) = scan.hayro_reference(at) {
        let id = |n: i64| usize::try_from(n).unwrap_or(usize::MAX);
        return Some(Length::Indirect((id(number), id(generation))));
    }
    let (n, _) = scan.hayro_integer(at)?;
    u32::try_from(n).ok().map(|n| Length::Direct(n as usize))
}

/// The sum of the first three numbers of a `/W` array, as hayro reads them.
fn width(scan: &mut Scan, (at, end): Span) -> Option<usize> {
    let bytes = scan.bytes;
    if bytes[at as usize] != b'[' {
        return None;
    }
    let mut j = scan.skip_white(at as usize + 1);
    let mut sum = 0usize;
    for _ in 0..3 {
        if j >= end as usize || bytes[j] == b']' {
            break;
        }
        let (n, after) = scan.hayro_integer(j)?;
        sum = sum.saturating_add(usize::try_from(n).ok()?);
        j = scan.skip_white(after);
    }
    Some(sum)
}

/// The name at `span` is `wanted` (with `#xx` escapes decoded), reading no
/// more of it than `wanted` needs.
fn name_is(bytes: &[u8], (at, end): Span, wanted: &[u8]) -> bool {
    let (mut j, end) = (at as usize, end as usize);
    if bytes.get(j) != Some(&b'/') {
        return false;
    }
    j += 1;
    let mut matched = 0;
    while j < end && !is_white(bytes[j]) && !is_delimiter(bytes[j]) {
        let byte = if bytes[j] == b'#' {
            let Some(byte) = bytes
                .get(j + 1..j + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            else {
                return false;
            };
            j += 3;
            byte
        } else {
            j += 1;
            bytes[j - 1]
        };
        if wanted.get(matched) != Some(&byte) {
            return false;
        }
        matched += 1;
    }
    matched == wanted.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Flate,
    AsciiHex,
    Ascii85,
    RunLength,
}

/// The filters hayro applies, as this check measures them. hayro skips a
/// filter name it does not know and applies the others (hayro-syntax
/// src/object/stream.rs, `filters_and_params`), so the plan does the same
/// and `unknown` says that a name was skipped.
struct Plan {
    filters: Vec<Filter>,
    /// The last filter is an image codec (not measured).
    image_codec: bool,
    unknown: bool,
}

/// The filters to measure. An error when hayro would decode something this
/// check cannot measure.
fn plan_filters(dict: &DictInfo) -> Result<Plan, Stop> {
    let mut plan = Plan {
        filters: Vec::new(),
        image_codec: false,
        unknown: false,
    };
    let names = match &dict.filters {
        Filters::None => return Ok(plan),
        Filters::Indirect => {
            return Err(Stop::unsupported(
                "A stream's filter list is stored elsewhere in the file; this reader does not guess its size.",
            ));
        }
        Filters::Names(names) => names,
    };
    if names.len() > MAX_FILTERS {
        return Err(Stop::unsupported(format!(
            "A stream has more than {MAX_FILTERS} filters."
        )));
    }
    for (index, name) in names.iter().enumerate() {
        let filter = match name.as_slice() {
            b"FlateDecode" | b"Fl" => Filter::Flate,
            b"ASCIIHexDecode" | b"AHx" => Filter::AsciiHex,
            b"ASCII85Decode" | b"A85" => Filter::Ascii85,
            b"RunLengthDecode" | b"RL" => Filter::RunLength,
            b"DCTDecode" | b"DCT" | b"JPXDecode" | b"JBIG2Decode" | b"CCITTFaxDecode" | b"CCF" => {
                if !dict.image || index + 1 != names.len() {
                    return Err(Stop::unsupported(
                        "An image codec is used on a stream that is not an image.",
                    ));
                }
                plan.image_codec = true;
                return Ok(plan);
            }
            b"LZWDecode" | b"LZW" => {
                return Err(Stop::unsupported(
                    "The PDF uses legacy LZW compression, which this experiment does not read.",
                ));
            }
            _ => {
                plan.unknown = true;
                continue;
            }
        };
        plan.filters.push(filter);
    }
    Ok(plan)
}

#[derive(Debug, PartialEq, Eq)]
struct Decoded {
    data: Vec<u8>,
    damaged: bool,
    /// Bytes every filter read and wrote.
    work: usize,
}

/// Applies the filters in order, each output capped at `cap` bytes. The
/// first filter reads from the stream's start to the end of the file; each
/// decoder stops at its own end marker, so a wrong `/Length` cannot hide
/// data from the check.
fn decode_chain(data: &[u8], filters: &[Filter], cap: usize) -> Result<Decoded, Stop> {
    let mut current: Option<Vec<u8>> = None;
    let mut damaged = false;
    let mut work = 0usize;
    for filter in filters {
        let input = current.as_deref().unwrap_or(data);
        let (output, broken, read) = match filter {
            Filter::Flate => inflate(input, cap),
            Filter::AsciiHex => ascii_hex(input, cap),
            Filter::Ascii85 => ascii85(input, cap),
            Filter::RunLength => run_length(input, cap),
        }
        .ok_or_else(|| {
            Stop::unsupported(format!(
                "A stream expands beyond this reader's limit of {} MiB.",
                cap >> 20
            ))
        })?;
        damaged |= broken;
        work += read + output.len();
        current = Some(output);
    }
    Ok(Decoded {
        data: current.unwrap_or_default(),
        damaged,
        work,
    })
}

/// Flate data: a two-byte zlib header (deflate, no preset dictionary), the
/// deflate data, and the Adler-32 of the output (RFC 1950). Anything else
/// is damaged. hayro is more lenient (it also reads data without the
/// header and ignores the checksum), so for damaged data both readings are
/// measured and the larger counts. `None` when an output would pass `cap`.
/// Also returns how many input bytes the readings used.
fn inflate(input: &[u8], cap: usize) -> Option<(Vec<u8>, bool, usize)> {
    if zlib_header(input) {
        let (output, used, broken) = inflate_raw(&input[2..], cap)?;
        let checksum = input.get(2 + used..).and_then(|rest| rest.get(..4));
        if !broken && checksum == Some(&adler32(&output).to_be_bytes()[..]) {
            return Some((output, false, used + 6));
        }
        let (raw, raw_used, _) = inflate_raw(input, cap)?;
        let read = used + 2 + raw_used;
        return Some((
            if raw.len() > output.len() {
                raw
            } else {
                output
            },
            true,
            read,
        ));
    }
    let (raw, used, _) = inflate_raw(input, cap)?;
    Some((raw, true, used))
}

fn zlib_header(input: &[u8]) -> bool {
    input.len() >= 2
        && input[0] & 0x0f == 8
        && input[0] >> 4 <= 7
        && input[1] & 0x20 == 0
        && (u16::from(input[0]) << 8 | u16::from(input[1])) % 31 == 0
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 is the most bytes before `b` can overflow 32 bits.
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// Raw deflate: the output, how many input bytes the deflate data used,
/// and whether it breaks or ends before its final block.
fn inflate_raw(input: &[u8], cap: usize) -> Option<(Vec<u8>, usize, bool)> {
    let mut decoder = Decompress::new(false);
    let mut output = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let (read, written) = (decoder.total_in(), decoder.total_out());
        let consumed = usize::try_from(read).ok()?;
        let status = decoder.decompress(&input[consumed..], &mut buffer, FlushDecompress::None);
        let produced = usize::try_from(decoder.total_out() - written).ok()?;
        if output.len() + produced > cap {
            return None;
        }
        output.extend_from_slice(&buffer[..produced]);
        let used = usize::try_from(decoder.total_in()).ok()?;
        match status {
            Ok(Status::StreamEnd) => return Some((output, used, false)),
            Ok(_) if decoder.total_in() != read || decoder.total_out() != written => {}
            _ => return Some((output, used, true)),
        }
    }
}

fn ascii_hex(input: &[u8], cap: usize) -> Option<(Vec<u8>, bool, usize)> {
    let mut output = Vec::new();
    let mut high: Option<u8> = None;
    let mut read = 0;
    for &byte in input {
        read += 1;
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            b'>' => break,
            _ if is_white(byte) => continue,
            _ => return Some((output, true, read)),
        };
        match high.take() {
            Some(h) => output.push(h << 4 | digit),
            None => high = Some(digit),
        }
        if output.len() > cap {
            return None;
        }
    }
    if let Some(h) = high {
        output.push(h << 4);
    }
    Some((output, false, read))
}

fn ascii85(input: &[u8], cap: usize) -> Option<(Vec<u8>, bool, usize)> {
    let mut output = Vec::new();
    let mut group = [0u8; 5];
    let mut n = 0;
    let mut read = 0;
    for &byte in input {
        read += 1;
        match byte {
            b'~' => break,
            b'z' if n == 0 => output.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = byte - b'!';
                n += 1;
                if n == 5 {
                    let value = group.iter().fold(0u64, |v, &d| v * 85 + u64::from(d));
                    let Ok(value) = u32::try_from(value) else {
                        return Some((output, true, read));
                    };
                    output.extend_from_slice(&value.to_be_bytes());
                    n = 0;
                }
            }
            _ if is_white(byte) => continue,
            _ => return Some((output, true, read)),
        }
        if output.len() > cap {
            return None;
        }
    }
    if n > 1 {
        group[n..].fill(84);
        let value = group.iter().fold(0u64, |v, &d| v * 85 + u64::from(d));
        let bytes = u32::try_from(value).unwrap_or(u32::MAX).to_be_bytes();
        output.extend_from_slice(&bytes[..n - 1]);
    }
    Some((output, false, read))
}

fn run_length(input: &[u8], cap: usize) -> Option<(Vec<u8>, bool, usize)> {
    let mut output = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let length = input[i];
        i += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = usize::from(length) + 1;
                let Some(run) = input.get(i..i + count) else {
                    return Some((output, true, input.len()));
                };
                output.extend_from_slice(run);
                i += count;
            }
            _ => {
                let Some(&byte) = input.get(i) else {
                    return Some((output, true, i));
                };
                output.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
                i += 1;
            }
        }
        if output.len() > cap {
            return None;
        }
    }
    Some((output, false, i))
}
