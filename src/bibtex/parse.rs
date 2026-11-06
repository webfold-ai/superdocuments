//! The BibTeX reader. One pass over the bytes, with a brace-depth limit and per-value and
//! per-entry limits. Errors inside an entry skip to the next field, and an entry that runs to
//! the end of the input is kept with the fields read so far.

use std::collections::{HashMap, HashSet};

use super::{decode_value, Bibliography, Entry, Field, Limits, StringDef, Warning, MONTHS};

/// Parses `input` with explicit limits.
pub fn parse_with(input: &str, limits: &Limits) -> Bibliography {
    let mut bib = Bibliography::default();
    if input.len() > limits.max_input_bytes {
        bib.warnings.push(Warning {
            offset: 0,
            message: format!(
                "input is {} bytes, over the {} byte limit; nothing read",
                input.len(),
                limits.max_input_bytes
            ),
        });
        return bib;
    }
    let mut reader = Reader {
        src: input,
        bytes: input.as_bytes(),
        pos: 0,
        limits,
        macros: MONTHS
            .iter()
            .map(|(k, v)| (k.to_string(), (v.to_string(), true)))
            .collect(),
        bib,
        seen_keys: HashSet::new(),
    };
    reader.run();
    reader.bib
}

/// Why a piece of input could not be read.
enum Stop {
    /// The input ended inside a value or group.
    Eof,
    /// A limit was reached; the entry is abandoned.
    Limit(&'static str),
}

struct Reader<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    limits: &'a Limits,
    /// Macro name (lower-cased) to (raw value, built-in).
    macros: HashMap<String, (String, bool)>,
    bib: Bibliography,
    seen_keys: HashSet<String>,
}

