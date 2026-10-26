use super::*;
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[path = "pptx_projection_tests.rs"]
mod projections;

type Parts = Vec<(String, Vec<u8>)>;
fn xml_part(name: &str, value: &str) -> (String, Vec<u8>) {
    (name.into(), value.as_bytes().to_vec())
}
fn rels(content: &str) -> String {
    format!("<Relationships xmlns=\"{RELS}\">{content}</Relationships>")
}
fn rel(id: &str, kind: &str, target: &str) -> String {
    format!("<Relationship Id=\"{id}\" Type=\"{R}/{kind}\" Target=\"{target}\"/>")
}
fn shape(text: &str, ph: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"
    )
}
fn slide(body: &str, root: &str) -> String {
    format!(
        "<p:{root} xmlns:p=\"{P}\" xmlns:a=\"{A}\"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>{body}</p:spTree></p:cSld></p:{root}>"
    )
}
fn types(entries: &[(&str, &str)]) -> String {
    format!(
        "<Types xmlns=\"{TYPES}\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>{}</Types>",
        entries
            .iter()
            .map(|(name, kind)| format!("<Override PartName=\"/{name}\" ContentType=\"{kind}\"/>"))
            .collect::<String>()
    )
}
fn base(body: &str) -> Parts {
    vec![
        xml_part(
            "[Content_Types].xml",
            &types(&[
                ("ppt/presentation.xml", MAIN_TYPE),
                ("ppt/slides/slide1.xml", SLIDE_TYPE),
            ]),
        ),
        xml_part(
            "_rels/.rels",
            &rels(&rel("main", "officeDocument", "ppt/presentation.xml")),
        ),
        xml_part(
            "ppt/presentation.xml",
            &format!(
                "<p:presentation xmlns:p=\"{P}\" xmlns:r=\"{R}\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"s1\"/></p:sldIdLst></p:presentation>"
            ),
        ),
        xml_part(
            "ppt/_rels/presentation.xml.rels",
            &rels(&rel("s1", "slide", "slides/slide1.xml")),
        ),
        xml_part("ppt/slides/slide1.xml", &slide(body, "sld")),
    ]
}
fn set(parts: &mut Parts, name: &str, value: &str) {
    let data = &mut parts.iter_mut().find(|(n, _)| n == name).unwrap().1;
    *data = value.as_bytes().to_vec();
}
fn change(parts: &mut Parts, name: &str, before: &str, after: &str) {
    let data = &mut parts.iter_mut().find(|(n, _)| n == name).unwrap().1;
    *data = String::from_utf8(data.clone())
        .unwrap()
        .replace(before, after)
        .into_bytes();
}
fn archive(parts: &Parts, method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn read(parts: &Parts) -> TextExtraction {
    read_pptx(&archive(parts, CompressionMethod::Stored))
}
fn valid(out: &TextExtraction) {
    assert_eq!(out.summary.chars, out.text.chars().count());
    assert!(!out.text.contains('\r'));
    assert_eq!(out.summary.reader, PPTX_READER);
    match out.summary.status {
        TextStatus::Complete => {
            assert!(!out.summary.covered.is_empty());
            assert!(out.summary.left_out.is_empty());
            assert!(out.summary.note.is_none());
        }
        TextStatus::Partial => {
            assert!(!out.text.is_empty());
            assert!(!out.summary.covered.is_empty());
            assert!(!out.summary.left_out.is_empty());
            assert!(out.summary.note.is_some());
        }
        _ => {
            assert!(out.text.is_empty());
            assert!(out.summary.covered.is_empty());
            assert!(out.summary.left_out.is_empty());
            assert!(out.summary.note.is_some());
        }
    }
}
fn without(bytes: &[u8], status: TextStatus) {
    let out = read_pptx(bytes);
    valid(&out);
    assert_eq!(out.summary.status, status, "{:?}", out.summary);
    assert!(out.text.is_empty());
}
fn bad(parts: &Parts, status: TextStatus) {
    without(&archive(parts, CompressionMethod::Stored), status);
}
fn central(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap()
}
fn set16(bytes: &mut [u8], i: usize, value: u16) {
    bytes[i..i + 2].copy_from_slice(&value.to_le_bytes());
}
fn set32(bytes: &mut [u8], i: usize, value: u32) {
    bytes[i..i + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn title_body_unicode_entities_and_labels() {
    let out = read(&base(&shape(
        "中国 🦀 A&amp;B &#x1f642;",
        "<p:ph type=\"title\"/>",
    )));
    valid(&out);
    assert_eq!(out.summary.status, TextStatus::Complete);
    assert_eq!(out.text, "[Slide 1]\n[Title]\n中国 🦀 A&B 🙂");
}
#[test]
fn strict_namespace_aliases() {
    let parts: Parts = base(&shape("ok", ""))
        .into_iter()
        .map(|(n, b)| {
            (
                n,
                String::from_utf8(b)
                    .unwrap()
                    .replace(P, SP)
                    .replace(A, SA)
                    .replace(R, SR)
                    .replace("<p:", "<z:")
                    .replace("</p:", "</z:")
                    .replace("xmlns:p", "xmlns:z")
                    .into_bytes(),
            )
        })
        .collect();
    assert_eq!(read(&parts).text, "[Slide 1]\n[Text]\nok");
}
#[test]
fn deflate_matches_stored() {
    let parts = base(&shape("hello", ""));
    assert_eq!(
        read(&parts),
        read_pptx(&archive(&parts, CompressionMethod::Deflated))
    );
}
#[test]
fn empty_slide_and_empty_deck_complete() {
    let mut parts = base("");
    assert_eq!(read(&parts).summary.status, TextStatus::Complete);
    change(
        &mut parts,
        "ppt/presentation.xml",
        "<p:sldId id=\"256\" r:id=\"s1\"/>",
        "",
    );
    assert_eq!(read(&parts).text, "");
    assert_eq!(read(&parts).summary.status, TextStatus::Complete);
}
#[test]
fn paragraphs_breaks_empty_paragraphs_and_crlf() {
    let body = shape("a\r\nb\rc", "").replace(
        "</a:r></a:p>",
        "</a:r><a:br/><a:r><a:t>next</a:t></a:r></a:p><a:p/><a:p><a:r><a:t>end</a:t></a:r></a:p>",
    );
    assert_eq!(
        read(&base(&body)).text,
        "[Slide 1]\n[Text]\na\nb\nc\nnext\n\nend"
    );
}
#[test]
fn grouped_shapes_keep_xml_order() {
    let body = format!(
        "<p:grpSp><p:nvGrpSpPr/><p:grpSpPr/>{}{}</p:grpSp>{}",
        shape("two", ""),
        shape("one", ""),
        shape("three", "")
    );
    assert_eq!(
        read(&base(&body)).text,
        "[Slide 1]\n[Text]\ntwo\n[Text]\none\n[Text]\nthree"
    );
}
#[test]
fn fields_omitted_preserving_other_text() {
    let body = shape("before", "").replace(
        "</a:r></a:p>",
        "</a:r><a:fld><a:t>SECRET</a:t></a:fld><a:r><a:t>after</a:t></a:r></a:p>",
    );
    let out = read(&base(&body));
    valid(&out);
    assert_eq!(out.text, "[Slide 1]\n[Text]\nbeforeafter");
    assert_eq!(out.summary.left_out, vec!["dynamic fields"]);
}
#[test]
fn images_charts_tables_animations_disclose_actual_categories() {
    let body = format!(
        "{}<p:pic/><p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:t>SECRET</a:t></a:tc></a:tr></a:tbl><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"/></a:graphicData></a:graphic></p:graphicFrame>",
        shape("ok", "")
    );
    let mut parts = base(&body);
    change(
        &mut parts,
        "ppt/slides/slide1.xml",
        "</p:sld>",
        "<p:timing><p:tnLst/></p:timing><p:transition><p:fade/></p:transition></p:sld>",
    );
    let out = read(&parts);
    valid(&out);
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(!out.text.contains("SECRET"));
    for label in [
        "images (including alternative text)",
        "charts",
        "tables",
        "animations/timing",
        "slide transitions",
    ] {
        assert!(
            out.summary.left_out.contains(&label.into()),
            "{label}: {:?}",
            out.summary
        );
    }
}
#[test]
fn image_only_is_unsupported_and_labels_cannot_make_partial() {
    bad(&base("<p:pic/>"), TextStatus::Unsupported);
}
#[test]
fn foreign_text_is_not_silently_included() {
    let out = read(&base(&format!(
        "{}<p:weird><x:text xmlns:x=\"urn:other\">SECRET</x:text></p:weird>",
        shape("ok", "")
    )));
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(!out.text.contains("SECRET"));
}
#[test]
fn alternate_content_is_not_double_counted() {
    let body = format!(
        "{}<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"x\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>",
        shape("ok", ""),
        shape("SECRET1", ""),
        shape("SECRET2", "")
    );
    let out = read(&base(&body));
    assert_eq!(out.text, "[Slide 1]\n[Text]\nok");
    assert!(out.summary.left_out.contains(&"alternate content".into()));
}
#[test]
fn empty_alternate_content_and_timing_do_not_claim_omissions() {
    let mut parts = base(&format!(
        "{}<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"/>",
        shape("ok", "")
    ));
    change(
        &mut parts,
        "ppt/slides/slide1.xml",
        "</p:sld>",
        "<p:timing/><p:transition/></p:sld>",
    );
    assert_eq!(read(&parts).summary.status, TextStatus::Complete);
}
#[test]
fn presentation_order_not_filename_order_and_hidden_slides_included() {
    let mut parts = base(&shape("one", ""));
    parts.push(xml_part(
        "ppt/slides/slide2.xml",
        &slide(&shape("two", ""), "sld").replace("<p:sld ", "<p:sld show=\"0\" "),
    ));
    change(
        &mut parts,
        "[Content_Types].xml",
        "</Types>",
        &format!(
            "<Override PartName=\"/ppt/slides/slide2.xml\" ContentType=\"{SLIDE_TYPE}\"/></Types>"
        ),
    );
    change(
        &mut parts,
        "ppt/_rels/presentation.xml.rels",
        "</Relationships>",
        &format!(
            "{}</Relationships>",
            rel("s2", "slide", "slides/slide2.xml")
        ),
    );
    change(
        &mut parts,
        "ppt/presentation.xml",
        "<p:sldId id=\"256\" r:id=\"s1\"/>",
        "<p:sldId id=\"257\" r:id=\"s2\"/><p:sldId id=\"256\" r:id=\"s1\"/>",
    );
    let out = read(&parts);
    assert_eq!(out.summary.status, TextStatus::Complete);
    assert_eq!(out.text, "[Slide 1]\n[Text]\ntwo\n\n[Slide 2]\n[Text]\none");
}
#[test]
fn title_inherits_layout_placeholder_type() {
    let mut parts = base(&shape("title", "<p:ph idx=\"4\"/>"));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(&rel("l1", "slideLayout", "../slideLayouts/custom.xml")),
    ));
    parts.push(xml_part(
        "ppt/slideLayouts/custom.xml",
        &slide(
            &shape("TEMPLATE SAMPLE", "<p:ph type=\"title\" idx=\"4\"/>"),
            "sldLayout",
        ),
    ));
    change(
        &mut parts,
        "[Content_Types].xml",
        "</Types>",
        &format!(
            "<Override PartName=\"/ppt/slideLayouts/custom.xml\" ContentType=\"{LAYOUT_TYPE}\"/></Types>"
        ),
    );
    let out = read(&parts);
    assert_eq!(out.text, "[Slide 1]\n[Title]\ntitle");
    assert_eq!(out.summary.status, TextStatus::Complete);
    change(
        &mut parts,
        "ppt/slideLayouts/custom.xml",
        "<p:ph type=\"title\" idx=\"4\"/>",
        "",
    );
    assert_eq!(read(&parts).summary.status, TextStatus::Partial);
}
#[test]
fn notes_body_follows_slide_while_page_metadata_omitted() {
    let mut parts = base(&shape("body", ""));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(&rel("n1", "notesSlide", "../notesSlides/renamed.xml")),
    ));
    parts.push(xml_part(
        "ppt/notesSlides/renamed.xml",
        &slide(
            &format!(
                "{}{}",
                shape("note 中国", "<p:ph type=\"body\"/>"),
                shape("42", "<p:ph type=\"sldNum\"/>")
            ),
            "notes",
        ),
    ));
    change(
        &mut parts,
        "[Content_Types].xml",
        "</Types>",
        &format!(
            "<Override PartName=\"/ppt/notesSlides/renamed.xml\" ContentType=\"{NOTES_TYPE}\"/></Types>"
        ),
    );
    let out = read(&parts);
    valid(&out);
    assert_eq!(out.text, "[Slide 1]\n[Text]\nbody\n[Notes]\nnote 中国");
    assert!(
        out.summary
            .left_out
            .contains(&"notes page header/footer/date/number text".into())
    );
}
#[test]
fn unknown_or_duplicate_slide_relationship_fails() {
    for (before, after) in [
        ("r:id=\"s1\"", "r:id=\"absent\""),
        ("id=\"256\"", "id=\"2\""),
    ] {
        let mut parts = base(&shape("discard prefix", ""));
        change(&mut parts, "ppt/presentation.xml", before, after);
        bad(&parts, TextStatus::Failed);
    }
    let mut parts = base(&shape("discard prefix", ""));
    change(
        &mut parts,
        "ppt/presentation.xml",
        "</p:sldIdLst>",
        "<p:sldId id=\"257\" r:id=\"s1\"/></p:sldIdLst>",
    );
    bad(&parts, TextStatus::Failed);
}
#[test]
fn duplicate_relationship_id_fails() {
    let mut parts = base(&shape("prefix", ""));
    change(
        &mut parts,
        "ppt/_rels/presentation.xml.rels",
        "</Relationships>",
        &format!(
            "{}</Relationships>",
            rel("s1", "slide", "slides/slide1.xml")
        ),
    );
    bad(&parts, TextStatus::Failed);
}
#[test]
fn missing_internal_relationship_target_fails() {
    let mut parts = base(&shape("prefix", ""));
    change(
        &mut parts,
        "ppt/_rels/presentation.xml.rels",
        "slides/slide1.xml",
        "slides/absent.xml",
    );
    bad(&parts, TextStatus::Failed);
}
#[test]
fn relationship_target_containment_and_url_aliases() {
    for value in ["../../../../escape.xml", "//server/file.xml"] {
        let mut parts = base(&shape("prefix", ""));
        change(
            &mut parts,
            "ppt/_rels/presentation.xml.rels",
            "slides/slide1.xml",
            value,
        );
        bad(&parts, TextStatus::Failed);
    }
    for value in [
        "https://server/file.xml",
        "slides/%73lide1.xml",
        "slides/slide1.xml#fragment",
        "slides\\slide1.xml",
    ] {
        let mut parts = base(&shape("prefix", ""));
        change(
            &mut parts,
            "ppt/_rels/presentation.xml.rels",
            "slides/slide1.xml",
            value,
        );
        bad(&parts, TextStatus::Unsupported);
    }
}
#[test]
fn absolute_internal_and_case_varied_targets_supported() {
    let mut parts = base(&shape("ok", ""));
    change(
        &mut parts,
        "ppt/_rels/presentation.xml.rels",
        "slides/slide1.xml",
        "/PPT/SLIDES/SLIDE1.XML",
    );
    assert_eq!(read(&parts).summary.status, TextStatus::Complete);
}
#[test]
fn external_main_slide_notes_unsupported_without_access() {
    for name in ["_rels/.rels", "ppt/_rels/presentation.xml.rels"] {
        let mut parts = base(&shape("prefix", ""));
        change(
            &mut parts,
            name,
            " Target=",
            " TargetMode=\"External\" Target=",
        );
        bad(&parts, TextStatus::Unsupported);
    }
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(
            &rel("n1", "notesSlide", "https://invalid.example/never-read")
                .replace(" Target=", " TargetMode=\"External\" Target="),
        ),
    ));
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn external_image_partial_and_hyperlink_label_complete() {
    let mut parts = base(&shape("label", ""));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(
            &rel("link", "hyperlink", "https://invalid.example/never-read")
                .replace(" Target=", " TargetMode=\"External\" Target="),
        ),
    ));
    assert_eq!(read(&parts).summary.status, TextStatus::Complete);
    change(
        &mut parts,
        "ppt/slides/_rels/slide1.xml.rels",
        "/hyperlink",
        "/image",
    );
    let out = read(&parts);
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(
        out.summary
            .left_out
            .contains(&"external content targets (never accessed)".into())
    );
}
#[test]
fn macro_content_type_and_vba_part_refused() {
    let mut parts = base(&shape("prefix", ""));
    change(
        &mut parts,
        "[Content_Types].xml",
        MAIN_TYPE,
        "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml",
    );
    bad(&parts, TextStatus::Unsupported);
    let mut parts = base(&shape("prefix", ""));
    parts.push(("ppt/vbaProject.bin".into(), vec![0]));
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn templates_shows_and_non_pptx_main_refused() {
    for kind in [
        "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml",
        "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    ] {
        let mut parts = base(&shape("prefix", ""));
        change(&mut parts, "[Content_Types].xml", MAIN_TYPE, kind);
        bad(&parts, TextStatus::Unsupported);
    }
}
#[test]
fn unreferenced_actual_slide_text_disclosed() {
    let mut parts = base(&shape("ok", ""));
    change(
        &mut parts,
        "ppt/presentation.xml",
        "<p:sldId id=\"256\" r:id=\"s1\"/>",
        "",
    );
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn corrupt_unused_xml_and_binary_crc_discard_prefix() {
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part("ppt/unused.xml", "<x>"));
    bad(&parts, TextStatus::Failed);
    let mut bytes = archive(&base(&shape("prefix", "")), CompressionMethod::Stored);
    bytes[50] ^= 1;
    without(&bytes, TextStatus::Failed);
}
#[test]
fn content_type_xml_without_xml_suffix_is_validated() {
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part("ppt/odd.dat", "<x>"));
    change(
        &mut parts,
        "[Content_Types].xml",
        "</Types>",
        "<Default Extension=\"dat\" ContentType=\"application/xml\"/></Types>",
    );
    bad(&parts, TextStatus::Failed);
}
#[test]
fn malformed_root_namespace_and_multiple_roots_fail() {
    for xml in [
        "<sld/>",
        "<p:sld xmlns:p=\"urn:fake\"/>",
        "<x/><x/>",
        "",
        "<p:sld xmlns:p=\"x\"><p:cSld></p:sld>",
    ] {
        let mut parts = base(&shape("prefix", ""));
        set(&mut parts, "ppt/slides/slide1.xml", xml);
        bad(&parts, TextStatus::Failed);
    }
}
#[test]
fn dtd_pi_and_utf16_refused() {
    for xml in [
        "<!DOCTYPE x [<!ENTITY e SYSTEM 'file:///never-read'>]><x/>",
        "<?mso-contentType x?><x/>",
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?><x/>",
    ] {
        let mut parts = base(&shape("prefix", ""));
        parts.push(xml_part("ppt/unused.xml", xml));
        bad(&parts, TextStatus::Unsupported);
    }
    let mut parts = base(&shape("prefix", ""));
    parts.push(("ppt/unused.xml".into(), vec![0xff, 0xfe, b'<', 0]));
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn xml_entity_and_duplicate_expanded_attributes_fail() {
    for xml in [
        "<x>&unknown;</x>",
        "<x xmlns:a=\"u\" xmlns:b=\"u\" a:z=\"1\" b:z=\"2\"/>",
        "<x a=\"1\"b=\"2\"/>",
        "<x>&#0;</x>",
        "<x>\u{0}</x>",
    ] {
        let mut parts = base(&shape("prefix", ""));
        parts.push(xml_part("ppt/unused.xml", xml));
        bad(&parts, TextStatus::Failed);
    }
}
#[test]
fn xml_depth_attribute_namespace_limits() {
    let deep = format!(
        "{}{}",
        "<x>".repeat(MAX_XML_DEPTH + 1),
        "</x>".repeat(MAX_XML_DEPTH + 1)
    );
    let attrs = format!(
        "<x {}/>",
        (0..65).map(|i| format!("a{i}=\"v\" ")).collect::<String>()
    );
    let uri = format!("<x xmlns=\"{}\"/>", "u".repeat(1025));
    let name = format!("<{} />", "x".repeat(257));
    for value in [deep, attrs, uri, name] {
        let mut parts = base(&shape("prefix", ""));
        parts.push(xml_part("ppt/unused.xml", &value));
        bad(&parts, TextStatus::Unsupported);
    }
}
#[test]
fn xml_part_event_and_namespace_work_budgets() {
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part(
        "ppt/unused.xml",
        &format!("<x>{}</x>", " ".repeat(MAX_XML_BYTES)),
    ));
    bad(&parts, TextStatus::Unsupported);
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part(
        "ppt/unused.xml",
        &format!("<x>{}</x>", "<a/>".repeat(MAX_XML_EVENTS)),
    ));
    bad(&parts, TextStatus::Unsupported);
    let bindings = (0..60)
        .map(|i| format!(" xmlns:n{i}=\"u{i}\""))
        .collect::<String>();
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part(
        "ppt/unused.xml",
        &format!("<x{bindings}>{}</x>", "<a/>".repeat(17000)),
    ));
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn slide_count_bound() {
    let mut parts = base(&shape("prefix", ""));
    let ids = (0..=MAX_SLIDES)
        .map(|i| format!("<p:sldId id=\"{}\" r:id=\"r{i}\"/>", 256 + i))
        .collect::<String>();
    set(
        &mut parts,
        "ppt/presentation.xml",
        &format!(
            "<p:presentation xmlns:p=\"{P}\" xmlns:r=\"{R}\"><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"
        ),
    );
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn text_cap_is_unicode_safe_and_exact_tail_count() {
    let parts = base(&shape(&"中".repeat(MAX_MEMORY_TEXT_CHARS + 7), ""));
    let out = read(&parts);
    valid(&out);
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.summary.chars, MAX_MEMORY_TEXT_CHARS);
    assert!(
        out.summary
            .left_out
            .iter()
            .any(|s| s == "the last 24 characters of generated labels and supported text")
    );
}
#[test]
fn corruption_after_text_cap_still_fails_without_prefix() {
    let mut parts = base(&shape(&"a".repeat(MAX_MEMORY_TEXT_CHARS + 1), ""));
    parts.push(xml_part("ppt/tail.xml", "<broken>"));
    bad(&parts, TextStatus::Failed);
}
#[test]
fn zip_traversal_aliases_and_case_duplicates_fail() {
    for name in [
        "../evil",
        "/evil",
        "a/../evil",
        "a//evil",
        "a\\evil",
        "a:evil",
        "a%20evil",
        "a\u{0}evil",
        "évil",
    ] {
        let mut parts = base(&shape("prefix", ""));
        parts.push((name.into(), vec![0]));
        bad(&parts, TextStatus::Failed);
    }
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part("PPT/SLIDES/SLIDE1.XML", "<x/>"));
    bad(&parts, TextStatus::Failed);
}
#[test]
fn zip_local_central_inconsistency_and_crc_fail() {
    let source = archive(&base(&shape("prefix", "")), CompressionMethod::Stored);
    for offset in [6, 8, 14, 18, 22, 30] {
        let mut bytes = source.clone();
        bytes[offset] ^= 1;
        without(&bytes, TextStatus::Failed);
    }
}
#[test]
fn zip_comment_encryption_method_zip64_and_multi_volume_refused() {
    let source = archive(&base(&shape("prefix", "")), CompressionMethod::Stored);
    let c = central(&source);
    let mut bytes = source.clone();
    let e = bytes.len() - 22;
    set16(&mut bytes, e + 20, 1);
    bytes.push(0);
    without(&bytes, TextStatus::Unsupported);
    for (offset, value) in [(c + 8, 1), (c + 10, 99), (c + 34, 1)] {
        let mut bytes = source.clone();
        set16(&mut bytes, offset, value);
        without(&bytes, TextStatus::Unsupported);
    }
    let mut bytes = source.clone();
    set32(&mut bytes, c + 20, u32::MAX);
    without(&bytes, TextStatus::Unsupported);
    let mut bytes = source;
    let e = bytes.len() - 22;
    set16(&mut bytes, e + 4, 1);
    without(&bytes, TextStatus::Unsupported);
}
#[test]
fn zip_size_entries_expansion_and_ratio_limits() {
    without(&vec![0; MAX_PPTX_BYTES + 1], TextStatus::Unsupported);
    let mut parts = base(&shape("prefix", ""));
    for i in 0..MAX_ZIP_ENTRIES {
        parts.push((format!("empty{i}"), vec![]));
    }
    bad(&parts, TextStatus::Unsupported);
    let mut parts = base(&shape("prefix", ""));
    parts.push(("large.bin".into(), vec![0; MAX_ENTRY_BYTES + 1]));
    bad(&parts, TextStatus::Unsupported);
    let mut parts = base(&shape("prefix", ""));
    parts.push(("large.bin".into(), vec![0; MAX_ENTRY_BYTES]));
    without(
        &archive(&parts, CompressionMethod::Deflated),
        TextStatus::Unsupported,
    );
}
#[test]
fn zip_truncated_or_trailing_bytes_fail() {
    let bytes = archive(&base(&shape("prefix", "")), CompressionMethod::Stored);
    for cut in [0, 1, 20, bytes.len() / 2, bytes.len() - 1] {
        without(&bytes[..cut], TextStatus::Failed);
    }
    let mut bytes = bytes;
    bytes.extend_from_slice(b"tail");
    without(&bytes, TextStatus::Failed);
}
#[test]
fn encrypted_legacy_container_unsupported() {
    without(
        &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1],
        TextStatus::Unsupported,
    );
}
#[test]
fn summaries_and_debug_do_not_leak_content_or_external_target() {
    let secret = "PERSONAL-SENTINEL-SECRET";
    let mut parts = base(&shape(secret, ""));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(
            &rel("x", "image", "https://invalid.example/SECRET-URL")
                .replace(" Target=", " TargetMode=\"External\" Target="),
        ),
    ));
    let out = read(&parts);
    assert!(!format!("{out:?}").contains(secret));
    assert!(!format!("{:?}", out.summary).contains("SECRET-URL"));
    change(
        &mut parts,
        "ppt/slides/slide1.xml",
        "</p:sld>",
        "<broken></p:sld>",
    );
    let out = read(&parts);
    assert!(!format!("{out:?}").contains(secret));
}
// Core's dispatch of .pptx to `pptx-1` is checked in core's reader_tests.rs
// (#28 wiring); this file is also compiled by E's validation crate.
#[test]
fn deterministic() {
    let parts = base(&shape("ok", ""));
    assert_eq!(read(&parts), read(&parts));
}
#[test]
fn synthetic_realistic_sources_match_expected() {
    let expected = include_str!("../tests/fixtures/pptx/expected.txt");
    for bytes in [
        include_bytes!("../tests/fixtures/pptx/basic-source.pptx").as_slice(),
        include_bytes!("../tests/fixtures/pptx/rich-source.pptx").as_slice(),
    ] {
        let out = read_pptx(bytes);
        valid(&out);
        assert_eq!(out.text, expected, "{:?}", out.summary);
    }
    let out = read_pptx(include_bytes!("../tests/fixtures/pptx/rich-source.pptx"));
    assert_eq!(out.summary.status, TextStatus::Partial);
    for category in [
        "images (including alternative text)",
        "tables",
        "charts",
        "animations/timing",
        "slide transitions",
    ] {
        assert!(
            out.summary.left_out.contains(&category.into()),
            "{:?}",
            out.summary
        );
    }
}

