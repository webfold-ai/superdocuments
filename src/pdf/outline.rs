//! LLM-free document structure from the PDF's own outline (bookmarks).
//!
//! The heading-detection tree that failed memoryfold's registered quality clause
//! over-segments flat PDF text (median 175 nodes/PDF, title F1 0.056 against
//! PageIndex). A PDF's `/Outlines` chain is the producer's own statement of the
//! document structure — the structure source PageIndex replaces with LLM calls —
//! so this module reads it directly and reports honestly how often it exists.
//!
//! Everything here is additive and defensive: malformed PDFs return
//! `Err(OutlineError)` or an empty tree, never a panic. Cyclic `/Next` chains,
//! `/Parent` loops, absurd nesting and absurd node counts are all bounded.

use std::collections::{HashMap, HashSet};
use std::fmt;

use lopdf::{Dictionary, Document, Object, ObjectId};

/// Hard cap on outline nesting. Real documents stay far below this; anything
/// deeper is treated as malformed rather than followed.
pub const MAX_OUTLINE_DEPTH: u32 = 64;

/// Hard cap on total outline nodes walked per document.
pub const MAX_OUTLINE_NODES: usize = 20_000;

/// One bookmark, with its resolved (1-based) destination page when the document
/// declares one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineNode {
    /// The `/Title` text, decoded from UTF-16BE-with-BOM or PDFDocEncoding.
    pub title: String,
    /// 1-based nesting level; top-level bookmarks are level 1.
    pub level: u32,
    /// 1-based page number this bookmark points at, when it resolved.
    pub start_page: Option<u32>,
    pub children: Vec<OutlineNode>,
}

impl OutlineNode {
    /// Every node in this subtree, in document (pre-order) order.
    pub fn flatten(&self) -> Vec<&OutlineNode> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.flatten());
        }
        out
    }
}

/// Why an outline could not be produced. A PDF without an `/Outlines` entry is
/// not an error — it yields `Ok(vec![])`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlineError {
    /// The bytes are not a PDF lopdf could load (bad header, broken xref, …).
    Parse(String),
    /// The document has a catalog but no page tree, so no destination can map.
    NoCatalog,
}

impl fmt::Display for OutlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "could not parse PDF: {message}"),
            Self::NoCatalog => write!(f, "PDF has no document catalog"),
        }
    }
}

impl std::error::Error for OutlineError {}

/// Number of pages in the document, from the page tree (1-based numbering:
/// page 1 is the first entry in page-tree order).
pub fn page_count(bytes: &[u8]) -> Result<u32, OutlineError> {
    let doc = load(bytes)?;
    let resolver = Resolver::new(&doc);
    Ok(resolver.page_of.len() as u32)
}

/// Build the document's bookmark tree from the catalog `/Outlines` chain.
///
/// Returns `Ok(vec![])` when the document has no outline at all. Never panics.
pub fn outline_tree(bytes: &[u8]) -> Result<Vec<OutlineNode>, OutlineError> {
    let doc = load(bytes)?;
    let catalog = doc.catalog().map_err(|_| OutlineError::NoCatalog)?;
    let Some(outlines_ref) = reference_to(catalog.get(b"Outlines").ok()) else {
        return Ok(Vec::new());
    };
    let Ok(outlines) = doc.get_dictionary(outlines_ref) else {
        return Ok(Vec::new());
    };
    let Some(first) = reference_to(outlines.get(b"First").ok()) else {
        return Ok(Vec::new());
    };

    let resolver = Resolver::new(&doc);
    let mut walker = Walker {
        doc: &doc,
        resolver: &resolver,
        nodes: 0,
        visited: HashSet::new(),
    };
    Ok(walker.siblings(first, 1))
}

fn load(bytes: &[u8]) -> Result<Document, OutlineError> {
    let mut doc =
        Document::load_mem(bytes).map_err(|error| OutlineError::Parse(error.to_string()))?;
    // Many producers encrypt with an empty user password. Decrypting with it is
    // a no-op when it does not work; the outline simply stays unreadable.
    if doc.is_encrypted() {
        let _ = doc.decrypt(b"");
    }
    Ok(doc)
}

