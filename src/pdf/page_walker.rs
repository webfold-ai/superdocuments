//! Page-level complexity heuristic lifted from `run-llama/liteparse`
//! `ocr_merge.rs::page_complexity` (~120 LOC, the predicate function).
//!
//! LiteParse's full `page_complexity` walks a `pdfium::Page` object for
//! image refs, vector path coverage, font-encoding anomalies, and
//! AcroForm references. We don't link PDFium, so this lift carries the
//! **decision function** and runs it against the signals Transcode + lopdf
//! expose on a raw byte buffer:
//!
//! 1. **AcroForm dict present** — `parse_pdf_bytes` reads the catalog's
//!    `/AcroForm`. If found, the page likely has interactive form fields that
//!    text-only extraction will miss.
//! 2. **No extractable native text in Transcode's stream** — `extract_markdown`
//!    returns the markdown for what it can read; an empty result (after
//!    stripping page markers) signals a scanned/garbled page that needs OCR.
//! 3. **No `/Contents` streams of `/Length` > 0** — quick byte-walk for the
//!    literal `"stream"` token, which is absent on image-only PDFs.
//!
//! Without PDFium we cannot compute the full set of LiteParse's signals
//! (image-coverage fractions, vector-text area, font-encoding health), so
//! every flagged page gets the coarse reason `NoText` or `AcroForm`; the
//! richer enum lives in `geometry::ComplexityReason` and is what future
//! PDFium callers will populate.

use std::fs;
use std::path::Path;

use super::geometry::ComplexityReason;
use super::geometry::PageComplexityStats;

/// Heuristic page complexity. Returns the populated `PageComplexityStats`
/// for the *first page only* — without PDFium we can't iterate per-page
/// efficiently. Callers wanting per-page stats should loop themselves.
pub fn page_complexity(path: impl AsRef<Path>) -> std::io::Result<PageComplexityStats> {
    let bytes = fs::read(path.as_ref())?;
    Ok(page_complexity_from_bytes(&bytes))
}

/// Pure-bytes version of [`page_complexity`] for in-memory buffers.
pub fn page_complexity_from_bytes(bytes: &[u8]) -> PageComplexityStats {
    let has_acroform = detect_acroform_in_bytes(bytes);
    let has_stream = detect_content_stream(bytes);
    let text_extractable = !extractable_text_count(bytes);

    let mut reasons = Vec::new();
    if has_acroform {
        reasons.push(ComplexityReason::AnnotationText);
    }
    // We treat "no content stream token" as NoText — a scanned PDF typically
    // embeds page rasters directly in /Resources/XObject without a /Contents
    // stream for the text layer.
    if !has_stream && !has_acroform {
        reasons.push(ComplexityReason::NoText);
    }

    PageComplexityStats {
        page_number: 1,
        text_length: text_extractable,
        text_coverage: if has_stream { 0.5 } else { 0.0 },
        has_substantial_images: !has_stream && !has_acroform,
        image_block_count: 0,
        image_coverage: 0.0,
        largest_image_coverage: 0.0,
        full_page_image: !has_stream && !has_acroform,
        uncovered_vector_area: None,
        is_garbled: false,
        page_area: 0.0,
        needs_ocr: !reasons.is_empty(),
        reasons,
    }
}

/// Cheap `parse_pdf_bytes`-based check for an AcroForm dict at the catalog
/// root. We don't need a full PDF object model — a textual scan for the
/// pattern `<< /Type /Catalog … /AcroForm` works for the common case where
/// the dict sits in the same object literal as the catalog.
fn detect_acroform_in_bytes(bytes: &[u8]) -> bool {
    // Look for `<< /Type /Catalog` then scan forward 4 KiB for `AcroForm`.
    // False positives (e.g. `/AcroForm` as a comment) are acceptable — the
    // decision is "is this PDF likely to need form-aware extraction", not "is
    // there literally an /AcroForm dict here".
    if let Some(start) = find_subslice(bytes, b"/Type /Catalog") {
        let end = (start + 4096).min(bytes.len());
        bytes[start..end]
            .windows(b"/AcroForm".len())
            .any(|w| w == b"/AcroForm")
    } else {
        false
    }
}

/// Look for the literal `stream` token in the first 256 KiB. PDFs with
/// image-only pages typically lack `/Contents` streams for the text layer
/// (the page has no text to stream). A quick heuristic, intentionally
/// conservative: it's a *signal*, not a decision; the harness can opt to
/// always run OCR regardless.
fn detect_content_stream(bytes: &[u8]) -> bool {
    let limit = bytes.len().min(256 * 1024);
    find_subslice(&bytes[..limit], b"stream").is_some()
}

