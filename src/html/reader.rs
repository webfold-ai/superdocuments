//! HTML as a document: every element is kept and mapped to the tree in `ast`, the way Pandoc's
//! HTML reader does. Headings, paragraphs, nested lists, definition lists, tables (with header
//! rows, alignment and spans), code blocks with a language, block quotes, rules, figures,
//! links, images, emphasis, strikeout, super and subscripts, inline and display math (TeX
//! delimiters and MathML) and footnotes (`doc-noteref`/`footnote-ref` links, EPUB `footnote`
//! asides, LibreOffice `sdfootnote` anchors) are read. `<title>` and the author and date `meta`
//! tags become the document's metadata.
//!
//! This is not the web-page extractor (`extract`), which drops navigation, headers, footers and
//! boilerplate to find the article. A document is read in full.

use std::collections::HashMap;

use super::dom::{self, Elem, Node};
use crate::ast::{Align, Block, Cell, Doc, Inline, ListStyle, Meta, Table};

/// Reads HTML into a document tree. Footnote links stay links, as they do in Pandoc's HTML
/// reader; `to_doc_epub` turns them into footnotes.
pub fn to_doc(html: &str) -> Doc {
    read(html, false)
}

/// Reads a chapter of an EPUB: footnote references (`doc-noteref`, `epub:type="noteref"`,
/// `footnote-ref`) are replaced by the text of the note they point to.
pub fn to_doc_epub(html: &str) -> Doc {
    read(html, true)
}

fn read(html: &str, epub: bool) -> Doc {
    let mut nodes = dom::parse(html);
    let mut meta = Meta::default();
    let mut r = Reader::default();
    r.collect_meta(&nodes, &mut meta);
    if epub {
        // Footnote bodies are taken out of the tree first so they are not also read in place.
        r.take_notes(&mut nodes);
    }
    let body = find_body(&nodes).cloned().map(|b| b.children).unwrap_or(nodes);
    let blocks = r.blocks(&body, false);
    Doc { meta, blocks }
}

/// Reads HTML bytes and writes Pandoc-flavoured Markdown.
pub fn to_markdown_document(html: &[u8]) -> String {
    crate::ast::to_markdown(&to_doc(&String::from_utf8_lossy(html)))
}

fn find_body(nodes: &[Node]) -> Option<&Elem> {
    for n in nodes {
        if let Node::Elem(e) = n {
            if e.tag == "body" {
                return Some(e);
            }
            if let Some(b) = find_body(&e.children) {
                return Some(b);
            }
        }
    }
    None
}

#[derive(Default)]
struct Reader {
    notes: HashMap<String, Elem>,
    /// The inline run being built ends in a space (so the next one need not start with one).
    ends_space: bool,
    depth: usize,
}

fn is_skipped(tag: &str) -> bool {
    matches!(
        tag,
        "script" | "style" | "head" | "noscript" | "template" | "title" | "svg" | "canvas" | "select"
            | "button" | "textarea" | "iframe" | "object" | "embed" | "audio" | "video" | "meta"
            | "link" | "base" | "datalist" | "dialog" | "map" | "colgroup" | "col"
    )
}

fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "address" | "article" | "aside" | "blockquote" | "body" | "center" | "details" | "dir"
            | "div" | "dl" | "fieldset" | "figure" | "figcaption" | "footer" | "form" | "h1" | "h2"
            | "h3" | "h4" | "h5" | "h6" | "header" | "hgroup" | "hr" | "html" | "legend" | "main"
            | "menu" | "nav" | "ol" | "p" | "pre" | "section" | "summary" | "table" | "ul" | "li"
            | "dt" | "dd" | "tr" | "td" | "th" | "tbody" | "thead" | "tfoot" | "caption"
    )
}

impl Reader {
    // ---- metadata and footnote collection -------------------------------------------------

