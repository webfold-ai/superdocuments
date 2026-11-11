//! Embedded pictures of the document formats: the bytes of each image in a package, with its media
//! type, its pixel size where the header states it, the part that places it first, and its alt
//! text. The DOCX and PPTX Markdown writers name the same files (`![alt](media/<name>)`), so the
//! file name is the last segment of `ExtractedImage::name`.
//!
//! Sources: OOXML relationships (`word/media`, `ppt/media`, `xl/media`, followed through each part's
//! `.rels` and its `blip` / `imagedata` elements), ODF `draw:image` (`Pictures/`), and EPUB manifest
//! items with an `image/` media type. Entries are read through a size cap that counts the bytes
//! actually decompressed, not only the declared size.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use crate::document::{entry, push_ref, text};
use crate::ooxml::{attr_local, read_rels, resolve, Rel};

/// An image stored in a document package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedImage {
    /// The path inside the archive, such as `word/media/image1.png` or `Pictures/10000201.png`.
    pub name: String,
    /// The media type named by the leading bytes: `image/png`, `image/jpeg`, `image/gif`,
    /// `image/webp`, `image/bmp`, `image/tiff`, `image/emf`, `image/wmf` or `image/svg+xml`.
    pub mime: String,
    pub bytes: Vec<u8>,
    /// Pixel size from the header, for PNG, JPEG, GIF and BMP. `None` for other formats, or when
    /// the header is damaged.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The part that places the image first: `word/document.xml`, `ppt/slides/slide3.xml`,
    /// `content.xml`, or the XHTML document of an EPUB. `None` when nothing in the package refers
    /// to it.
    pub referenced_from: Option<String>,
    /// The alt text at that first reference: `descr` in DOCX and PPTX, the `svg:desc` of the
    /// frame in ODF, the `alt` of the `img` in EPUB. `None` when empty or absent.
    pub alt: Option<String>,
}

/// Limits on what is read. Errors are returned, never truncated results.
#[derive(Clone, Copy)]
struct Limits {
    per_image: u64,
    total: u64,
    count: usize,
}

const LIMITS: Limits = Limits {
    per_image: 64 << 20,
    total: 512 << 20,
    count: 10_000,
};

/// Zip central-directory entries accepted before the archive is refused.
const MAX_ZIP_ENTRIES: usize = 100_000;

/// A picture found in the package, before its bytes are read.
struct Found {
    path: String,
    referenced_from: Option<String>,
    alt: Option<String>,
}

/// The pictures of a DOCX, DOCM, PPTX, XLSX, XLSM, ODT, ODP, ODS or EPUB file, in reading order:
/// first the pictures each part places, in the order it places them, then the pictures nothing
/// refers to, by name.
///
/// An entry whose name is absolute, contains a backslash, has `..` in it or a drive-like first
/// segment makes the whole call an error. A picture over 64 MiB, or pictures over 512 MiB in total
/// or more than 10,000 of them, are errors too. Files that are not images (by their leading bytes)
/// are left out. Other extensions are an error.
pub fn extract_images(data: &[u8], ext: &str) -> Result<Vec<ExtractedImage>, String> {
    extract_with_limits(data, ext, LIMITS)
}

fn extract_with_limits(
    data: &[u8],
    ext: &str,
    limits: Limits,
) -> Result<Vec<ExtractedImage>, String> {
    let format = ext.to_ascii_lowercase();
    if !matches!(
        format.as_str(),
        "docx" | "docm" | "pptx" | "xlsx" | "xlsm" | "odt" | "odp" | "ods" | "epub"
    ) {
        return Err(format!(
            "image extraction is not supported for .{format} files"
        ));
    }
    let mut zip = ZipArchive::new(Cursor::new(data)).map_err(|e| format!("invalid ZIP: {e}"))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err("too many archive entries".into());
    }
    let mut names = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i).map_err(|e| e.to_string())?;
        let name = file.name().map_err(|e| e.to_string())?.to_string();
        if !safe_name(&name) {
            return Err(format!("unsafe entry name in archive: {name:?}"));
        }
        names.push(name);
    }

    let found = match format.as_str() {
        "epub" => epub_candidates(&mut zip)?,
        "odt" | "odp" | "ods" => odf_candidates(&mut zip, &names)?,
        _ => ooxml_candidates(&mut zip, &names, &format)?,
    };
    if found.len() > limits.count {
        return Err(format!("more than {} images", limits.count));
    }

    let mut images = Vec::new();
    let mut used = 0u64;
    let mut seen = BTreeSet::new();
    for item in found {
        if !seen.insert(item.path.clone()) {
            continue;
        }
        let Some(bytes) = read_image(&mut zip, &item.path, limits, &mut used)? else {
            continue;
        };
        let Some(sniffed) = sniff(&bytes) else {
            continue;
        };
        images.push(ExtractedImage {
            name: item.path,
            mime: sniffed.mime.to_owned(),
            bytes,
            width: sniffed.width,
            height: sniffed.height,
            referenced_from: item.referenced_from,
            alt: item.alt,
        });
    }
    Ok(images)
}

