// Compiled against A's real reader.rs at the pinned #151 dependency, not a replica.
use traeva_core::{
    CellField, CellValueType, SheetVisibility, StructureLine, escape_source_line, structure_line,
    structure_name, unescape_source_line,
};
use traeva_xlsx_projection_candidate::{XLSX_PREVIEW_READER, XLSX_READER};

#[test]
fn four_reader_gate_never_reclassifies_txt_md_or_docx() {
    for reader in ["pptx-1", "pptx-preview-1", "xlsx-1", "xlsx-preview-1"] {
        assert!(structure_line(reader, "⟦unknown future line⟧").is_some());
        assert!(structure_line(reader, "⟦missing closing bracket").is_some());
    }
    for reader in ["txt-md-1", "docx-2", "docx-cache-local-1", "none", "xlsx-2"] {
        assert_eq!(structure_line(reader, "⟦cell A1 · literal · text⟧"), None);
        assert_eq!(
            unescape_source_line(reader, "\\⟦user text⟧"),
            "\\⟦user text⟧"
        );
    }
}

#[test]
fn actual_name_helper_roundtrips_every_delimiter_and_non_bmp() {
    let name = "Q · \"quoted\" ⟦bracket⟧ \\ \r\n\t中文🙂";
    let encoded = structure_name(name);
    assert!(!encoded.contains(" · "));
    assert!(!encoded.contains('⟦'));
    assert!(!encoded.contains('⟧'));
    assert!(encoded.contains("\\u0022"));
    assert!(encoded.contains("\\u005c"));
    let line = format!("⟦sheet 2 · id 7 · name {encoded}⟧");
    assert_eq!(
        structure_line(XLSX_READER, &line),
        Some(StructureLine::Sheet {
            number: 2,
            id: 7,
            name: name.into(),
            visibility: SheetVisibility::Visible
        })
    );
    assert_eq!(structure_line(XLSX_READER, &format!(" {line}")), None);
    assert_eq!(
        structure_line(XLSX_READER, &format!("{line} ")),
        Some(StructureLine::Unrecognized)
    );
}

#[test]
fn escaped_source_is_preserved_and_never_becomes_known_or_unknown_structure() {
    for source in [
        "⟦cell C2 · cache · number · not verified⟧",
        "⟦unknown⟧",
        "\\already",
        "[old bracket]",
        "中文🙂",
        " ⟦spaced⟧",
        "",
    ] {
        let written = escape_source_line(source);
        for reader in [XLSX_READER, XLSX_PREVIEW_READER] {
            assert_eq!(structure_line(reader, &written), None);
            assert_eq!(unescape_source_line(reader, &written), source);
        }
    }
    let written = escape_source_line("⟦🙂⟧");
    assert_eq!(written.chars().count(), 4); // QuoteAt counts the escape as an output scalar.
}

#[test]
fn preview_visibility_and_cache_qualifier_have_actual_typed_recognition() {
    for (suffix, visibility) in [
        ("hidden", SheetVisibility::Hidden),
        ("very hidden", SheetVisibility::VeryHidden),
    ] {
        let line = format!("⟦sheet 2 · id 2 · name \"Old\" · {suffix}⟧");
        assert_eq!(
            structure_line(XLSX_PREVIEW_READER, &line),
            Some(StructureLine::Sheet {
                number: 2,
                id: 2,
                name: "Old".into(),
                visibility
            })
        );
        assert_eq!(
            structure_line(XLSX_READER, &line),
            Some(StructureLine::Unrecognized)
        );
    }
    assert_eq!(
        structure_line(XLSX_READER, "⟦cell C2 · cache · number · not verified⟧"),
        Some(StructureLine::Cell {
            address: "C2".into(),
            field: CellField::Cache,
            value_type: Some(CellValueType::Number)
        })
    );
    assert_eq!(
        structure_line(XLSX_READER, "⟦cell C2 · cache · number⟧"),
        Some(StructureLine::Unrecognized)
    );
}

#[test]
fn candidate_sheet_label_is_recognized_by_the_real_helper() {
    // A decided that part remains internal; the final label must be a known Sheet.
    let line = "⟦sheet 1 · id 1 · name \"Data\"⟧";
    assert!(matches!(
        structure_line(XLSX_READER, line),
        Some(StructureLine::Sheet { .. })
    ));
}