#[test]
fn empty_comments_do_not_claim_omission_and_nonempty_comments_do() {
    let mut parts = base(&shape("ok", ""));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(&rel("c", "comments", "../comments/renamed.xml")),
    ));
    parts.push(xml_part(
        "ppt/comments/renamed.xml",
        &format!("<p:cmLst xmlns:p=\"{P}\"/>"),
    ));
    assert_eq!(read(&parts).summary.status, TextStatus::Complete);
    set(
        &mut parts,
        "ppt/comments/renamed.xml",
        &format!("<p:cmLst xmlns:p=\"{P}\"><p:cm><p:text>SECRET COMMENT</p:text></p:cm></p:cmLst>"),
    );
    let out = read(&parts);
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.summary.left_out, vec!["comments"]);
    assert!(!out.text.contains("SECRET"));
}
#[test]
fn unknown_content_relationships_are_disclosed() {
    let mut parts = base(&shape("ok", ""));
    parts.push(xml_part("ppt/extension.xml", "<x/>"));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels("<Relationship Id=\"e\" Type=\"urn:foreign:story\" Target=\"../extension.xml\"/>"),
    ));
    let out = read(&parts);
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(
        out.summary
            .left_out
            .contains(&"unknown content relationship targets".into())
    );
}
#[test]
fn zip_symlinks_and_namespace_binding_limit_refused() {
    let mut bytes = archive(&base(&shape("prefix", "")), CompressionMethod::Stored);
    let c = central(&bytes);
    bytes[c + 5] = 3;
    set32(&mut bytes, c + 38, 0o120777 << 16);
    without(&bytes, TextStatus::Unsupported);
    let names = (0..60)
        .map(|i| format!(" xmlns:n{i}=\"u{i}\""))
        .collect::<String>();
    // Distinct prefixes at each nested scope exceed the parser's active-binding cap.
    let deep = (0..5)
        .map(|d| format!("<x{}>", names.replace("xmlns:n", &format!("xmlns:d{d}n"))))
        .collect::<String>()
        + &"</x>".repeat(5);
    let mut parts = base(&shape("prefix", ""));
    parts.push(xml_part("ppt/unused.xml", &deep));
    bad(&parts, TextStatus::Unsupported);
}
#[test]
fn docx_adversarial_container_seeds_reused_before_presentation_parsing() {
    for (bytes, note) in [
        (
            include_bytes!("../tests/fixtures/docx/nested-eocd.docx").as_slice(),
            "archive comments",
        ),
        (
            include_bytes!("../tests/fixtures/docx/aes-method-override.docx").as_slice(),
            "AES ZIP metadata",
        ),
        (
            include_bytes!("../tests/fixtures/docx/zip-unicode-path-alias.docx").as_slice(),
            "Unicode ZIP path",
        ),
    ] {
        let out = read_pptx(bytes);
        valid(&out);
        assert_eq!(out.summary.status, TextStatus::Unsupported);
        assert!(out.summary.note.unwrap().contains(note));
    }
}
#[test]
fn every_truncation_and_deterministic_byte_mutations_never_panic_or_leak_failed_prefixes() {
    let bytes = archive(&base(&shape("SAFE TEXT", "")), CompressionMethod::Deflated);
    for i in 0..bytes.len() {
        let out = read_pptx(&bytes[..i]);
        valid(&out);
        assert!(out.text.is_empty());
    }
    for i in (0..bytes.len()).step_by(3) {
        let mut mutant = bytes.clone();
        mutant[i] ^= 0x80;
        valid(&read_pptx(&mutant));
    }
}

