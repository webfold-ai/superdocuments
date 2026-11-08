//! DOCX / DOCM to Markdown.
//!
//! Reads the package the way Word lays it out rather than guessing from element prefixes:
//! `styles.xml` decides what a heading is (name, `outlineLvl`, `basedOn` chain), `numbering.xml`
//! decides bullet or number, `document.xml.rels` supplies hyperlink targets, and
//! `footnotes.xml` / `endnotes.xml` become Markdown footnotes. Tracked deletions are left out,
//! the `Fallback` half of a `mc:AlternateContent` is skipped so text boxes are not read twice,
//! and tables keep `gridSpan` and `vMerge` positions.
//!
//! Not read: headers and footers, comments, SmartArt layout (its text is appended as a
//! `Diagram:` block) and chart parts other than their text and cached values.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{
    attr_local, attrs_local, main_part, read_rels, rels_path_of, render, resolve, Rel, Seg,
};
use crate::document::{archive, entry, markdown_table, push_ref, text, xml_text};

#[derive(Default)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u32>,
    num: Option<(String, u32)>,
}

#[derive(Default)]
struct Styles(HashMap<String, Style>);

impl Styles {
    fn parse(xml: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        let mut map = HashMap::new();
        let mut current: Option<(String, Style)> = None;
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) | Event::Empty(tag) => {
                    let name = tag.local_name().as_ref().to_owned();
                    match (name.as_str(), current.as_mut()) {
                        ("style", _) => {
                            current = Some((
                                attr_local(&tag, "styleId").unwrap_or_default(),
                                Style::default(),
                            ));
                        }
                        ("name", Some((_, s))) => {
                            s.name = attr_local(&tag, "val").unwrap_or_default()
                        }
                        ("basedOn", Some((_, s))) => s.based_on = attr_local(&tag, "val"),
                        ("outlineLvl", Some((_, s))) => {
                            s.outline = attr_local(&tag, "val").and_then(|v| v.parse().ok());
                        }
                        ("numId", Some((_, s))) => {
                            let id = attr_local(&tag, "val").unwrap_or_default();
                            let lvl = s.num.as_ref().map_or(0, |n| n.1);
                            s.num = Some((id, lvl));
                        }
                        ("ilvl", Some((_, s))) => {
                            let lvl = attr_local(&tag, "val")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(0);
                            let id = s.num.take().map(|n| n.0).unwrap_or_default();
                            s.num = Some((id, lvl));
                        }
                        _ => {}
                    }
                }
                Event::End(tag) if tag.local_name().as_ref() == "style" => {
                    if let Some((id, style)) = current.take() {
                        map.insert(id, style);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(Self(map))
    }

    /// The style and then every style it is based on, nearest first.
    fn chain<'a>(&'a self, id: &str) -> Vec<&'a Style> {
        let mut out = Vec::new();
        let mut next = self.0.get(id);
        while let Some(style) = next {
            if out.len() >= 16 {
                break;
            }
            out.push(style);
            next = style.based_on.as_deref().and_then(|b| self.0.get(b));
        }
        out
    }

    /// Markdown heading depth (1-6) of a paragraph style, if it is a heading or the title.
    fn heading_level(&self, id: &str) -> Option<usize> {
        for style in self.chain(id) {
            if let Some(level) = style.outline {
                return (level < 9).then(|| (level as usize + 1).min(6));
            }
            if let Some(level) = name_level(&style.name) {
                return Some(level);
            }
        }
        // No styles.xml entry: fall back to the conventional ids.
        name_level(id)
    }

    fn list(&self, id: &str) -> Option<(String, u32)> {
        self.chain(id).into_iter().find_map(|s| s.num.clone())
    }

    fn name(&self, id: &str) -> String {
        self.0
            .get(id)
            .map_or_else(|| id.to_owned(), |s| s.name.clone())
            .to_lowercase()
    }
}

fn name_level(name: &str) -> Option<usize> {
    let lower = name.to_lowercase().replace(' ', "");
    if lower == "title" {
        return Some(1);
    }
    lower
        .strip_prefix("heading")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .map(|n| n.min(6))
}

#[derive(Default)]
struct Numbering {
    /// numId -> abstractNumId
    nums: HashMap<String, String>,
    /// abstractNumId -> ilvl -> numFmt
    formats: HashMap<String, HashMap<u32, String>>,
}

