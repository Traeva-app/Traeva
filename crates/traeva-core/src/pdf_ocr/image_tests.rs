//! The image side of text recognition, without Vision: the header and its
//! limits, ImageIO's decoding (first frame, EXIF orientation, transparent,
//! 16-bit and CMYK images, damaged files) and the text rules with a
//! stand-in recognizer. The images are synthetic
//! (experiments/image-ocr/make_images.py).

use std::fs;
use std::path::PathBuf;

use super::*;

const BUDGET: usize = 8_000_000;

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/images")
            .join(name),
    )
    .unwrap()
}

/// A PNG's signature and IHDR, nothing after.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    bytes.extend(b"IHDR");
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 2, 0, 0, 0]);
    bytes
}

/// A JPEG's start, an APP0 segment, a fill byte, then a frame header.
fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0, 0xFF, 0xFF, 0xC0, 0x00, 0x0B, 8,
    ];
    bytes.extend(height.to_be_bytes());
    bytes.extend(width.to_be_bytes());
    bytes.extend([1, 1, 0x11, 0]);
    bytes
}

fn status(result: Result<(ImageKind, u32, u32), Refused>) -> Option<TextStatus> {
    result.err().map(|(status, _)| status)
}

fn pixel(gray: &Gray, x: usize, y: usize) -> u8 {
    gray.pixels[y * gray.width + x]
}

/// 60 x 40, white, with a black 10 x 10 block at the top left.
fn assert_block_upright(gray: &Gray) {
    assert_eq!((gray.width, gray.height), (60, 40));
    assert!(pixel(gray, 3, 3) < 64, "the block is dark");
    assert!(pixel(gray, 30, 20) > 192, "the middle is white");
    assert!(pixel(gray, 56, 36) > 192, "the far corner is white");
}

#[test]
fn the_header_gives_the_size_without_decoding() {
    assert_eq!(
        header(&png_header(900, 320)).unwrap(),
        (ImageKind::Png, 900, 320)
    );
    assert_eq!(
        header(&jpeg_header(640, 480)).unwrap(),
        (ImageKind::Jpeg, 640, 480)
    );
    assert_eq!(
        header(&fixture("text-plain.png")).unwrap(),
        (ImageKind::Png, 900, 320)
    );
    assert_eq!(
        header(&fixture("text-meta.jpg")).unwrap(),
        (ImageKind::Jpeg, 900, 320)
    );
    // As stored, before the EXIF orientation turns it.
    assert_eq!(
        header(&fixture("text-exif6.jpg")).unwrap(),
        (ImageKind::Jpeg, 320, 900)
    );
}

#[test]
fn an_image_over_the_limits_is_refused_from_its_header() {
    assert!(
        header(&png_header(20_000, 2_500)).is_ok(),
        "exactly 50 million pixels"
    );
    for bytes in [
        png_header(20_001, 10),
        png_header(10, 20_001),
        png_header(8_000, 7_000),
        png_header(u32::MAX, u32::MAX),
        jpeg_header(u16::MAX, u16::MAX),
    ] {
        assert_eq!(status(header(&bytes)), Some(TextStatus::Unsupported));
        // Only a header: decoding would fail. Unsupported shows it never ran.
        let (status, note) = decode(&bytes, BUDGET).err().unwrap();
        assert_eq!(status, TextStatus::Unsupported);
        assert!(note.contains("larger than"), "{note}");
    }
}

#[test]
fn only_png_and_jpeg_with_a_readable_header_are_accepted() {
    for other in [&b"%PDF-1.7\n"[..], b"GIF89a....", b"II*\0", b"", b"\x89PNG"] {
        assert_eq!(
            status(header(other)),
            Some(TextStatus::Unsupported),
            "{other:?}"
        );
    }
    let mut wrong_ihdr = png_header(100, 100);
    wrong_ihdr[11] = 12;
    let scan_before_frame = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
    let end_before_frame = vec![0xFF, 0xD8, 0xFF, 0xD9];
    let zero_height = jpeg_header(100, 0);
    let past_the_end = vec![0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF, 0x00];
    let not_a_marker = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0, 0x12, 0x34];
    // FF 00 before the real (huge) frame header: never read as a segment
    // whose length could skip to a small decoy frame header (#265 review).
    let mut stuffed_zero = vec![0xFF, 0xD8, 0xFF, 0x00, 0x00, 0x17];
    stuffed_zero.extend([
        0xFF, 0xC0, 0x00, 0x0B, 8, 0xFF, 0xFF, 0xFF, 0xFF, 1, 1, 0x11, 0,
    ]);
    stuffed_zero.extend([0, 0, 0, 0, 0, 0, 0, 0]);
    stuffed_zero.extend(jpeg_header(16, 16)[2..].iter());
    for damaged in [
        png_header(100, 100)[..20].to_vec(),
        wrong_ihdr,
        png_header(0, 100),
        scan_before_frame,
        end_before_frame,
        zero_height,
        past_the_end,
        not_a_marker,
        stuffed_zero,
        jpeg_header(100, 100)[..16].to_vec(),
    ] {
        assert_eq!(
            status(header(&damaged)),
            Some(TextStatus::Failed),
            "{damaged:?}"
        );
    }
}

