//! Experimental bytes-only PPTX reader (#28); not registered in read_text.

use crate::{MAX_MEMORY_TEXT_CHARS, TextExtraction, TextStatus, TextSummary};
use std::collections::{BTreeMap, BTreeSet};

#[path = "pptx_package.rs"]
mod package;
#[path = "pptx_projection.rs"]
mod projection;
use package::{Element, XmlBudget, XmlEvent, xml, xml_whitespace};
pub use package::{
    MAX_COMPRESSION_RATIO, MAX_ENTRY_BYTES, MAX_NAMESPACE_WORK, MAX_PPTX_BYTES, MAX_UNPACKED_BYTES,
    MAX_XML_BYTES, MAX_XML_DEPTH, MAX_XML_EVENTS, MAX_ZIP_ENTRIES,
};
use projection::Projection;
pub use projection::{PPTX_MEMORY_READER, PPTX_PREVIEW_READER};

pub const PPTX_READER: &str = "pptx-experiment-1";
pub const MAX_SLIDES: usize = 200;
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const SP: &str = "http://purl.oclc.org/ooxml/presentationml/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const SA: &str = "http://purl.oclc.org/ooxml/drawingml/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SR: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const MAIN_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
const SLIDE_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const NOTES_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
const LAYOUT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";

#[derive(Debug)]
struct Problem(TextStatus, &'static str);
type Result<T> = std::result::Result<T, Problem>;
fn damaged() -> Problem {
    Problem(
        TextStatus::Failed,
        "The PPTX package, relationships or XML are damaged or inconsistent.",
    )
}
fn unsupported(note: &'static str) -> Problem {
    Problem(TextStatus::Unsupported, note)
}
fn is(e: &Element, ns: &str, name: &str) -> bool {
    e.name == name && (e.ns == ns || (ns == P && e.ns == SP) || (ns == A && e.ns == SA))
}
fn rid(e: &Element) -> Option<&str> {
    e.attr(&format!("{R}|id"))
        .or_else(|| e.attr(&format!("{SR}|id")))
}
fn relation_kind(kind: &str) -> &str {
    kind.strip_prefix(&format!("{R}/"))
        .or_else(|| kind.strip_prefix(&format!("{SR}/")))
        .unwrap_or("")
}

/// Produces stored text in presentation order, with generated slide/role labels.
/// Does not open paths, execute content, fetch targets, or write ZIP entries.
pub fn read_pptx(bytes: &[u8]) -> TextExtraction {
    read_projection(bytes, Projection::Legacy)
}

/// Visible slides and their Notes, for a future permission-gated Memory route.
/// Not registered in the production reader or any IPC command.
pub fn read_pptx_memory(bytes: &[u8]) -> TextExtraction {
    read_projection(bytes, Projection::Memory)
}

/// All stored slides and Notes, with hidden slides explicitly marked.
/// Does not grant sending permission or open a historical presentation.
pub fn read_pptx_preview(bytes: &[u8]) -> TextExtraction {
    read_projection(bytes, Projection::Preview)
}

fn read_projection(bytes: &[u8], mode: Projection) -> TextExtraction {
    match extract(bytes, mode) {
        Err(Problem(status, note)) => TextExtraction {
            summary: TextSummary {
                reader: mode.reader().into(),
                status,
                chars: 0,
                covered: vec![],
                left_out: vec![],
                note: Some(note.into()),
            },
            text: String::new(),
        },
        Ok(out)
            if out.source_chars == 0
                && (mode != Projection::Legacy || !out.omissions.is_empty()) =>
        {
            TextExtraction {
                summary: TextSummary {
                    reader: mode.reader().into(),
                    status: TextStatus::Unsupported,
                    chars: 0,
                    covered: vec![],
                    left_out: vec![],
                    note: Some(if mode != Projection::Legacy && out.omissions.is_empty() {
                        "No stored source text remains in this projection; generated structure is not source evidence.".into()
                    } else {
                        format!(
                            "No supported stored text remains. Omitted content: {}.",
                            out.omissions.into_iter().collect::<Vec<_>>().join(", ")
                        )
                    }),
                },
                text: String::new(),
            }
        }
        Ok(mut out) => {
            if out.total > out.kept {
                out.omissions.insert(if mode == Projection::Legacy {
                    format!("the last {} characters of generated labels and supported text", out.total - out.kept)
                } else {
                    format!("the last {} Unicode scalars of projected text (including structure and escaping)", out.total - out.kept)
                });
            }
            let partial = !out.omissions.is_empty();
            TextExtraction {
                summary: TextSummary { reader: mode.reader().into(),
                    status: if partial { TextStatus::Partial } else { TextStatus::Complete },
                    chars: out.text.chars().count(),
                    covered: vec![mode.coverage().into()],
                    left_out: out.omissions.into_iter().collect(),
                    note: partial.then(|| "Only the stated stored-text subset was read; see left_out. Visual order, formatting, layout, bullet numbering and slideshow behavior are not interpreted.".into()) },
                text: out.text,
            }
        }
    }
}

#[derive(Default)]
struct Output {
    text: String,
    total: usize,
    source_chars: usize,
    omissions: BTreeSet<String>,
    mode: Projection,
    selected: bool,
    kept: usize,
    cut: bool,
    hidden_slides: usize,
    emitted_slides: usize,
}
impl Output {
    fn append(&mut self, text: &str) {
        for c in text.chars() {
            if !self.cut && self.kept < MAX_MEMORY_TEXT_CHARS {
                self.text.push(c);
                self.kept += 1;
            } else {
                self.cut = true;
            }
            self.total += 1;
        }
    }
    fn shape(&mut self, role: &str, text: &str) {
        if text.is_empty() || !self.selected {
            return;
        }
        self.source_chars += text.chars().count();
        if self.mode == Projection::Legacy {
            self.append(&format!("\n[{role}]\n"));
            self.append(text);
        } else {
            self.project_shape(role, text);
        }
    }
}

fn target(owner: &str, value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 240
        || !value.is_ascii()
        || value
            .bytes()
            .any(|b| b <= 32 || b == 127 || matches!(b, b'\\' | b':' | b'%' | b'?' | b'#'))
    {
        return Err(unsupported(
            "This internal relationship target is unsupported.",
        ));
    }
    let mut pieces: Vec<&str> = if value.starts_with('/') {
        vec![]
    } else {
        owner
            .rsplit_once('/')
            .map_or(vec![], |(dir, _)| dir.split('/').collect())
    };
    for piece in value.strip_prefix('/').unwrap_or(value).split('/') {
        match piece {
            "." => (),
            ".." => {
                pieces.pop().ok_or_else(damaged)?;
            }
            "" => return Err(damaged()),
            _ => pieces.push(piece),
        }
    }
    if pieces.is_empty() {
        return Err(damaged());
    }
    Ok(pieces.join("/").to_ascii_lowercase())
}
fn rel_owner(name: &str) -> Result<String> {
    if name == "_rels/.rels" {
        return Ok(String::new());
    }
    let (dir, file) = name.rsplit_once("/_rels/").ok_or_else(damaged)?;
    let file = file.strip_suffix(".rels").ok_or_else(damaged)?;
    if dir.is_empty() || file.is_empty() || file.contains('/') {
        return Err(damaged());
    }
    Ok(format!("{dir}/{file}"))
}

#[derive(Clone)]
struct Relationship {
    kind: String,
    target: String,
    external: bool,
}
type Relations = BTreeMap<String, BTreeMap<String, Relationship>>;
fn relationships(parts: &BTreeMap<String, Vec<u8>>, budget: &mut XmlBudget) -> Result<Relations> {
    let mut all = BTreeMap::new();
    for (name, bytes) in parts.iter().filter(|(name, _)| name.ends_with(".rels")) {
        let owner = rel_owner(name)?;
        if !owner.is_empty() && !parts.contains_key(&owner) {
            return Err(damaged());
        }
        let mut map = BTreeMap::new();
        let mut stack = Vec::new();
        xml(bytes, budget, |event| {
            match event {
                XmlEvent::Start(e) => {
                    if stack.is_empty() {
                        if !is(&e, RELS, "Relationships") {
                            return Err(damaged());
                        }
                    } else {
                        if stack.len() != 1 || !is(&e, RELS, "Relationship") {
                            return Err(damaged());
                        }
                        let id = e
                            .attr("Id")
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or_else(damaged)?;
                        let kind = e
                            .attr("Type")
                            .filter(|s| !s.is_empty())
                            .ok_or_else(damaged)?;
                        let value = e
                            .attr("Target")
                            .filter(|s| !s.is_empty())
                            .ok_or_else(damaged)?;
                        let external = match e.attr("TargetMode") {
                            None | Some("Internal") => false,
                            Some("External") => true,
                            _ => return Err(damaged()),
                        };
                        if relation_kind(kind).eq_ignore_ascii_case("vbaProject") {
                            return Err(unsupported(
                                "Macro-bearing PowerPoint packages are unsupported.",
                            ));
                        }
                        let value = if external {
                            String::new()
                        } else {
                            target(&owner, value)?
                        };
                        if !external && !parts.contains_key(&value) {
                            return Err(damaged());
                        }
                        if map
                            .insert(
                                id.into(),
                                Relationship {
                                    kind: relation_kind(kind).into(),
                                    target: value,
                                    external,
                                },
                            )
                            .is_some()
                        {
                            return Err(damaged());
                        }
                    }
                    stack.push(e);
                }
                XmlEvent::End => {
                    stack.pop().ok_or_else(damaged)?;
                }
                XmlEvent::Text(t) if !xml_whitespace(&t) => return Err(damaged()),
                XmlEvent::Text(_) => (),
            }
            Ok(())
        })?;
        all.insert(owner, map);
    }
    Ok(all)
}

struct ContentTypes {
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
}
impl ContentTypes {
    fn get(&self, name: &str) -> Option<&str> {
        self.overrides
            .get(name)
            .or_else(|| {
                name.rsplit_once('.')
                    .and_then(|(_, ext)| self.defaults.get(ext))
            })
            .map(String::as_str)
    }
}
fn content_types(
    parts: &BTreeMap<String, Vec<u8>>,
    budget: &mut XmlBudget,
) -> Result<ContentTypes> {
    let mut types = ContentTypes {
        defaults: BTreeMap::new(),
        overrides: BTreeMap::new(),
    };
    let mut depth = 0;
    xml(
        parts.get("[content_types].xml").ok_or_else(damaged)?,
        budget,
        |event| {
            match event {
                XmlEvent::Start(e) => {
                    if depth == 0 {
                        if !is(&e, TYPES, "Types") {
                            return Err(damaged());
                        }
                    } else {
                        if depth != 1 || e.ns != TYPES {
                            return Err(damaged());
                        }
                        let kind = e
                            .attr("ContentType")
                            .filter(|s| !s.is_empty())
                            .ok_or_else(damaged)?;
                        let lower = kind.to_ascii_lowercase();
                        if lower.contains("macroenabled") || lower.contains("vbaproject") {
                            return Err(unsupported(
                                "Macro-bearing PowerPoint packages are unsupported.",
                            ));
                        }
                        let old = match e.name.as_str() {
                            "Default" => {
                                let ext = e
                                    .attr("Extension")
                                    .filter(|s| {
                                        !s.is_empty()
                                            && s.bytes().all(|b| b.is_ascii_alphanumeric())
                                    })
                                    .ok_or_else(damaged)?;
                                types.defaults.insert(ext.to_ascii_lowercase(), kind.into())
                            }
                            "Override" => {
                                let value = e
                                    .attr("PartName")
                                    .filter(|s| s.starts_with('/'))
                                    .ok_or_else(damaged)?;
                                let part = target("", value)?;
                                if !parts.contains_key(&part) {
                                    return Err(damaged());
                                }
                                types.overrides.insert(part, kind.into())
                            }
                            _ => return Err(damaged()),
                        };
                        if old.is_some() {
                            return Err(damaged());
                        }
                    }
                    depth += 1;
                }
                XmlEvent::End => {
                    depth -= 1;
                }
                XmlEvent::Text(t) if !xml_whitespace(&t) => return Err(damaged()),
                XmlEvent::Text(_) => (),
            }
            Ok(())
        },
    )?;
    // XML identity follows content type too, not only a filename extension.
    for (name, bytes) in parts {
        if !name.ends_with(".xml")
            && !name.ends_with(".rels")
            && types
                .get(name)
                .is_some_and(|t| t.ends_with("+xml") || t == "application/xml" || t == "text/xml")
        {
            xml(bytes, budget, |_| Ok(()))?;
        }
    }
    Ok(types)
}
fn unique_relation<'a>(
    relations: &'a Relations,
    owner: &str,
    kind: &str,
) -> Result<Option<&'a Relationship>> {
    let mut found = None;
    if let Some(map) = relations.get(owner) {
        for relation in map.values().filter(|r| r.kind == kind) {
            if found.is_some() {
                return Err(damaged());
            }
            if relation.external {
                return Err(unsupported(
                    "An external presentation, slide, layout or notes target is unsupported; no target was accessed.",
                ));
            }
            found = Some(relation);
        }
    }
    Ok(found)
}
struct SlideRef {
    id: u32,
    relationship: String,
}
fn slides(bytes: &[u8], budget: &mut XmlBudget) -> Result<Vec<SlideRef>> {
    let mut stack = Vec::new();
    let mut ids = BTreeSet::new();
    let mut rels = BTreeSet::new();
    let mut order = Vec::new();
    let mut list = false;
    xml(bytes, budget, |event| {
        match event {
            XmlEvent::Start(e) => {
                if stack.is_empty() && !is(&e, P, "presentation") {
                    return Err(damaged());
                }
                if is(&e, P, "sldIdLst") {
                    if stack.len() != 1 || list {
                        return Err(damaged());
                    }
                    list = true;
                }
                if is(&e, P, "sldId") {
                    if stack.len() != 2 || !stack.last().is_some_and(|p| is(p, P, "sldIdLst")) {
                        return Err(damaged());
                    }
                    let id: u32 = e
                        .attr("id")
                        .ok_or_else(damaged)?
                        .parse()
                        .map_err(|_| damaged())?;
                    let r = rid(&e).filter(|s| !s.is_empty()).ok_or_else(damaged)?;
                    if !(256..2_147_483_648).contains(&id)
                        || !ids.insert(id)
                        || !rels.insert(r.to_string())
                    {
                        return Err(damaged());
                    }
                    if order.len() == MAX_SLIDES {
                        return Err(unsupported("PPTX exceeds the 200-slide limit."));
                    }
                    order.push(SlideRef {
                        id,
                        relationship: r.to_string(),
                    });
                }
                stack.push(e);
            }
            XmlEvent::End => {
                stack.pop().ok_or_else(damaged)?;
            }
            XmlEvent::Text(t) if !xml_whitespace(&t) => return Err(damaged()),
            XmlEvent::Text(_) => (),
        }
        Ok(())
    })?;
    Ok(order)
}

