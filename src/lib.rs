//! Shared Ptyktos document parsing/conversion APIs. Parsers return structured data first;
//! product-specific presentation and import policy belong to their callers.

pub mod arxiv_source;
pub mod bibtex;
pub mod document;
pub mod epub;
pub mod graph;
pub mod html;
pub mod latex;
pub mod markdown;
pub mod mathml;
pub mod pdf;
pub mod rtf;
pub mod tex_text;
pub mod xlsx;

/// Optional native composition-to-frame/video renderer. Enable `framefold` to use it.
#[cfg(feature = "framefold")]
pub use framefold as frame_renderer;

/// Optional Transcode image/audio/video codec APIs (`media-codecs` feature).
#[cfg(feature = "media-codecs")]
pub use transcode as media_codecs;

/// A parsed worksheet. `index` is zero-based in workbook order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worksheet {
    pub index: usize,
    /// Current Webfold-compatible display name (`Sheet N`). The workbook XML's human name
    /// resolution is a follow-up; callers can still select sheets deterministically by index.
    pub name: String,
    pub rows: Vec<Vec<String>>,
}
