//! Evaluating a CSL style against one reference: the output is a flat token list (text and
//! formatting open/close marks), which `render` turns into the shared inline tree.

use std::collections::HashSet;

use super::reference::{DateValue, Field, Name, Reference};
use super::style::{text_of, Style};
use crate::xmltree::{XElem, XNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fmt {
    Italic,
    Upright,
    Bold,
    LightWeight,
    SmallCaps,
    Underline,
    Sup,
    Sub,
    Quote,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Text(String),
    Open(Fmt),
    Close(Fmt),
}

pub type Toks = Vec<Tok>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Citation,
    Bibliography,
}

/// What a citation item says about the cite beyond the reference.
#[derive(Debug, Clone, Default)]
pub struct ItemCtx {
    pub locator: Option<String>,
    pub label: Option<String>,
    pub suppress_author: bool,
    pub author_only: bool,
    pub position: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    First,
    Subsequent,
    Ibid,
    IbidWithLocator,
}

pub struct Ctx<'a> {
    pub style: &'a Style,
    pub reference: &'a Reference,
    pub mode: Mode,
    pub item: ItemCtx,
    pub citation_number: Option<usize>,
    pub year_suffix: Option<String>,
    /// Variables already used by a `substitute` and so not printed again.
    pub suppressed: HashSet<String>,
    /// Name-list overrides from disambiguation: names to show instead of et al.
    pub names_min_override: Option<usize>,
    depth: usize,
    /// Set while rendering the author part for `author_only` (names output is kept, the rest
    /// dropped) or the rest for `suppress_author` (names output is dropped).
    names_seen: usize,
    /// With `item.author_only`: the output of the first author `names` element.
    pub captured: Option<Toks>,
    /// Rendering a sort key: names are in sort order.
    pub sorting: bool,
}

impl<'a> Ctx<'a> {
    pub fn new(style: &'a Style, reference: &'a Reference, mode: Mode) -> Self {
        Ctx {
            style,
            reference,
            mode,
            item: ItemCtx::default(),
            citation_number: None,
            year_suffix: None,
            suppressed: HashSet::new(),
            names_min_override: None,
            depth: 0,
            names_seen: 0,
            captured: None,
            sorting: false,
        }
    }
}

/// The result of evaluating one element.
#[derive(Debug, Default, Clone)]
pub struct R {
    pub toks: Toks,
    /// Variables the element (or a macro it called) looked at, and how many had a value.
    pub called: usize,
    pub found: usize,
}

impl R {
    fn empty(&self) -> bool {
        !self.toks.iter().any(|t| matches!(t, Tok::Text(s) if !s.is_empty()))
    }
    fn absorb_counts(&mut self, o: &R) {
        self.called += o.called;
        self.found += o.found;
    }
}

fn text(s: impl Into<String>) -> Toks {
    vec![Tok::Text(s.into())]
}

// ---- element evaluation -----------------------------------------------------------------------

/// Evaluates the children of `layout` (or any container) in order, concatenated.
pub fn eval_children(e: &XElem, ctx: &mut Ctx) -> R {
    let mut out = R::default();
    for c in e.elems() {
        if matches!(c.name.as_str(), "sort" | "key" | "info" | "locale") {
            continue;
        }
        let r = eval(c, ctx);
        out.absorb_counts(&r);
        out.toks.extend(r.toks);
    }
    out
}

pub fn eval(e: &XElem, ctx: &mut Ctx) -> R {
    if ctx.depth > 60 {
        return R::default();
    }
    ctx.depth += 1;
    let mut r = match e.name.as_str() {
        "text" => eval_text(e, ctx),
        "number" => eval_number(e, ctx),
        "label" => eval_label(e, ctx),
        "date" => eval_date(e, ctx),
        "names" => eval_names(e, ctx),
        "group" => eval_group(e, ctx),
        "choose" => eval_choose(e, ctx),
        "layout" => eval_children(e, ctx),
        _ => R::default(),
    };
    ctx.depth -= 1;
    decorate(e, &mut r, ctx);
    r
}

pub(crate) fn decorate_public(e: &XElem, r: &mut R) {
    // `decorate` only needs the context for future per-style options.
    decorate_nocontext(e, r)
}

fn decorate_nocontext(e: &XElem, r: &mut R) {
    if r.empty() {
        r.toks.clear();
        return;
    }
    let mut toks = std::mem::take(&mut r.toks);
    let wrap = |toks: &mut Toks, f: Fmt| {
        toks.insert(0, Tok::Open(f));
        toks.push(Tok::Close(f));
    };
    match e.attr("font-style") {
        Some("italic") | Some("oblique") => wrap(&mut toks, Fmt::Italic),
        _ => {}
    }
    if e.attr("font-weight") == Some("bold") {
        wrap(&mut toks, Fmt::Bold);
    }
    match e.attr("vertical-align") {
        Some("sup") => wrap(&mut toks, Fmt::Sup),
        Some("sub") => wrap(&mut toks, Fmt::Sub),
        _ => {}
    }
    if let Some(p) = e.attr("prefix") {
        toks.insert(0, Tok::Text(p.to_string()));
    }
    if let Some(s) = e.attr("suffix") {
        toks.push(Tok::Text(s.to_string()));
    }
    r.toks = toks;
}

/// Quotes, text case, strip-periods, font and the affixes, in that order of nesting.
fn decorate(e: &XElem, r: &mut R, ctx: &Ctx) {
    if r.empty() {
        r.toks.clear();
        return;
    }
    if e.attr("strip-periods") == Some("true") {
        for t in r.toks.iter_mut() {
            if let Tok::Text(s) = t {
                *s = s.replace('.', "");
            }
        }
    }
    if let Some(tc) = e.attr("text-case") {
        apply_case(&mut r.toks, tc);
    }
    let mut toks = std::mem::take(&mut r.toks);
    if e.attr("quotes") == Some("true") {
        toks.insert(0, Tok::Open(Fmt::Quote));
        toks.push(Tok::Close(Fmt::Quote));
    }
    let wrap = |toks: &mut Toks, f: Fmt| {
        toks.insert(0, Tok::Open(f));
        toks.push(Tok::Close(f));
    };
    match e.attr("font-style") {
        Some("italic") | Some("oblique") => wrap(&mut toks, Fmt::Italic),
        Some("normal") => wrap(&mut toks, Fmt::Upright),
        _ => {}
    }
    match e.attr("font-weight") {
        Some("bold") => wrap(&mut toks, Fmt::Bold),
        Some("normal") => wrap(&mut toks, Fmt::LightWeight),
        _ => {}
    }
    if e.attr("font-variant") == Some("small-caps") {
        wrap(&mut toks, Fmt::SmallCaps);
    }
    if e.attr("text-decoration") == Some("underline") {
        wrap(&mut toks, Fmt::Underline);
    }
    match e.attr("vertical-align") {
        Some("sup") => wrap(&mut toks, Fmt::Sup),
        Some("sub") => wrap(&mut toks, Fmt::Sub),
        _ => {}
    }
    let _ = ctx;
    if let Some(p) = e.attr("prefix") {
        toks.insert(0, Tok::Text(p.to_string()));
    }
    if let Some(s) = e.attr("suffix") {
        toks.push(Tok::Text(s.to_string()));
    }
    r.toks = toks;
}

