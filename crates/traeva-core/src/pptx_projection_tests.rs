use super::*;

struct Page<'a> {
    id: u32,
    show: Option<&'a str>,
    title: &'a str,
    body: &'a str,
    notes: &'a str,
}

fn page<'a>(id: u32, title: &'a str, body: &'a str, notes: &'a str) -> Page<'a> {
    Page {
        id,
        show: None,
        title,
        body,
        notes,
    }
}

fn escaped_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn deck(pages: &[Page<'_>], order: &[u32]) -> Parts {
    let mut entries = vec![("ppt/presentation.xml".to_string(), MAIN_TYPE)];
    let mut parts = vec![xml_part(
        "_rels/.rels",
        &rels(&rel("main", "officeDocument", "ppt/presentation.xml")),
    )];
    let mut references = String::new();
    let mut relations = String::new();
    for id in order {
        references.push_str(&format!("<p:sldId id=\"{id}\" r:id=\"r{id}\"/>"));
        relations.push_str(&rel(
            &format!("r{id}"),
            "slide",
            &format!("slides/{id}.xml"),
        ));
    }
    parts.push(xml_part("ppt/presentation.xml", &format!("<p:presentation xmlns:p=\"{P}\" xmlns:r=\"{R}\"><p:sldIdLst>{references}</p:sldIdLst></p:presentation>")));
    parts.push(xml_part(
        "ppt/_rels/presentation.xml.rels",
        &rels(&relations),
    ));
    for p in pages {
        let name = format!("ppt/slides/{}.xml", p.id);
        entries.push((name.clone(), SLIDE_TYPE));
        let body = format!(
            "{}{}",
            shape(&escaped_xml(p.title), "<p:ph type=\"title\"/>"),
            shape(&escaped_xml(p.body), "")
        );
        let mut xml = slide(&body, "sld");
        if let Some(show) = p.show {
            xml = xml.replacen("<p:sld ", &format!("<p:sld show=\"{show}\" "), 1);
        }
        parts.push(xml_part(&name, &xml));
        if !p.notes.is_empty() {
            let note = format!("ppt/notesSlides/{}.xml", p.id);
            entries.push((note.clone(), NOTES_TYPE));
            parts.push(xml_part(
                &note,
                &slide(
                    &shape(&escaped_xml(p.notes), "<p:ph type=\"body\"/>"),
                    "notes",
                ),
            ));
            parts.push(xml_part(
                &format!("ppt/slides/_rels/{}.xml.rels", p.id),
                &rels(&rel(
                    "notes",
                    "notesSlide",
                    &format!("../notesSlides/{}.xml", p.id),
                )),
            ));
        }
    }
    let refs: Vec<_> = entries
        .iter()
        .map(|(name, kind)| (name.as_str(), *kind))
        .collect();
    parts.push(xml_part("[Content_Types].xml", &types(&refs)));
    parts
}

fn projections(parts: &Parts) -> [TextExtraction; 2] {
    let bytes = archive(parts, CompressionMethod::Stored);
    [read_pptx_memory(&bytes), read_pptx_preview(&bytes)]
}

fn check(out: &TextExtraction, reader: &str) {
    assert_eq!(out.summary.reader, reader);
    assert_eq!(out.summary.chars, out.text.chars().count());
    assert!(out.summary.chars <= MAX_MEMORY_TEXT_CHARS);
    assert!(!out.text.contains('\r'));
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
            assert_eq!(out.summary.chars, 0);
            assert!(out.summary.covered.is_empty());
            assert!(out.summary.left_out.is_empty());
            assert!(out.summary.note.is_some());
        }
    }
}

