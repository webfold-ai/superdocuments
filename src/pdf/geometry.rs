//! Geometry types lifted from `run-llama/liteparse` (Apache-2.0) — the
//! surface area for items 1, 2, 3, 5, 8, 9 of the LiteParse feature list.
//!
//! These are the **types** LiteParse's `extract.rs` + `ocr_merge.rs` +
//! `layout.rs` populate. They are shape-only here — every field is `Option`
//! or empty until a geometry-aware PDF backend (PDFium, a Transcode patch,
//! or a Rust-side geometry pass on the Op tree) is wired in. The harness
//! uses them to expose the same JSON schema as LiteParse so the
//! `superdocuments-bench/ocr` cross-stack comparison is apples-to-apples.
//!
//! Coordinate system: top-left origin, 72 DPI viewport space (matches
//! LiteParse's `text_items`, `layout_blocks`, and the public JSON output).
//! PDF user-space (bottom-left origin) coords need `y = page_height - y` to
//! convert.
//!
//! Each type's docstring notes the LiteParse source it was lifted from and
//! what the field would be populated from once a backend lands.

use serde::{Deserialize, Serialize};

/// One rectangle in viewport coordinates (top-left origin, 72 DPI).
/// Lifted from `run-llama/liteparse` `types.rs::Rect` (~36 KB module).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Smallest rect containing both inputs. Used by the block classifier to
    /// grow a block's bbox as source lines are appended, and to merge blocks
    /// that are spliced together after construction.
    pub fn union(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }

    /// Fold `rect` into an optional accumulator. `None` adopts the rect; `Some`
    /// unions. Blocks whose sources carry no geometry stay `None` rather than
    /// reporting a bogus rect anchored at the origin.
    pub fn extend(acc: &mut Option<Rect>, rect: &Rect) {
        match acc.as_mut() {
            Some(existing) => *existing = existing.union(*rect),
            None => *acc = Some(*rect),
        }
    }

    /// Area in pt². Returns 0.0 for negative-dimension rects (defensive — a
    /// well-formed page never produces those).
    pub fn area(self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }
}

/// One word's bounding box within a `TextItem`, in the same viewport space
/// (top-left origin, 72 DPI) as the parent item. `text` is the word's content
/// with inter-word spaces excluded.
///
/// Lifted from `run-llama/liteparse` `types.rs::WordBox`. Populated by
/// `extract.rs` when `ExtractRequest.output.include_word_boxes` is set; the
/// public JSON output (`output/json.rs`) builds its own view from this shape,
/// so we surface it byte-compatible.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WordBox {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// One text item extracted from a PDF page (viewport coordinates).
/// Lifted from `run-llama/liteparse` `types.rs::TextItem` (~80 lines of
/// fields including font metadata, link annotations, strikethrough detection,
/// and per-word sub-boxes).
///
/// `font_height` is the font size * scale_y from the text matrix — it
/// accounts for CTM scaling that raw `font_size` ignores.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Rotation in degrees (counter-clockwise, adjusted for page rotation).
    pub rotation: f32,
    pub font_name: Option<String>,
    pub font_size: Option<f32>,
    pub font_height: Option<f32>,
    pub font_ascent: Option<f32>,
    pub font_descent: Option<f32>,
    pub font_weight: Option<i32>,
    pub text_width: Option<f32>,
    pub font_is_buggy: bool,
    pub has_unicode_map_error: bool,
    pub mcid: Option<i32>,
    pub fill_color: Option<String>,
    pub stroke_color: Option<String>,
    pub char_codes: Vec<u32>,
    pub trailing_space_generated: bool,
    /// OCR confidence 0.0–1.0. `None` for native PDF text.
    pub confidence: Option<f32>,
    /// Target URI when this item falls inside a hyperlink annotation's rectangle.
    pub link: Option<String>,
    /// Whether a thin horizontal stroke crosses this item's vertical middle band
    /// (a strikethrough line). Consumed by the markdown emitter to wrap as `~~…~~`.
    pub strike: bool,
    /// Per-word sub-boxes within this item. Empty unless a backend populates it.
    pub words: Vec<WordBox>,
}

/// One entry in the document outline (bookmarks). Coordinates in PDF user
/// space — convert to viewport with `page_height - y` once you know the page.
///
/// Lifted from `run-llama/liteparse` `types.rs::OutlineTarget`. Populated by
/// `extract.rs::extract_outline()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineTarget {
    /// Hierarchy depth, 1-based.
    pub level: u8,
    pub title: String,
    /// Zero-based page index of the destination.
    pub page_index: i32,
    /// Y in PDF user space. `None` when the destination doesn't specify a Y.
    pub y_pdf: Option<f32>,
}

