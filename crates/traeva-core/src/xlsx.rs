//! Bytes-only XLSX parser and dual projections (#28); not registered in core lib.rs.

#[path = "xlsx_package.rs"]
mod package;
#[path = "xlsx_projection.rs"]
mod projection;
#[path = "xlsx_xml.rs"]
mod xml;
pub use projection::{
    ProjectionMode, XLSX_PREVIEW_READER, XLSX_PROJECTION_REVISION, XLSX_READER,
    project_verified_snapshot, read_xlsx_memory, read_xlsx_preview,
};

#[cfg(test)]
#[path = "xlsx_helper_tests.rs"]
mod helper_tests;
#[cfg(test)]
#[path = "xlsx_projection_tests.rs"]
mod projection_tests;

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use xml::Node;

const SS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const MAIN_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const SHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const STRINGS_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Failed,
    Unsupported,
    ResourceLimit,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub code: &'static str,
}

impl Error {
    fn failed(code: &'static str) -> Self {
        Self {
            kind: ErrorKind::Failed,
            code,
        }
    }
    fn unsupported(code: &'static str) -> Self {
        Self {
            kind: ErrorKind::Unsupported,
            code,
        }
    }
    fn limit(code: &'static str) -> Self {
        Self {
            kind: ErrorKind::ResourceLimit,
            code,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.code)
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Limits {
    pub input_bytes: usize,
    pub entries: usize,
    pub central_bytes: usize,
    pub entry_bytes: usize,
    pub expanded_bytes: usize,
    pub compression_ratio: usize,
    pub xml_event_bytes: usize,
    pub xml_depth: usize,
    pub xml_attributes: usize,
    pub xml_nodes: usize,
    pub sheets: usize,
    pub shared_strings: usize,
    pub cells: usize,
    pub text_bytes: usize,
    pub retained_text_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 8 * 1024 * 1024,
            entries: 256,
            central_bytes: 256 * 1024,
            entry_bytes: 4 * 1024 * 1024,
            expanded_bytes: 32 * 1024 * 1024,
            compression_ratio: 200,
            xml_event_bytes: 64 * 1024,
            xml_depth: 32,
            xml_attributes: 32,
            xml_nodes: 200_000,
            sheets: 32,
            shared_strings: 20_000,
            cells: 10_000,
            text_bytes: 32 * 1024,
            retained_text_bytes: 4 * 1024 * 1024,
        }
    }
}

