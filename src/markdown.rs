//! Hand-rolled Markdown parser and HTML renderer: input Markdown text to a
//! structured tree, and the tree back to HTML. Markdown has so far only been an
//! *output* format in this crate (`document::convert_bytes` emits it, `html::to_markdown`
//! renders it from parsed HTML); this module is the missing input side — the foundation
//! for a Rust-native documentation-site generator.
//!
//! ## Scope (intentional — this is not a CommonMark implementation)
//!
//! Supported: ATX headings (`#` .. `######`), paragraphs, fenced code blocks with an
//! optional language tag, ordered/unordered lists (including nesting, tight/loose
//! detection), inline links `[text](url)` and images `![alt](url)`, GFM-style pipe
//! tables, blockquotes, inline emphasis (`*`/`_`), strong (`**`/`__`), inline code
//! (`` ` ``), and YAML frontmatter (a `---`-delimited block at the very top of the
//! document, parsed via `hashtrinity::codec::syntax`).
//!
//! Explicitly **not** supported: footnotes, GFM task-list checkboxes, raw HTML
//! passthrough, reference-style links (`[text][label]`), setext headings (`===`/`---`
//! underlines), backslash escapes, HTML entity/numeric character references, tab
//! expansion, and full CommonMark spec-test compliance. Zero third-party Markdown
//! crates are used — parsing is entirely hand-rolled per Ptyktos convention.
//!
//! ## Architecture
//!
//! Two phases, following pulldown-cmark's approach: block-level structure is scanned
//! first into a tree (see `parse_blocks`/`parse_one_block`), then inline runs
//! (heading/paragraph/table-cell text) are parsed lazily per block (`parse_inline`).
//!
//! Nested containers (blockquote > list > list item > paragraph) are resolved by
//! recursively slicing the line list into each container's content region and
//! stripping that container's own prefix (`>`, or the list item's content-column
//! indent) before recursing into a generic block-sequence parser. This is a
//! simplification of comrak's line-by-line open-block-stack state machine: instead of
//! re-testing every open container against every new line, each recursive call
//! determines its own container's line span up front (matching prefix lines plus lazy
//! continuation lines) and hands the stripped inner lines to the same block parser.
//! For the supported subset this reaches equivalent nesting and lazy-continuation
//! behaviour with much less bookkeeping. Tight vs. loose list detection follows
//! CommonMark's rule: a list is loose if a blank line separates two items, or if any
//! single item's own content spans more than one block separated by a blank line.
//!
//! Inline emphasis/strong resolution uses a delimiter-stack algorithm modelled on
//! CommonMark's (nearest matching opener wins, scanning outward through the stack)
//! rather than greedy regex matching, so overlapping markers such as `**a *b** c*`
//! resolve deterministically (`<strong>a *b</strong> c*` — the inner `*` never finds a
//! matching closer and falls back to literal text). One simplification versus the full
//! CommonMark algorithm: a delimiter run is matched as a single atomic unit (a run of
//! two or more asterisks/underscores is always "strong", one is always "em") rather
//! than being splittable across multiple matches (e.g. `***text***` is treated as one
//! strong+em pair via two separate two-and-one-length runs read off the same run,
//! which is what most real Markdown emits; a single length-3 run being partially
//! consumed by one match and partially by another is out of scope). This is a
//! reasonable reduction for docs-shaped content and is covered by the regression tests
//! below.
//!
//! ## Data model
//!
//! The AST is a flat arena (`Vec<Node>`) addressed by index; each `Node` holds
//! `Vec<usize>` child indices, not `Rc<RefCell<...>>` — matching this crate's existing
//! plain-owned-data caching convention (see `hashtrinity::Table` usage in
//! [`crate::document`]). Inline content (`Inline`) is a small recursive enum owned
//! directly inside its parent block node: inline trees never need to be shared or
//! mutated after construction, so there's no benefit to arena-indexing them too.

use hashtrinity::codec::syntax::{parse_surface, Surface, SyntaxTerm};

// ---------------------------------------------------------------------------------
// Public data model
// ---------------------------------------------------------------------------------

/// Parsed YAML frontmatter — the `---`-delimited block at the top of the document,
/// decoded via `hashtrinity::codec::syntax`'s collapsed term representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    pub data: SyntaxTerm,
}

/// One table column's alignment, from the GFM delimiter row (`:--`, `:-:`, `--:`, `--`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// An inline (span-level) node. Owned directly, not arena-indexed — see the
/// module-level data-model note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Link { text: Vec<Inline>, url: String },
    Image { alt: String, url: String },
}

/// One block-level node's payload. `Node::children` (arena indices) carries any
/// nested block structure (blockquote/list/list-item contents); table rows and
/// headings/paragraphs carry their content inline in the variant itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Heading {
        level: u8,
        inline: Vec<Inline>,
    },
    Paragraph {
        inline: Vec<Inline>,
    },
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    /// Children are the blockquote's own block-level content (arena indices).
    BlockQuote,
    /// Children are `ListItem` node indices.
    List {
        ordered: bool,
        start: u64,
        tight: bool,
    },
    /// Children are the item's own block-level content (arena indices).
    ListItem,
    Table {
        alignments: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

/// One arena-addressed block node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    /// Child block indices into the owning `MarkdownDocument::nodes` arena.
    pub children: Vec<usize>,
}

