# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]


### Added

- `pdf::page_profile` (`page_ocr_verdicts`, `pages_needing_ocr`, `classify_page`), `Document::page_geometry` and `pdf::extract_bytes_with_ocr_options`: OCR is decided per page. A page with no alphanumeric text, or a scan whose only text is a stamp (images cover at least half the page and fewer than 0.4 alphanumeric characters per 1000 pt²), goes to OCR; the other pages of the same file keep their native text.
- Legacy Word binary (`.doc`, `.dot`): `document::convert_bytes("doc", ..)` reads the main story's text natively (`src/doc.rs`), with named errors for encrypted files and non-Word content. `DocFormat::Doc`.
- Spreadsheet number formats (`src/ooxml/numfmt.rs`) and `xlsx::read_workbook_typed` (`CellValue`, `TypedCell`, `TypedWorksheet`): cells show what their number format shows; typed values stay available.

- Legacy `.xls` (BIFF) conversion through the in-repo `calamine/` crate, now a path dependency of the root crate (default features off). The output has the same Markdown shape as XLSX: one `## <sheet>` section per worksheet in tab order, error cells, booleans and formula results. `DocFormat::Xls` goes through `DocumentCache` and `convert_streaming`.

- `document::extract_images` and `ExtractedImage` (`src/media.rs`): the pictures of DOCX, DOCM, PPTX, XLSX, XLSM, ODT, ODP, ODS and EPUB, with name, media type (from magic bytes), header size for PNG, JPEG, GIF and BMP, first referencing part and alt text. Limits and zip safety are described in the function's doc comment.
- DOCX and PPTX Markdown writes each picture as `![alt](media/<name>)` where its drawing is. The alt text is the drawing's `descr`; a drawing without a picture still gets its description as text, as before.

### Added (Pandoc compatibility)

- `src/citeproc/`: a CSL citation processor with Pandoc's citation syntax (see `docs/CITEPROC.md`); `bibtex::csl_json_pandoc` and `bibtex::looks_like_biblatex`.
- `src/rst.rs`: a reStructuredText reader (`.rst`), on the shared tree in `src/ast.rs`.
- `src/ooxml/omml.rs`: Office Math to TeX in DOCX; DOCX block quotes, code blocks, strike, super/subscript, internal links, list start and numbering style, and Title/Author/Date as YAML metadata.
- `latex::RenderOptions::pandoc` (used by `convert_bytes`): LaTeX as Pandoc reads it (YAML metadata, heading levels, `[@key]` citations, reference links, definition lists, table and figure captions, super/subscript, theorem layout). `latex::Section::division`, `latex::Inline::{Sup, Sub, Strike}`.
- `benches/pandoc-compat`: the benchmark (README there), `tests/pandoc_compat_golden.rs`.

### Changed (Pandoc compatibility)

- `convert_bytes` for `.tex` now writes the Pandoc layout above (a document title is YAML metadata, not a level-1 heading; sections start at level 1). Callers of `latex::render_markdown` are unchanged (`RenderOptions::pandoc` is off by default).
- HTML and EPUB keep footnote back-links as links in HTML, drop the EPUB title page, and read `<span class="math">` as math only when it holds TeX.

### Changed

- Code: Clippy with warnings denied passes on Rust 1.89 (38 findings: dead `&& false` conditions removed, identical branches merged, three unused functions removed, a few char patterns and collapsible `if`s). No other behaviour changes.
- Minimum supported Rust version is 1.89 (`rust-version`, CI, release and supply-chain workflows). The git dependencies `apeiron` and `wirefold` require 1.89, `calamine`, `zip` and `encoding_rs` require 1.88, so the previous 1.85 pin could not build.

- The root crate depends on `calamine` (path). The lockfile gains that one edge; no new crates.

### Fixed

- Citations in Pandoc Markdown (`citeproc::markdown::parse_citations`): an indented code block (four spaces or a tab after a blank line, outside lists, definitions, footnotes and quotes) holds no citations, and `[@a][ref]` is a reference link whose text is the in-text citation `@a`, as `pandoc --citeproc` 3.1.3 reads them. Both were written in the parser behind a condition that was always false and so never took effect.
- XLSX: a stored double is shown to 15 significant digits like Excel (`4.0999999999999996` is `4.1`, `0.30000000000000004` is `0.3`), and dates with a custom format show as formatted (`Sep-2026`, not `2026-09-01`). This changes the Markdown for such cells; the stored text is in `CellValue::Number::raw`.
- DOCX: an embedded picture object (`Word.Picture.8`, `Paint.Picture`, `PBrush`) no longer leaves an `Embedded object: Word.Picture.8` line; other embedded objects keep it.
- PDF: a scanned page carrying only a stamp is OCRed instead of returning the stamp (see Added).
- PDF native extraction: a page whose `/Contents` is an indirect reference to an array of content streams (`/Contents 13 0 R`, `13 0 obj [14 0 R 15 0 R] endobj`, as Aspose.Pdf 10.1.0 writes it) now yields its text. Before, such pages read as empty and callers fell back to OCR. On the 60 PDFs of the Federal Register's "Census and Statistics" titles, 34 returned under half of poppler's words before the fix and none after. Regression tests: `tests/pdf_contents_indirect_array.rs`.
- `calamine` CFB reader: a truncated sector, a sector past the end of the file, a short directory entry, a header-driven allocation larger than the file, and a FAT chain that cycles or leaves the FAT now give errors instead of panics, aborts or endless loops. A BIFF dimensions record no longer reserves gigabytes up front.

