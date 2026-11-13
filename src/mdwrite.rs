//! Writer for `ast::Doc`: Pandoc-flavoured Markdown.

use std::fmt::Write as _;

use crate::ast::{Align, Block, Cell, Doc, Inline, ListStyle, Meta, Table};

/// Pandoc-flavoured Markdown for a document: YAML front matter for the metadata, then the blocks.
pub fn to_markdown(doc: &Doc) -> String {
    let mut w = Writer::default();
    let mut out = String::new();
    if !doc.meta.is_empty() {
        out.push_str(&w.front_matter(&doc.meta));
    }
    out.push_str(&w.blocks(&doc.blocks));
    w.finish(out)
}

/// Markdown for blocks only (no front matter).
pub fn blocks_to_markdown(blocks: &[Block]) -> String {
    let mut w = Writer::default();
    let out = w.blocks(blocks);
    w.finish(out)
}

/// Markdown for inlines on one line. Footnotes inside are written as their text in brackets.
pub fn inlines_to_markdown(inlines: &[Inline]) -> String {
    Writer::default().inlines(inlines)
}

#[derive(Default)]
struct Writer {
    notes: Vec<String>,
}

impl Writer {
    fn finish(self, out: String) -> String {
        let mut out = out.trim_end().to_string();
        if !self.notes.is_empty() {
            out.push_str("\n\n");
            for (i, n) in self.notes.iter().enumerate() {
                let _ = write!(
                    out,
                    "[^{}]: {}\n\n",
                    i + 1,
                    indent_rest(n.trim_end(), "    ")
                );
            }
            out = out.trim_end().to_string();
        }
        out.push('\n');
        out
    }

    fn front_matter(&mut self, m: &Meta) -> String {
        let mut s = String::from("---\n");
        if !m.title.is_empty() {
            let t = self.inlines(&m.title).replace('\n', " ");
            let _ = writeln!(s, "title: {}", yaml_scalar(t.trim()));
        }
        match m.authors.len() {
            0 => {}
            1 => {
                let a = self.inlines(&m.authors[0]).replace('\n', " ");
                let _ = writeln!(s, "author: {}", yaml_scalar(a.trim()));
            }
            _ => {
                s.push_str("author:\n");
                for a in &m.authors {
                    let a = self.inlines(a).replace('\n', " ");
                    let _ = writeln!(s, "- {}", yaml_scalar(a.trim()));
                }
            }
        }
        if let Some(d) = &m.date {
            let _ = writeln!(s, "date: {}", yaml_scalar(d));
        }
        if !m.abstract_blocks.is_empty() {
            let body = self.blocks(&m.abstract_blocks);
            s.push_str("abstract: |\n");
            for line in body.trim_end().lines() {
                if line.is_empty() {
                    s.push('\n');
                } else {
                    let _ = writeln!(s, "  {line}");
                }
            }
        }
        s.push_str("---\n\n");
        s
    }

    fn blocks(&mut self, blocks: &[Block]) -> String {
        let mut out = String::new();
        let mut prev: Option<&Block> = None;
        for b in blocks {
            let piece = self.block(b);
            if piece.trim().is_empty() {
                continue;
            }
            // Two lists of the same kind in a row would be read back as one list; an empty
            // comment keeps them apart, as Pandoc does. A code block after a list needs the same.
            let is_list = |b: &Block| matches!(b, Block::BulletList(_) | Block::OrderedList { .. });
            if let Some(p) = prev {
                if (is_list(p) && is_list(b))
                    || (is_list(p) && matches!(b, Block::Code { lang: None, .. }))
                {
                    out.push_str("<!-- -->\n\n");
                }
            }
            out.push_str(piece.trim_end_matches('\n'));
            out.push_str("\n\n");
            prev = Some(b);
        }
        out
    }

