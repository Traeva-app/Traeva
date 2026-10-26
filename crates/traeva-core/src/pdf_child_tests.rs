//! Tests for pdf_child.rs (#186, organize-spec.md §14) inside the process:
//! a stand-in reader answers, so these check what Organize does with the
//! answer. The process itself is tested in tests/pdf_child.rs. Offline:
//! the local test server plays OpenAI. Synthetic files only.

use std::collections::HashMap;
use std::fs;

use serde_json::{Value, json};

use super::*;
use crate::contracts::OperationStage;
use crate::memory_contracts::{TextExtraction, TextSummary};
use crate::model::tests::{Planned, TestServer, completed};
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::test_fixture::{Fixture, quick_read};

const TEXT: &str = "Trip to Kyoto in May: train tickets and a hotel near the station.\n\n\
                    Budget about 400 USD for the week.\n";

fn reading(status: TextStatus, text: &str, note: Option<&str>) -> TextExtraction {
    TextExtraction {
        summary: TextSummary {
            reader: "pdf-1".to_string(),
            status,
            chars: text.chars().count(),
            covered: Vec::new(),
            left_out: Vec::new(),
            note: note.map(str::to_string),
        },
        text: text.to_string(),
    }
}

fn with_text(_bytes: &[u8]) -> TextExtraction {
    reading(TextStatus::Complete, TEXT, None)
}

fn partly(_bytes: &[u8]) -> TextExtraction {
    reading(
        TextStatus::Partial,
        TEXT,
        Some("Images on 2 pages were not read."),
    )
}

fn no_text_layer(_bytes: &[u8]) -> TextExtraction {
    reading(
        TextStatus::Unsupported,
        "",
        Some("This PDF has no text layer: its pages are images."),
    )
}

struct Setup {
    f: Fixture,
    server: TestServer,
}

/// A root with Travel/ and trip.pdf, model settings and Organize's sending
/// agreement, the stand-in reader `read`, and a server recommending Travel.
fn setup(read: fn(&[u8]) -> TextExtraction) -> Setup {
    let mut f = Fixture::new();
    fs::create_dir(f.path("Travel")).unwrap();
    fs::write(f.path("trip.pdf"), b"%PDF-1.7 stand-in\n").unwrap();
    let ids: HashMap<String, String> = f
        .service()
        .list_folders(&f.root_id)
        .unwrap()
        .folders
        .into_iter()
        .map(|folder| (folder.relative_path, folder.folder_id))
        .collect();
    let answer = json!({
        "disposition": "recommend",
        "summary": "Notes for a trip.",
        "candidates": [{ "id": "C1", "folder_id": ids["Travel"], "new_segments": [],
                         "reason": "Fits the trip.", "evidence_ids": ["E1"] }],
        "preferred_id": "C1",
        "confidence": 0.9,
        "uncertain_field": null,
        "clarification": null
    });
    let server = TestServer::start(Planned::answer(200, completed(&answer)));
    f.service_mut().model.endpoint = server.url.clone();
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    f.service().set_analysis_consent(true).unwrap();
    f.service()
        .set_pdf_helper(Some(PdfHelper::in_process(read)));
    Setup { f, server }
}

impl Setup {
    /// Chooses trip.pdf in a new operation and reads it.
    fn choose(&self) -> String {
        let id = self
            .f
            .service()
            .start_operation(&self.f.root_id, &self.f.root_id)
            .unwrap()
            .operation_id;
        self.f.service().run_operation_with(
            &id,
            &self.f.path("trip.pdf"),
            &mut |root, parent, name| quick_read(parent, name, root.id().dev),
        );
        id
    }

    fn analyze(&self, id: &str) {
        self.f.service().start_analysis(id).unwrap();
        self.f
            .service()
            .run_analysis_with(id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            });
    }

    /// The data part of the last request, parsed.
    fn sent_data(&self) -> Value {
        let input = self
            .server
            .last(|r| r.json()["input"].as_str().unwrap().to_string());
        let (_, data) = input.split_once('\n').unwrap();
        serde_json::from_str(data).unwrap()
    }
}

#[test]
fn a_pdf_with_a_text_layer_is_suggested_like_a_text_file() {
    let s = setup(with_text);
    let id = s.choose();
    let view = s.f.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::ReadyForAnalysis, "{view:?}");
    let extraction = &view.file.unwrap().extraction;
    assert!(extraction.usable_text);
    assert_eq!(extraction.status, ExtractionStatus::Ok);

    s.analyze(&id);
    let view = s.f.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::RecommendationReady, "{view:?}");
    // What was sent is what §5 sends for any file (§14.4): its name and the
    // text read from the PDF, cut into evidence pieces.
    let data = s.sent_data();
    assert_eq!(data["file_name"], "trip.pdf");
    assert!(data.to_string().contains("Trip to Kyoto in May"));
    assert_eq!(s.server.count(), 1);
}

#[test]
fn a_partly_read_pdf_says_so_and_can_still_be_suggested() {
    let s = setup(partly);
    let id = s.choose();
    let view = s.f.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::ReadyForAnalysis, "{view:?}");
    let extraction = view.file.unwrap().extraction;
    assert!(extraction.usable_text);
    // Short enough to be used whole, and still partial: the reader left
    // the images out (R2-PDF-01, #196).
    assert_eq!(extraction.status, ExtractionStatus::Partial);
    assert!(!extraction.truncated);
    assert_eq!(
        extraction.note.as_deref(),
        Some("Images on 2 pages were not read.")
    );
}

