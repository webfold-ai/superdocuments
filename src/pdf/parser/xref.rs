//! xref.rs — the cross-reference loader (ISO 32000 §7.5), G18.
//!
//! The object table is loaded through the file's cross-reference structure, not by scanning for
//! `N G obj` headers. The chain starts at `startxref` (§7.5.5) and follows `/Prev` and `/XRefStm`
//! (§7.5.6, §7.5.8.4), reading xref tables (§7.5.4) and xref streams (§7.5.8). A compressed entry
//! is read from its object stream (§7.5.7). The newest section governs each object number: an
//! entry, once set, is kept when an older section is read. A free entry, a generation mismatch, an
//! unknown type or an offset outside the file each resolve to null.
//!
//! Every integer that drives an offset, a count, a row or an allocation passes through the
//! proof-backed kernels in `super::pdf_kernels`; their refinements and contracts are proved in
//! `proof/PdfProofs.lean`. When the chain yields no catalog, `recover` scans for `N G obj`
//! headers as a last resort. The scan is bounded per object and keeps the last header of each
//! number, which is the newest revision in file order.

use super::decode::{decode_within, inflates, MAX_DECODED};
use super::parse::{is_delim, is_ws, parse_dict, parse_value, Lex, Tok};
use super::pdf_kernels as k;
use super::{Dict, Document, Object, PdfError, Table};
use std::cell::Cell;

/// Inflated bytes one document may produce across all of its xref streams and object streams. A
/// failed inflate is charged its whole cap, so an attacker cannot repeat a bomb to multiply work.
const DOC_DECODE_BUDGET: usize = 512 << 20;

/// One cross-reference entry (Table 18), with the fields the kernels check.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Entry {
    Free,
    InUse { offset: u64, gen: u64 },
    Compressed { stream: u64, index: u64 },
    Null,
}

/// A decoded object stream: its bytes, `/First`, whether it declares `/N`, and the header's
/// `(object number, offset)` pairs.
struct ObjStm {
    data: Vec<u8>,
    first: u64,
    has_n: bool,
    pairs: Vec<(u32, u64)>,
}

struct Loader<'a> {
    bytes: &'a [u8],
    len: u64,
    /// Every `endobj` and `endstream` keyword, in order. One pass indexes them, so the end of an
    /// object or a stream is found by a binary search rather than a scan to the end of the file.
    endobj_at: Vec<usize>,
    endstream_at: Vec<usize>,
    /// The governing entry of each object number. A newer section's entry is never replaced.
    map: Table<u32, Entry>,
    /// Decoded object streams, and for each object-stream number the index in `streams` of its
    /// load, or `None` when the load refused. A refusal is cached too: each stream is decoded at
    /// most once per map generation.
    streams: Vec<ObjStm>,
    stream_index: Table<u32, Option<usize>>,
    /// Trailer dictionaries, newest first.
    trailers: Vec<Dict>,
    /// The section offsets already read, shared by the `/Prev` chain and `/XRefStm` links, so that
    /// every link goes through `xref_prev_ok` against one visited set (chainOk).
    visited: Table<u64, bool>,
    /// Inflated bytes this document may still produce (see `DOC_DECODE_BUDGET`).
    decode_budget: usize,
    /// Stream decodes attempted, for the regression tests.
    decode_attempts: usize,
    /// Bytes of object bodies parsed so far, and the most that may be parsed. Each object body is
    /// lexed to its `endobj`; the limit keeps a file of overlapping headers from being lexed
    /// quadratically.
    work: Cell<u64>,
    work_limit: u64,
}

/// Skip white space from `pos`; the position of the first byte that is not white space.
fn skip_ws(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && is_ws(bytes[i]) {
        i += 1;
    }
    i
}

/// A decimal integer at `pos` (after white space): `(value, end)`. Overflow and more than 20 digits
/// refuse.
fn int_at(bytes: &[u8], pos: usize) -> Option<(u64, usize)> {
    let start = skip_ws(bytes, pos);
    let mut i = start;
    let mut v: u64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        v = v.checked_mul(10)?.checked_add((bytes[i] - b'0') as u64)?;
        i += 1;
    }
    if i == start || i - start > 20 {
        return None;
    }
    Some((v, i))
}

