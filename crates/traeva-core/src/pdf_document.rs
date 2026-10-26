//! Opens the PDF with hayro (from memory only), checks the object, page
//! and content limits, and draws each page into a `Collector` that keeps
//! what text extraction needs.
//!
//! hayro never runs JavaScript, never fetches anything and never opens a
//! file. Annotations (form fields, comments) are not drawn: they are
//! counted and reported instead. Fonts that a PDF does not embed are drawn
//! with hayro's bundled stand-in fonts, so their Unicode still comes from
//! the PDF's own encoding and ToUnicode data.

use std::borrow::Cow;
use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hayro_interpret::font::{Glyph as HayroGlyph, GlyphRun};
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, InterpreterWarning, SoftMask, TransformExt,
    interpret_page,
};
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::Page;
use hayro_syntax::xref::XRef;
use hayro_syntax::{LoadPdfError, Pdf};
use kurbo::{BezPath, PathSeg, Point, Rect, Shape};

use super::layout::{self, Glyph, Line};
use super::preflight::{Preflight, StreamAt, StreamCheck};
use super::{
    MAX_CACHED_CONTENT_BYTES, MAX_DICT_DEPTH, MAX_DRAW_EVENTS, MAX_GLYPHS, MAX_OBJECTS,
    MAX_PAGE_CONTENT_BYTES, MAX_PAGES, Stop,
};

/// What the reader saw in the whole document.
#[derive(Debug, Default)]
pub(crate) struct DocumentFacts {
    pub pages: Vec<PageFacts>,
    /// Fields in the document's form (`/AcroForm /Fields`).
    pub form_fields: usize,
    pub xfa: bool,
    /// Optional content groups that are off by default.
    pub hidden_layers: bool,
    /// JavaScript actions or document scripts, from the parsed objects.
    pub javascript: bool,
    /// Embedded files (attachments), from the parsed objects.
    pub embedded_files: bool,
}

#[derive(Debug, Default)]
pub(crate) struct PageFacts {
    pub lines: Vec<Line>,
    /// Glyphs with a usable Unicode mapping, inside the page.
    pub glyphs: usize,
    pub unmapped: usize,
    pub outside: usize,
    pub images: usize,
    /// Share of the page the images cover (overlaps counted once each, so
    /// this is an upper estimate), 0..=1.
    pub image_cover: f64,
    pub grid: bool,
    pub drawings: usize,
    pub math: bool,
    pub annotations: usize,
    pub widgets: usize,
    /// The page shows text, but no glyph came out (a font hayro could not
    /// load), or hayro reported an unsupported font.
    pub undecoded_text: bool,
}

