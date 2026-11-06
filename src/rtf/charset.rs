//! Code-page handling for RTF bytes: `\ansicpg`, `\fcharset` numbers, per-font code pages,
//! Mac Roman, the Symbol font, and multi-byte sequences split across consecutive `\'hh` escapes.
//!
//! Decoding goes through `encoding_rs` for the Windows and CJK code pages it knows. Mac Roman and
//! the Symbol font are tables kept here.

use encoding_rs::Encoding;

/// How bytes in a code page become characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One byte per character, decoded by `encoding_rs`.
    SingleByte(&'static Encoding),
    /// Lead and trail byte pairs, decoded by `encoding_rs`.
    Dbcs(&'static Encoding),
    /// UTF-8 sequences, which may be split across `\'hh` escapes.
    Utf8,
    /// Apple Mac OS Roman (code page 10000). Not in `encoding_rs`, so it has its own table.
    MacRoman,
    /// Windows Symbol font (charset 2). Only the bullet and the ASCII punctuation range are mapped.
    Symbol,
}

/// A code page as used for decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodePage {
    pub number: u32,
    pub kind: Kind,
}

/// The meaning of an `\fcharset` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// ANSI or default: use the document code page (`\ansicpg`).
    Document,
    Symbol,
    Number(u32),
}

/// Windows-1252. The fallback for unknown code pages.
pub fn windows_1252() -> CodePage {
    CodePage {
        number: 1252,
        kind: Kind::SingleByte(encoding_rs::WINDOWS_1252),
    }
}

/// The Symbol font. Windows uses code page 42 for it.
pub fn symbol() -> CodePage {
    CodePage {
        number: 42,
        kind: Kind::Symbol,
    }
}

/// Code page numbers that RTF files use but this module cannot decode. Reported as "not supported"
/// rather than "unknown".
pub fn is_known_unsupported(number: u32) -> bool {
    matches!(
        number,
        37 | 437
            | 500
            | 708
            | 720
            | 737
            | 775
            | 850
            | 852
            | 855
            | 857
            | 858
            | 860..=866
            | 869
            | 1361
            | 10001..=10006
            | 10008
            | 10010
            | 10017
            | 10021
            | 10029
            | 10079
            | 10081
            | 10082
            | 20936
            | 51936
    )
}

/// Looks up a code page by Windows code page number.
pub fn from_number(number: u32) -> Option<CodePage> {
    if number == 65001 {
        return Some(CodePage {
            number,
            kind: Kind::Utf8,
        });
    }
    if number == 10000 {
        return Some(CodePage {
            number,
            kind: Kind::MacRoman,
        });
    }
    let (label, multi_byte) = match number {
        874 => ("windows-874", false),
        1250 => ("windows-1250", false),
        1251 => ("windows-1251", false),
        1252 => ("windows-1252", false),
        1253 => ("windows-1253", false),
        1254 => ("windows-1254", false),
        1255 => ("windows-1255", false),
        1256 => ("windows-1256", false),
        1257 => ("windows-1257", false),
        1258 => ("windows-1258", false),
        866 => ("ibm866", false),
        20866 => ("koi8-r", false),
        21866 => ("koi8-u", false),
        10007 => ("x-mac-cyrillic", false),
        28592 => ("iso-8859-2", false),
        28593 => ("iso-8859-3", false),
        28594 => ("iso-8859-4", false),
        28595 => ("iso-8859-5", false),
        28596 => ("iso-8859-6", false),
        28597 => ("iso-8859-7", false),
        28598 => ("iso-8859-8", false),
        28603 => ("iso-8859-13", false),
        28605 => ("iso-8859-15", false),
        932 => ("shift_jis", true),
        936 => ("gbk", true),
        949 => ("euc-kr", true),
        950 => ("big5", true),
        _ => return None,
    };
    let encoding = Encoding::for_label(label.as_bytes())?;
    let kind = if multi_byte {
        Kind::Dbcs(encoding)
    } else {
        Kind::SingleByte(encoding)
    };
    Some(CodePage { number, kind })
}

/// Maps an `\fcharsetN` value. `None` means the value is not in the RTF charset list.
pub fn charset_from_fcharset(value: u32) -> Option<Charset> {
    let number = match value {
        0 | 1 => return Some(Charset::Document),
        2 => return Some(Charset::Symbol),
        // Mac charsets: the Mac DBCS ones are mapped to their Windows equivalents.
        77 => 10000,
        78 => 932,
        79 => 949,
        80 => 936,
        81 => 950,
        128 => 932,
        129 => 949,
        130 => 1361,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178..=180 => 1256,
        181 => 1255,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        254 | 255 => 437,
        _ => return None,
    };
    Some(Charset::Number(number))
}

