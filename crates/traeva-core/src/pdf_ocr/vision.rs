//! Text recognition with Apple's Vision framework (VNRecognizeTextRequest),
//! on this Mac. Vision gets only the rendered pixels, as a CGImage; it
//! never sees the PDF.

use std::ptr;
use std::time::Instant;

use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2_core_foundation::CFData;
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::{NSArray, NSDictionary, NSProcessInfo, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};
use serde::Serialize;

use super::render::Gray;
use super::{Preparation, Settings};

/// One recognized line, in Vision's order.
#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub text: String,
    /// Vision's confidence for the line (0 to 1). Recorded only.
    pub confidence: f32,
    /// The line's box in the page, normalized (0 to 1), origin at the
    /// bottom left: x, y, width, height.
    pub bbox: [f64; 4],
}

/// What this Mac's Vision offers for text recognition.
#[derive(Debug, Clone, Serialize)]
pub struct VisionInfo {
    /// The revision a new request uses (the newest this Mac has).
    pub current_revision: usize,
    pub supported_languages_accurate: Vec<String>,
}

fn require_supported_version(major_version: isize) -> Result<(), String> {
    if major_version < 13 {
        return Err("unsupported: local PDF OCR requires macOS 13 or later".to_string());
    }
    Ok(())
}

fn require_supported_system() -> Result<(), String> {
    let version = autoreleasepool(|_| NSProcessInfo::processInfo().operatingSystemVersion());
    require_supported_version(version.majorVersion)
}

/// Whether this Mac's macOS can recognize text at all (13 or later). The
/// app asks before it sets up recognition, so that an older system shows
/// recognition as off instead of failing at every launch.
pub fn system_supported() -> bool {
    require_supported_system().is_ok()
}

fn require_supported_revision(revision: usize) -> Result<(), String> {
    if revision < 3 {
        return Err(
            "unsupported: local PDF OCR requires Vision text revision 3 or later".to_string(),
        );
    }
    Ok(())
}

pub fn vision_info() -> Result<VisionInfo, String> {
    require_supported_system()?;
    autoreleasepool(|_| {
        let request = VNRecognizeTextRequest::new();
        let current_revision = unsafe { request.revision() };
        require_supported_revision(current_revision)?;
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        let languages = unsafe { request.supportedRecognitionLanguagesAndReturnError() }
            .map_err(|e| e.localizedDescription().to_string())?;
        Ok(VisionInfo {
            current_revision,
            supported_languages_accurate: languages.iter().map(|l| l.to_string()).collect(),
        })
    })
}

/// Initialize with fixed in-memory pixels and the same request settings as OCR.
/// The supervising process owns cancellation and the preparation deadline.
pub fn prepare_ocr(settings: &Settings) -> Result<Preparation, String> {
    require_supported_system()?;
    let gray = preparation_image();
    let started = Instant::now();
    let lines = recognize(&gray, settings)?;
    let recognized_chars = lines
        .iter()
        .map(|line| line.text.chars().filter(|c| !c.is_whitespace()).count())
        .sum::<usize>();
    if recognized_chars == 0 {
        return Err(
            "Vision completed but did not recognize the synthetic preparation image".to_string(),
        );
    }
    Ok(Preparation {
        synthetic_fixture: "ocr-ready-bitmap-v1",
        pixels: (gray.width, gray.height),
        ocr_ms: started.elapsed().as_millis(),
        recognized_chars,
        lines,
    })
}