impl Limits {
    fn validate(&self) -> Result<()> {
        let max = Self::default();
        let actual = [
            self.input_bytes,
            self.entries,
            self.central_bytes,
            self.entry_bytes,
            self.expanded_bytes,
            self.compression_ratio,
            self.xml_event_bytes,
            self.xml_depth,
            self.xml_attributes,
            self.xml_nodes,
            self.sheets,
            self.shared_strings,
            self.cells,
            self.text_bytes,
            self.retained_text_bytes,
        ];
        let ceiling = [
            max.input_bytes,
            max.entries,
            max.central_bytes,
            max.entry_bytes,
            max.expanded_bytes,
            max.compression_ratio,
            max.xml_event_bytes,
            max.xml_depth,
            max.xml_attributes,
            max.xml_nodes,
            max.sheets,
            max.shared_strings,
            max.cells,
            max.text_bytes,
            max.retained_text_bytes,
        ];
        if actual
            .iter()
            .zip(ceiling)
            .any(|(&v, cap)| v == 0 || v > cap)
        {
            return Err(Error::unsupported("limits_must_only_tighten_profile"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Number(String),
    Text(String),
    Boolean(bool),
    Error(String),
    IsoDate(String),
    Blank,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Formula {
    pub text: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub interpretation: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FormulaCache {
    pub value: Option<Value>,
    pub freshness: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Cell {
    pub address: String,
    pub row: u32,
    pub column: u32,
    pub style_index: Option<u32>,
    pub literal: Option<Value>,
    pub formula: Option<Formula>,
    pub cached_result: Option<FormulaCache>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Sheet {
    pub name: String,
    pub sheet_id: String,
    pub part: String,
    pub visibility: String,
    pub kind: String,
    pub declared_dimension: Option<String>,
    pub merged_ranges: Vec<String>,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub code: String,
    pub part: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Workbook {
    pub profile: &'static str,
    pub status: &'static str,
    pub input_sha256: String,
    pub date_1904: bool,
    pub calculation_properties: BTreeMap<String, String>,
    pub sheets: Vec<Sheet>,
    pub omissions: Vec<Finding>,
    pub limitations: Vec<&'static str>,
}

struct Budget<'a> {
    limits: &'a Limits,
    cell_count: usize,
    text_count: usize,
    findings: BTreeSet<Finding>,
}

impl Budget<'_> {
    fn finding(&mut self, code: &str, part: &str, detail: &str) {
        self.findings.insert(Finding {
            code: code.into(),
            part: part.into(),
            detail: detail.into(),
        });
    }
    fn text(&mut self, text: &str) -> Result<String> {
        self.text_count += text.len();
        if text.len() > self.limits.text_bytes || self.text_count > self.limits.retained_text_bytes
        {
            return Err(Error::limit("retained_text_limit"));
        }
        Ok(text.to_string())
    }
}

fn expect_root<'a>(
    roots: &'a BTreeMap<String, Node>,
    part: &str,
    ns: &str,
    name: &str,
) -> Result<&'a Node> {
    let node = roots
        .get(part)
        .ok_or_else(|| Error::failed("required_xml_part_missing"))?;
    if !node.is(ns, name) {
        return Err(Error::unsupported("unsupported_xml_root_or_namespace"));
    }
    Ok(node)
}

fn bool_attr(value: Option<&str>, default: bool) -> Result<bool> {
    match value {
        None => Ok(default),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        _ => Err(Error::failed("invalid_boolean_attribute")),
    }
}

fn index(value: &str) -> Result<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::failed("invalid_unsigned_integer"));
    }
    value.parse().map_err(|_| Error::failed("integer_overflow"))
}

fn address(value: &str) -> Result<(u32, u32)> {
    let split = value
        .find(|c: char| !c.is_ascii_uppercase())
        .ok_or_else(|| Error::failed("invalid_cell_address"))?;
    let (letters, digits) = value.split_at(split);
    if letters.is_empty() || letters.len() > 3 || digits.starts_with('0') {
        return Err(Error::failed("invalid_cell_address"));
    }
    let mut column = 0u32;
    for b in letters.bytes() {
        column = column * 26 + u32::from(b - b'A') + 1;
    }
    let row = index(digits)?;
    if row == 0 || row > 1_048_576 || column > 16_384 {
        return Err(Error::failed("cell_address_out_of_bounds"));
    }
    Ok((row, column))
}

fn range(value: &str) -> Result<()> {
    let (a, b) = value.split_once(':').unwrap_or((value, value));
    let a = address(a)?;
    let b = address(b)?;
    if a.0 > b.0 || a.1 > b.1 {
        return Err(Error::failed("reversed_cell_range"));
    }
    Ok(())
}

fn number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut p = usize::from(bytes.first().is_some_and(|b| matches!(b, b'+' | b'-')));
    let mut digits = 0;
    while bytes.get(p).is_some_and(u8::is_ascii_digit) {
        p += 1;
        digits += 1;
    }
    if bytes.get(p) == Some(&b'.') {
        p += 1;
        while bytes.get(p).is_some_and(u8::is_ascii_digit) {
            p += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if bytes.get(p).is_some_and(|b| matches!(b, b'e' | b'E')) {
        p += 1;
        if bytes.get(p).is_some_and(|b| matches!(b, b'+' | b'-')) {
            p += 1;
        }
        let start = p;
        while bytes.get(p).is_some_and(u8::is_ascii_digit) {
            p += 1;
        }
        if start == p {
            return false;
        }
    }
    p == bytes.len()
}

fn rich_string(node: &Node, part: &str, budget: &mut Budget<'_>) -> Result<String> {
    let mut value = String::new();
    if !node.text.trim().is_empty() {
        return Err(Error::failed("text_outside_string_run"));
    }
    for child in &node.children {
        if child.is(SS, "t") {
            value.push_str(child.simple_text()?);
        } else if child.is(SS, "r") {
            for run in &child.children {
                if run.is(SS, "t") {
                    value.push_str(run.simple_text()?);
                } else {
                    budget.finding("rich_text_formatting_or_extension", part, &run.name);
                }
            }
        } else {
            budget.finding("phonetics_or_string_extension", part, &child.name);
        }
    }
    text_value(&value, part, budget)
}

fn text_value(value: &str, part: &str, budget: &mut Budget<'_>) -> Result<String> {
    if value
        .as_bytes()
        .windows(7)
        .any(|b| b[0..2] == *b"_x" && b[2..6].iter().all(u8::is_ascii_hexdigit) && b[6] == b'_')
    {
        budget.finding(
            "excel_escaped_text_not_decoded",
            part,
            "original _xHHHH_ token preserved",
        );
    }
    budget.text(value)
}

#[derive(Debug)]
struct Relationship {
    kind: String,
    target: String,
    external: bool,
}

fn rel_part(source: &str) -> String {
    source.rsplit_once('/').map_or_else(
        || format!("_rels/{source}.rels"),
        |(dir, name)| format!("{dir}/_rels/{name}.rels"),
    )
}

fn relationships(
    node: &Node,
    part: &str,
    budget: &mut Budget<'_>,
) -> Result<BTreeMap<String, Relationship>> {
    if !node.is(REL, "Relationships") {
        return Err(Error::unsupported("relationship_namespace"));
    }
    let mut rels = BTreeMap::new();
    for n in &node.children {
        if !n.is(REL, "Relationship") {
            return Err(Error::unsupported("relationship_extension"));
        }
        let id = n.required("Id")?;
        let external = match n.attr("TargetMode") {
            None | Some("Internal") => false,
            Some("External") => true,
            _ => return Err(Error::failed("relationship_target_mode")),
        };
        let kind = n.required("Type")?.to_string();
        let target = n.required("Target")?.to_string();
        if id.is_empty()
            || target.is_empty()
            || rels
                .insert(
                    id.to_string(),
                    Relationship {
                        kind: kind.clone(),
                        target,
                        external,
                    },
                )
                .is_some()
        {
            return Err(Error::failed("duplicate_or_empty_relationship"));
        }
        if external {
            budget.finding("external_relationship_not_followed", part, &kind);
        }
        let suffix = kind.rsplit('/').next().unwrap_or("");
        if !matches!(
            suffix,
            "officeDocument"
                | "worksheet"
                | "sharedStrings"
                | "styles"
                | "core-properties"
                | "extended-properties"
        ) {
            budget.finding(feature(&kind), part, &kind);
        }
    }
    Ok(rels)
}

fn feature(value: &str) -> &'static str {
    let lower = value.to_ascii_lowercase();
    if lower.contains("pivot") {
        "pivot_tables_not_interpreted"
    } else if lower.contains("chart") {
        "charts_not_interpreted"
    } else if lower.contains("vba") || lower.contains("macro") {
        "macros_not_executed_or_interpreted"
    } else if lower.contains("externallink") {
        "external_links_not_refreshed"
    } else if lower.contains("query") || lower.contains("connection") || lower.contains("model") {
        "external_data_not_refreshed"
    } else if lower.contains("table") {
        "tables_not_interpreted"
    } else if lower.contains("drawing")
        || lower.contains("image")
        || lower.contains("ole")
        || lower.contains("embedding")
    {
        "objects_not_interpreted"
    } else {
        "unsupported_content"
    }
}

fn content_types(
    root: &Node,
    budget: &mut Budget<'_>,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let mut overrides = BTreeMap::new();
    let mut defaults = BTreeMap::new();
    for n in &root.children {
        let typ = n.required("ContentType")?.to_string();
        if n.is(CT, "Override") {
            let part = n
                .required("PartName")?
                .strip_prefix('/')
                .ok_or_else(|| Error::failed("content_type_part_name"))?;
            if !package::valid_part(part)
                || overrides.insert(part.to_string(), typ.clone()).is_some()
            {
                return Err(Error::failed("duplicate_or_invalid_content_type"));
            }
            if typ != MAIN_TYPE
                && typ != SHEET_TYPE
                && typ != STRINGS_TYPE
                && !typ.ends_with(".styles+xml")
                && !typ.ends_with(".core-properties+xml")
                && !typ.ends_with(".extended-properties+xml")
            {
                budget.finding(feature(&typ), part, &typ);
            }
        } else if n.is(CT, "Default") {
            let ext = n.required("Extension")?.to_string();
            if ext.is_empty() || defaults.insert(ext, typ).is_some() {
                return Err(Error::failed("duplicate_default_content_type"));
            }
        } else {
            return Err(Error::unsupported("content_types_extension"));
        }
    }
    Ok((overrides, defaults))
}

fn value(
    cell: &Node,
    part: &str,
    strings: &[String],
    budget: &mut Budget<'_>,
) -> Result<Option<Value>> {
    let typ = cell.attr("t").unwrap_or("n");
    if !matches!(typ, "n" | "s" | "str" | "b" | "e" | "d" | "inlineStr") {
        return Err(Error::unsupported("cell_type"));
    }
    let v = cell.one(SS, "v")?;
    let inline = cell.one(SS, "is")?;
    if typ == "inlineStr" {
        if v.is_some() {
            return Err(Error::failed("inline_string_with_value"));
        }
        return inline
            .map(|n| rich_string(n, part, budget).map(Value::Text))
            .transpose();
    }
    if inline.is_some() {
        return Err(Error::failed("unexpected_inline_string"));
    }
    let Some(v) = v else {
        if !matches!(typ, "n" | "s" | "str" | "b" | "e" | "d") {
            return Err(Error::unsupported("cell_type"));
        }
        return Ok(None);
    };
    let raw = v.simple_text()?;
    if raw.is_empty() && typ != "str" {
        return Ok(Some(Value::Blank));
    }
    Ok(Some(match typ {
        "n" => {
            if !number(raw) {
                return Err(Error::failed("invalid_numeric_value"));
            }
            Value::Number(budget.text(raw)?)
        }
        "s" => {
            let idx = index(raw)? as usize;
            Value::Text(
                budget.text(
                    strings
                        .get(idx)
                        .ok_or_else(|| Error::failed("shared_string_index"))?,
                )?,
            )
        }
        "str" => Value::Text(text_value(raw, part, budget)?),
        "b" => Value::Boolean(bool_attr(Some(raw), false)?),
        "e" => Value::Error(budget.text(raw)?),
        "d" => {
            budget.finding(
                "iso_date_not_validated_or_normalized",
                part,
                "raw t=d token preserved",
            );
            Value::IsoDate(budget.text(raw)?)
        }
        _ => return Err(Error::unsupported("cell_type")),
    }))
}

fn read_cell(
    node: &Node,
    row: u32,
    part: &str,
    strings: &[String],
    budget: &mut Budget<'_>,
) -> Result<Cell> {
    budget.cell_count += 1;
    if budget.cell_count > budget.limits.cells {
        return Err(Error::limit("cell_count_limit"));
    }
    let a = node.required("r")?;
    let (actual_row, column) = address(a)?;
    if actual_row != row {
        return Err(Error::failed("cell_row_mismatch"));
    }
    let style_index = node.attr("s").map(index).transpose()?;
    for attr in node.attrs.keys() {
        if !matches!(attr.as_str(), "r" | "s" | "t") {
            budget.finding("cell_metadata_not_interpreted", part, attr);
        }
    }
    for child in &node.children {
        if !(child.ns == SS && matches!(child.name.as_str(), "f" | "v" | "is")) {
            budget.finding("cell_extension_not_interpreted", part, &child.name);
        }
    }
    let f = node.one(SS, "f")?;
    let val = value(node, part, strings, budget)?;
    let formula = if let Some(f) = f {
        if node.attr("t") == Some("inlineStr") {
            return Err(Error::failed("inline_string_formula"));
        }
        let raw = f.simple_text()?;
        let typ = f.attr("t").unwrap_or("normal");
        let interpretation = match typ {
            "normal" if !raw.is_empty() => "literal_formula_text",
            "shared" => {
                index(f.required("si")?)?;
                budget.finding(
                    "shared_formula_not_expanded",
                    part,
                    "original anchor/follower metadata retained",
                );
                if raw.is_empty() {
                    "unresolved_shared_follower"
                } else {
                    "shared_anchor_text"
                }
            }
            "array" | "dataTable" => {
                range(f.required("ref")?)?;
                budget.finding("array_or_data_table_formula_not_interpreted", part, typ);
                "metadata_only"
            }
            "normal" => return Err(Error::failed("empty_normal_formula")),
            _ => {
                budget.finding("unknown_formula_type_not_interpreted", part, typ);
                "metadata_only"
            }
        };
        if let Some(reference) = f.attr("ref") {
            range(reference)?;
        }
        let mut attrs = BTreeMap::new();
        for (key, val) in &f.attrs {
            attrs.insert(key.clone(), budget.text(val)?);
        }
        Some(Formula {
            text: if raw.is_empty() {
                None
            } else {
                Some(budget.text(raw)?)
            },
            attributes: attrs,
            interpretation,
        })
    } else {
        None
    };
    let cached_result = formula.as_ref().map(|_| FormulaCache {
        value: val.clone(),
        freshness: "unverified",
    });
    let literal = if formula.is_none() {
        Some(val.unwrap_or(Value::Blank))
    } else {
        None
    };
    Ok(Cell {
        address: a.into(),
        row,
        column,
        style_index,
        literal,
        formula,
        cached_result,
    })
}

fn read_sheet(
    sheet: &mut Sheet,
    node: &Node,
    strings: &[String],
    budget: &mut Budget<'_>,
) -> Result<()> {
    let part = sheet.part.as_str();
    if !node.is(SS, "worksheet") {
        return Err(Error::unsupported("worksheet_namespace_or_root"));
    }
    if let Some(dim) = node.one(SS, "dimension")? {
        let value = dim.required("ref")?;
        range(value)?;
        sheet.declared_dimension = Some(value.to_string());
    }
    if let Some(merges) = node.one(SS, "mergeCells")? {
        let mut unique = BTreeSet::new();
        for n in &merges.children {
            if !n.is(SS, "mergeCell") {
                return Err(Error::unsupported("merge_extension"));
            }
            let value = n.required("ref")?;
            range(value)?;
            if !unique.insert(value.to_string()) {
                return Err(Error::failed("duplicate_merge_range"));
            }
            sheet.merged_ranges.push(value.into());
        }
    }
    for child in &node.children {
        if !(child.ns == SS
            && matches!(
                child.name.as_str(),
                "dimension" | "sheetData" | "mergeCells"
            ))
        {
            budget.finding(
                feature(&child.name),
                part,
                &format!("worksheet element {} not interpreted", child.name),
            );
        }
    }
    let data = node
        .one(SS, "sheetData")?
        .ok_or_else(|| Error::failed("sheet_data_missing"))?;
    let mut formula_regions = Vec::new();
    for row in &data.children {
        for cell in row.children.iter().filter(|c| c.is(SS, "c")) {
            if let Some(f) = cell.one(SS, "f")?
                && matches!(f.attr("t"), Some("array" | "dataTable"))
            {
                if formula_regions.len() >= 64 {
                    return Err(Error::limit("formula_region_count_limit"));
                }
                let reference = f.required("ref")?;
                range(reference)?;
                let (start, end) = reference.split_once(':').unwrap_or((reference, reference));
                let a = address(start)?;
                let b = address(end)?;
                let anchor = address(cell.required("r")?)?;
                if anchor.0 < a.0 || anchor.0 > b.0 || anchor.1 < a.1 || anchor.1 > b.1 {
                    return Err(Error::failed("formula_anchor_outside_region"));
                }
                formula_regions.push((
                    a,
                    b,
                    cell.required("r")?.to_string(),
                    reference.to_string(),
                    f.required("t")?.to_string(),
                ));
            }
        }
    }
    let mut rows = BTreeSet::new();
    let mut cells = BTreeSet::new();
    for row in &data.children {
        if !row.is(SS, "row") {
            return Err(Error::unsupported("sheet_data_extension"));
        }
        let idx = index(row.required("r")?)?;
        if idx == 0 || idx > 1_048_576 || !rows.insert(idx) {
            return Err(Error::failed("invalid_or_duplicate_row"));
        }
        for n in &row.children {
            if !n.is(SS, "c") {
                budget.finding("row_extension_not_interpreted", part, &n.name);
                continue;
            }
            let mut cell = read_cell(n, idx, part, strings, budget)?;
            let regions: Vec<_> = formula_regions
                .iter()
                .filter(|(a, b, _, _, _)| {
                    a.0 <= cell.row && cell.row <= b.0 && a.1 <= cell.column && cell.column <= b.1
                })
                .collect();
            if regions.len() > 1 {
                return Err(Error::unsupported("overlapping_formula_regions"));
            }
            if let Some((_, _, anchor, reference, typ)) = regions.first()
                && cell.address != *anchor
            {
                if cell.formula.is_some() {
                    return Err(Error::failed("formula_inside_other_formula_region"));
                }
                if n.attr("t") == Some("inlineStr") {
                    return Err(Error::failed("inline_string_formula_region_member"));
                }
                let value = if n.one(SS, "v")?.is_some() {
                    cell.literal.take()
                } else {
                    cell.literal = None;
                    None
                };
                cell.cached_result = Some(FormulaCache {
                    value,
                    freshness: "unverified",
                });
                cell.formula = Some(Formula {
                    text: None,
                    attributes: BTreeMap::from([
                        ("anchor_address".into(), anchor.clone()),
                        ("ref".into(), reference.clone()),
                        ("t".into(), typ.clone()),
                    ]),
                    interpretation: "unresolved_array_or_data_table_member",
                });
            }
            if !cells.insert(cell.address.clone()) {
                return Err(Error::failed("duplicate_cell_address"));
            }
            sheet.cells.push(cell);
        }
    }
    sheet.cells.sort_by_key(|c| (c.row, c.column));
    Ok(())
}

/// Reads already-captured bytes only. No paths, network, writes or formula evaluator.
pub fn read_xlsx(bytes: &[u8], limits: &Limits) -> Result<Workbook> {
    limits.validate()?;
    if bytes.len() > limits.input_bytes {
        return Err(Error::limit("input_size_limit"));
    }
    let parts = package::unpack(bytes, limits)?;
    let mut budget = Budget {
        limits,
        cell_count: 0,
        text_count: 0,
        findings: BTreeSet::new(),
    };
    let mut roots = BTreeMap::new();
    let mut nodes = 0usize;
    for (name, data) in &parts {
        if name.ends_with(".xml") || name.ends_with(".rels") || name == "[Content_Types].xml" {
            roots.insert(name.clone(), xml::parse(data, limits, &mut nodes)?);
        }
    }
    let ct = expect_root(&roots, "[Content_Types].xml", CT, "Types")?;
    let (overrides, defaults) = content_types(ct, &mut budget)?;
    for name in parts.keys().filter(|n| n.as_str() != "[Content_Types].xml") {
        if !overrides.contains_key(name)
            && !name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| defaults.contains_key(ext))
        {
            return Err(Error::failed("part_has_no_content_type"));
        }
    }
    if overrides.keys().any(|p| !parts.contains_key(p)) {
        return Err(Error::failed("content_type_missing_part"));
    }
    let mut rels_by_part = BTreeMap::new();
    for (name, root) in roots.iter().filter(|(p, _)| p.ends_with(".rels")) {
        rels_by_part.insert(name.clone(), relationships(root, name, &mut budget)?);
    }
    let package_rels = rels_by_part
        .get("_rels/.rels")
        .ok_or_else(|| Error::failed("package_relationships_missing"))?;
    let main_rels: Vec<_> = package_rels
        .values()
        .filter(|r| r.kind == format!("{DOC_REL}/officeDocument"))
        .collect();
    if main_rels.len() != 1 {
        return Err(Error::failed("workbook_relationship_missing_or_duplicate"));
    }
    let main_rel = main_rels[0];
    if main_rel.external {
        return Err(Error::unsupported("external_workbook"));
    }
    let main_part = package::resolve("", &main_rel.target)?;
    if overrides.get(&main_part).map(String::as_str) != Some(MAIN_TYPE) {
        return Err(Error::unsupported("not_plain_xlsx_workbook_content_type"));
    }
    let main = expect_root(&roots, &main_part, SS, "workbook")?;
    let main_rel_part = rel_part(&main_part);
    let main_rels = rels_by_part
        .get(&main_rel_part)
        .ok_or_else(|| Error::failed("workbook_relationships_missing"))?;
    let mut used = BTreeSet::from([
        "[Content_Types].xml".to_string(),
        "_rels/.rels".to_string(),
        main_part.clone(),
        main_rel_part.clone(),
    ]);
    let mut strings = Vec::new();
    let shared: Vec<_> = main_rels
        .values()
        .filter(|r| r.kind == format!("{DOC_REL}/sharedStrings"))
        .collect();
    if shared.len() > 1 {
        return Err(Error::failed("duplicate_shared_string_relationship"));
    }
    if let Some(rel) = shared.first() {
        if rel.external {
            return Err(Error::unsupported("external_shared_strings"));
        }
        let part = package::resolve(&main_part, &rel.target)?;
        if overrides.get(&part).map(String::as_str) != Some(STRINGS_TYPE) {
            return Err(Error::failed("shared_strings_content_type"));
        }
        let root = expect_root(&roots, &part, SS, "sst")?;
        for n in &root.children {
            if !n.is(SS, "si") {
                budget.finding("shared_strings_extension", &part, &n.name);
                continue;
            }
            if strings.len() >= limits.shared_strings {
                return Err(Error::limit("shared_string_count_limit"));
            }
            strings.push(rich_string(n, &part, &mut budget)?);
        }
        used.insert(part);
    }
    let date_1904 = main
        .one(SS, "workbookPr")?
        .map_or(Ok(false), |n| bool_attr(n.attr("date1904"), false))?;
    let calculation_properties = main
        .one(SS, "calcPr")?
        .map_or_else(BTreeMap::new, |n| n.attrs.clone());
    for n in &main.children {
        if !(n.ns == SS && matches!(n.name.as_str(), "sheets" | "workbookPr" | "calcPr")) {
            budget.finding(
                feature(&n.name),
                &main_part,
                &format!("workbook element {} not interpreted", n.name),
            );
        }
    }
    let container = main
        .one(SS, "sheets")?
        .ok_or_else(|| Error::failed("sheets_missing"))?;
    let mut sheets = Vec::new();
    let mut names = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut sheet_parts = BTreeSet::new();
    for n in &container.children {
        if !n.is(SS, "sheet") {
            return Err(Error::unsupported("sheets_extension"));
        }
        if sheets.len() >= limits.sheets {
            return Err(Error::limit("sheet_count_limit"));
        }
        let name = n.required("name")?;
        let id = n.required("sheetId")?;
        let numeric_id = index(id)?;
        if name.is_empty()
            || name.chars().count() > 31
            || name.chars().any(|c| "[]:*?/\\".contains(c))
            || numeric_id == 0
            || !names.insert(name.to_lowercase())
            || !ids.insert(numeric_id)
        {
            return Err(Error::failed("invalid_or_duplicate_sheet_identity"));
        }
        let rel_id = n.required(&format!("{{{DOC_REL}}}id"))?;
        let rel = main_rels
            .get(rel_id)
            .ok_or_else(|| Error::failed("sheet_relationship_missing"))?;
        if rel.external {
            return Err(Error::unsupported("external_sheet"));
        }
        let part = package::resolve(&main_part, &rel.target)?;
        if !parts.contains_key(&part) || !sheet_parts.insert(part.clone()) {
            return Err(Error::failed("missing_or_duplicate_sheet_part"));
        }
        let visibility = n.attr("state").unwrap_or("visible");
        if !matches!(visibility, "visible" | "hidden" | "veryHidden") {
            return Err(Error::failed("sheet_visibility"));
        }
        let kind = rel
            .kind
            .strip_prefix(&format!("{DOC_REL}/"))
            .ok_or_else(|| Error::unsupported("sheet_relationship_type"))?;
        let mut sheet = Sheet {
            name: name.into(),
            sheet_id: id.into(),
            part: part.clone(),
            visibility: visibility.into(),
            kind: kind.into(),
            declared_dimension: None,
            merged_ranges: Vec::new(),
            cells: Vec::new(),
        };
        if kind == "worksheet" {
            if overrides.get(&part).map(String::as_str) != Some(SHEET_TYPE) {
                return Err(Error::failed("worksheet_content_type"));
            }
            read_sheet(
                &mut sheet,
                expect_root(&roots, &part, SS, "worksheet")?,
                &strings,
                &mut budget,
            )?;
        } else if matches!(
            kind,
            "chartsheet" | "dialogsheet" | "macrosheet" | "intlMacrosheet"
        ) {
            budget.finding(
                feature(kind),
                &part,
                "sheet name retained; non-worksheet cells not read",
            );
        } else {
            return Err(Error::unsupported("unknown_sheet_kind"));
        }
        used.insert(part);
        sheets.push(sheet);
    }
    if sheets.is_empty() {
        return Err(Error::failed("empty_sheet_list"));
    }
    for (part, rels) in &rels_by_part {
        let source = if part == "_rels/.rels" {
            "".to_string()
        } else if let Some((dir, leaf)) = part.rsplit_once("/_rels/") {
            format!("{dir}/{}", leaf.trim_end_matches(".rels"))
        } else if let Some(leaf) = part.strip_prefix("_rels/") {
            leaf.trim_end_matches(".rels").into()
        } else {
            return Err(Error::failed("relationship_part_location"));
        };
        if !source.is_empty() && !parts.contains_key(&source) {
            return Err(Error::failed("orphan_relationship_part"));
        }
        for rel in rels.values().filter(|r| !r.external) {
            let target = package::resolve(&source, &rel.target)?;
            if !parts.contains_key(&target) {
                return Err(Error::failed("internal_relationship_missing_part"));
            }
        }
    }
    for part in parts.keys().filter(|p| !used.contains(*p)) {
        budget.finding(feature(part), part, "part not interpreted by this profile");
    }
    Ok(Workbook {
        profile: "traeva-xlsx-experiment-v1",
        status: if budget.findings.is_empty() {
            "complete_within_profile"
        } else {
            "partial"
        },
        input_sha256: Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        date_1904,
        calculation_properties,
        sheets,
        omissions: budget.findings.into_iter().collect(),
        limitations: vec![
            "Formulas are never executed. All cached results have unverified freshness.",
            "Formatting, number formats, date serial conversion and visual layout are not interpreted.",
            "Charts, pivots, macros, external data, objects and extensions are not interpreted or refreshed.",
            "Sheet/address context is positional; row alignment, lineage and semantic meaning are not inferred.",
        ],
    })
}

#[derive(Debug, Serialize)]
pub struct CellChange {
    pub sheet_name: String,
    pub sheet_id: String,
    pub part: String,
    pub address: String,
    pub change: &'static str,
    pub literal_changed: bool,
    pub formula_changed: bool,
    pub cache_changed: bool,
    pub style_changed: bool,
    pub before: Option<Cell>,
    pub after: Option<Cell>,
}

/// Coordinate evidence only; callers must supply a separately authorized snapshot pair.
pub fn coordinate_changes(before: &Workbook, after: &Workbook) -> Result<Vec<CellChange>> {
    let sheet_key = |s: &Sheet| {
        (
            s.sheet_id.clone(),
            s.name.clone(),
            s.part.clone(),
            s.visibility.clone(),
            s.kind.clone(),
            s.merged_ranges.clone(),
        )
    };
    let left: BTreeMap<_, _> = before
        .sheets
        .iter()
        .map(|s| (s.sheet_id.as_str(), s))
        .collect();
    let right: BTreeMap<_, _> = after
        .sheets
        .iter()
        .map(|s| (s.sheet_id.as_str(), s))
        .collect();
    if left.len() != right.len()
        || left
            .iter()
            .any(|(id, s)| right.get(id).is_none_or(|t| sheet_key(s) != sheet_key(t)))
        || before.date_1904 != after.date_1904
    {
        return Err(Error::unsupported(
            "coordinate_diff_requires_same_sheet_context",
        ));
    }
    let mut changes = Vec::new();
    for (id, sheet) in &left {
        let other = right[id];
        let a: BTreeMap<_, _> = sheet
            .cells
            .iter()
            .map(|c| (c.address.as_str(), c))
            .collect();
        let b: BTreeMap<_, _> = other
            .cells
            .iter()
            .map(|c| (c.address.as_str(), c))
            .collect();
        for address in a.keys().chain(b.keys()).copied().collect::<BTreeSet<_>>() {
            let old = a.get(address).copied();
            let new = b.get(address).copied();
            if old == new {
                continue;
            }
            changes.push(CellChange {
                sheet_name: sheet.name.clone(),
                sheet_id: sheet.sheet_id.clone(),
                part: sheet.part.clone(),
                address: address.into(),
                change: match (old, new) {
                    (None, _) => "added",
                    (_, None) => "removed",
                    _ => "changed",
                },
                literal_changed: old.and_then(|c| c.literal.as_ref())
                    != new.and_then(|c| c.literal.as_ref()),
                formula_changed: old.and_then(|c| c.formula.as_ref())
                    != new.and_then(|c| c.formula.as_ref()),
                cache_changed: old.and_then(|c| c.cached_result.as_ref())
                    != new.and_then(|c| c.cached_result.as_ref()),
                style_changed: old.and_then(|c| c.style_index) != new.and_then(|c| c.style_index),
                before: old.cloned(),
                after: new.cloned(),
            });
        }
    }
    Ok(changes)
}