/// Mac OS Roman bytes 0x80 to 0xFF.
const MAC_ROMAN_HIGH: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3,
    0x00E5, 0x00E7, 0x00E9, 0x00E8, 0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3,
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020, 0x00B0, 0x00A2, 0x00A3,
    0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA,
    0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D,
    0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE,
    0x00CF, 0x00CC, 0x00D3, 0x00D4, 0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC,
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

const REPLACEMENT: char = '\u{FFFD}';

/// Streaming decoder. A lead byte that starts a multi-byte sequence is held until its trail bytes
/// arrive, which may come in later `\'hh` tokens. Call `flush` before the state changes (control
/// word, group boundary) so that an incomplete sequence is reported.
#[derive(Debug, Default, Clone, Copy)]
pub struct Decoder {
    pending: [u8; 4],
    len: usize,
    need: usize,
}

impl Decoder {
    /// True when no multi-byte sequence is pending.
    pub fn is_idle(&self) -> bool {
        self.len == 0
    }

    /// Decodes `bytes` into `out`. Returns the number of replacement characters produced.
    pub fn push(&mut self, cp: CodePage, bytes: &[u8], out: &mut String) -> usize {
        match cp.kind {
            Kind::SingleByte(encoding) => {
                let decoded = encoding.decode_without_bom_handling(bytes).0;
                out.push_str(&decoded);
                0
            }
            Kind::MacRoman => {
                for &b in bytes {
                    if b < 0x80 {
                        out.push(char::from(b));
                    } else {
                        let code = MAC_ROMAN_HIGH[usize::from(b - 0x80)];
                        out.push(char::from_u32(u32::from(code)).unwrap_or(REPLACEMENT));
                    }
                }
                0
            }
            Kind::Symbol => {
                let mut replaced = 0;
                for &b in bytes {
                    match b {
                        0x20..=0x3F => out.push(char::from(b)),
                        0xB7 => out.push('\u{2022}'),
                        _ => {
                            out.push(REPLACEMENT);
                            replaced += 1;
                        }
                    }
                }
                replaced
            }
            Kind::Dbcs(_) | Kind::Utf8 => {
                let mut replaced = 0;
                for &b in bytes {
                    replaced += self.step(cp.kind, b, out);
                }
                replaced
            }
        }
    }

    /// Reports a pending incomplete sequence as one replacement character.
    pub fn flush(&mut self, out: &mut String) -> usize {
        if self.len == 0 {
            return 0;
        }
        self.len = 0;
        out.push(REPLACEMENT);
        1
    }

    fn step(&mut self, kind: Kind, b: u8, out: &mut String) -> usize {
        if self.len == 0 {
            match lead_length(kind, b) {
                Some(1) => {
                    emit_single(kind, b, out);
                    0
                }
                Some(need) => {
                    self.pending[0] = b;
                    self.len = 1;
                    self.need = need;
                    0
                }
                None => {
                    out.push(REPLACEMENT);
                    1
                }
            }
        } else if trail_ok(kind, b) {
            self.pending[self.len] = b;
            self.len += 1;
            if self.len < self.need {
                return 0;
            }
            let replaced = decode_pending(kind, &self.pending[..self.len], out);
            self.len = 0;
            replaced
        } else {
            // The pending lead byte is invalid. Report it, then process `b` from a clean state.
            out.push(REPLACEMENT);
            self.len = 0;
            1 + self.step(kind, b, out)
        }
    }
}

/// Number of bytes in the sequence that starts with `b`, or `None` if `b` cannot start one.
fn lead_length(kind: Kind, b: u8) -> Option<usize> {
    match kind {
        Kind::Utf8 => match b {
            0x00..=0x7F => Some(1),
            0xC2..=0xDF => Some(2),
            0xE0..=0xEF => Some(3),
            0xF0..=0xF4 => Some(4),
            _ => None,
        },
        Kind::Dbcs(encoding) => {
            if b < 0x80 {
                return Some(1);
            }
            let shift_jis = encoding.name() == "Shift_JIS";
            let lead = if shift_jis {
                matches!(b, 0x81..=0x9F | 0xE0..=0xFC)
            } else {
                matches!(b, 0x81..=0xFE)
            };
            if lead {
                Some(2)
            } else {
                Some(1)
            }
        }
        _ => Some(1),
    }
}

fn trail_ok(kind: Kind, b: u8) -> bool {
    match kind {
        Kind::Utf8 => b & 0xC0 == 0x80,
        _ => (0x40..=0xFE).contains(&b) && b != 0x7F,
    }
}

fn emit_single(kind: Kind, b: u8, out: &mut String) {
    match kind {
        Kind::Dbcs(encoding) => {
            if b < 0x80 {
                out.push(char::from(b));
            } else {
                out.push_str(&encoding.decode_without_bom_handling(&[b]).0);
            }
        }
        _ => out.push(char::from(b)),
    }
}

