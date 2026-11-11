//! A bounded LaTeX reader. It is not a TeX engine: it reads the document structure and keeps
//! math as TeX source.
//!
//! `parse` returns a `Document`: title, authors, abstract, blocks (sections, paragraphs, lists,
//! tables, display math, verbatim text, figures, theorem-like environments, quotations), labels
//! with their numbers, `thebibliography` items, and warnings. Inline content keeps emphasis,
//! citations, references, links, footnotes and inline math. `figures` lists the figure
//! environments with captions, labels, image paths and subfigures.
//!
//! What it does and does not do:
//! - Macros defined with `\newcommand`, `\renewcommand`, `\providecommand`, `\def`,
//!   `\DeclareMathOperator`, `\newenvironment` and `\newtheorem` (starred forms included) are
//!   expanded in text, with limits on the number of expansions, the expansion size and the
//!   nesting depth. Inside math they are expanded only with `Options::expand_math_macros`.
//!   `\def` with delimited parameters is not supported.
//! - Conditionals are not evaluated, except `\iffalse`, `\iftrue` and switches made with
//!   `\newif`. For any other `\if...` the first branch is kept.
//! - Unknown commands produce a warning and the content of their arguments is kept.
//! - `\input`, `\include` and `\subfile` are read only through a `FileProvider`. Without one
//!   they are recorded in `Document::inputs` with a warning. With one, the file is inlined; a
//!   file that includes itself is refused, and at most 32 files are open at once. `DirFiles`
//!   is a provider for one directory tree on disk, which never reads outside its root.
//!   `parse` itself never touches the file system.
//! - Math is kept as TeX. Its labels and numbering are computed from the environment, not from
//!   a TeX run.
//! - Packages are not loaded. Only what the built-in tables describe is recognised.
//!
//! Renderers (`render_text`, `render_markdown`, `render_html`) turn a `Document` into text.
//! `resolve_citations` links `\cite` keys to a `bibtex::Bibliography`.

mod files;
mod lexer;
mod parser;
mod render;

#[cfg(test)]
mod tests;

pub use files::{DirFiles, DEFAULT_MAX_FILE_BYTES, DEFAULT_MAX_TOTAL_BYTES};
pub use render::{render_html, render_markdown, render_text, RenderOptions};

use crate::bibtex::Bibliography;

/// Bounds for reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Largest accepted source, in bytes. Larger input yields an empty document and a warning.
    pub max_input_bytes: usize,
    /// Most macro expansions in one parse.
    pub max_expansions: usize,
    /// Largest total size of macro expansions, in bytes.
    pub max_expansion_bytes: usize,
    /// Deepest group, environment or expansion nesting.
    pub max_depth: usize,
    /// Most input files read through the `FileProvider` in one parse.
    pub max_files: usize,
    /// Most warnings recorded; further warnings are counted but not stored.
    pub max_warnings: usize,
    /// Expand the document's own macros inside math as well as in text, so that the TeX in
    /// `Inline::Math` and `MathRow::tex` uses no command the document defined itself. Off by
    /// default: math is then kept exactly as written. `document::convert_bytes` and
    /// `convert_path` turn it on, because Markdown math renderers do not know the macros.
    pub expand_math_macros: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_input_bytes: 32 << 20,
            max_expansions: 100_000,
            max_expansion_bytes: 4 << 20,
            max_depth: 128,
            max_files: 256,
            max_warnings: 10_000,
            expand_math_macros: false,
        }
    }
}

/// Supplies the text of `\input` and `\include` files. The name is as written in the source;
/// the parser also tries the name with `.tex` appended.
pub trait FileProvider {
    fn read_tex(&self, name: &str) -> Option<String>;
}

/// A `FileProvider` that resolves nothing.
pub struct NoFiles;

impl FileProvider for NoFiles {
    fn read_tex(&self, _name: &str) -> Option<String> {
        None
    }
}

