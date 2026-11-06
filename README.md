# superdocuments

Convert documents to Markdown and extract content in Rust.

`document::convert_bytes` and `convert_path` turn DOCX, DOCM, PPTX, XLSX, XLSM, XLSB, ODT, ODP, ODS, EPUB, RTF, CSV, HTML and plain text or Markdown into Markdown, choosing the converter from the file extension. PDF is handled separately by the `pdf` module: in-process text extraction to Markdown, with an optional OCR fallback that calls external `pdftoppm` and `tesseract` binaries. Legacy binary `.xls` is not converted (`convert_bytes` returns an error for it).

> **Status: research build.** Version 0.8.1 (`Cargo.toml`). This code is not production-ready. It is being hardened toward a production release, and interfaces, formats and results may change without notice.

The repository is `webfold-ai/superdocuments`; the Cargo package and library crate are named `superdocuments`.

## Format matrix

Written from reading the source in `src/`; the test suite was not run when this table was prepared. "Not verified" means the code was not enough to say, and no test was run to find out. Details of each path are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

| Format | Markdown | Structured output | Images | OCR | Known limitations |
|---|---|---|---|---|---|
| DOCX, DOCM | `document::convert_bytes` / `convert_path` / `convert_streaming` | None public | Not extracted. `convert_bytes` inlines drawing alt text (`descr`) | No | Headings only from style ids `Heading1`..`Heading6`. No list markers, hyperlinks, emphasis, footnotes or headers/footers. `convert_bytes` also appends chart text, diagram text and OLE object names; `convert_streaming` does not, and writes table rows after the paragraphs |
| PPTX | `convert_bytes` | None public | Not extracted | No | Text runs (`a:t`) only, under `## Slide N`. Slides ordered by file name number, not presentation order. No notes, tables or images |
| XLSX, XLSM | `convert_bytes`, `convert_streaming` | `xlsx::read_workbook`, `xlsx::read_sheet` return `Worksheet { index, name, rows }` | Not extracted | No | Sheets are named `Sheet N` from the `sheetN.xml` file name, not the workbook's sheet names. Values are the stored text: no number or date formatting, no formula text. Not verified: inline strings |
| XLSB | `convert_bytes` | None public | Not extracted | No | Cached scalar cells and shared strings only; formulas and styles are not reconstructed (comment in `document.rs`). Sheet names read from `xl/workbook.bin` |
| XLS (legacy) | Not supported: returns an error | None | n/a | No | Conversion is not implemented in this crate |
| ODT, ODP | `convert_bytes` (same reader for both) | None public | Not extracted | No | `text:p` and `text:h` only; every heading becomes `##`. No list, table or image handling in the code. ODP slides are not separated |
| ODS | `convert_bytes` | None public | Not extracted | No | One `## <table name>` section per sheet. Repeated empty columns are capped at 16,384. Not verified: numeric cells that carry both an `office:value` attribute and a `text:p` (the reader appends both) |
| EPUB | `convert_bytes` (every `.xhtml`/`.html` entry, sorted by file name); `epub::extract_text` (spine order) | `epub::read` returns a `Book` (manifest, spine, title) | Image references as `![alt](src)` only, no image data | No | `convert_bytes` does not follow the spine. `epub::read` requires `META-INF/container.xml`. No CSS, fonts, scripting or DRM handling ([docs/EPUB.md](docs/EPUB.md)) |
| RTF | `convert_bytes`; `rtf::parse`, `rtf::to_text`, `rtf::to_markdown`, `rtf::to_rtf`, `rtf::markdown_to_rtf` | `rtf::RtfDocument`: metadata, blocks (paragraph, list item, table), hyperlinks, footnotes, optional headers and footers, warnings | Pictures (`\pict`, `\shppict`) are skipped; `\object` shows its `\result` | No | Bold, italic, strike, headings (outline level, style or large bold text), lists from `\listtext`, nested tables, `HYPERLINK` fields, footnotes, code pages from `\ansicpg`, `\fcharset` and `\cpg` (including Mac Roman, Shift_JIS, GBK, Big5, EUC-KR and UTF-8 escapes). Not supported: underline and colour in Markdown, font names, revision marks, fields other than `HYPERLINK`, right-to-left reordering, numbering without `\listtext` (shown as bullets), Symbol-font glyphs other than the bullet, and code pages `encoding_rs` lacks (reported as warnings). Bad headers and exceeded limits are errors; other damage is reported as warnings. See the limits note in `CHANGELOG.md` |
| LaTeX (`.tex`, fragments and documents) | Not routed by `convert_bytes`. Use `latex::parse` with `latex::render_markdown`, `render_text` or `render_html` | `latex::Document`: title, authors, abstract, sections with numbers, lists, tables, display and inline math as TeX, figures with captions, labels and image candidates, citations, `thebibliography` items, labels and warnings | Figures and subfigures are structured; `tikzpicture` is opaque. Math is kept as TeX and not rendered. Packages are not loaded; unknown commands keep their argument text with a warning. `\input` is read only through a `FileProvider`. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| BibTeX, BibLaTeX (`.bib`) | Not routed by `convert_bytes` | `bibtex::parse` returns a `Bibliography` with entries, `@string`, `@preamble`, `@comment` and warnings; `bibtex::write` round-trips; `bibtex::csl_json` exports CSL-JSON | Names, pages, dates, DOIs and URLs are parsed; `crossref` is followed up to 8 levels. Only the standard entry types are typed; others keep their fields. Bounds in `bibtex::Limits` |
| arXiv e-print (gzip tar or gzip TeX) | Not routed | `arxiv_source::read_eprint` takes bytes; returns the main document, bibliography and figures with their image bytes and detected formats | No network access. Absolute, `..` and backslash paths, links and CRC mismatches are errors. Non-UTF-8 source is decoded as Latin-1 with a warning |
| MathML | Not routed | `mathml::to_tex` (uses the TeX annotation when present) | Covers the common presentation elements; unknown elements keep their content with a warning |
| CSV | `convert_bytes`, `convert_streaming` | None public | n/a | No | First row becomes the header. Delimiter (`,` `;` or tab) is guessed from the first line. UTF-8 (BOM stripped) or UTF-16 with BOM only; other encodings are an error |
| HTML, HTM | `html::to_markdown`, `convert_bytes` | `html::parse` returns `HtmlDocument` (blocks with signature and links); `html::to_blocks`, `html::links` | Image references as `![alt](src)`; `data:` URIs and `<figure>` subtrees dropped | No | Input decoded as lossy UTF-8, no charset detection. `script`, `style`, `nav`, `header`, `footer`, `aside`, `form`, `head` and similar subtrees are dropped, so `<title>` is not emitted. `convert_bytes` passes an empty base URL |
| TXT, MD, MARKDOWN | `convert_bytes` returns the input as lossy UTF-8 | None | n/a | No | No processing |
| PDF | `pdf::extract_file`, `pdf::extract_markdown`, `pdf::extract_pages_markdown` | `pdf::extract_with_format` (JSON), `pdf::extract_images` (page, size, colour space, JPEG bytes unchanged, Flate images as PNG), `pdf::geometry_from_transcode`, `pdf::outline`, `graph::DocumentGraph` | Output contains `![](img_N.png)` markers. With the `images` feature, `Document::images()` decodes embedded images and `PdfImage::to_png` encodes them; the crate does not write files | Optional: `pdf::extract_file_with_ocr` and `_options` (needs `pdftoppm` and `tesseract` on `PATH`; file paths only) | Parser filters: Flate, ASCII85, ASCIIHex, RunLength. No decryption code. 256 MiB input cap. `extract_with_format` page entries are placeholders (empty text, zero size) and its metadata is only the file size. Not verified: encrypted PDFs, reading order on multi-column pages. Page rasterisation is not implemented |

