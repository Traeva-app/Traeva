//! Text recognition from one PNG or JPEG image, the parent's side (#37):
//! the same child (a shell stand-in here) with `--input image`, the same
//! limits, slot and preparation, and the image's own answer rules. No
//! Vision; the real child on real images is the opt-in tests/ocr_image.rs.

use super::*;

fn image(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/images")
            .join(name),
    )
    .unwrap()
}

/// `text-plain.png` with one more byte at the end: other bytes, still a PNG.
fn another_png(tag: u8) -> Vec<u8> {
    let mut bytes = image("text-plain.png");
    bytes.push(tag);
    bytes
}

const IMAGE_TEXT: &str = "{\"status\":\"partial\",\"text\":\"Invoice number 4471\\nTotal due \
                          128.50\",\"note\":\"OCR read one image; may contain errors.\"}";

/// Answers `$1` only when the parent added `--input image` (`$2 $3`).
const FOR_AN_IMAGE: &str = "/bin/cat > /dev/null; \
                            if [ \"$2\" = --input ] && [ \"$3\" = image ]; then \
                            printf '%s' \"$1\"; else exit 9; fi";

fn not_usable(extraction: &Extraction, words: &str) {
    assert!(!extraction.summary.usable_text, "{extraction:?}");
    assert!(!extraction.summary.recognized);
    let note = extraction.summary.note.clone().unwrap_or_default();
    assert!(note.contains(words), "note {note:?} should say {words:?}");
}

#[test]
fn an_image_is_recognized_by_the_same_child_and_marked_as_recognized() {
    let s = setup();
    s.ready_with(s.recognizer(FOR_AN_IMAGE, IMAGE_TEXT), small);
    let bytes = image("text-plain.png");

    let extraction = s.service().image_extraction(&bytes);

    assert_eq!(extraction.summary.status, ExtractionStatus::Partial);
    assert!(extraction.summary.usable_text && extraction.summary.recognized);
    assert_eq!(
        extraction.summary.note.as_deref(),
        Some(IMAGE_RECOGNIZED_NOTE)
    );
    assert_eq!(
        extraction.excerpt.concat(),
        "Invoice number 4471\nTotal due 128.50"
    );
    assert_eq!(s.runs(), 1);
    // The same bytes again: the kept result, no second child.
    assert_eq!(
        s.service().image_extraction(&bytes).excerpt,
        extraction.excerpt
    );
    assert_eq!(s.runs(), 1);
}

#[test]
fn a_scanned_pdf_never_gets_the_image_input() {
    let s = setup();
    let pdf_only = "/bin/cat > /dev/null; [ -z \"$2\" ] || exit 9; printf '%s' \"$1\"";
    s.ready_with(s.recognizer(pdf_only, RECOGNIZED), small);
    let id = s.choose("scan.pdf");
    assert!(s.view(&id).file.unwrap().extraction.recognized);
}

#[test]
fn only_png_and_jpeg_bytes_reach_the_child() {
    let s = setup();
    s.ready_with(s.recognizer(FOR_AN_IMAGE, IMAGE_TEXT), small);
    for bytes in [&b"%PDF-1.7\n"[..], b"GIF89a....", b"", b"\x89PNG"] {
        let extraction = s.service().image_extraction(bytes);
        assert_eq!(extraction.summary.status, ExtractionStatus::Unsupported);
        not_usable(&extraction, "Only PNG and JPEG");
    }
    assert_eq!(s.runs(), 0);
    let jpeg = s.service().image_extraction(&image("text-meta.jpg"));
    assert!(jpeg.summary.recognized);
    assert_eq!(s.runs(), 1);
}

#[test]
fn no_text_or_a_damaged_image_goes_to_a_manual_pick_once() {
    for (answer, status, words) in [
        (
            "{\"status\":\"unsupported\",\"text\":\"\",\"note\":\"No usable OCR text was found in \
             the image.\"}",
            ExtractionStatus::Unsupported,
            "No usable OCR text",
        ),
        (
            "{\"status\":\"failed\",\"text\":\"\",\"note\":\"The image is damaged or could not \
             be decoded.\"}",
            ExtractionStatus::Failed,
            "damaged",
        ),
        (
            "{\"status\":\"failed\",\"text\":\"\",\"note\":null}",
            ExtractionStatus::Failed,
            "could not read text in this image",
        ),
    ] {
        let s = setup();
        s.ready_with(s.recognizer(FOR_AN_IMAGE, answer), small);
        let bytes = image("truncated.png");
        for _ in 0..2 {
            let extraction = s.service().image_extraction(&bytes);
            assert_eq!(extraction.summary.status, status);
            not_usable(&extraction, words);
        }
        assert_eq!(s.runs(), 1, "never tried again in this launch");
        assert_eq!(s.service().ocr_state(), OcrState::Ready);
    }
}