/// Raised from inside a drawing callback to stop hayro at a limit. Carried
/// with `resume_unwind`, so no panic message is printed.
pub(crate) struct LimitHit(pub &'static str);

#[derive(Debug, Default)]
pub(crate) struct Budget {
    glyphs: usize,
    events: usize,
}

impl Budget {
    fn event(&mut self) {
        self.events += 1;
        if self.events > MAX_DRAW_EVENTS {
            panic::resume_unwind(Box::new(LimitHit("drawing operations")));
        }
    }

    fn glyph(&mut self) {
        self.glyphs += 1;
        if self.glyphs > MAX_GLYPHS {
            panic::resume_unwind(Box::new(LimitHit("characters drawn")));
        }
    }
}

/// A password no file uses. hayro opens an unencrypted file whatever the
/// password, and fails with a decryption error on a file it reads as
/// encrypted, also when it had to rebuild the cross-reference data and
/// found the encryption dictionary itself.
const NOT_A_PASSWORD: &str = "\u{0}traeva-pdf-experiment: not a password\u{0}";

fn open(bytes: &[u8]) -> Result<Pdf, Stop> {
    match Pdf::new_with_password(bytes.to_vec(), NOT_A_PASSWORD) {
        Ok(pdf) => Ok(pdf),
        Err(LoadPdfError::Decryption(_)) => Err(Stop::unsupported(crate::pdf::ENCRYPTED_NOTE)),
        Err(LoadPdfError::Invalid) => Err(Stop::failed("The PDF's structure could not be read.")),
    }
}

pub(crate) fn read(
    bytes: &[u8],
    checked: &Preflight,
    budget: &mut Budget,
) -> Result<DocumentFacts, Stop> {
    let pdf = open(bytes)?;
    if pdf.len() > MAX_OBJECTS {
        return Err(Stop::unsupported(format!(
            "The PDF has more objects than this reader's limit ({MAX_OBJECTS})."
        )));
    }
    let page_count = pdf.pages().len();
    if page_count == 0 {
        return Err(Stop::failed("The PDF has no pages."));
    }
    if page_count > MAX_PAGES {
        return Err(Stop::unsupported(format!(
            "The PDF has more pages than this reader's limit ({MAX_PAGES})."
        )));
    }
    let mut facts = catalog_facts(&pdf, page_count)?;
    object_facts(&pdf, checked, &mut facts)?;

    // Every page's content streams are checked before any page is drawn.
    let mut content_sizes = Vec::with_capacity(page_count);
    for page in pdf.pages().iter() {
        content_sizes.push(content_size(page, &pdf, checked)?);
    }

    // hayro keeps each drawn page's decoded content for as long as the
    // document is open, so it is reopened after every 64 MiB of content.
    let mut first = Some(pdf);
    let mut index = 0;
    while index < page_count {
        let pdf = match first.take() {
            Some(pdf) => pdf,
            None => {
                // The same checks on the reopened document, which may
                // rebuild its cross-reference data at another moment.
                let pdf = open(bytes)?;
                object_facts(&pdf, checked, &mut DocumentFacts::default())?;
                pdf
            }
        };
        if pdf.pages().len() != page_count {
            return Err(Stop::failed(
                "The PDF's page tree reads differently each time.",
            ));
        }
        let unsupported_fonts = Arc::new(AtomicUsize::new(0));
        let sink = Arc::clone(&unsupported_fonts);
        let settings = InterpreterSettings {
            render_annotations: false,
            warning_sink: Arc::new(move |warning| {
                if matches!(warning, InterpreterWarning::UnsupportedFont) {
                    sink.fetch_add(1, Ordering::Relaxed);
                }
            }),
            ..InterpreterSettings::default()
        };
        let cache = InterpreterCache::new();
        let mut cached = 0;
        while index < page_count && cached <= MAX_CACHED_CONTENT_BYTES {
            let page = &pdf.pages()[index];
            let before = unsupported_fonts.load(Ordering::Relaxed);
            let mut page_facts = read_page(page, &pdf, &cache, settings.clone(), budget);
            if unsupported_fonts.load(Ordering::Relaxed) != before {
                page_facts.undecoded_text = true;
            }
            facts.pages.push(page_facts);
            cached += content_sizes[index];
            index += 1;
        }
    }
    Ok(facts)
}

fn catalog_facts(pdf: &Pdf, page_count: usize) -> Result<DocumentFacts, Stop> {
    let mut facts = DocumentFacts::default();
    let xref = pdf.xref();
    let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) else {
        return Err(Stop::failed("The PDF's catalog could not be read."));
    };
    if let Some(declared) = catalog
        .get::<Dict<'_>>(b"Pages")
        .and_then(|pages| pages.get::<u32>(b"Count"))
        && usize::try_from(declared).ok() != Some(page_count)
    {
        return Err(Stop::failed(format!(
            "The PDF's page tree is damaged: it declares {declared} pages but {page_count} were found."
        )));
    }
    if let Some(form) = catalog.get::<Dict<'_>>(b"AcroForm") {
        facts.form_fields = form
            .get::<Array<'_>>(b"Fields")
            .map_or(0, |fields| fields.iter::<Dict<'_>>().count());
        facts.xfa = form.contains_key(b"XFA");
    }
    if let Some(defaults) = catalog
        .get::<Dict<'_>>(b"OCProperties")
        .and_then(|properties| properties.get::<Dict<'_>>(b"D"))
    {
        let off = defaults
            .get::<Array<'_>>(b"OFF")
            .is_some_and(|off| off.iter::<Dict<'_>>().next().is_some());
        let base_off = defaults
            .get::<Name<'_>>(b"BaseState")
            .is_some_and(|state| state.as_str() == "OFF");
        facts.hidden_layers = off || base_off;
    }
    Ok(facts)
}

/// Every object hayro can parse: each stream must be one the raw-byte
/// check measured, found by where its dictionary and its data start (a
/// stream the check missed must not be decoded; one it kept a finding for
/// is refused now), and JavaScript and embedded files are found in the
/// parsed dictionaries, where name escapes are already decoded. Words in
/// text, strings or comments are not dictionary keys.
///
/// Twice: if an object cannot be read where the cross-reference data says,
/// hayro rebuilds that data once, from the whole file, part way through the
/// first pass. The second pass sees the rebuilt objects; none change after.
///
/// The reading of object-stream members in each pass is bounded before
/// hayro opens the file (`preflight::scan_streams`).
fn object_facts(pdf: &Pdf, checked: &Preflight, facts: &mut DocumentFacts) -> Result<(), Stop> {
    for _ in 0..2 {
        for object in pdf.objects() {
            match &object {
                Object::Stream(stream) => {
                    let Some((check, length)) = stream_check(pdf, stream, checked) else {
                        return Err(unchecked());
                    };
                    if let Some(refusal) = &check.refusal {
                        return Err(refusal.clone());
                    }
                    if !check.lengths.binary_search(&length).is_ok() {
                        return Err(unchecked());
                    }
                    scan_dict(stream.dict(), 0, facts);
                }
                Object::Dict(dict) => scan_dict(dict, 0, facts),
                Object::Array(array) => scan_array(array, 0, facts),
                _ => {}
            }
        }
    }
    Ok(())
}

fn unchecked() -> Stop {
    Stop::unsupported("A stream in the PDF could not be checked before reading.")
}

/// The raw-byte check's record of a stream hayro read, found by where the
/// stream's dictionary and data are in the file, and the length of the data
/// hayro took. hayro reads an unencrypted file in place, so both are slices
/// of the bytes it was given. The caller checks that length against the
/// lengths the raw-byte check measured.
fn stream_check<'c>(
    pdf: &Pdf,
    stream: &Stream<'_>,
    checked: &'c Preflight,
) -> Option<(&'c StreamCheck, usize)> {
    let base = pdf.data().as_ref();
    let offset = |part: &[u8]| {
        (part.as_ptr() as usize)
            .checked_sub(base.as_ptr() as usize)
            .filter(|&at| at <= base.len())
    };
    let Cow::Borrowed(data) = stream.raw_data() else {
        return None;
    };
    let at = StreamAt {
        dict: offset(stream.dict().data())?,
        data: offset(data)?,
    };
    Some((checked.streams.get(&at)?, data.len()))
}

/// JavaScript: a `/JS` key (a script action), `/S /JavaScript`, or a
/// `/JavaScript` name tree. Embedded files: an `/EmbeddedFiles` name tree,
/// an `/EF` file specification, or a stream of `/Type /EmbeddedFile`.
fn scan_dict(dict: &Dict<'_>, depth: usize, facts: &mut DocumentFacts) {
    if depth > MAX_DICT_DEPTH {
        return;
    }
    for (key, value) in dict.entries() {
        match &*key {
            b"JS" | b"JavaScript" => facts.javascript = true,
            b"EmbeddedFiles" | b"EF" => facts.embedded_files = true,
            _ => {}
        }
        let MaybeRef::NotRef(value) = value else {
            continue;
        };
        match value {
            Object::Name(name) if &*key == b"S" && &*name == b"JavaScript" => {
                facts.javascript = true;
            }
            Object::Name(name) if &*key == b"Type" && &*name == b"EmbeddedFile" => {
                facts.embedded_files = true;
            }
            Object::Dict(inner) => scan_dict(&inner, depth + 1, facts),
            Object::Array(inner) => scan_array(&inner, depth + 1, facts),
            _ => {}
        }
    }
}

fn scan_array(array: &Array<'_>, depth: usize, facts: &mut DocumentFacts) {
    if depth > MAX_DICT_DEPTH {
        return;
    }
    for item in array.raw_iter() {
        match item {
            MaybeRef::NotRef(Object::Dict(inner)) => scan_dict(&inner, depth + 1, facts),
            MaybeRef::NotRef(Object::Array(inner)) => scan_array(&inner, depth + 1, facts),
            _ => {}
        }
    }
}

/// The page's content streams, from its raw `/Contents`: no key, null, or
/// a reference to an object that is null is an empty page (a null value is
/// the same as no entry); a reference must otherwise lead to a stream, or
/// to an array whose every member is a reference to a stream. Anything
/// else (a missing object, a string, a number, a null array member) means
/// the page's content is damaged, rather than quietly reading the parts
/// that are left.
fn content_streams<'a>(page: &Page<'a>, xref: &'a XRef) -> Result<Vec<Stream<'a>>, Stop> {
    let damaged = || {
        Stop::failed(
            "A page's content list is damaged: it refers to an object that is missing or is not a content stream.",
        )
    };
    let stream_at = |id: ObjectIdentifier| xref.get::<Stream<'_>>(id).ok_or_else(damaged);
    let members = |array: Array<'a>| -> Result<Vec<Stream<'a>>, Stop> {
        array
            .raw_iter()
            .map(|item| match item {
                MaybeRef::Ref(r) => stream_at(r.into()),
                MaybeRef::NotRef(_) => Err(damaged()),
            })
            .collect()
    };
    match page.raw().get_raw::<Object<'_>>(b"Contents") {
        None | Some(MaybeRef::NotRef(Object::Null(_))) => Ok(Vec::new()),
        Some(MaybeRef::NotRef(Object::Array(array))) => members(array),
        Some(MaybeRef::NotRef(_)) => Err(damaged()),
        Some(MaybeRef::Ref(r)) => match xref.get::<Object<'_>>(r.into()) {
            Some(Object::Stream(stream)) => Ok(vec![stream]),
            Some(Object::Array(array)) => members(array),
            Some(Object::Null(_)) => Ok(Vec::new()),
            _ => Err(damaged()),
        },
    }
}

