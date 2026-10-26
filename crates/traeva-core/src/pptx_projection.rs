use super::*;

pub const PPTX_MEMORY_READER: &str = "pptx-1";
pub const PPTX_PREVIEW_READER: &str = "pptx-preview-1";

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum Projection {
    #[default]
    Legacy,
    Memory,
    Preview,
}

impl Projection {
    pub(super) fn reader(self) -> &'static str {
        match self {
            Self::Legacy => PPTX_READER,
            Self::Memory => PPTX_MEMORY_READER,
            Self::Preview => PPTX_PREVIEW_READER,
        }
    }

    pub(super) fn coverage(self) -> &'static str {
        match self {
            Self::Legacy => {
                "presentation slide order (including hidden slides); generated slide/role labels; stored shape text in XML order; explicit or layout-inherited title placeholders; speaker notes"
            }
            Self::Memory => {
                "visible-slide stored titles/text and associated Notes in presentation order; package sldId and original ordinal structure lines; escaped source lines; hidden slides and their Notes excluded"
            }
            Self::Preview => {
                "all stored titles/text and associated Notes in presentation order; package sldId, original ordinal and hidden-slide markers; escaped source lines; local preview projection only"
            }
        }
    }
}

pub(super) fn slide_hidden(bytes: &[u8], budget: &mut XmlBudget) -> Result<bool> {
    let mut first = true;
    let mut hidden = false;
    xml(bytes, budget, |event| {
        if let XmlEvent::Start(e) = event
            && first
        {
            first = false;
            if !is(&e, P, "sld") {
                return Err(damaged());
            }
            hidden = match e
                .attr("show")
                .map(|s| s.trim_matches([' ', '\t', '\r', '\n']))
            {
                None | Some("1" | "true") => false,
                Some("0" | "false") => true,
                _ => return Err(damaged()),
            };
        }
        Ok(())
    })?;
    Ok(hidden)
}

impl Output {
    pub(super) fn begin_slide(&mut self, ordinal: usize, id: u32, hidden: bool) {
        self.selected = !(self.mode == Projection::Memory && hidden);
        if !self.selected {
            self.hidden_slides += 1;
            return;
        }
        let separator = if self.emitted_slides == 0 {
            ""
        } else if self.mode == Projection::Legacy {
            "\n\n"
        } else {
            "\n"
        };
        if self.mode == Projection::Legacy {
            self.append(&format!("{separator}[Slide {ordinal}]"));
        } else {
            let visibility = if hidden { " · hidden" } else { "" };
            self.append_atomic(&format!(
                "{separator}⟦slide {ordinal} · id {id}{visibility}⟧"
            ));
        }
        self.emitted_slides += 1;
    }

    // An incomplete generated label must never become apparent source text.
    fn append_atomic(&mut self, text: &str) {
        let chars = text.chars().count();
        self.total += chars;
        if !self.cut && self.kept + chars <= MAX_MEMORY_TEXT_CHARS {
            self.text.push_str(text);
            self.kept += chars;
        } else {
            self.cut = true;
        }
    }

    pub(super) fn project_shape(&mut self, role: &str, text: &str) {
        let role = match role {
            "Title" => "title",
            "Notes" => "notes",
            _ => "text",
        };
        self.append_atomic(&format!("\n⟦{role}⟧\n"));
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.append("\n");
            }
            if let Some(first @ ('⟦' | '\\')) = line.chars().next() {
                // Keep the escape and its first source scalar together at the cap.
                self.append_atomic(&format!("\\{first}"));
                self.append(&line[first.len_utf8()..]);
            } else {
                self.append(line);
            }
        }
    }
}