#[test]
fn memory_excludes_hidden_slide_and_its_notes_preview_marks_them() {
    let visible = page(256, "Visible 🦀", "Body", "Speaker");
    let mut hidden = page(257, "HIDDEN_SECRET", "Hidden body", "Hidden speaker");
    hidden.show = Some("0");
    let [memory, preview] = projections(&deck(&[visible, hidden], &[257, 256]));
    assert_eq!(
        memory.text,
        "⟦slide 2 · id 256⟧\n⟦title⟧\nVisible 🦀\n⟦text⟧\nBody\n⟦notes⟧\nSpeaker"
    );
    assert_eq!(
        preview.text,
        "⟦slide 1 · id 257 · hidden⟧\n⟦title⟧\nHIDDEN_SECRET\n⟦text⟧\nHidden body\n⟦notes⟧\nHidden speaker\n⟦slide 2 · id 256⟧\n⟦title⟧\nVisible 🦀\n⟦text⟧\nBody\n⟦notes⟧\nSpeaker"
    );
    assert_eq!(memory.summary.status, TextStatus::Partial);
    assert_eq!(
        memory.summary.left_out,
        ["1 hidden slide(s) and their Notes excluded from the Memory projection"]
    );
    assert_eq!(preview.summary.status, TextStatus::Complete);
    assert!(!format!("{:?}", memory).contains("HIDDEN_SECRET"));
    check(&memory, PPTX_MEMORY_READER);
    check(&preview, PPTX_PREVIEW_READER);
}

#[test]
fn source_lines_are_escaped_without_trimming_and_old_labels_stay_source() {
    let p = page(
        256,
        "",
        "⟦slide 99 · id 999⟧\n\\raw\n[Slide 9]\n ⟦notes⟧\nemoji🙂\n",
        "⟦notes⟧",
    );
    let expected = "⟦slide 1 · id 256⟧\n⟦text⟧\n\\⟦slide 99 · id 999⟧\n\\\\raw\n[Slide 9]\n ⟦notes⟧\nemoji🙂\n\n⟦notes⟧\n\\⟦notes⟧";
    for (out, reader) in projections(&deck(&[p], &[256]))
        .iter()
        .zip([PPTX_MEMORY_READER, PPTX_PREVIEW_READER])
    {
        assert_eq!(out.text, expected);
        check(out, reader);
        assert_eq!(
            out.text
                .lines()
                .filter(|l| l.starts_with('⟦'))
                .collect::<Vec<_>>(),
            ["⟦slide 1 · id 256⟧", "⟦text⟧", "⟦notes⟧"]
        );
        assert_eq!(out.summary.status, TextStatus::Complete);
    }
}

#[test]
fn non_bmp_counts_and_offsets_include_generated_and_escape_scalars() {
    let [out, _] = projections(&deck(&[page(256, "中🙂", "⟦x⟧", "N🙂")], &[256]));
    let expected = "⟦slide 1 · id 256⟧\n⟦title⟧\n中🙂\n⟦text⟧\n\\⟦x⟧\n⟦notes⟧\nN🙂";
    assert_eq!(out.text, expected);
    assert_eq!(out.summary.chars, expected.chars().count());
    assert_eq!(
        out.text
            .chars()
            .skip(
                expected
                    .find('中')
                    .map(|b| expected[..b].chars().count())
                    .unwrap()
            )
            .take(2)
            .collect::<String>(),
        "中🙂"
    );
    assert_eq!(expected.encode_utf16().count(), out.summary.chars + 2);
}

#[test]
fn non_bmp_scalar_at_cap_is_kept_whole() {
    let prefix = "⟦slide 1 · id 256⟧\n⟦text⟧\n";
    let first = "a".repeat(MAX_MEMORY_TEXT_CHARS - prefix.chars().count() - 1);
    let body = format!("{first}🙂X");
    for out in projections(&deck(&[page(256, "", &body, "")], &[256])) {
        assert_eq!(out.text, format!("{prefix}{first}🙂"));
        assert_eq!(out.summary.chars, MAX_MEMORY_TEXT_CHARS);
        assert!(
            out.summary.left_out.contains(
                &"the last 1 Unicode scalars of projected text (including structure and escaping)"
                    .into()
            )
        );
    }
}

