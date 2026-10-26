use super::*;
use crate::contracts::{AppError, ErrorCode, ErrorStage};
use crate::memory_contracts::SnapshotRef;

#[test]
fn a_text_file_is_read_whole_with_lf_line_breaks() {
    let read = read_text(
        "Notes.md",
        b"\xEF\xBB\xBFline one\r\nline two\rline three\n",
    );
    assert_eq!(read.text, "line one\nline two\nline three\n");
    assert_eq!(read.summary.reader, PLAIN_TEXT_READER);
    assert_eq!(read.summary.status, TextStatus::Complete);
    assert_eq!(read.summary.chars, 29);
    assert_eq!(read.summary.covered, ["the whole text"]);
    assert!(read.summary.left_out.is_empty());
    assert_eq!(read.summary.note, None);
}

#[test]
fn memory_and_organize_decode_the_same_bytes_to_the_same_text() {
    let bytes = "Café\r\n\r\nzweite Zeile — 第二行\n".as_bytes();
    let memory = read_text("a.txt", bytes);
    let organize = observe::extract_text("a.txt", bytes);
    assert_eq!(organize.excerpt, std::slice::from_ref(&memory.text));
    assert_eq!(organize.summary.total_chars, memory.summary.chars);
}

#[test]
fn blank_text_is_complete_and_empty_text_too() {
    let blank = read_text("blank.txt", b"  \n\t\n");
    assert_eq!(blank.summary.status, TextStatus::Complete);
    assert_eq!(blank.text, "  \n\t\n");
    let empty = read_text("empty.md", b"");
    assert_eq!(empty.summary.status, TextStatus::Complete);
    assert_eq!(empty.summary.chars, 0);
    assert!(empty.text.is_empty());
}

#[test]
fn a_text_over_the_limit_is_partial_and_says_what_was_left_out() {
    // Three-byte characters, so a cut by bytes would split one.
    let text = "語".repeat(MAX_MEMORY_TEXT_CHARS + 5);
    let read = read_text("long.txt", text.as_bytes());
    assert_eq!(read.summary.status, TextStatus::Partial);
    assert_eq!(read.summary.chars, MAX_MEMORY_TEXT_CHARS);
    assert_eq!(read.text.chars().count(), MAX_MEMORY_TEXT_CHARS);
    assert!(text.starts_with(&read.text));
    assert_eq!(
        read.summary.covered,
        [format!("the first {MAX_MEMORY_TEXT_CHARS} characters")]
    );
    assert_eq!(read.summary.left_out, ["the last 5 characters"]);
    assert!(read.summary.note.is_some());
}

#[test]
fn a_text_exactly_at_the_limit_is_complete() {
    let text = "a".repeat(MAX_MEMORY_TEXT_CHARS);
    let read = read_text("full.txt", text.as_bytes());
    assert_eq!(read.summary.status, TextStatus::Complete);
    assert_eq!(read.text, text);
}

#[test]
fn bytes_that_are_not_plain_text_give_no_text_and_say_why() {
    let cases: [(&str, &[u8], TextStatus); 4] = [
        ("a.txt", b"\xFF\xFEh\x00i\x00", TextStatus::Unsupported),
        ("a.txt", b"one\x00two", TextStatus::Unsupported),
        ("a.md", b"caf\xE9", TextStatus::Failed),
        ("a.pdf", b"%PDF-1.7", TextStatus::Unsupported),
    ];
    for (name, bytes, status) in cases {
        let read = read_text(name, bytes);
        assert_eq!(read.summary.status, status, "{name} {bytes:?}");
        assert!(read.text.is_empty());
        assert_eq!(read.summary.chars, 0);
        assert!(read.summary.covered.is_empty());
        assert!(read.summary.left_out.is_empty());
        assert!(read.summary.note.is_some());
    }
    assert_eq!(read_text("a.pdf", b"x").summary.reader, NO_READER);
    assert_eq!(read_text("a.md", b"\xE9").summary.reader, PLAIN_TEXT_READER);
}

#[test]
fn debug_output_never_shows_the_text() {
    let read = read_text("x.txt", b"private words");
    let shown = format!("{read:?}");
    assert!(!shown.contains("private"));
    assert!(shown.contains("<13 bytes>"));
}

#[test]
fn the_same_bytes_always_give_the_same_extraction() {
    let bytes = b"draft\n\nconclusion\n";
    assert_eq!(read_text("x.md", bytes), read_text("x.md", bytes));
}

// The JSON field names below are part of the shared contract (#65): other
// lanes and the window rely on them. Change them only through Claude-A.

