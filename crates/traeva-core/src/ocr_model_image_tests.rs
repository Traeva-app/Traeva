//! Pictures for the model, the parent's side (tier 2): the same child (a
//! shell stand-in here) with `--output jpeg`, the same limits, slot and
//! Ready rules as text recognition, the answer's checks, and nothing kept.
//! No ImageIO and no Vision; the real child on real files is the opt-in
//! tests/model_image.rs.

use super::*;
use crate::pdf_ocr::base64_encode;
use crate::settings::ImageSending;

fn image(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/images")
            .join(name),
    )
    .unwrap()
}

fn segment(code: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, code];
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend(body);
    out
}

/// A made-up JPEG in the shape the child writes (the parent never decodes
/// it): JFIF header, tables, frame header, one scan of `data`, end.
fn jpeg_with(data: &[u8], extra_segment: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    out.extend(segment(0xE0, b"JFIF\0\x01\x01\x00\x00\x48\x00\x48\x00\x00"));
    out.extend(extra_segment.unwrap_or_default());
    out.extend(segment(0xDB, &[0; 65]));
    out.extend(segment(0xC0, &[8, 0, 16, 0, 32, 1, 1, 0x11, 0]));
    out.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
    out.extend(data);
    out.extend([0xFF, 0xD9]);
    out
}

/// A made-up JPEG whose frame header says `width` × `height`.
fn sized(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    out.extend(segment(0xE0, b"JFIF\0\x01\x01\x00\x00\x48\x00\x48\x00\x00"));
    out.extend(segment(0xDB, &[0; 65]));
    let mut frame = vec![8];
    frame.extend(height.to_be_bytes());
    frame.extend(width.to_be_bytes());
    frame.extend([1, 1, 0x11, 0]);
    out.extend(segment(0xC0, &frame));
    out.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
    out.extend([1, 2, 3, 0xFF, 0xD9]);
    out
}

fn picture(tag: u8) -> Vec<u8> {
    jpeg_with(&[tag, 0xFF, 0x00, 0x34], None)
}

/// The child's answer with these JPEGs, one base64 line each.
fn answer_with(pictures: &[Vec<u8>]) -> String {
    let lines: Vec<String> = pictures.iter().map(|p| base64_encode(p)).collect();
    format!(
        "{{\"status\":\"partial\",\"text\":\"{}\",\"note\":null}}",
        lines.join("\\n")
    )
}

/// Answers `$1` only for an image with the JPEG output, nothing else after.
const IMAGE_PICTURES: &str = "/bin/cat > /dev/null; \
                              if [ \"$2\" = --input ] && [ \"$3\" = image ] && \
                              [ \"$4\" = --output ] && [ \"$5\" = jpeg ] && [ -z \"$6\" ]; \
                              then printf '%s' \"$1\"; else exit 9; fi";

/// Answers `$1` only for a scanned PDF with the JPEG output.
const PDF_PICTURES: &str = "/bin/cat > /dev/null; \
                            if [ \"$2\" = --output ] && [ \"$3\" = jpeg ] && [ -z \"$4\" ]; \
                            then printf '%s' \"$1\"; else exit 9; fi";

fn refused(result: Result<Vec<Vec<u8>>, String>, words: &str) {
    let reason = result.expect_err("no pictures");
    assert!(reason.contains(words), "{reason:?} should say {words:?}");
}

#[test]
fn an_image_gives_one_picture_from_the_same_child_with_the_jpeg_output() {
    let s = setup();
    s.ready_with(
        s.recognizer(IMAGE_PICTURES, &answer_with(&[picture(1)])),
        small,
    );
    let bytes = image("text-meta.jpg");

    let pictures = s.service().images_for_model(&bytes, Input::Image).unwrap();

    assert_eq!(pictures, [picture(1)]);
    assert_eq!(s.runs(), 1);
    // Nothing is kept: the same bytes start the child again.
    assert_eq!(
        s.service().images_for_model(&bytes, Input::Image).unwrap(),
        [picture(1)]
    );
    assert_eq!(s.runs(), 2);
    assert_eq!(s.service().ocr_state(), OcrState::Ready);
}

