#[derive(Debug, Clone)]
pub struct Block {
    pub md: String,
    pub sig: u64,
    pub links: Vec<String>,
}

#[inline]
fn fnv1a_bytes(h0: u64, bytes: &[u8]) -> u64 {
    let mut h = h0;
    for &c in bytes {
        h ^= c as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// HashTrinity's one operation in Rust: update one key by merging the new value with the old.
/// The merge `f` is the only freedom (`HashTrinityCore.merge_is_the_only_freedom`).
/// Four standard merges = four data-structure faces:
///   NB (Set)     = `\|_, old\| old \|\| new`          → presence (template fold)
///   B  (Map)     = `\|new, _old\| new`               → ChunkSeed push
///   B/U (Accum)  = `\|new, old\| new + old`           → Template.freq counting
///   4th (Graph)  = `\|new, old\| [new, old].concat()` → links adjacency
#[inline]
pub fn insert_with<K: hashtrinity::Coord + Eq, V>(
    m: &mut hashtrinity::Table<K, V>,
    k: K,
    v: V,
    f: impl FnOnce(V, V) -> V,
) {
    m.insert_with(k, v, f);
}

/// Chrome subtrees dropped wholesale — never content.
/// HashTrinity face: **NB (idempotent Set merge)** — once in the drop set, never content.
/// These are the static chrome cone: script/style/nav/header/footer/aside/form/iframe.
/// The NB face collapses to the cone; every page pays it once, never N times.
/// Chrome tags, matched case-insensitively. A match opens a subtree that is dropped wholesale.
const DROP_TAGS: &[&[u8]] = &[
    b"script",
    b"style",
    b"noscript",
    b"svg",
    b"head",
    b"nav",
    b"header",
    b"footer",
    b"aside",
    b"form",
    b"button",
    b"iframe",
    b"template",
    b"dialog",
    b"select",
    b"option",
    b"figure",
    b"ix:header", // inline XBRL: spec-invisible metadata container
    b"ix:hidden", // inline XBRL: hidden contexts (SEC EDGAR wraps these in display:none divs)
    b"ix:references",
];

/// The canonical drop tag for `tag`, if it opens a chrome subtree.
#[inline]
fn drop_tag(tag: &[u8]) -> Option<&'static [u8]> {
    DROP_TAGS.iter().copied().find(|t| eqi(tag, t))
}

/// True if the raw attribute region marks the element browser-invisible:
/// `style="display:none"`, `style="visibility:hidden"`, or the HTML5 `hidden` attr.
/// Case-insensitive, whitespace-tolerant between tokens.
fn is_style_hidden(attrs: &[u8]) -> bool {
    let a = attrs;
    let n = a.len();
    let has_prop = |prop: &[u8], val: &[u8]| -> bool {
        let mut i = 0;
        while let Some(p) = find_ci(a, i, prop) {
            let mut j = p + prop.len();
            while j < n && (a[j] == b' ' || a[j] == b'\t' || a[j] == b'\n' || a[j] == b'\r') {
                j += 1;
            }
            if j < n && a[j] == b':' {
                j += 1;
                while j < n && (a[j] == b' ' || a[j] == b'\t' || a[j] == b'\n' || a[j] == b'\r') {
                    j += 1;
                }
                let end = j + val.len();
                if end <= n && eqi(&a[j..end], val) {
                    // must be token-end (not "noneX")
                    if end == n || !a[end].is_ascii_alphanumeric() {
                        return true;
                    }
                }
            }
            i = p + prop.len();
        }
        false
    };
    if has_prop(b"display", b"none") || has_prop(b"visibility", b"hidden") {
        return true;
    }
    // HTML5 boolean `hidden` attribute: a token OUTSIDE quoted attribute values.
    // `overflow:hidden` / `overflow: hidden` inside a style string is CSS, not the
    // attribute — so scan quote-aware: only a `hidden` preceded by attr-boundary
    // whitespace (or region start) and followed by whitespace/`=`/end counts.
    // `aria-hidden` is excluded (element may still be visible).
    let mut i = 0;
    while i < n {
        let c = a[i];
        if c == b'"' || c == b'\'' {
            // skip the quoted attribute value entirely
            let q = c;
            i += 1;
            while i < n && a[i] != q {
                i += 1;
            }
            i += 1; // past closing quote (or past end)
            continue;
        }
        if (c == b'h' || c == b'H') && i + 6 <= n && eqi(&a[i..i + 6], b"hidden") {
            let before_ok = i == 0 || (a[i - 1] as char).is_ascii_whitespace();
            let after = i + 6;
            let after_ok =
                after >= n || a[after] == b'=' || (a[after] as char).is_ascii_whitespace();
            let not_aria = i < 5 || !eqi(&a[i - 5..i], b"aria-");
            if before_ok && after_ok && not_aria {
                return true;
            }
            i += 6;
            continue;
        }
        i += 1;
    }
    false
}

#[inline]
fn heading_level(tag: &[u8]) -> Option<u8> {
    if tag.len() == 2 && tag[0] == b'h' && (b'1'..=b'6').contains(&tag[1]) {
        Some(tag[1] - b'0')
    } else {
        None
    }
}

/// State carried through the one pass.
struct Walk<'a> {
    blocks: Vec<Block>,
    cur: String, // current inline buffer (the only per-block alloc beyond output)
    cur_links: Vec<String>,
    prefix: String, // pending block prefix (#, -, > …)
    // Open chrome subtrees, innermost last. A close tag only pops back to its own matching
    // open, so a stray `</header>` inside `<nav>` cannot end the drop and leak nav/script text.
    drop_stack: Vec<&'static [u8]>,
    pre_depth: i32, // inside <pre> (verbatim)
    list_depth: usize,
    ordered: Vec<bool>,
    emph: Vec<u8>, // emphasis stack: 1 = *, 2 = **
    href_stack: Vec<String>,
    sp: bool,                // last char was space (inline whitespace collapse)
    hidden: Option<i32>,     // tag-nesting depth inside a style-hidden (display:none) container
    revealed: Vec<&'a [u8]>, // RSC streaming: ids revealed by $RS/$RC/$RM scripts
}