/// A problem found while reading. `offset` is in the main source, or the invocation offset for
/// text that came from a macro expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub offset: usize,
    pub message: String,
}

/// A parsed LaTeX document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub title: Option<Vec<Inline>>,
    /// One entry per `\and` or `\AND` in `\author`. A line break inside an author's block
    /// (an affiliation) stays in that author's inlines.
    pub authors: Vec<Vec<Inline>>,
    pub date: Option<String>,
    pub abstract_body: Option<Vec<Block>>,
    pub blocks: Vec<Block>,
    /// Every `\label` with its number, in source order.
    pub labels: Vec<LabelInfo>,
    /// Items of `thebibliography`.
    pub bibitems: Vec<BibItem>,
    /// Keys and files named by `\bibliography` and `\addbibresource`.
    pub bibliography_files: Vec<String>,
    /// `\input` and `\include` names that were not resolved.
    pub inputs: Vec<String>,
    pub warnings: Vec<Warning>,
    /// Warnings dropped after `Options::max_warnings` was reached.
    pub suppressed_warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelInfo {
    pub name: String,
    /// `section`, `appendix`, `figure`, `subfigure`, `table`, `equation`, `theorem`,
    /// `algorithm`, or `label` for a label in running text.
    pub kind: String,
    pub number: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BibItem {
    pub key: String,
    /// The optional `[label]` of the item.
    pub label: Option<String>,
    pub text: Vec<Inline>,
    /// The source text of the item, up to the next `\bibitem` or `\end{thebibliography}`. Empty
    /// when the item came from a macro expansion.
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Section(Section),
    Paragraph(Vec<Inline>),
    List(List),
    Table(Table),
    /// Display math (`$$`, `\[`, `equation`, `align`, ...).
    Math(MathBlock),
    Verbatim(Verbatim),
    Figure(Figure),
    Theorem(Theorem),
    Quote(Vec<Block>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// 1 for `\section` (and `\chapter`), 2 for `\subsection`, 3 for `\subsubsection`,
    /// 4 for `\paragraph`, 5 for `\subparagraph`.
    pub level: u8,
    pub starred: bool,
    pub title: Vec<Inline>,
    pub number: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListKind {
    Itemize,
    Enumerate,
    Description,
}

#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub kind: ListKind,
    pub items: Vec<ListItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    /// The `[label]` of a description item, or of an item with a custom label.
    pub label: Option<Vec<Inline>>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// `table`, `table*`, `tabular`, `tabularx`, `longtable` or `tabular*`.
    pub env: String,
    pub caption: Option<Vec<Inline>>,
    pub caption_text: Option<String>,
    pub label: Option<String>,
    pub number: Option<String>,
    /// The column specification, such as `lcr` or `|c|c|`.
    pub columns: Option<String>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub content: Vec<Inline>,
    /// Columns spanned (`\multicolumn{n}`), at least 1.
    pub colspan: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MathBlock {
    /// `$$`, `\[`, `equation`, `align*`, and so on.
    pub env: String,
    pub rows: Vec<MathRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MathRow {
    /// TeX source with `\label`, `\nonumber` and `\notag` removed.
    pub tex: String,
    pub number: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verbatim {
    /// `verbatim`, `lstlisting`, `minted`, `alltt`, and so on.
    pub env: String,
    pub language: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theorem {
    /// The environment name, such as `theorem` or `proof`.
    pub env: String,
    /// The printed name, such as `Theorem`. Empty for an unnumbered environment.
    pub name: String,
    /// The optional `[title]` argument.
    pub title: Option<Vec<Inline>>,
    pub number: Option<String>,
    pub label: Option<String>,
    pub body: Vec<Block>,
}

/// The source of a picture environment (`tikzpicture`, `pgfpicture`, `picture`, `circuitikz`,
/// `forest`, `pspicture`, `tikzcd`, `asy`). Pictures are not drawn: the source is kept whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// The environment name.
    pub env: String,
    /// The environment as written, from `\begin{...}` (with its options) to `\end{...}`.
    pub source: String,
}

/// A figure environment (`figure`, `figure*`, `wrapfigure`, `subfigure`, `subfloat`) or a
/// picture environment that stands on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    /// Sequential number among all figures and subfigures, in source order.
    pub id: usize,
    pub env: String,
    pub starred: bool,
    pub label: Option<String>,
    pub number: Option<String>,
    pub caption: Option<Vec<Inline>>,
    /// The caption as plain text.
    pub caption_text: Option<String>,
    /// `\includegraphics` paths directly in this figure.
    pub image_paths: Vec<ImagePath>,
    pub subfigures: Vec<Figure>,
    /// The body of the first `tikzpicture`, without its options. Kept for callers that read
    /// it; `pictures` has every picture with its complete source.
    pub tikz: Option<String>,
    /// Every picture environment directly in this figure, in source order. The renderers
    /// write each one as a fenced `latex` code block; none is drawn.
    pub pictures: Vec<Picture>,
    /// Byte range of the environment in the main source (approximate inside expansions).
    pub source_span: (usize, usize),
}

/// One `\includegraphics`: the declared path, the candidate files, and the options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePath {
    /// The path exactly as written.
    pub declared: String,
    /// The files to try, in the order graphicx tries them. A path that already has a known
    /// image extension gives only itself.
    pub candidates: Vec<String>,
    /// Options as key and value pairs, such as `("width", "0.5\\textwidth")`.
    pub options: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    /// `\texttt` and `\tt`.
    Mono(Vec<Inline>),
    SmallCaps(Vec<Inline>),
    /// `\verb` content.
    Code(String),
    /// Inline math (`$...$`, `\(...\)`, `math`), or display math inside a paragraph.
    Math {
        tex: String,
        display: bool,
    },
    Cite(Cite),
    Ref(Ref),
    Link {
        url: String,
        text: Vec<Inline>,
    },
    Image(ImagePath),
    Footnote(Vec<Inline>),
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cite {
    /// `cite`, `citep`, `citet`, and so on.
    pub command: String,
    pub pre: Option<String>,
    pub post: Option<String>,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// `ref`, `eqref`, `pageref`, `cref`, `autoref`, `subref`, and so on.
    pub command: String,
    /// The label as written. `\cref` and `\Cref` take several, separated by commas.
    pub label: String,
}

impl Ref {
    /// The labels this reference names: one, or several for the cleveref commands.
    pub fn labels(&self) -> Vec<&str> {
        if matches!(self.command.as_str(), "cref" | "Cref" | "labelcref") {
            self.label
                .split(',')
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect()
        } else {
            vec![self.label.as_str()]
        }
    }
}

impl Document {
    /// The number recorded for a label (the first with that name).
    pub fn label_number(&self, name: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|l| l.name == name)
            .and_then(|l| l.number.as_deref())
    }

    /// Every `\ref`, `\eqref` and similar whose label is not defined in the document.
    pub fn unresolved_references(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut visit = |inl: &Inline| {
            if let Inline::Ref(r) = inl {
                for name in r.labels() {
                    if !self.labels.iter().any(|l| l.name == name) && !out.iter().any(|o| o == name)
                    {
                        out.push(name.to_string());
                    }
                }
            }
        };
        for_each_inline(&self.blocks, &mut visit);
        out
    }
}

/// The plain text of inline content: markup removed, runs of spaces collapsed.
pub fn plain_text(inlines: &[Inline]) -> String {
    parser::plain(inlines)
}

/// Parses LaTeX with the default options and no file access.
pub fn parse(source: &str) -> Document {
    parser::parse(source, &Options::default(), None)
}

/// Parses LaTeX with explicit options and an optional file provider for `\input`.
pub fn parse_with(source: &str, options: &Options, files: Option<&dyn FileProvider>) -> Document {
    parser::parse(source, options, files)
}

/// Every top-level figure environment, in source order, with subfigures nested. Figures inside
/// lists, theorems and quotations are included. Bare `\includegraphics` in running text is not
/// a figure: it is `Inline::Image`.
pub fn figures(doc: &Document) -> Vec<Figure> {
    let mut out = Vec::new();
    collect_figures(&doc.blocks, &mut out);
    out
}

fn collect_figures(blocks: &[Block], out: &mut Vec<Figure>) {
    for block in blocks {
        match block {
            Block::Figure(f) => out.push(f.clone()),
            Block::List(list) => {
                for item in &list.items {
                    collect_figures(&item.blocks, out);
                }
            }
            Block::Theorem(t) => collect_figures(&t.body, out),
            Block::Quote(inner) => collect_figures(inner, out),
            _ => {}
        }
    }
}

/// Links each citation key to the document's `thebibliography` items first, then to the given
/// `Bibliography`. The result has one entry per key in order of first appearance.
pub fn resolve_citations(doc: &Document, bib: Option<&Bibliography>) -> Vec<CitationLink> {
    let mut keys: Vec<String> = Vec::new();
    let mut visit = |inl: &Inline| {
        if let Inline::Cite(c) = inl {
            for k in &c.keys {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
    };
    for_each_inline(&doc.blocks, &mut visit);
    keys.into_iter()
        .map(|key| {
            let in_doc = doc.bibitems.iter().position(|b| b.key == key);
            let in_bib = bib.and_then(|b| b.entries.iter().position(|e| e.key == key));
            let source = if in_doc.is_some() {
                CitationSource::Thebibliography
            } else if in_bib.is_some() {
                CitationSource::Bibliography
            } else {
                CitationSource::Unresolved
            };
            CitationLink { key, source }
        })
        .collect()
}

/// Where a citation key was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationLink {
    pub key: String,
    pub source: CitationSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CitationSource {
    Thebibliography,
    Bibliography,
    Unresolved,
}

/// Calls `f` for every inline in the blocks, including inlines inside lists, tables, figures,
/// theorems, quotations, footnotes and captions.
fn for_each_inline(blocks: &[Block], f: &mut dyn FnMut(&Inline)) {
    fn walk_inlines(inls: &[Inline], f: &mut dyn FnMut(&Inline)) {
        for inl in inls {
            f(inl);
            match inl {
                Inline::Emph(v)
                | Inline::Strong(v)
                | Inline::Mono(v)
                | Inline::SmallCaps(v)
                | Inline::Footnote(v) => walk_inlines(v, f),
                Inline::Link { text, .. } => walk_inlines(text, f),
                _ => {}
            }
        }
    }
    for block in blocks {
        match block {
            Block::Section(s) => walk_inlines(&s.title, f),
            Block::Paragraph(inls) => walk_inlines(inls, f),
            Block::List(list) => {
                for item in &list.items {
                    if let Some(label) = &item.label {
                        walk_inlines(label, f);
                    }
                    for_each_inline(&item.blocks, f);
                }
            }
            Block::Table(t) => {
                if let Some(c) = &t.caption {
                    walk_inlines(c, f);
                }
                for row in &t.rows {
                    for cell in &row.cells {
                        walk_inlines(&cell.content, f);
                    }
                }
            }
            Block::Figure(fig) => {
                if let Some(c) = &fig.caption {
                    walk_inlines(c, f);
                }
                for sub in &fig.subfigures {
                    if let Some(c) = &sub.caption {
                        walk_inlines(c, f);
                    }
                }
            }
            Block::Theorem(t) => {
                if let Some(title) = &t.title {
                    walk_inlines(title, f);
                }
                for_each_inline(&t.body, f);
            }
            Block::Quote(inner) => for_each_inline(inner, f),
            Block::Math(_) | Block::Verbatim(_) => {}
        }
    }
}
