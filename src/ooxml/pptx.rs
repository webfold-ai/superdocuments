//! PPTX to Markdown.
//!
//! Slides come out in *presentation* order (`sldIdLst` in `presentation.xml`, resolved through
//! its relationships) and are numbered by position, not by the number in the file name, which
//! stops matching as soon as a slide is moved. Each slide gives its title, its text one
//! paragraph at a time (a body paragraph without its own bullet setting takes the bullet of its
//! slide layout's placeholder, then of the master's body style, level by level), tables as Markdown tables, hyperlinks, and the speaker notes. Footers, dates and
//! slide-number placeholders are dropped.
//!
//! Chart titles and data and the text of SmartArt diagrams are read. Not read: comments, and text that only the layout
//! or master carries.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{
    attr_local, attrs_local, main_part, markdown_alt, read_rels, rels_path_of, render, resolve, Seg,
};
use crate::document::{archive, entry, markdown_table, push_ref, text};

#[derive(Default)]
struct Content {
    title: Vec<String>,
    /// (is list item, text)
    body: Vec<(bool, String)>,
}

#[derive(Default)]
struct Cells {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: Vec<String>,
    continued: bool,
}

#[derive(Default)]
struct Para {
    segs: Vec<Seg>,
    /// `Some(ordered)` for an explicit bullet.
    bullet: Option<bool>,
    /// The paragraph says something about its bullet (`buChar`, `buAutoNum` or `buNone`).
    bullet_set: bool,
    level: u32,
}

/// The bullet of a list level: `Some(false)` a character, `Some(true)` a number, `None` no bullet.
type Bullet = Option<bool>;

/// Bullets a slide inherits: per layout placeholder and level, and per level from the master's
/// body style.
#[derive(Default)]
struct Inherited {
    layout: HashMap<(String, u32), Bullet>,
    master: HashMap<u32, Bullet>,
}

impl Inherited {
    fn resolve(&self, key: &str, level: u32) -> Bullet {
        self.layout
            .get(&(key.to_owned(), level))
            .or_else(|| self.master.get(&level))
            .copied()
            .flatten()
    }
}

