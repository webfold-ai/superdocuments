//! parse.rs — the PDF as ONE fold (the universal lift), every step O(1) on the Table.
//!
//! `Document::parse` is the one-time read (the streaming-law `prepare`): scan the bytes once, fold
//! each `N G obj …` into the object Table (`put`, O(1) each). No xref bsearch — the objects ARE the
//! table. `markdown` walks the page tree by O(1) `get`s, inflates content (FlateDecode), and runs the
//! **text accumulator** over the content stream: each operator is O(1), line breaks come from the
//! move operators (document order = reading order), so there is no sort and no binary search.

use super::{Dict, Document, Object};

// ── token scanner over PDF bytes (the universal lexer, α = byte) ──
pub(super) struct Lex<'a> {
    pub(super) d: &'a [u8],
    pub(super) i: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    ArrOpen,
    ArrClose,
    DictOpen,
    DictClose,
    Kw(String),
}

pub(super) fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0)
}
pub(super) fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl<'a> Lex<'a> {
    pub(super) fn new(d: &'a [u8]) -> Self {
        Lex { d, i: 0 }
    }
    pub(super) fn skip_ws(&mut self) {
        while self.i < self.d.len() {
            let b = self.d[self.i];
            if b == b'%' {
                while self.i < self.d.len() && self.d[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if is_ws(b) {
                self.i += 1;
            } else {
                break;
            }
        }
    }
    /// The next token. A stray delimiter is skipped, not an error, so this never recurses.
    pub(super) fn next(&mut self) -> Option<Tok> {
        loop {
            self.skip_ws();
            if self.i >= self.d.len() {
                return None;
            }
            let b = self.d[self.i];
            let tok = match b {
                b'[' => {
                    self.i += 1;
                    Some(Tok::ArrOpen)
                }
                b']' => {
                    self.i += 1;
                    Some(Tok::ArrClose)
                }
                b'<' if self.d.get(self.i + 1) == Some(&b'<') => {
                    self.i += 2;
                    Some(Tok::DictOpen)
                }
                b'>' if self.d.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    Some(Tok::DictClose)
                }
                b'<' => self.hex_string(),
                b'(' => self.lit_string(),
                b'/' => self.name(),
                b'0'..=b'9' | b'+' | b'-' | b'.' => self.number(),
                _ => self.keyword(),
            };
            if tok.is_some() {
                return tok;
            }
        }
    }
    fn name(&mut self) -> Option<Tok> {
        self.i += 1;
        let s = self.i;
        while self.i < self.d.len() && !is_ws(self.d[self.i]) && !is_delim(self.d[self.i]) {
            self.i += 1;
        }
        Some(Tok::Name(
            String::from_utf8_lossy(&self.d[s..self.i]).into_owned(),
        ))
    }
    fn number(&mut self) -> Option<Tok> {
        let s = self.i;
        while self.i < self.d.len()
            && matches!(
                self.d[self.i],
                b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E'
            )
        {
            self.i += 1;
        }
        let txt = String::from_utf8_lossy(&self.d[s..self.i]);
        Some(Tok::Num(txt.parse().unwrap_or(0.0)))
    }
    /// A keyword run. A byte that starts no token (a stray delimiter) is skipped: `None`.
    fn keyword(&mut self) -> Option<Tok> {
        let s = self.i;
        while self.i < self.d.len() && !is_ws(self.d[self.i]) && !is_delim(self.d[self.i]) {
            self.i += 1;
        }
        if self.i == s {
            self.i += 1;
            return None;
        }
        Some(Tok::Kw(
            String::from_utf8_lossy(&self.d[s..self.i]).into_owned(),
        ))
    }
    fn lit_string(&mut self) -> Option<Tok> {
        self.i += 1;
        let mut out = Vec::new();
        let mut depth = 1;
        while self.i < self.d.len() {
            let c = self.d[self.i];
            self.i += 1;
            match c {
                b'\\' => {
                    let e = self.d.get(self.i).copied().unwrap_or(0);
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            // octal escape \ddd (1-3 digits) — the subset-font codes live here
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.d.get(self.i) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        _ => out.push(e), // \( \) \\ etc.
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        // A backslash as the last byte steps past the end: clamp, so `i <= d.len()` always holds.
        self.i = self.i.min(self.d.len());
        Some(Tok::Str(out))
    }
    fn hex_string(&mut self) -> Option<Tok> {
        self.i += 1;
        let mut out = Vec::new();
        let (mut hi, mut have) = (0u8, false);
        while self.i < self.d.len() {
            let c = self.d[self.i];
            self.i += 1;
            if c == b'>' {
                break;
            }
            let v = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => continue,
            };
            if have {
                out.push((hi << 4) | v);
                have = false;
            } else {
                hi = v;
                have = true;
            }
        }
        if have {
            out.push(hi << 4);
        }
        self.i = self.i.min(self.d.len());
        Some(Tok::Str(out))
    }
}

// ── parse one object from a token stream (recursive on dict/array; refs collapse `N G R`) ──

/// Nesting bound for arrays and dictionaries; deeper input reads as null, so the recursion has a
/// fixed depth whatever the file says.
const MAX_DEPTH: u32 = 64;

/// The object number of `n` when it is an integer in `0..2^32`.
fn ref_num(n: f64) -> Option<u32> {
    if n.fract() == 0.0 && (0.0..4_294_967_296.0).contains(&n) {
        Some(n as u32)
    } else {
        None
    }
}

/// The generation of `g` when it is an integer in `0..=65535`.
fn ref_gen(g: f64) -> Option<u16> {
    if g.fract() == 0.0 && (0.0..=65_535.0).contains(&g) {
        Some(g as u16)
    } else {
        None
    }
}

pub(super) fn parse_value(lex: &mut Lex, first: Tok, depth: u32) -> Object {
    match first {
        Tok::Num(n) => {
            // `N G R` is a reference when both numbers are in range; otherwise `N` is a number
            let save = lex.i;
            if let Some(Tok::Num(g)) = lex.next() {
                let save2 = lex.i;
                if let Some(Tok::Kw(k)) = lex.next() {
                    if k == "R" {
                        if let (Some(num), Some(gen)) = (ref_num(n), ref_gen(g)) {
                            return Object::Ref(num, gen);
                        }
                    }
                }
                lex.i = save2;
            }
            lex.i = save;
            if n.fract() == 0.0 {
                Object::Int(n as i64)
            } else {
                Object::Real(n)
            }
        }
        Tok::Str(s) => Object::Str(s),
        Tok::Name(n) => Object::Name(n),
        Tok::ArrOpen => {
            if depth >= MAX_DEPTH {
                return Object::Null;
            }
            let mut v = Vec::new();
            while let Some(t) = lex.next() {
                if t == Tok::ArrClose {
                    break;
                }
                v.push(parse_value(lex, t, depth + 1));
            }
            Object::Array(v)
        }
        Tok::DictOpen => {
            if depth >= MAX_DEPTH {
                return Object::Null;
            }
            Object::Dict(parse_dict(lex, depth + 1))
        }
        Tok::Kw(k) => match k.as_str() {
            "true" => Object::Bool(true),
            "false" => Object::Bool(false),
            _ => Object::Null,
        },
        _ => Object::Null,
    }
}

pub(super) fn parse_dict(lex: &mut Lex, depth: u32) -> Dict {
    let mut d = Dict::new();
    while let Some(t) = lex.next() {
        match t {
            Tok::DictClose => break,
            Tok::Name(key) => {
                if let Some(vt) = lex.next() {
                    d.put(key, parse_value(lex, vt, depth));
                }
            }
            _ => {}
        }
    }
    d
}

impl Document {
    /// Load the document through its cross-reference structure (see `xref`): `startxref`, the
    /// `/Prev` chain, xref tables and streams, and object streams. O(file) once; lookups are O(1).
    pub fn parse(bytes: &[u8]) -> Result<Document, super::PdfError> {
        super::xref::load(bytes)
    }

    /// Decode a stream's bytes through its `/Filter` chain and predictor (see `decode`). The
    /// Map face decides the filter. `None` for a non-stream or an undecodable stream.
    pub fn stream_bytes(&self, o: &Object) -> Option<Vec<u8>> {
        match o {
            Object::Stream(d, raw) => {
                let d =
                    super::decode::with_resolved_parms(d, &|n, g| self.ref_object(n, g).cloned());
                super::decode::decode(&d, raw)
            }
            _ => None,
        }
    }
}

// ── the layout fold: page tree → content → the text accumulator → markdown ──

/// The drawn rules of a page: horizontal `(y, x0, x1)` and vertical `(x, y0, y1)`.
type PageRules = (Vec<(f64, f64, f64)>, Vec<(f64, f64, f64)>);
/// One page's laid-out lines with its drawn rules.
type PageData = Vec<(Vec<Line>, PageRules)>;
/// One row of `Document::table_debug`.
type TableDebugRow = (usize, usize, usize, usize, usize, bool, bool, usize, usize);

fn num(t: &Tok) -> Option<f64> {
    if let Tok::Num(n) = t {
        Some(*n)
    } else {
        None
    }
}

/// Is this line a heading? Three signals, any of which suffices: a font **larger** than the body, a
/// **bold** short title line, or a **numbered section** (`11 …`, `1.2 …`, `3.1.4 …`). Each is gated to
/// short, lettered lines that don't read as a full sentence — so body text and captions don't trip it.
fn is_heading(l: &str, size: f64, bold: bool, body: f64) -> bool {
    let n = l.chars().count();
    if n == 0 || n >= 150 || !l.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    if body > 0.0 && size > body * 1.15 {
        return true;
    } // larger than body
    if bold && n < 90 && !l.ends_with('.') {
        return true;
    } // bold short title
    n < 90 && is_section_heading(l) // numbered section
}

/// A leading section number — `\d+(\.\d+)*` followed by whitespace then an uppercase letter
/// (`11 Dual-…`, `1.2 Methods`). Page numbers (no letter) and captions (`Figure …`) don't match.
fn is_section_heading(l: &str) -> bool {
    let c: Vec<char> = l.trim_start().chars().collect();
    if c.first().is_none_or(|ch| !ch.is_ascii_digit()) {
        return false;
    }
    let mut i = 0;
    while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
        i += 1;
    }
    if i == 0 || i >= c.len() || !c[i].is_whitespace() {
        return false;
    }
    while i < c.len() && c[i].is_whitespace() {
        i += 1;
    }
    c.get(i).is_some_and(|ch| ch.is_uppercase())
}

fn html_escape(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
}

// ── ruling lines: the table grid IS a drawn line-segment DAG (`re`/`l` path ops). Read it. ──

/// Concatenate two PDF affine matrices `[a b c d e f]` (point: x'=a·x+c·y+e, y'=b·x+d·y+f), `m` first.
fn matmul(m: &[f64; 6], n: &[f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

/// Extract the axis-aligned ruling lines from a content stream — tracking the CTM (`cm`/`q`/`Q`) and
/// the path (`m`/`l`/`re`), emitting each segment in user space on a paint op. The grid the table is
/// drawn with. Returns (horizontal rules `(y, x0, x1)`, vertical rules `(x, y0, y1)`).
fn extract_rules(content: &[u8]) -> PageRules {
    let (mut h, mut v): PageRules = (Vec::new(), Vec::new());
    let mut ctm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut stack: Vec<[f64; 6]> = Vec::new();
    let map = |c: &[f64; 6], px: f64, py: f64| {
        (c[0] * px + c[2] * py + c[4], c[1] * px + c[3] * py + c[5])
    };
    let (mut cur, mut start) = ((0.0, 0.0), (0.0, 0.0));
    let mut segs: Vec<((f64, f64), (f64, f64))> = Vec::new();
    let mut ops: Vec<f64> = Vec::new();
    let mut lex = Lex::new(content);
    while let Some(t) = lex.next() {
        match &t {
            Tok::Num(x) => ops.push(*x),
            Tok::Kw(op) => {
                let n = ops.len();
                match op.as_str() {
                    "cm" if n >= 6 => {
                        ctm = matmul(
                            &[
                                ops[n - 6],
                                ops[n - 5],
                                ops[n - 4],
                                ops[n - 3],
                                ops[n - 2],
                                ops[n - 1],
                            ],
                            &ctm,
                        )
                    }
                    "q" => stack.push(ctm),
                    "Q" => {
                        if let Some(s) = stack.pop() {
                            ctm = s;
                        }
                    }
                    "m" if n >= 2 => {
                        let p = map(&ctm, ops[n - 2], ops[n - 1]);
                        cur = p;
                        start = p;
                    }
                    "l" if n >= 2 => {
                        let p = map(&ctm, ops[n - 2], ops[n - 1]);
                        segs.push((cur, p));
                        cur = p;
                    }
                    "re" if n >= 4 => {
                        let (x, y, w, hh) = (ops[n - 4], ops[n - 3], ops[n - 2], ops[n - 1]);
                        let (a, b, c, e) = (
                            map(&ctm, x, y),
                            map(&ctm, x + w, y),
                            map(&ctm, x + w, y + hh),
                            map(&ctm, x, y + hh),
                        );
                        segs.extend_from_slice(&[(a, b), (b, c), (c, e), (e, a)]);
                        cur = a;
                        start = a;
                    }
                    "h" => {
                        segs.push((cur, start));
                        cur = start;
                    }
                    "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                        for ((x0, y0), (x1, y1)) in segs.drain(..) {
                            if (y0 - y1).abs() < 1.0 && (x0 - x1).abs() > 3.0 {
                                h.push((y0, x0.min(x1), x0.max(x1)));
                            } else if (x0 - x1).abs() < 1.0 && (y0 - y1).abs() > 3.0 {
                                v.push((x0, y0.min(y1), y0.max(y1)));
                            }
                        }
                    }
                    _ => {}
                }
                ops.clear();
            }
            _ => ops.clear(),
        }
    }
    (h, v)
}

/// Cluster coordinates within `tol` into `(center, count)`, sorted by center. Sorts the coordinates
/// once (`O(n log n)`), then a single linear pass accumulates each cluster as a running sum+count —
/// the center is the B/U ratio `sum/count`, `O(1)` amortized per coordinate *within that pass*.
fn cluster_counts(mut xs: Vec<f64>, tol: f64) -> Vec<(f64, usize)> {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut cl: Vec<(f64, usize)> = Vec::new(); // (sum, count)
    for x in xs {
        match cl.last_mut() {
            Some(c) if (x - c.0 / c.1 as f64).abs() <= tol => {
                c.0 += x;
                c.1 += 1;
            }
            _ => cl.push((x, 1)),
        }
    }
    cl.iter().map(|c| (c.0 / c.1 as f64, c.1)).collect()
}

fn cluster_coords(xs: Vec<f64>, tol: f64) -> Vec<f64> {
    cluster_counts(xs, tol)
        .into_iter()
        .map(|(x, _)| x)
        .collect()
}

/// Merge sorted centers closer than `min_sep` — a doubled/tripled drawn border (the same logical edge
/// stroked 2-3×) bounds a sub-minimal "column" no cell fits in; collapse it to one edge (running mean).
/// O(rules) on the tiny rule list (the null cell), not the document.
fn merge_close(mut cs: Vec<f64>, min_sep: f64) -> Vec<f64> {
    cs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<f64> = Vec::new();
    for x in cs {
        match out.last_mut() {
            Some(p) if x - *p < min_sep => *p = (*p + x) / 2.0,
            _ => out.push(x),
        }
    }
    out
}

/// Trim edge rules that are page borders / far outliers — a rule whose gap to the body is more than 3×
/// the median inter-rule gap is not part of the table lattice (it's the page frame swallowing the
/// title). Returns the rules sorted ascending. O(rules) on the rule list (the null cell).
fn trim_outliers(mut ys: Vec<f64>) -> Vec<f64> {
    if ys.len() < 4 {
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        return ys;
    }
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut gaps: Vec<f64> = ys.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let thr = (gaps[gaps.len() / 2] * 3.0).max(20.0);
    while ys.len() > 3 && ys[1] - ys[0] > thr {
        ys.remove(0);
    }
    while ys.len() > 3 && ys[ys.len() - 1] - ys[ys.len() - 2] > thr {
        ys.pop();
    }
    ys
}

/// **Columns from whitespace gutters** (the Multi-Type-TD-TSR borderless method in the framework's
/// terms): the **soft projection**. Project each row's text-extent onto an x-coverage profile, but
/// accumulate the **depth** — how many *rows* occupy each x (the accumulator/B-U face, `cov[x]` a
/// running count) — not a boolean. A **column gutter** is a *valley*: an x-band that is empty across
/// *most* rows (`cov ≤ ⌈nrows/5⌉`), the Non-Being that separates the columns (`x = B/U`: the empty
/// defines the structure). Counting depth, not "empty in EVERY row", is what survives a single
/// spanning header/wide cell — the boolean read collapses there, the valley does not. Prose fills the
/// width at every depth → no valley → not a table. One pass per segment (O(1)/segment); the valley
/// scan is O(page width) — the null cell, bounded by the page, never the document. Column boundaries.
fn columns_from_gutters(region: &[&Line]) -> Option<Vec<f64>> {
    let (mut xmin, mut xmax) = (f64::MAX, f64::MIN);
    for l in region {
        for &(x0, x1, _, _) in &l.segs {
            xmin = xmin.min(x0);
            xmax = xmax.max(x1);
        }
    }
    // The projection is as wide as the region, so the region must lie within a PDF user space (at
    // most 14400 points). A wider or non-finite span is a malformed content stream, not a table, and
    // is refused before any buffer is sized from it.
    if !(xmax - xmin > 10.0 && xmax - xmin <= MAX_TABLE_SPAN) {
        return None;
    }
    let w = (xmax - xmin).ceil() as usize + 2;
    let mut cov = vec![0u32; w]; // x-projection DEPTH: rows covering x
    let mut nrows = 0u32;
    let mut row = vec![false; w]; // this row's coverage, cleared after each row (mark once per row)
    let mut touched: Vec<usize> = Vec::new();
    for l in region {
        if l.segs.is_empty() {
            continue;
        }
        nrows += 1;
        for &(x0, x1, _, _) in &l.segs {
            let a = ((x0 - xmin).floor().max(0.0) as usize).min(w - 1);
            let b = ((x1 - xmin).ceil() as usize).min(w - 1);
            for (cell, slot) in row.iter_mut().enumerate().take(b + 1).skip(a) {
                if !*slot {
                    *slot = true;
                    touched.push(cell);
                }
            }
        }
        for &i in &touched {
            cov[i] += 1;
            row[i] = false;
        }
        touched.clear();
    }
    if nrows < 2 {
        return None;
    }
    let floor = (nrows / 5).max(1); // a gutter is empty in ≥80% of rows
    let mut bounds = vec![xmin - 1.0]; // boundaries = interior gutter centers
    let mut i = 0;
    while i < w {
        if cov[i] > floor {
            i += 1;
            continue;
        }
        let start = i;
        while i < w && cov[i] <= floor {
            i += 1;
        }
        if start > 0 && i < w && (i - start) as f64 >= 4.0 {
            // an interior valley wider than a space
            bounds.push(xmin + (start + i) as f64 / 2.0);
        }
    }
    bounds.push(xmax + 1.0);
    if bounds.len() < 3 {
        return None;
    } // ≥2 columns; the gate decides table-ness
    Some(bounds)
}

/// **Columns from row-support** — trained on FinTabNet (born-digital PDFs, same coordinate space as
/// pdffold; 82.3% col-exact val vs 8.7% naive). A real column is an x-start **occupied across ≥35% of
/// the region's rows** (the B/U accumulator over rows): cluster segment x-starts at `1.2×text-height`,
/// keep clusters that appear in ≥`supp` of rows. A spanning header (few rows) can't fake a column; a
/// true column repeats down the table. Fitted params `a=1.2, supp=0.35` (FinTabNet `Structure`). Each
/// pdffold line IS a row, so row membership is the line index — no y-cluster. O(1)/segment.
fn columns_from_row_support(region: &[&Line]) -> Option<Vec<f64>> {
    let nrows = region.len();
    if nrows < 2 {
        return None;
    }
    let mut sizes: Vec<f64> = region
        .iter()
        .filter(|l| l.size > 0.0)
        .map(|l| l.size)
        .collect();
    if sizes.is_empty() {
        return None;
    }
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let tol = 1.2 * sizes[sizes.len() / 2]; // 1.2 × median text-height
    let mut xr: Vec<(f64, usize)> = Vec::new();
    for (ri, l) in region.iter().enumerate() {
        for &(x0, _, _, _) in &l.segs {
            xr.push((x0, ri));
        }
    }
    if xr.len() < 2 {
        return None;
    }
    xr.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut clusters: Vec<(f64, usize, Vec<bool>)> = Vec::new(); // (sum_x, count, rows-seen)
    for &(x, ri) in &xr {
        match clusters.last_mut() {
            Some(c) if x - c.0 / c.1 as f64 <= tol => {
                c.0 += x;
                c.1 += 1;
                c.2[ri] = true;
            }
            _ => {
                let mut seen = vec![false; nrows];
                seen[ri] = true;
                clusters.push((x, 1, seen));
            }
        }
    }
    let need = (((nrows as f64) * 0.35).ceil() as usize).max(2);
    let cols: Vec<f64> = clusters
        .iter()
        .filter(|c| c.2.iter().filter(|&&b| b).count() >= need)
        .map(|c| c.0 / c.1 as f64)
        .collect();
    if cols.len() < 2 {
        return None;
    }
    let xmax = region
        .iter()
        .flat_map(|l| l.segs.iter().map(|s| s.1))
        .fold(f64::MIN, f64::max);
    let mut b = vec![cols[0] - 2.0];
    for w in cols.windows(2) {
        b.push((w[0] + w[1]) / 2.0);
    }
    b.push((xmax + 2.0).max(cols[cols.len() - 1] + 2.0));
    Some(b)
}

/// Row boundaries between the region's text lines (each line a row): midpoints + outer edges, top→bottom.
fn row_boundaries(region: &[&Line]) -> Vec<f64> {
    let mut ys: Vec<f64> = region.iter().map(|l| l.y).collect();
    ys.sort_by(|a, b| b.partial_cmp(a).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() < 2.0);
    if ys.len() < 2 {
        return Vec::new();
    }
    let gap = (ys[0] - ys[ys.len() - 1]) / (ys.len() - 1) as f64;
    let mut b = vec![ys[0] + gap / 2.0];
    for win in ys.windows(2) {
        b.push((win[0] + win[1]) / 2.0);
    }
    b.push(ys[ys.len() - 1] - gap / 2.0);
    b
}

/// A candidate table region — its grid (`rowys` × `colxs`), y-span, the feature signature the seed
/// reads, and the geometric hand-read verdict (the fallback gate when no seed is supplied).
struct Cand {
    ytop: f64,
    ybot: f64,
    rowys: Vec<f64>,
    colxs: Vec<f64>,
    feat: u64,
    geo_ok: bool,
}

/// The widest a table region may be, in points: the largest PDF user-space extent.
const MAX_TABLE_SPAN: f64 = 14_400.0;
/// The most cells a table candidate may have; larger grids are not tables this reader proposes.
const MAX_GRID_CELLS: usize = 600;

/// Push a candidate (if it forms a ≥2×2 grid). RECALL is generous here — the gate (seed or geometric)
/// decides table-ness later; this only proposes structurally-plausible regions.
fn add_cand(out: &mut Vec<Cand>, lines: &[Line], rowys: Vec<f64>, colxs: Vec<f64>, ruled: bool) {
    if rowys.len() < 3 || colxs.len() < 3 {
        return;
    } // ≥2 rows, ≥2 columns
      // Refuse an oversized grid before `region_stats` sizes its per-cell buffers from it.
    if (rowys.len() - 1) * (colxs.len() - 1) > MAX_GRID_CELLS {
        return;
    }
    let st = region_stats(lines, &rowys, &colxs, ruled);
    if st.nr < 2 || st.nc < 2 || st.nr * st.nc > MAX_GRID_CELLS {
        return;
    }
    // A RULED grid IS the table — the drawn lattice is the author's own structure, so accept on a loose
    // read: text actually inside it (`fill ≥ 15`) and rows that span columns (`multi_col ≥ 50`). The
    // strict prose gate (`avg_cell ≤ 24`) wrongly killed real tables with long cells, so it's dropped
    // here. A BORDERLESS region (gutters only) keeps the strict gate — that's where false positives live.
    let geo_ok = if ruled {
        st.fill_pct >= 15 && st.multi_col_pct >= 50
    } else {
        st.fill_pct >= 50 && st.multi_col_pct >= 50 && st.avg_cell <= 24
    };
    out.push(Cand {
        ytop: rowys[0],
        ybot: *rowys.last().unwrap(),
        feat: featurize(&st),
        rowys,
        colxs,
        geo_ok,
    });
}

// Segment a page's lines into blocks — maximal runs of consecutive lines with normal line spacing
// (a y-gap > 2 ems, an image, or the reading order jumping back up = a block boundary). Each block is
// a candidate table region for the borderless (no-rules) case.

/// **Propose the candidate table region** bounded by the drawn horizontal rules. The grid the author
/// drew IS the table; we read its structure off the geometry:
/// * **rows** = the (page-border-trimmed) horizontal rules when they form a lattice (≥3), else the
///   text lines' own boundaries (the borderless case).
/// * **columns** = the **merged** vertical rules (doubled strokes collapsed to one edge); if there are
///   too few real vertical rules, the text's own whitespace **gutter valleys** (the soft projection).
///
/// The gate (in [`add_cand`]) decides table-ness — ruled grids on a loose `fill`/`multi_col` read,
/// borderless gutters on the strict prose gate. Every step is O(1)/element on the accumulator
/// (coordinate buckets, projection depth); the only sorts are on the tiny rule/column lists (the page,
/// the null cell) — never the document.
fn table_candidates(lines: &[Line], h: &[(f64, f64, f64)], v: &[(f64, f64, f64)]) -> Vec<Cand> {
    let mut out = Vec::new();
    let hys = cluster_coords(h.iter().map(|r| r.0).collect(), 3.0);
    if hys.len() < 2 {
        return out;
    } // need a top+bottom rule to bound a region
    let rows = trim_outliers(hys); // drop page-border outliers (ascending)
    let (ybot, ytop) = (rows[0], rows[rows.len() - 1]);
    let region: Vec<&Line> = lines
        .iter()
        .filter(|l| l.y <= ytop + 2.0 && l.y >= ybot - 2.0)
        .collect();
    if region.len() < 2 {
        return out;
    }
    // Vertical dividers must CROSS the rows — the lattice property. A table column rule spans the table
    // height; a figure-frame edge / multi-column-layout boundary spans only part of it. Keep the
    // full-height verticals (the geometry's own table/figure discriminator), then collapse doubled strokes.
    let height = (ytop - ybot).max(1.0);
    let vfull: Vec<f64> = v
        .iter()
        .filter(|r| (r.2 - r.1) >= 0.5 * height)
        .map(|r| r.0)
        .collect();
    let merged_v = merge_close(vfull, 10.0); // doubled strokes → one edge
    let ruled = merged_v.len() >= 3; // ≥2 real vertical dividers = a ruled grid
                                     // columns: drawn rules first; else the gutter valleys. (A row-support column read trained on
                                     // FinTabNet hit 82.3% col-exact in-domain but was INERT here: it only runs inside already-detected
                                     // regions, whose columns are already fine — the broken-column tables are the ones never DETECTED.
                                     // The bottleneck is detection/recall, not column reading. `columns_from_row_support` kept for the
                                     // detection restructure, where it can actually move the number.)
    let _ = columns_from_row_support;
    let colxs = if ruled {
        Some(merged_v)
    } else {
        columns_from_gutters(&region)
    };
    let Some(colxs) = colxs else { return out };
    // rows: the trimmed horizontal rules if a real lattice (≥3), else the text lines' boundaries.
    let rowys = if rows.len() >= 3 {
        let mut r = rows;
        r.sort_by(|a, b| b.partial_cmp(a).unwrap());
        r
    } else {
        row_boundaries(&region)
    };
    add_cand(&mut out, lines, rowys, colxs, ruled);
    out
}

/// Emit the lines that fall inside a drawn grid as an HTML `<table>`: each text fragment is placed in
/// the cell whose row-band (between horizontal rules) holds its y and whose column-band (between
/// vertical rules) holds its x. The structure comes from the rules, the content from the text.
fn emit_grid_table(out: &mut String, lines: &[Line], rowys: &[f64], colxs: &[f64]) {
    let (nr, nc) = (rowys.len() - 1, colxs.len() - 1);
    let mut cells = vec![vec![String::new(); nc]; nr];
    let mut maxx = vec![vec![f64::MIN; nc]; nr]; // per-cell rightmost text edge → colspan
    for l in lines {
        let r = (0..nr).find(|&r| l.y <= rowys[r] + 2.0 && l.y >= rowys[r + 1] - 2.0);
        let Some(r) = r else { continue };
        for (si, &(sx, sex, soff, _)) in l.segs.iter().enumerate() {
            let send = l.segs.get(si + 1).map(|s| s.2).unwrap_or(l.text.len());
            let txt = l.text.get(soff..send).unwrap_or("").trim();
            if txt.is_empty() {
                continue;
            }
            let c = (0..nc)
                .find(|&c| sx >= colxs[c] - 2.0 && sx < colxs[c + 1] + 2.0)
                .unwrap_or(0);
            if !cells[r][c].is_empty() {
                cells[r][c].push(' ');
            }
            cells[r][c].push_str(txt);
            maxx[r][c] = maxx[r][c].max(sex);
        }
    }
    out.push_str("<table>\n");
    for r in 0..nr {
        out.push_str("<tr>");
        let mut c = 0;
        while c < nc {
            // colspan (pure geometry): the cell's text reaches PAST THE MIDPOINT of the next column(s),
            // and those columns are otherwise empty — the wide cell occupies them (a spanning header).
            // The empty-neighbour guard stops a real adjacent cell being swallowed; the midpoint guard
            // stops a legitimately-empty trailing column being absorbed.
            let mut span = 1;
            while c + span < nc
                && cells[r][c + span].is_empty()
                && maxx[r][c] > (colxs[c + span] + colxs[c + span + 1]) / 2.0
            {
                span += 1;
            }
            if span > 1 {
                out.push_str(&format!("<td colspan=\"{span}\">"));
            } else {
                out.push_str("<td>");
            }
            html_escape(out, &cells[r][c]);
            out.push_str("</td>");
            c += span;
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

/// Geometric statistics of a candidate grid — the features both the hand-read gate and the region
/// SEED classify on. `multi_col_pct` (rows crossing ≥2 columns) separates a table from a 1-col flow;
/// `avg_cell` (chars per cell) separates a table's short cells from a prose layout's long ones.
struct RegionStats {
    nr: usize,
    nc: usize,
    fill_pct: usize,
    avg_cell: usize,
    multi_col_pct: usize,
    ruled: bool,
}

fn region_stats(lines: &[Line], rowys: &[f64], colxs: &[f64], ruled: bool) -> RegionStats {
    let (nr, nc) = (rowys.len().saturating_sub(1), colxs.len().saturating_sub(1));
    if nr == 0 || nc == 0 {
        return RegionStats {
            nr,
            nc,
            fill_pct: 0,
            avg_cell: 999,
            multi_col_pct: 0,
            ruled,
        };
    }
    let mut seen = vec![vec![false; nc]; nr];
    let mut per_row = vec![0usize; nr];
    let mut text_len = 0usize;
    for l in lines {
        let Some(r) = (0..nr).find(|&r| l.y <= rowys[r] + 2.0 && l.y >= rowys[r + 1] - 2.0) else {
            continue;
        };
        text_len += l.text.chars().count();
        for &(sx, _, _, _) in &l.segs {
            if let Some(c) = (0..nc).find(|&c| sx >= colxs[c] - 2.0 && sx < colxs[c + 1] + 2.0) {
                if !seen[r][c] {
                    seen[r][c] = true;
                    per_row[r] += 1;
                }
            }
        }
    }
    let filled: usize = per_row.iter().sum();
    let rows_with_text = per_row.iter().filter(|&&c| c >= 1).count().max(1);
    let multi_col_rows = per_row.iter().filter(|&&c| c >= 2).count();
    RegionStats {
        nr,
        nc,
        fill_pct: filled * 100 / (nr * nc),
        avg_cell: text_len.checked_div(filled).unwrap_or(999),
        multi_col_pct: multi_col_rows * 100 / rows_with_text,
        ruled,
    }
}

/// The feature signature the region seed reads (the model-is-a-seed): each stat bucketed and packed
/// into one key. The seed folds `key → (table, ¬table)` counts; classify is one `get` — O(1).
fn featurize(s: &RegionStats) -> u64 {
    let bucket =
        |v: usize, edges: &[usize]| edges.iter().position(|&e| v < e).unwrap_or(edges.len()) as u64;
    bucket(s.nc, &[3, 4, 5, 7])
        | (bucket(s.nr, &[3, 5, 8, 14]) << 3)
        | (bucket(s.fill_pct, &[60, 75, 90]) << 6)
        | (bucket(s.avg_cell, &[6, 12, 20, 30, 45]) << 9)
        | (bucket(s.multi_col_pct, &[60, 80, 95]) << 12)
        | ((s.ruled as u64) << 15)
}

/// Emit one page's lines, with the already-gated table candidates inserted at their positions: a line
/// inside a candidate's y-span is consumed into its `<table>`; everything else is heading / image / body.
fn emit_lines(out: &mut String, lines: &[Line], cands: &[Cand], body: f64) {
    let mut emitted = vec![false; cands.len()];
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let hit = cands
            .iter()
            .enumerate()
            .find(|(ci, c)| !emitted[*ci] && l.y <= c.ytop + 2.0 && l.y >= c.ybot - 2.0)
            .map(|(ci, _)| ci);
        if let Some(ci) = hit {
            let c = &cands[ci];
            emit_grid_table(out, lines, &c.rowys, &c.colxs);
            emitted[ci] = true;
            while i < lines.len() && lines[i].y <= c.ytop + 2.0 && lines[i].y >= c.ybot - 2.0 {
                i += 1;
            }
            continue;
        }
        if let Some(obj) = l.text.strip_prefix('\u{0}') {
            // an image marker (the Do position)
            out.push_str(&format!("![](img_{}.png)\n", obj));
            i += 1;
            continue;
        }
        let heading = is_heading(&l.text, l.size, l.bold, body);
        if heading {
            out.push_str("# ");
        }
        // A word broken at a line end (`compari-` / `son`) is one word: the hyphen is dropped and
        // the next line continues it. The next line must be ordinary text starting in lower case.
        let continues = lines.get(i + 1).is_some_and(|n| {
            let in_table = cands
                .iter()
                .enumerate()
                .any(|(ci, c)| !emitted[ci] && n.y <= c.ytop + 2.0 && n.y >= c.ybot - 2.0);
            !in_table
                && !n.text.starts_with('\u{0}')
                && n.text.starts_with(|c: char| c.is_lowercase())
        });
        match l.text.strip_suffix('-') {
            Some(stem) if !heading && continues && stem.ends_with(|c: char| c.is_alphabetic()) => {
                out.push_str(stem);
            }
            _ => {
                out.push_str(&l.text);
                out.push('\n');
            }
        }
        i += 1;
    }
}
/// PDF string bytes → text. Latin-1 for now (each byte a char); the CMap glyph→unicode is wave 3.
fn decode_str(s: &[u8]) -> String {
    s.iter().map(|&b| b as char).collect()
}

impl Document {
    /// PDF → markdown with the **geometric** table gate (no seed). No sort, no bsearch — reading order.
    pub fn markdown(&self) -> String {
        self.markdown_inner(None)
    }

    /// PDF → markdown with the **region seed** gating which candidate regions become tables (the
    /// model-is-a-seed, O(1) classify per candidate).
    pub fn markdown_seeded(&self, seed: &super::RegionSeed) -> String {
        self.markdown_inner(Some(seed))
    }

    /// One markdown string per page, in page order (the same layout as `markdown`). A page with no
    /// selectable text yields an empty or whitespace-only string.
    pub fn markdown_pages(&self) -> Vec<String> {
        self.markdown_pages_inner(None)
    }

    fn markdown_inner(&self, seed: Option<&super::RegionSeed>) -> String {
        self.markdown_pages_inner(seed).concat()
    }

    fn markdown_pages_inner(&self, seed: Option<&super::RegionSeed>) -> Vec<String> {
        let mut pages_out = Vec::new();
        let pdata = self.page_data();
        // The body font size = the char-weighted modal size (the THRONE of the size histogram); headings
        // are the off-throne lines, OR bold / section-numbered same-size lines.
        let mut hist: Table<i64, i64> = Table::new();
        for (lines, _) in &pdata {
            for l in lines {
                if l.size > 0.0 {
                    hist.add((l.size * 2.0).round() as i64, l.text.chars().count() as i64);
                }
            }
        }
        let body = hist
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(k, _)| *k as f64 / 2.0)
            .unwrap_or(0.0);
        for (lines, (h, v)) in &pdata {
            // generate candidates, then GATE by the seed (if given) or the geometric hand-read.
            let cands: Vec<Cand> = table_candidates(lines, h, v)
                .into_iter()
                .filter(|c| match seed {
                    Some(s) => s.classify_or(c.feat, c.geo_ok),
                    None => c.geo_ok,
                })
                .collect();
            let mut out = String::new();
            emit_lines(&mut out, lines, &cands, body);
            out.push('\n'); // page break
            pages_out.push(out);
        }
        pages_out
    }

    /// Per page: the laid-out lines (document = reading order) + the drawn ruling lines. The shared
    /// prepare for markdown / training.
    fn page_data(&self) -> PageData {
        let mut pages = Vec::new();
        if let Some(Object::Dict(cat)) = self.object(self.root) {
            let mut seen = super::Table::new();
            match cat.get(&"Pages".to_string()) {
                Some(Object::Ref(pr, pg)) => self.collect_pages(*pr, *pg, &mut pages, 0, &mut seen),
                _ => self.collect_pages(self.root, 0, &mut pages, 0, &mut seen),
            }
        }
        let mut pdata = Vec::new();
        let mut contents = ContentCache::new();
        for pnum in pages {
            if let Some(Object::Dict(page)) = self.object(pnum) {
                let fonts = self.page_fonts(page, &mut contents);
                let imgs = self.image_names(page);
                let content = self.page_content(page, &mut contents);
                let forms = self.form_texts(page, &mut contents);
                let lines: Vec<Line> = extract_text(&content, &fonts, &imgs, &forms)
                    .into_iter()
                    .filter(|l| !l.text.trim().is_empty())
                    .collect();
                pdata.push((lines, extract_rules(&content)));
            }
        }
        pdata
    }

    /// The Form XObjects of a page's resources, decoded: each one's content and, when it has its own
    /// `/Resources`, its own fonts and images.
    fn form_texts(&self, page: &Dict, cache: &mut ContentCache) -> Table<String, FormText> {
        let mut out = Table::new();
        let Some(res) = self.as_dict(page.get(&"Resources".to_string())) else {
            return out;
        };
        let Some(xobjects) = self.as_dict(res.get(&"XObject".to_string())) else {
            return out;
        };
        for (name, r) in xobjects.iter() {
            let Object::Ref(n, g) = r else {
                continue;
            };
            let Some(Object::Stream(d, _)) = self.ref_object(*n, *g) else {
                continue;
            };
            if !matches!(d.get(&"Subtype".to_string()), Some(Object::Name(s)) if s == "Form") {
                continue;
            }
            let Some(content) = cache.stream(self, *n, *g) else {
                continue;
            };
            let own = if d.get(&"Resources".to_string()).is_some() {
                Some((self.page_fonts(d, cache), self.image_names(d)))
            } else {
                None
            };
            let matrix = form_matrix(d);
            out.put(
                name.clone(),
                FormText {
                    content,
                    own,
                    matrix,
                },
            );
        }
        out
    }

    /// Every candidate region's `(feature signature, geometric-verdict)` across the doc — for FOLDING
    /// the region seed. The geometric verdict lets the trainer label cleanly (a table doc's geo-ok
    /// candidate is a positive; a non-table doc's any candidate is a negative; the rest are ambiguous).
    pub fn table_candidate_feats(&self) -> Vec<(u64, bool)> {
        let mut out = Vec::new();
        for (lines, (h, v)) in &self.page_data() {
            for c in table_candidates(lines, h, v) {
                out.push((c.feat, c.geo_ok));
            }
        }
        out
    }

    /// Per page, the positioned text **boxes** — `(text, x0, x1, y)` per run/segment — the native α=2D
    /// box layout the structure read (the `Web.sweep` edge-read) operates on. This is pdffold's
    /// content-stream lift exposed for the structure fold: the geometry is already here, no pixels.
    pub fn page_boxes(&self) -> Vec<Vec<(String, f64, f64, f64)>> {
        let mut out = Vec::new();
        for (lines, _) in &self.page_data() {
            let mut boxes = Vec::new();
            for l in lines {
                for (si, &(x0, x1, soff, _)) in l.segs.iter().enumerate() {
                    let send = l.segs.get(si + 1).map(|s| s.2).unwrap_or(l.text.len());
                    let txt = l.text.get(soff..send).unwrap_or("").trim().to_string();
                    if !txt.is_empty() {
                        boxes.push((txt, x0, x1, l.y));
                    }
                }
            }
            out.push(boxes);
        }
        out
    }

    /// Diagnostic: per page, the clustered horizontal-rule y's and vertical-rule x's (the drawn grid).
    pub fn rule_dump(&self) -> Vec<(Vec<f64>, Vec<f64>)> {
        let mut out = Vec::new();
        for (_lines, (h, v)) in &self.page_data() {
            out.push((
                cluster_coords(h.iter().map(|r| r.0).collect(), 3.0),
                cluster_coords(v.iter().map(|r| r.0).collect(), 3.0),
            ));
        }
        out
    }

    /// Diagnostic: per page with an hrule-bounded region, `(rule_cols, gutter_cols)` — does reading
    /// columns from the TEXT's whitespace gutters give the logical column count vs the raw rule count?
    pub fn gutter_vs_rule(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (lines, (h, v)) in &self.page_data() {
            let hys = cluster_coords(h.iter().map(|r| r.0).collect(), 3.0);
            let vxs = cluster_coords(v.iter().map(|r| r.0).collect(), 3.0);
            if hys.len() < 2 {
                continue;
            }
            let (ytop, ybot) = (
                hys.iter().cloned().fold(f64::MIN, f64::max),
                hys.iter().cloned().fold(f64::MAX, f64::min),
            );
            let region: Vec<&Line> = lines
                .iter()
                .filter(|l| l.y <= ytop + 2.0 && l.y >= ybot - 2.0)
                .collect();
            let gcols = columns_from_gutters(&region)
                .map(|c| c.len().saturating_sub(1))
                .unwrap_or(0);
            out.push((vxs.len().saturating_sub(1), gcols));
        }
        out
    }

    /// Diagnostic: per page, the raw candidate stats + rule counts — why a region did or didn't pass the
    /// gate. Each entry `(nr, nc, fill_pct, avg_cell, multi_col_pct, ruled, geo_ok, n_hrules, n_vrules)`;
    /// a page with no candidate emits one all-zero row carrying just the rule counts (ruled vs borderless).
    pub fn table_debug(&self) -> Vec<TableDebugRow> {
        let mut out = Vec::new();
        for (lines, (h, v)) in &self.page_data() {
            let nh = cluster_coords(h.iter().map(|r| r.0).collect(), 3.0).len();
            let nv = cluster_coords(v.iter().map(|r| r.0).collect(), 3.0).len();
            let cands = table_candidates(lines, h, v);
            if cands.is_empty() {
                out.push((0, 0, 0, 0, 0, false, false, nh, nv));
                continue;
            }
            for c in cands {
                let st = region_stats(lines, &c.rowys, &c.colxs, nv >= 2);
                out.push((
                    st.nr,
                    st.nc,
                    st.fill_pct,
                    st.avg_cell,
                    st.multi_col_pct,
                    st.ruled,
                    c.geo_ok,
                    nh,
                    nv,
                ));
            }
        }
        out
    }

    /// The page's image XObjects: `/Resources /XObject` name → object number, for the `Do` operator
    /// to resolve. Only `/Subtype /Image` entries (form XObjects are not images).
    fn image_names(&self, page: &Dict) -> Table<String, u32> {
        let mut m = Table::new();
        if let Some(res) = self.as_dict(page.get(&"Resources".to_string())) {
            if let Some(xo) = self.as_dict(res.get(&"XObject".to_string())) {
                for (name, r) in xo.iter() {
                    if let Object::Ref(n, g) = r {
                        if let Some(Object::Stream(d, _)) = self.ref_object(*n, *g) {
                            if matches!(d.get(&"Subtype".to_string()), Some(Object::Name(s)) if s == "Image")
                            {
                                m.put(name.clone(), *n);
                            }
                        }
                    }
                }
            }
        }
        m
    }

    /// The object numbers of the pages, in page-tree order.
    pub(crate) fn page_numbers(&self) -> Vec<u32> {
        let mut pages = Vec::new();
        if let Some(Object::Dict(cat)) = self.object(self.root) {
            let mut seen = super::Table::new();
            match cat.get(&"Pages".to_string()) {
                Some(Object::Ref(pr, pg)) => self.collect_pages(*pr, *pg, &mut pages, 0, &mut seen),
                _ => self.collect_pages(self.root, 0, &mut pages, 0, &mut seen),
            }
        }
        pages
    }

    /// Walk the page tree. Each node is visited once (`seen`), so a `/Kids` list that names a node
    /// twice, or names an ancestor, cannot multiply the walk; the depth bound stays as well.
    fn collect_pages(
        &self,
        num: u32,
        gen: u16,
        out: &mut Vec<u32>,
        depth: u32,
        seen: &mut super::Table<u32, bool>,
    ) {
        if depth > 64 || seen.has(&num) {
            return;
        }
        seen.put(num, true);
        let Some(Object::Dict(d)) = self.ref_object(num, gen) else {
            return;
        };
        match d.get(&"Type".to_string()) {
            Some(Object::Name(t)) if t == "Page" => out.push(num),
            _ => {
                if let Some(kids) = d.get(&"Kids".to_string()) {
                    let kids = match kids {
                        Object::Array(a) => a.clone(),
                        _ => return,
                    };
                    for k in &kids {
                        if let Object::Ref(n, g) = k {
                            self.collect_pages(*n, *g, out, depth + 1, seen);
                        }
                    }
                }
            }
        }
    }

    fn page_content(&self, page: &Dict, cache: &mut ContentCache) -> Vec<u8> {
        let mut bytes = Vec::new();
        match page.get(&"Contents".to_string()) {
            Some(Object::Ref(n, g)) => {
                if let Some(b) = cache.stream(self, *n, *g) {
                    bytes.extend_from_slice(&b);
                }
            }
            Some(Object::Array(a)) => {
                for r in a {
                    if let Object::Ref(n, g) = r {
                        if let Some(b) = cache.stream(self, *n, *g) {
                            bytes.extend_from_slice(&b);
                            bytes.push(b'\n');
                        }
                    }
                }
            }
            _ => {}
        }
        bytes
    }
}

/// Inflate budget for the page contents of one walk of the page tree.
const CONTENT_BUDGET: usize = 512 << 20;

/// The decoded content streams of one walk, by object number. A stream that several pages share
/// is inflated once, and the walk as a whole inflates at most `CONTENT_BUDGET` bytes: a failed
/// inflate is charged its cap, so repeated bombs cannot multiply the work.
pub(crate) struct ContentCache {
    decoded: super::Table<u32, Option<std::rc::Rc<Vec<u8>>>>,
    budget: usize,
}

impl ContentCache {
    fn new() -> Self {
        ContentCache {
            decoded: super::Table::new(),
            budget: CONTENT_BUDGET,
        }
    }

    /// The decoded bytes of the stream that `n g R` names, or `None` when the reference is stale,
    /// absent, or refuses. The generation is checked before the cache, so a stale reference never
    /// receives the bytes cached for the current object with the same number.
    fn stream(&mut self, doc: &Document, n: u32, g: u16) -> Option<std::rc::Rc<Vec<u8>>> {
        doc.ref_object(n, g)?;
        if let Some(done) = self.decoded.get(&n) {
            return done.clone();
        }
        let out = match doc.object(n) {
            Some(Object::Stream(d, raw)) => {
                let cap = self.budget.min(super::decode::MAX_DECODED);
                let resolved = super::decode::with_resolved_parms(d, &|pn, pg| {
                    doc.ref_object(pn, pg).cloned()
                });
                let out = super::decode::decode_within(&resolved, raw, cap);
                let charge = match &out {
                    Some(v) => v.len(),
                    None if super::decode::inflates(d) => cap,
                    None => 0,
                };
                self.budget = self.budget.saturating_sub(charge);
                out.map(std::rc::Rc::new)
            }
            _ => None,
        };
        self.decoded.put(n, out.clone());
        out
    }
}

/// The text-positioning state — the text matrix reduced to its h/v scale + pen position, the spacing
/// params, and the **previous run's end-x**. A space is emitted only when a run opens a real gap past
/// its predecessor (`x − end_x > ¼ em`): the radius-1 **edge read** (`os/.../Web.lean` `sweep`,   CITATION-EXEMPT: an elided path, not a real one — see `proof/Substrate/Algebra/Web/Web.lean`
/// `sweep_depth_is_radius`), NOT a pointwise per-operand guess (which provably dies). Newlines come
/// from genuine vertical moves. Each op O(1), each show O(run len) — no sort, no search.
/// One laid-out line — the joined text plus its font signals AND its **segments**: each positioned run
/// `(x, byte-offset into `text`)`. The segments are the cell candidates; a table is recovered by
/// clustering their x-positions into columns (the geometry's grid DAG, not a parse).
struct Line {
    text: String,
    size: f64,
    bold: bool,
    y: f64,
    segs: Vec<(f64, f64, usize, bool)>,
}

struct Pen {
    lines: Vec<Line>,
    cur: String,
    cur_size: f64,
    cur_bold_n: usize,
    cur_n: usize,
    cur_y: f64,
    cur_segs: Vec<(f64, f64, usize, bool)>,
    x: f64,
    y: f64,
    lx: f64,
    ly: f64,
    end_x: f64,
    last_y: f64,
    started: bool,
    fs: f64,
    a: f64,
    d: f64,
    tc: f64,
    tw: f64,
    tz: f64,
    leading: f64,
    /// The one-em scales of the previous run, horizontal and vertical (see `show`).
    last_sh: f64,
    last_sv: f64,
}
impl Pen {
    fn new() -> Pen {
        Pen {
            lines: Vec::new(),
            cur: String::new(),
            cur_size: 0.0,
            cur_bold_n: 0,
            cur_n: 0,
            cur_y: 0.0,
            cur_segs: Vec::new(),
            x: 0.0,
            y: 0.0,
            lx: 0.0,
            ly: 0.0,
            end_x: 0.0,
            last_y: 0.0,
            started: false,
            fs: 1.0,
            a: 1.0,
            d: 1.0,
            tc: 0.0,
            tw: 0.0,
            tz: 1.0,
            leading: 0.0,
            last_sh: 0.0,
            last_sv: 0.0,
        }
    }
    /// Flush the line, carrying its font size (heading signal), bold flag, baseline y, and segments.
    fn flush(&mut self) {
        if !self.cur.is_empty() {
            let bold = self.cur_n > 0 && self.cur_bold_n * 2 >= self.cur_n;
            self.lines.push(Line {
                text: std::mem::take(&mut self.cur),
                size: self.cur_size,
                bold,
                y: self.cur_y,
                segs: std::mem::take(&mut self.cur_segs),
            });
        }
        self.cur_size = 0.0;
        self.cur_bold_n = 0;
        self.cur_n = 0;
        self.cur_segs.clear();
    }
    /// Show a run (`Tj` string, or `TJ` strings + kern numbers). Two edge signals, kept separate so
    /// width-estimate drift can't manufacture spaces:
    /// * **within a run** — the kern number is the *exact* inter-glyph displacement; `−k/1000 > ¼ em`
    ///   is a space (no width estimate involved).
    /// * **between runs** — at the run's first glyph, the gap `x − prev_end_x` (estimated end vs the
    ///   reposition's exact start) opens a space past a conservative threshold; one word's width error
    ///   is small, and long-run drift is never checked here (only the first glyph is).
    ///
    /// A vertical move since the last glyph is a newline. The pen advances by each glyph's `/Widths`.
    fn show(&mut self, elems: &[Tok], font: Option<&CMap>) {
        let sh = self.fs * self.a * self.tz; // one em → user-space (horizontal)
        let sv = self.fs * self.d; // one em → user-space (vertical), for the line-break test
                                   // A vertical move is a new line only past half the larger of this run's and the previous run's
                                   // em, so a raised label of a smaller size stays on its line.
        let line_em = 0.5 * sv.abs().max(self.last_sv.abs());
        self.last_sv = sv;
        // between-run gap: a word space. Exact widths leave little drift, and an interword space in
        // a 10pt Latin Modern font measures about 0.27 em (2.73 pt), so the bar is 0.15 em. The em is
        // the larger of this run's and the previous run's, so a small raised label (a 7pt exponent
        // after 10pt text) does not read a space it does not have.
        let thr = 0.15 * sh.abs().max(self.last_sh.abs()).max(1e-6);
        self.last_sh = sh;
        // The between-run width-gap is trustworthy only when this font declares its widths (`/Widths`,
        // or a Type0 font's `/W`), so end-x is exact. Without them the end-x would drift and fabricate
        // gaps; those fonts fall back to the exact signals (kern + explicit space).
        let has_w = font.is_some_and(|c| c.widths);
        let mut first = true; // first glyph of THIS run carries the between-run edge
        let mut pending = false; // an exact kern gap awaiting the next glyph
        for el in elems {
            match el {
                Tok::Num(k) => {
                    self.x += -(k / 1000.0) * sh; // +k moves left (tighter), −k opens a gap
                    if -(k / 1000.0) > 0.18 {
                        pending = true;
                    }
                }
                Tok::Str(s) => {
                    let mut big = false;
                    if self.started && first && (self.y - self.last_y).abs() > line_em {
                        self.flush(); // new line
                    } else if self.started && !self.cur.is_empty() && !self.cur.ends_with(' ') {
                        let gap = self.x - self.end_x;
                        if first && has_w && gap > 0.45 * sh.abs() {
                            self.cur.push(' ');
                            big = true;
                        }
                        // COLUMN gap
                        else if (first && has_w && gap > thr) || pending {
                            self.cur.push(' ');
                        } // word gap
                    }
                    // A segment per positioned run (every Tj/TJ), flagged `big` if a column gap preceded
                    // it: the rules-grid path uses all positions, the text-column path only the big ones.
                    if first || self.cur_segs.is_empty() {
                        self.cur_segs.push((self.x, self.x, self.cur.len(), big));
                    }
                    pending = false;
                    first = false;
                    match font {
                        Some(c) => self.cur.push_str(&c.decode(s)),
                        None => self.cur.push_str(&decode_str(s)),
                    }
                    let (em, nch, nsp) = match font {
                        Some(c) => c.run_metrics(s),
                        None => {
                            let n = s.len();
                            (n as f64 * 0.5, n, 0)
                        }
                    };
                    self.x += em * sh
                        + nch as f64 * self.tc * self.a * self.tz
                        + nsp as f64 * self.tw * self.a * self.tz;
                    if let Some(last) = self.cur_segs.last_mut() {
                        last.1 = self.x;
                    } // the run's end-x (its extent)
                    self.cur_size = self.cur_size.max((self.fs * self.d).abs()); // the line's font size (heading signal)
                    self.cur_n += nch;
                    if font.is_some_and(|c| c.bold) {
                        self.cur_bold_n += nch;
                    } // bold fraction
                    self.end_x = self.x;
                    self.last_y = self.y;
                    self.cur_y = self.y;
                    self.started = true;
                }
                _ => {}
            }
        }
    }
}

/// The content-stream **text accumulator**, geometry-aware. Tracks the text matrix (`Tm`/`Td`/`TD`/
/// `T*`) and font/spacing state; `Tj`/`TJ`/`'`/`"` show runs through [`Pen::show`], which reads the
/// inter-run gap (the edge) to place spaces and the vertical move to break lines. Document order is
/// the stream order — no sort, no binary search.
/// A Form XObject a page paints with `Do`: its decoded content, and its own fonts and images when it
/// has `/Resources` (otherwise the page's are used).
struct FormText {
    content: std::rc::Rc<Vec<u8>>,
    own: Option<(Table<String, CMap>, Table<String, u32>)>,
    /// The form's `/Matrix` (form space to user space); the identity when absent or malformed.
    matrix: [f64; 6],
}

/// A `/Matrix` array of six numbers, or the identity when it is absent or not six numbers.
fn form_matrix(d: &Dict) -> [f64; 6] {
    const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let Some(Object::Array(items)) = d.get(&"Matrix".to_string()) else {
        return IDENTITY;
    };
    let nums: Vec<f64> = items
        .iter()
        .filter_map(|o| match o {
            Object::Int(n) => Some(*n as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        })
        .collect();
    match <[f64; 6]>::try_from(nums.as_slice()) {
        Ok(m) if m.iter().all(|v| v.is_finite()) => m,
        _ => IDENTITY,
    }
}

/// The inverse of an affine matrix, or `None` when it is singular.
fn invert(m: &[f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

/// Map a form's lines from form space into the frame the page's own text is laid out in.
fn place_form_lines(lines: &mut [Line], m: &[f64; 6]) {
    for l in lines {
        let x0 = l.segs.first().map_or(0.0, |s| s.0);
        let y = l.y;
        l.y = m[1] * x0 + m[3] * y + m[5];
        for seg in &mut l.segs {
            seg.0 = m[0] * seg.0 + m[2] * y + m[4];
            seg.1 = m[0] * seg.1 + m[2] * y + m[4];
        }
    }
}

fn extract_text(
    content: &[u8],
    fonts: &Table<String, CMap>,
    imgs: &Table<String, u32>,
    forms: &Table<String, FormText>,
) -> Vec<Line> {
    let mut p = Pen::new();
    // The CTM (`cm`/`q`/`Q`), tracked only to place Form XObjects: page text is laid out in text
    // space, so a form's lines are mapped relative to the CTM in force at the last `BT`.
    let mut ctm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut ctm_stack: Vec<[f64; 6]> = Vec::new();
    let mut text_ctm = ctm;
    let mut font: Option<&CMap> = None; // the current font's CMap, set by Tf (the Map face)
    let mut ops: Vec<Tok> = Vec::new();
    let mut lex = Lex::new(content);
    while let Some(t) = lex.next() {
        if let Tok::Kw(op) = &t {
            let nums: Vec<f64> = ops.iter().filter_map(num).collect();
            let last_str = || ops.iter().rev().find(|t| matches!(t, Tok::Str(_)));
            match op.as_str() {
                "cm" if nums.len() >= 6 => {
                    let n = nums.len();
                    ctm = matmul(
                        &[
                            nums[n - 6],
                            nums[n - 5],
                            nums[n - 4],
                            nums[n - 3],
                            nums[n - 2],
                            nums[n - 1],
                        ],
                        &ctm,
                    );
                }
                "q" => ctm_stack.push(ctm),
                "Q" => {
                    if let Some(c) = ctm_stack.pop() {
                        ctm = c;
                    }
                }
                "BT" => {
                    text_ctm = ctm;
                    p.x = 0.0;
                    p.y = 0.0;
                    p.lx = 0.0;
                    p.ly = 0.0;
                    p.a = 1.0;
                    p.d = 1.0;
                    p.started = false;
                }
                "ET" => {
                    p.flush();
                    p.started = false;
                }
                "Tf" => {
                    if let Some(Tok::Name(n)) = ops.iter().find(|t| matches!(t, Tok::Name(_))) {
                        font = fonts.get(n);
                    }
                    if let Some(sz) = nums.last() {
                        p.fs = *sz;
                    }
                }
                "Tc" => {
                    if let Some(v) = nums.last() {
                        p.tc = *v;
                    }
                }
                "Tw" => {
                    if let Some(v) = nums.last() {
                        p.tw = *v;
                    }
                }
                "Tz" => {
                    if let Some(v) = nums.last() {
                        p.tz = *v / 100.0;
                    }
                }
                "TL" => {
                    if let Some(v) = nums.last() {
                        p.leading = *v;
                    }
                }
                "Tm" => {
                    if nums.len() >= 6 {
                        p.a = nums[0];
                        p.d = nums[3];
                        p.lx = nums[4];
                        p.ly = nums[5];
                        p.x = nums[4];
                        p.y = nums[5];
                    }
                }
                "Td" | "TD" => {
                    if nums.len() >= 2 {
                        let (tx, ty) = (nums[nums.len() - 2], nums[nums.len() - 1]);
                        p.lx += p.a * tx;
                        p.ly += p.d * ty;
                        p.x = p.lx;
                        p.y = p.ly;
                        if op.as_str() == "TD" {
                            p.leading = -ty;
                        }
                    }
                }
                "T*" => {
                    p.ly += p.d * (-p.leading);
                    p.x = p.lx;
                    p.y = p.ly;
                }
                "Tj" => {
                    if let Some(s) = last_str() {
                        p.show(std::slice::from_ref(s), font);
                    }
                }
                "TJ" => {
                    p.show(&ops, font);
                }
                "'" => {
                    p.ly += p.d * (-p.leading);
                    p.x = p.lx;
                    p.y = p.ly;
                    if let Some(s) = last_str() {
                        p.show(std::slice::from_ref(s), font);
                    }
                }
                "\"" => {
                    if nums.len() >= 2 {
                        p.tw = nums[0];
                        p.tc = nums[1];
                    }
                    p.ly += p.d * (-p.leading);
                    p.x = p.lx;
                    p.y = p.ly;
                    if let Some(s) = last_str() {
                        p.show(std::slice::from_ref(s), font);
                    }
                }
                // Do: paint an XObject. If it names an image, drop an image marker at THIS point in the
                // document flow (the picture's reading-order position) — markdown() renders it `![](…)`.
                "Do" => {
                    if let Some(Tok::Name(n)) = ops.iter().rev().find(|t| matches!(t, Tok::Name(_)))
                    {
                        if let Some(form) = forms.get(n) {
                            // A form's text is laid out at its paint point, with its own fonts (or the
                            // page's). Forms are not nested: the empty table ends the recursion.
                            let (ff, fi) = match &form.own {
                                Some((f, i)) => (f, i),
                                None => (fonts, imgs),
                            };
                            p.flush();
                            let mut inner = extract_text(&form.content, ff, fi, &Table::new());
                            // Form space -> user space at this paint point -> the page-text frame.
                            let user = matmul(&form.matrix, &ctm);
                            let placed = invert(&text_ctm).map_or(user, |inv| matmul(&user, &inv));
                            place_form_lines(&mut inner, &placed);
                            p.lines.extend(inner);
                            p.started = false;
                        } else if let Some(obj) = imgs.get(n) {
                            p.flush();
                            p.lines.push(Line {
                                text: format!("\u{0}{}", obj),
                                size: 0.0,
                                bold: false,
                                y: p.y,
                                segs: Vec::new(),
                            });
                            p.started = false;
                        }
                    }
                }
                _ => {}
            }
            ops.clear();
        } else {
            ops.push(t);
        }
    }
    p.flush();
    p.lines
}

// ── the glyph→unicode decode: the font's /ToUnicode CMap IS the Map face (code → unicode), O(1) get.
//    `meaningOf` with the cmap as the applicative — no per-format branch, just another `get`. ──

use super::Table;

/// A font's code→unicode decode: two Map faces (the `/ToUnicode` cmap and the `/Encoding /Differences`
/// glyph map) + the code byte-width. Decode is `get` per code (O(1)): ToUnicode first, then Encoding.
pub struct CMap {
    map: Table<u32, String>,
    enc: Table<u32, String>,
    width: usize,
    /// code → glyph advance (1000-em units), the `/Widths` Map face — built once, `get` per code.
    /// This is what makes a run's **end-x** real, so the inter-run gap (the edge) can be read.
    adv: Table<u32, i64>,
    dw: i64,
    /// Whether the font declares widths (`/Widths` or a CIDFont `/W`), so `adv` gives end-x exactly.
    widths: bool,
    /// is this a bold font? (BaseFont name or FontDescriptor ForceBold/FontWeight) — a heading signal.
    bold: bool,
}
impl CMap {
    /// Decode a PDF text string by reading `width`-byte codes and `get`-ing each (O(1) per code).
    fn decode(&self, bytes: &[u8]) -> String {
        let w = self.width.max(1);
        let mut out = String::new();
        let mut i = 0;
        while i < bytes.len() {
            let mut code = 0u32;
            for j in 0..w {
                if i + j < bytes.len() {
                    code = (code << 8) | bytes[i + j] as u32;
                }
            }
            i += w;
            match self.map.get(&code).or_else(|| self.enc.get(&code)) {
                Some(s) => out.push_str(s),
                None => {
                    if w == 1 && code >= 0x20 {
                        out.push(code as u8 as char);
                    }
                } // fallback: Latin-1 (printable only)
            }
        }
        out
    }

    /// **Run metrics for the edge read:** sum the glyph advances (in em units, `Σ width/1000`) and
    /// count chars + spaces. The caller scales by `fs·a·tz` (+ `tc`/`tw` spacing) to get the run's
    /// user-space width, hence its **end-x** — so the gap to the next run can be measured. `get` per
    /// code against the `/Widths` Map face (O(1)/code); empty widths fall back to a half-em default.
    fn run_metrics(&self, bytes: &[u8]) -> (f64, usize, usize) {
        let w = self.width.max(1);
        let (mut em, mut nchars, mut nspaces) = (0.0f64, 0usize, 0usize);
        let mut i = 0;
        while i < bytes.len() {
            let mut code = 0u32;
            for j in 0..w {
                if i + j < bytes.len() {
                    code = (code << 8) | bytes[i + j] as u32;
                }
            }
            i += w;
            let gw = *self.adv.get(&code).unwrap_or(&self.dw);
            em += gw as f64 / 1000.0;
            nchars += 1;
            if code == 32 {
                nspaces += 1;
            }
        }
        (em, nchars, nspaces)
    }
}

/// glyph name → unicode, the **Adobe Glyph List algorithm**. Try the whole name (`base_glyph`); else
/// the name is a ligature/variant: split on `_` into components, strip each component's `.variant`
/// suffix (`.oldstyle`/`.sc`/`.a` — oldstyle figures, small caps, alternates), map each base, join.
/// So `f_l`→"fl", `five.oldstyle`→"5", `T_h`→"Th", `f_f_i`→"ffi". Resolution is per glyph name at the
/// one-time `encoding_map` build (the prepare); the result fills the `enc` Map face, decode stays `get`.
fn glyph_to_unicode(name: &str) -> Option<String> {
    if let Some(s) = base_glyph(name) {
        return Some(s);
    }
    let mut out = String::new();
    let mut any = false;
    for comp in name.split('_') {
        let core = comp.split('.').next().unwrap_or(comp); // drop the .variant suffix
        if let Some(s) = base_glyph(core) {
            out.push_str(&s);
            any = true;
        }
    }
    if any {
        Some(out)
    } else {
        None
    }
}

/// The base glyph-name lookup: `/uniXXXX`, single ASCII letters (the name IS the char), digit names,
/// and the standard punctuation/ligature/accent names. The `/Encoding /Differences` Map value side.
fn base_glyph(name: &str) -> Option<String> {
    if let Some(h) = name.strip_prefix("uni") {
        if h.len() >= 4 {
            if let Ok(cp) = u32::from_str_radix(&h[..4], 16) {
                return char::from_u32(cp).map(|c| c.to_string());
            }
        }
    }
    if let Some(h) = name.strip_prefix('u') {
        if (4..=6).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit()) {
            if let Ok(cp) = u32::from_str_radix(h, 16) {
                return char::from_u32(cp).map(|c| c.to_string());
            }
        }
    }
    if name.len() == 1 {
        let c = name.as_bytes()[0];
        if c.is_ascii_graphic() {
            return Some((c as char).to_string());
        }
    }
    let s: &str = match name {
        "zero" => "0",
        "one" => "1",
        "two" => "2",
        "three" => "3",
        "four" => "4",
        "five" => "5",
        "six" => "6",
        "seven" => "7",
        "eight" => "8",
        "nine" => "9",
        "space" => " ",
        "period" => ".",
        "comma" => ",",
        "colon" => ":",
        "semicolon" => ";",
        "exclam" => "!",
        "question" => "?",
        "quotesingle" => "'",
        "quotedbl" => "\"",
        "grave" => "`",
        "hyphen" | "minus" | "sfthyphen" => "-",
        "underscore" => "_",
        "slash" => "/",
        "backslash" => "\\",
        "parenleft" => "(",
        "parenright" => ")",
        "bracketleft" => "[",
        "bracketright" => "]",
        "braceleft" => "{",
        "braceright" => "}",
        "bar" => "|",
        "asciitilde" => "~",
        "asciicircum" => "^",
        "at" => "@",
        "numbersign" => "#",
        "dollar" => "$",
        "percent" => "%",
        "ampersand" => "&",
        "asterisk" => "*",
        "plus" => "+",
        "equal" => "=",
        "less" => "<",
        "greater" => ">",
        "quoteleft" => "\u{2018}",
        "quoteright" => "\u{2019}",
        "quotedblleft" => "\u{201C}",
        "quotedblright" => "\u{201D}",
        "endash" => "\u{2013}",
        "emdash" => "\u{2014}",
        "bullet" => "\u{2022}",
        "ellipsis" => "\u{2026}",
        "periodcentered" => "\u{00B7}",
        "dagger" => "\u{2020}",
        "daggerdbl" => "\u{2021}",
        "section" => "\u{00A7}",
        "paragraph" => "\u{00B6}",
        "degree" => "\u{00B0}",
        "trademark" => "\u{2122}",
        "copyright" => "\u{00A9}",
        "registered" => "\u{00AE}",
        "currency" => "\u{00A4}",
        "cent" => "\u{00A2}",
        "sterling" => "\u{00A3}",
        "euro" => "\u{20AC}",
        "yen" => "\u{00A5}",
        "guilsinglleft" => "\u{2039}",
        "guilsinglright" => "\u{203A}",
        "guillemotleft" => "\u{00AB}",
        "guillemotright" => "\u{00BB}",
        "fi" => "fi",
        "fl" => "fl",
        "ff" => "ff",
        "ffi" => "ffi",
        "ffl" => "ffl",
        "eacute" => "é",
        "egrave" => "è",
        "ecircumflex" => "ê",
        "edieresis" => "ë",
        "agrave" => "à",
        "aacute" => "á",
        "acircumflex" => "â",
        "adieresis" => "ä",
        "atilde" => "ã",
        "aring" => "å",
        "ae" => "æ",
        "ccedilla" => "ç",
        "iacute" => "í",
        "igrave" => "ì",
        "icircumflex" => "î",
        "idieresis" => "ï",
        "ntilde" => "ñ",
        "oacute" => "ó",
        "ograve" => "ò",
        "ocircumflex" => "ô",
        "odieresis" => "ö",
        "otilde" => "õ",
        "oslash" => "ø",
        "uacute" => "ú",
        "ugrave" => "ù",
        "ucircumflex" => "û",
        "udieresis" => "ü",
        "yacute" => "ý",
        "ydieresis" => "ÿ",
        "germandbls" => "ß",
        "Eacute" => "É",
        "Egrave" => "È",
        "Agrave" => "À",
        "Aacute" => "Á",
        "Adieresis" => "Ä",
        "Ccedilla" => "Ç",
        "Ntilde" => "Ñ",
        "Oacute" => "Ó",
        "Odieresis" => "Ö",
        "Uacute" => "Ú",
        "Udieresis" => "Ü",
        _ => return None,
    };
    Some(s.to_string())
}

fn be_code(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |a, &x| (a << 8) | x as u32)
}
fn utf16be(b: &[u8]) -> String {
    let mut u = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        u.push(((b[i] as u16) << 8) | b[i + 1] as u16);
        i += 2;
    }
    String::from_utf16_lossy(&u)
}

/// Parse a /ToUnicode CMap: `beginbfchar`/`bfrange` blocks → the code→unicode Table. The whole CMap
/// is the Map face built once; the text decode is then `get` per code.
fn parse_cmap(bytes: &[u8]) -> CMap {
    let mut map: Table<u32, String> = Table::new();
    let mut width = 1usize;
    let mut lex = Lex::new(bytes);
    let mut toks = Vec::new();
    while let Some(t) = lex.next() {
        toks.push(t);
    }
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Kw(k) = &toks[i] {
            match k.as_str() {
                "begincodespacerange" => {
                    if let Some(Tok::Str(lo)) = toks.get(i + 1) {
                        width = lo.len().max(1);
                    }
                }
                "beginbfchar" => {
                    let mut j = i + 1;
                    while j + 1 < toks.len() {
                        if matches!(&toks[j], Tok::Kw(e) if e == "endbfchar") {
                            break;
                        }
                        if let (Tok::Str(src), Tok::Str(dst)) = (&toks[j], &toks[j + 1]) {
                            map.put(be_code(src), utf16be(dst));
                            j += 2;
                        } else {
                            j += 1;
                        }
                    }
                }
                "beginbfrange" => {
                    let mut j = i + 1;
                    while j + 2 < toks.len() {
                        if matches!(&toks[j], Tok::Kw(e) if e == "endbfrange") {
                            break;
                        }
                        if let (Tok::Str(lo), Tok::Str(hi), Tok::Str(dst)) =
                            (&toks[j], &toks[j + 1], &toks[j + 2])
                        {
                            let (lo_c, hi_c, base) = (be_code(lo), be_code(hi), be_code(dst));
                            for c in lo_c..=hi_c.min(lo_c.saturating_add(65535)) {
                                if let Some(ch) = char::from_u32(base + (c - lo_c)) {
                                    map.put(c, ch.to_string());
                                }
                            }
                            j += 3;
                        } else {
                            j += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    CMap {
        map,
        enc: Table::new(),
        width,
        adv: Table::new(),
        dw: 500,
        widths: false,
        bold: false,
    }
}

impl Document {
    /// Resolve a ref-or-dict to its dictionary (one O(1) hop).
    fn as_dict<'a>(&'a self, o: Option<&'a Object>) -> Option<&'a Dict> {
        match o.map(|x| self.resolve(x)) {
            Some(Some(Object::Dict(d))) => Some(d),
            _ => None,
        }
    }
    /// Build the page's fonts: name → CMap (the two Map faces — `/ToUnicode` + `/Encoding /Differences`).
    /// Built for EVERY font, not just ones with ToUnicode (the subset headers live in `/Encoding`).
    /// The fonts of a page's resources. A `/ToUnicode` CMap is decoded through the walk's cache, so a
    /// CMap shared by many pages is inflated once.
    pub(crate) fn page_fonts(&self, page: &Dict, cache: &mut ContentCache) -> Table<String, CMap> {
        let mut fonts = Table::new();
        if let Some(res) = self.as_dict(page.get(&"Resources".to_string())) {
            if let Some(fd) = self.as_dict(res.get(&"Font".to_string())) {
                for (name, fref) in fd.iter() {
                    if let Some(font) = self.as_dict(Some(fref)) {
                        // /ToUnicode (if present) is the first Map face
                        let mut cm = match font.get(&"ToUnicode".to_string()) {
                            Some(Object::Ref(tn, tg)) => {
                                cache.stream(self, *tn, *tg).map(|b| parse_cmap(&b))
                            }
                            _ => None,
                        }
                        .unwrap_or(CMap {
                            map: Table::new(),
                            enc: Table::new(),
                            width: 1,
                            adv: Table::new(),
                            dw: 500,
                            widths: false,
                            bold: false,
                        });
                        // simple fonts are 1-byte; only Type0 (CID) is multi-byte
                        let type0 = matches!(font.get(&"Subtype".to_string()), Some(Object::Name(s)) if s == "Type0");
                        if !type0 {
                            cm.width = 1;
                        }
                        // /Encoding /Differences is the second Map face (code → glyph name → unicode)
                        cm.enc = self.encoding_map(font);
                        // /Widths (+ /FirstChar) → the advance Map face (code → width), for the edge read
                        self.load_widths(font, &mut cm);
                        cm.bold = self.font_is_bold(font); // BaseFont name / descriptor — the heading signal
                        fonts.put(name.clone(), cm);
                    }
                }
            }
        }
        fonts
    }

    /// The font's `/Encoding /Differences`: code → glyph name → unicode (the second Map face).
    fn encoding_map(&self, font: &Dict) -> Table<u32, String> {
        let mut m = Table::new();
        if let Some(ed) = self.as_dict(font.get(&"Encoding".to_string())) {
            if let Some(Object::Array(diffs)) = ed.get(&"Differences".to_string()) {
                let mut code = 0u32;
                for it in diffs {
                    match it {
                        Object::Int(n) => code = *n as u32,
                        Object::Name(nm) => {
                            if let Some(u) = glyph_to_unicode(nm) {
                                m.put(code, u);
                            }
                            code += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        m
    }

    /// The CIDFont of a Type0 font: the first entry of its `/DescendantFonts`.
    fn descendant_font<'a>(&'a self, font: &'a Dict) -> Option<&'a Dict> {
        match self.resolve(font.get(&"DescendantFonts".to_string())?)? {
            Object::Array(a) => match self.resolve(a.first()?)? {
                Object::Dict(d) => Some(d),
                _ => None,
            },
            _ => None,
        }
    }

    /// The CIDFont widths of a Type0 font (§9.7.4.3): `/DW` (default 1000) and `/W`, whose entries are
    /// `c [w1 w2 …]` or `cfirst clast w`. Each CID's advance goes into the advance Map face, keyed
    /// by the code (Identity-H codes are CIDs). A range is bounded, so a hostile `/W` cannot size
    /// the table.
    fn cid_widths(&self, font: &Dict, cm: &mut CMap) {
        let Some(desc) = self.descendant_font(font) else {
            return;
        };
        let num = |o: &Object| match self.resolve(o) {
            Some(Object::Int(n)) => Some(*n),
            Some(Object::Real(r)) => Some(*r as i64),
            _ => None,
        };
        cm.dw = desc.get(&"DW".to_string()).and_then(num).unwrap_or(1000);
        let Some(Object::Array(w)) = desc.get(&"W".to_string()).and_then(|o| self.resolve(o))
        else {
            return;
        };
        cm.widths = true;
        let mut budget = 1usize << 20;
        let mut i = 0usize;
        while i < w.len() {
            let Some(c0) = num(&w[i]) else { break };
            match w.get(i + 1).and_then(|o| self.resolve(o)) {
                Some(Object::Array(list)) => {
                    for (k, o) in list.iter().enumerate() {
                        if let (Some(cid), Some(v)) = (u32::try_from(c0 + k as i64).ok(), num(o)) {
                            if budget > 0 {
                                budget -= 1;
                                cm.adv.put(cid, v);
                            }
                        }
                    }
                    i += 2;
                }
                _ => {
                    let (Some(c1), Some(v)) =
                        (w.get(i + 1).and_then(num), w.get(i + 2).and_then(num))
                    else {
                        break;
                    };
                    let mut cid = c0;
                    while cid <= c1 && budget > 0 {
                        if let Ok(c) = u32::try_from(cid) {
                            cm.adv.put(c, v);
                        }
                        budget -= 1;
                        cid += 1;
                    }
                    i += 3;
                }
            }
        }
    }

    /// Load the font's `/Widths` (indexed from `/FirstChar`) into the advance Map face (code → width).
    /// `/MissingWidth` from the descriptor (or a half-em) is the default. The one-time prepare; the
    /// per-run advance is then `get` per code — so a run's end-x is known and the gap is measurable.
    /// A Type0 font takes its widths from its CIDFont instead (`cid_widths`).
    fn load_widths(&self, font: &Dict, cm: &mut CMap) {
        if matches!(font.get(&"Subtype".to_string()), Some(Object::Name(s)) if s == "Type0") {
            self.cid_widths(font, cm);
            return;
        }
        let first = match font.get(&"FirstChar".to_string()).map(|o| self.resolve(o)) {
            Some(Some(Object::Int(n))) => *n as u32,
            _ => 0,
        };
        // A Type 3 font's widths are in glyph space: its `/FontMatrix` gives the scale to 1/1000 em.
        let scale = if matches!(font.get(&"Subtype".to_string()), Some(Object::Name(s)) if s == "Type3")
        {
            match font
                .get(&"FontMatrix".to_string())
                .and_then(|o| self.resolve(o))
            {
                Some(Object::Array(m)) => match m.first().and_then(|o| self.resolve(o)) {
                    Some(Object::Int(a)) if *a != 0 => *a as f64 * 1000.0,
                    Some(Object::Real(a)) if *a != 0.0 => *a * 1000.0,
                    _ => 1.0,
                },
                _ => 1.0,
            }
        } else {
            1.0
        };
        if let Some(Object::Array(ws)) = font
            .get(&"Widths".to_string())
            .and_then(|o| self.resolve(o))
        {
            for (i, w) in ws.iter().enumerate() {
                let v = match self.resolve(w) {
                    Some(Object::Int(n)) => (*n as f64 * scale).round() as i64,
                    Some(Object::Real(r)) => (*r * scale).round() as i64,
                    _ => continue,
                };
                if v != 0 {
                    cm.adv.put(first + i as u32, v);
                }
            }
            cm.widths = true;
        }
        // default width: descriptor /MissingWidth, else keep the half-em fallback
        if let Some(fdesc) = self.as_dict(font.get(&"FontDescriptor".to_string())) {
            match fdesc.get(&"MissingWidth".to_string()) {
                Some(Object::Int(n)) => cm.dw = (*n as f64 * scale).round() as i64,
                Some(Object::Real(r)) => cm.dw = (*r * scale).round() as i64,
                _ => {}
            }
        }
    }

    /// Is this font bold? — the BaseFont subset name (`...-Bold`/`Black`/`Heavy`/`Semibold`), or the
    /// descriptor's ForceBold flag (bit 18) / `/FontWeight ≥ 600` / high `/StemV`. For Type0 the
    /// descriptor lives in the descendant font. A heading signal where size alone doesn't separate.
    fn font_is_bold(&self, font: &Dict) -> bool {
        if let Some(Object::Name(bf)) = font.get(&"BaseFont".to_string()) {
            let b = bf.to_ascii_lowercase();
            if [
                "bold", "black", "heavy", "semibold", "demibold", "-bd", "extrab",
            ]
            .iter()
            .any(|k| b.contains(k))
            {
                return true;
            }
        }
        // descend to the descriptor (Type0 keeps it on the descendant font)
        let desc = self
            .as_dict(font.get(&"FontDescriptor".to_string()))
            .or_else(|| {
                match font
                    .get(&"DescendantFonts".to_string())
                    .and_then(|o| self.resolve(o))
                {
                    Some(Object::Array(a)) => self
                        .as_dict(a.first())
                        .and_then(|df| self.as_dict(df.get(&"FontDescriptor".to_string()))),
                    _ => None,
                }
            });
        if let Some(fd) = desc {
            if let Some(Object::Int(flags)) = fd.get(&"Flags".to_string()) {
                if flags & (1 << 18) != 0 {
                    return true;
                }
            }
            match fd.get(&"FontWeight".to_string()) {
                Some(Object::Int(w)) if *w >= 600 => return true,
                _ => {}
            }
            match fd.get(&"StemV".to_string()) {
                Some(Object::Int(v)) if *v >= 140 => return true,
                Some(Object::Real(v)) if *v >= 140.0 => return true,
                _ => {}
            }
        }
        false
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    fn form(content: &str, matrix: [f64; 6]) -> FormText {
        FormText {
            content: std::rc::Rc::new(content.as_bytes().to_vec()),
            own: None,
            matrix,
        }
    }

    /// A form painted at two places lands at two places: its lines are mapped by the CTM at
    /// each `Do` (and by its own `/Matrix`), not left in form space.
    #[test]
    fn form_lines_follow_the_invocation_transform() {
        let mut forms = Table::new();
        forms.put(
            "Fm".to_string(),
            form(
                "BT /F1 12 Tf 10 20 Td (Stamp) Tj ET",
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            ),
        );
        let page = b"q 1 0 0 1 0 100 cm /Fm Do Q q 1 0 0 1 50 400 cm /Fm Do Q";
        let lines = extract_text(page, &Table::new(), &Table::new(), &forms);
        let ys: Vec<f64> = lines.iter().map(|l| l.y).collect();
        assert_eq!(ys, vec![120.0, 420.0], "lines: {:?}", ys);
        assert_eq!(lines[1].segs[0].0, 60.0);

        // The form's own /Matrix composes with the CTM.
        let mut scaled = Table::new();
        scaled.put(
            "Fm".to_string(),
            form(
                "BT /F1 12 Tf 10 20 Td (Stamp) Tj ET",
                [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
            ),
        );
        let lines = extract_text(b"/Fm Do", &Table::new(), &Table::new(), &scaled);
        assert_eq!(lines[0].y, 40.0);
    }

    /// A hyphenated line before a detected table is not joined to the table's first row.
    #[test]
    fn dehyphenation_does_not_reach_into_a_table() {
        let line = |text: &str, y: f64| Line {
            text: text.to_string(),
            size: 10.0,
            bold: false,
            y,
            segs: vec![(0.0, 50.0, 0, false)],
        };
        let lines = vec![line("intro-", 300.0), line("alpha beta", 200.0)];
        let cands = vec![Cand {
            ytop: 205.0,
            ybot: 195.0,
            rowys: vec![205.0, 195.0],
            colxs: vec![0.0, 25.0, 50.0],
            feat: 0,
            geo_ok: true,
        }];
        let mut out = String::new();
        emit_lines(&mut out, &lines, &cands, 10.0);
        assert!(out.starts_with("intro-\n"), "markdown: {out:?}");

        // Ordinary text after it still joins the word.
        let mut out = String::new();
        emit_lines(&mut out, &lines, &[], 10.0);
        assert!(out.starts_with("introalpha"), "markdown: {out:?}");
    }
}