fn plural_attr(e: &XElem, count: usize) -> bool {
    match e.attr("plural") {
        Some("always") => true,
        Some("never") => false,
        _ => count > 1,
    }
}

fn eval_text(e: &XElem, ctx: &mut Ctx) -> R {
    let mut r = R::default();
    if let Some(name) = e.attr("macro") {
        let Some(m) = ctx.style.macros.get(name).cloned() else { return r };
        return eval_children(&m, ctx);
    }
    if let Some(v) = e.attr("value") {
        r.toks = text(v);
        return r;
    }
    if let Some(t) = e.attr("term") {
        let form = e.attr("form").unwrap_or("long");
        let plural = e.attr("plural") == Some("true");
        if let Some(s) = ctx.style.term(t, form, plural) {
            r.toks = text(s);
        }
        return r;
    }
    if let Some(var) = e.attr("variable") {
        r.called = 1;
        let form = e.attr("form").unwrap_or("long");
        if let Some(s) = variable_text(var, form, ctx) {
            r.found = 1;
            r.toks = text_with_markup(&s);
        }
    }
    r
}

/// CSL text values may carry simple HTML-like markup (`<i>`, `<b>`, `<sup>`, `<sub>`,
/// `<span class="nocase">`), as Pandoc's citeproc reads it.
fn text_with_markup(s: &str) -> Toks {
    if !s.contains('<') {
        return text(s);
    }
    let mut out = Toks::new();
    let mut buf = String::new();
    let mut rest = s;
    let flush = |buf: &mut String, out: &mut Toks| {
        if !buf.is_empty() {
            out.push(Tok::Text(std::mem::take(buf)));
        }
    };
    while let Some(p) = rest.find('<') {
        buf.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(q) = rest.find('>') else { break };
        let tag = &rest[1..q];
        let consumed = match tag {
            "i" | "em" => Some(Tok::Open(Fmt::Italic)),
            "/i" | "/em" => Some(Tok::Close(Fmt::Italic)),
            "b" | "strong" => Some(Tok::Open(Fmt::Bold)),
            "/b" | "/strong" => Some(Tok::Close(Fmt::Bold)),
            "sup" => Some(Tok::Open(Fmt::Sup)),
            "/sup" => Some(Tok::Close(Fmt::Sup)),
            "sub" => Some(Tok::Open(Fmt::Sub)),
            "/sub" => Some(Tok::Close(Fmt::Sub)),
            "sc" => Some(Tok::Open(Fmt::SmallCaps)),
            "/sc" => Some(Tok::Close(Fmt::SmallCaps)),
            t if t.starts_with("span") || t == "/span" => {
                flush(&mut buf, &mut out);
                rest = &rest[q + 1..];
                continue;
            }
            _ => None,
        };
        match consumed {
            Some(t) => {
                flush(&mut buf, &mut out);
                out.push(t);
            }
            None => buf.push_str(&rest[..=q]),
        }
        rest = &rest[q + 1..];
    }
    buf.push_str(rest);
    flush(&mut buf, &mut out);
    out
}

fn is_suppressed(var: &str, ctx: &Ctx) -> bool {
    ctx.suppressed.contains(var)
}

/// The text of a variable for `<text variable>`: the reference field, or one the processor
/// supplies (`locator`, `citation-number`, `year-suffix`).
fn variable_text(var: &str, form: &str, ctx: &mut Ctx) -> Option<String> {
    if is_suppressed(var, ctx) {
        return None;
    }
    match var {
        "locator" => {
            return if ctx.mode == Mode::Citation { ctx.item.locator.clone().filter(|l| !l.is_empty()).map(|l| format_page_ranges(&l, "expanded")) } else { None };
        }
        "citation-number" => return ctx.citation_number.map(|n| n.to_string()),
        "year-suffix" => return ctx.year_suffix.clone(),
        "citation-label" => return Some(citation_label(ctx.reference)),
        "first-reference-note-number" => return None,
        _ => {}
    }
    let r = ctx.reference;
    if form == "short" {
        let short = format!("{var}-short");
        if let Some(t) = r.text(&short) {
            return Some(t.to_string());
        }
        if var == "title" || var == "container-title" {
            // No short form recorded: the long form is used.
        }
    }
    match r.fields.get(var) {
        Some(Field::Text(t)) if !t.is_empty() => {
            if var == "page" {
                let fmt = ctx.style.inherited("page-range-format", &[]).unwrap_or("");
                let fmt = ctx.style.citation().filter(|_| ctx.mode == Mode::Citation).and_then(|c| c.attr("page-range-format")).or(ctx.style.bibliography().filter(|_| ctx.mode == Mode::Bibliography).and_then(|c| c.attr("page-range-format"))).unwrap_or(fmt);
                return Some(format_page_ranges(t, fmt));
            }
            Some(t.clone())
        }
        Some(Field::Date(d)) if !d.is_empty() => Some(date_plain(d)),
        Some(Field::Names(n)) if !n.is_empty() => Some(n.iter().map(|x| name_display(x, false, "", false)).collect::<Vec<_>>().join(", ")),
        _ => None,
    }
}

fn citation_label(r: &Reference) -> String {
    let fam = r.names("author").first().map(|n| n.family.clone()).unwrap_or_else(|| r.id.clone());
    let year = r.date("issued").and_then(|d| d.year()).map(|y| format!("{:02}", y % 100)).unwrap_or_default();
    format!("{}{}", fam.chars().take(3).collect::<String>(), year)
}

