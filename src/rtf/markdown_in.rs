//! Reads the Markdown subset that `render` produces, so that `markdown_to_rtf` can write it:
//! ATX headings, paragraphs, `-`/`*`/`+` and `N.` lists (nested by indentation), pipe tables,
//! `**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `[text](url)`, backslash escapes, and
//! `<br>` inside table cells.

use super::model::{
    Alignment, Block, Cell, Hyperlink, ListItem, Paragraph, Row, RtfDocument, Run, Style, Table,
};

/// Parses Markdown into a document. Unsupported syntax is kept as literal text.
pub(super) fn parse(markdown: &str) -> RtfDocument {
    let mut doc = RtfDocument::default();
    let lines: Vec<&str> = markdown.lines().collect();
    let mut index = 0;
    let mut list_indents: Vec<usize> = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            index += 1;
            list_indents.clear();
            continue;
        }
        if trimmed.starts_with('|') && lines.get(index + 1).is_some_and(|l| is_separator(l)) {
            let (table, next) = parse_table(&lines, index, &mut doc);
            doc.blocks.push(Block::Table(table));
            index = next;
            continue;
        }
        if let Some((level, text)) = heading(trimmed) {
            let runs = inline(text, &mut doc);
            doc.blocks.push(Block::Paragraph(Paragraph {
                runs,
                heading_level: Some(level),
                ..Paragraph::default()
            }));
            index += 1;
            list_indents.clear();
            continue;
        }
        if let Some((indent, ordered_number, text)) = list_item(line) {
            while list_indents.len() > 1 && indent < *list_indents.last().unwrap_or(&0) {
                list_indents.pop();
            }
            match list_indents.last() {
                Some(&top) if indent > top => list_indents.push(indent),
                None => list_indents.push(indent),
                _ => {}
            }
            let level = list_indents.len().saturating_sub(1);
            let runs = inline(text, &mut doc);
            let (ordered, marker) = match ordered_number {
                Some(n) => (true, format!("{n}.")),
                None => (false, "\u{2022}".to_string()),
            };
            doc.blocks.push(Block::ListItem(ListItem {
                level: u8::try_from(level).unwrap_or(u8::MAX),
                ordered,
                marker,
                runs,
            }));
            index += 1;
            continue;
        }
        // A paragraph runs until a blank line or the start of another block.
        let mut text = String::new();
        while index < lines.len() {
            let current = lines[index].trim();
            if current.is_empty()
                || heading(current).is_some()
                || (current.starts_with('|')
                    && lines.get(index + 1).is_some_and(|l| is_separator(l)))
                || list_item(lines[index]).is_some()
            {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(current);
            index += 1;
        }
        let runs = inline(&text, &mut doc);
        doc.blocks.push(Block::Paragraph(Paragraph {
            runs,
            alignment: Alignment::Left,
            ..Paragraph::default()
        }));
        list_indents.clear();
    }
    doc
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        let level = u8::try_from(hashes).ok()?;
        Some((level, trimmed[hashes..].trim()))
    } else {
        None
    }
}

/// Returns (indent width, ordered number, text) for a list line.
fn list_item(line: &str) -> Option<(usize, Option<u32>, &str)> {
    let indent_str: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let indent: usize = indent_str
        .chars()
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum();
    let rest = &line[indent_str.len()..];
    let bytes = rest.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ') {
        return Some((indent, None, rest[2..].trim()));
    }
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0
        && matches!(bytes.get(digits), Some(b'.' | b')'))
        && bytes.get(digits + 1) == Some(&b' ')
    {
        let number = rest[..digits].parse::<u32>().ok()?;
        return Some((indent, Some(number), rest[digits + 2..].trim()));
    }
    None
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn parse_table(lines: &[&str], start: usize, doc: &mut RtfDocument) -> (Table, usize) {
    let mut rows = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index].trim();
        if !line.starts_with('|') {
            break;
        }
        if index != start + 1 {
            let cells = split_row(line)
                .into_iter()
                .map(|text| Cell {
                    blocks: vec![Block::Paragraph(Paragraph {
                        runs: inline(&text, doc),
                        ..Paragraph::default()
                    })],
                })
                .collect();
            rows.push(Row {
                header: rows.is_empty(),
                cells,
                edges: Vec::new(),
            });
        }
        index += 1;
    }
    (Table { rows }, index)
}

/// Splits a pipe-table row on unescaped `|`, dropping the outer pipes.
fn split_row(line: &str) -> Vec<String> {
    let inner = line.strip_prefix('|').unwrap_or(line);
    let inner = if inner.ends_with('|') && !inner.ends_with("\\|") {
        &inner[..inner.len() - 1]
    } else {
        inner
    };
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                current.push('\\');
                current.push(next);
                chars.next();
                continue;
            }
        }
        if c == '|' {
            cells.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    cells.push(current.trim().to_string());
    cells
}