#[test]
fn a_pdf_without_a_text_layer_goes_to_a_manual_pick_and_nothing_is_sent() {
    let s = setup(no_text_layer);
    let id = s.choose();
    let view = s.f.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::NeedsManualLocation, "{view:?}");
    let extraction = view.file.unwrap().extraction;
    assert_eq!(extraction.status, ExtractionStatus::Unsupported);
    assert!(extraction.note.unwrap().contains("no text layer"));
    assert!(s.f.service().start_analysis(&id).is_err());
    assert_eq!(s.server.count(), 0);
}

#[test]
fn memory_never_reads_a_pdf() {
    let read = crate::reader::read_text("trip.pdf", b"%PDF-1.7 stand-in\n");
    assert_eq!(read.summary.status, TextStatus::Unsupported);
    assert!(read.text.is_empty());
}

// Word, PowerPoint and Excel in Organize (the student's decision 10/17,
// organize-spec v0.16): Memory's readers, then the usual excerpt.

#[test]
fn organize_reads_word_powerpoint_and_excel_with_memory_s_readers() {
    let word: &[u8] = include_bytes!("../tests/fixtures/docx/word-basic-v1.docx");
    let deck: &[u8] = include_bytes!("../tests/fixtures/pptx/basic-source.pptx");
    let f = Fixture::new();
    let read = |name: &str, bytes: &[u8]| {
        f.service()
            .organize_extraction(name, bytes, OcrUse::Allowed)
    };

    for (name, bytes) in [
        ("notes.docx", word),
        ("Notes.DOCX", word),
        ("deck.pptx", deck),
    ] {
        let extraction = read(name, bytes);
        let memory = crate::reader::read_text(name, bytes);
        // The reader's own status: a deck is partial (only its stored
        // text is read), and the note says what was left out.
        let expected = if memory.summary.status == TextStatus::Complete {
            ExtractionStatus::Ok
        } else {
            ExtractionStatus::Partial
        };
        assert_eq!(extraction.summary.status, expected, "{name}");
        assert_eq!(
            extraction.summary.note.is_some(),
            expected == ExtractionStatus::Partial,
            "{name}"
        );
        assert!(extraction.summary.usable_text, "{name}");
        assert_eq!(extraction.excerpt, [memory.text], "{name}: the whole text");
        assert!(!extraction.evidence.is_empty(), "{name}: pieces to cite");
    }

    // A broken workbook goes to the Excel reader and keeps its reason.
    let broken = read("book.xlsx", b"PK not a workbook");
    assert!(!broken.summary.usable_text);
    assert_ne!(
        broken.summary.note.as_deref(),
        Some("Only .txt and .md files are read.")
    );

    // Other names are not Office files.
    for name in [
        "notes.doc",
        "deck.ppt",
        "book.xls",
        ".docx",
        "notes.docx.zip",
    ] {
        let other = read(name, word);
        assert_eq!(
            other.summary.note.as_deref(),
            Some("Only .txt and .md files are read."),
            "{name}"
        );
    }
}

#[test]
fn a_word_document_is_suggested_like_a_text_file() {
    let s = setup(with_text);
    let word: &[u8] = include_bytes!("../tests/fixtures/docx/word-basic-v1.docx");
    fs::write(s.f.path("notes.docx"), word).unwrap();
    let id =
        s.f.service()
            .start_operation(&s.f.root_id, &s.f.root_id)
            .unwrap()
            .operation_id;
    s.f.service()
        .run_operation_with(&id, &s.f.path("notes.docx"), &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        });
    let view = s.f.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::ReadyForAnalysis, "{view:?}");
    assert!(view.file.unwrap().extraction.usable_text);

    s.analyze(&id);
    let view = s.f.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::RecommendationReady, "{view:?}");
    let data = s.sent_data();
    assert_eq!(data["file_name"], "notes.docx");
    let first_line = include_str!("../tests/fixtures/docx/word-basic-v1.txt")
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap();
    assert!(data.to_string().contains(first_line), "{first_line}");
}

#[test]
fn an_excel_sheet_s_name_is_not_sent_and_a_partial_reading_says_what_was_left_out() {
    // A synthetic workbook from F's experiment (its sheets have names).
    let book: &[u8] =
        include_bytes!("../../../experiments/xlsx-reader/excel-save/f-excel-seed.xlsx");
    let memory = crate::reader::read_text("seed.xlsx", book);
    assert!(
        memory.text.contains(" · name "),
        "the reading itself has the sheets' names"
    );
    let f = Fixture::new();
    let extraction = f
        .service()
        .organize_extraction("seed.xlsx", book, OcrUse::Allowed);

    assert!(extraction.summary.usable_text, "{extraction:?}");
    let sent = extraction.excerpt.concat();
    assert!(sent.contains("⟦sheet 1 · id "), "{sent}");
    assert!(!sent.contains(" · name "), "no sheet name: {sent}");
    for piece in &extraction.evidence {
        assert!(!piece.text.contains(" · name "), "{}", piece.text);
    }
    if extraction.summary.status == ExtractionStatus::Partial {
        let note = extraction.summary.note.unwrap();
        assert!(
            note.starts_with("Traeva read only part of this file's text."),
            "{note}"
        );
        assert!(
            !note.contains("left_out"),
            "no field names for people: {note}"
        );
    }
}

#[test]
fn sheet_names_are_taken_out_of_excel_readings_only() {
    let text = "⟦sheet 1 · id 1 · name \"Grades – Jane Doe\"⟧\n⟦cell A1 · literal · text⟧\nScore";
    assert_eq!(
        crate::reader::without_sheet_names(crate::reader::XLSX_MEMORY_READER, text),
        "⟦sheet 1 · id 1⟧\n⟦cell A1 · literal · text⟧\nScore"
    );
    assert_eq!(
        crate::reader::without_sheet_names(crate::reader::PPTX_MEMORY_READER, text),
        text
    );
}