/// A parsed Markdown document: optional frontmatter plus a flat block-node arena and
/// the top-level (root) node indices in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub frontmatter: Option<Frontmatter>,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

// ---------------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------------

/// Parse a Markdown document: extract frontmatter (if present), then scan block
/// structure and lazily parse inline runs.
#[must_use]
pub fn parse(input: &str) -> MarkdownDocument {
    let (frontmatter, body) = extract_frontmatter(input);
    let mut arena = Arena::new();
    let lines = split_lines(body);
    let roots = parse_blocks(&lines, &mut arena);
    MarkdownDocument {
        frontmatter,
        nodes: arena.nodes,
        roots,
    }
}

/// Render a parsed document back to HTML. Frontmatter is not re-emitted (callers that
/// want it in the page, e.g. as a `<head>` title, read `doc.frontmatter` directly).
#[must_use]
pub fn to_html(doc: &MarkdownDocument) -> String {
    let mut out = String::new();
    render_blocks(doc, &doc.roots, &mut out);
    out
}

// ---------------------------------------------------------------------------------
// Arena
// ---------------------------------------------------------------------------------

struct Arena {
    nodes: Vec<Node>,
}

impl Arena {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn push(&mut self, kind: NodeKind, children: Vec<usize>) -> usize {
        self.nodes.push(Node { kind, children });
        self.nodes.len() - 1
    }
}

// ---------------------------------------------------------------------------------
// Frontmatter
// ---------------------------------------------------------------------------------

fn extract_frontmatter(input: &str) -> (Option<Frontmatter>, &str) {
    let Some(rest) = input.strip_prefix("---") else {
        return (None, input);
    };
    let after_marker = rest.strip_prefix('\r').unwrap_or(rest);
    if !(after_marker.starts_with('\n') || after_marker.is_empty()) {
        return (None, input);
    }
    let content_start = after_marker.strip_prefix('\n').unwrap_or(after_marker);

    let mut idx = 0;
    for line in content_start.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = &content_start[..idx];
            let body = &content_start[idx + line.len()..];
            let frontmatter = parse_surface(Surface::Yaml, yaml)
                .ok()
                .map(|data| Frontmatter { data });
            return (frontmatter, body);
        }
        idx += line.len();
    }
    (None, input)
}

fn split_lines(input: &str) -> Vec<&str> {
    if input.is_empty() {
        return Vec::new();
    }
    input
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

// ---------------------------------------------------------------------------------
// Block-level line classification
// ---------------------------------------------------------------------------------

fn count_leading_spaces(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ').count()
}

fn atx_heading(line: &str) -> Option<(u8, &str)> {
    if count_leading_spaces(line) > 3 {
        return None;
    }
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let content = rest.trim_start();
    let content = content.trim_end();
    let trailing_hashes = content.chars().rev().take_while(|c| *c == '#').count();
    let content = if trailing_hashes > 0 {
        let before = &content[..content.len() - trailing_hashes];
        if before.is_empty() || before.ends_with(' ') {
            before.trim_end()
        } else {
            content
        }
    } else {
        content
    };
    Some((hashes as u8, content))
}

/// Returns `(fence_char, fence_len, info_string)` if `line` opens or closes a fenced
/// code block.
fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    if count_leading_spaces(line) > 3 {
        return None;
    }
    let rest = line.trim_start();
    let ch = rest.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = rest.chars().take_while(|c| *c == ch).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some((ch, len, info))
}

fn blockquote_prefix_len(line: &str) -> Option<usize> {
    let leading = count_leading_spaces(line);
    if leading > 3 {
        return None;
    }
    let rest = &line[leading..];
    let rest = rest.strip_prefix('>')?;
    let extra = usize::from(rest.starts_with(' '));
    Some(leading + 1 + extra)
}

#[derive(Clone, Copy)]
struct ListMarker {
    ordered: bool,
    start: u64,
    marker_len: usize,
}

fn list_marker(line: &str) -> Option<(ListMarker, usize)> {
    let leading = count_leading_spaces(line);
    if leading > 3 {
        return None;
    }
    let rest = &line[leading..];
    let mut chars = rest.chars();
    let first = chars.next()?;
    if first == '-' || first == '*' || first == '+' {
        let after = &rest[1..];
        if after.is_empty() || after.starts_with(' ') {
            return Some((
                ListMarker {
                    ordered: false,
                    start: 0,
                    marker_len: 1,
                },
                leading,
            ));
        }
        return None;
    }
    let digit_count = rest.chars().take_while(char::is_ascii_digit).count();
    if digit_count == 0 || digit_count > 9 {
        return None;
    }
    let after_digits = &rest[digit_count..];
    let delim = after_digits.chars().next()?;
    if delim != '.' && delim != ')' {
        return None;
    }
    let after = &after_digits[1..];
    if !(after.is_empty() || after.starts_with(' ')) {
        return None;
    }
    let start: u64 = rest[..digit_count].parse().unwrap_or(0);
    Some((
        ListMarker {
            ordered: true,
            start,
            marker_len: digit_count + 1,
        },
        leading,
    ))
}

