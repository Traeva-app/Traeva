use super::*;
use std::io::Write;
use zip::{ZipWriter, write::SimpleFileOptions};

const CONTENT_TYPES: &str = "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>";
const ROOT_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"r1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
fn document(body: &str) -> String {
    format!("<w:document xmlns:w=\"{WORD}\"><w:body>{body}</w:body></w:document>")
}
fn paragraph(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}
fn zip_parts(parts: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn package(body: &str, extra: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let main = document(body);
    let mut parts = vec![
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", main.as_bytes()),
    ];
    parts.extend_from_slice(extra);
    zip_parts(&parts, method)
}
fn read(body: &str) -> TextExtraction {
    read_docx(&package(body, &[], CompressionMethod::Stored))
}
fn without_text(bytes: &[u8], status: TextStatus) {
    let extraction = read_docx(bytes);
    assert_eq!(
        extraction.summary.status, status,
        "{:?}",
        extraction.summary
    );
    assert!(extraction.text.is_empty());
    assert_eq!(extraction.summary.chars, 0);
    assert!(extraction.summary.covered.is_empty());
    assert!(extraction.summary.left_out.is_empty());
    assert!(extraction.summary.note.is_some());
}
fn central(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap()
}
fn set16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn set32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn plain_paragraphs_empty_paragraphs_and_unicode() {
    let out = read(
        "<w:p/><w:p><w:r><w:t xml:space=\"preserve\"> 中国 🦀 </w:t><w:tab/><w:t>A&amp;B &#x1f642;</w:t><w:br/><w:t>next</w:t></w:r></w:p><w:p/>",
    );
    assert_eq!(out.summary.status, TextStatus::Complete);
    assert_eq!(out.text, "\n 中国 🦀 \tA&B 🙂\nnext\n");
    assert_eq!(out.summary.chars, out.text.chars().count());
    assert!(out.summary.note.is_none());
}
#[test]
fn empty_body_complete() {
    assert_eq!(read("").summary.status, TextStatus::Complete);
}
#[test]
fn deflated_and_stored_identical() {
    let body = paragraph("Hello World");
    assert_eq!(
        read_docx(&package(&body, &[], CompressionMethod::Deflated)).text,
        read(&body).text
    );
}
#[test]
fn strict_and_namespace_aliases() {
    let doc = document(&paragraph("ok"))
        .replace(WORD, STRICT_WORD)
        .replace("w:", "x:")
        .replace("xmlns:w", "xmlns:x");
    let bytes = zip_parts(
        &[
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("word/document.xml", doc.as_bytes()),
        ],
        CompressionMethod::Stored,
    );
    assert_eq!(read_docx(&bytes).text, "ok");
}
#[test]
fn formatting_ignored() {
    let out = read(
        "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>styled</w:t></w:r></w:p><w:sectPr/>",
    );
    assert_eq!(out.text, "styled");
    assert_eq!(out.summary.status, TextStatus::Complete);
}
#[test]
fn tables_are_omitted_not_flattened() {
    let out = read(&format!(
        "{}<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>{}",
        paragraph("before"),
        paragraph("secret table"),
        paragraph("after")
    ));
    assert_eq!(out.text, "before\nafter");
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(out.summary.left_out.iter().any(|s| s == "tables"));
}
#[test]
fn revisions_not_accepted_or_rejected() {
    let out = read(
        "<w:p><w:r><w:t>plain</w:t></w:r><w:ins><w:r><w:t>added</w:t></w:r></w:ins><w:del><w:r><w:delText>removed</w:delText></w:r></w:del></w:p>",
    );
    assert_eq!(out.text, "plain");
    assert_eq!(out.summary.status, TextStatus::Partial);
}
#[test]
fn foreign_text_is_not_body_text() {
    let out = read("<w:p><x:t xmlns:x=\"urn:evil\">secret</x:t><w:r><w:t>ok</w:t></w:r></w:p>");
    assert_eq!(out.text, "ok");
    assert_eq!(out.summary.status, TextStatus::Partial);
}
#[test]
fn unclosed_complex_field_is_damaged() {
    without_text(
        &package(
            "<w:p><w:r><w:t>discard prefix</w:t><w:fldChar w:fldCharType=\"begin\"/></w:r></w:p>",
            &[],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
}
#[test]
fn simple_field_without_supported_text_unsupported() {
    let out = read(
        "<w:p><w:fldSimple w:instr=\"INCLUDETEXT file:///private/secret\"><w:r><w:t>cached</w:t></w:r></w:fldSimple></w:p>",
    );
    assert!(out.text.is_empty());
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert_eq!(
        out.summary.note.as_deref(),
        Some(
            "The document has no text in the supported main-body subset. Omitted content: fields and field instructions."
        )
    );
}
#[test]
fn unsupported_body_note_names_every_omitted_category_without_content() {
    for (body, categories) in [
        (
            "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>secret table</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
            "tables",
        ),
        (
            "<w:tbl/><w:p><w:fldSimple w:instr=\"SECRET\"><w:r><w:t>secret field</w:t></w:r></w:fldSimple></w:p>",
            "fields and field instructions, tables",
        ),
    ] {
        let out = read(body);
        assert_eq!(out.summary.status, TextStatus::Unsupported);
        assert!(out.text.is_empty());
        assert!(out.summary.covered.is_empty());
        assert!(out.summary.left_out.is_empty());
        assert_eq!(
            out.summary.note,
            Some(format!(
                "The document has no text in the supported main-body subset. Omitted content: {categories}."
            ))
        );
    }
}
#[test]
fn external_relationships_never_followed() {
    let rels=b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"url\" Type=\"hyperlink\" Target=\"http://127.0.0.1:1/private\" TargetMode=\"External\"/></Relationships>";
    let out = read_docx(&package(
        "<w:p><w:hyperlink><w:r><w:t>label</w:t></w:r></w:hyperlink></w:p>",
        &[("word/_rels/document.xml.rels", rels)],
        CompressionMethod::Stored,
    ));
    assert_eq!(out.text, "label");
    assert_eq!(out.summary.status, TextStatus::Complete);
    assert!(out.summary.left_out.is_empty());
}
#[test]
fn ancillary_stories_reported() {
    let rels = story_rels(&[
        ("header", "hdr1.xml"),
        ("footnotes", "footnotes.xml"),
        ("comments", "comments.xml"),
    ]);
    let header = format!(
        "<w:hdr xmlns:w=\"{WORD}\">{}</w:hdr>",
        paragraph("header text")
    );
    let notes = format!(
        "<w:footnotes xmlns:w=\"{WORD}\"><w:footnote w:id=\"1\">{}</w:footnote></w:footnotes>",
        paragraph("note")
    );
    let comments = format!(
        "<w:comments xmlns:w=\"{WORD}\"><w:comment w:id=\"0\">{}</w:comment></w:comments>",
        paragraph("comment")
    );
    let out = read_docx(&package(
        &paragraph("body"),
        &[
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/hdr1.xml", header.as_bytes()),
            ("word/footnotes.xml", notes.as_bytes()),
            ("word/comments.xml", comments.as_bytes()),
        ],
        CompressionMethod::Stored,
    ));
    assert_eq!(out.text, "body");
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(
        out.summary.left_out,
        ["comments", "footnotes and endnotes", "headers and footers"]
    );
}
#[test]
fn missing_required_parts_failed() {
    without_text(
        &zip_parts(
            &[("word/document.xml", document("").as_bytes())],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
}
#[test]
fn unsupported_main_content_type() {
    let types = CONTENT_TYPES.replace(MAIN_TYPE, "application/xml");
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", types.as_bytes()),
                ("_rels/.rels", ROOT_RELS.as_bytes()),
                ("word/document.xml", document("").as_bytes()),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn external_main_unsupported() {
    let rels = ROOT_RELS.replace("Target=", "TargetMode=\"External\" Target=");
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                ("_rels/.rels", rels.as_bytes()),
                ("word/document.xml", document("").as_bytes()),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn duplicate_relationship_id_failed() {
    let rels = ROOT_RELS.replace(
        "</Relationships>",
        "<Relationship Id=\"r1\" Type=\"x\" Target=\"y\"/></Relationships>",
    );
    without_text(
        &package(
            "",
            &[("word/_rels/document.xml.rels", rels.as_bytes())],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
}
#[test]
fn macros_unsupported() {
    without_text(
        &package(
            "",
            &[("word/vbaProject.bin", b"macro")],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn ole_container_unsupported() {
    without_text(
        &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1],
        TextStatus::Unsupported,
    );
}
#[test]
fn encrypted_zip_unsupported() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    let cd = central(&bytes);
    set16(&mut bytes, cd + 8, 1);
    set16(&mut bytes, 6, 1);
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn unknown_compression_unsupported() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    let cd = central(&bytes);
    set16(&mut bytes, cd + 10, 99);
    set16(&mut bytes, 8, 99);
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn input_limit_unsupported() {
    without_text(&vec![0; MAX_DOCX_BYTES + 1], TextStatus::Unsupported);
}
#[test]
fn entry_count_limit_before_archive_allocation() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    let end = bytes.len() - 22;
    set16(&mut bytes, end + 8, 513);
    set16(&mut bytes, end + 10, 513);
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn declared_entry_size_limit() {
    let mut bytes = package("", &[], CompressionMethod::Deflated);
    let cd = central(&bytes);
    set32(&mut bytes, cd + 24, MAX_ENTRY_BYTES as u32 + 1);
    set32(&mut bytes, 22, MAX_ENTRY_BYTES as u32 + 1);
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn compression_bomb_ratio_unsupported() {
    let bytes = package(
        "",
        &[("word/media/bomb", &vec![b'X'; 1024 * 1024])],
        CompressionMethod::Deflated,
    );
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn repetitive_long_body_is_unsupported_not_partial_truncation() {
    let text = "Repetitive legitimate paragraph. ".repeat(20_000);
    assert!(text.chars().count() < MAX_MEMORY_TEXT_CHARS);
    // Written at the highest compression level, so the package passes the
    // ratio below with any deflate backend in the build (the PDF reader's
    // hayro turns on flate2's zlib-rs, #186).
    let main = document(&paragraph(&text));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("word/document.xml", main.as_str()),
    ] {
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .compression_level(Some(9)),
            )
            .unwrap();
        writer.write_all(data.as_bytes()).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let main = archive.by_name("word/document.xml").unwrap();
    assert!(main.size() < MAX_XML_BYTES as u64);
    assert!(main.size() > main.compressed_size() * MAX_COMPRESSION_RATIO);
    let out = read_docx(&bytes);
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert!(out.text.is_empty());
    assert_eq!(out.summary.chars, 0);
    assert!(out.summary.covered.is_empty());
    assert!(out.summary.left_out.is_empty());
    let note = out.summary.note.unwrap();
    assert!(note.contains("200:1"));
    assert!(note.contains("Highly repetitive long documents"));
    assert!(note.contains("Unsupported, not truncated Partial"));
}
#[test]
fn false_declared_size_discards_all_text() {
    let mut bytes = package("", &[], CompressionMethod::Deflated);
    let cd = central(&bytes);
    let n = u32_at(&bytes, cd + 24).unwrap();
    set32(&mut bytes, cd + 24, n - 1);
    set32(&mut bytes, 22, n - 1);
    without_text(&bytes, TextStatus::Failed);
}
#[test]
fn crc_corruption_failed() {
    let mut bytes = package(&paragraph("body"), &[], CompressionMethod::Stored);
    let cd = central(&bytes);
    let data = 30 + u16_at(&bytes, 26).unwrap() as usize + u16_at(&bytes, 28).unwrap() as usize;
    bytes[data] ^= 1;
    assert!(cd > data);
    without_text(&bytes, TextStatus::Failed);
}
#[test]
fn every_truncation_failed() {
    let bytes = package(&paragraph("ok"), &[], CompressionMethod::Stored);
    for at in 0..bytes.len() {
        without_text(&bytes[..at], TextStatus::Failed);
    }
}
#[test]
fn trailing_zip_junk_failed() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    bytes.extend(b"junk");
    without_text(&bytes, TextStatus::Failed);
}
#[test]
fn zip_paths_rejected() {
    for name in [
        "../escape",
        "/absolute",
        "word/../x",
        "word\\x",
        "word//x",
        "word/%2e/x",
        "C:evil",
    ] {
        without_text(
            &package("", &[(name, b"x")], CompressionMethod::Stored),
            TextStatus::Failed,
        );
    }
}
#[test]
fn local_central_disagreement_failed() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    bytes[30] ^= 1;
    without_text(&bytes, TextStatus::Failed);
}
#[test]
fn duplicate_body_failed() {
    without_text(
        &package("</w:body><w:body>", &[], CompressionMethod::Stored),
        TextStatus::Failed,
    );
}
#[test]
fn malformed_xml_and_entities_failed() {
    for body in [
        "<w:p>",
        "<w:p><w:r><w:t>&unknown;</w:t></w:r></w:p>",
        "<w:p><w:r><w:t>&#0;</w:t></w:r></w:p>",
        "<w:p><w:r><w:t>\u{1}</w:t></w:r></w:p>",
        "<bad:p/>",
        "<w:p x=\"1\" x=\"2\"/>",
    ] {
        without_text(
            &package(body, &[], CompressionMethod::Stored),
            TextStatus::Failed,
        );
    }
}
#[test]
fn malformed_auxiliary_xml_also_failed() {
    without_text(
        &package(
            &paragraph("discard"),
            &[("word/settings.xml", b"<x>")],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
}
#[test]
fn dtd_unsupported() {
    let doc = format!(
        "<!DOCTYPE w:document [<!ENTITY x SYSTEM 'file:///etc/passwd'>]>{}",
        document(&paragraph("&x;"))
    );
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                ("_rels/.rels", ROOT_RELS.as_bytes()),
                ("word/document.xml", doc.as_bytes()),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn processing_instruction_unsupported() {
    without_text(
        &package("<?open external?>", &[], CompressionMethod::Stored),
        TextStatus::Unsupported,
    );
}
#[test]
fn excessive_depth_unsupported() {
    let body = format!(
        "{}{}",
        "<w:unknown>".repeat(MAX_XML_DEPTH),
        "</w:unknown>".repeat(MAX_XML_DEPTH)
    );
    without_text(
        &package(&body, &[], CompressionMethod::Stored),
        TextStatus::Unsupported,
    );
}
#[test]
fn excessive_events_unsupported() {
    let body = "<w:p/>".repeat(MAX_XML_EVENTS);
    without_text(
        &package(&body, &[], CompressionMethod::Stored),
        TextStatus::Unsupported,
    );
}
#[test]
fn xml_size_limit_unsupported() {
    let body = paragraph(&"x".repeat(MAX_XML_BYTES));
    without_text(
        &package(&body, &[], CompressionMethod::Stored),
        TextStatus::Unsupported,
    );
}
#[test]
fn text_limit_partial_unicode_boundary() {
    let text = "🦀".repeat(MAX_MEMORY_TEXT_CHARS + 1);
    let out = read(&paragraph(&text));
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.text.chars().count(), MAX_MEMORY_TEXT_CHARS);
    let out = read(&paragraph(&"x".repeat(MAX_MEMORY_TEXT_CHARS + 1)));
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.text.chars().count(), MAX_MEMORY_TEXT_CHARS);
}
#[test]
fn deterministic_bit_flip_corpus_never_panics() {
    let bytes = package(&paragraph("fuzz me 中国"), &[], CompressionMethod::Deflated);
    for i in 0..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0x80;
        let out = read_docx(&b);
        assert!(out.summary.chars <= MAX_MEMORY_TEXT_CHARS);
        if matches!(
            out.summary.status,
            TextStatus::Failed | TextStatus::Unsupported
        ) {
            assert!(out.text.is_empty());
        }
    }
}

#[test]
fn numeric_cr_and_crlf_normalized_across_events() {
    assert_eq!(read(&paragraph("A&#13;B&#13;&#10;C")).text, "A\nB\nC");
    assert_eq!(
        read(&format!("{}{}", paragraph("A&#13;"), paragraph("B"))).text,
        "A\n\nB"
    );
}
#[test]
fn xml_names_and_expanded_attributes_must_be_valid() {
    for body in [
        "<w:p bad:attr=\"x\"><w:r><w:t>ok</w:t></w:r></w:p>",
        "<w:p 1x=\"a\"/>",
        "<w:p xmlns:a=\"urn:a\" xmlns:b=\"urn:a\" a:x=\"1\" b:x=\"2\"/>",
        "<w:p x=\"<\"/>",
        "<w:p><w:r><w:t>]]></w:t></w:r></w:p>",
        "<w:p><w:r><w:t>ok</w:t></w:r><w:1bad/></w:p>",
    ] {
        without_text(
            &package(body, &[], CompressionMethod::Stored),
            TextStatus::Failed,
        );
    }
}

#[test]
fn invalid_declarations_and_namespace_bindings_failed() {
    for prefix in [
        "<?xml version=\"1.0\" version=\"1.1\"?>",
        "<?xml version=\"1.0\" standalone=\"bogus\"?>",
        " <?xml version=\"1.0\"?>",
        "<!--x--><?xml version=\"1.0\"?>",
        "<?xml version=\"1.0\" standalone=\"yes\" encoding=\"UTF-8\"?>",
    ] {
        let doc = format!("{prefix}{}", document(&paragraph("ok")));
        without_text(
            &zip_parts(
                &[
                    ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                    ("_rels/.rels", ROOT_RELS.as_bytes()),
                    ("word/document.xml", doc.as_bytes()),
                ],
                CompressionMethod::Stored,
            ),
            TextStatus::Failed,
        );
    }
    for attrs in [
        "xmlns:a=\"\"",
        "xmlns:xml=\"urn:wrong\"",
        "xmlns:a=\"http://www.w3.org/XML/1998/namespace\"",
        "xmlns=\"http://www.w3.org/2000/xmlns/\"",
    ] {
        without_text(
            &package(
                &format!("<w:p {attrs}><w:r><w:t>ok</w:t></w:r></w:p>"),
                &[],
                CompressionMethod::Stored,
            ),
            TextStatus::Failed,
        );
    }
}
#[test]
fn main_override_must_be_direct_child() {
    let types = CONTENT_TYPES
        .replace("<Override", "<Other><Override")
        .replace("</Types>", "</Other></Types>");
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", types.as_bytes()),
                ("_rels/.rels", ROOT_RELS.as_bytes()),
                ("word/document.xml", document("").as_bytes()),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
}

#[test]
fn renamed_macro_with_default_content_type_unsupported() {
    let types = CONTENT_TYPES.replace(
        "</Types>",
        "<Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/></Types>",
    );
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", types.as_bytes()),
                ("_rels/.rels", ROOT_RELS.as_bytes()),
                ("word/document.xml", document("").as_bytes()),
                ("word/vba.bin", b"macro"),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn package_expanded_total_limit() {
    let mut state = 0x1234_5678u32;
    let chunk: Vec<u8> = (0..8192)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let blob = chunk.repeat(896);
    let names: Vec<_> = (0..5).map(|i| format!("part{i}.bin")).collect();
    let parts: Vec<_> = names
        .iter()
        .map(|name| (name.as_str(), blob.as_slice()))
        .collect();
    let bytes = zip_parts(&parts, CompressionMethod::Deflated);
    let result = read_docx(&bytes);
    assert_eq!(result.summary.status, TextStatus::Unsupported);
    assert!(result.summary.note.unwrap().contains("32 MiB"));
}
#[test]
fn corruption_after_text_limit_discards_the_prefix() {
    let body = format!(
        "{}<w:bad>",
        paragraph(&"x".repeat(MAX_MEMORY_TEXT_CHARS + 1))
    );
    without_text(
        &package(&body, &[], CompressionMethod::Stored),
        TextStatus::Failed,
    );
}

#[test]
fn missing_attribute_separators_and_bad_numeric_entities_failed() {
    for body in [
        "<w:p x=\"1\"y=\"2\"/>",
        "<w:p x=\"&#+13;\"/>",
        "<w:p><w:r><w:t>&#+13;</w:t></w:r></w:p>",
    ] {
        without_text(
            &package(body, &[], CompressionMethod::Stored),
            TextStatus::Failed,
        );
    }
}
#[test]
fn only_xml_space_allowed_outside_root() {
    for doc in [
        format!("\u{a0}{}", document("")),
        format!("{}\u{a0}", document("")),
        format!("<?xml version=\"1.0\"standalone=\"yes\"?>{}", document("")),
    ] {
        without_text(
            &zip_parts(
                &[
                    ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                    ("_rels/.rels", ROOT_RELS.as_bytes()),
                    ("word/document.xml", doc.as_bytes()),
                ],
                CompressionMethod::Stored,
            ),
            TextStatus::Failed,
        );
    }
}

#[test]
fn unicode_extra_cannot_override_the_preflight_part_name() {
    without_text(
        include_bytes!("../tests/fixtures/docx/zip-unicode-path-alias.docx"),
        TextStatus::Unsupported,
    );
}

#[test]
fn genuine_word_body_regression_after_metadata_scrub() {
    for (bytes, expected) in [
        (
            include_bytes!("../tests/fixtures/docx/word-basic-v1.docx").as_slice(),
            include_str!("../tests/fixtures/docx/word-basic-v1.txt"),
        ),
        (
            include_bytes!("../tests/fixtures/docx/word-basic-v2.docx").as_slice(),
            include_str!("../tests/fixtures/docx/word-basic-v2.txt"),
        ),
    ] {
        let out = read_docx(bytes);
        assert_eq!(out.summary.status, TextStatus::Complete);
        assert_eq!(out.text, expected);
    }
}
#[test]
fn symlink_entry_unsupported() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    let at = central(&bytes);
    bytes[at + 5] = 3;
    set32(&mut bytes, at + 38, 0o120777 << 16);
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn zip64_and_multi_volume_unsupported() {
    let mut bytes = package("", &[], CompressionMethod::Stored);
    let end = bytes.len() - 22;
    set32(&mut bytes, end + 12, u32::MAX);
    without_text(&bytes, TextStatus::Unsupported);
    let mut bytes = package("", &[], CompressionMethod::Stored);
    let end = bytes.len() - 22;
    set16(&mut bytes, end + 4, 1);
    without_text(&bytes, TextStatus::Unsupported);
}
#[test]
fn duplicate_entry_names_failed_before_zip_library_deduplication() {
    let mut bytes = zip_parts(
        &[("one.xml", b"<x/>"), ("two.xml", b"<x/>")],
        CompressionMethod::Stored,
    );
    for i in 0..bytes.len() - 7 {
        if &bytes[i..i + 7] == b"two.xml" {
            bytes[i..i + 7].copy_from_slice(b"one.xml");
        }
    }
    without_text(&bytes, TextStatus::Failed);
}
#[test]
fn xml_limits_on_attributes_and_namespaces() {
    let attrs = (0..65).map(|i| format!(" a{i}=\"v\"")).collect::<String>();
    without_text(
        &package(&format!("<w:p{attrs}/>"), &[], CompressionMethod::Stored),
        TextStatus::Unsupported,
    );
}

#[test]
fn namespace_resolver_limit_is_unsupported_not_damage() {
    let attrs = (0..257)
        .map(|i| format!(" xmlns:a{i}=\"urn:{i}\""))
        .collect::<String>();
    without_text(
        &package(&format!("<w:p{attrs}/>"), &[], CompressionMethod::Stored),
        TextStatus::Unsupported,
    );
}
#[test]
fn excessive_qname_parts_rejected_without_a_parts_vector() {
    let name = format!("w:{}", "a:".repeat(100000));
    without_text(
        &package(&format!("<{name}/>"), &[], CompressionMethod::Stored),
        TextStatus::Failed,
    );
}

fn story_rels(parts: &[(&str, &str)]) -> String {
    let entries: String = parts.iter().enumerate().map(|(i,(kind,target))|format!("<Relationship Id=\"r{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>")).collect();
    format!("<Relationships xmlns=\"{RELS}\">{entries}</Relationships>")
}
#[test]
fn empty_header_separator_notes_and_external_template_do_not_omit_content() {
    let rels=story_rels(&[("header","Header1.xml"),("footnotes","footnotes.xml")]).replace("</Relationships>","<Relationship Id=\"template\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate\" Target=\"https://invalid.example/template.dotx\" TargetMode=\"External\"/></Relationships>");
    let header = format!("<w:hdr xmlns:w=\"{WORD}\"><w:p/></w:hdr>");
    let notes = format!(
        "<x:footnotes xmlns:x=\"{WORD}\"><x:footnote x:type=\"separator\" x:id=\"-1\"><x:p><x:r><x:separator/></x:r></x:p></x:footnote><x:footnote x:type=\"continuationSeparator\" x:id=\"0\"><x:p><x:r><x:continuationSeparator/></x:r></x:p></x:footnote></x:footnotes>"
    );
    for body in ["".to_string(), paragraph("body")] {
        let out = read_docx(&package(
            &body,
            &[
                ("word/_rels/document.xml.rels", rels.as_bytes()),
                ("word/Header1.xml", header.as_bytes()),
                ("word/footnotes.xml", notes.as_bytes()),
            ],
            CompressionMethod::Stored,
        ));
        assert_eq!(
            out.summary.status,
            TextStatus::Complete,
            "{:?}",
            out.summary
        );
        assert!(out.summary.left_out.is_empty());
    }
}
#[test]
fn header_relationship_resolves_custom_name_case_and_relative_target() {
    for target in ["hdr1.xml", "../word/HDR1.XML", "/word/Hdr1.xml"] {
        let rels = story_rels(&[("header", target)]);
        let header = format!(
            "<w:hdr xmlns:w=\"{WORD}\">{}</w:hdr>",
            paragraph("actual header")
        );
        let out = read_docx(&package(
            &paragraph("body"),
            &[
                ("word/_rels/document.xml.rels", rels.as_bytes()),
                ("word/Hdr1.xml", header.as_bytes()),
            ],
            CompressionMethod::Stored,
        ));
        assert_eq!(out.summary.status, TextStatus::Partial);
        assert_eq!(out.summary.left_out, ["headers and footers"]);
        assert_eq!(out.text, "body");
    }
}
#[test]
fn unrelated_media_file_does_not_claim_an_omitted_image() {
    let out = read_docx(&package(
        &paragraph("body"),
        &[("word/media/unused.bin", b"synthetic")],
        CompressionMethod::Stored,
    ));
    assert_eq!(out.summary.status, TextStatus::Complete);
}
#[test]
fn foreign_story_text_is_an_actual_omission() {
    let rels = story_rels(&[("header", "hdr.xml")]);
    let header = format!(
        "<w:hdr xmlns:w=\"{WORD}\"><a:t xmlns:a=\"urn:drawing\">actual foreign text</a:t></w:hdr>"
    );
    let out = read_docx(&package(
        &paragraph("body"),
        &[
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/hdr.xml", header.as_bytes()),
        ],
        CompressionMethod::Stored,
    ));
    assert_eq!(out.text, "body");
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.summary.left_out, ["headers and footers"]);
}
#[test]
fn complex_fields_omit_instructions_and_cached_results_including_nested_fields() {
    let body = "<w:p><w:r><w:t>before</w:t><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> SEQ Figure </w:instrText><w:fldChar w:fldCharType=\"separate\"/><w:t>cached secret</w:t><w:fldChar w:fldCharType=\"begin\"/><w:t>nested secret</w:t><w:fldChar w:fldCharType=\"end\"/><w:fldChar w:fldCharType=\"end\"/><w:t>after</w:t></w:r></w:p>";
    for body in [body.to_string(), body.replace("w:", "x:")] {
        let doc = if body.contains("x:") {
            document(&body)
                .replace("w:", "x:")
                .replace("xmlns:w", "xmlns:x")
        } else {
            document(&body)
        };
        let out = read_docx(&zip_parts(
            &[
                ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                ("_rels/.rels", ROOT_RELS.as_bytes()),
                ("word/document.xml", doc.as_bytes()),
            ],
            CompressionMethod::Stored,
        ));
        assert_eq!(out.text, "beforeafter");
        assert_eq!(out.summary.status, TextStatus::Partial);
        assert_eq!(out.summary.left_out, ["fields and field instructions"]);
    }
}
#[test]
fn malformed_field_controls_discard_supported_prefix() {
    for controls in [
        "<w:fldChar w:fldCharType=\"end\"/>",
        "<w:fldChar w:fldCharType=\"separate\"/>",
        "<w:fldChar w:fldCharType=\"begin\"/><w:fldChar w:fldCharType=\"separate\"/><w:fldChar w:fldCharType=\"separate\"/><w:fldChar w:fldCharType=\"end\"/>",
    ] {
        without_text(
            &package(
                &format!("<w:p><w:r><w:t>prefix</w:t>{controls}</w:r></w:p>"),
                &[],
                CompressionMethod::Stored,
            ),
            TextStatus::Failed,
        );
    }
}
#[test]
fn fields_spanning_paragraphs_preserve_retained_text_boundaries() {
    let prefix = "<w:p><w:r><w:t>BEFORE</w:t><w:fldChar w:fldCharType=\"begin\"/><w:instrText>REF secret</w:instrText></w:r></w:p>";
    let suffix = "<w:p><w:r><w:t>cached secret</w:t><w:fldChar w:fldCharType=\"end\"/><w:t>AFTER</w:t></w:r></w:p>";
    for middle in ["", "<w:p><w:r><w:t>more cached secret</w:t></w:r></w:p>"] {
        let out = read(&format!("{prefix}{middle}{suffix}"));
        assert_eq!(out.text, "BEFORE\nAFTER");
        assert_eq!(out.summary.status, TextStatus::Partial);
    }
    let out = read(&format!(
        "{}{}",
        prefix.replace("<w:t>BEFORE</w:t>", ""),
        suffix.replace("<w:t>AFTER</w:t>", "")
    ));
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert!(out.text.is_empty());
}
#[test]
fn paragraph_mark_revisions_are_reported_without_accepting_them() {
    let out = read(
        "<w:p><w:pPr><w:rPr><w:del w:id=\"1\"/></w:rPr></w:pPr><w:r><w:t>body</w:t></w:r></w:p>",
    );
    assert_eq!(out.text, "body");
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(
        out.summary
            .left_out
            .iter()
            .any(|s| s.starts_with("tracked changes"))
    );
}
#[test]
fn utf16_without_bom_and_word_templates_are_unsupported() {
    for big_endian in [false, true] {
        let xml: Vec<u8> = document("")
            .encode_utf16()
            .flat_map(|u| {
                if big_endian {
                    u.to_be_bytes()
                } else {
                    u.to_le_bytes()
                }
            })
            .collect();
        without_text(
            &zip_parts(
                &[
                    ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                    ("_rels/.rels", ROOT_RELS.as_bytes()),
                    ("word/document.xml", &xml),
                ],
                CompressionMethod::Stored,
            ),
            TextStatus::Unsupported,
        );
    }
    let types = CONTENT_TYPES.replace(
        MAIN_TYPE,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
    );
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", types.as_bytes()),
                ("_rels/.rels", ROOT_RELS.as_bytes()),
                ("word/document.xml", document("").as_bytes()),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn case_fold_duplicate_package_parts_are_rejected() {
    for alias in ["word/Document.xml", "WORD/DOCUMENT.XML"] {
        without_text(
            &package(
                &paragraph("body"),
                &[(alias, document(&paragraph("other")).as_bytes())],
                CompressionMethod::Stored,
            ),
            TextStatus::Failed,
        );
    }
}
#[test]
fn namespace_work_and_namespace_byte_limits_precede_large_clones() {
    let uri = "u".repeat(1025);
    without_text(
        &package(
            &format!("<w:p xmlns:x=\"{uri}\"><w:r><w:t>body</w:t></w:r></w:p>"),
            &[],
            CompressionMethod::Stored,
        ),
        TextStatus::Unsupported,
    );
    let decls: String = (0..50)
        .map(|i| format!(" xmlns:n{i}=\"urn:n{i}\""))
        .collect();
    let attrs: String = (0..50).map(|i| format!(" n{i}:a=\"x\"")).collect();
    let heavy = format!(
        "<w:p{decls}>{}<w:r><w:t>body</w:t></w:r></w:p>",
        format!("<w:bookmarkStart{attrs}/>").repeat(400)
    );
    let out = read(&heavy);
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert!(out.summary.note.unwrap().contains("namespace-resolution"));
    assert!(out.text.is_empty());
}
#[test]
fn text_truncation_reports_tail_and_deflated_text_uses_same_limit() {
    // Deterministic incompressible text also exercises deflate below the ratio cap.
    let mut state = 17u32;
    let text: String = (0..MAX_MEMORY_TEXT_CHARS + 7)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[(state % 62) as usize]
                as char
        })
        .collect();
    let main = document(&paragraph(&text));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("word/document.xml", main.as_str()),
    ] {
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .compression_level(Some(1)),
            )
            .unwrap();
        writer.write_all(data.as_bytes()).unwrap();
    }
    let deflated = writer.finish().unwrap().into_inner();
    for bytes in [
        package(&paragraph(&text), &[], CompressionMethod::Stored),
        deflated,
    ] {
        let out = read_docx(&bytes);
        assert_eq!(out.summary.status, TextStatus::Partial);
        assert_eq!(out.summary.chars, MAX_MEMORY_TEXT_CHARS);
        assert!(
            out.summary
                .left_out
                .iter()
                .any(|s| s.starts_with("the last 7 characters"))
        );
        assert!(out.summary.note.unwrap().contains("1000007 characters"));
    }
}

#[test]
fn nested_eocd_comment_cannot_select_an_unchecked_body() {
    let control = include_bytes!("../tests/fixtures/docx/nested-eocd-control.docx");
    assert_eq!(read_docx(control).text, "REAL BODY");
    let bytes = include_bytes!("../tests/fixtures/docx/nested-eocd.docx");
    let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
    let mut library_xml = String::new();
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut library_xml)
        .unwrap();
    assert!(library_xml.contains("FAKE BODY"));
    assert!(!library_xml.contains("REAL BODY"));
    assert_eq!(archive.len(), 4);
    assert_eq!(archive.offset(), 0);
    let out = read_docx(bytes);
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert!(out.text.is_empty());
    assert!(out.summary.note.unwrap().contains("archive comments"));
}

#[test]
fn aes_extra_cannot_override_outer_stored_method() {
    let bytes = include_bytes!("../tests/fixtures/docx/aes-method-override.docx");
    let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
    let file = archive.by_index_raw(2).unwrap();
    assert_eq!(file.compression(), CompressionMethod::Deflated);
    assert!(!file.encrypted());
    let out = read_docx(bytes);
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert!(out.text.is_empty());
    assert!(out.summary.note.unwrap().contains("AES ZIP metadata"));
}

#[test]
fn zip_library_view_must_match_every_preflight_field() {
    let bytes = package(&paragraph("body"), &[], CompressionMethod::Stored);
    let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
    let file = archive.by_index_raw(2).unwrap();
    for change in 0..9 {
        let mut entries = preflight(&bytes).unwrap();
        let entry = &mut entries[2];
        assert!(same_entry(&file, entry));
        match change {
            0 => entry.name = "word/Document.xml".into(),
            1 => entry.local += 1,
            2 => entry.central += 1,
            3 => entry.data += 1,
            4 => entry.flags ^= 2,
            5 => entry.method = CompressionMethod::Deflated,
            6 => entry.crc ^= 1,
            7 => entry.packed += 1,
            8 => entry.unpacked += 1,
            _ => unreachable!(),
        }
        assert!(!same_entry(&file, entry));
    }
}

#[test]
fn word_original_layout_growth_hints_preserved() {
    for (bytes, expected) in [
        (
            include_bytes!("../tests/fixtures/docx/word-layout-v1.docx").as_slice(),
            include_str!("../tests/fixtures/docx/word-basic-v1.txt"),
        ),
        (
            include_bytes!("../tests/fixtures/docx/word-layout-v2.docx").as_slice(),
            include_str!("../tests/fixtures/docx/word-basic-v2.txt"),
        ),
    ] {
        assert!(bytes.windows(2).any(|w| w == [0x20, 0xa2]));
        let out = read_docx(bytes);
        assert_eq!(
            out.summary.status,
            TextStatus::Complete,
            "{:?}",
            out.summary
        );
        assert_eq!(out.text, expected);
    }
}

#[test]
fn actual_rich_word_save_matches_text_and_actual_omissions() {
    let out = read_docx(include_bytes!("../tests/fixtures/docx/word-rich.docx"));
    assert_eq!(
        out.text,
        include_str!("../tests/fixtures/docx/rich-source.txt")
    );
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.summary.chars, 108);
    assert_eq!(
        out.summary.left_out,
        [
            "comments",
            "fields and field instructions",
            "footnotes and endnotes",
            "headers and footers",
            "tables",
            "tracked changes (no revision acceptance policy applied)"
        ]
    );
}

#[test]
fn only_valid_inert_office_growth_hint_extra_is_allowed() {
    assert!(check_extra(&[0x20, 0xa2, 6, 0, 0x28, 0xa0, 2, 0, 0, 0]).is_ok());
    for extra in [
        &[0x20, 0xa2, 6, 0, 0x28, 0xa0, 3, 0, 0, 0][..],
        &[0x20, 0xa2, 6, 0, 0x28, 0xa0, 2, 0, 0, 1],
        &[0x20, 0xa2, 6, 0, 0x29, 0xa0, 2, 0, 0, 0],
        &[0x55, 0x54, 0, 0],
    ] {
        assert!(matches!(
            check_extra(extra),
            Err(Problem(TextStatus::Unsupported, _))
        ));
    }
}

#[test]
fn network_path_relationships_are_not_internal_part_paths() {
    let root = ROOT_RELS.replace(
        "Target=\"word/document.xml\"",
        "Target=\"//word/document.xml\"",
    );
    without_text(
        &zip_parts(
            &[
                ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                ("_rels/.rels", root.as_bytes()),
                ("word/document.xml", document(&paragraph("body")).as_bytes()),
            ],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
    let rels = story_rels(&[("header", "//word/header.xml")]);
    without_text(
        &package(
            &paragraph("body"),
            &[("word/_rels/document.xml.rels", rels.as_bytes())],
            CompressionMethod::Stored,
        ),
        TextStatus::Failed,
    );
}
