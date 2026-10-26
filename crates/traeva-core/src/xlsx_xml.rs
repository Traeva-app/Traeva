use super::{Error, Limits, Result};
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Reader};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct Node {
    pub ns: String,
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn is(&self, ns: &str, name: &str) -> bool {
        self.ns == ns && self.name == name
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn required(&self, name: &str) -> Result<&str> {
        self.attr(name)
            .ok_or_else(|| Error::failed("missing_xml_attribute"))
    }
    pub fn one(&self, ns: &str, name: &str) -> Result<Option<&Node>> {
        let mut found = self.children.iter().filter(|n| n.is(ns, name));
        let first = found.next();
        if found.next().is_some() {
            return Err(Error::failed("duplicate_xml_element"));
        }
        Ok(first)
    }
    pub fn simple_text(&self) -> Result<&str> {
        if !self.children.is_empty() {
            return Err(Error::unsupported("nested_value_content"));
        }
        Ok(&self.text)
    }
}

fn namespace(ns: ResolveResult<'_>) -> Result<String> {
    match ns {
        ResolveResult::Unbound => Ok(String::new()),
        ResolveResult::Bound(n) => Ok(n.as_ref().to_string()),
        ResolveResult::Unknown(_) => Err(Error::failed("undeclared_xml_prefix")),
    }
}

fn valid_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')
}

fn xml_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

// XML 1.0 (Fifth Edition) productions [4] and [4a], excluding the QName colon.
fn name_start(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}'
    )
}

fn name_char(c: char) -> bool {
    name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

fn xml_name(name: &str) -> Result<()> {
    let mut count = 0;
    for part in name.split(':') {
        count += 1;
        let mut chars = part.chars();
        if count > 2 || !chars.next().is_some_and(name_start) || !chars.all(name_char) {
            return Err(Error::failed("invalid_xml_name"));
        }
    }
    Ok(())
}

fn attribute_separators(raw: &str, name_len: usize) -> Result<()> {
    let bytes = raw.as_bytes();
    let mut pos = name_len;
    while pos < bytes.len() {
        if !xml_space(bytes[pos]) {
            return Err(Error::failed("missing_xml_attribute_separator"));
        }
        while pos < bytes.len() && xml_space(bytes[pos]) {
            pos += 1;
        }
        if pos == bytes.len() {
            break;
        }
        let begin = pos;
        while pos < bytes.len() && !xml_space(bytes[pos]) && bytes[pos] != b'=' {
            pos += 1;
        }
        if pos == begin {
            return Err(Error::failed("malformed_xml_attribute"));
        }
        xml_name(&raw[begin..pos])?;
        while pos < bytes.len() && xml_space(bytes[pos]) {
            pos += 1;
        }
        if bytes.get(pos) != Some(&b'=') {
            return Err(Error::failed("malformed_xml_attribute"));
        }
        pos += 1;
        while pos < bytes.len() && xml_space(bytes[pos]) {
            pos += 1;
        }
        let quote = *bytes
            .get(pos)
            .ok_or_else(|| Error::failed("malformed_xml_attribute"))?;
        if !matches!(quote, b'\'' | b'"') {
            return Err(Error::failed("malformed_xml_attribute"));
        }
        pos += 1;
        while pos < bytes.len() && bytes[pos] != quote {
            if bytes[pos] == b'<' {
                return Err(Error::failed("malformed_xml_attribute"));
            }
            pos += 1;
        }
        if pos == bytes.len() {
            return Err(Error::failed("malformed_xml_attribute"));
        }
        pos += 1;
    }
    Ok(())
}

fn preflight(bytes: &[u8], limits: &Limits, total_nodes: &mut usize) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::unsupported("xml_not_utf8"))?;
    if !text.chars().all(valid_char) {
        return Err(Error::failed("illegal_xml_character"));
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_comments = true;
    let mut depth = 0usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|_| Error::failed("malformed_xml"))?;
        if event.len() > limits.xml_event_bytes {
            return Err(Error::limit("xml_event_limit"));
        }
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                xml_name(start.name().as_ref())?;
                attribute_separators(start.as_ref(), start.name().as_ref().len())?;
                *total_nodes += 1;
                if *total_nodes > limits.xml_nodes || depth + 1 > limits.xml_depth {
                    return Err(Error::limit("xml_node_or_depth_limit"));
                }
                for (i, a) in start.attributes().enumerate() {
                    if i >= limits.xml_attributes {
                        return Err(Error::limit("xml_attribute_limit"));
                    }
                    a.map_err(|_| Error::failed("malformed_xml_attribute"))?;
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(end) => {
                xml_name(end.name().as_ref())?;
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::failed("xml_depth"))?
            }
            Event::DocType(_) => return Err(Error::unsupported("xml_doctype_forbidden")),
            Event::PI(_) => return Err(Error::unsupported("xml_processing_instruction")),
            Event::Text(t) if t.as_ref().contains("]]>") => {
                return Err(Error::failed("invalid_xml_character_data"));
            }
            Event::Decl(d) => {
                attribute_separators(d.as_ref(), 3)?;
                if d.encoding()
                    .is_some_and(|e| !e.is_ok_and(|e| e.eq_ignore_ascii_case("utf-8")))
                {
                    return Err(Error::unsupported("xml_encoding"));
                }
            }
            Event::Eof => {
                return if depth == 0 {
                    Ok(())
                } else {
                    Err(Error::failed("truncated_xml"))
                };
            }
            _ => (),
        }
    }
}

