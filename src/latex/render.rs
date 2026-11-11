//! Renderers for `Document`: plain text, Markdown and HTML. They share one walker, so the three
//! outputs have the same structure. Math is written as TeX source in every format: `$...$` and
//! `$$...$$` in Markdown, `\(...\)` and `\[...\]` in HTML inside `span`/`div` elements with the
//! classes `math inline` and `math display`.

use std::fmt::Write as _;

use super::parser::plain;
use super::{Block, Cite, Document, Figure, ImagePath, Inline, ListKind, Picture, Table};
use crate::bibtex::Bibliography;

/// Optional inputs for rendering.
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderOptions<'a> {
    /// Used to label `\cite` keys that are not in the document's `thebibliography`.
    pub bibliography: Option<&'a Bibliography>,
}

/// Renders plain text.
pub fn render_text(doc: &Document, options: &RenderOptions) -> String {
    Renderer::new(doc, options, Mode::Text).document()
}

/// Renders Markdown, with `$...$` and `$$...$$` math.
pub fn render_markdown(doc: &Document, options: &RenderOptions) -> String {
    Renderer::new(doc, options, Mode::Markdown).document()
}

/// Renders an HTML fragment.
pub fn render_html(doc: &Document, options: &RenderOptions) -> String {
    Renderer::new(doc, options, Mode::Html).document()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Markdown,
    Html,
}

struct Renderer<'a> {
    doc: &'a Document,
    options: &'a RenderOptions<'a>,
    mode: Mode,
    notes: Vec<String>,
}

impl<'a> Renderer<'a> {
    fn new(doc: &'a Document, options: &'a RenderOptions<'a>, mode: Mode) -> Self {
        Self {
            doc,
            options,
            mode,
            notes: Vec::new(),
        }
    }

    fn document(mut self) -> String {
        let mut out = String::new();
        if let Some(title) = &self.doc.title {
            let t = self.inlines(title);
            out.push_str(&self.heading(1, &t));
        }
        if !self.doc.authors.is_empty() {
            let authors: Vec<String> = self
                .doc
                .authors
                .clone()
                .iter()
                .map(|a| self.inlines(a))
                .collect();
            out.push_str(&self.paragraph(&authors.join(", ")));
        }
        if let Some(date) = &self.doc.date {
            if !date.is_empty() {
                let d = self.escape(date);
                out.push_str(&self.paragraph(&d));
            }
        }
        if let Some(abs) = &self.doc.abstract_body {
            let heading_depth = if self.doc.title.is_some() { 2 } else { 1 };
            let h = self.heading(heading_depth, "Abstract");
            out.push_str(&h);
            out.push_str(&self.blocks(abs));
        }
        out.push_str(&self.blocks(&self.doc.blocks));
        if !self.doc.bibitems.is_empty() {
            let depth = if self.doc.title.is_some() { 2 } else { 1 };
            out.push_str(&self.heading(depth, "References"));
            let bib: Vec<Vec<Inline>> = self.doc.bibitems.iter().map(|b| b.text.clone()).collect();
            for (i, text) in bib.iter().enumerate() {
                let body = self.inlines(text);
                let label = self.doc.bibitems[i]
                    .label
                    .as_deref()
                    .map(bibitem_label)
                    .unwrap_or_else(|| format!("{}", i + 1));
                let line = match self.mode {
                    Mode::Html => format!("<p>[{}] {}</p>\n", escape_html(&label), body),
                    _ => format!("[{}] {}\n\n", self.escape(&label), body),
                };
                out.push_str(&line);
            }
        }
        if !self.notes.is_empty() {
            let notes = std::mem::take(&mut self.notes);
            for (i, note) in notes.iter().enumerate() {
                match self.mode {
                    Mode::Markdown => {
                        let _ = writeln!(out, "[^{}]: {}", i + 1, note);
                    }
                    Mode::Html => {
                        let _ = writeln!(out, "<p id=\"fn-{}\">{} {}</p>", i + 1, i + 1, note);
                    }
                    Mode::Text => {
                        let _ = writeln!(out, "[{}] {}", i + 1, note);
                    }
                }
            }
        }
        match self.mode {
            Mode::Html => out,
            _ => out.trim_end().to_string() + "\n",
        }
    }

