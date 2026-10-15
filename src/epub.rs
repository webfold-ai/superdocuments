//! Bounded EPUB reader for the Ptyktos substrate.
//!
//! EPUB is a ZIP container: `META-INF/container.xml` points at an OPF (Open Packaging Format)
//! "package" document, which carries the manifest of all resources and the spine (the canonical
//! reading order) as a list of `idref`s. The spine is what the reader walks; the table of
//! contents is navigation metadata, not a list of bodies to extract.
//!
//! The module is spine-driven, not TOC-driven. Every decision that the EPUB and OCF rules fix is
//! taken by a kernel of `proof/extract_epub.rs`, which the proofs in `proof/EpubProofs.lean` relate
//! to the specification in `proof/EpubSpec.lean`:
//!
//! * the role of each package element (`opf_child_role`), so only an OPF `item` directly inside the
//!   manifest is a manifest entry and only an OPF `itemref` directly inside the spine is a spine
//!   reference;
//! * manifest acceptance (`manifest_item_accept`) and the id index built by one insert per
//!   manifest entry (`manifest_index_step`) in an `apeiron::Map`;
//! * each spine reference's decision (`spine_step`): read, missing, duplicate, repeated or foreign,
//!   with a corrupt table answer refused rather than read;
//! * href resolution against the package directory: percent-decoding (`pct_hex`), dot segments
//!   (`path_segment_kind`) and the separators (`href_byte_class`);
//! * the ZIP entry bound on both the declared and the actually read size (`zip_entry_bound`) and
//!   the text budget with its two-byte chapter separator (`epub_text_budget`).
//!
//! A spine reference that names no manifest item, or two or more items with one id, is reported
//! and never resolved; a foreign item (not XHTML or SVG) is reported and not read. Nothing is
//! silently dropped: [`Book::skipped`] lists each refusal.
//!
//! ## What is NOT here
//!
//! No CSS, no embedded fonts, no images, no scripting, no DRM. The extract is a typed text spine;
//! if a caller needs a chapter's media, it re-opens the EPUB with its own reader.

use crate::html::to_markdown;
use apeiron::Map;
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// The loop-free EPUB kernels of `proof/extract_epub.rs`, compiled into the library unchanged: the
/// reader calls these same functions on its decision path, and the proofs in
/// `proof/EpubProofs.lean` are about them (`proof/regen.sh` extracts the same source).
// The lints allowed here are about the kernels' shape (a range check written out, a bool-returning
// if). That shape is what Charon/Aeneas extract, so the kernels keep it and the proofs stay about
// this exact code.
#[allow(
    dead_code,
    clippy::manual_range_contains,
    clippy::needless_bool,
    clippy::nonminimal_bool
)]
#[path = "../proof/extract_epub.rs"]
mod kernels;

const OPF_NAMESPACE: &str = "http://www.idpf.org/2007/opf";

/// Limits on one ZIP entry (declared and actually read), the whole archive, the entry count, the
/// manifest, and the rendered text.
const MAX_ENTRY: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;
const MAX_XHTML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

/// The media classes of the EPUB content documents (`EpubSpec.isContentDoc`).
const MEDIA_XHTML: u8 = 1;
const MEDIA_SVG: u8 = 2;

/// One manifest item, indexed by its package id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestItem {
    /// The id from the OPF `<item id="...">`. Spine idrefs point at this.
    pub id: String,
    /// The ZIP entry the item names: its `href`, resolved against the package directory with
    /// percent-decoding and dot segments removed (so `OEBPS/chapter1.xhtml`).
    pub href: String,
    /// The MIME type from the OPF (`application/xhtml+xml` for chapters, `image/png` for covers).
    pub media_type: String,
}

/// One spine entry that is read, in canonical reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpineItem {
    pub id: String,
    pub href: String,
}

/// A spine reference that is reported and not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpineSkip {
    /// The idref names no manifest item.
    Missing { idref: String },
    /// Two or more manifest items have this id; none is resolved.
    Duplicate { idref: String },
    /// The item is not an EPUB content document (no fallback chain is followed).
    Foreign { idref: String, href: String },
    /// The item was already read earlier in the spine.
    Repeated { idref: String, href: String },
}

