//! Pandoc's citation syntax in Markdown text, and a whole-document pass that replaces the
//! citations with formatted text and appends the bibliography (`pandoc --citeproc`'s job).

use super::terms::LOCATOR_LABELS;
use super::{quote_marks, to_inlines, Citation, CitationItem, Processor};
use crate::ast;

/// A citation found in text: byte range and the parsed citation.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub citation: Citation,
}

fn key_char(c: char) -> bool {
    c.is_alphanumeric() || "_:.#$%&-+?<>~/".contains(c)
}

/// The key after an `@` at `at` (the byte index of the `@`): `(key, end)`.
fn parse_key(s: &str, at: usize) -> Option<(String, usize)> {
    let rest = &s[at + 1..];
    if let Some(r) = rest.strip_prefix('{') {
        let close = r.find('}')?;
        return Some((r[..close].to_string(), at + 1 + 1 + close + 1));
    }
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if !(first.is_alphanumeric() || first == '_') {
        return None;
    }
    let mut end = first.len_utf8();
    for (i, c) in rest.char_indices().skip(1) {
        if key_char(c) {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    // Trailing punctuation is not part of the key.
    let mut key = &rest[..end];
    while let Some(c) = key.chars().last() {
        if c.is_alphanumeric() || c == '_' {
            break;
        }
        key = &key[..key.len() - c.len_utf8()];
    }
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), at + 1 + key.len()))
}

/// Splits a suffix into locator label, locator and the rest, with the labels built in.
pub fn parse_locator(suffix: &str) -> (Option<String>, Option<String>, String) {
    parse_locator_with(suffix, &|w| LOCATOR_LABELS.iter().find(|(l, _)| l.eq_ignore_ascii_case(w)).map(|(_, n)| (*n).to_string()))
}

/// The same with a caller-supplied label lookup (the words the style's own locale uses).
pub fn parse_locator_with(suffix: &str, lookup: &dyn Fn(&str) -> Option<String>) -> (Option<String>, Option<String>, String) {
    let t = suffix.trim_start_matches([',', ' ']);
    // A label word (with its dot) followed by a space.
    let mut label: Option<String> = None;
    let mut rest = t;
    let mut first_word_end = t.find(' ').unwrap_or(t.len());
    // "p.3": a label ending in a full stop may run into the number.
    if let Some(dot) = t.find('.') {
        if dot < first_word_end && dot + 1 < t.len() && t[dot + 1..].starts_with(|c: char| c.is_ascii_digit()) && lookup(&t[..=dot]).is_some() {
            first_word_end = dot + 1;
        }
    }
    let first = &t[..first_word_end];
    if let Some(name) = lookup(first) {
        label = Some(name);
        rest = t[first_word_end..].trim_start();
    } else if first.starts_with('\u{a7}') || first.starts_with('\u{b6}') {
        // "§3" without a space.
        let sym: String = first.chars().take_while(|c| *c == '\u{a7}' || *c == '\u{b6}').collect();
        if let Some(name) = lookup(&sym) {
            label = Some(name);
            rest = &t[sym.len()..];
        }
    }
    // The locator: number-like tokens joined by dashes, commas and ampersands.
    let tok_len = |r: &str| -> usize {
        let n: usize = r.chars().take_while(|c| c.is_alphanumeric() || *c == '.').map(char::len_utf8).sum();
        let tok = &r[..n];
        let roman = label.is_some() && !tok.is_empty() && tok.chars().all(|c| "ivxlcdmIVXLCDM".contains(c));
        if tok.chars().any(|c| c.is_ascii_digit()) || roman {
            // A trailing full stop ends the sentence, not the number.
            if tok.ends_with('.') && !tok.trim_end_matches('.').is_empty() && r[n..].starts_with(' ') && false {
                n - 1
            } else {
                n
            }
        } else {
            0
        }
    };
    let mut end = 0;
    loop {
        let n = tok_len(&rest[end..]);
        if n == 0 {
            break;
        }
        end += n;
        let r = &rest[end..];
        if r.starts_with('-') || r.starts_with('\u{2013}') {
            let d = r.chars().next().map_or(1, char::len_utf8);
            if tok_len(&r[d..]) > 0 {
                end += d;
                continue;
            }
            break;
        }
        let sep = if r.starts_with(", & ") {
            4
        } else if r.starts_with(", ") || r.starts_with(" & ") {
            if r.starts_with(", ") { 2 } else { 3 }
        } else {
            break;
        };
        if tok_len(&r[sep..]) > 0 {
            end += sep;
        } else {
            break;
        }
    }
    let locator = rest[..end].trim().trim_end_matches(',').to_string();
    if locator.is_empty() {
        return (None, None, suffix.to_string());
    }
    let tail = &rest[end..];
    if label.is_none() {
        label = Some("page".into());
    }
    (label, Some(locator), tail.to_string())
}

