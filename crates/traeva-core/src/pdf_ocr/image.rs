//! Text recognition from one PNG or JPEG image, the child's side (#37,
//! #186; the student's decision of 2026-10-14: only the recognized text is
//! used, the image never leaves this Mac).
//!
//! Before anything is decoded, the width and height are read from the
//! file's own header (PNG's IHDR, JPEG's frame header) and must keep to
//! `MAX_IMAGE_SIDE` and `MAX_IMAGE_PIXELS`, and the file must end the way a
//! whole PNG or JPEG ends (ImageIO decodes a cut-off file without saying
//! so, the missing part grey). Then ImageIO decodes the first
//! frame only, scaled down to the OCR pixel budget and turned upright by
//! its EXIF orientation. ImageIO applies the orientation itself; this
//! module never asks for the image's properties, so location and the other
//! metadata are never read, kept or returned. CoreGraphics draws the frame
//! onto white in 8-bit grey, so transparent, 16-bit and CMYK images come
//! out the same way, and Vision reads the grey pixels.
//!
//! The same checks and the same upright first frame (`upright_frame`) also
//! start a picture for the model (`model_image`, tier 2), which is drawn in
//! colour instead and sent only with the Settings opt-in.

use objc2::rc::autoreleasepool;
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    CGInterpolationQuality,
};
use objc2_image_io::{
    CGImageSource, CGImageSourceStatus, kCGImageSourceCreateThumbnailFromImageAlways,
    kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCache,
    kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize,
};

use crate::memory_contracts::TextStatus;
use crate::reader::MAX_MEMORY_TEXT_CHARS;

use super::render::Gray;
use super::vision::{self, Line};
use super::{ImageKind, Settings, image_kind};

/// The most pixels an image may have, from its header.
pub const MAX_IMAGE_PIXELS: u64 = 50_000_000;
/// The longest side an image may have, from its header.
pub const MAX_IMAGE_SIDE: u32 = 20_000;

/// Why no text came out: the answer's status and note.
pub(crate) type Refused = (TextStatus, String);

pub(crate) fn refused(status: TextStatus, note: &str) -> Refused {
    (status, note.to_string())
}

const NOT_AN_IMAGE: &str = "Only PNG and JPEG images are recognized.";
const NO_HEADER: &str = "The image's header could not be read.";
const TOO_LARGE: &str =
    "The image is larger than text recognition reads (50 million pixels, 20,000 pixels a side).";
const NOT_DECODED: &str = "The image is damaged or could not be decoded.";
const NOT_WHOLE: &str = "The image does not end where a whole PNG or JPEG ends: it may be cut off, \
                         or carry extra data after its end.";
const NO_TEXT: &str = "No usable OCR text was found in the image.";
const TOO_MUCH_TEXT: &str =
    "OCR text exceeds the one-million-character limit; all output was discarded.";

/// The recognized text of one PNG or JPEG image: its lines in Vision's
/// order, one per line, without page markers.
pub(crate) fn image_text(bytes: &[u8], settings: &Settings) -> Result<String, Refused> {
    text_from_image(bytes, settings, |gray| vision::recognize(gray, settings))
}

/// `image_text` with any recognizer (the tests use stand-ins).
fn text_from_image(
    bytes: &[u8],
    settings: &Settings,
    recognize: impl FnOnce(&Gray) -> Result<Vec<Line>, String>,
) -> Result<String, Refused> {
    let gray = decode(bytes, settings.max_pixels)?;
    let lines = recognize(&gray)
        .map_err(|_| refused(TextStatus::Failed, "Text recognition failed on this image."))?;
    drop(gray);
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let chars = text.chars().filter(|c| !c.is_whitespace()).count();
    if chars < settings.min_page_chars {
        return Err(refused(TextStatus::Unsupported, NO_TEXT));
    }
    if text.chars().count() > MAX_MEMORY_TEXT_CHARS {
        return Err(refused(TextStatus::Unsupported, TOO_MUCH_TEXT));
    }
    Ok(text)
}