/// A parsed EPUB package: the accepted manifest, its id index, the spine in reading order, and
/// the references that were reported. Holds no file bytes; the original container is passed back
/// to [`extract_text`].
pub struct Book {
    /// Accepted manifest items in document order; the position is the index the kernels see.
    items: Vec<ManifestItem>,
    /// Id to `(first position, duplicated)`, built by one insert per item (`manifest_index_step`).
    index: Map<String, (u32, bool)>,
    spine: Vec<SpineItem>,
    skipped: Vec<SpineSkip>,
    /// The package document's ZIP entry name (e.g. `OEBPS/content.opf`).
    opf_path: String,
    /// Title from the OPF `<metadata><dc:title>`, if present.
    title: Option<String>,
}

/// Read the EPUB's container, parse the package document, and return the manifest index and the
/// spine. Bytes are not copied; the caller keeps the `&[u8]` alive for the subsequent
/// [`extract_text`] call.
pub fn read(bytes: &[u8]) -> Result<Book, String> {
    if bytes.len() as u64 > MAX_TOTAL {
        return Err(format!("EPUB too large: {} bytes", bytes.len()));
    }
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("invalid EPUB ZIP: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("too many entries: {}", zip.len()));
    }

    // 1. META-INF/container.xml names the package document; its full-path is a URL relative to
    //    the container root, so it is resolved against an empty base.
    let container = read_entry(&mut zip, "META-INF/container.xml", MAX_ENTRY, "entry")?;
    let opf_url = parse_container(&container)?;
    let opf_path = String::from_utf8_lossy(&resolve_href(b"", opf_url.as_bytes())).into_owned();

    // 2. The package document: manifest items and spine references, in document order.
    let opf = read_entry(&mut zip, &opf_path, MAX_ENTRY, "entry")?;
    let package = parse_package(&opf)?;
    if package.spine.len() > MAX_ENTRIES || package.manifest.len() > MAX_ENTRIES {
        return Err("package too large".into());
    }

    // 3. The manifest index: one insert per accepted item, in document order.
    let mut index: Map<String, (u32, bool)> = Map::new();
    let mut items: Vec<ManifestItem> = Vec::with_capacity(package.manifest.len());
    for raw in package.manifest {
        let k = items.len() as u32;
        let prev = index.get_at(raw.id.as_str()).copied();
        let (first, duplicated) =
            kernels::manifest_index_step(prev.is_some(), prev.map_or(0, |p| p.0), k);
        index.insert(raw.id.clone(), (first, duplicated));
        let href = String::from_utf8_lossy(&resolve_href(opf_url.as_bytes(), raw.href.as_bytes()))
            .into_owned();
        items.push(ManifestItem {
            id: raw.id,
            href,
            media_type: raw.media_type,
        });
    }

    // 4. The spine walk: one kernel decision per itemref, in order.
    let count = items.len() as u32;
    let mut seen = vec![false; items.len()];
    let mut spine = Vec::new();
    let mut skipped = Vec::new();
    for idref in package.spine {
        let (found, first, dup) = match index.get_at(idref.as_str()).copied() {
            Some((first, dup)) => (true, first, dup),
            None => (false, 0, false),
        };
        let entry = if found {
            items.get(first as usize)
        } else {
            None
        };
        let id_eq = entry.is_some_and(|it| it.id == idref);
        let content_doc = entry.is_some_and(|it| is_content_doc(media_class(&it.media_type)));
        let is_seen = seen.get(first as usize).copied().unwrap_or(false);
        let code = kernels::spine_step(found, dup, first, count, id_eq, content_doc, is_seen);
        let href = entry.map(|it| it.href.clone()).unwrap_or_default();
        match code {
            0 => {
                seen[first as usize] = true;
                spine.push(SpineItem { id: idref, href });
            }
            1 => skipped.push(SpineSkip::Missing { idref }),
            2 => skipped.push(SpineSkip::Duplicate { idref }),
            3 => skipped.push(SpineSkip::Repeated { idref, href }),
            4 => skipped.push(SpineSkip::Foreign { idref, href }),
            _ => return Err(format!("spine: refused table answer for {idref:?}")),
        }
    }

    Ok(Book {
        items,
        index,
        spine,
        skipped,
        opf_path,
        title: package.title,
    })
}