    // ----- mode helpers ---------------------------------------------------------------------

    fn escape(&self, s: &str) -> String {
        match self.mode {
            Mode::Text => s.to_string(),
            Mode::Html => escape_html(s),
            Mode::Markdown => escape_markdown(s),
        }
    }

    fn heading(&self, level: u8, text: &str) -> String {
        match self.mode {
            Mode::Text => format!("{text}\n\n"),
            Mode::Markdown => format!(
                "{} {}\n\n",
                "#".repeat(usize::from(level).clamp(1, 6)),
                text
            ),
            Mode::Html => format!("<h{l}>{text}</h{l}>\n", l = level.clamp(1, 6)),
        }
    }

    fn paragraph(&self, text: &str) -> String {
        match self.mode {
            Mode::Html => format!("<p>{text}</p>\n"),
            _ => format!("{text}\n\n"),
        }
    }

    fn emph(&self, s: &str) -> String {
        match self.mode {
            Mode::Text => s.to_string(),
            Mode::Markdown => format!("*{s}*"),
            Mode::Html => format!("<em>{s}</em>"),
        }
    }

    fn strong(&self, s: &str) -> String {
        match self.mode {
            Mode::Text => s.to_string(),
            Mode::Markdown => format!("**{s}**"),
            Mode::Html => format!("<strong>{s}</strong>"),
        }
    }

    fn code(&self, s: &str) -> String {
        match self.mode {
            Mode::Text => s.to_string(),
            Mode::Markdown => {
                if s.contains('`') {
                    format!("`` {s} ``")
                } else {
                    format!("`{s}`")
                }
            }
            Mode::Html => format!("<code>{}</code>", escape_html(s)),
        }
    }

    fn inline_math(&self, tex: &str) -> String {
        match self.mode {
            Mode::Text => tex.to_string(),
            Mode::Markdown => format!("${tex}$"),
            Mode::Html => format!(
                "<span class=\"math inline\">\\({}\\)</span>",
                escape_html(tex)
            ),
        }
    }

    fn display_math(&self, tex: &str, tag_value: Option<&str>) -> String {
        // A row that already carries its own \tag keeps it; otherwise the number is added.
        let tag = match tag_value {
            Some(t) if !tex.contains("\\tag") => format!(" \\tag{{{t}}}"),
            _ => String::new(),
        };
        // Alignment characters need an aligned environment outside align/eqnarray.
        let aligned = tex.contains('&') && !tex.contains("\\begin{");
        let tex = if aligned {
            format!("\\begin{{aligned}}\n{tex}\n\\end{{aligned}}")
        } else {
            tex.to_string()
        };
        let tex = tex.as_str();
        match self.mode {
            Mode::Text => match tag_value {
                Some(t) => format!("{tex} ({t})\n\n"),
                None => format!("{tex}\n\n"),
            },
            Mode::Markdown => format!("$$\n{tex}{tag}\n$$\n\n"),
            Mode::Html => format!(
                "<div class=\"math display\">\\[{}{}\\]</div>\n",
                escape_html(tex),
                escape_html(&tag)
            ),
        }
    }

    fn link(&self, url: &str, text: &str) -> String {
        match self.mode {
            Mode::Text => {
                if text == url {
                    url.to_string()
                } else {
                    format!("{text} ({url})")
                }
            }
            Mode::Markdown => {
                if text == url {
                    format!("<{url}>")
                } else {
                    format!("[{text}]({url})")
                }
            }
            Mode::Html => format!("<a href=\"{}\">{}</a>", escape_html(url), text),
        }
    }

    fn image(&self, p: &ImagePath) -> String {
        match self.mode {
            Mode::Text => format!("[image: {}]", p.declared),
            Mode::Markdown => format!("![]({})", p.declared),
            Mode::Html => format!("<img src=\"{}\" alt=\"\">", escape_html(&p.declared)),
        }
    }