/// Page numbers plus the two named-destination lookup tables, resolved once.
struct Resolver {
    /// Page object id -> 1-based page number, in page-tree order.
    page_of: HashMap<ObjectId, u32>,
    /// Catalog `/Dests` dictionary: name -> destination array.
    dests: HashMap<Vec<u8>, Object>,
    /// Catalog `/Names /Dests` name tree, flattened: name -> destination value.
    name_tree: HashMap<Vec<u8>, Object>,
}

impl Resolver {
    fn new(doc: &Document) -> Self {
        let page_of = doc
            .get_pages()
            .into_iter()
            .map(|(number, id)| (id, number))
            .collect::<HashMap<_, _>>();

        let mut dests = HashMap::new();
        let mut name_tree = HashMap::new();
        if let Ok(catalog) = doc.catalog() {
            if let Some(dict) = deref_dict(doc, catalog.get(b"Dests").ok()) {
                for (key, value) in dict.iter() {
                    dests.insert(key.clone(), value.clone());
                }
            }
            if let Some(names) = deref_dict(doc, catalog.get(b"Names").ok()) {
                if let Some(root) = deref_dict(doc, names.get(b"Dests").ok()) {
                    // Bounded like the outline walk itself: a cyclic /Kids chain
                    // must not loop here either.
                    let mut seen = HashSet::new();
                    flatten_name_tree(doc, root, &mut name_tree, &mut seen, 0);
                }
            }
        }
        Self {
            page_of,
            dests,
            name_tree,
        }
    }

    /// Resolve a destination to a 1-based page number.
    ///
    /// Handles `/Dest` arrays, `/A` GoTo actions (whose `/D` is the same array),
    /// a named destination as a name or string, and a destination dictionary
    /// with a `/D` entry. Anything else resolves to `None`.
    fn destination_page(&self, doc: &Document, item: &Dictionary) -> Option<u32> {
        if let Some(dest) = item.get(b"Dest").ok() {
            if let Some(page) = self.dest_page(doc, dest) {
                return Some(page);
            }
        }
        // /A may be an action dictionary or an array of them; only GoTo matters.
        if let Some(action) = item.get(b"A").ok() {
            let action = match action {
                Object::Array(actions) => actions.first(),
                other => Some(other),
            };
            if let Some(action) = action {
                let action = deref_obj(doc, Some(action))?;
                let dict = action.as_dict().ok()?;
                let kind = dict.get(b"S").ok()?.as_name().ok()?;
                if kind != b"GoTo" {
                    return None;
                }
                return self.dest_page(doc, dict.get(b"D").ok()?);
            }
        }
        None
    }

    fn dest_page(&self, doc: &Document, dest: &Object) -> Option<u32> {
        // The destination itself is often an indirect object (`/D 1444 0 R`).
        match deref_obj(doc, Some(dest))? {
            Object::Array(items) => {
                let target = items.first()?;
                self.page_reference(doc, target)
            }
            Object::Dictionary(dict) => {
                let inner = dict.get(b"D").ok()?;
                self.dest_page(doc, inner)
            }
            Object::Name(name) | Object::String(name, _) => self
                .name_tree
                .get(&name.to_vec())
                .and_then(|value| self.dest_page(doc, value))
                .or_else(|| {
                    self.dests
                        .get(&name.to_vec())
                        .and_then(|value| self.dest_page(doc, value))
                }),
            _ => None,
        }
    }

    fn page_reference(&self, doc: &Document, target: &Object) -> Option<u32> {
        match target {
            // `dereference` follows the reference chain and hands back the id it
            // resolved to, which is what the page tree is keyed by.
            Object::Reference(_) | Object::Dictionary(_) => {
                let (id, _) = doc.dereference(target).ok()?;
                self.page_of.get(&id?).copied()
            }
            // Some writers put a page index in a destination whose first element
            // is an integer rather than a reference.
            Object::Integer(number) if *number >= 1 => Some(*number as u32),
            _ => None,
        }
    }
}