/// Walk the book's spine, one chapter at a time, and concatenate each chapter's body as Markdown.
/// Returns an error if any chapter is malformed or the text does not fit the budget; partial
/// output is NOT returned on error.
pub fn extract_text(book: &Book, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_TOTAL {
        return Err(format!("EPUB too large: {} bytes", bytes.len()));
    }
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("invalid EPUB ZIP: {e}"))?;
    let mut chapters = Vec::with_capacity(book.spine.len());
    for item in &book.spine {
        let entry = read_entry(&mut zip, &item.href, MAX_XHTML_BYTES, "spine entry")?;
        // base_url is local because the chapter is being read for its own text; relative links do
        // not survive extraction here.
        chapters.push(to_markdown(&entry, ""));
    }
    join_chapters(&chapters, MAX_TEXT_BYTES)
}

/// Join the rendered chapters with the two-byte `\n\n` separator, refusing the whole book when the
/// joined text would exceed `max` (`epub_text_budget` decides each step).
fn join_chapters(chapters: &[String], max: u64) -> Result<String, String> {
    let mut out = String::new();
    for (i, md) in chapters.iter().enumerate() {
        let sep: u64 = if i == 0 { 0 } else { 2 };
        let (fits, _) = kernels::epub_text_budget(out.len() as u64, sep, md.len() as u64, max);
        if !fits {
            return Err(format!(
                "spine text exceeds {max} bytes at chapter {i} (with separator)"
            ));
        }
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(md);
    }
    Ok(out)
}

/// Spine in reading order: the chapters [`extract_text`] reads.
pub fn spine(book: &Book) -> &[SpineItem] {
    &book.spine
}

/// The spine references that were reported and not read, in spine order.
pub fn skipped(book: &Book) -> &[SpineSkip] {
    &book.skipped
}

/// Title from the OPF metadata, if the package document declared one.
pub fn title(book: &Book) -> Option<&str> {
    book.title.as_deref()
}

/// The package document's ZIP entry name.
pub fn opf_path(book: &Book) -> &str {
    &book.opf_path
}

/// Manifest lookup by package id. Returns `None` for an unknown id and for an id that two or more
/// items share: a duplicated id is never resolved to one of them.
pub fn manifest_get<'a>(book: &'a Book, id: &str) -> Option<&'a ManifestItem> {
    match book.index.get_at(id) {
        Some(&(first, false)) => book.items.get(first as usize),
        _ => None,
    }
}

// ── internal: ZIP entries ───────────────────────────────────────────────

/// Read one ZIP entry, refusing it when the declared size or the bytes actually read exceed `max`.
fn read_entry(
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    max: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    let file = zip
        .by_name(name)
        .map_err(|e| format!("missing {label} {name:?}: {e}"))?;
    let declared = file.size();
    let mut out = Vec::with_capacity(declared.min(max) as usize);
    file.take(max.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| format!("read error for {name:?}: {e}"))?;
    if !kernels::zip_entry_bound(declared, out.len() as u64, max) {
        return Err(format!(
            "{label} {name:?} too large: declared {declared} bytes, read {} bytes (limit {max})",
            out.len()
        ));
    }
    Ok(out)
}

// ── internal: container and package parsing ─────────────────────────────

