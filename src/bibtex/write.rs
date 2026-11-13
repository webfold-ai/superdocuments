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

/// CSL-JSON the way Pandoc reads a `.bib` file for citation processing, which is what a citation
/// style expects as input: titles in Title Case are put in sentence case (the style puts the case
/// it wants back), `howpublished`, `school` and `institution` become `publisher`, a thesis gets its
/// `genre`, `number` is `issue` for an article and `number` otherwise, and a BibTeX "von" part is a
/// dropping particle. A `misc` entry has an empty `type`, as in Pandoc.
/// Whether the file uses BibLaTeX-only fields or entry types (`location`, `journaltitle`, `date`,
/// `eventtitle`, `@online`, `@report`, ...), so that a reader should take it as BibLaTeX.
pub fn looks_like_biblatex(bib: &Bibliography) -> bool {
    const FIELDS: &[&str] = &["location", "journaltitle", "date", "eventtitle", "shortjournal", "origdate", "urldate", "eprint", "pagetotal", "booksubtitle"];
    const TYPES: &[&str] = &["online", "report", "thesis", "software", "dataset", "suppbook", "collection", "mvbook", "inreference", "reference"];
    bib.entries.iter().any(|e| TYPES.contains(&e.entry_type.as_str()) || FIELDS.iter().any(|f| e.field(f).is_some()))
}

pub fn csl_json_pandoc(bib: &Bibliography, biblatex: bool) -> Value {
    Value::Array(bib.entries.iter().map(|e| csl_entry_with(bib, e, if biblatex { 2 } else { 1 })).collect())
}

fn csl_entry(bib: &Bibliography, entry: &Entry) -> Value {
    csl_entry_with(bib, entry, 0)
}

/// Title Case to sentence case, if the title is in Title Case: every word after the first is put
/// in lower case except all-capital words, words with capitals inside and braced words, and the
/// word after `:`, `?`, `!` or `.` keeps its case.
fn sentence_case(title: &str) -> String {
    const SMALL: &[&str] = &["a", "an", "and", "as", "at", "but", "by", "for", "from", "in", "into", "nor", "of", "on", "or", "the", "to", "with", "via", "von", "van", "de"];
    let words: Vec<&str> = title.split(' ').collect();
    let is_title = words.iter().filter(|w| !w.is_empty()).all(|w| {
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric());
        bare.is_empty() || bare.starts_with(|c: char| c.is_uppercase() || !c.is_alphabetic()) || SMALL.contains(&bare.to_lowercase().as_str()) || w.contains('{')
    });
    if !is_title || words.iter().filter(|w| !w.is_empty()).count() < 2 {
        return title.to_string();
    }
    let mut out = Vec::new();
    let mut boundary = true;
    let mut depth = 0i32;
    for w in words {
        let protected = depth > 0 || w.contains('{');
        for c in w.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        let letters: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
        let all_caps = letters.len() > 1 && letters.iter().all(|c| c.is_uppercase());
        let inner_caps = letters.iter().skip(1).any(|c| c.is_uppercase());
        let keep = boundary || protected || all_caps || inner_caps;
        out.push(if keep { w.to_string() } else { w.to_lowercase() });
        boundary = w.ends_with([':', '?', '!', '.']) && !w.is_empty();
    }
    out.join(" ")
}

fn csl_type_pandoc(bibtex: &str) -> (&'static str, Option<&'static str>) {
    match bibtex {
        "article" => ("article-journal", None),
        "book" | "manual" | "proceedings" => ("book", None),
        "booklet" => ("pamphlet", None),
        "inbook" | "incollection" => ("chapter", None),
        "inproceedings" | "conference" => ("paper-conference", None),
        "phdthesis" => ("thesis", Some("PhD thesis")),
        "mastersthesis" => ("thesis", Some("Master's thesis")),
        "techreport" => ("report", None),
        "unpublished" => ("manuscript", None),
        "patent" => ("patent", None),
        "online" | "electronic" => ("webpage", None),
        _ => ("", None),
    }
}

