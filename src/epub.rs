//! Bounded EPUB reader for the Ptyktos substrate.
//!
//! EPUB is a ZIP container: `META-INF/container.xml` points at an OPF
//! (Open Packaging Format) "package" document, which carries the
//! manifest of all resources and the spine — the canonical reading
//! order — as a list of `idref`s. The spine is what the renderer walks;
//! the table of contents is *navigation metadata attached to that output*,
//! not a list of bodies to extract. This is the same shape as `rbook`'s
//! `package` / `manifest` / `spine` API and what `danigm/epub-rs` does
//! when asked for the chapters.
//!
//! The module is **spine-driven, not TOC-driven**: the manifest is
//! resolved id → href once, the spine is an in-order `Vec<SpineItem>`
//! holding pre-resolved hrefs, and `extract_text` walks the spine
//! one entry at a time. Memory cost is one XHTML chapter plus its
//! markdown render, regardless of how big the book is.
//!
//! ## Why this lives here
//!
//! The dogfood record
//! `bench/dogfood/2026-10-07-voip-regulatory/dogfood-webfold.md` (Gap
//! #1, #2) reported that `legislation.gov.au`'s `*/text` and
//! `/asmade/.../text` URLs return only the EPUB TOC: the body of every
//! section lives in the EPUB-internal `document_N.html` files. The fix
//! is to download the EPUB (the `*.epub` link is on the same landing
//! page) and walk the spine here. Webfold's `--dump text` will route
//! `legislation.gov.au` URLs through this module.
//!
//! ## Why an apeiron-backed manifest
//!
//! The id → item lookup is the hot path on every spine walk. `HashMap`
//! pays the string hash twice (once in `Hash` and once in `Eq`) and
//! owns the keys it stores. `apeiron::Map<String, _>` is the table
//! the rest of the substrate uses (`calamine/` in this workspace for XLSX
//! path/relationship caches, `hashtrinity` for the fact registry), and
//! the same `String: Coord` impl handles the OPF id strings. Manifest
//! resolution is O(1) and the keys stay borrowed between the
//! `read()` and `extract_text()` calls, so the spine walk does not
//! re-allocate them.
//!
//! ## What is NOT here
//!
//! No CSS, no embedded fonts, no images, no scripting, no DRM. The
//! extract is a typed text spine; if a caller needs a chapter's media,
//! they re-open the EPUB with their own reader.
use crate::html::to_markdown;
use apeiron::Map;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// One EPUB chapter or resource, indexed by its package id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestItem {
    /// The id from the OPF `<item id="...">`. Spine idrefs point at this.
    pub id: String,
    /// The href from the OPF, resolved against the OPF directory (so
    /// `OEBPS/chapter1.xhtml`, not just `chapter1.xhtml`).
    pub href: String,
    /// The MIME type from the OPF (`application/xhtml+xml` for chapters,
    /// `image/png` for covers, etc.).
    pub media_type: String,
}

/// One spine entry, in canonical reading order. The href is
/// pre-resolved against the OPF directory at `read()` time so the
/// `extract_text` walk is just `Vec::iter` + `zip.by_name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpineItem {
    pub id: String,
    pub href: String,
}

/// A parsed EPUB package: the manifest index (id → item) and the
/// spine in reading order. Holds no file bytes; the original
/// container is passed back to `extract_text`.
pub struct Book {
    manifest: Map<String, ManifestItem>,
    spine: Vec<SpineItem>,
    /// Absolute path of the OPF inside the ZIP (e.g. `OEBPS/content.opf`).
    /// Recorded for callers that want it; not consumed by the in-tree
    /// extract path so silence the dead-code lint until a caller uses it.
    #[allow(dead_code)]
    opf_path: String,
    /// Title from the OPF `<metadata><dc:title>`, if present.
    title: Option<String>,
}

const MAX_ENTRY: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;
const MAX_XHTML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

fn text(raw: &str) -> String {
    quick_xml::escape::unescape(raw).map_or_else(|_| raw.to_string(), |s| s.into_owned())
}

fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref().as_bytes() == name)
        .map(|a| text(a.value.as_ref()))
}

