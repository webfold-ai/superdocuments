//! The LaTeX tokenizer. It is not a TeX engine: it only splits the source into control words,
//! control symbols, text runs, groups, math delimiters and spaces. It also gives the parser raw
//! access to the source, for arguments, math, verbatim bodies and file paths.
//!
//! Comments (`%` to the end of the line) are skipped. A blank line becomes `Tok::Par`. A control
//! word swallows the spaces and at most one line break after it, as TeX does. `\verb` is read
//! here, because its delimiter can be any character.

use std::rc::Rc;

/// One token. `Text` runs never contain special characters, so their boundaries are always
/// character boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    /// `\name` (ASCII letters).
    Ctrl(String),
    /// `\` followed by one non-letter character.
    Sym(char),
    Text(String),
    Space,
    /// A blank line.
    Par,
    Open,
    Close,
    /// `$`
    Dollar,
    /// `$$`
    DDollar,
    Amp,
    /// `#1` to `#9` (only meaningful in macro bodies).
    Param(u8),
    /// `\verb` content.
    Verb(String),
}

/// A source with a read position. `base` is the offset that tokens from an expansion report,
/// since they have no place in the main source; `main` is true only for the document itself.
#[derive(Debug, Clone)]
pub(crate) struct Lexer {
    src: Rc<str>,
    pub(crate) pos: usize,
    base: usize,
    main: bool,
    /// The `\input` name this source was read from, for sources that are files. The parser
    /// uses it to refuse a file that includes itself.
    pub(crate) file: Option<String>,
}

impl Lexer {
    pub(crate) fn new(src: Rc<str>, base: usize, main: bool) -> Self {
        Self {
            src,
            pos: 0,
            base,
            main,
            file: None,
        }
    }

    /// A source that is the text of an `\input` file.
    pub(crate) fn for_file(src: Rc<str>, base: usize, name: String) -> Self {
        Self {
            src,
            pos: 0,
            base,
            main: false,
            file: Some(name),
        }
    }

    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// The source offset for diagnostics: the real position in the main source, or the
    /// invocation offset inside an expansion.
    pub(crate) fn offset(&self) -> usize {
        if self.main {
            self.pos
        } else {
            self.base
        }
    }

    fn at(&self, i: usize) -> Option<u8> {
        self.src.as_bytes().get(i).copied()
    }

    fn char_at(&self, i: usize) -> Option<char> {
        self.src.get(i..).and_then(|s| s.chars().next())
    }

    fn report(&self, start: usize) -> usize {
        if self.main {
            start
        } else {
            self.base
        }
    }

