//! Embedded image extraction from the in-tree PDF parser.
//!
//! `extract_images` walks the page tree (with inherited `/Resources`), follows `/XObject`
//! entries, and descends into form XObjects. Each image occurrence is reported once per page,
//! so an image used on two pages appears twice. For each image it gives the page index (from 0),
//! the object number, size, colour space, bits per component, the filter names, and the data:
//!
//! - DCTDecode (JPEG): the JPEG bytes, unchanged. `pdfimages -j` writes the same bytes.
//! - Flate or unfiltered 8-bit DeviceGray or DeviceRGB: a PNG, written with stored (uncompressed)
//!   deflate blocks. The PNG is valid and lossless, but not compressed.
//! - Everything else that the parser can decode (other bit depths, Indexed, CMYK, ICC-based,
//!   ASCII and predictor filters): the decoded samples, as they are, with the colour space
//!   stated so that the caller can convert them.
//! - JPXDecode: the JPEG 2000 codestream, unchanged.
//! - Filters the parser cannot decode: `Unsupported`, with an empty data buffer.
//!
//! Page rasterisation is not here. Nothing in this module draws a page.

use crate::pdf::parser::{Dict, Document, Object};

/// The most pages and XObjects walked. A larger file is cut short with the count reached.
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 32;

/// An embedded image occurrence on one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    /// Zero-based page index in the page tree's order.
    pub page: usize,
    /// The image XObject's object number.
    pub object: u32,
    pub width: u32,
    pub height: u32,
    /// `DeviceGray`, `DeviceRGB`, `DeviceCMYK`, `Indexed`, `ICCBased(n)`, or the name as written.
    pub color_space: String,
    pub bits_per_component: u32,
    /// The `/Filter` names, in order.
    pub filters: Vec<String>,
    pub encoding: ImageEncoding,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageEncoding {
    Jpeg,
    Png,
    /// Decoded samples; see `ImageInfo::color_space` and `bits_per_component`.
    RawSamples,
    Jpx,
    Unsupported(String),
}

/// Lists the embedded image XObjects of a PDF, page by page.
pub fn extract_images(bytes: &[u8]) -> Result<Vec<ImageInfo>, String> {
    let doc = Document::parse(bytes).map_err(|e| format!("{e:?}"))?;
    let mut pages: Vec<Option<Dict>> = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut budget = MAX_NODES;
    let root = match doc.object(doc.root()).and_then(|o| dict_of(&doc, o)) {
        Some(root) => root.clone(),
        None => return Err("the catalog is missing".into()),
    };
    if let Some(pages_ref) = root.get(&"Pages".to_string()) {
        collect_pages(
            &doc,
            pages_ref,
            None,
            0,
            &mut visited,
            &mut budget,
            &mut pages,
        );
    }
    let mut out = Vec::new();
    let mut seen_forms = std::collections::HashSet::new();
    for (index, resources) in pages.iter().enumerate() {
        if let Some(res) = resources {
            walk_resources(&doc, res, index, 0, &mut seen_forms, &mut budget, &mut out);
        }
    }
    Ok(out)
}

fn dict_of<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dict> {
    match doc.resolve(o)? {
        Object::Dict(d) | Object::Stream(d, _) => Some(d),
        _ => None,
    }
}

fn get<'a>(doc: &'a Document, d: &'a Dict, key: &str) -> Option<&'a Object> {
    doc.resolve(d.get(&key.to_string())?)
}

fn int(doc: &Document, d: &Dict, key: &str) -> Option<i64> {
    match get(doc, d, key)? {
        Object::Int(n) => Some(*n),
        Object::Real(r) if r.fract() == 0.0 => Some(*r as i64),
        _ => None,
    }
}

/// Collects the page dictionaries' resources, in page-tree order. A page without its own
/// `/Resources` uses the nearest ancestor's.
fn collect_pages(
    doc: &Document,
    node: &Object,
    inherited: Option<Dict>,
    depth: usize,
    visited: &mut std::collections::HashSet<u32>,
    budget: &mut usize,
    out: &mut Vec<Option<Dict>>,
) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    *budget -= 1;
    if let Object::Ref(n, _) = node {
        if !visited.insert(*n) {
            return;
        }
    }
    let Some(d) = dict_of(doc, node) else {
        return;
    };
    let own = match get(doc, d, "Resources") {
        Some(Object::Dict(r)) => Some(r.clone()),
        _ => inherited.clone(),
    };
    let is_page = matches!(get(doc, d, "Type"), Some(Object::Name(t)) if t == "Page");
    if is_page {
        out.push(own);
        return;
    }
    if let Some(Object::Array(kids)) = get(doc, d, "Kids") {
        for kid in kids {
            collect_pages(doc, kid, own.clone(), depth + 1, visited, budget, out);
        }
    }
}

