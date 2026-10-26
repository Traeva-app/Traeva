use std::io::{Cursor, Write};
use traeva_xlsx_experiment::{ErrorKind, Limits, Value, coordinate_changes, read_xlsx};
use zip::write::SimpleFileOptions;

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const T: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

fn parts(sheet: &str) -> Vec<(String, String)> {
    vec![
        (
            "[Content_Types].xml".into(),
            format!(
                r#"<Types xmlns="{T}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels".into(),
            format!(
                r#"<Relationships xmlns="{P}"><Relationship Id="main" Type="{R}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml".into(),
            format!(
                r#"<workbook xmlns="{NS}" xmlns:r="{R}"><workbookPr date1904="0"/><sheets><sheet name="Budget" sheetId="1" r:id="s1"/></sheets><calcPr calcMode="manual" fullCalcOnLoad="1"/></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".into(),
            format!(
                r#"<Relationships xmlns="{P}"><Relationship Id="s1" Type="{R}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml".into(),
            format!(r#"<worksheet xmlns="{NS}">{sheet}</worksheet>"#),
        ),
    ]
}

fn zip_parts(parts: &[(String, String)], method: zip::CompressionMethod) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in parts {
        zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(method),
        )
        .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[derive(Clone, Copy)]
enum Descriptor {
    Absent,
    Signed,
    Unsigned,
    Missing,
}

fn raw_zip(
    parts: &[(String, String)],
    deflate: bool,
    descriptor: Descriptor,
    tail: &[u8],
    gap: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let flag = if matches!(descriptor, Descriptor::Absent) {
        0u16
    } else {
        8
    };
    let method = if deflate { 8u16 } else { 0 };
    for (name, content) in parts {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(content.as_bytes());
        let expanded = content.len() as u32;
        let mut payload = if deflate {
            let mut compressor =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            compressor.write_all(content.as_bytes()).unwrap();
            compressor.finish().unwrap()
        } else {
            content.as_bytes().to_vec()
        };
        payload.extend_from_slice(tail);
        let compressed = payload.len() as u32;
        let sizes = if flag == 0 {
            [crc, compressed, expanded]
        } else {
            [0; 3]
        };
        out.extend_from_slice(b"PK\x03\x04");
        for n in [20u16, flag, method, 0, 0] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        for n in sizes {
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&payload);
        if matches!(descriptor, Descriptor::Signed) {
            out.extend_from_slice(b"PK\x07\x08");
        }
        if matches!(descriptor, Descriptor::Signed | Descriptor::Unsigned) {
            for n in [crc, compressed, expanded] {
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        out.extend_from_slice(gap);
        central.extend_from_slice(b"PK\x01\x02");
        for n in [20u16, 20, flag, method, 0, 0] {
            central.extend_from_slice(&n.to_le_bytes());
        }
        for n in [crc, compressed, expanded] {
            central.extend_from_slice(&n.to_le_bytes());
        }
        for n in [name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&n.to_le_bytes());
        }
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    let size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    for n in [0u16, 0, parts.len() as u16, parts.len() as u16] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn archive_comment(mut bytes: Vec<u8>, comment: &[u8]) -> Vec<u8> {
    let end = bytes.len();
    bytes[end - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);
    bytes
}

#[test]
fn nested_zip_end_cannot_substitute_unchecked_directory() {
    let mut p = parts("<sheetData/>");
    p.push(("xl/opaque.bin".into(), "x".into()));
    let outer = raw_zip(&p, false, Descriptor::Absent, &[], &[]);
    p.last_mut().unwrap().1 = "x".repeat(50_000);
    let inner = raw_zip(&p, true, Descriptor::Absent, &[], &[]);
    reject(&inner, ErrorKind::ResourceLimit);
    for absolute in [false, true] {
        let mut nested = inner.clone();
        if absolute {
            let eocd = nested.len() - 22;
            let mut pos = le32(&nested, eocd + 16) as usize;
            while pos < eocd {
                let offset = le32(&nested, pos + 42) + outer.len() as u32;
                nested[pos + 42..pos + 46].copy_from_slice(&offset.to_le_bytes());
                let name_len = u16::from_le_bytes(nested[pos + 28..pos + 30].try_into().unwrap());
                pos += 46 + usize::from(name_len);
            }
            let start = le32(&nested, eocd + 16) + outer.len() as u32;
            nested[eocd + 16..eocd + 20].copy_from_slice(&start.to_le_bytes());
        }
        for suffix in [&b""[..], &b"comment suffix"[..]] {
            let mut comment = nested.clone();
            comment.extend_from_slice(suffix);
            assert!(
                read_xlsx(
                    &archive_comment(outer.clone(), &comment),
                    &Limits::default()
                )
                .is_err()
            );
        }
    }
}

#[test]
fn plain_comment_is_supported_but_end_signature_in_comment_is_explicitly_rejected() {
    let bytes = workbook("<sheetData/>");
    assert_eq!(
        get(&archive_comment(bytes.clone(), b"synthetic comment"))
            .sheets
            .len(),
        1
    );
    let error = read_xlsx(&archive_comment(bytes, b"PK\x05\x06"), &Limits::default()).unwrap_err();
    assert_eq!(error.code, "ambiguous_zip_end_in_comment");
}

#[test]
fn signed_and_unsigned_descriptors_support_stored_deflate_and_empty_parts() {
    let mut p =
        parts(r#"<sheetData><row r="2"><c r="B2"><f>1+1</f><v>999</v></c></row></sheetData>"#);
    p.push(("xl/empty.bin".into(), String::new()));
    for method in [false, true] {
        for descriptor in [Descriptor::Signed, Descriptor::Unsigned] {
            let bytes = raw_zip(&p, method, descriptor, &[], &[]);
            let mut independent = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
            assert_eq!(independent.len(), p.len());
            for i in 0..independent.len() {
                let mut part = independent.by_index(i).unwrap();
                std::io::copy(&mut part, &mut std::io::sink()).unwrap();
            }
            let w = get(&bytes);
            assert_eq!(
                w.sheets[0].cells[0]
                    .cached_result
                    .as_ref()
                    .unwrap()
                    .freshness,
                "unverified"
            );
        }
    }
}

#[test]
fn missing_wrong_and_truncated_descriptors_fail_before_xml() {
    let p = parts("<sheetData/>");
    for method in [false, true] {
        reject(
            &raw_zip(&p, method, Descriptor::Missing, &[], &[]),
            ErrorKind::Failed,
        );
        for descriptor in [Descriptor::Signed, Descriptor::Unsigned] {
            let bytes = raw_zip(&p, method, descriptor, &[], &[]);
            let central = le32(&bytes, bytes.len() - 6) as usize;
            let compressed = le32(&bytes, central + 20) as usize;
            let at = 30 + p[0].0.len() + compressed;
            let skip = if matches!(descriptor, Descriptor::Signed) {
                4
            } else {
                0
            };
            for offset in [0, 4, 8] {
                let mut bad = bytes.clone();
                bad[at + skip + offset] ^= 1;
                reject(&bad, ErrorKind::Failed);
            }
            for length in 0..skip + 12 {
                assert!(read_xlsx(&bytes[..at + length], &Limits::default()).is_err());
            }
            let mut local_lie = bytes;
            local_lie[14..18].copy_from_slice(&1u32.to_le_bytes());
            reject(&local_lie, ErrorKind::Failed);
        }
    }
}

#[test]
fn deflate_tail_zero_padding_and_second_stream_are_rejected() {
    let p = parts("<sheetData/>");
    for descriptor in [Descriptor::Absent, Descriptor::Signed, Descriptor::Unsigned] {
        get(&raw_zip(&p, true, descriptor, &[], &[]));
        for tail in [&b"\0"[..], &b"trailing bytes"[..], &b"\x03\0"[..]] {
            let err = read_xlsx(
                &raw_zip(&p, true, descriptor, tail, &[]),
                &Limits::default(),
            )
            .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Failed);
            assert_eq!(err.code, "zip_deflate_trailing_bytes");
        }
    }
}

#[test]
fn bytes_outside_local_records_are_not_silently_ignored() {
    for descriptor in [Descriptor::Absent, Descriptor::Signed, Descriptor::Unsigned] {
        for method in [false, true] {
            reject(
                &raw_zip(
                    &parts("<sheetData/>"),
                    method,
                    descriptor,
                    &[],
                    b"unclaimed",
                ),
                ErrorKind::Unsupported,
            );
        }
    }
}

#[test]
fn deflate_multibuffer_output_and_actual_size_and_crc_are_checked() {
    let mut p = parts("<sheetData/>");
    let mut seed = 17u32;
    let text: String = (0..80_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            char::from(b' ' + (seed % 95) as u8)
        })
        .collect();
    p.push(("xl/large.bin".into(), text));
    let bytes = raw_zip(&p, true, Descriptor::Absent, &[], &[]);
    assert_eq!(get(&bytes).sheets.len(), 1);
    let central = le32(&bytes, bytes.len() - 6) as usize;
    for offset in [16, 24] {
        let mut bad = bytes.clone();
        let declared = le32(&bad, central + offset) - 1;
        bad[central + offset..central + offset + 4].copy_from_slice(&declared.to_le_bytes());
        let local_offset = if offset == 16 { 14 } else { 22 };
        bad[local_offset..local_offset + 4].copy_from_slice(&declared.to_le_bytes());
        reject(&bad, ErrorKind::Failed);
    }
    let mut bad = bytes;
    bad[30 + p[0].0.len()] = 0xff;
    reject(&bad, ErrorKind::Failed);
}

#[test]
fn central_symlink_and_directory_markers_remain_unsupported() {
    let bytes = workbook("<sheetData/>");
    let central = le32(&bytes, bytes.len() - 6) as usize;
    for attributes in [0x10u32, 0o120777 << 16, 0o040755 << 16] {
        for creator in [0, 3, 19] {
            let mut bad = bytes.clone();
            bad[central + 5] = creator;
            bad[central + 38..central + 42].copy_from_slice(&attributes.to_le_bytes());
            reject(&bad, ErrorKind::Unsupported);
        }
    }
}

#[test]
fn truncated_deflate_with_consistent_zip_headers_is_rejected() {
    let mut p = parts("<sheetData/>");
    p.push(("xl/last.bin".into(), "small synthetic payload".into()));
    let mut bytes = raw_zip(&p, true, Descriptor::Absent, &[], &[]);
    let mut eocd = bytes.len() - 22;
    let start = le32(&bytes, eocd + 16) as usize;
    let mut pos = start;
    let mut last = pos;
    while pos < eocd {
        last = pos;
        let name_len = u16::from_le_bytes(bytes[pos + 28..pos + 30].try_into().unwrap());
        pos += 46 + usize::from(name_len);
    }
    let local = le32(&bytes, last + 42) as usize;
    let compressed = le32(&bytes, last + 20) - 1;
    bytes[local + 18..local + 22].copy_from_slice(&compressed.to_le_bytes());
    bytes[last + 20..last + 24].copy_from_slice(&compressed.to_le_bytes());
    bytes.remove(start - 1);
    eocd -= 1;
    bytes[eocd + 16..eocd + 20].copy_from_slice(&((start - 1) as u32).to_le_bytes());
    let err = read_xlsx(&bytes, &Limits::default()).unwrap_err();
    assert_eq!(err.code, "zip_incomplete_deflate_stream");
}

#[test]
fn xml_attributes_require_separators_in_empty_start_namespace_and_declaration() {
    for (part, from, to) in [
        (
            "xl/workbook.xml",
            r#"name="Budget" sheetId="#,
            r#"name="Budget"sheetId="#,
        ),
        (
            "xl/workbook.xml",
            r#"sheetId="1" r:id="#,
            r#"sheetId="1"r:id="#,
        ),
        ("xl/workbook.xml", r#"" xmlns:r="#, r#""xmlns:r="#),
        ("xl/worksheets/sheet1.xml", r#"r="B2" t="#, r#"r="B2"t="#),
    ] {
        let mut p = parts(
            r#"<sheetData><row r="2"><c r="B2" t="inlineStr"><is><t>text</t></is></c></row></sheetData>"#,
        );
        modify(&mut p, part, from, to);
        let err = read_xlsx(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(err.code, "missing_xml_attribute_separator");
    }
    let mut p = parts("<sheetData/>");
    let workbook = p.iter_mut().find(|(n, _)| n == "xl/workbook.xml").unwrap();
    workbook
        .1
        .insert_str(0, r#"<?xml version="1.0"encoding="UTF-8"?>"#);
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
}

#[test]
fn xml_attribute_whitespace_quotes_entities_and_unicode_values_are_preserved() {
    for separator in [" ", "\t", "\r", "\n", " \t\n\r "] {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/workbook.xml",
            r#"name="Budget" sheetId="1" r:id="s1""#,
            &format!("name = '中文 &amp; &#x1F4CA;'{separator}sheetId=\"1\"{separator}r:id = 's1'"),
        );
        assert_eq!(
            get(&zip_parts(&p, zip::CompressionMethod::Stored)).sheets[0].name,
            "中文 & 📊"
        );
    }
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/workbook.xml",
        r#"name="Budget" sheetId="1""#,
        "name=\"Budget\"\u{a0}sheetId=\"1\"",
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
}

#[test]
fn malformed_xml_element_and_attribute_names_are_rejected() {
    for name in [
        "1bad",
        "bad^name",
        "-bad",
        ".bad",
        "bad?name",
        "bad@name",
        "bad/name",
        "bad\u{37e}",
    ] {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/workbook.xml",
            "<sheets>",
            &format!("<sheets {name}=\"synthetic\">"),
        );
        let err = read_xlsx(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Failed);
        assert_eq!(err.code, "invalid_xml_name");
        for element in [
            format!("<{name}/><sheetData/>"),
            format!("<{name}></{name}><sheetData/>"),
        ] {
            reject(&workbook(&element), ErrorKind::Failed);
        }
    }
}

#[test]
fn unicode_xml_names_follow_start_and_continuation_ranges() {
    let starts = [
        'A',
        '_',
        'z',
        '\u{c0}',
        '\u{d6}',
        '\u{d8}',
        '\u{f6}',
        '\u{f8}',
        '\u{2ff}',
        '\u{370}',
        '\u{37d}',
        '\u{37f}',
        '\u{1fff}',
        '\u{200c}',
        '\u{200d}',
        '\u{2070}',
        '\u{218f}',
        '\u{2c00}',
        '\u{2fef}',
        '\u{3001}',
        '\u{d7ff}',
        '\u{f900}',
        '\u{fdcf}',
        '\u{fdf0}',
        '\u{fffd}',
        '\u{10000}',
        '\u{effff}',
    ];
    for first in starts {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/workbook.xml",
            "<sheets>",
            &format!("<sheets {first}field=\"synthetic\">"),
        );
        assert_eq!(
            get(&zip_parts(&p, zip::CompressionMethod::Stored)).sheets[0].name,
            "Budget"
        );
    }
    for extra in [
        '-', '.', '0', '9', '\u{b7}', '\u{300}', '\u{36f}', '\u{203f}', '\u{2040}',
    ] {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/workbook.xml",
            "<sheets>",
            &format!("<sheets field{extra}=\"synthetic\">"),
        );
        get(&zip_parts(&p, zip::CompressionMethod::Stored));
    }
    for invalid in [
        '\u{b7}',
        '\u{d7}',
        '\u{f7}',
        '\u{300}',
        '\u{36f}',
        '\u{37e}',
        '\u{200b}',
        '\u{200e}',
        '\u{2190}',
        '\u{2bff}',
        '\u{2ff0}',
        '\u{3000}',
        '\u{fdd0}',
        '\u{fdef}',
        '\u{f0000}',
        '\u{10ffff}',
    ] {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/workbook.xml",
            "<sheets>",
            &format!("<sheets {invalid}field=\"synthetic\">"),
        );
        reject(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            ErrorKind::Failed,
        );
    }
}

#[test]
fn qualified_names_require_one_colon_and_nonempty_valid_components() {
    for name in [
        ":field",
        "p:",
        "p::field",
        "p:field:extra",
        "p:1field",
        "1p:field",
    ] {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/workbook.xml",
            "<sheets>",
            &format!("<sheets xmlns:p=\"urn:synthetic\" {name}=\"1\">"),
        );
        reject(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            ErrorKind::Failed,
        );
        reject(
            &workbook(&format!("<{name} xmlns:p=\"urn:synthetic\"/><sheetData/>")),
            ErrorKind::Failed,
        );
    }
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/workbook.xml",
        "<sheets>",
        "<sheets xmlns:测试=\"urn:synthetic\" 测试:金额=\"1\">",
    );
    get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert_eq!(
        get(&workbook(
            "<测试:记录 xmlns:测试=\"urn:synthetic\"></测试:记录><sheetData/>"
        ))
        .status,
        "partial"
    );
}

#[test]
fn invalid_xml_comments_fail_in_prolog_content_and_epilog() {
    for comment in [
        "<!-- a -- b -->",
        "<!-- tail--->",
        "<!-- a---b -->",
        "<!-- unclosed",
    ] {
        for location in [0, 1, 2] {
            let mut p = parts("<sheetData/>");
            let (_, xml) = p
                .iter_mut()
                .find(|(name, _)| name == "xl/workbook.xml")
                .unwrap();
            match location {
                0 => xml.insert_str(0, comment),
                1 => *xml = xml.replace("<sheets>", &format!("{comment}<sheets>")),
                _ => xml.push_str(comment),
            }
            reject(
                &zip_parts(&p, zip::CompressionMethod::Stored),
                ErrorKind::Failed,
            );
        }
    }
}

#[test]
fn legal_comments_do_not_change_cell_text_or_reject_markup_inside_comments() {
    for comment in ["<!---->", "<!-- a-b -->", "<!-- 中文 <>& ]]> -->"] {
        let w = get(&one(&format!(
            "<c r=\"B2\" t=\"inlineStr\"><is><t>before{comment}after</t></is></c>"
        )));
        assert_eq!(
            w.sheets[0].cells[0].literal,
            Some(Value::Text("beforeafter".into()))
        );
    }
}

#[test]
fn raw_cdata_close_in_character_data_is_rejected_but_escaped_and_cdata_text_survive() {
    for text in ["]]>", "before ]]> after", "]]]>", "]]>]]>"] {
        let err = read_xlsx(
            &one(&format!(
                "<c r=\"B2\" t=\"inlineStr\"><is><t>{text}</t></is></c>"
            )),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_xml_character_data");
    }
    for text in [
        "]]&gt;",
        "]&#93;>",
        "&#93;]>",
        "<![CDATA[]]]]><![CDATA[>]]>",
    ] {
        let w = get(&one(&format!(
            "<c r=\"B2\" t=\"inlineStr\" synthetic=\"]]>\"><is><t>{text}</t></is></c>"
        )));
        assert_eq!(
            w.sheets[0].cells[0].literal,
            Some(Value::Text("]]>".into()))
        );
    }
    let w = get(&one(
        r#"<c r="B2" t="inlineStr"><is><t><![CDATA[<>&]]></t></is></c>"#,
    ));
    assert_eq!(
        w.sheets[0].cells[0].literal,
        Some(Value::Text("<>&".into()))
    );
}

fn workbook(sheet: &str) -> Vec<u8> {
    zip_parts(&parts(sheet), zip::CompressionMethod::Stored)
}
fn one(cell: &str) -> Vec<u8> {
    workbook(&format!("<sheetData><row r=\"2\">{cell}</row></sheetData>"))
}
fn get(bytes: &[u8]) -> traeva_xlsx_experiment::Workbook {
    read_xlsx(bytes, &Limits::default()).unwrap()
}
fn modify(parts: &mut [(String, String)], name: &str, from: &str, to: &str) {
    let p = parts.iter_mut().find(|(p, _)| p == name).unwrap();
    assert!(p.1.contains(from));
    p.1 = p.1.replace(from, to);
}
fn reject(bytes: &[u8], kind: ErrorKind) {
    assert_eq!(read_xlsx(bytes, &Limits::default()).unwrap_err().kind, kind);
}

#[test]
fn formula_and_stale_cache_are_separate() {
    let w = get(&one(r#"<c r="B2"><f>A2*2</f><v>999</v></c>"#));
    let c = &w.sheets[0].cells[0];
    assert_eq!(c.formula.as_ref().unwrap().text.as_deref(), Some("A2*2"));
    assert!(c.literal.is_none());
    assert_eq!(c.cached_result.as_ref().unwrap().freshness, "unverified");
    assert_eq!(
        c.cached_result.as_ref().unwrap().value,
        Some(Value::Number("999".into()))
    );
    assert_eq!(w.calculation_properties["calcMode"], "manual");
}

#[test]
fn formula_cache_absent_empty_text_boolean_and_error() {
    let cases = [
        (r#"<c r="B2"><f>1+1</f></c>"#, None),
        (r#"<c r="B2"><f>1+1</f><v/></c>"#, Some(Value::Blank)),
        (
            r#"<c r="B2" t="str"><f>"x"</f><v/></c>"#,
            Some(Value::Text("".into())),
        ),
        (
            r#"<c r="B2" t="str"><f>"x"</f><v>x</v></c>"#,
            Some(Value::Text("x".into())),
        ),
        (
            r#"<c r="B2" t="b"><f>1=1</f><v>1</v></c>"#,
            Some(Value::Boolean(true)),
        ),
        (
            r#"<c r="B2" t="e"><f>1/0</f><v>#DIV/0!</v></c>"#,
            Some(Value::Error("#DIV/0!".into())),
        ),
    ];
    for (cell, expected) in cases {
        let w = get(&one(cell));
        let c = &w.sheets[0].cells[0];
        assert!(c.literal.is_none());
        assert_eq!(c.cached_result.as_ref().unwrap().value, expected);
        assert_eq!(c.cached_result.as_ref().unwrap().freshness, "unverified");
    }
}

#[test]
fn sparse_extreme_address_never_materializes_rectangle() {
    let w = get(&workbook(
        r#"<dimension ref="A1:XFD1048576"/><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row></sheetData>"#,
    ));
    assert_eq!(w.sheets[0].cells.len(), 2);
    let c = &w.sheets[0].cells[1];
    assert_eq!((c.row, c.column), (1_048_576, 16_384));
}

#[test]
fn raw_decimal_precision_and_scientific_notation() {
    for token in [
        "9007199254740993",
        "001.2300",
        "-3.141592653589793238462643383",
        "1E+10000",
        ".5",
    ] {
        let w = get(&one(&format!(r#"<c r="B2"><v>{token}</v></c>"#)));
        assert_eq!(
            w.sheets[0].cells[0].literal,
            Some(Value::Number(token.into()))
        );
    }
    for token in ["NaN", "inf", "1e", " 12", "+", "."] {
        reject(
            &one(&format!(r#"<c r="B2"><v>{token}</v></c>"#)),
            ErrorKind::Failed,
        );
    }
}

#[test]
fn inline_unicode_rich_text_entities_and_whitespace() {
    let w = get(&one(
        r#"<c r="B2" t="inlineStr"><is><r><t xml:space="preserve">  项目 &amp; </t></r><r><t>&#x1F4CA; &lt;x&gt;  </t></r></is></c>"#,
    ));
    assert_eq!(
        w.sheets[0].cells[0].literal,
        Some(Value::Text("  项目 & 📊 <x>  ".into()))
    );
}

#[test]
fn shared_strings_follow_relationship_not_count_attribute() {
    let mut p = parts(r#"<sheetData><row r="2"><c r="B2" t="s"><v>1</v></c></row></sheetData>"#);
    modify(
        &mut p,
        "[Content_Types].xml",
        "</Types>",
        r#"<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#,
    );
    modify(
        &mut p,
        "xl/_rels/workbook.xml.rels",
        "</Relationships>",
        &format!(
            r#"<Relationship Id="ss" Type="{R}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
        ),
    );
    p.push(("xl/sharedStrings.xml".into(), format!(r#"<sst xmlns="{NS}" count="4294967295" uniqueCount="4294967295"><si><t>unused</t></si><si><r><t>Revenue</t></r><r><t> USD</t></r></si></sst>"#)));
    let w = get(&zip_parts(&p, zip::CompressionMethod::Deflated));
    assert_eq!(
        w.sheets[0].cells[0].literal,
        Some(Value::Text("Revenue USD".into()))
    );
    modify(&mut p, "xl/worksheets/sheet1.xml", "<v>1</v>", "<v>2</v>");
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
}

#[test]
fn shared_formula_follower_is_not_expanded_or_literal() {
    let w = get(&workbook(
        r#"<sheetData><row r="2"><c r="B2"><f t="shared" si="0" ref="B2:B3">A2*2</f><v>4</v></c></row><row r="3"><c r="B3"><f t="shared" si="0"/><v>6</v></c></row></sheetData>"#,
    ));
    assert_eq!(w.status, "partial");
    let f = w.sheets[0].cells[1].formula.as_ref().unwrap();
    assert!(f.text.is_none());
    assert_eq!(f.interpretation, "unresolved_shared_follower");
    assert_eq!(f.attributes["si"], "0");
    assert!(w.sheets[0].cells[1].literal.is_none());
}

#[test]
fn array_followers_are_unverified_caches_not_literals() {
    let w = get(&workbook(
        r#"<sheetData><row r="2"><c r="B2"><f t="array" ref="B2:B3">A2:A3*2</f><v>4</v></c></row><row r="3"><c r="B3"><v>6</v></c></row></sheetData>"#,
    ));
    let member = &w.sheets[0].cells[1];
    assert!(member.literal.is_none());
    assert_eq!(
        member.formula.as_ref().unwrap().interpretation,
        "unresolved_array_or_data_table_member"
    );
    assert_eq!(
        member.formula.as_ref().unwrap().attributes["anchor_address"],
        "B2"
    );
    assert_eq!(
        member.cached_result.as_ref().unwrap().freshness,
        "unverified"
    );
}

#[test]
fn data_table_formula_and_unknown_type_disclose_limits() {
    for typ in ["dataTable", "futureFormula"] {
        let w = get(&one(&format!(
            r#"<c r="B2"><f t="{typ}" ref="B2:B3" dt2D="1"/><v>9</v></c>"#
        )));
        assert_eq!(w.status, "partial");
        assert_eq!(
            w.sheets[0].cells[0].formula.as_ref().unwrap().attributes["dt2D"],
            "1"
        );
    }
}

#[test]
fn dangerous_formula_text_is_only_returned_as_text() {
    for formula in [
        "WEBSERVICE(\"https://127.0.0.1:1/test\")",
        "cmd|' /C touch /tmp/should-never-exist'!A0",
        "[remote.xlsx]Sheet1!A1",
        "HYPERLINK(\"file:///tmp/test\")",
    ] {
        let encoded = formula.replace('&', "&amp;").replace('<', "&lt;");
        let w = get(&one(&format!(r#"<c r="B2"><f>{encoded}</f><v>0</v></c>"#)));
        assert_eq!(
            w.sheets[0].cells[0]
                .formula
                .as_ref()
                .unwrap()
                .text
                .as_deref(),
            Some(formula)
        );
        assert_eq!(
            w.sheets[0].cells[0].cached_result.as_ref().unwrap().value,
            Some(Value::Number("0".into()))
        );
    }
}

#[test]
fn date_epoch_style_and_merges_preserve_context() {
    let mut p = parts(
        r#"<dimension ref="A2:C2"/><sheetData><row r="2"><c r="B2" s="3"><v>45000</v></c><c r="C2" t="d"><v>2026-10-11T12:30:00Z</v></c></row></sheetData><mergeCells count="1"><mergeCell ref="A2:B2"/></mergeCells>"#,
    );
    modify(
        &mut p,
        "xl/workbook.xml",
        "date1904=\"0\"",
        "date1904=\"1\"",
    );
    let w = get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert!(w.date_1904);
    assert_eq!(w.sheets[0].merged_ranges, ["A2:B2"]);
    assert_eq!(w.sheets[0].cells[0].style_index, Some(3));
    assert_eq!(
        w.sheets[0].cells[0].literal,
        Some(Value::Number("45000".into()))
    );
}

#[test]
fn hidden_and_empty_sheets_are_retained() {
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/workbook.xml",
        "sheetId=\"1\"",
        "sheetId=\"1\" state=\"veryHidden\"",
    );
    let w = get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert_eq!(w.sheets[0].visibility, "veryHidden");
    assert!(w.sheets[0].cells.is_empty());
}

#[test]
fn explicit_feature_omissions_keep_supported_cells() {
    let mut p = parts(
        r#"<sheetData><row r="2"><c r="B2"><v>7</v></c></row></sheetData><drawing/><pivotTableParts/><tableParts/><extLst/>"#,
    );
    for name in [
        "xl/charts/chart1.xml",
        "xl/pivotTables/pivot1.xml",
        "xl/vbaProject.bin",
        "xl/connections.xml",
        "xl/externalLinks/externalLink1.xml",
        "xl/embeddings/object.bin",
    ] {
        p.push((
            name.into(),
            if name.ends_with("xml") {
                "<unknown/>"
            } else {
                "SYNTHETIC"
            }
            .into(),
        ));
    }
    let w = get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert_eq!(w.status, "partial");
    assert_eq!(
        w.sheets[0].cells[0].literal,
        Some(Value::Number("7".into()))
    );
    for code in [
        "charts_not_interpreted",
        "pivot_tables_not_interpreted",
        "macros_not_executed_or_interpreted",
        "external_data_not_refreshed",
        "external_links_not_refreshed",
        "objects_not_interpreted",
        "tables_not_interpreted",
    ] {
        assert!(w.omissions.iter().any(|f| f.code == code), "{code}");
    }
}

#[test]
fn external_relationship_is_reported_and_not_opened() {
    let mut p = parts("<sheetData/>");
    p.push(("xl/worksheets/_rels/sheet1.xml.rels".into(), format!(r#"<Relationships xmlns="{P}"><Relationship Id="outside" Type="{R}/hyperlink" Target="file:///DOES/NOT/EXIST" TargetMode="External"/></Relationships>"#)));
    let w = get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert!(
        w.omissions
            .iter()
            .any(|f| f.code == "external_relationship_not_followed")
    );
}

#[test]
fn external_sheet_is_unsupported() {
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/_rels/workbook.xml.rels",
        "Target=\"worksheets/sheet1.xml\"",
        "Target=\"https://127.0.0.1:1/sheet.xml\" TargetMode=\"External\"",
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Unsupported,
    );
}

#[test]
fn relationship_escape_missing_duplicate_and_unqualified_id_fail() {
    for target in [
        "../../outside.xml",
        "file:///etc/passwd",
        "worksheets/%73heet1.xml",
        "worksheets\\sheet1.xml",
        "worksheets/missing.xml",
    ] {
        let mut p = parts("<sheetData/>");
        modify(
            &mut p,
            "xl/_rels/workbook.xml.rels",
            "worksheets/sheet1.xml",
            target,
        );
        assert!(
            read_xlsx(
                &zip_parts(&p, zip::CompressionMethod::Stored),
                &Limits::default()
            )
            .is_err()
        );
    }
    let mut p = parts("<sheetData/>");
    modify(&mut p, "xl/workbook.xml", "r:id=", "id=");
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/_rels/workbook.xml.rels",
        "</Relationships>",
        &format!(
            r#"<Relationship Id="s1" Type="{R}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
        ),
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
}

#[test]
fn namespace_prefixes_work_and_foreign_lookalikes_do_not() {
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/worksheets/sheet1.xml",
        &format!(r#"<worksheet xmlns="{NS}"><sheetData/></worksheet>"#),
        &format!(
            r#"<s:worksheet xmlns:s="{NS}"><s:sheetData><s:row r="2"><s:c r="B2"><s:v>7</s:v></s:c></s:row></s:sheetData></s:worksheet>"#
        ),
    );
    assert_eq!(
        get(&zip_parts(&p, zip::CompressionMethod::Stored)).sheets[0]
            .cells
            .len(),
        1
    );
    let w = get(&workbook(
        r#"<sheetData xmlns:fake="urn:fake"><row r="2"><fake:c r="B2"><fake:v>7</fake:v></fake:c></row></sheetData>"#,
    ));
    assert!(w.sheets[0].cells.is_empty());
    assert_eq!(w.status, "partial");
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/worksheets/sheet1.xml",
        NS,
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Unsupported,
    );
}

#[test]
fn invalid_addresses_duplicate_cells_and_rows_fail() {
    for addr in [
        "A0",
        "A01",
        "a2",
        "XFE2",
        "A1048577",
        "$B$2",
        "AAA999999999999",
        "2",
        "B3",
    ] {
        reject(
            &one(&format!(r#"<c r="{addr}"><v>1</v></c>"#)),
            ErrorKind::Failed,
        );
    }
    reject(
        &one(r#"<c r="B2"><v>1</v></c><c r="B2"><v>2</v></c>"#),
        ErrorKind::Failed,
    );
    reject(
        &workbook(r#"<sheetData><row r="2"/><row r="2"/></sheetData>"#),
        ErrorKind::Failed,
    );
}

#[test]
fn malformed_cell_structures_fail() {
    for cell in [
        r#"<c r="B2"><v>1</v><v>2</v></c>"#,
        r#"<c r="B2"><f/></c>"#,
        r#"<c r="B2"><f>1</f><f>2</f></c>"#,
        r#"<c r="B2" t="inlineStr"><is><t>x</t></is><v>1</v></c>"#,
        r#"<c r="B2"><v><x>1</x></v></c>"#,
        r#"<c r="B2"><f t="shared">1</f></c>"#,
        r#"<c r="B2" t="b"><v>2</v></c>"#,
    ] {
        assert!(read_xlsx(&one(cell), &Limits::default()).is_err(), "{cell}");
    }
}

#[test]
fn malformed_xml_entities_doctype_and_encoding_are_rejected() {
    for sheet in [
        "<sheetData>",
        "<sheetData/></worksheet><worksheet>",
        "<sheetData><row r=\"2\"><c r=\"B2\" t=\"str\"><v>&unknown;</v></c></row></sheetData>",
        "<sheetData><row r=\"2\"><c r=\"B2\" t=\"str\"><v>&#0;</v></c></row></sheetData>",
        "<sheetData><x:a/></sheetData>",
        "<sheetData a=\"1\" a=\"2\"/>",
    ] {
        assert!(
            read_xlsx(&workbook(sheet), &Limits::default()).is_err(),
            "{sheet}"
        );
    }
    for prefix in [
        r#"<!DOCTYPE worksheet [<!ENTITY x SYSTEM "file:///etc/passwd">]>"#,
        r#"<?xml version="1.0" encoding="utf-16"?>"#,
        "<?run evil?>",
    ] {
        let mut p = parts("<sheetData/>");
        p[4].1 = format!("{prefix}{}", p[4].1);
        reject(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            ErrorKind::Unsupported,
        );
    }
}

#[test]
fn invalid_required_parts_and_macro_enabled_workbook_fail() {
    let mut p = parts("<sheetData/>");
    p.remove(4);
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "[Content_Types].xml",
        "spreadsheetml.sheet.main+xml",
        "spreadsheetml.template.main+xml",
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Unsupported,
    );
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "[Content_Types].xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Unsupported,
    );
}

#[test]
fn input_and_all_structural_limits_reject_without_workbook() {
    let bytes = one(r#"<c r="B2" t="str"><v>longer than one byte</v></c>"#);
    let cases = [
        Limits {
            input_bytes: 10,
            ..Limits::default()
        },
        Limits {
            entries: 1,
            ..Limits::default()
        },
        Limits {
            central_bytes: 1,
            ..Limits::default()
        },
        Limits {
            entry_bytes: 1,
            ..Limits::default()
        },
        Limits {
            expanded_bytes: 10,
            ..Limits::default()
        },
        Limits {
            xml_event_bytes: 10,
            ..Limits::default()
        },
        Limits {
            xml_depth: 1,
            ..Limits::default()
        },
        Limits {
            xml_attributes: 1,
            ..Limits::default()
        },
        Limits {
            xml_nodes: 1,
            ..Limits::default()
        },
        Limits {
            text_bytes: 1,
            ..Limits::default()
        },
        Limits {
            retained_text_bytes: 1,
            ..Limits::default()
        },
    ];
    for limit in cases {
        assert_eq!(
            read_xlsx(&bytes, &limit).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
    }
    let two = one(r#"<c r="B2"><v>1</v></c><c r="C2"><v>2</v></c>"#);
    assert_eq!(
        read_xlsx(
            &two,
            &Limits {
                cells: 1,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        read_xlsx(
            &bytes,
            &Limits {
                cells: 10_001,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn compression_bomb_and_declared_entry_size_fail_before_parsing() {
    let mut p = parts("<sheetData/>");
    p.push(("xl/bomb.bin".into(), "x".repeat(500_000)));
    reject(
        &zip_parts(&p, zip::CompressionMethod::Deflated),
        ErrorKind::ResourceLimit,
    );
    let mut bytes = workbook("<sheetData/>");
    let c = bytes.windows(4).position(|s| s == b"PK\x01\x02").unwrap();
    bytes[c + 24..c + 28].copy_from_slice(&5_000_000u32.to_le_bytes());
    reject(&bytes, ErrorKind::ResourceLimit);
}

#[test]
fn cfb_and_arbitrary_non_zip_never_reach_office_parser() {
    let mut cfb = vec![0u8; 512];
    cfb[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    cfb[44..48].copy_from_slice(&u32::MAX.to_le_bytes());
    reject(&cfb, ErrorKind::Unsupported);
    for bytes in [&[][..], &b"not xlsx"[..], &b"PK\x03\x04"[..]] {
        assert!(read_xlsx(bytes, &Limits::default()).is_err());
    }
}

#[test]
fn zip_truncation_crc_and_local_header_lies_fail() {
    let bytes = workbook("<sheetData/>");
    for cut in [1, bytes.len() / 2, bytes.len() - 1] {
        assert!(read_xlsx(&bytes[..cut], &Limits::default()).is_err());
    }
    let mut corrupt = bytes.clone();
    let text = corrupt.windows(5).position(|b| b == b"Types").unwrap();
    corrupt[text] ^= 1;
    reject(&corrupt, ErrorKind::Failed);
    let mut local = bytes;
    local[22..26].copy_from_slice(&1u32.to_le_bytes());
    reject(&local, ErrorKind::Failed);
}

#[test]
fn zip_encryption_zip64_multidisk_alias_and_duplicate_are_rejected() {
    let bytes = workbook("<sheetData/>");
    let e = bytes.len() - 22;
    let c = bytes.windows(4).position(|s| s == b"PK\x01\x02").unwrap();
    let mut encrypted = bytes.clone();
    encrypted[c + 8] |= 1;
    reject(&encrypted, ErrorKind::Unsupported);
    let mut zip64 = bytes.clone();
    zip64[e + 10..e + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    reject(&zip64, ErrorKind::Unsupported);
    let mut multidisk = bytes.clone();
    multidisk[e + 4..e + 6].copy_from_slice(&1u16.to_le_bytes());
    reject(&multidisk, ErrorKind::Unsupported);
    for name in [
        "../escape.xml",
        "/absolute.xml",
        "xl/./fake.xml",
        "xl/fake%20.xml",
        "xl\\fake.xml",
        "XL/WORKBOOK.XML",
    ] {
        let mut p = parts("<sheetData/>");
        p.push((name.into(), "<fake/>".into()));
        reject(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            ErrorKind::Unsupported,
        );
    }
}

#[test]
fn cache_only_change_is_not_a_formula_or_literal_edit() {
    let before = get(&one(r#"<c r="B2"><f>A2*2</f><v>4</v></c>"#));
    let after = get(&one(r#"<c r="B2"><f>A2*2</f><v>999</v></c>"#));
    let diff = coordinate_changes(&before, &after).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].address, "B2");
    assert_eq!(diff[0].sheet_name, "Budget");
    assert!(diff[0].cache_changed);
    assert!(!diff[0].formula_changed && !diff[0].literal_changed);
}

#[test]
fn coordinate_changes_distinguish_literals_formulas_and_removal() {
    let before = get(&one(
        r#"<c r="B2"><v>10</v></c><c r="C2"><f>B2*2</f><v>20</v></c><c r="D2"><v>7</v></c>"#,
    ));
    let after = get(&one(
        r#"<c r="B2"><v>11</v></c><c r="C2"><f>B2*3</f><v>20</v></c><c r="E2"><v>8</v></c>"#,
    ));
    let diff = coordinate_changes(&before, &after).unwrap();
    assert_eq!(diff.len(), 4);
    assert!(diff[0].literal_changed && !diff[0].formula_changed);
    assert!(diff[1].formula_changed && !diff[1].cache_changed);
    assert_eq!(diff[2].change, "removed");
    assert_eq!(diff[3].change, "added");
}

#[test]
fn renamed_sheet_requires_separate_alignment_decision() {
    let before = get(&workbook("<sheetData/>"));
    let mut p = parts("<sheetData/>");
    modify(&mut p, "xl/workbook.xml", "Budget", "Renamed");
    let after = get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert_eq!(
        coordinate_changes(&before, &after).unwrap_err().code,
        "coordinate_diff_requires_same_sheet_context"
    );
}

#[test]
fn deterministic_output_and_content_hash_follow_input_bytes() {
    let bytes = one(r#"<c r="B2"><v>1</v></c>"#);
    let a = get(&bytes);
    let b = get(&bytes);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(a.input_sha256.len(), 64);
    let other = get(&one(r#"<c r="B2"><v>2</v></c>"#));
    assert_ne!(a.input_sha256, other.input_sha256);
}

#[test]
fn chartsheet_name_is_preserved_without_claiming_cell_support() {
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/_rels/workbook.xml.rels",
        "/worksheet\"",
        "/chartsheet\"",
    );
    modify(
        &mut p,
        "[Content_Types].xml",
        "spreadsheetml.worksheet+xml",
        "spreadsheetml.chartsheet+xml",
    );
    p[4].1 = format!(r#"<chartsheet xmlns="{NS}"/>"#);
    let w = get(&zip_parts(&p, zip::CompressionMethod::Stored));
    assert_eq!(w.status, "partial");
    assert_eq!(w.sheets[0].name, "Budget");
    assert_eq!(w.sheets[0].kind, "chartsheet");
    assert!(w.sheets[0].cells.is_empty());
    assert!(
        w.omissions
            .iter()
            .any(|f| f.code == "charts_not_interpreted")
    );
}

#[test]
fn cell_sheet_and_shared_string_caps_are_package_wide() {
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "[Content_Types].xml",
        "</Types>",
        r#"<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
    );
    modify(
        &mut p,
        "xl/_rels/workbook.xml.rels",
        "</Relationships>",
        &format!(
            r#"<Relationship Id="s2" Type="{R}/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#
        ),
    );
    modify(
        &mut p,
        "xl/workbook.xml",
        "</sheets>",
        r#"<sheet name="Other" sheetId="2" r:id="s2"/></sheets>"#,
    );
    let sheet = format!(
        r#"<worksheet xmlns="{NS}"><sheetData><row r="2"><c r="B2"><v>1</v></c></row></sheetData></worksheet>"#
    );
    p[4].1 = sheet.clone();
    p.push(("xl/worksheets/sheet2.xml".into(), sheet));
    let bytes = zip_parts(&p, zip::CompressionMethod::Stored);
    for limit in [
        Limits {
            sheets: 1,
            ..Limits::default()
        },
        Limits {
            cells: 1,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            read_xlsx(&bytes, &limit).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
    }
    modify(
        &mut p,
        "[Content_Types].xml",
        "</Types>",
        r#"<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#,
    );
    modify(
        &mut p,
        "xl/_rels/workbook.xml.rels",
        "</Relationships>",
        &format!(
            r#"<Relationship Id="ss" Type="{R}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
        ),
    );
    p.push((
        "xl/sharedStrings.xml".into(),
        format!(r#"<sst xmlns="{NS}"><si><t>1</t></si><si><t>2</t></si></sst>"#),
    ));
    assert_eq!(
        read_xlsx(
            &zip_parts(&p, zip::CompressionMethod::Stored),
            &Limits {
                shared_strings: 1,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn formula_regions_validate_anchors_and_absent_member_cache() {
    reject(
        &one(r#"<c r="B2"><f t="array" ref="C2:C3">A2:A3*2</f><v>4</v></c>"#),
        ErrorKind::Failed,
    );
    let w = get(&workbook(
        r#"<sheetData><row r="2"><c r="B2"><f t="array" ref="B2:B3">A2:A3*2</f><v>4</v></c></row><row r="3"><c r="B3"/></row></sheetData>"#,
    ));
    let c = &w.sheets[0].cells[1];
    assert!(c.literal.is_none());
    assert!(c.cached_result.as_ref().unwrap().value.is_none());
}

#[test]
fn crc_of_unsupported_part_is_checked_too() {
    let mut p = parts("<sheetData/>");
    p.push(("xl/vbaProject.bin".into(), "inert_marker".into()));
    let mut bytes = zip_parts(&p, zip::CompressionMethod::Stored);
    let at = bytes
        .windows(12)
        .position(|b| b == b"inert_marker")
        .unwrap();
    bytes[at] ^= 1;
    reject(&bytes, ErrorKind::Failed);
}

#[test]
fn empty_sheet_and_duplicate_sheet_contexts_are_not_silently_accepted() {
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/workbook.xml",
        "<sheet name=\"Budget\" sheetId=\"1\" r:id=\"s1\"/>",
        "",
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
    let mut p = parts("<sheetData/>");
    modify(
        &mut p,
        "xl/workbook.xml",
        "</sheets>",
        r#"<sheet name="budget" sheetId="2" r:id="s1"/></sheets>"#,
    );
    reject(
        &zip_parts(&p, zip::CompressionMethod::Stored),
        ErrorKind::Failed,
    );
}

#[test]
fn truncated_and_mutated_synthetic_inputs_return_without_panicking() {
    let bytes = workbook("<sheetData/>");
    for cut in 0..bytes.len() {
        assert!(read_xlsx(&bytes[..cut], &Limits::default()).is_err());
    }
    let mut seed = 37u32;
    for _ in 0..300 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let at = seed as usize % bytes.len();
        let mut mutated = bytes.clone();
        mutated[at] ^= ((seed >> 16) as u8) | 1;
        let _ = read_xlsx(&mutated, &Limits::default());
    }
}

#[test]
fn escaped_text_is_preserved_with_explicit_limitation() {
    for cell in [
        r#"<c r="B2" t="inlineStr"><is><t>_x000A_</t></is></c>"#,
        r#"<c r="B2" t="str"><f>"x"</f><v>_x005F_x000A_</v></c>"#,
    ] {
        let w = get(&one(cell));
        assert_eq!(w.status, "partial");
        assert!(
            w.omissions
                .iter()
                .any(|f| f.code == "excel_escaped_text_not_decoded")
        );
    }
    for cell in [
        r#"<c r="B2" t="future"/>"#,
        r#"<c r="B2" t="future"><v/></c>"#,
    ] {
        reject(&one(cell), ErrorKind::Unsupported);
    }
}
