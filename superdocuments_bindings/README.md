# superdocuments_bindings

Python and TypeScript (WebAssembly) bindings for [superdocuments](../README.md), through one small API.

| Function | What it does |
|---|---|
| `convert(data, extension)` | a document's bytes to Markdown, chosen by extension: `docx docm pptx xlsx xlsm odt odp ods xls xlsb epub rtf csv html htm xhtml txt md markdown tex latex ltx bib bibtex mml mathml gz tgz tar pdf` (`extensions()` lists them) |
| `pdf_to_markdown(data)` / `pdfToMarkdown` | a PDF's bytes to Markdown, in process |
| `pdf_pages(data)` / `pdfPages` | the same, one string per page |
| `worksheets(data)` | an XLSX/XLSM workbook as JSON text: `[{"index", "name", "rows"}]` |
| `extensions()` / `version()` | the accepted extensions / the version |

Bytes and an extension go in, strings come out; a failure raises `ValueError` (Python) or throws `Error`
(JavaScript) naming what was rejected. The extension may be written `docx`, `.DOCX` or ` docx `.

**Not exposed:** anything that touches the filesystem (`convert_path`, LaTeX `\input` resolution for a
project on disk), and the OCR path, which runs external `pdftoppm` and `tesseract` processes. A scanned PDF
therefore gives an image reference at most, never recognised text.

## Python

```python
import superdocuments
md = superdocuments.convert(open("report.docx", "rb").read(), "docx")
```

```bash
scripts/test-python.sh        # builds the extension, runs the tests (no maturin needed)
maturin build --release       # a wheel (abi3: one wheel for Python 3.9+)
```

## TypeScript / JavaScript

```bash
scripts/build-wasm.sh         # needs the wasm32-unknown-unknown target and a wasm-bindgen CLI matching Cargo.lock
node tests/wasm/test.mjs
```

All of superdocuments builds for `wasm32-unknown-unknown` with default features, including PDF, the in-repo
calamine (`.xls`) reader, zip and `encoding_rs`; nothing had to be left out. Two things differ from native:
on `wasm32-unknown-unknown` a panic aborts, so the PDF parser's `catch_unwind` cannot turn a parser panic
into an error (JavaScript sees a `WebAssembly.RuntimeError`; load a fresh instance after one), and the
optional `framefold`, `transcode` (images) features are not part of the bindings.

## What the tests check

Against the **built** extension and the **built** `.wasm`, on the repository's own fixtures
(`tests/fixtures`) plus an EPUB and CSV/HTML built in the test: DOCX, XLSX, PPTX, ODT, ODS, RTF, legacy XLS,
EPUB, CSV, HTML, native-text PDF, structured worksheets; and hostile input (empty, truncated archives,
random bytes, a 400 MB deflate zip bomb) which must be an error, quickly, with the module still working
afterwards.

Known library behaviour, pinned by the tests: a *valid* Office zip handed to a different OOXML or EPUB
converter (a `.docx` converted as `xlsx`) yields empty Markdown instead of an error. Formats with a header
check (`xls`, `pdf`, `rtf`, `odt`, `ods`) do raise.
