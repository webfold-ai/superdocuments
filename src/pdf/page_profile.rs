//! Page-level OCR routing.
//!
//! A PDF page needs OCR when its native text cannot be the page's content:
//!
//! * [`OcrReason::NoText`]: the page has no alphanumeric native text at all (an image-only page or
//!   a blank page), or
//! * [`OcrReason::ScannedImage`]: images cover at least [`SCANNED_MIN_IMAGE_COVERAGE`] of the page
//!   and the native text is sparse for the page's size, fewer than
//!   [`SCANNED_MAX_CHARS_PER_1000_PT2`] alphanumeric characters per 1000 square points (about 200
//!   on A4). That is what a scan carrying a one-line stamp, header or page number looks like: the
//!   stamp is real text but it is not the page.
//!
//! A page with a crest or logo (small image coverage) or with a full publisher text layer is never
//! flagged, however many images it holds. The rule needs no OCR engine; it reads the page geometry,
//! the images painted by the page's own content stream and the native text extraction.

use super::parser::PageGeometry;
use std::io;

/// Images must cover at least this share of the page area for the page to count as a scan.
pub const SCANNED_MIN_IMAGE_COVERAGE: f64 = 0.5;
/// A scanned page's native text is "sparse" below this many alphanumeric characters per 1000 pt².
pub const SCANNED_MAX_CHARS_PER_1000_PT2: f64 = 0.4;

/// Why a page was routed to OCR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrReason {
    /// No alphanumeric native text on the page.
    NoText,
    /// Image-dominated page whose native text (a stamp, header or caption) is sparse for its size.
    ScannedImage,
}

/// The routing facts and verdict for one page (numbered from 1, in extraction order).
#[derive(Clone, Debug, PartialEq)]
pub struct PageOcrVerdict {
    pub page: u32,
    /// Alphanumeric characters of native text, not counting image placeholders.
    pub text_chars: usize,
    /// Share of the page area covered by images (0.0..=1.0).
    pub image_coverage: f64,
    /// Native alphanumeric characters per 1000 pt² of page area.
    pub text_density: f64,
    pub reason: Option<OcrReason>,
}

/// Alphanumeric characters of a page's native Markdown, ignoring image placeholder lines.
pub(crate) fn native_text_chars(markdown: &str) -> usize {
    markdown
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("![]("))
        .map(|l| l.chars().filter(|c| c.is_alphanumeric()).count())
        .sum()
}

/// The page rule on its own inputs.
pub fn classify_page(text_chars: usize, geometry: &PageGeometry) -> Option<OcrReason> {
    if text_chars == 0 {
        return Some(OcrReason::NoText);
    }
    let density = text_density(text_chars, geometry);
    (geometry.image_coverage >= SCANNED_MIN_IMAGE_COVERAGE
        && density < SCANNED_MAX_CHARS_PER_1000_PT2)
        .then_some(OcrReason::ScannedImage)
}

fn text_density(text_chars: usize, geometry: &PageGeometry) -> f64 {
    text_chars as f64 / (geometry.width * geometry.height / 1000.0)
}

/// Verdicts for every page, from the pages' native Markdown and geometry (same order).
pub(crate) fn verdicts(geometry: &[PageGeometry], page_texts: &[String]) -> Vec<PageOcrVerdict> {
    page_texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let text_chars = native_text_chars(text);
            // A page whose geometry is missing has no image evidence: only NoText applies.
            let g = geometry.get(i).copied().unwrap_or(PageGeometry {
                width: 612.0,
                height: 792.0,
                image_coverage: 0.0,
            });
            PageOcrVerdict {
                page: i as u32 + 1,
                text_chars,
                image_coverage: g.image_coverage,
                text_density: text_density(text_chars, &g),
                reason: classify_page(text_chars, &g),
            }
        })
        .collect()
}

/// Per-page OCR verdicts of a PDF held in memory (native parse only; no OCR is run).
pub fn page_ocr_verdicts(bytes: &[u8]) -> io::Result<Vec<PageOcrVerdict>> {
    let (pages, geometry) = super::pages_and_geometry(bytes)?;
    Ok(verdicts(&geometry, &pages))
}

/// The 1-based numbers of the pages the page rule routes to OCR.
pub fn pages_needing_ocr(bytes: &[u8]) -> io::Result<Vec<u32>> {
    Ok(page_ocr_verdicts(bytes)?
        .into_iter()
        .filter(|v| v.reason.is_some())
        .map(|v| v.page)
        .collect())
}
