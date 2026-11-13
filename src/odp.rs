//! OpenDocument presentations (`.odp`) as Markdown, the way the PPTX reader writes them: one
//! `## Slide N: Title` section per slide in presentation order, the slide's text in reading order
//! (document order of its frames, groups included), bullets with their nesting and numbered lists
//! from the list style, tables as Markdown tables, links, picture references, and the speaker
//! notes. Headers, footers, dates and slide numbers are dropped.
//!
//! The title is the frame LibreOffice and Impress mark `presentation:class="title"`; a file that
//! does not mark one (written by a library) has slides without a title, and its first frame stays
//! ordinary text. Not read: charts, embedded objects, drawing shapes without text, animations.

use std::collections::HashSet;

use crate::document::{archive, entry, markdown_table};
use crate::xmltree::{self, XElem, XNode};

#[derive(Default)]
struct Slide {
    title: Vec<String>,
    /// (is list item, text)
    body: Vec<(bool, String)>,
    notes: Vec<String>,
}

struct Reader {
    /// Names of the list styles whose first level is numbered.
    ordered: HashSet<String>,
}

/// Reads an `.odp` file and writes it as Markdown.
pub fn to_markdown(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let content = xmltree::parse(&entry(&mut zip, "content.xml")?)?;
    let mut reader = Reader {
        ordered: HashSet::new(),
    };
    reader.collect_list_styles(&content);
    let Some(presentation) = content
        .find("office:body")
        .and_then(|body| body.find("office:presentation"))
    else {
        return Ok(String::new());
    };
    let mut out = String::new();
    for (index, page) in presentation
        .elems()
        .filter(|e| e.name == "draw:page")
        .enumerate()
    {
        let mut slide = Slide::default();
        reader.shapes(page, &mut slide);
        let body = join(&slide.body);
        let notes = slide.notes.join(" ");
        if slide.title.is_empty() && body.is_empty() && notes.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n---\n\n");
        }
        out.push_str(&format!("## Slide {}", index + 1));
        if !slide.title.is_empty() {
            out.push_str(&format!(": {}", slide.title.join(" ")));
        }
        if !body.is_empty() {
            out.push_str("\n\n");
            out.push_str(&body);
        }
        if !notes.trim().is_empty() {
            out.push_str(&format!("\n\n**Notes:** {}", notes.trim()));
        }
    }
    Ok(out)
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

impl Reader {
    fn collect_list_styles(&mut self, root: &XElem) {
        for e in root.elems() {
            if e.name == "text:list-style" {
                let numbered = e
                    .elems()
                    .next()
                    .is_some_and(|level| level.name == "text:list-level-style-number");
                if let (true, Some(name)) = (numbered, e.attr("style:name")) {
                    self.ordered.insert(name.to_owned());
                }
            } else {
                self.collect_list_styles(e);
            }
        }
    }

    /// The shapes of a page or group, in document order.
    fn shapes(&self, parent: &XElem, slide: &mut Slide) {
        for e in parent.elems() {
            match e.name.as_str() {
                "draw:frame" => self.frame(e, slide),
                "draw:g" | "draw:a" => self.shapes(e, slide),
                "draw:custom-shape" | "draw:text-box" => self.blocks(e, None, slide),
                "presentation:notes" => {
                    let mut notes = Slide::default();
                    self.shapes(e, &mut notes);
                    slide
                        .notes
                        .extend(notes.body.into_iter().map(|(_, text)| text));
                }
                _ => {}
            }
        }
    }

    fn frame(&self, frame: &XElem, slide: &mut Slide) {
        let class = frame.attr("presentation:class");
        if matches!(
            class,
            Some("page-number" | "date-time" | "footer" | "header" | "handout")
        ) {
            return;
        }
        let alt = frame
            .elems()
            .find(|e| e.name == "svg:title" || e.name == "svg:desc")
            .map(|e| text_of(&e.children))
            .unwrap_or_default();
        for child in frame.elems() {
            match child.name.as_str() {
                "draw:text-box" => self.blocks(child, class, slide),
                "table:table" => {
                    let markdown = markdown_table(&table_rows(child));
                    if !markdown.is_empty() {
                        slide.body.push((false, markdown.trim_end().to_owned()));
                    }
                }
                "draw:image" => {
                    if let Some(href) = child.attr("xlink:href") {
                        slide.body.push((false, format!("![{alt}]({href})")));
                    }
                }
                "draw:g" => self.shapes(child, slide),
                _ => {}
            }
        }
    }