fn date_plain(d: &DateValue) -> String {
    if !d.literal.is_empty() {
        return d.literal.clone();
    }
    d.parts.iter().map(|p| p.iter().filter(|x| **x != 0).map(|x| x.to_string()).collect::<Vec<_>>().join("-")).collect::<Vec<_>>().join("/")
}

// ---- numbers, pages ---------------------------------------------------------------------------

fn is_numeric_value(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    // A number, or a run of numbers joined by ", ", "-", "–" or "&"; a leading or trailing letter
    // is allowed on each (`S12`, `4a`).
    t.split(|c: char| c == ',' || c == '&' || c == '-' || c == '\u{2013}')
        .map(str::trim)
        .all(|part| {
            !part.is_empty()
                && part.chars().any(|c| c.is_ascii_digit())
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
                && (part.chars().all(|c| c.is_ascii_digit())
                    || part.trim_start_matches(|c: char| c.is_ascii_alphabetic()).chars().all(|c| c.is_ascii_digit())
                    || part.trim_end_matches(|c: char| c.is_ascii_alphabetic()).chars().all(|c| c.is_ascii_digit()))
        })
}

fn ordinal(n: u32, style: &Style) -> String {
    let two = n % 100;
    let key = if (11..=13).contains(&two) {
        format!("ordinal-{two}")
    } else {
        format!("ordinal-{:02}", n % 10)
    };
    let suffix = style
        .term(&key, "long", false)
        .or_else(|| style.term("ordinal", "long", false))
        .unwrap_or_default();
    format!("{n}{suffix}")
}