/// One structured block on a page. Lifted from `run-llama/liteparse`
/// `layout.rs::LayoutBlock`.
///
/// `kind` discriminates: one of `heading`, `paragraph`, `list_item`,
/// `code`, `table`, `merged_table`, `grid_fallback`, `rule`, `figure`.
/// Blocks appear in reading order, matching the order the markdown emitter
/// emits them, so the Nth block here is the Nth block of that page's markdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutBlock {
    pub kind: String,
    /// Rendered text for the text-bearing kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Heading level (1–6) or list nesting depth for `list_item`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Whether the block's text is uniformly bold / italic.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    /// `list_item`: ordered? + original marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    /// Verbatim source lines for `code` and `grid_fallback`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    /// Best-effort language hint for `code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// `table`: header row, when one was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<LayoutCell>>,
    /// `table`: body rows. Ragged — cells covered by a neighbour's span are absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<LayoutCell>>>,
    /// `merged_table`: how many leading rows are header rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_rows: Option<usize>,
    /// `figure`: image id + format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Region of the page this block occupies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
}

/// One table cell. Lifted from `run-llama/liteparse` `layout.rs::LayoutCell`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutCell {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
    /// Merge spans. `None`/1 means the cell is single.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colspan: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rowspan: Option<u16>,
}

/// Why a page was flagged as needing more than the cheap text-only path.
/// Multiple reasons can apply to one page. Empty exactly when `needs_ocr`
/// is false.
///
/// Lifted from `run-llama/liteparse` `ocr_merge.rs::ComplexityReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComplexityReason {
    /// A single raster covers essentially the whole page and there is little
    /// or no extractable text behind it — a scanned/photographed page.
    Scanned,
    /// Almost no extractable native text, and no full-page raster behind it
    /// (a blank page, or a near-empty cover/divider).
    NoText,
    /// Some real text, but it covers very little of the page — typically a
    /// figure-heavy page with only thin captions.
    SparseText,
    /// Substantial embedded raster figures sit alongside the native text.
    EmbeddedImages,
    /// The native text decodes to garbage (broken cmap / Type3 char-code
    /// fallback), so the visible glyphs and the extracted text disagree.
    Garbled,
    /// Text is painted as filled vector outlines, outside the text layer.
    VectorText,
    /// Text painted through an annotation's appearance stream.
    AnnotationText,
}

impl ComplexityReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scanned => "scanned",
            Self::NoText => "no-text",
            Self::SparseText => "sparse-text",
            Self::EmbeddedImages => "embedded-images",
            Self::Garbled => "garbled",
            Self::VectorText => "vector-text",
            Self::AnnotationText => "annotation-text",
        }
    }
}

/// Per-page complexity signals. Lifted from `run-llama/liteparse`
/// `ocr_merge.rs::PageComplexityStats`.
///
/// `needs_ocr` is the public predicate; the harness uses it to decide whether
/// the `tesseract` fallback fires for a given page. Empty `reasons` exactly
/// when `needs_ocr` is false.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageComplexityStats {
    pub page_number: usize,
    pub text_length: usize,
    /// Fraction of the page covered by extracted text.
    pub text_coverage: f32,
    pub has_substantial_images: bool,
    pub image_block_count: usize,
    pub image_coverage: f32,
    pub largest_image_coverage: f32,
    pub full_page_image: bool,
    /// Filled vector-outline area not covered by native text (pt²). `None` when
    /// a cheaper predicate already flagged the page, so this expensive walk
    /// was skipped.
    pub uncovered_vector_area: Option<f32>,
    pub is_garbled: bool,
    pub page_area: f32,
    /// Whether the page needs more than the cheap text-only path. Equivalent to
    /// `!reasons.is_empty()`.
    pub needs_ocr: bool,
    pub reasons: Vec<ComplexityReason>,
}

/// Document-level provenance metadata. Lifted from `run-llama/liteparse`
/// `types.rs::DocumentMetadata` and `document_metadata.rs` (the latter is
/// the extractor).
///
/// All fields stay `Option` so malformed metadata never prevents the
/// document itself from being parsed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mod_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_encrypted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_handler_revision: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eof_section_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startxref_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailer_id_pair_reaches_eof: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_file_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmp_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_byte_range_reaches_eof: Option<bool>,
}

/// Output-format selector for `extract_with_format`. Lifted from
/// `run-llama/liteparse` `config.rs::OutputFormat`. The harness now exposes
/// all three so the LiteParse subprocess target and the superdocuments
/// target can be compared on the same document at the same time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    #[default]
    Markdown,
    Json,
    Text,
}

impl OutputFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Text => "text",
        }
    }
}