    /// Paragraphs, headings and lists of a text box.
    fn blocks(&self, container: &XElem, class: Option<&str>, slide: &mut Slide) {
        for e in container.elems() {
            match e.name.as_str() {
                "text:p" | "text:h" => {
                    let text = text_of(&e.children);
                    if text.is_empty() {
                        continue;
                    }
                    if class == Some("title") {
                        slide.title.push(text);
                    } else {
                        slide.body.push((false, text));
                    }
                }
                "text:list" => self.list(e, 0, false, slide),
                _ => {}
            }
        }
    }

    fn list(&self, list: &XElem, depth: usize, inherited_ordered: bool, slide: &mut Slide) {
        let ordered = list
            .attr("text:style-name")
            .map_or(inherited_ordered, |s| self.ordered.contains(s));
        let mut number = 0;
        for item in list.elems().filter(|e| e.name == "text:list-item") {
            for child in item.elems() {
                match child.name.as_str() {
                    "text:p" | "text:h" => {
                        let text = text_of(&child.children);
                        if text.is_empty() {
                            continue;
                        }
                        number += 1;
                        let marker = if ordered {
                            format!("{number}.")
                        } else {
                            "-".to_owned()
                        };
                        slide.body.push((
                            true,
                            format!("{}{marker} {text}", "    ".repeat(depth.min(8))),
                        ));
                    }
                    "text:list" => self.list(child, depth + 1, ordered, slide),
                    _ => {}
                }
            }
        }
    }
}

/// Plain text of inline content, links as Markdown links. Runs of whitespace are one space, as
/// in ODF unless `text:s` is used.
fn text_of(nodes: &[XNode]) -> String {
    let mut out = String::new();
    push_text(nodes, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_text(nodes: &[XNode], out: &mut String) {
    for node in nodes {
        match node {
            XNode::Text(t) => out.push_str(t),
            XNode::Elem(e) => match e.name.as_str() {
                "text:s" | "text:tab" | "text:line-break" => out.push(' '),
                "text:a" => {
                    let mut inner = String::new();
                    push_text(&e.children, &mut inner);
                    match e.attr("xlink:href") {
                        Some(href) if !inner.trim().is_empty() => {
                            out.push_str(&format!("[{}]({href})", inner.trim()));
                        }
                        _ => out.push_str(&inner),
                    }
                }
                "text:note" | "office:annotation" | "draw:frame" => {}
                _ => push_text(&e.children, out),
            },
        }
    }
}

fn table_rows(table: &XElem) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    collect_rows(table, &mut rows);
    rows
}

fn collect_rows(parent: &XElem, rows: &mut Vec<Vec<String>>) {
    for e in parent.elems() {
        match e.name.as_str() {
            "table:table-row" => {
                let mut row = Vec::new();
                for cell in e.elems().filter(|c| c.name == "table:table-cell") {
                    let repeat = cell
                        .attr("table:number-columns-repeated")
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(64);
                    let text = cell
                        .elems()
                        .filter(|p| matches!(p.name.as_str(), "text:p" | "text:h"))
                        .map(|p| text_of(&p.children))
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join("<br>");
                    for _ in 0..repeat {
                        row.push(text.clone());
                    }
                }
                while row.last().is_some_and(String::is_empty) {
                    row.pop();
                }
                if !row.is_empty() {
                    rows.push(row);
                }
            }
            "table:table-header-rows" | "table:table-rows" | "table:table-row-group" => {
                collect_rows(e, rows)
            }
            _ => {}
        }
    }
}
