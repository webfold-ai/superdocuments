//! A small document tree shared by the readers that do not have a model of their own (HTML,
//! EPUB, OpenDocument text, reStructuredText), and a writer that turns it into Pandoc-flavoured
//! Markdown. It is deliberately close to Pandoc's own `Block` and `Inline` types, because the
//! point of these readers is to give the same structure Pandoc gives: headings with levels,
//! nested and ordered lists, tables with alignment, code blocks with a language, footnotes,
//! links, images, emphasis, strikeout, super and subscripts, and math kept as TeX.
//!
//! The writer in `mdwrite` emits the Pandoc Markdown that reads back to the same tree: pipe
//! tables, fenced code, `$` math, `[^n]` footnotes, `~~strike~~`, `^sup^`, `~sub~`, definition
//! lists, and YAML front matter for title, author, date and abstract.

/// Title, authors, date and abstract of a document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub title: Vec<Inline>,
    pub authors: Vec<Vec<Inline>>,
    pub date: Option<String>,
    pub abstract_blocks: Vec<Block>,
}

impl Meta {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty()
            && self.authors.is_empty()
            && self.date.is_none()
            && self.abstract_blocks.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub meta: Meta,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Default,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStyle {
    #[default]
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub colspan: usize,
    pub rowspan: usize,
}

impl Cell {
    pub fn text(inlines: Vec<Inline>) -> Self {
        Cell {
            blocks: if inlines.is_empty() {
                Vec::new()
            } else {
                vec![Block::Plain(inlines)]
            },
            colspan: 1,
            rowspan: 1,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub caption: Vec<Inline>,
    pub align: Vec<Align>,
    /// Header rows (usually zero or one).
    pub head: Vec<Vec<Cell>>,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Para(Vec<Inline>),
    /// Like `Para`, but tight (list items and table cells).
    Plain(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        id: Option<String>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    BulletList(Vec<Vec<Block>>),
    OrderedList {
        start: u32,
        style: ListStyle,
        items: Vec<Vec<Block>>,
    },
    DefList(Vec<(Vec<Inline>, Vec<Vec<Block>>)>),
    Table(Table),
    /// An image on its own with a caption (a Pandoc `Figure`).
    Figure {
        url: String,
        caption: Vec<Inline>,
    },
    Rule,
    /// Text kept as written for a format Markdown cannot express (`format` is `tex`, `html`...).
    Raw {
        format: String,
        text: String,
    },
    /// A container with classes (admonitions, theorem environments); written as a fenced div.
    Div {
        classes: Vec<String>,
        blocks: Vec<Block>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    Sup(Vec<Inline>),
    Sub(Vec<Inline>),
    SmallCaps(Vec<Inline>),
    Underline(Vec<Inline>),
    Code(String),
    Math {
        display: bool,
        tex: String,
    },
    Link {
        url: String,
        title: Option<String>,
        inlines: Vec<Inline>,
    },
    Image {
        url: String,
        alt: Vec<Inline>,
        title: Option<String>,
    },
    Note(Vec<Block>),
    LineBreak,
    SoftBreak,
    /// A citation in Pandoc syntax, already formatted: written as given, not escaped.
    Cite(String),
    /// Inline text written as given (no escaping).
    Verbatim(String),
}

/// Plain text of inlines (markup removed).
pub fn inlines_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Cite(t) | Inline::Verbatim(t) => {
                s.push_str(t)
            }
            Inline::Emph(v)
            | Inline::Strong(v)
            | Inline::Strike(v)
            | Inline::Sup(v)
            | Inline::Sub(v)
            | Inline::SmallCaps(v)
            | Inline::Underline(v) => s.push_str(&inlines_text(v)),
            Inline::Link { inlines, .. } => s.push_str(&inlines_text(inlines)),
            Inline::Image { alt, .. } => s.push_str(&inlines_text(alt)),
            Inline::Math { tex, .. } => s.push_str(tex),
            Inline::LineBreak | Inline::SoftBreak => s.push(' '),
            Inline::Note(_) => {}
        }
    }
    s
}

pub use crate::mdwrite::{blocks_to_markdown, inlines_to_markdown, to_markdown};
