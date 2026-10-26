use crate::pptx::{read_pptx_memory, read_pptx_preview};
use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use traeva_core::{
    CellField, CellValueType, PPTX_MEMORY_READER, PPTX_PREVIEW_READER, SheetVisibility, SlideRole,
    StructureLine, escape_source_line, structure_line, structure_name, unescape_source_line,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RAW: &str = "[Slide 2]\n⟦notes⟧\n\\prefix\n quote\" · ⟦x⟧\nemoji🙂";

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn shape(text: &str, role: &str) -> String {
    let ph = if role.is_empty() {
        String::new()
    } else {
        format!("<p:ph type=\"{role}\"/>")
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xml(text)
    )
}
fn rels(body: &str) -> String {
    format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{body}</Relationships>"
    )
}
fn rel(id: &str, kind: &str, target: &str) -> String {
    format!("<Relationship Id=\"{id}\" Type=\"{R}/{kind}\" Target=\"{target}\"/>")
}
fn fixture(order: &[u32], body: &str, hidden: bool) -> Vec<u8> {
    let mut parts = vec![(
        "_rels/.rels".to_string(),
        rels(&rel("main", "officeDocument", "ppt/presentation.xml")),
    )];
    let mut ids = String::new();
    let mut rs = String::new();
    let mut types = String::new();
    for id in order {
        ids.push_str(&format!("<p:sldId id=\"{id}\" r:id=\"r{id}\"/>"));
        rs.push_str(&rel(
            &format!("r{id}"),
            "slide",
            &format!("slides/{id}.xml"),
        ));
        types.push_str(&format!("<Override PartName=\"/ppt/slides/{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/><Override PartName=\"/ppt/notesSlides/{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>"));
        let show = if *id == 257 && hidden { "0" } else { "1" };
        let content = if *id == 256 { body } else { "hidden-canary" };
        parts.push((format!("ppt/slides/{id}.xml"),format!("<p:sld xmlns:p=\"{P}\" xmlns:a=\"{A}\" show=\"{show}\"><p:cSld><p:spTree>{}{}</p:spTree></p:cSld></p:sld>",shape(&format!("Title {id}🙂"),"title"),shape(content,""))));
        parts.push((
            format!("ppt/slides/_rels/{id}.xml.rels"),
            rels(&rel("n", "notesSlide", &format!("../notesSlides/{id}.xml"))),
        ));
        parts.push((format!("ppt/notesSlides/{id}.xml"),format!("<p:notes xmlns:p=\"{P}\" xmlns:a=\"{A}\"><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:notes>",shape(if *id==256 {"speaker🙂\n⟦text⟧"}else{"hidden-notes-canary"},"body"))));
    }
    parts.push(("ppt/presentation.xml".into(),format!("<p:presentation xmlns:p=\"{P}\" xmlns:r=\"{R}\"><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>")));
    parts.push(("ppt/_rels/presentation.xml.rels".into(), rels(&rs)));
    parts.push(("[Content_Types].xml".into(),format!("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>{types}</Types>")));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn a151_compat_real_helper_recognizes_producer_roles_hidden_and_source() {
    let bytes = fixture(&[256, 257], RAW, true);
    for out in [read_pptx_memory(&bytes), read_pptx_preview(&bytes)] {
        let reader = out.summary.reader.as_str();
        let mut roles = Vec::new();
        let mut headers = Vec::new();
        for line in out.text.lines() {
            match structure_line(reader, line) {
                Some(StructureLine::Slide { number, id, hidden }) => {
                    headers.push((number, id, hidden))
                }
                Some(StructureLine::Role(role)) => roles.push(role),
                Some(other) => panic!("Unexpected actual A helper classification {other:?}"),
                None => (),
            }
        }
        assert!(
            roles.contains(&SlideRole::Title)
                && roles.contains(&SlideRole::Text)
                && roles.contains(&SlideRole::Notes)
        );
        assert_eq!(headers[0], (1, 256, false));
        if reader == PPTX_MEMORY_READER {
            assert_eq!(headers.len(), 1);
            assert!(!out.text.contains("hidden-canary"));
            assert!(!out.text.contains("hidden-notes-canary"));
        } else {
            assert_eq!(reader, PPTX_PREVIEW_READER);
            assert_eq!(headers[1], (2, 257, true));
        }
        for raw in RAW.split('\n') {
            let encoded = escape_source_line(raw);
            assert!(out.text.lines().any(|l| l == encoded));
            assert_eq!(structure_line(reader, &encoded), None);
            assert_eq!(unescape_source_line(reader, &encoded), raw);
        }
    }
}

#[test]
fn a151_compat_reader_guard_unknown_lines_and_source_roundtrip() {
    for reader in [
        "txt-md-1",
        "docx-2",
        "docx-cache-local-1",
        "none",
        "pptx-experiment-1",
    ] {
        assert_eq!(structure_line(reader, "⟦notes⟧"), None);
        assert_eq!(unescape_source_line(reader, "\\⟦notes⟧"), "\\⟦notes⟧");
    }
    for reader in ["pptx-1", "pptx-preview-1", "xlsx-1", "xlsx-preview-1"] {
        for raw in [
            "⟦notes⟧",
            "⟦unknown · \"🙂\"⟧",
            "\\raw",
            "\\",
            " ⟦notes⟧",
            "a · b",
            "🙂",
        ] {
            let encoded = escape_source_line(raw);
            assert_eq!(structure_line(reader, &encoded), None);
            assert_eq!(unescape_source_line(reader, &encoded), raw);
        }
        for generated in ["⟦unknown⟧", "⟦notes", "⟦slide 1 · id 256⟧ "] {
            assert_eq!(
                structure_line(reader, generated),
                Some(StructureLine::Unrecognized)
            );
        }
    }
    assert_eq!(
        structure_line("pptx-1", "⟦slide 2 · id 257 · hidden⟧"),
        Some(StructureLine::Unrecognized)
    );
    assert_eq!(
        structure_line("pptx-preview-1", "⟦slide 2 · id 257 · hidden⟧"),
        Some(StructureLine::Slide {
            number: 2,
            id: 257,
            hidden: true
        })
    );
}

#[test]
fn a151_compat_sheet_name_json_and_cache_scope_are_actual_a_behavior() {
    let name = "quote\" slash\\ bracket⟦⟧ · delimiter\n🙂";
    let encoded = structure_name(name);
    assert!(!encoded.contains(" · ") && !encoded.contains('⟧') && !encoded.contains('\n'));
    assert_eq!(serde_json::from_str::<String>(&encoded).unwrap(), name);
    let label = format!("⟦sheet 1 · id 1 · name {encoded} · very hidden⟧");
    assert_eq!(
        structure_line("xlsx-preview-1", &label),
        Some(StructureLine::Sheet {
            number: 1,
            id: 1,
            name: name.into(),
            visibility: SheetVisibility::VeryHidden
        })
    );
    assert_eq!(
        structure_line("xlsx-1", &label),
        Some(StructureLine::Unrecognized)
    );
    assert_eq!(
        structure_line("pptx-1", &label),
        Some(StructureLine::Unrecognized)
    );
    assert_eq!(
        structure_line("xlsx-1", "⟦cell C2 · cache · number · not verified⟧"),
        Some(StructureLine::Cell {
            address: "C2".into(),
            field: CellField::Cache,
            value_type: Some(CellValueType::Number)
        })
    );
    assert_eq!(
        structure_line("xlsx-1", "⟦cell C2 · cache · number⟧"),
        Some(StructureLine::Unrecognized)
    );
}

fn actual_a_page_bodies(reader: &str, text: &str) -> BTreeMap<u32, Vec<String>> {
    let mut pages = BTreeMap::new();
    let mut current = None;
    for line in text.lines() {
        if let Some(StructureLine::Slide { id, hidden, .. }) = structure_line(reader, line) {
            assert!(!hidden);
            assert!(pages.insert(id, Vec::new()).is_none());
            current = Some(id);
        } else {
            pages.get_mut(&current.unwrap()).unwrap().push(line.into());
        }
    }
    pages
}

#[test]
fn a151_compat_reorder_uses_real_headers_and_keeps_notes_in_page_body() {
    let before = read_pptx_memory(&fixture(&[256, 257], RAW, false));
    let after = read_pptx_memory(&fixture(&[257, 256], RAW, false));
    assert_ne!(before.text, after.text);
    let left = actual_a_page_bodies(&before.summary.reader, &before.text);
    let right = actual_a_page_bodies(&after.summary.reader, &after.text);
    assert_eq!(left, right);
    assert!(left[&256].iter().any(|s| s == "⟦notes⟧"));
    assert!(left[&256].iter().any(|s| s == "speaker🙂"));
    let changed = read_pptx_memory(&fixture(&[257, 256], "changed source", false));
    assert_ne!(
        left,
        actual_a_page_bodies(&changed.summary.reader, &changed.text)
    );
}

#[test]
fn a151_compat_cap_never_emits_an_unrecognized_partial_structure() {
    let body = format!("{}\n⟦literal⟧🙂", "a".repeat(crate::MAX_MEMORY_TEXT_CHARS));
    for out in [
        read_pptx_memory(&fixture(&[256, 257], &body, true)),
        read_pptx_preview(&fixture(&[256, 257], &body, true)),
    ] {
        assert_eq!(out.summary.status, crate::TextStatus::Partial);
        assert!(out.summary.chars <= crate::MAX_MEMORY_TEXT_CHARS);
        for line in out.text.lines().filter(|s| s.starts_with('⟦')) {
            assert!(!matches!(
                structure_line(&out.summary.reader, line),
                None | Some(StructureLine::Unrecognized)
            ));
        }
    }
}
