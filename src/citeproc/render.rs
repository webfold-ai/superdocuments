//! Token lists to text: the shared inline tree (`ast::Inline`), and a comparison string in which
//! formatting is marked with private-use characters so two renderings can be compared exactly.

use super::eval::{Fmt, Tok, Toks};
use crate::ast::Inline;

fn visible_last(out: &[Tok]) -> Option<char> {
    for t in out.iter().rev() {
        if let Tok::Text(s) = t {
            if let Some(c) = s.chars().last() {
                return Some(c);
            }
        }
    }
    None
}

/// Tidies a token list the way citation processors do: adjacent text joined, a full stop never
/// doubled and dropped after `?` or `!`, a comma or full stop after a closing quote moved inside
/// it (American punctuation), repeated spaces collapsed, ends trimmed.
pub fn normalize(toks: Toks) -> Toks {
    let mut out: Toks = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Text(s) => {
                let mut s = s.clone();
                if s.is_empty() {
                    i += 1;
                    continue;
                }
                // Moving punctuation inside a quote that just closed.
                if matches!(out.last(), Some(Tok::Close(Fmt::Quote))) && (s.starts_with(',') || s.starts_with('.')) {
                    let c = s.remove(0);
                    let n = out.len() - 1;
                    let dup = visible_last(&out[..n]) == Some('.') && c == '.';
                    if !dup {
                        out.insert(n, Tok::Text(c.to_string()));
                    }
                    if s.is_empty() {
                        i += 1;
                        continue;
                    }
                }
                if s.starts_with('.') {
                    if let Some(l) = visible_last(&out) {
                        if l == '.' || l == '?' || l == '!' {
                            s.remove(0);
                        }
                    }
                }
                if s.is_empty() {
                    i += 1;
                    continue;
                }
                if let Some(Tok::Text(prev)) = out.last_mut() {
                    prev.push_str(&s);
                } else {
                    out.push(Tok::Text(s));
                }
            }
            t => out.push(t.clone()),
        }
        i += 1;
    }
    // Collapse repeated spaces, also across marks.
    let mut prev_space = true;
    for t in out.iter_mut() {
        if let Tok::Text(s) = t {
            let mut n = String::new();
            for c in s.chars() {
                let sp = c == ' ' || c == '\u{a0}' && false;
                if sp && prev_space {
                    continue;
                }
                prev_space = sp;
                n.push(c);
            }
            *s = n;
        }
    }
    // Trim the end.
    while let Some(Tok::Text(s)) = out.last_mut() {
        let t = s.trim_end().to_string();
        if t.is_empty() {
            out.pop();
        } else {
            *s = t;
            break;
        }
    }
    out
}

/// Builds the inline tree. Nested italics flip to upright, as CSL asks.
pub fn to_inlines(toks: &[Tok], open_quote: (&str, &str), inner_quote: (&str, &str)) -> Vec<Inline> {
    struct Frame {
        kind: Option<Fmt>,
        items: Vec<Inline>,
    }
    let mut stack: Vec<Frame> = vec![Frame { kind: None, items: Vec::new() }];
    let mut italic_depth = 0usize;
    let mut bold_depth = 0usize;
    let mut quote_depth = 0usize;
    // Frames that were opened as no-ops (a nested italic) are tracked so closes match.
    for t in toks {
        match t {
            Tok::Text(s) => stack.last_mut().unwrap().items.push(Inline::Text(s.clone())),
            Tok::Open(f) => match f {
                Fmt::Italic if italic_depth % 2 == 1 => {
                    italic_depth += 1;
                    stack.push(Frame { kind: None, items: Vec::new() });
                }
                Fmt::Bold if bold_depth % 2 == 1 => {
                    bold_depth += 1;
                    stack.push(Frame { kind: None, items: Vec::new() });
                }
                Fmt::Italic => {
                    italic_depth += 1;
                    stack.push(Frame { kind: Some(Fmt::Italic), items: Vec::new() });
                }
                Fmt::Bold => {
                    bold_depth += 1;
                    stack.push(Frame { kind: Some(Fmt::Bold), items: Vec::new() });
                }
                Fmt::Quote => {
                    quote_depth += 1;
                    let q = if quote_depth % 2 == 1 { open_quote.0 } else { inner_quote.0 };
                    stack.push(Frame { kind: Some(Fmt::Quote), items: vec![Inline::Text(q.to_string())] });
                }
                other => stack.push(Frame { kind: Some(*other), items: Vec::new() }),
            },
            Tok::Close(f) => {
                if stack.len() < 2 {
                    continue;
                }
                let fr = stack.pop().unwrap();
                match f {
                    Fmt::Italic => italic_depth = italic_depth.saturating_sub(1),
                    Fmt::Bold => bold_depth = bold_depth.saturating_sub(1),
                    _ => {}
                }
                let mut items = fr.items;
                let node = match fr.kind {
                    None | Some(Fmt::Upright) | Some(Fmt::LightWeight) => {
                        stack.last_mut().unwrap().items.extend(items);
                        continue;
                    }
                    Some(Fmt::Italic) => Inline::Emph(items),
                    Some(Fmt::Bold) => Inline::Strong(items),
                    Some(Fmt::SmallCaps) => Inline::SmallCaps(items),
                    Some(Fmt::Underline) => Inline::Underline(items),
                    Some(Fmt::Sup) => Inline::Sup(items),
                    Some(Fmt::Sub) => Inline::Sub(items),
                    Some(Fmt::Quote) => {
                        let q = if quote_depth % 2 == 1 { open_quote.1 } else { inner_quote.1 };
                        quote_depth = quote_depth.saturating_sub(1);
                        items.push(Inline::Text(q.to_string()));
                        stack.last_mut().unwrap().items.extend(items);
                        continue;
                    }
                };
                stack.last_mut().unwrap().items.push(node);
            }
        }
    }
    while stack.len() > 1 {
        let fr = stack.pop().unwrap();
        stack.last_mut().unwrap().items.extend(fr.items);
    }
    merge_text(stack.pop().unwrap().items)
}