#[test]
fn a_text_summary_is_sent_without_text() {
    let read = read_text("x.txt", b"secret body");
    let json = serde_json::to_value(&read.summary).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "reader": "txt-md-1",
            "status": "complete",
            "chars": 11,
            "covered": ["the whole text"],
            "left_out": [],
            "note": null,
        })
    );
    assert!(!json.to_string().contains("secret"));
}

#[test]
fn a_snapshot_ref_has_these_fields() {
    let snapshot = SnapshotRef {
        snapshot_id: "7d0c3b0e-4b1f-4d55-9a57-0d2c8f9d1a11".to_string(),
        file_id: "0f5b7b0c-58a4-4b0a-8f41-2a6a2c1f3e22".to_string(),
        sequence: 2,
        file_revision: 3,
        root_id: "3c9e4f2a-1b6d-4c8e-9f0a-5d7b2e1c4a33".to_string(),
        relative_path: "Essays/draft.md".to_string(),
        sha256: "ab".repeat(32),
        byte_length: 1234,
        captured_at_ms: 1_791_500_000_000,
    };
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        serde_json::json!({
            "snapshot_id": "7d0c3b0e-4b1f-4d55-9a57-0d2c8f9d1a11",
            "file_id": "0f5b7b0c-58a4-4b0a-8f41-2a6a2c1f3e22",
            "sequence": 2,
            "file_revision": 3,
            "root_id": "3c9e4f2a-1b6d-4c8e-9f0a-5d7b2e1c4a33",
            "relative_path": "Essays/draft.md",
            "sha256": "ab".repeat(32),
            "byte_length": 1234,
            "captured_at_ms": 1_791_500_000_000_i64,
        })
    );
}

#[test]
fn reading_a_saved_version_fails_with_these_codes() {
    for (code, text) in [
        (ErrorCode::UnknownSnapshot, "unknown_snapshot"),
        (ErrorCode::SnapshotUnavailable, "snapshot_unavailable"),
        (ErrorCode::SnapshotCorrupt, "snapshot_corrupt"),
    ] {
        let error = AppError::snapshot(code, "message", Some("file"));
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], text);
        assert_eq!(json["stage"], "memory");
        assert_eq!(error.stage, ErrorStage::Memory);
        assert!(!error.retryable);
        assert!(error.allowed_next.is_empty());
        assert_eq!(error.file_id.as_deref(), Some("file"));
    }
}

const WORD_V1: &[u8] = include_bytes!("../tests/fixtures/docx/word-basic-v1.docx");

#[test]
fn a_word_document_goes_to_the_docx_reader_in_any_letter_case() {
    let direct = crate::docx::read_docx(WORD_V1);
    assert_eq!(direct.summary.reader, crate::DOCX_READER);
    for name in ["report.docx", "Report.DOCX", "a.b.Docx"] {
        assert_eq!(read_text(name, WORD_V1), direct, "{name}");
    }
    let expected = include_str!("../tests/fixtures/docx/word-basic-v1.txt");
    assert_eq!(direct.summary.status, TextStatus::Complete);
    assert_eq!(direct.text, expected.trim_end_matches('\n'));
}

#[test]
fn only_a_docx_name_reaches_the_docx_reader() {
    // No name before the dot, another Office type, a template: no reader.
    for name in [
        ".docx",
        "report.docm",
        "report.dotx",
        "report.docx.zip",
        "docx",
    ] {
        let read = read_text(name, WORD_V1);
        assert_eq!(read.summary.reader, NO_READER, "{name}");
        assert_eq!(read.summary.status, TextStatus::Unsupported, "{name}");
    }
}

#[test]
fn bytes_that_are_not_a_word_document_fail_without_showing_them() {
    for bytes in [&b""[..], b"plain text, not a zip", b"PK\x03\x04 broken"] {
        let read = read_text("report.docx", bytes);
        assert_eq!(read.summary.reader, crate::DOCX_READER);
        assert!(
            matches!(
                read.summary.status,
                TextStatus::Failed | TextStatus::Unsupported
            ),
            "{:?}",
            read.summary
        );
        assert!(read.text.is_empty());
        let note = read.summary.note.clone().unwrap_or_default();
        assert!(
            !note.contains("plain text"),
            "the note never shows the bytes"
        );
    }
}

// Structure lines (contract §4.4, v23).

