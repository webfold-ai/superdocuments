# superdocuments

Private Ptyktos workspace for reusable document and media parsing/conversion. Webfold remains the web product/facade and must retain its current supported format behavior while delegating shared parsing here. This crate must build standalone; no sibling checkout paths.

## Scope

- **Document conversion/parsing:** HTML, PDF, CSV, DOCX/DOCM, PPTX, XLS/XLSB/XLSX/XLSM, ODT/ODS/ODP, EPUB, RTF, and any additional formats discovered in the full capability audit.
- **Structured extraction:** return text blocks, tables, sheets, coordinates, metadata and diagnostics—not only Markdown—so import, search, OCR and downstream AI consumers can share parsing without format-specific forks.
- **Media/render sibling capabilities:** Framefold rendering and Transcode codecs are part of the wider workspace plan, but remain separate crates/APIs from document parsing. Webfold's fetching/crawling/browser responsibilities stay in Webfold.

## First extraction slice

The initial crate exposes a bounded XLSX/XLSM worksheet reader, migrated from Webfold's native parser. It is the path consumed by `biota-importer`; Webfold keeps a compatibility API. Remaining formats are migrated incrementally with existing golden/regression fixtures and output parity gates. No all-formats or speed-parity claim until those tests exist.

`framefold`, `pdffold`, Apeiron, Transcode and the actual Webfold consumers require a full dependency/license/API audit before they are composed or copied here. In particular, do not pull Webfold's network stack into this crate.

## License

This private repository retains Webfold's FSL-1.1-ALv2 terms for the extracted implementation. Keep the source notice with derived code; resolve Framefold's missing manifest license before redistribution or embedding.