fn merge_text(items: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for i in items {
        match (out.last_mut(), i) {
            (Some(Inline::Text(a)), Inline::Text(b)) => a.push_str(&b),
            (_, i) => out.push(i),
        }
    }
    out
}

const I_OPEN: char = '\u{e001}';
const I_CLOSE: char = '\u{e002}';
const B_OPEN: char = '\u{e003}';
const B_CLOSE: char = '\u{e004}';
const SC_OPEN: char = '\u{e005}';
const SC_CLOSE: char = '\u{e006}';
const SUP_OPEN: char = '\u{e007}';
const SUP_CLOSE: char = '\u{e008}';
const SUB_OPEN: char = '\u{e009}';
const SUB_CLOSE: char = '\u{e00a}';

/// The comparison string of an inline tree: text with formatting marks, whitespace runs
/// (non-breaking spaces included) as one space, adjacent identical marks merged.
pub fn comparison_string(inlines: &[Inline]) -> String {
    let mut s = String::new();
    write_cmp(inlines, &mut s);
    normalize_comparison(&s)
}

fn write_cmp(inlines: &[Inline], out: &mut String) {
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Verbatim(t) | Inline::Cite(t) | Inline::Code(t) => out.push_str(t),
            Inline::Emph(v) => wrap(out, I_OPEN, I_CLOSE, v),
            Inline::Strong(v) => wrap(out, B_OPEN, B_CLOSE, v),
            Inline::SmallCaps(v) => wrap(out, SC_OPEN, SC_CLOSE, v),
            Inline::Sup(v) => wrap(out, SUP_OPEN, SUP_CLOSE, v),
            Inline::Sub(v) => wrap(out, SUB_OPEN, SUB_CLOSE, v),
            Inline::Underline(v) | Inline::Strike(v) => write_cmp(v, out),
            Inline::Link { inlines, .. } => write_cmp(inlines, out),
            Inline::SoftBreak | Inline::LineBreak => out.push(' '),
            Inline::Image { alt, .. } => write_cmp(alt, out),
            Inline::Math { tex, .. } => out.push_str(tex),
            Inline::Note(_) => {}
        }
    }
}

fn wrap(out: &mut String, o: char, c: char, v: &[Inline]) {
    out.push(o);
    write_cmp(v, out);
    out.push(c);
}

/// Merges `close open` pairs of the same kind, collapses whitespace and trims.
pub fn normalize_comparison(s: &str) -> String {
    let mut t = s.replace(['\u{a0}', '\u{2009}', '\u{202f}'], " ");
    for (o, c) in [(I_OPEN, I_CLOSE), (B_OPEN, B_CLOSE), (SC_OPEN, SC_CLOSE), (SUP_OPEN, SUP_CLOSE), (SUB_OPEN, SUB_CLOSE)] {
        let pair = format!("{c}{o}");
        while t.contains(&pair) {
            t = t.replace(&pair, "");
        }
        // Empty formatting disappears.
        let empty = format!("{o}{c}");
        t = t.replace(&empty, "");
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}