#[test]
fn actual_powerpoint_saves_are_golden_partial_not_complete() {
    for (bytes, expected) in [
        (
            include_bytes!("../tests/fixtures/pptx/powerpoint-basic-v1.pptx").as_slice(),
            include_str!("../tests/fixtures/pptx/expected.txt"),
        ),
        (
            include_bytes!("../tests/fixtures/pptx/powerpoint-basic-v2.pptx").as_slice(),
            include_str!("../tests/fixtures/pptx/expected-v2.txt"),
        ),
        (
            include_bytes!("../tests/fixtures/pptx/powerpoint-rich.pptx").as_slice(),
            include_str!("../tests/fixtures/pptx/expected.txt"),
        ),
        (
            include_bytes!("../tests/fixtures/pptx/powerpoint-rich-clean.pptx").as_slice(),
            include_str!("../tests/fixtures/pptx/expected.txt"),
        ),
    ] {
        let out = read_pptx(bytes);
        valid(&out);
        assert_eq!(out.text, expected);
        assert_eq!(out.summary.status, TextStatus::Partial);
    }
    let out = read_pptx(include_bytes!(
        "../tests/fixtures/pptx/powerpoint-rich.pptx"
    ));
    assert_eq!(
        out.summary.left_out,
        vec![
            "animations/timing",
            "charts",
            "dynamic fields",
            "images (including alternative text)",
            "slide transitions",
            "tables"
        ]
    );
}
#[test]
fn stored_content_retention_in_actual_powerpoint_rich_fixture() {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(Cursor::new(include_bytes!(
        "../tests/fixtures/pptx/powerpoint-rich.pptx"
    )))
    .unwrap();
    let mut xml = String::new();
    zip.by_name("ppt/slides/slide2.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    for marker in [
        "<p:pic>",
        "<c:chart ",
        "<a:tbl>",
        "<p:timing>",
        "<p:animEffect ",
        "<p:transition",
        "OMITTED TABLE",
    ] {
        assert!(xml.contains(marker), "missing {marker}");
    }
}
#[test]
fn package_total_expansion_cap_precedes_text_extraction() {
    let mut state = 123u32;
    let filler: Vec<u8> = (0..7 * 1024 * 1024)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 30) as u8
        })
        .collect();
    let mut parts = base(&shape("prefix", ""));
    for i in 0..5 {
        parts.push((format!("ppt/data{i}.bin"), filler.clone()));
    }
    let out = read_pptx(&archive(&parts, CompressionMethod::Deflated));
    valid(&out);
    assert_eq!(out.summary.status, TextStatus::Unsupported);
    assert!(out.summary.note.unwrap().contains("32 MiB"));
}
#[test]
fn deflate_trailing_bytes_and_incomplete_stream_fail() {
    let bytes = archive(&base(&shape("prefix", "")), CompressionMethod::Deflated);
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let file = zip.by_name("ppt/slides/slide1.xml").unwrap();
    let local = file.header_start() as usize;
    let central = file.central_header_start() as usize;
    let data = file.data_start().unwrap() as usize;
    let packed = file.compressed_size() as usize;
    drop(file);
    drop(zip);
    let mut extra = bytes.clone();
    extra.insert(data + packed, 0);
    set32(&mut extra, local + 18, (packed + 1) as u32);
    set32(&mut extra, central + 1 + 20, (packed + 1) as u32);
    let end = extra.len() - 22;
    let old_c = u32::from_le_bytes(extra[end + 16..end + 20].try_into().unwrap());
    set32(&mut extra, end + 16, old_c + 1);
    without(&extra, TextStatus::Failed);
    let mut incomplete = bytes;
    incomplete[data + packed - 1] ^= 0xff;
    without(&incomplete, TextStatus::Failed);
}