/// The kind, width and height from the file's own header, checked against
/// the limits; nothing is decoded.
pub(crate) fn header(bytes: &[u8]) -> Result<(ImageKind, u32, u32), Refused> {
    let kind = image_kind(bytes).ok_or_else(|| refused(TextStatus::Unsupported, NOT_AN_IMAGE))?;
    let size = match kind {
        ImageKind::Png => png_size(bytes),
        ImageKind::Jpeg => jpeg_size(bytes),
    };
    let (width, height) = size.ok_or_else(|| refused(TextStatus::Failed, NO_HEADER))?;
    if width == 0 || height == 0 {
        return Err(refused(TextStatus::Failed, NO_HEADER));
    }
    if width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(refused(TextStatus::Unsupported, TOO_LARGE));
    }
    Ok((kind, width, height))
}

/// PNG: the first chunk must be IHDR (13 bytes): width, then height.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let ihdr = bytes.get(8..24)?;
    if ihdr[0..4] != [0, 0, 0, 13] || &ihdr[4..8] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(ihdr[8..12].try_into().ok()?);
    let height = u32::from_be_bytes(ihdr[12..16].try_into().ok()?);
    Some((width, height))
}

/// JPEG: walks the segments after the start marker to the first frame
/// header (SOF0 to SOF15, not DHT, JPG or DAC): its height, then width.
/// None at the start of the scan or the end before any frame header.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        // A marker may follow any number of 0xFF fill bytes.
        while bytes.get(at) == Some(&0xFF) && bytes.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let code = *bytes.get(at + 1)?;
        at += 2;
        match code {
            // A stuffed zero is no marker here (a decoder skips it and looks
            // on, so a size read after it may not be the one it decodes);
            // start of image again, end of image, start of scan.
            0x00 | 0xD8..=0xDA => return None,
            0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        if length < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&code) && !matches!(code, 0xC4 | 0xC8 | 0xCC) {
            // Length (2), precision (1), height (2), width (2).
            let frame = bytes.get(at + 3..at + 7)?;
            let height = u16::from_be_bytes([frame[0], frame[1]]);
            let width = u16::from_be_bytes([frame[2], frame[3]]);
            return Some((u32::from(width), u32::from(height)));
        }
        at += length;
    }
}

/// Whether the file ends as a whole PNG (its IEND chunk) or JPEG (its end
/// marker; zero bytes after it are allowed) does.
fn ends_whole(bytes: &[u8], kind: ImageKind) -> bool {
    const IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
    match kind {
        ImageKind::Png => bytes.ends_with(&IEND),
        ImageKind::Jpeg => {
            let end = bytes
                .iter()
                .rposition(|&b| b != 0)
                .map_or(0, |last| last + 1);
            bytes[..end].ends_with(&[0xFF, 0xD9])
        }
    }
}

/// The longest side to ask ImageIO for, so that the decoded frame keeps to
/// `max_pixels`; never larger than the image itself.
fn longest_side(width: u32, height: u32, max_pixels: usize) -> usize {
    let (long, short) = (width.max(height) as usize, width.min(height) as usize);
    if long.saturating_mul(short) <= max_pixels {
        return long;
    }
    let scale = (max_pixels as f64 / (long as f64 * short as f64)).sqrt();
    let mut side = (long as f64 * scale) as usize;
    // ImageIO rounds the short side; leave room for that rounding.
    while side > 1 && side.saturating_mul((short * side).div_ceil(long) + 1) > max_pixels {
        side -= 1;
    }
    side.max(1)
}

/// Decodes the first frame of a PNG or JPEG into 8-bit grey on white, upright
/// by its EXIF orientation, at most `max_pixels`.
pub(crate) fn decode(bytes: &[u8], max_pixels: usize) -> Result<Gray, Refused> {
    let frame = upright_frame(bytes, |width, height| {
        longest_side(width, height, max_pixels)
    })?;
    grey_on_white(&frame, max_pixels).ok_or_else(|| refused(TextStatus::Failed, NOT_DECODED))
}