/// `mode`: 0 the plain export, 1 Pandoc reading BibTeX, 2 Pandoc reading BibLaTeX (`.bib`).
fn csl_entry_with(bib: &Bibliography, entry: &Entry, mode: u8) -> Value {
    let pandoc = mode > 0;
    let mut out = Map::new();
    out.insert("id".into(), json!(entry.key));
    let (pandoc_type, genre) = csl_type_pandoc(&entry.entry_type);
    out.insert("type".into(), json!(if pandoc { pandoc_type } else { csl_type(&entry.entry_type) }));
    let text = |name: &str| bib.resolved_value(entry, name);
    if pandoc {
        if let Some(g) = genre {
            out.insert("genre".into(), json!(g));
        }
    }
    if let Some(t) = text("title") {
        out.insert("title".into(), json!(if pandoc { smart(&sentence_case(&t)) } else { t }));
    }
    for (bib_field, csl_field) in [("author", "author"), ("editor", "editor")] {
        let names = resolved_names(bib, entry, bib_field);
        if !names.names.is_empty() || names.others {
            out.insert(csl_field.into(), names_to_csl(&names, pandoc));
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
            bib.resolved_value(entry, "booktitle").map(|c| if pandoc { sentence_case(&c) } else { c })
        }
        _ => bib.resolved_value(entry, "journal").or_else(|| bib.resolved_value(entry, "journaltitle")),
    };
    if let Some(c) = container {
        out.insert("container-title".into(), json!(c));
    }
    if pandoc {
        // Where it was issued: the first of these that the entry has.
        for f in ["publisher", "school", "institution", "organization", "howpublished"] {
            if let Some(v) = text(f).filter(|v| !v.trim().is_empty()) {
                let v = v.trim().strip_prefix("\\url").map(|u| u.trim_start_matches('{').trim_end_matches('}').to_string()).unwrap_or(v);
                out.insert("publisher".into(), json!(v));
                break;
            }
        }
        if let Some(n) = text("number").filter(|n| !n.trim().is_empty()) {
            let key = if entry.entry_type == "article" { "issue" } else { "number" };
            out.insert(key.into(), json!(n));
        }
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
        if pandoc && (csl == "issue" || csl == "publisher") {
            continue;
        }
        if let Some(v) = text(bibtex).or_else(|| if bibtex == "address" && mode != 1 { text("location") } else { None }) {
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

fn names_to_csl(list: &NameList, pandoc: bool) -> Value {
    let mut items: Vec<Value> = list.names.iter().map(|n| name_to_csl(n, pandoc)).collect();
    if list.others {
        items.push(json!({ "literal": "et al." }));
    }
    Value::Array(items)
}

fn smart(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, ch) in c.iter().enumerate() {
        if *ch == '\'' && i > 0 && c[i - 1].is_alphanumeric() && c.get(i + 1).is_some_and(|n| n.is_alphanumeric()) {
            out.push('\u{2019}');
        } else {
            out.push(*ch);
        }
    }
    out
}

fn name_to_csl(name: &Name, pandoc: bool) -> Value {
    if name.literal {
        return json!({ "literal": name.family });
    }
    let mut obj = Map::new();
    if !name.family.is_empty() {
        obj.insert("family".into(), json!(if pandoc { smart(&name.family) } else { name.family.clone() }));
    }
    if !name.given.is_empty() {
        obj.insert("given".into(), json!(if pandoc { smart(&name.given) } else { name.given.clone() }));
    }
    if !name.prefix.is_empty() {
        obj.insert(if pandoc { "dropping-particle" } else { "non-dropping-particle" }.into(), json!(name.prefix));
    }
    if !name.suffix.is_empty() {
        obj.insert("suffix".into(), json!(name.suffix));
    }
    Value::Object(obj)
}
