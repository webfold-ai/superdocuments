# superdocuments-bench

Private benchmark harness for the Ptyktos OCR / document-AI stack. Companion
repo to [`superdocuments/`](https://github.com/Ptyktos/superdocuments),
[`webfold/`](https://github.com/Ptyktos/webfold), and the local
[`ai-gateway/`](https://github.com/Ptyktos/ai-gateway) vendoring.

## Layout

| Path | Role |
|---|---|
| `ocr/` | OCR quality / latency / graph-fidelity harness. CPU baselines + GPU/OpenAI-compat inference targets. |
| `marathondocbench/` | Baidu's MarathonDocBench (hundred-page document parsing): page images in, one Markdown file per document out, for the OCR fallback and Tesseract. Official scorer not yet run. |

## How this relates to existing bench work

`webfold-bench/bench/document-vs-incumbents/` measures CPU document-conversion
baselines against incumbent converters, and `webfold-bench/bench/STRUCTURED-CONVERSION.md`
(Tracks A–D) publishes the HTML, document, calamine and document-cache runs. Those tracks
measure superdocuments code (webfold re-exports it), so this repo is their intended home. This repo's `ocr/`
bench complements it with **inference-stack** OCR: sglang/vllm-served
PaddleOCR-VL / GLM-OCR, candle-native Rust VLMs, and the OpenAI-compat
gateways (ai-gateway, litellm, bifrost, portkey) routed at those upstreams.

Both benches consume the same `superdocuments::graph::DocumentGraph`
substrate and the same fixture corpus; their scoring layers differ in
emphasis (CPU → CER; inference → CER + graph fidelity + TTFT/ITL).

## Where things live

superdocuments-bench is one of the evidence repos in a small set. Each kind of content has exactly one home.

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

`ocr/` pins superdocuments by Git revision, so every result names the exact revision it
measured. The PDF fixtures under `ocr/fixtures/` duplicate `superdocuments/tests/fixtures/`;
superdocuments is the source of truth for them.

## License

Private. Do not redistribute.
