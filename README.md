# superdocuments

Convert documents to Markdown and extract content in Rust.

`document::convert_bytes` and `convert_path` turn DOCX, DOCM, PPTX, XLSX, XLSM, XLS, XLSB, ODT, ODP, ODS, EPUB, RTF, CSV, HTML and plain text or Markdown into Markdown, choosing the converter from the file extension. PDF is handled separately by the `pdf` module: in-process text extraction to Markdown, with an optional OCR fallback that calls external `pdftoppm` and `tesseract` binaries. Legacy binary `.xls` is read through the in-repo `calamine/` crate (BIFF, Excel 97-2003 and older).

> **Status: research build.** Version 0.9.0 (`Cargo.toml`). This code is not production-ready. It is being hardened toward a production release, and interfaces, formats and results may change without notice.

The repository is `webfold-ai/superdocuments`; the Cargo package and library crate are named `superdocuments`.

## Format matrix

Written from reading the source in `src/`; the test suite was not run when this table was prepared. "Not verified" means the code was not enough to say, and no test was run to find out. Details of each path are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

| Format | Markdown | Structured output | Images | OCR | Known limitations |
|---|---|---|---|---|---|
| DOCX, DOCM | `document::convert_bytes` / `convert_path` / `convert_streaming` | None public | `document::extract_images` returns the pictures (name, media type, size, first referencing part, alt). Markdown shows each one as `![alt](media/<name>)` where its drawing is | No | Headings from the document's styles (outline level, `heading N`, `Title`), bullet and numbered lists, bold and italic, hyperlinks with targets, tables (`gridSpan`, `vMerge`, nested tables flattened), footnotes and endnotes; tracked deletions omitted. Chart and diagram text is appended. Not read: headers, footers, comments, list start overrides, underline, colour. Tested with python-docx, LibreOffice and hand-built XML; no Word-written file |
| PPTX | `convert_bytes`, `convert_streaming` | None public | `extract_images` returns the pictures, with the slide that places them. Markdown shows each as `![alt](media/<name>)` in its slide | No | Slides in presentation order (`sldIdLst`), numbered by position, under `## Slide N: Title`. Body paragraphs, explicit bullets, hyperlinks, tables and speaker notes; footers, dates and slide numbers dropped. Not read: charts, SmartArt, comments, bullets that only the layout supplies. Tested with python-pptx and LibreOffice; no PowerPoint-written file |
| XLSX, XLSM | `convert_bytes`, `convert_streaming` | `xlsx::read_workbook`, `xlsx::read_sheet` return `Worksheet { index, name, rows }` | `extract_images` (pictures in drawings, then `xl/media`). Not in the Markdown | No | Sheets use the workbook's names in tab order (`Sheet N` only without a workbook part). Dates and times follow the cell's number format (1900 and 1904 epochs), booleans are `TRUE`/`FALSE`, inline and rich strings are read, uncalculated formulas show as `=formula`. Numbers are shown as their number format shows them: `General` and fixed decimals after Excel's rounding to 15 significant digits (`4.0999999999999996` shows `4.1`), thousands separators, percent, quoted/escaped literals, sections, and date and time codes (`mmm-yyyy` shows `Sep-2026`). Not implemented, and shown as stored (dates in ISO form): conditions, fractions, scientific notation, eras and the locale-dependent built-in formats (ids 14, 22, 27 to 36, 41 to 44, 50 to 58). `xlsx::read_workbook_typed` returns each cell's display text and typed value (`CellValue`: number with its stored text, date serial with its ISO form, ...). Not read: merged cells, hidden state, comments, charts. Tested with openpyxl and LibreOffice; no Excel-written file |
| XLSB | `convert_bytes` | None public | Not extracted | No | Cached scalar cells and shared strings only; formulas and styles are not reconstructed (comment in `document.rs`). Sheet names read from `xl/workbook.bin` |
| XLS (legacy) | `convert_bytes`, `convert_streaming` | None public | Not extracted | No | Same Markdown shape as XLSX: `## <sheet>` per worksheet in tab order, error cells as Excel error text (`#DIV/0!`), booleans, formula results (the stored value). Dates and times follow the cell's format as calamine classifies it (date-time or duration); the date and time parts are taken from the value, so a value under one day shows as a time and a whole number as a date. Charts and chart sheets are skipped, hidden sheets are kept. Tested with LibreOffice output (checked against its XLSX export) and calamine's BIFF fixtures; no Excel-written file |
| DOC, DOT (legacy Word) | `convert_bytes`, `convert_path` (`doc::convert`) | None public | Not extracted | No | The main story's text from the OLE compound file: Word 97-2003 piece table (8-bit and UTF-16 pieces), Word 6/95 by its text range, field results without field instructions, table cells as Markdown tables (a row ends at an empty cell mark, so a row whose last cells are empty can merge with the next). No styles, so no headings, lists or emphasis; headers, footers, footnotes, text boxes, comments and pictures are not read. Encrypted files and OLE files that are not Word documents give a named error. The content is sniffed: RTF and OOXML saved as `.doc` go to their readers. Tested on the Register's 21 `.doc` downloads (7 are OLE and use this reader, 14 are OOXML under a `.doc` name and go to the DOCX reader) against LibreOffice's text export |
| ODT, ODP | `convert_bytes` (same reader for both) | None public | `extract_images` (`Pictures/`, alt from `svg:desc`). Not in the Markdown | No | `text:p` and `text:h` only; every heading becomes `##`. No list, table or image handling in the code. ODP slides are not separated |
| ODS | `convert_bytes` | None public | `extract_images` (`Pictures/`). Not in the Markdown | No | One `## <table name>` section per sheet. Repeated empty columns are capped at 16,384. A cell that carries a typed value (`office:value`, `office:date-value`, ...) is read from the attribute alone; the display text in its `text:p` is ignored. Before this was fixed, such a cell came out twice (42 as 4242) |
| EPUB | `convert_bytes` (every `.xhtml`/`.html` entry, sorted by file name); `epub::extract_text` (spine order) | `epub::read` returns a `Book` (manifest, spine, title) | Image references as `![alt](src)` in the Markdown; `extract_images` returns each manifest image with the XHTML document that shows it | No | `convert_bytes` does not follow the spine. `epub::read` requires `META-INF/container.xml`. No CSS, fonts, scripting or DRM handling ([docs/EPUB.md](docs/EPUB.md)) |
| RTF | `convert_bytes`; `rtf::parse`, `rtf::to_text`, `rtf::to_markdown`, `rtf::to_rtf`, `rtf::markdown_to_rtf` | `rtf::RtfDocument`: metadata, blocks (paragraph, list item, table), hyperlinks, footnotes, optional headers and footers, warnings | Pictures (`\pict`, `\shppict`) are skipped; `\object` shows its `\result` | No | Bold, italic, strike, headings (outline level, style or large bold text), lists from `\listtext`, nested tables, `HYPERLINK` fields, footnotes, code pages from `\ansicpg`, `\fcharset` and `\cpg` (including Mac Roman, Shift_JIS, GBK, Big5, EUC-KR and UTF-8 escapes). Not supported: underline and colour in Markdown, font names, revision marks, fields other than `HYPERLINK`, right-to-left reordering, numbering without `\listtext` (shown as bullets), Symbol-font glyphs other than the bullet, and code pages `encoding_rs` lacks (reported as warnings). Bad headers and exceeded limits are errors; other damage is reported as warnings. See the limits note in `CHANGELOG.md` |
| LaTeX (`.tex`, fragments and documents) | Not routed by `convert_bytes`. Use `latex::parse` with `latex::render_markdown`, `render_text` or `render_html` | `latex::Document`: title, authors, abstract, sections with numbers, lists, tables, display and inline math as TeX, figures with captions, labels and image candidates, citations, `thebibliography` items, labels and warnings | Figures and subfigures are structured; `tikzpicture` is opaque. Math is kept as TeX and not rendered. Packages are not loaded; unknown commands keep their argument text with a warning. `\input` is read only through a `FileProvider`. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| BibTeX, BibLaTeX (`.bib`) | Not routed by `convert_bytes` | `bibtex::parse` returns a `Bibliography` with entries, `@string`, `@preamble`, `@comment` and warnings; `bibtex::write` round-trips; `bibtex::csl_json` exports CSL-JSON | Names, pages, dates, DOIs and URLs are parsed; `crossref` is followed up to 8 levels. Only the standard entry types are typed; others keep their fields. Bounds in `bibtex::Limits` |
| arXiv e-print (gzip tar or gzip TeX) | Not routed | `arxiv_source::read_eprint` takes bytes; returns the main document, bibliography and figures with their image bytes and detected formats | No network access. Absolute, `..` and backslash paths, links and CRC mismatches are errors. Non-UTF-8 source is decoded as Latin-1 with a warning |
| MathML | Not routed | `mathml::to_tex` (uses the TeX annotation when present) | Covers the common presentation elements; unknown elements keep their content with a warning |
| CSV | `convert_bytes`, `convert_streaming` | None public | n/a | No | First row becomes the header. Delimiter (`,` `;` or tab) is guessed from the first line. UTF-8 (BOM stripped) or UTF-16 with BOM only; other encodings are an error |
| HTML, HTM | `html::to_markdown`, `convert_bytes` | `html::parse` returns `HtmlDocument` (blocks with signature and links); `html::to_blocks`, `html::links` | Image references as `![alt](src)`; `data:` URIs and `<figure>` subtrees dropped | No | Input decoded as lossy UTF-8, no charset detection. `script`, `style`, `nav`, `header`, `footer`, `aside`, `form`, `head` and similar subtrees are dropped, so `<title>` is not emitted. `convert_bytes` passes an empty base URL |
| TXT, MD, MARKDOWN | `convert_bytes` returns the input as lossy UTF-8 | None | n/a | No | No processing |
| PDF | `pdf::extract_file`, `pdf::extract_markdown`, `pdf::extract_pages_markdown` | `pdf::extract_with_format` (JSON), `pdf::extract_images` (page, size, colour space, JPEG bytes unchanged, Flate images as PNG), `pdf::geometry_from_transcode`, `pdf::outline`, `graph::DocumentGraph` | Output contains `![](img_N.png)` markers, where `N` is the PDF object number of the image, not a running index. With the `images` feature, `Document::images()` decodes embedded images and `PdfImage::to_png` encodes them; the crate does not write files | Optional: `pdf::extract_file_with_ocr` and `_options`, `extract_bytes_with_ocr_options` (needs `pdftoppm` and `tesseract` on `PATH`). OCR is decided per page by `pdf::page_profile`: a page with no text, or whose images cover at least half of it while its text is under 0.4 alphanumeric characters per 1000 pt² (a scan with a stamp), is OCRed and keeps its stamp after the OCR text; other pages keep their native text. Images inside form XObjects and inline images do not count towards coverage | Parser filters: Flate, ASCII85, ASCIIHex, RunLength. No decryption code. 256 MiB input cap. `extract_with_format` page entries are placeholders (empty text, zero size) and its metadata is only the file size. Not verified: encrypted PDFs, reading order on multi-column pages. Page rasterisation is not implemented |