#[test]
fn slide_lines_are_read_for_the_pptx_readers_only() {
    use super::{SlideRole::*, StructureLine::*};
    for reader in [PPTX_MEMORY_READER, PPTX_PREVIEW_READER] {
        assert_eq!(
            structure_line(reader, "⟦slide 3 · id 257⟧"),
            Some(Slide {
                number: 3,
                id: 257,
                hidden: false
            })
        );
        assert_eq!(structure_line(reader, "⟦title⟧"), Some(Role(Title)));
        assert_eq!(structure_line(reader, "⟦text⟧"), Some(Role(Text)));
        assert_eq!(structure_line(reader, "⟦notes⟧"), Some(Role(Notes)));
        assert_eq!(structure_line(reader, "Q3 results"), None);
        assert_eq!(structure_line(reader, "\\⟦notes⟧"), None);
    }
    // Hidden slides are only in the preview.
    assert_eq!(
        structure_line(PPTX_PREVIEW_READER, "⟦slide 4 · id 260 · hidden⟧"),
        Some(Slide {
            number: 4,
            id: 260,
            hidden: true
        })
    );
    assert_eq!(
        structure_line(PPTX_MEMORY_READER, "⟦slide 4 · id 260 · hidden⟧"),
        Some(Unrecognized)
    );
    // In other readers' output a ⟦…⟧ line is the user's own text.
    for reader in [PLAIN_TEXT_READER, crate::DOCX_READER, NO_READER] {
        assert_eq!(structure_line(reader, "⟦notes⟧"), None);
        assert!(!has_structure_lines(reader));
    }
}

#[test]
fn sheet_and_cell_lines_are_read_for_the_xlsx_readers_only() {
    use super::{CellField::*, CellValueType::*, SheetVisibility::*, StructureLine::*};
    let reader = XLSX_MEMORY_READER;
    assert_eq!(
        structure_line(reader, "⟦sheet 1 · id 1 · name \"F Data\"⟧"),
        Some(Sheet {
            number: 1,
            id: 1,
            name: "F Data".into(),
            visibility: Visible
        })
    );
    let cell = |address: &str, field, value_type| {
        Some(Cell {
            address: address.into(),
            field,
            value_type,
        })
    };
    assert_eq!(
        structure_line(reader, "⟦cell C2 · literal · number⟧"),
        cell("C2", Literal, Some(Number))
    );
    assert_eq!(
        structure_line(reader, "⟦cell XFD1048576 · formula⟧"),
        cell("XFD1048576", Formula, None)
    );
    assert_eq!(
        structure_line(reader, "⟦cell C2 · cache · number · not verified⟧"),
        cell("C2", Cache, Some(Number))
    );
    for kind in ["text", "boolean", "error", "date"] {
        assert!(matches!(
            structure_line(reader, &format!("⟦cell A1 · literal · {kind}⟧")),
            Some(Cell { .. })
        ));
    }
    // An empty cache, and a formula with no cache in the file.
    assert_eq!(
        structure_line(reader, "⟦cell C2 · cache · blank · not verified⟧"),
        cell("C2", Cache, Some(Blank))
    );
    assert_eq!(
        structure_line(reader, "⟦cell C2 · cache · missing · not verified⟧"),
        cell("C2", Cache, None)
    );
    assert_eq!(
        structure_line(
            XLSX_PREVIEW_READER,
            "⟦sheet 3 · id 4 · name \"Old\" · very hidden⟧"
        ),
        Some(Sheet {
            number: 3,
            id: 4,
            name: "Old".into(),
            visibility: VeryHidden
        })
    );
    // Not one of the forms: still the reader's line, never the document's.
    for odd in [
        "⟦sheet 3 · id 4 · name \"Old\" · hidden⟧", // hidden is preview only
        "⟦cell C2 · cache · number⟧",               // a cache is always "not verified"
        "⟦cell C2 · formula · number⟧",
        "⟦cell C2 · literal · blank⟧", // a cell without a value has no literal line
        "⟦cell C2 · literal · missing⟧",
        "⟦cell C2 · cache · missing⟧",
        "⟦cell c2 · formula⟧",
        "⟦cell C02 · formula⟧",
        "⟦cell ABCD1 · formula⟧",
        "⟦sheet 0 · id 1 · name \"x\"⟧",
        "⟦sheet 1 · id 1 · name x⟧",
        "⟦slide 1 · id 256⟧",
        "⟦cell C2 · formula",
        "⟦⟧",
    ] {
        assert_eq!(structure_line(reader, odd), Some(Unrecognized), "{odd}");
    }
    // The whole line, as it is: a trailing space is not trimmed.
    assert_eq!(
        structure_line(reader, "⟦cell C2 · formula⟧ "),
        Some(Unrecognized)
    );
    assert_eq!(structure_line(reader, " ⟦cell C2 · formula⟧"), None);
}

