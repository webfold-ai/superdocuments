//! Group-stack interpreter. Turns lexer tokens into an `RtfDocument`.
//!
//! Each group records its destination (body, skipped, metadata, field instruction, list marker,
//! and so on), character and paragraph state, `\uc`, font and hyperlink. Text is written to a
//! "sink": the body, a header or footer, or a footnote. Each sink has its own paragraph and
//! table state, so a footnote can appear in the middle of a body paragraph.

use super::charset::{self, Charset, CodePage, Decoder};
use super::lexer::{Lexed, Lexer, Token};
use super::model::{
    Alignment, Block, Cell, Footnote, Hyperlink, ListItem, Metadata, Options, Paragraph, Part,
    PartKind, Row, RtfDocument, RtfError, Run, Style, Table, Warning,
};
use std::collections::HashMap;

/// Warnings beyond this count are counted, not stored.
const MAX_WARNINGS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meta {
    Title,
    Subject,
    Author,
    Keywords,
    Comments,
    Generator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateKind {
    Created,
    Modified,
}

/// Where the text of a group goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Dest {
    #[default]
    Normal,
    /// Ignored, including everything inside it.
    Skip,
    /// `\object`: only a `\result` child is shown.
    Object,
    FontTable,
    StyleSheet,
    StyleDef,
    Info,
    Meta(Meta),
    Date(DateKind),
    FieldInst,
    ListText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkKind {
    Footnote(u32),
    Part(PartKind),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Chr {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    hidden: bool,
    size: u16,
}

#[derive(Debug, Clone, Copy, Default)]
struct Para {
    style: Option<u16>,
    outline: Option<u8>,
    align: Alignment,
    li: i32,
    fi: i32,
    intbl: bool,
    itap: u8,
    list: Option<u32>,
    ilvl: u8,
}

#[derive(Debug, Clone, Copy, Default)]
struct Group {
    dest: Dest,
    /// No word, text or hex token has been seen in this group yet.
    fresh: bool,
    /// `\*` was seen; the next control word decides whether the destination is ignorable.
    star: bool,
    /// This group set an auxiliary destination and must finish it when it closes.
    owns_aux: bool,
    /// The sink this group writes to.
    sink: usize,
    /// The sink this group created, if any.
    owns_sink: Option<SinkKind>,
    chr: Chr,
    font: u16,
    para: Para,
    /// `\ucN`: how many fallback characters follow each `\uN`.
    uc: u32,
    link: Option<usize>,
    style_id: Option<u16>,
    style_outline: Option<u8>,
    font_entry: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default)]
struct FontInfo {
    charset: Option<u32>,
    cpg: Option<u32>,
}

#[derive(Debug, Clone, Default)]
struct StyleInfo {
    outline: Option<u8>,
    name: String,
}

#[derive(Debug, Default)]
struct TableLevel {
    rows: Vec<Row>,
    cells: Vec<Cell>,
    cell_blocks: Vec<Block>,
}

#[derive(Debug, Default)]
struct Sink {
    blocks: Vec<Block>,
    runs: Vec<Run>,
    tables: Vec<TableLevel>,
    marker: Option<String>,
    pending_row_header: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// A paragraph mark: honours `\intbl` and `\itapN`.
    Auto,
    /// End of a cell or row: always goes to the innermost table.
    Cell,
}

/// Reads RTF bytes into a structured document. The caller has checked the header.
pub(super) fn read(data: &[u8], opts: &Options) -> Result<RtfDocument, RtfError> {
    let limits = opts.limits;
    if data.len() > limits.max_input_bytes {
        return Err(RtfError::InputTooLarge {
            size: data.len(),
            limit: limits.max_input_bytes,
        });
    }
    let start = data
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(data.len());
    if !data[start..].starts_with(b"{\\rtf") {
        return Err(RtfError::InvalidHeader);
    }
    let mut reader = Reader::new(opts, start);
    reader.run(&data[start..])?;
    reader.finish()
}

struct Reader<'o> {
    opts: &'o Options,
    base: usize,
    offset: usize,
    groups: Vec<Group>,
    sinks: Vec<Sink>,
    fonts: HashMap<u16, FontInfo>,
    styles: HashMap<u16, StyleInfo>,
    resolved: HashMap<u16, CodePage>,
    default_font: u16,
    doc_cp: CodePage,
    ansicpg_set: bool,
    decoder: Decoder,
    scratch: String,
    aux: String,
    date: [i32; 5],
    /// Fallback characters still to skip after a `\uN`.
    skip: u32,
    /// A high surrogate from `\uN` waiting for its low half.
    high_surrogate: Option<u16>,
    replacements: usize,
    out_bytes: usize,
    hyperlinks: Vec<Hyperlink>,
    footnotes: Vec<Footnote>,
    parts: Vec<Part>,
    metadata: Metadata,
    warnings: Vec<Warning>,
    suppressed: usize,
    note_count: u32,
    error: Option<RtfError>,
}