/// Entry names that stay inside the package: relative, with no `..`, no backslash and no drive or
/// scheme in the first segment.
fn safe_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        return false;
    }
    if name.split('/').any(|segment| segment == "..") {
        return false;
    }
    name.split('/')
        .next()
        .is_some_and(|first| !first.contains(':'))
}

/// `word/_rels/document.xml.rels` belongs to `word/document.xml`. The package's own `_rels/.rels`
/// belongs to no part.
fn part_of_rels(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".rels")?;
    let (dir, file) = stem.rsplit_once("/_rels/")?;
    Some(format!("{dir}/{file}"))
}

fn is_media_entry(name: &str) -> bool {
    !name.ends_with('/') && (name.contains("/media/") || name.starts_with("media/"))
}

fn ooxml_candidates(
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    names: &[String],
    format: &str,
) -> Result<Vec<Found>, String> {
    let mut by_part: BTreeMap<String, Vec<Rel>> = BTreeMap::new();
    for name in names {
        if let Some(part) = part_of_rels(name) {
            by_part.insert(part, read_rels(zip, name)?);
        }
    }

    // Parts that place pictures in reading order come first: the main document, then the slides.
    let mut order: Vec<String> = Vec::new();
    match format {
        "docx" | "docm" => {
            if let Ok(main) = crate::ooxml::main_part(zip, "word/document.xml") {
                order.push(main);
            }
        }
        "pptx" => order.extend(crate::ooxml::pptx::slide_order(zip).unwrap_or_default()),
        _ => {}
    }
    for part in by_part.keys() {
        if !order.contains(part) {
            order.push(part.clone());
        }
    }

    let mut found = Vec::new();
    for part in &order {
        let Some(rels) = by_part.get(part) else {
            continue;
        };
        let targets = crate::ooxml::image_refs(rels, part);
        if targets.is_empty() || zip.by_name(part).is_err() {
            continue;
        }
        let xml = entry(zip, part)?;
        for (id, alt) in blip_refs(&xml)? {
            if let Some(path) = targets.get(&id) {
                found.push(Found {
                    path: path.clone(),
                    referenced_from: Some(part.clone()),
                    alt,
                });
            }
        }
    }

    let mut unreferenced: Vec<&String> = names.iter().filter(|n| is_media_entry(n)).collect();
    unreferenced.sort();
    for name in unreferenced {
        found.push(Found {
            path: name.clone(),
            referenced_from: None,
            alt: None,
        });
    }
    Ok(found)
}

/// The picture references of one OOXML part, in document order: `(relationship id, alt text)`.
/// `blip` (DrawingML) and `imagedata` (VML) are the picture elements; their alt text is the
/// `descr` of the nearest `docPr` or `cNvPr` before them, or the VML `title`. Content inside
/// `mc:Fallback` is skipped, since it repeats the `mc:Choice` picture.
fn blip_refs(xml: &[u8]) -> Result<Vec<(String, Option<String>)>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut alt: Option<String> = None;
    let mut fallback = 0usize;
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) | Event::Empty(tag) => {
                let local = tag.local_name();
                match local.as_ref() {
                    "Fallback" => fallback += 1,
                    _ if fallback > 0 => {}
                    "docPr" | "cNvPr" => alt = non_empty(attr_local(&tag, "descr")),
                    "blip" => {
                        if let Some(id) = attr_local(&tag, "embed") {
                            out.push((id, alt.clone()));
                        }
                    }
                    "imagedata" => {
                        if let Some(id) = attr_local(&tag, "id") {
                            out.push((id, non_empty(attr_local(&tag, "title")).or(alt.clone())));
                        }
                    }
                    _ => {}
                }
            }
            Event::End(tag) if tag.local_name().as_ref() == "Fallback" => {
                fallback = fallback.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn odf_candidates(
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    names: &[String],
) -> Result<Vec<Found>, String> {
    let mut found = Vec::new();
    for part in ["content.xml", "styles.xml"] {
        if zip.by_name(part).is_err() {
            continue;
        }
        let xml = entry(zip, part)?;
        for (path, alt) in odf_image_refs(&xml, part) {
            found.push(Found {
                path,
                referenced_from: Some(part.to_owned()),
                alt,
            });
        }
    }
    let mut pictures: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with("Pictures/") && !n.ends_with('/'))
        .collect();
    pictures.sort();
    for name in pictures {
        found.push(Found {
            path: name.clone(),
            referenced_from: None,
            alt: None,
        });
    }
    Ok(found)
}