#[test]
fn a_sheet_name_with_separators_and_brackets_survives_the_line() {
    let name = "Q3 · \"final\" ⟦draft⟧ \\ tab\there";
    let encoded = structure_name(name);
    assert!(!encoded.contains(" · ") && !encoded.contains('⟧') && !encoded.contains('⟦'));
    let line = format!("⟦sheet 2 · id 7 · name {encoded}⟧");
    assert_eq!(
        structure_line(XLSX_MEMORY_READER, &line),
        Some(super::StructureLine::Sheet {
            number: 2,
            id: 7,
            name: name.into(),
            visibility: super::SheetVisibility::Visible,
        })
    );
    // Characters outside the BMP still round-trip.
    let emoji = structure_name("🦀 data");
    assert_eq!(serde_json::from_str::<String>(&emoji).unwrap(), "🦀 data");
}

#[test]
fn source_lines_that_look_like_structure_are_escaped_and_shown_as_written() {
    for (source, written) in [
        ("⟦slide 1 · id 256⟧", "\\⟦slide 1 · id 256⟧"),
        ("\\already", "\\\\already"),
        ("plain ⟦not at the start⟧", "plain ⟦not at the start⟧"),
        ("", ""),
    ] {
        assert_eq!(escape_source_line(source), written);
        assert_eq!(structure_line(PPTX_MEMORY_READER, written), None);
        assert_eq!(unescape_source_line(PPTX_MEMORY_READER, written), source);
    }
    // Other readers do not escape, so nothing is taken off.
    assert_eq!(unescape_source_line(PLAIN_TEXT_READER, "\\⟦x⟧"), "\\⟦x⟧");
    // A lone backslash that is not an escape stays.
    assert_eq!(unescape_source_line(PPTX_MEMORY_READER, "\\n"), "\\n");
}

// PPTX wiring (#28, contract §4.4 v23).

const DECK: &[u8] = include_bytes!("../tests/fixtures/pptx/basic-source.pptx");

#[test]
fn a_powerpoint_goes_to_its_memory_projection_in_any_letter_case() {
    let direct = crate::pptx::read_pptx_memory(DECK);
    assert_eq!(direct.summary.reader, PPTX_MEMORY_READER);
    assert!(!direct.text.is_empty());
    for name in ["deck.pptx", "Deck.PPTX", "a.b.Pptx"] {
        assert_eq!(read_text(name, DECK), direct, "{name}");
    }
    for name in ["deck.pptm", "deck.ppsx", "deck.pptx.zip", ".pptx", "pptx"] {
        assert_ne!(
            read_text(name, DECK).summary.reader,
            PPTX_MEMORY_READER,
            "{name}"
        );
    }
    // One name for each reader: the module's and the contract's agree.
    assert_eq!(crate::pptx::PPTX_MEMORY_READER, PPTX_MEMORY_READER);
    assert_eq!(crate::pptx::PPTX_PREVIEW_READER, PPTX_PREVIEW_READER);
    // Every structure line the projection writes is one structure_line reads.
    for line in direct.text.lines().filter(|line| line.starts_with('⟦')) {
        assert!(
            !matches!(
                structure_line(PPTX_MEMORY_READER, line),
                None | Some(StructureLine::Unrecognized)
            ),
            "{line}"
        );
    }
}

// XLSX wiring (#28, contract §4.4 v23). F's own tests cover what the
// projections say; here only that core sends a workbook name to them.

#[test]
fn an_excel_workbook_goes_to_its_memory_projection_in_any_letter_case() {
    let bytes: &[u8] = b"PK not a workbook";
    let direct = crate::xlsx::read_xlsx_memory(bytes);
    assert_eq!(direct.summary.reader, XLSX_MEMORY_READER);
    assert!(direct.text.is_empty());
    for name in ["book.xlsx", "Book.XLSX", "a.b.Xlsx"] {
        assert_eq!(read_text(name, bytes), direct, "{name}");
    }
    for name in ["book.xlsm", "book.xls", "book.xlsx.zip", ".xlsx", "xlsx"] {
        assert_ne!(
            read_text(name, bytes).summary.reader,
            XLSX_MEMORY_READER,
            "{name}"
        );
    }
    assert_eq!(crate::xlsx::XLSX_READER, XLSX_MEMORY_READER);
    assert_eq!(crate::xlsx::XLSX_PREVIEW_READER, XLSX_PREVIEW_READER);
}