#[derive(Default, Clone, Copy)]
struct Emphasis {
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<usize>,
}

/// Parses inline Markdown into runs. Links are added to `doc.hyperlinks`.
fn inline(text: &str, doc: &mut RtfDocument) -> Vec<Run> {
    let mut runs = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut state = Emphasis::default();
    let mut buffer = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\\' {
            if let Some(n) = next.filter(|n| n.is_ascii_punctuation()) {
                buffer.push(n);
                i += 2;
                continue;
            }
        }
        if c == '*' && next == Some('*') {
            flush(&mut buffer, state, &mut runs);
            state.bold = !state.bold;
            i += 2;
            continue;
        }
        if c == '*' {
            flush(&mut buffer, state, &mut runs);
            state.italic = !state.italic;
            i += 1;
            continue;
        }
        if c == '~' && next == Some('~') {
            flush(&mut buffer, state, &mut runs);
            state.strike = !state.strike;
            i += 2;
            continue;
        }
        if c == '_' {
            let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
            let next_alnum = next.is_some_and(char::is_alphanumeric);
            if prev_alnum != next_alnum {
                flush(&mut buffer, state, &mut runs);
                state.italic = !state.italic;
                i += 1;
                continue;
            }
        }
        if c == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == '`') {
                buffer.extend(chars[i + 1..i + 1 + end].iter());
                i += end + 2;
                continue;
            }
        }
        if c == '<' && chars[i..].iter().take(4).collect::<String>() == "<br>" {
            buffer.push('\n');
            i += 4;
            continue;
        }
        if c == '[' {
            if let Some((label, url, consumed)) = link_at(&chars, i) {
                flush(&mut buffer, state, &mut runs);
                doc.hyperlinks.push(Hyperlink { target: url });
                let link = doc.hyperlinks.len() - 1;
                let inner: String = label;
                let inner_runs = inline(&inner, doc);
                for mut run in inner_runs {
                    run.style.link = Some(link);
                    runs.push(run);
                }
                i += consumed;
                continue;
            }
        }
        buffer.push(c);
        i += 1;
    }
    flush(&mut buffer, state, &mut runs);
    runs
}

/// Recognises `[label](url)` starting at `start`. Returns the label, the URL and the number of
/// characters consumed.
fn link_at(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut depth = 0usize;
    let mut close = None;
    for (offset, &c) in chars.iter().enumerate().skip(start) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    close = Some(offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let url_end = chars[close + 2..].iter().position(|&c| c == ')')? + close + 2;
    let label: String = chars[start + 1..close].iter().collect();
    let raw_url: String = chars[close + 2..url_end].iter().collect();
    let url = raw_url
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string();
    Some((label, url, url_end + 1 - start))
}

fn flush(buffer: &mut String, state: Emphasis, runs: &mut Vec<Run>) {
    if buffer.is_empty() {
        return;
    }
    runs.push(Run {
        text: std::mem::take(buffer),
        style: Style {
            bold: state.bold,
            italic: state.italic,
            strike: state.strike,
            link: state.link,
            ..Style::default()
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_emphasis_and_escapes() {
        let mut doc = RtfDocument::default();
        let runs = inline("a **b** *c* \\*d\\*", &mut doc);
        let summary: Vec<(String, bool, bool)> = runs
            .iter()
            .map(|r| (r.text.clone(), r.style.bold, r.style.italic))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("a ".to_string(), false, false),
                ("b".to_string(), true, false),
                (" ".to_string(), false, false),
                ("c".to_string(), false, true),
                (" *d*".to_string(), false, false),
            ]
        );
    }

    #[test]
    fn snake_case_underscores_are_literal() {
        let mut doc = RtfDocument::default();
        let runs = inline("snake_case_name", &mut doc);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "snake_case_name");
        assert!(!runs[0].style.italic);
    }

    #[test]
    fn links_are_recorded() {
        let mut doc = RtfDocument::default();
        let runs = inline("see [the site](https://example.test/a) now", &mut doc);
        assert_eq!(doc.hyperlinks[0].target, "https://example.test/a");
        assert_eq!(runs[1].text, "the site");
        assert_eq!(runs[1].style.link, Some(0));
    }

    #[test]
    fn list_nesting_by_indentation() {
        let doc = parse("- a\n    - b\n- c\n");
        let levels: Vec<u8> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::ListItem(item) => item.level,
                _ => 99,
            })
            .collect();
        assert_eq!(levels, vec![0, 1, 0]);
    }

    #[test]
    fn pipe_table_header_and_rows() {
        let doc = parse("| h1 | h2 |\n| --- | --- |\n| x | y |\n");
        let Block::Table(table) = &doc.blocks[0] else {
            panic!("expected a table");
        };
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[0].header);
        assert!(!table.rows[1].header);
        assert_eq!(table.rows[1].cells.len(), 2);
    }
}