## [0.9.1] - 2026-11-13

### Changed

- `wirefold` and `apeiron` move to `98df8bfb8051` and `1c3d4a1da565`, the revisions the rest of the webfold graph (sangdb, transport, relayfold, servelet) is built on, so a consumer resolves one copy of each. No API or output change in this crate.

## [0.9.0] - 2026-11-09

### Added

- `src/ooxml/`: shared OOXML helpers (local-name matching, relationship reading, package-safe path resolution) and new DOCX, PPTX and XLSX readers. DOCX: headings from styles, lists from `numbering.xml`, bold and italic, hyperlinks, footnotes and endnotes, tracked deletions omitted, nested tables, merged cells. PPTX: presentation order, notes, tables. XLSX: workbook sheet names in tab order, dates from number formats, booleans, rich and inline strings.
- RTF: a merged cell written through `\cellx` edges keeps its columns; an outline-numbered heading stays a heading.

### Changed

- `convert_streaming` and `DocumentCache` use one implementation per format for DOCX, PPTX and XLSX (the separate streaming and cache-lift code for DOCX and XLSX was removed; `LiftedDoc` is CSV-only).
- `hashtrinity` is pinned to a revision that exists upstream.
- Version 0.9.0, so a dependent can pin `tag = "v0.9.0"`; a tag keeps the commit reachable, which a revision on a branch does not once the branch is squash-merged.

### Fixed

- Every XML reader dropped the character an entity reference stands for (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, numeric references).


### Added

- RTF reading and writing in `superdocuments::rtf`. `rtf::parse` returns an `RtfDocument` (metadata, paragraphs, list items, nested tables, hyperlinks, footnotes, optional headers and footers, warnings). `rtf::to_text` and `rtf::to_markdown` render it. `rtf::to_rtf` and `rtf::markdown_to_rtf` write ASCII-only RTF.
- RTF code pages: `\ansicpg`, `\fcharset`, per-font `\cpg`, Mac Roman, the Symbol-font bullet, and double-byte or UTF-8 sequences split across `\'hh` escapes.

RTF limits: input and output are each limited to 128 MiB, and group nesting to 256 levels. These are `rtf::Limits`, and exceeding one is an error. The reader keeps the whole document in memory and does not stream. Not supported: underline and colour in Markdown, font names, pictures, revision marks, fields other than `HYPERLINK`, right-to-left reordering, and code pages that `encoding_rs` does not provide.

- LaTeX reading in `superdocuments::latex`: a bounded reader (not a TeX engine) with macro expansion under limits, sections and numbering, lists, tables, math kept as TeX, figures (captions, labels, `\includegraphics` candidates, subfigures, opaque `tikzpicture`), citations linked to `thebibliography` or a supplied `Bibliography`, and text, Markdown and HTML renderers. `\input` is read through a `FileProvider`.
- BibTeX and BibLaTeX in `superdocuments::bibtex`: reader with warnings, typed names, pages, dates, DOI and URL parsing, a round-tripping writer, and CSL-JSON export. `crossref` is followed with a cycle stop.
- `superdocuments::tex_text`: TeX accents, letter commands and symbols to Unicode, shared by the BibTeX and LaTeX readers.
- `superdocuments::arxiv_source::read_eprint`: reads an arXiv e-print from bytes, with archive safety limits (path, link, CRC, size, entry count and compression ratio), main-file selection, `\input` inlining from the archive, bibliography collection, and figure image bytes with magic-number format detection. No network access.
- `superdocuments::mathml::to_tex`: MathML to TeX for the common presentation elements, using the TeX annotation when present.
- `superdocuments::pdf::extract_images`: lists embedded image XObjects per page. JPEG data is returned unchanged; 8-bit gray and RGB Flate images are written as PNG (stored deflate, uncompressed). Page rasterisation is not implemented.

Measured, not claimed: the BibTeX reader and the LaTeX reader were checked by 30 and 29 hand-made mutants (BibTeX 30 of 30 detected; LaTeX 27 detected, 2 equivalent). Against pandoc 3.1.3 on 16 LaTeX and 5 BibTeX fixtures, structural counts agree except for one starred figure environment, and pandoc fails on three BibTeX inputs (see `benches/latex-vs-pandoc/`). Parse-and-render takes 0.02 to 0.4 ms in process on these fixtures. Poppler's `pdfimages` agrees on the PDF fixtures (`benches/pdf-images/`).

Not wired: `convert_bytes` does not accept `tex`, `latex` or `bib`. The new APIs are feature-free and are used directly.

### Changed

- `convert_bytes("rtf")` now returns Markdown rather than plain text. Headings, bold, italic, lists, tables and links are rendered, and Markdown-significant characters are escaped. Text after the document's closing brace is dropped. `\line` is a line break, not a paragraph.
- Truncated or unbalanced RTF no longer fails the conversion. The reader keeps what it can and records warnings (`rtf::parse`). A bad header still returns an error.
- Documentation: README opening, format matrix and dependency table rewritten from the source; added `docs/ARCHITECTURE.md`; repository links point at `webfold-ai/superdocuments`; removed statements the code does not support (cache keying, EPUB panic containment, test counts, timings).
- Prepared the repository for public release: README rewritten to match the code, governance files, CI, supply-chain and release workflows, and `deny.toml`.

## [0.8.1]

Earlier history is in the git log.