    /// The next token and its offset, or `None` at the end of the source.
    pub(crate) fn next_tok(&mut self) -> Option<(Tok, usize)> {
        loop {
            let c = self.at(self.pos)?;
            let start = self.pos;
            match c {
                b'%' => {
                    self.skip_comment();
                    continue;
                }
                b' ' | b'\t' | b'\r' | b'\n' => {
                    let mut newlines = 0;
                    while let Some(c) = self.at(self.pos) {
                        match c {
                            b' ' | b'\t' | b'\r' => self.pos += 1,
                            b'\n' => {
                                newlines += 1;
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    let tok = if newlines >= 2 { Tok::Par } else { Tok::Space };
                    return Some((tok, self.report(start)));
                }
                b'\\' => return Some(self.control()),
                b'{' => {
                    self.pos += 1;
                    return Some((Tok::Open, self.report(start)));
                }
                b'}' => {
                    self.pos += 1;
                    return Some((Tok::Close, self.report(start)));
                }
                b'$' => {
                    let tok = if self.at(self.pos + 1) == Some(b'$') {
                        self.pos += 2;
                        Tok::DDollar
                    } else {
                        self.pos += 1;
                        Tok::Dollar
                    };
                    return Some((tok, self.report(start)));
                }
                b'&' => {
                    self.pos += 1;
                    return Some((Tok::Amp, self.report(start)));
                }
                b'#' => {
                    if let Some(d @ b'1'..=b'9') = self.at(self.pos + 1) {
                        self.pos += 2;
                        return Some((Tok::Param(d - b'0'), self.report(start)));
                    }
                    self.pos += 1;
                    return Some((Tok::Text("#".into()), self.report(start)));
                }
                _ => {
                    let mut end = self.pos;
                    while let Some(b) = self.at(end) {
                        if matches!(
                            b,
                            b'\\'
                                | b'{'
                                | b'}'
                                | b'$'
                                | b'&'
                                | b'#'
                                | b'%'
                                | b' '
                                | b'\t'
                                | b'\r'
                                | b'\n'
                        ) {
                            break;
                        }
                        end += 1;
                    }
                    let text = self.src.get(start..end).unwrap_or("").to_string();
                    self.pos = end;
                    return Some((Tok::Text(text), self.report(start)));
                }
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(c) = self.at(self.pos) {
            self.pos += 1;
            if c == b'\n' {
                break;
            }
        }
    }

    fn control(&mut self) -> (Tok, usize) {
        let start = self.pos;
        self.pos += 1;
        match self.at(self.pos) {
            None => (Tok::Text("\\".into()), self.report(start)),
            Some(c) if c.is_ascii_alphabetic() => {
                let name_start = self.pos;
                while self.at(self.pos).is_some_and(|b| b.is_ascii_alphabetic()) {
                    self.pos += 1;
                }
                let name = self.src.get(name_start..self.pos).unwrap_or("").to_string();
                if name == "verb" {
                    if let Some(tok) = self.verb() {
                        return (tok, self.report(start));
                    }
                }
                self.skip_word_spaces();
                (Tok::Ctrl(name), self.report(start))
            }
            Some(_) => {
                let c = self.char_at(self.pos).unwrap_or('\\');
                self.pos += c.len_utf8();
                (Tok::Sym(c), self.report(start))
            }
        }
    }

    /// TeX skips spaces after a control word, and one line break.
    fn skip_word_spaces(&mut self) {
        while self.at(self.pos).is_some_and(|b| b == b' ' || b == b'\t') {
            self.pos += 1;
        }
        if self.at(self.pos) == Some(b'\r') {
            self.pos += 1;
        }
        if self.at(self.pos) == Some(b'\n') {
            self.pos += 1;
            while self.at(self.pos).is_some_and(|b| b == b' ' || b == b'\t') {
                self.pos += 1;
            }
        }
    }

    /// `\verb*|text|`. Returns `None` (without consuming) when no delimiter follows.
    fn verb(&mut self) -> Option<Tok> {
        let save = self.pos;
        if self.at(self.pos) == Some(b'*') {
            self.pos += 1;
        }
        let Some(delim) = self.char_at(self.pos) else {
            self.pos = save;
            return None;
        };
        if delim.is_alphabetic() || delim.is_whitespace() {
            self.pos = save;
            return None;
        }
        self.pos += delim.len_utf8();
        let body_start = self.pos;
        while let Some(c) = self.char_at(self.pos) {
            if c == delim || c == '\n' {
                break;
            }
            self.pos += c.len_utf8();
        }
        let body = self.src.get(body_start..self.pos).unwrap_or("").to_string();
        if self.char_at(self.pos) == Some(delim) {
            self.pos += delim.len_utf8();
        }
        Some(Tok::Verb(body))
    }

    /// Skips spaces, line breaks and comments.
    pub(crate) fn skip_space(&mut self) {
        loop {
            match self.at(self.pos) {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'%') => self.skip_comment(),
                _ => break,
            }
        }
    }

    /// The next non-space byte, without moving past it.
    pub(crate) fn peek_byte(&mut self) -> Option<u8> {
        self.skip_space();
        self.at(self.pos)
    }

    /// Reads `{...}` and returns the inner source text. Escapes are kept, comments are removed.
    /// An unclosed group is read to the end. Returns `None` when no `{` follows.
    pub(crate) fn group_raw(&mut self) -> Option<String> {
        if self.peek_byte() != Some(b'{') {
            return None;
        }
        self.pos += 1;
        Some(self.balanced(b'{', b'}'))
    }

    /// Reads `[...]` and returns the inner text, or `None` when no `[` follows.
    pub(crate) fn optional_raw(&mut self) -> Option<String> {
        if self.peek_byte() != Some(b'[') {
            return None;
        }
        self.pos += 1;
        Some(self.balanced(b'[', b']'))
    }

    /// Consumes one byte if it is `b` (after spaces). Used for `*` in starred commands.
    pub(crate) fn eat(&mut self, b: u8) -> bool {
        if self.peek_byte() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// The source between two offsets, or empty when they are not valid boundaries.
    pub(crate) fn source_slice(&self, a: usize, b: usize) -> String {
        self.src.get(a..b).unwrap_or("").to_string()
    }

    /// One undelimited argument: a group, a control word, or one character. A closing brace is
    /// never taken, because it belongs to the enclosing group.
    pub(crate) fn single_raw(&mut self) -> Option<String> {
        self.skip_space();
        match self.at(self.pos)? {
            b'}' => None,
            b'{' => self.group_raw(),
            b'\\' => {
                let start = self.pos;
                self.pos += 1;
                if self.at(self.pos).is_some_and(|b| b.is_ascii_alphabetic()) {
                    while self.at(self.pos).is_some_and(|b| b.is_ascii_alphabetic()) {
                        self.pos += 1;
                    }
                } else if let Some(c) = self.char_at(self.pos) {
                    self.pos += c.len_utf8();
                }
                Some(self.src.get(start..self.pos).unwrap_or("").to_string())
            }
            _ => {
                let c = self.char_at(self.pos)?;
                self.pos += c.len_utf8();
                Some(c.to_string())
            }
        }
    }

    /// The letters of a control word `\name` (or one character after `\`) at the current
    /// position, without the backslash. Returns `None` when there is no backslash.
    pub(crate) fn ctrl_name_raw(&mut self) -> Option<String> {
        self.skip_space();
        if self.at(self.pos)? != b'\\' {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        if self.at(self.pos).is_some_and(|b| b.is_ascii_alphabetic()) {
            while self.at(self.pos).is_some_and(|b| b.is_ascii_alphabetic()) {
                self.pos += 1;
            }
        } else if let Some(c) = self.char_at(self.pos) {
            self.pos += c.len_utf8();
        }
        Some(self.src.get(start..self.pos).unwrap_or("").to_string())
    }

    /// Raw text up to (not including) the next `{`, for `\def` parameter text.
    pub(crate) fn text_until_brace(&mut self) -> String {
        let start = self.pos;
        while self.at(self.pos).is_some_and(|b| b != b'{') {
            self.pos += 1;
        }
        self.src.get(start..self.pos).unwrap_or("").to_string()
    }

    /// Reads up to `term` (the delimiter is consumed). With `math` set, escapes are copied
    /// as written and comments are removed. Returns the text and whether `term` was found.
    pub(crate) fn until_raw(&mut self, term: &str, math: bool) -> (String, bool) {
        let src = self.src.clone();
        let bytes = src.as_bytes();
        let tb = term.as_bytes();
        let mut out = String::new();
        let mut i = self.pos;
        while i < bytes.len() {
            if bytes[i..].starts_with(tb) {
                self.pos = i + tb.len();
                return (out, true);
            }
            if math && bytes[i] == b'%' {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if math && bytes[i] == b'\\' && i + 1 < bytes.len() {
                out.push('\\');
                i += 1;
                if let Some(c) = src.get(i..).and_then(|s| s.chars().next()) {
                    out.push(c);
                    i += c.len_utf8();
                }
                continue;
            }
            let Some(c) = src.get(i..).and_then(|s| s.chars().next()) else {
                break;
            };
            out.push(c);
            i += c.len_utf8();
        }
        self.pos = bytes.len();
        (out, false)
    }

    /// Reads the body of environment `name` as written, up to its `\end{name}` (consumed).
    /// A nested `\begin{name}` is matched with its own `\end{name}`. Nothing is removed from
    /// the text, but a `\begin` or `\end` inside a comment does not count.
    pub(crate) fn until_env_end(&mut self, name: &str) -> (String, bool) {
        let src = self.src.clone();
        let bytes = src.as_bytes();
        let begin = format!("\\begin{{{name}}}");
        let end = format!("\\end{{{name}}}");
        let start = self.pos;
        let mut depth = 0usize;
        let mut i = self.pos;
        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'\\' if bytes[i..].starts_with(end.as_bytes()) => {
                    if depth == 0 {
                        self.pos = i + end.len();
                        return (src.get(start..i).unwrap_or("").to_string(), true);
                    }
                    depth -= 1;
                    i += end.len();
                }
                b'\\' if bytes[i..].starts_with(begin.as_bytes()) => {
                    depth += 1;
                    i += begin.len();
                }
                // An escaped character (`\%`, `\\`) is never a comment or a command start.
                b'\\' => i += 2,
                _ => i += 1,
            }
        }
        self.pos = bytes.len();
        (src.get(start..).unwrap_or("").to_string(), false)
    }

    /// Reads to the matching `close` for an already consumed `open`. Escapes are kept, comments
    /// are removed. Returns the inner text. An unclosed group is returned up to the end.
    fn balanced(&mut self, open: u8, close: u8) -> String {
        let src = self.src.clone();
        let bytes = src.as_bytes();
        let mut out = String::new();
        let mut depth = 1usize;
        let mut i = self.pos;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && i + 1 < bytes.len() {
                out.push('\\');
                i += 1;
                if let Some(c) = src.get(i..).and_then(|s| s.chars().next()) {
                    out.push(c);
                    i += c.len_utf8();
                }
                continue;
            }
            if b == b'%' {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    self.pos = i + 1;
                    return out;
                }
            }
            let Some(c) = src.get(i..).and_then(|s| s.chars().next()) else {
                break;
            };
            out.push(c);
            i += c.len_utf8();
        }
        self.pos = bytes.len();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(src: &str) -> Vec<Tok> {
        let mut lx = Lexer::new(Rc::from(src), 0, true);
        let mut out = Vec::new();
        while let Some((t, _)) = lx.next_tok() {
            out.push(t);
        }
        out
    }

    #[test]
    fn control_words_swallow_following_spaces() {
        assert_eq!(
            all("\\emph  {x}"),
            vec![
                Tok::Ctrl("emph".into()),
                Tok::Open,
                Tok::Text("x".into()),
                Tok::Close
            ]
        );
    }

    #[test]
    fn blank_line_is_paragraph_break_and_comments_vanish() {
        assert_eq!(
            all("a % hidden\nb\n\nc"),
            vec![
                Tok::Text("a".into()),
                Tok::Space,
                Tok::Text("b".into()),
                Tok::Par,
                Tok::Text("c".into())
            ]
        );
    }

    #[test]
    fn dollars_and_symbols() {
        assert_eq!(
            all("$x$$$\\&\\\\"),
            vec![
                Tok::Dollar,
                Tok::Text("x".into()),
                Tok::DDollar,
                Tok::Dollar,
                Tok::Sym('&'),
                Tok::Sym('\\')
            ]
        );
    }

    #[test]
    fn verb_takes_any_delimiter() {
        assert_eq!(all("\\verb|a{b}|"), vec![Tok::Verb("a{b}".into())]);
        assert_eq!(all("\\verb+x+"), vec![Tok::Verb("x".into())]);
    }

    #[test]
    fn raw_group_removes_comments_and_keeps_escapes() {
        let mut lx = Lexer::new(Rc::from("{a \\{ % c\n b}rest"), 0, true);
        assert_eq!(lx.group_raw().as_deref(), Some("a \\{ \n b"));
        assert_eq!(lx.src.get(lx.pos..), Some("rest"));
    }

    #[test]
    fn until_raw_finds_delimiter_past_escaped_dollar() {
        let mut lx = Lexer::new(Rc::from("a\\$b$ tail"), 0, true);
        let (text, found) = lx.until_raw("$", true);
        assert!(found);
        assert_eq!(text, "a\\$b");
    }

    #[test]
    fn multibyte_text_is_never_split() {
        let toks = all("é{ß}ü");
        assert_eq!(toks[0], Tok::Text("é".into()));
        assert_eq!(toks[2], Tok::Text("ß".into()));
    }

    #[test]
    fn truncated_inputs_do_not_panic() {
        for s in ["\\", "\\verb", "\\verb|", "{", "$", "#", "\\verb*", "\\é"] {
            let _ = all(s);
            let mut lx = Lexer::new(Rc::from(s), 0, true);
            let _ = lx.group_raw();
            let _ = lx.optional_raw();
            let _ = lx.single_raw();
            let _ = lx.until_raw("$$", true);
        }
    }
}
