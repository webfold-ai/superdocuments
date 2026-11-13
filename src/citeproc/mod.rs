//! A CSL citation processor: formats citations and a bibliography from a CSL 1.0.x style and
//! CSL-JSON (or BibTeX/BibLaTeX) references, with Pandoc's citation syntax for the input
//! (`[@key; @key2, p. 4]`, `-@key`, `@key [p. 3]`).
//!
//! Everything is local: the style is supplied by the caller, the English terms are built in, and
//! nothing is fetched. The result of `Processor::process` is token lists (`Tok`); `to_inlines` and
//! `comparison_string` turn them into the shared inline tree or a string for exact comparison.
//!
//! What is implemented: `text`, `number`, `label`, `date` (with ranges, seasons, locale date
//! forms and `date-part` overrides), `names` (et al., initials, particles, `and`, labels,
//! `substitute`), `group` (including the implicit suppression), `choose` (type, variable,
//! is-numeric, is-uncertain-date, locator, position), macros, sorting (citation and
//! bibliography), `disambiguate-add-year-suffix`, `collapse` (year, year-suffix and
//! citation-number), `subsequent-author-substitute`, text cases, quotes, affixes, font
//! attributes and page-range formats. Not implemented: `disambiguate-add-names` and
//! `-givenname`, `cite-group-delimiter`, `year-suffix-ranged`, `page-range-format` `expanded`
//! beyond en dashes for non-numeric ranges, locale files other than English, and ibid-style
//! positions for note styles beyond first and subsequent. See `docs/CITEPROC.md`.

mod eval;
pub mod markdown;
mod reference;
mod render;
mod style;
mod terms;

use std::collections::HashMap;

pub use eval::{plain, Fmt, Tok, Toks};
pub use reference::{references_from_json, DateValue, Field, Name, Reference};
pub use render::{comparison_string, normalize, normalize_comparison, to_inlines};
pub use style::Style;

use eval::{eval, Ctx, ItemCtx, Mode, Position};

use crate::xmltree::XElem;

/// One cited work in a citation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CitationItem {
    pub id: String,
    pub prefix: String,
    pub suffix: String,
    /// The locator label as a CSL term name (`page`, `chapter`, ...).
    pub label: Option<String>,
    pub locator: Option<String>,
    /// `-@key`: print the cite without the author.
    pub suppress_author: bool,
    /// `@key`: the author in the text, the rest in the usual place.
    pub author_in_text: bool,
}

/// A citation: one or more items cited together.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Citation {
    pub items: Vec<CitationItem>,
}

#[derive(Debug, Clone)]
pub struct BibEntry {
    pub id: String,
    pub toks: Toks,
}

#[derive(Debug, Clone, Default)]
pub struct Processed {
    /// One token list per input citation, in order.
    pub citations: Vec<Toks>,
    pub bibliography: Vec<BibEntry>,
}

pub struct Processor {
    style: Style,
    refs: HashMap<String, Reference>,
}

