//! Public data model and options for RTF reading and writing.

use std::fmt;

/// A parsed RTF document. Structure first; plain text and Markdown are rendered from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RtfDocument {
    pub metadata: Metadata,
    /// Body blocks in reading order.
    pub blocks: Vec<Block>,
    /// Targets of hyperlink fields. `Style::link` indexes this list.
    pub hyperlinks: Vec<Hyperlink>,
    /// Footnote bodies in note-number order. Empty when footnotes are excluded.
    pub footnotes: Vec<Footnote>,
    /// Headers and footers. Empty unless `Options::include_headers_footers` is set.
    pub parts: Vec<Part>,
    /// Recoverable problems found while reading, in input order.
    pub warnings: Vec<Warning>,
}

/// Values from the `\info` destination. Dates are `YYYY-MM-DDTHH:MM`; empty when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: String,
    pub subject: String,
    pub author: String,
    pub keywords: String,
    pub comments: String,
    pub generator: String,
    pub created: String,
    pub modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footnote {
    pub number: u32,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Header,
    HeaderLeft,
    HeaderRight,
    HeaderFirst,
    Footer,
    FooterLeft,
    FooterRight,
    FooterFirst,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub kind: PartKind,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Byte offset of the token that caused the warning.
    pub offset: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Paragraph),
    ListItem(ListItem),
    Table(Table),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    /// 1 to 6 when the paragraph is a heading.
    pub heading_level: Option<u8>,
    pub alignment: Alignment,
    pub left_indent_twips: i32,
    pub first_line_indent_twips: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// Zero-based list level (`\ilvlN`).
    pub level: u8,
    pub ordered: bool,
    /// Marker text from `\listtext` (for example `•` or `2.`). Empty when the document gave none.
    pub marker: String,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub header: bool,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cell {
    /// Paragraphs, lists and nested tables, in order.
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Font size in half-points (`\fsN`); 0 when unset.
    pub size_half_points: u16,
    /// Index into `RtfDocument::hyperlinks`.
    pub link: Option<usize>,
    /// Footnote number when the run is a footnote reference.
    pub footnote: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// How tables are written in text and Markdown output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableStyle {
    /// Markdown pipe tables (Markdown output); cells joined by ` | ` in plain text.
    #[default]
    Pipes,
    /// Tab-separated rows, one line per row, in both plain text and Markdown output.
    Tabs,
}

/// Resource limits. Exceeding one is an error, never a silent truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum group nesting depth.
    pub max_depth: usize,
    /// Maximum input size in bytes.
    pub max_input_bytes: usize,
    /// Maximum size of rendered text or Markdown, and of the text collected while reading.
    pub max_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_input_bytes: 128 * 1024 * 1024,
            max_output_bytes: 128 * 1024 * 1024,
        }
    }
}

/// Reader and renderer options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Read headers and footers into `RtfDocument::parts`. Default: false.
    pub include_headers_footers: bool,
    /// Read footnotes into `RtfDocument::footnotes` and mark references in the body. Default: true.
    pub include_footnotes: bool,
    /// Use outline levels, styles and large bold paragraphs to find headings. Default: true.
    pub detect_headings: bool,
    pub table_style: TableStyle,
    pub limits: Limits,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            include_headers_footers: false,
            include_footnotes: true,
            detect_headings: true,
            table_style: TableStyle::Pipes,
            limits: Limits::default(),
        }
    }
}

/// Errors returned by the RTF reader and renderers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtfError {
    /// The input does not start with `{\rtf`.
    InvalidHeader,
    /// Group nesting exceeded `Limits::max_depth`.
    TooDeep { limit: usize },
    /// Input exceeded `Limits::max_input_bytes`.
    InputTooLarge { size: usize, limit: usize },
    /// Output exceeded `Limits::max_output_bytes`.
    OutputTooLarge { limit: usize },
}

impl fmt::Display for RtfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "invalid RTF header"),
            Self::TooDeep { limit } => write!(f, "RTF group nesting exceeds {limit}"),
            Self::InputTooLarge { size, limit } => {
                write!(
                    f,
                    "RTF input of {size} bytes exceeds the {limit} byte limit"
                )
            }
            Self::OutputTooLarge { limit } => {
                write!(f, "RTF output exceeds the {limit} byte limit")
            }
        }
    }
}

impl std::error::Error for RtfError {}
