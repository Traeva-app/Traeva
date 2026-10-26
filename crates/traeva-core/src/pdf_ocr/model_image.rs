//! Pictures for the model, the child's side (tier 2; the student's decision
//! of 2026-10-14). With the Settings opt-in, Organize may send one PNG or
//! JPEG image, or a scanned PDF's first scanned pages, to the model so that
//! it can look at them. This module sends nothing: the child prints the
//! pictures for its parent, which checks them again (`ocr.rs`).
//!
//! Every picture is made afresh, so nothing of the file but its pixels goes
//! into it. An image passes the same header limits and whole-file check as
//! text recognition, then ImageIO gives its first frame, upright by its EXIF
//! orientation (`image::upright_frame`), its longest side at most
//! `MODEL_IMAGE_MAX_SIDE`. A scanned page is drawn by the PDF renderer in
//! grey, its longest side at most `MODEL_IMAGE_MAX_SIDE`, never above the
//! recognition dpi. Either is drawn onto white in an 8-bit sRGB bitmap (so
//! transparency becomes white) and written as a JPEG at quality 0.8 by
//! ImageIO, which is given no properties but that quality. The file's own
//! properties are never read. ImageIO's own Exif and Photoshop segments are
//! then taken out (`jpeg::bare_jpeg`). At most 3 pictures, each at most
//! 1 MiB.

use std::ptr;

use objc2::rc::autoreleasepool;
use objc2_core_foundation::{
    CFMutableData, CFNumber, CFRetained, CFString, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage,
    CGImageAlphaInfo, CGImageByteOrderInfo, CGInterpolationQuality, kCGColorSpaceSRGB,
};
use objc2_image_io::{CGImageDestination, kCGImageDestinationLossyCompressionQuality};

use crate::memory_contracts::TextStatus;
use crate::pdf;

use super::image::{self, Refused, refused};
use super::jpeg::{MODEL_IMAGE_MAX_BYTES, MODEL_IMAGE_MAX_COUNT, MODEL_IMAGE_MAX_SIDE, bare_jpeg};
use super::render::{self, Gray};
use super::{Settings, has_usable_text_layer, scanned_pages, vision};

/// The JPEG quality ImageIO is given (0 to 1).
const JPEG_QUALITY: f64 = 0.8;

const NOT_PREPARED: &str = "The picture could not be prepared.";
const TOO_LARGE_TO_SEND: &str = "The prepared picture is larger than Traeva sends (1 MiB).";
const PDF_NOT_READ: &str = "The PDF could not be read, so no picture was made.";
const HAS_TEXT_LAYER: &str = "Only a PDF without a text layer is sent as pictures.";
const NO_SCANNED_PAGE: &str = "The PDF has no scanned page to send as a picture.";
const PAGE_NOT_DRAWN: &str = "A scanned page could not be drawn, so no picture was kept.";

/// One PNG or JPEG image as a picture for the model.
pub(crate) fn image_jpeg(bytes: &[u8]) -> Result<Vec<u8>, Refused> {
    let frame = image::upright_frame(bytes, |width, height| {
        (width.max(height) as usize).min(MODEL_IMAGE_MAX_SIDE)
    })?;
    jpeg_on_white(&frame)
}

/// A PDF with no text layer as pictures for the model: its first scanned
/// pages (the pages text recognition reads, at most 3), one picture each,
/// in page order. A page that cannot be drawn or written: no picture at all.
pub(crate) fn scanned_pdf_jpegs(
    bytes: &[u8],
    settings: &Settings,
) -> Result<Vec<Vec<u8>>, Refused> {
    let reading = pdf::read_with_pages(bytes);
    if reading.pages.is_empty() {
        let status = match reading.extraction.summary.status {
            TextStatus::Failed => TextStatus::Failed,
            _ => TextStatus::Unsupported,
        };
        return Err(refused(status, PDF_NOT_READ));
    }
    if has_usable_text_layer(reading.pages.iter().map(|page| page.state)) {
        return Err(refused(TextStatus::Unsupported, HAS_TEXT_LAYER));
    }
    let pages = scanned_pages(&reading, settings.max_ocr_pages.min(MODEL_IMAGE_MAX_COUNT));
    let page_count = reading.pages.len();
    drop(reading);
    if pages.is_empty() {
        return Err(refused(TextStatus::Unsupported, NO_SCANNED_PAGE));
    }
    let mut pictures = Vec::with_capacity(pages.len());
    render::for_each_page_planned(
        bytes,
        &pages,
        page_count,
        settings,
        |width, height| plan_for_model(width, height, settings),
        |_, rendered| {
            pictures.push(match rendered {
                Ok(rendered) => jpeg_from_grey(&rendered.gray),
                Err(_) => Err(refused(TextStatus::Failed, PAGE_NOT_DRAWN)),
            });
        },
    );
    if pictures.len() != pages.len() {
        return Err(refused(TextStatus::Failed, PAGE_NOT_DRAWN));
    }
    pictures.into_iter().collect()
}