impl Processor {
    pub fn new(style: Style, references: Vec<Reference>) -> Self {
        let refs = references.into_iter().map(|r| (r.id.clone(), r)).collect();
        Processor { style, refs }
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    /// Every reference id, sorted (for `nocite: @*`; the bibliography order is the style's).
    pub fn refs_in_order(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.refs.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Formats the citations and the bibliography of the works they cite (plus `nocite`, the
    /// ids that are listed but not cited, in the given order).
    pub fn process(&self, citations: &[Citation], nocite: &[String]) -> Processed {
        // Order of first citation.
        let mut order: Vec<String> = Vec::new();
        for c in citations {
            for it in &c.items {
                if self.refs.contains_key(&it.id) && !order.contains(&it.id) {
                    order.push(it.id.clone());
                }
            }
        }
        let cited_count = order.len();
        for id in nocite {
            if self.refs.contains_key(id) && !order.contains(id) {
                order.push(id.clone());
            }
        }
        let numbers: HashMap<String, usize> = order.iter().enumerate().map(|(i, id)| (id.clone(), i + 1)).collect();
        // Bibliography order.
        let mut bib_order = order.clone();
        if let Some(sort) = self.style.bibliography().and_then(|b| b.find("sort")) {
            self.sort_ids(&mut bib_order, sort, Mode::Bibliography, &numbers);
        }
        // Year suffixes.
        let suffixes = self.year_suffixes(&bib_order[..], &numbers, cited_count.max(order.len()));
        // Citations.
        let mut seen: Vec<String> = Vec::new();
        let mut result = Processed::default();
        let mut prev_ids: Vec<String> = Vec::new();
        for c in citations {
            let toks = self.render_citation(c, &numbers, &suffixes, &seen, &prev_ids);
            for it in &c.items {
                if !seen.contains(&it.id) {
                    seen.push(it.id.clone());
                }
            }
            prev_ids = c.items.iter().map(|i| i.id.clone()).collect();
            result.citations.push(toks);
        }
        // Bibliography.
        if let Some(bib) = self.style.bibliography() {
            if let Some(layout) = bib.find("layout") {
                let mut prev_author: Option<Toks> = None;
                let sub = bib.attr("subsequent-author-substitute");
                for id in &bib_order {
                    let Some(r) = self.refs.get(id) else { continue };
                    let mut ctx = Ctx::new(&self.style, r, Mode::Bibliography);
                    ctx.citation_number = numbers.get(id).copied();
                    ctx.year_suffix = suffixes.get(id).cloned();
                    let mut toks = eval(layout, &mut ctx).toks;
                    if let Some(sub) = sub {
                        let mut actx = Ctx::new(&self.style, r, Mode::Bibliography);
                        actx.item.author_only = true;
                        actx.year_suffix = ctx.year_suffix.clone();
                        actx.citation_number = ctx.citation_number;
                        let _ = eval(layout, &mut actx);
                        let author = actx.captured.take();
                        if let (Some(a), Some(p)) = (&author, &prev_author) {
                            if plain(a) == plain(p) {
                                toks = replace_subsequence(toks, a, sub);
                            }
                        }
                        prev_author = author;
                    }
                    result.bibliography.push(BibEntry { id: id.clone(), toks: normalize(toks) });
                }
            }
        }
        result
    }

    fn item_ctx<'a>(&'a self, r: &'a Reference, it: &CitationItem, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>, position: Position) -> Ctx<'a> {
        let mut ctx = Ctx::new(&self.style, r, Mode::Citation);
        ctx.citation_number = numbers.get(&it.id).copied();
        ctx.year_suffix = suffixes.get(&it.id).cloned();
        ctx.item = ItemCtx {
            locator: it.locator.clone(),
            label: it.label.clone(),
            suppress_author: it.suppress_author,
            author_only: false,
            position,
        };
        ctx
    }

    fn render_item(&self, it: &CitationItem, layout: &XElem, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>, position: Position) -> Toks {
        let Some(r) = self.refs.get(&it.id) else {
            return vec![Tok::Text("???".into())];
        };
        let body = if it.author_in_text {
            // The author (rendered alone), then the rest of the cite without the author.
            let mut actx = self.item_ctx(r, it, numbers, suffixes, position);
            actx.item.suppress_author = false;
            actx.item.author_only = true;
            let _ = eval_layout_items(layout, &mut actx);
            let author = actx.captured.take().or_else(|| self.author_from_bibliography(r));
            let mut rctx = self.item_ctx(r, it, numbers, suffixes, position);
            rctx.item.suppress_author = true;
            let rest = eval_layout_items(layout, &mut rctx);
            let mut t = author.unwrap_or_default();
            if !t.is_empty() && !rest.is_empty() {
                t.push(Tok::Text(" ".into()));
            }
            t.extend(rest);
            t
        } else {
            let mut ctx = self.item_ctx(r, it, numbers, suffixes, position);
            eval_layout_items(layout, &mut ctx)
        };
        let mut out = Toks::new();
        if !it.prefix.is_empty() {
            let p = it.prefix.trim_end();
            out.push(Tok::Text(format!("{p} ")));
        }
        out.extend(body);
        if !it.suffix.is_empty() {
            let s = it.suffix.trim_start();
            let attach = s.starts_with([',', ';', ':', '.', ')', ']']);
            out.push(Tok::Text(if attach { s.to_string() } else { format!(" {s}") }));
        }
        out
    }

    /// For styles whose citations print no author (numeric styles): the names of the
    /// bibliography layout, which is what an in-text citation then shows.
    fn author_from_bibliography(&self, r: &Reference) -> Option<Toks> {
        let bib = self.style.bibliography()?;
        let layout = bib.find("layout")?;
        let mut ctx = Ctx::new(&self.style, r, Mode::Bibliography);
        ctx.item.author_only = true;
        let _ = eval(layout, &mut ctx);
        let cap = ctx.captured.take();
        // In-text names read "A and B", not in sort order, with et al. as the style sets it.
        cap
    }

    fn render_citation(&self, c: &Citation, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>, seen: &[String], prev: &[String]) -> Toks {
        let Some(cit) = self.style.citation() else { return Vec::new() };
        let Some(layout) = cit.find("layout") else { return Vec::new() };
        let mut items: Vec<&CitationItem> = c.items.iter().collect();
        if let Some(sort) = cit.find("sort") {
            let mut keyed: Vec<(Vec<SortKey>, &CitationItem)> = items
                .iter()
                .map(|it| (self.keys_for(&it.id, sort, Mode::Citation, numbers), *it))
                .collect();
            sort_keyed(&mut keyed, sort);
            items = keyed.into_iter().map(|(_, it)| it).collect();
        }
        let delim = layout.attr("delimiter").unwrap_or("");
        let mut pieces: Vec<(Toks, &CitationItem)> = Vec::new();
        for it in &items {
            let position = if !seen.contains(&it.id) {
                Position::First
            } else if prev.len() == 1 && prev[0] == it.id && c.items.len() == 1 {
                if it.locator.is_some() { Position::IbidWithLocator } else { Position::Ibid }
            } else {
                Position::Subsequent
            };
            pieces.push((self.render_item(it, layout, numbers, suffixes, position), it));
        }
        let collapse = cit.attr("collapse").unwrap_or("");
        let mut out: Toks = Vec::new();
        match collapse {
            "citation-number" => {
                out = collapse_numbers(&pieces, delim);
            }
            "year" | "year-suffix" | "year-suffix-ranged" => {
                out = self.collapse_years(&pieces, layout, cit, delim, collapse, numbers, suffixes);
            }
            _ => {
                for (i, (p, _)) in pieces.iter().enumerate() {
                    if i > 0 {
                        out.push(Tok::Text(delim.to_string()));
                    }
                    out.extend(p.clone());
                }
            }
        }
        if out.is_empty() {
            return out;
        }
        // The layout's own affixes and formatting wrap the whole citation.
        let mut r = eval::R { toks: out, called: 0, found: 0 };
        decorate_layout(layout, &mut r);
        normalize(r.toks)
    }

    #[allow(clippy::too_many_arguments)]
    fn collapse_years(&self, pieces: &[(Toks, &CitationItem)], layout: &XElem, cit: &XElem, delim: &str, mode: &str, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>) -> Toks {
        // Group consecutive items with the same author text.
        let mut authors: Vec<String> = Vec::new();
        for (_, it) in pieces {
            let a = self.refs.get(&it.id).map(|r| {
                let mut ctx = self.item_ctx(r, it, numbers, suffixes, Position::First);
                ctx.item.author_only = true;
                let _ = eval_layout_items(layout, &mut ctx);
                ctx.captured.map(|t| plain(&t)).unwrap_or_default()
            });
            authors.push(a.unwrap_or_default());
        }
        let group_delim = cit.attr("after-collapse-delimiter").unwrap_or(delim);
        let year_delim = cit.attr("cite-group-delimiter").unwrap_or(", ");
        let mut out = Toks::new();
        let mut i = 0;
        let mut first_group = true;
        while i < pieces.len() {
            let mut j = i + 1;
            while j < pieces.len() && !authors[i].is_empty() && authors[j] == authors[i] && !pieces[j].1.author_in_text {
                j += 1;
            }
            if !first_group {
                out.push(Tok::Text(group_delim.to_string()));
            }
            first_group = false;
            out.extend(pieces[i].0.clone());
            for k in i + 1..j {
                let (it, r) = (pieces[k].1, self.refs.get(&pieces[k].1.id));
                let Some(r) = r else { continue };
                let mut ctx = self.item_ctx(r, it, numbers, suffixes, Position::First);
                ctx.item.suppress_author = true;
                let rest = eval_layout_items(layout, &mut ctx);
                out.push(Tok::Text(year_delim.to_string()));
                let rest = if mode != "year" && same_year(self.refs.get(&pieces[k - 1].1.id), Some(r)) {
                    // Same year: only the suffix letter follows.
                    suffixes.get(&it.id).cloned().map(|s| text_toks(&s)).unwrap_or(rest)
                } else {
                    rest
                };
                out.extend(rest);
            }
            i = j;
        }
        out
    }

    // ---- sorting ----------------------------------------------------------------------------

    fn keys_for(&self, id: &str, sort: &XElem, mode: Mode, numbers: &HashMap<String, usize>) -> Vec<SortKey> {
        let Some(r) = self.refs.get(id) else { return Vec::new() };
        sort.elems()
            .filter(|k| k.name == "key")
            .map(|k| {
                if let Some(var) = k.attr("variable") {
                    return match var {
                        "citation-number" => SortKey::Num(numbers.get(id).copied().unwrap_or(0) as i64),
                        v => match r.fields.get(v) {
                            Some(Field::Date(d)) => {
                                let p = d.parts.first().copied().unwrap_or([0, 0, 0]);
                                SortKey::Num(i64::from(p[0]) * 10000 + i64::from(p[1]) * 100 + i64::from(p[2]))
                            }
                            Some(Field::Text(t)) if is_plain_number(t) => SortKey::Num(t.trim().parse().unwrap_or(0)),
                            Some(Field::Text(t)) => SortKey::Str(sort_string(t)),
                            Some(Field::Names(n)) => SortKey::Str(sort_string(&n.iter().map(|x| format!("{} {}", x.family, x.given)).collect::<Vec<_>>().join(" "))),
                            None => SortKey::Empty,
                        },
                    };
                }
                if let Some(m) = k.attr("macro") {
                    if let Some(mac) = self.style.macros.get(m) {
                        let mut ctx = Ctx::new(&self.style, r, mode);
                        ctx.citation_number = numbers.get(id).copied();
                        ctx.sorting = true;
                        let toks = eval::eval_children(mac, &mut ctx).toks;
                        let s = sort_string(&plain(&toks));
                        return if s.is_empty() { SortKey::Empty } else { SortKey::Str(s) };
                    }
                }
                SortKey::Empty
            })
            .collect()
    }

    fn sort_ids(&self, ids: &mut Vec<String>, sort: &XElem, mode: Mode, numbers: &HashMap<String, usize>) {
        let mut keyed: Vec<(Vec<SortKey>, String)> = ids.iter().map(|id| (self.keys_for(id, sort, mode, numbers), id.clone())).collect();
        sort_keyed(&mut keyed, sort);
        *ids = keyed.into_iter().map(|(_, id)| id).collect();
    }

    // ---- disambiguation ---------------------------------------------------------------------

    fn year_suffixes(&self, bib_order: &[String], numbers: &HashMap<String, usize>, _n: usize) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let Some(cit) = self.style.citation() else { return out };
        if cit.attr("disambiguate-add-year-suffix") != Some("true") {
            return out;
        }
        let Some(layout) = cit.find("layout") else { return out };
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for id in bib_order {
            let Some(r) = self.refs.get(id) else { continue };
            let it = CitationItem { id: id.clone(), ..Default::default() };
            let mut ctx = self.item_ctx(r, &it, numbers, &HashMap::new(), Position::First);
            let key = plain(&eval_layout_items(layout, &mut ctx));
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(id.clone()),
                None => groups.push((key, vec![id.clone()])),
            }
        }
        for (_, ids) in groups {
            if ids.len() < 2 {
                continue;
            }
            for (i, id) in ids.iter().enumerate() {
                out.insert(id.clone(), letters(i));
            }
        }
        out
    }
}

fn same_year(a: Option<&Reference>, b: Option<&Reference>) -> bool {
    match (a.and_then(|r| r.date("issued")), b.and_then(|r| r.date("issued"))) {
        (Some(x), Some(y)) => x.year() == y.year(),
        _ => false,
    }
}

fn text_toks(s: &str) -> Toks {
    vec![Tok::Text(s.to_string())]
}

fn letters(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s
}

/// Evaluates the children of a layout without applying the layout's own affixes (they wrap the
/// whole citation).
fn eval_layout_items(layout: &XElem, ctx: &mut Ctx) -> Toks {
    eval::eval_children(layout, ctx).toks
}

fn decorate_layout(layout: &XElem, r: &mut eval::R) {
    eval::decorate_public(layout, r);
}

fn replace_subsequence(toks: Toks, needle: &Toks, with: &str) -> Toks {
    let n = needle.len();
    if n == 0 || toks.len() < n {
        return toks;
    }
    for i in 0..=toks.len() - n {
        if toks[i..i + n] == needle[..] {
            let mut out: Toks = toks[..i].to_vec();
            out.push(Tok::Text(with.to_string()));
            out.extend(toks[i + n..].iter().cloned());
            return out;
        }
    }
    toks
}

fn collapse_numbers(pieces: &[(Toks, &CitationItem)], delim: &str) -> Toks {
    let nums: Vec<Option<u64>> = pieces.iter().map(|(t, _)| plain(t).trim().parse::<u64>().ok()).collect();
    let mut out = Toks::new();
    let mut i = 0;
    let mut first = true;
    while i < pieces.len() {
        let mut j = i;
        if let Some(n) = nums[i] {
            while j + 1 < pieces.len() && nums[j + 1] == Some(nums[j].unwrap_or(0) + 1) && nums[j].is_some() {
                j += 1;
            }
            let _ = n;
        }
        if !first {
            out.push(Tok::Text(delim.to_string()));
        }
        first = false;
        if j >= i + 2 {
            out.extend(pieces[i].0.clone());
            out.push(Tok::Text("\u{2013}".into()));
            out.extend(pieces[j].0.clone());
            i = j + 1;
        } else {
            out.extend(pieces[i].0.clone());
            i += 1;
        }
    }
    out
}

// ---- sort keys --------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum SortKey {
    Str(String),
    Num(i64),
    Empty,
}