/// The decoded size of a page's content, from the raw-byte check. A content
/// stream the check did not measure stops the reading.
fn content_size(page: &Page<'_>, pdf: &Pdf, checked: &Preflight) -> Result<usize, Stop> {
    let streams = content_streams(page, pdf.xref())?;
    let mut total = 0usize;
    let unchecked = || Stop::unsupported("A page's content could not be checked before reading.");
    for stream in streams {
        let Some((check, length)) = stream_check(pdf, &stream, checked) else {
            return Err(unchecked());
        };
        if let Some(refusal) = &check.refusal {
            return Err(refusal.clone());
        }
        if !check.lengths.binary_search(&length).is_ok() {
            return Err(unchecked());
        }
        if check.damaged {
            return Err(Stop::failed("A page's content stream is damaged."));
        }
        if check.image_codec {
            return Err(Stop::unsupported("A page's content uses an image codec."));
        }
        total = total.saturating_add(check.decoded);
    }
    if total > MAX_PAGE_CONTENT_BYTES {
        return Err(Stop::unsupported(format!(
            "A page's content is larger than this reader's limit ({} MiB).",
            MAX_PAGE_CONTENT_BYTES >> 20
        )));
    }
    Ok(total)
}

fn read_page<'a>(
    page: &Page<'a>,
    pdf: &'a Pdf,
    cache: &InterpreterCache<'a>,
    settings: InterpreterSettings,
    budget: &mut Budget,
) -> PageFacts {
    let (width, height) = page.render_dimensions();
    let area = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let mut context = Context::new(
        page.initial_transform(true).to_kurbo(),
        area,
        cache,
        pdf.xref(),
        settings,
    );
    let mut collector = Collector {
        glyphs: Vec::new(),
        unmapped: 0,
        outside: 0,
        images: 0,
        image_area: 0.0,
        drawings: 0,
        rules: Rules::default(),
        page: area,
        budget,
        previous_run: Vec::new(),
    };
    interpret_page(page, &mut context, &mut collector);

    let lines = layout::lines(&collector.glyphs);
    let math_chars = lines
        .iter()
        .flat_map(|line| line.text.chars())
        .filter(|&c| layout::is_math(c))
        .count();
    let (annotations, widgets) = annotations(page);
    let page_area = area.area().max(1.0);
    let shows_text = collector.glyphs.is_empty()
        && collector.unmapped == 0
        && collector.outside == 0
        && shows_text(page);
    PageFacts {
        glyphs: collector.glyphs.iter().filter(|g| g.text != " ").count(),
        unmapped: collector.unmapped,
        outside: collector.outside,
        images: collector.images,
        image_cover: (collector.image_area / page_area).min(1.0),
        grid: collector.rules.is_grid(),
        drawings: collector.drawings,
        math: math_chars >= 2 || uses_math_font(page),
        annotations,
        widgets,
        undecoded_text: shows_text,
        lines,
    }
}

