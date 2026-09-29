# superdocuments

Private Ptyktos crate for reusable document parsing and conversion. It has no Webfold crawler, HTTP client, browser, or sibling-checkout path dependencies. Webfold keeps its existing module/API as a compatibility facade.

## Current shared capabilities

- Bounded DOCX/DOCM, PPTX, XLSB, XLSX/XLSM, ODT/ODS/ODP, EPUB, RTF, CSV, text, and HTML-to-Markdown conversion.
- Structured XLSX/XLSM sheet names and cell rows via `superdocuments::xlsx::read_sheet` (including sparse-cell preservation).
- Streaming conversion and the existing document-fold cache APIs.
- The single-pass HTML-to-Markdown extractor (`superdocuments::html::{to_blocks, render, Template, links, resolve, chunk_markdown}`) plus structured blocks with Markdown, stable signatures, and extracted links via `superdocuments::html::parse`, independent of fetching and URL acquisition. Chrome subtrees (`nav`, `header`, `script`, …) are tracked on a tag stack, so a mismatched close tag cannot end a dropped subtree early.
- In-process PDF-to-Markdown parsing through the crate's own PDF parser (`superdocuments::pdf::parser`, `extract_markdown` / `extract_file`), with bounded input and panic containment; empty/unsupported text layers fall back to Poppler + Tesseract via `extract_file_with_ocr`.
- Optional Transcode image/audio/video codec re-export (`media-codecs` feature) and embedded PDF image extraction (`images` feature), sharing the same pinned revision used by Framefold.

Legacy binary XLS remains behind Webfold's compatibility adapter; Superdocuments deliberately does not introduce Calamine. The HTML extractor and PDF parser were moved here from Transcode's `doc` module, which is not part of the GitHub Transcode (now media codecs only); `superdocuments::pdf` exposes the parser in-process and retains the process adapter for explicit legacy overrides. OCR fallback uses the existing Tesseract backend and Poppler renderer from `PATH`, with configurable language, page/pixel caps, a shared deadline, and output limits. Modelfold's OCR examples are dataset-supervised research pipelines without a shipped general-purpose checkpoint, so they are not misrepresented as a runtime OCR engine. Mixed PDFs with both born-digital and scanned pages, unusual PDF filters, and encrypted/object-stream variants need broader parity coverage. Framefold is an optional sibling renderer (not a document parser), exposed as `superdocuments::frame_renderer` with the `framefold` feature; enable `framefold-js` only when GSAP timeline execution is needed. Framefold's pinned Ruler revision still points at the internal GitLab host, so the `framefold` feature (and fresh dependency resolution, which also resolves optional dependencies) needs that host until that Ruler revision is on GitHub.

## Migration boundary

