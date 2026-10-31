//! # pdffold — PDF → markdown as the hashmap-trinity fold, every operation O(1)
//!
//! Merges [run-llama/liteparse] (PDFium + markdown layout) and [firecrawl/pdf-inspector]
//! (self-contained parse + classify + extract) to their **one invariant** — confirmed by
//! `sang adapt`: both collapse to ~1.56 bits/symbol irreducible (the same kind). The invariant is
//! `PDF bytes → (glyph, position) stream → layout (lines / tables / markdown)`. PDFium is dropped
//! (sovereignty); the parse spine is self-contained.
//!
//! ## The O(1) law (`proof/Substrate/Algebra/HashTrinity*.lean` — do not change the proofs)
//!
//! Every operation is a `get` (O(1)) or an `insert_with` (O(1)) on [`Table`] — one of the four
//! faces. **No scan (O(n)), no binary search (O(log n)), no sort (O(k log n)).** The only O(n) is
//! the one-time read of the bytes (the streaming-law `prepare`); after it, all queries are O(1).
//!
//! | stage | face | O(1) form |
//! |---|---|---|
//! | object lookup (xref) | Map | `Document::object(num)` — the xref **is** the table |
//! | glyph → unicode (CMap) | Map | `cmap.get(code)` — the encoding tables are hashmaps |
//! | font/glyph present? | Set | `marks.has(g)` |
//! | text/layout accumulate | Accumulator (commutative monoid) | the running fold, line via bucket `get` |
//! | object refs / table cells | Graph | `refs.neighbors(n)`, `grid.get((r,c))` |
//! | classify text/scanned | Accumulator ratio | `x = B/U` then `get` |
//!
//! ## Waves (one at a time, verify by collapse)
//! 1. **lift + seed** — done: `sang adapt` both → one kind (1.56 ≈ 1.57 b/s).
//! 2. **object table** — xref + objects → [`Table`] (Map), O(1) [`Document::object`].  ← parse fold
//! 3. **decode table** — glyph→unicode via the CMap hashmap (Map), O(1)/glyph.
//! 4. **layout fold** — the accumulator ruler (text→lines) + table grid as hashmap cells.
//! 5. **classify** — text-vs-scanned as the `x = B/U` accumulator ratio.

pub mod ascii_filters;
pub mod inflate;
mod parse;
/// The loop-free PDF kernels of `proof/extract_pdf.rs`, compiled into the library unchanged: the
/// production xref, object-stream and predictor code calls these same functions, and the proofs in
/// `proof/PdfProofs.lean` are about them (`proof/regen.sh` extracts the same source).
// The lints allowed here are about the kernels' shape (a range check written out, a bool-returning
// if, a manual ceiling division or absolute difference). That shape is what Charon/Aeneas extract,
// so the kernels keep it and the proofs stay about this exact code.
#[allow(
    dead_code,
    clippy::manual_range_contains,
    clippy::needless_bool,
    clippy::manual_div_ceil,
    clippy::manual_abs_diff
)]
#[path = "../../../proof/extract_pdf.rs"]
mod pdf_kernels;
mod xref;
// The HashTrinity primitive now lives in the shared `hashtrinity` crate (the Rust rung of
// HashTrinityCore.lean); pdffold was its template. Re-exported so `crate::Table` is unchanged.
pub use hashtrinity::Table;

/// **The table-region classifier as a seed** — the model-is-a-seed: a [`Table`] (the HashTrinity)
/// from a candidate region's feature signature to `(table votes, ¬table votes)`. The **fold** over
/// labelled regions IS the training (the B/U accumulator merge); the **classify** is one `get` — O(1),
/// no scan, no `k log n`. This is the framework's native classifier, not an external VLM: a region's
/// geometry (column count, cell density, row alignment) is read into the key, the key votes.
#[derive(Default)]
pub struct RegionSeed {
    votes: Table<u64, (i64, i64)>,
}

impl RegionSeed {
    pub fn new() -> Self {
        RegionSeed {
            votes: Table::new(),
        }
    }

    /// Fold one labelled region into the seed (the training step — the accumulator merge).
    pub fn fold(&mut self, feat: u64, is_table: bool) {
        let d = if is_table { (1, 0) } else { (0, 1) };
        self.votes
            .insert_with(feat, d, |n, o| (n.0 + o.0, n.1 + o.1));
    }

    /// Classify a region by its signature — O(1) `get`, majority vote. Unseen signature ⇒ not a table.
    pub fn classify(&self, feat: u64) -> bool {
        matches!(self.votes.get(&feat), Some(&(t, n)) if t > n)
    }