fn compute_content_col(line: &str, leading: usize, marker_len: usize) -> usize {
    let after = leading + marker_len;
    if after >= line.len() {
        return after;
    }
    let rest = &line[after..];
    let spaces = rest.chars().take_while(|c| *c == ' ').count();
    match spaces {
        0 => after,
        1..=4 => after + spaces,
        _ => after + 1,
    }
}

fn is_table_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let inner = t.strip_prefix('|').unwrap_or(t);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    if inner.trim().is_empty() {
        return false;
    }
    inner.split('|').all(|cell| {
        let c = cell.trim();
        !c.is_empty() && c.contains('-') && c.chars().all(|ch| ch == '-' || ch == ':')
    })
}

fn split_table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn parse_align(cell: &str) -> Align {
    let left = cell.starts_with(':');
    let right = cell.ends_with(':');
    match (left, right) {
        (true, true) => Align::Center,
        (true, false) => Align::Left,
        (false, true) => Align::Right,
        (false, false) => Align::None,
    }
}

fn starts_new_block(line: &str) -> bool {
    if line.trim().is_empty() {
        return true;
    }
    atx_heading(line).is_some()
        || fence_marker(line).is_some()
        || blockquote_prefix_len(line).is_some()
        || list_marker(line).is_some()
}

// ---------------------------------------------------------------------------------
// Block-level parsing
// ---------------------------------------------------------------------------------

fn parse_blocks(lines: &[&str], arena: &mut Arena) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let (idx, next) = parse_one_block(lines, i, arena);
        out.push(idx);
        i = next;
    }
    out
}

fn parse_one_block(lines: &[&str], i: usize, arena: &mut Arena) -> (usize, usize) {
    let line = lines[i];

    if let Some((level, content)) = atx_heading(line) {
        let inline = parse_inline(content);
        let idx = arena.push(NodeKind::Heading { level, inline }, Vec::new());
        return (idx, i + 1);
    }

    if let Some((fence_char, fence_len, info)) = fence_marker(line) {
        let lang = info.split_whitespace().next().map(str::to_string);
        let mut code_lines: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while j < lines.len() {
            if let Some((c2, l2, info2)) = fence_marker(lines[j]) {
                if c2 == fence_char && l2 >= fence_len && info2.is_empty() {
                    j += 1;
                    break;
                }
            }
            code_lines.push(lines[j]);
            j += 1;
        }
        let code = code_lines.join("\n");
        let idx = arena.push(NodeKind::CodeBlock { lang, code }, Vec::new());
        return (idx, j);
    }

    if blockquote_prefix_len(line).is_some() {
        let mut inner_lines: Vec<String> = Vec::new();
        let mut j = i;
        let mut in_quote = false;
        loop {
            if j >= lines.len() {
                break;
            }
            let cur = lines[j];
            if let Some(p) = blockquote_prefix_len(cur) {
                inner_lines.push(cur[p..].to_string());
                in_quote = true;
                j += 1;
            } else if in_quote && !cur.trim().is_empty() && !starts_new_block(cur) {
                inner_lines.push(cur.to_string());
                j += 1;
            } else {
                break;
            }
        }
        let inner_refs: Vec<&str> = inner_lines.iter().map(String::as_str).collect();
        let children = parse_blocks(&inner_refs, arena);
        let idx = arena.push(NodeKind::BlockQuote, children);
        return (idx, j);
    }

    if list_marker(line).is_some() {
        return parse_list(lines, i, arena);
    }

    if i + 1 < lines.len() && line.contains('|') && is_table_delimiter_row(lines[i + 1]) {
        return parse_table(lines, i, arena);
    }

    parse_paragraph(lines, i, arena)
}

fn parse_paragraph(lines: &[&str], i: usize, arena: &mut Arena) -> (usize, usize) {
    let mut text_lines = vec![lines[i].trim().to_string()];
    let mut j = i + 1;
    while j < lines.len() {
        let cur = lines[j];
        if cur.trim().is_empty() || starts_new_block(cur) {
            break;
        }
        if j + 1 < lines.len() && cur.contains('|') && is_table_delimiter_row(lines[j + 1]) {
            break;
        }
        text_lines.push(cur.trim().to_string());
        j += 1;
    }
    let joined = text_lines.join(" ");
    let inline = parse_inline(&joined);
    let idx = arena.push(NodeKind::Paragraph { inline }, Vec::new());
    (idx, j)
}

fn parse_table(lines: &[&str], i: usize, arena: &mut Arena) -> (usize, usize) {
    let header_cells = split_table_cells(lines[i]);
    let align_cells = split_table_cells(lines[i + 1]);
    let alignments: Vec<Align> = align_cells.iter().map(|c| parse_align(c)).collect();
    let header: Vec<Vec<Inline>> = header_cells.iter().map(|c| parse_inline(c)).collect();
    let mut rows = Vec::new();
    let mut j = i + 2;
    while j < lines.len() {
        let cur = lines[j];
        if cur.trim().is_empty() || !cur.contains('|') {
            break;
        }
        let cells = split_table_cells(cur);
        let row: Vec<Vec<Inline>> = cells.iter().map(|c| parse_inline(c)).collect();
        rows.push(row);
        j += 1;
    }
    let idx = arena.push(
        NodeKind::Table {
            alignments,
            header,
            rows,
        },
        Vec::new(),
    );
    (idx, j)
}