/// The page's own content stream has a text-showing operator.
fn shows_text(page: &Page<'_>) -> bool {
    let mut operations = page.typed_operations();
    while let Some(op) = operations.next() {
        if matches!(
            op,
            TypedInstruction::ShowText(_)
                | TypedInstruction::ShowTexts(_)
                | TypedInstruction::NextLineAndShowText(_)
                | TypedInstruction::ShowTextWithParameters(_)
        ) {
            return true;
        }
    }
    false
}

/// (annotations other than links and pop-ups, form-field widgets)
fn annotations(page: &Page<'_>) -> (usize, usize) {
    let Some(list) = page.raw().get::<Array<'_>>(b"Annots") else {
        return (0, 0);
    };
    let mut other = 0;
    let mut widgets = 0;
    for annotation in list.iter::<Dict<'_>>() {
        match annotation
            .get::<Name<'_>>(b"Subtype")
            .as_ref()
            .map(Name::as_str)
        {
            Some("Link" | "Popup") => {}
            Some("Widget") => widgets += 1,
            _ => other += 1,
        }
    }
    (other, widgets)
}

/// Font names of TeX and OpenType math fonts on the page.
fn uses_math_font(page: &Page<'_>) -> bool {
    let fonts = &page.resources().fonts;
    fonts.keys().any(|key| {
        fonts
            .get::<Dict<'_>>(key.as_str())
            .and_then(|font| font.get::<Name<'_>>(b"BaseFont"))
            .is_some_and(|name| {
                let name = name.as_str();
                let name = name.split_once('+').map_or(name, |(_, rest)| rest);
                ["CMMI", "CMSY", "CMEX", "MSAM", "MSBM", "Math"]
                    .iter()
                    .any(|part| name.contains(part))
            })
    })
}

/// Horizontal and vertical straight lines, to spot ruled tables.
#[derive(Debug, Default)]
struct Rules {
    across: Vec<f64>,
    down: Vec<f64>,
}

const RULE_MIN: f64 = 10.0;
const RULE_SLACK: f64 = 1.0;
const MAX_RULES: usize = 10_000;

impl Rules {
    /// A grid: at least three separate horizontal and three separate
    /// vertical lines (positions closer than 3 points count as one).
    fn is_grid(&self) -> bool {
        distinct(&self.across) >= 3 && distinct(&self.down) >= 3
    }
}

fn distinct(positions: &[f64]) -> usize {
    let mut sorted = positions.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut count = 0;
    let mut last = f64::NEG_INFINITY;
    for p in sorted {
        if p - last > 3.0 {
            count += 1;
            last = p;
        }
    }
    count
}

struct Collector<'b> {
    glyphs: Vec<Glyph>,
    unmapped: usize,
    outside: usize,
    images: usize,
    image_area: f64,
    drawings: usize,
    rules: Rules,
    page: Rect,
    budget: &'b mut Budget,
    /// The previous glyph run, to drop the copy drawn by fill-and-stroke
    /// text or by "fake bold" (the same text drawn twice, slightly offset).
    previous_run: Vec<(String, Point, f64)>,
}