struct Walker<'a> {
    doc: &'a Document,
    resolver: &'a Resolver,
    nodes: usize,
    /// Every outline object already walked, so a `/First`/`/Next` cycle — or an
    /// item that claims itself as its own child — terminates instead of
    /// recursing forever.
    visited: HashSet<ObjectId>,
}

impl Walker<'_> {
    /// Walk one sibling chain (`/First`, then `/Next`), recursing into `/First`
    /// children at `level + 1`.
    fn siblings(&mut self, first: ObjectId, level: u32) -> Vec<OutlineNode> {
        let mut out = Vec::new();
        if level > MAX_OUTLINE_DEPTH {
            return out;
        }
        let mut current = Some(first);
        while let Some(id) = current {
            // Cycle detection: an outline object already walked stops the walk.
            if !self.visited.insert(id) || self.nodes >= MAX_OUTLINE_NODES {
                break;
            }
            let Ok(item) = self.doc.get_dictionary(id) else {
                break;
            };
            self.nodes += 1;
            out.push(OutlineNode {
                // The title string is usually indirect too (`/Title 665 0 R`).
                title: deref_obj(self.doc, item.get(b"Title").ok())
                    .and_then(decode_title)
                    .unwrap_or_default(),
                level,
                start_page: self.resolver.destination_page(self.doc, item),
                children: match reference_to(item.get(b"First").ok()) {
                    Some(child_first) => self.siblings(child_first, level + 1),
                    None => Vec::new(),
                },
            });
            current = reference_to(item.get(b"Next").ok());
        }
        out
    }
}

/// A PDF string or name object reduced to its raw bytes.
fn reference_to(object: Option<&Object>) -> Option<ObjectId> {
    match object {
        Some(Object::Reference(id)) => Some(*id),
        _ => None,
    }
}

/// Resolve an optional (possibly indirect) object to its direct value.
fn deref_obj<'a>(doc: &'a Document, object: Option<&'a Object>) -> Option<&'a Object> {
    doc.dereference(object?).ok().map(|(_, o)| o)
}

/// [`deref_obj`], narrowed to a dictionary.
fn deref_dict<'a>(doc: &'a Document, object: Option<&'a Object>) -> Option<&'a Dictionary> {
    deref_obj(doc, object).and_then(|object| object.as_dict().ok())
}

