# superdocuments-bench

Private benchmark harness for the Ptyktos OCR / document-AI stack. Companion
repo to [`superdocuments/`](https://github.com/Ptyktos/superdocuments),
[`webfold/`](https://github.com/Ptyktos/webfold), and the local
[`ai-gateway/`](https://github.com/Ptyktos/ai-gateway) vendoring.

## Layout

| Path | Role |
|---|---|
| `ocr/` | OCR quality / latency / graph-fidelity harness. CPU baselines + GPU/OpenAI-compat inference targets. |

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

superdocuments-bench is the conversion-benchmark repo in a small set. Each kind of content has exactly one home:

| Repo | Owns | Put here |
|---|---|---|
| [`webfold`](https://github.com/Ptyktos/webfold) | Web acquisition: fetch, crawl, headless browser + CDP server, SERP, proxy, `webfold` CLI and `webfold-mcp` | Anything that touches the network or a browser |
| [`superdocuments`](https://github.com/Ptyktos/superdocuments) | Conversion: HTML → Markdown (`to_blocks`/`render`/`Template`), DOCX/PPTX/XLSX/ODF/EPUB/RTF/CSV, PDF parsing + OCR fallback, `DocumentGraph` | Any bytes → Markdown/structure code. Webfold re-exports it; only legacy XLS stays in webfold |
| [`calamine-apeiron`](https://github.com/Ptyktos/calamine-apeiron) | Our calamine fork (Apeiron-backed workbook maps) | Spreadsheet-reader changes; used by webfold's XLS adapter |
| [`hashtrinity`](https://github.com/Ptyktos/hashtrinity), [`apeiron`](https://github.com/Ptyktos/apeiron) | Base data structures | One pinned revision shared by every crate above |
| [`webfold-bench`](https://github.com/Ptyktos/webfold-bench) | Web benchmarks: competitive (Firecrawl, Crawl4AI, OpenSERP, Steel, …), SERP, chaos/load, capacity | Measurements of fetching, crawling, browsing |
| [`superdocuments-bench`](https://github.com/Ptyktos/superdocuments-bench) | Conversion/OCR benchmarks | Measurements of HTML/document/PDF conversion quality and speed |
| [`hashtrinity-bench`](https://github.com/Ptyktos/hashtrinity-bench) | Base-structure benchmarks | Table/map microbenchmarks, block-signature hash comparisons |
| [`arxiv-search`](https://github.com/Ptyktos/arxiv-search), [`youtube-transcript-mcp`](https://github.com/Ptyktos/youtube-transcript-mcp) | Public MIT MCP servers for arXiv and YouTube sources | Source-specific search/retrieval; webfold may consume their `-core` crates, never the reverse |
| [`web-agents-marketing`](https://github.com/Ptyktos/web-agents-marketing) | Positioning, pitch and investor material | Anything written for buyers or investors. Every number it quotes links to a result file in a bench repo |

Rule of thumb: code lives where its dependencies are lightest, numbers live in a bench repo next to the raw results, and claims live in `web-agents-marketing` pointing at those numbers.

`ocr/` pins superdocuments by Git revision, so every result names the exact revision it
measured. The PDF fixtures under `ocr/fixtures/` duplicate `superdocuments/tests/fixtures/`;
superdocuments is the source of truth for them.

## License

Private. Do not redistribute.
