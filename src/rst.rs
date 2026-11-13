//! reStructuredText reader: text to the shared tree in `ast`, written as Pandoc-flavoured
//! Markdown by `mdwrite`. It reads what docutils and Pandoc read: sections (levels by order of
//! first use), paragraphs, bullet, enumerated, definition, field and line lists, block quotes,
//! literal blocks, grid and simple tables, the common directives (code, math, admonitions,
//! image, figure, raw, contents, toctree), explicit targets, substitutions, footnotes and
//! citations, and inline markup with roles and hyperlink references.
//!
//! Not read: the `include` directive (no file access), option lists, doctest blocks and
//! directives that need Sphinx (`only`, `ifconfig`); an unknown directive keeps its content
//! as a fenced div named after it, as Pandoc does.

use std::collections::HashMap;

use crate::ast::{self, Align, Block, Cell, Doc, Inline, ListStyle, Meta, Table};

/// Reads reStructuredText into a document tree.
pub fn to_doc(text: &str) -> Doc {
    let lines: Vec<String> = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|l| expand_tabs(l.trim_end()))
        .collect();
    let mut r = Reader::default();
    r.collect_definitions(&lines);
    let blocks = r.blocks(&lines, 0);
    Doc {
        meta: Meta::default(),
        blocks,
    }
}

/// Reads reStructuredText bytes and writes Pandoc-flavoured Markdown.
pub fn to_markdown(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    ast::to_markdown(&to_doc(text.trim_start_matches('\u{feff}')))
}

fn expand_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let mut out = String::new();
    for c in line.chars() {
        if c == '\t' {
            let n = 8 - out.chars().count() % 8;
            out.push_str(&" ".repeat(n));
        } else {
            out.push(c);
        }
    }
    out
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn normalize_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Pandoc's identifier for a heading: lower case, spaces to hyphens, only letters, digits and
/// `_-.` kept, starting at the first letter.
fn identifier(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            out.extend(c.to_lowercase());
            prev_dash = false;
        } else if (c == '-' || c.is_whitespace()) && !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    out.trim_start_matches(|c: char| !c.is_alphabetic())
        .to_string()
}