## Dependencies

Some dependencies are git repositories rather than crates.io packages. A clean build needs access to them; Cargo resolves optional dependencies into `Cargo.lock`, so `framefold` and `transcode` must be reachable even when their features are off.

| Dependency | Source | Pinned revision or version | Required or feature-gated | Purpose |
|---|---|---|---|---|
| `quick-xml` | crates.io | `0.42` | Required | XML parsing for OOXML, ODF and EPUB |
| `zip` | crates.io | `9` (`deflate` only) | Required | ZIP containers |
| `encoding_rs` | crates.io | `0.8` | Required | Code pages for RTF escapes (`src/rtf/charset.rs`) |
| `serde`, `serde_json` | crates.io | `1` | Required | Geometry types and JSON output in `pdf` and `graph` |
| `wirefold` | git, `webfold-ai/wirefold` | rev `ba9a71284844cd1a7f14ae3b7105ad972aaa9e07` | Required (`syntax` feature) | Syntax codec (YAML surface) for Markdown front matter |
| `apeiron` | git, `webfold-ai/apeiron` | rev `d0eda3226193542949306f767d7280404921546f` | Required | `apeiron::freq::Table` for the EPUB manifest index and the XLSX/XLSB path and relationship caches (zero-allocation `get_at(&str)`); `apeiron::map::hashtrinity::Table` is the hash map used by the cache, HTML extractor and PDF parser |
| `framefold` | git, `Ptyktos/framefold` | rev `5a097c2e2448640c79d43b37b9246d0f7baea0d4` | Feature `framefold` (and `framefold-js`) | Re-exported as `frame_renderer` |
| `transcode` | git, `Ptyktos/transcode` | rev `8e65c5afeb8adb9663511760b80e19156970207e` | Features `media-codecs`, `images` | Codec re-export as `media_codecs`; embedded PDF image decode and PNG encode |
| `calamine` (local) | `calamine/` in this repository | version 0.36.1 | Required | Path dependency with default features off, for the legacy `.xls` reader (`document::xls`); see below |