    fn block(&mut self, b: &Block) -> String {
        match b {
            Block::Para(i) | Block::Plain(i) => {
                if let [Inline::Math { display: true, tex }] = i.as_slice() {
                    return format!("$$\n{}\n$$", tex.trim());
                }
                let t = self.inlines(i);
                let t = t.trim().to_string();
                escape_block_start(&t)
            }
            Block::Heading { level, inlines, .. } => {
                let t = self.inlines(inlines).replace('\n', " ");
                let t = t.trim();
                if t.is_empty() {
                    return String::new();
                }
                format!("{} {}", "#".repeat(usize::from((*level).clamp(1, 6))), t)
            }
            Block::Code { lang, text } => {
                let fence = fence_for(text);
                let body = text.trim_end_matches('\n');
                match lang {
                    Some(l) if !l.is_empty() => format!("{fence}{l}\n{body}\n{fence}"),
                    _ => format!("{fence}\n{body}\n{fence}"),
                }
            }
            Block::Quote(inner) => {
                let body = self.blocks(inner);
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
            }
            Block::BulletList(items) => {
                self.list(items.iter().map(|i| ("-   ".to_string(), i)).collect())
            }
            Block::OrderedList {
                start,
                style,
                items,
            } => {
                let marks = items
                    .iter()
                    .enumerate()
                    .map(|(n, it)| {
                        let num = start + u32::try_from(n).unwrap_or(0);
                        let label = match style {
                            ListStyle::Decimal => num.to_string(),
                            ListStyle::LowerAlpha => alpha(num, false),
                            ListStyle::UpperAlpha => alpha(num, true),
                            ListStyle::LowerRoman => roman(num, false),
                            ListStyle::UpperRoman => roman(num, true),
                        };
                        (format!("{:<4}", format!("{label}.")), it)
                    })
                    .collect();
                self.list(marks)
            }
            Block::DefList(items) => {
                let mut out = String::new();
                for (term, defs) in items {
                    let _ = writeln!(out, "{}", self.inlines(term).replace('\n', " ").trim());
                    for d in defs {
                        let body = self.blocks(d);
                        let mut first = true;
                        for line in body.trim_end().lines() {
                            if first {
                                let _ = writeln!(out, ":   {line}");
                                first = false;
                            } else if line.is_empty() {
                                out.push('\n');
                            } else {
                                let _ = writeln!(out, "    {line}");
                            }
                        }
                    }
                    out.push('\n');
                }
                out
            }
            Block::Table(t) => self.table(t),
            Block::Figure { url, caption } => {
                format!(
                    "![{}]({})",
                    self.inlines(caption).replace('\n', " ").trim(),
                    escape_url(url)
                )
            }
            Block::Rule => "---".to_string(),
            Block::Raw { format, text } => match format.as_str() {
                "html" => text.trim().to_string(),
                other => format!("```{{={other}}}\n{}\n```", text.trim_end()),
            },
            Block::Div { classes, blocks } => {
                let body = self.blocks(blocks);
                let cls: Vec<String> = classes.iter().map(|c| format!(".{c}")).collect();
                format!("::: {{{}}}\n{}\n:::", cls.join(" "), body.trim_end())
            }
        }
    }

    fn list(&mut self, items: Vec<(String, &Vec<Block>)>) -> String {
        let tight = items
            .iter()
            .all(|(_, it)| !it.iter().any(|b| matches!(b, Block::Para(_))));
        let mut out = String::new();
        for (mark, item) in items {
            let body = if tight {
                let mut s = String::new();
                for b in item.iter() {
                    let piece = self.block(b);
                    if piece.trim().is_empty() {
                        continue;
                    }
                    s.push_str(piece.trim_end_matches('\n'));
                    s.push('\n');
                }
                s
            } else {
                self.blocks(item)
            };
            let body = body.trim_end();
            if body.is_empty() {
                out.push_str(mark.trim_end());
                out.push('\n');
            } else {
                let _ = writeln!(out, "{}{}", mark, indent_rest(body, "    "));
            }
            if !tight {
                out.push('\n');
            }
        }
        out
    }