impl Numbering {
    fn parse(xml: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        let mut out = Self::default();
        let (mut abstract_id, mut level, mut num_id): (
            Option<String>,
            Option<u32>,
            Option<String>,
        ) = (None, None, None);
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                    "abstractNum" => abstract_id = attr_local(&tag, "abstractNumId"),
                    "lvl" if abstract_id.is_some() => {
                        level = attr_local(&tag, "ilvl").and_then(|v| v.parse().ok());
                    }
                    "numFmt" => {
                        if let (Some(a), Some(l), Some(f)) =
                            (abstract_id.as_ref(), level, attr_local(&tag, "val"))
                        {
                            out.formats
                                .entry(a.clone())
                                .or_default()
                                .entry(l)
                                .or_insert(f);
                        }
                    }
                    "num" => num_id = attr_local(&tag, "numId"),
                    "abstractNumId" if abstract_id.is_none() => {
                        if let (Some(n), Some(a)) = (num_id.as_ref(), attr_local(&tag, "val")) {
                            out.nums.insert(n.clone(), a);
                        }
                    }
                    _ => {}
                },
                Event::End(tag) => match tag.local_name().as_ref() {
                    "abstractNum" => {
                        abstract_id = None;
                        level = None;
                    }
                    "lvl" => level = None,
                    "num" => num_id = None,
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(out)
    }

    /// `Some(true)` for a bullet, `Some(false)` for a numbered level, `None` when the level is
    /// numbered `none` (a plain paragraph).
    fn is_bullet(&self, num_id: &str, level: u32) -> Option<bool> {
        let format = self
            .nums
            .get(num_id)
            .and_then(|a| self.formats.get(a))
            .and_then(|levels| levels.get(&level));
        match format.map(String::as_str) {
            None | Some("bullet") => Some(true),
            Some("none") => None,
            Some(_) => Some(false),
        }
    }
}

#[derive(Default)]
struct Para {
    segs: Vec<Seg>,
    style: Option<String>,
    outline: Option<u32>,
    num: Option<(String, Option<u32>)>,
}

enum Block {
    Heading(String),
    Para(String),
    Item(String),
    Table(String),
}

#[derive(Default)]
struct Tbl {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: Vec<String>,
    span: usize,
    continued: bool,
}

#[derive(Default, Clone)]
struct RunState {
    in_run: bool,
    in_rpr: bool,
    in_t: bool,
    bold: bool,
    italic: bool,
}

#[derive(Default)]
struct Field {
    instr: String,
    href: Option<String>,
}

struct Parser<'a> {
    styles: &'a Styles,
    numbering: &'a Numbering,
    links: HashMap<String, String>,
    paras: Vec<Para>,
    tables: Vec<Tbl>,
    blocks: Vec<Block>,
    run: RunState,
    saved: Vec<RunState>,
    in_ppr: bool,
    in_instr: bool,
    fields: Vec<Field>,
    hyperlink: Option<String>,
    skip: usize,
    counters: HashMap<(String, u32), u32>,
    references: Vec<String>,
    note: Option<String>,
    notes: Vec<(String, Vec<Block>)>,
    outer: Vec<Block>,
}

impl<'a> Parser<'a> {
    fn new(styles: &'a Styles, numbering: &'a Numbering, links: HashMap<String, String>) -> Self {
        Self {
            styles,
            numbering,
            links,
            paras: Vec::new(),
            tables: Vec::new(),
            blocks: Vec::new(),
            run: RunState::default(),
            saved: Vec::new(),
            in_ppr: false,
            in_instr: false,
            fields: Vec::new(),
            hyperlink: None,
            skip: 0,
            counters: HashMap::new(),
            references: Vec::new(),
            note: None,
            notes: Vec::new(),
            outer: Vec::new(),
        }
    }

    fn add_text(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        let href = self
            .hyperlink
            .clone()
            .or_else(|| self.fields.iter().rev().find_map(|f| f.href.clone()));
        let (bold, italic) = (self.run.bold, self.run.italic);
        let Some(para) = self.paras.last_mut() else {
            return;
        };
        if let Some(Seg::Text {
            text,
            bold: b,
            italic: i,
            href: h,
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
            href,
        });
    }