At run time, OCR also needs the external `pdftoppm` (Poppler) and `tesseract` programs. The legacy `pdf::to_markdown` adapter needs a `pdffold` executable (`PDFFOLD_BIN`, `PDFFOLD_MANIFEST` or `PATH`).

**Repository name versus package name.** A git dependency is named by repository URL, but Cargo uses the package name declared in the manifest there. The `apeiron` package is built from the `webfold-ai/apeiron` repository. The GitHub repository `webfold-ai/superdocuments` publishes the package `superdocuments`. The `calamine/` directory is part of this workspace: an in-house spreadsheet reader forked from tafia/calamine (`calamine/APEIRON-FORK.md`), not the crates.io release.

## Cargo features

All are off by default.

- `framefold`, `framefold-js`: the optional renderer, exposed as `superdocuments::frame_renderer`.
- `media-codecs`: re-exports the Transcode codecs as `superdocuments::media_codecs`.
- `images`: embedded-image extraction in the PDF parser.

## Known gaps

- PDF coverage is limited. Mixed born-digital and scanned pages, unusual PDF filters, and encrypted or object-stream variants need more fixtures and tests.
- HTML parity fixtures and the public API are still being settled.
- The test suite was not run in the environment that prepared the public-release changes, because the git dependencies are not reachable from there.
- CI (`.github/workflows/ci.yml`) builds and tests default features only, so the feature-gated code is not exercised there.

## Build and test

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

The minimum supported Rust version is 1.85 (`rust-version` in `Cargo.toml`). At the repository root these commands cover the `superdocuments` package; the `calamine/` workspace member is built only with `-p calamine` or `--workspace`.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): inputs and outputs, main path, modules, caching, limits, optional features.
- [docs/EPUB.md](docs/EPUB.md): the EPUB reader.
- [docs/FORMATS.md](docs/FORMATS.md): one section per format with what it reads, what comes out, the evidence behind each claim and the limits.
- `docs/SETUP.md`, `RUN.md`, `EVAL.md` and `REPORT.md` describe a separate benchmark suite, not this crate's API.

## Licence

The crate is licensed under the Apache License, Version 2.0 (`LICENSE`). The `calamine/` sub-crate is MIT (`calamine/Cargo.toml`), and its upstream authorship is retained there.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and [SECURITY.md](SECURITY.md).