/// A chart part as Markdown: `Chart: <title>` and a table with one column per series. Titles of
/// axes are not the chart's title and are left out.
fn chart_markdown(xml: &[u8]) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Part {
        None,
        Name,
        Categories,
        Values,
    }
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let (mut title, mut in_title, mut in_plot) = (String::new(), false, false);
    type Points = Vec<(usize, String)>;
    let mut series: Vec<(String, Points, Points)> = Vec::new();
    let (mut part, mut point, mut in_v, mut in_t) = (Part::None, 0usize, false, false);
    loop {
        let Ok(event) = reader.read_event_into(&mut buf) else {
            break;
        };
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let name = tag.local_name().as_ref().to_owned();
                match name.as_str() {
                    "plotArea" => in_plot = true,
                    "title" if !in_plot => in_title = true,
                    "ser" => series.push((String::new(), Vec::new(), Vec::new())),
                    "tx" if !series.is_empty() => part = Part::Name,
                    "cat" | "xVal" if !series.is_empty() => part = Part::Categories,
                    "val" | "yVal" if !series.is_empty() => part = Part::Values,
                    "pt" => {
                        point = attr_local(tag, "idx")
                            .and_then(|i| i.parse().ok())
                            .unwrap_or(0)
                    }
                    "v" => in_v = true,
                    "t" if in_title => in_t = true,
                    _ => {}
                }
            }
            Event::End(tag) => {
                let name = tag.local_name().as_ref().to_owned();
                match name.as_str() {
                    "plotArea" => in_plot = false,
                    "title" => in_title = false,
                    "tx" | "cat" | "xVal" | "val" | "yVal" => part = Part::None,
                    "v" => in_v = false,
                    "t" => in_t = false,
                    _ => {}
                }
            }
            Event::Text(value) => {
                let t = text(value.as_ref());
                if in_t {
                    title.push_str(&t);
                } else if in_v {
                    if let Some(s) = series.last_mut() {
                        match part {
                            Part::Name => s.0.push_str(&t),
                            Part::Categories => s.1.push((point, t)),
                            Part::Values => s.2.push((point, t)),
                            Part::None => {}
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let mut out = format!("Chart: {}", title.trim())
        .trim_end()
        .trim_end_matches(':')
        .to_owned();
    let rows = series
        .iter()
        .flat_map(|s| s.1.iter().chain(s.2.iter()).map(|p| p.0 + 1))
        .max()
        .unwrap_or(0);
    if !series.is_empty() && rows > 0 {
        let mut table = vec![std::iter::once(String::new())
            .chain(series.iter().map(|s| s.0.clone()))
            .collect::<Vec<_>>()];
        for i in 0..rows {
            let label = series
                .iter()
                .find_map(|s| s.1.iter().find(|p| p.0 == i).map(|p| p.1.clone()))
                .unwrap_or_default();
            let mut row = vec![label];
            for s in &series {
                row.push(
                    s.2.iter()
                        .find(|p| p.0 == i)
                        .map(|p| p.1.clone())
                        .unwrap_or_default(),
                );
            }
            table.push(row);
        }
        out.push_str("\n\n");
        out.push_str(markdown_table(&table).trim_end());
    }
    out
}

/// The text of a SmartArt diagram's data model: one bullet per node (`dgm:pt` of type `node`, `asst`
/// or `doc`), in the order the model lists them. Presentation, parent- and sibling-transition points
/// carry layout, not text.
fn diagram_markdown(xml: &[u8]) -> String {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let (mut in_node, mut in_t) = (false, false);
    let mut current = String::new();
    let mut items = Vec::new();
    loop {
        let Ok(event) = reader.read_event_into(&mut buf) else {
            break;
        };
        match event {
            Event::Start(ref tag) => match tag.local_name().as_ref() {
                "pt" => {
                    in_node = attr_local(tag, "type")
                        .is_none_or(|t| !matches!(t.as_str(), "pres" | "parTrans" | "sibTrans"));
                    current.clear();
                }
                "t" if in_node => in_t = true,
                _ => {}
            },
            Event::End(tag) => match tag.local_name().as_ref() {
                "pt" => {
                    let text = current.split_whitespace().collect::<Vec<_>>().join(" ");
                    if in_node && !text.is_empty() {
                        items.push(format!("- {text}"));
                    }
                    in_node = false;
                }
                "t" => {
                    in_t = false;
                    current.push(' ');
                }
                _ => {}
            },
            Event::Text(value) if in_t => current.push_str(&text(value.as_ref())),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    items.join("\n")
}

/// `lvl3pPr` -> `Some(2)`.
fn level_of(name: &str) -> Option<u32> {
    name.strip_prefix("lvl")?
        .strip_suffix("pPr")?
        .parse::<u32>()
        .ok()?
        .checked_sub(1)
}

/// The key a placeholder is matched by between a slide and its layout: `idx`, else its type.
fn placeholder_key(tag: &BytesStart<'_>) -> String {
    attr_local(tag, "idx")
        .map(|i| format!("idx:{i}"))
        .unwrap_or_else(|| {
            format!(
                "type:{}",
                attr_local(tag, "type").unwrap_or_else(|| "body".to_owned())
            )
        })
}

/// Reads the list-level bullets out of a layout (`lstStyle` of each placeholder) or a master
/// (`bodyStyle`).
fn read_bullets(xml: &[u8], master: bool, inherited: &mut Inherited) {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let (mut key, mut level) = (None::<String>, None::<u32>);
    let (mut in_list_style, mut in_body_style) = (false, false);
    loop {
        let Ok(event) = reader.read_event_into(&mut buf) else {
            return;
        };
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let name = tag.local_name().as_ref().to_owned();
                match name.as_str() {
                    "sp" => key = None,
                    "ph" => key = Some(placeholder_key(tag)),
                    "lstStyle" => in_list_style = true,
                    "bodyStyle" => in_body_style = true,
                    "buChar" | "buAutoNum" | "buNone" => {
                        if let Some(l) = level {
                            let bullet = match name.as_str() {
                                "buChar" => Some(false),
                                "buAutoNum" => Some(true),
                                _ => None,
                            };
                            if master && in_body_style {
                                inherited.master.entry(l).or_insert(bullet);
                            } else if !master && in_list_style {
                                if let Some(k) = key.clone() {
                                    inherited.layout.entry((k, l)).or_insert(bullet);
                                }
                            }
                        }
                    }
                    other => {
                        if let Some(l) = level_of(other) {
                            level = Some(l);
                        }
                    }
                }
                if matches!(event, Event::Empty(_)) {
                    let n = tag.local_name().as_ref().to_owned();
                    match n.as_str() {
                        "lstStyle" => in_list_style = false,
                        "bodyStyle" => in_body_style = false,
                        _ => {
                            if level_of(&n).is_some() {
                                level = None;
                            }
                        }
                    }
                }
            }
            Event::End(tag) => {
                let n = tag.local_name().as_ref().to_owned();
                match n.as_str() {
                    "lstStyle" => in_list_style = false,
                    "bodyStyle" => in_body_style = false,
                    _ => {
                        if level_of(&n).is_some() {
                            level = None;
                        }
                    }
                }
            }
            Event::Eof => return,
            _ => {}
        }
        buf.clear();
    }
}

struct Slide<'a> {
    links: &'a HashMap<String, String>,
    /// Image relationship ids to the file name written in the Markdown (`media/<name>`).
    images: &'a HashMap<String, String>,
    /// The `descr` of the most recent `cNvPr`, the alt text of the picture that follows it.
    alt: Option<String>,
    notes: bool,
    content: Content,
    para: Option<Para>,
    placeholder: Option<String>,
    placeholder_key: Option<String>,
    inherited: &'a Inherited,
    /// Chart relationship ids to the chart rendered as Markdown.
    charts: &'a HashMap<String, String>,
    table: Option<Cells>,
    in_tc: bool,
    in_run: bool,
    in_rpr: bool,
    in_t: bool,
    bold: bool,
    italic: bool,
    href: Option<String>,
    skip: usize,
    counters: HashMap<u32, u32>,
}

impl<'a> Slide<'a> {
    fn add(&mut self, value: &str) {
        let (bold, italic, href) = (self.bold, self.italic, self.href.clone());
        let Some(para) = self.para.as_mut() else {
            return;
        };
        if let Some(Seg::Text {
            text,
            bold: b,
            italic: i,
            href: h,
            ..
        }) = para.segs.last_mut()
        {
            if *b == bold && *i == italic && *h == href {
                text.push_str(value);
                return;
            }
        }
        para.segs.push(Seg::Text {
            text: value.to_owned(),
            bold,
            italic,
            marks: 0,
            href,
        });
    }

    fn start(&mut self, tag: &BytesStart<'_>) {
        if self.skip > 0 {
            self.skip += 1;
            return;
        }
        let name = tag.local_name().as_ref().to_owned();
        if name == "Fallback" {
            self.skip = 1;
            return;
        }
        match name.as_str() {
            "ph" => {
                self.placeholder =
                    Some(attr_local(tag, "type").unwrap_or_else(|| "body".to_owned()));
                self.placeholder_key = Some(placeholder_key(tag));
            }
            "p" => self.para = Some(Para::default()),
            "pPr" => {
                if let Some(p) = self.para.as_mut() {
                    p.level = attr_local(tag, "lvl")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                }
            }
            "buChar" => {
                if let Some(p) = self.para.as_mut() {
                    p.bullet = Some(false);
                    p.bullet_set = true;
                }
            }
            "buAutoNum" => {
                if let Some(p) = self.para.as_mut() {
                    p.bullet = Some(true);
                    p.bullet_set = true;
                }
            }
            "buNone" => {
                if let Some(p) = self.para.as_mut() {
                    p.bullet = None;
                    p.bullet_set = true;
                }
            }
            "r" | "fld" => {
                self.in_run = true;
                self.bold = false;
                self.italic = false;
                self.href = None;
            }
            "rPr" if self.in_run => {
                self.in_rpr = true;
                self.bold = attr_local(tag, "b").is_some_and(|v| v == "1" || v == "true");
                self.italic = attr_local(tag, "i").is_some_and(|v| v == "1" || v == "true");
            }
            "hlinkClick" if self.in_rpr => {
                self.href = attrs_local(tag, "id")
                    .iter()
                    .find_map(|id| self.links.get(id).cloned());
            }
            "cNvPr" => {
                self.alt = attr_local(tag, "descr")
                    .map(|d| d.trim().to_owned())
                    .filter(|d| !d.is_empty());
            }
            "blip" => {
                if let Some(id) = attr_local(tag, "embed") {
                    if let Some(file) = self.images.get(&id) {
                        let alt = self.alt.take().unwrap_or_default();
                        let markdown = format!("![{}](media/{file})", markdown_alt(&alt));
                        match self.para.as_mut() {
                            Some(para) => para.segs.push(Seg::Raw(markdown)),
                            None => self.content.body.push((false, markdown)),
                        }
                    }
                }
            }
            "t" => self.in_t = true, // lenient: `t` outside `r`
            "br" if self.para.is_some() => {
                if let Some(p) = self.para.as_mut() {
                    p.segs.push(Seg::Br);
                }
            }
            "chart" if !self.notes => {
                if let Some(markdown) = attrs_local(tag, "id")
                    .iter()
                    .find_map(|id| self.charts.get(id))
                {
                    self.content.body.push((false, markdown.clone()));
                }
            }
            // a SmartArt graphic frame: its text is in the diagram data part named by `r:dm`
            "relIds" if !self.notes => {
                if let Some(markdown) = attrs_local(tag, "dm")
                    .iter()
                    .find_map(|id| self.charts.get(id))
                {
                    self.content.body.push((true, markdown.clone()));
                }
            }
            "tbl" => self.table = Some(Cells::default()),
            "tr" => {
                if let Some(t) = self.table.as_mut() {
                    t.row.clear();
                }
            }
            "tc" => {
                self.in_tc = true;
                if let Some(t) = self.table.as_mut() {
                    t.cell.clear();
                    t.continued = ["hMerge", "vMerge"]
                        .iter()
                        .any(|a| attr_local(tag, a).is_some_and(|v| v == "1" || v == "true"));
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, name: &str) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        match name {
            "t" => self.in_t = false,
            "rPr" => self.in_rpr = false,
            "r" | "fld" => {
                self.in_run = false;
                self.in_rpr = false;
            }
            "sp" => {
                self.placeholder = None;
                self.placeholder_key = None;
            }
            "p" => self.finish_paragraph(),
            "tc" => {
                self.in_tc = false;
                if let Some(t) = self.table.as_mut() {
                    let cell = if t.continued {
                        String::new()
                    } else {
                        t.cell.join("<br>")
                    };
                    t.row.push(cell);
                }
            }
            "tr" => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            "tbl" => {
                if let Some(t) = self.table.take() {
                    let markdown = markdown_table(&t.rows);
                    if !markdown.is_empty() && !self.notes {
                        self.content
                            .body
                            .push((false, markdown.trim_end().to_owned()));
                    }
                }
            }
            _ => {}
        }
    }

    fn finish_paragraph(&mut self) {
        let Some(para) = self.para.take() else { return };
        if self.in_tc {
            let line = render(&para.segs, "<br>", true);
            if let (Some(t), false) = (self.table.as_mut(), line.trim().is_empty()) {
                t.cell.push(line.trim().to_owned());
            }
            return;
        }
        let placeholder = self.placeholder.as_deref().unwrap_or("");
        if matches!(placeholder, "sldNum" | "dt" | "ftr" | "hdr" | "sldImg") {
            return;
        }
        if self.notes && placeholder != "body" {
            return;
        }
        if matches!(placeholder, "title" | "ctrTitle") {
            let line = render(&para.segs, " ", false);
            if !line.trim().is_empty() {
                self.content.title.push(line.trim().to_owned());
            }
            return;
        }
        let line = render(&para.segs, "  \n", true);
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let bullet = match (&self.placeholder_key, para.bullet_set) {
            (Some(key), false) if placeholder == "body" && !self.notes => {
                self.inherited.resolve(key, para.level)
            }
            _ => para.bullet,
        };
        match bullet {
            Some(ordered) => {
                self.counters.retain(|l, _| *l <= para.level);
                let marker = if ordered {
                    let n = self.counters.entry(para.level).or_insert(0);
                    *n += 1;
                    format!("{n}. ")
                } else {
                    "- ".to_owned()
                };
                let indent = "    ".repeat(para.level.min(8) as usize);
                self.content.body.push((
                    true,
                    format!("{indent}{marker}{}", line.replace("  \n", " ")),
                ));
            }
            None => {
                self.counters.clear();
                self.content.body.push((false, line.to_owned()));
            }
        }
    }
}

fn read_part(
    xml: &[u8],
    links: &HashMap<String, String>,
    images: &HashMap<String, String>,
    inherited: &Inherited,
    charts: &HashMap<String, String>,
    notes: bool,
) -> Result<Content, String> {
    let mut slide = Slide {
        links,
        images,
        inherited,
        charts,
        placeholder_key: None,
        alt: None,
        notes,
        content: Content::default(),
        para: None,
        placeholder: None,
        table: None,
        in_tc: false,
        in_run: false,
        in_rpr: false,
        in_t: false,
        bold: false,
        italic: false,
        href: None,
        skip: 0,
        counters: HashMap::new(),
    };
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) => slide.start(&tag),
            Event::Empty(tag) => {
                let name = tag.local_name().as_ref().to_owned();
                slide.start(&tag);
                slide.end(&name);
            }
            Event::End(tag) => {
                let name = tag.local_name().as_ref().to_owned();
                slide.end(&name);
            }
            Event::Text(value) if slide.skip == 0 && slide.in_t => slide.add(&text(value.as_ref())),
            Event::GeneralRef(r) if slide.skip == 0 && slide.in_t => {
                let mut s = String::new();
                push_ref(&mut s, &r);
                slide.add(&s);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(slide.content)
}

fn join(items: &[(bool, String)]) -> String {
    let mut out = String::new();
    let mut previous_item = false;
    for (item, body) in items {
        if !out.is_empty() {
            out.push_str(if *item && previous_item { "\n" } else { "\n\n" });
        }
        out.push_str(body);
        previous_item = *item;
    }
    out
}

/// Slide part names in presentation order.
pub(crate) fn slide_order(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
) -> Result<Vec<String>, String> {
    let mut ordered = Vec::new();
    if let Ok(presentation) = main_part(zip, "ppt/presentation.xml") {
        let rels = read_rels(zip, &rels_path_of(&presentation))?;
        let xml = entry(zip, &presentation)?;
        let mut reader = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) | Event::Empty(tag) if tag.local_name().as_ref() == "sldId" => {
                    let ids = attrs_local(&tag, "id");
                    let target = rels
                        .iter()
                        .find(|r| r.kind == "slide" && !r.external && ids.contains(&r.id))
                        .and_then(|r| resolve(&presentation, &r.target));
                    if let Some(part) = target {
                        ordered.push(part);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }
    if !ordered.is_empty() {
        return Ok(ordered);
    }
    // No usable presentation part: fall back to the numbers in the file names.
    let mut numbered = Vec::new();
    for i in 0..zip.len() {
        let name = zip
            .by_index_raw(i)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if let Some(n) = name
            .strip_prefix("ppt/slides/slide")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<usize>().ok())
        {
            numbered.push((n, name));
        }
    }
    numbered.sort();
    Ok(numbered.into_iter().map(|(_, name)| name).collect())
}

/// Bullets from the slide's layout and that layout's master. A missing or unreadable part gives none.
fn inherited_bullets(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    slide: &str,
    rels: &[super::Rel],
) -> Inherited {
    let mut inherited = Inherited::default();
    let Some(layout) = rels
        .iter()
        .find(|r| r.kind == "slideLayout" && !r.external)
        .and_then(|r| resolve(slide, &r.target))
    else {
        return inherited;
    };
    if let Ok(xml) = entry(zip, &layout) {
        read_bullets(&xml, false, &mut inherited);
    }
    let master = read_rels(zip, &rels_path_of(&layout)).ok().and_then(|r| {
        r.into_iter()
            .find(|r| r.kind == "slideMaster" && !r.external)
            .and_then(|r| resolve(&layout, &r.target))
    });
    if let Some(master) = master {
        if let Ok(xml) = entry(zip, &master) {
            read_bullets(&xml, true, &mut inherited);
        }
    }
    inherited
}

pub(crate) fn convert(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let order = slide_order(&mut zip)?;
    let mut out = String::new();
    for (position, part) in order.iter().enumerate() {
        let rels = read_rels(&mut zip, &rels_path_of(part))?;
        let links: HashMap<String, String> = rels
            .iter()
            .filter(|r| r.kind == "hyperlink" && r.external)
            .map(|r| (r.id.clone(), r.target.clone()))
            .collect();
        let images: HashMap<String, String> = super::image_refs(&rels, part)
            .into_iter()
            .map(|(id, path)| (id, super::media_name(&path).to_owned()))
            .collect();
        let inherited = inherited_bullets(&mut zip, part, &rels);
        // charts and SmartArt diagrams, by the relationship id the slide uses for them
        let charts: HashMap<String, String> = rels
            .iter()
            .filter(|r| matches!(r.kind.as_str(), "chart" | "diagramData") && !r.external)
            .filter_map(|r| {
                let xml = entry(&mut zip, &resolve(part, &r.target)?).ok()?;
                let markdown = if r.kind == "chart" {
                    chart_markdown(&xml)
                } else {
                    diagram_markdown(&xml)
                };
                Some((r.id.clone(), markdown))
            })
            .collect();
        let slide = read_part(
            &entry(&mut zip, part)?,
            &links,
            &images,
            &inherited,
            &charts,
            false,
        )?;
        let mut notes = Content::default();
        if let Some(notes_part) = rels
            .iter()
            .find(|r| r.kind == "notesSlide" && !r.external)
            .and_then(|r| resolve(part, &r.target))
        {
            notes = read_part(
                &entry(&mut zip, &notes_part)?,
                &HashMap::new(),
                &HashMap::new(),
                &Inherited::default(),
                &HashMap::new(),
                true,
            )?;
        }
        let body = join(&slide.body);
        let notes_text = notes
            .body
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if slide.title.is_empty() && body.is_empty() && notes_text.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n---\n\n");
        }
        out.push_str(&format!("## Slide {}", position + 1));
        if !slide.title.is_empty() {
            out.push_str(&format!(": {}", slide.title.join(" ")));
        }
        if !body.is_empty() {
            out.push_str("\n\n");
            out.push_str(&body);
        }
        if !notes_text.trim().is_empty() {
            out.push_str(&format!("\n\n**Notes:** {}", notes_text.trim()));
        }
    }
    Ok(out)
}