fn preparation_image() -> Gray {
    let mut gray = Gray {
        width: 960,
        height: 240,
        pixels: vec![255; 960 * 240],
    };
    let scale = 8;
    for (text, top) in [("OCR READY", 30), ("0123456789", 140)] {
        let left = (gray.width - text.len() * 6 * scale) / 2;
        for (position, letter) in text.bytes().enumerate() {
            let rows: [u8; 7] = match letter {
                b'O' | b'0' => [14, 17, 17, 17, 17, 17, 14],
                b'C' => [14, 17, 16, 16, 16, 17, 14],
                b'R' => [30, 17, 17, 30, 20, 18, 17],
                b'E' => [31, 16, 16, 30, 16, 16, 31],
                b'A' => [14, 17, 17, 31, 17, 17, 17],
                b'D' => [30, 17, 17, 17, 17, 17, 30],
                b'Y' => [17, 17, 10, 4, 4, 4, 4],
                b'1' => [4, 12, 4, 4, 4, 4, 14],
                b'2' => [14, 17, 1, 2, 4, 8, 31],
                b'3' => [30, 1, 1, 14, 1, 1, 30],
                b'4' => [2, 6, 10, 18, 31, 2, 2],
                b'5' => [31, 16, 16, 30, 1, 1, 30],
                b'6' => [14, 16, 16, 30, 17, 17, 14],
                b'7' => [31, 1, 2, 4, 8, 8, 8],
                b'8' => [14, 17, 17, 14, 17, 17, 14],
                b'9' => [14, 17, 17, 15, 1, 1, 14],
                _ => [0; 7],
            };
            for (row, bits) in rows.into_iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) == 0 {
                        continue;
                    }
                    for y in top + row * scale..top + (row + 1) * scale {
                        for x in left + (position * 6 + column) * scale
                            ..left + (position * 6 + column + 1) * scale
                        {
                            gray.pixels[y * gray.width + x] = 0;
                        }
                    }
                }
            }
        }
    }
    gray
}

/// Recognizes the text of one grey page. An error when Vision fails; an
/// empty list when it finds no text.
pub(crate) fn recognize(gray: &Gray, settings: &Settings) -> Result<Vec<Line>, String> {
    require_supported_system()?;
    autoreleasepool(|_| {
        let image = cg_image(gray)?;
        let request = VNRecognizeTextRequest::new();
        require_supported_revision(unsafe { request.revision() })?;
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(settings.language_correction);
        request.setAutomaticallyDetectsLanguage(settings.auto_language);
        let languages: Vec<_> = settings
            .languages
            .iter()
            .map(|l| NSString::from_str(l))
            .collect();
        request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
        let options = NSDictionary::new();
        let handler = unsafe {
            VNImageRequestHandler::initWithCGImage_options(
                VNImageRequestHandler::alloc(),
                &image,
                &options,
            )
        };
        let as_request: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[as_request]))
            .map_err(|e| e.localizedDescription().to_string())?;
        let Some(results) = request.results() else {
            return Ok(Vec::new());
        };
        let mut lines = Vec::with_capacity(results.len());
        for observation in results.iter() {
            let candidates = observation.topCandidates(1);
            let Some(best) = candidates.firstObject() else {
                continue;
            };
            let bbox = unsafe { observation.boundingBox() };
            lines.push(Line {
                text: best.string().to_string(),
                confidence: best.confidence(),
                bbox: [
                    bbox.origin.x,
                    bbox.origin.y,
                    bbox.size.width,
                    bbox.size.height,
                ],
            });
        }
        Ok(lines)
    })
}

/// The grey pixels as a CGImage (also a scanned page for the model,
/// `model_image`).
pub(crate) fn cg_image(gray: &Gray) -> Result<objc2_core_foundation::CFRetained<CGImage>, String> {
    if gray.width == 0
        || gray.height == 0
        || gray.width.checked_mul(gray.height) != Some(gray.pixels.len())
    {
        return Err("invalid grey pixel buffer".to_string());
    }
    let data = CFData::from_bytes(&gray.pixels);
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .ok_or("could not hand the pixels to CoreGraphics")?;
    let space = CGColorSpace::new_device_gray().ok_or("no grey colour space")?;
    unsafe {
        CGImage::new(
            gray.width,
            gray.height,
            8,
            8,
            gray.width,
            Some(&space),
            CGBitmapInfo(CGImageAlphaInfo::None.0),
            Some(&provider),
            ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(|| "could not make the page image".to_string())
}

#[cfg(test)]
mod tests {
    use super::{require_supported_revision, require_supported_version};

    #[test]
    fn unsupported_systems_are_rejected_before_vision_calls() {
        for version in [-1, 0, 10, 11, 12] {
            assert!(
                require_supported_version(version)
                    .unwrap_err()
                    .starts_with("unsupported:")
            );
        }
        assert!(require_supported_version(13).is_ok());
        assert!(require_supported_version(27).is_ok());
    }

    #[test]
    fn automatic_language_requires_revision_three() {
        for revision in [0, 1, 2] {
            assert!(
                require_supported_revision(revision)
                    .unwrap_err()
                    .starts_with("unsupported:")
            );
        }
        assert!(require_supported_revision(3).is_ok());
        assert!(require_supported_revision(4).is_ok());
    }
}