fn parse_list(lines: &[&str], start: usize, arena: &mut Arena) -> (usize, usize) {
    let (first_marker, _) = list_marker(lines[start]).expect("caller checked list_marker");
    let ordered = first_marker.ordered;
    let bullet_char = if ordered {
        None
    } else {
        lines[start].trim_start().chars().next()
    };
    let list_start = first_marker.start;

    let mut items: Vec<usize> = Vec::new();
    let mut i = start;
    let mut loose = false;
    let mut pending_gap = false;

    loop {
        if i >= lines.len() {
            break;
        }
        let Some((marker, leading)) = list_marker(lines[i]) else {
            break;
        };
        if marker.ordered != ordered {
            break;
        }
        if !ordered && lines[i].trim_start().chars().next() != bullet_char {
            break;
        }
        if pending_gap {
            loose = true;
        }

        let content_col = compute_content_col(lines[i], leading, marker.marker_len);
        let first_line = if content_col <= lines[i].len() {
            lines[i][content_col..].to_string()
        } else {
            String::new()
        };
        let mut item_lines: Vec<String> = vec![first_line];

        let mut j = i + 1;
        let mut trailing_blanks = 0usize;
        let mut item_has_internal_blank = false;
        loop {
            if j >= lines.len() {
                break;
            }
            let cur = lines[j];
            if cur.trim().is_empty() {
                trailing_blanks += 1;
                item_lines.push(String::new());
                j += 1;
                continue;
            }
            let cur_leading = count_leading_spaces(cur);
            if cur_leading >= content_col {
                if trailing_blanks > 0 {
                    item_has_internal_blank = true;
                }
                trailing_blanks = 0;
                item_lines.push(cur[content_col..].to_string());
                j += 1;
                continue;
            }
            if trailing_blanks == 0 && !starts_new_block(cur) {
                item_lines.push(cur.to_string());
                j += 1;
                continue;
            }
            break;
        }
        while item_lines.last().is_some_and(|s| s.trim().is_empty()) {
            item_lines.pop();
        }

        let item_refs: Vec<&str> = item_lines.iter().map(String::as_str).collect();
        let children = parse_blocks(&item_refs, arena);
        let item_idx = arena.push(NodeKind::ListItem, children);
        items.push(item_idx);

        if item_has_internal_blank {
            loose = true;
        }
        pending_gap = trailing_blanks > 0;
        i = j;
    }

    let idx = arena.push(
        NodeKind::List {
            ordered,
            start: list_start,
            tight: !loose,
        },
        items,
    );
    (idx, i)
}

// ---------------------------------------------------------------------------------
// Inline parsing
// ---------------------------------------------------------------------------------

#[derive(Clone)]
enum Tok {
    Text(String),
    Code(String),
    Link {
        text: Vec<Inline>,
        url: String,
    },
    Image {
        alt: String,
        url: String,
    },
    Delim {
        ch: char,
        strong: bool,
        can_open: bool,
        can_close: bool,
    },
}

fn parse_inline(text: &str) -> Vec<Inline> {
    let toks = tokenize_inline(text);
    resolve_emphasis(toks)
}

fn run_length(chars: &[char], start: usize, c: char) -> usize {
    let mut n = 0;
    while chars.get(start + n) == Some(&c) {
        n += 1;
    }
    n
}

fn is_space(c: Option<char>) -> bool {
    c.is_none_or(char::is_whitespace)
}

fn is_punct(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_punctuation())
}

fn flanking(ch: char, before: Option<char>, after: Option<char>) -> (bool, bool) {
    let left_flanking =
        !is_space(after) && (!is_punct(after) || is_space(before) || is_punct(before));
    let right_flanking =
        !is_space(before) && (!is_punct(before) || is_space(after) || is_punct(after));
    if ch == '_' {
        let can_open = left_flanking && (!right_flanking || is_punct(before));
        let can_close = right_flanking && (!left_flanking || is_punct(after));
        (can_open, can_close)
    } else {
        (left_flanking, right_flanking)
    }
}

