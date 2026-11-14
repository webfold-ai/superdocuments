//! The `superdocuments` Python module (PyO3).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::api;

fn err(e: api::ApiError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// A document's bytes to Markdown, chosen by extension.
#[pyfunction]
#[pyo3(signature = (data, extension))]
fn convert(py: Python<'_>, data: &[u8], extension: &str) -> PyResult<String> {
    // Conversion is CPU-bound and does no Python calls: let other Python threads run.
    py.detach(|| api::convert(data, extension)).map_err(err)
}

/// A PDF's bytes to Markdown, in process (no OCR).
#[pyfunction]
fn pdf_to_markdown(py: Python<'_>, data: &[u8]) -> PyResult<String> {
    py.detach(|| api::pdf_to_markdown(data)).map_err(err)
}

/// A PDF's Markdown, one string per page.
#[pyfunction]
fn pdf_pages(py: Python<'_>, data: &[u8]) -> PyResult<Vec<String>> {
    py.detach(|| api::pdf_pages(data)).map_err(err)
}

/// An XLSX/XLSM workbook as JSON text: `[{"index", "name", "rows"}]`.
#[pyfunction]
fn worksheets(py: Python<'_>, data: &[u8]) -> PyResult<String> {
    py.detach(|| api::worksheets(data)).map_err(err)
}

/// The extensions `convert` accepts.
#[pyfunction]
fn extensions() -> Vec<&'static str> {
    api::EXTENSIONS.to_vec()
}

/// The version of the bindings.
#[pyfunction]
fn version() -> &'static str {
    api::version()
}

#[pymodule]
#[pyo3(name = "superdocuments")]
fn superdocuments_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(convert, m)?)?;
    m.add_function(wrap_pyfunction!(pdf_to_markdown, m)?)?;
    m.add_function(wrap_pyfunction!(pdf_pages, m)?)?;
    m.add_function(wrap_pyfunction!(worksheets, m)?)?;
    m.add_function(wrap_pyfunction!(extensions, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
