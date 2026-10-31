//! PDF xref / object-stream / predictor kernels for the Charon/Aeneas extraction (G18).
//!
//! Each function is loop-free, uses only checked u8/u32/u64 arithmetic and returns an
//! explicit `(accepted, value)` pair rather than an `Option`, so the extraction is total.
//! `proof/regen.sh` extracts exactly these 13 names into `PdfMath.lean`; `PdfProofs.lean`
//! proves each equals its field function in `PdfSpec.lean`. Production (`src/pdf/parser`)
//! includes this file with `#[path]` and calls the same functions on its decision path.
//! Changing a signature or a contract is a specification change (see `PdfSpec.lean`).

// Literals (not `const` items, which the extraction would add as definitions):
// 0xFFFF_FFFF_FFFF_FFFF is 2^64 - 1; 0x00FF_FFFF_FFFF_FFFF is 2^56 - 1, the largest acc with
// acc * 256 + b < 2^64; 0x4000_0000_0000_0000 is 2^62, the largest n with 4 * n <= 2^64.

/// One big-endian step: `acc * 256 + byte`, refused (not wrapped) at `2^64`.
pub fn xref_be_step(acc: u64, byte: u8) -> (bool, u64) {
    if acc <= 0x00FF_FFFF_FFFF_FFFF {
        (true, acc * 256 + (byte as u64))
    } else {
        (false, 0)
    }
}

/// Row length `W[0] + W[1] + W[2]`, accepted only when it is non-zero and below `2^64`.
pub fn xref_row_len(w1: u64, w2: u64, w3: u64) -> (bool, u64) {
    if w2 <= 0xFFFF_FFFF_FFFF_FFFF - w1 {
        let s12 = w1 + w2;
        if w3 <= 0xFFFF_FFFF_FFFF_FFFF - s12 {
            let s = s12 + w3;
            if s > 0 {
                (true, s)
            } else {
                (false, 0)
            }
        } else {
            (false, 0)
        }
    } else {
        (false, 0)
    }
}

/// Start of row `index` of `row_len` bytes, accepted iff `(index+1)*row_len <= data_len`.
pub fn row_start(index: u64, row_len: u64, data_len: u64) -> (bool, u64) {
    if row_len == 0 {
        (true, 0)
    } else if index < data_len / row_len {
        (true, index * row_len)
    } else {
        (false, 0)
    }
}

/// Table 18 type code: 1 when the type field is absent (`W[0] = 0`), else the field if it is
/// 0, 1 or 2, else 3 (the null object).
pub fn xref_entry_type(w1_zero: bool, f1: u64) -> u8 {
    if w1_zero {
        1
    } else if f1 <= 2 {
        f1 as u8
    } else {
        3
    }
}

/// A byte offset must lie inside the file.
pub fn xref_offset_ok(offset: u64, file_len: u64) -> bool {
    offset < file_len
}

/// A type-2 reference resolves only inside an accepted direct stream, never to the stream
/// itself, and only below the stream's `/N`.
pub fn objstm_ref_ok(stream_direct: bool, is_self: bool, has_n: bool, n: u64, index: u64) -> bool {
    stream_direct && !is_self && has_n && index < n
}

/// Object-stream header: `/First <= len` and `4*n <= first+1`, without overflow.
pub fn objstm_header_ok(first: u64, len: u64, n: u64) -> bool {
    if first > len {
        false
    } else if first < 0xFFFF_FFFF_FFFF_FFFF {
        n <= (first + 1) / 4
    } else {
        n <= 0x4000_0000_0000_0000
    }
}

/// Extent of object `i` in an object stream: `[first+off, end)` where `end` is the next
/// object's offset (or `len`), accepted iff `first+off < end <= len`.
pub fn objstm_span(
    first: u64,
    off: u64,
    has_next: bool,
    next_off: u64,
    len: u64,
) -> (bool, u64, u64) {
    if has_next && next_off > 0xFFFF_FFFF_FFFF_FFFF - first {
        (false, 0, 0)
    } else {
        let end = if has_next { first + next_off } else { len };
        if off > 0xFFFF_FFFF_FFFF_FFFF - first {
            (false, 0, 0)
        } else {
            let start = first + off;
            if start < end && end <= len {
                (true, start, end)
            } else {
                (false, 0, 0)
            }
        }
    }
}

/// A `/Prev` or `/XRefStm` offset is followed only inside the file and only once.
pub fn xref_prev_ok(offset: u64, file_len: u64, seen: bool) -> bool {
    offset < file_len && !seen
}

/// Stream data `[start, start+length)` must end inside the file.
pub fn stream_extent(start: u64, length: u64, file_len: u64) -> (bool, u64) {
    if start <= file_len && length <= file_len - start {
        (true, start + length)
    } else {
        (false, 0)
    }
}

/// `/Predictor` class: 0 none (1), 1 TIFF (2), 2 PNG (10..=15), 3 undefined.
pub fn predictor_kind(p: u64) -> u8 {
    if p == 1 {
        0
    } else if p == 2 {
        1
    } else if p >= 10 && p <= 15 {
        2
    } else {
        3
    }
}

/// PNG geometry from `/Colors /BitsPerComponent /Columns`: `(accepted, rowBytes, pixelBytes)`
/// where `rowBytes = ceil(colors*bpc*columns/8)` and `pixelBytes = ceil(colors*bpc/8)`.
pub fn png_geometry(colors: u64, bpc: u64, columns: u64) -> (bool, u64, u64) {
    let bpc_ok = if bpc == 1 || bpc == 2 || bpc == 4 || bpc == 8 || bpc == 16 {
        true
    } else {
        false
    };
    if colors >= 1 && bpc_ok && columns >= 1 && colors <= 0xFFFF_FFFF_FFFF_FFFF / bpc {
        let cb = colors * bpc;
        if cb <= (0xFFFF_FFFF_FFFF_FFFF - 7) / columns {
            let row_bits = cb * columns;
            (true, (row_bits + 7) / 8, (cb + 7) / 8)
        } else {
            (false, 0, 0)
        }
    } else {
        (false, 0, 0)
    }
}

/// One reconstructed byte `x + predict(...)` modulo 256; refused for an invalid filter tag.
/// The predictor is inlined (0 None, 1 Sub, 2 Up, 3 Average, 4 Paeth with ties to a, b, c).
/// `|u - v|` is computed as an unsigned difference on widened bytes.
pub fn png_recon_byte(tag: u8, x: u8, a: u8, b: u8, c: u8) -> (bool, u8) {
    if tag <= 4 {
        let aw = a as u32;
        let bw = b as u32;
        let cw = c as u32;
        let pred: u32 = if tag == 0 {
            0
        } else if tag == 1 {
            aw
        } else if tag == 2 {
            bw
        } else if tag == 3 {
            (aw + bw) / 2
        } else {
            let pa = if bw >= cw { bw - cw } else { cw - bw };
            let pb = if aw >= cw { aw - cw } else { cw - aw };
            let s_ab = aw + bw;
            let s_cc = cw + cw;
            let pc = if s_ab >= s_cc {
                s_ab - s_cc
            } else {
                s_cc - s_ab
            };
            if pa <= pb && pa <= pc {
                aw
            } else if pb <= pc {
                bw
            } else {
                cw
            }
        };
        let sum = (x as u32) + pred;
        let r = if sum < 256 { sum } else { sum - 256 };
        (true, r as u8)
    } else {
        (false, 0)
    }
}