fn roman(mut n: u32) -> String {
    let t = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in t {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn eval_number(e: &XElem, ctx: &mut Ctx) -> R {
    let mut r = R::default();
    let Some(var) = e.attr("variable") else { return r };
    r.called = 1;
    let raw = match var {
        "citation-number" => ctx.citation_number.map(|n| n.to_string()),
        "locator" => ctx.item.locator.clone(),
        _ => variable_text(var, "long", ctx),
    };
    let Some(raw) = raw else { return r };
    r.found = 1;
    let form = e.attr("form").unwrap_or("numeric");
    if !is_numeric_value(&raw) || form == "numeric" {
        r.toks = text(page_dash(&raw));
        return r;
    }
    let convert = |part: &str| -> String {
        match part.trim().parse::<u32>() {
            Ok(n) => match form {
                "ordinal" => ordinal(n, ctx.style),
                "long-ordinal" if (1..=10).contains(&n) => ctx.style.term(&format!("long-ordinal-{n:02}"), "long", false).unwrap_or_else(|| ordinal(n, ctx.style)),
                "long-ordinal" => ordinal(n, ctx.style),
                "roman" => roman(n),
                _ => n.to_string(),
            },
            Err(_) => part.trim().to_string(),
        }
    };
    let mut out = String::new();
    let mut cur = String::new();
    for ch in raw.chars() {
        if ch == '-' || ch == '\u{2013}' || ch == ',' || ch == '&' {
            out.push_str(&convert(&cur));
            cur.clear();
            out.push(if ch == '-' { '\u{2013}' } else { ch });
            if ch == ',' || ch == '&' {
                out.push(' ');
            }
        } else if !(ch == ' ' && cur.is_empty()) {
            cur.push(ch);
        }
    }
    out.push_str(&convert(&cur));
    r.toks = text(out);
    r
}

/// A hyphen between digits becomes an en dash (the page-range-delimiter term).
fn page_dash(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == '-' && i > 0 && i + 1 < chars.len() && chars[i - 1].is_ascii_alphanumeric() && chars[i + 1].is_ascii_alphanumeric() {
            out.push('\u{2013}');
        } else if *c == '-' && i > 0 && i + 1 < chars.len() && chars[i - 1] == ' ' {
            out.push('\u{2013}');
        } else {
            out.push(*c);
        }
    }
    out
}

pub(crate) fn format_page_ranges(s: &str, format: &str) -> String {
    let s = s.replace("--", "-").replace('\u{2014}', "-");
    let mut out = String::new();
    // Split on commas and ampersands, keeping them.
    let mut piece = String::new();
    let flush = |piece: &mut String, out: &mut String| {
        if piece.is_empty() {
            return;
        }
        let p = std::mem::take(piece);
        let t = p.trim();
        let lead = &p[..p.len() - p.trim_start().len()];
        out.push_str(lead);
        let dash = t.find(|c| c == '-' || c == '\u{2013}');
        match dash {
            Some(i) if i > 0 => {
                let (a, b) = (t[..i].trim(), t[i + 1..].trim());
                out.push_str(&format_range(a, b, format));
            }
            _ => out.push_str(t),
        }
        let trail = &p[p.trim_end().len()..];
        out.push_str(trail);
    };
    for ch in s.chars() {
        if ch == ',' || ch == '&' {
            flush(&mut piece, &mut out);
            out.push(ch);
        } else {
            piece.push(ch);
        }
    }
    flush(&mut piece, &mut out);
    out
}

fn format_range(a: &str, b: &str, format: &str) -> String {
    let dash = '\u{2013}';
    let (ad, bd) = (a.chars().all(|c| c.is_ascii_digit()), b.chars().all(|c| c.is_ascii_digit()));
    if !(ad && bd) || a.is_empty() || b.is_empty() {
        return format!("{a}{dash}{b}");
    }
    let (na, nb) = (a.parse::<u64>().unwrap_or(0), b.parse::<u64>().unwrap_or(0));
    if nb < na && b.len() < a.len() {
        // `321-28` means 321-328: restore the dropped leading digits first.
        let full = format!("{}{}", &a[..a.len() - b.len()], b);
        return format_range(a, &full, format);
    }
    let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let tail = |keep: usize| -> String {
        let cut = common.min(b.len().saturating_sub(keep));
        b[cut..].to_string()
    };
    match format {
        "minimal" => format!("{a}{dash}{}", tail(1)),
        "minimal-two" => format!("{a}{dash}{}", tail(2)),
        "chicago" | "chicago-15" => {
            if a.len() < 3 || na % 100 == 0 {
                format!("{a}{dash}{b}")
            } else if na % 100 >= 1 && na % 100 <= 9 {
                format!("{a}{dash}{}", tail(1))
            } else {
                format!("{a}{dash}{}", tail(2))
            }
        }
        "chicago-16" => {
            if a.len() < 3 || na % 100 == 0 {
                format!("{a}{dash}{b}")
            } else if na % 100 >= 1 && na % 100 <= 9 {
                format!("{a}{dash}{}", tail(1))
            } else {
                format!("{a}{dash}{}", tail(2))
            }
        }
        _ => format!("{a}{dash}{b}"),
    }
}

// ---- labels -----------------------------------------------------------------------------------

fn eval_label(e: &XElem, ctx: &mut Ctx) -> R {
    let mut r = R::default();
    let Some(var) = e.attr("variable") else { return r };
    let form = e.attr("form").unwrap_or("long");
    let (value, term_name) = match var {
        "locator" => {
            if ctx.mode != Mode::Citation {
                return r;
            }
            let Some(loc) = ctx.item.locator.clone().filter(|l| !l.is_empty()) else { return r };
            (loc, ctx.item.label.clone().unwrap_or_else(|| "page".into()))
        }
        "citation-number" => (ctx.citation_number.map(|n| n.to_string()).unwrap_or_default(), "citation-number".to_string()),
        _ => (ctx.reference.text(var).map(str::to_string).unwrap_or_default(), var.to_string()),
    };
    if value.is_empty() {
        return r;
    }
    let plural = match e.attr("plural") {
        Some("always") => true,
        Some("never") => false,
        _ => label_is_plural(var, &value),
    };
    if let Some(t) = ctx.style.term(&term_name, form, plural) {
        r.toks = text(t);
    }
    r
}

/// A label is plural when the value is a range or a list, or (for `number-of-pages` and the
/// like) a number other than one.
fn label_is_plural(var: &str, value: &str) -> bool {
    match var {
        "number-of-pages" | "number-of-volumes" => value.trim().parse::<u64>().map(|n| n != 1).unwrap_or(false),
        _ => {
            let digits = value.chars().filter(|c| c.is_ascii_digit()).count();
            digits > 0 && value.contains(|c: char| c == '-' || c == '\u{2013}' || c == ',' || c == '&')
        }
    }
}

// ---- groups and conditions --------------------------------------------------------------------

fn eval_group(e: &XElem, ctx: &mut Ctx) -> R {
    let delim = e.attr("delimiter").unwrap_or("");
    let mut out = R::default();
    let mut first = true;
    for c in e.elems() {
        let r = eval(c, ctx);
        out.absorb_counts(&r);
        if r.empty() {
            continue;
        }
        if !first && !delim.is_empty() {
            out.toks.push(Tok::Text(delim.to_string()));
        }
        first = false;
        out.toks.extend(r.toks);
    }
    // A group that asked for variables and found none is dropped.
    if out.called > 0 && out.found == 0 {
        out.toks.clear();
    }
    out
}

fn cond_values(attr: Option<&str>) -> Vec<&str> {
    attr.map(|a| a.split_whitespace().collect()).unwrap_or_default()
}

fn test_element(c: &XElem, ctx: &Ctx) -> Option<bool> {
    let any_of = |vals: Vec<&str>, f: &dyn Fn(&str) -> bool| -> (bool, bool) {
        (vals.iter().any(|v| f(v)), vals.iter().all(|v| f(v)))
    };
    let mode = c.attr("match").unwrap_or("all");
    let mut results: Vec<bool> = Vec::new();
    let combine = |(any, all): (bool, bool)| -> bool {
        match mode {
            "any" => any,
            "none" => !any,
            _ => all,
        }
    };
    let mut has = false;
    if let Some(t) = c.attr("type") {
        has = true;
        results.push(combine(any_of(cond_values(Some(t)), &|v| ctx.reference.kind == v)));
    }
    if let Some(t) = c.attr("variable") {
        has = true;
        results.push(combine(any_of(cond_values(Some(t)), &|v| variable_present(v, ctx))));
    }
    if let Some(t) = c.attr("is-numeric") {
        has = true;
        results.push(combine(any_of(cond_values(Some(t)), &|v| {
            variable_raw(v, ctx).is_some_and(|s| is_numeric_value(&s))
        })));
    }
    if let Some(t) = c.attr("is-uncertain-date") {
        has = true;
        results.push(combine(any_of(cond_values(Some(t)), &|v| ctx.reference.date(v).is_some_and(|d| d.circa))));
    }
    if let Some(t) = c.attr("locator") {
        has = true;
        results.push(combine(any_of(cond_values(Some(t)), &|v| {
            ctx.item.locator.is_some() && ctx.item.label.as_deref().unwrap_or("page") == v
        })));
    }
    if let Some(t) = c.attr("position") {
        has = true;
        results.push(combine(any_of(cond_values(Some(t)), &|v| match v {
            "first" => ctx.item.position == Position::First,
            "subsequent" => ctx.item.position != Position::First,
            "ibid" => matches!(ctx.item.position, Position::Ibid | Position::IbidWithLocator),
            "ibid-with-locator" => ctx.item.position == Position::IbidWithLocator,
            _ => false,
        })));
    }
    if let Some(t) = c.attr("disambiguate") {
        has = true;
        let _ = t;
        results.push(combine((false, false)));
    }
    if c.attr("object-type").is_some() {
        has = true;
        results.push(false);
    }
    if !has {
        return None;
    }
    Some(match mode {
        "any" => results.iter().any(|b| *b),
        "none" => results.iter().all(|b| *b),
        _ => results.iter().all(|b| *b),
    })
}

fn variable_raw(v: &str, ctx: &Ctx) -> Option<String> {
    match v {
        "locator" => ctx.item.locator.clone(),
        "citation-number" => ctx.citation_number.map(|n| n.to_string()),
        "year-suffix" => ctx.year_suffix.clone(),
        _ => match ctx.reference.fields.get(v) {
            Some(Field::Text(t)) => Some(t.clone()),
            _ => None,
        },
    }
}

fn variable_present(v: &str, ctx: &Ctx) -> bool {
    if ctx.suppressed.contains(v) {
        return false;
    }
    match v {
        "locator" => ctx.mode == Mode::Citation && ctx.item.locator.as_deref().is_some_and(|l| !l.is_empty()),
        "citation-number" => ctx.citation_number.is_some(),
        "year-suffix" => ctx.year_suffix.is_some(),
        _ => match ctx.reference.fields.get(v) {
            Some(Field::Text(t)) => !t.is_empty(),
            Some(Field::Names(n)) => !n.is_empty(),
            Some(Field::Date(d)) => !d.is_empty(),
            None => false,
        },
    }
}

fn eval_choose(e: &XElem, ctx: &mut Ctx) -> R {
    for c in e.elems() {
        match c.name.as_str() {
            "if" | "else-if" => {
                if test_element(c, ctx).unwrap_or(false) {
                    return eval_children(c, ctx);
                }
            }
            "else" => return eval_children(c, ctx),
            _ => {}
        }
    }
    R::default()
}

// ---- text case --------------------------------------------------------------------------------

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "down", "for", "from", "in", "into", "nor", "of", "on", "onto", "or",
    "over", "so", "the", "till", "to", "up", "via", "with", "yet",
];

