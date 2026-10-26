use super::{prototype::*, *};
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub fn package(body: &str, extras: &[(&str, &[u8])]) -> Vec<u8> {
    let xml = format!("<w:document xmlns:w=\"{W}\"><w:body>{body}</w:body></w:document>");
    let mut parts = vec![
        ("[Content_Types].xml", b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>".as_slice()),
        ("_rels/.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"r1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".as_slice()),
        ("word/document.xml", xml.as_bytes()),
    ];
    parts.extend_from_slice(extras);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn text(t: &str) -> String {
    format!("<w:r><w:t>{t}</w:t></w:r>")
}
fn marker(t: &str) -> String {
    format!("<w:r><w:fldChar w:fldCharType=\"{t}\"/></w:r>")
}
fn field(code: &str, result: &str) -> String {
    format!(
        "{}<w:r><w:instrText>{code}</w:instrText></w:r>{}{result}{}",
        marker("begin"),
        marker("separate"),
        marker("end")
    )
}
fn read(runs: &str) -> TextExtraction {
    read_docx(&package(&format!("<w:p>{runs}</w:p>"), &[]))
}
fn no_text(runs: &str, status: TextStatus) {
    let r = read(runs);
    assert_eq!(r.summary.status, status, "{:?}", r.summary);
    assert!(r.text.is_empty());
}
#[test]
fn plain_and_empty_paragraphs_remain_complete() {
    let r = read_docx(&package(
        "<w:p/><w:p><w:r><w:t>中国 🙂</w:t></w:r></w:p><w:p/>",
        &[],
    ));
    assert_eq!(r.text, "\n中国 🙂\n");
    assert_eq!(r.summary.status, TextStatus::Complete);
}
#[test]
fn citation_cache_excludes_code_and_json_metadata() {
    for code in [
        "CITATION Smith2020",
        "ADDIN ZOTERO_ITEM CSL_CITATION {SECRET_METADATA}",
        "ADDIN EN.CITE SECRET_ENDNOTE_METADATA",
    ] {
        let r = read(&field(code, &text("(Smith, 2020)")));
        assert_eq!(r.text, "(Smith, 2020)");
        assert_eq!(r.summary.status, TextStatus::Partial);
        assert!(r.summary.note.as_ref().unwrap().contains("stale"));
        assert!(!format!("{:?}", r).contains("SECRET"));
    }
}
#[test]
fn nested_result_under_result_is_visible() {
    assert_eq!(
        read(&field(
            "OUTER",
            &(text("[") + &field("INNER", &text("inner")) + &text("]"))
        ))
        .text,
        "[inner]"
    );
}
#[test]
fn nested_result_under_outer_code_cannot_leak() {
    let runs = format!(
        "{}{}{}{}{}{}",
        marker("begin"),
        field("INNER", &text("SECRET_CODE_RESULT")),
        text("SECRET_CODE_T"),
        marker("separate"),
        text("shown"),
        marker("end")
    );
    assert_eq!(read(&runs).text, "shown");
}
#[test]
fn instruction_elements_in_result_stay_excluded() {
    let r = read(&field(
        "CITATION A",
        "<w:r><w:instrText>SECRET</w:instrText><w:delInstrText>SECRET_OLD</w:delInstrText><w:t>visible</w:t></w:r>",
    ));
    assert_eq!(r.text, "visible");
}
#[test]
fn missing_separator_has_no_result() {
    let r = read(&(marker("begin") + &text("CODE_ONLY") + &marker("end")));
    assert_eq!(r.summary.status, TextStatus::Unsupported);
    assert!(r.text.is_empty());
    assert!(r.summary.note.unwrap().contains("without retained"));
}
#[test]
fn empty_result_not_invented() {
    no_text(&field("CITATION absent", ""), TextStatus::Unsupported);
}
#[test]
fn dirty_cache_retained_but_disclosed() {
    let s = field("CITATION changed", &text("OLD_CITATION")).replacen(
        "fldCharType=\"begin\"",
        "fldCharType=\"begin\" w:dirty=\"true\"",
        1,
    );
    let r = read(&s);
    assert_eq!(r.text, "OLD_CITATION");
    assert!(
        r.summary
            .left_out
            .iter()
            .any(|s| s.contains("marked dirty"))
    );
}
#[test]
fn unmarked_cache_freshness_still_unknown() {
    let r = read(&field("CITATION new", &text("OLD_CITATION")));
    assert!(
        r.summary
            .left_out
            .iter()
            .any(|s| s.contains("freshness (unknown"))
    );
}
#[test]
fn locked_field_never_evaluated() {
    let s = field("INCLUDETEXT file:///never/read", &text("cached")).replacen(
        "fldCharType=\"begin\"",
        "fldCharType=\"begin\" w:fldLock=\"true\"",
        1,
    );
    assert_eq!(read(&s).text, "cached");
}
#[test]
fn missing_end_discards_prefix_and_cache() {
    no_text(
        &(text("prefix") + &marker("begin") + &marker("separate") + &text("cached")),
        TextStatus::Failed,
    );
}
#[test]
fn orphan_and_duplicate_markers_fail() {
    for s in [
        marker("end"),
        marker("separate"),
        marker("begin") + &marker("separate") + &marker("separate") + &marker("end"),
        marker("unknown"),
    ] {
        no_text(&s, TextStatus::Failed);
    }
}
#[test]
fn fields_in_revisions_refused_as_a_whole() {
    for revision in ["ins", "del", "moveFrom", "moveTo"] {
        no_text(
            &format!(
                "{}<w:{revision}>{}</w:{revision}>",
                text("prefix"),
                field("CITATION A", &text("revision"))
            ),
            TextStatus::Unsupported,
        );
        no_text(
            &field(
                "CITATION A",
                &format!("<w:{revision}>{}</w:{revision}>", text("revision")),
            ),
            TextStatus::Unsupported,
        );
    }
}
#[test]
fn ordinary_revision_text_remains_omitted() {
    assert_eq!(
        read(&format!(
            "{}<w:ins>{}</w:ins>{}",
            text("before"),
            text("secret"),
            text("after")
        ))
        .text,
        "beforeafter"
    );
}
#[test]
fn field_in_table_refused() {
    let bytes = package(
        &format!(
            "<w:tbl><w:tr><w:tc><w:p>{}</w:p></w:tc></w:tr></w:tbl>",
            field("CITATION A", &text("secret"))
        ),
        &[],
    );
    assert_eq!(read_docx(&bytes).summary.status, TextStatus::Unsupported);
}
#[test]
fn simple_field_metadata_remains_unsupported() {
    no_text(
        "<w:fldSimple w:instr=\"SECRET\"><w:r><w:t>cached</w:t></w:r></w:fldSimple>",
        TextStatus::Unsupported,
    );
}
#[test]
fn custom_xml_metadata_is_not_body_text() {
    let bytes = package(
        &format!(
            "<w:p>{}</w:p>",
            field("ADDIN ZOTERO_ITEM SECRET", &text("shown"))
        ),
        &[(
            "customXml/item1.xml",
            b"<metadata>SECRET_INTERNAL</metadata>",
        )],
    );
    assert_eq!(read_docx(&bytes).text, "shown");
    assert!(!opening_reasons(&bytes).is_empty());
}
#[test]
fn flattened_manager_code_is_refused_even_across_runs() {
    for code in [
        "ADDIN ZOTERO_ITEM SECRET",
        "ADDIN ZOTERO_BIBL SECRET",
        "CSL_CITATION SECRET",
        "ADDIN EN.CITE SECRET",
    ] {
        no_text(&text(code), TextStatus::Unsupported);
        no_text(&field("OUTER", &text(code)), TextStatus::Unsupported);
    }
    no_text(
        &(text("ADDI") + &text("N ZOTERO_ITEM SECRET")),
        TextStatus::Unsupported,
    );
    no_text(&text("Addin  Zotero_Item SECRET"), TextStatus::Unsupported);
}
#[test]
fn property_revisions_remain_disclosed_and_do_not_supply_cache() {
    let p = package(
        "<w:p><w:pPr><w:pPrChange w:id=\"1\"><w:pPr/></w:pPrChange></w:pPr><w:r><w:t>plain</w:t></w:r></w:p>",
        &[],
    );
    let r = read_docx(&p);
    assert_eq!(r.text, "plain");
    assert_eq!(r.summary.status, TextStatus::Partial);
    assert!(
        r.summary
            .left_out
            .iter()
            .any(|s| s.contains("tracked changes"))
    );
    no_text(
        &field(
            "CITATION A",
            "<w:r><w:rPr><w:rPrChange w:id=\"1\"><w:rPr/></w:rPrChange></w:rPr><w:t>revised</w:t></w:r>",
        ),
        TextStatus::Unsupported,
    );
}
#[test]
fn cached_result_keeps_the_existing_text_budget() {
    let r = read(&field(
        "CITATION A",
        &text(&"x".repeat(MAX_MEMORY_TEXT_CHARS + 1)),
    ));
    assert_eq!(r.text.chars().count(), MAX_MEMORY_TEXT_CHARS);
    assert_eq!(r.summary.status, TextStatus::Partial);
    assert!(
        r.summary
            .left_out
            .iter()
            .any(|s| s.contains("last 1 characters"))
    );
    assert!(
        r.summary
            .left_out
            .iter()
            .any(|s| s.contains("freshness (unknown"))
    );
}
#[test]
fn cross_paragraph_result_keeps_structure() {
    let b = format!(
        "<w:p>{}{}{}</w:p><w:p>{}{}</w:p>",
        marker("begin"),
        marker("separate"),
        text("one"),
        text("two"),
        marker("end")
    );
    assert_eq!(read_docx(&package(&b, &[])).text, "one\ntwo");
}
#[test]
fn controls_and_entities_in_cache_are_text_only() {
    let r = read(&field(
        "CITATION A",
        "<w:r><w:t>A&amp;B &#x1f642;</w:t><w:tab/><w:t>x</w:t><w:br/><w:t>y</w:t></w:r>",
    ));
    assert_eq!(r.text, "A&B 🙂\tx\ny");
}
#[test]
fn field_stack_budget_is_bounded() {
    let s = marker("begin").repeat(33)
        + &marker("separate")
        + &text("cached")
        + &marker("end").repeat(33);
    no_text(&s, TextStatus::Unsupported);
}
#[test]
fn invalid_dirty_value_fails() {
    no_text(
        &field("X", &text("cache")).replacen(
            "fldCharType=\"begin\"",
            "fldCharType=\"begin\" w:dirty=\"maybe\"",
            1,
        ),
        TextStatus::Failed,
    );
}
#[test]
fn zip_xml_guards_still_refuse_corruption_and_dtd() {
    let mut bytes = package("<w:p/>", &[]);
    bytes.pop();
    assert_eq!(read_docx(&bytes).summary.status, TextStatus::Failed);
    let bytes = package(
        "<!DOCTYPE bomb [<!ENTITY x SYSTEM 'file:///secret'>]><w:p/>",
        &[],
    );
    assert!(read_docx(&bytes).text.is_empty());
}
#[test]
fn foreign_namespace_cannot_spoof_field_or_text() {
    no_text(
        "<x:t xmlns:x=\"urn:foreign\">SECRET</x:t>",
        TextStatus::Unsupported,
    );
}
#[test]
fn plain_opening_gate_only_research_subset() {
    assert!(opening_reasons(&package("<w:p><w:r><w:t>plain</w:t></w:r></w:p>", &[])).is_empty());
    assert!(
        !opening_reasons(&package(
            "<w:p><w:r><w:t>plain</w:t></w:r></w:p>",
            &[("word/embeddings/object1.bin", b"never execute")]
        ))
        .is_empty()
    );
}
#[test]
fn external_relationship_never_passes_opening_gate() {
    let rel=b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"x\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"http://127.0.0.1:65179/canary\" TargetMode=\"External\"/></Relationships>";
    let bytes = package("<w:p/>", &[("word/_rels/document.xml.rels", rel)]);
    assert!(opening_reasons(&bytes).contains(&"external relationship (not handed to Word)"));
}
#[test]
fn even_cached_reference_fields_refuse_system_open() {
    for code in [
        "CITATION A",
        "DDEAUTO never run",
        "INCLUDETEXT file:///never/read",
        "LINK never connect",
    ] {
        assert!(
            !opening_reasons(&package(
                &format!("<w:p>{}</w:p>", field(code, &text("cache"))),
                &[]
            ))
            .is_empty()
        );
    }
}
#[test]
fn update_and_template_flags_refuse_opening() {
    for s in [
        "<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:updateFields w:val=\"true\"/></w:settings>",
        "<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:attachedTemplate/></w:settings>",
    ] {
        assert!(
            !opening_reasons(&package("<w:p/>", &[("word/settings.xml", s.as_bytes())])).is_empty()
        );
    }
}

#[test]
fn opening_gate_refuses_final_unsupported_summary_r126_01() {
    let bytes = package("<x:fldChar xmlns:x=\"urn:foreign\"/>", &[]);
    let result = read_docx(&bytes);
    assert_eq!(result.summary.status, TextStatus::Unsupported);
    assert!(result.text.is_empty());
    assert_eq!(opening_reasons(&bytes), vec!["package validation refused"]);
    assert_eq!(
        opening_reasons(b"damaged"),
        vec!["package validation refused"]
    );
}
