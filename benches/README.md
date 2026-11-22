# benches/ — the superdocuments benchmark harnesses

These harnesses live **inside the superdocuments repo**, under `benches/`.
They were imported from the standalone `superdocuments-bench` repo with their
git history preserved (the import is a merge commit whose second parent is the
bench repo's last `main`), so every old commit is still reachable from this
repo's history. The standalone repo is archived; this directory is its home.

Companion code lives in [`superdocuments/`](https://github.com/webfold-ai/superdocuments),
[`webfold/`](https://github.com/Ptyktos/webfold), and the
[`ai-gateway/`](https://github.com/Ptyktos/ai-gateway) vendoring.

## Layout

| Path | Role |
|---|---|
| `ocr/` | OCR quality / latency / graph-fidelity harness. CPU baselines + GPU/OpenAI-compat inference targets. |
| `marathondocbench/` | Baidu's MarathonDocBench (hundred-page document parsing): page images in, one Markdown file per document out, for the OCR fallback and Tesseract. Official scorer not yet run. |
| `services/` | Sub HTTP services the `ocr/` inference targets need (e.g. `candle-serve`). |

`ocr/` and `services/candle-serve/` are their own cargo workspaces with their own
lockfiles — they pin the superdocuments revision they measure, so they must not
inherit the crate's workspace lock. They are listed in the root
[`Cargo.toml`](../Cargo.toml) `exclude` list.

## How this relates to existing bench work

`webfold-bench/bench/document-vs-incumbents/` measures CPU document-conversion
baselines against incumbent converters, and `webfold-bench/bench/STRUCTURED-CONVERSION.md`
(Tracks A–D) publishes the HTML, document, sheetfold and document-cache runs. Those tracks
measure superdocuments code (webfold re-exports it), so this directory is their intended home. The `ocr/`
bench complements it with **inference-stack** OCR: sglang/vllm-served
PaddleOCR-VL / GLM-OCR, candle-native Rust VLMs, and the OpenAI-compat
gateways (ai-gateway, litellm, bifrost, portkey) routed at those upstreams.

Both benches consume the same `superdocuments::graph::DocumentGraph`
substrate and the same fixture corpus; their scoring layers differ in
emphasis (CPU → CER; inference → CER + graph fidelity + TTFT/ITL).

## Where things live

This directory is one of the evidence homes in a small set. Each kind of content has exactly one home.

**Code**

| Repo | Owns | Put here |
|---|---|---|
| [`webfold`](https://github.com/Ptyktos/webfold) | Web acquisition: fetch, crawl, WARC, headless browser + CDP server, SERP/search, proxy, `webfold` CLI and `webfold-mcp` | Anything that touches the network, a browser or an archive |
| [`superdocuments`](https://github.com/webfold-ai/superdocuments) | Conversion: HTML → Markdown (`to_blocks`/`render`/`Template`), DOCX/PPTX/XLSX/XLSB/ODF/EPUB/RTF/CSV, PDF parsing + OCR fallback, `DocumentGraph` — **and this `benches/` directory** | Any bytes → Markdown/structure code, and the conversion/OCR benchmark harnesses for it |
| [`calamine-apeiron`](https://github.com/Ptyktos/calamine-apeiron) | Our sheetfold fork (Apeiron-backed workbook maps) | Spreadsheet-reader changes; used by webfold's XLS adapter |
| [`hashtrinity`](https://github.com/Ptyktos/hashtrinity), [`apeiron`](https://github.com/Ptyktos/apeiron) | Base data structures | One pinned revision shared by every crate above |
| [`arxiv-search`](https://github.com/Ptyktos/arxiv-search), [`youtube-transcript-mcp`](https://github.com/Ptyktos/youtube-transcript-mcp) | Public MIT MCP servers for arXiv and YouTube sources | Source-specific retrieval; webfold may consume their `-core` crates, never the reverse |

**Evidence (benchmarks, evals, chaos).** [`ptyktos-evals`](https://github.com/Ptyktos/ptyktos-evals) is the index: its `registry.toml` maps every stack layer to the bench that covers it, the command, the baseline and the pass/fail thresholds. Each bench keeps its raw results next to its harness:

- [`webfold-bench`](https://github.com/Ptyktos/webfold-bench) — web acquisition: competitive runs (Firecrawl, Crawl4AI, OpenSERP, Steel, Maxun), SERP, chaos/load, capacity; also the published conversion write-up (`bench/STRUCTURED-CONVERSION.md`, Tracks A–F)
- **this directory** (`superdocuments/benches/`) — conversion and OCR quality/latency; new conversion benchmarks go here
- [`spbench`](https://github.com/Ptyktos/spbench) — the runner behind the conversion tracks (`webfold-structured/run.sh`) and persona-driven CDP task benchmarks
- [`faultbox`](https://github.com/Ptyktos/faultbox) — fault injection: toxiproxy-compatible TCP chaos proxy plus process/container/VM chaos, used for the failure mode of every layer
- [`hashtrinity-bench`](https://github.com/Ptyktos/hashtrinity-bench) — base data structures (tables, maps, signature hashing)
- [`iotadb-bench`](https://github.com/webfold-ai/iotadb-bench) — iotadb store, ontology CRDT, replication, ClickBench-style analytics
- [`relayfold-bench`](https://github.com/Ptyktos/relayfold-bench) — proxy load and latency vs nginx and others
- [`memorybench`](https://github.com/Ptyktos/memorybench) — memory, retrieval, long-context
- [`web-search-bench`](https://github.com/Ptyktos/web-search-bench) — web search and deep-research federation (WANDR, WideSearch, DeepSearchQA, Find-All Company, webarena, …)
- [`agent-evals`](https://github.com/Ptyktos/agent-evals) — model evals (code, reasoning, safety, vision, …)

**Claims.** [`web-agents-marketing`](https://github.com/Ptyktos/web-agents-marketing) holds positioning, pitch and investor material; every number it quotes links to a result file in one of the repos above.

Rule of thumb: code lives where its dependencies are lightest, numbers live next to the harness that produced them (indexed by ptyktos-evals), and claims live in `web-agents-marketing` pointing at those numbers.

`ocr/` pins superdocuments by Git revision (`https://github.com/webfold-ai/superdocuments`
at a named rev), so every result names the exact revision it
measured. The PDF fixtures under `ocr/fixtures/` duplicate `superdocuments/tests/fixtures/`;
superdocuments is the source of truth for them.

## License

Apache-2.0. See [LICENSE](LICENSE).