fn attach(node: Node, stack: &mut [Node], root: &mut Option<Node>) -> Result<()> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.replace(node).is_some() {
        return Err(Error::failed("multiple_xml_roots"));
    }
    Ok(())
}

pub(crate) fn parse(bytes: &[u8], limits: &Limits, total_nodes: &mut usize) -> Result<Node> {
    preflight(bytes, limits, total_nodes)?;
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().check_comments = true;
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut declared = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|_| Error::failed("malformed_xml"))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let (ns, local) = reader.resolver().resolve_element(start.name());
                let mut node = Node {
                    ns: namespace(ns)?,
                    name: local.as_ref().to_string(),
                    attrs: BTreeMap::new(),
                    text: String::new(),
                    children: Vec::new(),
                };
                for a in start.attributes() {
                    let a = a.map_err(|_| Error::failed("xml_attribute"))?;
                    if a.key.as_ref() == "xmlns" || a.key.as_ref().starts_with("xmlns:") {
                        continue;
                    }
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    let ns = namespace(ns)?;
                    let local = local.as_ref();
                    let key = if ns.is_empty() {
                        local.to_string()
                    } else {
                        format!("{{{ns}}}{local}")
                    };
                    let value = a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|_| Error::failed("xml_attribute_value"))?
                        .into_owned();
                    if !value.chars().all(valid_char) || node.attrs.insert(key, value).is_some() {
                        return Err(Error::failed("invalid_or_duplicate_expanded_attribute"));
                    }
                }
                if matches!(event, Event::Empty(_)) {
                    attach(node, &mut stack, &mut root)?;
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| Error::failed("xml_end_without_start"))?;
                attach(node, &mut stack, &mut root)?;
            }
            Event::Text(t) => {
                let value = t.xml_content(quick_xml::XmlVersion::Implicit1_0);
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err(Error::failed("text_outside_xml_root"));
                }
            }
            Event::CData(t) => {
                let node = stack
                    .last_mut()
                    .ok_or_else(|| Error::failed("cdata_outside_root"))?;
                node.text
                    .push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
            }
            Event::GeneralRef(r) => {
                let encoded = format!("&{};", r.as_ref());
                let decoded = quick_xml::escape::unescape(&encoded)
                    .map_err(|_| Error::failed("unknown_xml_entity"))?;
                if !decoded.chars().all(valid_char) {
                    return Err(Error::failed("illegal_xml_entity_character"));
                }
                stack
                    .last_mut()
                    .ok_or_else(|| Error::failed("entity_outside_root"))?
                    .text
                    .push_str(&decoded);
            }
            Event::Decl(d) => {
                if declared
                    || root.is_some()
                    || !stack.is_empty()
                    || !d.version().is_ok_and(|v| v.as_ref() == "1.0")
                {
                    return Err(Error::unsupported("xml_declaration"));
                }
                declared = true;
            }
            Event::Eof => return root.ok_or_else(|| Error::failed("missing_xml_root")),
            _ => (),
        }
    }
}