#[test]
fn only_a_whole_file_is_decoded() {
    let png = fixture("text-plain.png");
    let jpeg = fixture("text-meta.jpg");
    assert!(ends_whole(&png, ImageKind::Png) && ends_whole(&jpeg, ImageKind::Jpeg));
    assert!(ends_whole(
        &[jpeg.clone(), vec![0; 64]].concat(),
        ImageKind::Jpeg
    ));
    for (bytes, kind) in [
        (png[..png.len() - 1].to_vec(), ImageKind::Png),
        ([png.clone(), b"trailing".to_vec()].concat(), ImageKind::Png),
        (jpeg[..jpeg.len() - 1].to_vec(), ImageKind::Jpeg),
        (vec![0; 8], ImageKind::Jpeg),
    ] {
        assert!(!ends_whole(&bytes, kind));
    }
}

#[test]
fn a_damaged_image_is_failed_not_a_crash() {
    for bytes in [
        fixture("truncated.png"),
        fixture("truncated.jpg"),
        [png_header(100, 100), vec![0x55; 4096]].concat(),
        [jpeg_header(100, 100), vec![0x55; 4096]].concat(),
    ] {
        let (status, note) = decode(&bytes, BUDGET).err().unwrap();
        assert_eq!(status, TextStatus::Failed);
        // These all end before a whole PNG or JPEG would.
        assert!(note.contains("cut off"), "{note}");
    }
}

#[test]
fn the_first_frame_is_decoded_upright_in_grey() {
    assert_block_upright(&decode(&fixture("block-exif6.jpg"), BUDGET).unwrap());
    assert_block_upright(&decode(&fixture("block-two-frames.png"), BUDGET).unwrap());
    let text = decode(&fixture("text-exif6.jpg"), BUDGET).unwrap();
    assert_eq!((text.width, text.height), (900, 320));
}

#[test]
fn transparent_sixteen_bit_and_cmyk_images_come_out_as_grey_on_white() {
    assert_block_upright(&decode(&fixture("block-gray16.png"), BUDGET).unwrap());
    assert_block_upright(&decode(&fixture("block-cmyk.jpg"), BUDGET).unwrap());
    let alpha = decode(&fixture("text-alpha.png"), BUDGET).unwrap();
    assert_eq!((alpha.width, alpha.height), (700, 160));
    assert!(
        pixel(&alpha, 5, 5) > 250,
        "the transparent background is white"
    );
    assert!(alpha.pixels.iter().any(|&p| p < 64), "the text is dark");
}

#[test]
fn a_large_image_is_decoded_within_the_pixel_budget() {
    let gray = decode(&fixture("white-12mp.png"), BUDGET).unwrap();
    assert!(gray.width * gray.height <= BUDGET);
    assert!(
        gray.width * gray.height > BUDGET * 9 / 10,
        "not scaled more than needed"
    );
    assert!(
        (gray.width * 3 / 4).abs_diff(gray.height) <= 1,
        "the shape is kept: {} x {}",
        gray.width,
        gray.height
    );
    assert!(gray.pixels.iter().all(|&p| p > 250));
}

#[test]
fn the_longest_side_keeps_the_frame_within_the_budget() {
    assert_eq!(longest_side(900, 320, BUDGET), 900);
    assert_eq!(longest_side(20_000, 400, BUDGET), 20_000);
    for (width, height) in [
        (4_000, 3_000),
        (3_000, 4_000),
        (7_071, 7_071),
        (20_000, 2_500),
    ] {
        let side = longest_side(width, height, BUDGET);
        let (long, short) = (width.max(height) as usize, width.min(height) as usize);
        let other = (short * side).div_ceil(long) + 1;
        assert!(side * other <= BUDGET, "{width} x {height}: {side}");
        assert!(side < long);
    }
}

fn line(text: &str) -> Line {
    Line {
        text: text.to_string(),
        confidence: 1.0,
        bbox: [0.0; 4],
    }
}

#[test]
fn the_text_is_the_recognized_lines_without_page_markers() {
    let settings = Settings::default();
    let mut seen = None;
    let text = text_from_image(&fixture("text-plain.png"), &settings, |gray| {
        seen = Some((gray.width, gray.height));
        Ok(vec![line("Field notes"), line("Invoice number 4471")])
    })
    .unwrap();
    assert_eq!(text, "Field notes\nInvoice number 4471");
    assert_eq!(seen, Some((900, 320)));
}

#[test]
fn too_little_text_a_recognizer_error_or_too_much_text_gives_no_text() {
    let settings = Settings::default();
    let bytes = fixture("text-plain.png");
    for lines in [vec![], vec![line("  \n ")], vec![line("123456789")]] {
        let (status, note) = text_from_image(&bytes, &settings, |_| Ok(lines)).unwrap_err();
        assert_eq!(status, TextStatus::Unsupported);
        assert!(note.contains("No usable OCR text"), "{note}");
    }
    assert!(text_from_image(&bytes, &settings, |_| Ok(vec![line("1234567890")])).is_ok());
    let (status, _) =
        text_from_image(&bytes, &settings, |_| Err("Vision failed".to_string())).unwrap_err();
    assert_eq!(status, TextStatus::Failed);
    let long = "文".repeat(MAX_MEMORY_TEXT_CHARS + 1);
    let (status, note) = text_from_image(&bytes, &settings, |_| Ok(vec![line(&long)])).unwrap_err();
    assert_eq!(status, TextStatus::Unsupported);
    assert!(note.contains("one-million-character"), "{note}");
}

#[test]
fn a_refused_image_never_reaches_the_recognizer() {
    let settings = Settings::default();
    for bytes in [
        png_header(30_000, 10),
        fixture("truncated.png"),
        b"GIF89a".to_vec(),
    ] {
        let result = text_from_image(&bytes, &settings, |_| -> Result<Vec<Line>, String> {
            panic!("the recognizer must not run")
        });
        assert!(result.is_err());
    }
}