fn read_bounded_entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    let file = zip
        .by_name(name)
        .map_err(|e| format!("missing entry {name:?}: {e}"))?;
    if file.size() > MAX_ENTRY {
        return Err(format!("entry {name:?} too large: {} bytes", file.size()));
    }
    let mut out = Vec::with_capacity(file.size() as usize);
    file.take(MAX_ENTRY + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("read error for {name:?}: {e}"))?;
    Ok(out)
}

/// Read the EPUB's container, parse the package document, and return
/// the manifest index + spine. Bytes are not copied; the caller
/// keeps the `&[u8]` alive for the subsequent `extract_text` call.
pub fn read(bytes: &[u8]) -> Result<Book, String> {
    if bytes.len() > MAX_TOTAL as usize {
        return Err(format!("EPUB too large: {} bytes", bytes.len()));
    }
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("invalid EPUB ZIP: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("too many entries: {}", zip.len()));
    }

    // 1. Read META-INF/container.xml to find the OPF path.
    let container_bytes = read_bounded_entry(&mut zip, "META-INF/container.xml")?;
    let opf_path = parse_container(&container_bytes)?;
    let opf_dir = opf_dirname(&opf_path);

    // 2. Read the OPF (package document). Build manifest + spine.
    let opf_bytes = read_bounded_entry(&mut zip, &opf_path)?;
    let (manifest, spine_ids, title) = parse_package(&opf_bytes)?;

    // 3. Pre-resolve spine hrefs against the OPF directory. One O(1)
    //    apeiron::Map lookup per idref.
    let mut spine = Vec::with_capacity(spine_ids.len());
    for id in spine_ids {
        match manifest.get_at(id.as_str()) {
            Some(item) => {
                let href = join_href(&opf_dir, &item.href);
                spine.push(SpineItem {
                    id: id.clone(),
                    href,
                });
            }
            None => {
                // Skip a broken idref rather than fail the whole book;
                // log it on stderr so a caller that wants strict mode
                // can grep for it.
                eprintln!("epub: spine idref {id:?} not in manifest; skipping");
            }
        }
    }

    Ok(Book {
        manifest,
        spine,
        opf_path,
        title,
    })
}

/// Walk the book's spine, one chapter at a time, and concatenate each
/// chapter's body as Markdown. Returns an error if any chapter is
/// malformed; partial output is NOT returned on error, so the caller
/// does not have to reason about a half-rendered book.
///
/// Bounded by `MAX_TEXT_BYTES` (64 MiB) of total rendered text. A book
/// whose spine produces more than that is refused, with a clear error.
pub fn extract_text(book: &Book, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_TOTAL as usize {
        return Err(format!("EPUB too large: {} bytes", bytes.len()));
    }
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("invalid EPUB ZIP: {e}"))?;
    let mut out = String::new();
    for (i, item) in book.spine.iter().enumerate() {
        let entry = read_bounded_xhtml(&mut zip, &item.href)?;
        // base_url is local because the chapter is being read for its
        // own text; relative links do not survive extraction here.
        let md = to_markdown(&entry, "");
        if out.len() + md.len() > MAX_TEXT_BYTES as usize {
            return Err(format!(
                "spine text exceeds {} bytes at chapter {} ({:?})",
                MAX_TEXT_BYTES, i, item.href
            ));
        }
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&md);
    }
    Ok(out)
}

/// Spine in reading order.
pub fn spine(book: &Book) -> &[SpineItem] {
    &book.spine
}

/// Title from the OPF metadata, if the package document declared one.
pub fn title(book: &Book) -> Option<&str> {
    book.title.as_deref()
}

/// O(1) manifest lookup by package id. Returns `None` for an unknown
/// id (the spine walk already skipped those, but a caller iterating
/// the manifest by id may need the same lookup).
pub fn manifest_get<'a>(book: &'a Book, id: &str) -> Option<&'a ManifestItem> {
    book.manifest.get_at(id)
}

// ── internal: container + package parsing ──────────────────────────────