/// The first frame of a PNG or JPEG that passed `header` and ends whole,
/// decoded by ImageIO, upright by its EXIF orientation, its longest side at
/// most `side(width, height)` pixels (from the header's size). The image's
/// properties are never asked for. Also the start of a picture for the
/// model (`model_image`).
pub(crate) fn upright_frame(
    bytes: &[u8],
    side: impl FnOnce(u32, u32) -> usize,
) -> Result<CFRetained<CGImage>, Refused> {
    let (kind, width, height) = header(bytes)?;
    let not_decoded = || refused(TextStatus::Failed, NOT_DECODED);
    if !ends_whole(bytes, kind) {
        return Err(refused(TextStatus::Failed, NOT_WHOLE));
    }
    let yes: &CFType = CFBoolean::new(true).as_ref();
    let no: &CFType = CFBoolean::new(false).as_ref();
    autoreleasepool(|_| {
        let data = CFData::from_bytes(bytes);
        let no_cache = options(&[(unsafe { kCGImageSourceShouldCache }, no)]);
        let source = unsafe { CGImageSource::with_data(&data, Some(no_cache.as_opaque())) }
            .ok_or_else(not_decoded)?;
        let expected = match kind {
            ImageKind::Png => "public.png",
            ImageKind::Jpeg => "public.jpeg",
        };
        let actual = unsafe { source.r#type() }.map(|t| t.to_string());
        if actual.as_deref() != Some(expected)
            || unsafe { source.count() } == 0
            || unsafe { source.status_at_index(0) } != CGImageSourceStatus::StatusComplete
        {
            return Err(not_decoded());
        }
        let side = CFNumber::new_i64(side(width, height) as i64);
        // The frame itself, never an embedded preview; turned upright by its
        // EXIF orientation; at most `side` pixels long.
        let thumbnail = options(&[
            (unsafe { kCGImageSourceCreateThumbnailFromImageAlways }, yes),
            (unsafe { kCGImageSourceCreateThumbnailWithTransform }, yes),
            (
                unsafe { kCGImageSourceThumbnailMaxPixelSize },
                side.as_ref(),
            ),
            (unsafe { kCGImageSourceShouldCacheImmediately }, yes),
            (unsafe { kCGImageSourceShouldCache }, no),
        ]);
        unsafe { source.thumbnail_at_index(0, Some(thumbnail.as_opaque())) }.ok_or_else(not_decoded)
    })
}

/// Draws `image` onto white in an 8-bit grey bitmap of the same size.
fn grey_on_white(image: &CGImage, max_pixels: usize) -> Option<Gray> {
    let (width, height) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    if width == 0 || height == 0 || width.checked_mul(height)? > max_pixels {
        return None;
    }
    let mut pixels = vec![255u8; width * height];
    let space = CGColorSpace::new_device_gray()?;
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            width,
            Some(&space),
            CGImageAlphaInfo::None.0,
        )
    }?;
    let all = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::set_interpolation_quality(Some(&context), CGInterpolationQuality::High);
    CGContext::set_gray_fill_color(Some(&context), 1.0, 1.0);
    CGContext::fill_rect(Some(&context), all);
    CGContext::draw_image(Some(&context), all, Some(image));
    // The context draws into `pixels`; it must be gone before they are used.
    drop(context);
    Some(Gray {
        width,
        height,
        pixels,
    })
}

/// ImageIO's options: keys and values.
pub(crate) fn options(
    pairs: &[(&CFString, &CFType)],
) -> CFRetained<CFDictionary<CFString, CFType>> {
    let (keys, values): (Vec<&CFString>, Vec<&CFType>) = pairs.iter().copied().unzip();
    CFDictionary::from_slices(&keys, &values)
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