fn dedent(lines: &[String]) -> Vec<String> {
    let min = lines
        .iter()
        .filter(|l| !is_blank(l))
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.len() >= min {
                l[min..].to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

#[derive(Default)]
struct Reader {
    /// Normalised target name -> URL (external) or `#id` (internal).
    targets: HashMap<String, String>,
    subs: HashMap<String, Vec<Inline>>,
    /// Footnote/citation definitions: label -> raw content lines.
    notes: HashMap<String, Vec<String>>,
    /// Auto-numbered definitions in order, and how many were used.
    auto_notes: Vec<Vec<String>>,
    auto_used: usize,
    symbol_used: usize,
    /// Section adornments in order of first use: (character, has overline).
    adornments: Vec<(char, bool)>,
    /// Implicit section targets: normalised title -> id.
    sections: HashMap<String, String>,
    anonymous: Vec<String>,
    anon_used: usize,
    pending_target: Option<String>,
    depth: usize,
}

struct Directive {
    name: String,
    arg: String,
    options: Vec<(String, String)>,
    content: Vec<String>,
}

impl Reader {
    // ---- first pass: targets, substitutions, notes ------------------------------------------

    fn collect_definitions(&mut self, lines: &[String]) {
        let mut i = 0;
        // Implicit section targets: a title line followed by an adornment.
        while i < lines.len() {
            let line = &lines[i];
            if indent_of(line) == 0 && line.starts_with("__ ") {
                self.anonymous.push(line[3..].trim().to_string());
                i += 1;
                continue;
            }
            if indent_of(line) == 0 && line.starts_with(".. ") || line == ".." {
                let (end, _) = explicit_extent(lines, i);
                let first = line[2..].trim().to_string();
                let body: Vec<String> = lines[i + 1..end].to_vec();
                if let Some(rest) = first.strip_prefix("__:") {
                    let url = join_target(rest, &body);
                    self.anonymous.push(url);
                } else if let Some(rest) = first.strip_prefix('_') {
                    if let Some((name, url)) = split_target(rest) {
                        let url = join_target(url, &body);
                        let name = name.trim().trim_matches('`').replace("\\:", ":");
                        if url.is_empty() {
                            // An internal target: resolved to the id of what follows it.
                            self.targets
                                .entry(normalize_name(&name))
                                .or_insert_with(|| format!("#{}", identifier(&name)));
                        } else {
                            self.targets.insert(normalize_name(&name), target_url(&url));
                        }
                    }
                } else if let Some(rest) = first.strip_prefix('|') {
                    if let Some(close) = rest.find('|') {
                        let name = rest[..close].to_string();
                        let after = rest[close + 1..].trim();
                        if let Some(d) = after.strip_prefix("replace::") {
                            let mut text = d.trim().to_string();
                            for b in &body {
                                text.push(' ');
                                text.push_str(b.trim());
                            }
                            let inl = self.inlines(text.trim());
                            self.subs.insert(name, inl);
                        } else if let Some(d) = after.strip_prefix("image::") {
                            let alt = body
                                .iter()
                                .find_map(|l| {
                                    l.trim().strip_prefix(":alt:").map(|v| v.trim().to_string())
                                })
                                .unwrap_or_default();
                            self.subs.insert(
                                name,
                                vec![Inline::Image {
                                    url: d.trim().to_string(),
                                    alt: vec![Inline::Text(alt)],
                                    title: None,
                                }],
                            );
                        } else if let Some(d) = after.strip_prefix("unicode::") {
                            self.subs
                                .insert(name, vec![Inline::Text(unicode_directive(d))]);
                        }
                    }
                } else if first.starts_with('[') {
                    if let Some(close) = first.find(']') {
                        let label = first[1..close].to_string();
                        let mut content = vec![first[close + 1..].trim().to_string()];
                        content.extend(dedent(&body));
                        match label.as_str() {
                            "#" | "*" => self.auto_notes.push(content),
                            _ => {
                                self.notes
                                    .insert(label.trim_start_matches('#').to_string(), content);
                            }
                        }
                    }
                }
                i = end.max(i + 1);
                continue;
            }
            // Section title (underline only): remember its implicit target.
            if i + 1 < lines.len()
                && !is_blank(line)
                && indent_of(line) == 0
                && is_adornment(&lines[i + 1], line.trim().chars().count())
            {
                let t = line.trim();
                if !is_adornment(line, 1) {
                    self.sections
                        .entry(normalize_name(t))
                        .or_insert_with(|| identifier(t));
                }
            }
            i += 1;
        }
    }

    // ---- blocks -----------------------------------------------------------------------------

    fn blocks(&mut self, lines: &[String], base: usize) -> Vec<Block> {
        self.depth += 1;
        let out = if self.depth > 40 {
            Vec::new()
        } else {
            self.blocks_inner(lines, base)
        };
        self.depth -= 1;
        out
    }

    fn blocks_inner(&mut self, lines: &[String], _base: usize) -> Vec<Block> {
        let mut out: Vec<Block> = Vec::new();
        let mut i = 0;
        let n = lines.len();
        while i < n {
            let line = &lines[i];
            if is_blank(line) {
                i += 1;
                continue;
            }
            let ind = indent_of(line);
            // Block quote: an indented region.
            if ind > 0 {
                let mut j = i;
                while j < n && (is_blank(&lines[j]) || indent_of(&lines[j]) > 0) {
                    j += 1;
                }
                let inner = dedent(&lines[i..j]);
                // An attribution line `-- name` stays inside the quote.
                let blocks = self.blocks(&inner, 0);
                out.push(Block::Quote(blocks));
                i = j;
                continue;
            }
            let text = line.as_str();
            // Section with overline.
            if i + 2 < n
                && is_adornment(text, 3)
                && !is_blank(&lines[i + 1])
                && is_adornment(&lines[i + 2], 1)
                && lines[i + 2].chars().next() == text.chars().next()
            {
                let title = lines[i + 1].trim();
                let ch = text.chars().next().unwrap_or('=');
                self.push_section(&mut out, ch, true, title);
                i += 3;
                continue;
            }
            // Section with underline.
            if i + 1 < n
                && !is_adornment(text, 1)
                && is_adornment(&lines[i + 1], text.trim().chars().count().max(1))
                && indent_of(&lines[i + 1]) == 0
            {
                let ch = lines[i + 1].chars().next().unwrap_or('=');
                self.push_section(&mut out, ch, false, text.trim());
                i += 2;
                continue;
            }
            // Transition.
            if is_adornment(text, 4) {
                out.push(Block::Rule);
                i += 1;
                continue;
            }
            if text.starts_with("__ ") {
                i += 1;
                continue;
            }
            // Explicit markup.
            if text == ".." || text.starts_with(".. ") {
                let (end, _) = explicit_extent(lines, i);
                self.explicit(&lines[i..end], &mut out);
                i = end.max(i + 1);
                continue;
            }
            // Tables.
            if text.starts_with("+-") || text.starts_with("+=") {
                let mut j = i;
                while j < n && (lines[j].starts_with('+') || lines[j].starts_with('|')) {
                    j += 1;
                }
                if let Some(t) = self.grid_table(&lines[i..j]) {
                    out.push(Block::Table(t));
                    i = j;
                    continue;
                }
            }
            if is_simple_border(text) {
                if let Some((t, end)) = self.simple_table(lines, i) {
                    out.push(Block::Table(t));
                    i = end;
                    continue;
                }
            }
            // Bullet list.
            if let Some(off) = bullet_offset(text) {
                let (items, end) = self.list_items(lines, i, bullet_offset);
                let _ = off;
                out.push(Block::BulletList(items));
                i = end;
                continue;
            }
            // Enumerated list.
            if let Some(e) = enum_marker(text) {
                let nxt = lines.get(i + 1).map(String::as_str).unwrap_or("");
                // A lone "A." paragraph is only a list when its sequence continues.
                if e.sure || is_blank(nxt) || indent_of(nxt) > 0 || enum_marker(nxt).is_some() {
                    let (items, end) =
                        self.list_items(lines, i, |l| enum_marker(l).map(|m| m.offset));
                    out.push(Block::OrderedList {
                        start: e.start,
                        style: e.style,
                        items,
                    });
                    i = end;
                    continue;
                }
            }
            // Field list.
            if field_name(text).is_some() {
                let mut items = Vec::new();
                let mut j = i;
                while j < n && !is_blank(&lines[j])
                    || (j < n
                        && j + 1 < n
                        && field_name(&lines[j + 1]).is_some()
                        && is_blank(&lines[j]))
                {
                    if is_blank(&lines[j]) {
                        j += 1;
                        continue;
                    }
                    let Some((name, rest)) = field_name(&lines[j]) else {
                        break;
                    };
                    let mut body = vec![rest.trim().to_string()];
                    let mut k = j + 1;
                    while k < n
                        && (is_blank(&lines[k]) && k + 1 < n && indent_of(&lines[k + 1]) > 0
                            || indent_of(&lines[k]) > 0 && !is_blank(&lines[k]))
                    {
                        body.push(lines[k].clone());
                        k += 1;
                    }
                    let first = body.remove(0);
                    let mut all = vec![first];
                    all.extend(dedent(&body));
                    let blocks = self.blocks(&all, 0);
                    items.push((self.inlines(&name), vec![blocks]));
                    j = k;
                }
                out.push(Block::DefList(items));
                i = j;
                continue;
            }
            // Line block.
            if text == "|" || text.starts_with("| ") {
                let mut j = i;
                let mut rows: Vec<String> = Vec::new();
                while j < n
                    && (lines[j] == "|"
                        || lines[j].starts_with("| ")
                        || (indent_of(&lines[j]) > 0 && !is_blank(&lines[j]) && !rows.is_empty()))
                {
                    if lines[j] == "|" {
                        rows.push(String::new());
                    } else if let Some(r) = lines[j].strip_prefix("| ") {
                        rows.push(format!("| {}", r.trim_end()));
                    } else {
                        let last = rows.len() - 1;
                        rows[last].push(' ');
                        rows[last].push_str(lines[j].trim());
                    }
                    j += 1;
                }
                let rows: Vec<String> = rows
                    .into_iter()
                    .map(|r| if r.is_empty() { "|".into() } else { r })
                    .collect();
                out.push(Block::Para(vec![Inline::Verbatim(rows.join("\n"))]));
                i = j;
                continue;
            }
            // Definition list: a line followed by an indented line.
            if i + 1 < n
                && indent_of(&lines[i + 1]) > 0
                && !is_blank(&lines[i + 1])
                && !text.ends_with("::")
            {
                let mut items = Vec::new();
                let mut j = i;
                while j + 1 < n
                    && !is_blank(&lines[j])
                    && indent_of(&lines[j]) == 0
                    && indent_of(&lines[j + 1]) > 0
                    && !is_blank(&lines[j + 1])
                {
                    let term_line = lines[j].trim();
                    let term = match term_line.find(" : ") {
                        Some(p) => &term_line[..p],
                        None => term_line,
                    };
                    let mut k = j + 1;
                    while k < n
                        && (indent_of(&lines[k]) > 0
                            || (is_blank(&lines[k]) && k + 1 < n && indent_of(&lines[k + 1]) > 0))
                    {
                        k += 1;
                    }
                    let body = dedent(&lines[j + 1..k]);
                    let blocks = self.blocks(&body, 0);
                    items.push((self.inlines(term), vec![blocks]));
                    j = k;
                    while j < n && is_blank(&lines[j]) {
                        j += 1;
                    }
                    if j >= n
                        || indent_of(&lines[j]) != 0
                        || j + 1 >= n
                        || indent_of(&lines[j + 1]) == 0
                        || is_blank(&lines[j + 1])
                    {
                        break;
                    }
                }
                out.push(Block::DefList(items));
                i = j;
                continue;
            }
            // Paragraph (and a literal block after `::`).
            let mut j = i;
            let mut para: Vec<&str> = Vec::new();
            while j < n && !is_blank(&lines[j]) {
                para.push(lines[j].trim());
                j += 1;
            }
            let mut text = para.join("\n");
            let mut literal = false;
            if text == "::" {
                text.clear();
                literal = true;
            } else if let Some(t) = text
                .strip_suffix(" ::")
                .or_else(|| text.strip_suffix("\n::"))
            {
                text = t.to_string();
                literal = true;
            } else if let Some(t) = text.strip_suffix("::") {
                text = format!("{t}:");
                literal = true;
            }
            if !text.is_empty() {
                let inl = self.inlines(&text);
                out.push(Block::Para(inl));
            }
            i = j;
            if literal {
                let mut k = i;
                while k < n && is_blank(&lines[k]) {
                    k += 1;
                }
                if k < n && indent_of(&lines[k]) > 0 {
                    let mut e = k;
                    while e < n && (is_blank(&lines[e]) || indent_of(&lines[e]) > 0) {
                        e += 1;
                    }
                    let mut body = dedent(&lines[k..e]);
                    while body.last().is_some_and(|l| l.trim().is_empty()) {
                        body.pop();
                    }
                    out.push(Block::Code {
                        lang: None,
                        text: body.join("\n"),
                    });
                    i = e;
                } else if k < n && is_quoted_literal(&lines[k]) {
                    let mut e = k;
                    while e < n && !is_blank(&lines[e]) {
                        e += 1;
                    }
                    out.push(Block::Code {
                        lang: None,
                        text: lines[k..e].join("\n"),
                    });
                    i = e;
                }
            }
        }
        tighten(&mut out);
        out
    }

    fn push_section(&mut self, out: &mut Vec<Block>, ch: char, over: bool, title: &str) {
        let level = match self.adornments.iter().position(|a| *a == (ch, over)) {
            Some(p) => p + 1,
            None => {
                self.adornments.push((ch, over));
                self.adornments.len()
            }
        };
        let id = self.pending_target.take();
        let inlines = self.inlines(title);
        out.push(Block::Heading {
            level: u8::try_from(level.min(6)).unwrap_or(6),
            inlines,
            id,
        });
    }

    /// Items of a bullet or enumerated list starting at `start`; `marker` gives the content
    /// offset of a list line.
    fn list_items<F: Fn(&str) -> Option<usize>>(
        &mut self,
        lines: &[String],
        start: usize,
        marker: F,
    ) -> (Vec<Vec<Block>>, usize) {
        let n = lines.len();
        let mut items = Vec::new();
        let mut i = start;
        let mut tight = true;
        while i < n {
            let Some(off) = marker(&lines[i]) else { break };
            let mut body = vec![lines[i][off.min(lines[i].len())..].to_string()];
            let mut j = i + 1;
            while j < n {
                if is_blank(&lines[j]) {
                    // Blank lines continue the item only when what follows is indented to it.
                    let mut k = j;
                    while k < n && is_blank(&lines[k]) {
                        k += 1;
                    }
                    if k < n && indent_of(&lines[k]) >= off {
                        for _ in j..k {
                            body.push(String::new());
                        }
                        j = k;
                        tight = false;
                        continue;
                    }
                    break;
                }
                if indent_of(&lines[j]) >= off {
                    body.push(lines[j][off..].to_string());
                    j += 1;
                } else if indent_of(&lines[j]) > 0 && marker(&lines[j]).is_none() {
                    body.push(lines[j].trim_start().to_string());
                    j += 1;
                } else {
                    break;
                }
            }
            let mut blocks = self.blocks(&body, 0);
            if tight || blocks.len() == 1 {
                for b in blocks.iter_mut().take(1) {
                    if let Block::Para(inl) = b {
                        *b = Block::Plain(std::mem::take(inl));
                    }
                }
            }
            items.push(blocks);
            // Skip one blank line between items; stop at anything that is not another item.
            let mut k = j;
            while k < n && is_blank(&lines[k]) {
                k += 1;
            }
            if k > j {
                tight = false;
            }
            if k < n && indent_of(&lines[k]) == 0 && marker(&lines[k]).is_some() {
                i = k;
            } else {
                i = j;
                break;
            }
        }
        if !tight {
            for item in &mut items {
                for b in item.iter_mut() {
                    if let Block::Plain(inl) = b {
                        *b = Block::Para(std::mem::take(inl));
                    }
                }
            }
        }
        (items, i)
    }

    // ---- explicit markup ----------------------------------------------------------------------

    fn explicit(&mut self, lines: &[String], out: &mut Vec<Block>) {
        let first = lines[0][2..].trim_start().to_string();
        let body: Vec<String> = lines[1..].to_vec();
        // Targets, substitutions and footnotes were collected in the first pass.
        if first.starts_with('_')
            || first.starts_with('|') && first.contains("|") && !first.starts_with("|`")
        {
            if let Some(rest) = first.strip_prefix('_') {
                if let Some((name, url)) = split_target(rest) {
                    if join_target(url, &body).is_empty() {
                        self.pending_target = Some(identifier(name.trim().trim_matches('`')));
                    }
                }
            }
            return;
        }
        if first.starts_with('[') && first.contains(']') {
            if let Some(close) = first.find(']') {
                let label = &first[1..close];
                if label.chars().all(|c| {
                    c.is_ascii_digit()
                        || c == '#'
                        || c == '*'
                        || c.is_alphanumeric()
                        || c == '-'
                        || c == '_'
                        || c == '.'
                }) && !label
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '#' || c == '*')
                    && !label.starts_with('#')
                {
                    // A citation definition: a definition-list entry in a `citations` div.
                    let mut content = vec![first[close + 1..].trim().to_string()];
                    content.extend(dedent(&body));
                    let blocks = self.blocks(&content, 0);
                    out.push(Block::Div {
                        classes: vec!["citations".into()],
                        blocks: vec![Block::DefList(vec![(
                            vec![Inline::Verbatim(format!(
                                "[{label}]{{#{label} .citation-label}}"
                            ))],
                            vec![blocks],
                        )])],
                    });
                }
            }
            return;
        }
        let Some(pos) = first.find("::") else { return };
        let name = first[..pos].trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
        {
            return; // a comment
        }
        let arg = first[pos + 2..].trim().to_string();
        let d = parse_directive(name, arg, &body);
        self.directive(d, out);
    }

    fn directive(&mut self, d: Directive, out: &mut Vec<Block>) {
        let opt = |k: &str| {
            d.options
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        match d.name.to_lowercase().as_str() {
            "code" | "code-block" | "sourcecode" | "parsed-literal" => {
                let lang = d.arg.split_whitespace().next().map(str::to_string);
                let mut text = d.content.clone();
                while text.last().is_some_and(|l| l.trim().is_empty()) {
                    text.pop();
                }
                let lang = if d.name == "parsed-literal" {
                    None
                } else {
                    lang
                };
                out.push(Block::Code {
                    lang,
                    text: text.join("\n"),
                });
            }
            "math" => {
                let mut eqs: Vec<String> = Vec::new();
                let mut cur: Vec<String> = Vec::new();
                let mut all = Vec::new();
                if !d.arg.is_empty() {
                    all.push(d.arg.clone());
                    all.push(String::new());
                }
                all.extend(d.content.clone());
                for l in &all {
                    if l.trim().is_empty() {
                        if !cur.is_empty() {
                            eqs.push(cur.join("\n"));
                            cur.clear();
                        }
                    } else {
                        cur.push(l.trim().to_string());
                    }
                }
                if !cur.is_empty() {
                    eqs.push(cur.join("\n"));
                }
                for e in eqs {
                    out.push(Block::Para(vec![Inline::Math {
                        display: true,
                        tex: e,
                    }]));
                }
            }
            "attention" | "caution" | "danger" | "error" | "hint" | "important" | "note"
            | "tip" | "warning" | "admonition" => {
                let generic = d.name == "admonition";
                let title = if generic {
                    d.arg.clone()
                } else {
                    title_case(&d.name)
                };
                let mut body = Vec::new();
                if !generic && !d.arg.is_empty() {
                    body.push(d.arg.clone());
                    body.push(String::new());
                }
                body.extend(d.content.clone());
                let mut blocks = vec![Block::Div {
                    classes: vec!["title".into()],
                    blocks: vec![Block::Para(self.inlines(&title))],
                }];
                blocks.extend(self.blocks(&body, 0));
                out.push(Block::Div {
                    classes: vec![d.name.to_lowercase()],
                    blocks,
                });
            }
            "versionadded" | "versionchanged" | "deprecated" | "seealso" | "topic" | "sidebar"
            | "rubric" | "epigraph" | "highlights" | "pull-quote" | "compound" | "container" => {
                let mut body = vec![d.arg.clone()];
                body.extend(d.content.clone());
                let body = if d.name.starts_with("version")
                    || d.name == "deprecated"
                    || d.name == "seealso"
                {
                    // The argument and the indented text read as one paragraph.
                    let joined: Vec<String> = std::iter::once(d.arg.clone())
                        .chain(d.content.iter().cloned())
                        .collect();
                    joined
                } else {
                    body
                };
                let blocks = self.blocks(&body, 0);
                out.push(Block::Div {
                    classes: vec![d.name.to_lowercase()],
                    blocks,
                });
            }
            "image" => {
                let alt = opt("alt").unwrap_or_default();
                out.push(Block::Para(vec![Inline::Image {
                    url: d.arg.clone(),
                    alt: self.inlines(&alt),
                    title: None,
                }]));
            }
            "figure" => {
                let mut caption = Vec::new();
                let blocks = self.blocks(&d.content, 0);
                if let Some(Block::Para(inl) | Block::Plain(inl)) = blocks.into_iter().next() {
                    caption = inl;
                }
                out.push(Block::Figure {
                    url: d.arg.clone(),
                    caption,
                });
            }
            "raw" => {
                out.push(Block::Raw {
                    format: d.arg.trim().to_string(),
                    text: d.content.join("\n").trim_end().to_string(),
                });
            }
            "contents" | "toctree" | "sectnum" | "header" | "footer" | "meta" | "title"
            | "highlight" | "default-role" | "role" | "index" | "only" | "include"
            | "literalinclude" | "target-notes" | "tabularcolumns" | "module" | "currentmodule"
            | "autosummary" => {
                if matches!(d.name.as_str(), "contents" | "toctree") {
                    out.push(Block::Div {
                        classes: vec![d.name.clone()],
                        blocks: Vec::new(),
                    });
                }
            }
            "csv-table" | "list-table" | "table" => {
                if let Some(t) = self.directive_table(&d) {
                    out.push(Block::Table(t));
                }
            }
            other => {
                let mut body = Vec::new();
                if !d.arg.is_empty() {
                    body.push(d.arg.clone());
                    body.push(String::new());
                }
                body.extend(d.content.clone());
                let blocks = self.blocks(&body, 0);
                out.push(Block::Div {
                    classes: vec![other.to_string()],
                    blocks,
                });
            }
        }
    }

    fn directive_table(&mut self, d: &Directive) -> Option<Table> {
        let header_rows = d
            .options
            .iter()
            .find(|(k, _)| k == "header-rows")
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        match d.name.as_str() {
            "csv-table" => {
                let delim = d
                    .options
                    .iter()
                    .find(|(k, _)| k == "delim")
                    .and_then(|(_, v)| v.chars().next())
                    .unwrap_or(',');
                let mut head: Vec<Vec<String>> = Vec::new();
                if let Some((_, h)) = d.options.iter().find(|(k, _)| k == "header") {
                    head.push(csv_row(h, delim));
                }
                let mut body: Vec<Vec<String>> = d
                    .content
                    .iter()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| csv_row(l, delim))
                    .collect();
                let mut all = head;
                all.append(&mut body);
                let nhead = if d.options.iter().any(|(k, _)| k == "header") {
                    1
                } else {
                    header_rows
                };
                for r in all {
                    rows.push(
                        r.iter()
                            .map(|c| Cell::text(self.inlines(c.trim())))
                            .collect(),
                    );
                }
                let head: Vec<Vec<Cell>> = rows.drain(..nhead.min(rows.len())).collect();
                return Some(Table {
                    caption: self.inlines(&d.arg),
                    align: vec![Align::Default; rows.first().or(head.first()).map_or(0, Vec::len)],
                    head,
                    rows,
                });
            }
            "list-table" => {
                // A bullet list of bullet lists.
                let body = dedent(&d.content);
                let outer = self.blocks(&body, 0);
                if let Some(Block::BulletList(items)) = outer.into_iter().next() {
                    for item in items {
                        if let Some(Block::BulletList(cells)) = item.into_iter().next() {
                            rows.push(
                                cells
                                    .into_iter()
                                    .map(|c| Cell {
                                        blocks: c,
                                        colspan: 1,
                                        rowspan: 1,
                                    })
                                    .collect(),
                            );
                        }
                    }
                }
                let head: Vec<Vec<Cell>> = rows.drain(..header_rows.min(rows.len())).collect();
                return Some(Table {
                    caption: self.inlines(&d.arg),
                    align: vec![Align::Default; rows.first().or(head.first()).map_or(0, Vec::len)],
                    head,
                    rows,
                });
            }
            _ => {
                // `table` wraps a normal table; its argument is the caption.
                let body = dedent(&d.content);
                if let Some(Block::Table(mut t)) = self
                    .blocks(&body, 0)
                    .into_iter()
                    .find(|b| matches!(b, Block::Table(_)))
                {
                    t.caption = self.inlines(&d.arg);
                    return Some(t);
                }
            }
        }
        None
    }

    // ---- tables -------------------------------------------------------------------------------

    fn grid_table(&mut self, lines: &[String]) -> Option<Table> {
        let first = lines.first()?;
        let cols: Vec<usize> = first
            .char_indices()
            .filter(|(_, c)| *c == '+')
            .map(|(i, _)| i)
            .collect();
        if cols.len() < 2 || lines.len() < 3 {
            return None;
        }
        let is_sep = |l: &str| {
            l.starts_with('+')
                && l.chars()
                    .all(|c| c == '+' || c == '-' || c == '=' || c == ' ' || c == ':')
        };
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut head_rows = 0;
        let mut cur: Vec<String> = vec![String::new(); cols.len() - 1];
        let mut have = false;
        let chars: Vec<Vec<char>> = lines.iter().map(|l| l.chars().collect()).collect();
        for (li, l) in lines.iter().enumerate().skip(1) {
            if is_sep(l) {
                if have {
                    rows.push(std::mem::replace(
                        &mut cur,
                        vec![String::new(); cols.len() - 1],
                    ));
                    have = false;
                }
                if l.contains('=') && head_rows == 0 {
                    head_rows = rows.len();
                }
            } else {
                have = true;
                for c in 0..cols.len() - 1 {
                    let (a, b) = (cols[c] + 1, cols[c + 1]);
                    let cell: String = chars[li].iter().skip(a).take(b.saturating_sub(a)).collect();
                    if !cur[c].is_empty() {
                        cur[c].push('\n');
                    }
                    cur[c].push_str(cell.trim_end());
                }
            }
        }
        if have {
            rows.push(cur);
        }
        let mut to_cells = |r: &Vec<String>| -> Vec<Cell> {
            r.iter()
                .map(|c| {
                    let ls: Vec<String> = c.split('\n').map(str::to_string).collect();
                    let d = dedent(&ls);
                    let mut blocks = self.blocks(&d, 0);
                    for b in blocks.iter_mut() {
                        if let Block::Para(inl) = b {
                            *b = Block::Plain(std::mem::take(inl));
                        }
                    }
                    Cell {
                        blocks,
                        colspan: 1,
                        rowspan: 1,
                    }
                })
                .collect()
        };
        let all: Vec<Vec<Cell>> = rows.iter().map(&mut to_cells).collect();
        let (head, body) = all.split_at(head_rows.min(all.len()));
        Some(Table {
            caption: Vec::new(),
            align: vec![Align::Default; cols.len() - 1],
            head: head.to_vec(),
            rows: body.to_vec(),
        })
    }

    fn simple_table(&mut self, lines: &[String], start: usize) -> Option<(Table, usize)> {
        let border = &lines[start];
        let mut cols: Vec<(usize, usize)> = Vec::new();
        let chars: Vec<char> = border.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '=' || chars[i] == '-' {
                let s = i;
                while i < chars.len() && (chars[i] == '=' || chars[i] == '-') {
                    i += 1;
                }
                cols.push((s, i));
            } else {
                i += 1;
            }
        }
        if cols.is_empty() {
            return None;
        }
        let mut borders = vec![start];
        let mut j = start + 1;
        while j < lines.len() {
            if is_simple_border(&lines[j]) {
                borders.push(j);
                // The closing border is followed by a blank line or the end.
                if (borders.len() >= 2 && j + 1 >= lines.len())
                    || (j + 1 < lines.len() && is_blank(&lines[j + 1]) && borders.len() >= 2)
                {
                    break;
                }
            } else if is_blank(&lines[j]) && borders.len() == 1 {
                return None;
            }
            j += 1;
        }
        if borders.len() < 2 {
            return None;
        }
        let end = *borders.last()?;
        let split = |l: &str, last: bool| -> Vec<String> {
            let ch: Vec<char> = l.chars().collect();
            cols.iter()
                .enumerate()
                .map(|(ci, (a, b))| {
                    let b = if ci + 1 == cols.len() && last {
                        ch.len().max(*b)
                    } else {
                        *b
                    };
                    ch.iter()
                        .skip(*a)
                        .take(b.saturating_sub(*a))
                        .collect::<String>()
                        .trim()
                        .to_string()
                })
                .collect()
        };
        let sep_at = if borders.len() >= 3 {
            Some(borders[1])
        } else {
            None
        };
        let mut head: Vec<Vec<Cell>> = Vec::new();
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut cur: Option<Vec<String>> = None;
        let mut in_head = sep_at.is_some();
        let flush = |me: &mut Reader, cur: &mut Option<Vec<String>>, dest: &mut Vec<Vec<Cell>>| {
            if let Some(r) = cur.take() {
                dest.push(r.iter().map(|c| Cell::text(me.inlines(c))).collect());
            }
        };
        for (li, l) in lines.iter().enumerate().take(end).skip(start + 1) {
            if is_simple_border(l) {
                if Some(li) == sep_at {
                    flush(self, &mut cur, &mut head);
                    in_head = false;
                }
                continue;
            }
            if is_blank(l) {
                continue;
            }
            let cells = split(l, true);
            let continuation = cells[0].is_empty() && cur.is_some() && cols.len() > 1;
            if continuation {
                if let Some(c) = cur.as_mut() {
                    for (k, t) in cells.iter().enumerate() {
                        if !t.is_empty() {
                            c[k].push(' ');
                            c[k].push_str(t);
                        }
                    }
                }
            } else {
                if in_head {
                    flush(self, &mut cur, &mut head);
                } else {
                    flush(self, &mut cur, &mut rows);
                }
                cur = Some(cells);
            }
        }
        if in_head {
            flush(self, &mut cur, &mut head);
        } else {
            flush(self, &mut cur, &mut rows);
        }
        Some((
            Table {
                caption: Vec::new(),
                align: vec![Align::Default; cols.len()],
                head,
                rows,
            },
            end + 1,
        ))
    }

    // ---- inline markup --------------------------------------------------------------------------

    fn inlines(&mut self, text: &str) -> Vec<Inline> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        self.inline_run(&chars, &mut out, 0);
        out
    }

    fn inline_run(&mut self, c: &[char], out: &mut Vec<Inline>, depth: usize) {
        let mut buf = String::new();
        let mut i = 0;
        let n = c.len();
        let flush = |buf: &mut String, out: &mut Vec<Inline>| {
            if buf.is_empty() {
                return;
            }
            let mut first = true;
            for part in buf.split('\n') {
                if !first {
                    out.push(Inline::SoftBreak);
                }
                first = false;
                if !part.is_empty() {
                    out.push(Inline::Text(part.to_string()));
                }
            }
            buf.clear();
        };
        while i < n {
            let ch = c[i];
            let prev = if i == 0 { None } else { Some(c[i - 1]) };
            let start_ok = prev.is_none_or(|p| p.is_whitespace() || "'\"([{<-/:".contains(p));
            if ch == '\\' && i + 1 < n {
                if c[i + 1] == ' ' || c[i + 1] == '\n' {
                    i += 2;
                    continue;
                }
                buf.push(c[i + 1]);
                i += 2;
                continue;
            }
            // Interpreted role prefix :role:`text`
            if ch == ':' && start_ok {
                if let Some((role, end_role)) = role_at(c, i) {
                    if end_role < n && c[end_role] == '`' {
                        if let Some(close) = find_close(c, end_role + 1, '`') {
                            let body: String = c[end_role + 1..close].iter().collect();
                            flush(&mut buf, out);
                            self.role(&role, &body, out);
                            i = close + 1;
                            continue;
                        }
                    }
                }
            }
            if ch == '`' && (start_ok || prev == Some('\\')) {
                // ``literal``
                if i + 1 < n && c[i + 1] == '`' {
                    if let Some(close) = find_literal_close(c, i + 2) {
                        flush(&mut buf, out);
                        out.push(Inline::Code(c[i + 2..close].iter().collect()));
                        i = close + 2;
                        continue;
                    }
                } else if let Some(close) = find_close(c, i + 1, '`') {
                    let body: String = c[i + 1..close].iter().collect();
                    let mut after = close + 1;
                    // Suffix role: `text`:role:
                    if after < n && c[after] == ':' {
                        if let Some((role, end_role)) = role_at(c, after) {
                            flush(&mut buf, out);
                            self.role(&role, &body, out);
                            i = end_role;
                            continue;
                        }
                    }
                    // Hyperlink reference `text <url>`_ or `name`_ (anonymous with __).
                    if after < n && c[after] == '_' {
                        let anon = after + 1 < n && c[after + 1] == '_';
                        after += if anon { 2 } else { 1 };
                        flush(&mut buf, out);
                        self.hyperlink(&body, anon, out);
                        i = after;
                        continue;
                    }
                    flush(&mut buf, out);
                    // Default role: title reference, an emphasis.
                    out.push(Inline::Emph(self.nested(&body, depth)));
                    i = close + 1;
                    continue;
                }
            }
            if ch == '*' && start_ok && i + 1 < n && !c[i + 1].is_whitespace() {
                let strong = c[i + 1] == '*';
                let s = i + if strong { 2 } else { 1 };
                if s < n && !c[s].is_whitespace() {
                    if let Some(close) = find_emph_close(c, s, strong) {
                        let body: String = c[s..close].iter().collect();
                        flush(&mut buf, out);
                        let inner = self.nested(&body, depth);
                        out.push(if strong {
                            Inline::Strong(inner)
                        } else {
                            Inline::Emph(inner)
                        });
                        i = close + if strong { 2 } else { 1 };
                        continue;
                    }
                }
            }
            if ch == '|' && start_ok && i + 1 < n && !c[i + 1].is_whitespace() {
                if let Some(close) = find_close(c, i + 1, '|') {
                    let name: String = c[i + 1..close].iter().collect();
                    if let Some(sub) = self.subs.get(&name).cloned() {
                        flush(&mut buf, out);
                        out.extend(sub);
                        i = close + 1;
                        // `|name|_` makes the substitution a link; the underscore is consumed.
                        continue;
                    }
                }
            }
            if ch == '[' && start_ok {
                if let Some(close) = find_close(c, i + 1, ']') {
                    if close + 1 < n && c[close + 1] == '_' && close > i + 1 {
                        let label: String = c[i + 1..close].iter().collect();
                        if let Some(inl) = self.note_ref(&label) {
                            flush(&mut buf, out);
                            out.push(inl);
                            i = close + 2;
                            continue;
                        }
                    }
                }
            }
            // Standalone hyperlink and simple reference names.
            if start_ok && (ch == 'h' || ch == 'f' || ch == 'm') {
                let rest: String = c[i..].iter().take(8).collect();
                if rest.starts_with("http://")
                    || rest.starts_with("https://")
                    || rest.starts_with("ftp://")
                    || rest.starts_with("mailto:")
                {
                    let mut e = i;
                    while e < n && !c[e].is_whitespace() && c[e] != '<' && c[e] != '>' {
                        e += 1;
                    }
                    while e > i && ".,;:!?)'\"]}".contains(c[e - 1]) {
                        e -= 1;
                    }
                    let url: String = c[i..e].iter().collect();
                    flush(&mut buf, out);
                    out.push(Inline::Link {
                        url: url.clone(),
                        title: None,
                        inlines: vec![Inline::Text(url)],
                    });
                    i = e;
                    continue;
                }
            }
            if start_ok && (ch.is_alphanumeric()) {
                // word_ or word__
                let mut e = i;
                while e < n
                    && (c[e].is_alphanumeric()
                        || ((c[e] == '-'
                            || c[e] == '_'
                            || c[e] == '.'
                            || c[e] == ':'
                            || c[e] == '+')
                            && e + 1 < n
                            && c[e + 1].is_alphanumeric()))
                {
                    e += 1;
                }
                if e < n && c[e] == '_' && (e + 1 >= n || !c[e + 1].is_alphanumeric()) {
                    let anon = e + 1 < n && c[e + 1] == '_';
                    let name: String = c[i..e].iter().collect();
                    let after = e + if anon { 2 } else { 1 };
                    let ok_end = after >= n || !c[after].is_alphanumeric();
                    if ok_end && (anon || self.resolve(&name).is_some()) {
                        flush(&mut buf, out);
                        self.hyperlink(&name, anon, out);
                        i = after;
                        continue;
                    }
                }
                // Plain word: copy it whole so a start string inside it is not recognised.
                let mut e2 = i;
                while e2 < n && !c[e2].is_whitespace() && !"`*|[\\:".contains(c[e2]) {
                    e2 += 1;
                }
                if e2 > i {
                    // Keep URLs and `word_` references for the branches above.
                    let word: String = c[i..e2].iter().collect();
                    buf.push_str(&word);
                    i = e2;
                    continue;
                }
            }
            buf.push(ch);
            i += 1;
        }
        flush(&mut buf, out);
    }

    fn nested(&mut self, body: &str, depth: usize) -> Vec<Inline> {
        if depth > 8 {
            return vec![Inline::Text(body.to_string())];
        }
        let chars: Vec<char> = body.chars().collect();
        let mut v = Vec::new();
        self.inline_run(&chars, &mut v, depth + 1);
        v
    }

    fn resolve(&self, name: &str) -> Option<String> {
        let key = normalize_name(name);
        if let Some(u) = self.targets.get(&key) {
            return Some(u.clone());
        }
        self.sections.get(&key).map(|id| format!("#{id}"))
    }

    fn hyperlink(&mut self, body: &str, anon: bool, out: &mut Vec<Inline>) {
        let body = body.trim();
        // `text <url>`_
        if body.ends_with('>') {
            if let Some(lt) = body.rfind('<') {
                let text = body[..lt].trim();
                let url = body[lt + 1..body.len() - 1].trim().replace(['\n', ' '], "");
                if let Some(name) = url.strip_suffix('_') {
                    let name = name.trim_matches('`');
                    let target = self
                        .resolve(name)
                        .unwrap_or_else(|| format!("#{}", identifier(name)));
                    let t = if text.is_empty() { name } else { text };
                    out.push(Inline::Link {
                        url: target,
                        title: None,
                        inlines: self.nested(t, 8),
                    });
                } else {
                    let label = if text.is_empty() {
                        url.clone()
                    } else {
                        text.to_string()
                    };
                    let inl = if text.is_empty() {
                        vec![Inline::Text(label)]
                    } else {
                        self.nested(&label, 8)
                    };
                    out.push(Inline::Link {
                        url: target_url(&url),
                        title: None,
                        inlines: inl,
                    });
                }
                return;
            }
        }
        let url = if anon {
            let u = self.anonymous.get(self.anon_used).cloned();
            self.anon_used += 1;
            u
        } else {
            self.resolve(body)
        };
        match url {
            Some(u) => {
                let inl = self.nested(body, 8);
                out.push(Inline::Link {
                    url: u,
                    title: None,
                    inlines: inl,
                });
            }
            None => {
                let inl = self.nested(body, 8);
                out.push(Inline::Link {
                    url: format!("#{}", identifier(body)),
                    title: None,
                    inlines: inl,
                });
            }
        }
    }

    fn note_ref(&mut self, label: &str) -> Option<Inline> {
        let content = if label == "#" || label == "*" {
            let c = self
                .auto_notes
                .get(if label == "#" {
                    self.auto_used
                } else {
                    self.auto_used + self.symbol_used
                })
                .cloned();
            if c.is_some() {
                self.auto_used += 1;
            }
            c
        } else if label.chars().all(|c| c.is_ascii_digit()) || label.starts_with('#') {
            self.notes.get(label.trim_start_matches('#')).cloned()
        } else {
            // A citation reference: a link to the definition.
            return Some(Inline::Link {
                url: format!("#{label}"),
                title: None,
                inlines: vec![Inline::Verbatim(format!("\\[{label}\\]"))],
            })
            .filter(|_| self.notes.contains_key(label));
        };
        let content = content?;
        let blocks = self.blocks(&content, 0);
        Some(Inline::Note(blocks))
    }

    fn role(&mut self, role: &str, body: &str, out: &mut Vec<Inline>) {
        match role {
            "emphasis" | "title-reference" | "title" | "t" => {
                out.push(Inline::Emph(self.nested(body, 8)))
            }
            "strong" => out.push(Inline::Strong(self.nested(body, 8))),
            "literal" | "code" | "samp" | "kbd" | "file" | "command" | "program" | "option"
            | "envvar" | "makevar" | "regexp" => out.push(Inline::Code(body.to_string())),
            "math" => out.push(Inline::Math {
                display: false,
                tex: body.to_string(),
            }),
            "sub" | "subscript" => out.push(Inline::Sub(self.nested(body, 8))),
            "sup" | "superscript" => out.push(Inline::Sup(self.nested(body, 8))),
            "pep-reference" | "rfc-reference" | "pep" | "rfc" => {
                out.push(Inline::Text(body.to_string()))
            }
            other => out.push(Inline::Verbatim(format!(
                "`{}`{{.interpreted-text role=\"{}\"}}",
                body.replace('`', "\\`"),
                other
            ))),
        }
    }
}