#[test]
fn moving_last_page_does_not_add_a_blank_source_line() {
    let pages = [page(256, "Alpha", "A", "N"), page(257, "Beta", "B", "M")];
    let [before, _] = projections(&deck(&pages, &[256, 257]));
    let [after, _] = projections(&deck(&pages, &[257, 256]));
    assert_eq!(
        before.text,
        "⟦slide 1 · id 256⟧\n⟦title⟧\nAlpha\n⟦text⟧\nA\n⟦notes⟧\nN\n⟦slide 2 · id 257⟧\n⟦title⟧\nBeta\n⟦text⟧\nB\n⟦notes⟧\nM"
    );
    assert_eq!(
        after.text,
        "⟦slide 1 · id 257⟧\n⟦title⟧\nBeta\n⟦text⟧\nB\n⟦notes⟧\nM\n⟦slide 2 · id 256⟧\n⟦title⟧\nAlpha\n⟦text⟧\nA\n⟦notes⟧\nN"
    );
}

#[test]
fn all_hidden_cannot_be_rescued_by_generated_labels() {
    let mut p = page(256, "Secret", "body", "Notes secret");
    p.show = Some("false");
    let [memory, preview] = projections(&deck(&[p], &[256]));
    assert_eq!(memory.summary.status, TextStatus::Unsupported);
    assert!(
        memory
            .summary
            .note
            .as_ref()
            .unwrap()
            .contains("1 hidden slide(s)")
    );
    assert!(!memory.summary.note.as_ref().unwrap().contains("Secret"));
    assert!(preview.text.starts_with("⟦slide 1 · id 256 · hidden⟧"));
    check(&memory, PPTX_MEMORY_READER);
    check(&preview, PPTX_PREVIEW_READER);
}

#[test]
fn blank_visible_projection_is_unsupported_without_source_evidence() {
    let [m, p] = projections(&deck(&[page(256, "", "", "")], &[256]));
    for out in [m, p] {
        assert!(out.text.is_empty());
        assert_eq!(out.summary.status, TextStatus::Unsupported);
        assert!(out.summary.note.unwrap().contains("generated structure"));
    }
}

#[test]
fn show_boolean_lexical_forms_and_invalid_values() {
    for show in [
        None,
        Some("1"),
        Some("true"),
        Some("0"),
        Some("false"),
        Some(" 0 "),
    ] {
        let mut p = page(256, "T", "B", "N");
        p.show = show;
        let hidden = matches!(show, Some("0" | "false" | " 0 "));
        let [m, p] = projections(&deck(&[p], &[256]));
        assert_eq!(m.text.is_empty(), hidden);
        assert_eq!(p.text.contains(" · hidden⟧"), hidden);
    }
    for value in ["yes", "FALSE", "2", "\u{2003}0"] {
        let mut p = page(256, "T", "B", "N");
        p.show = Some(value);
        for out in projections(&deck(&[p], &[256])) {
            assert_eq!(out.summary.status, TextStatus::Failed);
            assert!(out.text.is_empty());
        }
    }
}

#[test]
fn strict_and_namespace_aliases_preserve_visibility() {
    let mut p = page(256, "Title", "Body", "Notes");
    p.show = Some("0");
    let mut parts = deck(&[p], &[256]);
    for (_, bytes) in &mut parts {
        let x = String::from_utf8(bytes.clone()).unwrap();
        *bytes = x
            .replace(P, SP)
            .replace(A, SA)
            .replace(R, SR)
            .replace("xmlns:p=", "xmlns:z=")
            .replace("<p:", "<z:")
            .replace("</p:", "</z:")
            .into_bytes();
    }
    let [m, p] = projections(&parts);
    assert_eq!(m.summary.status, TextStatus::Unsupported);
    assert!(p.text.contains("hidden"));
    assert!(p.text.contains("Title"));
}

