//! A readable Markdown bibliography. This is a plain reference list, not a citation-style
//! processor: one fixed layout, with `crossref` inheritance applied. For styled output, export
//! `csl_json` and give it to a CSL processor.
//!
//! One entry is one line:
//! `Authors (Year). Title. *Container* volume(number), pages. Publisher. <link>`
//! Parts the entry does not have are left out. An entry with no title (for example one made
//! from a `\bibitem`) shows its `note` text instead.

use super::{parse_names, parse_pages, Bibliography, Entry, Name, NameList};

/// Every entry of the bibliography as a Markdown list, in file order. Each item starts with
/// the citation key in backticks. Returns an empty string for an empty bibliography.
pub fn render_markdown(bib: &Bibliography) -> String {
    let mut out = String::new();
    for entry in &bib.entries {
        out.push_str(&format!(
            "- `{}` — {}\n",
            entry.key.replace('`', "'"),
            reference_markdown(bib, entry)
        ));
    }
    out
}

/// One reference as a line of Markdown, without the key.
pub fn reference_markdown(bib: &Bibliography, entry: &Entry) -> String {
    let text = |name: &str| {
        bib.resolved_value(entry, name)
            .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|v| !v.is_empty())
    };
    let mut parts: Vec<String> = Vec::new();

    let mut who = names_text(&resolved_names(bib, entry, "author"));
    if who.is_empty() {
        let editors = resolved_names(bib, entry, "editor");
        who = names_text(&editors);
        if !who.is_empty() {
            who.push_str(if editors.names.len() > 1 || editors.others {
                " (eds.)"
            } else {
                " (ed.)"
            });
        }
    }
    let year = entry
        .year()
        .map(|y| y.to_string())
        .or_else(|| text("year"))
        .or_else(|| {
            text("date")
                .and_then(|d| super::parse_date(&d))
                .map(|d| d.0.year.to_string())
        });
    match (who.is_empty(), &year) {
        (false, Some(y)) => parts.push(format!("{} ({y}).", escape(&who))),
        (false, None) => parts.push(format!("{}.", escape(who.trim_end_matches('.')))),
        (true, Some(y)) => parts.push(format!("({y}).")),
        (true, None) => {}
    }

    let title = text("title");
    if let Some(title) = &title {
        parts.push(sentence(&escape(title)));
    }

    let container = match entry.entry_type.as_str() {
        "inproceedings" | "conference" | "incollection" | "inbook" => {
            text("booktitle").or_else(|| text("journal"))
        }
        "phdthesis" | "mastersthesis" | "thesis" => text("school").or_else(|| text("institution")),
        "techreport" | "report" => text("institution"),
        _ => text("journal")
            .or_else(|| text("journaltitle"))
            .or_else(|| text("booktitle"))
            .or_else(|| text("howpublished")),
    };
    let mut venue = String::new();
    if let Some(c) = container {
        venue.push_str(&format!("*{}*", escape(&c)));
    }
    if let Some(v) = text("volume") {
        if !venue.is_empty() {
            venue.push(' ');
        }
        venue.push_str(&escape(&v));
        if let Some(n) = text("number").or_else(|| text("issue")) {
            venue.push_str(&format!("({})", escape(&n)));
        }
    }
    let pages: Vec<String> = text("pages")
        .map(|p| parse_pages(&p))
        .unwrap_or_default()
        .into_iter()
        .map(|r| match r.end {
            Some(e) => format!("{}\u{2013}{}", r.start, e),
            None => r.start,
        })
        .collect();
    if !pages.is_empty() {
        if !venue.is_empty() {
            venue.push_str(", ");
        }
        venue.push_str(&escape(&pages.join(", ")));
    }
    if !venue.is_empty() {
        parts.push(sentence(&venue));
    }
    if let Some(p) = text("publisher") {
        parts.push(sentence(&escape(&p)));
    }
    if title.is_none() {
        if let Some(note) = text("note") {
            parts.push(sentence(&escape(&note)));
        }
    }
    if let Some(id) = entry.arxiv_id() {
        parts.push(format!("arXiv:{}.", escape(&id)));
    }
    if let Some(doi) = entry.doi() {
        parts.push(format!("<https://doi.org/{doi}>"));
    } else if let Some(url) = entry.url() {
        parts.push(format!("<{url}>"));
    }
    if parts.is_empty() {
        return escape(&entry.key);
    }
    parts.join(" ")
}

/// `author` or `editor`, following `crossref` like `Bibliography::resolved_value`.
fn resolved_names(bib: &Bibliography, entry: &Entry, field: &str) -> NameList {
    let mut current = entry;
    for _ in 0..8 {
        if let Some(f) = current.field(field) {
            return parse_names(&f.raw);
        }
        let Some(parent) = current.value("crossref").and_then(|k| bib.lookup(k)) else {
            break;
        };
        current = parent;
    }
    NameList::default()
}

fn name_text(name: &Name) -> String {
    if name.literal {
        return name.family.clone();
    }
    let mut out = String::new();
    for part in [&name.given, &name.prefix, &name.family] {
        if !part.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(part);
        }
    }
    if !name.suffix.is_empty() {
        out.push_str(", ");
        out.push_str(&name.suffix);
    }
    out
}

/// "A", "A and B", "A, B and C"; `and others` adds "et al.".
fn names_text(list: &NameList) -> String {
    let names: Vec<String> = list
        .names
        .iter()
        .map(name_text)
        .filter(|n| !n.is_empty())
        .collect();
    let mut out = match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n if list.others => names[..n].join(", "),
        n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
    };
    if list.others && !out.is_empty() {
        out.push_str(" et al.");
    }
    out
}

/// Ends a part with a full stop unless it already ends a sentence.
fn sentence(text: &str) -> String {
    let trimmed = text.trim_end();
    let last = trimmed.trim_end_matches(['*', '"', '\u{201d}', ')']);
    if last.ends_with(['.', '?', '!']) {
        trimmed.to_string()
    } else {
        format!("{trimmed}.")
    }
}

/// Escapes Markdown punctuation outside `$...$` math, which is kept as written.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut math = false;
    for c in text.chars() {
        if c == '$' {
            math = !math;
            out.push(c);
            continue;
        }
        if !math && matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
