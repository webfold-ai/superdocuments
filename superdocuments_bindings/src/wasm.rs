//! The WebAssembly module (wasm-bindgen), which also produces the TypeScript declarations.
//!
//! On `wasm32-unknown-unknown` a panic aborts (there is no unwinding), so the `catch_unwind` in the
//! PDF parser cannot turn a parser panic into an error: it traps, and JavaScript sees a
//! `WebAssembly.RuntimeError`. Treat that as a failed call, and load a fresh module instance after it.

use wasm_bindgen::prelude::*;

use crate::api;

fn err(e: api::ApiError) -> JsError {
    JsError::new(&e.to_string())
}

/// A document's bytes to Markdown, chosen by extension.
#[wasm_bindgen]
pub fn convert(data: &[u8], extension: &str) -> Result<String, JsError> {
    api::convert(data, extension).map_err(err)
}

/// A PDF's bytes to Markdown, in process (no OCR).
#[wasm_bindgen(js_name = pdfToMarkdown)]
pub fn pdf_to_markdown(data: &[u8]) -> Result<String, JsError> {
    api::pdf_to_markdown(data).map_err(err)
}

/// A PDF's Markdown, one string per page.
#[wasm_bindgen(js_name = pdfPages)]
pub fn pdf_pages(data: &[u8]) -> Result<Vec<String>, JsError> {
    api::pdf_pages(data).map_err(err)
}

/// An XLSX/XLSM workbook as JSON text: `[{"index", "name", "rows"}]`.
#[wasm_bindgen]
pub fn worksheets(data: &[u8]) -> Result<String, JsError> {
    api::worksheets(data).map_err(err)
}

/// The extensions `convert` accepts.
#[wasm_bindgen]
#[must_use]
pub fn extensions() -> Vec<String> {
    api::EXTENSIONS.iter().map(|s| (*s).to_owned()).collect()
}

/// The version of the bindings.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    api::version().to_owned()
}
