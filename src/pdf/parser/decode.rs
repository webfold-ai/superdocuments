//! decode.rs — stream filters and predictors (ISO 32000 §7.4), G18.
//!
//! `/Filter` is applied in order, each stage with its own `/DecodeParms` entry. A FlateDecode
//! stage is inflated, then undone by its predictor (Table 8): `/Predictor 1` passes the bytes
//! through, PNG predictors 10–15 reconstruct whole rows, and TIFF (2) or an undefined value
//! refuses the stream. Any other filter is refused rather than handing on undecoded bytes.
//!
//! Every size and offset on this path is checked by the proof-backed kernels in
//! `super::pdf_kernels`: `predictor_kind`, `png_geometry`, `row_start` and `png_recon_byte`.

use super::inflate::zlib_capped;
use super::parse::is_ws;
use super::pdf_kernels as k;
use super::{Dict, Object};

/// The most a single FlateDecode stage may produce. A stream larger than this is refused, so a
/// hostile stream cannot force an unbounded allocation.
pub(super) const MAX_DECODED: usize = 64 << 20;

/// The `/Filter` names of a stream dictionary, in order. `None` if an entry is not a name.
fn filter_names(d: &Dict) -> Option<Vec<String>> {
    match d.get(&"Filter".to_string()) {
        None => Some(Vec::new()),
        Some(Object::Name(n)) => Some(vec![n.clone()]),
        Some(Object::Array(a)) => a
            .iter()
            .map(|o| match o {
                Object::Name(n) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        Some(_) => None,
    }
}

/// The `/DecodeParms` entry for stage `i`: a dictionary, a null, or absent.
/// A `Ref` is not followed here, so it counts as absent.
fn stage_parms(d: &Dict, i: usize) -> Option<&Dict> {
    match d.get(&"DecodeParms".to_string()) {
        Some(Object::Dict(p)) if i == 0 => Some(p),
        Some(Object::Array(a)) => match a.get(i) {
            Some(Object::Dict(p)) => Some(p),
            _ => None,
        },
        _ => None,
    }
}

/// A non-negative integer entry of a dictionary, if present and in range.
fn uint_of(d: Option<&Dict>, key: &str) -> Option<Option<u64>> {
    match d {
        None => Some(None),
        Some(d) => match d.get(&key.to_string()) {
            None => Some(None),
            Some(Object::Int(n)) if *n >= 0 => Some(Some(*n as u64)),
            Some(_) => None,
        },
    }
}

/// Undo one row predictor (PNG, Table 8 and §9). `data` is a whole number of
/// `rowBytes + 1`-byte rows, each a filter tag followed by the filtered row; a trailing partial
/// row is dropped. An invalid tag refuses the stream. The output is whole decoded rows.
fn png_unpredict(data: &[u8], row_bytes: u64, bpp: u64) -> Option<Vec<u8>> {
    let len = data.len() as u64;
    let row_len = row_bytes.checked_add(1)?;
    // Row 0 must be accepted before anything is allocated for the rows.
    let (ok0, _) = k::row_start(0, row_len, len);
    if !ok0 {
        return Some(Vec::new());
    }
    let rb = row_bytes as usize;
    let rows_fit = (len / row_len) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(rows_fit.saturating_mul(rb));
    let mut prev = vec![0u8; rb];
    let mut cur = vec![0u8; rb];
    let mut i: u64 = 0;
    loop {
        let (ok, start) = k::row_start(i, row_len, len);
        if !ok {
            break;
        }
        let start = start as usize;
        let tag = data[start];
        for j in 0..rb {
            let x = data[start + 1 + j];
            let a = if (j as u64) >= bpp { cur[j - bpp as usize] } else { 0 };
            let b = prev[j];
            let c = if (j as u64) >= bpp { prev[j - bpp as usize] } else { 0 };
            let (ok_byte, v) = k::png_recon_byte(tag, x, a, b, c);
            if !ok_byte {
                return None;
            }
            cur[j] = v;
        }
        out.extend_from_slice(&cur);
        prev.copy_from_slice(&cur);
        i += 1;
    }
    Some(out)
}

/// Undo the predictor of one stage (`/DecodeParms`): `None` refuses the stream.
fn unpredict(data: Vec<u8>, parms: Option<&Dict>) -> Option<Vec<u8>> {
    let predictor = uint_of(parms, "Predictor")?.unwrap_or(1);
    match k::predictor_kind(predictor) {
        0 => Some(data),
        2 => {
            let colors = uint_of(parms, "Colors")?.unwrap_or(1);
            let bpc = uint_of(parms, "BitsPerComponent")?.unwrap_or(8);
            let columns = uint_of(parms, "Columns")?.unwrap_or(1);
            let (ok, row_bytes, bpp) = k::png_geometry(colors, bpc, columns);
            if !ok {
                return None;
            }
            png_unpredict(&data, row_bytes, bpp)
        }
        // TIFF (2) and undefined predictor values refuse the stream.
        _ => None,
    }
}

/// ASCIIHexDecode (§7.4.2): pairs of hex digits, white space ignored, `>` ends the data. An odd
/// final digit is read as if followed by 0. A byte that is not a hex digit is skipped.
fn ascii_hex(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &c in data {
        if c == b'>' {
            break;
        }
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => continue,
        };
        match high.take() {
            None => high = Some(v),
            Some(h) => out.push((h << 4) | v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Some(out)
}

/// ASCII85Decode (§7.4.3): groups of five digits `!`..`u` give four bytes, `z` gives four zero
/// bytes, `~>` ends the data, and a final partial group of `n` digits gives `n - 1` bytes.
/// White space and other stray bytes are skipped. A group whose value exceeds 32 bits refuses.
fn ascii85(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 5 * 4 + 4);
    let mut group = [84u8; 5];
    let mut n = 0usize;
    for &c in data {
        if c == b'~' {
            break;
        }
        if is_ws(c) {
            continue;
        }
        if c == b'z' && n == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        if !(33..=117).contains(&c) {
            continue;
        }
        group[n] = c - 33;
        n += 1;
        if n == 5 {
            out.extend_from_slice(&a85_value(&group)?.to_be_bytes());
            n = 0;
        }
    }
    if n == 1 {
        return None;
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = 84;
        }
        let bytes = a85_value(&group)?.to_be_bytes();
        out.extend_from_slice(&bytes[..n - 1]);
    }
    Some(out)
}

/// The 32-bit value of five base-85 digits, refused when it does not fit.
fn a85_value(group: &[u8; 5]) -> Option<u32> {
    let v = group.iter().fold(0u64, |acc, &d| acc * 85 + d as u64);
    u32::try_from(v).ok()
}

/// RunLengthDecode (§7.4.5): a length byte `0..=127` copies the next `n + 1` bytes; `129..=255`
/// repeats the next byte `257 - n` times; `128` ends the data. Output is capped.
fn run_length(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let len = data[i];
        i += 1;
        match len {
            0..=127 => {
                let n = len as usize + 1;
                out.extend_from_slice(data.get(i..i + n)?);
                i += n;
            }
            128 => break,
            _ => {
                let byte = *data.get(i)?;
                i += 1;
                out.resize(out.len() + (257 - len as usize), byte);
            }
        }
        if out.len() > MAX_DECODED {
            return None;
        }
    }
    Some(out)
}

/// Decode a stream's raw bytes through its `/Filter` chain, one stage per name, each with its own
/// `/DecodeParms`. Filter-free streams come back unchanged. LZWDecode and the image filters, and
/// data a stage cannot decode, give `None`: the stream is refused rather than handed on undecoded.
pub(super) fn decode(d: &Dict, raw: &[u8]) -> Option<Vec<u8>> {
    let names = filter_names(d)?;
    let mut data = raw.to_vec();
    for (i, name) in names.iter().enumerate() {
        data = match name.as_str() {
            "FlateDecode" | "Fl" => {
                let inflated = zlib_capped(&data, MAX_DECODED)?;
                unpredict(inflated, stage_parms(d, i))?
            }
            "ASCIIHexDecode" | "AHx" => ascii_hex(&data)?,
            "ASCII85Decode" | "A85" => ascii85(&data)?,
            "RunLengthDecode" | "RL" => run_length(&data)?,
            _ => return None,
        };
        if data.len() > MAX_DECODED {
            return None;
        }
    }
    Some(data)
}