    fn collect_meta(&self, nodes: &[Node], meta: &mut Meta) {
        for n in nodes {
            let Node::Elem(e) = n else { continue };
            match e.tag.as_str() {
                "title" if meta.title.is_empty() => {
                    let t = collapse(&e.text());
                    if !t.trim().is_empty() {
                        meta.title = vec![Inline::Text(t.trim().to_string())];
                    }
                }
                "meta" => {
                    let name = e.attr("name").unwrap_or("").to_ascii_lowercase();
                    let content = e.attr("content").unwrap_or("").trim();
                    if content.is_empty() {
                        continue;
                    }
                    match name.as_str() {
                        "author" | "dc.creator" | "dcterms.creator" => {
                            meta.authors.push(vec![Inline::Text(content.to_string())]);
                        }
                        "date" | "dcterms.date" | "dc.date" if meta.date.is_none() => {
                            meta.date = Some(content.to_string());
                        }
                        _ => {}
                    }
                }
                _ => self.collect_meta(&e.children, meta),
            }
        }
    }

    /// Removes footnote bodies from the tree and keeps them by id.
    fn take_notes(&mut self, nodes: &mut Vec<Node>) {
        let mut i = 0;
        while i < nodes.len() {
            let mut remove = false;
            if let Node::Elem(e) = &mut nodes[i] {
                let ty = e.attr("epub:type").unwrap_or("").to_string();
                let role = e.attr("role").unwrap_or("").to_string();
                let is_container = matches!(role.as_str(), "doc-endnotes" | "doc-footnotes")
                    || ty.split_whitespace().any(|t| matches!(t, "footnotes" | "endnotes"))
                    || (e.tag == "section" && e.has_class("footnotes"))
                    || (e.tag == "div" && e.has_class("footnotes"));
                let is_note = matches!(role.as_str(), "doc-footnote" | "doc-endnote")
                    || ty.split_whitespace().any(|t| matches!(t, "footnote" | "endnote" | "rearnote"));
                let sd = e.attr("id").is_some_and(|id| id.starts_with("sdfootnote") && !id.ends_with("anc") && !id.ends_with("sym"));
                if is_container {
                    let mut found = false;
                    collect_note_items(&e.children, &mut self.notes, &mut found);
                    if found {
                        remove = true;
                    }
                } else if (is_note || sd) && e.attr("id").is_some() {
                    self.notes.insert(e.attr("id").unwrap_or("").to_string(), e.clone());
                    remove = true;
                } else {
                    self.take_notes(&mut e.children);
                }
            }
            if remove {
                nodes.remove(i);
            } else {
                i += 1;
            }
        }
    }

    // ---- blocks ----------------------------------------------------------------------------

    fn blocks(&mut self, nodes: &[Node], plain: bool) -> Vec<Block> {
        let mut out = Vec::new();
        let mut run: Vec<&Node> = Vec::new();
        for n in nodes {
            match n {
                Node::Text(_) => run.push(n),
                Node::Elem(e) => {
                    if is_skipped(&e.tag) {
                        continue;
                    }
                    if is_block_tag(&e.tag) || e.tag == "math" && e.attr("display") == Some("block") || has_block_child(e) {
                        self.flush(&mut run, plain, &mut out);
                        self.block_elem(e, plain, &mut out);
                    } else {
                        run.push(n);
                    }
                }
            }
        }
        self.flush(&mut run, plain, &mut out);
        out
    }

    fn flush(&mut self, run: &mut Vec<&Node>, plain: bool, out: &mut Vec<Block>) {
        if run.is_empty() {
            return;
        }
        let inl = self.inline_run(run.iter().copied());
        run.clear();
        if !inl.is_empty() {
            out.push(if plain { Block::Plain(inl) } else { Block::Para(inl) });
        }
    }