fn walk_resources(
    doc: &Document,
    res: &Dict,
    page: usize,
    depth: usize,
    seen_forms: &mut std::collections::HashSet<(usize, u32)>,
    budget: &mut usize,
    out: &mut Vec<ImageInfo>,
) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    let Some(xobjects) = get(doc, res, "XObject").and_then(|o| dict_of(doc, o)) else {
        return;
    };
    // Names are sorted so that the output does not depend on the hash order.
    let mut names: Vec<(&String, &Object)> = xobjects.iter().collect();
    names.sort_by(|a, b| a.0.cmp(b.0));
    for (_, value) in names {
        *budget = budget.saturating_sub(1);
        let number = match value {
            Object::Ref(n, _) => *n,
            _ => continue,
        };
        let Some(Object::Stream(dict, raw)) = doc.resolve(value) else {
            continue;
        };
        match get(doc, dict, "Subtype") {
            Some(Object::Name(s)) if s == "Image" => {
                out.push(image_info(doc, dict, raw, page, number));
            }
            Some(Object::Name(s)) if s == "Form" => {
                if !seen_forms.insert((page, number)) {
                    continue;
                }
                let form_res = get(doc, dict, "Resources").and_then(|o| dict_of(doc, o));
                if let Some(form_res) = form_res {
                    walk_resources(doc, form_res, page, depth + 1, seen_forms, budget, out);
                }
            }
            _ => {}
        }
    }
}

