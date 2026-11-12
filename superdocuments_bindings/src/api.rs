//! The API both front ends wrap.

use std::fmt;

use superdocuments::{document, pdf, xlsx};

/// Why a call failed. The text names what was rejected and is meant for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError(String);

impl ApiError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ApiError {}

/// The extensions [`convert`] accepts, in the order `document::convert_bytes` matches them.
/// `pdf` is handled here, by the in-process parser; the rest are `document::convert_bytes`'s.
pub const EXTENSIONS: [&str; 30] = [
    "docx", "docm", "pptx", "xlsx", "xlsm", "odt", "odp", "ods", "xls", "xlsb", "epub", "rtf",
    "csv", "html", "htm", "xhtml", "txt", "md", "markdown", "tex", "latex", "ltx", "bib", "bibtex",
    "mml", "mathml", "gz", "tgz", "tar", "pdf",
];

/// The crate version.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Normalise `.DOCX`, ` docx ` and `docx` to `docx`.
fn extension(raw: &str) -> String {
    raw.trim().trim_start_matches('.').to_ascii_lowercase()
}

/// Convert a document's bytes to Markdown, choosing the converter by `extension`.
///
/// # Errors
/// An unsupported extension, or bytes that are not a valid document of that kind.
pub fn convert(data: &[u8], ext: &str) -> Result<String, ApiError> {
    let ext = extension(ext);
    if ext == "pdf" {
        return pdf_to_markdown(data);
    }
    document::convert_bytes(&ext, data)
        .map_err(|e| ApiError::new(format!("cannot convert as {ext}: {e}")))
}

/// Convert a PDF's bytes to Markdown with the in-process parser. Pages with no selectable text
/// (scans) produce nothing: OCR is not part of this API.
///
/// # Errors
/// Bytes that are not a readable PDF, or one over the size limit.
pub fn pdf_to_markdown(data: &[u8]) -> Result<String, ApiError> {
    pdf::extract_markdown(data).map_err(|e| ApiError::new(format!("cannot convert as pdf: {e}")))
}

/// A PDF's Markdown, one string per page (empty for a page with no selectable text).
///
/// # Errors
/// As [`pdf_to_markdown`].
pub fn pdf_pages(data: &[u8]) -> Result<Vec<String>, ApiError> {
    pdf::extract_pages_markdown(data)
        .map_err(|e| ApiError::new(format!("cannot convert as pdf: {e}")))
}

/// An XLSX or XLSM workbook as JSON text: `[{"index": 0, "name": "Sheet1", "rows": [["a", "b"]]}]`.
///
/// # Errors
/// Bytes that are not a readable workbook.
pub fn worksheets(data: &[u8]) -> Result<String, ApiError> {
    let sheets = xlsx::read_workbook(data)
        .map_err(|e| ApiError::new(format!("cannot read as xlsx: {e}")))?;
    let json: Vec<serde_json::Value> = sheets
        .into_iter()
        .map(|s| serde_json::json!({ "index": s.index, "name": s.name, "rows": s.rows }))
        .collect();
    serde_json::to_string(&json).map_err(|e| ApiError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_text_convert() {
        let md = convert(b"a,b\n1,2\n", "CSV").expect("csv");
        assert!(md.contains('|') && md.contains('a'), "{md}");
        assert_eq!(convert(b"# hi", ".md").expect("md"), "# hi");
    }

    #[test]
    fn every_listed_extension_is_known_to_the_converter() {
        // Garbage bytes: a format-specific error is fine, "unsupported" is not.
        for e in EXTENSIONS {
            if let Err(err) = convert(b"\x00\x01garbage", e) {
                assert!(
                    !err.to_string().contains("unsupported document extension"),
                    "{e}: {err}"
                );
            }
        }
    }

    #[test]
    fn an_unknown_extension_is_named() {
        let e = convert(b"x", "exe").expect_err("unsupported");
        assert!(
            e.to_string()
                .contains("unsupported document extension: exe"),
            "{e}"
        );
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        for e in [
            "docx", "xlsx", "pptx", "odt", "epub", "xls", "xlsb", "rtf", "pdf",
        ] {
            assert!(convert(b"", e).is_err(), "{e} on empty input");
            assert!(
                convert(b"PK\x03\x04truncated", e).is_err(),
                "{e} on a truncated zip"
            );
        }
        assert!(worksheets(b"not a zip").is_err());
        assert!(pdf_pages(b"%PDF-1.4 nope").is_err());
    }
}