impl Walk<'_> {
    /// `hidden` mode entry check with the RSC exception: a hidden element whose
    /// id is referenced by a reveal script is content, not chrome.
    fn hidden_but_revealed(&self, attrs: &[u8]) -> bool {
        if self.revealed.is_empty() {
            return false;
        }
        let id = attr_val(attrs, b"id");
        !id.is_empty() && self.revealed.contains(&id.as_bytes())
    }
}

/// HTML void elements — never have a close tag, so must not open hidden tracking.
fn is_void_tag(tag: &[u8]) -> bool {
    matches!(
        tag,
        b"img"
            | b"br"
            | b"hr"
            | b"input"
            | b"meta"
            | b"link"
            | b"source"
            | b"wbr"
            | b"col"
            | b"area"
            | b"base"
            | b"embed"
            | b"track"
            | b"param"
    )
}

impl Walk<'_> {
    fn flush(&mut self) {
        let body = self.cur.trim();
        if !body.is_empty() {
            let md = if self.prefix.is_empty() {
                body.to_string()
            } else {
                format!("{}{body}", self.prefix)
            };
            let sig = norm_sig(body);
            self.blocks.push(Block {
                md,
                sig,
                links: std::mem::take(&mut self.cur_links),
            });
        }
        self.cur.clear();
        self.cur_links.clear();
        self.sp = true;
    }
    #[inline]
    fn push_text(&mut self, s: &str, verbatim: bool) {
        if verbatim {
            self.cur.push_str(s);
            return;
        }
        for c in s.chars() {
            if c.is_whitespace() {
                if !self.sp {
                    self.cur.push(' ');
                    self.sp = true;
                }
            } else {
                // Text like `W</sup> TRIMS` in an alt attr or `&lt;del&gt;` source
                // content must not become live markup downstream: backslash-escape
                // `<` (CommonMark) so it stays literal text, not HTML.
                if c == '<' {
                    self.cur.push('\\');
                    self.cur.push('<');
                    self.sp = false;
                    continue;
                }
                self.cur.push(c);
                self.sp = false;
            }
        }
    }
}

