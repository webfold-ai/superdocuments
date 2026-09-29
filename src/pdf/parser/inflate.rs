//! inflate.rs — DEFLATE (RFC 1951) + zlib (RFC 1950), sovereign and zero-dep. PDF content streams are
//! FlateDecode; this reads them. The Huffman decode is itself the hashmap face — a code→symbol table,
//! built once, `get` O(1) per symbol (here a flat lookup array, the densest Map).

/// LSB-first bit reader over the DEFLATE byte stream.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u32,
    n: u32,
}
impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits {
            d,
            pos: 0,
            buf: 0,
            n: 0,
        }
    }
    #[inline]
    fn need(&mut self, k: u32) {
        while self.n < k {
            let b = if self.pos < self.d.len() {
                self.d[self.pos]
            } else {
                0
            } as u32;
            self.pos += 1;
            self.buf |= b << self.n;
            self.n += 8;
        }
    }
    #[inline]
    fn bits(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        self.need(k);
        let v = self.buf & ((1u32 << k) - 1);
        self.buf >>= k;
        self.n -= k;
        v
    }
    #[inline]
    fn align(&mut self) {
        let drop = self.n % 8;
        self.buf >>= drop;
        self.n -= drop;
    }
}

/// A canonical-Huffman decoder: code lengths → a fast table. Decode is one bit-at-a-time walk,
/// O(max code length) ≤ 15 = O(1). (The classic inflate decoder; the table IS the Map face.)
struct Huff {
    counts: [u16; 16], // number of codes of each length
    symbols: Vec<u16>, // symbols sorted by (length, value)
}
impl Huff {
    fn new(lengths: &[u16]) -> Huff {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Huff { counts, symbols }
    }
    #[inline]
    fn decode(&self, b: &mut Bits) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bits(1) as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Some(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn fixed_huffs() -> (Huff, Huff) {
    let mut ll = [0u16; 288];
    for i in 0..144 {
        ll[i] = 8;
    }
    for i in 144..256 {
        ll[i] = 9;
    }
    for i in 256..280 {
        ll[i] = 7;
    }
    for i in 280..288 {
        ll[i] = 8;
    }
    (Huff::new(&ll), Huff::new(&[5u16; 30]))
}

fn inflate_block(
    b: &mut Bits,
    out: &mut Vec<u8>,
    lit: &Huff,
    dist: &Huff,
    cap: usize,
) -> Option<()> {
    loop {
        if out.len() > cap {
            return None;
        } // anti-bomb: output bounded by input, never unbounded
        let sym = lit.decode(b)?;
        if sym == 256 {
            return Some(());
        }
        if sym < 256 {
            out.push(sym as u8);
            continue;
        }
        let s = (sym - 257) as usize;
        if s >= 29 {
            return None;
        }
        let length = LEN_BASE[s] as usize + b.bits(LEN_EXTRA[s]) as usize;
        let dsym = dist.decode(b)? as usize;
        if dsym >= 30 {
            return None;
        }
        let d = DIST_BASE[dsym] as usize + b.bits(DIST_EXTRA[dsym]) as usize;
        if d > out.len() {
            return None;
        }
        let start = out.len() - d;
        for i in 0..length {
            let c = out[start + i];
            out.push(c);
        }
    }
}

/// Raw DEFLATE (RFC 1951) → bytes.
pub fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    inflate_capped(data, default_cap(data))
}

/// The default decompression-bomb bound when the caller has no exact target: ≤ 256× the compressed
/// size, hard-capped at 256 MiB. 256× admits legitimately highly-compressible streams while rejecting
/// bombs in O(1). When the decoded size IS known (an image's `stride·h`), pass it via `inflate_capped`.
fn default_cap(data: &[u8]) -> usize {
    data.len().saturating_mul(256).min(256 << 20).max(1 << 20)
}

/// Raw DEFLATE with an explicit output ceiling (the streaming-law bound: output bounded, never a
/// declared-size allocation). Returns None if the output would exceed `cap`.
pub fn inflate_capped(data: &[u8], cap: usize) -> Option<Vec<u8>> {
    let mut b = Bits::new(data);
    let mut out = Vec::new();
    loop {
        if b.pos > data.len() + 8 {
            return None;
        } // reading past the data into zero-padding ⇒ corrupt
        let bfinal = b.bits(1);
        match b.bits(2) {
            0 => {
                // stored
                b.align();
                let len = b.bits(16) as usize;
                let _nlen = b.bits(16);
                if out.len() + len > cap {
                    return None;
                }
                for _ in 0..len {
                    out.push(b.bits(8) as u8);
                }
            }
            1 => {
                let (l, d) = fixed_huffs();
                inflate_block(&mut b, &mut out, &l, &d, cap)?;
            }
            2 => {
                let hlit = b.bits(5) as usize + 257;
                let hdist = b.bits(5) as usize + 1;
                let hclen = b.bits(4) as usize + 4;
                const ORD: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let mut cl = [0u16; 19];
                for i in 0..hclen {
                    cl[ORD[i]] = b.bits(3) as u16;
                }
                let clh = Huff::new(&cl);
                let mut lengths = vec![0u16; hlit + hdist];
                let mut i = 0;
                while i < lengths.len() {
                    let s = clh.decode(&mut b)?;
                    match s {
                        0..=15 => {
                            lengths[i] = s;
                            i += 1;
                        }
                        16 => {
                            let r = 3 + b.bits(2) as usize;
                            let prev = if i > 0 { lengths[i - 1] } else { 0 };
                            for _ in 0..r {
                                if i < lengths.len() {
                                    lengths[i] = prev;
                                    i += 1;
                                }
                            }
                        }
                        17 => {
                            let r = 3 + b.bits(3) as usize;
                            for _ in 0..r {
                                if i < lengths.len() {
                                    lengths[i] = 0;
                                    i += 1;
                                }
                            }
                        }
                        18 => {
                            let r = 11 + b.bits(7) as usize;
                            for _ in 0..r {
                                if i < lengths.len() {
                                    lengths[i] = 0;
                                    i += 1;
                                }
                            }
                        }
                        _ => return None,
                    }
                }
                let lit = Huff::new(&lengths[..hlit]);
                let dist = Huff::new(&lengths[hlit..]);
                inflate_block(&mut b, &mut out, &lit, &dist, cap)?;
            }
            _ => return None,
        }
        if bfinal == 1 {
            break;
        }
    }
    Some(out)
}

/// zlib (RFC 1950) → bytes: 2-byte header (CM=8), DEFLATE body, adler32 (ignored). PDF FlateDecode.
pub fn zlib(data: &[u8]) -> Option<Vec<u8>> {
    zlib_capped(data, default_cap(data))
}

/// zlib with an explicit output ceiling — used by the image decoder, which knows the exact decoded
/// size (`stride·h`), so the bound is the image's own declared size, not a blind ratio.
pub fn zlib_capped(data: &[u8], cap: usize) -> Option<Vec<u8>> {
    if data.len() < 2 {
        return None;
    }
    if data[0] & 0x0f != 8 {
        return inflate_capped(data, cap);
    } // not zlib-wrapped → raw deflate
    inflate_capped(&data[2..], cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_stored() {
        // a stored (uncompressed) deflate block: BFINAL=1,BTYPE=00, then LEN/NLEN + bytes
        let payload = b"hello";
        let mut s = Vec::new();
        s.push(0x01); // bfinal=1, btype=00 (low bits)
        s.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        s.extend_from_slice(&(!(payload.len() as u16)).to_le_bytes());
        s.extend_from_slice(payload);
        assert_eq!(inflate(&s).as_deref(), Some(&payload[..]));
    }
}