fn capitalize_word(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub(crate) fn apply_case(toks: &mut Toks, case: &str) {
    let mut at_start = true;
    let mut after_colon = false;
    for t in toks.iter_mut() {
        let Tok::Text(s) = t else { continue };
        *s = match case {
            "lowercase" => s.to_lowercase(),
            "uppercase" => s.to_uppercase(),
            "capitalize-first" => {
                if at_start {
                    capitalize_word(s)
                } else {
                    s.clone()
                }
            }
            "capitalize-all" => split_words(s, |w, _| capitalize_word(w)),
            "title" => {
                let mut first = at_start || after_colon;
                let out = split_words(s, |w, idx| {
                    let lw = w.to_lowercase();
                    let stop = STOP_WORDS.contains(&lw.as_str());
                    let leading_first = first && idx == 0;
                    let r = if w.chars().skip(1).any(|c| c.is_uppercase()) || w.chars().any(|c| c.is_ascii_digit()) {
                        w.to_string()
                    } else if leading_first || !stop {
                        capitalize_word(w)
                    } else {
                        lw
                    };
                    first = false;
                    r
                });
                out
            }
            "sentence" => {
                if at_start {
                    capitalize_word(s)
                } else {
                    s.clone()
                }
            }
            _ => s.clone(),
        };
        if !s.is_empty() {
            at_start = false;
            after_colon = s.trim_end().ends_with(':');
        }
    }
}

/// Applies `f` to each whitespace-separated word (hyphenated parts are separate words).
fn split_words(s: &str, mut f: impl FnMut(&str, usize) -> String) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let mut idx = 0;
    for ch in s.chars() {
        if ch.is_whitespace() || ch == '-' || ch == '\u{2013}' || ch == '/' {
            if !word.is_empty() {
                out.push_str(&f(&word, idx));
                word.clear();
                idx += 1;
            }
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        out.push_str(&f(&word, idx));
    }
    out
}

// ---- dates ------------------------------------------------------------------------------------

fn month_term(style: &Style, m: i32, form: &str) -> String {
    style.term(&format!("month-{m:02}"), if form == "short" { "short" } else { "long" }, false).unwrap_or_default()
}

fn eval_date(e: &XElem, ctx: &mut Ctx) -> R {
    let mut r = R::default();
    let Some(var) = e.attr("variable") else { return r };
    r.called = 1;
    if ctx.suppressed.contains(var) {
        return r;
    }
    let Some(d) = ctx.reference.date(var).cloned() else { return r };
    r.found = 1;
    if !d.literal.is_empty() && d.parts.is_empty() {
        r.toks = text(d.literal.clone());
        return r;
    }
    let form = e.attr("form");
    // The date parts to print, with their per-part attributes.
    let (parts_attr, templates): (String, Vec<XElem>) = match form {
        Some(f) => {
            let base = ctx.style.date_formats.get(f).cloned();
            let mut tpl: Vec<XElem> = match &base {
                Some(b) => b.elems().filter(|x| x.name == "date-part").cloned().collect(),
                None => builtin_date_parts(f),
            };
            // `<date-part>` children of the calling element override the locale's.
            for over in e.elems().filter(|x| x.name == "date-part") {
                if let Some(t) = tpl.iter_mut().find(|t| t.attr("name") == over.attr("name")) {
                    for (k, v) in &over.attrs {
                        if let Some(slot) = t.attrs.iter_mut().find(|(a, _)| a == k) {
                            slot.1 = v.clone();
                        } else {
                            t.attrs.push((k.clone(), v.clone()));
                        }
                    }
                }
            }
            (e.attr("date-parts").unwrap_or("year-month-day").to_string(), tpl)
        }
        None => ("year-month-day".to_string(), e.elems().filter(|x| x.name == "date-part").cloned().collect()),
    };
    let allowed: Vec<&str> = parts_attr.split('-').collect();
    let templates: Vec<XElem> = templates.into_iter().filter(|t| t.attr("name").is_some_and(|n| allowed.contains(&n))).collect();
    let delim = e.attr("delimiter").unwrap_or("");
    let one = |p: &[i32; 3], season: Option<i32>, skip_year: bool, ctx: &Ctx| -> Toks {
        let mut out: Toks = Vec::new();
        let mut first = true;
        for t in &templates {
            let name = t.attr("name").unwrap_or("");
            let mut piece: Option<Toks> = None;
            match name {
                "year" if !skip_year && p[0] != 0 => {
                    let mut y = p[0].abs().to_string();
                    if t.attr("form") == Some("short") {
                        y = format!("{:02}", p[0].abs() % 100);
                    }
                    if p[0] < 0 {
                        y.push_str(ctx.style.term("bc", "long", false).unwrap_or_default().as_str());
                    }
                    let mut v = text(y);
                    if let Some(suf) = ctx.year_suffix.as_ref().filter(|_| e.attr("variable") == Some("issued") && !ctx.style.calls_year_suffix()) {
                        v.push(Tok::Text(suf.clone()));
                    }
                    piece = Some(v);
                }
                "month" if p[1] != 0 => {
                    let f = t.attr("form").unwrap_or("long");
                    let s = match f {
                        "numeric" => p[1].to_string(),
                        "numeric-leading-zero" => format!("{:02}", p[1]),
                        _ => month_term(ctx.style, p[1], f),
                    };
                    let s = if t.attr("strip-periods") == Some("true") { s.replace('.', "") } else { s };
                    piece = Some(text(s));
                }
                "month" if p[1] == 0 && season.is_some() => {
                    let s = ctx.style.term(&format!("season-{:02}", season.unwrap_or(1)), "long", false).unwrap_or_default();
                    piece = Some(text(s));
                }
                "day" if p[2] != 0 => {
                    let s = match t.attr("form").unwrap_or("numeric") {
                        "numeric-leading-zero" => format!("{:02}", p[2]),
                        "ordinal" => ordinal(p[2] as u32, ctx.style),
                        _ => p[2].to_string(),
                    };
                    piece = Some(text(s));
                }
                _ => {}
            }
            let Some(mut toks) = piece else { continue };
            let mut tmp = R { toks: std::mem::take(&mut toks), called: 0, found: 0 };
            decorate(t, &mut tmp, ctx);
            if !first && !delim.is_empty() {
                out.push(Tok::Text(delim.to_string()));
            }
            first = false;
            out.extend(tmp.toks);
        }
        out
    };
    if d.parts.len() >= 2 {
        let (a, b) = (&d.parts[0], &d.parts[1]);
        let skip_first_year = a[0] == b[0] && a[0] != 0 && templates.iter().any(|t| t.attr("name") == Some("year")) && b[1] != 0;
        let mut toks = one(a, d.season, skip_first_year, ctx);
        toks.push(Tok::Text("\u{2013}".into()));
        toks.extend(one(b, None, false, ctx));
        r.toks = toks;
    } else if let Some(a) = d.parts.first() {
        r.toks = one(a, d.season, false, ctx);
    }
    if d.circa && r.found == 1 {
        let c = ctx.style.term("circa", "short", false).unwrap_or_default();
        let _ = c;
    }
    r
}

fn builtin_date_parts(form: &str) -> Vec<XElem> {
    let mk = |name: &str, attrs: &[(&str, &str)]| XElem {
        name: "date-part".into(),
        attrs: std::iter::once(("name".to_string(), name.to_string())).chain(attrs.iter().map(|(k, v)| (k.to_string(), v.to_string()))).collect(),
        children: vec![],
    };
    if form == "numeric" {
        vec![mk("month", &[("form", "numeric"), ("suffix", "/")]), mk("day", &[("form", "numeric"), ("suffix", "/")]), mk("year", &[])]
    } else {
        vec![mk("month", &[("suffix", " ")]), mk("day", &[("suffix", ", ")]), mk("year", &[])]
    }
}

impl Style {
    pub(crate) fn calls_year_suffix(&self) -> bool {
        self.calls_year_suffix
    }
}

// ---- names ------------------------------------------------------------------------------------

#[derive(Clone)]
struct NameOpts {
    and: Option<String>,
    delimiter: String,
    delimiter_precedes_last: String,
    delimiter_precedes_et_al: String,
    et_al_min: usize,
    et_al_use_first: usize,
    et_al_use_last: bool,
    form: String,
    initialize: bool,
    initialize_with: Option<String>,
    inverted: String,
    sort_separator: String,
    demote: bool,
}

fn name_opts(names_el: &XElem, name_el: Option<&XElem>, ctx: &Ctx) -> NameOpts {
    let scope = if ctx.mode == Mode::Citation { ctx.style.citation() } else { ctx.style.bibliography() };
    let mut chain: Vec<&XElem> = Vec::new();
    if let Some(n) = name_el {
        chain.push(n);
    }
    chain.push(names_el);
    if let Some(s) = scope {
        chain.push(s);
    }
    let get = |a: &str| -> Option<String> { ctx.style.inherited(a, &chain).map(str::to_string) };
    let min = get("et-al-min").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let use_first = get("et-al-use-first").and_then(|v| v.parse().ok()).unwrap_or(1);
    let (min, use_first) = match (ctx.names_min_override, ctx.item.position != Position::First) {
        (Some(o), _) => (o, o.min(use_first)),
        (None, true) => (
            get("et-al-subsequent-min").and_then(|v| v.parse().ok()).unwrap_or(min),
            get("et-al-subsequent-use-first").and_then(|v| v.parse().ok()).unwrap_or(use_first),
        ),
        _ => (min, use_first),
    };
    let and = name_el.and_then(|n| n.attr("and")).or_else(|| ctx.style.inherited("and", &chain)).map(str::to_string);
    NameOpts {
        and,
        delimiter: name_el.and_then(|n| n.attr("delimiter")).map(str::to_string).or_else(|| get("name-delimiter")).unwrap_or_else(|| ", ".into()),
        delimiter_precedes_last: get("delimiter-precedes-last").unwrap_or_else(|| "contextual".into()),
        delimiter_precedes_et_al: get("delimiter-precedes-et-al").unwrap_or_else(|| "contextual".into()),
        et_al_min: min,
        et_al_use_first: use_first,
        et_al_use_last: get("et-al-use-last").as_deref() == Some("true"),
        form: get("name-form").or_else(|| name_el.and_then(|n| n.attr("form")).map(str::to_string)).unwrap_or_else(|| "long".into()),
        initialize: get("initialize").as_deref() != Some("false"),
        initialize_with: get("initialize-with"),
        inverted: if ctx.sorting { "all".to_string() } else { get("name-as-sort-order").unwrap_or_default() },
        sort_separator: get("sort-separator").unwrap_or_else(|| ", ".into()),
        demote: ctx.style.inherited("demote-non-dropping-particle", &[]).map(|v| v != "never" && v != "sort-only").unwrap_or(true),
    }
}

fn initials(given: &str, with: &str, init: bool) -> String {
    if !init {
        return given.to_string();
    }
    // Each given-name part becomes its first letter followed by `with`; hyphenated parts keep
    // the hyphen ("Jean-Paul" -> "J.-P.").
    let mut out = String::new();
    let trimmed_with = with.trim_end();
    let words: Vec<&str> = given.split_whitespace().collect();
    for (wi, w) in words.iter().enumerate() {
        let mut parts = w.split('-').peekable();
        let mut piece = String::new();
        while let Some(p) = parts.next() {
            let is_initial = p.chars().filter(|c| c.is_alphabetic()).count() <= 1 && p.trim_end_matches('.').chars().count() <= 1;
            let first = p.chars().next();
            let lower_particle = p.chars().next().is_some_and(char::is_lowercase) && p.chars().count() > 1;
            if lower_particle {
                // "de", "van": kept as a word.
                piece.push_str(p);
                if parts.peek().is_some() {
                    piece.push('-');
                }
                continue;
            }
            if let Some(f) = first {
                piece.push(f);
                if !(is_initial && p.ends_with('.') && trimmed_with.is_empty()) {
                    piece.push_str(trimmed_with);
                }
            }
            if parts.peek().is_some() {
                piece.push('-');
            }
        }
        out.push_str(&piece);
        if wi + 1 < words.len() {
            // Between initials the separator is `with` as given (".", ". ", "" or " ").
            let sep = if with.ends_with(' ') { " " } else { "" };
            out.push_str(sep);
        }
    }
    if with.ends_with(' ') && !with.trim().is_empty() {
        // A trailing space is only a separator between initials; it is not printed at the end.
    }
    out
}

fn name_display(n: &Name, invert: bool, init_with: &str, initialize: bool) -> String {
    if !n.literal.is_empty() {
        return n.literal.clone();
    }
    let given = if init_with.is_empty() && !initialize { n.given.clone() } else { initials(&n.given, init_with, initialize) };
    let mut family = n.family.clone();
    if !n.non_dropping_particle.is_empty() {
        family = format!("{} {}", n.non_dropping_particle, family);
    }
    let suffix = if n.suffix.is_empty() {
        String::new()
    } else if n.comma_suffix {
        format!(", {}", n.suffix)
    } else {
        format!(" {}", n.suffix)
    };
    if invert {
        let mut s = family;
        let mut tail = given;
        if !n.dropping_particle.is_empty() {
            tail = if tail.is_empty() { n.dropping_particle.clone() } else { format!("{} {}", tail, n.dropping_particle) };
        }
        if !tail.is_empty() {
            s.push_str(", ");
            s.push_str(&tail);
        }
        s.push_str(&suffix.replace(' ', ", ").replacen(", ,", ",", 1));
        s
    } else {
        let mut parts: Vec<String> = Vec::new();
        if !given.is_empty() {
            parts.push(given);
        }
        if !n.dropping_particle.is_empty() {
            parts.push(n.dropping_particle.clone());
        }
        parts.push(family);
        format!("{}{}", parts.join(" "), suffix)
    }
}

fn eval_names(e: &XElem, ctx: &mut Ctx) -> R {
    let mut out = R::default();
    let vars: Vec<String> = e.attr("variable").unwrap_or("").split_whitespace().map(str::to_string).collect();
    out.called = vars.len().max(1);
    // author-only / suppress-author apply to the first `names` element that prints the author.
    let is_author_names = vars.iter().any(|v| v == "author");
    let name_el = e.find("name");
    let et_al_el = e.find("et-al");
    let opts = name_opts(e, name_el, ctx);
    let mut rendered: Vec<(String, Toks)> = Vec::new();
    for v in &vars {
        if ctx.suppressed.contains(v) {
            continue;
        }
        let list: Vec<Name> = ctx.reference.names(v).to_vec();
        if list.is_empty() {
            continue;
        }
        rendered.push((v.clone(), render_name_list(&list, &opts, name_el, et_al_el, ctx)));
    }
    // `editor` and `translator` with the same people print once, labelled with both roles.
    let mut result_toks: Toks = Toks::new();
    let mut used: Vec<String> = Vec::new();
    for (i, (v, toks)) in rendered.iter().enumerate() {
        if i > 0 {
            result_toks.push(Tok::Text(e.attr("delimiter").unwrap_or("").to_string()));
        }
        result_toks.extend(toks.clone());
        // Label.
        if let Some(l) = e.find("label") {
            let count = ctx.reference.names(v).len();
            let form = l.attr("form").unwrap_or("long");
            let plural = match l.attr("plural") {
                Some("always") => true,
                Some("never") => false,
                _ => count > 1,
            };
            if let Some(t) = ctx.style.term(v, form, plural) {
                let mut lr = R { toks: text(t), called: 0, found: 0 };
                decorate(l, &mut lr, ctx);
                // Labels follow the names unless the `label` element comes first.
                let label_first = e.elems().position(|c| c.name == "label") < e.elems().position(|c| c.name == "name");
                if label_first {
                    let mut t2 = lr.toks;
                    t2.append(&mut result_toks);
                    result_toks = t2;
                } else {
                    result_toks.extend(lr.toks);
                }
            }
        }
        used.push(v.clone());
    }
    if !rendered.is_empty() {
        out.found = rendered.len();
        out.toks = result_toks;
        for u in &used {
            // Variables printed here are not printed again by a later substitute.
            if let Some(sub_parent) = e.attr("__substituting") {
                let _ = sub_parent;
            }
            let _ = u;
        }
    } else if let Some(sub) = e.find("substitute") {
        for c in sub.elems() {
            let before = ctx.suppressed.clone();
            let r = if c.name == "names" {
                // Substituted names inherit the options of the names element they stand in for.
                let mut c2 = c.clone();
                for (k, v) in &e.attrs {
                    if k != "variable" && c2.attr(k).is_none() {
                        c2.attrs.push((k.clone(), v.clone()));
                    }
                }
                if c2.find("name").is_none() {
                    if let Some(n) = name_el {
                        c2.children.insert(0, XNode::Elem(n.clone()));
                    }
                }
                if c2.find("et-al").is_none() {
                    if let Some(n) = et_al_el {
                        c2.children.push(XNode::Elem(n.clone()));
                    }
                }
                if c2.find("label").is_none() {
                    if let Some(n) = e.find("label") {
                        c2.children.push(XNode::Elem(n.clone()));
                    }
                }
                eval(&c2, ctx)
            } else {
                eval(c, ctx)
            };
            if !r.empty() {
                out.found = 1;
                out.toks = r.toks;
                // The variable that stood in is not printed again in this cite.
                if let Some(vv) = c.attr("variable") {
                    for x in vv.split_whitespace() {
                        ctx.suppressed.insert(x.to_string());
                    }
                }
                if c.name == "text" && c.attr("macro").is_some() {
                    // A macro that printed names (title/editor): its variables are marked by
                    // looking at which name variables it could have printed.
                    for v in ["editor", "translator", "director", "container-author", "composer"] {
                        if before.contains(v) {
                            continue;
                        }
                    }
                }
                break;
            }
        }
    }
    // author-only: only the author names are kept; suppress-author: they are dropped.
    if is_author_names && ctx.mode == Mode::Citation {
        ctx.names_seen += 1;
        if ctx.item.author_only && ctx.captured.is_none() && !out.toks.is_empty() {
            ctx.captured = Some(out.toks.clone());
        }
        if ctx.item.suppress_author {
            out.toks.clear();
            out.found = 0;
            out.called = 0;
        }
    }
    out
}

fn render_name_list(list: &[Name], o: &NameOpts, name_el: Option<&XElem>, et_al_el: Option<&XElem>, ctx: &Ctx) -> Toks {
    let total = list.len();
    let (shown, et_al) = if total >= o.et_al_min && o.et_al_use_first < total {
        (o.et_al_use_first.max(1), true)
    } else {
        (total, false)
    };
    let use_last = et_al && o.et_al_use_last && total > shown + 1;
    let init_with = o.initialize_with.clone().unwrap_or_default();
    let initialize = o.initialize_with.is_some() && o.initialize;
    let count_form = o.form == "count";
    if count_form {
        return text(total.min(shown).to_string());
    }
    let mut parts: Vec<(Toks, bool)> = Vec::new();
    let mut shown_names: Vec<&Name> = list.iter().take(shown).collect();
    if use_last {
        shown_names.push(&list[total - 1]);
    }
    for (i, n) in shown_names.iter().enumerate() {
        let invert = match o.inverted.as_str() {
            "all" => true,
            "first" => i == 0,
            _ => false,
        } && n.literal.is_empty();
        let toks = if n.literal.is_empty() { name_toks(n, invert, o, &init_with, initialize, name_el, ctx) } else { text(n.literal.clone()) };
        parts.push((toks, invert));
    }
    let mut out = Toks::new();
    let n = parts.len();
    for (i, (toks, _)) in parts.iter().enumerate() {
        if i > 0 {
            let last_pos = i == n - 1 && !et_al;
            if use_last && i == n - 1 {
                out.push(Tok::Text(format!("{}\u{2026} ", o.delimiter)));
            } else if last_pos && o.and.is_some() {
                let prev_inverted = parts[i - 1].1;
                let precede = match o.delimiter_precedes_last.as_str() {
                    "always" => true,
                    "never" => false,
                    "after-inverted-name" => prev_inverted,
                    _ => n >= 3,
                };
                let and = match o.and.as_deref() {
                    Some("symbol") => ctx.style.term("and", "symbol", false).unwrap_or_else(|| "&".into()),
                    _ => ctx.style.term("and", "long", false).unwrap_or_else(|| "and".into()),
                };
                if precede {
                    out.push(Tok::Text(o.delimiter.clone()));
                    out.push(Tok::Text(format!("{and} ")));
                } else {
                    out.push(Tok::Text(format!(" {and} ")));
                }
            } else {
                out.push(Tok::Text(o.delimiter.clone()));
            }
        }
        out.extend(toks.clone());
    }
    if et_al && !use_last {
        let term = et_al_el.and_then(|e| e.attr("term")).unwrap_or("et-al");
        let t = ctx.style.term(term, "long", false).unwrap_or_else(|| "et al.".into());
        let precede = match o.delimiter_precedes_et_al.as_str() {
            "always" => true,
            "never" => false,
            "after-inverted-name" => parts.last().is_some_and(|p| p.1),
            _ => n >= 2,
        };
        out.push(Tok::Text(if precede { o.delimiter.clone() } else { " ".into() }));
        let mut r = R { toks: text(t), called: 0, found: 0 };
        if let Some(el) = et_al_el {
            decorate(el, &mut r, ctx);
        }
        out.extend(r.toks);
    }
    out
}

/// One name, honouring `<name-part>` formatting.
fn name_toks(n: &Name, invert: bool, o: &NameOpts, init_with: &str, initialize: bool, name_el: Option<&XElem>, ctx: &Ctx) -> Toks {
    let _ = o.sort_separator.as_str();
    let short = o.form == "short";
    let parts_el: Vec<&XElem> = name_el.map(|n| n.elems().filter(|e| e.name == "name-part").collect()).unwrap_or_default();
    let part_el = |name: &str| parts_el.iter().find(|e| e.attr("name") == Some(name)).copied();
    // Plain composition first.
    let given = if init_with.is_empty() && !initialize { n.given.clone() } else { initials(&n.given, init_with, initialize) };
    let mut family_tok = {
        let mut r = R { toks: text(if n.non_dropping_particle.is_empty() { n.family.clone() } else if o.demote && invert { n.family.clone() } else { format!("{} {}", n.non_dropping_particle, n.family) }), called: 0, found: 0 };
        if let Some(el) = part_el("family") {
            decorate(el, &mut r, ctx);
        }
        r.toks
    };
    if short {
        return family_tok;
    }
    let given_tok = {
        let mut r = R { toks: text(given.clone()), called: 0, found: 0 };
        if let Some(el) = part_el("given") {
            decorate(el, &mut r, ctx);
        }
        r.toks
    };
    let mut out = Toks::new();
    let sep = o.sort_separator.clone();
    let mut particle_after = String::new();
    if !n.dropping_particle.is_empty() {
        particle_after = n.dropping_particle.clone();
    }
    if o.demote && invert && !n.non_dropping_particle.is_empty() {
        particle_after = if particle_after.is_empty() { n.non_dropping_particle.clone() } else { format!("{} {}", particle_after, n.non_dropping_particle) };
    }
    let suffix = if n.suffix.is_empty() {
        String::new()
    } else if n.comma_suffix || invert {
        format!(", {}", n.suffix)
    } else {
        format!(" {}", n.suffix)
    };
    if invert {
        out.append(&mut family_tok);
        if !given.is_empty() || !particle_after.is_empty() {
            out.push(Tok::Text(sep));
            out.extend(given_tok.clone());
            if !particle_after.is_empty() {
                out.push(Tok::Text(format!("{}{}", if given.is_empty() { "" } else { " " }, particle_after)));
            }
        }
        out.push(Tok::Text(suffix));
    } else {
        let mut lead = Toks::new();
        lead.extend(given_tok.clone());
        let mut pieces: Vec<Toks> = Vec::new();
        if !given.is_empty() {
            pieces.push(lead);
        }
        if !n.dropping_particle.is_empty() {
            pieces.push(text(n.dropping_particle.clone()));
        }
        pieces.push(family_tok);
        for (i, p) in pieces.into_iter().enumerate() {
            if i > 0 {
                out.push(Tok::Text(" ".into()));
            }
            out.extend(p);
        }
        out.push(Tok::Text(suffix));
    }
    out
}

// ---- helpers shared with the processor ----------------------------------------------------------

/// Plain text of a token list (formatting marks dropped).
pub fn plain(toks: &[Tok]) -> String {
    toks.iter()
        .filter_map(|t| match t {
            Tok::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

pub(crate) fn child_text(e: &XElem) -> String {
    text_of(e)
}