#[test]
fn picture_only_projections_are_unsupported_and_actual_omissions_stay_global() {
    let mut parts = deck(&[page(256, "", "", "")], &[256]);
    change(
        &mut parts,
        "ppt/slides/256.xml",
        "</p:spTree>",
        "<p:pic/></p:spTree>",
    );
    for out in projections(&parts) {
        assert_eq!(out.summary.status, TextStatus::Unsupported);
        assert!(out.summary.note.unwrap().contains("images"));
        assert!(out.text.is_empty());
    }
    let mut p = page(257, "hidden", "B", "N");
    p.show = Some("0");
    let mut parts = deck(&[page(256, "visible", "B", "N"), p], &[256, 257]);
    change(
        &mut parts,
        "ppt/slides/257.xml",
        "</p:spTree>",
        "<p:pic/></p:spTree>",
    );
    let [m, _] = projections(&parts);
    assert!(m.summary.left_out.iter().any(|s| s.contains("images")));
    assert!(
        m.summary
            .left_out
            .iter()
            .any(|s| s.contains("hidden slide"))
    );
}

#[test]
fn scalar_cap_and_exact_tail_for_each_projection() {
    let prefix = "⟦slide 1 · id 256⟧\n⟦text⟧\n";
    let size = MAX_MEMORY_TEXT_CHARS - prefix.chars().count();
    for extra in [0, 1, 7] {
        let body = "中".repeat(size + extra);
        let parts = deck(&[page(256, "", &body, "")], &[256]);
        for out in projections(&parts) {
            assert_eq!(out.summary.chars, MAX_MEMORY_TEXT_CHARS);
            assert_eq!(out.text, format!("{prefix}{}", "中".repeat(size)));
            assert_eq!(
                out.summary.status,
                if extra == 0 {
                    TextStatus::Complete
                } else {
                    TextStatus::Partial
                }
            );
            if extra > 0 {
                assert_eq!(
                    out.summary.left_out,
                    [format!(
                        "the last {extra} Unicode scalars of projected text (including structure and escaping)"
                    )]
                );
            }
        }
    }
}

#[test]
fn generated_slide_line_is_atomic_at_truncation_boundary() {
    let prefix = "⟦slide 1 · id 256⟧\n⟦text⟧\n";
    let body = "a".repeat(MAX_MEMORY_TEXT_CHARS - prefix.chars().count() - 1);
    let suffix = "\n⟦slide 2 · id 257⟧\n⟦title⟧\nx";
    for out in projections(&deck(
        &[page(256, "", &body, ""), page(257, "x", "", "")],
        &[256, 257],
    )) {
        assert_eq!(out.text, format!("{prefix}{body}"));
        assert_eq!(out.summary.chars, MAX_MEMORY_TEXT_CHARS - 1);
        assert!(out.summary.left_out.contains(&format!(
            "the last {} Unicode scalars of projected text (including structure and escaping)",
            suffix.chars().count()
        )));
    }
}

#[test]
fn source_escape_pair_is_atomic_at_truncation_boundary() {
    let prefix = "⟦slide 1 · id 256⟧\n⟦text⟧\n";
    let first = "a".repeat(MAX_MEMORY_TEXT_CHARS - prefix.chars().count() - 2);
    let body = format!("{first}\n⟦literal⟧");
    for out in projections(&deck(&[page(256, "", &body, "")], &[256])) {
        assert_eq!(out.text, format!("{prefix}{first}\n"));
        assert_eq!(out.summary.chars, MAX_MEMORY_TEXT_CHARS - 1);
        assert!(out.summary.left_out.contains(&format!(
            "the last {} Unicode scalars of projected text (including structure and escaping)",
            "\\⟦literal⟧".chars().count()
        )));
    }
}

