//! BibTeX and BibLaTeX reading and writing.
//!
//! `parse` reads a `.bib` file into a `Bibliography`: entries, `@string` definitions, `@preamble`
//! and `@comment` blocks, and warnings. It never fails and never panics. Damage is reported in
//! `Bibliography::warnings`, and everything that could be read is kept. `write` produces BibTeX
//! text that reads back to the same entries, strings, preambles and comments.
//!
//! Each `Field` keeps two values. `raw` is the value as written, with `@string` names and `#`
//! concatenation expanded and the outer delimiters removed, but accents and braces left alone.
//! `value` is `raw` decoded to Unicode with `tex_text`, braces removed. Names, pages and dates
//! are parsed from `raw` (names need the braces) or from `value` as documented in `names`.
//!
//! `csl_json` exports CSL-JSON (the citation-processor input format) with `crossref` inheritance
//! applied. `Bibliography::resolved_value` gives the same inheritance for any field.
//! `render_markdown` writes a plain, readable reference list (one fixed layout, not a citation
//! style); `document::convert_bytes("bib", ..)` returns it.
//!
//! Limits (`Limits`) bound input size, entry and field counts, value size and brace nesting.
//! The defaults are generous for real bibliographies; hitting a limit stops parsing with a
//! warning and keeps what was read.

mod names;
mod parse;
mod render;
mod write;

#[cfg(test)]
mod tests;

pub use names::{
    normalize_doi, normalize_url, parse_date, parse_month, parse_name, parse_names, parse_pages,
    parse_year, Date, Name, NameList, PageRange,
};
pub use parse::parse_with;
pub use render::{reference_markdown, render_markdown};
pub use write::{csl_json, write};

use crate::tex_text::{self, Ligatures};

/// Bounds for reading. Defaults: 64 MiB input, 100,000 entries, 1,000 fields per entry, 1 MiB
/// per value, 64 levels of brace nesting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_entries: usize,
    pub max_fields: usize,
    pub max_value_bytes: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 << 20,
            max_entries: 100_000,
            max_fields: 1_000,
            max_value_bytes: 1 << 20,
            max_depth: 64,
        }
    }
}

/// A parsed `.bib` file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bibliography {
    pub entries: Vec<Entry>,
    /// `@string` definitions in source order. The twelve month macros are built in and are not
    /// listed here.
    pub strings: Vec<StringDef>,
    /// Raw text of each `@preamble`.
    pub preambles: Vec<String>,
    /// Raw text of each `@comment` block.
    pub comments: Vec<String>,
    /// Recoverable problems, in input order.
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringDef {
    pub name: String,
    pub raw: String,
    pub value: String,
}

/// One `@type{key, ...}` entry. `entry_type` is lower-cased; `key` is as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub entry_type: String,
    pub key: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Lower-cased field name.
    pub name: String,
    /// The value with `@string` and `#` expanded and the outer delimiters removed. Braces and
    /// TeX commands are kept as written.
    pub raw: String,
    /// `raw` decoded to Unicode (braces removed, accents and symbols converted).
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Byte offset in the input where the problem was found.
    pub offset: usize,
    pub message: String,
}

impl Entry {
    /// The field with this name (case-insensitive), without `crossref` inheritance.
    pub fn field(&self, name: &str) -> Option<&Field> {
        let lower = name.to_ascii_lowercase();
        self.fields.iter().find(|f| f.name == lower)
    }

    /// The decoded value of a field, without `crossref` inheritance.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.field(name).map(|f| f.value.as_str())
    }

    /// Authors from the `author` field, parsed from the raw text.
    pub fn authors(&self) -> NameList {
        self.name_list("author")
    }

    /// Editors from the `editor` field, parsed from the raw text.
    pub fn editors(&self) -> NameList {
        self.name_list("editor")
    }

    fn name_list(&self, field: &str) -> NameList {
        self.field(field)
            .map(|f| parse_names(&f.raw))
            .unwrap_or_default()
    }

    /// Page ranges from the `pages` field.
    pub fn pages(&self) -> Vec<PageRange> {
        self.value("pages").map(parse_pages).unwrap_or_default()
    }

    /// The year: `year`, or the year of `date`.
    pub fn year(&self) -> Option<i32> {
        self.value("year")
            .and_then(parse_year)
            .or_else(|| self.value("date").and_then(parse_date).map(|d| d.0.year))
    }

    /// The month number (1 to 12) from `month`, or from the start of `date`.
    pub fn month(&self) -> Option<u8> {
        self.value("month").and_then(parse_month).or_else(|| {
            self.value("date")
                .and_then(parse_date)
                .and_then(|d| d.0.month)
        })
    }

    /// The normalised DOI (without the `https://doi.org/` prefix).
    pub fn doi(&self) -> Option<String> {
        self.value("doi").and_then(normalize_doi)
    }

    /// The normalised URL.
    pub fn url(&self) -> Option<String> {
        self.value("url").and_then(normalize_url)
    }

    /// The arXiv identifier, from `eprint` when `eprinttype` or `archiveprefix` says arXiv.
    /// The `arXiv:` prefix is removed.
    pub fn arxiv_id(&self) -> Option<String> {
        let kind = self
            .value("eprinttype")
            .or_else(|| self.value("archiveprefix"))?;
        if !kind.trim().eq_ignore_ascii_case("arxiv") {
            return None;
        }
        let id = self.value("eprint")?.trim();
        let id = id
            .strip_prefix("arXiv:")
            .or_else(|| id.strip_prefix("arxiv:"))
            .unwrap_or(id)
            .trim();
        (!id.is_empty()).then(|| id.to_string())
    }
}

impl Bibliography {
    /// The first entry with this key (case-insensitive, as BibTeX compares keys).
    pub fn lookup(&self, key: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(key))
    }

    /// The decoded value of `name` for `entry`, following `crossref` when the entry does not
    /// have the field. Chains are followed to 8 levels, and a cycle stops the search.
    pub fn resolved_value(&self, entry: &Entry, name: &str) -> Option<String> {
        let mut current = entry;
        for _ in 0..8 {
            if let Some(v) = current.value(name) {
                return Some(v.to_string());
            }
            let parent = current.value("crossref")?;
            current = self.lookup(parent)?;
        }
        None
    }
}

/// Decodes raw field text as BibTeX does (no TeX ligatures).
pub(crate) fn decode_value(raw: &str) -> String {
    tex_text::decode(raw, Ligatures::None)
}

/// The twelve month macros every BibTeX style defines.
pub(crate) const MONTHS: [(&str, &str); 12] = [
    ("jan", "January"),
    ("feb", "February"),
    ("mar", "March"),
    ("apr", "April"),
    ("may", "May"),
    ("jun", "June"),
    ("jul", "July"),
    ("aug", "August"),
    ("sep", "September"),
    ("oct", "October"),
    ("nov", "November"),
    ("dec", "December"),
];

/// Parses BibTeX with the default limits.
pub fn parse(input: &str) -> Bibliography {
    parse_with(input, &Limits::default())
}
