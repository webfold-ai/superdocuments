//! Rich Text Format (RTF) reading and writing.
//!
//! `parse` turns RTF bytes into an `RtfDocument`: metadata, blocks (paragraphs, list items and
//! tables), hyperlinks, footnotes, headers and footers, and warnings. `to_text` and `to_markdown`
//! render that document. `to_rtf` and `markdown_to_rtf` write RTF back out, as ASCII.
//!
//! The reader never panics. It returns an error only for a bad header or an exceeded limit
//! (`Limits`). Other damage, such as unbalanced groups, unknown code pages or truncated escapes,
//! is reported in `RtfDocument::warnings`.

mod charset;
mod lexer;
mod markdown_in;
mod model;
mod reader;
mod render;
mod writer;

pub use charset::{CodePage, Kind as CodePageKind};
pub use lexer::{Lexed, Lexer, Token};
pub use model::{
    Alignment, Block, Cell, Footnote, Hyperlink, Limits, ListItem, Metadata, Options, Paragraph,
    Part, PartKind, Row, RtfDocument, RtfError, Run, Style, Table, TableStyle, Warning,
};

/// Parses RTF with default options.
pub fn parse(data: &[u8]) -> Result<RtfDocument, RtfError> {
    parse_with(data, &Options::default())
}

/// Parses RTF with the given options.
pub fn parse_with(data: &[u8], options: &Options) -> Result<RtfDocument, RtfError> {
    reader::read(data, options)
}

/// Renders a document as plain text.
pub fn to_text(doc: &RtfDocument, options: &Options) -> Result<String, RtfError> {
    render::text(doc, options)
}

/// Renders a document as Markdown.
pub fn to_markdown(doc: &RtfDocument, options: &Options) -> Result<String, RtfError> {
    render::markdown(doc, options)
}

/// Writes a document as ASCII-only RTF.
pub fn to_rtf(doc: &RtfDocument) -> Vec<u8> {
    writer::write(doc)
}

/// Converts the Markdown subset that `to_markdown` produces into RTF.
pub fn markdown_to_rtf(markdown: &str) -> Vec<u8> {
    writer::write(&markdown_in::parse(markdown))
}

/// Converts RTF bytes to Markdown with default options. This is what `convert_bytes("rtf", ..)`
/// calls.
pub fn convert(data: &[u8]) -> Result<String, String> {
    let options = Options::default();
    let doc = parse_with(data, &options).map_err(|e| e.to_string())?;
    to_markdown(&doc, &options).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
