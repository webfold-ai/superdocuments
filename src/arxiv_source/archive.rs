//! Safe, bounded unpacking of an arXiv e-print: a gzip-compressed tar archive, or a gzip-compressed
//! single TeX file. Everything is held in memory. Nothing is written to disk.
//!
//! Rejected, with an error and no partial result:
//! - a path that is absolute, contains `..` or a backslash, or is empty
//! - a symbolic or hard link
//! - a gzip stream whose CRC or length does not match its trailer
//! - a limit exceeded: compressed size, unpacked size, file size, entry count, or the
//!   compression ratio (unpacked size over compressed size)

use std::fmt;

use crate::pdf::parser::inflate::inflate_capped;

/// Bounds for unpacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Largest archive accepted, compressed.
    pub max_compressed: usize,
    /// Largest total of unpacked file contents.
    pub max_unpacked: usize,
    /// Largest single file.
    pub max_file: usize,
    /// Most entries (files and directories).
    pub max_entries: usize,
    /// Largest unpacked-to-compressed ratio. Checked once the output passes 1 MiB.
    pub max_ratio: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_compressed: 64 << 20,
            max_unpacked: 256 << 20,
            max_file: 64 << 20,
            max_entries: 4096,
            max_ratio: 200,
        }
    }
}

/// One regular file from the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Normalised relative path, with `/` separators and no `.` or empty components.
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Neither gzip nor a tar archive.
    NotAnArchive,
    /// The data is malformed. The message says which part.
    Corrupt(&'static str),
    /// A limit in `Limits` was reached.
    Limit(&'static str),
    /// An unsafe path, with the path as it appeared.
    UnsafePath(String),
    /// A symbolic or hard link, with its name.
    Link(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotAnArchive => write!(f, "not a gzip stream or a tar archive"),
            Error::Corrupt(what) => write!(f, "corrupt archive: {what}"),
            Error::Limit(what) => write!(f, "archive limit reached: {what}"),
            Error::UnsafePath(p) => write!(f, "unsafe path in archive: {p:?}"),
            Error::Link(p) => write!(f, "links are not accepted: {p:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Unpacks an e-print. A gzip stream holding a tar archive gives its regular files. A gzip
/// stream holding anything else is one TeX file, named `main.tex`. A plain (uncompressed) tar
/// archive is also accepted.
pub fn unpack(data: &[u8], limits: &Limits) -> Result<Vec<Entry>, Error> {
    if data.len() > limits.max_compressed {
        return Err(Error::Limit("compressed size"));
    }
    let gzipped = data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b;
    let raw = if gzipped {
        gunzip(data, limits)?
    } else {
        data.to_vec()
    };
    if is_tar(&raw) {
        return read_tar(&raw, limits, data.len());
    }
    if gzipped {
        if raw.len() > limits.max_file {
            return Err(Error::Limit("file size"));
        }
        return Ok(vec![Entry {
            path: "main.tex".into(),
            bytes: raw,
        }]);
    }
    Err(Error::NotAnArchive)
}

fn is_tar(raw: &[u8]) -> bool {
    raw.len() >= 512 && (raw.get(257..262) == Some(b"ustar") || looks_like_header(&raw[..512]))
}

/// Old-style tar headers have no magic. A header is accepted when its checksum matches.
fn looks_like_header(h: &[u8]) -> bool {
    header_checksum_ok(h)
}

fn header_checksum_ok(h: &[u8]) -> bool {
    let Ok(stored) = parse_octal(&h[148..156]) else {
        return false;
    };
    let sum: u64 = h
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                u64::from(b' ')
            } else {
                u64::from(b)
            }
        })
        .sum();
    sum == stored
}

// ----- gzip -----------------------------------------------------------------------------------

/// Decompresses the first gzip member and checks its CRC and length.
fn gunzip(data: &[u8], limits: &Limits) -> Result<Vec<u8>, Error> {
    let body = gzip_body(data)?;
    if body.len() < 8 {
        return Err(Error::Corrupt("gzip trailer missing"));
    }
    let (deflate, trailer) = body.split_at(body.len() - 8);
    let want_crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let want_len = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    // The output ceiling follows the compressed size, so a corrupt stream fails quickly instead
    // of decoding up to the global unpacked limit.
    let ceiling = limits
        .max_unpacked
        .min((data.len().saturating_mul(limits.max_ratio)).max(1 << 20));
    let out = inflate_capped(deflate, ceiling).ok_or(Error::Corrupt(
        "deflate stream is invalid or expands beyond the ratio or size limit",
    ))?;
    if out.len() > 1 << 20 && out.len() > data.len().saturating_mul(limits.max_ratio) {
        return Err(Error::Limit("compression ratio"));
    }
    if crc32(&out) != want_crc {
        return Err(Error::Corrupt("gzip CRC mismatch"));
    }
    if out.len() as u32 != want_len {
        return Err(Error::Corrupt("gzip length mismatch"));
    }
    Ok(out)
}

/// The bytes after the gzip header (the deflate data and the trailer).
fn gzip_body(data: &[u8]) -> Result<&[u8], Error> {
    if data.len() < 10 || data[2] != 8 {
        return Err(Error::Corrupt("gzip header"));
    }
    let flags = data[3];
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        let xlen = usize::from(*data.get(pos).ok_or(Error::Corrupt("gzip header"))?)
            | (usize::from(*data.get(pos + 1).ok_or(Error::Corrupt("gzip header"))?) << 8);
        pos += 2 + xlen;
    }
    for flag in [0x08u8, 0x10] {
        if flags & flag != 0 {
            while *data.get(pos).ok_or(Error::Corrupt("gzip header"))? != 0 {
                pos += 1;
            }
            pos += 1;
        }
    }
    if flags & 0x02 != 0 {
        pos += 2;
    }
    data.get(pos..).ok_or(Error::Corrupt("gzip header"))
}

