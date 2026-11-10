# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