#[derive(Default)]
struct Shape {
    text: String,
    ph: bool,
    role: Option<String>,
    index: String,
    paragraphs: usize,
}
fn shape_tree(
    bytes: &[u8],
    root: &str,
    budget: &mut XmlBudget,
    omissions: &mut BTreeSet<String>,
) -> Result<Vec<Shape>> {
    let mut stack: Vec<Element> = Vec::new();
    let mut shapes = Vec::new();
    let mut shape: Option<(usize, Shape)> = None;
    let mut skip: Option<(usize, &'static str)> = None;
    let mut tree = false;
    xml(bytes, budget, |event| {
        match event {
            XmlEvent::Start(e) => {
                if stack.is_empty() && !is(&e, P, root) {
                    return Err(damaged());
                }
                let parent = stack.last();
                let depth = stack.len();
                if is(&e, P, "spTree") {
                    if stack.len() != 2 || !parent.is_some_and(|p| is(p, P, "cSld")) || tree {
                        return Err(damaged());
                    }
                    tree = true;
                }
                let in_tree = stack.iter().any(|p| is(p, P, "spTree"));
                if skip.is_none() {
                    let category = if is(&e, P, "pic") {
                        Some("images (including alternative text)")
                    } else if is(&e, A, "tbl") {
                        Some("tables")
                    } else if e.name == "chart" && e.ns.contains("/chart") {
                        Some("charts")
                    } else if e.name == "relIds" && e.ns.contains("/diagram") {
                        Some("SmartArt/diagrams")
                    } else if is(&e, P, "oleObj") {
                        Some("embedded objects")
                    } else if is(&e, A, "fld") {
                        Some("dynamic fields")
                    } else if e.name == "AlternateContent"
                        && e.ns == "http://schemas.openxmlformats.org/markup-compatibility/2006"
                    {
                        Some("alternate content")
                    } else {
                        None
                    };
                    if let Some(category) = category {
                        // AlternateContent can be an empty compatibility container.
                        if category != "alternate content" {
                            omissions.insert(category.into());
                        }
                        skip = Some((depth, category));
                    } else if in_tree
                        && parent.is_some_and(|p| is(p, P, "spTree") || is(p, P, "grpSp"))
                        && ![
                            "sp",
                            "grpSp",
                            "nvGrpSpPr",
                            "grpSpPr",
                            "graphicFrame",
                            "cxnSp",
                            "extLst",
                        ]
                        .iter()
                        .any(|n| is(&e, P, n))
                    {
                        omissions.insert("unsupported shape content".into());
                        skip = Some((depth, "unsupported shape content"));
                    }
                } else if let Some((_, category)) = skip {
                    omissions.insert(category.into());
                }
                if is(&e, A, "blip") {
                    omissions.insert("images (including alternative text)".into());
                }
                if matches!(e.name.as_str(), "videoFile" | "audioFile" | "wavAudioFile")
                    && (e.ns == A || e.ns == SA)
                {
                    omissions.insert("audio/video".into());
                }
                if parent.is_some_and(|p| is(p, P, "timing")) {
                    omissions.insert("animations/timing".into());
                }
                if is(&e, P, "transition") && (e.attr("advTm").is_some() || e.attr("spd").is_some())
                {
                    omissions.insert("slide transitions".into());
                }
                if parent.is_some_and(|p| is(p, P, "transition")) {
                    omissions.insert("slide transitions".into());
                }
                if skip.is_none()
                    && is(&e, P, "sp")
                    && in_tree
                    && parent.is_some_and(|p| is(p, P, "spTree") || is(p, P, "grpSp"))
                {
                    if shape.is_some() {
                        return Err(damaged());
                    }
                    shape = Some((
                        depth,
                        Shape {
                            index: "0".into(),
                            ..Shape::default()
                        },
                    ));
                }
                if skip.is_none()
                    && let Some((shape_depth, s)) = &mut shape
                {
                    if is(&e, P, "ph")
                        && parent.is_some_and(|p| is(p, P, "nvPr"))
                        && stack.len() == *shape_depth + 3
                        && stack
                            .get(*shape_depth + 1)
                            .is_some_and(|p| is(p, P, "nvSpPr"))
                    {
                        if s.ph {
                            return Err(damaged());
                        }
                        s.ph = true;
                        s.role = e.attr("type").map(String::from);
                        s.index = e
                            .attr("idx")
                            .unwrap_or("0")
                            .parse::<u32>()
                            .map_err(|_| damaged())?
                            .to_string();
                    }
                    if is(&e, A, "p")
                        && parent.is_some_and(|p| is(p, P, "txBody"))
                        && stack.len() == *shape_depth + 2
                    {
                        if s.paragraphs > 0 {
                            s.text.push('\n');
                        }
                        s.paragraphs += 1;
                    }
                    if is(&e, A, "br")
                        && parent.is_some_and(|p| is(p, A, "p"))
                        && stack.len() == *shape_depth + 3
                    {
                        s.text.push('\n');
                    }
                }
                stack.push(e);
            }
            XmlEvent::End => {
                let depth = stack.len() - 1;
                if shape.as_ref().is_some_and(|(d, _)| *d == depth) {
                    shapes.push(shape.take().ok_or_else(damaged)?.1);
                }
                if skip.is_some_and(|(d, _)| d == depth) {
                    skip = None;
                }
                stack.pop().ok_or_else(damaged)?;
            }
            XmlEvent::Text(t) => {
                let supported = if let Some((d, _)) = &shape {
                    skip.is_none()
                        && stack.len() == *d + 5
                        && is(&stack[*d + 1], P, "txBody")
                        && is(&stack[*d + 2], A, "p")
                        && is(&stack[*d + 3], A, "r")
                        && is(&stack[*d + 4], A, "t")
                } else {
                    false
                };
                if supported {
                    shape.as_mut().ok_or_else(damaged)?.1.text.push_str(&t);
                } else if !xml_whitespace(&t) && stack.iter().any(|p| is(p, P, "timing")) {
                    omissions.insert("animations/timing".into());
                } else if !xml_whitespace(&t) && stack.iter().any(|p| is(p, P, "transition")) {
                    omissions.insert("slide transitions".into());
                } else if !xml_whitespace(&t)
                    && stack.last().is_some_and(|p| is(p, A, "t"))
                    && skip.is_none()
                {
                    omissions.insert("text outside supported shape paragraphs".into());
                } else if !xml_whitespace(&t)
                    && let Some((_, category)) = skip
                {
                    omissions.insert(category.into());
                } else if !xml_whitespace(&t)
                    && stack
                        .last()
                        .is_some_and(|p| p.ns != P && p.ns != SP && p.ns != A && p.ns != SA)
                {
                    omissions.insert("foreign text content".into());
                } else if !xml_whitespace(&t) {
                    omissions.insert("text outside supported shape paragraphs".into());
                }
            }
        }
        Ok(())
    })?;
    if !tree {
        return Err(damaged());
    }
    Ok(shapes)
}

fn feature_relations(
    relations: &Relations,
    parts: &BTreeMap<String, Vec<u8>>,
    budget: &mut XmlBudget,
    owner: &str,
    omissions: &mut BTreeSet<String>,
) -> Result<()> {
    if let Some(map) = relations.get(owner) {
        for r in map.values() {
            let category = match r.kind.as_str() {
                "image" => Some("images (including alternative text)"),
                "chart" | "chartEx" => Some("charts"),
                "diagramData" | "diagramDrawing" | "diagramLayout" | "diagramColors" => {
                    Some("SmartArt/diagrams")
                }
                "audio" | "video" | "media" => Some("audio/video"),
                "oleObject" | "package" | "control" => Some("embedded objects"),
                "comments" | "comment" if r.external => Some("comments"),
                "comments" | "comment" => {
                    let mut has_text = false;
                    xml(parts.get(&r.target).ok_or_else(damaged)?, budget, |event| {
                        if let XmlEvent::Text(t) = event {
                            has_text |= !xml_whitespace(&t);
                        }
                        Ok(())
                    })?;
                    has_text.then_some("comments")
                }
                "slide" | "slideLayout" | "slideMaster" | "notesSlide" | "notesMaster"
                | "theme" | "presProps" | "viewProps" | "tableStyles" | "hyperlink"
                | "commentAuthors" | "tags" | "printerSettings" => None,
                "customXml" => Some("custom XML content"),
                _ => Some("unknown content relationship targets"),
            };
            if let Some(c) = category {
                omissions.insert(c.into());
            }
            // External hyperlink labels are supported; the destination is never used.
            if r.external && r.kind != "hyperlink" {
                omissions.insert("external content targets (never accessed)".into());
            }
        }
    }
    Ok(())
}

fn extract(bytes: &[u8], mode: Projection) -> Result<Output> {
    let mut budget = XmlBudget::default();
    let parts = package::load(bytes, &mut budget)?;
    let types = content_types(&parts, &mut budget)?;
    let relations = relationships(&parts, &mut budget)?;
    let main = unique_relation(&relations, "", "officeDocument")?.ok_or_else(damaged)?;
    if types.get(&main.target) != Some(MAIN_TYPE) {
        return Err(unsupported(
            "Only ordinary PPTX presentation content types are supported (no templates, shows or macros).",
        ));
    }
    let order = slides(parts.get(&main.target).ok_or_else(damaged)?, &mut budget)?;
    let mut out = Output {
        mode,
        ..Output::default()
    };
    feature_relations(
        &relations,
        &parts,
        &mut budget,
        &main.target,
        &mut out.omissions,
    )?;
    let mut used_slides = BTreeSet::new();
    let mut used_notes = BTreeSet::new();
    let mut examined_layouts: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut examined_masters = BTreeSet::new();
    for (index, id) in order.iter().enumerate() {
        let r = relations
            .get(&main.target)
            .and_then(|r| r.get(&id.relationship))
            .ok_or_else(damaged)?;
        if r.kind != "slide" {
            return Err(damaged());
        }
        if r.external {
            return Err(unsupported(
                "External slide targets are unsupported; no target was accessed.",
            ));
        }
        if types.get(&r.target) != Some(SLIDE_TYPE) || !used_slides.insert(r.target.clone()) {
            return Err(damaged());
        }
        let mut roles = BTreeMap::new();
        if let Some(layout) = unique_relation(&relations, &r.target, "slideLayout")? {
            if types.get(&layout.target) != Some(LAYOUT_TYPE) {
                return Err(damaged());
            }
            if let Some(cached) = examined_layouts.get(&layout.target) {
                roles = cached.clone();
            } else {
                let shapes = shape_tree(
                    parts.get(&layout.target).ok_or_else(damaged)?,
                    "sldLayout",
                    &mut budget,
                    &mut out.omissions,
                )?;
                for s in shapes {
                    if !s.ph && !s.text.is_empty() {
                        out.omissions.insert(
                            "master/layout non-placeholder text (visibility not resolved)".into(),
                        );
                    }
                    if s.ph
                        && roles
                            .insert(s.index, s.role.unwrap_or_else(|| "obj".into()))
                            .is_some()
                    {
                        return Err(damaged());
                    }
                }
                feature_relations(
                    &relations,
                    &parts,
                    &mut budget,
                    &layout.target,
                    &mut out.omissions,
                )?;
                if let Some(master) = unique_relation(&relations, &layout.target, "slideMaster")?
                    && examined_masters.insert(master.target.clone())
                {
                    let shapes = shape_tree(
                        parts.get(&master.target).ok_or_else(damaged)?,
                        "sldMaster",
                        &mut budget,
                        &mut out.omissions,
                    )?;
                    if shapes.iter().any(|s| !s.ph && !s.text.is_empty()) {
                        out.omissions.insert(
                            "master/layout non-placeholder text (visibility not resolved)".into(),
                        );
                    }
                    feature_relations(
                        &relations,
                        &parts,
                        &mut budget,
                        &master.target,
                        &mut out.omissions,
                    )?;
                }
                examined_layouts.insert(layout.target.clone(), roles.clone());
            }
        }
        let shapes = shape_tree(
            parts.get(&r.target).ok_or_else(damaged)?,
            "sld",
            &mut budget,
            &mut out.omissions,
        )?;
        let hidden = mode != Projection::Legacy
            && projection::slide_hidden(parts.get(&r.target).ok_or_else(damaged)?, &mut budget)?;
        out.begin_slide(index + 1, id.id, hidden);
        for s in shapes {
            let role = s
                .role
                .as_deref()
                .or_else(|| roles.get(&s.index).map(String::as_str));
            out.shape(
                if s.ph && matches!(role, Some("title" | "ctrTitle")) {
                    "Title"
                } else {
                    "Text"
                },
                &s.text,
            );
        }
        feature_relations(
            &relations,
            &parts,
            &mut budget,
            &r.target,
            &mut out.omissions,
        )?;
        if let Some(notes) = unique_relation(&relations, &r.target, "notesSlide")? {
            if types.get(&notes.target) != Some(NOTES_TYPE)
                || !used_notes.insert(notes.target.clone())
            {
                return Err(damaged());
            }
            if let Some(back) = unique_relation(&relations, &notes.target, "slide")?
                && back.target != r.target
            {
                return Err(damaged());
            }
            if let Some(master) = unique_relation(&relations, &notes.target, "notesMaster")?
                && examined_masters.insert(master.target.clone())
            {
                let shapes = shape_tree(
                    parts.get(&master.target).ok_or_else(damaged)?,
                    "notesMaster",
                    &mut budget,
                    &mut out.omissions,
                )?;
                if shapes.iter().any(|s| !s.ph && !s.text.is_empty()) {
                    out.omissions.insert(
                        "notes master non-placeholder text (visibility not resolved)".into(),
                    );
                }
                feature_relations(
                    &relations,
                    &parts,
                    &mut budget,
                    &master.target,
                    &mut out.omissions,
                )?;
            }
            let shapes = shape_tree(
                parts.get(&notes.target).ok_or_else(damaged)?,
                "notes",
                &mut budget,
                &mut out.omissions,
            )?;
            for s in shapes {
                if s.ph
                    && matches!(
                        s.role.as_deref(),
                        Some("sldImg" | "sldNum" | "dt" | "hdr" | "ftr")
                    )
                {
                    if !s.text.is_empty() {
                        out.omissions
                            .insert("notes page header/footer/date/number text".into());
                    }
                } else if s.ph && s.role.as_deref() != Some("body") {
                    if !s.text.is_empty() {
                        out.omissions.insert("unknown notes placeholders".into());
                    }
                } else {
                    out.shape("Notes", &s.text);
                }
            }
            feature_relations(
                &relations,
                &parts,
                &mut budget,
                &notes.target,
                &mut out.omissions,
            )?;
        }
    }
    for (name, data) in &parts {
        let unused_root = if types.get(name) == Some(SLIDE_TYPE) && !used_slides.contains(name) {
            Some("sld")
        } else if types.get(name) == Some(NOTES_TYPE) && !used_notes.contains(name) {
            Some("notes")
        } else {
            None
        };
        if let Some(root) = unused_root {
            let mut unused_omissions = BTreeSet::new();
            let shapes = shape_tree(data, root, &mut budget, &mut unused_omissions)?;
            if shapes.iter().any(|s| !s.text.is_empty()) || !unused_omissions.is_empty() {
                out.omissions
                    .insert("unreferenced slide/notes content".into());
            }
        }
    }
    if out.hidden_slides > 0 {
        out.omissions.insert(format!(
            "{} hidden slide(s) and their Notes excluded from the Memory projection",
            out.hidden_slides
        ));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "pptx_tests.rs"]
mod tests;