fn filter_names(doc: &Document, d: &Dict) -> Vec<String> {
    match get(doc, d, "Filter") {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|o| match doc.resolve(o) {
                Some(Object::Name(n)) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The colour space's name and component count.
fn colour_space(doc: &Document, d: &Dict) -> (String, u32) {
    match get(doc, d, "ColorSpace") {
        Some(Object::Name(n)) => (n.clone(), components_of_name(n)),
        Some(Object::Array(items)) => {
            let first = items.first().and_then(|o| match doc.resolve(o) {
                Some(Object::Name(n)) => Some(n.clone()),
                _ => None,
            });
            match first.as_deref() {
                Some("Indexed") => ("Indexed".into(), 1),
                Some("ICCBased") => {
                    let n = items
                        .get(1)
                        .and_then(|o| dict_of(doc, o))
                        .and_then(|s| int(doc, s, "N"))
                        .unwrap_or(0) as u32;
                    (format!("ICCBased({n})"), n)
                }
                Some(other) => (other.to_string(), components_of_name(other)),
                None => ("unknown".into(), 0),
            }
        }
        _ => ("unknown".into(), 0),
    }
}

fn components_of_name(name: &str) -> u32 {
    match name {
        "DeviceGray" | "CalGray" | "G" => 1,
        "DeviceRGB" | "CalRGB" | "RGB" => 3,
        "DeviceCMYK" | "CMYK" => 4,
        _ => 0,
    }
}

fn image_info(doc: &Document, d: &Dict, raw: &[u8], page: usize, object: u32) -> ImageInfo {
    let width = int(doc, d, "Width").unwrap_or(0).max(0) as u32;
    let height = int(doc, d, "Height").unwrap_or(0).max(0) as u32;
    let is_mask = matches!(get(doc, d, "ImageMask"), Some(Object::Bool(true)));
    let bpc = if is_mask {
        1
    } else {
        int(doc, d, "BitsPerComponent").unwrap_or(8).max(0) as u32
    };
    let (color_space, comps) = if is_mask {
        ("ImageMask".to_string(), 1)
    } else {
        colour_space(doc, d)
    };
    let filters = filter_names(doc, d);
    let mut info = ImageInfo {
        page,
        object,
        width,
        height,
        color_space: color_space.clone(),
        bits_per_component: bpc,
        filters: filters.clone(),
        encoding: ImageEncoding::Unsupported(filters.join(",")),
        data: Vec::new(),
    };
    match filters.last().map(String::as_str) {
        Some("DCTDecode") => {
            let before = &filters[..filters.len() - 1];
            if before.is_empty() {
                info.encoding = ImageEncoding::Jpeg;
                info.data = raw.to_vec();
            } else if before.iter().all(|f| f == "FlateDecode") {
                let mut stripped = d.clone();
                stripped.put("Filter".to_string(), Object::Name("FlateDecode".into()));
                if let Some(decoded) = doc.stream_bytes(&Object::Stream(stripped, raw.to_vec())) {
                    info.encoding = ImageEncoding::Jpeg;
                    info.data = decoded;
                }
            }
        }
        Some("JPXDecode") => {
            info.encoding = ImageEncoding::Jpx;
            info.data = raw.to_vec();
        }
        _ => {
            let Some(samples) = doc.stream_bytes(&Object::Stream(d.clone(), raw.to_vec())) else {
                return info;
            };
            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|p| p.checked_mul(comps as usize));
            let is_png_shape = bpc == 8
                && (color_space == "DeviceGray" || color_space == "DeviceRGB")
                && expected == Some(samples.len());
            if is_png_shape {
                let colour_type = if comps == 1 { 0 } else { 2 };
                info.data = encode_png(width, height, colour_type, &samples);
                info.encoding = ImageEncoding::Png;
            } else {
                info.data = samples;
                info.encoding = ImageEncoding::RawSamples;
            }
        }
    }
    info
}

// ----- PNG writing (stored deflate) -----------------------------------------------------------

/// A PNG of 8-bit samples with colour type 0 (gray) or 2 (RGB). Each scanline uses filter type
/// 0, and the zlib stream holds stored (uncompressed) blocks.
pub(crate) fn encode_png(width: u32, height: u32, colour_type: u8, samples: &[u8]) -> Vec<u8> {
    let comps = if colour_type == 2 { 3 } else { 1 };
    let row = width as usize * comps;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        let start = y * row;
        raw.extend_from_slice(&samples[start.min(samples.len())..(start + row).min(samples.len())]);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        zlib.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        zlib.push(last);
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, colour_type, 0, 0, 0]);

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &zlib);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crate::arxiv_source::crc32(&body).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/tests/fixtures/pdf-images/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_default()
    }

    /// Reads a PNG written by `encode_png`: checks every chunk CRC, inflates the stored zlib
    /// stream, and returns (width, height, colour type, scanlines without filter bytes).
    fn read_png(png: &[u8]) -> Option<(u32, u32, u8, Vec<u8>)> {
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return None;
        }
        let mut pos = 8;
        let (mut w, mut h, mut ct) = (0, 0, 0);
        let mut idat = Vec::new();
        while pos + 12 <= png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().ok()?) as usize;
            let kind = &png[pos + 4..pos + 8];
            let body = png.get(pos + 8..pos + 8 + len)?;
            let crc = u32::from_be_bytes(png.get(pos + 8 + len..pos + 12 + len)?.try_into().ok()?);
            let mut check = kind.to_vec();
            check.extend_from_slice(body);
            if crate::arxiv_source::crc32(&check) != crc {
                return None;
            }
            match kind {
                b"IHDR" => {
                    w = u32::from_be_bytes(body[0..4].try_into().ok()?);
                    h = u32::from_be_bytes(body[4..8].try_into().ok()?);
                    if body[8] != 8 {
                        return None;
                    }
                    ct = body[9];
                }
                b"IDAT" => idat.extend_from_slice(body),
                _ => {}
            }
            pos += 12 + len;
        }
        // zlib: header, stored blocks, adler32.
        if idat.len() < 6 || idat[0] != 0x78 {
            return None;
        }
        let stream = &idat[2..idat.len() - 4];
        let raw = crate::pdf::parser::inflate::inflate_capped(stream, 1 << 20)?;
        let comps = if ct == 2 { 3 } else { 1 };
        let row = w as usize * comps + 1;
        if raw.len() != row * h as usize {
            return None;
        }
        let mut pixels = Vec::new();
        for line in raw.chunks(row) {
            if line[0] != 0 {
                return None;
            }
            pixels.extend_from_slice(&line[1..]);
        }
        Some((w, h, ct, pixels))
    }

    #[test]
    fn mixed_fixture_lists_every_occurrence_with_its_page() {
        let images = extract_images(&fixture("images-mixed.pdf")).unwrap_or_default();
        let shape: Vec<(usize, u32, u32, u32, String, u32, ImageEncoding)> = images
            .iter()
            .map(|i| {
                (
                    i.page,
                    i.object,
                    i.width,
                    i.height,
                    i.color_space.clone(),
                    i.bits_per_component,
                    i.encoding.clone(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, 3, 16, 8, "DeviceRGB".into(), 8, ImageEncoding::Png),
                (0, 4, 8, 4, "DeviceGray".into(), 8, ImageEncoding::Png),
                (0, 5, 32, 24, "DeviceRGB".into(), 8, ImageEncoding::Jpeg),
                (1, 6, 4, 2, "Indexed".into(), 2, ImageEncoding::RawSamples),
                (2, 7, 16, 16, "DeviceGray".into(), 8, ImageEncoding::Jpeg),
                (3, 3, 16, 8, "DeviceRGB".into(), 8, ImageEncoding::Png),
            ]
        );
    }

    #[test]
    fn jpeg_data_is_the_original_codestream() {
        let images = extract_images(&fixture("images-mixed.pdf")).unwrap_or_default();
        for i in images.iter().filter(|i| i.encoding == ImageEncoding::Jpeg) {
            assert_eq!(&i.data[..3], &[0xff, 0xd8, 0xff]);
            assert_eq!(&i.data[i.data.len() - 2..], &[0xff, 0xd9]);
        }
    }

    #[test]
    fn flate_rgb_pixels_are_exact_in_the_png() {
        let images = extract_images(&fixture("images-mixed.pdf")).unwrap_or_default();
        let rgb = &images[0];
        let (w, h, ct, pixels) = read_png(&rgb.data).unwrap_or_default();
        assert_eq!((w, h, ct), (16, 8, 2));
        let expected: Vec<u8> = (0..8u32)
            .flat_map(|y| {
                (0..16u32).flat_map(move |x| [((x * 16) & 255) as u8, ((y * 32) & 255) as u8, 200])
            })
            .collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn gray_flate_pixels_are_exact_in_the_png() {
        let images = extract_images(&fixture("images-mixed.pdf")).unwrap_or_default();
        let (w, h, ct, pixels) = read_png(&images[1].data).unwrap_or_default();
        assert_eq!((w, h, ct), (8, 4, 0));
        let expected: Vec<u8> = (0..4)
            .flat_map(|_| (0..8u32).map(|x| ((x * 255) / 7) as u8))
            .collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn indexed_samples_are_the_raw_packed_bits() {
        let images = extract_images(&fixture("images-mixed.pdf")).unwrap_or_default();
        assert_eq!(images[3].data, vec![0b0001_1011, 0b1110_0100]);
    }

    #[test]
    fn png_predictor_is_undone_before_the_png_is_written() {
        let images = extract_images(&fixture("inherited-predictor.pdf")).unwrap_or_default();
        assert_eq!(images.len(), 1, "the inherited Resources reach the page");
        let (w, h, ct, pixels) = read_png(&images[0].data).unwrap_or_default();
        assert_eq!((w, h, ct), (5, 6, 2));
        let expected: Vec<u8> = (0..6u32)
            .flat_map(|y| {
                (0..5u32)
                    .flat_map(move |x| [((x * 40 + y * 7) & 255) as u8, ((y * 30) & 255) as u8, 90])
            })
            .collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn png_writer_handles_multi_block_data() {
        // More than one 64 KiB stored block.
        let (w, h) = (300u32, 300u32);
        let samples: Vec<u8> = (0..(w * h * 3)).map(|i| (i % 251) as u8).collect();
        let png = encode_png(w, h, 2, &samples);
        let (rw, rh, ct, pixels) = read_png(&png).unwrap_or_default();
        assert_eq!((rw, rh, ct), (w, h, 2));
        assert_eq!(pixels, samples);
    }

    #[test]
    fn adler32_matches_the_reference_value() {
        // RFC 1950 example: adler32("Wikipedia") = 0x11E60398.
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn broken_and_hostile_inputs_never_panic() {
        assert!(extract_images(b"not a pdf").is_err());
        let original = fixture("images-mixed.pdf");
        for cut in (0..original.len()).step_by(29) {
            let _ = extract_images(&original[..cut]);
        }
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..150 {
            let mut bytes = original.clone();
            for _ in 0..3 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let at = (state as usize) % bytes.len();
                bytes[at] = (state >> 9) as u8;
            }
            let _ = extract_images(&bytes);
        }
    }
}
