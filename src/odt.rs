//! OpenDocument text (`.odt`) as a document: headings with their outline level, paragraphs,
//! nested bullet and numbered lists (kind taken from the list style), tables with header rows,
//! spans and alignment, block quotes and preformatted text (from the paragraph style), links,
//! footnotes and endnotes, images, emphasis, strikeout, underline, super and subscripts, inline
//! code (from a monospace font or the `Source Text` style) and math objects, mapped to the
//! tree in `ast` as Pandoc's ODT reader maps them.
//!
//! Style names are resolved through `style:parent-style-name`, so a paragraph style derived from
//! `Quotations` is a quotation. A style that is not defined contributes nothing.

use std::collections::HashMap;

use crate::ast::{Align, Block, Cell, Doc, Inline, ListStyle, Meta, Table};
use crate::document::{archive, entry};
use crate::xmltree::{self, XElem, XNode};

/// Reads an `.odt` file into a document tree.
pub fn to_doc(data: &[u8]) -> Result<Doc, String> {
    let mut zip = archive(data)?;
    let content = xmltree::parse(&entry(&mut zip, "content.xml")?)?;
    let mut styles = Styles::default();
    if let Ok(bytes) = entry(&mut zip, "styles.xml") {
        if let Ok(root) = xmltree::parse(&bytes) {
            styles.collect(&root);
        }
    }
    styles.collect(&content);
    let mut r = Reader { styles, notes: 0 };
    let mut blocks = Vec::new();
    match content
        .find("office:body")
        .and_then(|body| body.find("office:text"))
    {
        Some(text) => r.body(&text.children, &mut blocks),
        // Hand-made packages without the body wrapper: read the root's children.
        None => r.body(&content.children, &mut blocks),
    }
    Ok(Doc {
        meta: Meta::default(),
        blocks: merge_runs(blocks),
    })
}

/// Reads an `.odt` file and writes Pandoc-flavoured Markdown.
pub fn to_markdown(data: &[u8]) -> Result<String, String> {
    Ok(crate::ast::to_markdown(&to_doc(data)?))
}

#[derive(Debug, Clone, Default)]
struct StyleInfo {
    parent: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    sup: bool,
    sub: bool,
    mono: bool,
    align: Align,
    display_name: String,
}

#[derive(Default)]
struct Styles {
    by_name: HashMap<String, StyleInfo>,
    /// list style name -> per level: Some(start, style) for numbered, None for bullets.
    lists: HashMap<String, Vec<Option<(u32, ListStyle)>>>,
}

