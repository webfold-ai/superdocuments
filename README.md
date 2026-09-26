# superdocuments

Private Ptyktos crate for reusable document parsing and conversion. It has no Webfold crawler, HTTP client, browser, or sibling-checkout path dependencies. Webfold keeps its existing module/API as a compatibility facade.

## Current shared capabilities

- Bounded DOCX/DOCM, PPTX, XLSB, XLSX/XLSM, ODT/ODS/ODP, EPUB, RTF, CSV, text, and HTML-to-Markdown conversion.
- Structured XLSX/XLSM sheet names and cell rows via `superdocuments::xlsx::read_sheet` (including sparse-cell preservation).
- Streaming conversion and the existing document-fold cache APIs.
- HTML-to-Markdown through the native `transcode` HTML parser, independent of fetching and URL acquisition.

Legacy binary XLS remains behind Webfold's compatibility adapter; Superdocuments deliberately does not introduce Calamine. PDF now has a shared process adapter (`superdocuments::pdf`) for `PDFFOLD_BIN`, a configured Cargo manifest, or `pdffold` on `PATH`; the actual parser remains external because its source/API is not available in this workspace. Framefold is now an optional sibling renderer (not a document parser), exposed as `superdocuments::frame_renderer` with the `framefold` feature; enable `framefold-js` only when GSAP timeline execution is needed. Its pinned Ruler and Transcode dependencies are portable Git revisions.

## Migration boundary

The document conversion implementation was migrated from Webfold and its former public API path now delegates to this crate. Webfold retains the legacy XLS adapter and web-specific fetching/crawling/browser features. The initial importer uses the shared XLSX parser directly. Archived `extract4llm` and `pademelon` are not dependencies.

## Build and tests

```sh
cargo test
```

The crate is pinned by private Git tag for consumers; GitHub authorization to `Ptyktos/superdocuments` is required. Code remains under the carried FSL-1.1-ALv2 terms (see `LICENSE`).

## Remaining stack work

In-process PDF parser integration, legacy XLS extraction, additional HTML parity fixtures/API ergonomics, and the full media-project/consumer audit remain open. Framefold is available as an opt-in sibling capability; do not treat this crate as a complete umbrella for every media project yet.
