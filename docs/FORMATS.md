# Formats

One section per format: what it reads, what comes out, where each claim comes from, and what it does not do.

This file is a snapshot of the format guides published on the Webfold website, written on 2026-11-07 from the source in this repository and in `webfold`. Paths are `repo/path`. An evidence row marked *named test* names a test that exists; it was not run to produce this document (the ODS fix is the exception, noted where it appears). Features on an open pull request are marked at the top of their section. When the code and this file disagree, the code is right; please open an issue.

| Section | Reads |
|---|---|
| [PDF](#pdf) | Text, headings, tables and images from born-digital PDFs, read in process. |
| [OCR](#ocr) | Text from scanned PDF pages, through Poppler and Tesseract run on your machine. |
| [DOCX](#docx) | Word files (DOCX, DOCM) to Markdown: paragraphs, heading styles and pipe tables. |
| [PPTX](#pptx) | PowerPoint (PPTX) to Markdown: each slide's text runs under a Slide N heading. |
| [RTF](#rtf) | RTF in and out: a structured reader, Markdown and plain text, and an ASCII writer. |
| [Spreadsheets](#spreadsheets) | XLSX, XLSB, ODS, CSV and legacy XLS to Markdown tables and structured cell rows. |
| [OpenDocument](#opendocument) | ODT, ODP and ODS to Markdown, and the structure the reader drops. |
| [EPUB](#epub) | EPUB books to Markdown, with two reading orders depending on the route. |
| [Scientific sources](#scientific-sources) | LaTeX, BibTeX, arXiv source archives and MathML, read as structure. |

## PDF

Webfold reads the text layer of a PDF with its own parser and returns Markdown with headings, HTML tables and image markers. It also returns per-page text, a JSON form, the bookmark outline and the embedded images. Pages with no text layer are not read natively; they go to the OCR fallback, described in the OCR guide.

### Reads

- **Classic and compressed cross-reference structures.** Classic `xref` tables, cross-reference streams and object streams (`/Type /ObjStm`). The chain follows `/Prev` and `/XRefStm`, so hybrid files are read, and compressed objects are read from their object stream, which is decoded once and cached.
- **Stream filters.** FlateDecode with `/Predictor` rows undone, ASCIIHexDecode, ASCII85Decode and RunLengthDecode, applied in the order listed in `/Filter`. A stream with any other filter, such as LZWDecode, or a DCT or JPX filter on a content stream, yields no data, so that stream's text is missing.
- **Fonts and character codes.** Character codes map to Unicode through the font's `/ToUnicode` CMap (`beginbfchar` and `beginbfrange`), then through `/Encoding /Differences` glyph names. Type0 (CID) fonts keep their multi-byte codes and take widths from the CIDFont `/W` array. A simple font with neither map reads single bytes from 0x20 upward as Latin-1.
- **Page content and Form XObjects.** Text is read from each page's content stream, including Form XObjects painted with `Do`. A form uses its own fonts and images when it has `/Resources`. Line breaks follow the text matrix moves (`Tm`, `Td`, `TD`, `T*`), and the output keeps content-stream order.
- **Embedded images.** `pdf::extract_images` lists image XObjects page by page. JPEG (`DCTDecode`) and JPEG 2000 (`JPXDecode`) bytes come back unchanged. 8-bit DeviceGray or DeviceRGB images with Flate or no filter come back as PNG, written with uncompressed (stored) deflate blocks. Other decodable images come back as raw samples with their colour space, and images with an undecodable filter are reported as unsupported with no data.
- **Input limits and failure modes.** The bytes must start with `%PDF-`, and the file must be at most 256 MiB (`MAX_PDF_BYTES`). Malformed input and parser panics are returned as errors. Encrypted files are not decrypted, and pages without a text layer produce no native text.

### Produces

- **Markdown.** One Markdown string for the whole file, with pages concatenated. Headings are `# ` lines at one level only. Each text line is one output line, and a word broken with a hyphen at a line end is joined when the next line starts with a lower-case letter. Native output has no page markers.
- **Tables as HTML.** Tables are written as HTML `<table>` with `<tr>` and `<td>`, not Markdown pipe tables. A cell gets a `colspan` when its text runs into empty columns. A table comes from drawn ruling lines when they form a grid, or from text gutters when the table has no rules.
- **Image markers and image bytes.** A picture painted on a page appears as `![](img_<object number>.png)` at its place in the output. The crate does not write image files. `pdf::extract_images` returns the bytes; with the `images` feature, `Document::images()` decodes to RGBA and `PdfImage::to_png()` encodes a PNG.
- **Page, text and JSON forms.** `extract_pages_markdown` returns one Markdown string per page, empty for pages with no text. `extract_with_format` returns Markdown, plain text, or a JSON object with `markdown`, `text`, `metadata`, `outline` and `pages`. In that JSON, `metadata` holds only `raw_file_size`, and each `pages` entry is a placeholder with a page number and empty text. `pdf::outline` returns the `/Outlines` bookmark entries separately.
- **Command line and MCP output.** `webfold pdf` prints the Markdown. With `--chunk N` it prints chunks sized to a byte target, and with `--jsonl` each chunk is one JSON object with `source`, `chunk`, `start`, `end` and `text`. The MCP tool `webfold_pdf` runs the same command, with `--chunk` when `chunk_chars` is given and without `--jsonl`.
- **Layout blocks and document graph (library).** `pdf::page_blocks` takes Markdown bytes, not PDF bytes, and labels each line as heading, paragraph, list item or table row. `graph::from_pdf_path` builds a `DocumentGraph` from the OCR-capable extraction and always labels its source as OCR, whether or not OCR ran.

### How it works

1. **Check the input.** The file is read up to a 256 MiB cap, and the bytes must begin with `%PDF-`. A file over the cap is refused before parsing.
2. **Expand compressed objects (webfold CLI only).** Before native parsing, `webfold pdf` rewrites object-stream members as ordinary objects (`webfold/src/cli/pdf_objstm.rs`). It accepts only unfiltered or FlateDecode object streams and stops at 128 streams, 32,768 objects, 32 MiB per stream and 64 MiB expanded. If this adapter fails, the file goes straight to the OCR entry point.
3. **Build the object table.** The parser reads the cross-reference chain from `startxref`, follows `/Prev` and `/XRefStm`, and stores each object by number. Objects load on first use. A compressed object is read from its object stream, and a refused stream is cached so it is decoded only once.
4. **Read text from content streams.** For each page, the parser decodes the content stream through its filters, builds the page's font maps (`/ToUnicode`, `/Differences`, `/Widths`) and runs a text accumulator over the operators. Each positioned run keeps its gap from the previous run, so spacing and table columns can be read from geometry.
5. **Classify lines.** Each line is tested for a heading, a table region and an image draw. A heading is a line larger than the page body size, a bold short line, or a numbered section such as `1.2 Methods`. The body size is the character-weighted modal size across the document. Table regions come from drawn rules when they form a grid, otherwise from text gutters.
6. **Emit Markdown per page.** Table regions are written as `<table>`, image draws as markers at their place in the stream, and hyphenated line ends are joined. Pages are concatenated with a newline between them, and no page marker is written. If the result has no alphanumeric text line, or native parsing fails, the caller can run the OCR fallback (see the OCR guide).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| The native entry point checks the size cap and the %PDF- header, then parses inside catch_unwind and returns errors rather than panicking. | read from the code | superdocuments/src/pdf/mod.rs:62-86 | native_pages_of_bytes checks MAX_PDF_BYTES and the header, then wraps Document::parse and markdown_pages in catch_unwind. |
| The filter chain supports FlateDecode, ASCIIHexDecode, ASCII85Decode and RunLengthDecode. Any other filter returns no data for that stream. | read from the code | superdocuments/src/pdf/parser/decode.rs:266-300 | decode_within applies each named stage in order. LZWDecode, the image filters and any undecodable stage hit the default arm and return None. The doc comment on decode_within states this. |
| The parser has no decryption code. /Encrypt appears only in the AcroForm repair path, which skips encrypted files. | read from the code | superdocuments/src/pdf/acroform.rs:57 | A search of superdocuments/src/pdf/parser finds no Encrypt, password or RC4/AES handling. README.md format matrix (PDF row) says there is no decryption code. The only encrypted-file test is acroform.rs:430, encrypted_pdf_is_skipped. |
| Character codes decode through the ToUnicode CMap first, then the /Differences map, then a Latin-1 fallback for single-byte codes at 0x20 and above. | read from the code | superdocuments/src/pdf/parser/parse.rs:1814-1830 | CMap::decode looks up map then enc, and falls back to the byte as a character only when the code width is one and the code is at least 0x20. page_fonts (parse.rs:2127-2170) builds the ToUnicode map and the Differences map for every font. |
| Headings are classified by font size against the body size, by bold short lines, and by numbered section prefixes, and are written as a single-level `# ` prefix. | read from the code | superdocuments/src/pdf/parser/parse.rs:347-358 | is_heading gates each signal to lines under 150 characters, and to under 90 for bold and numbered lines. emit_lines writes the prefix `# ` only (parse.rs:977). No level 2 or deeper heading is produced. |
| Tables are emitted as HTML table markup with colspan for cells that run into empty columns. | read from the code | superdocuments/src/pdf/parser/parse.rs:828-870 | emit_grid_table writes `<table>` (line 851), one `<tr>` per row and one `<td>` per cell, and adds colspan when the next columns are empty and the text reaches past their midpoint. |
| Reading order is the order of the content stream. The text accumulator does not sort text by position. | read from the code | superdocuments/src/pdf/parser/parse.rs:1557-1560 | The doc comment on the accumulator says document order is the stream order, with no sort and no binary search. Columns are detected only inside table candidates (columns_from_gutters, parse.rs:560). |
| A picture painted with Do is written as an image marker at its place in the document flow. | read from the code | superdocuments/src/pdf/parser/parse.rs:968-972 | emit_lines writes `![](img_<object>.png)` for an image marker line. The Do operator (parse.rs:1752) places the marker where the image is painted. |
| Image listing returns JPEG and JPEG 2000 bytes unchanged, Flate or unfiltered 8-bit gray and RGB images as PNG with stored deflate blocks, and other images as raw samples. | read from the code | superdocuments/src/pdf/images.rs:1-20, 262-300 | The module doc lists the encodings. The match on the last filter handles DCTDecode (JPEG bytes unchanged when no filter precedes it, or Flate only), JPXDecode, and the default arm, which uses encode_png only when bpc is 8 and the colour space is DeviceGray or DeviceRGB. |
| Object streams and xref streams are loaded natively, and a refused object stream is decoded only once. | named test (not run for this document) | superdocuments/src/pdf/parser/xref_tests.rs:234 and superdocuments/src/pdf/parser/xref_tests.rs:650 | Named tests: xref_stream_with_object_stream_loads_compressed_catalog and refused_object_stream_is_decoded_once. Not run for this sheet. Also flate_xref_stream_with_png_predictor_is_undone (xref_tests.rs:245). |
| On the mixed fixture, extract_pages_markdown returns two pages, with native text on page 1 and no text on page 2. | named test (not run for this document) | superdocuments/src/pdf/mod.rs:1619 | Named test: mixed_fixture_keeps_native_page_and_ocrs_only_the_scan. It checks page count and text presence only. Not run for this sheet. |
| Embedded image listing agrees with poppler's pdfimages on the recorded fixtures, with zero disagreements. | measured | superdocuments/benches/pdf-images/results-images-mixed.txt and superdocuments/benches/pdf-images/results-inherited-predictor.txt | Six images in the mixed file and one in the inherited-predictor file, 0 disagreements in both. compare_pdfimages.py compares page, size, colour space, bits, encoding category, JPEG bytes and decoded PNG pixels. The results files carry no date and the run was not repeated here. |
| The webfold CLI reads PDFs with `pdf` and `convert`, with OCR flags that set environment variables. | read from the code | webfold/src/main.rs:724-737 | The usage string is `webfold pdf <file.pdf> [--chunk n] [--ocr-lang eng+chi_sim] [--ocr-max-pages n] [--ocr-max-dim px] [--ocr-timeout secs]`. Each OCR flag sets WEBFOLD_OCR_* for the call. `convert` (main.rs:214-226) also routes .pdf through pdf_to_markdown. |
| The CLI returns native text whenever native extraction gives any readable line. Per-page OCR is not applied in that case. | read from the code | webfold/src/cli/document.rs:279-311 | pdf_to_markdown calls extract_file_with_ocr_options only when has_readable_text is false or native parsing errors. A scanned page inside a document that also has text is therefore not OCRed by the CLI. The library function does merge per page (superdocuments/src/pdf/mod.rs:923-948). |

### Limits

- Encryption is not handled. superdocuments has no decryption code, so an encrypted file's content streams are not read, native text is expected to be empty, and the OCR fallback runs. No test covers text from an encrypted file.
- Some filters are refused. LZWDecode, DCTDecode, JPXDecode, CCITTFaxDecode and JBIG2Decode on a content stream give no data for that stream. The webfold object-stream adapter accepts only unfiltered or FlateDecode object streams.
- Font encodings are partial. The source has no WinAnsiEncoding, MacRomanEncoding or StandardEncoding table. Without ToUnicode or /Differences, bytes from 0x80 to 0xFF are read as Latin-1, so curly quotes, dashes and the euro sign in WinAnsi text can come out wrong.
- Reading order is content-stream order, not visual order. Prose in two columns is not reordered, so a producer that writes column by column can interleave the columns. The two-column fixture has an expected-text file, but no Rust test reads it.
- Headings, tables and figures are heuristics. Headings have one level. Tables depend on drawn rules or repeated text gutters, and the borderless fixture is not wired to a test. Figures are image markers only, with no captions linked to them. Equations come out as the glyphs in the file, with no formula recognition.
- Output is thin beyond the text. Native output has no page markers. In JSON, metadata is only the file size, so title, author and date from the /Info dictionary are not returned, and the pages entries carry no text or geometry. There is no PDF benchmark: the `documents` benchmark (slug documents) covers office and ebook files, and its own limits say there is no corpus-scale PDF benchmark yet.
- The CLI and the library differ for mixed files. `webfold pdf` drops the text of a scanned page when the same document has a text layer, because it returns the native output as soon as it has any readable line. The library function merges OCR per page.
- The arXiv PDF fallback does not work on the local source. The webfold arXiv module calls document::convert_bytes with pdf (webfold/src/arxiv/mod.rs:280), and superdocuments returns `unsupported document extension: pdf` for that (src/document.rs:849-867). Webfold pins superdocuments at rev e0fb9a8 (Cargo.toml:202), which is not in the local clone (HEAD 0e8d51b), so this is not verified at the pinned revision.

## OCR

When a PDF has no readable text, or a page has none, the library renders the page with pdftoppm and recognises it with tesseract. The output is Markdown with a marker before each recognised page. Image files such as PNG or JPEG are not accepted; the input must be a PDF path.

### Reads

- **When OCR runs.** Native parsing runs first. The library OCRs the whole file when no page has an alphanumeric text line, or when native parsing fails. When some pages have text and some do not, it OCRs only the empty pages and keeps the native text of the rest. The webfold CLI runs OCR only when native output has no readable line or fails, so it does not do the per-page merge.
- **Input.** A PDF file path. The entry points take a path, not bytes, and pass that path to pdftoppm. No code path converts PNG, JPEG or TIFF files; convert_bytes has no arm for image extensions.
- **Languages.** Tesseract language codes in `OcrOptions::languages`, default `eng`. Several languages are joined with `+`, for example `eng+deu`. Only ASCII letters, digits, `_` and `+` are accepted, up to 64 characters. The code does not check that the language data is installed.
- **Page, size and time limits.** Pages: default 20, allowed 1 to 2,000. Longest rendered edge: default 2,400 px, allowed 256 to 4,000. Wall-clock budget: default 120 seconds, allowed 1 to 3,600, shared by rendering and recognition. Workers: 1 to 16 Tesseract processes at once. Pages render in windows of up to 25, sized so that one window's PNG files stay within 512 MiB.
- **Output shape.** Markdown with `<!-- page N -->` before each page that has recognised text. Pages with blank OCR output are dropped. If the page cap is reached while the document continues, the output ends with a notice that later pages may be omitted.
- **Metrics and log (library).** `OcrConfig` takes an `OcrMetrics` object, which renders Prometheus text, and a JSON-lines log path. The log has `started` and `finished` events and per-stage `flame` events. The webfold CLI sets neither.

### Produces

- **Page-marked Markdown.** Each recognised page is written after a `<!-- page N -->` marker, in page order. In a mixed file, native pages keep their native text under the same markers, and OCR text fills the empty pages.
- **Tesseract plain text.** The pipeline reads the `.txt` file that Tesseract writes for each page, capped at 4 MiB. No bounding boxes, word confidences or hOCR are kept, so the output has no coordinates.
- **Truncation notice.** When the page limit is reached and the document has more pages, the output ends with `[OCR stopped at the configured limit of N pages; later pages may be omitted]`. The notice is part of the string, so downstream code must check for it.
- **Document graph (library).** `graph::from_pdf_path` builds a `DocumentGraph` from the extraction output with one region per page, and labels the source as OCR with the model string `superdocuments-0.8.1`, psm 3 and language `eng`, whether or not OCR ran.
- **JSON-lines log entries (library, when a log path is set).** A `started` event records the input size, languages, page limit, image size and timeout. A `finished` event records the outcome (native, ocr or error), whether OCR was used, whether the run was truncated, pages rendered, bytes rendered, bytes OCR'd, and elapsed, render and recognise times in milliseconds.

### How it works

1. **Read and parse natively.** The PDF is read up to the 256 MiB cap and the native parser runs. If it returns text on every page, the result is returned as is. If it fails, the whole file goes to OCR. If only some pages are empty, only those pages go to OCR.
2. **Choose the pages.** The selected pages are the whole file, or only the empty pages in a mixed file. Both are limited to the first `max_pages` pages, and a window is rendered only when it contains a selected page.
3. **Render a window.** pdftoppm runs as `pdftoppm -f FIRST -l LAST -scale-to PX -png INPUT PREFIX` for up to 25 pages at a time, under the shared deadline. The code does not set a DPI flag. The PNG files of one window must total no more than 512 MiB.
4. **Recognise each page.** tesseract runs as `tesseract IMAGE BASE -l LANGS --psm 3`, with `workers` processes at a time. When there is more than one worker, each process gets `OMP_THREAD_LIMIT=1`. Results are collected in page order.
5. **Collect and cap the text.** The `.txt` output of each page is read with a 4 MiB limit, trimmed, and dropped if blank. If the collected text passes 16 MiB across the request, the run fails with an error.
6. **Assemble and clean up.** Pages are joined with `<!-- page N -->` markers, and the truncation notice is appended when the cap was hit with document remaining. Rendered PNG files are deleted after each window, and the temporary directory is removed when the run ends.

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| OCR is entered only when native parsing has no alphanumeric text on any page, or native parsing fails. In the mixed case, only empty pages are passed to OCR. | read from the code | superdocuments/src/pdf/mod.rs:911-966 | extract_file_with_ocr_inner runs extract_markdown first, then has_embedded_text. A failed native parse goes straight to tesseract_ocr_with_observer on the whole file. |
| In a mixed file, the native text of each page is kept, and only the pages with no text are OCRed and spliced in page order. | read from the code | superdocuments/src/pdf/mod.rs:923-948 and superdocuments/src/pdf/mod.rs:1264-1311 | extract_pages_markdown gives per-page text, and merge_native_and_ocr writes `<!-- page N -->` before each page with text. |
| Pages are rendered with pdftoppm using a page range, a longest-edge size and PNG output, and no DPI flag. | read from the code | superdocuments/src/pdf/mod.rs:1078-1088 | Arguments: -f first, -l last, -scale-to max_image_dimension, -png, the input path and a temporary prefix. stdout and stderr are discarded (run_bounded, mod.rs:1360-1366). |
| Pages are recognised with tesseract in PSM 3, with the language list from the options, and with OpenMP limited to one thread when more than one worker runs. | read from the code | superdocuments/src/pdf/mod.rs:1143-1157 | Arguments: IMAGE BASE -l LANGUAGES --psm 3. The text is read from BASE.txt. OMP_THREAD_LIMIT=1 is set when workers > 1. |
| Option ranges are checked before any process starts: language string, page count 1 to 2,000, image size 256 to 4,000, workers 1 to 16, timeout 1 to 3,600 seconds. | read from the code | superdocuments/src/pdf/mod.rs:713-752 | OcrOptions::validate returns InvalidInput errors. Its limit constants are at superdocuments/src/pdf/mod.rs:22-29. |
| Every child process runs under one shared deadline. A child that passes the deadline is killed and reported as timed out. | read from the code | superdocuments/src/pdf/mod.rs:1360-1384 | run_bounded polls try_wait every 10 ms and calls kill and wait at the deadline. The deadline is set once per request (mod.rs:1054). |
| Temporary files live in a per-request directory under the system temp directory, created with mode 0700 on Unix and removed on drop. | read from the code | superdocuments/src/pdf/mod.rs:1320-1358 | OcrTempDir::create uses std::env::temp_dir() and a process id plus counter name. Drop calls remove_dir_all. |
| The webfold CLI sets OCR defaults from environment variables: language eng, 20 pages, 2,400 px, 120 seconds, and one worker. Its help text says the timeout is at most 600 seconds, but the code does not enforce that limit. | read from the code | webfold/src/cli/document.rs:256-277 | ocr_options_from_env reads WEBFOLD_OCR_LANG, WEBFOLD_OCR_MAX_PAGES, WEBFOLD_OCR_MAX_DIM and WEBFOLD_OCR_TIMEOUT_SECS. The doc comment at line 258 says at most 600. The library accepts up to 3,600 seconds. |
| The webfold pdf command takes the OCR flags and writes each into its environment variable before extraction. | read from the code | webfold/src/main.rs:724-737 | The flags --ocr-lang, --ocr-max-pages, --ocr-max-dim and --ocr-timeout map to WEBFOLD_OCR_LANG, WEBFOLD_OCR_MAX_PAGES, WEBFOLD_OCR_MAX_DIM and WEBFOLD_OCR_TIMEOUT_SECS. The MCP tool webfold_pdf (src/bin/webfold_mcp.rs:681-690) takes no OCR flags. |
| The input is a PDF path only. No image-file arm exists in the document converter. | read from the code | superdocuments/src/document.rs:849-867 | convert_bytes handles office, ebook, CSV, HTML and text extensions and returns `unsupported document extension` for anything else, including png, jpg and tif. README.md format matrix says the OCR entry points are file paths only. |
| The test for the OCR path is ignored by default and needs pdftoppm and tesseract on PATH. It checks a raster-only fixture against its phrase and a Flate and ASCII85 fixture. | named test (not run for this document) | superdocuments/src/pdf/mod.rs:1629 and superdocuments/tests/fixtures/README.md | Named test: tesseract_fallback_handles_raster_and_unsupported_filter_pdfs. It asserts that scanned-text.pdf recovers the phrase `Scanned fixture: OCR works.` after normalising case and punctuation. Not run for this sheet. |
| The mixed-file page cap keeps native text and notes the truncation. | named test (not run for this document) | superdocuments/src/pdf/mod.rs:1599 | Named test: mixed_fixture_scan_beyond_the_page_cap_keeps_native_text_and_notes_truncation, with max_pages set to 1. It expects the native first page and the limit notice. It is not marked ignored, and it calls pdftoppm. Not run for this sheet. |
| A child process that passes its deadline is killed and reported as an error. | named test (not run for this document) | superdocuments/src/pdf/mod.rs:1691 | Named test: ocr_child_process_respects_deadline. It uses sleep and sh, not pdftoppm or tesseract. Not run for this sheet. |
| Ranges of OCR pages are split into windows with no gaps and no overlaps. | named test (not run for this document) | superdocuments/src/pdf/mod.rs:1536 | Named test: ocr_windows_cover_the_page_range_without_gaps_or_overlap. It is pure and needs no tools. Not run for this sheet. |

### Limits

- External tools are required and not probed. pdftoppm (Poppler) and tesseract must be on PATH, and the Tesseract language data for each requested language must be installed. The code does not check either at start-up. A missing binary is a spawn error, which the webfold CLI prints as `pdf error reading` and exits with status 1.
- Recognition quality is not measured in these repositories. No accuracy or speed result for OCR is recorded, and the only OCR test is one phrase on one raster page. There is no orientation detection, deskew or image cleanup: the only Tesseract option is `--psm 3`. A rotated fixture exists in tests, and the code has no rotation handling for it. Treat OCR text as a first pass.
- Resolution is set by page size, not by DPI. Pages are scaled so the longest edge equals the image dimension (2,400 px by default, up to 4,000). Effective resolution therefore depends on the page's physical size, and no DPI is passed to pdftoppm.
- Page and time caps apply by default. The default page cap is 20 in both the library and the CLI, and a longer document is cut with a notice. The default time budget is 120 seconds for rendering and recognition together. The CLI help text says at most 600 seconds, but that is not enforced, and the library accepts up to 3,600.
- Only PDF input is supported. PNG, JPEG and TIFF files are not converted, because every OCR path starts from pdftoppm on a PDF path, and the converter has no image arm.
- The output is plain text with no layout. Tesseract's text file is the only result kept, so bounding boxes, confidences, table cells and reading-order repair are not available. Columns and tables are recognised only as far as Tesseract's own segmentation reads them.
- The CLI does not merge OCR per page, and the MCP tool cannot set OCR options. `webfold pdf` returns native text whenever any line is readable, so scanned pages in a document with a text layer are not OCRed by the CLI. The MCP tool webfold_pdf takes only path, chunk_chars and timeout_seconds (its own run timeout, 1 to 600 seconds, default 120), so OCR options come from WEBFOLD_OCR_* environment variables and a long OCR run can exceed the MCP timeout.
- Deployment needs a writable temporary directory and enough disk for a render window of up to 512 MiB, plus one Tesseract process per worker, up to 16. Nothing in the PDF or OCR code makes a network call, so the pages stay on the machine that runs the command.

## DOCX

Reads the main document part of a DOCX or DOCM package and writes Markdown: paragraph text, headings for the Heading1 to Heading6 style ids, and pipe tables. Images are not extracted, but drawing alt text is written as text. The whole-document writer and the streaming writer do not give the same output.

### Reads

- **DOCX and DOCM packages.** Both extensions share one reader (superdocuments/src/document.rs:851-852 in convert_bytes). The main part is word/document.xml, or the part named by the officeDocument relationship in _rels/.rels. That target must be relative and must not contain '..' (document.rs:212-246).
- **Paragraph text.** Text comes from w:t and math m:t elements (document.rs:260, 290). w:tab becomes a tab and w:br becomes a newline (document.rs:288-289). Run formatting is not read, so bold, italic, underline and colour are dropped.
- **Headings by style id.** A paragraph is a heading only when its w:pStyle value is Heading1 to Heading6 (document.rs:279-287). The match is on the style id, not the display name, so a style called 'Heading 1' with another id, or the Title style, stays a plain paragraph.
- **Tables.** w:tbl, w:tr and w:tc build a grid. The paragraphs in one cell are joined with a space (document.rs:294-316). Each finished table is written as a Markdown pipe table, and its first row is the header (document.rs:93-119, 317-322).
- **Drawing alt text and embedded objects.** The descr attribute of wp:docPr is added to the paragraph text (document.rs:266-273). An embedded OLE object is written as 'Embedded object:' followed by its ProgID (document.rs:274-278). No image bytes are read.
- **Charts and diagrams (convert_bytes only).** After the body, every word/charts/*.xml part gets its title and values written in bold, and every word/diagrams/*.xml part gets its text appended (document.rs:328-352). convert_streaming does not do this.

### Produces

- **Markdown string from convert_bytes.** Paragraphs are separated by blank lines. A heading is the matching number of '#' followed by the text. Tables are pipe tables, and the output is trimmed at the end (document.rs:298-322, 354). The README lists no public structured output for DOCX.
- **Markdown written by convert_streaming.** Paragraphs are written while the XML is read. Table rows are not: all of them go into one buffer that is written after the last paragraph, as a single table (document.rs:1121, 1184-1201). Tabs, line breaks, alt text, charts, diagrams and OLE names are not written (document.rs:1137-1152).
- **Table shape.** Every row is padded with empty cells to the widest row in the table, so the column count is the widest row (document.rs:97-108). A table with no rows writes nothing.
- **Internal lift for the cache.** superdocuments keeps DOCX paragraphs as coordinates (paragraph index, row 0, column 0) for its own lifted-document cache, and heading level in a second coordinate (document.rs:1275-1281). This type is public in the crate but is not documented as a DOCX output.

### How it works

1. **Dispatch.** convert_bytes sends docx and docm to the DOCX reader (document.rs:850-852). The webfold CLI sends .docx and .docm to webfold's convert_path, which delegates to this function (webfold/src/cli/document.rs:130-140; webfold/src/document.rs:140-147).
2. **Bound the archive.** The ZIP is opened only after its entry count (at most 4,096), each declared entry size (at most 32 MiB) and the declared total (at most 128 MiB) pass (document.rs:12-14, 27-49).
3. **Find the main part.** Use word/document.xml. If it is absent, follow the officeDocument relationship in _rels/.rels, rejecting absolute paths and '..' (document.rs:213-246).
4. **Read XML events.** Each w:p end closes a paragraph. A w:pStyle value sets the heading level for that paragraph. Inside a table, the paragraph text goes into the current cell instead of the output (document.rs:247-310).
5. **Close tables.** Each w:tbl end writes a Markdown table from the collected rows, then clears the rows and the in-table flag (document.rs:317-322).
6. **Append chart and diagram text.** After the body, each word/charts/*.xml and word/diagrams/*.xml part is read and its text is appended to the output (document.rs:328-353).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| Headings come only from the style ids Heading1 to Heading6, capped at level 6 | read from the code | superdocuments/src/document.rs:279-287 | strip_prefix('Heading') then parse; any other id gives level 0. |
| Tabs and line breaks become a tab and a newline inside the paragraph text | read from the code | superdocuments/src/document.rs:288-289 | w:tab pushes '\t'; w:br pushes '\n'. |
| Tables become Markdown pipe tables; the first row is the header; pipes in cells are escaped | read from the code | superdocuments/src/document.rs:93-119, 317-322 | markdown_table replaces '\|' with '\\|' (line 106) and writes the separator after row 0. |
| Drawing alt text is inlined; no image data is extracted | read from the code | superdocuments/src/document.rs:266-273 | Only the descr attribute of wp:docPr is read. |
| OLE objects are written as 'Embedded object:' plus the ProgID | read from the code | superdocuments/src/document.rs:274-278 | o:OLEObject ProgID attribute. |
| convert_streaming writes all table rows together, after the last paragraph | read from the code | superdocuments/src/document.rs:1121, 1184-1201 | The table buffer is pushed to on every w:tr end and never cleared until the final flush, so all tables in the document merge into one table at the end. Not run. |
| The streaming writer has no arms for w:tab, w:br, wp:docPr, OLE objects or charts | read from the code | superdocuments/src/document.rs:1131-1152 | Its match handles w:tbl, w:t, m:t, w:pStyle, text, w:p end, w:tc and w:tr only. |
| ZIP bounds: 4,096 entries, 32 MiB per declared entry, 128 MiB declared total | read from the code | superdocuments/src/document.rs:12-14, 27-49 | archive() checks the declared sizes from the central directory before any entry is read. |
| Entry reads are capped at the entry limit plus one byte, whatever the declared size | read from the code | superdocuments/src/document.rs:51-64 | entry() uses file.take(MAX_ENTRY + 1) and rejects anything longer. |
| The officeDocument target must be relative and must not contain '..' | read from the code | superdocuments/src/document.rs:241-245 | Absolute paths and any '..' component return an error. |
| Routing: the webfold CLI sends docx and docm to the superdocuments conversion | read from the code | webfold/src/cli/document.rs:130-140 | markdown_source lists docx and docm in the document branch and calls webfold::document::convert_path. |
| A DOCX heading and body convert to '# Report' and 'Body'; an officeDocument target at content/main.xml is followed | named test (not run for this document) | superdocuments/src/document.rs:2025-2030 (test office_packages_convert_through_public_api) | Named test; not run. It covers one heading and one paragraph only. |
| convert_streaming and convert_bytes are compared on a one-heading, one-paragraph DOCX | named test (not run for this document) | superdocuments/src/document.rs:2169-2183 (test convert_streaming_matches_convert_bytes_for_csv_docx_xlsx) | Named test; not run. The input has no tables, tabs, alt text or charts, so the gaps above are not covered. |
| Webfold converts mixed office and ebook fixtures in about 27.7 microseconds median on one document | measured | webfold-website/src/lib/marketing/benchmarks.ts:46 (slug documents, Measured 2026) | 45 public fixtures across formats, uncached. The figure is not broken out by format, so it is not a DOCX number. |

### Limits

- Headings are read only from the style ids Heading1 to Heading6. Heading styles with other ids and the Title style stay plain text (document.rs:279-287).
- No list markers and no run formatting. w:numPr is never read, so list items are plain paragraphs, and bold, italic, underline and colour are dropped.
- Hyperlink text is kept but the target URL is dropped. Only w:t and m:t are read (document.rs:260).
- Footnotes, endnotes, comments, headers and footers are not read. Only word/document.xml, _rels/.rels, word/charts and word/diagrams parts are opened.
- Text in w:delText (deleted tracked changes) is not read, because only w:t and m:t are collected (document.rs:260).
- A soft line break (w:br) in a table cell puts a newline into the cell, and markdown_table escapes only pipes, so the Markdown row breaks (document.rs:289, 298, 106). Not run.
- Nested tables are not tracked. The first w:tbl end writes and clears the shared row buffer, so the outer table layout breaks (document.rs:259, 317-322). Not run.
- convert_streaming and convert_bytes give different Markdown for the same DOCX (document.rs:1137-1152 against 266-352).

## PPTX

Reads the slide parts of a PPTX package (ppt/slides/slideN.xml) and writes each slide as a '## Slide N' section holding its text runs. Speaker notes, tables, images and the slide order in presentation.xml are not used, so the output shows only the text that sits in the slide parts.

### Reads

- **PPTX packages.** Routed only by the pptx extension (superdocuments/src/document.rs:853). Each slide is a part named ppt/slides/slideN.xml, and N is parsed from the file name (document.rs:357-373).
- **Text runs.** Every a:t element in a slide part is read. The reader keeps no record of the surrounding shape, paragraph or table, so text from any a:t is taken (document.rs:379, xml_text at 66-91). Each run is followed by a space, so a slide comes out as one line of text.
- **Slide order by file name.** Slides are sorted by the number in the file name, not by the order in presentation.xml (document.rs:375). The presentation part is never opened.
- **Parts that are opened.** Only names under ppt/slides/ that end in .xml and have a numeric middle part are opened. Notes, layouts, masters, charts and media parts are never opened (document.rs:359-374).
- **Empty slides.** A slide whose text is empty after trimming is left out (document.rs:380). Heading numbers can therefore skip.

### Produces

- **Markdown string.** Each slide becomes '## Slide N', a blank line, and its text on one line. Each slide's text is trimmed before it is written, and slides are joined with a horizontal rule (document.rs:377-386). The whole result is returned as one String.
- **No streaming writer.** convert_streaming has no PPTX arm. It falls back to convert_bytes and writes the whole string once (document.rs:913-916).
- **No document metadata.** The docProps parts (title, author, dates) are never opened, so none of that metadata appears in the output (document.rs:357-388).
- **No structured type.** The README lists no public structured output for PPTX (format matrix, PPTX row).

### How it works

1. **Dispatch.** convert_bytes sends pptx to pptx() (document.rs:853). The webfold CLI sends .pptx to webfold's convert_path, which delegates to this function (webfold/src/cli/document.rs:130-140; webfold/src/document.rs:140-147).
2. **Bound the archive.** archive() applies the same ZIP limits as the DOCX reader: 4,096 entries, 32 MiB per declared entry, 128 MiB declared total (document.rs:12-14, 27-49).
3. **Collect slide parts.** Keep names that start with ppt/slides/slide, end with .xml and have a numeric middle part (document.rs:359-374).
4. **Sort.** Sort the list by that number (document.rs:375).
5. **Read text.** For each slide, read the part and join every a:t text with spaces, then trim (document.rs:377-379; xml_text at 66-91).
6. **Write sections.** Write '## Slide N' and the text for each non-empty slide, separated by a horizontal rule (document.rs:380-386).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| Slides are read from ppt/slides/slideN.xml and sorted numerically by N | read from the code | superdocuments/src/document.rs:357-375 | The name prefix and suffix are stripped, then the middle is parsed as a number. |
| Only a:t text is extracted from a slide | read from the code | superdocuments/src/document.rs:379, 66-91 | xml_text(&xml, b'a:t') collects text inside a:t elements and adds a space after each one. |
| Slide sections are separated by a horizontal rule; empty slides are dropped | read from the code | superdocuments/src/document.rs:377-386 | The separator is written only between non-empty slides. |
| The presentation part is not opened, so presentation order is not used | read from the code | superdocuments/src/document.rs:357-375 | The only parts read are names under ppt/slides/. |
| convert_streaming has no pptx arm and falls back to convert_bytes | read from the code | superdocuments/src/document.rs:913-916 | The fallback arm converts the whole document, then writes one string. |
| Only the pptx extension is routed to the PPTX reader | read from the code | superdocuments/src/document.rs:853 | No ppt arm exists in convert_bytes. The webfold extension list (webfold/src/cli/document.rs:130-140) also has no ppt. |
| XML text is unescaped with quick-xml's unescape; if that fails, the raw text is kept | read from the code | superdocuments/src/document.rs:16-18 | text() falls back to the raw string on an unescape error. The readers have no DTD or custom-entity handling. |
| The ZIP limits are shared with the DOCX and spreadsheet readers | read from the code | superdocuments/src/document.rs:12-14, 27-49 | Same constants and archive() function as the other OOXML readers. |
| A slide containing the text Launch converts and contains that word | named test (not run for this document) | superdocuments/src/document.rs:2031-2035 (test office_packages_convert_through_public_api) | Named test; not run. It uses one minimal slide part with one run. |
| No test covers slide ordering, tables, notes or empty slides | named test (not run for this document) | superdocuments/src/document.rs:1973-2200 (test module) | The test names I read include no PPTX case beyond the minimal slide above. Not run. |

### Limits

- Slide order is file-name order. A deck reordered in PowerPoint can come out in the original order, because presentation.xml is not read (document.rs:375).
- Speaker notes are never read. The notes parts are not opened.
- Tables are not structured. Cell text is flattened into the slide's text line, since every a:t is taken (document.rs:379). Not run against a table deck.
- Images, image alt text and charts are not read. The README lists no image output for PPTX.
- Paragraphs and line breaks inside a shape are lost. Each run is separated by a single space, so a whole slide is one line (document.rs:379, 66-91).
- Slides with no text are omitted, so the slide numbers in the headings can skip.
- No streaming writer. convert_streaming builds the whole result in memory first (document.rs:913-916).
- SmartArt, group shapes and chart parts are not opened. Only the slide part itself is read.

## RTF

Parses RTF into blocks (paragraphs, list items and nested tables), hyperlinks, footnotes, metadata and warnings, then renders Markdown or plain text from that model. The same module writes ASCII-only RTF from the model, and from the Markdown subset that its own renderer produces.

> **Not yet released.** The RTF reader, writer and Markdown-to-RTF converter are on an open pull request (superdocuments PR 23). Released builds use an older flat plain-text RTF converter that fails on unclosed groups.

### Reads

- **RTF documents.** The input must begin with {\rtf after optional spaces, tabs and line breaks. Anything else is an invalid-header error (superdocuments/src/rtf/reader.rs:144-162; model.rs:202-203).
- **Code pages.** A run's code page comes from its font's \cpg, then the font's \fcharset, then the document's \ansicpg. Windows and CJK code pages go through encoding_rs (Shift_JIS 932, GBK 936, EUC-KR 949, Big5 950 and others). Mac Roman has its own table, and UTF-8 sequences split across \'hh escapes are joined (charset.rs:90-139, 143-172; reader.rs:895-911).
- **Unicode escapes and symbols.** \uN is read as a signed 16-bit value. The following \ucN fallback characters are skipped. Surrogate pairs are joined. The Symbol font maps only the bullet and the ASCII punctuation range (reader.rs:849-876; charset.rs:20, 49-53).
- **Paragraphs, lists and tables.** Paragraph marks (\par), page and section breaks, \line and \tab are handled. A paragraph with a \listtext marker or a \ls value is a list item, with level from \ilvl (reader.rs:457-459, 559-562, 759-762, 773-784). Tables use \trowd, \cellx, \cell, \row, \nestcell and \nestrow, with \intbl and \itapN for nesting and \trhdr for header rows (reader.rs:785-816, 1049-1191).
- **Headings.** A paragraph is a heading when it has an outline level (\outlinelevel 0 to 8), when its style is named 'Heading N' or 'Title', or, with detect_headings on, when all its visible text is bold and at least 14 pt. At 18 pt or more the level is 1, and at 14 pt or more it is 2. The bold rule ignores text over 200 bytes (reader.rs:1200-1224, 1255-1297).
- **Hyperlinks and footnotes.** A HYPERLINK field keeps its target, and an \l anchor is appended as #anchor (reader.rs:1319-1366). A \footnote becomes a note body numbered in order, and the body gets a [^n] reference (reader.rs:612-623; render.rs:64-76).
- **Metadata and optional parts.** The \info group gives title, subject, author, keywords, comments, generator and creation and revision dates in YYYY-MM-DDTHH:MM (reader.rs:523-549, 1304-1316; model.rs:21-32). Headers and footers are read only when include_headers_footers is set (reader.rs:605-610; model.rs:176-197).

### Produces

- **RtfDocument.** A public struct with metadata, blocks in reading order, hyperlinks, footnotes, optional header and footer parts, and warnings. Blocks are Paragraph (runs, heading level, alignment, indents), ListItem (level, ordered flag, marker, runs) and Table (rows of cells, each cell holding blocks). Runs carry bold, italic, underline, strike, font size in half-points, a link index and a footnote number (model.rs:5-19, 70-132).
- **Markdown from rtf::to_markdown or convert_bytes('rtf').** Headings as #, bold as **, italic as *, strike as ~~, links as [text](url), lists as - or N., tables as pipe tables with the first row as header, and footnote references as [^n]. Markdown characters in text are escaped (render.rs:51-95, 164-254, 298-397).
- **Plain text from rtf::to_text.** Paragraphs and list items without markup. Table cells are joined with ' | ', or with tabs when TableStyle::Tabs is chosen (render.rs:100-148; model.rs:143-151).
- **ASCII RTF from rtf::to_rtf.** Output is ASCII only. cp1252 characters are written as \'hh, and other characters as \uN? with \uc1. The writer adds a font table, a stylesheet with heading styles 1 to 6, the info group, and footnote bodies written inline (writer.rs:1-5, 13-17, 41-87, 220-264).
- **RTF from Markdown via rtf::markdown_to_rtf.** Reads the Markdown subset that the renderer writes: ATX headings, paragraphs, bullet and numbered lists, pipe tables, bold, italic, strike, links and <br>. Code spans keep their text without backticks. Other syntax stays as literal text (markdown_in.rs:1-4, 11-92, 247-253).
- **Warnings.** Damage that does not stop the read (unbalanced groups, unknown code pages, truncated escapes) is kept in RtfDocument::warnings, up to 200 stored warnings (reader.rs:16-17; model.rs:63-68; mod.rs:7-9).

### How it works

1. **Check the header and bounds.** Skip leading whitespace and require {\rtf. The input must be within 128 MiB (reader.rs:144-158).
2. **Tokenise and keep a group stack.** The lexer yields control words, symbols, hex bytes and \bin payloads. The interpreter keeps a stack of groups, each with its destination, character state, font, paragraph state and hyperlink (lexer.rs; reader.rs:19-105, 241-270).
3. **Choose each group's destination.** Body text, skipped destinations (pictures, the colour and list tables, unknown starred groups), font table, stylesheet, info, field instruction, list marker, footnote or header/footer part (reader.rs:440-630).
4. **Decode text.** Each byte run goes through the group's code page. A multi-byte decoder holds partial sequences until the next byte or a flush (reader.rs:913-964; charset.rs:195-300).
5. **Close blocks.** A paragraph mark, a cell end or a row end makes a block and places it in the current table level. Nesting follows \itap (reader.rs:1049-1158, 1163-1191).
6. **Render or write.** Headings and lists are decided from the paragraph's properties (reader.rs:1200-1224). rtf::to_markdown and rtf::to_text render the model; rtf::to_rtf writes it (render.rs; writer.rs:20-33).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| The header must be {\rtf after optional whitespace; otherwise InvalidHeader | read from the code | superdocuments/src/rtf/reader.rs:144-158; model.rs:202-203 | read() skips spaces, tabs, CR and LF, then checks the prefix. |
| Defaults: 256 group levels, 128 MiB input, 128 MiB output; exceeding one is an error | read from the code | superdocuments/src/rtf/model.rs:155-172; reader.rs:146-151, 401-404, 1013-1017 | Input size and depth are checked directly; output size is checked in emit and push_run_with_style. |
| A run's code page is font cpg, then font fcharset, then the document ansicpg | read from the code | superdocuments/src/rtf/reader.rs:895-911 | code_page() resolves in that order and caches the result per font. |
| Unsupported code pages (for example 437 and 850) warn and fall back to cp1252 | read from the code | superdocuments/src/rtf/reader.rs:880-893; charset.rs:58-87 | resolve_number() picks the message from is_known_unsupported() or reports an unknown page. |
| The Symbol font decodes only the bullet and the ASCII punctuation range | read from the code | superdocuments/src/rtf/charset.rs:20, 49-53, 463 | Documented on the Kind::Symbol variant. Test symbol_font_bullet_and_unmapped at charset.rs:463, not run. |
| Headings come from outline level, the style name 'Heading N' or 'Title', or large bold text | read from the code | superdocuments/src/rtf/reader.rs:1200-1224, 1255-1297 | heading_from_style_name() and large_bold_level() implement the three rules. |
| Only HYPERLINK fields keep their instruction; other fields keep their result text | read from the code | superdocuments/src/rtf/reader.rs:448-456, 1319-1366 | parse_hyperlink() returns None for anything that is not HYPERLINK, so the field adds no link. |
| Markdown output escapes markup characters and writes a hard break for each newline | read from the code | superdocuments/src/rtf/render.rs:378-397 | escape_markdown() escapes backslash, backtick, asterisk, underscore, brackets, angle brackets and pipe, and writes backslash-newline for newline. |
| Writer output is ASCII: cp1252 as \'hh, other BMP characters as \uN? with \uc1, and it reads back with the same structure for every fixture | read from the code | superdocuments/src/rtf/writer.rs:1-5, 13-14, 230-264; superdocuments/src/rtf/tests.rs:513-525 (writer_output_is_ascii_and_reads_back_with_the_same_structure) | non_ascii() and cp1252_byte() implement the escapes; the preamble declares \uc1. The named test is not run and compares a subset of the structure. |
| markdown_to_rtf parses only the Markdown subset the renderer writes; other syntax stays literal | read from the code | superdocuments/src/rtf/markdown_in.rs:1-4, 11-92 | The module comment lists the supported syntax; unsupported text becomes a paragraph. |
| The branch replaces the flat RTF converter on main; main's convert_bytes arm is the old plain-text function | read from the code | superdocuments git history: a2dacab..0e8d51b, src/document.rs (commit 773115f) | git diff a2dacab HEAD shows the removal of fn rtf() and the new arm 'rtf' => crate::rtf::convert(data). The checkout is on branch ccr-7abbae09-xdtlfe. |
| Golden test: every fixture's Markdown equals its .expected.md | named test (not run for this document) | superdocuments/src/rtf/tests.rs:99-111 (golden_markdown_for_every_fixture) | Named test; not run. The FIXTURES list covers all 23 .rtf files in tests/fixtures/rtf. |
| A Markdown sample with headings, emphasis, a link, lists and a table round-trips to the same Markdown | named test (not run for this document) | superdocuments/src/rtf/tests.rs:553-560 (markdown_to_rtf_round_trips_through_markdown) | Named test; not run. One sample, compared as exact text. |
| Depth, input size and output limits are explicit errors; truncated, mutated and random inputs do not panic | named test (not run for this document) | superdocuments/src/rtf/tests.rs:438-480, 840-880 (exceeding_a_limit_is_an_explicit_error, truncated_and_mutated_fixtures_never_panic_or_hang, random_bytes_never_panic) | Named tests; not run. |

### Limits

- Status: the reader, writer and markdown_to_rtf are on the open superdocuments PR 23 (commits 773115f and 9477575). The rewrite is on an open pull request and is not merged into main. On main (a2dacab) RTF is a flat plain-text converter that fails on unclosed groups. The webfold pin of superdocuments (rev e0fb9a8) is not in the local clone, so what the webfold CLI does for .rtf was not verified.
- Dropped in Markdown output: colour (the colour table is skipped), font names (the font table keeps only charset and code page), underline (kept in the model, ignored by the renderer), and font size (kept in the model, used only for the heading rule). Right-to-left reordering is not implemented (CHANGELOG.md, RTF limits).
- Pictures are dropped: \pict, \shppict and object data are skipped. An \object shows only its \result (reader.rs:580-583; 503-511).
- Fields other than HYPERLINK keep their result text and lose the instruction (reader.rs:448-456).
- Numbering without \listtext is shown as a '-' bullet in Markdown and text output (render.rs:113-117, 224). Ordered numbers appear only when the document supplies a marker.
- The writer drops headers, footers and unreferenced footnotes, and writes every list with \ls1 and no list table (writer.rs:1-5, 132-147). A numbered list item with no stored marker is written with the marker '1.' (writer.rs:135-141).
- The whole document is held in memory. There is no streaming reader (CHANGELOG.md, RTF limits).
- Code pages that encoding_rs lacks (such as 437, 850, 1361) produce warnings and fall back to cp1252. Only the Symbol-font bullet is mapped (charset.rs:58-87; reader.rs:880-893).

## Spreadsheets

Reads workbook cells into Markdown tables, one section per sheet, and gives XLSX a structured Worksheet type with rows of text. Values are the stored text, so number formats and formulas are not applied. Legacy XLS is read only by the webfold adapter (calamine), not by the superdocuments crate, which returns an error for it.

### Reads

- **XLSX and XLSM.** A ZIP with xl/worksheets/sheetN.xml parts and an optional xl/sharedStrings.xml. Sheet parts are sorted by N (superdocuments/src/document.rs:481-514; xlsx.rs:157-192).
- **XLSB.** The binary workbook. Cell records are read for their cached values, and shared strings come from xl/sharedStrings.bin. Sheet names come from xl/workbook.bin when that part exists, matched to sheets by position (document.rs:741-846).
- **ODS.** content.xml only. Each table:table becomes one section, named by its table:name attribute (document.rs:552-628).
- **CSV.** UTF-8 (BOM removed) or UTF-16 with a byte-order mark. The delimiter is guessed from the first line among comma, semicolon and tab (document.rs:121-177). The webfold CLI and webfold's document module use their own CSV parser instead (webfold/src/document.rs:122-125).
- **Legacy XLS.** superdocuments returns an error for xls (document.rs:857). webfold reads it with calamine::Xls and writes one section per worksheet (webfold/src/document.rs:118-121, 170-190).
- **Worksheet values.** xlsx::read_workbook and xlsx::read_sheet return Worksheet values, with index in workbook order, name 'Sheet N' and cell text as rows (superdocuments/src/xlsx.rs:157-200; webfold/src/lib.rs:26-34).

### Produces

- **Markdown tables.** A '## ' heading for each sheet, then a pipe table whose first row is the header and whose rows are padded to the widest row. XLSX sections are named 'Sheet N' from the file number; XLSB, ODS and XLS use the real sheet names (document.rs:93-119, 505-512, 839-843).
- **Worksheet structs.** Worksheet { index, name, rows: Vec<Vec<String>> }. Rows are indexed by column position, with empty strings for gaps. Row numbers are not kept (lib.rs:26-34; xlsx.rs:102-153).
- **Streamed XLSX and CSV.** convert_streaming for XLSX and XLSM writes each sheet heading and row as the XML is read, with a separator after the first row (document.rs:978-1074). CSV streams through write_csv_rows, which pads rows to a common width (document.rs:920-976).
- **Lifted document (public type).** superdocuments also exposes LiftedDoc, which keys cells by (sheet, row, column) and keeps a row width for each row (document.rs:1275-1299, 1435-1537). webfold does not use it: its cache calls convert_bytes (webfold/src/document/cache.rs:100).
- **webfold table renderer.** CSV and XLS output in webfold uses its own renderer. Line breaks inside a cell become <br> and pipes are escaped. Rows are padded to the widest row only when (rows + 1) times the widest row is at most twice the total cell count plus the widest row. Otherwise the rows are written unpadded and no separator row is written (webfold/src/document/table.rs:4-83).

### How it works

1. **Dispatch by extension.** convert_bytes sends xlsx and xlsm to xlsx(), xlsb to xlsb(), ods to ods() and csv to the CSV path. It returns an error for xls (document.rs:850-869). webfold intercepts xls and csv before that call (webfold/src/document.rs:118-126).
2. **Bound the archive.** The ZIP is checked for entry count (4,096), declared entry size (32 MiB) and declared total (128 MiB) before any part is read (document.rs:12-14, 27-49; xlsx.rs:10-12, 25-47).
3. **Read shared strings.** xl/sharedStrings.xml is read into a list. A missing part gives an empty list (document.rs:483-486; xlsx.rs:159-162).
4. **Read sheet cells.** A row element starts and ends a row. Each c element takes its column from r. Its value is the shared string at its index when t is s, otherwise the stored v or inline t text (document.rs:428-479; xlsx.rs:102-153). Formula text in f is not read.
5. **Place and write.** Cells go into their column index, gaps are filled with empty strings, and each sheet is written as a Markdown table (document.rs:464-468, 505-512, 93-119).
6. **XLSB and ODS cell rules.** XLSB decodes cell records by type: RK numbers (scaled by 100 when the low bit is set), real numbers, booleans, strings and shared-string indexes. Error and blank records become empty (document.rs:789-831). ODS takes the office value attribute of each cell, then appends the text of its text:p children. A self-closing repeated cell is expanded up to 16,384 times (document.rs:577-650).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| XLSX sections are named 'Sheet N' from the sheet file number, not from workbook.xml | read from the code | superdocuments/src/xlsx.rs:179-187; superdocuments/src/document.rs:503, 511 | name: format!("Sheet {number}") with number parsed from xl/worksheets/sheetN.xml. |
| Shared strings are read from xl/sharedStrings.xml; a missing part gives an empty table | read from the code | superdocuments/src/document.rs:390-416, 483-486; superdocuments/src/xlsx.rs:64-90, 159-162 | shared_strings() collects the text of every t inside each si element. |
| Column index is capped at 16,384 (XFD); cells beyond the cap are placed differently by the two writers | read from the code | superdocuments/src/xlsx.rs:92-100, 138; superdocuments/src/document.rs:425, 1023 | column() returns None above the cap. The DOM path appends at the row end (row.len()). The streaming path uses column 0, so the cell overwrites column A. |
| The row r attribute is not read, and rows without c children are not emitted | read from the code | superdocuments/src/document.rs:439-472; superdocuments/src/xlsx.rs:113-146 | Rows are numbered only by their position in the sequence. An empty row element with no start or end tag never appears. |
| Formula text is not read; the cached v value is | read from the code | superdocuments/src/document.rs:445-449; superdocuments/src/xlsx.rs:119-124 | Only v and t children set the in-value flag, so f text is ignored. |
| Streaming XLSX: an empty self-closing c element is passed to read_cell_value, which keeps reading into the next cell | read from the code | superdocuments/src/document.rs:1021-1025, 1947-1950 | read_cell_value stops only at a matching v, t or is end, an Empty event, a c end or EOF. For Empty c it reads on, so the next cell's value can be taken and the next cell skipped. Not run; no test uses blank cells (document.rs:2169-2196). |
| Streaming XLSX: a shared-formula cell with a self-closing f before v loses its value, and rich-text inline strings keep only the first run | read from the code | superdocuments/src/document.rs:1947-1950, 1940-1946 | An Empty f element hits the Empty arm and returns an empty string; an inline string's first t end also returns. Not run. The lift path calls the same function (document.rs:1497-1503). |
| ODS repeat counts apply only to self-closing cells | read from the code | superdocuments/src/document.rs:577-584, 629-634 | number-columns-repeated is read only in the Empty arm, and the count is capped at 16,384 by odf_repeat. A repeated cell that has content is emitted once. |
| XLSB sheet names come from workbook.bin record 156, matched to sheets by position | read from the code | superdocuments/src/document.rs:745-754, 839-842 | names.get(number - 1), falling back to 'Sheet N'. |
| webfold reads legacy XLS through calamine::Xls and renders cells with calamine's Display | read from the code | webfold/src/document.rs:170-190; webfold/Cargo.toml:94-98, 221 | calamine is an optional dependency enabled by the default native feature. Data::Empty writes nothing. Merge data is not used. |
| No webfold test names XLS | named test (not run for this document) | webfold/tests/ (search for xls) and webfold/src/document.rs | A search found no XLS test in tests/ or in any unit test in src/document.rs. The superdocuments test for an XLSB sheet uses an empty sheet part (document.rs:2065-2066). |
| webfold's CSV renderer keeps line breaks as <br> and escapes pipes, and does not pad sparse tables | named test (not run for this document) | webfold/tests/document_review.rs:18-31, 49-59; webfold/src/document/table.rs:4-23, 43-49, 56-71 | Named tests multiline_cells_are_single_markdown_cells, lone_cr_is_a_record_boundary_and_limits_delimiter_detection and ragged_rows_do_not_expand_to_a_cartesian_grid. Not run. |
| On 45 public mixed-format fixtures, webfold's median conversion time is about 27.7 microseconds | measured | webfold-website/src/lib/marketing/benchmarks.ts:46 (slug documents, Measured 2026) | Uncached, mixed formats; spreadsheets are not reported separately, so this is not an XLSX figure. |
| A numeric ODS cell with a value attribute and display text is read once, not twice, on the PR 23 branch | named test (not run for this document) | superdocuments/src/document.rs, test ods_cell_with_a_value_attribute_and_display_text_is_not_doubled | Run on 2026-11-07: failed before the change with '\| 4242 \| Name \| 7.57.50 \|' and passes after it; the library suite is 375 passed, 0 failed, 1 ignored. Branch commit 1b340fa, not yet merged. |

### Limits

- XLSX sections are named 'Sheet N' from the sheet file number, not the tab name, and they follow file-number order, not tab order (xlsx.rs:187; document.rs:503, 511). The lib.rs comment says resolving workbook names is a follow-up.
- Values are stored text. No number or date format is applied, so a date appears as its serial number. Booleans appear as 0 or 1. Formulas show only their cached value (document.rs:453-463; xlsx.rs:127-137).
- Row numbers are not kept. Blank rows and gaps between rows are not reproduced in the Markdown or Worksheet output. Column positions are kept.
- Cells beyond column XFD (16,384) are placed at the end of the row by convert_bytes and read_sheet, but written into column A by convert_streaming (xlsx.rs:138; document.rs:1023).
- convert_streaming for XLSX is not verified. Rows are not padded to a common width, the separator takes the first row's width, and blank self-closing cells and some value shapes can take the wrong value (document.rs:1021-1066, 1947-1950). Use convert_bytes until these are tested.
- Before the fix on superdocuments PR 23 (commit 1b340fa), a numeric ODS cell that carried both an office:value attribute and its formatted text:p child came out twice: 42 as 4242, and 7.5 shown as 7.50 as 7.57.50. LibreOffice writes number cells that way, so any ODS saved by LibreOffice was affected. A cell that starts with a typed value is now taken from the attribute alone. That fix is on the unmerged branch, so released builds still have the defect. Covered (merged) cells are not read.
- CSV differs between the Rust API and the CLI. superdocuments treats a lone CR as data and leaves a newline in a quoted cell as raw text, which breaks the Markdown row. webfold treats CR as a row end and writes <br> (superdocuments document.rs:93-119, 193-198; webfold document/table.rs:44-49). CSV is still an error for an unterminated quote, with no size limit (ARCHITECTURE.md).
- Legacy XLS: superdocuments returns an error. The webfold adapter has no test in this repo and drops merge information. The adapter is at webfold/src/document.rs:118-121 and enabled by default (Cargo.toml:73, 94-98).

## OpenDocument

Reads OpenDocument text (ODT), presentation (ODP) and spreadsheet (ODS) files by parsing content.xml inside the ZIP package. Paragraphs and headings come out as Markdown text, and each ODS sheet becomes a Markdown table under its own heading. Lists, images, slide boundaries and document metadata are not returned.

### Reads

- **ODT and ODP (text and presentation).** One reader, odt() in src/document.rs, serves both extensions. It opens the ZIP and reads content.xml only, and keeps the text of text:p and text:h elements wherever they sit in the tree. styles.xml, meta.xml and settings.xml are never opened, so style-based headings, document properties and view settings do not reach the output.
- **ODS (spreadsheet).** Read by ods(). It walks table:table, table:table-row and table:table-cell in content.xml. A cell value comes from the first of office:string-value, office:value, office:date-value, office:time-value or office:boolean-value that is present, and then any text:p text inside the cell is appended to it.
- **Package limits.** archive() rejects a ZIP with more than 4,096 entries, any entry whose declared size is over 32 MiB, or more than 128 MiB declared in total. entry() applies the same 32 MiB cap to the bytes it reads from content.xml.
- **Not read.** Flat ODF (.fodt) is not routed: convert_bytes matches only odt, odp and ods. Encrypted packages are not decrypted, and the source has no encryption code. Legacy binary Office formats are outside this reader.
- **Entry points.** document::convert_bytes(extension, bytes) takes 'odt', 'odp' or 'ods' (case-insensitive). document::convert_path reads a file and dispatches on its extension. The webfold CLI routes .odt, .odp and .ods to the same code through webfold convert (superdocuments src/document.rs:849; webfold webfold/src/cli/document.rs:130).

### Produces

- **Markdown from ODT and ODP.** Paragraphs separated by one blank line. Each text:h is written as a line that starts with '## ', whatever its outline level. Text inside list items and table cells is written as plain paragraphs, because the reader does not look at the parent element. There are no bullets, numbers, table syntax, image references or slide markers.
- **Markdown tables from ODS.** One section per sheet, written as '## <sheet name>' followed by a pipe table. The first row of the sheet is the header row. Rows are padded to the widest row, pipe characters in cells are escaped, trailing empty cells and empty rows are dropped. Sheet names come from table:name, with newlines replaced by spaces.
- **One string, no metadata.** convert_bytes returns a single String, with sheets or paragraphs in document order. Title, author and dates are not returned. convert_streaming falls back to the same string for these extensions.
- **Errors.** A package with no content.xml returns 'missing content.xml'. An unknown extension returns 'unsupported document extension: <ext>'. Malformed XML returns the parser's message as a String error.

### How it works

1. **Check the package.** archive() reads the ZIP directory and rejects it if the entry count, any entry or the declared total is over its cap (src/document.rs:27).
2. **Read content.xml.** entry() reads the named file, capped at 32 MiB (src/document.rs:51). No other part of the package is opened.
3. **ODT and ODP.** The XML is streamed. Text collection starts at a text:p or text:h start tag. At the matching end tag the trimmed paragraph is written with a blank line after it, and text:h paragraphs get '## ' first. Empty paragraphs are skipped (odt, src/document.rs:516-549).
4. **ODS cells and rows.** The current sheet is taken from table:table. Each row collects cell values. Empty cells that carry a repeat count are expanded, up to 16,384 copies per element (odf_repeat, src/document.rs:629). A non-empty cell is read once, whatever its repeat count.
5. **ODS sheets.** At the end of a row, trailing empty cells are removed and the row is kept if anything is left. At the end of a sheet, '## <name>' and the Markdown table are written (ods, src/document.rs:552-625; markdown_table, :93).
6. **Join.** The sections are joined in document order and the trailing newline is trimmed.

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| ODT and ODP text is read from content.xml only, from text:p and text:h | read from the code | superdocuments/src/document.rs:516 | odt() matches Start events for text:p and text:h, collects Text events while active, and writes each paragraph on its End event. No other element is matched. |
| Every ODT heading is written as '## ' whatever its outline level | read from the code | superdocuments/src/document.rs:533 | The End arm writes '## ' only when the element is text:h. text:outline-level and styles.xml are not referenced anywhere in src/document.rs. |
| ODS sheets become '## <name>' sections followed by a Markdown table | read from the code | superdocuments/src/document.rs:611 | The table:table End arm writes the heading and markdown_table(rows). The name comes from table:name at :570. |
| Empty repeated cells are expanded up to 16,384 copies; non-empty repeated cells are read once | read from the code | superdocuments/src/document.rs:581 | Only the Empty table-cell arm calls odf_repeat (capped with .min(16_384) at :633). The Start arm pushes one cell at its End. |
| Package bounds are 4,096 entries, 32 MiB per entry and 128 MiB total | read from the code | superdocuments/src/document.rs:12 | MAX_ENTRY, MAX_TOTAL and MAX_ENTRIES constants at :12-14. archive() checks entry count and declared sizes at :27-49. |
| Spaces, tabs and line breaks in ODF text are not emitted | read from the code | superdocuments/src/document.rs:524 | text:s, text:tab and text:line-break are empty elements. The odt() match has no Empty arm, so they contribute no text. |
| ODS cell value attributes are read in a fixed order | read from the code | superdocuments/src/document.rs:636 | odf_cell_value returns the first present attribute of office:string-value, office:value, office:date-value, office:time-value, office:boolean-value. |
| convert_bytes routes odt and odp to odt(), ods to ods(), and rejects other extensions | read from the code | superdocuments/src/document.rs:849 | The dispatch matches odt \| odp to odt(), ods to ods(). fodt is not in the match. Unknown extensions return 'unsupported document extension'. |
| webfold convert sends odt, ods and odp to superdocuments | read from the code | webfold/src/cli/document.rs:130 | markdown_source tests the extension list and calls webfold::document::convert_path, which calls superdocuments::document::convert_bytes (webfold src/document.rs:126). The CLI usage is in webfold/src/main.rs:214. |
| The ODF test covers the heading, paragraph, sheet and presentation cases | named test (not run for this document) | superdocuments/src/document.rs:2025 | office_packages_convert_through_public_api asserts that odt gives '## Title\n\nBody', that an ods cell of 42 gives a row containing '\| 42 \|', and that odp contains 'Body'. Not run. |
| The sparse-column spreadsheet test covers empty-cell repeats | named test (not run for this document) | superdocuments/src/document.rs:2016 | spreadsheet_cells_preserve_sparse_columns. Named in the repo; not run. |
| A numeric ODS cell with a value attribute and display text is read once, not twice, on the PR 23 branch | named test (not run for this document) | superdocuments/src/document.rs, test ods_cell_with_a_value_attribute_and_display_text_is_not_doubled | Run on 2026-11-07: failed before the change with '\| 4242 \| Name \| 7.57.50 \|' and passes after it; the library suite is 375 passed, 0 failed, 1 ignored. Branch commit 1b340fa, not yet merged. |

### Limits

- Headings are flattened. Every text:h becomes '## ', whatever its outline level, because the reader does not read text:outline-level or styles.xml (src/document.rs:533).
- Before the fix on superdocuments PR 23 (commit 1b340fa), a numeric ODS cell that carried both an office:value attribute and its formatted text:p child came out twice: 42 as 4242, and 7.5 shown as 7.50 as 7.57.50. LibreOffice writes number cells that way, so any ODS saved by LibreOffice was affected. A cell that starts with a typed value is now taken from the attribute alone. That fix is on the unmerged branch, so released builds still have the defect. Covered (merged) cells are not read.
- No lists, tables or images inside text documents. List items and table cells come out as plain paragraphs. draw:frame images produce no reference.
- Spacing can be lost. text:s, text:tab and text:line-break produce no text, so words separated only by those elements join together (src/document.rs:524-546).
- ODP slides are not separated. The reader does not match draw:page, so slide text comes out as one stream in document order. Speaker-notes text is included the same way, because text:p is matched wherever it appears.
- ODS rows and merged cells. A row marked table:number-rows-repeated is written once. Merged cells (table:covered-table-cell) are not matched, so a row with a merged cell comes out shorter and later cells can move left. Both come from reading the code and have not been run.
- Only content.xml is read. Comments, tracked changes, metadata and header or footer text in styles.xml are not returned.
- Flat ODF (.fodt) is not routed, and encrypted packages are not decrypted.

## EPUB

Reads EPUB files, which are ZIP packages of XHTML chapters. The spine reader (epub::read and epub::extract_text) follows the OPF reading order and returns one Markdown string. The convert route used by webfold convert ignores the package structure and converts every .xhtml or .html entry in file-name order, and images, styles, fonts and DRM are not handled.

### Reads

- **EPUB packages (container and OPF).** epub::read needs META-INF/container.xml with a rootfile full-path, then the OPF package it names. It reads manifest items (id, href, media-type), spine itemrefs and the OPF title. The version field is not checked, so EPUB 2 and EPUB 3 packages go through the same parser. It returns an error if the container or the OPF is not UTF-8.
- **Spine order, including non-linear items.** The spine is the list of itemref elements in order. The linear attribute is not read, so items marked linear="no" (often notes, covers or supplementary pages) are included in the walk.
- **Routes that use the spine.** epub::extract_text, and webfold fetch <url> --dump text when the response is an EPUB or an HTML page that links to one (cli/epub_walk.rs:260).
- **Route that does not use the spine.** document::convert_bytes('epub'), which webfold convert book.epub uses. It does not call epub::read. It takes every entry whose name ends in .xhtml or .html and sorts the names as strings.
- **Size bounds.** read() refuses input over 256 MiB, an archive with more than 8,192 entries, or an entry over 64 MiB. extract_text() reads each chapter with a 16 MiB cap and stops with an error if the rendered text passes 64 MiB.

### Produces

- **Markdown from the spine walk.** One string. Each chapter is converted with html::to_markdown and joined to the next by a blank line, in spine order. The base URL is empty, so relative links are not resolved.
- **Markdown from the convert route.** One string, with the same blank-line join. Each chapter is converted with html::to_blocks and html::render using the base epub:///<entry name>. Chapters with no text are skipped. Entries are in file-name order, so a cover or navigation page appears wherever its name sorts.
- **Book values.** Book holds the manifest (id, href, media-type), the spine (id, href) in order, the OPF path and the title. title() returns the OPF title, spine() returns the resolved spine, and manifest_get looks an item up by id.
- **Image references.** An img element becomes ![alt](src), with src resolved against the base that was passed in. Image bytes are not returned, and CSS and fonts are not read.
- **Errors instead of partial output.** extract_text returns Err with no text when a spine entry is missing, a chapter is over 16 MiB, or the total passes 64 MiB. A broken spine idref is skipped with a message on stderr and the rest of the book is read.

### How it works

1. **Container.** Read META-INF/container.xml and take the first rootfile full-path as the OPF path (epub.rs:126; parse_container at :243).
2. **Package.** Read the OPF and build the manifest keyed by id, the spine idrefs in order, and the title (parse_package at :272).
3. **Resolve the spine.** Each spine idref is resolved to a path under the OPF directory (join_href at :334). An idref that is not in the manifest is skipped with a stderr message (epub.rs:161).
4. **Spine walk.** extract_text reads each spine entry, capped at 16 MiB, converts it with html::to_markdown, and appends it in spine order (epub.rs:181-205).
5. **Convert route.** The container and OPF are skipped. Entries ending in .xhtml or .html are listed, the names are sorted, and each is converted with html::to_blocks and html::render (src/document.rs:651-675).
6. **fetch --dump text.** The fetch command checks whether the response is an EPUB, or points at one, and uses the spine walk (webfold/src/cli/epub_walk.rs:260-265; webfold/src/main.rs:387-392).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| epub::read parses the container, the OPF, the manifest and the spine, and skips broken idrefs | read from the code | superdocuments/src/epub.rs:126 | read() runs parse_container, parse_package, then resolves each spine idref against the manifest. A missing idref is logged with eprintln (:161) and skipped. |
| extract_text walks the spine in order and refuses partial output | read from the code | superdocuments/src/epub.rs:181 | Each spine href is read with a 16 MiB cap, converted with to_markdown(&entry, ""), and appended. Exceeding MAX_TEXT_BYTES returns an error. |
| The OPF parser reads item, itemref and title elements only; linear is not read | read from the code | superdocuments/src/epub.rs:272 | parse_package matches item, itemref and title by local name. A search of superdocuments/src/epub.rs for 'linear' finds nothing. |
| The convert route does not use the spine: it sorts .xhtml and .html names | read from the code | superdocuments/src/document.rs:651 | epub() lists entries ending in .xhtml or .html, sorts the names, converts each with html::to_blocks (base epub:///name) and html::render, and joins them. It does not call epub::read. |
| fetch --dump text uses the spine walk for EPUB bodies | read from the code | webfold/src/cli/epub_walk.rs:260 | try_epub_walk calls superdocuments::epub::read and extract_text for an EPUB body, and for HTML that points at one. |
| The fetch command calls the EPUB walk for dump text or markdown | read from the code | webfold/src/main.rs:387 | If --dump is text or markdown, the body is read and try_epub_walk is tried before the normal HTML path. |
| EPUB size bounds are 256 MiB, 8,192 entries, 64 MiB per entry, 16 MiB per chapter, 64 MiB of text | read from the code | superdocuments/src/epub.rs:92 | MAX_ENTRY, MAX_TOTAL, MAX_ENTRIES, MAX_XHTML_BYTES and MAX_TEXT_BYTES constants. |
| img elements become ![alt](src) with the src resolved against the base | read from the code | superdocuments/src/html/extract.rs:425 | The img arm writes ![alt](resolve(base, src)) and skips data: URIs. resolve() returns absolute and empty references unchanged. |
| The EPUB reader has no panic containment and no encryption handling | read from the code | superdocuments/src/epub.rs:1 | A grep of superdocuments/src/epub.rs and src/document.rs finds no catch_unwind and no reference to encryption.xml. |
| Named unit test: manifest and spine order | named test (not run for this document) | superdocuments/src/epub.rs:418 | read_parses_manifest_and_spine_in_order. Not run. |
| Named unit test: spine walk order | named test (not run for this document) | superdocuments/src/epub.rs:439 | extract_text_walks_spine_in_order, a two-chapter fixture built in the test. Not run. |
| Named unit test: broken idref is skipped | named test (not run for this document) | superdocuments/src/epub.rs:460 | spine_with_broken_idref_is_skipped_not_panicked. Not run. |
| Named unit test: missing spine entry is an error | named test (not run for this document) | superdocuments/src/epub.rs:490 | extract_text_refuses_a_missing_spine_entry. Not run. |

### Limits

- Two reading orders. webfold convert follows sorted file names (src/document.rs:651). epub::extract_text and fetch --dump text follow the spine (superdocuments/src/epub.rs:181). Where file names do not match the reading order, the two outputs differ.
- Non-linear spine items are included. The linear attribute is not read (superdocuments/src/epub.rs:272).
- No DRM or encryption handling. META-INF/encryption.xml is not read and nothing decrypts content. A protected book is read as the bytes stored in its chapter files. No protected file was tested.
- Media is not returned. Image bytes, CSS and fonts are not read. An image appears only as ![alt](src).
- No table of contents. Navigation and NCX files are not parsed for labels. The convert route reads a navigation file as ordinary XHTML if its name ends in .xhtml or .html.
- Hard caps: 256 MiB per EPUB, 8,192 entries, 64 MiB per entry, 16 MiB per chapter and 64 MiB of rendered text. A book over the text cap fails as a whole.
- No panic containment in the epub or document modules. docs/ARCHITECTURE.md says so (line 68), and a grep finds no catch_unwind in either file.
- Broken spine references are logged to stderr and skipped, so a caller that checks only the return value does not see them (superdocuments/src/epub.rs:161).

## Scientific sources

superdocuments has readers for LaTeX source, BibTeX files, arXiv e-print archives (a paper's source tarball) and MathML, returning sections, math kept as TeX, figures with image bytes, and bibliography entries. The webfold CLI and MCP server do not call these readers; webfold arxiv get reads the arXiv HTML version of a paper, and the PDF when there is no HTML.

> **Not yet released.** The LaTeX, BibTeX, arXiv source and MathML readers are on an open pull request (superdocuments PR 23) and are not reachable from the Webfold command line or MCP server today. Webfold's arXiv tool uses its own HTML route.

### Reads

- **LaTeX source (.tex).** latex::parse reads a document or fragment from a string. It is a bounded reader, not a TeX engine, and it loads no packages. \documentclass and \usepackage are dropped with no warning. Commands that a package would add are unknown: they keep their arguments and produce a warning. Macros defined in the file with \newcommand, \renewcommand, \providecommand, \def, \DeclareMathOperator, \newenvironment or \newtheorem are expanded in running text, within limits on the number of expansions, their total size and nesting depth.
- **Multi-file projects (\input, \include, \subfile).** These are read only through a FileProvider. Without one, the name is recorded in Document::inputs and a warning is added. arxiv_source supplies a provider that reads from the archive only. It tries the name relative to the main file's folder, then as written, and adds .tex when the name has no .tex ending.
- **arXiv e-print (the bytes served at /e-print/<id>).** read_eprint takes the bytes of the archive. It accepts a gzip-compressed tar, a plain tar, or a gzip-compressed single TeX file, which is named main.tex. The module does no network or file-system access. It chooses the main file, parses it, inlines \input files from the archive, builds a bibliography and resolves each figure's images to archive bytes.
- **BibTeX and BibLaTeX (.bib).** bibtex::parse reads a string into entries, @string, @preamble and @comment blocks, with warnings. Helpers read names, pages, years, months, DOIs, URLs and arXiv ids from field values. resolved_value follows crossref up to eight levels, and a cycle stops the search.
- **MathML (<math> elements).** mathml::to_tex takes a string containing a <math> element. If it holds a TeX annotation, that TeX is returned unchanged. Otherwise the presentation markup is converted: identifiers, numbers, operators, fractions, roots, scripts, tables and fences.
- **PDF with no source (text and embedded images).** pdf::extract_markdown returns the text as Markdown. superdocuments::pdf::images::extract_images lists each embedded image XObject per page with its bytes: JPEG and JPX unchanged, Flate images as PNG, other decodable images as raw samples. An image's position in the text appears as a marker, ![](img_<object number>.png).

### Produces

- **A structured LaTeX document.** latex::Document holds the title and authors (one entry per \and), date, abstract, a block list (Section at levels 1 to 5, Paragraph, List, Table, Math, Verbatim, Figure, Theorem, Quote), labels with their numbers, thebibliography items, names from \bibliography and \addbibresource, unresolved \input names, and warnings.
- **Markdown, text and HTML.** render_markdown writes headings as # repeated to the section level, inline math as $...$ and display math as $$ blocks. A numbered display row gets \tag{n}. Citations are text such as (Vaswani et al., 2017), not links. A figure becomes a bold 'Figure N:' caption and one line per image, ![](path), with the path as written in the source. render_text and render_html give the same content in their formats.
- **Figures with image bytes (read_eprint).** Source.figures has one FigureAssets per top-level figure environment, including figure* and subfigures, in source order. Each has its label, number and caption text, and one ImageAsset per \includegraphics. The asset gives the declared path and, when found, the archive path, the bytes and a format: Png, Jpeg, Pdf, Eps or Other, detected from the first bytes. A missing image is recorded as None and a warning is added.
- **Bibliography from an e-print.** The bibliography comes from the archive's .bib files. If there are none, it comes from thebibliography items in the main file, and if there are none of those, from .bbl files. Items from thebibliography become misc entries with their label, raw source and plain text.
- **BibTeX and CSL-JSON output.** bibtex::write produces BibTeX text: @string, @preamble and @comment blocks first, then the entries with their fields. Field values are written from the expanded text, so macro names are not kept. csl_json returns a CSL-JSON array with crossref inheritance applied.
- **MathML as TeX, with a flag.** to_tex returns a Converted value: the TeX string, from_annotation (true when the TeX came from the annotation), and warnings. An unsupported MathML element produces a warning and its content is kept.

### How it works

1. **Lex.** The lexer reads control words, control symbols, text, groups and math delimiters. Comments are removed, and \verb takes any delimiter (superdocuments/src/latex/lexer.rs).
2. **Parse with a stack of sources.** The main document is at the bottom of a stack. Macro expansions and \input files are pushed on top and read in place. Math, verbatim bodies and file paths are read as raw text (superdocuments/src/latex/parser.rs:99).
3. **Number and label as it goes.** The reader counts sections, figures, tables, equations and theorems, and records each \label with its number (push_label, superdocuments/src/latex/parser.rs:224). References are resolved by the renderer after the whole document is read, so a forward reference works (docs/ARCHITECTURE.md, lines 111-116).
4. **Choose and read the main file of an e-print.** select_main prefers a .tex with both \documentclass and \begin{document}, then one with only \begin{document}, then one with only \documentclass, then the largest .tex. read_eprint_with then parses it, decoding a non-UTF-8 file as Latin-1 with a warning (superdocuments/src/arxiv_source/mod.rs:123-165).
5. **Resolve figure images inside the archive.** For each \includegraphics the candidates are tried in order: .pdf, .png, .jpg, .jpeg, .eps, then the path as written. A path that already ends in one of those extensions gets only itself (image_candidates, superdocuments/src/latex/parser.rs:2070). A path with another ending, such as .svg, gets the extension list first, then itself.
6. **Render, and report problems.** The renderers write Markdown, text or HTML from the same Document (superdocuments/src/latex/render.rs). Archive errors stop the read with no partial result. Parser problems are kept in Document.warnings, up to 10,000 entries (latex::Options.max_warnings).

### Evidence

| Claim | How we know | Where | Detail |
|---|---|---|---|
| read_eprint takes bytes and runs unpack, main-file selection, parsing, bibliography and figure resolution, with no I/O | read from the code | superdocuments/src/arxiv_source/mod.rs:123 | read_eprint_with unpacks the data, selects the main file, parses it with an archive-backed FileProvider, collects bibliography and figures, and returns warnings. A grep of src/arxiv_source for std::net, reqwest and std::fs finds only a test helper that reads a fixture. |
| The archive unpacker accepts gzip tar, plain tar or a single gzip TeX file, and refuses unsafe paths and links | read from the code | superdocuments/src/arxiv_source/archive.rs:81 | unpack() returns the tar's regular files, or main.tex for a single gzip file, or NotAnArchive. read_tar refuses hard and symbolic links (:239). validate_path refuses absolute paths, .., backslashes and empty names (:297-317). |
| \input and \include resolve through a FileProvider, with the archive lookup relative to the main file | read from the code | superdocuments/src/latex/parser.rs:1087 | include() asks the provider for the name, then the name with .tex. Without a provider it records the name in inputs and warns. arxiv_source/mod.rs find_entry tries base plus name, then name as written. |
| Macros expand in text; \usepackage is dropped silently; unknown commands warn and keep their arguments | read from the code | superdocuments/src/latex/parser.rs:715 | newcommand, renewcommand, providecommand, DeclareMathOperator are handled at :715-723. The unknown-command warning is at :850. drop_spec (:2084) drops usepackage, documentclass and others without a warning. |
| Math is written as $...$ and $$ blocks, figures as captions and ![](declared), citations as text | read from the code | superdocuments/src/latex/render.rs:185 | inline_math and display_math (:185-222) write $ and $$ with \tag for numbered rows. image (:244) writes ![](p.declared), the path as written. cite (:323) writes (label) text from thebibliography or a supplied Bibliography, else the key. |
| BibTeX write emits expanded field values; undefined macros warn; duplicate keys keep both entries | read from the code | superdocuments/src/bibtex/write.rs:18 | write() prints each field as its raw (expanded) value. parse.rs:303 warns on an undefined macro, parse.rs:207 on a duplicate key, and parse.rs:263 keeps the first of repeated fields. |
| MathML returns an annotation's TeX when one exists, and keeps unknown element content with a warning | read from the code | superdocuments/src/mathml.rs:85 | to_tex calls tex_annotation (:163), which accepts an annotation under semantics whose encoding contains 'tex'. Unknown elements produce 'unsupported MathML element' (:718) and keep their children. |
| PDF images are listed with bytes; the text output marks each image as an object-numbered marker | read from the code | superdocuments/src/pdf/images.rs:54 | extract_images returns page, object number, size, colour space, filters and encoding, with JPEG and JPX unchanged and Flate as stored-deflate PNG. The text path writes ![](img_{obj}.png) at superdocuments/src/pdf/parser/parse.rs:971, using the object number, not a sequence index. |
| webfold arxiv get reads arXiv HTML first and the PDF on a 404; it does not fetch the e-print | read from the code | webfold/src/arxiv/mod.rs:264 | retrieve() gets /html/<id>. On a 404 it gets /pdf/<id> and calls document::convert_bytes("pdf") (:280). The src/arxiv tree has no e-print or tar code. |
| The webfold PDF fallback has no pdf arm in convert_bytes, so it returns an unsupported-extension error | read from the code | superdocuments/src/document.rs:849 | superdocuments convert_bytes has no pdf case, so the error is 'unsupported document extension: pdf'. The webfold wrapper (src/document.rs:126) adds only xls and csv before delegating. |
| webfold uses superdocuments for document, pdf, epub and graph only; not latex, bibtex, arxiv_source or mathml | read from the code | webfold/src/cli/document.rs:260 | The superdocuments paths used in webfold are document::convert_bytes and convert_path, pdf::extract_markdown and related pdf functions, epub::read and extract_text, and graph. A grep of webfold src for latex::, bibtex::, arxiv_source:: and mathml:: finds no use outside its own arxiv code. |
| Named test: an archive gives its main file, document, bibliography and figures | named test (not run for this document) | superdocuments/src/arxiv_source/tests.rs:36 | paper_archive_gives_main_file_document_bibliography_and_figures checks the \input inlining, bibliography keys, figure label and format, and EPS detection. Not run. |
| Measured: pandoc 3.1.3 agrees on 16 LaTeX fixtures except figure*; pandoc fails on three of five BibTeX files | measured | superdocuments/benches/latex-vs-pandoc/results.json | 20 runs per timing. figures.tex differs on figures_top_level (ours 4, pandoc 5) and figures_with_caption (ours 6, pandoc 5). pandoc fails on basic.bib (unexpected '('), malformed.bib and crossref-biblatex.bib (out of memory on a crossref cycle). The file has no date; disagreements.txt and the comparison matrix date from 2026-11-06. Hand-made fixtures. |
| Measured: PDF image listing agrees with poppler pdfimages on 7 test images | measured | superdocuments/benches/pdf-images/results-images-mixed.txt | 6 images in images-mixed.pdf and 1 in inherited-predictor.pdf, with 0 disagreements against pdfimages (results-inherited-predictor.txt). The files carry no date. |

### Limits

- Not a TeX engine, and packages are not loaded. natbib, cleveref, siunitx, algorithmic, hyperref and custom .sty files are not read. Their commands come out as unknown-command warnings with their arguments kept as text. The local comparison notes say pandoc covers many more packages and macros (benches/latex-vs-pandoc/explanations.txt).
- Math is kept as TeX, not rendered, and not checked. The reader computes equation numbers from the environment and does not run TeX (superdocuments/src/latex/mod.rs, lines 1-20). A PDF gives no TeX at all: equations come out as the glyphs the page draws, and no module in src/pdf refers to LaTeX, MathML or tex_text.
- Figures are partial. A PDF's embedded images appear only as ![](img_<object>.png) markers with no captions linked: graph.rs defines Figure and Caption region kinds, but no builder assigns them. A tikzpicture is kept as its source and not drawn (superdocuments/src/latex/render.rs:511). Markdown writes the declared path, not the resolved file, so a path given without an extension does not match the file on disk (superdocuments/src/latex/render.rs:244).
- Citations are text. A \cite key resolves first to a thebibliography item, then to a supplied Bibliography, and is shown as text such as (Author, Year). With neither, the key itself is shown. Nothing is a hyperlink (superdocuments/src/latex/render.rs:323-368; superdocuments/src/latex/mod.rs:378).
- A BibTeX round trip changes the source. write() emits expanded field values, so @string names and # joins are not kept, and layout changes. Duplicate keys are both kept with a warning, and a repeated field keeps its first value (superdocuments/src/bibtex/write.rs:18; superdocuments/src/bibtex/parse.rs:207, :263).
- Webfold does not expose these readers. The CLI and MCP server do not call them. These modules are on the open superdocuments PR 23, and webfold pins superdocuments to revision e0fb9a8, which was not checked against them (webfold Cargo.toml:202; docs/competitors/COMPARISON-MATRIX-2026-11.md, section 1).
- webfold arxiv get fails for papers with no HTML rendition in this checkout. The PDF fallback calls convert_bytes("pdf"), which has no pdf arm, so the call returns an unsupported-extension error (webfold/src/arxiv/mod.rs:280; src/document.rs:126). The e-print is never used.
- arXiv HTML formula cleanup changes TeX. finish() restores formulas and then runs clean_latex_artifacts over the whole text, which turns \\ into \ and collapses double spaces inside formulas as well as prose (webfold/src/arxiv/html.rs:30-32 and :149-152). Read from code and not run; the unit test covers prose residue only.
