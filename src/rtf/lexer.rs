//! Single-pass RTF tokenizer. Tokens borrow from the input. Every call to `next` either returns
//! `None` or consumes at least one byte, so iteration always terminates.

/// One lexical token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    /// `{`
    Open,
    /// `}`
    Close,
    /// `\name` with an optional signed parameter. The one delimiter (space, CR or LF) is consumed.
    Word { name: &'a [u8], param: Option<i32> },
    /// `\` followed by one non-letter byte, such as `\\`, `\{`, `\~` or `\*`. A backslash followed
    /// by CR or LF is `Symbol(b'\n')`, which RTF defines as a paragraph break.
    Symbol(u8),
    /// `\'hh`
    Hex(u8),
    /// A run of plain bytes in the current code page. CR and LF are never included.
    Text(&'a [u8]),
    /// Raw bytes following `\binN`. `declared` is N. `data` is shorter when the input ends first.
    Binary { data: &'a [u8], declared: usize },
    /// A recoverable syntax problem. The lexer has already moved past it.
    Malformed(&'static str),
}

/// A token and the byte offset where it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lexed<'a> {
    pub token: Token<'a>,
    pub offset: usize,
}

/// Parameters are clamped to this magnitude before conversion to `i32`.
const PARAM_CAP: i64 = 1 << 40;

/// Tokenizer over a byte slice.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Byte offset of the next unread byte.
    pub fn position(&self) -> usize {
        self.pos
    }

    fn control(&mut self) -> Token<'a> {
        let data = self.data;
        let start = self.pos;
        let Some(&next) = data.get(start + 1) else {
            self.pos = data.len();
            return Token::Malformed("backslash at end of input");
        };
        if next == b'\'' {
            // Two hex digits make a byte. Otherwise the escape is dropped together with any
            // hex digits that did follow it, so a truncated `\'e` does not leak an `e` into text.
            let rest = data.get(start + 2..).unwrap_or(&[]);
            let digits = &rest[..rest.len().min(2)];
            let valid = digits
                .iter()
                .take_while(|b| hex_digit(**b).is_some())
                .count();
            if valid == 2 {
                self.pos = start + 4;
                let value =
                    hex_digit(digits[0]).unwrap_or(0) << 4 | hex_digit(digits[1]).unwrap_or(0);
                return Token::Hex(value);
            }
            self.pos = start + 2 + valid;
            return Token::Malformed("invalid or truncated hex escape");
        }
        if next == b'\r' || next == b'\n' {
            self.pos = start + 2;
            return Token::Symbol(b'\n');
        }
        if !next.is_ascii_alphabetic() {
            self.pos = start + 2;
            return Token::Symbol(next);
        }

        let name_start = start + 1;
        let mut i = name_start;
        while data.get(i).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
        }
        let name = &data[name_start..i];

        let mut param = None;
        let mut j = i;
        let negative = data.get(j) == Some(&b'-');
        if negative {
            j += 1;
        }
        if data.get(j).is_some_and(u8::is_ascii_digit) {
            let mut value: i64 = 0;
            while let Some(&digit) = data.get(j) {
                if !digit.is_ascii_digit() {
                    break;
                }
                value = value
                    .saturating_mul(10)
                    .saturating_add(i64::from(digit - b'0'))
                    .min(PARAM_CAP);
                j += 1;
            }
            if negative {
                value = -value;
            }
            let clamped = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            param = Some(clamped as i32);
            i = j;
        }
        if matches!(data.get(i), Some(b' ' | b'\r' | b'\n')) {
            i += 1;
        }
        self.pos = i;

        if name == b"bin" {
            return match param {
                Some(n) if n >= 0 => {
                    let declared = n as usize;
                    let end = i.saturating_add(declared).min(data.len());
                    self.pos = end;
                    Token::Binary {
                        data: &data[i..end],
                        declared,
                    }
                }
                _ => Token::Malformed("\\bin without a valid length"),
            };
        }
        Token::Word { name, param }
    }

    fn text(&mut self) -> Token<'a> {
        let data = self.data;
        let start = self.pos;
        let mut i = start;
        while let Some(&b) = data.get(i) {
            if matches!(b, b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                break;
            }
            i += 1;
        }
        self.pos = i;
        Token::Text(&data[start..i])
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Lexed<'a>;

    fn next(&mut self) -> Option<Lexed<'a>> {
        // CR and LF carry no meaning between tokens.
        while matches!(self.data.get(self.pos), Some(b'\r' | b'\n')) {
            self.pos += 1;
        }
        let offset = self.pos;
        let byte = *self.data.get(self.pos)?;
        let token = match byte {
            b'{' => {
                self.pos += 1;
                Token::Open
            }
            b'}' => {
                self.pos += 1;
                Token::Close
            }
            b'\\' => self.control(),
            _ => self.text(),
        };
        Some(Lexed { token, offset })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(data: &[u8]) -> Vec<Token<'_>> {
        Lexer::new(data).map(|l| l.token).collect()
    }

    fn word(name: &[u8], param: Option<i32>) -> Token<'_> {
        Token::Word { name, param }
    }

    #[test]
    fn groups_and_plain_text() {
        assert_eq!(
            tokens(b"{abc}"),
            vec![Token::Open, Token::Text(b"abc"), Token::Close]
        );
    }

    #[test]
    fn control_word_consumes_one_delimiter_space() {
        assert_eq!(
            tokens(b"\\par  x"),
            vec![word(b"par", None), Token::Text(b" x")]
        );
    }

    #[test]
    fn control_word_parameters_signed() {
        assert_eq!(tokens(b"\\u-3913 "), vec![word(b"u", Some(-3913))]);
        assert_eq!(tokens(b"\\fs24"), vec![word(b"fs", Some(24))]);
        assert_eq!(tokens(b"\\b0 "), vec![word(b"b", Some(0))]);
    }

    #[test]
    fn missing_space_delimiter_is_tolerated() {
        assert_eq!(
            tokens(b"\\b1\\i0x"),
            vec![word(b"b", Some(1)), word(b"i", Some(0)), Token::Text(b"x")]
        );
    }

    #[test]
    fn lone_minus_is_text_not_a_parameter() {
        assert_eq!(
            tokens(b"\\par-x"),
            vec![word(b"par", None), Token::Text(b"-x")]
        );
    }

    #[test]
    fn huge_parameters_saturate() {
        let huge = b"\\fs99999999999999999999999999 ";
        assert_eq!(tokens(huge), vec![word(b"fs", Some(i32::MAX))]);
        let neg = b"\\u-99999999999999999999 ";
        assert_eq!(tokens(neg), vec![word(b"u", Some(i32::MIN))]);
    }

    #[test]
    fn control_symbols() {
        assert_eq!(
            tokens(b"\\\\\\{\\}\\~\\-\\_\\*"),
            vec![
                Token::Symbol(b'\\'),
                Token::Symbol(b'{'),
                Token::Symbol(b'}'),
                Token::Symbol(b'~'),
                Token::Symbol(b'-'),
                Token::Symbol(b'_'),
                Token::Symbol(b'*'),
            ]
        );
    }

    #[test]
    fn hex_escapes_decode_both_cases() {
        assert_eq!(
            tokens(b"\\'e9\\'E9\\'0a"),
            vec![Token::Hex(0xe9), Token::Hex(0xe9), Token::Hex(0x0a)]
        );
    }

    #[test]
    fn invalid_and_truncated_hex_are_malformed_and_progress() {
        assert_eq!(
            tokens(b"\\'zzA"),
            vec![
                Token::Malformed("invalid or truncated hex escape"),
                Token::Text(b"zzA"),
            ]
        );
        // One valid digit followed by the end: the digit is part of the broken escape.
        assert_eq!(
            tokens(b"\\'4"),
            vec![Token::Malformed("invalid or truncated hex escape")]
        );
        assert_eq!(
            tokens(b"\\'4 x"),
            vec![
                Token::Malformed("invalid or truncated hex escape"),
                Token::Text(b" x"),
            ]
        );
        assert_eq!(
            tokens(b"\\'"),
            vec![Token::Malformed("invalid or truncated hex escape")]
        );
    }

    #[test]
    fn backslash_at_end_of_input() {
        assert_eq!(
            tokens(b"a\\"),
            vec![
                Token::Text(b"a"),
                Token::Malformed("backslash at end of input")
            ]
        );
    }

    #[test]
    fn crlf_between_tokens_is_ignored_and_backslash_crlf_is_par() {
        assert_eq!(
            tokens(b"a\r\n{\r\n\\par\r\nb\n}"),
            vec![
                Token::Text(b"a"),
                Token::Open,
                word(b"par", None),
                Token::Text(b"b"),
                Token::Close,
            ]
        );
        assert_eq!(
            tokens(b"x\\\ny"),
            vec![Token::Text(b"x"), Token::Symbol(b'\n'), Token::Text(b"y")]
        );
    }

    #[test]
    fn bin_reads_exact_raw_length() {
        // The bytes `}` and `{` inside the binary run are data, not groups.
        assert_eq!(
            tokens(b"\\bin3 }{\\b"),
            vec![
                Token::Binary {
                    data: b"}{\\",
                    declared: 3
                },
                Token::Text(b"b"),
            ]
        );
    }

    #[test]
    fn bin_truncated_at_end_of_input() {
        assert_eq!(
            tokens(b"\\bin10 abc"),
            vec![Token::Binary {
                data: b"abc",
                declared: 10
            }]
        );
    }

    #[test]
    fn bin_without_length_or_negative_is_malformed() {
        assert_eq!(
            tokens(b"\\bin "),
            vec![Token::Malformed("\\bin without a valid length")]
        );
        assert_eq!(
            tokens(b"\\bin-5 x"),
            vec![
                Token::Malformed("\\bin without a valid length"),
                Token::Text(b"x")
            ]
        );
    }

    #[test]
    fn offsets_point_at_token_start() {
        let items: Vec<Lexed<'_>> = Lexer::new(b"{\\b x}").collect();
        let offsets: Vec<usize> = items.iter().map(|l| l.offset).collect();
        assert_eq!(offsets, vec![0, 1, 4, 5]);
    }

    #[test]
    fn progress_on_every_byte_pattern() {
        // Every token consumes at least one byte, so the token count is bounded by the length.
        let mut state: u32 = 0x1234_5678;
        for _ in 0..200 {
            let mut data = Vec::new();
            for _ in 0..512 {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let alphabet = b"{}\\'*\r\n -0123456789abzAZ~_\xe9\x00";
                data.push(alphabet[(state >> 16) as usize % alphabet.len()]);
            }
            let count = Lexer::new(&data).count();
            assert!(
                count <= data.len(),
                "tokens {count} exceed bytes {}",
                data.len()
            );
        }
    }
}