    fn push_seg(&mut self, seg: Seg) {
        if let Some(para) = self.paras.last_mut() {
            para.segs.push(seg);
        }
    }

    fn start(&mut self, tag: &BytesStart<'_>) {
        let name = tag.local_name().as_ref().to_owned();
        if self.skip > 0 {
            self.skip += 1;
            return;
        }
        if name == "Fallback"
            || matches!(
                name.as_str(),
                "pPrChange"
                    | "rPrChange"
                    | "tblPrChange"
                    | "tcPrChange"
                    | "trPrChange"
                    | "sectPrChange"
            )
        {
            self.skip = 1;
            return;
        }
        let val = || attr_local(tag, "val");
        match name.as_str() {
            "p" => self.paras.push(Para::default()),
            "pPr" => self.in_ppr = true,
            "pStyle" if self.in_ppr => {
                if let Some(p) = self.paras.last_mut() {
                    p.style = val();
                }
            }
            "outlineLvl" if self.in_ppr => {
                if let Some(p) = self.paras.last_mut() {
                    p.outline = val().and_then(|v| v.parse().ok());
                }
            }
            "numId" if self.in_ppr => {
                if let Some(p) = self.paras.last_mut() {
                    let level = p.num.as_ref().and_then(|n| n.1);
                    p.num = Some((val().unwrap_or_default(), level));
                }
            }
            "ilvl" if self.in_ppr => {
                if let Some(p) = self.paras.last_mut() {
                    let id = p.num.take().map(|n| n.0).unwrap_or_default();
                    p.num = Some((id, val().and_then(|v| v.parse().ok())));
                }
            }
            "r" => {
                self.run.in_run = true;
                self.run.bold = false;
                self.run.italic = false;
            }
            "rPr" if self.run.in_run && !self.in_ppr => self.run.in_rpr = true,
            "b" if self.run.in_rpr => self.run.bold = !is_off(val().as_deref()),
            "i" if self.run.in_rpr => self.run.italic = !is_off(val().as_deref()),
            "rStyle" if self.run.in_rpr => {
                let style = self.styles.name(&val().unwrap_or_default());
                if style.contains("strong") {
                    self.run.bold = true;
                }
                if style.contains("emphasis") {
                    self.run.italic = true;
                }
            }
            "t" => self.run.in_t = true, // lenient: older writers emit `t` outside `r`
            "instrText" => self.in_instr = true,
            "fldChar" => match val_of(tag, "fldCharType").as_deref() {
                Some("begin") => self.fields.push(Field::default()),
                Some("separate") => {
                    if let Some(field) = self.fields.last_mut() {
                        field.href = hyperlink_target(&field.instr);
                    }
                }
                Some("end") => {
                    self.fields.pop();
                }
                _ => {}
            },
            "tab" if self.run.in_run && !self.run.in_rpr && !self.in_ppr => self.add_text(" "),
            "noBreakHyphen" if self.run.in_run => self.add_text("-"),
            "br" | "cr" if self.run.in_run => {
                if !matches!(val_of(tag, "type").as_deref(), Some("page" | "column")) {
                    self.push_seg(Seg::Br);
                }
            }
            "footnoteReference" | "endnoteReference" if self.run.in_run => {
                if let Some(id) = attr_local(tag, "id") {
                    let key = if name == "endnoteReference" {
                        format!("e{id}")
                    } else {
                        id
                    };
                    self.push_seg(Seg::Raw(format!("[^{key}]")));
                    if !self.references.contains(&key) {
                        self.references.push(key);
                    }
                }
            }
            "hyperlink" => {
                self.hyperlink = attrs_local(tag, "id")
                    .iter()
                    .find_map(|id| self.links.get(id).cloned());
            }
            "docPr" => {
                if let Some(description) = attr_local(tag, "descr").filter(|d| !d.trim().is_empty())
                {
                    self.add_text(description.trim());
                }
            }
            "OLEObject" => {
                if let Some(kind) = attr_local(tag, "ProgID") {
                    self.add_text(&format!("Embedded object: {kind}"));
                }
            }
            "txbxContent" => {
                self.saved.push(self.run.clone());
                self.run = RunState::default();
            }
            "footnote" | "endnote" => {
                let kind = attr_local(tag, "type").unwrap_or_default();
                if matches!(
                    kind.as_str(),
                    "separator" | "continuationSeparator" | "continuationNotice"
                ) {
                    self.skip = 1;
                } else {
                    let id = attr_local(tag, "id").unwrap_or_default();
                    self.note = Some(if name == "endnote" {
                        format!("e{id}")
                    } else {
                        id
                    });
                    self.outer = std::mem::take(&mut self.blocks);
                }
            }
            "tbl" => self.tables.push(Tbl::default()),
            "tr" => {
                if let Some(t) = self.tables.last_mut() {
                    t.row.clear();
                }
            }
            "tc" => {
                if let Some(t) = self.tables.last_mut() {
                    t.cell.clear();
                    t.span = 1;
                    t.continued = false;
                }
            }
            "gridSpan" => {
                if let (Some(t), Some(n)) = (
                    self.tables.last_mut(),
                    val().and_then(|v| v.parse::<usize>().ok()),
                ) {
                    t.span = n.clamp(1, 1000);
                }
            }
            "vMerge" => {
                if let Some(t) = self.tables.last_mut() {
                    t.continued = val().as_deref() != Some("restart");
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
            "t" => self.run.in_t = false,
            "instrText" => self.in_instr = false,
            "r" => self.run = RunState::default(),
            "rPr" => self.run.in_rpr = false,
            "pPr" => self.in_ppr = false,
            "hyperlink" => self.hyperlink = None,
            "txbxContent" => {
                if let Some(state) = self.saved.pop() {
                    self.run = state;
                }
            }
            "p" => self.finish_paragraph(),
            "tc" => {
                if let Some(t) = self.tables.last_mut() {
                    let cell = if t.continued {
                        String::new()
                    } else {
                        t.cell.join("<br>")
                    };
                    t.row.push(cell);
                    for _ in 1..t.span {
                        t.row.push(String::new());
                    }
                }
            }
            "tr" => {
                if let Some(t) = self.tables.last_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            "tbl" => self.finish_table(),
            "footnote" | "endnote" => {
                if let Some(id) = self.note.take() {
                    let body = std::mem::replace(&mut self.blocks, std::mem::take(&mut self.outer));
                    self.notes.push((id, body));
                }
            }
            _ => {}
        }
    }

    fn finish_table(&mut self) {
        let Some(table) = self.tables.pop() else {
            return;
        };
        if self.tables.is_empty() {
            let markdown = markdown_table(&table.rows);
            if !markdown.is_empty() {
                self.blocks
                    .push(Block::Table(markdown.trim_end().to_owned()));
            }
        } else {
            // Markdown cannot nest a table: flatten it into the parent cell, one line per row.
            let flat: Vec<String> = table
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .filter(|c| !c.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .filter(|line| !line.is_empty())
                .collect();
            if let Some(parent) = self.tables.last_mut() {
                if !flat.is_empty() {
                    parent.cell.push(flat.join("<br>"));
                }
            }
        }
    }

    fn finish_paragraph(&mut self) {
        let Some(para) = self.paras.pop() else { return };
        let in_cell = !self.tables.is_empty();
        let style = para.style.clone().unwrap_or_default();
        let heading = match para.outline {
            Some(level) => (level < 9).then(|| (level as usize + 1).min(6)),
            None => self.styles.heading_level(&style),
        };
        if let Some(level) = heading {
            let line = render(&para.segs, " ", false);
            let line = line.trim();
            if line.is_empty() {
                return;
            }
            if let Some(t) = self.tables.last_mut() {
                t.cell.push(line.to_owned());
            } else {
                self.blocks
                    .push(Block::Heading(format!("{} {line}", "#".repeat(level))));
            }
            return;
        }
        let list = match &para.num {
            Some((id, level)) => Some((id.clone(), level.unwrap_or(0))),
            None => self.styles.list(&style),
        }
        .filter(|(id, _)| id != "0" && !id.is_empty());
        let line = render(&para.segs, if in_cell { "<br>" } else { "  \n" }, true);
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let prefix = list.and_then(|(id, level)| {
            let bullet = self.numbering.is_bullet(&id, level)?;
            let marker = if bullet {
                "- ".to_owned()
            } else {
                self.counters.retain(|(n, l), _| !(*n == id && *l > level));
                let count = self.counters.entry((id.clone(), level)).or_insert(0);
                *count += 1;
                format!("{count}. ")
            };
            Some(if in_cell {
                marker
            } else {
                format!("{}{marker}", "    ".repeat(level.min(8) as usize))
            })
        });
        if let Some(t) = self.tables.last_mut() {
            t.cell.push(format!("{}{line}", prefix.unwrap_or_default()));
        } else if let Some(prefix) = prefix {
            self.blocks.push(Block::Item(format!(
                "{prefix}{}",
                line.replace("  \n", " ")
            )));
        } else {
            self.blocks.push(Block::Para(line.to_owned()));
        }
    }

    fn run_xml(&mut self, xml: &[u8]) -> Result<(), String> {
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) => self.start(&tag),
                Event::Empty(tag) => {
                    let name = tag.local_name().as_ref().to_owned();
                    self.start(&tag);
                    self.end(&name);
                }
                Event::End(tag) => {
                    let name = tag.local_name().as_ref().to_owned();
                    self.end(&name);
                }
                Event::Text(value) if self.skip == 0 => {
                    if self.in_instr {
                        if let Some(f) = self.fields.last_mut() {
                            f.instr.push_str(&text(value.as_ref()));
                        }
                    } else if self.run.in_t {
                        self.add_text(&text(value.as_ref()));
                    }
                }
                Event::GeneralRef(r) if self.skip == 0 && (self.run.in_t || self.in_instr) => {
                    let mut s = String::new();
                    push_ref(&mut s, &r);
                    if self.in_instr {
                        if let Some(f) = self.fields.last_mut() {
                            f.instr.push_str(&s);
                        }
                    } else {
                        self.add_text(&s);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }
}

fn val_of(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    attr_local(tag, name)
}

fn is_off(value: Option<&str>) -> bool {
    matches!(value, Some("0" | "false" | "off" | "none"))
}

/// The URL of a `HYPERLINK "url"` field instruction, if that is what it is.
fn hyperlink_target(instr: &str) -> Option<String> {
    let rest = instr.trim().strip_prefix("HYPERLINK")?.trim();
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted
            .split('"')
            .next()
            .map(str::to_owned)
            .filter(|u| !u.is_empty());
    }
    rest.split_whitespace()
        .next()
        .filter(|u| !u.starts_with('\\'))
        .map(str::to_owned)
}

fn find_part(
    rels: &[Rel],
    kind: &str,
    base: &str,
    default: &str,
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
) -> Option<String> {
    rels.iter()
        .find(|r| r.kind == kind && !r.external)
        .and_then(|r| resolve(base, &r.target))
        .or_else(|| zip.by_name(default).is_ok().then(|| default.to_owned()))
}

fn links_of(rels: &[Rel]) -> HashMap<String, String> {
    rels.iter()
        .filter(|r| r.kind == "hyperlink" && r.external)
        .map(|r| (r.id.clone(), r.target.clone()))
        .collect()
}

fn join_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut previous_item = false;
    for block in blocks {
        let (body, item) = match block {
            Block::Heading(t) | Block::Para(t) | Block::Table(t) => (t.as_str(), false),
            Block::Item(t) => (t.as_str(), true),
        };
        if !out.is_empty() {
            out.push_str(if item && previous_item { "\n" } else { "\n\n" });
        }
        out.push_str(body);
        previous_item = item;
    }
    out
}

pub(crate) fn convert(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let document = main_part(&mut zip, "word/document.xml")?;
    let rels = read_rels(&mut zip, &rels_path_of(&document))?;

    let styles = match find_part(&rels, "styles", &document, "word/styles.xml", &mut zip) {
        Some(part) => Styles::parse(&entry(&mut zip, &part)?)?,
        None => Styles::default(),
    };
    let numbering = match find_part(
        &rels,
        "numbering",
        &document,
        "word/numbering.xml",
        &mut zip,
    ) {
        Some(part) => Numbering::parse(&entry(&mut zip, &part)?)?,
        None => Numbering::default(),
    };

    let xml = entry(&mut zip, &document)?;
    let mut main = Parser::new(&styles, &numbering, links_of(&rels));
    main.run_xml(&xml)?;
    let mut output = join_blocks(&main.blocks);

    // Notes that the body refers to, in order of first reference.
    let mut notes: HashMap<String, String> = HashMap::new();
    for (kind, default) in [
        ("footnotes", "word/footnotes.xml"),
        ("endnotes", "word/endnotes.xml"),
    ] {
        let Some(part) = find_part(&rels, kind, &document, default, &mut zip) else {
            continue;
        };
        let note_rels = read_rels(&mut zip, &rels_path_of(&part))?;
        let xml = entry(&mut zip, &part)?;
        let mut parser = Parser::new(&styles, &numbering, links_of(&note_rels));
        parser.run_xml(&xml)?;
        for (id, blocks) in parser.notes {
            let body = join_blocks(&blocks)
                .split("\n\n")
                .collect::<Vec<_>>()
                .join(" ");
            notes.insert(id, body);
        }
    }
    let mut definitions = Vec::new();
    for id in &main.references {
        if let Some(body) = notes.get(id).filter(|b| !b.trim().is_empty()) {
            definitions.push(format!("[^{id}]: {}", body.trim()));
        }
    }
    if !definitions.is_empty() {
        output.push_str("\n\n");
        output.push_str(&definitions.join("\n"));
    }

    for index in 0..zip.len() {
        let name = zip
            .by_index_raw(index)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if name.starts_with("word/charts/") && name.ends_with(".xml") {
            let chart = entry(&mut zip, &name)?;
            let title = xml_text(&chart, b"a:t")?;
            let values = xml_text(&chart, b"c:v")?;
            if !title.trim().is_empty() || !values.trim().is_empty() {
                output.push_str(&format!(
                    "\n\n**Chart: {}**\n\n{}",
                    title.trim(),
                    values.trim()
                ));
            }
        } else if name.starts_with("word/diagrams/") && name.ends_with(".xml") {
            let diagram = entry(&mut zip, &name)?;
            let values = xml_text(&diagram, b"a:t")?;
            if !values.trim().is_empty() {
                output.push_str(&format!("\n\nDiagram: {}", values.trim()));
            }
        }
    }
    Ok(output.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in parts {
                writer.start_file(*name, options).expect("start");
                writer.write_all(body.as_bytes()).expect("write");
            }
            writer.finish().expect("finish");
        }
        cursor.into_inner()
    }

    const NS: &str = r#"xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    fn docx_with(body: &str) -> String {
        let document = format!(r#"<x:document {NS}><x:body>{body}</x:body></x:document>"#);
        convert(&package(&[("word/document.xml", &document)])).expect("convert")
    }

    #[test]
    fn any_namespace_prefix_reads() {
        assert_eq!(
            docx_with(r#"<x:p><x:r><x:t>plain</x:t></x:r></x:p>"#),
            "plain"
        );
    }

    #[test]
    fn direct_outline_level_makes_a_heading_and_nine_means_body() {
        let md = docx_with(
            r#"<x:p><x:pPr><x:outlineLvl x:val="2"/></x:pPr><x:r><x:t>Third</x:t></x:r></x:p>
               <x:p><x:pPr><x:outlineLvl x:val="9"/></x:pPr><x:r><x:t>Body</x:t></x:r></x:p>"#,
        );
        assert_eq!(md, "### Third\n\nBody");
    }

    #[test]
    fn tracked_deletions_are_omitted_and_insertions_kept() {
        let md = docx_with(
            r#"<x:p><x:r><x:t>keep </x:t></x:r><x:del><x:r><x:delText>gone</x:delText></x:r></x:del><x:ins><x:r><x:t>added</x:t></x:r></x:ins></x:p>"#,
        );
        assert_eq!(md, "keep added");
    }

    #[test]
    fn alternate_content_text_is_read_once() {
        let md = docx_with(
            r#"<x:p><x:r><mc:AlternateContent><mc:Choice Requires="wps"><x:txbxContent><x:p><x:r><x:t>boxed</x:t></x:r></x:p></x:txbxContent></mc:Choice><mc:Fallback><x:txbxContent><x:p><x:r><x:t>boxed</x:t></x:r></x:p></x:txbxContent></mc:Fallback></mc:AlternateContent></x:r><x:r><x:t>after</x:t></x:r></x:p>"#,
        );
        assert_eq!(md.matches("boxed").count(), 1, "{md}");
        assert!(md.contains("after"), "{md}");
    }

    #[test]
    fn nested_table_stays_inside_its_cell_and_the_outer_table_survives() {
        let md = docx_with(
            r#"<x:tbl><x:tr><x:tc><x:p><x:r><x:t>outer</x:t></x:r></x:p>
                 <x:tbl><x:tr><x:tc><x:p><x:r><x:t>in1</x:t></x:r></x:p></x:tc><x:tc><x:p><x:r><x:t>in2</x:t></x:r></x:p></x:tc></x:tr></x:tbl>
                 <x:p/></x:tc><x:tc><x:p><x:r><x:t>side</x:t></x:r></x:p></x:tc></x:tr>
               <x:tr><x:tc><x:p><x:r><x:t>r2</x:t></x:r></x:p></x:tc><x:tc><x:p><x:r><x:t>r2b</x:t></x:r></x:p></x:tc></x:tr></x:tbl>
               <x:p><x:r><x:t>after</x:t></x:r></x:p>"#,
        );
        assert!(md.contains("| outer<br>in1 · in2 | side |"), "{md}");
        assert!(md.contains("| r2 | r2b |"), "{md}");
        assert!(md.ends_with("after"), "{md}");
    }

    #[test]
    fn grid_span_and_vertical_merge_keep_column_positions() {
        let md = docx_with(
            r#"<x:tbl>
              <x:tr><x:tc><x:tcPr><x:gridSpan x:val="2"/></x:tcPr><x:p><x:r><x:t>wide</x:t></x:r></x:p></x:tc><x:tc><x:p><x:r><x:t>c</x:t></x:r></x:p></x:tc></x:tr>
              <x:tr><x:tc><x:tcPr><x:vMerge x:val="restart"/></x:tcPr><x:p><x:r><x:t>tall</x:t></x:r></x:p></x:tc><x:tc><x:p><x:r><x:t>b</x:t></x:r></x:p></x:tc><x:tc><x:p><x:r><x:t>c</x:t></x:r></x:p></x:tc></x:tr>
              <x:tr><x:tc><x:tcPr><x:vMerge/></x:tcPr><x:p/></x:tc><x:tc><x:p><x:r><x:t>b2</x:t></x:r></x:p></x:tc><x:tc><x:p><x:r><x:t>c2</x:t></x:r></x:p></x:tc></x:tr>
            </x:tbl>"#,
        );
        assert!(md.contains("| wide |  | c |"), "{md}");
        assert!(md.contains("| tall | b | c |"), "{md}");
        assert!(md.contains("|  | b2 | c2 |"), "{md}");
    }

    #[test]
    fn hyperlink_field_instruction_becomes_a_link() {
        let md = docx_with(
            r#"<x:p><x:r><x:fldChar x:fldCharType="begin"/></x:r><x:r><x:instrText> HYPERLINK "https://example.org/p?a=1&amp;b=2" </x:instrText></x:r><x:r><x:fldChar x:fldCharType="separate"/></x:r><x:r><x:t>linked</x:t></x:r><x:r><x:fldChar x:fldCharType="end"/></x:r><x:r><x:t> text</x:t></x:r></x:p>"#,
        );
        assert_eq!(md, "[linked](https://example.org/p?a=1&b=2) text");
    }

    #[test]
    fn explicit_false_turns_bold_off() {
        let md = docx_with(
            r#"<x:p><x:r><x:rPr><x:b x:val="0"/></x:rPr><x:t>not bold</x:t></x:r></x:p>"#,
        );
        assert_eq!(md, "not bold");
    }

    #[test]
    fn title_and_unknown_style_without_styles_part() {
        let md = docx_with(
            r#"<x:p><x:pPr><x:pStyle x:val="Title"/></x:pPr><x:r><x:t>T</x:t></x:r></x:p><x:p><x:pPr><x:pStyle x:val="Heading3"/></x:pPr><x:r><x:t>H</x:t></x:r></x:p>"#,
        );
        assert_eq!(md, "# T\n\n### H");
    }
}