/// Flatten a `/Names /Dests` name tree (`/Names [key value …]` plus `/Kids`)
/// into one map. Depth- and node-bounded; a `/Kids` cycle terminates.
fn flatten_name_tree(
    doc: &Document,
    node: &Dictionary,
    out: &mut HashMap<Vec<u8>, Object>,
    seen: &mut HashSet<ObjectId>,
    depth: u32,
) {
    if depth > MAX_OUTLINE_DEPTH || out.len() >= MAX_OUTLINE_NODES {
        return;
    }
    if let Ok(Object::Array(names)) = node.get(b"Names") {
        for pair in names.chunks(2) {
            if let (Object::String(key, _), Some(value)) = (&pair[0], pair.get(1)) {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    let Some(kids) = deref_obj(doc, node.get(b"Kids").ok()).and_then(|o| o.as_array().ok()) else {
        return;
    };
    for kid in kids {
        if let Some(kid_id) = reference_to(Some(kid)) {
            if !seen.insert(kid_id) {
                continue;
            }
        }
        if let Some(dict) = deref_dict(doc, Some(kid)) {
            flatten_name_tree(doc, dict, out, seen, depth + 1);
        }
    }
}

/// Decode a `/Title`: UTF-16BE when it starts with a BOM, otherwise PDFDocEncoding.
pub fn decode_title(value: &Object) -> Option<String> {
    let Object::String(bytes, _) = value else {
        return None;
    };
    Some(decode_pdf_text(bytes))
}

/// Decode PDF text bytes: UTF-16BE with a byte-order mark, else PDFDocEncoding.
pub fn decode_pdf_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units = bytes[2..]
            .chunks(2)
            .map(|pair| {
                let high = u16::from(pair[0]);
                let low = *pair.get(1).unwrap_or(&0) as u16;
                (high << 8) | low
            })
            .collect::<Vec<u16>>();
        let mut out = String::with_capacity(units.len());
        for unit in char::decode_utf16(units) {
            out.push(unit.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        return out;
    }
    bytes
        .iter()
        .map(|byte| pdf_doc_encoding_char(*byte))
        .collect()
}

/// One byte of PDFDocEncoding → char. ASCII and Latin-1 pass through; the
/// 0x18–0x1F and 0x80–0x9F ranges carry the encoding's own glyphs.
fn pdf_doc_encoding_char(byte: u8) -> char {
    match byte {
        0x18 => '\u{02D8}',
        0x19 => '\u{02C7}',
        0x1A => '\u{02C6}',
        0x1B => '\u{02D9}',
        0x1C => '\u{02DD}',
        0x1D => '\u{02DB}',
        0x1E => '\u{02DA}',
        0x1F => '\u{02DC}',
        0x80 => '\u{2022}',
        0x81 => '\u{2020}',
        0x82 => '\u{2021}',
        0x83 => '\u{2026}',
        0x84 => '\u{2014}',
        0x85 => '\u{2013}',
        0x86 => '\u{0192}',
        0x87 => '\u{2044}',
        0x88 => '\u{2039}',
        0x89 => '\u{203A}',
        0x8A => '\u{2212}',
        0x8B => '\u{2030}',
        0x8C => '\u{201E}',
        0x8D => '\u{201C}',
        0x8E => '\u{201D}',
        0x8F => '\u{2018}',
        0x90 => '\u{2019}',
        0x91 => '\u{201A}',
        0x92 => '\u{2122}',
        0x93 => '\u{FB01}',
        0x94 => '\u{FB02}',
        0x95 => '\u{0141}',
        0x96 => '\u{0152}',
        0x97 => '\u{0160}',
        0x98 => '\u{0178}',
        0x99 => '\u{017D}',
        0x9A => '\u{0131}',
        0x9B => '\u{0142}',
        0x9C => '\u{0153}',
        0x9D => '\u{0161}',
        0x9E => '\u{017E}',
        0x9F => '\u{FFFD}',
        // PDFDocEncoding differs from Latin-1 only in the two ranges above plus
        // the no-break space at 0xA0, which Latin-1 agrees with.
        other => other as char,
    }
}

/// The PageIndex-shaped projection plus the titles whose destination did not
/// resolve (those inherit their previous sibling's start page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageIndexTree {
    /// `[{title, node_id, start_index, end_index, nodes: [...]}, …]`.
    pub json: String,
    /// Titles whose `/Dest` did not resolve, in document order.
    pub unresolved_titles: Vec<String>,
}

/// Render the outline as PageIndex-shaped JSON.
///
/// `start_index` is the node's own start page (1-based; nodes whose destination
/// did not resolve inherit the previous node's start page at the same or a
/// shallower level). `end_index` is the page before the next node at the same or
/// a shallower level starts, or `page_count` for the last one. Nodes stay in
/// document order and `node_id` is a zero-padded pre-order index.
pub fn to_pageindex_tree_json(nodes: &[OutlineNode], page_count: u32) -> String {
    to_pageindex_tree_json_reporting(nodes, page_count).json
}

/// [`to_pageindex_tree_json`], plus the list of titles whose destination did not
/// resolve, so callers can score the projection without silently inheriting.
pub fn to_pageindex_tree_json_reporting(nodes: &[OutlineNode], page_count: u32) -> PageIndexTree {
    // Flatten to pre-order, recording where each root starts.
    let mut flat: Vec<FlatNode<'_>> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    for node in nodes {
        roots.push(flat.len());
        flatten(node, &mut flat);
    }
    let page_count = page_count.max(1);

    // An unresolved destination inherits the previous node's start page.
    let mut previous: Option<u32> = None;
    for node in &mut flat {
        node.start_index = match node.start_page {
            Some(page) => {
                previous = Some(page);
                page.clamp(1, page_count)
            }
            None => previous.unwrap_or(1),
        };
    }
    // `end_index` is the page BEFORE the next node at the same or a shallower
    // level starts, or `page_count` when this subtree is the last thing in the
    // document. (Note: PageIndex's own trees use an exclusive end_index equal to
    // the next node's start page — one page later than this inclusive form.)
    for index in 0..flat.len() {
        let boundary = index + flat[index].size;
        let end = flat
            .get(boundary)
            .map_or(page_count, |next| next.start_index.saturating_sub(1))
            .max(flat[index].start_index);
        flat[index].end_index = end;
    }

    let mut json = String::with_capacity(256 + flat.len() * 128);
    json.push('[');
    for (position, root) in roots.iter().enumerate() {
        if position > 0 {
            json.push(',');
        }
        write_node(&mut json, &flat, *root);
    }
    json.push(']');

    PageIndexTree {
        json,
        unresolved_titles: flat
            .iter()
            .filter(|node| node.start_page.is_none())
            .map(|node| node.title.to_string())
            .collect(),
    }
}

/// Emit the node at `index` and its subtree, from the pre-order flattening.
fn write_node(json: &mut String, flat: &[FlatNode<'_>], index: usize) {
    let node = &flat[index];
    json.push_str("{\"title\":");
    write_json_string(json, node.title);
    json.push_str(",\"node_id\":");
    write_json_string(json, &format!("{index:04}"));
    json.push_str(",\"start_index\":");
    json.push_str(&node.start_index.to_string());
    json.push_str(",\"end_index\":");
    json.push_str(&node.end_index.to_string());
    json.push_str(",\"nodes\":[");
    for offset in 0..node.child_count {
        if offset > 0 {
            json.push(',');
        }
        // In pre-order a node's children follow it, one past its own index.
        write_node(json, flat, index + 1 + offset);
    }
    json.push_str("]}");
}

/// One node in the pre-order flattening of the tree.
struct FlatNode<'a> {
    title: &'a str,
    /// Number of nodes in this subtree, including the node itself, so
    /// `flat[index + size]` is the next node after the whole subtree — which is
    /// the next node at the same or a shallower level whenever one exists.
    size: usize,
    /// How many direct children this node has; they sit at `index + 1..`.
    child_count: usize,
    start_page: Option<u32>,
    /// The projected start page, after inheritance for unresolved destinations.
    start_index: u32,
    end_index: u32,
}

/// Append `node`'s subtree to `out` in pre-order (parent before children).
fn flatten<'a>(node: &'a OutlineNode, out: &mut Vec<FlatNode<'a>>) {
    let index = out.len();
    out.push(FlatNode {
        title: &node.title,
        size: 1,
        child_count: node.children.len(),
        start_page: node.start_page,
        start_index: 1,
        end_index: 1,
    });
    let mut size = 1;
    for child in &node.children {
        flatten(child, out);
        size += out[index + 1 + (size - 1)].size;
    }
    out[index].size = size;
}