    fn break_line(&self) -> &'static str {
        match self.mode {
            Mode::Text => "\n",
            Mode::Markdown => "\\\n",
            Mode::Html => "<br>\n",
        }
    }

    // ----- inline content -------------------------------------------------------------------

    fn inlines(&mut self, inls: &[Inline]) -> String {
        let mut out = String::new();
        for inl in inls {
            let piece = self.inline(inl);
            out.push_str(&piece);
        }
        out.trim().to_string()
    }

    fn inline(&mut self, inl: &Inline) -> String {
        match inl {
            Inline::Text(t) => self.escape(t),
            Inline::Emph(v) => {
                let s = self.inlines(v);
                self.emph(&s)
            }
            Inline::Strong(v) => {
                let s = self.inlines(v);
                self.strong(&s)
            }
            Inline::Mono(v) => {
                let s = plain(v);
                self.code(&s)
            }
            Inline::SmallCaps(v) => self.inlines(v),
            Inline::Code(s) => self.code(s),
            Inline::Math { tex, display } => {
                if *display {
                    format!(" {} ", self.inline_math(tex))
                } else {
                    self.inline_math(tex)
                }
            }
            Inline::Cite(c) => self.cite(c),
            Inline::Ref(r) => {
                let number = self.doc.label_number(&r.label).map(str::to_string);
                match (number, r.command.as_str()) {
                    (Some(n), "eqref") => self.escape(&format!("({n})")),
                    (Some(n), _) => self.escape(&n),
                    (None, _) => self.escape("??"),
                }
            }
            Inline::Link { url, text } => {
                let t = self.inlines(text);
                self.link(url, &t)
            }
            Inline::Image(p) => self.image(p),
            Inline::Footnote(v) => {
                let text = self.inlines(v);
                self.notes.push(text);
                let n = self.notes.len();
                match self.mode {
                    Mode::Text => format!("[{n}]"),
                    Mode::Markdown => format!("[^{n}]"),
                    Mode::Html => format!("<sup><a href=\"#fn-{n}\">{n}</a></sup>"),
                }
            }
            Inline::Break => self.break_line().to_string(),
        }
    }

    fn cite(&self, c: &Cite) -> String {
        let mut labels = Vec::new();
        for key in &c.keys {
            labels.push(self.cite_label(key, &c.command));
        }
        let body = labels.join("; ");
        let body = match (&c.pre, &c.post) {
            (Some(p), Some(q)) => format!("{p} {body}, {q}"),
            (None, Some(q)) => format!("{body}, {q}"),
            _ => body,
        };
        let textual = matches!(c.command.as_str(), "citet" | "citeauthor" | "citeyear");
        let text = if textual { body } else { format!("({body})") };
        self.escape(&text)
    }

    /// The text shown for one key: "Author Year" from a bibliography, or the item label from
    /// `thebibliography`, or the key itself when nothing matches.
    fn cite_label(&self, key: &str, command: &str) -> String {
        if let Some(i) = self.doc.bibitems.iter().position(|b| b.key == key) {
            return self.doc.bibitems[i]
                .label
                .as_deref()
                .map(bibitem_label)
                .unwrap_or_else(|| (i + 1).to_string());
        }
        let Some(bib) = self.options.bibliography else {
            return key.to_string();
        };
        let Some(entry) = bib.lookup(key) else {
            return key.to_string();
        };
        let names = entry.authors();
        if names.names.is_empty() && entry.year().is_none() {
            // Nothing to build "Author, Year" from (for example an entry made from a
            // `\bibitem`): its own label, or the key.
            return match entry.value("label") {
                Some(label) => bibitem_label(label),
                None => entry.key.clone(),
            };
        }
        let family: Vec<String> = names.names.iter().map(|n| n.family.clone()).collect();
        let who = match family.len() {
            0 => entry.key.clone(),
            1 => family[0].clone(),
            2 => format!("{} and {}", family[0], family[1]),
            _ => format!("{} et al.", family[0]),
        };
        let year = entry
            .year()
            .map(|y| y.to_string())
            .unwrap_or_else(|| "n.d.".to_string());
        if command == "citeyear" {
            year
        } else {
            format!("{who}, {year}")
        }
    }

    // ----- blocks ---------------------------------------------------------------------------

    fn blocks(&mut self, blocks: &[Block]) -> String {
        let mut out = String::new();
        for b in blocks {
            out.push_str(&self.block(b));
        }
        out
    }

    fn block(&mut self, block: &Block) -> String {
        match block {
            Block::Section(s) => {
                let depth = if self.doc.title.is_some() {
                    usize::from(s.level) + 1
                } else {
                    usize::from(s.level)
                };
                let title = self.inlines(&s.title);
                self.heading(u8::try_from(depth).unwrap_or(6), &title)
            }
            Block::Paragraph(inl) => {
                let text = self.inlines(inl);
                if text.is_empty() {
                    String::new()
                } else {
                    self.paragraph(&text)
                }
            }
            Block::List(list) => self.list(list),
            Block::Table(t) => self.table(t),
            Block::Math(m) => {
                let mut out = String::new();
                for row in &m.rows {
                    out.push_str(&self.display_math(&row.tex, row.number.as_deref()));
                }
                out
            }
            Block::Verbatim(v) => match self.mode {
                Mode::Text => format!("{}\n\n", v.text),
                Mode::Markdown => format!("```\n{}\n```\n\n", v.text.trim_end_matches('\n')),
                Mode::Html => format!("<pre><code>{}</code></pre>\n", escape_html(&v.text)),
            },
            Block::Figure(f) => self.figure(f),
            Block::Theorem(t) => {
                let mut head = t.name.clone();
                if let Some(n) = &t.number {
                    head = format!("{head} {n}");
                }
                if let Some(title) = &t.title {
                    let tt = self.inlines(title);
                    head = format!("{head} ({tt})");
                }
                let head = self.escape(&head);
                let head = self.strong(&format!("{head}."));
                let body = self.blocks(&t.body);
                format!("{head} {}", body.trim_start()).replace("\n\n\n", "\n\n")
            }
            Block::Quote(inner) => {
                let body = self.blocks(inner);
                match self.mode {
                    Mode::Html => format!("<blockquote>\n{body}</blockquote>\n"),
                    Mode::Markdown => {
                        body.trim_end()
                            .lines()
                            .map(|l| {
                                if l.is_empty() {
                                    ">".to_string()
                                } else {
                                    format!("> {l}")
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                            + "\n\n"
                    }
                    Mode::Text => body,
                }
            }
        }
    }

    fn list(&mut self, list: &super::List) -> String {
        let ordered = list.kind == ListKind::Enumerate;
        let mut out = String::new();
        if self.mode == Mode::Html {
            out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
        }
        for (i, item) in list.items.iter().enumerate() {
            let content = self.blocks(&item.blocks);
            let label = item.label.as_ref().map(|l| self.inlines(l));
            match self.mode {
                Mode::Html => {
                    let prefix = match &label {
                        Some(l) => format!("<strong>{l}</strong> "),
                        None => String::new(),
                    };
                    let _ = writeln!(out, "<li>{prefix}{}</li>", content.trim());
                }
                Mode::Markdown | Mode::Text => {
                    let marker = match (&label, ordered) {
                        (Some(l), _) if self.mode == Mode::Markdown => format!("- **{l}** "),
                        (Some(l), _) => format!("{l} "),
                        (None, true) => format!("{}. ", i + 1),
                        (None, false) => "- ".to_string(),
                    };
                    let indent = " ".repeat(marker.chars().count());
                    let body = content.trim_end();
                    let mut first = true;
                    for line in body.lines() {
                        if first {
                            out.push_str(&marker);
                            first = false;
                        } else if line.is_empty() {
                            out.push('\n');
                            continue;
                        } else {
                            out.push_str(&indent);
                        }
                        out.push_str(line);
                        out.push('\n');
                    }
                    if first {
                        out.push_str(marker.trim_end());
                        out.push('\n');
                    }
                }
            }
        }
        match self.mode {
            Mode::Html => {
                out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
            }
            _ => out.push('\n'),
        }
        out
    }

    fn figure(&mut self, f: &Figure) -> String {
        let mut out = String::new();
        // A figure built by hand may carry only the older `tikz` body.
        let legacy;
        let pictures: &[Picture] = if f.pictures.is_empty() {
            legacy = f
                .tikz
                .as_ref()
                .map(|body| Picture {
                    env: "tikzpicture".into(),
                    source: body.trim().to_string(),
                })
                .into_iter()
                .collect::<Vec<_>>();
            &legacy
        } else {
            &f.pictures
        };
        for picture in pictures {
            let what = match picture.env.as_str() {
                "tikzpicture" => "TikZ picture".to_string(),
                "picture" => "LaTeX picture".to_string(),
                other => format!("{other} picture"),
            };
            out.push_str(&match self.mode {
                Mode::Text => format!("[{what}, not rendered]\n\n"),
                Mode::Markdown => format!(
                    "*{}, not rendered.*\n\n{}",
                    self.escape(&what),
                    fenced("latex", &picture.source)
                ),
                Mode::Html => format!(
                    "<pre><code class=\"language-latex\">{}</code></pre>\n",
                    escape_html(&picture.source)
                ),
            });
        }
        let caption = match (&f.caption, &f.number) {
            (Some(c), Some(n)) => {
                let text = self.inlines(c);
                let label = match f.env.as_str() {
                    "subfigure" | "subfloat" => format!("({n})"),
                    _ => format!("Figure {n}:"),
                };
                Some(format!("{} {}", self.strong(&label), text))
            }
            (Some(c), None) => Some(self.inlines(c)),
            _ => None,
        };
        if let Some(c) = caption {
            out.push_str(&self.paragraph(&c));
        }
        for img in &f.image_paths {
            let line = self.image(img);
            out.push_str(&self.paragraph(&line));
        }
        for sub in &f.subfigures {
            out.push_str(&self.figure(sub));
        }
        out
    }

    fn table(&mut self, t: &Table) -> String {
        let mut out = String::new();
        if let Some(c) = &t.caption {
            let number = t.number.clone().unwrap_or_default();
            let text = self.inlines(c);
            let label = if number.is_empty() {
                String::new()
            } else {
                format!("Table {number}: ")
            };
            let line = format!(
                "{}{}",
                self.strong(label.trim_end()),
                if label.is_empty() {
                    text.clone()
                } else {
                    format!(" {text}")
                }
            );
            out.push_str(&self.paragraph(&line));
        }
        let width = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        if width == 0 {
            return out;
        }
        let mut grid: Vec<Vec<String>> = Vec::new();
        for row in &t.rows {
            let mut cells = Vec::new();
            for cell in &row.cells {
                let text = self.inlines(&cell.content).replace('\n', " ");
                cells.push(text);
            }
            while cells.len() < width {
                cells.push(String::new());
            }
            grid.push(cells);
        }
        match self.mode {
            Mode::Text => {
                for row in &grid {
                    let _ = writeln!(out, "{}", row.join("\t"));
                }
                out.push('\n');
            }
            Mode::Markdown => {
                for (i, row) in grid.iter().enumerate() {
                    let cells: Vec<String> = row.iter().map(|c| c.replace('|', "\\|")).collect();
                    let _ = writeln!(out, "| {} |", cells.join(" | "));
                    if i == 0 {
                        let sep = vec!["---"; width].join(" | ");
                        let _ = writeln!(out, "| {sep} |");
                    }
                }
                out.push('\n');
            }
            Mode::Html => {
                out.push_str("<table>\n");
                for row in &grid {
                    out.push_str("<tr>");
                    for cell in row {
                        let _ = write!(out, "<td>{cell}</td>");
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</table>\n");
            }
        }
        out
    }
}

/// A `\bibitem` label as shown in a citation. natbib writes `Short(Year)Long`, which is shown
/// as "Short, Year"; any other label is only decoded (`~`, accents).
fn bibitem_label(label: &str) -> String {
    let text = crate::tex_text::decode(label, crate::tex_text::Ligatures::Tex)
        .replace('\u{a0}', " ")
        .trim()
        .to_string();
    if let Some(open) = text.rfind('(') {
        if let Some(len) = text[open..].find(')') {
            let year = &text[open + 1..open + len];
            let short = text[..open].trim();
            let is_year = year.len() >= 4 && year.chars().take(4).all(|c| c.is_ascii_digit());
            if !short.is_empty() && is_year {
                return format!("{short}, {year}");
            }
        }
    }
    text
}

/// A fenced Markdown code block. The fence is longer than any run of backticks in the text.
fn fenced(language: &str, text: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest.max(2) + 1);
    format!(
        "{fence}{language}\n{}\n{fence}\n\n",
        text.trim_matches('\n').trim_end()
    )
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '$' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}