impl Styles {
    fn collect(&mut self, root: &XElem) {
        for section in root.elems() {
            if section.name != "office:automatic-styles" && section.name != "office:styles" {
                continue;
            }
            for st in section.elems() {
                match st.name.as_str() {
                    "style:style" => {
                        let Some(name) = st.attr("style:name") else {
                            continue;
                        };
                        let mut info = StyleInfo {
                            parent: st.attr("style:parent-style-name").map(str::to_string),
                            display_name: st.attr("style:display-name").unwrap_or(name).to_string(),
                            ..Default::default()
                        };
                        if let Some(tp) = st.find("style:text-properties") {
                            info.bold = tp.attr("fo:font-weight").is_some_and(|w| {
                                w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600)
                            });
                            info.italic = tp
                                .attr("fo:font-style")
                                .is_some_and(|w| w == "italic" || w == "oblique");
                            info.underline = tp
                                .attr("style:text-underline-style")
                                .is_some_and(|w| w != "none");
                            info.strike = tp
                                .attr("style:text-line-through-style")
                                .is_some_and(|w| w != "none");
                            if let Some(pos) = tp.attr("style:text-position") {
                                let first = pos.split_whitespace().next().unwrap_or("");
                                if first == "super"
                                    || first
                                        .trim_end_matches('%')
                                        .parse::<f64>()
                                        .is_ok_and(|p| p > 0.0)
                                {
                                    info.sup = true;
                                } else if first == "sub"
                                    || first
                                        .trim_end_matches('%')
                                        .parse::<f64>()
                                        .is_ok_and(|p| p < 0.0)
                                {
                                    info.sub = true;
                                }
                            }
                            let font = tp
                                .attr("style:font-name")
                                .or_else(|| tp.attr("fo:font-family"))
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            info.mono = ["courier", "mono", "consolas", "menlo", "monaco"]
                                .iter()
                                .any(|m| font.contains(m));
                        }
                        if let Some(pp) = st.find("style:paragraph-properties") {
                            info.align = match pp.attr("fo:text-align") {
                                Some("left") | Some("start") => Align::Left,
                                Some("right") | Some("end") => Align::Right,
                                Some("center") => Align::Center,
                                _ => Align::Default,
                            };
                        }
                        self.by_name.insert(name.to_string(), info);
                    }
                    "text:list-style" => {
                        let Some(name) = st.attr("style:name") else {
                            continue;
                        };
                        let mut levels: Vec<Option<(u32, ListStyle)>> = Vec::new();
                        for lv in st.elems() {
                            let level = lv
                                .attr("text:level")
                                .and_then(|l| l.parse::<usize>().ok())
                                .unwrap_or(levels.len() + 1);
                            let entry = match lv.name.as_str() {
                                "text:list-level-style-number" => {
                                    let start = lv
                                        .attr("text:start-value")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(1);
                                    let fmt = match lv.attr("style:num-format") {
                                        Some("a") => ListStyle::LowerAlpha,
                                        Some("A") => ListStyle::UpperAlpha,
                                        Some("i") => ListStyle::LowerRoman,
                                        Some("I") => ListStyle::UpperRoman,
                                        _ => ListStyle::Decimal,
                                    };
                                    Some((start, fmt))
                                }
                                _ => None,
                            };
                            if levels.len() < level {
                                levels.resize(level, None);
                            }
                            levels[level - 1] = entry;
                        }
                        self.lists.insert(name.to_string(), levels);
                    }
                    _ => {}
                }
            }
        }
    }

    /// The chain of style names from this one to its root (this one first).
    fn chain(&self, name: &str) -> Vec<&StyleInfo> {
        let mut out = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            match self.by_name.get(&n) {
                Some(s) if out.len() < 16 => {
                    cur = s.parent.clone();
                    out.push(s);
                }
                _ => break,
            }
        }
        out
    }

    fn names(&self, name: &str) -> Vec<String> {
        let mut v: Vec<String> = vec![norm_name(name)];
        for s in self.chain(name) {
            v.push(norm_name(&s.display_name));
            if let Some(p) = &s.parent {
                v.push(norm_name(p));
            }
        }
        v
    }

    fn is(&self, style: &str, wanted: &[&str]) -> bool {
        self.names(style)
            .iter()
            .any(|n| wanted.contains(&n.as_str()))
    }

    fn any(&self, style: &str, f: impl Fn(&StyleInfo) -> bool) -> bool {
        self.chain(style).iter().any(|s| f(s))
    }
}

/// `Source_20_Text` and `Source Text` are the same name.
fn norm_name(n: &str) -> String {
    n.replace("_20_", " ")
        .replace('_', " ")
        .to_ascii_lowercase()
}

struct Reader {
    styles: Styles,
    notes: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Normal,
    Quote,
    Code,
    Rule,
}

impl Reader {
    fn body(&mut self, nodes: &[XNode], out: &mut Vec<Block>) {
        for n in nodes {
            let XNode::Elem(e) = n else { continue };
            match e.name.as_str() {
                "text:h" => {
                    let level = e
                        .attr("text:outline-level")
                        .and_then(|l| l.parse().ok())
                        .unwrap_or(1u8);
                    let inl = self.inlines(&e.children, None);
                    if !inl.is_empty() {
                        out.push(Block::Heading {
                            level: level.clamp(1, 6),
                            inlines: inl,
                            id: None,
                        });
                    }
                }
                "text:p" => self.paragraph(e, false, out),
                "text:list" => {
                    let b = self.list(e, 0, None);
                    out.extend(b);
                }
                "table:table" => {
                    if let Some(t) = self.table(e) {
                        out.push(Block::Table(t));
                    }
                }
                "text:section"
                | "text:index-body"
                | "office:text"
                | "text:table-of-content-source" => {
                    if e.name != "text:table-of-content-source" {
                        self.body(&e.children, out);
                    }
                }
                "text:table-of-content"
                | "text:alphabetical-index"
                | "text:illustration-index"
                | "text:table-index" => {
                    // The generated index: its entries repeat the headings.
                }
                "text:soft-page-break"
                | "text:tracked-changes"
                | "text:sequence-decls"
                | "text:variable-decls" => {}
                _ => {}
            }
        }
    }

    fn kind(&self, e: &XElem) -> Kind {
        let Some(style) = e.attr("text:style-name") else {
            return Kind::Normal;
        };
        if self.styles.is(
            style,
            &["quotations", "quotation", "block quotation", "block quote"],
        ) {
            Kind::Quote
        } else if self
            .styles
            .is(style, &["preformatted text", "source code", "code"])
        {
            Kind::Code
        } else if self.styles.is(style, &["horizontal line"]) {
            Kind::Rule
        } else {
            Kind::Normal
        }
    }