// ----- tar ------------------------------------------------------------------------------------

fn read_tar(raw: &[u8], limits: &Limits, compressed_len: usize) -> Result<Vec<Entry>, Error> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut pos = 0usize;
    let mut total = 0usize;
    let mut long_name: Option<String> = None;
    let mut pax_path: Option<String> = None;
    while pos + 512 <= raw.len() {
        let header = &raw[pos..pos + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        if !header_checksum_ok(header) {
            return Err(Error::Corrupt("tar header checksum"));
        }
        let size = usize::try_from(parse_octal(&header[124..136])?)
            .map_err(|_| Error::Corrupt("tar entry size"))?;
        let typeflag = header[156];
        let name = if header.get(257..262) == Some(b"ustar") {
            let prefix = cstr(&header[345..500]);
            let base = cstr(&header[0..100]);
            if prefix.is_empty() {
                base
            } else {
                format!("{prefix}/{base}")
            }
        } else {
            cstr(&header[0..100])
        };
        let data_start = pos + 512;
        let data_end = data_start
            .checked_add(size)
            .ok_or(Error::Corrupt("tar entry size"))?;
        if data_end > raw.len() {
            return Err(Error::Corrupt("tar entry runs past the end of the archive"));
        }
        let data = &raw[data_start..data_end];
        pos = data_start + size.div_ceil(512) * 512;
        match typeflag {
            b'L' => {
                long_name = Some(cstr(data));
                continue;
            }
            b'x' => {
                pax_path = pax_record(data, "path");
                continue;
            }
            b'g' => continue,
            b'1' | b'2' => return Err(Error::Link(name)),
            b'5' => {
                // A directory may be named "./"; only traversal and absolute names are refused.
                if !name.trim_matches(['.', '/']).is_empty() {
                    validate_path(&name)?;
                }
                continue;
            }
            b'0' | 0 => {}
            _ => continue,
        }
        let raw_name = pax_path.take().or_else(|| long_name.take()).unwrap_or(name);
        let path = validate_path(&raw_name)?;
        if entries.len() >= limits.max_entries {
            return Err(Error::Limit("entry count"));
        }
        if size > limits.max_file {
            return Err(Error::Limit("file size"));
        }
        total += size;
        if total > limits.max_unpacked {
            return Err(Error::Limit("unpacked size"));
        }
        if total > 1 << 20 && total > compressed_len.saturating_mul(limits.max_ratio) {
            return Err(Error::Limit("compression ratio"));
        }
        entries.push(Entry {
            path,
            bytes: data.to_vec(),
        });
    }
    Ok(entries)
}