fn is_plain_number(s: &str) -> bool {
    !s.trim().is_empty() && s.trim().chars().all(|c| c.is_ascii_digit())
}

/// A sort string: lower case, letters and digits and single spaces only.
fn sort_string(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(fold(c).to_lowercase());
        } else if c.is_whitespace() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        c => c,
    }
}

fn sort_keyed<T>(items: &mut [(Vec<SortKey>, T)], sort: &XElem) {
    let dirs: Vec<bool> = sort.elems().filter(|k| k.name == "key").map(|k| k.attr("sort") == Some("descending")).collect();
    items.sort_by(|a, b| {
        for (i, desc) in dirs.iter().enumerate() {
            let (x, y) = (a.0.get(i).cloned().unwrap_or(SortKey::Empty), b.0.get(i).cloned().unwrap_or(SortKey::Empty));
            use std::cmp::Ordering::*;
            let o = match (&x, &y) {
                (SortKey::Empty, SortKey::Empty) => Equal,
                (SortKey::Empty, _) => Greater,
                (_, SortKey::Empty) => Less,
                (SortKey::Num(p), SortKey::Num(q)) => if *desc { q.cmp(p) } else { p.cmp(q) },
                (SortKey::Str(p), SortKey::Str(q)) => if *desc { q.cmp(p) } else { p.cmp(q) },
                (SortKey::Num(_), SortKey::Str(_)) => Less,
                (SortKey::Str(_), SortKey::Num(_)) => Greater,
            };
            if o != Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    });
}

/// Quote marks of the style (English).
pub fn quote_marks(style: &Style) -> ((String, String), (String, String)) {
    let t = |n: &str, d: &str| style.term(n, "long", false).unwrap_or_else(|| d.to_string());
    (
        (t("open-quote", "\u{201c}"), t("close-quote", "\u{201d}")),
        (t("open-inner-quote", "\u{2018}"), t("close-inner-quote", "\u{2019}")),
    )
}

impl Processed {
    /// The comparison strings of the citations and of the bibliography entries.
    pub fn comparison(&self, style: &Style) -> (Vec<String>, Vec<String>) {
        let (o, i) = quote_marks(style);
        let conv = |t: &Toks| comparison_string(&to_inlines(t, (&o.0, &o.1), (&i.0, &i.1)));
        (self.citations.iter().map(conv).collect(), self.bibliography.iter().map(|e| conv(&e.toks)).collect())
    }
}