fn item_from(prefix: &str, suppress: bool, key: String, suffix: &str, in_text: bool) -> CitationItem {
    // The locator is read later, with the labels of the style that formats the citation.
    let (label, locator, rest) = (None, None, suffix.to_string());
    let prefix = prefix.trim_start();
    let prefix = if prefix.trim().is_empty() { "" } else { prefix };
    CitationItem {
        id: key,
        prefix: prefix.to_string(),
        suffix: rest,
        label,
        locator,
        suppress_author: suppress,
        author_in_text: in_text,
        unparsed: true,
    }
}

/// Parses the inside of `[...]`: items separated by `;`, each `prefix -?@key suffix`.
fn parse_bracketed(inner: &str) -> Option<Citation> {
    let mut items = Vec::new();
    let mut any = false;
    for part in split_semicolons(inner) {
        // Find the first `@` that starts a key: at the start, or after whitespace, `-` or `[`.
        let mut found: Option<(usize, bool)> = None;
        for (i, c) in part.char_indices() {
            if c == '@' {
                let before = part[..i].chars().last();
                let ok = before.is_none_or(|b| b.is_whitespace() || b == '-' || b == '(' || b == '[' || b == '\u{2018}' || b == '\u{201c}');
                if ok {
                    let suppress = before == Some('-') && (i == 1 || part[..i - 1].chars().last().is_none_or(char::is_whitespace));
                    found = Some((i, suppress));
                    break;
                }
            }
        }
        let (at, suppress) = found?;
        let (key, end) = parse_key(part, at)?;
        let prefix_end = if suppress { at - 1 } else { at };
        let prefix = &part[..prefix_end];
        let suffix = &part[end..];
        items.push(item_from(prefix, suppress, key, suffix, false));
        any = true;
    }
    any.then_some(Citation { items })
}