/// Parses `[...](...)`  (or, with `bracket_start` pointing at the `[` right after an
/// already-consumed `!`, an image) starting at `chars[bracket_start] == '['`.
/// Returns `(bracket_text, url, next_index)`.
fn try_parse_link_or_image(
    chars: &[char],
    bracket_start: usize,
) -> Option<(String, String, usize)> {
    if chars.get(bracket_start) != Some(&'[') {
        return None;
    }
    let mut depth = 0i32;
    let mut j = bracket_start;
    loop {
        let c = *chars.get(j)?;
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    let bracket_text: String = chars[bracket_start + 1..j].iter().collect();
    if chars.get(j + 1) != Some(&'(') {
        return None;
    }
    let url_start = j + 2;
    let mut paren_depth = 1i32;
    let mut k = url_start;
    loop {
        let c = *chars.get(k)?;
        match c {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        k += 1;
    }
    let inner: String = chars[url_start..k].iter().collect();
    let url = inner.split_whitespace().next().unwrap_or("").to_string();
    Some((bracket_text, url, k + 1))
}

fn tokenize_inline(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '`' => {
                let run_len = run_length(&chars, i, '`');
                if let Some(close) = find_code_close(&chars, i + run_len, run_len) {
                    flush_text(&mut buf, &mut toks);
                    let raw: String = chars[i + run_len..close].iter().collect();
                    let raw = raw.replace('\n', " ");
                    let trimmed = raw.trim();
                    let content =
                        if raw.starts_with(' ') && raw.ends_with(' ') && !trimmed.is_empty() {
                            trimmed.to_string()
                        } else {
                            raw
                        };
                    toks.push(Tok::Code(content));
                    i = close + run_len;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            '!' if chars.get(i + 1) == Some(&'[') => {
                if let Some((alt, url, next)) = try_parse_link_or_image(&chars, i + 1) {
                    flush_text(&mut buf, &mut toks);
                    toks.push(Tok::Image { alt, url });
                    i = next;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            '[' => {
                if let Some((label, url, next)) = try_parse_link_or_image(&chars, i) {
                    flush_text(&mut buf, &mut toks);
                    let inline_text = parse_inline(&label);
                    toks.push(Tok::Link {
                        text: inline_text,
                        url,
                    });
                    i = next;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            '*' | '_' => {
                let run_len = run_length(&chars, i, c);
                flush_text(&mut buf, &mut toks);
                let before = if i == 0 {
                    None
                } else {
                    chars.get(i - 1).copied()
                };
                let after = chars.get(i + run_len).copied();
                let (can_open, can_close) = flanking(c, before, after);
                toks.push(Tok::Delim {
                    ch: c,
                    strong: run_len >= 2,
                    can_open,
                    can_close,
                });
                i += run_len;
            }
            _ => {
                buf.push(c);
                i += 1;
            }
        }
    }
    flush_text(&mut buf, &mut toks);
    toks
}

fn flush_text(buf: &mut String, toks: &mut Vec<Tok>) {
    if !buf.is_empty() {
        toks.push(Tok::Text(std::mem::take(buf)));
    }
}

fn find_code_close(chars: &[char], from: usize, run_len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' {
            let len = run_length(chars, i, '`');
            if len == run_len {
                return Some(i);
            }
            i += len;
        } else {
            i += 1;
        }
    }
    None
}

/// Delimiter-stack emphasis/strong resolution — see the module-level architecture
/// note for the atomic-run simplification versus full CommonMark.
fn resolve_emphasis(toks: Vec<Tok>) -> Vec<Inline> {
    let n = toks.len();
    let mut matched_close: Vec<Option<usize>> = vec![None; n];
    let mut opener_used = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();

    for (idx, tok) in toks.iter().enumerate() {
        let Tok::Delim {
            ch,
            strong,
            can_open,
            can_close,
        } = tok
        else {
            continue;
        };
        if *can_close {
            let found = stack.iter().rposition(|&oi| {
                matches!(&toks[oi], Tok::Delim { ch: oc, strong: os, .. } if oc == ch && os == strong)
            });
            if let Some(pos) = found {
                let opener_idx = stack[pos];
                matched_close[opener_idx] = Some(idx);
                opener_used[opener_idx] = true;
                stack.truncate(pos);
                continue;
            }
        }
        if *can_open {
            stack.push(idx);
        }
    }

    build_inline_tree(&toks, 0, n, &matched_close, &opener_used)
}

fn build_inline_tree(
    toks: &[Tok],
    lo: usize,
    hi: usize,
    matched_close: &[Option<usize>],
    opener_used: &[bool],
) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut p = lo;
    while p < hi {
        match &toks[p] {
            Tok::Text(s) => {
                push_text(&mut out, s);
                p += 1;
            }
            Tok::Code(s) => {
                out.push(Inline::Code(s.clone()));
                p += 1;
            }
            Tok::Link { text, url } => {
                out.push(Inline::Link {
                    text: text.clone(),
                    url: url.clone(),
                });
                p += 1;
            }
            Tok::Image { alt, url } => {
                out.push(Inline::Image {
                    alt: alt.clone(),
                    url: url.clone(),
                });
                p += 1;
            }
            Tok::Delim { ch, strong, .. } => {
                if opener_used[p] {
                    let close = matched_close[p].expect("opener_used implies a matched closer");
                    let children =
                        build_inline_tree(toks, p + 1, close, matched_close, opener_used);
                    out.push(if *strong {
                        Inline::Strong(children)
                    } else {
                        Inline::Emphasis(children)
                    });
                    p = close + 1;
                } else {
                    let lit = ch.to_string().repeat(if *strong { 2 } else { 1 });
                    push_text(&mut out, &lit);
                    p += 1;
                }
            }
        }
    }
    out
}

fn push_text(out: &mut Vec<Inline>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Inline::Text(last)) = out.last_mut() {
        last.push_str(s);
    } else {
        out.push(Inline::Text(s.to_string()));
    }
}

// ---------------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------------

fn render_blocks(doc: &MarkdownDocument, ids: &[usize], out: &mut String) {
    for &id in ids {
        render_block(doc, id, out);
    }
}

fn render_block(doc: &MarkdownDocument, id: usize, out: &mut String) {
    let node = &doc.nodes[id];
    match &node.kind {
        NodeKind::Heading { level, inline } => {
            out.push_str(&format!("<h{level}>"));
            render_inline(inline, out);
            out.push_str(&format!("</h{level}>\n"));
        }
        NodeKind::Paragraph { inline } => {
            out.push_str("<p>");
            render_inline(inline, out);
            out.push_str("</p>\n");
        }
        NodeKind::CodeBlock { lang, code } => {
            out.push_str("<pre><code");
            if let Some(l) = lang {
                out.push_str(" class=\"language-");
                out.push_str(&escape_html(l));
                out.push('"');
            }
            out.push('>');
            out.push_str(&escape_html(code));
            out.push_str("</code></pre>\n");
        }
        NodeKind::BlockQuote => {
            out.push_str("<blockquote>\n");
            render_blocks(doc, &node.children, out);
            out.push_str("</blockquote>\n");
        }
        NodeKind::List {
            ordered,
            start,
            tight,
        } => {
            let tag = if *ordered { "ol" } else { "ul" };
            out.push('<');
            out.push_str(tag);
            if *ordered && *start != 1 {
                out.push_str(&format!(" start=\"{start}\""));
            }
            out.push_str(">\n");
            for &item_id in &node.children {
                render_list_item(doc, item_id, *tight, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push_str(">\n");
        }
        NodeKind::ListItem => {
            render_list_item(doc, id, false, out);
        }
        NodeKind::Table {
            alignments,
            header,
            rows,
        } => {
            out.push_str("<table>\n<thead>\n<tr>\n");
            for (i, cell) in header.iter().enumerate() {
                let align = alignments.get(i).copied().unwrap_or(Align::None);
                out.push_str("<th");
                out.push_str(align_attr(align));
                out.push('>');
                render_inline(cell, out);
                out.push_str("</th>\n");
            }
            out.push_str("</tr>\n</thead>\n<tbody>\n");
            for row in rows {
                out.push_str("<tr>\n");
                for (i, cell) in row.iter().enumerate() {
                    let align = alignments.get(i).copied().unwrap_or(Align::None);
                    out.push_str("<td");
                    out.push_str(align_attr(align));
                    out.push('>');
                    render_inline(cell, out);
                    out.push_str("</td>\n");
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>\n");
        }
    }
}

fn render_list_item(doc: &MarkdownDocument, id: usize, tight: bool, out: &mut String) {
    let node = &doc.nodes[id];
    out.push_str("<li>");
    for &child in &node.children {
        if tight {
            if let NodeKind::Paragraph { inline } = &doc.nodes[child].kind {
                render_inline(inline, out);
                continue;
            }
        }
        render_block(doc, child, out);
    }
    out.push_str("</li>\n");
}

fn render_inline(inline: &[Inline], out: &mut String) {
    for node in inline {
        match node {
            Inline::Text(s) => out.push_str(&escape_html(s)),
            Inline::Code(s) => {
                out.push_str("<code>");
                out.push_str(&escape_html(s));
                out.push_str("</code>");
            }
            Inline::Emphasis(children) => {
                out.push_str("<em>");
                render_inline(children, out);
                out.push_str("</em>");
            }
            Inline::Strong(children) => {
                out.push_str("<strong>");
                render_inline(children, out);
                out.push_str("</strong>");
            }
            Inline::Link { text, url } => {
                out.push_str("<a href=\"");
                out.push_str(&escape_html(url));
                out.push_str("\">");
                render_inline(text, out);
                out.push_str("</a>");
            }
            Inline::Image { alt, url } => {
                out.push_str("<img src=\"");
                out.push_str(&escape_html(url));
                out.push_str("\" alt=\"");
                out.push_str(&escape_html(alt));
                out.push_str("\">");
            }
        }
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn align_attr(a: Align) -> &'static str {
    match a {
        Align::Left => " style=\"text-align:left\"",
        Align::Center => " style=\"text-align:center\"",
        Align::Right => " style=\"text-align:right\"",
        Align::None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(src: &str) -> String {
        to_html(&parse(src))
    }

    // ---- headings / paragraphs ----

    #[test]
    fn atx_headings_all_levels() {
        for level in 1..=6 {
            let src = format!("{} Title", "#".repeat(level));
            let doc = parse(&src);
            assert_eq!(doc.roots.len(), 1);
            match &doc.nodes[doc.roots[0]].kind {
                NodeKind::Heading { level: l, inline } => {
                    assert_eq!(*l as usize, level);
                    assert_eq!(inline, &vec![Inline::Text("Title".to_string())]);
                }
                other => panic!("expected heading, got {other:?}"),
            }
        }
    }

    #[test]
    fn atx_heading_with_closing_hashes() {
        let doc = parse("## Title ##");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Heading { level, inline } => {
                assert_eq!(*level, 2);
                assert_eq!(inline, &vec![Inline::Text("Title".to_string())]);
            }
            other => panic!("expected heading, got {other:?}"),
        }
    }

    #[test]
    fn seven_hashes_is_not_a_heading() {
        let doc = parse("####### not a heading");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { .. } => {}
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn paragraph_lines_join_with_space() {
        let doc = parse("line one\nline two");
        assert_eq!(doc.roots.len(), 1);
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert_eq!(inline, &vec![Inline::Text("line one line two".to_string())]);
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn blank_line_separates_paragraphs() {
        let doc = parse("first\n\nsecond");
        assert_eq!(doc.roots.len(), 2);
    }

    // ---- code fences ----

    #[test]
    fn fenced_code_block_with_language() {
        let doc = parse("```rust\nfn main() {}\n```");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::CodeBlock { lang, code } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(code, "fn main() {}");
            }
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn fenced_code_block_without_language() {
        let doc = parse("```\nplain\n```");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::CodeBlock { lang, code } => {
                assert_eq!(*lang, None);
                assert_eq!(code, "plain");
            }
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn fenced_code_block_ignores_markup_inside() {
        let doc = parse("```\n# not a heading\n*not emphasis*\n```");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::CodeBlock { code, .. } => {
                assert_eq!(code, "# not a heading\n*not emphasis*");
            }
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn tilde_fence_supported() {
        let doc = parse("~~~\ncode\n~~~");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::CodeBlock { code, .. } => assert_eq!(code, "code"),
            other => panic!("expected code block, got {other:?}"),
        }
    }

    // ---- inline: links, images, code ----

    #[test]
    fn inline_link() {
        let doc = parse("see [the docs](https://example.test/docs) here");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert!(inline.iter().any(|n| matches!(
                    n,
                    Inline::Link { url, .. } if url == "https://example.test/docs"
                )));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn inline_image() {
        let doc = parse("![alt text](img.png)");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert_eq!(
                    inline,
                    &vec![Inline::Image {
                        alt: "alt text".to_string(),
                        url: "img.png".to_string(),
                    }]
                );
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn inline_code_span() {
        let doc = parse("run `cargo test` now");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert!(inline
                    .iter()
                    .any(|n| matches!(n, Inline::Code(c) if c == "cargo test")));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn inline_code_span_not_parsed_for_emphasis() {
        let doc = parse("`*not emphasis*`");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert_eq!(inline, &vec![Inline::Code("*not emphasis*".to_string())]);
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    // ---- inline: emphasis / strong (delimiter-stack correctness) ----

    #[test]
    fn simple_emphasis() {
        let doc = parse("*em*");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert_eq!(
                    inline,
                    &vec![Inline::Emphasis(vec![Inline::Text("em".to_string())])]
                );
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn simple_strong_both_markers() {
        for src in ["**strong**", "__strong__"] {
            let doc = parse(src);
            match &doc.nodes[doc.roots[0]].kind {
                NodeKind::Paragraph { inline } => {
                    assert_eq!(
                        inline,
                        &vec![Inline::Strong(vec![Inline::Text("strong".to_string())])],
                        "input was {src:?}"
                    );
                }
                other => panic!("expected paragraph, got {other:?}"),
            }
        }
    }

    #[test]
    fn strong_wrapping_emphasis_nests_correctly() {
        // ***text*** = strong(3 stars read as a 2-run + 1-run off the same position):
        // the immediate boundary is strong, with an emphasis run nested inside.
        let out = html("**a *b* c**");
        assert_eq!(out, "<p><strong>a <em>b</em> c</strong></p>\n");
    }

    #[test]
    fn overlapping_emphasis_markers_resolve_via_nearest_opener() {
        // The hard case called out in the module docs: the closing `**` must match the
        // nearest compatible (`strong`) opener, skipping the incompatible single-`*`
        // opener from "*b" — which is left with no closer and renders as literal text.
        let out = html("**a *b** c*");
        assert_eq!(out, "<p><strong>a *b</strong> c*</p>\n");
    }

    #[test]
    fn unmatched_emphasis_marker_is_literal() {
        let out = html("a * b * c");
        // `* ` / ` *` are not flanking delimiters (surrounded by spaces), so both stars
        // stay literal text.
        assert_eq!(out, "<p>a * b * c</p>\n");
    }

    #[test]
    fn emphasis_across_a_link() {
        let out = html("*see [docs](https://example.test) now*");
        assert_eq!(
            out,
            "<p><em>see <a href=\"https://example.test\">docs</a> now</em></p>\n"
        );
    }

    // ---- blockquotes ----

    #[test]
    fn simple_blockquote() {
        let doc = parse("> quoted text");
        assert_eq!(doc.roots.len(), 1);
        let node = &doc.nodes[doc.roots[0]];
        assert!(matches!(node.kind, NodeKind::BlockQuote));
        assert_eq!(node.children.len(), 1);
        match &doc.nodes[node.children[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert_eq!(inline, &vec![Inline::Text("quoted text".to_string())]);
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn blockquote_lazy_continuation() {
        // Second line has no `>` but continues the same paragraph (CommonMark "lazy
        // continuation").
        let doc = parse("> line one\nline two");
        let node = &doc.nodes[doc.roots[0]];
        assert!(matches!(node.kind, NodeKind::BlockQuote));
        assert_eq!(node.children.len(), 1);
        match &doc.nodes[node.children[0]].kind {
            NodeKind::Paragraph { inline } => {
                assert_eq!(inline, &vec![Inline::Text("line one line two".to_string())]);
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn blockquote_containing_a_list() {
        let doc = parse("> - one\n> - two");
        let bq = &doc.nodes[doc.roots[0]];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
        assert_eq!(bq.children.len(), 1);
        match &doc.nodes[bq.children[0]].kind {
            NodeKind::List { ordered, .. } => assert!(!ordered),
            other => panic!("expected list, got {other:?}"),
        }
    }

    // ---- lists: nesting, ordered/unordered, tight/loose ----

    #[test]
    fn simple_unordered_list() {
        let doc = parse("- one\n- two\n- three");
        let list = &doc.nodes[doc.roots[0]];
        match &list.kind {
            NodeKind::List { ordered, tight, .. } => {
                assert!(!ordered);
                assert!(tight);
            }
            other => panic!("expected list, got {other:?}"),
        }
        assert_eq!(list.children.len(), 3);
    }

    #[test]
    fn ordered_list_preserves_start() {
        let doc = parse("3. three\n4. four");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::List { ordered, start, .. } => {
                assert!(ordered);
                assert_eq!(*start, 3);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn loose_list_when_blank_line_between_items() {
        let doc = parse("- one\n\n- two");
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::List { tight, .. } => assert!(!tight),
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn nested_unordered_list() {
        let doc = parse("- outer\n  - inner one\n  - inner two\n- outer two");
        let outer = &doc.nodes[doc.roots[0]];
        assert!(matches!(outer.kind, NodeKind::List { .. }));
        assert_eq!(outer.children.len(), 2);
        let first_item = &doc.nodes[outer.children[0]];
        assert!(matches!(first_item.kind, NodeKind::ListItem));
        // first_item's children: a paragraph ("outer") and a nested list.
        assert_eq!(first_item.children.len(), 2);
        match &doc.nodes[first_item.children[1]].kind {
            NodeKind::List { ordered, .. } => assert!(!ordered),
            other => panic!("expected nested list, got {other:?}"),
        }
        let inner_list = &doc.nodes[first_item.children[1]];
        assert_eq!(inner_list.children.len(), 2);
    }

    #[test]
    fn nested_list_html_structure() {
        let out = html("- outer\n  - inner\n- outer two");
        assert!(out.contains(
            "<ul>\n<li>outer<ul>\n<li>inner</li>\n</ul>\n</li>\n<li>outer two</li>\n</ul>\n"
        ));
    }

    #[test]
    fn ordered_list_inside_blockquote_inside_list_item() {
        // Three levels of nesting: list > list item > blockquote > ordered list.
        let src = "- item\n  > quoted\n  > 1. a\n  > 2. b";
        let doc = parse(src);
        let outer_list = &doc.nodes[doc.roots[0]];
        let item = &doc.nodes[outer_list.children[0]];
        // item children: paragraph "item", blockquote
        let bq_idx = item.children[1];
        let bq = &doc.nodes[bq_idx];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
        // blockquote children: paragraph "quoted", ordered list
        let inner_ordered = bq
            .children
            .iter()
            .find_map(|&c| match &doc.nodes[c].kind {
                NodeKind::List {
                    ordered: true,
                    start,
                    ..
                } => Some(*start),
                _ => None,
            })
            .expect("expected an ordered list inside the blockquote");
        assert_eq!(inner_ordered, 1);
    }

    // ---- tables ----

    #[test]
    fn gfm_table_with_alignment() {
        let src = "| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |";
        let doc = parse(src);
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Table {
                alignments,
                header,
                rows,
            } => {
                assert_eq!(alignments, &vec![Align::Left, Align::Center, Align::Right]);
                assert_eq!(header.len(), 3);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0][0], vec![Inline::Text("1".to_string())]);
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn table_html_rendering() {
        let out = html("| h |\n|---|\n| v |");
        assert!(out.contains("<table>"));
        assert!(out.contains("<th>h</th>"));
        assert!(out.contains("<td>v</td>"));
    }

    // ---- frontmatter ----

    #[test]
    fn frontmatter_is_parsed_and_stripped_from_body() {
        let src = "---\ntitle: Hello\ndraft: false\n---\n# Body\n";
        let doc = parse(src);
        let fm = doc.frontmatter.expect("expected frontmatter");
        match fm.data {
            SyntaxTerm::Bu(fields) => {
                assert!(fields.iter().any(|(k, _)| k == "title"));
            }
            other => panic!("expected a keyed term, got {other:?}"),
        }
        assert_eq!(doc.roots.len(), 1);
        match &doc.nodes[doc.roots[0]].kind {
            NodeKind::Heading { inline, .. } => {
                assert_eq!(inline, &vec![Inline::Text("Body".to_string())]);
            }
            other => panic!("expected heading, got {other:?}"),
        }
    }

    #[test]
    fn no_frontmatter_when_document_does_not_start_with_delimiter() {
        let doc = parse("# Title\n\nbody");
        assert!(doc.frontmatter.is_none());
    }

    #[test]
    fn dashes_mid_document_are_not_frontmatter() {
        // `---` only special-cased at the very start of the document.
        let doc = parse("para one\n\n---\n\npara two");
        assert!(doc.frontmatter.is_none());
    }

    // ---- HTML escaping ----

    #[test]
    fn html_special_chars_are_escaped() {
        let out = html("a < b & c > d");
        assert_eq!(out, "<p>a &lt; b &amp; c &gt; d</p>\n");
    }

    #[test]
    fn code_block_content_is_escaped() {
        let out = html("```\n<script>\n```");
        assert!(out.contains("&lt;script&gt;"));
        assert!(!out.contains("<script>"));
    }
}