#[test]
fn an_image_answer_out_of_form_is_dropped() {
    let s = setup();
    for (tag, answer) in [
        "{\"status\":\"complete\",\"text\":\"Invoice number 4471\",\"note\":null}",
        "{\"status\":\"partial\",\"text\":\"123456789\",\"note\":null}",
        "{\"status\":\"partial\",\"text\":\" \\n \",\"note\":null}",
        "{\"status\":\"partial\",\"text\":\"Invoice number 4471\",\"note\":null,\"extra\":1}",
        "not json",
    ]
    .into_iter()
    .enumerate()
    {
        s.ready_with(s.recognizer(FOR_AN_IMAGE, answer), small);
        let extraction = s.service().image_extraction(&another_png(tag as u8));
        assert_eq!(
            extraction.summary.status,
            ExtractionStatus::Failed,
            "{answer}"
        );
        not_usable(&extraction, "cannot use");
    }
    let long = format!(
        "{{\"status\":\"partial\",\"text\":\"{}\",\"note\":null}}",
        "x".repeat(MAX_MEMORY_TEXT_CHARS + 1)
    );
    s.ready_with(s.recognizer(FOR_AN_IMAGE, &long), small);
    let extraction = s.service().image_extraction(&another_png(99));
    not_usable(&extraction, "more text than Traeva reads");
}

#[test]
fn the_child_s_note_never_reaches_the_image_s_note() {
    let s = setup();
    let answer = "{\"status\":\"partial\",\"text\":\"Library card renewal\",\"note\":\"FAKE-ARTIST-NAME \
                  at 0.0, 0.0\"}";
    s.ready_with(s.recognizer(FOR_AN_IMAGE, answer), small);
    let extraction = s.service().image_extraction(&image("text-meta.jpg"));
    assert_eq!(
        extraction.summary.note.as_deref(),
        Some(IMAGE_RECOGNIZED_NOTE)
    );
    assert_eq!(extraction.excerpt.concat(), "Library card renewal");
}

#[test]
fn a_slow_image_recognition_is_stopped_and_ready_is_revoked() {
    let s = setup();
    let slow = s.recognizer("/bin/cat > /dev/null; sleep 5", IMAGE_TEXT);
    s.ready_within(slow, small, Duration::from_millis(300));

    let extraction = s.service().image_extraction(&image("text-plain.png"));

    not_usable(&extraction, "took too long");
    assert_eq!(s.service().ocr_state(), OcrState::Failed);
    let next = s.service().image_extraction(&image("text-meta.jpg"));
    not_usable(&next, "not available right now");
    assert_eq!(s.runs(), 1);
}

/// Raised by the one test below once its preparation is over.
static OVER_BUDGET: AtomicBool = AtomicBool::new(false);

/// Small for the preparation, over the budget once the image is read.
fn huge_for_the_image(pid: u32) -> Option<ProcessMemory> {
    if OVER_BUDGET.load(Ordering::SeqCst) {
        huge(pid)
    } else {
        small(pid)
    }
}

#[test]
fn an_image_over_its_memory_limit_is_stopped_and_ready_is_revoked() {
    let s = setup();
    let recognize = s.recognizer(
        "/bin/cat > /dev/null; sleep 1; printf '%s' \"$1\"",
        IMAGE_TEXT,
    );
    s.service().set_ocr_helper(Some(
        OcrHelper::commands(recognize, ready(), huge_for_the_image)
            .with_time_limits(Duration::from_secs(30), Duration::from_secs(10)),
    ));
    assert_eq!(s.service().start_ocr_preparation(), OcrState::Ready);
    OVER_BUDGET.store(true, Ordering::SeqCst);

    let extraction = s.service().image_extraction(&image("text-plain.png"));

    not_usable(&extraction, "more memory");
    assert_eq!(s.service().ocr_state(), OcrState::Failed);
}

#[test]
fn when_recognition_is_off_or_stopped_an_image_goes_to_a_manual_pick() {
    let s = setup();
    not_usable(
        &s.service().image_extraction(&image("text-plain.png")),
        "not available right now",
    );
    s.ready_with(s.recognizer(FOR_AN_IMAGE, IMAGE_TEXT), small);
    s.service().stop_ocr();
    not_usable(
        &s.service().image_extraction(&image("text-plain.png")),
        "not available right now",
    );
    assert_eq!(s.runs(), 0);
}

/// A's routing (§14.9): Organize's own reading of a .jpg/.jpeg/.png name
/// recognizes it; Memory's and Source's registration (`Never`), other image
/// kinds and names without a stem never start the child.
#[test]
fn only_organize_routes_a_jpeg_or_png_name_to_recognition() {
    let s = setup();
    s.ready_with(s.recognizer(FOR_AN_IMAGE, IMAGE_TEXT), small);
    let bytes = image("text-plain.png");

    for name in ["photo.png", "photo.PNG", "notes.jpg", "notes.JPEG"] {
        let extraction = s
            .service()
            .organize_extraction(name, &bytes, OcrUse::Allowed);
        assert!(extraction.summary.recognized, "{name}: {extraction:?}");
    }
    // One child for the same bytes, whatever the name.
    assert_eq!(s.runs(), 1);

    for name in ["photo.png", "notes.jpg"] {
        let extraction = s.service().organize_extraction(name, &bytes, OcrUse::Never);
        assert!(!extraction.summary.recognized, "{name}");
        assert!(!extraction.summary.usable_text, "{name}");
    }
    for name in ["photo.heic", "photo.gif", "photo.png.txt", ".png"] {
        let extraction = s
            .service()
            .organize_extraction(name, &another_png(7), OcrUse::Allowed);
        assert!(!extraction.summary.recognized, "{name}");
    }
    assert_eq!(s.runs(), 1);
}
