//! Explicit local-only reader. The default read_docx entry remains conservative.
use super::*;

pub const DOCX_CACHED_READER: &str = "docx-cache-local-1";

/// Reads stored complex-field display text without evaluating or refreshing it.
/// This opt-in entry must not replace Memory/AI dispatch without separate review.
pub fn read_docx_cached(bytes: &[u8]) -> TextExtraction {
    let mut result = summarize(extract_with(bytes, body_text), DOCX_CACHED_READER);
    if matches!(
        result.summary.status,
        TextStatus::Complete | TextStatus::Partial
    ) {
        let cache = result
            .summary
            .left_out
            .iter()
            .any(|s| s.starts_with("field cache freshness"));
        let warning = if cache {
            " Stored field display text may be missing or stale; no field or citation database was evaluated or refreshed."
        } else {
            ""
        };
        let limit = result
            .summary
            .note
            .as_deref()
            .filter(|s| s.starts_with("The supported main-body text"))
            .unwrap_or_default();
        result.summary.note = Some(format!("Local text extraction only. Structurally marked field codes are omitted; unmarked flattened text cannot be identified reliably.{warning} {limit}").trim().into());
    }
    result
}

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
                    omissions.insert("structurally marked field codes and field metadata");
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
                                    "Complex fields exceed the 32-level limit.",
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
                                for field in &mut fields {
                                    field.visible_chars += 1;
                                }
                            }
                        }
                        "instrText" | "delInstrText" => {
                            omissions.insert("structurally marked field codes and field metadata");
                            skip = true;
                        }
                        "fldSimple" => {
                            omissions.insert("simple fields");
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