/// The value of one key in a pax extended header (`<len> key=value\n` records).
fn pax_record(data: &[u8], key: &str) -> Option<String> {
    let mut i = 0usize;
    while i < data.len() {
        let space = data[i..].iter().position(|&b| b == b' ')? + i;
        let len: usize = std::str::from_utf8(&data[i..space]).ok()?.parse().ok()?;
        let end = i.checked_add(len)?;
        if end > data.len() || end <= space {
            return None;
        }
        let record = &data[space + 1..end];
        let record = record.strip_suffix(b"\n").unwrap_or(record);
        if let Some(eq) = record.iter().position(|&b| b == b'=') {
            if &record[..eq] == key.as_bytes() {
                return Some(String::from_utf8_lossy(&record[eq + 1..]).into_owned());
            }
        }
        i = end;
    }
    None
}

/// Accepts a relative path with `/` separators. Rejects absolute paths, `..`, backslashes and
/// NUL bytes, and returns the normalised form.
pub(crate) fn validate_path(p: &str) -> Result<String, Error> {
    let unsafe_path = || Error::UnsafePath(p.to_string());
    if p.is_empty() || p.len() > 1024 || p.contains('\\') || p.contains('\0') {
        return Err(unsafe_path());
    }
    if p.starts_with('/') || p.as_bytes().get(1) == Some(&b':') {
        return Err(unsafe_path());
    }
    let mut parts = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(unsafe_path()),
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(unsafe_path());
    }
    Ok(parts.join("/"))
}

