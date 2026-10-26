#[derive(Default)]
struct Field {
    result: bool,
    visible_chars: usize,
}

fn body_text(
    bytes: &[u8],
    budget: &mut XmlBudget,
    omissions: &mut BTreeSet<&'static str>,
) -> Result<(String, usize)> {
    let mut elements = Vec::<(Element, bool)>::new();
    let mut fields = Vec::<Field>::new();
    let mut output = String::new();
    let mut chars = 0;
    let mut bodies = 0;
    let mut paragraphs = 0;
    let mut pending_break = false;
    let mut previous_cr = false;
    let mut append = |s: &str, boundary: bool, omissions: &mut BTreeSet<&'static str>| {
        if boundary {
            previous_cr = false;
        }
        for c in s.chars() {
            if c == '\n' && previous_cr {
                previous_cr = false;
                continue;
            }
            previous_cr = c == '\r';
            if chars < MAX_MEMORY_TEXT_CHARS {
                output.push(if c == '\r' { '\n' } else { c });
            } else {
                omissions.insert("text beyond the 1,000,000-character limit");
            }
            chars += 1;
        }
    };
    xml(bytes, budget, |event| {
        match event {
            XmlEvent::Start(e) => {
                if elements.is_empty() && (!e.word() || e.name != "document") {
                    return Err(damaged());
                }
                let mut skip = elements.last().is_some_and(|(_, s)| *s);
                if e.word()
                    && matches!(
                        e.name.as_str(),
                        "ins"
                            | "del"
                            | "moveFrom"
                            | "moveTo"
                            | "pPrChange"
                            | "rPrChange"
                            | "sectPrChange"
                            | "tblPrChange"
                            | "trPrChange"
                            | "tcPrChange"
                            | "numberingChange"
                    )
                {
                    omissions.insert("tracked changes (no revision acceptance policy applied)");
                    if !fields.is_empty() {
                        return Err(unsupported(
                            "A complex field intersecting tracked revisions is unsupported.",
                        ));
                    }
                }
                if e.word() && e.name == "fldChar" {
                    // Markers in omitted/revised content can change the meaning
                    // of later visible runs. Refuse the whole document.
                    if skip {
                        return Err(unsupported(
                            "Field boundaries in omitted or revised content are unsupported.",
                        ));
                    }
                    if !elements
                        .last()
                        .is_some_and(|(p, _)| p.word() && p.name == "r")
                    {
                        return Err(damaged());
                    }
                    omissions.insert("field codes and citation-manager internal metadata");
                    omissions.insert("field cache freshness (unknown; no evaluation or refresh)");
                    if let Some(dirty) = e.word_attr("dirty") {
                        match dirty {
                            "1" | "true" | "on" => {
                                omissions.insert("field cache marked dirty (may be stale)");
                            }
                            "0" | "false" | "off" => (),
                            _ => return Err(damaged()),
                        }
                    }
                    match e.word_attr("fldCharType") {
                        Some("begin") => {
                            if fields.len() == 32 {
                                return Err(unsupported(
                                    "Complex fields exceed the 32-level research limit.",
                                ));
                            }
                            fields.push(Field::default());
                        }
                        Some("separate") => {
                            let f = fields.last_mut().ok_or_else(damaged)?;
                            if f.result {
                                return Err(damaged());
                            }
                            f.result = true;
                        }
                        Some("end") => {
                            let f = fields.pop().ok_or_else(damaged)?;
                            if !f.result || f.visible_chars == 0 {
                                omissions.insert("fields without retained cached display text");
                            }
                        }
                        _ => return Err(damaged()),
                    }
                    skip = true;
                } else if !skip && e.word() {
                    match e.name.as_str() {
                        "document" => {
                            if !elements.is_empty() {
                                return Err(damaged());
                            }
                        }
                        "body" => {
                            if elements.len() != 1 || elements[0].0.name != "document" {
                                return Err(damaged());
                            }
                            bodies += 1;
                            if bodies != 1 {
                                return Err(damaged());
                            }
                        }
                        "p" => {
                            if !elements
                                .last()
                                .is_some_and(|(p, _)| p.word() && p.name == "body")
                            {
                                return Err(damaged());
                            }
                            if paragraphs > 0 {
                                if fields.iter().all(|f| f.result) {
                                    append("\n", true, omissions);
                                } else {
                                    pending_break = true;
                                }
                            }
                            paragraphs += 1;
                        }
                        "r" => {
                            if !elements.last().is_some_and(|(p, _)| {
                                p.word() && matches!(p.name.as_str(), "p" | "hyperlink")
                            }) {
                                return Err(damaged());
                            }
                        }
                        "hyperlink" => {
                            if !elements
                                .last()
                                .is_some_and(|(p, _)| p.word() && p.name == "p")
                            {
                                return Err(damaged());
                            }
                        }
                        "t" => {
                            if !elements
                                .last()
                                .is_some_and(|(p, _)| p.word() && p.name == "r")
                            {
                                return Err(damaged());
                            }
                        }
                        "tab" | "br" | "cr" => {
                            if !elements
                                .last()
                                .is_some_and(|(p, _)| p.word() && p.name == "r")
                            {
                                return Err(damaged());
                            }
                            if fields.iter().all(|f| f.result) {
                                if pending_break {
                                    append("\n", true, omissions);
                                    pending_break = false;
                                }
                                append(if e.name == "tab" { "\t" } else { "\n" }, true, omissions);
                            }
                        }
                        "instrText" | "delInstrText" => {
                            omissions.insert("field codes and citation-manager internal metadata");
                            skip = true;
                        }
                        "fldSimple" => {
                            omissions
                                .insert("simple fields (outside this complex-field prototype)");
                            skip = true;
                        }
                        "pPr"
                        | "rPr"
                        | "sectPr"
                        | "bookmarkStart"
                        | "bookmarkEnd"
                        | "proofErr"
                        | "lastRenderedPageBreak" => skip = true,
                        "tbl" => {
                            omissions.insert("tables");
                            skip = true;
                        }
                        "ins" | "del" | "moveFrom" | "moveTo" => {
                            if !fields.is_empty() {
                                return Err(unsupported(
                                    "A complex field intersecting tracked revisions is unsupported.",
                                ));
                            }
                            omissions
                                .insert("tracked changes (no revision acceptance policy applied)");
                            skip = true;
                        }
                        "drawing" | "pict" | "object" | "txbxContent" => {
                            omissions.insert("images, embedded objects and text boxes");
                            skip = true;
                        }
                        "footnoteReference" | "endnoteReference" => {
                            omissions.insert("footnotes and endnotes");
                            skip = true;
                        }
                        "commentReference" | "commentRangeStart" | "commentRangeEnd" => {
                            omissions.insert("comments");
                            skip = true;
                        }
                        _ => {
                            omissions.insert("unsupported WordprocessingML elements");
                            skip = true;
                        }
                    }
                } else if !skip {
                    omissions.insert("foreign namespaces and alternate content");
                    skip = true;
                }
                elements.push((e, skip));
            }
            XmlEvent::End => {
                elements.pop().ok_or_else(damaged)?;
            }
            XmlEvent::Text(t) => {
                let visible = fields.iter().all(|f| f.result);
                if visible
                    && elements
                        .last()
                        .is_some_and(|(e, skip)| !skip && e.word() && e.name == "t")
                {
                    if pending_break {
                        append("\n", true, omissions);
                        pending_break = false;
                    }
                    append(&t, false, omissions);
                    for f in &mut fields {
                        f.visible_chars += t.chars().count();
                    }
                } else if !elements.last().is_some_and(|(_, skip)| *skip)
                    && visible
                    && !xml_whitespace(&t)
                {
                    return Err(damaged());
                }
            }
        }
        Ok(())
    })?;
    if bodies != 1 || !fields.is_empty() {
        return Err(damaged());
    }
    // Word may flatten malformed fields into w:t on save. Known manager
    // markers then lack structural provenance, so refuse rather than emit.
    let marker_text = output
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    if [
        "ADDIN ZOTERO_ITEM",
        "ADDIN ZOTERO_BIBL",
        "CSL_CITATION",
        "ADDIN EN.CITE",
    ]
    .iter()
    .any(|marker| marker_text.contains(marker))
    {
        return Err(unsupported(
            "Citation-manager field-code markers occur in display text; the document is conservatively refused.",
        ));
    }
    Ok((output, chars))
}