#[test]
fn unknown_inline_same_namespace_text_is_disclosed() {
    let body = shape("ok", "").replace(
        "</p:txBody>",
        "<p:unhandled>SECRET INLINE</p:unhandled></p:txBody>",
    );
    let out = read(&base(&body));
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert_eq!(out.text, "[Slide 1]\n[Text]\nok");
    assert!(
        out.summary
            .left_out
            .contains(&"text outside supported shape paragraphs".into())
    );
}
#[test]
fn timed_slide_advance_is_disclosed_without_effect_child() {
    let mut parts = base(&shape("ok", ""));
    change(
        &mut parts,
        "ppt/slides/slide1.xml",
        "</p:sld>",
        "<p:transition advTm=\"2000\"/></p:sld>",
    );
    let out = read(&parts);
    assert_eq!(out.summary.status, TextStatus::Partial);
    assert!(out.summary.left_out.contains(&"slide transitions".into()));
}

#[test]
fn inconsistent_notes_backlink_fails_and_external_notes_master_refused() {
    let mut parts = base(&shape("body", ""));
    parts.push(xml_part(
        "ppt/slides/_rels/slide1.xml.rels",
        &rels(&rel("n1", "notesSlide", "../notesSlides/notes1.xml")),
    ));
    parts.push(xml_part(
        "ppt/notesSlides/notes1.xml",
        &slide(&shape("note", "<p:ph type=\"body\"/>"), "notes"),
    ));
    change(
        &mut parts,
        "[Content_Types].xml",
        "</Types>",
        &format!(
            "<Override PartName=\"/ppt/notesSlides/notes1.xml\" ContentType=\"{NOTES_TYPE}\"/></Types>"
        ),
    );
    parts.push(xml_part(
        "ppt/notesSlides/_rels/notes1.xml.rels",
        &rels(&rel("back", "slide", "../presentation.xml")),
    ));
    bad(&parts, TextStatus::Failed);
    set(
        &mut parts,
        "ppt/notesSlides/_rels/notes1.xml.rels",
        &rels(
            &rel(
                "master",
                "notesMaster",
                "https://invalid.example/never-read",
            )
            .replace(" Target=", " TargetMode=\"External\" Target="),
        ),
    );
    bad(&parts, TextStatus::Unsupported);
}
