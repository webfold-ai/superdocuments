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
    /// The suffix still holds the locator as written (`, p. 4`): the processor reads it with the
    /// labels of its style. Items built by hand leave this false and set `label` and `locator`.
    pub unparsed: bool,
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
    /// Reference ids in the order they were given (ties in sorting keep this order).
    input_order: Vec<String>,
    /// Per reference: names to show before "et al." after disambiguation.
    names_override: std::cell::RefCell<HashMap<String, usize>>,
    /// Pandoc's reading of `<names>` without `<name>` (see `Ctx::names_without_inheritance`).
    pandoc_names: bool,
    /// References whose cite is still ambiguous after the names step.
    ambiguous: std::cell::RefCell<std::collections::HashSet<String>>,
}

impl Processor {
    pub fn new(style: Style, references: Vec<Reference>) -> Self {
        let input_order = references.iter().map(|r| r.id.clone()).collect();
        let refs = references.into_iter().map(|r| (r.id.clone(), r)).collect();
        Processor { style, refs, input_order, pandoc_names: false, names_override: Default::default(), ambiguous: Default::default() }
    }

    /// Read `<names>` without a `<name>` child the way Pandoc does: with the CSL defaults, ignoring
    /// the name options set on `style`, `citation` or `bibliography`. Off by default: those options
    /// are meant to apply to the implicit name, and a style such as AMA relies on it.
    pub fn with_pandoc_names(mut self, on: bool) -> Self {
        self.pandoc_names = on;
        self
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    /// Every reference id in the order the references were given (for `nocite: @*`).
    pub fn refs_in_order(&self) -> Vec<String> {
        self.input_order.clone()
    }

    /// Formats the citations and the bibliography of the works they cite (plus `nocite`, the
    /// ids that are listed but not cited, in the given order).
    pub fn process(&self, citations: &[Citation], nocite: &[String]) -> Processed {
        let resolved: Vec<Citation> = citations.iter().map(|c| self.resolve_locators(c)).collect();
        let citations = &resolved[..];
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
            // Ties keep the order the references were given in.
            bib_order.sort_by_key(|id| self.input_order.iter().position(|x| x == id).unwrap_or(usize::MAX));
            self.sort_ids(&mut bib_order, sort, Mode::Bibliography, &numbers);
        }
        // Ambiguous cites: show more names, then year suffixes.
        self.names_override.borrow_mut().clear();
        self.ambiguous.borrow_mut().clear();
        self.disambiguate_names(&bib_order, &numbers);
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
                    ctx.names_without_inheritance = self.pandoc_names;
                    ctx.names_use_first_override = self.names_override.borrow().get(id).copied();
                    ctx.citation_number = numbers.get(id).copied();
                    ctx.year_suffix = suffixes.get(id).cloned();
                    let mut toks = if bib.attr("second-field-align").is_some() {
                        // The first field is the left margin: the rest is a box of its own.
                        let mut r = eval::R::default();
                        let mut done_first = false;
                        for c in layout.elems() {
                            let part = eval(c, &mut ctx);
                            let nonempty = part.toks.iter().any(|t| matches!(t, Tok::Text(x) if !x.is_empty()));
                            r.toks.extend(part.toks);
                            if nonempty && !done_first {
                                done_first = true;
                                r.toks.push(Tok::Text(" ".into()));
                            }
                        }
                        let mut wrapped = r;
                        decorate_layout(layout, &mut wrapped);
                        wrapped.toks
                    } else {
                        eval(layout, &mut ctx).toks
                    };
                    if let Some(sub) = sub {
                        let mut actx = Ctx::new(&self.style, r, Mode::Bibliography);
                        actx.names_without_inheritance = self.pandoc_names;
                        actx.names_use_first_override = ctx.names_use_first_override;
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

    /// Reads the locator out of each unparsed suffix. A word is a label when it is a locator
    /// term of the style's locale (any form, singular or plural), as in Pandoc.
    fn resolve_locators(&self, c: &Citation) -> Citation {
        const NAMES: &[&str] = &["number", "book", "chapter", "column", "figure", "folio", "issue", "line", "note", "opus", "page", "paragraph", "part", "section", "sub-verbo", "verse", "volume"];
        let lookup = |w: &str| -> Option<String> {
            let w = w.to_lowercase();
            for name in NAMES {
                for form in ["", "short", "symbol"] {
                    if let Some((s, p)) = self.style.terms.get(&(name.to_string(), form.to_string())) {
                        if s.to_lowercase() == w || p.to_lowercase() == w {
                            return Some((*name).to_string());
                        }
                    }
                }
            }
            None
        };
        let mut out = c.clone();
        for it in &mut out.items {
            if it.unparsed {
                let (label, locator, rest) = markdown::parse_locator_with(&it.suffix, &lookup);
                it.label = label;
                it.locator = locator;
                it.suffix = rest;
                it.unparsed = false;
            }
        }
        out
    }

    fn item_ctx<'a>(&'a self, r: &'a Reference, it: &CitationItem, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>, position: Position) -> Ctx<'a> {
        let mut ctx = Ctx::new(&self.style, r, Mode::Citation);
        ctx.names_without_inheritance = self.pandoc_names;
        ctx.names_use_first_override = self.names_override.borrow().get(&it.id).copied();
        ctx.disambiguate = self.ambiguous.borrow().contains(&it.id);
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

    /// The author part (for `@key`) and the cite proper of one item.
    fn render_item_parts(&self, it: &CitationItem, layout: &XElem, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>, position: Position) -> (Option<Toks>, Toks) {
        let Some(r) = self.refs.get(&it.id) else {
            return (None, vec![Tok::Text("???".into())]);
        };
        let (author, body) = if it.author_in_text {
            // The author (rendered alone), then the rest of the cite without the author.
            let mut actx = self.item_ctx(r, it, numbers, suffixes, position);
            actx.item.suppress_author = false;
            actx.item.author_only = true;
            let _ = eval_layout_items(layout, &mut actx);
            // A style whose cites print no author (numeric styles) prints none for `@key` either.
            let author = actx.captured.take();
            let mut rctx = self.item_ctx(r, it, numbers, suffixes, position);
            rctx.item.suppress_author = true;
            (author, eval_layout_items(layout, &mut rctx))
        } else {
            let mut ctx = self.item_ctx(r, it, numbers, suffixes, position);
            (None, eval_layout_items(layout, &mut ctx))
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
        (author, out)
    }

    fn render_item(&self, it: &CitationItem, layout: &XElem, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>, position: Position) -> Toks {
        let (author, body) = self.render_item_parts(it, layout, numbers, suffixes, position);
        let mut t = author.unwrap_or_default();
        if !t.is_empty() && !body.is_empty() {
            t.push(Tok::Text(" ".into()));
        }
        t.extend(body);
        t
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
        // `@key`: the author in the text and the layout (its brackets too) around the rest only.
        if let [it] = items[..] {
            if it.author_in_text {
                let position = if seen.contains(&it.id) { Position::Subsequent } else { Position::First };
                let (author, body) = self.render_item_parts(it, layout, numbers, suffixes, position);
                let mut r = eval::R { toks: body, called: 0, found: 0, hidden: false };
                decorate_layout(layout, &mut r);
                let mut out = author.unwrap_or_default();
                if !self.style.class_in_text {
                    // In a note style the rest goes to the note; what is left of it follows the
                    // author without its leading punctuation.
                    let mut rest = r.toks;
                    if let Some(Tok::Text(t)) = rest.iter_mut().find(|t| matches!(t, Tok::Text(x) if !x.is_empty())) {
                        *t = t.trim_start_matches([',', ';', ':', ' ']).to_string();
                    }
                    out.extend(rest);
                    return normalize(out);
                }
                if !out.is_empty() && !r.toks.is_empty() {
                    out.push(Tok::Text(" ".into()));
                }
                out.extend(r.toks);
                return normalize(out);
            }
        }
        let mut pieces: Vec<(Toks, &CitationItem)> = Vec::new();
        for it in &items {
            let position = if !seen.contains(&it.id) {
                Position::First
            } else if prev.len() == 1 && prev[0] == it.id && c.items.len() == 1 {
                if it.locator.is_some() { Position::IbidWithLocator } else { Position::Ibid }
            } else {
                Position::Subsequent
            };
            let toks = self.render_item(it, layout, numbers, suffixes, position);
            if !toks.is_empty() {
                pieces.push((toks, it));
            }
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
        let mut r = eval::R { toks: out, called: 0, found: 0, hidden: false };
        decorate_layout(layout, &mut r);
        let mut toks = normalize(r.toks);
        if !self.style.class_in_text {
            // A note starts a sentence: its first letter is a capital.
            if let Some(Tok::Text(t)) = toks.iter_mut().find(|t| matches!(t, Tok::Text(x) if !x.is_empty())) {
                if let Some(i) = t.char_indices().find(|(_, c)| c.is_alphabetic()).map(|(i, _)| i) {
                    if t[..i].chars().all(|c| !c.is_alphanumeric()) {
                        let c = t[i..].chars().next().unwrap_or(' ');
                        t.replace_range(i..i + c.len_utf8(), &c.to_uppercase().collect::<String>());
                    }
                }
            }
        }
        toks
    }

    #[allow(clippy::too_many_arguments)]
    fn collapse_years(&self, pieces: &[(Toks, &CitationItem)], layout: &XElem, cit: &XElem, delim: &str, mode: &str, numbers: &HashMap<String, usize>, suffixes: &HashMap<String, String>) -> Toks {
        // Cites whose author names print the same are moved next to each other (in the place of
        // the first). The names compared are those the layout prints directly, not through a
        // `substitute`.
        let author_key = |it: &CitationItem| -> String {
            let Some(r) = self.refs.get(&it.id) else { return String::new() };
            let mut ctx = self.item_ctx(r, it, numbers, suffixes, Position::First);
            ctx.item.suppress_author = false;
            let _ = eval_layout_items(layout, &mut ctx);
            match ctx.captured_direct {
                Some(t) => plain(&t),
                // Names that stand in through a `substitute` compare as the underlying names.
                None if r.names("author").is_empty() => String::new(),
                None => format!("{:?}", r.names("author")),
            }
        };
        let keys: Vec<String> = pieces.iter().map(|(_, it)| author_key(it)).collect();
        let mut order: Vec<usize> = Vec::new();
        for i in 0..pieces.len() {
            if order.contains(&i) {
                continue;
            }
            order.push(i);
            // A cite with a prefix or suffix of its own does not join the cites before it.
            let affixed = |it: &CitationItem| !it.prefix.is_empty() || !it.suffix.is_empty();
            if keys[i].is_empty() {
                continue;
            }
            for j in i + 1..pieces.len() {
                if !order.contains(&j) && keys[j] == keys[i] && !pieces[j].1.author_in_text && !affixed(pieces[j].1) {
                    order.push(j);
                }
            }
        }
        let authors: Vec<String> = order.iter().map(|i| keys[*i].clone()).collect();
        let pieces: Vec<(Toks, &CitationItem)> = order.iter().map(|i| (pieces[*i].0.clone(), pieces[*i].1)).collect();
        let pieces = &pieces[..];
        let group_delim = cit.attr("after-collapse-delimiter").unwrap_or(delim);
        let year_delim_attr = cit.attr("cite-group-delimiter");
        let year_delim = year_delim_attr.unwrap_or(", ");
        let mut out = Toks::new();
        let mut i = 0;
        let mut first_group = true;
        while i < pieces.len() {
            let mut j = i + 1;
            let affixed = |it: &CitationItem| !it.prefix.is_empty() || !it.suffix.is_empty();
            while j < pieces.len() && !authors[i].is_empty() && authors[j] == authors[i] && !pieces[j].1.author_in_text && !affixed(pieces[j].1) {
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
                let located = pieces[k].1.locator.is_some() || pieces[k - 1].1.locator.is_some();
                out.push(Tok::Text(if located && year_delim_attr.is_none() { delim.to_string() } else { year_delim.to_string() }));
                let same = mode != "year" && same_year(self.refs.get(&pieces[k - 1].1.id), Some(r));
                if same {
                    // After a year with a suffix, only the suffix letters follow.
                    if let (Some(d), Some(Tok::Text(last))) = (cit.attr("year-suffix-delimiter"), out.last_mut()) {
                        *last = d.to_string();
                    }
                }
                let rest = if same {
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
                        if std::env::var("CITEPROC_DEBUG").is_ok() {
                            eprintln!("sort {id} {m}: {:?}", plain(&toks));
                        }
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

    /// `disambiguate-add-names`: when two different references print the same cite, show more of
    /// the names of the colliding ones, one at a time, until the cites differ or the names run out.
    fn disambiguate_names(&self, bib_order: &[String], numbers: &HashMap<String, usize>) {
        let Some(cit) = self.style.citation() else { return };
        let Some(layout) = cit.find("layout") else { return };
        if cit.attr("disambiguate-add-names") != Some("true") {
            self.mark_ambiguous(layout, bib_order, numbers);
            return;
        }
        for _ in 0..30 {
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
            let mut changed = false;
            for (_, ids) in groups.iter().filter(|(_, v)| v.len() > 1) {
                for id in ids {
                    let Some(r) = self.refs.get(id) else { continue };
                    let total = r.names("author").len();
                    let current = self.names_override.borrow().get(id).copied();
                    let shown = current.unwrap_or_else(|| self.default_use_first(cit));
                    // Every member of an ambiguous group that still hides names shows one more.
                    if shown < total {
                        self.names_override.borrow_mut().insert(id.clone(), shown + 1);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        self.mark_ambiguous(layout, bib_order, numbers);
    }

    /// Fills `ambiguous` with the references that still print the same cite as another one.
    fn mark_ambiguous(&self, layout: &XElem, bib_order: &[String], numbers: &HashMap<String, usize>) {
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
        for (_, ids) in groups.into_iter().filter(|(_, v)| v.len() > 1) {
            self.ambiguous.borrow_mut().extend(ids);
        }
    }

    fn default_use_first(&self, cit: &XElem) -> usize {
        cit.attr("et-al-use-first").or_else(|| self.style.root.attr("et-al-use-first")).and_then(|v| v.parse().ok()).unwrap_or(1)
    }

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
