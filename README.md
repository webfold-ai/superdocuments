# superdocuments

Private Ptyktos crate for reusable document parsing and conversion. It has no Webfold crawler, HTTP client, browser, or sibling-checkout path dependencies. Webfold keeps its existing module/API as a compatibility facade.

## Current shared capabilities

- Bounded DOCX/DOCM, PPTX, XLSB, XLSX/XLSM, ODT/ODS/ODP, EPUB, RTF, CSV, text, and HTML-to-Markdown conversion.
- Structured XLSX/XLSM sheet names and cell rows via `superdocuments::xlsx::read_sheet` (including sparse-cell preservation).
- Streaming conversion and the existing document-fold cache APIs.
- HTML-to-Markdown through the native `transcode` HTML parser, independent of fetching and URL acquisition.

Legacy binary XLS remains behind Webfold's compatibility adapter; Superdocuments deliberately does not introduce a Calamine dependency. PDF extraction remains Webfold's existing `pdffold` process adapter because the parser source/API is not available in this workspace yet. Framefold remains a sibling renderer, not a document parser; its current manifest depends on a local `ruler-fix` checkout and requires a portable dependency/license audit before it can be composed here.

## Migration boundary

The document conversion implementation was migrated from Webfold and its former public API path now delegates to this crate. Webfold retains the legacy XLS adapter and web-specific fetching/crawling/browser features. The initial importer uses the shared XLSX parser directly. Archived `extract4llm` and `pademelon` are not dependencies.

## Build and tests

```sh
cargo test
```

The crate is pinned by private Git tag for consumers; GitHub authorization to `Ptyktos/superdocuments` is required. Code remains under the carried FSL-1.1-ALv2 terms (see `LICENSE`).

## Remaining stack work

PDF library integration, independent HTML fixtures/API ergonomics, Framefold and the full media-project/consumer audit remain open. Do not treat this crate as a complete umbrella for those separate capabilities yet.