/// The first `rootfile@full-path` of `container.xml`, as written (a URL relative to the root).
fn parse_container(bytes: &[u8]) -> Result<String, String> {
    let s = std::str::from_utf8(bytes).map_err(|e| format!("container.xml is not UTF-8: {e}"))?;
    let mut reader = NsReader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::with_capacity(256);
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("container.xml parse error: {e}"))?;
        match event {
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref().as_bytes() == b"rootfile" =>
            {
                if let Some(p) = attr(&e, b"full-path") {
                    return Ok(p);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Err("container.xml has no <rootfile full-path=...>".into())
}

/// The raw package content: manifest items accepted by the kernel, spine idrefs in order, and the
/// title.
struct Package {
    manifest: Vec<RawItem>,
    spine: Vec<String>,
    title: Option<String>,
}

struct RawItem {
    id: String,
    href: String,
    media_type: String,
}

/// The `opf_child_role` element-name code of a local name.
fn element_code(local: &str) -> u8 {
    match local {
        "package" => 1,
        "manifest" => 2,
        "spine" => 3,
        "item" => 4,
        "itemref" => 5,
        _ => 0,
    }
}

fn parse_package(bytes: &[u8]) -> Result<Package, String> {
    let s = std::str::from_utf8(bytes).map_err(|e| format!("OPF is not UTF-8: {e}"))?;
    let mut reader = NsReader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut manifest: Vec<RawItem> = Vec::new();
    let mut spine: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    // The role of each open element (the document is role 0 at the root).
    let mut roles: Vec<u8> = Vec::new();
    let mut buf = Vec::with_capacity(1024);
    loop {
        let (ns, event) = reader
            .read_resolved_event_into(&mut buf)
            .map_err(|e| format!("OPF parse error: {e}"))?;
        let opf = matches!(ns, ResolveResult::Bound(n) if n.0 == OPF_NAMESPACE);
        match event {
            Event::Start(e)
                if title.is_none() && e.local_name().as_ref().as_bytes() == b"title" =>
            {
                // The first title's text: read it whole, so its end tag is consumed here.
                let inner = reader
                    .read_text(e.name())
                    .map_err(|err| format!("OPF title read: {err}"))?;
                let raw: &str = inner.as_ref();
                title = Some(
                    quick_xml::escape::unescape(raw)
                        .map_or_else(|_| raw.to_string(), |text| text.into_owned()),
                );
            }
            Event::Start(e) => {
                let parent = roles.last().copied().unwrap_or(0);
                let role = role_of(parent, &e, opf);
                roles.push(role);
                visit(role, &e, &mut manifest, &mut spine);
            }
            Event::Empty(e) => {
                let parent = roles.last().copied().unwrap_or(0);
                let role = role_of(parent, &e, opf);
                visit(role, &e, &mut manifest, &mut spine);
            }
            Event::End(_) => {
                roles.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(Package {
        manifest,
        spine,
        title,
    })
}

/// The role of an element under its parent's role (`opf_child_role`).
fn role_of(parent: u8, e: &BytesStart<'_>, opf: bool) -> u8 {
    kernels::opf_child_role(parent, element_code(e.local_name().as_ref()), opf)
}

/// Act on an element by its role: a manifest item enters the manifest only if the kernel accepts
/// it, and a spine itemref is recorded in order (a reference without an idref is recorded empty,
/// so it is reported as missing rather than dropped).
fn visit(role: u8, e: &BytesStart<'_>, manifest: &mut Vec<RawItem>, spine: &mut Vec<String>) {
    match role {
        4 => {
            let id = attr(e, b"id");
            let href = attr(e, b"href");
            let media_type = attr(e, b"media-type").unwrap_or_default();
            let id_len = id.as_ref().map_or(0, |s| s.len() as u64);
            if kernels::manifest_item_accept(id.is_some(), href.is_some(), id_len) {
                if let (Some(id), Some(href)) = (id, href) {
                    manifest.push(RawItem {
                        id,
                        href,
                        media_type,
                    });
                }
            }
        }
        5 => spine.push(attr(e, b"idref").unwrap_or_default()),
        _ => {}
    }
}

/// `1` XHTML, `2` SVG, `0` any other media type (`EpubSpec` media classes).
fn media_class(media_type: &str) -> u8 {
    match media_type {
        "application/xhtml+xml" => MEDIA_XHTML,
        "image/svg+xml" => MEDIA_SVG,
        _ => 0,
    }
}

/// `EpubSpec.isContentDoc`: an EPUB content document is XHTML or SVG.
fn is_content_doc(media: u8) -> bool {
    media == MEDIA_XHTML || media == MEDIA_SVG
}

fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref().as_bytes() == name)
        .map(|a| {
            let raw: &str = a.value.as_ref();
            quick_xml::escape::unescape(raw).map_or_else(|_| raw.to_string(), |s| s.into_owned())
        })
}

// ── internal: href resolution (EPUB OCF, URL Standard, RFC 3986 5.2.4) ───

/// The ZIP entry name that an href names, resolved against the package document's URL `base` (an
/// empty base is the container root): the path part, percent-decoded per segment, with dot
/// segments removed, joined with `/`.
fn resolve_href(base: &[u8], href: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, seg) in resolve_segments(base, href).iter().enumerate() {
        if i > 0 {
            out.push(b'/');
        }
        out.extend_from_slice(seg);
    }
    out
}

/// The segments of [`resolve_href`]: the path part of `href`, resolved against the directory of
/// `base` (or the root for a path-absolute `href`), each segment percent-decoded, dot segments
/// removed.
fn resolve_segments(base: &[u8], href: &[u8]) -> Vec<Vec<u8>> {
    let path = &href[..path_part_len(href)];
    let (mut segs, body): (Vec<Vec<u8>>, &[u8]) = if path.first() == Some(&b'/') {
        (Vec::new(), &path[1..])
    } else {
        let mut dirs = split_segments(base);
        dirs.pop();
        (dirs, path)
    };
    segs.extend(split_segments(body));
    let decoded: Vec<Vec<u8>> = segs.iter().map(|s| pct_decode(s)).collect();
    remove_dots(decoded)
}

/// The length of the path part: everything before the first `#` or `?`.
fn path_part_len(href: &[u8]) -> usize {
    href.iter()
        .position(|&b| kernels::href_byte_class(b) == 3)
        .unwrap_or(href.len())
}

/// Split on `/`, always yielding at least one (possibly empty) segment.
fn split_segments(path: &[u8]) -> Vec<Vec<u8>> {
    let mut segs = Vec::new();
    let mut cur = Vec::new();
    for &b in path {
        if kernels::href_byte_class(b) == 1 {
            segs.push(std::mem::take(&mut cur));
        } else {
            cur.push(b);
        }
    }
    segs.push(cur);
    segs
}

/// URL percent-decoding of one segment: `%` followed by two hex digits is the byte they denote;
/// any other `%` is kept, and the bytes after it are read as they are.
fn pct_decode(seg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(seg.len());
    let mut i = 0;
    while i < seg.len() {
        if seg[i] == b'%' && i + 2 < seg.len() {
            let (hi_ok, hi) = kernels::pct_hex(seg[i + 1]);
            let (lo_ok, lo) = kernels::pct_hex(seg[i + 2]);
            if hi_ok && lo_ok {
                out.push(16 * hi + lo);
                i += 3;
                continue;
            }
        }
        out.push(seg[i]);
        i += 1;
    }
    out
}

/// RFC 3986 5.2.4 on decoded segments: `.` is dropped, `..` removes the segment before it and stays
/// at the root when there is none.
fn remove_dots(segs: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(segs.len());
    for seg in segs {
        let b0 = seg.first().copied().unwrap_or(0);
        let b1 = seg.get(1).copied().unwrap_or(0);
        match kernels::path_segment_kind(seg.len() as u64, b0, b1) {
            1 => {}
            2 => {
                out.pop();
            }
            _ => out.push(seg),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    // Minimal but valid EPUB built by hand: the container, the OPF, and two spine XHTML chapters.
    const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const PACKAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="BookId" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test Book</dc:title>
    <dc:identifier id="BookId">urn:uuid:00000000-0000-0000-0000-000000000000</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#;

    const CH1: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Chapter 1</title></head>
  <body>
    <h1>Chapter 1</h1>
    <p>Hello, world.</p>
  </body>
</html>"#;

    const CH2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Chapter 2</title></head>
  <body>
    <h1>Chapter 2</h1>
    <p>Second chapter.</p>
  </body>
</html>"#;

    /// Build a ZIP from `(name, bytes)` entries, stored uncompressed.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in entries {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(body).unwrap();
            }
            zip.finish().unwrap();
        }
        zip_bytes
    }

    fn build_epub() -> Vec<u8> {
        zip_of(&[
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", PACKAGE.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
            ("OEBPS/ch2.xhtml", CH2.as_bytes()),
        ])
    }

    /// A package with the given manifest and spine bodies, and the chapter files it can read.
    fn epub_with_package(package: &str, chapters: &[&str]) -> Vec<u8> {
        let mut entries: Vec<(String, Vec<u8>)> = vec![
            (
                "META-INF/container.xml".into(),
                CONTAINER.as_bytes().to_vec(),
            ),
            ("OEBPS/content.opf".into(), package.as_bytes().to_vec()),
        ];
        for name in chapters {
            entries.push(((*name).to_string(), CH1.as_bytes().to_vec()));
        }
        let borrowed: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        zip_of(&borrowed)
    }

    fn package_with(manifest: &str, spine: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>{manifest}</manifest>
  <spine>{spine}</spine>
</package>"#
        )
    }

    #[test]
    fn read_parses_manifest_and_spine_in_order() {
        let bytes = build_epub();
        let book = read(&bytes).expect("read should succeed");
        assert_eq!(spine(&book).len(), 2);
        assert_eq!(spine(&book)[0].id, "ch1");
        assert_eq!(spine(&book)[0].href, "OEBPS/ch1.xhtml");
        assert_eq!(spine(&book)[1].id, "ch2");
        assert_eq!(spine(&book)[1].href, "OEBPS/ch2.xhtml");
        assert_eq!(title(&book), Some("Test Book"));
        assert_eq!(opf_path(&book), "OEBPS/content.opf");
        assert!(skipped(&book).is_empty());
    }

    #[test]
    fn manifest_get_is_o1_lookups_by_id() {
        let bytes = build_epub();
        let book = read(&bytes).expect("read should succeed");
        let ch1 = manifest_get(&book, "ch1").expect("ch1 must resolve");
        assert_eq!(ch1.media_type, "application/xhtml+xml");
        assert_eq!(ch1.href, "OEBPS/ch1.xhtml");
        assert!(manifest_get(&book, "missing").is_none());
    }

    #[test]
    fn extract_text_walks_spine_in_order() {
        let bytes = build_epub();
        let book = read(&bytes).expect("read should succeed");
        let text = extract_text(&book, &bytes).expect("extract should succeed");
        let ch1_at = text.find("Chapter 1").expect("ch1 heading present");
        let ch2_at = text.find("Chapter 2").expect("ch2 heading present");
        assert!(ch1_at < ch2_at, "spine order must be ch1 then ch2");
        assert!(text.contains("Hello, world."));
        assert!(text.contains("Second chapter."));
    }

    #[test]
    fn spine_with_broken_idref_is_reported_not_resolved() {
        let bad_package = PACKAGE.replace(
            r#"<itemref idref="ch1"/>
    <itemref idref="ch2"/>"#,
            r#"<itemref idref="ch1"/>
    <itemref idref="ghost"/>"#,
        );
        let bytes = epub_with_package(&bad_package, &["OEBPS/ch1.xhtml"]);
        let book = read(&bytes).expect("read should still succeed");
        assert_eq!(spine(&book).len(), 1, "ghost idref is not read, ch1 kept");
        assert_eq!(spine(&book)[0].id, "ch1");
        assert_eq!(
            skipped(&book),
            &[SpineSkip::Missing {
                idref: "ghost".into()
            }]
        );
    }

    #[test]
    fn duplicated_id_is_reported_and_never_resolved() {
        // Two manifest items share the id "dup": the spine must not read either of them, and the
        // lookup must not return one of them as if it were the unique entry.
        let package = package_with(
            r#"<item id="dup" href="a.xhtml" media-type="application/xhtml+xml"/>
               <item id="dup" href="b.xhtml" media-type="application/xhtml+xml"/>"#,
            r#"<itemref idref="dup"/>"#,
        );
        let bytes = epub_with_package(&package, &["OEBPS/a.xhtml", "OEBPS/b.xhtml"]);
        let book = read(&bytes).expect("read should succeed");
        assert!(spine(&book).is_empty());
        assert_eq!(
            skipped(&book),
            &[SpineSkip::Duplicate {
                idref: "dup".into()
            }]
        );
        assert!(manifest_get(&book, "dup").is_none());
    }

    #[test]
    fn foreign_and_repeated_references_are_reported_and_not_read_again() {
        let package = package_with(
            r#"<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
               <item id="pic" href="pic.png" media-type="image/png"/>"#,
            r#"<itemref idref="ch1"/><itemref idref="pic"/><itemref idref="ch1"/>"#,
        );
        let bytes = epub_with_package(&package, &["OEBPS/ch1.xhtml"]);
        let book = read(&bytes).expect("read should succeed");
        assert_eq!(spine(&book).len(), 1);
        assert_eq!(
            skipped(&book),
            &[
                SpineSkip::Foreign {
                    idref: "pic".into(),
                    href: "OEBPS/pic.png".into()
                },
                SpineSkip::Repeated {
                    idref: "ch1".into(),
                    href: "OEBPS/ch1.xhtml".into()
                },
            ]
        );
        let text = extract_text(&book, &bytes).expect("extract should succeed");
        assert_eq!(text.matches("Hello, world.").count(), 1, "ch1 is read once");
    }

    #[test]
    fn items_outside_the_opf_namespace_are_not_manifest_entries() {
        let package = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata><item id="meta" href="ch1.xhtml" media-type="application/xhtml+xml"/></metadata>
  <manifest>
    <item xmlns="urn:example:other" id="other" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="meta"/><itemref idref="other"/><itemref idref="ch1"/></spine>
</package>"#;
        let bytes = epub_with_package(package, &["OEBPS/ch1.xhtml"]);
        let book = read(&bytes).expect("read should succeed");
        assert!(manifest_get(&book, "meta").is_none());
        assert!(manifest_get(&book, "other").is_none());
        assert!(manifest_get(&book, "ch1").is_some());
        assert_eq!(spine(&book).len(), 1);
        assert_eq!(skipped(&book).len(), 2);
    }

    #[test]
    fn extract_text_refuses_a_missing_spine_entry() {
        // The package is consistent (ch1 in manifest + spine) but the ZIP does not contain it.
        let bytes = zip_of(&[
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", PACKAGE.as_bytes()),
            ("OEBPS/ch2.xhtml", CH2.as_bytes()),
        ]);
        let book = read(&bytes).expect("read should succeed");
        let err = extract_text(&book, &bytes)
            .expect_err("extract must fail when a spine entry is missing");
        assert!(
            err.contains("missing spine entry") && err.contains("OEBPS/ch1.xhtml"),
            "error should name the missing entry: {err}"
        );
    }

    #[test]
    fn zip_entry_bound_refuses_declared_and_read_sizes() {
        let bytes = zip_of(&[("big.txt", b"0123456789abcdefghij".as_slice())]);
        let mut zip = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let ok = read_entry(&mut zip, "big.txt", 20, "entry").expect("within the bound");
        assert_eq!(ok.len(), 20);
        let err = read_entry(&mut zip, "big.txt", 10, "entry")
            .expect_err("a 20-byte entry exceeds a 10-byte bound");
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn join_chapters_counts_the_separator_and_refuses_partial_output() {
        let chapters = vec!["aaaa".to_string(), "bb".to_string()];
        // 4 + 2 (separator) + 2 = 8 bytes in total.
        assert_eq!(join_chapters(&chapters, 8).unwrap(), "aaaa\n\nbb");
        let err = join_chapters(&chapters, 7).expect_err("8 bytes do not fit in 7");
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn container_full_path_is_resolved_and_percent_decoded() {
        let container = CONTAINER.replace("OEBPS/content.opf", "OEBPS/my%20book.opf");
        let package = PACKAGE.to_string();
        let bytes = zip_of(&[
            ("META-INF/container.xml", container.as_bytes()),
            ("OEBPS/my book.opf", package.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
            ("OEBPS/ch2.xhtml", CH2.as_bytes()),
        ]);
        let book = read(&bytes).expect("read should succeed");
        assert_eq!(opf_path(&book), "OEBPS/my book.opf");
        assert_eq!(spine(&book)[0].href, "OEBPS/ch1.xhtml");
    }

    #[test]
    fn resolve_joins_against_the_package_directory() {
        assert_eq!(
            resolve_href(b"OEBPS/content.opf", b"ch1.xhtml"),
            b"OEBPS/ch1.xhtml"
        );
        assert_eq!(resolve_href(b"content.opf", b"ch1.xhtml"), b"ch1.xhtml");
        assert_eq!(resolve_href(b"", b"ch1.xhtml"), b"ch1.xhtml");
        // Backslashes are not separators; they stay in the name.
        assert_eq!(
            resolve_href(b"OEBPS/content.opf", b"a\\b.xhtml"),
            b"OEBPS/a\\b.xhtml"
        );
    }

    #[test]
    fn resolve_path_absolute_hrefs_ignore_the_package_directory() {
        assert_eq!(resolve_href(b"OEBPS/content.opf", b"/a.x"), b"a.x");
        assert_eq!(resolve_href(b"content.opf", b"/a.x"), b"a.x");
        assert_eq!(resolve_href(b"O/P/c.opf", b"/a/b.x"), b"a/b.x");
    }

    #[test]
    fn resolve_removes_dot_segments_and_stops_at_the_root() {
        assert_eq!(resolve_href(b"O/P/c.opf", b"../T/a.x"), b"O/T/a.x");
        assert_eq!(resolve_href(b"c.opf", b"../../a"), b"a");
        assert_eq!(resolve_href(b"O/c.opf", b"./a/./b.x"), b"O/a/b.x");
        // A dot segment written as percent-encoded octets is a dot segment once decoded.
        assert_eq!(resolve_href(b"O/c.opf", b"%2E%2E/a"), b"a");
    }

    #[test]
    fn resolve_decodes_percent_octets_and_drops_query_and_fragment() {
        assert_eq!(resolve_href(b"c", b"a%20b"), b"a b");
        // %2F is an octet inside one segment, not a separator.
        assert_eq!(resolve_segments(b"c", b"a%2Fb"), vec![b"a/b".to_vec()]);
        assert_eq!(resolve_href(b"c", b"a.x#s"), b"a.x");
        assert_eq!(resolve_href(b"c", b"a.x?q"), b"a.x");
        // A `%` that is not followed by two hex digits is kept.
        assert_eq!(resolve_href(b"c", b"a%zz.x"), b"a%zz.x");
    }

    #[test]
    fn pct_decode_matches_the_url_rules() {
        assert_eq!(pct_decode(b"%"), b"%");
        assert_eq!(pct_decode(b"%4"), b"%4");
        assert_eq!(pct_decode(b"%4g"), b"%4g");
        assert_eq!(pct_decode(b"%41"), b"A");
        assert_eq!(pct_decode(b"%%41"), b"%A");
        assert_eq!(pct_decode(b"%41%2"), b"A%2");
    }

    #[test]
    fn spine_entry_outside_the_zip_is_refused_not_panicked() {
        let package = package_with(
            r#"<item id="x" href="../../../escape.xhtml" media-type="application/xhtml+xml"/>"#,
            r#"<itemref idref="x"/>"#,
        );
        let bytes = epub_with_package(&package, &[]);
        let book = read(&bytes).expect("read should succeed");
        // The root stays the root: the entry is `escape.xhtml`, which is not in the archive.
        assert_eq!(spine(&book)[0].href, "escape.xhtml");
        let err = extract_text(&book, &bytes).expect_err("the entry is absent");
        assert!(err.contains("missing spine entry"), "{err}");
    }
}