impl<'o> Reader<'o> {
    fn new(opts: &'o Options, base: usize) -> Self {
        let base_group = Group {
            fresh: true,
            uc: 1,
            ..Group::default()
        };
        Self {
            opts,
            base,
            offset: base,
            groups: vec![base_group],
            sinks: vec![Sink::default()],
            fonts: HashMap::new(),
            styles: HashMap::new(),
            resolved: HashMap::new(),
            default_font: 0,
            doc_cp: charset::windows_1252(),
            ansicpg_set: false,
            decoder: Decoder::default(),
            scratch: String::new(),
            aux: String::new(),
            date: [0; 5],
            skip: 0,
            high_surrogate: None,
            replacements: 0,
            out_bytes: 0,
            hyperlinks: Vec::new(),
            footnotes: Vec::new(),
            parts: Vec::new(),
            metadata: Metadata::default(),
            warnings: Vec::new(),
            suppressed: 0,
            note_count: 0,
            error: None,
        }
    }

    fn run(&mut self, data: &[u8]) -> Result<(), RtfError> {
        let mut lexer = Lexer::new(data);
        let mut ended = false;
        for Lexed { token, offset } in lexer.by_ref() {
            self.offset = self.base + offset;
            match token {
                Token::Open => self.open_group(),
                Token::Close => {
                    if self.groups.len() <= 1 {
                        self.warn("unbalanced '}' ignored");
                    } else {
                        self.flush_all(true);
                        let outer = self.groups.len() == 2;
                        self.close_group();
                        if outer {
                            ended = true;
                            break;
                        }
                    }
                }
                Token::Word { name, param } => {
                    // `\u` is the one word that must see a pending high surrogate.
                    self.flush_decoder();
                    if name != b"u" {
                        self.flush_surrogate();
                    }
                    self.word(name, param);
                }
                Token::Symbol(byte) => {
                    self.flush_all(true);
                    self.symbol(byte);
                }
                Token::Hex(byte) => self.hex(byte),
                Token::Text(bytes) => self.text(bytes),
                Token::Binary { data, declared } => {
                    self.flush_all(true);
                    if data.len() < declared {
                        self.warn("truncated binary data (\\bin) at end of input");
                    }
                }
                Token::Malformed(message) => self.warn(message),
            }
            if let Some(error) = self.error.take() {
                return Err(error);
            }
        }
        if ended {
            let rest = &data[lexer.position()..];
            if rest.iter().any(|b| !b.is_ascii_whitespace()) {
                self.offset = self.base + lexer.position();
                self.warn("ignored content after the end of the document");
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<RtfDocument, RtfError> {
        self.flush_all(true);
        if self.groups.len() > 1 {
            let open = self.groups.len() - 1;
            self.warn(format!(
                "truncated group: {open} group(s) left open at end of input"
            ));
            while self.groups.len() > 1 {
                self.close_group();
            }
        }
        if self.replacements > 0 {
            let count = self.replacements;
            self.warn(format!(
                "{count} byte sequence(s) could not be decoded and were replaced with U+FFFD"
            ));
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.end_paragraph_if_any(0, Para::default());
        self.set_table_depth(0, 0);
        if self.suppressed > 0 {
            let message = format!("{} further warning(s) suppressed", self.suppressed);
            self.warnings.push(Warning {
                offset: self.base,
                message,
            });
        }
        let blocks = std::mem::take(&mut self.sink_mut(0).blocks);
        let mut footnotes = std::mem::take(&mut self.footnotes);
        footnotes.sort_by_key(|note| note.number);
        Ok(RtfDocument {
            metadata: std::mem::take(&mut self.metadata),
            blocks,
            hyperlinks: std::mem::take(&mut self.hyperlinks),
            footnotes,
            parts: std::mem::take(&mut self.parts),
            warnings: std::mem::take(&mut self.warnings),
        })
    }

    // ----- helpers -------------------------------------------------------------------------

    fn warn(&mut self, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(Warning {
                offset: self.offset,
                message: message.into(),
            });
        } else {
            self.suppressed += 1;
        }
    }

    fn top(&self) -> Group {
        self.groups.last().copied().unwrap_or_default()
    }

    fn top_mut(&mut self) -> &mut Group {
        if self.groups.is_empty() {
            self.groups.push(Group::default());
        }
        let last = self.groups.len() - 1;
        &mut self.groups[last]
    }

    fn parent_dest(&self) -> Option<Dest> {
        let n = self.groups.len();
        if n >= 2 {
            Some(self.groups[n - 2].dest)
        } else {
            None
        }
    }

    fn sink_mut(&mut self, idx: usize) -> &mut Sink {
        if idx >= self.sinks.len() {
            self.sinks.resize_with(idx + 1, Sink::default);
        }
        &mut self.sinks[idx]
    }

    fn new_sink(&mut self) -> usize {
        self.sinks.push(Sink::default());
        self.sinks.len() - 1
    }

    fn style_of(group: &Group) -> Style {
        Style {
            bold: group.chr.bold,
            italic: group.chr.italic,
            underline: group.chr.underline,
            strike: group.chr.strike,
            size_half_points: group.chr.size,
            link: group.link,
            footnote: None,
        }
    }

    fn accepts_text(dest: Dest) -> bool {
        matches!(
            dest,
            Dest::Normal | Dest::Meta(_) | Dest::FieldInst | Dest::ListText | Dest::StyleDef
        )
    }

    // ----- groups --------------------------------------------------------------------------

    fn open_group(&mut self) {
        self.flush_all(true);
        if self.groups.len() > self.opts.limits.max_depth {
            self.error = Some(RtfError::TooDeep {
                limit: self.opts.limits.max_depth,
            });
            return;
        }
        let mut child = self.top();
        child.fresh = true;
        child.star = false;
        child.owns_aux = false;
        child.owns_sink = None;
        child.font_entry = None;
        child.style_id = None;
        child.style_outline = None;
        self.groups.push(child);
    }

    fn close_group(&mut self) {
        if self.groups.len() <= 1 {
            return;
        }
        let Some(group) = self.groups.pop() else {
            return;
        };
        if group.owns_aux {
            self.finish_aux(group);
        }
        if let Some(kind) = group.owns_sink {
            let blocks = self.finish_sink(group.sink, group.para);
            match kind {
                SinkKind::Footnote(number) => self.footnotes.push(Footnote { number, blocks }),
                SinkKind::Part(part) => self.parts.push(Part { kind: part, blocks }),
            }
        }
    }

    fn finish_aux(&mut self, group: Group) {
        let text = std::mem::take(&mut self.aux);
        match group.dest {
            Dest::Meta(meta) => self.set_meta(meta, text.trim()),
            Dest::Date(kind) => {
                let formatted = format_date(self.date);
                match kind {
                    DateKind::Created => self.metadata.created = formatted,
                    DateKind::Modified => self.metadata.modified = formatted,
                }
            }
            Dest::FieldInst => {
                if let Some(target) = parse_hyperlink(&text) {
                    self.hyperlinks.push(Hyperlink { target });
                    let idx = self.hyperlinks.len() - 1;
                    if let Some(parent) = self.groups.last_mut() {
                        parent.link = Some(idx);
                    }
                }
            }
            Dest::ListText => {
                let marker = text.trim_matches(|c: char| c.is_whitespace()).to_string();
                self.sink_mut(group.sink).marker = Some(marker);
            }
            Dest::StyleDef => {
                if let Some(id) = group.style_id {
                    let name = text.trim().trim_end_matches(';').trim().to_string();
                    self.styles.insert(
                        id,
                        StyleInfo {
                            outline: group.style_outline,
                            name,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    fn set_meta(&mut self, meta: Meta, value: &str) {
        let target = match meta {
            Meta::Title => &mut self.metadata.title,
            Meta::Subject => &mut self.metadata.subject,
            Meta::Author => &mut self.metadata.author,
            Meta::Keywords => &mut self.metadata.keywords,
            Meta::Comments => &mut self.metadata.comments,
            Meta::Generator => &mut self.metadata.generator,
        };
        *target = value.trim_end_matches(';').trim_end().to_string();
    }

    // ----- control words -------------------------------------------------------------------

    fn word(&mut self, name: &[u8], param: Option<i32>) {
        let parent = self.parent_dest();
        let group = self.top();
        self.word_destination(name, group, parent);
        self.top_mut().star = false;
        self.control(name, param);
        self.top_mut().fresh = false;
    }

    /// Decides whether this word starts a destination, and sets it. Formatting words are
    /// handled by `control`.
    fn word_destination(&mut self, name: &[u8], group: Group, parent: Option<Dest>) {
        let star = group.star;
        let fresh = group.fresh;
        if fresh && parent == Some(Dest::Object) {
            let dest = if name == b"result" {
                Dest::Normal
            } else {
                Dest::Skip
            };
            self.top_mut().dest = dest;
            return;
        }
        if group.dest == Dest::Skip {
            return;
        }
        if fresh && parent == Some(Dest::StyleSheet) {
            self.start_aux(Dest::StyleDef);
            return;
        }
        if fresh && parent == Some(Dest::Info) {
            let target = match name {
                b"title" => Some(Dest::Meta(Meta::Title)),
                b"subject" => Some(Dest::Meta(Meta::Subject)),
                b"author" => Some(Dest::Meta(Meta::Author)),
                b"keywords" => Some(Dest::Meta(Meta::Keywords)),
                b"doccomm" => Some(Dest::Meta(Meta::Comments)),
                b"generator" => Some(Dest::Meta(Meta::Generator)),
                _ => None,
            };
            if let Some(dest) = target {
                self.start_aux(dest);
            } else if name == b"creatim" || name == b"revtim" {
                self.date = [0; 5];
                let kind = if name == b"creatim" {
                    DateKind::Created
                } else {
                    DateKind::Modified
                };
                self.start_aux(Dest::Date(kind));
            } else {
                self.top_mut().dest = Dest::Skip;
            }
            return;
        }
        let dest = match name {
            b"fonttbl" => Some(Dest::FontTable),
            b"stylesheet" => Some(Dest::StyleSheet),
            b"info" => Some(Dest::Info),
            b"object" => Some(Dest::Object),
            b"generator" => {
                self.start_aux(Dest::Meta(Meta::Generator));
                None
            }
            b"fldinst" => {
                self.start_aux(Dest::FieldInst);
                None
            }
            b"listtext" | b"pntext" => {
                self.start_aux(Dest::ListText);
                None
            }
            b"colortbl"
            | b"listtable"
            | b"listoverridetable"
            | b"pgdsctbl"
            | b"rsidtbl"
            | b"themedata"
            | b"colorschememapping"
            | b"xmlnstbl"
            | b"latentstyles"
            | b"datastore"
            | b"revtbl"
            | b"filetbl"
            | b"password"
            | b"pn"
            | b"mmathPr"
            | b"xe"
            | b"tc"
            | b"pict"
            | b"shppict"
            | b"nonshppict"
            | b"objdata"
            | b"objclass"
            | b"datafield"
            | b"sn"
            | b"sv"
            | b"sp"
            | b"ud"
            | b"bkmkstart"
            | b"bkmkend"
            | b"docvar" => Some(Dest::Skip),
            b"header" | b"headerl" | b"headerr" | b"headerf" | b"footer" | b"footerl"
            | b"footerr" | b"footerf" => {
                let kind = match name {
                    b"header" => PartKind::Header,
                    b"headerl" => PartKind::HeaderLeft,
                    b"headerr" => PartKind::HeaderRight,
                    b"headerf" => PartKind::HeaderFirst,
                    b"footer" => PartKind::Footer,
                    b"footerl" => PartKind::FooterLeft,
                    b"footerr" => PartKind::FooterRight,
                    _ => PartKind::FooterFirst,
                };
                if self.opts.include_headers_footers {
                    self.open_sink(SinkKind::Part(kind));
                    None
                } else {
                    Some(Dest::Skip)
                }
            }
            b"footnote" => {
                if self.opts.include_footnotes {
                    // The reference mark goes into the body; the note body becomes its own sink.
                    self.note_count += 1;
                    let number = self.note_count;
                    if self.top().dest == Dest::Normal {
                        let mut style = Self::style_of(&self.top());
                        style.footnote = Some(number);
                        self.push_run_with_style(&format!("[{number}]"), style);
                    }
                    self.open_sink(SinkKind::Footnote(number));
                    None
                } else {
                    Some(Dest::Skip)
                }
            }
            _ => {
                if star {
                    Some(Dest::Skip)
                } else {
                    None
                }
            }
        };
        if let Some(dest) = dest {
            self.top_mut().dest = dest;
        }
    }

    fn start_aux(&mut self, dest: Dest) {
        self.aux.clear();
        let group = self.top_mut();
        group.dest = dest;
        group.owns_aux = true;
    }

    /// Starts a new sink for the current group. The group keeps its own character and
    /// paragraph state, which the new sink's paragraphs use.
    fn open_sink(&mut self, kind: SinkKind) {
        let idx = self.new_sink();
        let group = self.top_mut();
        group.dest = Dest::Normal;
        group.sink = idx;
        group.owns_sink = Some(kind);
    }

    fn control(&mut self, name: &[u8], param: Option<i32>) {
        let dest = self.top().dest;
        let body = dest == Dest::Normal;
        let on = param != Some(0);
        match name {
            b"b" => self.top_mut().chr.bold = on,
            b"i" => self.top_mut().chr.italic = on,
            b"strike" => self.top_mut().chr.strike = on,
            b"v" => self.top_mut().chr.hidden = on,
            b"ulnone" => self.top_mut().chr.underline = false,
            b"ul" => self.top_mut().chr.underline = on,
            b"plain" => {
                let font = self.default_font;
                let group = self.top_mut();
                group.chr = Chr::default();
                group.font = font;
            }
            b"fs" => {
                let size = param
                    .filter(|v| *v > 0)
                    .map_or(0, |v| v.min(u16::MAX as i32) as u16);
                self.top_mut().chr.size = size;
            }
            b"f" => {
                if let Some(n) = param {
                    let number = n.max(0) as u16;
                    if dest == Dest::FontTable {
                        self.fonts.entry(number).or_default();
                        self.top_mut().font_entry = Some(number);
                    } else {
                        self.top_mut().font = number;
                    }
                }
            }
            b"fcharset" => {
                if let (Some(entry), Some(value)) = (self.top().font_entry, param) {
                    self.fonts.entry(entry).or_default().charset = Some(value.max(0) as u32);
                    self.resolved.clear();
                }
            }
            b"cpg" => {
                if let (Some(entry), Some(value)) = (self.top().font_entry, param) {
                    self.fonts.entry(entry).or_default().cpg = Some(value.max(0) as u32);
                    self.resolved.clear();
                }
            }
            b"deff" => {
                if let Some(n) = param {
                    self.default_font = n.max(0) as u16;
                }
            }
            b"ansicpg" => {
                if let Some(n) = param {
                    self.ansicpg_set = true;
                    self.doc_cp = self.resolve_number(n.max(0) as u32);
                    self.resolved.clear();
                }
            }
            b"ansi" => {
                if !self.ansicpg_set {
                    self.doc_cp = charset::windows_1252();
                    self.resolved.clear();
                }
            }
            b"mac" => {
                if !self.ansicpg_set {
                    self.doc_cp = self.resolve_number(10000);
                    self.resolved.clear();
                }
            }
            b"pc" | b"pca" => {
                if !self.ansicpg_set {
                    let n = if name == b"pc" { 437 } else { 850 };
                    self.doc_cp = self.resolve_number(n);
                    self.resolved.clear();
                }
            }
            b"uc" => self.top_mut().uc = param.unwrap_or(1).max(0) as u32,
            b"u" => self.unicode(param),
            b"pard" => self.top_mut().para = Para::default(),
            b"qc" => self.top_mut().para.align = Alignment::Center,
            b"ql" => self.top_mut().para.align = Alignment::Left,
            b"qr" => self.top_mut().para.align = Alignment::Right,
            b"qj" => self.top_mut().para.align = Alignment::Justify,
            b"li" => self.top_mut().para.li = param.unwrap_or(0),
            b"fi" => self.top_mut().para.fi = param.unwrap_or(0),
            b"s" => {
                if dest == Dest::StyleDef {
                    self.top_mut().style_id = param.map(|v| v.max(0) as u16);
                } else {
                    self.top_mut().para.style = param.map(|v| v.max(0) as u16);
                }
            }
            b"outlinelevel" => {
                let level = param.map(|v| v.clamp(0, 9) as u8);
                if dest == Dest::StyleDef {
                    self.top_mut().style_outline = level;
                } else {
                    self.top_mut().para.outline = level;
                }
            }
            b"intbl" => self.top_mut().para.intbl = true,
            b"itap" => self.top_mut().para.itap = param.unwrap_or(0).clamp(0, 255) as u8,
            b"ls" => self.top_mut().para.list = param.map(|v| v.max(0) as u32),
            b"ilvl" => self.top_mut().para.ilvl = param.unwrap_or(0).clamp(0, 8) as u8,
            b"yr" | b"mo" | b"dy" | b"hr" | b"min" if matches!(dest, Dest::Date(_)) => {
                let slot = match name {
                    b"yr" => 0,
                    b"mo" => 1,
                    b"dy" => 2,
                    b"hr" => 3,
                    _ => 4,
                };
                self.date[slot] = param.unwrap_or(0);
            }
            b"par" => {
                if body {
                    self.paragraph_break();
                }
            }
            b"page" | b"sect" | b"column" => {
                if body {
                    self.paragraph_break();
                }
            }
            b"line" => self.emit("\n"),
            b"tab" => self.emit("\t"),
            b"trowd" => {
                if body {
                    let idx = self.top().sink;
                    self.sink_mut(idx).pending_row_header = false;
                }
            }
            b"trhdr" => {
                if body {
                    let idx = self.top().sink;
                    self.sink_mut(idx).pending_row_header = true;
                }
            }
            b"cell" => {
                if body {
                    self.cell_end(false);
                }
            }
            b"nestcell" => {
                if body {
                    self.cell_end(true);
                }
            }
            b"row" => {
                if body {
                    self.row_end(false);
                }
            }
            b"nestrow" => {
                if body {
                    self.row_end(true);
                }
            }
            _ => {
                if let Some(c) = named_char(name) {
                    self.emit(c.encode_utf8(&mut [0; 4]));
                } else if name.starts_with(b"ul") && name != b"ulc" {
                    // Other underline styles (`\uld`, `\ulw`, `\uldb`, ...) are underline on.
                    self.top_mut().chr.underline = on;
                }
            }
        }
    }

    fn symbol(&mut self, byte: u8) {
        match byte {
            b'\\' | b'{' | b'}' => self.emit(char::from(byte).encode_utf8(&mut [0; 4])),
            b'~' => self.emit("\u{a0}"),
            b'_' => self.emit("\u{2011}"),
            b'\n' => {
                if self.top().dest == Dest::Normal {
                    self.paragraph_break();
                }
            }
            b'*' => {
                self.top_mut().star = true;
            }
            // `\-` optional hyphen, `\:` index subentry and `\|` formula character produce no text.
            _ => {}
        }
        if byte != b'*' {
            self.top_mut().fresh = false;
        }
    }

    fn unicode(&mut self, param: Option<i32>) {
        let Some(value) = param else {
            self.warn("\\u without a code point");
            return;
        };
        let unit = (value as i16) as u16;
        match unit {
            0xD800..=0xDBFF => {
                self.flush_surrogate();
                self.high_surrogate = Some(unit);
            }
            0xDC00..=0xDFFF => match self.high_surrogate.take() {
                Some(high) => {
                    let code =
                        0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00);
                    let c = char::from_u32(code).unwrap_or('\u{FFFD}');
                    self.emit(c.encode_utf8(&mut [0; 4]));
                }
                None => self.emit("\u{FFFD}"),
            },
            _ => {
                self.flush_surrogate();
                let c = char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}');
                self.emit(c.encode_utf8(&mut [0; 4]));
            }
        }
        self.skip = self.top().uc;
    }

    // ----- text ----------------------------------------------------------------------------

    fn resolve_number(&mut self, number: u32) -> CodePage {
        match charset::from_number(number) {
            Some(cp) => cp,
            None => {
                let message = if charset::is_known_unsupported(number) {
                    format!("code page {number} is not supported, used cp1252")
                } else {
                    format!("unknown code page {number}, used cp1252")
                };
                self.warn(message);
                charset::windows_1252()
            }
        }
    }

    fn code_page(&mut self, font: u16) -> CodePage {
        if let Some(cp) = self.resolved.get(&font) {
            return *cp;
        }
        let info = self.fonts.get(&font).copied().unwrap_or_default();
        let cp = if let Some(number) = info.cpg {
            self.resolve_number(number)
        } else {
            match info.charset.and_then(charset::charset_from_fcharset) {
                None | Some(Charset::Document) => self.doc_cp,
                Some(Charset::Symbol) => charset::symbol(),
                Some(Charset::Number(number)) => self.resolve_number(number),
            }
        };
        self.resolved.insert(font, cp);
        cp
    }

    fn text(&mut self, bytes: &[u8]) {
        self.top_mut().fresh = false;
        let mut bytes = bytes;
        if self.skip > 0 {
            let n = (self.skip as usize).min(bytes.len());
            self.skip -= n as u32;
            bytes = &bytes[n..];
        }
        if bytes.is_empty() {
            return;
        }
        let group = self.top();
        if !Self::accepts_text(group.dest) {
            return;
        }
        let cp = self.code_page(group.font);
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        let plain = self.decoder.is_idle()
            && bytes
                .iter()
                .all(|&b| (0x20..0x7f).contains(&b) || b == b'\t');
        if plain {
            if let Ok(s) = std::str::from_utf8(bytes) {
                out.push_str(s);
            }
        } else {
            self.replacements += self.decoder.push(cp, bytes, &mut out);
            strip_controls(&mut out);
        }
        self.emit(&out);
        self.scratch = out;
    }

    fn hex(&mut self, byte: u8) {
        self.top_mut().fresh = false;
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let group = self.top();
        if !Self::accepts_text(group.dest) {
            return;
        }
        let cp = self.code_page(group.font);
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        self.replacements += self.decoder.push(cp, &[byte], &mut out);
        strip_controls(&mut out);
        self.emit(&out);
        self.scratch = out;
    }

    /// Ends a multi-byte sequence the decoder is holding. `surrogate` also flushes a pending
    /// high surrogate, which is only correct when the next token is not `\u`.
    fn flush_all(&mut self, surrogate: bool) {
        self.flush_decoder();
        if surrogate {
            self.flush_surrogate();
        }
    }

    fn flush_decoder(&mut self) {
        if self.decoder.is_idle() {
            return;
        }
        let group = self.top();
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        self.replacements += self.decoder.flush(&mut out);
        // The pending bytes belong to the group that was current when they arrived, which is
        // still the current group here: a group boundary flushes before it changes state.
        if Self::accepts_text(group.dest) {
            self.emit(&out);
        }
        self.scratch = out;
    }

    fn flush_surrogate(&mut self) {
        if self.high_surrogate.take().is_some() {
            self.emit("\u{FFFD}");
        }
    }

    /// Sends text to the current destination. Any pending high surrogate is written first.
    fn emit(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.flush_surrogate();
        let group = self.top();
        match group.dest {
            Dest::Normal => {
                if !group.chr.hidden {
                    let style = Self::style_of(&group);
                    self.push_run_with_style(text, style);
                }
            }
            Dest::Meta(_) | Dest::FieldInst | Dest::ListText | Dest::StyleDef => {
                self.out_bytes = self.out_bytes.saturating_add(text.len());
                if self.out_bytes > self.opts.limits.max_output_bytes {
                    self.error = Some(RtfError::OutputTooLarge {
                        limit: self.opts.limits.max_output_bytes,
                    });
                    return;
                }
                self.aux.push_str(text);
            }
            _ => {}
        }
    }

    fn push_run_with_style(&mut self, text: &str, style: Style) {
        self.out_bytes = self.out_bytes.saturating_add(text.len());
        if self.out_bytes > self.opts.limits.max_output_bytes {
            self.error = Some(RtfError::OutputTooLarge {
                limit: self.opts.limits.max_output_bytes,
            });
            return;
        }
        let idx = self.top().sink;
        let sink = self.sink_mut(idx);
        if let Some(last) = sink.runs.last_mut() {
            if last.style == style {
                last.text.push_str(text);
                return;
            }
        }
        sink.runs.push(Run {
            text: text.to_string(),
            style,
        });
    }

    // ----- paragraphs, tables --------------------------------------------------------------

    fn paragraph_break(&mut self) {
        let group = self.top();
        self.end_paragraph(group.sink, group.para, Route::Auto);
    }

    fn end_paragraph_if_any(&mut self, idx: usize, para: Para) {
        let sink = self.sink_mut(idx);
        if !sink.runs.is_empty() || sink.marker.is_some() {
            self.end_paragraph(idx, para, Route::Auto);
        }
    }

    fn end_paragraph(&mut self, idx: usize, para: Para, route: Route) {
        let (runs, marker) = {
            let sink = self.sink_mut(idx);
            (std::mem::take(&mut sink.runs), sink.marker.take())
        };
        let blank = marker.is_none()
            && para.list.is_none()
            && runs.iter().all(|r| r.text.trim().is_empty());
        let block = if marker.is_some() || para.list.is_some() {
            let marker = marker.unwrap_or_default();
            Block::ListItem(ListItem {
                level: para.ilvl,
                ordered: is_ordered_marker(&marker),
                marker,
                runs,
            })
        } else {
            let heading_level = self.heading_level(&para, &runs);
            Block::Paragraph(Paragraph {
                runs,
                heading_level,
                alignment: para.align,
                left_indent_twips: para.li,
                first_line_indent_twips: para.fi,
            })
        };
        let level = usize::from(para.itap.max(1));
        match route {
            Route::Auto if !para.intbl => {
                self.set_table_depth(idx, 0);
                self.sink_mut(idx).blocks.push(block);
            }
            Route::Auto => {
                self.set_table_depth(idx, level);
                if !blank {
                    self.push_to_cell(idx, block);
                }
            }
            Route::Cell => {
                // A cell paragraph carries its own nesting level, so `\nestcell` after a nested
                // paragraph lands in the nested table.
                if para.intbl {
                    self.set_table_depth(idx, level);
                }
                if !blank {
                    self.push_to_cell(idx, block);
                }
            }
        }
    }

    fn push_to_cell(&mut self, idx: usize, block: Block) {
        let sink = self.sink_mut(idx);
        if sink.tables.is_empty() {
            sink.tables.push(TableLevel::default());
        }
        if let Some(level) = sink.tables.last_mut() {
            level.cell_blocks.push(block);
        }
    }

    fn cell_end(&mut self, nested: bool) {
        let group = self.top();
        let idx = group.sink;
        self.end_paragraph(idx, group.para, Route::Cell);
        if !nested {
            self.set_table_depth(idx, 1);
        }
        let sink = self.sink_mut(idx);
        if sink.tables.is_empty() {
            sink.tables.push(TableLevel::default());
        }
        if let Some(level) = sink.tables.last_mut() {
            let blocks = std::mem::take(&mut level.cell_blocks);
            level.cells.push(Cell { blocks });
        }
    }

    fn row_end(&mut self, nested: bool) {
        let group = self.top();
        let idx = group.sink;
        self.end_paragraph(idx, group.para, Route::Cell);
        if !nested {
            self.set_table_depth(idx, 1);
        }
        let sink = self.sink_mut(idx);
        let header = sink.pending_row_header;
        sink.pending_row_header = false;
        if let Some(level) = sink.tables.last_mut() {
            if !level.cell_blocks.is_empty() {
                let blocks = std::mem::take(&mut level.cell_blocks);
                level.cells.push(Cell { blocks });
            }
            if !level.cells.is_empty() {
                let cells = std::mem::take(&mut level.cells);
                level.rows.push(Row { header, cells });
            }
        }
    }

    /// Makes the table stack exactly `target` levels deep. Deeper tables are closed and attached
    /// to the cell that contains them, or to the sink when no cell is open.
    fn set_table_depth(&mut self, idx: usize, target: usize) {
        let sink = self.sink_mut(idx);
        while sink.tables.len() > target {
            let Some(mut level) = sink.tables.pop() else {
                break;
            };
            if !level.cell_blocks.is_empty() {
                let blocks = std::mem::take(&mut level.cell_blocks);
                level.cells.push(Cell { blocks });
            }
            if !level.cells.is_empty() {
                let cells = std::mem::take(&mut level.cells);
                let header = sink.pending_row_header;
                sink.pending_row_header = false;
                level.rows.push(Row { header, cells });
            }
            if !level.rows.is_empty() {
                let table = Block::Table(Table {
                    rows: std::mem::take(&mut level.rows),
                });
                match sink.tables.last_mut() {
                    Some(parent) => parent.cell_blocks.push(table),
                    None => sink.blocks.push(table),
                }
            }
        }
        while sink.tables.len() < target {
            sink.tables.push(TableLevel::default());
        }
    }

    fn finish_sink(&mut self, idx: usize, para: Para) -> Vec<Block> {
        self.end_paragraph_if_any(idx, para);
        self.set_table_depth(idx, 0);
        std::mem::take(&mut self.sink_mut(idx).blocks)
    }

    fn heading_level(&self, para: &Para, runs: &[Run]) -> Option<u8> {
        if let Some(outline) = para.outline {
            if outline < 9 {
                return Some(outline + 1);
            }
            return None;
        }
        if let Some(style) = para.style {
            if let Some(info) = self.styles.get(&style) {
                if let Some(outline) = info.outline {
                    if outline < 9 {
                        return Some(outline + 1);
                    }
                }
                if let Some(level) = heading_from_style_name(&info.name) {
                    return Some(level);
                }
            }
        }
        if self.opts.detect_headings {
            large_bold_level(runs)
        } else {
            None
        }
    }
}

/// Characters for named RTF control words that produce text.
fn named_char(name: &[u8]) -> Option<char> {
    Some(match name {
        b"lquote" => '\u{2018}',
        b"rquote" => '\u{2019}',
        b"ldblquote" => '\u{201c}',
        b"rdblquote" => '\u{201d}',
        b"emdash" => '\u{2014}',
        b"endash" => '\u{2013}',
        b"bullet" => '\u{2022}',
        b"emspace" => '\u{2003}',
        b"enspace" => '\u{2002}',
        b"qmspace" => '\u{2005}',
        b"zwj" => '\u{200d}',
        b"zwnj" => '\u{200c}',
        b"ltrmark" => '\u{200e}',
        b"rtlmark" => '\u{200f}',
        _ => return None,
    })
}

/// Removes C0 control characters other than tab. RTF has no use for them in text.
fn strip_controls(text: &mut String) {
    if text.bytes().any(|b| b < 0x20 && b != b'\t') {
        text.retain(|c| c >= ' ' || c == '\t');
    }
}

fn heading_from_style_name(name: &str) -> Option<u8> {
    let lower = name
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("heading ") {
        return rest
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=6).contains(n));
    }
    if lower == "title" {
        return Some(1);
    }
    None
}

/// A short paragraph whose visible text is all bold and at least 14 pt is a heading: 18 pt or
/// larger is level 1, 14 pt or larger is level 2.
pub(super) fn large_bold_level(runs: &[Run]) -> Option<u8> {
    let visible: Vec<&Run> = runs.iter().filter(|r| !r.text.trim().is_empty()).collect();
    if visible.is_empty() || visible.iter().any(|r| !r.style.bold) {
        return None;
    }
    let length: usize = visible.iter().map(|r| r.text.trim().len()).sum();
    if length > 200 {
        return None;
    }
    let size = visible
        .iter()
        .map(|r| r.style.size_half_points)
        .max()
        .unwrap_or(0);
    if size >= 36 {
        Some(1)
    } else if size >= 28 {
        Some(2)
    } else {
        None
    }
}

fn is_ordered_marker(marker: &str) -> bool {
    let m = marker.trim();
    (m.ends_with('.') || m.ends_with(')')) && m.chars().any(char::is_alphanumeric)
}

fn format_date(d: [i32; 5]) -> String {
    if d[0] <= 0 {
        return String::new();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        d[0],
        d[1].max(1),
        d[2].max(1),
        d[3].max(0),
        d[4].max(0)
    )
}

/// Reads the target of a `HYPERLINK` field instruction. `\l` gives an anchor, appended as `#anchor`.
pub(super) fn parse_hyperlink(instruction: &str) -> Option<String> {
    let s = instruction.trim();
    let head = s.get(..9)?;
    if !head.eq_ignore_ascii_case("HYPERLINK") {
        return None;
    }
    let mut rest = s[9..].trim_start();
    let mut target = String::new();
    let mut anchor: Option<String> = None;
    if !rest.is_empty() && !rest.starts_with('\\') {
        let (arg, tail) = take_argument(rest);
        target = arg;
        rest = tail;
    }
    loop {
        rest = rest.trim_start();
        let Some(after) = rest.strip_prefix('\\') else {
            break;
        };
        let mut chars = after.chars();
        let letter = chars.next();
        let tail = chars.as_str();
        match letter {
            Some('l') => {
                let (arg, next) = take_argument(tail.trim_start());
                anchor = Some(arg);
                rest = next;
            }
            Some('o') | Some('t') => {
                let (_, next) = take_argument(tail.trim_start());
                rest = next;
            }
            _ => rest = tail,
        }
    }
    let mut out = target;
    if let Some(a) = anchor {
        if !a.is_empty() {
            out.push('#');
            out.push_str(&a);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Reads one field argument, quoted or bare. Returns the argument and the rest of the input.
fn take_argument(s: &str) -> (String, &str) {
    if let Some(quoted) = s.strip_prefix('"') {
        return match quoted.find('"') {
            Some(end) => (quoted[..end].to_string(), &quoted[end + 1..]),
            None => (quoted.to_string(), ""),
        };
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    (s[..end].to_string(), &s[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperlink_targets() {
        assert_eq!(
            parse_hyperlink(" HYPERLINK \"https://example.test/a b\" "),
            Some("https://example.test/a b".into())
        );
        assert_eq!(
            parse_hyperlink("HYPERLINK \\l \"_Toc1\""),
            Some("#_Toc1".into())
        );
        assert_eq!(
            parse_hyperlink("HYPERLINK \"http://x\" \\l \"top\""),
            Some("http://x#top".into())
        );
        assert_eq!(parse_hyperlink("PAGE"), None);
        assert_eq!(parse_hyperlink("HYPERLINK"), None);
    }

    #[test]
    fn heading_names_and_sizes() {
        assert_eq!(heading_from_style_name("heading 3;"), Some(3));
        assert_eq!(heading_from_style_name("Heading 9"), None);
        assert_eq!(heading_from_style_name("Normal"), None);
    }

    #[test]
    fn ordered_markers() {
        assert!(is_ordered_marker("2."));
        assert!(is_ordered_marker("(a)"));
        assert!(is_ordered_marker("a)"));
        assert!(!is_ordered_marker("\u{2022}"));
        assert!(!is_ordered_marker("o"));
    }
}
