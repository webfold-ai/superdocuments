# OCR inference-stack benchmark

Companion to [`superdocuments/`](https://github.com/Ptyktos/superdocuments) and
the local [`ai-gateway/`](https://github.com/Ptyktos/ai-gateway) vendoring.
Measures inference-stack OCR — the seven targets below — on the same fixture
corpus as the CPU baselines in
[`webfold-bench/bench/ocr-vs-incumbents/`](https://github.com/Ptyktos/webfold-bench).

## Targets

| Target | Type | Endpoint | Notes |
|---|---|---|---|
| `candle-qwen2.5-vl` | in-process Rust | — | `cargo run --features candle`. Stub for now; returns an honest "not implemented" until the runner lands. |
| `ai-gateway` | OpenAI-compat HTTP | `http://localhost:3100/v1` | Traceloop Hub. The OpenAI-compat wrapper already exists in `ai-gateway/`. |
| `litellm` | OpenAI-compat HTTP | `http://localhost:4000/v1` | Standard proxy. |
| `bifrost` | OpenAI-compat HTTP | `http://localhost:8080/v1` | maximhq/bifrost. |
| `portkey` | OpenAI-compat HTTP | `http://localhost:8787/v1` | Portkey-AI/gateway. |
| `sglang-ocr` | OpenAI-compat HTTP | `http://localhost:30001/v1` | SGLang serving PaddleOCR-VL-1.6 / GLM-OCR / olmOCR via `sglang.launch_server`. |
| `vllm-ocr` | OpenAI-compat HTTP | `http://localhost:30000/v1` | vLLM serving the same models via its OpenAI-compat server. |

`target_available()` does a cheap `/v1/models` probe so unreachable gateways
report `UNREACHABLE` rather than wasting a chat-completion budget.

### What we deliberately excluded

| Candidate | Why excluded |
|---|---|
| `agentgateway` | MCP tool-federation proxy, not a chat-completions gateway. Wrong tool category. |
| `llm-chain` / `llmclient` | HTTP clients, not engines. Same as `reqwest` for our purposes — no separate row needed. |
| `higgsfield-oss` (multi-node training framework) | Not an inference runtime. Only relevant if we fine-tune PaddleOCR-VL ourselves, which is out of scope. |

## Scoring — three columns

| Column | Source |
|---|---|
| **Quality** | text CER (alphanumeric-lowercase) + graph-fidelity (reading-order agreement, table-cell agreement, native-page preservation) |
| **Latency** | wall-clock seconds + bytes/sec for now; TTFT/ITL for OpenAI-compat targets comes from a future `guidellm`/`aiperf` lane (the same harness ai-gateway/benches already wires up) |
| **Cost** | peak RSS KiB (parent-side only; subprocess OpenAI-compat upstreams are not aggregated), model output bytes |

### Graph-fidelity score

Each inference target's markdown output is projected through
`superdocuments::graph::from_ocr_markdown` into a `DocumentGraph`. The
ground-truth `<fixture>.expected.json` is the same projection done by hand.
Three agreement scores:

| Score | Definition |
|---|---|
| `median_reading_order_agreement` | fraction of reference regions whose (kind, text, reading_order) matches the hypothesis at the same reading_order within the same page |
| `median_table_cell_agreement` | fraction of reference table cells `(page, row, col)` whose text matches the hypothesis |
| `median_native_preservation` | fraction of reference pages marked `source: native` that the hypothesis also marks `source: native`. Critical for mixed-page PDFs — a hallucinating VLM that rewrites a native page as OCR text scores 0 here. |

All three return `null` when the reference has no rows of the relevant kind
(e.g. table_cell_agreement is null if the reference has no tables).

## Fixture format

```
ocr/fixtures/
├── <name>.pdf
├── <name>.expected.txt        # alphanumeric CER reference
└── <name>.expected.json       # DocumentGraph ground truth
```

`<name>.expected.json` is `{"markdown": "<!-- page 1 -->\n...", "source_label":
"ocr"}` — the markdown string is run through `from_ocr_markdown` at fixture
load time so the comparison uses the same builder the inference output goes
through.

**Important:** write `expected.json` via `json.dumps(...)` (e.g. with the
provided `make_smoke.py`). Hand-written heredocs that emit raw newlines inside
the `markdown` string produce invalid JSON (`\n` must be the two-byte escape
sequence, not the byte `\012`). The bench silently treats malformed JSON as
"no graph ground truth" — same surface error as a missing file.

## Quickstart

```sh
# Build (no candle — that's opt-in)
cargo build --release --manifest-path ocr/Cargo.toml

# Run against the smoke fixture; reports "no targets available" until you
# stand up at least one OpenAI-compat gateway on the documented ports.
cargo run --release --manifest-path ocr/Cargo.toml -- \
  --fixtures ocr/fixtures \
  --out ocr/results/ocr-inference-2026-09-27.json \
  --warmup 1 --samples 3 --iterations 2
```

To enable the in-process Rust target once the runner is implemented:

```sh
cargo run --release --features candle --manifest-path ocr/Cargo.toml -- \
  --fixtures ocr/fixtures --out ocr/results/candle-smoke.json
```

## Status

| Layer | Status |
|---|---|
| Skeleton + 7 targets + CER + 3-dim graph-fidelity scoring | ✅ implemented |
| `candle-qwen2.5-vl` runner | ⏳ stub (returns "not implemented"); gated behind `--features candle` so the default build stays clean |
| TTFT/ITL via `guidellm` + `aiperf` | ⏳ not yet wired; the same integration lives in `ai-gateway/benches/ttft.rs` and we will mirror it here |
| GPU SGLang / vLLM serving PaddleOCR-VL / GLM-OCR / olmOCR | ⏳ blocked on GPU host |
| Adversarial fixture corpus | ⏳ pending — `webfold-bench/bench/ocr-vs-incumbents/make_adversarial.py` already generates the same set we want here |

## What this is NOT

- **Not a Tesseract / Poppler / `pdftotext` benchmark.** Those live in
  `webfold-bench/bench/ocr-vs-incumbents/`. Different layer (CPU baseline vs
  inference serving).
- **Not a quality model leaderboard.** We're not claiming PaddleOCR-VL beats
  Tesseract — we're measuring what happens when each runs at production
  scale, with the same fixture set, against the same graph substrate.
- **Not a Rust-native PaddleOCR-VL port.** That's the multi-week candle port
  above. If/when it lands, it's one more target in this table.
