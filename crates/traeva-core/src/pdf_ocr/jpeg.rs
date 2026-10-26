//! Pictures for the model, the parts both sides share (tier 2; the
//! student's decision of 2026-10-14): the limits, the JPEG without any
//! metadata, and standard base64 for the answer. Pure byte work, no
//! decoding: the child (`model_image`) writes these JPEGs, the parent
//! (`ocr::images_for_model`) checks them again before anything is sent.
//!
//! ImageIO adds its own metadata to every JPEG it writes, even when it is
//! given no properties: an Exif segment (colour space, pixel size) and a
//! Photoshop/IPTC segment. So the encoded bytes are rewritten without them
//! (`bare_jpeg`): what is left is what a decoder needs to draw the picture.

/// The longest side of a picture sent to the model, in pixels.
pub const MODEL_IMAGE_MAX_SIDE: usize = 1568;
/// The most pictures one file gives: an image gives one, a scanned PDF its
/// first scanned pages (the same choice as text recognition).
pub const MODEL_IMAGE_MAX_COUNT: usize = 3;
/// The largest JPEG sent, in bytes.
pub const MODEL_IMAGE_MAX_BYTES: usize = 1 << 20;

/// A JPEG's start-of-image and end-of-image markers.
const START: [u8; 2] = [0xFF, 0xD8];
const END: [u8; 2] = [0xFF, 0xD9];

/// The JPEG without its metadata: every APP1 to APP15 segment (Exif, XMP,
/// ICC profiles, Photoshop/IPTC, ...) and every comment, and an APP0 that is
/// not a plain JFIF header (which could hold a thumbnail). The tables, the
/// frame header and the one scan are kept as they are.
///
/// Only the shape ImageIO writes is accepted: the start marker, segments,
/// one scan (baseline), and the end marker as the file's last two bytes.
/// Inside the scan's data 0xFF is only ever followed by a stuffed 0x00 or a
/// restart marker, so no segment can hide there. None for anything else.
pub(crate) fn bare_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&START) {
        return None;
    }
    let mut out = START.to_vec();
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let code = *bytes.get(at + 1)?;
        // Before the scan, every marker starts a segment with a length.
        if matches!(code, 0x00 | 0x01 | 0xD0..=0xD9 | 0xFF) {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(at + 2)?,
            *bytes.get(at + 3)?,
        ]));
        if length < 2 {
            return None;
        }
        let end = at + 2 + length;
        let segment = bytes.get(at..end)?;
        if code == 0xDA {
            // Start of scan: its header, then the image data to the end.
            if !scan_data_is_plain(&bytes[end..]) {
                return None;
            }
            out.extend_from_slice(&bytes[at..]);
            return Some(out);
        }
        // Only what a decoder needs stays: quantization and Huffman tables,
        // the baseline frame header and a restart interval (#37 review).
        // Application data and comments go; anything else is refused.
        let keep = match code {
            0xE0 => is_plain_jfif(segment),
            0xE1..=0xEF | 0xFE => false,
            0xDB | 0xC4 | 0xC0 | 0xDD => true,
            _ => return None,
        };
        if keep {
            out.extend_from_slice(segment);
        }
        at = end;
    }
}

/// The width and height in the one baseline frame header before the scan;
/// None without exactly one. Reads the segments only, never decodes.
pub(crate) fn frame_size(bytes: &[u8]) -> Option<(u16, u16)> {
    if !bytes.starts_with(&START) {
        return None;
    }
    let mut size = None;
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let code = *bytes.get(at + 1)?;
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(at + 2)?,
            *bytes.get(at + 3)?,
        ]));
        if length < 2 {
            return None;
        }
        match code {
            0xDA => return size,
            0xC0 => {
                if size.is_some() {
                    return None;
                }
                // Length (2), precision (1), height (2), width (2).
                let frame = bytes.get(at + 5..at + 9)?;
                let height = u16::from_be_bytes([frame[0], frame[1]]);
                let width = u16::from_be_bytes([frame[2], frame[3]]);
                size = Some((width, height));
            }
            _ => {}
        }
        at += 2 + length;
    }
}

/// Whether a JPEG has nothing `bare_jpeg` would take out (and so is one).
pub(crate) fn is_bare_jpeg(bytes: &[u8]) -> bool {
    bare_jpeg(bytes).is_some_and(|bare| bare == bytes)
}

/// An APP0 segment that is only JFIF's header: version, density units and
/// density, and no thumbnail (16 bytes with the marker's length field).
fn is_plain_jfif(segment: &[u8]) -> bool {
    // FF E0, length 16, "JFIF\0", version (2), units (1), density (4),
    // thumbnail width and height 0.
    segment.len() == 18 && &segment[4..9] == b"JFIF\0" && segment[16..18] == [0, 0]
}

/// The scan's image data: 0xFF only before a stuffed 0x00 or a restart
/// marker, and the end marker as the last two bytes.
fn scan_data_is_plain(data: &[u8]) -> bool {
    let Some(body) = data.strip_suffix(&END) else {
        return false;
    };
    let mut at = 0;
    while at < body.len() {
        if body[at] == 0xFF {
            match body.get(at + 1) {
                Some(0x00 | 0xD0..=0xD7) => at += 2,
                _ => return false,
            }
        } else {
            at += 1;
        }
    }
    true
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (RFC 4648, with `=` padding, no line breaks).
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(char::from(BASE64[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes of standard base64 text, only when the text is exactly what
/// `base64_encode` gives for them (padding, no line breaks, no other
/// characters, no stray bits).
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for group in text.as_bytes().chunks(4) {
        let mut n = 0u32;
        let mut padding = 0;
        for &c in group {
            let value = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => {
                    padding += 1;
                    0
                }
                _ => return None,
            };
            n = (n << 6) | u32::from(value);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3usize.checked_sub(padding)?]);
    }
    // Padding in the middle, `===` or stray bits all fail this.
    (base64_encode(&out) == text).then_some(out)
}

#[cfg(test)]
#[path = "jpeg_tests.rs"]
mod tests;