fn split_semicolons(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth -= 1,
            ';' if depth <= 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn skip_front_matter(s: &str) -> usize {
    if let Some(rest) = s.strip_prefix("---\n") {
        if let Some(i) = rest.find("\n---\n").or_else(|| rest.find("\n...\n")) {
            return 4 + i + 5;
        }
    }
    0
}

/// Every citation in the text, in order, outside code and the YAML header.
pub fn parse_citations(s: &str) -> Vec<Found> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = skip_front_matter(s);
    let mut line_start = true;
    let mut fence: Option<String> = None;
    while i < s.len() {
        let c = s[i..].chars().next().unwrap_or(' ');
        let cl = c.len_utf8();
        // Fenced code blocks.
        if line_start {
            let line_end = s[i..].find('\n').map(|x| i + x).unwrap_or(s.len());
            let line = &s[i..line_end];
            let t = line.trim_start();
            if let Some(f) = &fence {
                if t.starts_with(f.as_str()) {
                    fence = None;
                }
                i = (line_end + 1).min(s.len());
                continue;
            } else if t.starts_with("```") || t.starts_with("~~~") {
                fence = Some(t.chars().take(3).collect());
                i = (line_end + 1).min(s.len());
                continue;
            } else if line.starts_with("    ") && !line.trim().is_empty() && false {
                i = (line_end + 1).min(s.len());
                continue;
            }
        }
        line_start = c == '\n';
        match c {
            '\\' => {
                i += cl + s[i + cl..].chars().next().map(char::len_utf8).unwrap_or(0);
                continue;
            }
            '`' => {
                let n = s[i..].chars().take_while(|c| *c == '`').count();
                let ticks = "`".repeat(n);
                if let Some(p) = s[i + n..].find(&ticks) {
                    i += n + p + n;
                    continue;
                }
                i += n;
                continue;
            }
            '[' => {
                // Find the matching bracket.
                let mut depth = 0;
                let mut j = i;
                let mut close = None;
                while j < s.len() {
                    match b[j] {
                        b'\\' => j += 1,
                        b'[' => depth += 1,
                        b']' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if let Some(cl_idx) = close {
                    let inner = &s[i + 1..cl_idx];
                    let next = s[cl_idx + 1..].chars().next();
                    if inner.contains('@') && next != Some('(') && !(next == Some('[') && false) {
                        if let Some(c) = parse_bracketed(inner) {
                            out.push(Found { start: i, end: cl_idx + 1, citation: c });
                            i = cl_idx + 1;
                            continue;
                        }
                    }
                }
                i += 1;
                continue;
            }
            '@' => {
                let prev = s[..i].chars().last();
                let ok = prev.is_none_or(|p| !(p.is_alphanumeric() || p == '@' || p == '\\' || p == '_'));
                if ok {
                    if let Some((key, end)) = parse_key(s, i) {
                        let mut end_all = end;
                        let mut suffix = String::new();
                        // `@key [p. 33]`
                        let rest = &s[end..];
                        let trimmed = rest.trim_start_matches(' ');
                        if trimmed.starts_with('[') && rest.len() - trimmed.len() <= 1 {
                            let off = end + (rest.len() - trimmed.len());
                            if let Some(close) = s[off..].find(']') {
                                let inner = &s[off + 1..off + close];
                                if !inner.contains('@') && !s[off + close + 1..].starts_with('(') {
                                    suffix = inner.to_string();
                                    end_all = off + close + 1;
                                }
                            }
                        }
                        let item = item_from("", false, key, &suffix, true);
                        out.push(Found { start: i, end: end_all, citation: Citation { items: vec![item] } });
                        i = end_all;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        i += cl;
    }
    out
}

/// Replaces each citation in the Markdown with its formatted text and appends the bibliography
/// as a `refs` div, as `pandoc --citeproc` does. `nocite_all` lists every reference
/// (`nocite: '@*'`).
pub fn process_markdown(md: &str, p: &Processor) -> String {
    let found = parse_citations(md);
    let nocite_all = skip_front_matter(md) > 0 && md[..skip_front_matter(md)].contains("@*");
    let cites: Vec<Citation> = found.iter().map(|f| f.citation.clone()).collect();
    let nocite: Vec<String> = if nocite_all { p.refs_in_order() } else { Vec::new() };
    let done = p.process(&cites, &nocite);
    let (oq, iq) = quote_marks(p.style());
    let mut out = String::new();
    let mut last = 0;
    for (f, toks) in found.iter().zip(&done.citations) {
        out.push_str(&md[last..f.start]);
        let inl = to_inlines(toks, (&oq.0, &oq.1), (&iq.0, &iq.1));
        out.push_str(&ast::inlines_to_markdown(&inl));
        last = f.end;
    }
    out.push_str(&md[last..]);
    if !done.bibliography.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n::: {#refs}\n");
        for e in &done.bibliography {
            let inl = to_inlines(&e.toks, (&oq.0, &oq.1), (&iq.0, &iq.1));
            out.push_str(&format!("\n::: {{#ref-{} .csl-entry}}\n{}\n:::\n", e.id, ast::inlines_to_markdown(&inl)));
        }
        out.push_str(":::\n");
    }
    out
}

/// The comparison strings (see `comparison_string`) of every citation in a Markdown document, in
/// order, and of the bibliography entries the style prints for them. `nocite: '@*'` in the YAML
/// header lists every reference. This is what the benchmark and the golden tests compare.
pub fn comparison_for_document(md: &str, p: &Processor) -> (Vec<String>, Vec<String>) {
    let found = parse_citations(md);
    let cites: Vec<Citation> = found.iter().map(|f| f.citation.clone()).collect();
    let head = &md[..skip_front_matter(md)];
    let nocite = if head.contains("nocite") && head.contains("@*") { p.refs_in_order() } else { Vec::new() };
    p.process(&cites, &nocite).comparison(p.style())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bracketed_citations_with_prefix_locator_and_suppress() {
        let f = parse_citations("See [see @a; -@b, p. 4 and more; @c].");
        assert_eq!(f.len(), 1);
        let it = &f[0].citation.items;
        assert_eq!(it.len(), 3);
        assert_eq!((it[0].id.as_str(), it[0].prefix.as_str()), ("a", "see "));
        assert!(it[1].suppress_author);
        let (label, locator, rest) = parse_locator(&it[1].suffix);
        assert_eq!((label.as_deref(), locator.as_deref(), rest.trim()), (Some("page"), Some("4"), "and more"));
    }

    #[test]
    fn in_text_citations_and_non_citations() {
        let f = parse_citations("@smith2000 [p. 3] said mail me at a@b.com, `@no` and \\@no.");
        assert_eq!(f.len(), 1);
        assert!(f[0].citation.items[0].author_in_text);
        assert_eq!(parse_locator(&f[0].citation.items[0].suffix).1.as_deref(), Some("3"));
    }

    #[test]
    fn locator_labels_and_ranges() {
        assert_eq!(parse_locator(", chap. 2, pp. 3").0.as_deref(), Some("chapter"));
        assert_eq!(parse_locator(", p.3").1.as_deref(), Some("3"));
        let (l, loc, rest) = parse_locator(", pp. 33-35, with notes");
        assert_eq!((l.as_deref(), loc.as_deref()), (Some("page"), Some("33-35")));
        assert_eq!(rest.trim_start_matches([',', ' ']), "with notes");
        assert_eq!(parse_locator("see also").1, None);
    }
}