impl Reader<'_> {
    fn run(&mut self) {
        while let Some(at) = self.find_from(b'@') {
            self.pos = at + 1;
            self.item(at);
            if self.bib.entries.len() >= self.limits.max_entries {
                self.warn(
                    self.pos,
                    "entry limit reached; the rest of the file is not read",
                );
                break;
            }
        }
    }

    fn item(&mut self, at: usize) {
        self.skip_ws();
        let kind_start = self.pos;
        while self.peek().is_some_and(is_ident) {
            self.pos += 1;
        }
        let kind = self.slice(kind_start, self.pos).to_ascii_lowercase();
        self.skip_ws();
        let Some(open) = self.peek() else {
            self.warn(at, "@ with no entry type or delimiter");
            return;
        };
        let close = match open {
            b'{' => b'}',
            b'(' => b')',
            _ => {
                self.warn(at, "expected { or ( after the entry type");
                return;
            }
        };
        if kind.is_empty() {
            self.warn(at, "@ with no entry type");
            return;
        }
        self.pos += 1;
        let result = match kind.as_str() {
            "string" => self.string_block(close),
            "preamble" => self.preamble_block(close),
            "comment" => self.comment_block(open, close),
            _ => self.entry_block(at, kind, close),
        };
        match result {
            Ok(()) => {}
            Err(Stop::Eof) => self.warn(at, "unterminated block at the end of the input"),
            Err(Stop::Limit(msg)) => {
                self.warn(at, msg);
                self.pos = self.bytes.len();
            }
        }
    }

    fn string_block(&mut self, close: u8) -> Result<(), Stop> {
        loop {
            self.skip_ws_and_commas();
            match self.peek() {
                None => return Err(Stop::Eof),
                Some(c) if c == close => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(_) => {}
            }
            let name = self.ident();
            self.skip_ws();
            if name.is_empty() || self.peek() != Some(b'=') {
                self.warn(self.pos, "expected name = value in @string");
                self.skip_field(close);
                continue;
            }
            self.pos += 1;
            let raw = self.expression()?;
            let value = decode_value(&raw);
            let lower = name.to_ascii_lowercase();
            self.macros.insert(lower.clone(), (raw.clone(), false));
            self.bib.strings.push(StringDef {
                name: lower,
                raw,
                value,
            });
        }
    }

    fn preamble_block(&mut self, close: u8) -> Result<(), Stop> {
        self.skip_ws();
        if self.peek() == Some(close) {
            self.pos += 1;
            return Ok(());
        }
        let raw = self.expression()?;
        self.bib.preambles.push(raw);
        self.skip_ws();
        if self.peek() == Some(close) {
            self.pos += 1;
        } else {
            self.warn(self.pos, "expected the end of @preamble");
            self.skip_field(close);
        }
        Ok(())
    }

    fn comment_block(&mut self, open: u8, close: u8) -> Result<(), Stop> {
        // The content is everything up to the matching close delimiter.
        let start = self.pos;
        let mut depth = 1usize;
        let open_depth_char = open;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == open_depth_char {
                depth += 1;
                if depth > self.limits.max_depth {
                    return Err(Stop::Limit("nesting limit in @comment"));
                }
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    let raw = self.slice(start, self.pos - 1).to_string();
                    if braces_balanced(&raw) {
                        self.bib.comments.push(raw);
                    } else {
                        self.warn(start, "@comment has unbalanced braces; it is not kept");
                    }
                    return Ok(());
                }
            }
        }
        Err(Stop::Eof)
    }

    fn entry_block(&mut self, at: usize, kind: String, close: u8) -> Result<(), Stop> {
        self.skip_ws();
        let key_start = self.pos;
        while let Some(c) = self.peek() {
            if matches!(c, b',' | b'{' | b'}' | b'(' | b')')
                || c.is_ascii_whitespace()
                || c == close
            {
                break;
            }
            self.pos += 1;
        }
        let key = self.slice(key_start, self.pos).to_string();
        if key.is_empty() {
            self.warn(at, "entry has no key");
        } else if !self.seen_keys.insert(key.to_ascii_lowercase()) {
            self.warn(at, &format!("duplicate key {key}; both entries are kept"));
        }
        self.skip_ws();
        match self.peek() {
            Some(b',') => self.pos += 1,
            Some(c) if c == close => {}
            _ => {
                self.warn(self.pos, "malformed entry key; the entry is skipped");
                self.skip_field(close);
                return Ok(());
            }
        }
        let mut entry = Entry {
            entry_type: kind,
            key,
            fields: Vec::new(),
        };
        loop {
            self.skip_ws_and_commas();
            match self.peek() {
                None => {
                    self.warn(at, "unterminated entry at the end of the input");
                    self.bib.entries.push(entry);
                    return Ok(());
                }
                Some(c) if c == close => {
                    self.pos += 1;
                    self.bib.entries.push(entry);
                    return Ok(());
                }
                Some(_) => {}
            }
            let name_at = self.pos;
            let name = self.ident().to_ascii_lowercase();
            self.skip_ws();
            if name.is_empty() || self.peek() != Some(b'=') {
                self.warn(name_at, "expected name = value");
                self.skip_field(close);
                continue;
            }
            self.pos += 1;
            let raw = match self.expression() {
                Ok(raw) => raw,
                Err(Stop::Eof) => {
                    self.warn(at, "unterminated entry at the end of the input");
                    self.bib.entries.push(entry);
                    return Ok(());
                }
                Err(stop) => return Err(stop),
            };
            if entry.fields.len() >= self.limits.max_fields {
                return Err(Stop::Limit("field limit reached in one entry"));
            }
            if entry.fields.iter().any(|f| f.name == name) {
                self.warn(
                    name_at,
                    &format!("field {name} repeated; the later one is ignored"),
                );
            } else {
                let value = decode_value(&raw);
                entry.fields.push(Field { name, raw, value });
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(c) if c == close => {}
                None => {}
                Some(_) => {
                    self.warn(self.pos, "expected , or the end of the entry");
                    self.skip_field(close);
                }
            }
        }
    }

    /// Reads `piece # piece # ...` and returns the concatenated raw text.
    fn expression(&mut self) -> Result<String, Stop> {
        let mut raw = String::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'{') => {
                    let inner = self.braced()?;
                    raw.push_str(&inner);
                }
                Some(b'"') => {
                    let inner = self.quoted()?;
                    raw.push_str(&inner);
                }
                Some(c) if is_ident(c) => {
                    let name = self.ident();
                    if name.bytes().all(|b| b.is_ascii_digit()) {
                        raw.push_str(&name);
                    } else {
                        match self.macros.get(&name.to_ascii_lowercase()) {
                            Some((text, _)) => raw.push_str(text),
                            None => self.warn(self.pos, &format!("undefined macro {name}")),
                        }
                    }
                }
                None => return Err(Stop::Eof),
                Some(_) => {
                    self.warn(self.pos, "expected a value");
                    break;
                }
            }
            if raw.len() > self.limits.max_value_bytes {
                return Err(Stop::Limit("value limit reached"));
            }
            self.skip_ws();
            if self.peek() == Some(b'#') {
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(raw)
    }

    /// Reads `{ ... }` with nesting and returns the inner text.
    fn braced(&mut self) -> Result<String, Stop> {
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            match c {
                b'{' => {
                    depth += 1;
                    if depth > self.limits.max_depth {
                        return Err(Stop::Limit("brace nesting limit reached"));
                    }
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner = self.slice(start, self.pos).to_string();
                        self.pos += 1;
                        return Ok(inner);
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err(Stop::Eof)
    }

    /// Reads `" ... "`. A quote ends the value only at brace depth zero.
    fn quoted(&mut self) -> Result<String, Stop> {
        self.pos += 1;
        let start = self.pos;
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            match c {
                b'{' => {
                    depth += 1;
                    if depth > self.limits.max_depth {
                        return Err(Stop::Limit("brace nesting limit reached"));
                    }
                }
                b'}' if depth == 0 => {
                    // An unmatched brace ends the value here, so the raw text stays balanced.
                    self.warn(self.pos, "unmatched } inside a quoted value");
                    return Ok(self.slice(start, self.pos).to_string());
                }
                b'}' => depth -= 1,
                b'"' if depth == 0 => {
                    let inner = self.slice(start, self.pos).to_string();
                    self.pos += 1;
                    return Ok(inner);
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err(Stop::Eof)
    }

    fn skip_field(&mut self, close: u8) {
        // Skip to a comma or the closing delimiter at brace depth zero.
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            match c {
                b'{' => depth += 1,
                b'}' if depth > 0 => depth -= 1,
                b',' if depth == 0 => {
                    self.pos += 1;
                    return;
                }
                c if c == close && depth == 0 => return,
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn ident(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_ident) {
            self.pos += 1;
        }
        self.slice(start, self.pos).to_string()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn skip_ws_and_commas(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_whitespace() || c == b',')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn find_from(&self, needle: u8) -> Option<usize> {
        self.bytes
            .get(self.pos..)?
            .iter()
            .position(|&b| b == needle)
            .map(|i| i + self.pos)
    }

    fn slice(&self, a: usize, b: usize) -> &str {
        self.src.get(a..b).unwrap_or("")
    }

    fn warn(&mut self, offset: usize, message: &str) {
        self.bib.warnings.push(Warning {
            offset,
            message: message.to_string(),
        });
    }
}

fn braces_balanced(s: &str) -> bool {
    let mut depth = 0usize;
    for c in s.bytes() {
        match c {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    depth == 0
}

/// Bytes that may appear in an entry type, field name, macro name or number.
fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.' | b'/' | b'+' | b'*')
}