/// Append `value` to `out` as a JSON string literal.
///
/// Escapes quotes, backslashes and control characters; multi-byte UTF-8
/// (including non-BMP, i.e. surrogate pairs) is emitted literally, which is
/// valid JSON.
pub fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Total node count in an outline (recursively).
pub fn node_count(nodes: &[OutlineNode]) -> usize {
    nodes.iter().map(|node| node.flatten().len()).sum()
}

/// Deepest nesting level present in an outline (0 for an empty tree).
pub fn max_depth(nodes: &[OutlineNode]) -> u32 {
    nodes
        .iter()
        .flat_map(|node| node.flatten())
        .map(|node| node.level)
        .max()
        .unwrap_or(0)
}

/// How many nodes carry a resolved start page (recursively), and the total.
pub fn resolved_counts(nodes: &[OutlineNode]) -> (usize, usize) {
    let mut resolved = 0;
    let mut total = 0;
    for node in nodes.iter().flat_map(|node| node.flatten()) {
        total += 1;
        if node.start_page.is_some() {
            resolved += 1;
        }
    }
    (resolved, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble a PDF from 1-based-indexed object bodies (no streams needed).
    fn build(objects: Vec<Vec<u8>>) -> Vec<u8> {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }

    fn utf16be(text: &str) -> Vec<u8> {
        let mut out = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }

    fn hex_literal(bytes: &[u8]) -> String {
        format!(
            "<{}>",
            bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()
        )
    }

    /// 3 pages, no outline, no named destinations.
    fn no_outline_pdf() -> Vec<u8> {
        build(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
        ])
    }

    #[test]
    fn outline_absent_is_an_empty_tree_not_an_error() {
        let nodes = outline_tree(&no_outline_pdf()).unwrap();
        assert!(nodes.is_empty(), "{nodes:?}");
        assert_eq!(page_count(&no_outline_pdf()).unwrap(), 3);
        assert_eq!(to_pageindex_tree_json(&nodes, 3), "[]");
    }

    #[test]
    fn malformed_input_returns_an_error_and_never_panics() {
        assert!(matches!(
            outline_tree(b"not a pdf at all"),
            Err(OutlineError::Parse(_))
        ));
        assert!(outline_tree(b"%PDF-1.4\ntruncated").is_err());
        // A catalog pointing at a missing /Outlines object is survivable.
        let broken = build(vec![
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 9 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
        ]);
        assert!(outline_tree(&broken).unwrap().is_empty());
    }

    /// A two-level outline: top-level items with direct `/Dest` arrays, a child,
    /// an item reached by `/A` GoTo, and an item pointing at a named
    /// destination in both the catalog `/Dests` dict and the `/Names /Dests`
    /// name tree.
    fn full_outline_pdf() -> Vec<u8> {
        let utf16 = utf16be("Ünïcode");
        build(vec![
            format!(
                "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R /Dests << /named [5 0 R /XYZ 0 792 0] >> /Names << /Dests << /Names [(treed) [4 0 R /XYZ 0 792 0]] >> >> >>"
            )
            .into_bytes(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Outlines /First 7 0 R /Last 10 0 R /Count 4 >>"
                .to_vec(),
            b"<< /Title (Alpha) /Parent 6 0 R /Next 8 0 R /First 11 0 R /Last 11 0 R /Dest [3 0 R /XYZ 0 792 0] >>".to_vec(),
            b"<< /Title (Beta) /Parent 6 0 R /Next 9 0 R /A << /S /GoTo /D [4 0 R /Fit] >> >>".to_vec(),
            b"<< /Title (Gamma) /Parent 6 0 R /Next 10 0 R /Dest (named) >>".to_vec(),
            b"<< /Title (Delta) /Parent 6 0 R /Dest (treed) >>".to_vec(),
            format!(
                "<< /Title {} /Parent 7 0 R /Dest [5 0 R /XYZ 0 792 0] >>",
                hex_literal(&utf16)
            )
            .into_bytes(),
        ])
    }

    #[test]
    fn two_level_outline_resolves_direct_dest_goto_and_named_destinations() {
        let nodes = outline_tree(&full_outline_pdf()).unwrap();
        assert_eq!(nodes.len(), 4, "{nodes:?}");

        assert_eq!(nodes[0].title, "Alpha");
        assert_eq!(nodes[0].level, 1);
        assert_eq!(nodes[0].start_page, Some(1));
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].children[0].title, "Ünïcode", "UTF-16BE BOM title");
        assert_eq!(nodes[0].children[0].level, 2);
        assert_eq!(nodes[0].children[0].start_page, Some(3));

        assert_eq!(nodes[1].title, "Beta");
        assert_eq!(nodes[1].start_page, Some(2), "/A GoTo action");

        assert_eq!(nodes[2].title, "Gamma");
        assert_eq!(nodes[2].start_page, Some(3), "catalog /Dests dictionary");

        assert_eq!(nodes[3].title, "Delta");
        assert_eq!(nodes[3].start_page, Some(2), "/Names /Dests name tree");

        assert_eq!(node_count(&nodes), 5);
        assert_eq!(max_depth(&nodes), 2);
        assert_eq!(resolved_counts(&nodes), (5, 5));
    }

    #[test]
    fn cyclic_next_chain_terminates_and_keeps_each_node_once() {
        // 7 -> 8 -> 9 -> 7: the /Next chain loops back on itself.
        let pdf = build(vec![
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            b"<< /Type /Outlines /First 7 0 R /Count 3 >>".to_vec(),
            b"<< /Title (One) /Parent 6 0 R /Next 8 0 R /Dest [3 0 R /Fit] >>".to_vec(),
            b"<< /Title (Two) /Parent 6 0 R /Next 9 0 R /Dest [4 0 R /Fit] >>".to_vec(),
            b"<< /Title (Three) /Parent 6 0 R /Next 7 0 R /Dest [3 0 R /Fit] >>".to_vec(),
        ]);
        let nodes = outline_tree(&pdf).unwrap();
        let titles = nodes
            .iter()
            .flat_map(|node| node.flatten())
            .map(|node| node.title.clone())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["One", "Two", "Three"]);
    }

    #[test]
    fn self_parent_and_self_next_chains_terminate() {
        let pdf = build(vec![
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Outlines /First 5 0 R >>".to_vec(),
            b"<< /Title (Loop) /Parent 5 0 R /First 5 0 R /Next 5 0 R >>".to_vec(),
        ]);
        // A /First that points at its own parent must not recurse forever.
        let nodes = outline_tree(&pdf).unwrap();
        assert_eq!(node_count(&nodes), 1, "{nodes:?}");
        assert_eq!(nodes[0].title, "Loop");
        assert_eq!(nodes[0].start_page, None, "no destination to resolve");
    }

    #[test]
    fn unresolved_destinations_are_reported_and_inherit_the_previous_start() {
        let nodes = vec![
            OutlineNode {
                title: "First".into(),
                level: 1,
                start_page: Some(3),
                children: vec![OutlineNode {
                    title: "Unresolved child".into(),
                    level: 2,
                    start_page: None,
                    children: Vec::new(),
                }],
            },
            OutlineNode {
                title: "Second".into(),
                level: 1,
                start_page: Some(5),
                children: Vec::new(),
            },
        ];
        let report = to_pageindex_tree_json_reporting(&nodes, 8);
        assert_eq!(report.unresolved_titles, vec!["Unresolved child"]);
        // Unresolved child inherits "First"'s start page; the parent's end page is
        // the page before "Second" starts.
        assert_eq!(
            report.json,
            r#"[{"title":"First","node_id":"0000","start_index":3,"end_index":4,"nodes":[{"title":"Unresolved child","node_id":"0001","start_index":3,"end_index":4,"nodes":[]}]},{"title":"Second","node_id":"0002","start_index":5,"end_index":8,"nodes":[]}]"#
        );
    }

    #[test]
    fn last_node_ends_at_page_count_and_nodes_keep_document_order() {
        let nodes = vec![
            OutlineNode {
                title: "A".into(),
                level: 1,
                start_page: Some(1),
                children: vec![OutlineNode {
                    title: "A1".into(),
                    level: 2,
                    start_page: Some(2),
                    children: Vec::new(),
                }],
            },
            OutlineNode {
                title: "B".into(),
                level: 1,
                start_page: Some(4),
                children: Vec::new(),
            },
        ];
        let json = to_pageindex_tree_json(&nodes, 9);
        assert_eq!(
            json,
            r#"[{"title":"A","node_id":"0000","start_index":1,"end_index":3,"nodes":[{"title":"A1","node_id":"0001","start_index":2,"end_index":3,"nodes":[]}]},{"title":"B","node_id":"0002","start_index":4,"end_index":9,"nodes":[]}]"#
        );
        assert_eq!(to_pageindex_tree_json(&[], 9), "[]");
    }

    #[test]
    fn json_string_encoder_escapes_quotes_backslashes_control_chars_and_non_bmp() {
        let mut out = String::new();
        write_json_string(
            &mut out,
            "quote\" back\\slash\nnew\ttab\r\u{08}bsp\u{0C}ff\u{01}ctrl 😀 é",
        );
        assert_eq!(
            out,
            "\"quote\\\" back\\\\slash\\nnew\\ttab\\r\\bbsp\\fff\\u0001ctrl 😀 é\""
        );
        // Round-trips through a real JSON parser.
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(
            parsed.as_str().unwrap(),
            "quote\" back\\slash\nnew\ttab\r\u{08}bsp\u{0C}ff\u{01}ctrl 😀 é"
        );
    }

    #[test]
    fn pageindex_json_of_the_full_outline_pdf_is_parseable_and_page_shaped() {
        let nodes = outline_tree(&full_outline_pdf()).unwrap();
        let json = to_pageindex_tree_json(&nodes, page_count_of(&full_outline_pdf()));
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let top = parsed.as_array().expect("array");
        assert_eq!(top.len(), 4);
        assert_eq!(top[0]["title"], "Alpha");
        assert_eq!(top[0]["node_id"], "0000");
        assert_eq!(top[0]["start_index"], 1);
        assert_eq!(
            top[0]["end_index"], 1,
            "Alpha ends on the page before Beta starts"
        );
        assert_eq!(top[0]["nodes"][0]["title"], "Ünïcode");
        assert_eq!(top[0]["nodes"][0]["start_index"], 3);
        assert_eq!(top[3]["end_index"], 3, "the last node ends at page_count");
    }

    fn page_count_of(bytes: &[u8]) -> u32 {
        page_count(bytes).unwrap()
    }

    #[test]
    fn pdf_doc_encoding_titles_decode_and_reject_other_object_types() {
        // 0x92 is the PDFDocEncoding trademark sign, not Latin-1.
        assert_eq!(decode_pdf_text(b"Tk\x92"), "Tk™");
        assert_eq!(decode_pdf_text(b"plain ascii"), "plain ascii");
        assert_eq!(
            decode_pdf_text(&[0xFE, 0xFF, 0x00, 0x41, 0xD8, 0x3D]),
            "A\u{FFFD}",
            "an unpaired surrogate becomes one replacement char"
        );
        assert_eq!(
            decode_pdf_text(&utf16be("a😀b")),
            "a😀b",
            "a surrogate pair decodes"
        );
        assert!(decode_title(&Object::Integer(3)).is_none());
        assert_eq!(
            decode_title(&Object::String(vec![b'X'], lopdf::StringFormat::Literal)).as_deref(),
            Some("X")
        );
    }

    #[test]
    fn depth_and_node_limits_bound_a_degenerate_outline() {
        // A 200-deep chain: the walk stops at MAX_OUTLINE_DEPTH instead of
        // recursing without bound.
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Outlines /First 5 0 R >>".to_vec(),
        ];
        for index in 0..200 {
            let id = 5 + index;
            let next = id + 1;
            let child = if index + 1 < 200 {
                format!("/First {} 0 R", next)
            } else {
                String::new()
            };
            // Nesting only (no /Next), so the walk is a chain, not an explosion.
            objects.push(
                format!(
                    "<< /Title (n{index}) /Parent {} 0 R {child} /Dest [3 0 R /Fit] >>",
                    if index == 0 { 4 } else { id - 1 }
                )
                .into_bytes(),
            );
        }
        let nodes = outline_tree(&build(objects)).unwrap();
        assert_eq!(nodes.len(), 1);
        let depth = nodes[0].flatten().len();
        assert!(depth <= MAX_OUTLINE_DEPTH as usize, "depth {depth}");
    }
}