/// **The one pass.** Walk the bytes; tags are byte-slices (no allocation); emit markdown blocks.
pub fn to_blocks(html: &str, base: &str) -> Vec<Block> {
    let b = html.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut w = Walk {
        blocks: Vec::new(),
        cur: String::new(),
        cur_links: Vec::new(),
        prefix: String::new(),
        drop_stack: Vec::new(),
        pre_depth: 0,
        list_depth: 0,
        ordered: Vec::new(),
        emph: Vec::new(),
        href_stack: Vec::new(),
        sp: true,
        hidden: None,
        revealed: revealed_ids(b),
    };

    while i < n {
        if b[i] == b'<' {
            // comment / doctype / pi
            if i + 3 < n && &b[i + 1..i + 4] == b"!--" {
                i = find(b, i + 4, b"-->").map_or(n, |e| e + 3);
                continue;
            }
            if i + 1 < n && (b[i + 1] == b'!' || b[i + 1] == b'?') {
                i = memchr(b, i, b'>').map_or(n, |e| e + 1);
                continue;
            }
            let close = i + 1 < n && b[i + 1] == b'/';
            let start = i + if close { 2 } else { 1 };
            // tag name
            let mut j = start;
            while j < n && !matches!(b[j], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
                j += 1;
            }
            let tag = &b[start..j.min(n)];
            // find the end of the tag — QUOTE-AWARE: `>` legally appears inside
            // quoted attribute values (e.g. data-layer JSON containing `&lt;p>`),
            // so skip over "…" and '…' regions while hunting for the real `>`
            let mut k = j;
            while k < n {
                let c = b[k];
                if c == b'>' {
                    break;
                }
                if c == b'"' || c == b'\'' {
                    let q = c;
                    k += 1;
                    while k < n && b[k] != q {
                        k += 1;
                    }
                }
                k += 1;
            }
            if k >= n {
                break;
            }
            let attr_region = &b[j..k]; // raw attrs (sliced, not parsed unless needed)
            let self_close = k > start && b[k - 1] == b'/';
            i = k + 1;

            if close {
                handle_close(&mut w, tag);
            } else {
                // raw-text: skip script/style content verbatim
                if (eqi(tag, b"script") || eqi(tag, b"style")) && !self_close {
                    let end = if eqi(tag, b"script") {
                        b"</script" as &[u8]
                    } else {
                        b"</style"
                    };
                    i = find_ci(b, i, end).map_or(n, |e| e);
                    continue;
                }
                handle_open(&mut w, tag, attr_region, base, self_close);
            }
        } else {
            if !w.drop_stack.is_empty() || w.hidden.is_some() {
                while i < n && b[i] != b'<' {
                    i += 1;
                }
                // hidden or drop subtree: text is skipped, scanning continues
                continue;
            }
            let s = i;
            while i < n && b[i] != b'<' {
                i += 1;
            }
            let raw = &html[s..i];
            let decoded = decode_entities(raw);
            w.push_text(&decoded, w.pre_depth > 0);
        }
    }
    w.flush();
    w.blocks
}

fn handle_open(w: &mut Walk, tag: &[u8], attrs: &[u8], base: &str, self_close: bool) {
    // Active hidden mode owns ALL bookkeeping: every nested open just counts depth.
    // This must precede drop checks or drop-tag opens/closes desync the counters.
    if let Some(d) = w.hidden.as_mut() {
        if !self_close && !is_void_tag(tag) {
            *d += 1;
        }
        return;
    }
    if let Some(t) = drop_tag(tag) {
        if !self_close {
            w.drop_stack.push(t);
        }
        return;
    }
    if !w.drop_stack.is_empty() {
        return;
    }
    // style-hidden container tracking (display:none / visibility:hidden / hidden):
    // enter hidden mode with a tag-depth counter; every non-void open increments, every
    // close decrements; zero exits hidden mode. Void tags never close, so they never count.
    if let Some(d) = w.hidden.as_mut() {
        if !self_close && !is_void_tag(tag) {
            *d += 1;
        }
        return;
    }
    if is_style_hidden(attrs) && !w.hidden_but_revealed(attrs) {
        if !self_close && !is_void_tag(tag) {
            w.hidden = Some(1);
        }
        return;
    }
    if let Some(lvl) = heading_level(tag) {
        w.flush();
        w.prefix = format!("{} ", "#".repeat(lvl as usize));
        return;
    }
    match tag {
        b"p" | b"div" | b"section" | b"article" | b"main" | b"dd" | b"dt" | b"figcaption" => {
            w.flush();
            w.prefix.clear();
        }
        b"blockquote" => {
            w.flush();
            w.prefix = "> ".into();
        }
        b"ul" => {
            w.ordered.push(false);
            w.list_depth += 1;
        }
        b"ol" => {
            w.ordered.push(true);
            w.list_depth += 1;
        }
        b"li" => {
            w.flush();
            let ind = "  ".repeat(w.list_depth.saturating_sub(1));
            let mark = if *w.ordered.last().unwrap_or(&false) {
                "1. "
            } else {
                "- "
            };
            w.prefix = format!("{ind}{mark}");
        }
        b"pre" => {
            w.flush();
            w.pre_depth += 1;
            w.cur.push_str("```\n");
        }
        b"code" if w.pre_depth == 0 => w.cur.push('`'),
        b"a" => {
            let href = attr_val(attrs, b"href");
            w.cur.push('[');
            w.href_stack.push(resolve(base, &href));
            w.sp = false;
        }
        b"img" => {
            let src = attr_val(attrs, b"src");
            // data: URIs are inline decorative chrome (often huge escaped SVG
            // placeholders) — browsers never surface them as text; keep them
            // out of the markdown body entirely.
            if !src.is_empty() && !src.starts_with("data:") {
                let alt = attr_val(attrs, b"alt").replace('<', "\\<");
                w.cur
                    .push_str(&format!("![{alt}]({})", resolve(base, &src)));
                w.sp = false;
            }
        }
        b"strong" | b"b" => {
            w.cur.push_str("**");
            w.emph.push(2);
            w.sp = false;
        }
        b"em" | b"i" => {
            w.cur.push('*');
            w.emph.push(1);
            w.sp = false;
        }
        b"br" => {
            w.cur.push_str("  \n");
            w.sp = true;
        }
        b"hr" => {
            w.flush();
            w.cur.push_str("---");
            w.flush();
        }
        b"tr" => {
            w.flush();
            w.prefix.clear();
            w.cur.push_str("| ");
            w.sp = true;
        }
        b"td" | b"th" => {
            w.cur.push_str(" | ");
            w.sp = true;
        }
        _ => {}
    }
}