// ---- helpers --------------------------------------------------------------------------------------

fn tighten(blocks: &mut [Block]) {
    let _ = blocks;
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn unicode_directive(s: &str) -> String {
    s.split_whitespace()
        .filter(|w| !w.starts_with(".."))
        .filter_map(|w| {
            let w = w
                .trim_start_matches("U+")
                .trim_start_matches("0x")
                .trim_start_matches("&#x")
                .trim_end_matches(';');
            u32::from_str_radix(w, 16).ok().and_then(char::from_u32)
        })
        .collect()
}

fn csv_row(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for ch in line.chars() {
        if ch == '"' {
            q = !q;
        } else if ch == delim && !q {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out
}

fn is_adornment(line: &str, min: usize) -> bool {
    let t = line.trim_end();
    let mut ch = t.chars();
    let Some(f) = ch.next() else { return false };
    !f.is_alphanumeric()
        && !f.is_whitespace()
        && "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".contains(f)
        && t.chars().all(|c| c == f)
        && t.chars().count() >= min.max(1)
}

fn is_simple_border(line: &str) -> bool {
    let t = line.trim_end();
    t.starts_with('=') && t.chars().all(|c| c == '=' || c == ' ') && t.contains(' ')
}

fn is_quoted_literal(line: &str) -> bool {
    line.chars()
        .next()
        .is_some_and(|c| "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".contains(c))
}

fn bullet_offset(line: &str) -> Option<usize> {
    let mut chars = line.chars();
    let b = chars.next()?;
    if !matches!(b, '-' | '*' | '+' | '\u{2022}' | '\u{2023}' | '\u{2043}') {
        return None;
    }
    let rest: String = chars.collect();
    if rest.is_empty() {
        return Some(line.len());
    }
    if !rest.starts_with(' ') {
        return None;
    }
    let sp = rest.len() - rest.trim_start_matches(' ').len();
    Some(b.len_utf8() + sp)
}

struct EnumMarker {
    offset: usize,
    start: u32,
    style: ListStyle,
    sure: bool,
}

fn enum_marker(line: &str) -> Option<EnumMarker> {
    let (paren, rest) = match line.strip_prefix('(') {
        Some(r) => (true, r),
        None => (false, line),
    };
    let end = rest.find(|c: char| !c.is_alphanumeric() && c != '#')?;
    let token = &rest[..end];
    let delim = rest[end..].chars().next()?;
    if token.is_empty() || !(delim == '.' && !paren || delim == ')') {
        return None;
    }
    let after = &rest[end + 1..];
    if !(after.is_empty() || after.starts_with(' ')) {
        return None;
    }
    let (start, style, sure) = if token == "#" {
        (1, ListStyle::Decimal, true)
    } else if token.chars().all(|c| c.is_ascii_digit()) {
        (token.parse().ok()?, ListStyle::Decimal, true)
    } else if token.len() == 1 && token.chars().all(|c| c.is_ascii_lowercase()) {
        (
            u32::from(token.as_bytes()[0] - b'a') + 1,
            ListStyle::LowerAlpha,
            false,
        )
    } else if token.len() == 1 && token.chars().all(|c| c.is_ascii_uppercase()) {
        (
            u32::from(token.as_bytes()[0] - b'A') + 1,
            ListStyle::UpperAlpha,
            false,
        )
    } else if token.chars().all(|c| "ivxlcdm".contains(c)) {
        (roman(token)?, ListStyle::LowerRoman, false)
    } else if token.chars().all(|c| "IVXLCDM".contains(c)) {
        (roman(&token.to_lowercase())?, ListStyle::UpperRoman, false)
    } else {
        return None;
    };
    let sp = after.len() - after.trim_start_matches(' ').len();
    let offset = usize::from(paren) + token.len() + 1 + sp.max(1).min(after.len().max(1));
    Some(EnumMarker {
        offset: offset.min(line.len()),
        start,
        style,
        sure,
    })
}

fn roman(s: &str) -> Option<u32> {
    let v = |c| match c {
        'i' => 1,
        'v' => 5,
        'x' => 10,
        'l' => 50,
        'c' => 100,
        'd' => 500,
        'm' => 1000,
        _ => 0,
    };
    let d: Vec<u32> = s.chars().map(v).collect();
    let mut t = 0;
    for (i, x) in d.iter().enumerate() {
        if d.get(i + 1).is_some_and(|n| n > x) {
            t -= *x as i64;
        } else {
            t += *x as i64;
        }
    }
    u32::try_from(t).ok().filter(|v| *v > 0)
}

/// `:name: rest` at the start of a line.
fn field_name(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(':')?;
    if rest.starts_with(' ') || rest.starts_with(':') {
        return None;
    }
    let mut i = 0;
    let b = rest.as_bytes();
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' ') && i > 0 && b[i - 1] != b' ' {
            return Some((rest[..i].to_string(), rest[i + 1..].to_string()));
        }
        i += 1;
    }
    None
}

/// The end line (exclusive) of an explicit-markup block that starts at `i`, and whether it
/// holds any indented body.
fn explicit_extent(lines: &[String], i: usize) -> (usize, bool) {
    let mut j = i + 1;
    let n = lines.len();
    let mut body = false;
    while j < n {
        if is_blank(&lines[j]) {
            // Continues only while the following non-blank line is indented.
            let mut k = j;
            while k < n && is_blank(&lines[k]) {
                k += 1;
            }
            if k < n && indent_of(&lines[k]) > 0 {
                j = k;
                continue;
            }
            break;
        }
        if indent_of(&lines[j]) == 0 {
            break;
        }
        body = true;
        j += 1;
    }
    (j, body)
}

fn split_target(rest: &str) -> Option<(&str, &str)> {
    // `name: url` or `` `name with: colon`: url ``
    let rest = rest.trim_start();
    if let Some(r) = rest.strip_prefix('`') {
        let close = r.find('`')?;
        let after = r[close + 1..].strip_prefix(':')?;
        return Some((&r[..close], after));
    }
    let mut i = 0;
    let b = rest.as_bytes();
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' ') {
            return Some((&rest[..i], &rest[i + 1..]));
        }
        i += 1;
    }
    None
}

