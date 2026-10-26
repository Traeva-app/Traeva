//! Pictures for the model, the child's side, without Vision and without a
//! child process: ImageIO and the PDF renderer on the synthetic images
//! (experiments/image-ocr/make_images.py) and PDFs (tests/fixtures/pdf).
//! The real child with these arguments is the opt-in tests/model_image.rs.

use std::fs;
use std::path::PathBuf;

use super::super::image::{decode, header};
use super::super::jpeg::is_bare_jpeg;
use super::super::{ImageKind, Settings};
use super::*;

fn fixture(folder: &str, name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(folder)
            .join(name),
    )
    .unwrap()
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// A JPEG without metadata, its longest side at most 1568: its width and
/// height from its own frame header.
fn checked(jpeg: &[u8]) -> (u32, u32) {
    assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) && jpeg.ends_with(&[0xFF, 0xD9]));
    assert!(jpeg.len() <= MODEL_IMAGE_MAX_BYTES);
    assert!(is_bare_jpeg(jpeg), "metadata left");
    for word in [
        &b"Exif"[..],
        b"GPS",
        b"FAKE",
        b"XMP",
        b"ICC_PROFILE",
        b"Photoshop",
        b"8BIM",
    ] {
        assert!(!contains(jpeg, word), "{}", String::from_utf8_lossy(word));
    }
    let (kind, width, height) = header(jpeg).unwrap();
    assert_eq!(kind, ImageKind::Jpeg);
    assert!(width.max(height) as usize <= MODEL_IMAGE_MAX_SIDE);
    (width, height)
}

#[test]
fn an_image_becomes_one_jpeg_without_its_metadata() {
    for name in [
        "text-plain.png",
        "text-meta.jpg",
        "text-alpha.png",
        "block-cmyk.jpg",
        "block-gray16.png",
        "block-two-frames.png",
    ] {
        let bytes = fixture("images", name);
        let (_, width, height) = header(&bytes).unwrap();
        let jpeg = image_jpeg(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // Small enough already: the same size, not scaled up.
        assert_eq!(checked(&jpeg), (width, height), "{name}");
    }
}

#[test]
fn the_picture_is_upright_by_the_exif_orientation() {
    // Stored 320 x 900, a quarter turn to the left; orientation 6 turns it.
    assert_eq!(header(&fixture("images", "text-exif6.jpg")).unwrap().1, 320);
    let text = image_jpeg(&fixture("images", "text-exif6.jpg")).unwrap();
    assert_eq!(checked(&text), (900, 320));
    // The pixels are turned too, not only the size: the black block is at
    // the top left once the JPEG is decoded again.
    let block = image_jpeg(&fixture("images", "block-exif6.jpg")).unwrap();
    let gray = decode(&block, 8_000_000).unwrap();
    assert_eq!((gray.width, gray.height), (60, 40));
    let pixel = |x: usize, y: usize| gray.pixels[y * gray.width + x];
    assert!(pixel(3, 3) < 64, "the block is dark");
    assert!(pixel(56, 36) > 192, "the far corner is white");
}

#[test]
fn a_transparent_background_becomes_white() {
    let jpeg = image_jpeg(&fixture("images", "text-alpha.png")).unwrap();
    let gray = decode(&jpeg, 8_000_000).unwrap();
    assert!(gray.pixels[5 * gray.width + 5] > 245);
    assert!(gray.pixels.iter().any(|&p| p < 64), "the text is dark");
}

#[test]
fn a_large_image_is_scaled_to_the_longest_side() {
    let bytes = fixture("images", "white-12mp.png");
    let (_, width, height) = header(&bytes).unwrap();
    let (out_width, out_height) = checked(&image_jpeg(&bytes).unwrap());
    assert_eq!(out_width.max(out_height) as usize, MODEL_IMAGE_MAX_SIDE);
    // The shape is kept.
    let expected = f64::from(width.min(height)) * 1568.0 / f64::from(width.max(height));
    assert!((f64::from(out_width.min(out_height)) - expected).abs() <= 1.0);
}

#[test]
fn a_damaged_too_large_or_other_file_gives_no_picture() {
    for (bytes, status, words) in [
        (
            fixture("images", "truncated.png"),
            TextStatus::Failed,
            "cut off",
        ),
        (
            fixture("images", "truncated.jpg"),
            TextStatus::Failed,
            "cut off",
        ),
        (b"GIF89a....".to_vec(), TextStatus::Unsupported, "Only PNG"),
        (
            fixture("pdf", "p08-scanned-images.pdf"),
            TextStatus::Unsupported,
            "Only PNG",
        ),
    ] {
        let (got, note) = image_jpeg(&bytes).unwrap_err();
        assert_eq!(got, status, "{note}");
        assert!(note.contains(words), "{note}");
    }
    let mut huge = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    huge.extend(b"IHDR");
    huge.extend(30_000u32.to_be_bytes());
    huge.extend(10u32.to_be_bytes());
    let (status, note) = image_jpeg(&huge).unwrap_err();
    assert_eq!(status, TextStatus::Unsupported);
    assert!(note.contains("larger than"), "{note}");
}

#[test]
fn a_scanned_pdf_gives_one_jpeg_for_each_scanned_page() {
    let settings = Settings::default();
    let pictures = scanned_pdf_jpegs(&fixture("pdf", "p08-scanned-images.pdf"), &settings).unwrap();
    assert_eq!(pictures.len(), 2, "its two scanned pages");
    for picture in &pictures {
        let (width, height) = checked(picture);
        assert!(width > 0 && height > 0);
    }
}

#[test]
fn a_pdf_with_a_text_layer_or_one_the_reader_refuses_gives_no_picture() {
    let settings = Settings::default();
    for (name, words) in [
        ("p02-multi-en.pdf", "without a text layer"),
        ("p03-cjk-reportlab.pdf", "without a text layer"),
        ("e03a-libreoffice-open-password.pdf", "could not be read"),
    ] {
        let (status, note) = scanned_pdf_jpegs(&fixture("pdf", name), &settings).unwrap_err();
        assert_eq!(status, TextStatus::Unsupported, "{name}");
        assert!(note.contains(words), "{name}: {note}");
    }
    let (_, note) = scanned_pdf_jpegs(b"%PDF-1.7 nothing else", &settings).unwrap_err();
    assert!(note.contains("could not be read"), "{note}");
}

#[test]
fn a_page_is_drawn_at_most_1568_pixels_long_and_never_above_200_dpi() {
    let settings = Settings::default();
    // US Letter and A4 at 200 dpi would be 2200 and 2339 pixels long.
    for (width, height) in [(612.0, 792.0), (595.0, 842.0), (842.0, 595.0)] {
        let (_, w, h) = plan_for_model(width, height, &settings).unwrap();
        // Rounding in f32 can leave it one pixel short, never over.
        assert!(
            (MODEL_IMAGE_MAX_SIDE - 1..=MODEL_IMAGE_MAX_SIDE).contains(&w.max(h)),
            "{width} x {height}: {w} x {h}"
        );
    }
    // A small page stays at 200 dpi: 301 x 145 points is 836 x 402.
    let (_, w, h) = plan_for_model(301.0, 145.0, &settings).unwrap();
    assert_eq!((w, h), (836, 402));
    for (width, height) in [(0.0, 792.0), (f32::NAN, 792.0), (-612.0, 792.0)] {
        assert!(plan_for_model(width, height, &settings).is_err());
    }
}
