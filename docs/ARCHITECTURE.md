# Architecture

This page describes `superdocuments` 0.9.0 as it is in the source. Every statement refers to code under `src/`, `Cargo.toml` or `.github/`. It was written by reading the code; the test suite was not run when it was prepared. Where the code does not settle a point, the page says "not verified".

## Inputs and outputs

| Entry point | Input | Output | Error type |
|---|---|---|---|
| `document::convert_bytes(ext, &[u8])` | document bytes plus a file extension | `String` of Markdown | `String` |
| `document::convert_path(&Path)` | a path; the whole file is read with `std::fs::read` (no size cap) | `String` | `String` |
| `document::convert_streaming(ext, &[u8], &mut impl Write)` | bytes plus extension | Markdown written to the sink | `String` |
| `document::DocumentCache::convert` / `convert_cached` | caller-supplied `u64` identity, extension, bytes, sink | Markdown written to the sink | `String` |
| `xlsx::read_workbook`, `read_sheet` | XLSX bytes | `Worksheet { index, name, rows }` | `String` |
| `epub::read`, `epub::extract_text` | EPUB bytes | `Book` (manifest, spine, title); Markdown text | `String` |
| `html::to_markdown`, `parse`, `to_blocks`, `render`, `links` | HTML bytes or `&str`, plus a base URL | Markdown, or blocks with signature and links | none (infallible) |
| `markdown::parse`, `markdown::to_html` | Markdown text | a tree (`MarkdownDocument`), then HTML | none |
| `pdf::extract_file`, `extract_markdown`, `extract_pages_markdown` | PDF path or bytes | Markdown (whole document or per page) | `io::Error` |
| `pdf::extract_file_with_ocr`, `extract_file_with_ocr_options` | PDF path | Markdown, with OCR text for pages without a text layer | `io::Error` |
| `graph::from_pdf_path`, `from_ocr_markdown`, `from_native_markdown` | PDF path or Markdown | `DocumentGraph` (pages, regions, spans) with `to_markdown` and `to_json` | `io::Error` for the path form |

`convert_bytes` dispatches on the lower-cased extension: `docx`/`docm`, `pptx`, `xlsx`/`xlsm`, `odt`/`odp`, `ods`, `xlsb`, `epub`, `rtf`, `csv`, `html`/`htm`, `txt`/`md`/`markdown`. `xls` returns an explicit error and any other extension returns `unsupported document extension`. PDF is not routed through `convert_bytes`.

## Main path

1. Document formats: `convert_bytes` calls a per-format function in `src/document.rs`. ZIP-based formats (OOXML, ODF, EPUB, XLSB) first go through `archive()`, which checks entry count and declared sizes, then read named entries with a bounded `entry()`. XML is read as an event stream with `quick-xml`.
2. HTML and EPUB chapters go through `html::to_blocks` and `html::render` (`src/html/extract.rs`), a single pass over the bytes with a tag stack that drops chrome subtrees.
3. `convert_streaming` writes CSV straight to the sink. Every other extension, DOCX, PPTX and XLSX included, falls back to `convert_bytes` and writes the resulting string once: those readers need the styles, numbering and relationship parts before the first line, and a single implementation keeps the streamed output identical to `convert_bytes`.
4. PDF: `pdf::extract_markdown` checks the size cap and the `%PDF-` header, then calls the in-tree parser (`pdf::parser::Document::parse`, then `markdown_pages`) inside `catch_unwind`.
5. PDF with OCR: `extract_file_with_ocr_options` runs the native extraction first. If no page has text, the whole document is OCR'd. If some pages have text and some do not, only the empty pages are OCR'd and spliced in page order, each page prefixed with `<!-- page N -->`. OCR renders pages with `pdftoppm` and recognises them with `tesseract`, both found on `PATH`.

## Key modules