    fn paragraph(&mut self, e: &XElem, plain: bool, out: &mut Vec<Block>) {
        let kind = self.kind(e);
        if kind == Kind::Rule {
            out.push(Block::Rule);
            return;
        }
        if kind == Kind::Code {
            let mut text = String::new();
            raw_text(&e.children, &mut text);
            out.push(Block::Code { lang: None, text });
            return;
        }
        // A paragraph that holds only a math object is display math.
        if let Some(tex) = sole_math(e) {
            out.push(Block::Para(vec![Inline::Math { display: true, tex }]));
            return;
        }
        let inl = self.inlines(&e.children, None);
        if inl.is_empty() {
            return;
        }
        let block = if plain {
            Block::Plain(inl)
        } else {
            Block::Para(inl)
        };
        if kind == Kind::Quote {
            out.push(Block::Quote(vec![block]));
        } else {
            out.push(block);
        }
    }

    fn list(&mut self, e: &XElem, depth: usize, inherited: Option<&str>) -> Vec<Block> {
        let style = e.attr("text:style-name").or(inherited);
        let level = style
            .and_then(|s| self.styles.lists.get(s))
            .and_then(|l| l.get(depth))
            .cloned()
            .flatten();
        let mut items = Vec::new();
        for it in e.elems() {
            if it.name != "text:list-item" && it.name != "text:list-header" {
                continue;
            }
            let mut blocks = Vec::new();
            for c in it.elems() {
                match c.name.as_str() {
                    "text:p" => self.paragraph(c, true, &mut blocks),
                    "text:h" => {
                        let inl = self.inlines(&c.children, None);
                        if !inl.is_empty() {
                            blocks.push(Block::Plain(inl));
                        }
                    }
                    "text:list" => blocks.extend(self.list(c, depth + 1, style)),
                    "table:table" => {
                        if let Some(t) = self.table(c) {
                            blocks.push(Block::Table(t));
                        }
                    }
                    _ => {}
                }
            }
            items.push(blocks);
        }
        if items.is_empty() {
            return Vec::new();
        }
        match level {
            Some((start, style)) => vec![Block::OrderedList {
                start,
                style,
                items,
            }],
            None => vec![Block::BulletList(items)],
        }
    }

    fn table(&mut self, e: &XElem) -> Option<Table> {
        let mut t = Table::default();
        let mut align: Vec<Align> = Vec::new();
        let mut first_body = true;
        self.table_rows(&e.children, &mut t, &mut align, false, &mut first_body);
        t.align = align;
        if t.head.is_empty() && t.rows.is_empty() {
            None
        } else {
            Some(t)
        }
    }