fn handle_close(w: &mut Walk, tag: &[u8]) {
    // style-hidden container exit: decrement depth, exit at zero. This runs BEFORE
    // the drop stack so a hidden div nested inside a dropped subtree still closes correctly.
    if let Some(d) = w.hidden.as_mut() {
        if !is_void_tag(tag) {
            *d -= 1;
            if *d <= 0 {
                w.hidden = None;
            }
        }
        return;
    }
    if !w.drop_stack.is_empty() {
        if let Some(t) = drop_tag(tag) {
            if let Some(pos) = w.drop_stack.iter().rposition(|&open| open == t) {
                w.drop_stack.truncate(pos);
            }
        }
        return;
    }
    if heading_level(tag).is_some() {
        w.flush();
        w.prefix.clear();
        return;
    }
    match tag {
        b"p" | b"div" | b"section" | b"article" | b"main" | b"blockquote" | b"li" | b"dd"
        | b"dt" | b"figcaption" | b"tr" => {
            w.flush();
            w.prefix.clear();
        }
        b"ul" | b"ol" => {
            w.ordered.pop();
            w.list_depth = w.list_depth.saturating_sub(1);
        }
        b"pre" => {
            w.cur.push_str("\n```");
            w.pre_depth = (w.pre_depth - 1).max(0);
            w.flush();
        }
        b"code" if w.pre_depth == 0 => w.cur.push('`'),
        b"a" => {
            let href = w.href_stack.pop().unwrap_or_default();
            w.cur.push_str(&format!("]({href})"));
            if !href.is_empty() {
                w.cur_links.push(href);
            }
            w.sp = false;
        }
        b"strong" | b"b" | b"em" | b"i" => {
            if let Some(m) = w.emph.pop() {
                w.cur.push_str(if m == 2 { "**" } else { "*" });
            }
            w.sp = false;
        }
        _ => {}
    }
}

// ── byte helpers (no allocation) ──
#[inline]
fn eqi(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}
#[inline]
fn memchr(b: &[u8], from: usize, c: u8) -> Option<usize> {
    b[from..].iter().position(|&x| x == c).map(|p| p + from)
}
#[inline]
fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    b[from..]
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|p| p + from)
}
fn find_ci(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    b[from..]
        .windows(pat.len())
        .position(|w| w.eq_ignore_ascii_case(pat))
        .map(|p| p + from)
}

/// React Server Components streaming: content ships hidden in
/// `<div hidden id="S:n">…</div>` and a later inline script reveals it by id:
/// `$RS("S:n","P:n")` (move into slot) / `$RC(…)` (resume) / `$RM(…)`.
/// Such hidden-but-revealed subtrees ARE the page content — collect their ids
/// so `handle_open` can keep them. Byte-scan pre-pass; no DOM, no lookahead.
fn revealed_ids(b: &[u8]) -> Vec<&[u8]> {
    let mut ids = Vec::new();
    let mut i = 0;
    let n = b.len();
    while let Some(p) = find(b, i, b"$R") {
        // $R + 1..3 uppercase letters + ( + quote + id + quote
        let mut j = p + 2;
        let mut alpha = 0;
        while j < n && b[j].is_ascii_uppercase() && alpha < 3 {
            j += 1;
            alpha += 1;
        }
        if alpha == 0 || j >= n || b[j] != b'(' {
            i = p + 2;
            continue;
        }
        j += 1;
        if j < n && (b[j] == b'"' || b[j] == b'\'') {
            let q = b[j];
            j += 1;
            let s = j;
            while j < n && b[j] != q {
                j += 1;
            }
            if j > s {
                ids.push(&b[s..j]);
            }
        }
        i = p + 2;
    }
    ids
}

