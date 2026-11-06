# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- RTF reading and writing in `superdocuments::rtf`. `rtf::parse` returns an `RtfDocument` (metadata, paragraphs, list items, nested tables, hyperlinks, footnotes, optional headers and footers, warnings). `rtf::to_text` and `rtf::to_markdown` render it. `rtf::to_rtf` and `rtf::markdown_to_rtf` write ASCII-only RTF.
- RTF code pages: `\ansicpg`, `\fcharset`, per-font `\cpg`, Mac Roman, the Symbol-font bullet, and double-byte or UTF-8 sequences split across `\'hh` escapes.

RTF limits: input and output are each limited to 128 MiB, and group nesting to 256 levels. These are `rtf::Limits`, and exceeding one is an error. The reader keeps the whole document in memory and does not stream. Not supported: underline and colour in Markdown, font names, pictures, revision marks, fields other than `HYPERLINK`, right-to-left reordering, and code pages that `encoding_rs` does not provide.

### Changed

- `convert_bytes("rtf")` now returns Markdown rather than plain text. Headings, bold, italic, lists, tables and links are rendered, and Markdown-significant characters are escaped. Text after the document's closing brace is dropped. `\line` is a line break, not a paragraph.
- Truncated or unbalanced RTF no longer fails the conversion. The reader keeps what it can and records warnings (`rtf::parse`). A bad header still returns an error.
- Documentation: README opening, format matrix and dependency table rewritten from the source; added `docs/ARCHITECTURE.md`; repository links point at `webfold-ai/superdocuments`; removed statements the code does not support (cache keying, EPUB panic containment, test counts, timings).
- Prepared the repository for public release: README rewritten to match the code, governance files, CI, supply-chain and release workflows, and `deny.toml`.

## [0.8.1]

Earlier history is in the git log.