    fn table(&mut self, t: &Table) -> String {
        let ncols = t
            .head
            .iter()
            .chain(t.rows.iter())
            .map(|r| r.iter().map(|c| c.colspan.max(1)).sum::<usize>())
            .max()
            .unwrap_or(0);
        if ncols == 0 {
            return String::new();
        }
        let mut all: Vec<&Vec<Cell>> = t.head.iter().collect();
        all.extend(t.rows.iter());
        let grid = self.grid(&all, ncols);
        if grid.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for (ri, row) in grid.iter().enumerate() {
            let _ = writeln!(out, "| {} |", row.join(" | "));
            if ri == 0 {
                let seps: Vec<&str> = (0..ncols)
                    .map(|c| match t.align.get(c).copied().unwrap_or_default() {
                        Align::Left => ":---",
                        Align::Center => ":---:",
                        Align::Right => "---:",
                        Align::Default => "---",
                    })
                    .collect();
                let _ = writeln!(out, "|{}|", seps.join("|"));
            }
        }
        let cap = self.inlines(&t.caption).replace('\n', " ");
        if !cap.trim().is_empty() {
            let _ = write!(out, "\n: {}", cap.trim());
        }
        out
    }

    /// Cell texts laid out on a grid: a cell with a column span is followed by empty cells and
    /// one with a row span leaves empty cells in the rows below.
    fn grid(&mut self, rows: &[&Vec<Cell>], ncols: usize) -> Vec<Vec<String>> {
        let mut carry = vec![0usize; ncols];
        let mut grid = Vec::new();
        for row in rows {
            let mut out: Vec<String> = Vec::new();
            let mut cells = row.iter();
            let mut col = 0;
            while col < ncols {
                if carry[col] > 0 {
                    carry[col] -= 1;
                    out.push(String::new());
                    col += 1;
                    continue;
                }
                let Some(cell) = cells.next() else {
                    out.push(String::new());
                    col += 1;
                    continue;
                };
                let text = self.cell_text(cell);
                let span = cell.colspan.max(1).min(ncols - col);
                for k in 0..span {
                    out.push(if k == 0 { text.clone() } else { String::new() });
                    if cell.rowspan > 1 {
                        carry[col + k] = cell.rowspan - 1;
                    }
                }
                col += span;
            }
            grid.push(out);
        }
        grid
    }

    fn cell_text(&mut self, cell: &Cell) -> String {
        let mut parts = Vec::new();
        for b in &cell.blocks {
            let s = match b {
                Block::Para(i) | Block::Plain(i) => self.inlines(i),
                other => {
                    let s = self.block(other);
                    s.replace('\n', " ")
                }
            };
            let s = s.replace('\n', " ");
            if !s.trim().is_empty() {
                parts.push(s.trim().to_string());
            }
        }
        parts.join("<br>").replace('|', "\\|")
    }

    fn inlines(&mut self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for (i, inl) in inlines.iter().enumerate() {
            let prev_char = out.chars().last();
            let next = inlines.get(i + 1);
            match inl {
                Inline::Text(t) => {
                    let next_char = match next {
                        Some(Inline::Text(n)) => n.chars().next(),
                        _ => None,
                    };
                    out.push_str(&escape_text(t, prev_char, next_char));
                }
                Inline::Emph(v) => out.push_str(&self.wrap("*", v)),
                Inline::Strong(v) => out.push_str(&self.wrap("**", v)),
                Inline::Strike(v) => out.push_str(&self.wrap("~~", v)),
                Inline::Sup(v) => out.push_str(&self.script("^", v)),
                Inline::Sub(v) => out.push_str(&self.script("~", v)),
                Inline::SmallCaps(v) => {
                    let s = self.inlines(v);
                    if !s.trim().is_empty() {
                        let _ = write!(out, "[{}]{{.smallcaps}}", s.trim());
                    }
                }
                Inline::Underline(v) => {
                    let s = self.inlines(v);
                    if !s.trim().is_empty() {
                        let _ = write!(out, "[{}]{{.underline}}", s.trim());
                    }
                }
                Inline::Code(c) => out.push_str(&code_span(c)),
                Inline::Math { display, tex } => {
                    let tex = tex.trim();
                    if tex.is_empty() {
                        continue;
                    }
                    if *display {
                        let _ = write!(out, "$${tex}$$");
                    } else {
                        let _ = write!(out, "${tex}$");
                    }
                }
                Inline::Link {
                    url,
                    title,
                    inlines,
                } => {
                    let text = self.inlines(inlines).replace('\n', " ");
                    let text = text.trim();
                    let plain = crate::ast::inlines_text(inlines);
                    if text.is_empty()
                        || (plain.trim() == url && !url.contains(' ') && url.contains(':'))
                    {
                        let _ = write!(out, "<{url}>");
                    } else {
                        // `[^1^](..)` reads as a footnote reference; a space keeps it a link.
                        let lead = if text.starts_with('^') { " " } else { "" };
                        let _ = write!(
                            out,
                            "[{lead}{}]({}{})",
                            text,
                            escape_url(url),
                            title_suffix(title)
                        );
                    }
                }
                Inline::Image { url, alt, title } => {
                    let a = self.inlines(alt).replace('\n', " ");
                    let _ = write!(
                        out,
                        "![{}]({}{})",
                        a.trim(),
                        escape_url(url),
                        title_suffix(title)
                    );
                }
                Inline::Note(blocks) => {
                    let body = self.blocks(blocks);
                    self.notes.push(body);
                    let _ = write!(out, "[^{}]", self.notes.len());
                }
                Inline::LineBreak => out.push_str("\\\n"),
                Inline::SoftBreak => out.push('\n'),
                Inline::Cite(s) | Inline::Verbatim(s) => out.push_str(s),
            }
        }
        out
    }