/// The end of keyword `kw` at `pos` (after white space), when it is followed by a delimiter,
/// white space, or the end of the input.
fn keyword_at(bytes: &[u8], pos: usize, kw: &[u8]) -> Option<usize> {
    let p = skip_ws(bytes, pos);
    if !bytes[p..].starts_with(kw) {
        return None;
    }
    let end = p + kw.len();
    match bytes.get(end) {
        None => Some(end),
        Some(&b) if is_ws(b) || is_delim(b) => Some(end),
        Some(_) => None,
    }
}

/// The first occurrence of `needle` at or after `from`.
fn find_from(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from > bytes.len() || bytes.len() - from < needle.len() {
        return None;
    }
    bytes[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// The last occurrence of `needle` in `bytes`.
fn rfind(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || bytes.len() < needle.len() {
        return None;
    }
    bytes.windows(needle.len()).rposition(|w| w == needle)
}

/// `N G obj` at `pos`: `(object number, generation, end of the obj keyword)`. The number and the
/// generation are checked against their ranges.
fn header_at(bytes: &[u8], pos: usize) -> Option<(u32, u64, usize)> {
    let (num, a) = int_at(bytes, pos)?;
    let (gen, b) = int_at(bytes, a)?;
    let body = keyword_at(bytes, b, b"obj")?;
    if num > u32::MAX as u64 || gen > u16::MAX as u64 {
        return None;
    }
    Some((num as u32, gen, body))
}

/// Every occurrence of `needle`, in order (one linear pass).
fn occurrences(bytes: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || bytes.len() < needle.len() {
        return Vec::new();
    }
    bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}

/// The first position in the sorted `positions` that is at or after `from`.
/// The first `endobj` keyword at or after `from` that is a token: not inside a literal string
/// (nested parentheses and backslash escapes), a hex string or a comment, and delimited on both
/// sides. `None` when the input ends first, including inside an unterminated string.
fn endobj_token(bytes: &[u8], from: usize) -> Option<usize> {
    let delim = |b: u8| b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b);
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                let mut depth = 1usize;
                i += 1;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                if depth > 0 {
                    return None;
                }
            }
            b'<' if bytes.get(i + 1) != Some(&b'<') => {
                i += 1 + bytes[i + 1..].iter().position(|&b| b == b'>')? + 1;
            }
            b'%' => {
                i += bytes[i..]
                    .iter()
                    .position(|&b| b == b'\r' || b == b'\n')
                    .unwrap_or(bytes.len() - i);
            }
            b'e' if bytes[i..].starts_with(b"endobj")
                && (i == from || delim(bytes[i - 1]))
                && bytes.get(i + 6).is_none_or(|&b| delim(b)) =>
            {
                return Some(i);
            }
            _ => i += 1,
        }
    }
    None
}

fn next_at(positions: &[usize], from: usize) -> Option<usize> {
    let i = positions.partition_point(|&p| p < from);
    positions.get(i).copied()
}

/// The `N G` header that ends just before the `obj` keyword at `p`: `(number, generation, start)`.
/// Both numbers must be separated from what precedes them by white space or a delimiter.
fn header_before(bytes: &[u8], p: usize) -> Option<(u32, u64, usize)> {
    let mut k = p;
    while k > 0 && is_ws(bytes[k - 1]) {
        k -= 1;
    }
    let gen_end = k;
    while k > 0 && bytes[k - 1].is_ascii_digit() {
        k -= 1;
    }
    if k == gen_end || gen_end - k > 5 {
        return None;
    }
    let gen: u64 = std::str::from_utf8(&bytes[k..gen_end]).ok()?.parse().ok()?;
    if k == 0 || !(is_ws(bytes[k - 1]) || is_delim(bytes[k - 1])) {
        return None;
    }
    while k > 0 && is_ws(bytes[k - 1]) {
        k -= 1;
    }
    let num_end = k;
    while k > 0 && bytes[k - 1].is_ascii_digit() {
        k -= 1;
    }
    if k == num_end || num_end - k > 10 {
        return None;
    }
    if k > 0 && !(is_ws(bytes[k - 1]) || is_delim(bytes[k - 1])) {
        return None;
    }
    let num: u32 = std::str::from_utf8(&bytes[k..num_end]).ok()?.parse().ok()?;
    Some((num, gen, k))
}