/// Extract one attribute value from the raw attr region (only for `a`/`img` — lazy, no full parse).
fn attr_val(attrs: &[u8], key: &[u8]) -> String {
    let n = attrs.len();
    let mut i = 0;
    while i < n {
        while i < n && attrs[i].is_ascii_whitespace() {
            i += 1;
        }
        let ks = i;
        while i < n && attrs[i] != b'=' && !attrs[i].is_ascii_whitespace() && attrs[i] != b'/' {
            i += 1;
        }
        let k = &attrs[ks..i];
        while i < n && attrs[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < n && attrs[i] == b'=' {
            i += 1;
            while i < n && attrs[i].is_ascii_whitespace() {
                i += 1;
            }
            let (vs, ve);
            if i < n && (attrs[i] == b'"' || attrs[i] == b'\'') {
                let q = attrs[i];
                i += 1;
                vs = i;
                while i < n && attrs[i] != q {
                    i += 1;
                }
                ve = i;
                i += 1;
            } else {
                vs = i;
                while i < n && !attrs[i].is_ascii_whitespace() {
                    i += 1;
                }
                ve = i;
            }
            if eqi(k, key) {
                return decode_entities(&String::from_utf8_lossy(&attrs[vs..ve]));
            }
        } else if i == ks {
            i += 1;
        }
    }
    String::new()
}

/// Signature: collapse whitespace + lowercase, hash. Identical chrome across pages → same sig.
fn norm_sig(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut sp = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !sp {
                h = fnv1a_bytes(h, b" ");
                sp = true;
            }
        } else {
            let mut buf = [0u8; 4];
            for lc in c.to_lowercase() {
                h = fnv1a_bytes(h, lc.encode_utf8(&mut buf).as_bytes());
            }
            sp = false;
        }
    }
    h
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        if let Some(e) = rest.find(';').filter(|&e| e <= 10) {
            let ent = &rest[1..e];
            let rep = match ent {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" | "#39" => "'",
                "nbsp" | "#160" => " ",
                "mdash" | "#8212" => "—",
                "ndash" | "#8211" => "–",
                "hellip" | "#8230" => "…",
                "rsquo" | "#8217" => "'",
                "lsquo" | "#8216" => "'",
                "ldquo" | "#8220" => "\u{201c}",
                "rdquo" | "#8221" => "\u{201d}",
                _ if ent.starts_with('#') => {
                    let code = if let Some(h) =
                        ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X"))
                    {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        ent[1..].parse::<u32>().ok()
                    };
                    if let Some(c) = code.and_then(char::from_u32) {
                        out.push(c);
                        rest = &rest[e + 1..];
                        continue;
                    }
                    out.push('&');
                    rest = &rest[1..];
                    continue;
                }
                _ => {
                    // full WHATWG named table (2125 names) — eacute, hellip, ligatures …
                    if let Some(rep) = super::entities::lookup_named_entity(ent) {
                        rep
                    } else {
                        out.push('&');
                        rest = &rest[1..];
                        continue;
                    }
                }
            };
            out.push_str(rep);
            rest = &rest[e + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Resolve a possibly-relative URL against a base.
pub fn resolve(base: &str, href: &str) -> String {
    if href.is_empty() || href.starts_with("javascript:") || href.starts_with('#') {
        return href.to_string();
    }
    if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("//") {
        return href.to_string();
    }
    let scheme_end = base.find("://").map(|p| p + 3).unwrap_or(0);
    let host_end = base[scheme_end..]
        .find('/')
        .map(|p| scheme_end + p)
        .unwrap_or(base.len());
    let origin = &base[..host_end];
    if let Some(rest) = href.strip_prefix('/') {
        format!("{origin}/{rest}")
    } else {
        let dir_end = base
            .rfind('/')
            .filter(|&p| p > host_end)
            .unwrap_or(base.len());
        format!("{}/{href}", &base[..dir_end])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §2. THE TEMPLATE — the chrome cone, certified ONCE (the streaming-law crawl seed)
// ─────────────────────────────────────────────────────────────────────────────

/// The fold over a crawl: block signature → how many pages carry it.
///
/// HashTrinity face: **B/U accumulator merge** (counter). `insert_with(freq, sig, 1, |new, old| new + old)`
/// = `HashTrinityCore.merge_BU`. A signature on `>= min_pages` pages IS chrome (the template cone).
/// The monoid laws (commutative, associative, (0,0) unit) hold per `count_two` / `fusion_is_commutative_monoid`.
#[derive(Default)]
pub struct Template {
    pub freq: hashtrinity::Table<u64, u32>,
    pub pages: u32,
}

impl Template {
    pub fn new() -> Self {
        Template::default()
    }
    pub fn fold(&mut self, blocks: &[Block]) {
        self.pages += 1;
        let mut seen = hashtrinity::Table::new();
        for b in blocks {
            if !seen.has(&b.sig) {
                seen.mark(b.sig);
                self.freq.insert_with(b.sig, 1, |new, old| new + old);
            }
        }
    }
    #[inline]
    pub fn is_chrome(&self, sig: u64, min_pages: u32) -> bool {
        self.freq.get(&sig).is_some_and(|&c| c >= min_pages)
    }
}

/// Render content blocks to markdown — the scrape, or a crawl page after the template GET drops chrome.
/// HashTrinity face: **B (Map get) + NB (drop)** — template.chrome? drops NB content folds to cone.
/// The content that survives IS `x = B/U`: content/B over chrome/U. The streaming law: template
/// certified ONCE (the seed), each page O(Δ) — only the content blocks pass through.
pub fn render(
    blocks: &[Block],
    template: Option<&Template>,
    min_pages: u32,
) -> (String, usize, usize) {
    let mut out = String::new();
    let (mut kept, mut dropped) = (0, 0);
    for b in blocks {
        if let Some(t) = template {
            if t.is_chrome(b.sig, min_pages) {
                dropped += 1;
                continue;
            }
        }
        kept += 1;
        out.push_str(&b.md);
        out.push_str("\n\n");
    }
    (out.trim().to_string(), kept, dropped)
}

/// Extract all links across blocks.
/// HashTrinity face: **4th (Graph/list merge)** — `merge_loop` = `++` over the adjacency.
/// Every page's resolved links = the B/U medium over itself (`/∘/`), the reflexive fourth.
pub fn links(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks {
        out.extend(b.links.iter().cloned());
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// §3. THE FOLD-FAMILY READOUT — markdown chunks by certified seams
// ─────────────────────────────────────────────────────────────────────────────

/// One markdown chunk as byte offsets into the source markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextChunk {
    pub start: usize,
    pub end: usize,
}

impl TextChunk {
    #[inline]
    pub fn as_str<'a>(&self, text: &'a str) -> &'a str {
        text[self.start..self.end].trim()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// The certified chunk table. `chunk(i)` = one HashTrinity **B face** (Map replace) `get`.
/// `push` uses `insert_with(—, count, chunk, |new, _old| new)` = `HashTrinityCore.merge_B`.
/// Building the seed is the one certification pass; every later read is O(1) — the streaming law.
#[derive(Debug, Clone, Default)]
pub struct ChunkSeed {
    chunks: hashtrinity::Table<usize, TextChunk>,
    count: usize,
}

impl ChunkSeed {
    pub fn new() -> Self {
        ChunkSeed::default()
    }

    #[inline]
    fn push(&mut self, chunk: TextChunk) {
        self.chunks.put(self.count, chunk);
        self.count += 1;
    }

    /// Read chunk `i` in O(1): HashTrinity `get` on the chunk table.
    #[inline]
    pub fn chunk(&self, i: usize) -> Option<&TextChunk> {
        self.chunks.get(&i)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &TextChunk)> {
        (0..self.count).filter_map(|i| self.chunk(i).map(|chunk| (i, chunk)))
    }
}

/// Certify clean markdown into a HashTrinity chunk table. The one pass folds seams into keyed chunks;
/// each later chunk read is a table `get`, not a coordinate search.
pub fn chunk_markdown(markdown: &str, target_bytes: usize) -> ChunkSeed {
    let len = markdown.len();
    let mut seed = ChunkSeed::new();
    if len == 0 {
        return seed;
    }
    let target = target_bytes.max(1);
    if len <= target {
        seed.push(TextChunk { start: 0, end: len });
        return seed;
    }

    let mut start = 0usize;
    let mut last_seam: Option<usize> = None;
    let mut prev = '\0';
    for (i, c) in markdown.char_indices() {
        let end = i + c.len_utf8();
        if is_markdown_seam(c, prev) && end > start {
            last_seam = Some(end);
        }
        if end.saturating_sub(start) >= target {
            let cut = last_seam.filter(|&s| s > start).unwrap_or(end);
            seed.push(TextChunk { start, end: cut });
            start = cut;
            last_seam = if is_markdown_seam(c, prev) && end > start {
                Some(end)
            } else {
                None
            };
        }
        prev = c;
    }
    if start < len {
        seed.push(TextChunk { start, end: len });
    }
    seed
}

fn is_markdown_seam(c: char, prev: char) -> bool {
    let class = break_class(c);
    let sentence = matches!(
        c,
        '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '；' | '：'
    );
    let para = c == '\n' && prev == '\n';
    let soft = matches!(class, Break::Soft) && !matches!(c, ' ' | '\t');
    sentence || para || matches!(class, Break::Mandatory) || soft
}

/// Text boundary behavior lifted from `termfold`'s zero-dependency UAX-14 core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Break {
    None,
    Soft,
    Mandatory,
}

fn break_class(c: char) -> Break {
    match c {
        '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}' => {
            Break::Mandatory
        }
        '\u{00A0}' | '\u{202F}' | '\u{2060}' => Break::None,
        ' ' | '\t' | '\u{200B}' => Break::Soft,
        '-' | '\u{2010}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '/' => Break::Soft,
        c if is_ideograph(c) => Break::Soft,
        _ => Break::None,
    }
}

fn is_ideograph(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF |
        0x2E80..=0x2FFF |
        0x3040..=0x30FF |
        0x3400..=0x4DBF |
        0x4E00..=0x9FFF |
        0xAC00..=0xD7AF |
        0xF900..=0xFAFF |
        0x20000..=0x2FA1F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_markdown_tiles_without_losing_bytes() {
        let md = "# Title\n\nFirst sentence. Second sentence. Third sentence.\n\nTail.";
        let chunks = chunk_markdown(md, 24);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.chunk(0).unwrap().start, 0);
        assert_eq!(chunks.chunk(chunks.len() - 1).unwrap().end, md.len());
        let mut last = 0;
        for (_, chunk) in chunks.iter() {
            assert_eq!(chunk.start, last);
            last = chunk.end;
        }
    }

    #[test]
    fn chunk_markdown_keeps_utf8_boundaries() {
        let md = "Intro。第二段は長い。Tail paragraph with words.";
        let chunks = chunk_markdown(md, 10);
        for (_, c) in chunks.iter() {
            assert!(md.is_char_boundary(c.start));
            assert!(md.is_char_boundary(c.end));
            assert!(!c.as_str(md).is_empty());
        }
    }

    #[test]
    fn chunk_read_is_table_get() {
        let md = "First sentence. Second sentence. Third sentence.";
        let chunks = chunk_markdown(md, 16);
        assert_eq!(chunks.chunk(0).unwrap().as_str(md), "First sentence.");
    }

    fn md_of(html: &str) -> String {
        let blocks = to_blocks(html, "https://example.com/");
        let (md, _, _) = render(&blocks, None, 1);
        md
    }

    #[test]
    fn display_none_subtree_is_dropped() {
        // SEC EDGAR pattern: inline XBRL wrapped in <div style="display:none;">
        let html = "<html><body><h1>Report</h1>\
<div style=\"display:none;\"><ix:header><ix:hidden>ecd:AggtChngPnsnVal ecd:PeoMember</ix:hidden></ix:header></div>\
<p>Visible compensation discussion.</p></body></html>";
        let md = md_of(html);
        assert!(md.contains("Visible compensation discussion"), "md: {md}");
        assert!(!md.contains("ecd:"), "XBRL leak: {md}");
        assert!(!md.contains("ix:hidden"), "ix:hidden leak: {md}");
    }

    #[test]
    fn hidden_content_after_container_is_kept() {
        // content after a closed hidden div must survive (no runaway drop)
        let html = "<html><body>\
<div style=\"display: none\"><span>junk</span><p>hidden text</p></div>\
<p>after junk</p></body></html>";
        let md = md_of(html);
        assert!(md.contains("after junk"), "md: {md}");
        assert!(!md.contains("hidden text"), "md: {md}");
        assert!(
            !md.contains("junk</p") && !md.contains(">junk<"),
            "md: {md}"
        );
    }

    #[test]
    fn overflow_hidden_css_does_not_trigger_hidden_attr_drop() {
        // SEC filing content divs: `overflow:hidden` in style is CSS, not the
        // HTML5 boolean attr — content must survive.
        let html = "<html><body>\
<div style=\"align-items:stretch;overflow:hidden;position:relative;\"><p>visible content</p></div>\
</body></html>";
        let md = md_of(html);
        assert!(md.contains("visible content"), "md: {md}");
    }

    #[test]
    fn true_hidden_attr_still_drops() {
        let html = "<html><body><div hidden><p>secret</p></div><p>kept</p></body></html>";
        let md = md_of(html);
        assert!(!md.contains("secret"), "md: {md}");
        assert!(md.contains("kept"), "md: {md}");
    }

    #[test]
    fn full_page_layout_with_xbrl_header_survives() {
        // simulates the doc_0007 pattern: display:none wrapper around ix:header,
        // followed by overflow:hidden content divs that must NOT be dropped
        let html = "<html><head><title>t</title></head><body>\
<div style=\"display:none;\"><ix:header><ix:hidden>x</ix:hidden></ix:header></div>\
<div style=\"margin-top:30pt;\"></div>\
<div style=\"overflow:hidden;position:relative;\"><p>Item 407 content</p></div>\
</body></html>";
        let md = md_of(html);
        assert!(md.contains("Item 407 content"), "md: {md}");
        assert!(!md.contains("ix:hidden"), "md: {md}");
    }

    #[test]
    fn aria_hidden_and_data_attrs_do_not_drop() {
        // aria-hidden elements are usually still visible; data-hidden-x is not the hidden attr
        let html = "<html><body>\
<span aria-hidden=\"true\">icon</span><div data-hidden-by=\"x\">kept</div>\
<p>tail</p></body></html>";
        let md = md_of(html);
        assert!(md.contains("icon"), "aria-hidden dropped: {md}");
        assert!(md.contains("kept"), "data-hidden dropped: {md}");
        assert!(md.contains("tail"), "md: {md}");
    }

    #[test]
    fn visibility_hidden_and_html5_hidden_attr_drop() {
        let html = "<html><body>\
<div style=\"VISIBILITY:Hidden\">v gone</div>\
<section hidden>s gone</section>\
<p>stay</p></body></html>";
        let md = md_of(html);
        assert!(!md.contains("gone"), "md: {md}");
        assert!(md.contains("stay"), "md: {md}");
    }

    #[test]
    fn unclosed_hidden_void_tag_does_not_swallow_page() {
        // <img ... display:none> is void: never opens tracking, rest of page survives
        let html = "<html><body>\
<img src=\"x.png\" style=\"display:none\" alt=\"pic\">\
<p>survives</p></body></html>";
        let md = md_of(html);
        assert!(md.contains("survives"), "md: {md}");
    }

    #[test]
    fn inline_xbrl_tags_dropped_even_without_display_none() {
        // ix:hidden/ix:header are spec-invisible regardless of wrapper styling
        let html = "<html><body><p>start</p>\
<ix:hidden>ctx ecd:Zzz</ix:hidden>\
<ix:header>hdr</ix:header>\
<p>end</p></body></html>";
        let md = md_of(html);
        assert!(!md.contains("ecd:"), "md: {md}");
        assert!(!md.contains("hdr"), "md: {md}");
        assert!(md.contains("start") && md.contains("end"), "md: {md}");
    }

    #[test]
    fn rsc_streaming_revealed_content_is_kept() {
        // React Server Components streaming (qz.com, wvr c1): the article ships
        // inside `<div hidden id="S:6">…</div>` and is revealed client-side by
        // `$RS("S:6","P:6")` moving it into the `<template id="P:6">` slot.
        // Hidden-but-revealed content IS the page content — it must be kept.
        let html = "<html><body><template id=\"P:6\"></template>\
<div hidden id=\"S:6\"><main><h1>Revealed headline</h1><p>Streaming body text.</p></main></div>\
<script>$RS(\"S:6\",\"P:6\")</script></body></html>";
        let md = md_of(html);
        assert!(
            md.contains("Revealed headline"),
            "RSC content dropped: {md}"
        );
        assert!(
            md.contains("Streaming body text."),
            "RSC body dropped: {md}"
        );
    }

    #[test]
    fn rrc_style_hidden_with_reveal_id_is_kept() {
        // variant: reveal id on a style-hidden (display:none) wrapper — same rule
        let html = "<html><body><template id=\"P:a\"></template>\
<div style=\"display:none\" id=\"S:a\"><p>RC revealed</p></div>\
<script>$RC(\"S:a\",\"P:a\")</script><p>tail</p></body></html>";
        let md = md_of(html);
        assert!(md.contains("RC revealed"), "md: {md}");
        assert!(md.contains("tail"), "md: {md}");
    }

    #[test]
    fn plain_hidden_without_reveal_still_drops() {
        // guard: no reveal script → hidden semantics unchanged (SEC EDGAR case)
        let html = "<html><body><div hidden><p>secret</p></div><p>kept</p></body></html>";
        let md = md_of(html);
        assert!(!md.contains("secret"), "md: {md}");
        assert!(md.contains("kept"), "md: {md}");
    }

    #[test]
    fn wvr_regression_ey_data_layer_attr() {
        // from https://ey.com (wvr c1, minimized 461937 → 523 B): the tag-end
        // scanner was not quote-aware, so the raw `>` inside data-cmp-data-layer
        // JSON (`&lt;p>…&lt;/p>\r\n"`) terminated the tag early and the attribute
        // tail leaked into page text as tag soup / entities.
        let html = "<div data-cmp-data-layer=\"{&#34;text-1a99891ec6&#34;:{&#34;@type&#34;:&#34;ey-unified-site/components/text&#34;,&#34;repo:modifyDate&#34;:&#34;2025-03-31T07:28:45Z&#34;,&#34;xdm:text&#34;:&#34;&lt;p>EY refers to the global organisation, and may refer to one or more, of the member firms of Ernst &amp;amp; Young Global Limited, each of which is a separate legal entity. Ernst &amp;amp; Young Global Limited, a UK company limited by guarantee, does not provide services to clients.&lt;/p>\r\n&#34;}}\" class=\"cmp-text\">";
        let md = md_of(html);
        assert!(!md.contains("cmp-text"), "attr tail leaked: {md}");
        assert!(!md.contains("<p>"), "tag soup leaked: {md}");
        assert!(!md.contains("&#34;"), "entities leaked: {md}");
        assert!(md.trim().is_empty(), "nothing visible should remain: {md}");
    }
}
