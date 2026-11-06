//! BibTeX writing and CSL-JSON export.
//!
//! `write` emits each field as `name = {raw}`. `raw` is balanced by construction, so the output
//! reads back to the same entries, `@string` definitions, preambles and comments. Built-in month
//! macros are not written, because every reader defines them.
//!
//! `csl_json` builds a CSL-JSON array with `crossref` inheritance. It maps the BibTeX entry type
//! to a CSL type, names to `family`/`given`/`non-dropping-particle`/`suffix`, institutions to
//! `literal`, and dates to `issued.date-parts`. Field values are decoded Unicode. CSL fields that
//! BibTeX has no equivalent for are left out.

use serde_json::{json, Map, Value};

use super::names::{parse_date, parse_names, parse_pages, Name, NameList};
use super::{Bibliography, Entry};

/// Writes BibTeX text for `bib`.
pub fn write(bib: &Bibliography) -> String {
    let mut out = String::new();
    for s in &bib.strings {
        out.push_str(&format!("@string{{{} = {{{}}}}}\n\n", s.name, s.raw));
    }
    for p in &bib.preambles {
        out.push_str(&format!("@preamble{{{{{p}}}}}\n\n"));
    }
    for c in &bib.comments {
        out.push_str(&format!("@comment{{{c}}}\n\n"));
    }
    for entry in &bib.entries {
        out.push_str(&format!("@{}{{{},\n", entry.entry_type, entry.key));
        for field in &entry.fields {
            out.push_str(&format!("  {} = {{{}}},\n", field.name, field.raw));
        }
        out.push_str("}\n\n");
    }
    out
}

/// CSL-JSON for every entry, as a JSON array.
pub fn csl_json(bib: &Bibliography) -> Value {
    Value::Array(bib.entries.iter().map(|e| csl_entry(bib, e)).collect())
}

fn csl_entry(bib: &Bibliography, entry: &Entry) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(entry.key));
    out.insert("type".into(), json!(csl_type(&entry.entry_type)));
    let text = |name: &str| bib.resolved_value(entry, name);
    if let Some(t) = text("title") {
        out.insert("title".into(), json!(t));
    }
    for (bib_field, csl_field) in [("author", "author"), ("editor", "editor")] {
        let names = resolved_names(bib, entry, bib_field);
        if !names.names.is_empty() || names.others {
            out.insert(csl_field.into(), names_to_csl(&names));
        }
    }
    if let Some(date) = issued(bib, entry) {
        out.insert("issued".into(), json!({ "date-parts": [date] }));
    }
    if let Some(pages) = bib.resolved_value(entry, "pages") {
        let ranges: Vec<String> = parse_pages(&pages)
            .into_iter()
            .map(|r| match r.end {
                Some(e) => format!("{}-{}", r.start, e),
                None => r.start,
            })
            .collect();
        if !ranges.is_empty() {
            out.insert("page".into(), json!(ranges.join(", ")));
        }
    }
    let container = match entry.entry_type.as_str() {
        "inproceedings" | "conference" | "incollection" | "inbook" => {
            bib.resolved_value(entry, "booktitle")
        }
        _ => bib.resolved_value(entry, "journal"),
    };
    if let Some(c) = container {
        out.insert("container-title".into(), json!(c));
    }
    let simple = [
        ("volume", "volume"),
        ("number", "issue"),
        ("publisher", "publisher"),
        ("address", "publisher-place"),
        ("edition", "edition"),
        ("series", "collection-title"),
        ("isbn", "ISBN"),
        ("issn", "ISSN"),
        ("abstract", "abstract"),
        ("note", "note"),
        ("language", "language"),
    ];
    for (bibtex, csl) in simple {
        if let Some(v) = text(bibtex) {
            if !v.trim().is_empty() {
                out.insert(csl.into(), json!(v));
            }
        }
    }
    if let Some(doi) = entry.doi() {
        out.insert("DOI".into(), json!(doi));
    }
    if let Some(url) = entry.url() {
        out.insert("URL".into(), json!(url));
    }
    if let Some(id) = entry.arxiv_id() {
        out.insert("number".into(), json!(format!("arXiv:{id}")));
    }
    Value::Object(out)
}

fn csl_type(bibtex: &str) -> &'static str {
    match bibtex {
        "article" => "article-journal",
        "book" | "booklet" | "proceedings" | "manual" => "book",
        "inbook" | "incollection" => "chapter",
        "inproceedings" | "conference" => "paper-conference",
        "phdthesis" | "mastersthesis" => "thesis",
        "techreport" => "report",
        "patent" => "patent",
        "online" | "electronic" | "misc" | "unpublished" => "article",
        _ => "article",
    }
}

/// The `[year, month, day]` for `issued`, from `date` or from `year`/`month`/`day`.
fn issued(bib: &Bibliography, entry: &Entry) -> Option<Vec<i64>> {
    if let Some(d) = bib
        .resolved_value(entry, "date")
        .and_then(|d| parse_date(&d))
    {
        let (start, _) = d;
        let mut parts = vec![i64::from(start.year)];
        if let Some(m) = start.month {
            parts.push(i64::from(m));
            if let Some(day) = start.day {
                parts.push(i64::from(day));
            }
        }
        return Some(parts);
    }
    let year = bib.resolved_value(entry, "year")?;
    let year: i64 = super::parse_year(&year)?.into();
    let mut parts = vec![year];
    if let Some(m) = bib
        .resolved_value(entry, "month")
        .and_then(|m| super::parse_month(&m))
    {
        parts.push(i64::from(m));
    }
    Some(parts)
}

/// The name list for `field`, following `crossref`. The raw text is used so that braces (and
/// so institutional names) survive.
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

fn names_to_csl(list: &NameList) -> Value {
    let mut items: Vec<Value> = list.names.iter().map(name_to_csl).collect();
    if list.others {
        items.push(json!({ "literal": "et al." }));
    }
    Value::Array(items)
}

fn name_to_csl(name: &Name) -> Value {
    if name.literal {
        return json!({ "literal": name.family });
    }
    let mut obj = Map::new();
    if !name.family.is_empty() {
        obj.insert("family".into(), json!(name.family));
    }
    if !name.given.is_empty() {
        obj.insert("given".into(), json!(name.given));
    }
    if !name.prefix.is_empty() {
        obj.insert("non-dropping-particle".into(), json!(name.prefix));
    }
    if !name.suffix.is_empty() {
        obj.insert("suffix".into(), json!(name.suffix));
    }
    Value::Object(obj)
}
