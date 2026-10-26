use super::{Error, Limits, Result};
use flate2::{Decompress, FlushDecompress, Status};
use std::collections::{BTreeMap, BTreeSet};

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    let b = bytes
        .get(at..at + 2)
        .ok_or_else(|| Error::failed("truncated_zip"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let b = bytes
        .get(at..at + 4)
        .ok_or_else(|| Error::failed("truncated_zip"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn valid_part(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name.chars().any(|c| c.is_control() || "\\:%?#".contains(c))
        && name
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

pub(crate) fn resolve(source: &str, target: &str) -> Result<String> {
    if target.is_empty()
        || target.len() > 256
        || target
            .chars()
            .any(|c| c.is_control() || "\\:%?#".contains(c))
        || target.starts_with("//")
    {
        return Err(Error::unsupported("unsafe_relationship_target"));
    }
    let mut segments: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        source
            .rsplit_once('/')
            .map_or(Vec::new(), |(dir, _)| dir.split('/').collect())
    };
    for p in target.trim_start_matches('/').split('/') {
        match p {
            "" => return Err(Error::unsupported("ambiguous_relationship_target")),
            "." => (),
            ".." => {
                if segments.pop().is_none() {
                    return Err(Error::unsupported("relationship_escapes_package"));
                }
            }
            _ => segments.push(p),
        }
    }
    let name = segments.join("/");
    if !valid_part(&name) {
        return Err(Error::unsupported("unsafe_relationship_target"));
    }
    Ok(name)
}

fn check_extra(extra: &[u8]) -> Result<()> {
    let mut pos = 0;
    while pos < extra.len() {
        let kind = u16_at(extra, pos)?;
        let len = usize::from(u16_at(extra, pos + 2)?);
        pos += 4;
        if pos + len > extra.len() {
            return Err(Error::failed("malformed_zip_extra"));
        }
        if matches!(kind, 0x0001 | 0x7075) {
            return Err(Error::unsupported("zip64_or_alternate_zip_name"));
        }
        if kind == 0x9901 {
            return Err(Error::unsupported("encrypted_zip_extra"));
        }
        pos += len;
    }
    Ok(())
}

struct Entry {
    name: String,
    method: u16,
    crc: u32,
    expanded: usize,
    payload: std::ops::Range<usize>,
}

fn descriptor_end(bytes: &[u8], at: usize, entry: &Entry, central_start: usize) -> Result<usize> {
    let compressed = entry.payload.len() as u32;
    let expanded = entry.expanded as u32;
    let matches = |pos| {
        u32_at(bytes, pos) == Ok(entry.crc)
            && u32_at(bytes, pos + 4) == Ok(compressed)
            && u32_at(bytes, pos + 8) == Ok(expanded)
    };
    let signed =
        bytes.get(at..at + 4) == Some(b"PK\x07\x08") && at + 16 <= central_start && matches(at + 4);
    let unsigned = at + 12 <= central_start && matches(at);
    match (signed, unsigned) {
        (true, false) => Ok(at + 16),
        (false, true) => Ok(at + 12),
        _ => Err(Error::failed("missing_or_invalid_zip_descriptor")),
    }
}

fn preflight(bytes: &[u8], limits: &Limits) -> Result<Vec<Entry>> {
    if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]) {
        return Err(Error::unsupported("ole_or_encrypted_office_container"));
    }
    if !bytes.starts_with(b"PK\x03\x04") {
        return Err(Error::unsupported("not_an_xlsx_zip"));
    }
    let eocd = (bytes.len().saturating_sub(65_557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&p| {
            bytes.get(p..p + 4) == Some(b"PK\x05\x06")
                && u16_at(bytes, p + 20).is_ok_and(|len| p + 22 + usize::from(len) == bytes.len())
        })
        .ok_or_else(|| Error::failed("missing_zip_end"))?;
    if bytes[eocd + 22..].windows(4).any(|b| b == b"PK\x05\x06") {
        return Err(Error::unsupported("ambiguous_zip_end_in_comment"));
    }
    let count = usize::from(u16_at(bytes, eocd + 10)?);
    let size = u32_at(bytes, eocd + 12)? as usize;
    let start = u32_at(bytes, eocd + 16)? as usize;
    if count == 0xffff || size == 0xffff_ffff || start == 0xffff_ffff {
        return Err(Error::unsupported("zip64"));
    }
    if u16_at(bytes, eocd + 4)? != 0
        || u16_at(bytes, eocd + 6)? != 0
        || usize::from(u16_at(bytes, eocd + 8)?) != count
    {
        return Err(Error::unsupported("multi_disk_zip"));
    }
    if count > limits.entries || size > limits.central_bytes {
        return Err(Error::limit("zip_directory_limit"));
    }
    if count == 0 || start.checked_add(size) != Some(eocd) {
        return Err(Error::failed("zip_directory_bounds"));
    }
    let mut pos = start;
    let mut names = BTreeSet::new();
    let mut ranges = Vec::new();
    let mut entries = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if bytes.get(pos..pos + 4) != Some(b"PK\x01\x02") || pos + 46 > eocd {
            return Err(Error::failed("malformed_zip_directory"));
        }
        let flags = u16_at(bytes, pos + 8)?;
        let method = u16_at(bytes, pos + 10)?;
        let crc = u32_at(bytes, pos + 16)?;
        let compressed = u32_at(bytes, pos + 20)? as usize;
        let expanded = u32_at(bytes, pos + 24)? as usize;
        let name_len = usize::from(u16_at(bytes, pos + 28)?);
        let extra_len = usize::from(u16_at(bytes, pos + 30)?);
        let comment_len = usize::from(u16_at(bytes, pos + 32)?);
        let local = u32_at(bytes, pos + 42)? as usize;
        if flags & !0x080e != 0 || !matches!(method, 0 | 8) || (method == 0 && flags & 6 != 0) {
            return Err(Error::unsupported("encrypted_or_unsupported_zip_method"));
        }
        let attributes = u32_at(bytes, pos + 38)?;
        if attributes & 0x10 != 0 || matches!((attributes >> 16) & 0o170000, 0o120000 | 0o040000) {
            return Err(Error::unsupported("directory_or_symlink_part"));
        }
        if expanded == 0xffff_ffff || compressed == 0xffff_ffff || local == 0xffff_ffff {
            return Err(Error::unsupported("zip64"));
        }
        total = total
            .checked_add(expanded)
            .ok_or_else(|| Error::limit("expanded_total"))?;
        if expanded > limits.entry_bytes
            || total > limits.expanded_bytes
            || expanded > compressed.max(1).saturating_mul(limits.compression_ratio)
        {
            return Err(Error::limit("zip_expansion_limit"));
        }
        let end = pos + 46 + name_len + extra_len + comment_len;
        if end > eocd || u16_at(bytes, pos + 34)? != 0 {
            return Err(Error::failed("zip_entry_bounds"));
        }
        let raw_name = &bytes[pos + 46..pos + 46 + name_len];
        let name =
            std::str::from_utf8(raw_name).map_err(|_| Error::unsupported("zip_name_encoding"))?;
        if !valid_part(name) || !names.insert(name.to_lowercase()) {
            return Err(Error::unsupported("unsafe_or_duplicate_part_name"));
        }
        check_extra(&bytes[pos + 46 + name_len..pos + 46 + name_len + extra_len])?;
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04")
            || u16_at(bytes, local + 6)? != flags
            || u16_at(bytes, local + 8)? != method
        {
            return Err(Error::failed("zip_local_header_mismatch"));
        }
        let local_name_len = usize::from(u16_at(bytes, local + 26)?);
        let local_extra_len = usize::from(u16_at(bytes, local + 28)?);
        let payload = local + 30 + local_name_len + local_extra_len;
        let payload_end = payload
            .checked_add(compressed)
            .ok_or_else(|| Error::failed("zip_bounds"))?;
        if payload_end > start
            || bytes.get(local + 30..local + 30 + local_name_len) != Some(raw_name)
        {
            return Err(Error::failed("zip_local_bounds_or_name"));
        }
        check_extra(&bytes[local + 30 + local_name_len..payload])?;
        for (offset, expected) in [(14, crc), (18, compressed as u32), (22, expanded as u32)] {
            let actual = u32_at(bytes, local + offset)?;
            if actual != expected && !(flags & 8 != 0 && actual == 0) {
                return Err(Error::failed("zip_local_sizes_or_crc"));
            }
        }
        let entry = Entry {
            name: name.to_string(),
            method,
            crc,
            expanded,
            payload: payload..payload_end,
        };
        let local_end = if flags & 8 == 0 {
            payload_end
        } else {
            descriptor_end(bytes, payload_end, &entry, start)?
        };
        ranges.push((local, local_end));
        entries.push(entry);
        pos = end;
    }
    if pos != eocd {
        return Err(Error::failed("zip_entry_count_mismatch"));
    }
    ranges.sort_unstable();
    if ranges.first().is_none_or(|r| r.0 != 0)
        || ranges.windows(2).any(|r| r[0].1 != r[1].0)
        || ranges.last().is_none_or(|r| r.1 != start)
    {
        return Err(Error::unsupported("zip_overlap_prefix_or_unclaimed_bytes"));
    }
    Ok(entries)
}

fn inflate(payload: &[u8], declared: usize, cap: usize) -> Result<Vec<u8>> {
    let mut decoder = Decompress::new(false);
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let consumed = decoder.total_in();
        let expanded = decoder.total_out();
        let status = decoder
            .decompress(
                &payload[consumed as usize..],
                &mut buffer,
                FlushDecompress::None,
            )
            .map_err(|_| Error::failed("zip_decompression"))?;
        let produced = (decoder.total_out() - expanded) as usize;
        if data.len() + produced > cap {
            return Err(Error::limit("actual_zip_expansion_limit"));
        }
        if data.len() + produced > declared {
            return Err(Error::failed("zip_declared_size_mismatch"));
        }
        data.extend_from_slice(&buffer[..produced]);
        if status == Status::StreamEnd {
            if decoder.total_in() as usize != payload.len() {
                return Err(Error::failed("zip_deflate_trailing_bytes"));
            }
            return Ok(data);
        }
        if decoder.total_in() == consumed && produced == 0 {
            return Err(Error::failed("zip_incomplete_deflate_stream"));
        }
    }
}

pub(crate) fn unpack(bytes: &[u8], limits: &Limits) -> Result<BTreeMap<String, Vec<u8>>> {
    let entries = preflight(bytes, limits)?;
    let mut parts = BTreeMap::new();
    let mut total = 0usize;
    for entry in entries {
        let payload = &bytes[entry.payload];
        let cap = limits.entry_bytes.min(limits.expanded_bytes - total);
        let data = if entry.method == 0 {
            if payload.len() != entry.expanded {
                return Err(Error::failed("zip_stored_size_mismatch"));
            }
            payload.to_vec()
        } else {
            inflate(payload, entry.expanded, cap)?
        };
        total += data.len();
        if data.len() > limits.entry_bytes || total > limits.expanded_bytes {
            return Err(Error::limit("actual_zip_expansion_limit"));
        }
        if data.len() != entry.expanded {
            return Err(Error::failed("zip_declared_size_mismatch"));
        }
        if crc32fast::hash(&data) != entry.crc {
            return Err(Error::failed("zip_crc_mismatch"));
        }
        if parts.insert(entry.name, data).is_some() {
            return Err(Error::failed("duplicate_zip_part"));
        }
    }
    Ok(parts)
}