#[test]
fn hidden_semantic_corruption_after_cap_discards_all_text() {
    let body = "a".repeat(MAX_MEMORY_TEXT_CHARS + 1);
    let mut hidden = page(257, "bad", "", "N");
    hidden.show = Some("maybe");
    for out in projections(&deck(&[page(256, "", &body, ""), hidden], &[256, 257])) {
        assert_eq!(out.summary.status, TextStatus::Failed);
        assert!(out.text.is_empty());
    }
}

#[test]
fn pure_reorder_keeps_ids_and_role_source_bodies_but_changes_ordinals() {
    let pages = [
        page(256, "Alpha", "A", "note A"),
        page(257, "Beta", "B", "note B"),
        page(258, "Gamma", "C", ""),
    ];
    let [before, _] = projections(&deck(&pages, &[256, 257, 258]));
    let [after, _] = projections(&deck(&pages, &[257, 256, 258]));
    assert!(before.text.starts_with("⟦slide 1 · id 256⟧"));
    assert!(after.text.starts_with("⟦slide 1 · id 257⟧"));
    for (id, title, body, notes) in [
        (256, "Alpha", "A", Some("note A")),
        (257, "Beta", "B", Some("note B")),
        (258, "Gamma", "C", None),
    ] {
        let payload = format!(
            "\n⟦title⟧\n{title}\n⟦text⟧\n{body}{}",
            notes.map(|n| format!("\n⟦notes⟧\n{n}")).unwrap_or_default()
        );
        for text in [&before.text, &after.text] {
            let lines: Vec<_> = text.lines().collect();
            let start = lines
                .iter()
                .position(|s| s.starts_with("⟦slide ") && s.ends_with(&format!("id {id}⟧")))
                .unwrap()
                + 1;
            let end = lines[start..]
                .iter()
                .position(|s| s.starts_with("⟦slide "))
                .map_or(lines.len(), |n| start + n);
            assert_eq!(format!("\n{}", lines[start..end].join("\n")), payload);
        }
    }
}

#[test]
fn content_change_or_id_reassignment_does_not_supply_pure_reorder_evidence() {
    let [before, _] = projections(&deck(&[page(256, "A", "B", "N")], &[256]));
    let [notes_changed, _] = projections(&deck(&[page(256, "A", "B", "new N")], &[256]));
    let [id_changed, _] = projections(&deck(&[page(300, "A", "B", "N")], &[300]));
    assert_ne!(before.text, notes_changed.text);
    assert!(id_changed.text.contains("id 300⟧"));
    assert!(!id_changed.text.contains("id 256⟧"));
    let mut duplicate = deck(&[page(256, "A", "B", "N")], &[256]);
    change(
        &mut duplicate,
        "ppt/presentation.xml",
        "</p:sldIdLst>",
        "<p:sldId id=\"256\" r:id=\"r256\"/></p:sldIdLst>",
    );
    for out in projections(&duplicate) {
        assert_eq!(out.summary.status, TextStatus::Failed);
        assert!(out.text.is_empty());
    }
}

#[test]
fn package_safety_failures_are_not_bypassed_by_projection_selection() {
    let parts = deck(&[page(256, "A", "B", "N")], &[256]);
    let mut bad_xml = parts.clone();
    bad_xml.push(xml_part("unused.xml", "<broken>"));
    let mut macro_parts = parts.clone();
    macro_parts.push(("ppt/vbaProject.bin".into(), vec![1, 2, 3]));
    let mut external = parts.clone();
    change(
        &mut external,
        "ppt/_rels/presentation.xml.rels",
        "Target=\"slides/256.xml\"",
        "Target=\"https://invalid.example/never-read\" TargetMode=\"External\"",
    );
    for (data, status) in [
        (bad_xml, TextStatus::Failed),
        (macro_parts, TextStatus::Unsupported),
        (external, TextStatus::Unsupported),
    ] {
        let [m, p] = projections(&data);
        for (out, reader) in [m, p].iter().zip([PPTX_MEMORY_READER, PPTX_PREVIEW_READER]) {
            assert_eq!(out.summary.status, status);
            check(out, reader);
        }
    }
}