/// Per-page parsed result, ready for JSON serialization. Lifted from
/// `run-llama/liteparse` `types.rs::ParsedPage`. All geometry fields are
/// `Option` because Transcode (the superdocuments backend) doesn't surface
/// them; only `text` and `markdown` are populated today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedPage {
    pub page_number: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_label: Option<String>,
    pub page_width: f32,
    pub page_height: f32,
    /// Union bbox of the page's top-level content objects (viewport coords).
    /// `None` for empty pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_bounds: Option<Rect>,
    /// Plain text content (whitespace-normalized, layout-aware).
    pub text: String,
    /// Markdown rendering. Empty unless `OutputFormat::Markdown` is selected.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    /// Per-item geometry + font metadata. Empty unless a geometry backend
    /// populates it (PDFium, or a Transcode patch exposing the Op tree).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_items: Vec<TextItem>,
    /// Per-block layout decomposition. Empty unless the layout classifier
    /// runs (LiteParse's `projection.rs` — not lifted yet).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<LayoutBlock>,
    /// Per-page complexity signals. `None` until a backend populates it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<PageComplexityStats>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_union_contains_both() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let b = Rect {
            x: 5.0,
            y: 5.0,
            width: 10.0,
            height: 10.0,
        };
        let u = a.union(b);
        assert_eq!(
            u,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 15.0,
                height: 15.0
            }
        );
    }

    #[test]
    fn rect_extend_handles_none() {
        let mut acc: Option<Rect> = None;
        Rect::extend(
            &mut acc,
            &Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
            },
        );
        assert_eq!(
            acc,
            Some(Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0
            })
        );
        Rect::extend(
            &mut acc,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        );
        assert_eq!(
            acc,
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0
            })
        );
    }

    #[test]
    fn output_format_default_is_markdown() {
        assert_eq!(OutputFormat::default(), OutputFormat::Markdown);
        assert_eq!(OutputFormat::Json.as_str(), "json");
    }

    #[test]
    fn complexity_reason_serializes_kebab_case() {
        let r = ComplexityReason::SparseText;
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, "\"sparse-text\"");
    }

    #[test]
    fn document_metadata_serializes_with_skip() {
        let mut m = DocumentMetadata::default();
        m.creation_date = Some("D:20240101".into());
        m.raw_file_size = Some(1024);
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("creation_date"));
        assert!(s.contains("raw_file_size"));
        assert!(!s.contains("file_version"));
    }
}

#[cfg(test)]
mod lifted_tests {
    use super::*;

    #[test]
    fn complexity_reason_serializes_kebab_case() {
        let r = ComplexityReason::SparseText;
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, "\"sparse-text\"");
    }

    #[test]
    fn document_metadata_round_trips_with_only_some_fields() {
        let mut m = DocumentMetadata::default();
        m.creation_date = Some("D:20240101".into());
        m.raw_file_size = Some(2048);
        let s = serde_json::to_string(&m).unwrap();
        let back: DocumentMetadata = serde_json::from_str(&s).unwrap();
        assert_eq!(back.creation_date.as_deref(), Some("D:20240101"));
        assert_eq!(back.raw_file_size, Some(2048));
        assert_eq!(back.file_version, None);
    }

    #[test]
    fn output_format_round_trips_all_variants() {
        for f in [
            OutputFormat::Markdown,
            OutputFormat::Json,
            OutputFormat::Text,
        ] {
            let s = serde_json::to_string(&f).unwrap();
            let back: OutputFormat = serde_json::from_str(&s).unwrap();
            assert_eq!(back, f);
        }
    }

    #[test]
    fn parsed_page_default_serializes_with_empty_optionals() {
        let p = ParsedPage {
            page_number: 1,
            page_label: None,
            page_width: 612.0,
            page_height: 792.0,
            content_bounds: None,
            text: String::new(),
            markdown: String::new(),
            text_items: Vec::new(),
            blocks: Vec::new(),
            complexity: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"page_number\":1"));
        assert!(s.contains("\"page_width\":612"));
        // None / empty fields skipped.
        assert!(!s.contains("page_label"));
        assert!(!s.contains("content_bounds"));
        assert!(!s.contains("text_items"));
        assert!(!s.contains("blocks"));
    }

    #[test]
    fn word_box_serializes_with_text_and_xy() {
        let w = WordBox {
            text: "invoice".into(),
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 12.0,
        };
        let s = serde_json::to_string(&w).unwrap();
        assert!(s.contains("\"text\":\"invoice\""));
        assert!(s.contains("\"x\":10"));
        assert!(s.contains("\"y\":20"));
    }

    #[test]
    fn outline_target_serializes_with_all_fields() {
        let o = OutlineTarget {
            level: 1,
            title: "Chapter 1".into(),
            page_index: 0,
            y_pdf: Some(720.0),
        };
        let s = serde_json::to_string(&o).unwrap();
        assert!(s.contains("\"level\":1"));
        assert!(s.contains("\"title\":\"Chapter 1\""));
        assert!(s.contains("\"page_index\":0"));
        assert!(s.contains("\"y_pdf\":720"));
    }

    #[test]
    fn layout_block_table_kind_serializes_cells() {
        let block = LayoutBlock {
            kind: "table".into(),
            text: None,
            level: None,
            bold: false,
            italic: false,
            ordered: None,
            marker: None,
            lines: None,
            lang: None,
            header: Some(vec![LayoutCell {
                text: "H1".into(),
                bbox: Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 50.0,
                    height: 10.0,
                }),
                colspan: None,
                rowspan: None,
            }]),
            rows: Some(vec![vec![LayoutCell {
                text: "cell".into(),
                bbox: None,
                colspan: None,
                rowspan: None,
            }]]),
            header_rows: None,
            id: None,
            format: None,
            bbox: None,
        };
        let s = serde_json::to_string(&block).unwrap();
        assert!(s.contains("\"kind\":\"table\""));
        assert!(s.contains("\"header\""));
        assert!(s.contains("\"H1\""));
        assert!(s.contains("\"cell\""));
    }
}