fn join_target(first: &str, body: &[String]) -> String {
    let mut s = first.trim().to_string();
    for l in body {
        s.push_str(l.trim());
    }
    s.replace(' ', "")
}

fn target_url(u: &str) -> String {
    u.trim().to_string()
}

fn parse_directive(name: &str, arg: String, body: &[String]) -> Directive {
    let mut body = dedent(body);
    // The argument may continue on the following lines up to the first option or blank line.
    let mut arg = arg;
    let mut idx = 0;
    while idx < body.len()
        && !is_blank(&body[idx])
        && field_name(&body[idx]).is_none()
        && !matches!(
            name,
            "code"
                | "code-block"
                | "sourcecode"
                | "math"
                | "raw"
                | "parsed-literal"
                | "versionadded"
                | "versionchanged"
                | "deprecated"
                | "seealso"
        )
    {
        if !arg.is_empty() {
            arg.push(' ');
        }
        arg.push_str(body[idx].trim());
        idx += 1;
    }
    if idx > 0
        && (matches!(
            name,
            "versionadded" | "versionchanged" | "deprecated" | "seealso"
        ))
    {
        // Keep them as content for these admonition-like directives.
        arg = String::new();
        idx = 0;
    }
    body.drain(..idx);
    let mut options = Vec::new();
    let mut k = 0;
    while k < body.len() {
        if let Some((n, v)) = field_name(&body[k]) {
            let mut v = v.trim().to_string();
            k += 1;
            while k < body.len() && indent_of(&body[k]) > 0 {
                v.push(' ');
                v.push_str(body[k].trim());
                k += 1;
            }
            options.push((n, v));
        } else {
            break;
        }
    }
    body.drain(..k);
    while body.first().is_some_and(|l| is_blank(l)) {
        body.remove(0);
    }
    Directive {
        name: name.to_string(),
        arg,
        options,
        content: body,
    }
}