/// `startxref`: the byte offset of the newest cross-reference section. The keyword is searched
/// in the last KiB of the file (§7.5.5), so the search is bounded.
fn startxref(bytes: &[u8]) -> Option<u64> {
    let tail = bytes.len().saturating_sub(1024);
    let at = rfind(&bytes[tail..], b"startxref")? + tail;
    let (off, _) = int_at(bytes, at + "startxref".len())?;
    Some(off)
}

/// A non-negative integer entry of a dictionary.
fn dict_uint(d: &Dict, key: &str) -> Option<u64> {
    match d.get(&key.to_string()) {
        Some(Object::Int(n)) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

/// The three field widths of `/W`, each a non-negative integer.
fn w_array(d: &Dict) -> Option<[u64; 3]> {
    match d.get(&"W".to_string()) {
        Some(Object::Array(a)) if a.len() == 3 => {
            let mut w = [0u64; 3];
            for (slot, o) in w.iter_mut().zip(a) {
                match o {
                    Object::Int(n) if *n >= 0 => *slot = *n as u64,
                    _ => return None,
                }
            }
            Some(w)
        }
        _ => None,
    }
}

/// The `/Index` pairs `(first, count)` of an xref stream; the default is `[0 Size]`.
fn index_pairs(d: &Dict, default_count: u64) -> Option<Vec<(u64, u64)>> {
    match d.get(&"Index".to_string()) {
        Some(Object::Array(a)) => {
            if a.len() % 2 != 0 {
                return None;
            }
            a.chunks(2)
                .map(|p| match (&p[0], &p[1]) {
                    (Object::Int(f), Object::Int(c)) if *f >= 0 && *c >= 0 => {
                        Some((*f as u64, *c as u64))
                    }
                    _ => None,
                })
                .collect()
        }
        _ => Some(vec![(0, dict_uint(d, "Size").unwrap_or(default_count))]),
    }
}

/// A big-endian field of `width` bytes at `at`, folded through the checked step kernel. A zero
/// width reads as 0. A value that would not fit in 64 bits refuses.
fn read_be(data: &[u8], at: usize, width: u64) -> Option<u64> {
    let mut acc: u64 = 0;
    for &byte in data.get(at..at.checked_add(width as usize)?)? {
        let (ok, next) = k::xref_be_step(acc, byte);
        if !ok {
            return None;
        }
        acc = next;
    }
    Some(acc)
}

impl<'a> Loader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let len = bytes.len() as u64;
        Loader {
            bytes,
            len,
            endobj_at: occurrences(bytes, b"endobj"),
            endstream_at: occurrences(bytes, b"endstream"),
            map: Table::new(),
            streams: Vec::new(),
            stream_index: Table::new(),
            trailers: Vec::new(),
            visited: Table::new(),
            decode_budget: DOC_DECODE_BUDGET,
            decode_attempts: 0,
            work: Cell::new(0),
            work_limit: len.saturating_mul(16).saturating_add(4 << 20),
        }
    }

    /// Decode a stream against the document's remaining budget. Output is charged by its length;
    /// a failed inflate is charged its cap.
    fn decode_stream(&mut self, d: &Dict, raw: &[u8]) -> Option<Vec<u8>> {
        let cap = self.decode_budget.min(MAX_DECODED);
        self.decode_attempts += 1;
        let out = decode_within(d, raw, cap);
        let charge = match &out {
            Some(v) => v.len(),
            None if inflates(d) => cap,
            None => 0,
        };
        self.decode_budget = self.decode_budget.saturating_sub(charge);
        out
    }

    /// Set the entry of `num` unless a newer section already has one.
    fn insert_absent(&mut self, num: u32, entry: Entry) {
        if self.map.get(&num).is_none() {
            self.map.put(num, entry);
        }
    }

    /// The end of an object body: the next `endobj`, or the end of the input. Lexing an object
    /// stops there, so an unterminated array or dictionary cannot run on through later objects.
    fn body_end(&self, from: usize) -> usize {
        let raw = next_at(&self.endobj_at, from).unwrap_or(self.bytes.len());
        // Most bodies hold no string or comment before their `endobj`; only scan when one could.
        let head = &self.bytes[from.min(raw)..raw];
        if !head.iter().any(|b| matches!(b, b'(' | b'<' | b'%')) {
            return raw;
        }
        endobj_token(self.bytes, from).unwrap_or(raw)
    }

    /// An integer object at the in-use offset of `num`, read without following streams (so a
    /// `/Length` reference cannot recurse).
    fn direct_int(&self, num: u32, gen: u16) -> Option<u64> {
        let &Entry::InUse { offset, gen: have } = self.map.get(&num)? else {
            return None;
        };
        if have != u64::from(gen) {
            return None;
        }
        if !k::xref_offset_ok(offset, self.len) {
            return None;
        }
        let (_, _, body) = header_at(self.bytes, offset as usize)?;
        let mut lex = Lex::new(&self.bytes[..self.body_end(body)]);
        lex.i = body;
        let first = lex.next()?;
        match parse_value(&mut lex, first, 0) {
            Object::Int(n) if n >= 0 => Some(n as u64),
            _ => None,
        }
    }

    /// The raw bytes of the stream whose `stream` keyword ends at `kw`. `/Length` (direct or a
    /// reference to an integer) is used when it ends inside the file; otherwise the first
    /// `endstream` after the data, with one EOL before it removed.
    fn stream_raw(&self, dict: &Dict, kw: usize) -> Option<Vec<u8>> {
        let bytes = self.bytes;
        let mut start = kw + "stream".len();
        match bytes.get(start) {
            Some(b'\r') => {
                start += 1;
                if bytes.get(start) == Some(&b'\n') {
                    start += 1;
                }
            }
            Some(b'\n') => start += 1,
            _ => {}
        }
        let length = match dict.get(&"Length".to_string()) {
            Some(Object::Int(n)) if *n >= 0 => Some(*n as u64),
            Some(Object::Ref(m, g)) => self.direct_int(*m, *g),
            _ => None,
        };
        if let Some(length) = length {
            let (ok, end) = k::stream_extent(start as u64, length, self.len);
            if ok {
                return Some(bytes[start..end as usize].to_vec());
            }
        }
        let mut end = next_at(&self.endstream_at, start)?;
        if end > start && bytes[end - 1] == b'\n' {
            end -= 1;
        }
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        Some(bytes[start..end].to_vec())
    }

    /// The indirect object whose header is at `pos`: `(number, generation, object)`. A stream is
    /// returned as `Object::Stream` with its raw bytes.
    fn object_at(&self, pos: usize) -> Option<(u32, u64, Object)> {
        let (num, gen, body) = header_at(self.bytes, pos)?;
        let end = self.body_end(body);
        let work = self.work.get().saturating_add((end - body) as u64);
        if work > self.work_limit {
            return None;
        }
        self.work.set(work);
        let mut lex = Lex::new(&self.bytes[..end]);
        lex.i = body;
        let first = lex.next()?;
        let obj = parse_value(&mut lex, first, 0);
        if let Object::Dict(d) = obj {
            lex.skip_ws();
            if self
                .bytes
                .get(lex.i..)
                .is_some_and(|rest| rest.starts_with(b"stream"))
            {
                let raw = self.stream_raw(&d, lex.i)?;
                return Some((num, gen, Object::Stream(d, raw)));
            }
            return Some((num, gen, Object::Dict(d)));
        }
        Some((num, gen, obj))
    }

    /// A cross-reference section at `pos`: a table, or an xref stream. Returns its trailer.
    fn section_at(&mut self, pos: usize) -> Option<Dict> {
        let p = skip_ws(self.bytes, pos);
        if self.bytes[p..].starts_with(b"xref") {
            self.table_section(p + 4)
        } else {
            self.stream_section(p)
        }
    }

    /// An xref table (§7.5.4) from the first subsection header at `p`, through `trailer`.
    /// Entries of an `XRefStm` (hybrid files, §7.5.8.4) are read first and take precedence.
    fn table_section(&mut self, p: usize) -> Option<Dict> {
        let bytes = self.bytes;
        let mut entries: Vec<(u32, Entry)> = Vec::new();
        let mut q = p;
        loop {
            let r = skip_ws(bytes, q);
            if bytes[r..].starts_with(b"trailer") {
                q = r + "trailer".len();
                break;
            }
            let (first, a) = int_at(bytes, r)?;
            let (count, b) = int_at(bytes, a)?;
            q = b;
            for j in 0..count {
                let (off, c) = int_at(bytes, q)?;
                let (gen, d) = int_at(bytes, c)?;
                let kw = skip_ws(bytes, d);
                let flag = *bytes.get(kw)?;
                q = kw + 1;
                let num = first.checked_add(j)?;
                if num > u32::MAX as u64 {
                    return None;
                }
                let entry = match flag {
                    b'n' if k::xref_offset_ok(off, self.len) => Entry::InUse { offset: off, gen },
                    b'n' => Entry::Null,
                    b'f' => Entry::Free,
                    _ => return None,
                };
                entries.push((num as u32, entry));
            }
        }
        // The trailer ends before the next `startxref`; lexing stops there rather than at end of
        // file, and the scan is charged to the work budget like an object body.
        let start = skip_ws(bytes, q);
        let limit = bytes[start..]
            .windows("startxref".len())
            .position(|w| w == b"startxref")
            .map_or(bytes.len(), |p| start + p);
        let work = self.work.get().saturating_add((limit - start) as u64);
        if work > self.work_limit {
            return None;
        }
        self.work.set(work);
        let mut lex = Lex::new(&bytes[..limit]);
        lex.i = start;
        if lex.next()? != Tok::DictOpen {
            return None;
        }
        let trailer = parse_dict(&mut lex, 1);
        if let Some(xs) = dict_uint(&trailer, "XRefStm") {
            let seen = self.visited.has(&xs);
            if k::xref_prev_ok(xs, self.len, seen) {
                self.visited.put(xs, true);
                self.stream_section(xs as usize);
            }
        }
        for (num, entry) in entries {
            self.insert_absent(num, entry);
        }
        Some(trailer)
    }

    /// An xref stream (§7.5.8): its rows, decoded through the filter and predictor, give the
    /// entries of the objects in its `/Index`. Rows are read in order, each checked by the row
    /// kernel before its fields are read.
    fn stream_section(&mut self, pos: usize) -> Option<Dict> {
        let (_, _, obj) = self.object_at(pos)?;
        let Object::Stream(d, raw) = obj else {
            return None;
        };
        let data = self.decode_stream(&d, &raw)?;
        let w = w_array(&d)?;
        let (ok, row_len) = k::xref_row_len(w[0], w[1], w[2]);
        if !ok {
            return None;
        }
        let data_len = data.len() as u64;
        let rows_available = data_len / row_len;
        let index = index_pairs(&d, rows_available)?;
        let mut row: u64 = 0;
        'sections: for (first, count) in index {
            for j in 0..count {
                let (ok, start) = k::row_start(row, row_len, data_len);
                if !ok {
                    break 'sections;
                }
                let Some(num) = first.checked_add(j).filter(|n| *n <= u32::MAX as u64) else {
                    break 'sections;
                };
                let entry =
                    Self::row_entry(&data, start as usize, w, self.len).unwrap_or(Entry::Null);
                self.insert_absent(num as u32, entry);
                row += 1;
            }
        }
        Some(d)
    }

    /// The entry a row encodes (Table 18): the type field's default and code come from the kernels.
    /// An in-use offset outside the file resolves to null.
    fn row_entry(data: &[u8], start: usize, w: [u64; 3], file_len: u64) -> Option<Entry> {
        let w1_zero = w[0] == 0;
        let f1 = if w1_zero {
            0
        } else {
            read_be(data, start, w[0])?
        };
        let f2 = read_be(data, start + w[0] as usize, w[1])?;
        let f3 = read_be(data, start + (w[0] + w[1]) as usize, w[2])?;
        Some(match k::xref_entry_type(w1_zero, f1) {
            0 => Entry::Free,
            1 if k::xref_offset_ok(f2, file_len) => Entry::InUse {
                offset: f2,
                gen: f3,
            },
            1 => Entry::Null,
            2 => Entry::Compressed {
                stream: f2,
                index: f3,
            },
            _ => Entry::Null,
        })
    }

    /// Follow the chain of sections from `start`: each `/Prev` offset is read once, and only when
    /// it lies inside the file. `false` when the first section does not read.
    fn load_chain(&mut self, start: u64) -> bool {
        let mut next = Some(start);
        while let Some(off) = next {
            let seen = self.visited.has(&off);
            if !k::xref_prev_ok(off, self.len, seen) {
                break;
            }
            self.visited.put(off, true);
            let Some(trailer) = self.section_at(off as usize) else {
                break;
            };
            next = dict_uint(&trailer, "Prev");
            self.trailers.push(trailer);
        }
        !self.trailers.is_empty()
    }

    /// The `/Root` of the newest trailer that names one.
    fn root_num(&self) -> Option<u32> {
        self.trailers
            .iter()
            .find_map(|t| match t.get(&"Root".to_string()) {
                Some(Object::Ref(r, _)) => Some(*r),
                _ => None,
            })
    }

    /// Whether object stream `s` has a direct entry: in use, generation 0, inside the file.
    fn stream_direct(&self, s: u32) -> bool {
        matches!(
            self.map.get(&s),
            Some(&Entry::InUse { offset, gen }) if gen == 0 && k::xref_offset_ok(offset, self.len)
        )
    }

    /// The object stream `s`, decoded and with its header read; `None` when it refuses. The load
    /// (its success or its refusal) is cached, so a stream is decoded at most once.
    fn load_objstm(&mut self, s: u32) -> Option<usize> {
        if let Some(&cached) = self.stream_index.get(&s) {
            return cached;
        }
        let loaded = self.read_objstm(s);
        self.stream_index.put(s, loaded);
        loaded
    }

    /// Decode the object stream `s` and read its header. The header is accepted by the header
    /// kernel before anything is allocated for its pairs. A stream without `/N` is loaded with no
    /// pairs and `has_n` false, so no object is ever read from it.
    fn read_objstm(&mut self, s: u32) -> Option<usize> {
        if !self.stream_direct(s) {
            return None;
        }
        let &Entry::InUse { offset, .. } = self.map.get(&s)? else {
            return None;
        };
        let (num, g, obj) = self.object_at(offset as usize)?;
        if num != s || g != 0 {
            return None;
        }
        let Object::Stream(d, raw) = obj else {
            return None;
        };
        let declared_n = dict_uint(&d, "N");
        let has_n = declared_n.is_some();
        let n = declared_n.unwrap_or(0);
        let first = dict_uint(&d, "First")?;
        let data = self.decode_stream(&d, &raw)?;
        let len = data.len() as u64;
        if !k::objstm_header_ok(first, len, n) {
            return None;
        }
        let header = &data[..first as usize];
        let mut pairs = Vec::with_capacity(n as usize);
        let mut p = 0usize;
        for _ in 0..n {
            let (objnum, a) = int_at(header, p)?;
            let (off, b) = int_at(header, a)?;
            p = b;
            if objnum > u32::MAX as u64 {
                return None;
            }
            pairs.push((objnum as u32, off));
        }
        let idx = self.streams.len();
        self.streams.push(ObjStm {
            data,
            first,
            has_n,
            pairs,
        });
        Some(idx)
    }

    /// Object `num`, stored at `index` of object stream `s`. The reference kernel gates every
    /// condition on the stream (its entry, self-reference, `/N`, the index); the extent kernel
    /// decides whether the object lies inside the stream; the header must name `num`.
    fn load_compressed(&mut self, num: u32, s: u32, index: u64) -> Option<Object> {
        let stream_direct = self.stream_direct(s);
        // Only a stream with a direct entry is loaded at all.
        let idx = if stream_direct {
            self.load_objstm(s)
        } else {
            None
        };
        let (has_n, n) = match idx {
            Some(i) => (self.streams[i].has_n, self.streams[i].pairs.len() as u64),
            None => (false, 0),
        };
        if !k::objstm_ref_ok(stream_direct, s == num, has_n, n, index) {
            return None;
        }
        let st = &self.streams[idx?];
        let i = index as usize;
        let (objnum, off) = st.pairs[i];
        if objnum != num {
            return None;
        }
        let has_next = i + 1 < st.pairs.len();
        let next_off = if has_next { st.pairs[i + 1].1 } else { 0 };
        let (ok, start, end) =
            k::objstm_span(st.first, off, has_next, next_off, st.data.len() as u64);
        if !ok {
            return None;
        }
        let mut lex = Lex::new(&st.data[start as usize..end as usize]);
        let first = lex.next()?;
        Some(parse_value(&mut lex, first, 0))
    }

    /// Load every object the table names into the document.
    fn load_all(&mut self, doc: &mut Document) {
        let entries: Vec<(u32, Entry)> = self.map.iter().map(|(n, e)| (*n, *e)).collect();
        for (num, entry) in entries {
            let loaded = match entry {
                Entry::InUse { offset, gen } if k::xref_offset_ok(offset, self.len) => {
                    match self.object_at(offset as usize) {
                        Some((n2, g2, obj)) if n2 == num && g2 == gen => Some((obj, gen)),
                        _ => None,
                    }
                }
                Entry::Compressed { stream, index } => {
                    if stream > u32::MAX as u64 {
                        None
                    } else {
                        self.load_compressed(num, stream as u32, index)
                            .map(|o| (o, 0))
                    }
                }
                _ => None,
            };
            if let Some((obj, gen)) = loaded {
                doc.set_object(num, obj);
                doc.gens.put(num, gen as u16);
            }
        }
    }

    /// Last-resort recovery: every `N G obj` header, in file order, becomes an in-use entry, so the
    /// last header of a number is its newest revision. Object streams found this way contribute
    /// their members where no direct entry exists. The last `trailer` dictionary is read too.
    fn recover(&mut self) {
        let bytes = self.bytes;
        self.map = Table::new();
        let mut pos = 0usize;
        while let Some(p) = find_from(bytes, pos, b"obj") {
            pos = p + 3;
            if bytes
                .get(p + 3)
                .is_some_and(|&b| !(is_ws(b) || is_delim(b)))
            {
                continue;
            }
            let Some((num, gen, start)) = header_before(bytes, p) else {
                continue;
            };
            if self.object_at(start).is_some() {
                self.map.put(
                    num,
                    Entry::InUse {
                        offset: start as u64,
                        gen,
                    },
                );
            }
        }
        // The object-stream cache belongs to the map it was read against: start it afresh.
        self.stream_index = Table::new();
        let mut in_use: Vec<(u32, usize)> = Vec::new();
        for (num, entry) in self.map.iter() {
            if let Entry::InUse { offset, .. } = entry {
                in_use.push((*num, *offset as usize));
            }
        }
        for (s, offset) in in_use {
            let is_objstm = match self.object_at(offset) {
                Some((_, _, Object::Stream(d, _))) => {
                    matches!(d.get(&"Type".to_string()), Some(Object::Name(t)) if t == "ObjStm")
                }
                _ => false,
            };
            if !is_objstm {
                continue;
            }
            if let Some(idx) = self.load_objstm(s) {
                let pairs = self.streams[idx].pairs.clone();
                for (i, (objnum, _)) in pairs.into_iter().enumerate() {
                    if self.map.get(&objnum).is_none() {
                        self.map.put(
                            objnum,
                            Entry::Compressed {
                                stream: s as u64,
                                index: i as u64,
                            },
                        );
                    }
                }
            }
        }
        if let Some(at) = rfind(bytes, b"trailer") {
            let mut lex = Lex::new(bytes);
            lex.i = at + "trailer".len();
            if lex.next() == Some(Tok::DictOpen) {
                self.trailers.push(parse_dict(&mut lex, 1));
            }
        }
    }
}

