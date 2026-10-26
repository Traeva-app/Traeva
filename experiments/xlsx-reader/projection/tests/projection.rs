use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use traeva_core::{MAX_MEMORY_TEXT_CHARS, SnapshotRef, TextExtraction, TextStatus};
use traeva_xlsx_projection_candidate::{
    ProjectionMode, XLSX_PREVIEW_READER, XLSX_READER, project_verified_snapshot, read_xlsx_memory,
    read_xlsx_preview,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const T: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\r', "&#13;")
}

fn text_cell(address: &str, text: &str) -> String {
    format!(
        "<c r=\"{address}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
        xml(text)
    )
}

// Tuple order is the declared workbook order; IDs/parts remain stable when reordered.
fn package(sheets: &[(u32, &str, &str, String)], extra: &[(&str, &str)]) -> Vec<u8> {
    let mut types = format!(
        r#"<Types xmlns="{T}"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#
    );
    let mut workbook = format!(r#"<workbook xmlns="{NS}" xmlns:r="{R}"><sheets>"#);
    let mut rels = format!(r#"<Relationships xmlns="{P}">"#);
    let mut parts = Vec::new();
    for (id, name, state, content) in sheets {
        let part = format!("xl/worksheets/sheet{id}.xml");
        types += &format!(
            r#"<Override PartName="/{part}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        );
        workbook += &format!(
            r#"<sheet name="{}" sheetId="{id}" state="{state}" r:id="s{id}"/>"#,
            xml(name)
        );
        rels += &format!(
            r#"<Relationship Id="s{id}" Type="{R}/worksheet" Target="worksheets/sheet{id}.xml"/>"#
        );
        parts.push((
            part,
            format!(r#"<worksheet xmlns="{NS}">{content}</worksheet>"#),
        ));
    }
    types += "</Types>";
    workbook += "</sheets></workbook>";
    rels += "</Relationships>";
    parts.extend([
        ("[Content_Types].xml".into(), types), ("xl/workbook.xml".into(), workbook),
        ("xl/_rels/workbook.xml.rels".into(), rels),
        ("_rels/.rels".into(), format!(r#"<Relationships xmlns="{P}"><Relationship Id="main" Type="{R}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)),
    ]);
    parts.extend(extra.iter().map(|(p, s)| ((*p).into(), (*s).into())));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, body) in parts {
        zip.start_file(
            path,
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    if let Ok(directory) = std::env::var("F_XLSX_PROJECTION_EVIDENCE") {
        let directory = std::path::Path::new(&directory);
        let input_hash = hex(&bytes);
        std::fs::write(directory.join(format!("{input_hash}.xlsx")), &bytes).unwrap();
        let memory = read_xlsx_memory(&bytes);
        let preview = read_xlsx_preview(&bytes);
        let manifest = serde_json::json!({
            "input_sha256": input_hash, "input_bytes": bytes.len(),
            "memory": { "text_sha256": hex(memory.text.as_bytes()), "summary": memory.summary },
            "preview": { "text_sha256": hex(preview.text.as_bytes()), "summary": preview.summary }
        });
        for (mode, text) in [("memory", memory.text), ("preview", preview.text)] {
            std::fs::write(directory.join(format!("{input_hash}-{mode}.txt")), text).unwrap();
        }
        std::fs::write(
            directory.join(format!("{input_hash}.json")),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
    bytes
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn visible(body: &str) -> Vec<u8> {
    package(&[(1, "Data", "visible", body.into())], &[])
}

fn invariants(e: &TextExtraction) {
    for line in e.text.lines().filter(|s| s.starts_with('⟦')) {
        assert!(
            !matches!(
                traeva_core::structure_line(&e.summary.reader, line),
                None | Some(traeva_core::StructureLine::Unrecognized)
            ),
            "{line}"
        );
    }
    assert_eq!(e.summary.chars, e.text.chars().count());
    assert!(e.summary.chars <= MAX_MEMORY_TEXT_CHARS);
    assert!(!e.text.contains('\r'));
    match e.summary.status {
        TextStatus::Complete => {
            assert!(!e.summary.covered.is_empty());
            assert!(e.summary.left_out.is_empty());
            assert!(e.summary.note.is_none());
        }
        TextStatus::Partial => {
            assert!(!e.text.is_empty());
            assert!(!e.summary.covered.is_empty());
            assert!(!e.summary.left_out.is_empty());
            assert!(e.summary.note.is_some());
        }
        TextStatus::Failed | TextStatus::Unsupported => {
            assert!(e.text.is_empty());
            assert_eq!(e.summary.chars, 0);
            assert!(e.summary.covered.is_empty());
            assert!(e.summary.left_out.is_empty());
            assert!(e.summary.note.is_some());
        }
    }
}

#[test]
fn literal_formula_and_cache_have_exact_separate_lines() {
    let bytes = visible(
        r#"<sheetData><row r="2"><c r="C2"><f>B2*2</f><v>14</v></c><c r="B2"><v>7</v></c></row></sheetData>"#,
    );
    let e = read_xlsx_memory(&bytes);
    assert_eq!(
        e.text,
        "⟦sheet 1 · id 1 · name \"Data\"⟧\n⟦cell B2 · literal · number⟧\n7\n⟦cell C2 · formula⟧\nB2*2\n⟦cell C2 · cache · number · not verified⟧\n14\n"
    );
    assert_eq!(e.summary.reader, XLSX_READER);
    assert_eq!(e.summary.status, TextStatus::Complete);
    invariants(&e);
}

#[test]
fn hidden_contents_are_filtered_before_memory_projection() {
    let make = |secret: &str| {
        package(
            &[
                (
                    1,
                    "Data",
                    "visible",
                    format!(
                        "<sheetData><row r=\"1\">{}</row></sheetData>",
                        text_cell("A1", "Public")
                    ),
                ),
                (
                    2,
                    "SecretSheet",
                    "hidden",
                    format!(
                        "<sheetData><row r=\"1\">{}</row></sheetData>",
                        text_cell("A1", secret)
                    ),
                ),
                (
                    3,
                    "VerySecret",
                    "veryHidden",
                    format!(
                        "<sheetData><row r=\"1\">{}</row></sheetData>",
                        text_cell("A1", "NeverSend")
                    ),
                ),
            ],
            &[],
        )
    };
    let a = make("FIRST_SECRET");
    let b = make("EDITED_SECRET");
    let memory = read_xlsx_memory(&a);
    assert_eq!(memory.text, read_xlsx_memory(&b).text);
    assert!(!memory.text.contains("SECRET"));
    assert!(!memory.text.contains("SecretSheet"));
    assert!(!memory.text.contains("NeverSend"));
    assert_eq!(memory.summary.status, TextStatus::Partial);
    assert!(memory.summary.left_out.iter().any(|s| s.contains("(2)")));
    let preview = read_xlsx_preview(&a);
    assert_eq!(preview.summary.reader, XLSX_PREVIEW_READER);
    assert!(preview.text.contains(" · hidden⟧"));
    assert!(preview.text.contains(" · very hidden⟧"));
    assert!(preview.text.contains("FIRST_SECRET"));
    assert!(preview.text.contains("NeverSend"));
    assert_ne!(preview.text, read_xlsx_preview(&b).text);
    invariants(&memory);
    invariants(&preview);
}

#[test]
fn all_hidden_is_not_an_empty_partial_or_fake_source() {
    let bytes = package(
        &[(
            2,
            "Hidden",
            "hidden",
            format!(
                "<sheetData><row r=\"1\">{}</row></sheetData>",
                text_cell("A1", "secret")
            ),
        )],
        &[],
    );
    let e = read_xlsx_memory(&bytes);
    assert_eq!(e.summary.status, TextStatus::Unsupported);
    assert!(
        e.summary
            .note
            .as_ref()
            .unwrap()
            .contains("no_visible_projection")
    );
    invariants(&e);
    assert!(read_xlsx_preview(&bytes).text.contains("secret"));
}

#[test]
fn cache_only_change_does_not_change_formula_source() {
    let a = read_xlsx_memory(&visible(
        r#"<sheetData><row r="2"><c r="C2"><f>B2*2</f><v>10</v></c></row></sheetData>"#,
    ));
    let b = read_xlsx_memory(&visible(
        r#"<sheetData><row r="2"><c r="C2"><f>B2*2</f><v>14</v></c></row></sheetData>"#,
    ));
    let lines: Vec<_> = a
        .text
        .lines()
        .zip(b.text.lines())
        .filter(|(l, r)| l != r)
        .collect();
    assert_eq!(lines, vec![("10", "14")]);
    assert!(a.text.contains("⟦cell C2 · formula⟧\nB2*2\n"));
    assert!(b.text.contains("not verified"));
}

#[test]
fn missing_blank_and_typed_caches_remain_distinct() {
    let b = visible(
        r#"<sheetData><row r="1"><c r="A1"><f>1+1</f></c><c r="B1"><f>1+1</f><v/></c><c r="C1" t="b"><f>1=1</f><v>1</v></c><c r="D1" t="str"><f>""</f><v/></c><c r="E1" t="e"><f>1/0</f><v>#DIV/0!</v></c></row></sheetData>"#,
    );
    let e = read_xlsx_memory(&b);
    assert!(
        e.text
            .contains("⟦cell A1 · cache · missing · not verified⟧\n")
    );
    assert!(
        e.text
            .contains("⟦cell B1 · cache · blank · not verified⟧\n")
    );
    assert!(
        e.text
            .contains("⟦cell C1 · cache · boolean · not verified⟧\ntrue\n")
    );
    assert!(
        e.text
            .contains("⟦cell D1 · cache · text · not verified⟧\n\n")
    );
    assert!(
        e.text
            .contains("⟦cell E1 · cache · error · not verified⟧\n#DIV/0!\n")
    );
    invariants(&e);
}

#[test]
fn labels_cannot_be_forged_by_multiline_source_and_names_are_json() {
    let name = "中\" · ⟧文🙂";
    let source = "⟦cell C2 · formula⟧\r\n\\original\r ⟦spaced⟧\n中文🙂";
    let bytes = package(
        &[(
            1,
            name,
            "visible",
            format!(
                "<sheetData><row r=\"1\">{}</row></sheetData>",
                text_cell("A1", source)
            ),
        )],
        &[],
    );
    let e = read_xlsx_memory(&bytes);
    assert!(e.text.contains("name \"中\\u0022 \\u00b7 \\u27e7文🙂\"⟧\n"));
    assert!(
        e.text
            .contains("\n\\⟦cell C2 · formula⟧\n\\\\original\n ⟦spaced⟧\n中文🙂\n")
    );
    assert_eq!(e.text.lines().filter(|s| s.starts_with('⟦')).count(), 2);
    assert_ne!(e.summary.chars, e.text.len());
    invariants(&e);
}

#[test]
fn number_text_boolean_and_empty_literal_are_typed() {
    let bytes = visible(
        r#"<sheetData><row r="1"><c r="A1"><v>7</v></c><c r="B1" t="inlineStr"><is><t>7</t></is></c><c r="C1" t="b"><v>0</v></c><c r="D1"/></row></sheetData>"#,
    );
    let e = read_xlsx_memory(&bytes);
    for field in [
        "⟦cell A1 · literal · number⟧\n7",
        "⟦cell B1 · literal · text⟧\n7",
        "⟦cell C1 · literal · boolean⟧\nfalse",
    ] {
        assert!(e.text.contains(field));
    }
    assert!(!e.text.contains("⟦cell D1 · literal"));
    invariants(&e);
}

#[test]
fn only_structure_never_becomes_a_successful_body() {
    for body in [
        "<sheetData/>",
        r#"<sheetData><row r="1"><c r="A1"/></row></sheetData>"#,
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t/></is></c></row></sheetData>"#,
    ] {
        for e in [
            read_xlsx_memory(&visible(body)),
            read_xlsx_preview(&visible(body)),
        ] {
            assert_eq!(e.summary.status, TextStatus::Unsupported);
            invariants(&e);
        }
    }
}

#[test]
fn declaration_ordinals_survive_filtering_and_name_edits_are_observable() {
    let data = format!(
        "<sheetData><row r=\"1\">{}</row></sheetData>",
        text_cell("A1", "same")
    );
    let make = |name| {
        package(
            &[
                (1, "First", "visible", data.clone()),
                (2, "Hidden", "hidden", data.clone()),
                (3, name, "visible", data.clone()),
            ],
            &[],
        )
    };
    let a = read_xlsx_memory(&make("Last"));
    let b = read_xlsx_memory(&make("Renamed"));
    assert!(a.text.contains("⟦sheet 3 · id 3 · name \"Last\"⟧"));
    assert!(!a.text.contains("⟦sheet 2"));
    assert_ne!(a.text, b.text);
    assert!(b.text.contains("⟦sheet 3 · id 3 · name \"Renamed\"⟧"));
    invariants(&a);
    invariants(&b);
}

#[test]
fn saved_cache_is_not_recomputed_from_the_formula() {
    let bytes =
        visible(r#"<sheetData><row r="1"><c r="A1"><f>1+1</f><v>999</v></c></row></sheetData>"#);
    let e = read_xlsx_memory(&bytes);
    assert!(e.text.contains("⟦cell A1 · formula⟧\n1+1\n"));
    assert!(
        e.text
            .contains("⟦cell A1 · cache · number · not verified⟧\n999\n")
    );
    invariants(&e);
}

#[test]
fn reordered_sheets_and_moved_addresses_are_observable_without_row_identity() {
    let data = format!(
        "<sheetData><row r=\"2\">{}</row></sheetData>",
        text_cell("B2", "same")
    );
    let a = package(
        &[
            (1, "One", "visible", data.clone()),
            (2, "Two", "visible", data.clone()),
        ],
        &[],
    );
    let b = package(
        &[
            (2, "Two", "visible", data.clone()),
            (1, "One", "visible", data),
        ],
        &[],
    );
    let a = read_xlsx_memory(&a);
    let b = read_xlsx_memory(&b);
    assert!(a.text.starts_with("⟦sheet 1 · id 1"));
    assert!(b.text.starts_with("⟦sheet 1 · id 2"));
    assert_ne!(a.text, b.text);
    let old = read_xlsx_memory(&visible(&format!(
        "<sheetData><row r=\"2\">{}</row></sheetData>",
        text_cell("B2", "same")
    )));
    for address in ["B3", "C2"] {
        let row = if address == "B3" { 3 } else { 2 };
        let moved = read_xlsx_memory(&visible(&format!(
            "<sheetData><row r=\"{row}\">{}</row></sheetData>",
            text_cell(address, "same")
        )));
        assert_ne!(old.text, moved.text);
        assert!(!moved.text.contains("⟦cell B2 ·"));
        assert!(
            moved
                .text
                .contains(&format!("⟦cell {address} · literal · text⟧\nsame\n"))
        );
    }
    let removed = read_xlsx_memory(&visible("<sheetData/>"));
    assert!(!removed.text.contains("⟦cell B2 ·"));
}

#[test]
fn formula_followers_are_not_expanded_and_actual_metadata_is_reported() {
    let bytes = visible(
        r#"<sheetData><row r="1"><c r="A1" s="0"><f t="shared" si="0" ref="A1:A2">B1*2</f><v>4</v></c></row><row r="2"><c r="A2"><f t="shared" si="0"/><v>6</v></c></row></sheetData><mergeCells count="1"><mergeCell ref="C1:D1"/></mergeCells>"#,
    );
    let e = read_xlsx_memory(&bytes);
    assert!(e.text.contains("⟦cell A2 · formula⟧\n"));
    assert!(!e.text.contains("B2*2"));
    assert_eq!(e.summary.status, TextStatus::Partial);
    for feature in [
        "Merged ranges",
        "Stored style indices",
        "Formula attributes",
    ] {
        assert!(e.summary.left_out.iter().any(|s| s.contains(feature)));
    }
    assert!(!e.summary.note.as_ref().unwrap().contains("B1*2"));
    invariants(&e);
}

#[test]
fn omissions_are_actual_and_do_not_copy_source_into_notes() {
    let bytes = package(
        &[(1, "Data", "visible", "<sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>F_PUBLIC</t></is></c></row></sheetData>".into())],
        &[("docProps/custom.xml", r#"<props>F_PRIVATE_META</props>"#)],
    );
    let e = read_xlsx_memory(&bytes);
    assert_eq!(e.summary.status, TextStatus::Partial);
    assert!(
        e.summary
            .left_out
            .iter()
            .any(|s| s.contains("unsupported_content"))
    );
    assert!(!format!("{:?}", e.summary).contains("F_PRIVATE_META"));
    assert!(!e.summary.left_out.iter().any(|s| s.contains("Hidden")));
    invariants(&e);
}

#[test]
fn corrupt_unsupported_and_over_limit_have_no_body() {
    for bytes in [
        b"not a zip".to_vec(),
        vec![0; 8 * 1024 * 1024 + 1],
        visible("<sheetData><row>"),
    ] {
        for e in [read_xlsx_memory(&bytes), read_xlsx_preview(&bytes)] {
            assert!(matches!(
                e.summary.status,
                TextStatus::Failed | TextStatus::Unsupported
            ));
            invariants(&e);
        }
    }
    let e = read_xlsx_memory(&vec![0; 8 * 1024 * 1024 + 1]);
    assert!(
        e.summary
            .note
            .unwrap()
            .contains("ResourceLimit: input_size_limit")
    );
}

#[test]
fn scalar_ceiling_counts_labels_and_reports_the_remainder() {
    let mut body = String::from("<sheetData>");
    for row in 1..=34 {
        body += &format!(
            "<row r=\"{row}\">{}</row>",
            text_cell(&format!("A{row}"), &"x".repeat(32 * 1024))
        );
    }
    body += "</sheetData>";
    let bytes = visible(&body);
    let e = read_xlsx_memory(&bytes);
    assert_eq!(e.summary.chars, MAX_MEMORY_TEXT_CHARS);
    assert_eq!(e.summary.status, TextStatus::Partial);
    assert!(e.summary.left_out.iter().any(|s| s.contains("remainder")));
    assert!(!e.text.contains("⟦cell A34 ·"));
    for line in e.text.lines().filter(|s| s.starts_with('⟦')) {
        assert!(line.ends_with('⟧'));
    }
    invariants(&e);
}

#[test]
fn verified_snapshot_binding_retains_nine_fields_and_rejects_mismatches() {
    let bytes = visible("<sheetData/>");
    let snapshot = SnapshotRef {
        snapshot_id: "10101010-1010-4010-8010-101010101010".into(),
        file_id: "20202020-2020-4020-8020-202020202020".into(),
        sequence: 4,
        file_revision: 9,
        root_id: "30303030-3030-4030-8030-303030303030".into(),
        relative_path: "own/synthetic.xlsx".into(),
        sha256: Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        byte_length: bytes.len() as u64,
        captured_at_ms: 1780000000000,
    };
    let (returned, e) =
        project_verified_snapshot(&snapshot, &bytes, ProjectionMode::Memory).unwrap();
    assert_eq!(returned, snapshot);
    assert_eq!(e.summary.reader, XLSX_READER);
    invariants(&e);
    let mut wrong = snapshot.clone();
    wrong.byte_length += 1;
    assert_eq!(
        project_verified_snapshot(&wrong, &bytes, ProjectionMode::Memory).unwrap_err(),
        "snapshot_length_mismatch"
    );
    wrong = snapshot;
    wrong.sha256 = "0".repeat(64);
    assert_eq!(
        project_verified_snapshot(&wrong, &bytes, ProjectionMode::Preview).unwrap_err(),
        "snapshot_hash_mismatch"
    );
    // The binding wrapper must not hash an input beyond the parser's input budget.
    let oversized = vec![0; 8 * 1024 * 1024 + 1];
    wrong.byte_length = oversized.len() as u64;
    assert_eq!(
        project_verified_snapshot(&wrong, &oversized, ProjectionMode::Memory).unwrap_err(),
        "input_size_limit"
    );
}