fn cstr(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn parse_octal(field: &[u8]) -> Result<u64, Error> {
    if field.first().is_some_and(|&b| b & 0x80 != 0) {
        return Err(Error::Limit("base-256 tar size is not supported"));
    }
    let mut value: u64 = 0;
    let mut seen = false;
    for &b in field {
        match b {
            b' ' | 0 if !seen => continue,
            b' ' | 0 => break,
            b'0'..=b'7' => {
                seen = true;
                value = value
                    .checked_mul(8)
                    .and_then(|v| v.checked_add(u64::from(b - b'0')))
                    .ok_or(Error::Corrupt("tar number overflow"))?;
            }
            _ => return Err(Error::Corrupt("tar number")),
        }
    }
    Ok(value)
}

// ----- crc32 ----------------------------------------------------------------------------------

const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// CRC-32 (IEEE, as used by gzip).
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

#[cfg(test)]
pub(crate) mod test_support {
    /// A gzip stream of `data` using stored (uncompressed) deflate blocks.
    pub fn gzip_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
        let mut chunks = data.chunks(65535).peekable();
        if chunks.peek().is_none() {
            out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
        }
        while let Some(chunk) = chunks.next() {
            let last = u8::from(chunks.peek().is_none());
            out.push(last);
            let len = chunk.len() as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(chunk);
        }
        out.extend_from_slice(&super::crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out
    }

    /// A ustar archive from (name, bytes, typeflag) triples.
    pub fn tar(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data, kind) in entries {
            let mut h = [0u8; 512];
            let nb = name.as_bytes();
            h[..nb.len().min(100)].copy_from_slice(&nb[..nb.len().min(100)]);
            let mode = b"0000644\0";
            h[100..108].copy_from_slice(mode);
            h[108..116].copy_from_slice(b"0000000\0");
            h[116..124].copy_from_slice(b"0000000\0");
            let size = format!("{:011o}\0", data.len());
            h[124..136].copy_from_slice(size.as_bytes());
            h[136..148].copy_from_slice(b"00000000000\0");
            h[156] = *kind;
            h[257..263].copy_from_slice(b"ustar\0");
            h[263..265].copy_from_slice(b"00");
            h[148..156].copy_from_slice(b"        ");
            let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
            let chk = format!("{sum:06o}\0 ");
            h[148..156].copy_from_slice(chk.as_bytes());
            out.extend_from_slice(&h);
            out.extend_from_slice(data);
            let pad = (512 - data.len() % 512) % 512;
            out.extend(std::iter::repeat_n(0u8, pad));
        }
        out.extend(std::iter::repeat_n(0u8, 1024));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{gzip_stored, tar};
    use super::*;

    #[test]
    fn stored_gzip_and_tar_round_trip() {
        let archive = tar(&[("main.tex", b"\\documentclass{article}".as_slice(), b'0')]);
        let gz = gzip_stored(&archive);
        let files = unpack(&gz, &Limits::default()).unwrap_or_default();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "main.tex");
        assert_eq!(files[0].bytes, b"\\documentclass{article}");
    }

    #[test]
    fn a_gzip_of_one_tex_file_is_main_tex() {
        let gz = gzip_stored(b"\\begin{document}x\\end{document}");
        let files = unpack(&gz, &Limits::default()).unwrap_or_default();
        assert_eq!(files[0].path, "main.tex");
    }

    #[test]
    fn crc_and_length_are_checked() {
        let mut gz = gzip_stored(b"hello world");
        let n = gz.len();
        gz[n - 8] ^= 1;
        assert_eq!(
            unpack(&gz, &Limits::default()),
            Err(Error::Corrupt("gzip CRC mismatch"))
        );
        let mut gz = gzip_stored(b"hello world");
        let n = gz.len();
        gz[n - 1] ^= 1;
        assert!(unpack(&gz, &Limits::default()).is_err());
    }

    #[test]
    fn traversal_absolute_and_odd_paths_are_rejected() {
        for bad in [
            "../escape.tex",
            "/etc/passwd",
            "a/../../b.tex",
            "a\\b.tex",
            "C:/x.tex",
            "",
        ] {
            let archive = tar(&[(bad, b"x".as_slice(), b'0')]);
            let err = unpack(&archive, &Limits::default());
            assert!(matches!(err, Err(Error::UnsafePath(_))), "{bad:?}: {err:?}");
        }
    }

    #[test]
    fn links_are_rejected() {
        let archive = tar(&[("main.tex", b"x".as_slice(), b'2')]);
        assert!(matches!(
            unpack(&archive, &Limits::default()),
            Err(Error::Link(_))
        ));
        let archive = tar(&[("main.tex", b"".as_slice(), b'1')]);
        assert!(matches!(
            unpack(&archive, &Limits::default()),
            Err(Error::Link(_))
        ));
    }

    #[test]
    fn dot_components_are_normalised() {
        assert_eq!(validate_path("./a//b/./c.tex").as_deref(), Ok("a/b/c.tex"));
    }

    #[test]
    fn limits_apply_to_size_count_and_total() {
        let archive = tar(&[("a.tex", b"12345".as_slice(), b'0')]);
        let small = Limits {
            max_file: 4,
            ..Limits::default()
        };
        assert_eq!(unpack(&archive, &small), Err(Error::Limit("file size")));
        let few = Limits {
            max_entries: 1,
            ..Limits::default()
        };
        let two = tar(&[
            ("a.tex", b"1".as_slice(), b'0'),
            ("b.tex", b"2".as_slice(), b'0'),
        ]);
        assert_eq!(unpack(&two, &few), Err(Error::Limit("entry count")));
        let total = Limits {
            max_unpacked: 6,
            ..Limits::default()
        };
        assert_eq!(unpack(&archive, &total).map(|v| v.len()), Ok(1));
        let total = Limits {
            max_unpacked: 4,
            ..Limits::default()
        };
        assert_eq!(unpack(&archive, &total), Err(Error::Limit("unpacked size")));
        let compressed = Limits {
            max_compressed: 3,
            ..Limits::default()
        };
        assert_eq!(
            unpack(&archive, &compressed),
            Err(Error::Limit("compressed size"))
        );
    }

    #[test]
    fn plain_tar_is_accepted_and_garbage_is_not_an_archive() {
        let archive = tar(&[("main.tex", b"\\relax".as_slice(), b'0')]);
        assert_eq!(unpack(&archive, &Limits::default()).map(|v| v.len()), Ok(1));
        assert_eq!(
            unpack(
                b"this is plain text, not an archive at all, nothing more here.",
                &Limits::default()
            ),
            Err(Error::NotAnArchive)
        );
    }

    #[test]
    fn pax_long_paths_are_used() {
        let long = format!("{}/main.tex", "d".repeat(150));
        let record = {
            let body = format!("path={long}\n");
            let len = body.len() + 2 + 1;
            // "<len> <body>" where the length includes its own digits.
            let mut n = len;
            loop {
                let s = format!("{n} ");
                if s.len() + body.len() == n {
                    break format!("{s}{body}");
                }
                n = s.len() + body.len();
            }
        };
        let archive = tar(&[
            ("PaxHeader", record.as_bytes(), b'x'),
            ("short", b"\\relax".as_slice(), b'0'),
        ]);
        let files = unpack(&archive, &Limits::default()).unwrap_or_default();
        assert_eq!(files[0].path, long);
    }
}