Document and HTML conversion were migrated from Webfold (and the HTML extractor and PDF parser from Transcode's `doc` module, which is not part of the GitHub Transcode). Webfold's `webfold::document` and `webfold::{to_blocks, render, Template, …}` re-export this crate ([Ptyktos/webfold#2](https://github.com/Ptyktos/webfold/pull/2)); Webfold keeps only the legacy XLS adapter (through [calamine-apeiron](https://github.com/Ptyktos/calamine-apeiron)) and its web-specific fetching/crawling/browser features. New conversion code goes here, not in Webfold. The initial importer uses the shared XLSX parser directly. Archived `extract4llm` and `pademelon` are not dependencies.

## Build and tests

```sh
cargo test
```

Consumers pin a Git revision; GitHub authorization to `Ptyktos/superdocuments` is required. Code remains under the carried FSL-1.1-ALv2 terms (see `LICENSE`).

## Remaining stack work

PDF native extraction, malformed-input/resource-limit checks, and Tesseract fallback are implemented and tested with authored producer/scanned fixtures; broader real-world corpus parity and mixed-page OCR remain open, as do legacy XLS extraction, additional HTML parity fixtures/API ergonomics, and the full media-project/consumer audit. Framefold is available as an opt-in sibling capability; do not treat this crate as a complete umbrella for every media project yet.

## Where things live

Superdocuments is one repo in a small set. Each kind of content has exactly one home.

**Code**

| Repo | Owns | Put here |
|---|---|---|
| [`webfold`](https://github.com/Ptyktos/webfold) | Web acquisition: fetch, crawl, WARC, headless browser + CDP server, SERP/search, proxy, `webfold` CLI and `webfold-mcp` | Anything that touches the network, a browser or an archive |
| [`superdocuments`](https://github.com/Ptyktos/superdocuments) | Conversion: HTML → Markdown (`to_blocks`/`render`/`Template`), DOCX/PPTX/XLSX/XLSB/ODF/EPUB/RTF/CSV, PDF parsing + OCR fallback, `DocumentGraph` | Any bytes → Markdown/structure code. Webfold re-exports it; only legacy XLS stays in webfold |
| [`calamine-apeiron`](https://github.com/Ptyktos/calamine-apeiron) | Our calamine fork (Apeiron-backed workbook maps) | Spreadsheet-reader changes; used by webfold's XLS adapter |
| [`hashtrinity`](https://github.com/Ptyktos/hashtrinity), [`apeiron`](https://github.com/Ptyktos/apeiron) | Base data structures | One pinned revision shared by every crate above |
| [`arxiv-search`](https://github.com/Ptyktos/arxiv-search), [`youtube-transcript-mcp`](https://github.com/Ptyktos/youtube-transcript-mcp) | Public MIT MCP servers for arXiv and YouTube sources | Source-specific retrieval; webfold may consume their `-core` crates, never the reverse |

**Evidence (benchmarks, evals, chaos).** [`ptyktos-evals`](https://github.com/Ptyktos/ptyktos-evals) is the index: its `registry.toml` maps every stack layer to the bench that covers it, the command, the baseline and the pass/fail thresholds. Each bench repo keeps its raw results next to its harness:

- [`webfold-bench`](https://github.com/Ptyktos/webfold-bench) — web acquisition: competitive runs (Firecrawl, Crawl4AI, OpenSERP, Steel, Maxun), SERP, chaos/load, capacity; also the published conversion write-up (`bench/STRUCTURED-CONVERSION.md`, Tracks A–F)
- [`superdocuments-bench`](https://github.com/Ptyktos/superdocuments-bench) — conversion and OCR quality/latency (new conversion benchmarks go here)
- [`spbench`](https://github.com/Ptyktos/spbench) — the runner behind the conversion tracks (`webfold-structured/run.sh`) and persona-driven CDP task benchmarks
- [`faultbox`](https://github.com/Ptyktos/faultbox) — fault injection: toxiproxy-compatible TCP chaos proxy plus process/container/VM chaos, used for the failure mode of every layer
- [`hashtrinity-bench`](https://github.com/Ptyktos/hashtrinity-bench) — base data structures (tables, maps, signature hashing)
- [`sangdb-bench`](https://github.com/Ptyktos/sangdb-bench) — sangdb store, ontology CRDT, replication, ClickBench-style analytics
- [`relayfold-bench`](https://github.com/Ptyktos/relayfold-bench) — proxy load and latency vs nginx and others
- [`memorybench`](https://github.com/Ptyktos/memorybench) — memory, retrieval, long-context
- [`web-search-bench`](https://github.com/Ptyktos/web-search-bench) — web search and deep-research federation (WANDR, WideSearch, DeepSearchQA, Find-All Company, webarena, …)
- [`agent-evals`](https://github.com/Ptyktos/agent-evals) — model evals (code, reasoning, safety, vision, …)

**Claims.** [`web-agents-marketing`](https://github.com/Ptyktos/web-agents-marketing) holds positioning, pitch and investor material; every number it quotes links to a result file in one of the repos above.

Rule of thumb: code lives where its dependencies are lightest, numbers live in a bench repo next to the raw results (indexed by ptyktos-evals), and claims live in `web-agents-marketing` pointing at those numbers.

Conversion benchmarks (HTML fixtures, document formats, PDF/OCR) belong in [`superdocuments-bench`](https://github.com/Ptyktos/superdocuments-bench); the HTML/document tracks currently published in `webfold-bench/bench/STRUCTURED-CONVERSION.md` measure this crate's code.

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