#[test]
fn a_scanned_pdf_gives_its_pictures_in_page_order() {
    let s = setup();
    let pages = [picture(1), picture(2), picture(3)];
    s.ready_with(s.recognizer(PDF_PICTURES, &answer_with(&pages)), small);

    let pictures = s
        .service()
        .images_for_model(b"%PDF-1.7 scanned stand-in\n", Input::ScannedPdf)
        .unwrap();

    assert_eq!(pictures, pages);
    assert_eq!(s.runs(), 1);
}

#[test]
fn the_child_s_refusal_is_passed_on_and_ready_is_kept() {
    for (answer, words) in [
        (
            "{\"status\":\"unsupported\",\"text\":\"\",\"note\":\"Only a PDF without a text layer \
             is sent as pictures.\"}",
            "without a text layer",
        ),
        (
            "{\"status\":\"failed\",\"text\":\"\",\"note\":null}",
            "could not be prepared",
        ),
    ] {
        let s = setup();
        s.ready_with(s.recognizer(PDF_PICTURES, answer), small);
        refused(
            s.service()
                .images_for_model(b"%PDF-1.7\n", Input::ScannedPdf),
            words,
        );
        assert_eq!(s.service().ocr_state(), OcrState::Ready);
    }
}

#[test]
fn a_picture_answer_out_of_form_is_dropped() {
    let exif = Some(segment(0xE1, b"Exif\0\0MM\0*GPS"));
    let not_whole = picture(1)[..picture(1).len() - 1].to_vec();
    let one = base64_encode(&picture(1));
    let image_answers = [
        // Two pictures for one image.
        answer_with(&[picture(1), picture(2)]),
        // Metadata left in, cut off, or no JPEG at all.
        answer_with(&[jpeg_with(&[1, 2, 3], exif)]),
        answer_with(&[not_whole]),
        answer_with(&[b"\x89PNG\r\n\x1a\n".to_vec()]),
        answer_with(&[]),
        // Not standard base64, a note, an extra field, another status.
        format!(
            "{{\"status\":\"partial\",\"text\":\"{}\",\"note\":null}}",
            one.trim_end_matches('=')
        ),
        format!("{{\"status\":\"partial\",\"text\":\"{one}\\n\",\"note\":null}}"),
        format!("{{\"status\":\"partial\",\"text\":\" {one}\",\"note\":null}}"),
        format!("{{\"status\":\"partial\",\"text\":\"{one}\",\"note\":\"OCR read one image.\"}}"),
        format!("{{\"status\":\"partial\",\"text\":\"{one}\",\"note\":null,\"pages\":1}}"),
        format!("{{\"status\":\"complete\",\"text\":\"{one}\",\"note\":null}}"),
        "not json".to_string(),
    ];
    let s = setup();
    for (tag, answer) in image_answers.iter().enumerate() {
        s.ready_with(s.recognizer(IMAGE_PICTURES, answer), small);
        let mut bytes = image("text-plain.png");
        bytes.push(tag as u8);
        refused(
            s.service().images_for_model(&bytes, Input::Image),
            "cannot use",
        );
        assert_eq!(s.service().ocr_state(), OcrState::Ready, "{answer}");
    }
    // A scanned PDF gives at most 3.
    let four = answer_with(&[picture(1), picture(2), picture(3), picture(4)]);
    s.ready_with(s.recognizer(PDF_PICTURES, &four), small);
    refused(
        s.service()
            .images_for_model(b"%PDF-1.7\n", Input::ScannedPdf),
        "cannot use",
    );
}