| Module | Role |
|---|---|
| `document` | Format dispatch, per-format converters, the CSV streaming writer, `DocumentCache`, `fingerprint_bytes` |
| `ooxml` | Shared OOXML helpers (local-name matching, relationships, package-safe path resolution) and the DOCX, PPTX and XLSX readers |
| `xlsx` | Public XLSX/XLSM entry points returning structured `Worksheet` rows, backed by `ooxml::xlsx` |
| `media` | `extract_images` (`ExtractedImage`): pictures of DOCX, PPTX, XLSX, ODF and EPUB packages, with mime type from magic bytes, header size for PNG/JPEG/GIF/BMP, first referencing part and alt text. Re-exported from `document` |
| `epub` | Spine-driven EPUB reader (container, OPF, manifest, spine); see [EPUB.md](EPUB.md) |
| `html` | HTML to Markdown: `extract.rs` (single-pass extractor, `Template` for repeated-block detection, `chunk_markdown`), `entities.rs` |
| `markdown` | Hand-written Markdown parser and HTML renderer (not CommonMark-complete, per its module docs). Not used by `convert_bytes`. Only YAML frontmatter uses `hashtrinity::codec::syntax` |
| `pdf` | Entry points, OCR fallback, OCR metrics, legacy process adapter |
| `pdf::parser` | In-tree PDF parser: xref and object streams, filters (`decode.rs`, `inflate.rs`, `ascii_filters.rs`), text layout and Markdown output (`parse.rs`) |
| `pdf::geometry`, `pdf::transcode_populator`, `pdf::page_walker`, `pdf::acroform` | Geometry types, per-page spans and outline, page-complexity heuristics, AcroForm repair on the in-house parser |
| `graph` | `DocumentGraph` with provenance, projected from Markdown (`<!-- page N -->` blocks) and optionally overlaid with span geometry |
| `lib.rs` | Module list, feature-gated re-exports, the `Worksheet` type |

## State and caching

- There is no global state in the conversion code. The only locks are `Mutex`es in `pdf/mod.rs`, around OCR timings and the OCR metrics histograms.
- `DocumentCache` owns two `apeiron::map::hashtrinity::Table`s keyed by `(identity: u64, format: u8)`: parsed `LiftedDoc`s for CSV only, and rendered `Arc<String>`s for every other cacheable format (DOCX and XLSX included, so a warm hit does no parsing). It counts `hits()` and `misses()`. The code has no eviction, so entries live as long as the cache.
- The key is the identity the caller passes. The library does not hash or compare the bytes on a hit, so two different documents given the same identity return the first one's output. `fingerprint_bytes` is a helper: a 64-bit FNV-1a hash, documented in the source as not cryptographic.
- `convert_cached` builds a new `DocumentCache` on every call, so it never hits; hold a `DocumentCache` to reuse results.
- `DocumentCache::convert` accepts only extensions known to `DocFormat::from_extension` (`xlsx`, `xlsm`, `xlsb`, `xls`, `ods`, `docx`, `docm`, `pptx`, `odt`, `odp`, `epub`, `rtf`, `csv`). HTML, `htm`, `txt`, `md` and `markdown` are rejected by the cache although `convert_bytes` accepts them. `xls` passes the cache lookup and then fails in `convert_bytes`.
- OCR uses a temporary directory per request, removed by a drop guard (`OcrTempDir`). `OcrConfig` can attach an `OcrMetrics` (atomic counters) and a JSON-lines log path.

## Error and resource boundaries

| Boundary | Limit (constant) | Where |
|---|---|---|
| ZIP entries, `document.rs` (shared by `ooxml`) | 4,096 entries; 32 MiB per entry; 128 MiB total, checked against sizes declared in the archive | `MAX_ENTRIES`, `MAX_ENTRY`, `MAX_TOTAL` |
| XLSX worksheet cells | 20,000,000 cells allocated per worksheet; a larger sheet is an error | `MAX_CELLS` in `ooxml/xlsx.rs` |
| Extracted pictures | 64 MiB per picture (an error above it), 512 MiB in total, 10,000 pictures; an entry name that is absolute, has `..` or a backslash is an error | `media.rs` (`Limits`) |
| ZIP entries, `epub.rs` | 8,192 entries; 64 MiB per entry; 256 MiB for the whole file; 16 MiB per chapter; 64 MiB of rendered text | `epub.rs` constants |
| PDF input | 256 MiB, and the bytes must start with `%PDF-` | `MAX_PDF_BYTES` |
| OCR | Defaults: 20 pages, 2,400 px, 120 s, 1 worker, language `eng`. Accepted ranges: 1 to 2,000 pages, 256 to 4,000 px, 1 to 3,600 s, 1 to 16 workers. Language list limited to ASCII alphanumerics, `_` and `+` | `OcrOptions::validate` |
| ODS repeated columns | 16,384 | `odf_repeat` |
| CSV, HTML, plain text | No size limit in the code | `convert_bytes` |
| RTF | 128 MiB input and 128 MiB output, 256 group levels (`rtf::Limits`) | `convert_bytes` |