impl Device<'_> for Collector<'_> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'_>, _: &DrawMode) {
        self.budget.event();
        let path = props.transform * path.clone();
        let mut curved = false;
        for segment in path.segments() {
            match segment {
                PathSeg::Line(line) => {
                    let (dx, dy) = (line.p1.x - line.p0.x, line.p1.y - line.p0.y);
                    let rules = &mut self.rules;
                    if rules.across.len() + rules.down.len() >= MAX_RULES {
                        continue;
                    }
                    if dy.abs() <= RULE_SLACK && dx.abs() >= RULE_MIN {
                        rules.across.push((line.p0.y + line.p1.y) / 2.0);
                    } else if dx.abs() <= RULE_SLACK && dy.abs() >= RULE_MIN {
                        rules.down.push((line.p0.x + line.p1.x) / 2.0);
                    } else if dx.abs() > RULE_SLACK && dy.abs() > RULE_SLACK {
                        curved = true;
                    }
                }
                PathSeg::Quad(_) | PathSeg::Cubic(_) => curved = true,
            }
        }
        if curved {
            self.drawings += 1;
        }
    }

    fn push_clip_path(&mut self, _: &ClipPath) {
        self.budget.event();
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, '_>, props: DrawProps<'_>, mode: &DrawMode) {
        self.budget.event();
        let invisible = matches!(mode, DrawMode::Invisible);
        let mut current = Vec::new();
        for positioned in run.glyphs() {
            self.budget.glyph();
            let full = props.transform * positioned.transform();
            let origin = full * Point::ZERO;
            let along = (full * Point::new(1000.0, 0.0)) - origin;
            let size = along.hypot();
            if !size.is_finite() || size < 0.05 {
                self.outside += 1;
                continue;
            }
            let dir = along / size;
            let Some(text) = positioned.as_unicode().and_then(|unicode| match unicode {
                BfString::Char(c) => layout::clean(c.encode_utf8(&mut [0; 4])),
                BfString::String(s) => layout::clean(&s),
            }) else {
                self.unmapped += 1;
                continue;
            };
            if !self.page.inflate(size, size).contains(origin) {
                self.outside += 1;
                continue;
            }
            let advance = match &**positioned {
                HayroGlyph::Outline(outline) => outline
                    .advance_width()
                    .map(f64::from)
                    .filter(|a| a.is_finite() && *a > 0.0),
                HayroGlyph::Type3(_) => None,
            };
            let end = match advance {
                Some(advance) => full * Point::new(advance, 0.0),
                None => origin + dir * (size * 0.5),
            };
            current.push(Glyph {
                text,
                origin,
                end,
                dir,
                size,
                invisible,
            });
        }
        let key: Vec<(String, Point, f64)> = current
            .iter()
            .map(|g| (g.text.clone(), g.origin, g.size))
            .collect();
        if !key.is_empty() && repeats(&self.previous_run, &key) {
            return;
        }
        self.previous_run = key;
        self.glyphs.extend(current);
    }

    fn draw_image(&mut self, image: Image<'_, '_>, props: ImageDrawProps<'_>) {
        self.budget.event();
        self.images += 1;
        // hayro's transform maps the image's pixel rectangle onto the page.
        let pixels = Rect::new(
            0.0,
            0.0,
            f64::from(image.width()),
            f64::from(image.height()),
        );
        let bounds = (props.transform * pixels.to_path(0.1)).bounding_box();
        self.image_area += bounds.intersect(self.page).area().max(0.0);
    }

    fn pop_clip(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

/// The same glyphs at (nearly) the same places as the previous run.
fn repeats(previous: &[(String, Point, f64)], current: &[(String, Point, f64)]) -> bool {
    previous.len() == current.len()
        && previous
            .iter()
            .zip(current)
            .all(|(a, b)| a.0 == b.0 && (a.1 - b.1).hypot() <= 0.15 * a.2.max(b.2) && a.2 > 0.0)
}