    fn wrap(&mut self, mark: &str, v: &[Inline]) -> String {
        let s = self.inlines(v);
        wrap_marks(mark, &s)
    }

    fn script(&mut self, mark: &str, v: &[Inline]) -> String {
        let s = self.inlines(v);
        let t = s.trim();
        if t.is_empty() {
            return String::new();
        }
        // Pandoc scripts cannot contain spaces unescaped.
        format!("{mark}{}{mark}", t.replace(' ', "\\ "))
    }
}

/// `*text*` with the whitespace at either end moved outside the marks.
fn wrap_marks(mark: &str, s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return s.to_string();
    }
    let lead = &s[..s.len() - s.trim_start().len()];
    let trail = &s[s.trim_end().len()..];
    format!("{lead}{mark}{t}{mark}{trail}")
}

fn code_span(c: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for ch in c.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let ticks = "`".repeat(longest + 1);
    let c = c.replace('\n', " ");
    if longest > 0 || c.starts_with('`') || c.ends_with('`') {
        format!("{ticks} {c} {ticks}")
    } else {
        format!("{ticks}{c}{ticks}")
    }
}

fn fence_for(text: &str) -> String {
    let mut longest = 0;
    for line in text.lines() {
        let t = line.trim_start();
        let n = t.chars().take_while(|&c| c == '`').count();
        longest = longest.max(n);
    }
    "`".repeat((longest + 1).max(3))
}

fn title_suffix(title: &Option<String>) -> String {
    match title {
        Some(t) if !t.is_empty() => format!(" \"{}\"", t.replace('"', "\\\"")),
        _ => String::new(),
    }
}

fn escape_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '\n' | '\r' | '\t' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Escapes characters that Markdown would read as markup. `_` between two letters or digits is
/// left alone, as Pandoc does.
pub(crate) fn escape_text(t: &str, prev: Option<char>, next: Option<char>) -> String {
    let chars: Vec<char> = t.chars().collect();
    let mut out = String::with_capacity(t.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        let before = if i == 0 { prev } else { Some(chars[i - 1]) };
        let after = chars
            .get(i + 1)
            .copied()
            .or(if i + 1 == chars.len() { next } else { None });
        match c {
            '\\' | '*' | '`' | '<' | '>' | '[' | ']' | '$' | '~' | '^' | '|' => {
                out.push('\\');
                out.push(c);
            }
            '_' => {
                let inner = before.is_some_and(|b| b.is_alphanumeric())
                    && after.is_some_and(|a| a.is_alphanumeric());
                if !inner {
                    out.push('\\');
                }
                out.push('_');
            }
            '@' => {
                let cite = after.is_some_and(|a| a.is_alphanumeric() || a == '_' || a == '{')
                    && !before.is_some_and(|b| b.is_alphanumeric());
                if cite {
                    out.push('\\');
                }
                out.push('@');
            }
            '\u{a0}' => out.push_str("\\ "),
            _ => out.push(c),
        }
    }
    out
}