    /// Classify **with backoff** (the Kneser–Ney face): where the seed has evidence (a signature seen
    /// ≥2×) it decides; the unseen tail backs off to `default` (the geometric read). So the seed
    /// *corrects* the heuristic where it has data, and inherits its recall where it doesn't.
    pub fn classify_or(&self, feat: u64, default: bool) -> bool {
        match self.votes.get(&feat) {
            Some(&(t, n)) if t + n >= 2 => t > n,
            _ => default,
        }
    }

    /// Serialise the seed (`feat table ¬table` per line) — the trained model as data.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (k, (t, n)) in self.votes.iter() {
            s.push_str(&format!("{} {} {}\n", k, t, n));
        }
        s
    }

    /// Load a serialised seed.
    pub fn from_text(text: &str) -> Self {
        let mut seed = RegionSeed::new();
        for line in text.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            if let [k, t, n] = p[..] {
                if let (Ok(k), Ok(t), Ok(n)) = (k.parse(), t.parse(), n.parse()) {
                    seed.votes.put(k, (t, n));
                }
            }
        }
        seed
    }
}

/// A PDF object — the universal value (document nodes and content-stream operands are the same type,
/// the one Term). A dict is a [`Table`] (the Map face); a ref resolves through the object table in O(1).
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(Vec<u8>),
    Name(String),
    Array(Vec<Object>),
    Dict(Dict),
    /// Indirect reference `n g R` — resolved by [`Document::object`] (O(1) xref `get`).
    Ref(u32, u16),
    /// Stream = its dict + raw bytes (decoded lazily in wave 3).
    Stream(Dict, Vec<u8>),
}

/// A PDF dictionary: name → object, the **Map** face — `get` is O(1).
pub type Dict = Table<String, Object>;

/// The parsed document. The object store is the **xref as a hashmap** (object number → object):
/// access is O(1), never an xref scan or binary search.
/// The parsed document. The object store is the **xref as a hashmap** (object number → object):
/// access is O(1), never an xref scan or binary search. `parse`/`markdown`/`stream_bytes` live in
/// `parse.rs` (the fold). The one-time `parse` read is the only O(n); every access after is O(1).
#[derive(Default)]
pub struct Document {
    pub(crate) objects: Table<u32, Object>,
    /// The generation each loaded object was read with. A reference with another generation
    /// resolves to null (ISO 32000 §7.3.10).
    pub(crate) gens: Table<u32, u16>,
    pub(crate) root: u32,
}

impl Document {
    /// Insert a parsed object at its number (the xref Map face, `put` = last-write-wins). O(1).
    pub fn set_object(&mut self, num: u32, obj: Object) {
        self.objects.put(num, obj);
    }

    /// **O(1) object access** — the xref Map `get`. No scan, no binary search.
    pub fn object(&self, num: u32) -> Option<&Object> {
        self.objects.get(&num)
    }

    /// Resolve a reference in one O(1) hop (or pass a direct object through). A reference whose
    /// generation differs from the object's resolves to null.
    pub fn resolve<'a>(&'a self, o: &'a Object) -> Option<&'a Object> {
        match o {
            Object::Ref(n, g) => self.ref_object(*n, *g),
            other => Some(other),
        }
    }

    /// The object that the reference `n g R` names (7.3.10): `None` when the current xref entry
    /// for `n` has another generation (a stale reference into an incremental update) or is absent.
    /// Every lookup of a reference in the parser goes through here, never through `object` alone.
    pub fn ref_object(&self, n: u32, g: u16) -> Option<&Object> {
        match self.gens.get(&n) {
            Some(&have) if have != g => None,
            _ => self.object(n),
        }
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
    pub fn root(&self) -> u32 {
        self.root
    }
}

/// A decoded embedded image — RGBA8 pixels, decoded by **our own `transcode`** crate (the sovereign
/// codec), never the third-party `image` crate. The document path (pdffold) and the image path
/// (transcode) are both ours; the whole stack links no PDFium and no `image`.
///
/// Gated behind the `images` feature: the sovereign **core** (text/headings/markdown + resilience)
/// links nothing; only image *decode* pulls our `transcode` codec (`cargo build --features images`).
#[cfg(feature = "images")]
pub struct PdfImage {
    pub obj: u32,
    pub width: u32,
    pub height: u32,
    /// RGBA8, length = width*height*4.
    pub rgba: Vec<u8>,
}

#[cfg(feature = "images")]
impl PdfImage {
    /// Re-encode as a PNG via transcode (sovereign) — for extracting embedded images to disk.
    pub fn to_png(&self) -> Vec<u8> {
        transcode::png::encode(self.width, self.height, &self.rgba, true).unwrap_or_default()
    }
}

#[cfg(feature = "images")]
impl Document {
    /// **Decode every embedded `/Subtype /Image` XObject** — JPEG (DCTDecode) through `transcode::jpeg`
    /// (a DCTDecode stream *is* a JPEG), raw RGB/gray samples through pdffold's own inflate. The
    /// third-party `image` crate is never linked; this is the sovereign image path liteparse uses
    /// PDFium + `image` for. Each image is one `get`/decode — no scan of the page content.
    pub fn images(&self) -> Vec<PdfImage> {
        let mut out = Vec::new();
        for (num, obj) in self.objects.iter() {
            if let Object::Stream(d, raw) = obj {
                if !matches!(d.get(&"Subtype".to_string()), Some(Object::Name(s)) if s == "Image") {
                    continue;
                }
                if let Some((w, h, rgba)) = self.decode_image(d, raw) {
                    out.push(PdfImage {
                        obj: *num,
                        width: w,
                        height: h,
                        rgba,
                    });
                }
            }
        }
        out
    }