fn role_at(c: &[char], i: usize) -> Option<(String, usize)> {
    // `:role:` starting at i; returns the role and the index after the closing colon.
    if c.get(i) != Some(&':') {
        return None;
    }
    let mut j = i + 1;
    while j < c.len()
        && (c[j].is_alphanumeric() || c[j] == '-' || c[j] == '_' || c[j] == '.' || c[j] == '+')
    {
        j += 1;
    }
    if j > i + 1 && j < c.len() && c[j] == ':' {
        let role: String = c[i + 1..j].iter().collect();
        return Some((role, j + 1));
    }
    None
}

fn find_close(c: &[char], from: usize, ch: char) -> Option<usize> {
    let mut i = from;
    while i < c.len() {
        if c[i] == '\\' {
            i += 2;
            continue;
        }
        if c[i] == ch && i > from {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_literal_close(c: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < c.len() {
        if c[i] == '`' && c[i + 1] == '`' && i > from {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_emph_close(c: &[char], from: usize, strong: bool) -> Option<usize> {
    let mut i = from;
    while i < c.len() {
        if c[i] == '\\' {
            i += 2;
            continue;
        }
        if c[i] == '*' && i > from && !c[i - 1].is_whitespace() {
            if strong {
                if c.get(i + 1) == Some(&'*') {
                    let after = c.get(i + 2);
                    if after.is_none_or(|a| a.is_whitespace() || "'\")]}>-/:.,;!?\\".contains(*a)) {
                        return Some(i);
                    }
                }
            } else if c.get(i + 1) != Some(&'*') {
                let after = c.get(i + 1);
                if after.is_none_or(|a| a.is_whitespace() || "'\")]}>-/:.,;!?\\".contains(*a)) {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> String {
        to_markdown(s.as_bytes())
    }

    fn levels(s: &str) -> Vec<(u8, String)> {
        to_doc(s)
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, inlines, .. } => Some((*level, ast::inlines_text(inlines))),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sections_follow_the_order_of_adornments() {
        let l = levels("T\n=\n\ntext\n\nS\n-\n\nSS\n~~\n\nB\n=\n");
        assert_eq!(
            l,
            vec![
                (1, "T".into()),
                (2, "S".into()),
                (3, "SS".into()),
                (1, "B".into())
            ]
        );
        // An overline is a different style from the same character without one.
        let l = levels("===\nT\n===\n\nS\n===\n");
        assert_eq!(l, vec![(1, "T".into()), (2, "S".into())]);
    }

    #[test]
    fn inline_markup_roles_and_links() {
        let out = md(
            "*a* **b** ``c`` :sub:`2` `x <https://e.org>`__ and |s|\n\n.. |s| replace:: Subst\n",
        );
        assert!(
            out.contains("*a* **b** `c` ~2~ [x](https://e.org) and Subst"),
            "{out}"
        );
    }

    #[test]
    fn lists_literal_blocks_and_directives() {
        let d = to_doc("- a\n- b\n\nRun::\n\n    x = 1\n\n.. code:: rust\n\n   fn f() {}\n\n.. math:: E = mc^2\n");
        assert!(
            matches!(&d.blocks[0], Block::BulletList(items) if items.len() == 2),
            "{d:?}"
        );
        assert!(
            d.blocks
                .iter()
                .any(|b| matches!(b, Block::Code { lang: None, text } if text == "x = 1")),
            "{d:?}"
        );
        assert!(
            d.blocks
                .iter()
                .any(|b| matches!(b, Block::Code { lang: Some(l), .. } if l == "rust")),
            "{d:?}"
        );
        assert!(d.blocks.iter().any(|b| matches!(b, Block::Para(p) if matches!(p.first(), Some(Inline::Math { display: true, .. })))), "{d:?}");
    }

    #[test]
    fn tables_footnotes_and_fields() {
        let out = md("=== ===\nA   B\n=== ===\n1   2\n=== ===\n\nText [1]_.\n\n.. [1] The note.\n\n:Author: Me\n");
        assert!(
            out.contains("| A | B |") || out.contains("A   B") || out.contains("---"),
            "{out}"
        );
        assert!(out.contains("[^1]"), "{out}");
        assert!(out.contains("Author"), "{out}");
    }

    #[test]
    fn anonymous_targets_and_admonitions() {
        let out = md("See `x`__.\n\n__ https://e.org\n\n.. note:: Careful\n");
        assert!(out.contains("[x](https://e.org)"), "{out}");
        assert!(
            out.contains("::: {.note}") || out.contains("::: note"),
            "{out}"
        );
    }
}