/// The catalog when the trailer names none: the smallest-numbered `/Type /Catalog` dictionary.
fn catalog_scan(doc: &Document) -> u32 {
    let mut best: Option<u32> = None;
    for (num, o) in doc.objects.iter() {
        if let Object::Dict(d) = o {
            if matches!(d.get(&"Type".to_string()), Some(Object::Name(n)) if n == "Catalog") {
                best = Some(best.map_or(*num, |b| b.min(*num)));
            }
        }
    }
    best.unwrap_or(0)
}

/// Load a PDF through its cross-reference structure, falling back to header recovery when the
/// chain does not yield a catalog.
pub(super) fn load(bytes: &[u8]) -> Result<Document, PdfError> {
    let mut l = Loader::new(bytes);
    let chained = startxref(bytes).is_some_and(|off| l.load_chain(off));
    let recovered = !chained || l.root_num().is_none();
    if recovered {
        l.recover();
    }
    let mut doc = Document::default();
    l.load_all(&mut doc);
    // A table whose offsets do not name the objects (an off-by-one table, say) loads nothing
    // for the catalog: the headers in the file are then the only reliable map.
    let root_loaded = l.root_num().is_some_and(|r| doc.object(r).is_some());
    if !root_loaded && !recovered {
        l.recover();
        l.load_all(&mut doc);
    }
    doc.root = match l.root_num() {
        Some(r) if doc.object(r).is_some() => r,
        _ => catalog_scan(&doc),
    };
    Ok(doc)
}

#[cfg(test)]
#[path = "xref_tests.rs"]
mod tests;