/// A scanned page's scale and size for the model: the recognition dpi, or
/// less, so that its longest side is at most `MODEL_IMAGE_MAX_SIDE` pixels.
fn plan_for_model(
    width_pt: f32,
    height_pt: f32,
    settings: &Settings,
) -> Result<(f32, usize, usize), String> {
    let fitting_dpi = MODEL_IMAGE_MAX_SIDE as f32 * 72.0 / width_pt.max(height_pt);
    let planned = render::plan_dimensions(
        width_pt,
        height_pt,
        settings.dpi.min(fitting_dpi),
        settings.max_pixels,
    )?;
    if planned.1.max(planned.2) > MODEL_IMAGE_MAX_SIDE {
        return Err("the page does not fit a picture for the model".to_string());
    }
    Ok(planned)
}

fn jpeg_from_grey(gray: &Gray) -> Result<Vec<u8>, Refused> {
    let image = vision::cg_image(gray).map_err(|_| refused(TextStatus::Failed, NOT_PREPARED))?;
    jpeg_on_white(&image)
}

/// `image` drawn onto white in a new 8-bit sRGB bitmap of the same size,
/// written as a JPEG without metadata of at most `MODEL_IMAGE_MAX_BYTES`.
fn jpeg_on_white(image: &CGImage) -> Result<Vec<u8>, Refused> {
    let not_prepared = || refused(TextStatus::Failed, NOT_PREPARED);
    let (width, height) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    if width == 0 || height == 0 || width.max(height) > MODEL_IMAGE_MAX_SIDE {
        return Err(not_prepared());
    }
    let encoded = autoreleasepool(|_| {
        let drawn = srgb_on_white(image, width, height)?;
        encode_jpeg(&drawn)
    })
    .ok_or_else(not_prepared)?;
    let bare = bare_jpeg(&encoded).ok_or_else(not_prepared)?;
    if bare.len() > MODEL_IMAGE_MAX_BYTES {
        return Err(refused(TextStatus::Unsupported, TOO_LARGE_TO_SEND));
    }
    Ok(bare)
}

/// Draws `image` onto white in an 8-bit sRGB bitmap (the fourth byte of
/// each pixel unused: CoreGraphics has no 24-bit bitmap) that CoreGraphics
/// keeps, and returns the drawn picture.
fn srgb_on_white(image: &CGImage, width: usize, height: usize) -> Option<CFRetained<CGImage>> {
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    // SAFETY: no pixel buffer is passed in (null): CoreGraphics allocates
    // and owns the bitmap, so the picture made from it below stays valid
    // after the context is gone.
    let context = unsafe {
        CGBitmapContextCreate(
            ptr::null_mut(),
            width,
            height,
            8,
            0,
            Some(&space),
            CGImageAlphaInfo::NoneSkipLast.0 | CGImageByteOrderInfo::Order32Big.0,
        )
    }?;
    let all = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::set_interpolation_quality(Some(&context), CGInterpolationQuality::High);
    CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
    CGContext::fill_rect(Some(&context), all);
    CGContext::draw_image(Some(&context), all, Some(image));
    CGBitmapContextCreateImage(Some(&context))
}

/// ImageIO's JPEG of `image`, given only the quality: no other property, no
/// metadata, no thumbnail. ImageIO still adds its own Exif and Photoshop
/// segments; `jpeg_on_white` takes them out.
fn encode_jpeg(image: &CGImage) -> Option<Vec<u8>> {
    let data = CFMutableData::new(None, 0)?;
    let jpeg = CFString::from_static_str("public.jpeg");
    let destination = unsafe { CGImageDestination::with_data(&data, &jpeg, 1, None) }?;
    let quality = CFNumber::new_f64(JPEG_QUALITY);
    let properties = image::options(&[(
        unsafe { kCGImageDestinationLossyCompressionQuality },
        quality.as_ref(),
    )]);
    unsafe { destination.add_image(image, Some(properties.as_opaque())) };
    if !unsafe { destination.finalize() } {
        return None;
    }
    drop(destination);
    Some(data.to_vec())
}

#[cfg(test)]
#[path = "model_image_tests.rs"]
mod tests;