fn read_bounded_xhtml(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    let file = zip
        .by_name(name)
        .map_err(|e| format!("missing spine entry {name:?}: {e}"))?;
    if file.size() > MAX_XHTML_BYTES {
        return Err(format!(
            "spine entry {name:?} too large: {} bytes",
            file.size()
        ));
    }
    let mut out = Vec::with_capacity(file.size() as usize);
    file.take(MAX_XHTML_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("read error for {name:?}: {e}"))?;
    Ok(out)
}

fn parse_container(bytes: &[u8]) -> Result<String, String> {
    let s = std::str::from_utf8(bytes).map_err(|e| format!("container.xml is not UTF-8: {e}"))?;
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::with_capacity(256);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("container.xml parse error: {e}"))?
        {
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

/// A parsed OPF package: the manifest by item id, the spine's idrefs in order, and the package
/// title when the OPF declares one.
type Package = (Map<String, ManifestItem>, Vec<String>, Option<String>);

fn parse_package(bytes: &[u8]) -> Result<Package, String> {
    let s = std::str::from_utf8(bytes).map_err(|e| format!("OPF is not UTF-8: {e}"))?;
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut manifest = Map::<String, ManifestItem>::new();
    let mut spine_ids: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    let mut buf = Vec::with_capacity(1024);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("OPF parse error: {e}"))?
        {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref().as_bytes() == b"item" => {
                let id = attr(&e, b"id");
                let href = attr(&e, b"href");
                let media_type = attr(&e, b"media-type");
                if let (Some(id), Some(href)) = (id, href) {
                    manifest.insert(
                        id.clone(),
                        ManifestItem {
                            id,
                            href,
                            media_type: media_type.unwrap_or_default(),
                        },
                    );
                }
            }
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref().as_bytes() == b"itemref" =>
            {
                if let Some(idref) = attr(&e, b"idref") {
                    spine_ids.push(idref);
                }
            }
            Event::Start(e) if e.local_name().as_ref().as_bytes() == b"title" => {
                // The text is the next Text or CData event.
                let inner = reader
                    .read_text(e.name())
                    .map_err(|err| format!("OPF title read: {err}"))?;
                title = Some(inner.into_owned().to_string());
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok((manifest, spine_ids, title))
}

/// `<dir>` of an OPF path inside the ZIP. Pure string ops, no Path —
/// EPUBs are POSIX-style regardless of host.
fn opf_dirname(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => format!("{dir}/"),
        _ => String::new(),
    }
}

/// Resolve a manifest href (relative to the OPF) against the OPF
/// directory. Treats the OPF href as a POSIX path; an EPUB that ships
/// backslashes is malformed and gets the un-joined href returned.
fn join_href(opf_dir: &str, href: &str) -> String {
    if href.contains('\\') || opf_dir.is_empty() {
        return href.to_string();
    }
    // If href is already absolute (starts with '/'), keep as-is.
    if let Some(stripped) = href.strip_prefix('/') {
        return stripped.to_string();
    }
    format!("{opf_dir}{href}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    // Minimal but valid EPUB built by hand. The container, the OPF, and
    // two spine XHTML chapters. Enough to exercise the spine walk end to
    // end without depending on a real-world fixture.
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

    fn build_epub() -> Vec<u8> {
        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(CONTAINER.as_bytes()).unwrap();
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(PACKAGE.as_bytes()).unwrap();
            zip.start_file("OEBPS/ch1.xhtml", opts).unwrap();
            zip.write_all(CH1.as_bytes()).unwrap();
            zip.start_file("OEBPS/ch2.xhtml", opts).unwrap();
            zip.write_all(CH2.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        zip_bytes
    }

    #[test]
    fn read_parses_manifest_and_spine_in_order() {
        let bytes = build_epub();
        let book = read(&bytes).expect("read should succeed");
        assert_eq!(book.spine.len(), 2);
        assert_eq!(book.spine[0].id, "ch1");
        assert_eq!(book.spine[0].href, "OEBPS/ch1.xhtml");
        assert_eq!(book.spine[1].id, "ch2");
        assert_eq!(book.spine[1].href, "OEBPS/ch2.xhtml");
        assert_eq!(book.title.as_deref(), Some("Test Book"));
    }

    #[test]
    fn manifest_get_is_o1_lookups_by_id() {
        let bytes = build_epub();
        let book = read(&bytes).expect("read should succeed");
        let ch1 = manifest_get(&book, "ch1").expect("ch1 must resolve");
        assert_eq!(ch1.media_type, "application/xhtml+xml");
        assert!(manifest_get(&book, "missing").is_none());
    }

    #[test]
    fn extract_text_walks_spine_in_order() {
        let bytes = build_epub();
        let book = read(&bytes).expect("read should succeed");
        let text = extract_text(&book, &bytes).expect("extract should succeed");
        // Spine order: ch1 first, then ch2. The headings must appear in
        // that order, not in container or alphabetic order.
        let ch1_at = text.find("Chapter 1").expect("ch1 heading present");
        let ch2_at = text.find("Chapter 2").expect("ch2 heading present");
        assert!(ch1_at < ch2_at, "spine order must be ch1 then ch2");
        assert!(text.contains("Hello, world."));
        assert!(text.contains("Second chapter."));
    }

    // ── negative control ────────────────────────────────────────────────────
    // Deleting the spine walk OR the manifest resolution must turn one
    // of these red. The whole point of pinning both paths is so a
    // regression in the spine walk cannot silently re-emit a body that
    // does not exist in the manifest, and a regression in the manifest
    // resolution cannot silently drop a body the spine asked for.

    #[test]
    fn spine_with_broken_idref_is_skipped_not_panicked() {
        // A package whose spine lists an id that isn't in the manifest.
        // Reading must succeed; the broken idref is dropped, with a
        // stderr line that greppable callers can catch.
        let bad_package = PACKAGE.replace(
            r#"<itemref idref="ch1"/>
    <itemref idref="ch2"/>"#,
            r#"<itemref idref="ch1"/>
    <itemref idref="ghost"/>"#,
        );
        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(CONTAINER.as_bytes()).unwrap();
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(bad_package.as_bytes()).unwrap();
            zip.start_file("OEBPS/ch1.xhtml", opts).unwrap();
            zip.write_all(CH1.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let book = read(&zip_bytes).expect("read should still succeed");
        assert_eq!(book.spine.len(), 1, "ghost idref is dropped, ch1 kept");
        assert_eq!(book.spine[0].id, "ch1");
    }

    #[test]
    fn extract_text_refuses_a_missing_spine_entry() {
        // The package is consistent (ch1 in manifest + spine) but the
        // ZIP does not contain ch1.xhtml. extract_text must surface
        // that as a clean error, not panic.
        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(CONTAINER.as_bytes()).unwrap();
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(PACKAGE.as_bytes()).unwrap();
            // ch1.xhtml intentionally NOT written.
            zip.start_file("OEBPS/ch2.xhtml", opts).unwrap();
            zip.write_all(CH2.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let book = read(&zip_bytes).expect("read should succeed");
        let err = extract_text(&book, &zip_bytes)
            .expect_err("extract must fail when a spine entry is missing");
        assert!(
            err.contains("missing spine entry") && err.contains("OEBPS/ch1.xhtml"),
            "error should name the missing entry: {err}"
        );
    }

    #[test]
    fn opf_dirname_strips_the_filename_only() {
        assert_eq!(opf_dirname("OEBPS/content.opf"), "OEBPS/");
        assert_eq!(opf_dirname("content.opf"), "");
        assert_eq!(opf_dirname("a/b/c.opf"), "a/b/");
    }

    #[test]
    fn join_href_resolves_against_opf_dir() {
        assert_eq!(join_href("OEBPS/", "ch1.xhtml"), "OEBPS/ch1.xhtml");
        assert_eq!(join_href("", "ch1.xhtml"), "ch1.xhtml");
        assert_eq!(join_href("OEBPS/", "/abs.xhtml"), "abs.xhtml");
        // Backslashes are not POSIX; we pass them through unchanged.
        assert_eq!(join_href("OEBPS/", "a\\b.xhtml"), "a\\b.xhtml");
    }
}