    fn filter_has(&self, d: &Dict, name: &str) -> bool {
        match d.get(&"Filter".to_string()) {
            Some(Object::Name(f)) => f == name,
            Some(Object::Array(a)) => a.iter().any(|x| matches!(x, Object::Name(n) if n == name)),
            _ => false,
        }
    }

    fn int_of(&self, d: &Dict, key: &str) -> Option<i64> {
        match d.get(&key.to_string()).and_then(|o| self.resolve(o)) {
            Some(Object::Int(n)) => Some(*n),
            _ => None,
        }
    }

    /// Resolve a PDF /ColorSpace (name or array) to its kind. ICCBased reads `/N` (1/3/4 →
    /// gray/rgb/cmyk — the profile itself is ignored, the device approximation); Indexed carries its
    /// base + palette bytes. The Map face all the way down — each lookup an O(1) `get`.
    fn resolve_cs(&self, o: &Object) -> Option<Cs> {
        match self.resolve(o)? {
            Object::Name(s) => match s.as_str() {
                "DeviceGray" | "CalGray" | "G" => Some(Cs::Gray),
                "DeviceRGB" | "CalRGB" | "RGB" => Some(Cs::Rgb),
                "DeviceCMYK" | "CMYK" => Some(Cs::Cmyk),
                _ => None,
            },
            Object::Array(a) => match a.first() {
                Some(Object::Name(n)) => match n.as_str() {
                    "ICCBased" => match self.resolve(a.get(1)?)? {
                        Object::Stream(sd, _) => match self.int_of(sd, "N")? {
                            1 => Some(Cs::Gray),
                            3 => Some(Cs::Rgb),
                            4 => Some(Cs::Cmyk),
                            _ => None,
                        },
                        _ => None,
                    },
                    "Indexed" | "I" => {
                        let base = self.resolve_cs(a.get(1)?)?;
                        let pal = match self.resolve(a.get(3)?)? {
                            Object::Str(b) => b.clone(),
                            o @ Object::Stream(..) => self.stream_bytes(o)?,
                            _ => return None,
                        };
                        Some(Cs::Indexed(Box::new(base), pal))
                    }
                    "CalRGB" => Some(Cs::Rgb),
                    "CalGray" => Some(Cs::Gray),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// One image XObject → (width, height, RGBA8). DCTDecode → transcode::jpeg (a DCTDecode stream IS a
    /// JPEG); else raw samples interpreted by /ColorSpace + /BitsPerComponent — DeviceGray/RGB/CMYK,
    /// ICCBased (by `/N`), and Indexed palettes, at 1/2/4/8 bits per component.
    fn decode_image(&self, d: &Dict, raw: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
        if self.filter_has(d, "DCTDecode") {
            let img = transcode::jpeg::decode(raw).ok()?; // the stream IS a JPEG — our sovereign decoder
            return Some((img.width, img.height, img.pixels));
        }
        if matches!(d.get(&"ImageMask".to_string()), Some(Object::Bool(true))) {
            return None;
        } // a stencil, not content
        let (w, h) = (
            self.int_of(d, "Width")? as usize,
            self.int_of(d, "Height")? as usize,
        );
        // O(1) dimension bound (anti-bomb): no real raster exceeds 2¹⁶ per side; reject before any
        // arithmetic on the declared size can overflow the stride/allocation. (PDF/JPEG max is 65535.)
        if w == 0 || h == 0 || w > 1 << 16 || h > 1 << 16 {
            return None;
        }
        let bpc = self.int_of(d, "BitsPerComponent").unwrap_or(8) as usize;
        if !(bpc == 1 || bpc == 2 || bpc == 4 || bpc == 8) {
            return None;
        }
        let cs = self.resolve_cs(d.get(&"ColorSpace".to_string())?)?;
        let n = cs.ncomp(); // samples per pixel (Indexed = 1 index)
        let stride = (w * n * bpc + 7) / 8; // each row is byte-aligned
        let want = stride.saturating_mul(h); // the image's exact decoded size — the inflate bound
                                             // FlateDecode bounded by the image's OWN declared size (streaming-law): a bomb claiming tiny
                                             // dimensions is capped at tiny output, a legit large image gets its full size. No blind ratio.
        let s = if self.filter_has(d, "FlateDecode") {
            inflate::zlib_capped(raw, want.saturating_add(64))?
        } else {
            raw.to_vec()
        };
        if s.len() < want {
            return None;
        }
        let maxv = ((1u32 << bpc) - 1).max(1);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = &s[y * stride..];
            let mut bit = 0usize;
            for _ in 0..w {
                let mut samp = [0u32; 4];
                for c in samp.iter_mut().take(n) {
                    *c = read_bits(row, bit, bpc);
                    bit += bpc;
                }
                let (r, g, b) = cs.to_rgb(&samp, maxv);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Some((w as u32, h as u32, rgba))
    }
}

/// A resolved PDF colorspace — the components a pixel carries, and how they map to RGB.
#[cfg(feature = "images")]
enum Cs {
    Gray,
    Rgb,
    Cmyk,
    Indexed(Box<Cs>, Vec<u8>),
}

#[cfg(feature = "images")]
impl Cs {
    /// Samples per pixel in the image data (Indexed stores one index per pixel).
    fn ncomp(&self) -> usize {
        match self {
            Cs::Gray => 1,
            Cs::Rgb => 3,
            Cs::Cmyk => 4,
            Cs::Indexed(..) => 1,
        }
    }

    /// Map a pixel's samples (each in `0..=maxv`) to RGB. Indexed looks the index up in its 8-bit palette.
    fn to_rgb(&self, samp: &[u32; 4], maxv: u32) -> (u8, u8, u8) {
        let sc = |v: u32| (v * 255 / maxv) as u8;
        match self {
            Cs::Gray => {
                let v = sc(samp[0]);
                (v, v, v)
            }
            Cs::Rgb => (sc(samp[0]), sc(samp[1]), sc(samp[2])),
            Cs::Cmyk => cmyk_to_rgb(sc(samp[0]), sc(samp[1]), sc(samp[2]), sc(samp[3])),
            Cs::Indexed(base, pal) => {
                let off = samp[0] as usize * base.ncomp(); // index is raw, palette is 8-bit
                let g = |k: usize| *pal.get(off + k).unwrap_or(&0);
                match base.as_ref() {
                    Cs::Gray => {
                        let v = g(0);
                        (v, v, v)
                    }
                    Cs::Rgb => (g(0), g(1), g(2)),
                    Cs::Cmyk => cmyk_to_rgb(g(0), g(1), g(2), g(3)),
                    Cs::Indexed(..) => (0, 0, 0),
                }
            }
        }
    }
}

/// Subtractive CMYK → RGB (raw PDF DeviceCMYK, 0 = no ink): `R = (255−C)(255−K)/255`.
#[cfg(feature = "images")]
fn cmyk_to_rgb(c: u8, m: u8, y: u8, k: u8) -> (u8, u8, u8) {
    let f = |x: u8| (255 - x as u32) * (255 - k as u32) / 255;
    (f(c) as u8, f(m) as u8, f(y) as u8)
}

/// Read `n` bits big-endian from `data` at bit offset `off` (sub-byte sample unpacking).
#[cfg(feature = "images")]
fn read_bits(data: &[u8], off: usize, n: usize) -> u32 {
    let mut v = 0u32;
    for i in 0..n {
        let b = off + i;
        let bit = (data.get(b / 8).copied().unwrap_or(0) >> (7 - (b % 8))) & 1;
        v = (v << 1) | bit as u32;
    }
    v
}

#[derive(Debug)]
pub enum PdfError {
    Parse(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    // The object store is the xref hashmap: O(1) access + one-hop ref resolution. No scan/bsearch.
    #[test]
    fn object_store_is_o1_xref() {
        let mut doc = Document::default();
        doc.set_object(1, Object::Int(42));
        doc.set_object(2, Object::Ref(1, 0)); // an indirect ref to object 1
        assert_eq!(doc.object(1), Some(&Object::Int(42)));
        assert_eq!(doc.resolve(&Object::Ref(1, 0)), Some(&Object::Int(42)));
        assert_eq!(doc.object(99), None); // absence = the default
        assert_eq!(doc.object_count(), 2);
    }

    // A PDF dict is the Map face — name → object, O(1) get.
    #[test]
    fn dict_is_the_map_face() {
        let mut d: Dict = Table::new();
        d.put("Type".into(), Object::Name("Page".into()));
        d.put("Type".into(), Object::Name("Catalog".into())); // last write wins
        assert_eq!(
            d.get(&"Type".to_string()),
            Some(&Object::Name("Catalog".into()))
        );
    }
}
