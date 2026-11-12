//! Python and WebAssembly bindings for superdocuments.
//!
//! One small API, written once in [`api`] in plain Rust, with two thin front ends: a Python module
//! (PyO3, feature `python`) and a WebAssembly module (wasm-bindgen, feature `wasm`, which gives
//! TypeScript declarations). It takes bytes and returns strings, never superdocuments' own types.
//!
//! | Function | What it does |
//! |---|---|
//! | `convert(data, extension)` | a document's bytes to Markdown, chosen by extension (`docx`, `xlsx`, `pptx`, `epub`, `rtf`, `csv`, `html`, `pdf`, ...) |
//! | `pdf_to_markdown(data)` | a PDF's bytes to Markdown, in process (no OCR) |
//! | `pdf_pages(data)` | the same, one string per page |
//! | `worksheets(data)` | an XLSX/XLSM workbook as JSON text: `[{"index", "name", "rows"}]` |
//! | `extensions()` | the extensions `convert` accepts |
//! | `version()` | the crate version |
//!
//! Not exposed: anything that touches the filesystem (`convert_path`, `\input` resolution for LaTeX
//! projects), and the OCR path, which runs external `pdftoppm` and `tesseract` processes.

#![warn(missing_docs)]

pub mod api;

#[cfg(feature = "python")]
mod python;

#[cfg(feature = "wasm")]
mod wasm;
