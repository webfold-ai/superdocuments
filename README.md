# superdocuments

Private Ptyktos crate for reusable document parsing and conversion. It has no Webfold crawler, HTTP client, browser, or sibling-checkout path dependencies. Webfold keeps its existing module/API as a compatibility facade.

## Current shared capabilities

- Bounded DOCX/DOCM, PPTX, XLSB, XLSX/XLSM, ODT/ODS/ODP, EPUB, RTF, CSV, text, and HTML-to-Markdown conversion.
- Structured XLSX/XLSM sheet names and cell rows via `superdocuments::xlsx::read_sheet` (including sparse-cell preservation).
- Streaming conversion and the existing document-fold cache APIs.
- Structured HTML blocks with Markdown, stable signatures, and extracted links via `superdocuments::html::parse`, independent of fetching and URL acquisition.
- Optional Transcode image/audio/video codec re-export (`media-codecs` feature), sharing the same pinned revision used by Framefold.

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

## Benchmark suite docs

The Ptyktos benchmark suite spans four repos. Canonical setup /
run / eval / report guides live here in `docs/`:

- [`docs/SETUP.md`](docs/SETUP.md) — prerequisites, clone + build, layout
- [`docs/RUN.md`](docs/RUN.md) — get the data, run the backend, read the report
- [`docs/EVAL.md`](docs/EVAL.md) — adding adapters, recording results
- [`docs/REPORT.md`](docs/REPORT.md) — three-column shape + BLAKE3 regression gate

| repo | what it benchmarks | unified harness |
|---|---|---|
| `Ptyktos/agent-evals` | agents / code / reasoning / safety / vision / medical / financial / ops | `./target/release/agent_evals --list` |
| `Ptyktos/memorybench` | memory / retrieval / long-context | `./target/release/memorybench --list` |
| `Ptyktos/decision-bench` | decision / routing / intent / guardrails / NLI / LLM-as-judge | `reservoir_baseline`, `guardfold_direct`, `verdict_engine`, `score_capture`, `compare_backends` |