fn decode_pending(kind: Kind, bytes: &[u8], out: &mut String) -> usize {
    match kind {
        Kind::Utf8 => match std::str::from_utf8(bytes) {
            Ok(s) => {
                out.push_str(s);
                0
            }
            Err(_) => {
                out.push(REPLACEMENT);
                1
            }
        },
        Kind::Dbcs(encoding) => {
            let decoded = encoding.decode_without_bom_handling(bytes).0;
            let replaced = decoded.matches(REPLACEMENT).count();
            out.push_str(&decoded);
            replaced
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(number: u32, bytes: &[u8]) -> String {
        let cp = from_number(number).expect("known code page");
        let mut out = String::new();
        let mut dec = Decoder::default();
        dec.push(cp, bytes, &mut out);
        dec.flush(&mut out);
        out
    }

    #[test]
    fn windows_1252_latin1_and_smart_quotes() {
        assert_eq!(decode(1252, b"\xe9"), "é");
        assert_eq!(decode(1252, b"\x93x\x94"), "\u{201c}x\u{201d}");
        assert_eq!(decode(1252, b"\xa7\xa9"), "§©");
    }

    #[test]
    fn cyrillic_greek_hebrew_through_their_code_pages() {
        assert_eq!(decode(1251, b"\xc0\xe1"), "Аб");
        assert_eq!(decode(1253, b"\xe1"), "α");
        assert_eq!(decode(1255, b"\xe0"), "א");
        assert_eq!(decode(1250, b"\xa5"), "Ą");
        assert_eq!(decode(1254, b"\xde"), "Ş");
    }

    #[test]
    fn koi8_and_ibm866() {
        assert_eq!(decode(20866, b"\xc1"), "а");
        assert_eq!(decode(866, b"\x80"), "А");
    }

    #[test]
    fn cjk_double_byte_pairs() {
        assert_eq!(decode(932, b"\x82\xa0"), "あ");
        assert_eq!(decode(936, b"\xd6\xd0"), "中");
        assert_eq!(decode(950, b"\xa4\xa4"), "中");
        assert_eq!(decode(949, b"\xc7\xd1"), "한");
    }

    #[test]
    fn dbcs_pair_split_across_pushes() {
        let cp = from_number(932).unwrap();
        let mut dec = Decoder::default();
        let mut out = String::new();
        dec.push(cp, b"\x82", &mut out);
        assert_eq!(out, "", "lead byte alone produces nothing yet");
        dec.push(cp, b"\xa0", &mut out);
        assert_eq!(out, "あ");
    }

    #[test]
    fn invalid_trail_reports_one_replacement_and_keeps_the_byte() {
        let cp = from_number(950).unwrap();
        let mut dec = Decoder::default();
        let mut out = String::new();
        let replaced = dec.push(cp, b"\x81 ", &mut out);
        assert_eq!(out, "\u{FFFD} ");
        assert_eq!(replaced, 1);
    }

    #[test]
    fn incomplete_pair_at_flush_is_a_replacement() {
        let cp = from_number(932).unwrap();
        let mut dec = Decoder::default();
        let mut out = String::new();
        dec.push(cp, b"\x82", &mut out);
        assert_eq!(dec.flush(&mut out), 1);
        assert_eq!(out, "\u{FFFD}");
    }

    #[test]
    fn utf8_sequence_split_over_escapes() {
        let cp = from_number(65001).unwrap();
        let mut dec = Decoder::default();
        let mut out = String::new();
        for b in b"\xe2\x80\x9c" {
            dec.push(cp, std::slice::from_ref(b), &mut out);
        }
        assert_eq!(out, "\u{201c}");
    }

    #[test]
    fn mac_roman_high_half() {
        let cp = from_number(10000).unwrap();
        let mut out = String::new();
        let mut dec = Decoder::default();
        dec.push(cp, b"\x80\xd2\xd3\xa5\xdb", &mut out);
        assert_eq!(out, "Ä\u{201c}\u{201d}•€");
    }

    #[test]
    fn symbol_font_bullet_and_unmapped() {
        let cp = symbol();
        let mut out = String::new();
        let mut dec = Decoder::default();
        let replaced = dec.push(cp, b"\xb7\xe1", &mut out);
        assert_eq!(out, "\u{2022}\u{FFFD}");
        assert_eq!(replaced, 1);
    }

    #[test]
    fn fcharset_mapping() {
        assert_eq!(charset_from_fcharset(0), Some(Charset::Document));
        assert_eq!(charset_from_fcharset(2), Some(Charset::Symbol));
        assert_eq!(charset_from_fcharset(128), Some(Charset::Number(932)));
        assert_eq!(charset_from_fcharset(204), Some(Charset::Number(1251)));
        assert_eq!(charset_from_fcharset(77), Some(Charset::Number(10000)));
        assert_eq!(charset_from_fcharset(99), None);
    }

    #[test]
    fn unknown_and_unsupported_code_pages() {
        assert!(from_number(99999).is_none());
        assert!(from_number(437).is_none());
        assert!(is_known_unsupported(437));
        assert!(!is_known_unsupported(99999));
    }
}