    fn inline_run<'a>(&mut self, nodes: impl Iterator<Item = &'a Node>) -> Vec<Inline> {
        self.ends_space = true;
        let mut out = Vec::new();
        for n in nodes {
            self.inline(n, &mut out);
        }
        trim_inlines(&mut out);
        out
    }

    fn inline_children(&mut self, e: &Elem) -> Vec<Inline> {
        self.inline_run(e.children.iter())
    }

    fn block_elem(&mut self, e: &Elem, plain: bool, out: &mut Vec<Block>) {
        self.depth += 1;
        if self.depth > 200 {
            self.depth -= 1;
            return;
        }
        match e.tag.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = e.tag.as_bytes()[1] - b'0';
                let inlines = self.inline_children(e);
                if !inlines.is_empty() {
                    out.push(Block::Heading { level, inlines, id: e.attr("id").map(str::to_string) });
                }
            }
            "p" | "caption" | "figcaption" | "legend" | "summary" | "dt" | "dd" | "td" | "th" => {
                if has_block_child(e) {
                    out.extend(self.blocks(&e.children, plain));
                } else {
                    let inl = self.inline_children(e);
                    if !inl.is_empty() {
                        out.push(if plain { Block::Plain(inl) } else { Block::Para(inl) });
                    }
                }
            }
            "pre" => {
                let text = e.text();
                let text = text.strip_prefix('\n').unwrap_or(&text).trim_end_matches('\n').to_string();
                out.push(Block::Code { lang: code_lang(e), text });
            }
            "blockquote" => {
                let inner = self.blocks(&e.children, false);
                if !inner.is_empty() {
                    out.push(Block::Quote(inner));
                }
            }
            "ul" | "menu" | "dir" => {
                let items = self.list_items(e);
                if !items.is_empty() {
                    out.push(Block::BulletList(items));
                }
            }
            "ol" => {
                let items = self.list_items(e);
                if !items.is_empty() {
                    let start = e.attr("start").and_then(|s| s.trim().parse().ok()).unwrap_or(1);
                    let style = match e.attr("type") {
                        Some("a") => ListStyle::LowerAlpha,
                        Some("A") => ListStyle::UpperAlpha,
                        Some("i") => ListStyle::LowerRoman,
                        Some("I") => ListStyle::UpperRoman,
                        _ => ListStyle::Decimal,
                    };
                    out.push(Block::OrderedList { start, style, items });
                }
            }
            "li" => out.extend(self.blocks(&e.children, plain)),
            "dl" => {
                let defs = self.def_list(e);
                if !defs.is_empty() {
                    out.push(Block::DefList(defs));
                }
            }
            "table" => {
                if let Some(t) = self.table(e) {
                    out.push(Block::Table(t));
                }
            }
            "hr" => out.push(Block::Rule),
            "figure" => self.figure(e, out),
            "math" => {
                let tex = mathml_tex(e);
                if !tex.is_empty() {
                    out.push(Block::Para(vec![Inline::Math { display: true, tex }]));
                }
            }
            _ => out.extend(self.blocks(&e.children, plain)),
        }
        self.depth -= 1;
    }

    fn figure(&mut self, e: &Elem, out: &mut Vec<Block>) {
        let mut imgs = Vec::new();
        let mut caption: Option<&Elem> = None;
        find_all(&e.children, "img", &mut imgs);
        for n in &e.children {
            if let Node::Elem(c) = n {
                if c.tag == "figcaption" {
                    caption = Some(c);
                }
            }
        }
        if imgs.len() == 1 && caption.is_some() {
            let cap = caption.map(|c| self.inline_children(c)).unwrap_or_default();
            out.push(Block::Figure { url: imgs[0].attr("src").unwrap_or("").to_string(), caption: cap });
        } else {
            out.extend(self.blocks(&e.children, false));
        }
    }

    fn list_items(&mut self, e: &Elem) -> Vec<Vec<Block>> {
        let mut items = Vec::new();
        for n in &e.children {
            if let Node::Elem(li) = n {
                if li.tag == "li" {
                    let b = self.blocks(&li.children, true);
                    items.push(b);
                } else if li.tag == "ul" || li.tag == "ol" {
                    // A list directly inside a list: Pandoc attaches it to the previous item.
                    let mut inner = Vec::new();
                    self.block_elem(li, true, &mut inner);
                    if let Some(last) = items.last_mut() {
                        let last: &mut Vec<Block> = last;
                        last.extend(inner);
                    } else {
                        items.push(inner);
                    }
                }
            }
        }
        items
    }

    fn def_list(&mut self, e: &Elem) -> Vec<(Vec<Inline>, Vec<Vec<Block>>)> {
        let mut out: Vec<(Vec<Inline>, Vec<Vec<Block>>)> = Vec::new();
        for n in &e.children {
            let Node::Elem(c) = n else { continue };
            match c.tag.as_str() {
                "dt" => {
                    let term = self.inline_children(c);
                    out.push((term, Vec::new()));
                }
                "dd" => {
                    let body = self.blocks(&c.children, true);
                    match out.last_mut() {
                        Some(last) => last.1.push(body),
                        None => out.push((Vec::new(), vec![body])),
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn table(&mut self, e: &Elem) -> Option<Table> {
        let mut t = Table::default();
        let mut head_rows: Vec<&Elem> = Vec::new();
        let mut body_rows: Vec<&Elem> = Vec::new();
        for n in &e.children {
            let Node::Elem(c) = n else { continue };
            match c.tag.as_str() {
                "caption" => t.caption = self.inline_children(c),
                "thead" => collect_rows(c, &mut head_rows),
                "tbody" | "tfoot" => collect_rows(c, &mut body_rows),
                "tr" => body_rows.push(c),
                _ => {}
            }
        }
        // A table with no <thead> whose first row is all <th> has that row as its header.
        if head_rows.is_empty() {
            if let Some(first) = body_rows.first() {
                let cells: Vec<&Elem> = row_cells(first);
                if !cells.is_empty() && cells.iter().all(|c| c.tag == "th") {
                    head_rows.push(body_rows.remove(0));
                }
            }
        }
        let conv = |rows: &[&Elem], align: &mut Vec<Align>, me: &mut Self| -> Vec<Vec<Cell>> {
            let mut out = Vec::new();
            for r in rows {
                let mut cells = Vec::new();
                for (ci, c) in row_cells(r).into_iter().enumerate() {
                    let a = cell_align(c);
                    if a != Align::Default && align.get(ci).copied().unwrap_or_default() == Align::Default {
                        if align.len() <= ci {
                            align.resize(ci + 1, Align::Default);
                        }
                        align[ci] = a;
                    }
                    cells.push(Cell {
                        blocks: me.blocks(&c.children, true),
                        colspan: c.attr("colspan").and_then(|v| v.trim().parse().ok()).unwrap_or(1).clamp(1, 1000),
                        rowspan: c.attr("rowspan").and_then(|v| v.trim().parse().ok()).unwrap_or(1).clamp(1, 1000),
                    });
                }
                if !cells.is_empty() {
                    out.push(cells);
                }
            }
            out
        };
        let mut align = Vec::new();
        t.head = conv(&head_rows, &mut align, self);
        t.rows = conv(&body_rows, &mut align, self);
        t.align = align;
        if t.head.is_empty() && t.rows.is_empty() {
            None
        } else {
            Some(t)
        }
    }

    // ---- inlines ---------------------------------------------------------------------------

    fn inline(&mut self, n: &Node, out: &mut Vec<Inline>) {
        let e = match n {
            Node::Text(t) => {
                let mut s = collapse(t);
                if self.ends_space && s.starts_with(' ') {
                    s.remove(0);
                }
                if s.is_empty() {
                    return;
                }
                self.ends_space = s.ends_with(' ');
                out.push(Inline::Text(s));
                return;
            }
            Node::Elem(e) => e,
        };
        if is_skipped(&e.tag) {
            return;
        }
        self.depth += 1;
        if self.depth > 200 {
            self.depth -= 1;
            return;
        }
        match e.tag.as_str() {
            "br" => {
                out.push(Inline::LineBreak);
                self.ends_space = true;
            }
            "wbr" => {}
            "em" | "i" | "cite" | "dfn" | "var" => {
                let v = self.nested(e);
                if !v.is_empty() {
                    out.push(Inline::Emph(v));
                }
            }
            "strong" | "b" => {
                let v = self.nested(e);
                if !v.is_empty() {
                    out.push(Inline::Strong(v));
                }
            }
            "u" | "ins" => {
                let v = self.nested(e);
                if !v.is_empty() {
                    out.push(Inline::Underline(v));
                }
            }
            "s" | "strike" | "del" => {
                let v = self.nested(e);
                if !v.is_empty() {
                    out.push(Inline::Strike(v));
                }
            }
            "sup" => {
                let v = self.nested(e);
                if !v.is_empty() {
                    out.push(Inline::Sup(v));
                }
            }
            "sub" => {
                let v = self.nested(e);
                if !v.is_empty() {
                    out.push(Inline::Sub(v));
                }
            }
            "code" | "tt" | "kbd" | "samp" => {
                let t = e.text().replace('\n', " ");
                if !t.is_empty() {
                    out.push(Inline::Code(t));
                    self.ends_space = false;
                }
            }
            "q" => {
                let v = self.nested(e);
                out.push(Inline::Text("\u{201c}".into()));
                out.extend(v);
                out.push(Inline::Text("\u{201d}".into()));
                self.ends_space = false;
            }
            "img" => {
                let alt = e.attr("alt").unwrap_or("").to_string();
                let title = e.attr("title").filter(|t| !t.is_empty()).map(str::to_string);
                let alt = if alt.is_empty() { Vec::new() } else { vec![Inline::Text(alt)] };
                out.push(Inline::Image { url: e.attr("src").unwrap_or("").to_string(), alt, title });
                self.ends_space = false;
            }
            "math" => {
                let tex = mathml_tex(e);
                if !tex.is_empty() {
                    out.push(Inline::Math { display: e.attr("display") == Some("block"), tex });
                    self.ends_space = false;
                }
            }
            "a" => self.anchor(e, out),
            "span" if e.has_class("math") => {
                let raw = e.text();
                let raw = raw.trim();
                let display = e.has_class("display");
                let tex = raw
                    .strip_prefix("\\(")
                    .and_then(|s| s.strip_suffix("\\)"))
                    .or_else(|| raw.strip_prefix("\\[").and_then(|s| s.strip_suffix("\\]")))
                    .unwrap_or(raw);
                out.push(Inline::Math { display, tex: tex.trim().to_string() });
                self.ends_space = false;
            }
            "span" if e.has_class("smallcaps") => {
                let v = self.nested(e);
                out.push(Inline::SmallCaps(v));
            }
            "input" => {}
            _ => {
                // Unknown and structural elements met inside a paragraph: their text is kept.
                if is_block_tag(&e.tag) && !out.is_empty() && !self.ends_space {
                    out.push(Inline::Text(" ".into()));
                    self.ends_space = true;
                }
                for c in &e.children {
                    self.inline(c, out);
                }
            }
        }
        self.depth -= 1;
    }

    /// Children as inlines, keeping the running whitespace state.
    fn nested(&mut self, e: &Elem) -> Vec<Inline> {
        let mut v = Vec::new();
        for c in &e.children {
            self.inline(c, &mut v);
        }
        v
    }

    fn anchor(&mut self, e: &Elem, out: &mut Vec<Inline>) {
        let href = e.attr("href").unwrap_or("");
        let ty = e.attr("epub:type").unwrap_or("");
        let role = e.attr("role").unwrap_or("");
        let is_ref = role == "doc-noteref"
            || ty.split_whitespace().any(|t| t == "noteref")
            || e.has_class("footnote-ref")
            || e.has_class("sdfootnoteanc");
        if is_ref {
            if let Some(frag) = href.rsplit_once('#').map(|(_, f)| f) {
                let mut key = frag.to_string();
                if !self.notes.contains_key(&key) {
                    if let Some(k) = key.strip_suffix("sym") {
                        key = k.to_string();
                    }
                }
                if let Some(note) = self.notes.remove(&key) {
                    let saved = self.ends_space;
                    let blocks = self.note_blocks(&note);
                    self.ends_space = saved;
                    out.push(Inline::Note(blocks));
                    self.ends_space = false;
                    return;
                }
            }
        }
        // Backlinks from a footnote body to its reference are not content.
        if role == "doc-backlink" || e.has_class("footnote-back") {
            return;
        }
        let inner = self.nested(e);
        if href.is_empty() || inner.is_empty() {
            out.extend(inner);
            return;
        }
        let title = e.attr("title").filter(|t| !t.is_empty()).map(str::to_string);
        out.push(Inline::Link { url: href.to_string(), title, inlines: inner });
    }

    fn note_blocks(&mut self, e: &Elem) -> Vec<Block> {
        let mut kids: Vec<Node> = e.children.clone();
        strip_note_markers(&mut kids);
        let mut blocks = self.blocks(&kids, false);
        if blocks.is_empty() {
            blocks.push(Block::Para(Vec::new()));
        }
        blocks
    }
}

/// Footnote items inside a notes container: list items and elements with an id.
fn collect_note_items(nodes: &[Node], notes: &mut HashMap<String, Elem>, found: &mut bool) {
    for n in nodes {
        let Node::Elem(e) = n else { continue };
        if e.tag == "li" || e.tag == "aside" || (e.tag == "div" && e.attr("id").is_some()) || e.tag == "p" && e.attr("id").is_some() {
            if let Some(id) = e.attr("id") {
                notes.insert(id.to_string(), e.clone());
                *found = true;
                continue;
            }
        }
        collect_note_items(&e.children, notes, found);
    }
}

/// Drops the back-reference link and the leading marker (`<a class="sdfootnotesym">1</a>`).
fn strip_note_markers(nodes: &mut Vec<Node>) {
    nodes.retain(|n| match n {
        Node::Elem(e) => {
            !(e.attr("role") == Some("doc-backlink") || e.has_class("footnote-back") || e.has_class("sdfootnotesym"))
        }
        _ => true,
    });
    for n in nodes.iter_mut() {
        if let Node::Elem(e) = n {
            strip_note_markers(&mut e.children);
        }
    }
}

fn has_block_child(e: &Elem) -> bool {
    e.children.iter().any(|c| match c {
        Node::Elem(c) => is_block_tag(&c.tag) || (!is_skipped(&c.tag) && has_block_child(c)),
        _ => false,
    })
}

fn find_all<'a>(nodes: &'a [Node], tag: &str, out: &mut Vec<&'a Elem>) {
    for n in nodes {
        if let Node::Elem(e) = n {
            if e.tag == tag {
                out.push(e);
            } else {
                find_all(&e.children, tag, out);
            }
        }
    }
}

fn collect_rows<'a>(e: &'a Elem, rows: &mut Vec<&'a Elem>) {
    for n in &e.children {
        if let Node::Elem(c) = n {
            match c.tag.as_str() {
                "tr" => rows.push(c),
                "tbody" | "thead" | "tfoot" => collect_rows(c, rows),
                _ => {}
            }
        }
    }
}

fn row_cells(tr: &Elem) -> Vec<&Elem> {
    tr.children
        .iter()
        .filter_map(|n| match n {
            Node::Elem(c) if c.tag == "td" || c.tag == "th" => Some(c),
            _ => None,
        })
        .collect()
}

fn cell_align(c: &Elem) -> Align {
    let style = c.attr("style").unwrap_or("").replace(' ', "").to_ascii_lowercase();
    let a = c.attr("align").unwrap_or("").to_ascii_lowercase();
    if a == "left" || style.contains("text-align:left") {
        Align::Left
    } else if a == "right" || style.contains("text-align:right") {
        Align::Right
    } else if a == "center" || style.contains("text-align:center") {
        Align::Center
    } else {
        Align::Default
    }
}

fn code_lang(pre: &Elem) -> Option<String> {
    let mut classes: Vec<String> = pre.attr("class").unwrap_or("").split_whitespace().map(str::to_string).collect();
    for n in &pre.children {
        if let Node::Elem(c) = n {
            if c.tag == "code" {
                classes.extend(c.attr("class").unwrap_or("").split_whitespace().map(str::to_string));
            }
        }
    }
    classes
        .into_iter()
        .map(|c| c.strip_prefix("language-").map(str::to_string).unwrap_or(c))
        .find(|c| !matches!(c.as_str(), "sourceCode" | "numberSource" | "hljs" | "code" | "highlight" | "prettyprint" | "pre"))
}

fn mathml_tex(e: &Elem) -> String {
    crate::mathml::to_tex(&e.to_markup()).tex.trim().to_string()
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c == ' ' || c == '\n' || c == '\t' || c == '\r' || c == '\u{c}' {
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

/// Removes a trailing space (and a trailing hard break) from the end of a run.
fn trim_inlines(v: &mut Vec<Inline>) {
    loop {
        match v.last_mut() {
            Some(Inline::Text(t)) => {
                let trimmed = t.trim_end_matches(' ').len();
                t.truncate(trimmed);
                if t.is_empty() {
                    v.pop();
                    continue;
                }
            }
            Some(Inline::LineBreak) | Some(Inline::SoftBreak) => {
                v.pop();
                continue;
            }
            Some(Inline::Emph(i))
            | Some(Inline::Strong(i))
            | Some(Inline::Strike(i))
            | Some(Inline::Sup(i))
            | Some(Inline::Sub(i))
            | Some(Inline::Underline(i))
            | Some(Inline::SmallCaps(i)) => {
                trim_inlines(i);
                if i.is_empty() {
                    v.pop();
                    continue;
                }
            }
            _ => {}
        }
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        to_markdown_document(html.as_bytes())
    }

    #[test]
    fn headings_lists_tables_and_inline_markup() {
        let out = md("<html><head><title>T</title></head><body><h1>Head</h1><p>a <em>b</em> <strong>c</strong> <code>d</code> x<sup>2</sup> H<sub>2</sub>O <s>gone</s></p>\
            <ul><li>one<ul><li>nested</li></ul></li><li>two</li></ul>\
            <table><tr><th>A</th><th align=right>B</th></tr><tr><td>1</td><td>2</td></tr></table></body></html>");
        assert!(out.starts_with("---\ntitle: T\n---\n\n# Head\n"), "{out}");
        assert!(out.contains("a *b* **c** `d` x^2^ H~2~O ~~gone~~"), "{out}");
        assert!(out.contains("-   one\n    -   nested\n-   two"), "{out}");
        assert!(out.contains("| A | B |\n|---|---:|\n| 1 | 2 |"), "{out}");
    }

    const NOTE_HTML: &str = "<p>Text<a class=\"footnote-ref\" href=\"#fn1\" id=\"fnref1\" role=\"doc-noteref\"><sup>1</sup></a> and \
            <span class=\"math inline\">\\(x^2\\)</span>.</p>\
            <section class=\"footnotes\" role=\"doc-endnotes\"><ol><li id=\"fn1\"><p>The note.<a href=\"#fnref1\" class=\"footnote-back\" role=\"doc-backlink\">x</a></p></li></ol></section>";

    #[test]
    fn html_documents_keep_footnote_links_as_pandoc_does() {
        // Pandoc's HTML reader keeps the reference as a link and the note list as a list.
        let out = md(NOTE_HTML);
        assert!(out.contains("[^1^](#fn1)"), "{out}");
        assert!(out.contains("$x^2$"), "{out}");
    }

    #[test]
    fn epub_footnote_refs_become_notes_and_math_is_kept() {
        let out = crate::ast::to_markdown(&to_doc_epub(NOTE_HTML));
        assert!(out.contains("Text[^1] and $x^2$."), "{out}");
        assert!(out.contains("[^1]: The note."), "{out}");
        assert!(!out.contains("footnote"), "{out}");
    }

    #[test]
    fn code_blocks_keep_language_and_text() {
        let out = md("<pre><code class=\"language-rust\">fn main() {\n    1 &lt; 2\n}\n</code></pre>");
        assert!(out.contains("```rust\nfn main() {\n    1 < 2\n}\n```"), "{out}");
    }
}