/// A paragraph that starts like a block (a heading, list item, rule) gets its first character
/// escaped so that it is read back as a paragraph.
fn escape_block_start(t: &str) -> String {
    let b = t.as_bytes();
    let first_is = |s: &str| t.starts_with(s);
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    let list_num = digits > 0
        && matches!(b.get(digits), Some(b'.') | Some(b')'))
        && matches!(b.get(digits + 1), Some(b' ') | None);
    let marker = t
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '#' | '+' | '-' | '=' | ':' | '|'))
        && (first_is("#")
            || first_is("+ ")
            || first_is("- ")
            || first_is("---")
            || first_is("===")
            || first_is(":::")
            || first_is("|")
            || first_is("-\n")
            || t == "-"
            || t == "+");
    if list_num {
        format!("{}\\{}", &t[..digits], &t[digits..])
    } else if marker {
        format!("\\{t}")
    } else {
        t.to_string()
    }
}

fn indent_rest(s: &str, pad: &str) -> String {
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(pad);
            }
        }
        out.push_str(line);
    }
    out
}

fn yaml_scalar(s: &str) -> String {
    let simple = !s.is_empty()
        && s.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, ' ' | '.' | ',' | '-' | '\'' | '(' | ')' | '/')
        })
        && !s.starts_with(['-', ' ', '\''])
        && !s.ends_with(' ')
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off"
        )
        && s.parse::<f64>().is_err();
    if simple {
        s.to_string()
    } else {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

fn alpha(mut n: u32, upper: bool) -> String {
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        let c = u8::try_from(n % 26).unwrap_or(0) + if upper { b'A' } else { b'a' };
        s.push(c as char);
        n /= 26;
    }
    s.iter().rev().collect()
}

fn roman(mut n: u32, upper: bool) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper {
        s.to_uppercase()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<Inline> {
        vec![Inline::Text(s.into())]
    }

    #[test]
    fn writes_lists_tables_and_notes() {
        let doc = Doc {
            meta: Meta {
                title: t("A: title"),
                ..Default::default()
            },
            blocks: vec![
                Block::Heading {
                    level: 1,
                    inlines: t("Head"),
                    id: None,
                },
                Block::BulletList(vec![
                    vec![
                        Block::Plain(t("one")),
                        Block::OrderedList {
                            start: 1,
                            style: ListStyle::Decimal,
                            items: vec![vec![Block::Plain(t("a"))]],
                        },
                    ],
                    vec![Block::Plain(t("two"))],
                ]),
                Block::Para(vec![
                    Inline::Text("x".into()),
                    Inline::Note(vec![Block::Para(t("note"))]),
                ]),
                Block::Table(Table {
                    caption: t("Cap"),
                    align: vec![Align::Left, Align::Right],
                    head: vec![vec![Cell::text(t("A")), Cell::text(t("B"))]],
                    rows: vec![vec![Cell::text(t("1")), Cell::text(t("2"))]],
                }),
            ],
        };
        let md = to_markdown(&doc);
        assert!(md.starts_with("---\ntitle: \"A: title\"\n---\n"), "{md}");
        assert!(md.contains("-   one\n    1.  a\n-   two\n"), "{md}");
        assert!(md.contains("x[^1]"), "{md}");
        assert!(
            md.contains("| A | B |\n|:---|---:|\n| 1 | 2 |\n\n: Cap"),
            "{md}"
        );
        assert!(md.ends_with("[^1]: note\n"), "{md}");
    }

    #[test]
    fn escapes_markup_and_keeps_snake_case() {
        assert_eq!(
            escape_text("a*b_c snake_case [x]", None, None),
            "a\\*b_c snake_case \\[x\\]"
        );
        assert_eq!(escape_block_start("1. not a list"), "1\\. not a list");
        assert_eq!(escape_block_start("# not a heading"), "\\# not a heading");
    }

    #[test]
    fn emphasis_moves_spaces_outside() {
        assert_eq!(wrap_marks("*", " hi "), " *hi* ");
        assert_eq!(code_span("a`b"), "`` a`b ``");
    }
}