    fn table_rows(
        &mut self,
        nodes: &[XNode],
        t: &mut Table,
        align: &mut Vec<Align>,
        header: bool,
        first_body: &mut bool,
    ) {
        for n in nodes {
            let XNode::Elem(e) = n else { continue };
            match e.name.as_str() {
                "table:table-header-rows" => {
                    self.table_rows(&e.children, t, align, true, first_body)
                }
                "table:table-row-group" | "table:table-rows" => {
                    self.table_rows(&e.children, t, align, header, first_body)
                }
                "table:table-row" => {
                    let repeat = e
                        .attr("table:number-rows-repeated")
                        .and_then(|r| r.parse::<usize>().ok())
                        .unwrap_or(1)
                        .clamp(1, 50);
                    let mut cells = Vec::new();
                    for c in e.elems() {
                        if c.name == "table:covered-table-cell" {
                            continue;
                        }
                        if c.name != "table:table-cell" {
                            continue;
                        }
                        let colspan = c
                            .attr("table:number-columns-spanned")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1usize)
                            .clamp(1, 1000);
                        let rowspan = c
                            .attr("table:number-rows-spanned")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1usize)
                            .clamp(1, 1000);
                        let crepeat = c
                            .attr("table:number-columns-repeated")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(1)
                            .clamp(1, 50);
                        let mut blocks = Vec::new();
                        let mut cell_style = None;
                        for p in c.elems() {
                            match p.name.as_str() {
                                "text:p" => {
                                    if cell_style.is_none() {
                                        cell_style = p.attr("text:style-name").map(str::to_string);
                                    }
                                    self.paragraph(p, true, &mut blocks);
                                }
                                "text:h" => {
                                    let inl = self.inlines(&p.children, None);
                                    if !inl.is_empty() {
                                        blocks.push(Block::Plain(inl));
                                    }
                                }
                                "text:list" => blocks.extend(self.list(p, 0, None)),
                                _ => {}
                            }
                        }
                        let a = cell_style
                            .as_deref()
                            .and_then(|s| {
                                self.styles
                                    .chain(s)
                                    .into_iter()
                                    .map(|i| i.align)
                                    .find(|a| *a != Align::Default)
                            })
                            .unwrap_or_default();
                        for _ in 0..crepeat {
                            let col = cells.len();
                            if !header
                                && a != Align::Default
                                && align.get(col).copied().unwrap_or_default() == Align::Default
                            {
                                if align.len() <= col {
                                    align.resize(col + 1, Align::Default);
                                }
                                align[col] = a;
                            }
                            cells.push(Cell {
                                blocks: blocks.clone(),
                                colspan,
                                rowspan,
                            });
                        }
                    }
                    // A trailing run of repeated empty rows is a spreadsheet artefact: skip it.
                    let empty = cells.iter().all(|c| c.blocks.is_empty());
                    if empty && repeat > 1 {
                        continue;
                    }
                    for _ in 0..repeat {
                        if header {
                            t.head.push(cells.clone());
                        } else {
                            t.rows.push(cells.clone());
                        }
                    }
                    if !header {
                        *first_body = false;
                    }
                }
                _ => {}
            }
        }
    }

    fn inlines(&mut self, nodes: &[XNode], style: Option<&str>) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        for n in nodes {
            match n {
                XNode::Text(t) => {
                    let s = collapse(t);
                    if !s.is_empty() {
                        out.push(Inline::Text(s));
                    }
                }
                XNode::Elem(e) => self.inline_elem(e, style, &mut out),
            }
        }
        normalise(&mut out);
        out
    }

    fn inline_elem(&mut self, e: &XElem, _style: Option<&str>, out: &mut Vec<Inline>) {
        match e.name.as_str() {
            "text:span" => {
                let inner = self.inlines(&e.children, None);
                if inner.is_empty() {
                    return;
                }
                let style = e.attr("text:style-name");
                let wrapped = match style {
                    Some(s) => self.wrap_style(s, inner),
                    None => inner,
                };
                out.extend(wrapped);
            }
            "text:a" => {
                let inner = self.inlines(&e.children, None);
                let href = e.attr("xlink:href").unwrap_or("");
                if href.is_empty() || inner.is_empty() {
                    out.extend(inner);
                } else {
                    out.push(Inline::Link {
                        url: href.to_string(),
                        title: None,
                        inlines: inner,
                    });
                }
            }
            "text:line-break" => out.push(Inline::LineBreak),
            "text:tab" => out.push(Inline::Text(" ".into())),
            "text:s" => {
                let n = e
                    .attr("text:c")
                    .and_then(|c| c.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 80);
                out.push(Inline::Text(" ".repeat(n)));
            }
            "text:note" => {
                self.notes += 1;
                let mut blocks = Vec::new();
                if let Some(body) = e.find("text:note-body") {
                    for c in body.elems() {
                        match c.name.as_str() {
                            "text:p" => self.paragraph(c, false, &mut blocks),
                            "text:list" => blocks.extend(self.list(c, 0, None)),
                            _ => {}
                        }
                    }
                }
                out.push(Inline::Note(blocks));
            }
            "draw:frame" => {
                if let Some(img) = e.descendant("draw:image") {
                    let alt = e
                        .find("svg:title")
                        .map(|t| {
                            t.children
                                .iter()
                                .filter_map(|c| match c {
                                    XNode::Text(s) => Some(s.as_str()),
                                    _ => None,
                                })
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    let alt = if alt.is_empty() {
                        Vec::new()
                    } else {
                        vec![Inline::Text(alt)]
                    };
                    out.push(Inline::Image {
                        url: img.attr("xlink:href").unwrap_or("").to_string(),
                        alt,
                        title: None,
                    });
                } else if let Some(tex) = math_in(e) {
                    out.push(Inline::Math {
                        display: false,
                        tex,
                    });
                }
            }
            "text:bookmark"
            | "text:bookmark-start"
            | "text:bookmark-end"
            | "text:soft-page-break"
            | "text:reference-mark"
            | "text:reference-mark-start"
            | "text:reference-mark-end"
            | "text:change"
            | "text:change-start"
            | "text:change-end" => {}
            "office:annotation" | "office:annotation-end" => {}
            "text:note-citation" => {}
            _ => {
                let inner = self.inlines(&e.children, None);
                out.extend(inner);
            }
        }
    }

    /// Wraps inlines in what a character style says: bold, italic, underline, strikeout,
    /// super and subscript, and code.
    fn wrap_style(&self, style: &str, inner: Vec<Inline>) -> Vec<Inline> {
        let st = &self.styles;
        let mono = st.any(style, |s| s.mono)
            || st.is(style, &["source text", "source code", "preformatted text"]);
        let mut v = inner;
        if mono {
            let text = crate::ast::inlines_text(&v);
            return vec![Inline::Code(text)];
        }
        if st.any(style, |s| s.sup) {
            v = vec![Inline::Sup(v)];
        }
        if st.any(style, |s| s.sub) {
            v = vec![Inline::Sub(v)];
        }
        if st.any(style, |s| s.strike) {
            v = vec![Inline::Strike(v)];
        }
        if st.any(style, |s| s.underline)
            && !st.is(style, &["internet link", "visited internet link"])
        {
            v = vec![Inline::Underline(v)];
        }
        if st.any(style, |s| s.italic) || st.is(style, &["emphasis"]) {
            v = vec![Inline::Emph(v)];
        }
        if st.any(style, |s| s.bold) || st.is(style, &["strong emphasis"]) {
            v = vec![Inline::Strong(v)];
        }
        v
    }
}

fn math_in(e: &XElem) -> Option<String> {
    let m = e.descendant("math:math").or_else(|| e.descendant("math"))?;
    let tex = crate::mathml::to_tex(&m.to_markup()).tex;
    let tex = tex.trim();
    if tex.is_empty() {
        None
    } else {
        Some(tex.to_string())
    }
}

fn sole_math(p: &XElem) -> Option<String> {
    let mut found = None;
    for n in &p.children {
        match n {
            XNode::Text(t) if t.trim().is_empty() => {}
            XNode::Elem(e) if e.name == "draw:frame" && found.is_none() => {
                found = Some(math_in(e)?)
            }
            _ => return None,
        }
    }
    found
}

fn raw_text(nodes: &[XNode], out: &mut String) {
    for n in nodes {
        match n {
            XNode::Text(t) => out.push_str(t),
            XNode::Elem(e) => match e.name.as_str() {
                "text:line-break" => out.push('\n'),
                "text:tab" => out.push('\t'),
                "text:s" => out.push_str(
                    &" ".repeat(
                        e.attr("text:c")
                            .and_then(|c| c.parse::<usize>().ok())
                            .unwrap_or(1)
                            .clamp(1, 200),
                    ),
                ),
                _ => raw_text(&e.children, out),
            },
        }
    }
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\n' | '\t' | '\r') {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// Merges adjacent text, collapses doubled spaces across run boundaries and trims the ends.
fn normalise(v: &mut Vec<Inline>) {
    let mut out: Vec<Inline> = Vec::with_capacity(v.len());
    for i in v.drain(..) {
        match (out.last_mut(), i) {
            (Some(Inline::Text(a)), Inline::Text(b)) => {
                if a.ends_with(' ') && b.starts_with(' ') {
                    a.push_str(&b[1..]);
                } else {
                    a.push_str(&b);
                }
            }
            (_, i) => out.push(i),
        }
    }
    if let Some(Inline::Text(t)) = out.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(Inline::Text(t)) = out.last_mut() {
        *t = t.trim_end().to_string();
    }
    out.retain(|i| !matches!(i, Inline::Text(t) if t.is_empty()));
    while matches!(out.last(), Some(Inline::LineBreak)) {
        out.pop();
    }
    *v = out;
}

/// Adjacent quotations become one block quote and adjacent preformatted lines one code block.
fn merge_runs(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for b in blocks {
        match (out.last_mut(), b) {
            (Some(Block::Quote(a)), Block::Quote(mut b)) => a.append(&mut b),
            (
                Some(Block::Code {
                    lang: None,
                    text: a,
                }),
                Block::Code {
                    lang: None,
                    text: b,
                },
            ) => {
                a.push('\n');
                a.push_str(&b);
            }
            (_, b) => out.push(b),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_names_are_normalised() {
        assert_eq!(norm_name("Source_20_Text"), "source text");
        assert_eq!(norm_name("Quotations"), "quotations");
    }

    #[test]
    fn adjacent_runs_merge() {
        let blocks = merge_runs(vec![
            Block::Quote(vec![Block::Para(vec![Inline::Text("a".into())])]),
            Block::Quote(vec![Block::Para(vec![Inline::Text("b".into())])]),
            Block::Code {
                lang: None,
                text: "x".into(),
            },
            Block::Code {
                lang: None,
                text: "y".into(),
            },
        ]);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[1], Block::Code { text, .. } if text == "x\ny"));
    }
}