#[test]
fn a_picture_is_at_most_1_mib() {
    // Scan data without 0xFF, so that the JPEG is exactly `size` bytes.
    let of_size = |size: usize| {
        let frame = jpeg_with(&[], None).len();
        jpeg_with(&vec![0x55; size - frame], None)
    };
    let largest = of_size(MODEL_IMAGE_MAX_BYTES);
    assert_eq!(largest.len(), 1 << 20);
    let answer = answer_with(std::slice::from_ref(&largest));
    assert_eq!(
        answered_pictures(answer.as_bytes(), Input::Image).unwrap(),
        [largest]
    );
    let too_large = answer_with(&[of_size(MODEL_IMAGE_MAX_BYTES + 1)]);
    assert_eq!(
        answered_pictures(too_large.as_bytes(), Input::Image).unwrap_err(),
        PICTURE_UNUSABLE
    );
    // Three of the largest still fit the answer the parent reads.
    let three = answer_with(&vec![of_size(MODEL_IMAGE_MAX_BYTES); 3]);
    assert!(three.len() <= MAX_ANSWER_BYTES);
    assert_eq!(
        answered_pictures(three.as_bytes(), Input::ScannedPdf)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn pictures_need_a_ready_recognizer_and_a_png_or_jpeg() {
    let s = setup();
    let bytes = image("text-plain.png");
    // Off.
    refused(
        s.service().images_for_model(&bytes, Input::Image),
        "not available right now",
    );
    // Set, not prepared yet.
    let slow = sh("exec /bin/sleep 30", &[]);
    let recognize = s.recognizer(IMAGE_PICTURES, &answer_with(&[picture(1)]));
    s.service()
        .set_ocr_helper(Some(OcrHelper::commands(recognize, slow, small)));
    refused(
        s.service().images_for_model(&bytes, Input::Image),
        "not available right now",
    );
    // Preparing.
    std::thread::scope(|scope| {
        let preparing = scope.spawn(|| s.service().start_ocr_preparation());
        let deadline = Instant::now() + Duration::from_secs(10);
        while s.service().ocr_state() != OcrState::Preparing && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        refused(
            s.service().images_for_model(&bytes, Input::Image),
            "getting ready",
        );
        s.service().stop_ocr();
        assert_eq!(preparing.join().unwrap(), OcrState::Cancelled);
    });
    assert_eq!(s.runs(), 0);

    // Ready: only PNG and JPEG bytes reach the child; then the app quits.
    let s = setup();
    s.ready_with(
        s.recognizer(IMAGE_PICTURES, &answer_with(&[picture(1)])),
        small,
    );
    for other in [&b"GIF89a...."[..], b"%PDF-1.7\n", b""] {
        refused(
            s.service().images_for_model(other, Input::Image),
            "Only PNG and JPEG",
        );
    }
    s.service().stop_ocr();
    refused(
        s.service().images_for_model(&bytes, Input::Image),
        "not available right now",
    );
    assert_eq!(s.runs(), 0);
}

#[test]
fn a_crashed_picture_child_keeps_ready() {
    let s = setup();
    s.ready_with(
        s.recognizer("/bin/cat > /dev/null; kill -ABRT $$", ""),
        small,
    );
    refused(
        s.service()
            .images_for_model(&image("text-plain.png"), Input::Image),
        "stopped before it finished",
    );
    assert_eq!(s.service().ocr_state(), OcrState::Ready);
}

#[test]
fn a_slow_picture_child_is_stopped_and_ready_is_revoked() {
    let s = setup();
    let slow = s.recognizer("/bin/cat > /dev/null; sleep 5", "");
    s.ready_within(slow, small, Duration::from_millis(300));

    refused(
        s.service()
            .images_for_model(&image("text-plain.png"), Input::Image),
        "took too long",
    );

    assert_eq!(s.service().ocr_state(), OcrState::Failed);
    // No more children in this launch: neither pictures nor recognition.
    refused(
        s.service()
            .images_for_model(&image("text-meta.jpg"), Input::Image),
        "not available right now",
    );
    let text = s.service().image_extraction(&image("text-meta.jpg"));
    assert!(!text.summary.usable_text);
    assert_eq!(s.runs(), 1);
}

/// Raised by the one test below once its preparation is over.
static PICTURES_OVER_BUDGET: AtomicBool = AtomicBool::new(false);

fn huge_for_the_pictures(pid: u32) -> Option<ProcessMemory> {
    if PICTURES_OVER_BUDGET.load(Ordering::SeqCst) {
        huge(pid)
    } else {
        small(pid)
    }
}

#[test]
fn a_picture_child_over_its_memory_limit_is_stopped_and_ready_is_revoked() {
    let s = setup();
    let recognize = s.recognizer(
        "/bin/cat > /dev/null; sleep 1; printf '%s' \"$1\"",
        &answer_with(&[picture(1)]),
    );
    s.service().set_ocr_helper(Some(
        OcrHelper::commands(recognize, ready(), huge_for_the_pictures)
            .with_time_limits(Duration::from_secs(30), Duration::from_secs(10)),
    ));
    assert_eq!(s.service().start_ocr_preparation(), OcrState::Ready);
    PICTURES_OVER_BUDGET.store(true, Ordering::SeqCst);

    refused(
        s.service()
            .images_for_model(&image("text-plain.png"), Input::Image),
        "more memory",
    );

    assert_eq!(s.service().ocr_state(), OcrState::Failed);
}

/// Recognition's kept results and its "tried once" memory are not touched
/// by pictures, and pictures never use them: each asks the child itself.
#[test]
fn pictures_leave_recognition_s_kept_results_as_they_are() {
    let s = setup();
    let text = "{\"status\":\"partial\",\"text\":\"Invoice number 4471\\nTotal due 128.50\",\
                \"note\":null}";
    let script = format!(
        "/bin/cat > /dev/null; if [ \"$4\" = --output ]; then printf '%s' \"$1\"; \
         else printf '%s' '{text}'; fi"
    );
    s.ready_with(s.recognizer(&script, &answer_with(&[picture(1)])), small);
    let first = image("text-plain.png");

    assert!(s.service().image_extraction(&first).summary.recognized);
    assert_eq!(s.runs(), 1);
    assert!(s.service().images_for_model(&first, Input::Image).is_ok());
    assert_eq!(s.runs(), 2);
    // The text is still the kept one, without a child.
    let again = s.service().image_extraction(&first);
    assert!(again.summary.recognized);
    assert_eq!(
        again.excerpt.concat(),
        "Invoice number 4471\nTotal due 128.50"
    );
    assert_eq!(s.runs(), 2);
    assert!(s.service().images_for_model(&first, Input::Image).is_ok());
    assert_eq!(s.runs(), 3);

    // Pictures first: the bytes are still recognized afterwards.
    let second = image("text-meta.jpg");
    assert!(s.service().images_for_model(&second, Input::Image).is_ok());
    assert_eq!(s.runs(), 4);
    assert!(s.service().image_extraction(&second).summary.recognized);
    assert_eq!(s.runs(), 5);
}

/// Pictures and recognition share the one-child slot: the second waits.
#[test]
fn a_picture_child_and_a_recognition_run_one_at_a_time() {
    let s = setup();
    let log = s.runs.path().join("log").into_os_string();
    let script = format!(
        "echo start >> \"$0\"; /bin/cat > /dev/null; /bin/sleep 0.3; echo end >> \"$0\"; \
         if [ \"$2\" = --output ]; then printf '%s' \"$1\"; else printf '%s' '{RECOGNIZED}'; fi"
    );
    let pictures = answer_with(&[picture(1)]);
    s.ready_with(sh(&script, &[&log, &OsString::from(pictures)]), small);
    std::thread::scope(|scope| {
        let recognition = scope.spawn(|| s.choose("scan.pdf"));
        let picture_run = scope.spawn(|| {
            s.service()
                .images_for_model(b"%PDF-1.7 another scan\n", Input::ScannedPdf)
        });
        let id = recognition.join().unwrap();
        assert!(s.view(&id).file.unwrap().extraction.recognized);
        assert_eq!(picture_run.join().unwrap().unwrap(), [picture(1)]);
    });
    let log = fs::read_to_string(s.runs.path().join("log")).unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        ["start", "end", "start", "end"]
    );
}

// ---------------------------------------------------------------------------
// A's wiring (§14.10): when Organize sends the pictures with its request.
// ---------------------------------------------------------------------------

/// Text found in the image, for the recognition run.
const SEEN_TEXT: &str = "{\"status\":\"partial\",\"text\":\"Invoice number 4471\\nTotal due \
                         128.50\",\"note\":null}";
const NO_TEXT_SEEN: &str =
    "{\"status\":\"unsupported\",\"text\":\"\",\"note\":\"No usable OCR text was found.\"}";

/// One child for both runs: the pictures (`--output jpeg` after
/// `--input image`) come from the file `$1`, the recognition answer is `$2`'s
/// text after it.
fn both_runs(s: &Setup, recognition: &str) -> (PathBuf, Vec<OsString>) {
    let pictures = s.runs.path().join("pictures.json");
    fs::write(&pictures, answer_with(&[picture(7)])).unwrap();
    let script = "echo run >> \"$0\"; /bin/cat > /dev/null; \
                  if [ \"$5\" = --output ]; then /bin/cat \"$1\"; else printf '%s' \"$2\"; fi";
    sh(
        script,
        &[
            &s.count_file(),
            &pictures.into_os_string(),
            &OsString::from(recognition),
        ],
    )
}

/// Whether the last request carried a picture, and how many the metadata
/// says were attached.
fn sent_pictures(s: &Setup) -> (bool, Value) {
    let sent = s
        .server
        .last(|r| r.json()["input"].to_string().contains("input_image"));
    let meta = s.f.db().query_row(
        "SELECT request_meta_json FROM analyses ORDER BY rowid DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    );
    let meta: Value = serde_json::from_str(&meta.unwrap()).unwrap();
    (sent, meta["context"]["images"].clone())
}

#[test]
fn an_image_goes_to_the_model_only_as_the_setting_and_the_user_allow() {
    let s = setup();
    fs::write(s.f.path("photo.png"), image("text-plain.png")).unwrap();
    s.ready_with(both_runs(&s, SEEN_TEXT), small);

    // Off (the default): the recognized text only.
    let id = s.choose("photo.png");
    assert!(s.view(&id).file.unwrap().extraction.image_input);
    s.analyze(&id);
    assert_eq!(sent_pictures(&s), (false, json!(0)));

    // Ask each time, and the user did not say yes: still text only.
    s.service()
        .set_image_sending(ImageSending::AskEachTime)
        .unwrap();
    let id = s.choose("photo.png");
    s.analyze(&id);
    assert_eq!(sent_pictures(&s), (false, json!(0)));

    // Ask each time, and the user said yes: the picture goes too.
    let id = s.choose("photo.png");
    s.service().start_analysis_sending(&id, true).unwrap();
    s.service()
        .run_analysis_with(&id, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        });
    assert_eq!(sent_pictures(&s), (true, json!(1)));
    let image_url = s.server.last(|r| {
        r.json()["input"][0]["content"][1]["image_url"]
            .as_str()
            .unwrap()
            .to_string()
    });
    assert_eq!(
        image_url,
        format!("data:image/jpeg;base64,{}", base64_encode(&picture(7)))
    );

    // Without asking: the picture goes without the user's yes.
    s.service()
        .set_image_sending(ImageSending::WithoutAsking)
        .unwrap();
    let id = s.choose("photo.png");
    s.analyze(&id);
    assert_eq!(sent_pictures(&s), (true, json!(1)));
}

#[test]
fn an_image_without_text_is_analysed_only_with_sending_images_on() {
    let mut s = setup();
    fs::write(s.f.path("photo.png"), image("text-plain.png")).unwrap();
    s.ready_with(both_runs(&s, NO_TEXT_SEEN), small);

    let id = s.choose("photo.png");
    assert_eq!(s.view(&id).stage, OperationStage::NeedsManualLocation);

    // The model may cite the picture as its evidence.
    let travel = s
        .service()
        .list_folders(&s.f.root_id)
        .unwrap()
        .folders
        .into_iter()
        .find(|folder| folder.relative_path == "Travel")
        .unwrap()
        .folder_id;
    let answer = json!({
        "disposition": "recommend",
        "summary": "A receipt from a trip.",
        "candidates": [{ "id": "C1", "folder_id": travel, "new_segments": [],
                         "reason": "The picture shows a hotel receipt.", "evidence_ids": ["I1"] }],
        "preferred_id": "C1",
        "confidence": 0.8,
        "uncertain_field": null,
        "clarification": null
    });
    s.server = TestServer::start(Planned::answer(200, completed(&answer)));
    s.f.service_mut().model.endpoint = s.server.url.clone();
    s.service()
        .set_image_sending(ImageSending::WithoutAsking)
        .unwrap();

    let id = s.choose("photo.png");
    assert_eq!(s.view(&id).stage, OperationStage::ReadyForAnalysis);
    s.analyze(&id);
    let view = s.view(&id);
    assert_eq!(view.stage, OperationStage::RecommendationReady, "{view:?}");
    assert_eq!(sent_pictures(&s), (true, json!(1)));
    // With pictures the input is one message: the text first, then them.
    let input = s.server.last(|r| {
        r.json()["input"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    });
    let (_, data) = input.split_once('\n').unwrap();
    let data: Value = serde_json::from_str(data).unwrap();
    assert_eq!(data["text"]["pieces"], json!([]));
    assert_eq!(data["pictures"][0]["id"], "I1");
}

/// The pictures are chosen with the setup the request is sent with: image
/// sending turned off while the copy is being made stops the request, and
/// nothing goes out (§14.10, P31).
#[test]
fn image_sending_turned_off_while_the_copy_is_made_sends_nothing() {
    let s = setup();
    fs::write(s.f.path("photo.png"), image("text-plain.png")).unwrap();
    let pictures = s.runs.path().join("pictures.json");
    fs::write(&pictures, answer_with(&[picture(7)])).unwrap();
    let waiting = s.runs.path().join("waiting");
    let go = s.runs.path().join("go");
    // As `both_runs`, but the picture run says it started and then waits.
    let script = format!(
        "echo run >> \"$0\"; /bin/cat > /dev/null; \
         if [ \"$5\" = --output ]; then : > '{}'; \
         while [ ! -e '{}' ]; do /bin/sleep 0.05; done; /bin/cat \"$1\"; \
         else printf '%s' \"$2\"; fi",
        waiting.display(),
        go.display()
    );
    s.ready_with(
        sh(
            &script,
            &[
                &s.count_file(),
                &pictures.into_os_string(),
                &OsString::from(SEEN_TEXT),
            ],
        ),
        small,
    );
    s.service()
        .set_image_sending(ImageSending::WithoutAsking)
        .unwrap();
    let id = s.choose("photo.png");
    std::thread::scope(|scope| {
        let analysis = scope.spawn(|| s.analyze(&id));
        let started = Instant::now();
        while !waiting.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the picture run never started"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        s.service().set_image_sending(ImageSending::Off).unwrap();
        fs::write(&go, b"").unwrap();
        analysis.join().unwrap();
    });
    assert_eq!(s.server.count(), 0, "nothing reached the model");
    let status: String =
        s.f.db()
            .query_row(
                "SELECT status FROM analyses ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(status, "cancelled");
}

#[test]
fn a_diagnostic_record_never_keeps_the_picture() {
    let s = setup();
    fs::write(s.f.path("photo.png"), image("text-plain.png")).unwrap();
    s.ready_with(both_runs(&s, SEEN_TEXT), small);
    s.service()
        .set_image_sending(ImageSending::WithoutAsking)
        .unwrap();
    s.service().set_model_diagnostics(true).unwrap();

    let id = s.choose("photo.png");
    s.analyze(&id);
    assert!(sent_pictures(&s).0);

    let dir = crate::settings::diagnostics_dir(&s.f.app.path().join("Traeva"));
    let encoded = base64_encode(&picture(7));
    assert!(crate::model::tests::files_containing(&dir, &encoded).is_empty());
    assert_eq!(
        crate::model::tests::files_containing(&dir, "[image not kept]").len(),
        1
    );
}

/// The parent checks the picture's size itself (#37 review): a child that
/// went wrong cannot hand over a larger picture than 1568 pixels a side.
#[test]
fn a_picture_larger_than_1568_pixels_is_refused_by_the_parent() {
    for (width, height, kept) in [
        (1568, 1176, true),
        (1569, 10, false),
        (10, 1569, false),
        (0, 20, false),
    ] {
        let s = setup();
        s.ready_with(
            s.recognizer(IMAGE_PICTURES, &answer_with(&[sized(width, height)])),
            small,
        );
        let pictures = s
            .service()
            .images_for_model(&image("text-plain.png"), Input::Image);
        assert_eq!(pictures.is_ok(), kept, "{width}×{height}: {pictures:?}");
    }
}