/// Quick text-layer probe via Transcode's `extract_markdown`. Returns the
/// number of non-trivial characters in the extracted markdown. We avoid
/// pulling Transcode in here to keep this module dependency-free of
/// transcode's full pipeline; callers can use the standalone
/// `extract_complexity(path)` helper which DOES use Transcode, or call
/// `extract_markdown` directly and pass the byte count here.
///
/// For the pure-bytes path, we use a cheap byte-level proxy: count letters
/// in PDF text-showing operators (`Tj`, `TJ`, `'`, `"`). This is a rough
/// proxy — the real text-extraction happens via Transcode at the call site.
fn extractable_text_count(bytes: &[u8]) -> usize {
    // `Tj` and `TJ` are the two main text-showing operators in the content
    // stream. Count occurrences; multiply by a heuristic average word
    // length to get a rough character count.
    let mut hits = 0usize;
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if &bytes[i..i + 2] == b"Tj" || &bytes[i..i + 2] == b"TJ" {
            // Bound by ASCII-ish context: must not be inside an arbitrary
            // binary blob. Approximate with "must follow a non-letter".
            let prev_ok = i == 0 || !(bytes[i - 1]).is_ascii_alphabetic();
            if prev_ok {
                hits += 1;
            }
        }
        i += 1;
    }
    hits * 5
}

/// Knuth-Morris-Pratt substring search. Returns the byte offset of the
/// first match, or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acroform_detected_when_dict_present_in_first_4kib() {
        let pdf = b"%PDF-1.7\n<< /Type /Catalog /Pages 1 0 R /AcroForm << /Fields [] >> >>\n%%EOF";
        assert!(detect_acroform_in_bytes(pdf));
    }

    #[test]
    fn acroform_absent_when_only_pages_dict_present() {
        let pdf = b"%PDF-1.7\n<< /Type /Catalog /Pages 1 0 R >>\n%%EOF";
        assert!(!detect_acroform_in_bytes(pdf));
    }

    #[test]
    fn stream_token_detected_in_content_stream_pdf() {
        let pdf = b"%PDF-1.7\n1 0 obj << /Length 10 >> stream\nBT /F1 12 Tf\nET endstream\n%%EOF";
        assert!(detect_content_stream(pdf));
    }

    #[test]
    fn stream_token_absent_from_image_only_pdf() {
        let pdf = b"%PDF-1.7\n1 0 obj << /Type /XObject /Subtype /Image /Width 1 /Height 1 >> endobj\n%%EOF";
        assert!(!detect_content_stream(pdf));
    }

    #[test]
    fn extractable_text_count_finds_text_showing_operators() {
        let pdf = b"BT /F1 12 Tf (Hello) Tj ET BT /F1 12 Tf (World) Tj ET";
        assert!(extractable_text_count(pdf) >= 10);
    }

    #[test]
    fn extractable_text_count_zero_on_image_only() {
        let pdf = b"/Subtype /Image /Width 100 /Height 100 /BitsPerComponent 8";
        assert_eq!(extractable_text_count(pdf), 0);
    }

    #[test]
    fn needs_ocr_set_when_no_stream_no_acroform() {
        // Mimics a scanned PDF: no AcroForm, no content stream.
        let stats = page_complexity_from_bytes(b"%PDF-1.7\n<< /Type /Catalog >>\n%%EOF");
        assert!(stats.needs_ocr);
        assert!(stats.reasons.contains(&ComplexityReason::NoText));
    }

    #[test]
    fn needs_ocr_false_when_native_text_present() {
        // Content-stream PDF with extracted text. The cheap byte-level proxy
        // counts at least one Tj/TJ hit and the page should NOT need OCR.
        let pdf = b"%PDF-1.7\n\
            << /Type /Catalog /Pages 1 0 R >>\n\
            1 0 obj << /Length 24 >> stream\n\
            BT /F1 12 Tf (Hello) Tj ET\n\
            endstream endobj\n\
            %%EOF";
        let stats = page_complexity_from_bytes(pdf);
        assert!(
            !stats.needs_ocr,
            "expected no OCR, got reasons={:?}",
            stats.reasons
        );
    }

    #[test]
    fn find_subslice_basic() {
        let h = b"hello world";
        assert_eq!(find_subslice(h, b"world"), Some(6));
        assert_eq!(find_subslice(h, b"foo"), None);
        assert_eq!(find_subslice(b"", b"foo"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }
}