/// `draw:image` references of an ODF part as `(package path, alt)`. Inside a `draw:frame` the alt
/// text is its `svg:desc`, which can come before or after the image.
fn odf_image_refs(xml: &[u8], part: &str) -> Vec<(String, Option<String>)> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let (mut in_frame, mut in_desc) = (false, false);
    let mut frame_images: Vec<String> = Vec::new();
    let mut frame_desc: Option<String> = None;
    let mut desc_text = String::new();
    // A damaged part keeps what was read before the error.
    while let Ok(event) = reader.read_event_into(&mut buf) {
        match event {
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                "frame" => {
                    in_frame = true;
                    frame_images.clear();
                    frame_desc = None;
                }
                "desc" if in_frame => {
                    in_desc = true;
                    desc_text.clear();
                }
                "image" => {
                    if let Some(path) = attr_local(&tag, "href").and_then(|h| odf_path(part, &h)) {
                        if in_frame {
                            frame_images.push(path);
                        } else {
                            out.push((path, None));
                        }
                    }
                }
                _ => {}
            },
            Event::Text(value) if in_desc => desc_text.push_str(&text(value.as_ref())),
            Event::GeneralRef(r) if in_desc => push_ref(&mut desc_text, &r),
            Event::End(tag) => match tag.local_name().as_ref() {
                "desc" if in_desc => {
                    in_desc = false;
                    frame_desc = non_empty(Some(std::mem::take(&mut desc_text)));
                }
                "frame" => {
                    in_frame = false;
                    for path in frame_images.drain(..) {
                        out.push((path, frame_desc.clone()));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// The package path of an ODF `href`, or `None` for an external URL or a path that leaves the
/// package.
fn odf_path(part: &str, href: &str) -> Option<String> {
    if href.contains("://") || href.starts_with('#') {
        return None;
    }
    resolve(part, href)
}

fn epub_candidates(zip: &mut ZipArchive<Cursor<&[u8]>>) -> Result<Vec<Found>, String> {
    let container = entry(zip, "META-INF/container.xml")?;
    let opf = crate::epub::parse_container(&container)?;
    let opf_xml = entry(zip, &opf)?;

    let mut images: Vec<String> = Vec::new();
    let mut documents: Vec<String> = Vec::new();
    for (href, media_type) in manifest_items(&opf_xml)? {
        let Some(path) = resolve(&opf, &href) else {
            continue;
        };
        if media_type.starts_with("image/") {
            images.push(path);
        } else if media_type == "application/xhtml+xml" {
            documents.push(path);
        }
    }

    let mut found = Vec::new();
    for document in documents {
        if zip.by_name(&document).is_err() {
            continue;
        }
        let xml = entry(zip, &document)?;
        for (src, alt) in html_image_refs(&xml) {
            let Some(path) = resolve(&document, &src) else {
                continue;
            };
            if images.contains(&path) {
                found.push(Found {
                    path,
                    referenced_from: Some(document.clone()),
                    alt,
                });
            }
        }
    }
    for path in images {
        found.push(Found {
            path,
            referenced_from: None,
            alt: None,
        });
    }
    Ok(found)
}

/// `(href, media-type)` of each `item` in an OPF manifest.
fn manifest_items(opf: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut reader = Reader::from_reader(opf);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) | Event::Empty(tag) if tag.local_name().as_ref() == "item" => {
                if let (Some(href), Some(media_type)) =
                    (attr_local(&tag, "href"), attr_local(&tag, "media-type"))
                {
                    out.push((href, media_type));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// The image references of an XHTML document as `(src, alt)`: `img` elements and SVG `image`
/// elements.
fn html_image_refs(xml: &[u8]) -> Vec<(String, Option<String>)> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    while let Ok(event) = reader.read_event_into(&mut buf) {
        match event {
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                "img" => {
                    if let Some(src) = attr_local(&tag, "src") {
                        out.push((src, non_empty(attr_local(&tag, "alt"))));
                    }
                }
                "image" => {
                    if let Some(href) = attr_local(&tag, "href") {
                        out.push((href, None));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Reads one picture. `Ok(None)` when the archive has no such entry. Size is checked against the
/// declared size first, then against the bytes actually decompressed, so a small declared size
/// cannot hide a larger payload.
fn read_image(
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    path: &str,
    limits: Limits,
    used: &mut u64,
) -> Result<Option<Vec<u8>>, String> {
    let Ok(file) = zip.by_name(path) else {
        return Ok(None);
    };
    let declared = file.size();
    if declared > limits.per_image {
        return Err(format!("image too large: {path}"));
    }
    if used.saturating_add(declared) > limits.total {
        return Err("images exceed the total size limit".into());
    }
    let mut data = Vec::with_capacity(declared as usize);
    file.take(declared + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 != declared {
        return Err(format!(
            "image size does not match its archive header: {path}"
        ));
    }
    *used += declared;
    Ok(Some(data))
}

/// A picture's media type and, where cheap, its pixel size.
struct Sniffed {
    mime: &'static str,
    width: Option<u32>,
    height: Option<u32>,
}

/// The media type of a picture from its leading bytes, `None` if it is not one of the known
/// image formats. Sizes are read from the PNG, JPEG, GIF and BMP headers only.
fn sniff(bytes: &[u8]) -> Option<Sniffed> {
    let (mime, dims) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", be32(bytes, 16).zip(be32(bytes, 20)))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("image/jpeg", jpeg_size(bytes))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        (
            "image/gif",
            le16(bytes, 6)
                .zip(le16(bytes, 8))
                .map(|(w, h)| (u32::from(w), u32::from(h))),
        )
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        ("image/webp", None)
    } else if bytes.starts_with(b"BM") && bytes.get(6..10) == Some(&[0, 0, 0, 0]) {
        let width = le32(bytes, 18).map(|w| (w as i32).unsigned_abs());
        let height = le32(bytes, 22).map(|h| (h as i32).unsigned_abs());
        ("image/bmp", width.zip(height))
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        ("image/tiff", None)
    } else if bytes.starts_with(&[1, 0, 0, 0]) && bytes.get(40..44) == Some(b" EMF") {
        ("image/emf", None)
    } else if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        ("image/wmf", None)
    } else if is_svg(bytes) {
        ("image/svg+xml", None)
    } else {
        return None;
    };
    let dims = dims.filter(|(w, h)| *w > 0 && *h > 0);
    Some(Sniffed {
        mime,
        width: dims.map(|d| d.0),
        height: dims.map(|d| d.1),
    })
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let head = String::from_utf8_lossy(head).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    (head.starts_with("<svg") || head.starts_with("<?xml") || head.starts_with("<!doctype svg"))
        && head.contains("<svg")
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Width and height from the first start-of-frame marker of a JPEG. Walks the marker segments
/// without decoding any entropy-coded data.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2usize;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = be16(b, i + 5)?;
            let width = be16(b, i + 7)?;
            return Some((u32::from(width), u32::from(height)));
        }
        if matches!(marker, 0x01 | 0xD0..=0xD9) {
            i += 2;
            continue;
        }
        let length = be16(b, i + 2)? as usize;
        if length < 2 {
            return None;
        }
        i += 2 + length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn png_header_gives_mime_and_size() {
        let s = sniff(PNG_1X1).expect("png");
        assert_eq!((s.mime, s.width, s.height), ("image/png", Some(1), Some(1)));
    }

    #[test]
    fn truncated_headers_give_no_size_but_still_a_type() {
        let s = sniff(&PNG_1X1[..20]).expect("png prefix");
        assert_eq!(s.mime, "image/png");
        assert_eq!((s.width, s.height), (None, None));
        assert!(sniff(&PNG_1X1[..7]).is_none());
    }

    #[test]
    fn jpeg_gif_and_svg_are_recognised() {
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00,
            0x20, 0x00, 0x10,
        ];
        let s = sniff(&jpeg).expect("jpeg");
        assert_eq!(
            (s.mime, s.width, s.height),
            ("image/jpeg", Some(16), Some(32))
        );
        let gif = b"GIF89a\x03\x00\x02\x00";
        let s = sniff(gif).expect("gif");
        assert_eq!((s.mime, s.width, s.height), ("image/gif", Some(3), Some(2)));
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert_eq!(sniff(svg).expect("svg").mime, "image/svg+xml");
        assert!(sniff(b"<html><body>not svg</body></html>").is_none());
    }

    #[test]
    fn unsafe_entry_names_are_refused() {
        assert!(!safe_name("../evil.png"));
        assert!(!safe_name("word/../../evil.png"));
        assert!(!safe_name("/word/media/a.png"));
        assert!(!safe_name("word\\media\\a.png"));
        assert!(!safe_name("C:/a.png"));
        assert!(safe_name("word/media/image1.png"));
    }

    #[test]
    fn a_picture_over_the_per_image_limit_is_an_error() {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            writer
                .start_file(
                    "word/media/image1.png",
                    zip::write::SimpleFileOptions::default(),
                )
                .expect("start");
            std::io::Write::write_all(&mut writer, PNG_1X1).expect("write");
            writer.finish().expect("finish");
        }
        let small = Limits {
            per_image: 10,
            ..LIMITS
        };
        assert!(extract_with_limits(&buf, "docx", small).is_err());
        let images = extract_with_limits(&buf, "docx", LIMITS).expect("default limits");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "word/media/image1.png");
    }
}
