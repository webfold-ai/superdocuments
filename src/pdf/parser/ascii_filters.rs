//! ASCII85Decode and ASCIIHexDecode (PDF 32000-1 §7.4.2, §7.4.3). Both are total on arbitrary
//! bytes: malformed input yields `None` rather than a panic. Hex halves its input and ASCII85 maps
//! five bytes to four, except that `z` maps one byte to four, so ASCII85 takes an explicit output
//! ceiling.

/// Ceiling on the decoded size of one ASCII85 stream (a run of `z` otherwise expands 4x).
pub const MAX_ASCII85_OUT: usize = 64 * 1024 * 1024;

#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

/// ASCII85: `z` is four zero bytes, `~>` ends the data, a final group of n chars (2..=4) yields
/// n-1 bytes. An optional leading `<~` is accepted.
pub fn ascii85(input: &[u8]) -> Option<Vec<u8>> {
    ascii85_capped(input, MAX_ASCII85_OUT)
}

/// `ascii85` with an explicit output ceiling; `None` once the output would exceed it.
pub fn ascii85_capped(input: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut data = input;
    while let [first, rest @ ..] = data {
        if is_ws(*first) {
            data = rest;
        } else {
            break;
        }
    }
    if data.starts_with(b"<~") {
        data = &data[2..];
    }
    let mut out = Vec::with_capacity((data.len() / 5 * 4 + 4).min(max_out));
    let mut group = [0u8; 5];
    let mut n = 0usize;
    for &b in data {
        if is_ws(b) {
            continue;
        }
        match b {
            b'~' => break,
            b'z' => {
                if n != 0 {
                    return None;
                }
                if out.len() + 4 > max_out {
                    return None;
                }
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
            b'!'..=b'u' => {
                group[n] = b - b'!';
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |a, &d| a * 85 + d as u64);
                    if v > u32::MAX as u64 {
                        return None;
                    }
                    if out.len() + 4 > max_out {
                        return None;
                    }
                    out.extend_from_slice(&(v as u32).to_be_bytes());
                    n = 0;
                }
            }
            _ => return None,
        }
    }
    if n == 1 {
        return None;
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = 84; // pad with 'u'
        }
        let v = group.iter().fold(0u64, |a, &d| a * 85 + d as u64);
        if v > u32::MAX as u64 {
            return None;
        }
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    Some(out)
}

/// ASCIIHex: pairs of hex digits, `>` ends the data, an odd final digit is padded with 0.
pub fn ascii_hex(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 2 + 1);
    let mut hi: Option<u8> = None;
    for &b in input {
        if is_ws(b) {
            continue;
        }
        if b == b'>' {
            break;
        }
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        match hi.take() {
            Some(h) => out.push(h << 4 | d),
            None => hi = Some(d),
        }
    }
    if let Some(h) = hi {
        out.push(h << 4);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a85_known_vectors() {
        assert_eq!(ascii85(b"87cURD]i,\"Ebo80~>").unwrap(), b"Hello World!".to_vec());
        assert_eq!(ascii85(b"<~87cURD]i,\"Ebo80~>").unwrap(), b"Hello World!".to_vec());
        assert_eq!(ascii85(b"z~>").unwrap(), vec![0, 0, 0, 0]);
        assert_eq!(ascii85(b"87cU\nRD]i,\"Ebo80").unwrap(), b"Hello World!".to_vec());
    }
    #[test]
    fn a85_partial_group_and_errors() {
        assert_eq!(ascii85(b"@/~>").unwrap(), b"a".to_vec());
        assert_eq!(ascii85(b"~>").unwrap(), Vec::<u8>::new());
        assert!(ascii85(b"8~>").is_none(), "single trailing char is invalid");
        assert!(ascii85(b"87zcU~>").is_none(), "z inside a group is invalid");
        assert!(ascii85(b"s8W-!~>").is_some(), "max group 0xFFFFFFFF");
        assert!(ascii85(b"s8W-\"~>").is_none(), "group above 2^32-1 is invalid");
        assert!(ascii85(b"\x80~>").is_none());
    }
    #[test]
    fn a85_z_runs_are_capped() {
        let zs = vec![b'z'; 1000];
        assert_eq!(ascii85_capped(&zs, 4000).unwrap().len(), 4000);
        assert!(ascii85_capped(&zs, 3999).is_none());
    }
    #[test]
    fn hex_vectors() {
        assert_eq!(ascii_hex(b"48 65 6C6c 6F>").unwrap(), b"Hello".to_vec());
        assert_eq!(ascii_hex(b"7>").unwrap(), vec![0x70]);
        assert!(ascii_hex(b"4g>").is_none());
    }
}