## Dependencies

Some dependencies are git repositories rather than crates.io packages. A clean build needs access to them; Cargo resolves optional dependencies into `Cargo.lock`, so `framefold` and `transcode` must be reachable even when their features are off.

| Dependency | Source | Pinned revision or version | Required or feature-gated | Purpose |
|---|---|---|---|---|
| `quick-xml` | crates.io | `0.42` | Required | XML parsing for OOXML, ODF and EPUB |
| `zip` | crates.io | `9` (`deflate` only) | Required | ZIP containers |
| `encoding_rs` | crates.io | `0.8` | Required | Code pages for RTF escapes (`src/rtf/charset.rs`) |
| `serde`, `serde_json` | crates.io | `1` | Required | Geometry types and JSON output in `pdf` and `graph` |
| `hashtrinity` | git, `Ptyktos/hashtrinity` | rev `59fea1c338e22d8ef5b82968c4e0621d38d00ba5` | Required (`syntax` feature) | `Table` hash map used by the cache, HTML extractor and PDF parser; syntax codec for Markdown frontmatter |
| `apeiron` | git, `webfold-ai/apeiron` | rev `56b7943671ca8f87059dce02fcc862054968d5ca` | Required | `apeiron::freq::Table` for the EPUB manifest index and the XLSX/XLSB path and relationship caches (zero-allocation `get_at(&str)`) |
| `framefold` | git, `Ptyktos/framefold` | rev `5a097c2e2448640c79d43b37b9246d0f7baea0d4` | Feature `framefold` (and `framefold-js`) | Re-exported as `frame_renderer` |
| `transcode` | git, `Ptyktos/transcode` | rev `8e65c5afeb8adb9663511760b80e19156970207e` | Features `media-codecs`, `images` | Codec re-export as `media_codecs`; embedded PDF image decode and PNG encode |
| `calamine` (local) | `calamine/` in this repository | version 0.36.1 | Not a dependency of the root crate | Workspace member only; see below |

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
- `docs/SETUP.md`, `RUN.md`, `EVAL.md` and `REPORT.md` describe a separate benchmark suite, not this crate's API.

## Licence

The crate is licensed under the Apache License, Version 2.0 (`LICENSE`). The `calamine/` sub-crate is MIT (`calamine/Cargo.toml`), and its upstream authorship is retained there.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and [SECURITY.md](SECURITY.md).