- Errors are returned, not thrown. The `document`, `xlsx` and `epub` modules return `Result<_, String>`; the `pdf` module returns `io::Result`.
- Panic containment (`catch_unwind`) exists in `pdf::extract_markdown` and `pdf::extract_pages_markdown` only. `geometry_from_transcode`, `outline` and `complexity` call the parser directly. The `document`, `epub` and `html` modules have no panic containment.
- OCR runs external processes under a shared wall-clock deadline (`run_bounded`). When the page limit is reached, the output ends with a notice that later pages may be omitted.
- The `epub::extract_text` call returns an error rather than partial output when a chapter is missing or the text cap is exceeded.

## Dependencies and optional features

The dependency table, the repository-versus-package naming and the reason for the `[patch]` section are in the [README](../README.md#dependencies). Summary of the features in `Cargo.toml`, all off by default:

- `framefold` (and `framefold-js`) adds the `framefold` dependency, re-exported as `frame_renderer`. Nothing else in the crate uses it.
- `media-codecs` adds `transcode`, re-exported as `media_codecs`.
- `images` adds `transcode` and enables `Document::images()` and `PdfImage::to_png` in `pdf/parser/mod.rs`.

Despite their names, `pdf::transcode_populator` and `geometry_from_transcode` use the in-tree `pdf::parser::Document` and do not need the `transcode` crate.

`.github/workflows/ci.yml` builds and tests with default features only.

## Experimental, shape-only and excluded code

- `calamine/`: a workspace member (`[workspace] members = ["calamine"]`) and a path dependency of the root crate, used only for the legacy `.xls` reader (`document::xls`). It is a fork of calamine 0.36.1 with `apeiron` maps (`calamine/APEIRON-FORK.md`). Its `rust-version` is 1.88. The root crate also depends on `zip` 9, whose `rust-version` is 1.88, so the root's declared 1.85 was already not what a build needs. `calamine/fuzz` is excluded from the workspace. The fork's CFB and BIFF readers were hardened against damaged files (see the CHANGELOG). The `xls` entry point still runs behind `catch_unwind`, because the BIFF record parser can still panic on some damaged input.
- `pdf::extract_with_format`, `pdf::document_metadata`: the source comments call this surface shape-only. In `extract_with_format` the `pages` entries have empty text and zero size, and `document_metadata` fills in only the file size.
- `pdf::to_markdown` and `to_markdown_with_manifest`: a legacy adapter that runs an external `pdffold` program, a `PDFFOLD_BIN` binary, or `cargo run --example md` against `PDFFOLD_MANIFEST`. Not used by the native path.
- `pdf::acroform::repair_acroform`: finds orphaned widget fields with the in-house PDF parser and appends an incremental update that adds the missing `/AcroForm` and a new catalog revision. It runs on files whose newest cross-reference section is a classic table, and skips encrypted files. The PDFium verification step of the LiteParse original is not carried.
- `pdf::page_walker`: coarse heuristics; the module comment says richer signals are not computed.
- `graph::from_pdf_path` labels every graph's source as OCR (`Source::Ocr`), whether or not OCR ran.
- `proof/`: Lean files and `proof/extract_pdf.rs`. The Rust file is compiled into the library through a `#[path]` attribute in `pdf/parser/mod.rs`. The Lean files are not part of the Cargo build; this page makes no claim about their status.
- `examples/check_transcode` and `examples/transcode_geometry` read fixtures from a hard-coded developer path and will not find them elsewhere. `examples/ocr_file.rs` takes a path argument.
- `.harness/` holds developer-tool state, not library code.

## Reading LaTeX, BibTeX and arXiv sources

These modules were added after the description above and are documented here in the same terms: what the code does, and where it stops.

| Module | Role | Public entry points | Limits (constant or option) |
|---|---|---|---|
| `tex_text` | TeX accents, letter commands and symbols to Unicode, shared by `bibtex` and `latex` | `decode(s, Ligatures::{None, Tex})`, `accented`, `symbol`, `combining_mark` | None; unknown control words are kept as written |
| `bibtex` | BibTeX/BibLaTeX reader, writer and CSL-JSON export | `parse`, `parse_with(&Limits)`, `write`, `csl_json`, `Bibliography::resolved_value`, name, page, date, DOI and URL parsers | `bibtex::Limits`: input 64 MiB, 100,000 entries, 1,000 fields per entry, 1 MiB per value, 64 brace levels |
| `latex` | Bounded LaTeX reader (not a TeX engine) and Markdown, plain-text and HTML renderers | `parse`, `parse_with(&Options, Option<&dyn FileProvider>)`, `figures`, `resolve_citations`, `render_markdown`, `render_text`, `render_html` | `latex::Options`: 32 MiB input, 100,000 expansions, 4 MiB of expansion text, depth 128, 256 files, 10,000 recorded warnings |
| `mathml` | MathML to TeX; uses the TeX annotation when present | `to_tex`, `to_tex_with(&Options)` | 4 MiB input, depth 64, 100,000 nodes |
| `arxiv_source` | Reads an arXiv e-print (gzip tar or gzip single file) from bytes; no network or disk access | `read_eprint`, `read_eprint_with(&Options)`, `detect_image_format` | `ArchiveLimits`: 64 MiB compressed, 256 MiB unpacked, 64 MiB per file, 4,096 entries, compression ratio 200 |
| `pdf::images` | Lists embedded image XObjects per page, with JPEG bytes unchanged and Flate images as PNG | `pdf::extract_images` | Page and XObject walk bounded at 100,000 nodes and depth 32; the PDF parser's own limits apply |

### How the LaTeX reader works

- The lexer (`latex/lexer.rs`) yields control words, control symbols, text runs, groups, math delimiters and spaces. Comments are removed. A blank line is a paragraph break. `\verb` is read here because its delimiter can be any character.
- The parser (`latex/parser.rs`) reads tokens from a stack of sources. The bottom one is the document. Macro expansions and `\input` files are pushed on top, so they are read in place. Math, verbatim bodies, arguments and file paths are read as raw text from the top source.
- `body` returns when it meets a stop for its caller: a closing brace, an `\end`, an `\item`, a table cell boundary, or the end of input. Environments and groups are built from those returns, so an unmatched delimiter is reported and reading continues.
- Numbering is computed by the reader: sections, figures, tables, equations (with `align` rows and `\nonumber`) and theorem-like environments. `\ref` and `\eqref` are resolved in the renderers, after the whole document has been read, so forward references work.
- Unknown commands produce a warning. Their arguments are kept as text, because they are read as ordinary groups.

### Limits of the LaTeX reader

- No TeX engine: packages are not loaded, `\newenvironment` and `\newtheorem` are supported only as far as the reader's tables go, and `\def` supports undelimited parameters only.
- Math is never expanded. Macros used inside math stay as TeX, which is correct for the output but means the Markdown or HTML is only as clear as the source.
- Figures: `\includegraphics` candidates follow graphicx's order (`.pdf`, `.png`, `.jpg`, `.jpeg`, `.eps`). `tikzpicture` is recorded as an opaque figure and is not rendered.
- `\input` and `\include` are read only through a `FileProvider`; `arxiv_source` supplies one that reads only from the archive.

### arXiv e-prints

`arxiv_source::read_eprint` takes the bytes served at `/e-print/<id>`. It rejects, with an error and no partial result: absolute, `..` and backslash paths; symbolic and hard links; gzip streams whose CRC or length do not match; and archives over a limit. The inflate output ceiling follows the compressed size times the ratio limit, so a corrupt or hostile stream fails quickly. Problems that do not stop the read (non-UTF-8 source, which is decoded as Latin-1; figures whose image is missing from the archive) are returned as warnings.

### PDF images and page rasterisation

`pdf::extract_images` lists embedded images with their page, object number, size, colour space and bits per component. DCT (JPEG) data is returned unchanged. Flate or unfiltered 8-bit DeviceGray and DeviceRGB images are returned as PNG, written with stored deflate blocks, which are valid but uncompressed. Other decodable images are returned as raw samples with their colour space. Page rasterisation is not implemented; this crate does not draw pages.