/// Research-only deny gate, never an authorization to call Word. Quarantine
/// and read-only mode do not make arbitrary DOCX active content safe.
pub fn opening_reasons(bytes: &[u8]) -> Vec<&'static str> {
    if matches!(read_docx(bytes).summary.status, TextStatus::Unsupported | TextStatus::Failed) {
        return vec!["package validation refused"];
    }
    let mut archive = match ZipArchive::new(Cursor::new(bytes)) {
        Ok(a) => a,
        Err(_) => return vec!["package validation refused"],
    };
    let mut reasons = BTreeSet::new();
    let mut budget = XmlBudget::default();
    const PARTS: &[&str] = &[
        "[content_types].xml",
        "_rels/.rels",
        "docprops/core.xml",
        "docprops/app.xml",
        "word/document.xml",
        "word/styles.xml",
        "word/settings.xml",
        "word/websettings.xml",
        "word/fonttable.xml",
        "word/numbering.xml",
        "word/theme/theme1.xml",
        "word/_rels/document.xml.rels",
    ];
    for i in 0..archive.len() {
        let Ok(mut part) = archive.by_index(i) else {
            return vec!["package validation refused"];
        };
        let name = part.name().to_ascii_lowercase();
        if !PARTS.contains(&name.as_str()) {
            reasons.insert("part outside the plain-document research subset");
        }
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let mut data = Vec::new();
            if (&mut part)
                .take(MAX_XML_BYTES as u64 + 1)
                .read_to_end(&mut data)
                .is_err()
            {
                return vec!["package validation refused"];
            }
            if xml(&data, &mut budget, |event| {
                if let XmlEvent::Start(e) = event {
                    if e.ns == RELS
                        && e.name == "Relationship"
                        && e.attr("TargetMode") == Some("External")
                    {
                        reasons.insert("external relationship (not handed to Word)");
                    }
                    if e.word()
                        && matches!(
                            e.name.as_str(),
                            "fldChar" | "fldSimple" | "instrText" | "delInstrText"
                        )
                    {
                        reasons.insert("fields may evaluate or access content in Word");
                    }
                    if e.word()
                        && matches!(
                            e.name.as_str(),
                            "object"
                                | "pict"
                                | "drawing"
                                | "altChunk"
                                | "subDoc"
                                | "attachedTemplate"
                                | "updateFields"
                        )
                    {
                        reasons.insert("active, linked or automatic-update content");
                    }
                }
                Ok(())
            })
            .is_err()
            {
                return vec!["package validation refused"];
            }
        }
    }
    reasons.into_iter().collect()
}
