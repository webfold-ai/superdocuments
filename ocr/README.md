# OCR inference-stack benchmark

Companion to [`superdocuments/`](https://github.com/Ptyktos/superdocuments) and
the local [`ai-gateway/`](https://github.com/Ptyktos/ai-gateway) vendoring.
Measures inference-stack OCR — seven targets — on the same fixture corpus as
the CPU baselines in
[`webfold-bench/bench/ocr-vs-incumbents/`](https://github.com/Ptyktos/webfold-bench).

## Targets

| Target | Type | Endpoint | Notes |
|---|---|---|---|
| `candle-qwen2.5-vl` | in-process Rust | — | `cargo run --features candle`. **Text-completion stub today** (Qwen2.5-0.5B reads the extracted PDF text; the VLM vision-tower port is deferred). Honest reporting: every row carries `target_kind = "candle-text-completion"` so the reader sees what the model actually got. |
| `ai-gateway` | OpenAI-compat HTTP | `http://localhost:3100/v1` | Traceloop Hub. OpenAI-compat wrapper already in `ai-gateway/`. |
| `litellm` | OpenAI-compat HTTP | `http://localhost:4000/v1` | Standard proxy. |
| `bifrost` | OpenAI-compat HTTP | `http://localhost:8080/v1` | maximhq/bifrost. |
| `portkey` | OpenAI-compat HTTP | `http://localhost:8787/v1` | Portkey-AI/gateway. |
| `sglang-ocr` | OpenAI-compat HTTP | `http://localhost:30001/v1` | SGLang serving PaddleOCR-VL-1.6 via `sglang.launch_server`. |
| `vllm-ocr` | OpenAI-compat HTTP | `http://localhost:30000/v1` | vLLM serving the same models via its OpenAI-compat server. |
| `liteparse` | subprocess (`lit`) | — | [`run-llama/liteparse`](https://github.com/run-llama/liteparse) (12.7k★, Apache-2.0, Rust core + Python bindings). Drives the `lit` CLI; built-in Tesseract OCR; complexity detection routes native-only PDFs to the 2-5 ms/page path. Set `LITEPARSE_OCR=none` for pure native, `=tesseract` (default) for OCR-on, `=paddle` for the HTTP PaddleOCR server. |
| `parse-bench` | subprocess (`parse-bench`) | — | [`run-llama/ParseBench`](https://github.com/run-llama/ParseBench) (591★, Apache-2.0, LlamaIndex). 5-dimension evaluation (Tables, Charts, Content Faithfulness, Semantic Formatting, Visual Grounding) across ~2,000 human-verified pages. The subprocess layer here currently invokes `parse-bench inference run` against the local fixture; full ParseBench-scored rows are a follow-up that wires parse-bench's own rules into our JSON output. |
| `marker` | subprocess (`marker_single`) | — | [`datalab-to/marker`](https://github.com/datalab-to/marker) (40k★, Apache-2.0 code / OpenRAIL-M weights). GPU PDF→Markdown. `MARKER_DISABLE_IMAGES=1` env var strips image extraction (we don't score images). First run downloads ~2 GB of model weights. |
| `docling` | subprocess (`docling`) | — | [`docling-project/docling`](https://github.com/docling-project/docling) (68k★, MIT, IBM). Drives the `docling` CLI; output written to a tmp dir, we read the produced `<stem>.md`. Honors `DOCLING_DEVICE=cpu\|cuda`. First run downloads ~1.5 GB of weights. |
| `surya-ocr` | subprocess (`surya_ocr`) | — | [`datalab-to/surya`](https://github.com/datalab-to/surya) (21k★, Apache-2.0 code / OpenRAIL-M weights). OCR + layout + reading order + tables in 90+ languages. First run downloads ~3 GB of weights and starts a vllm/llama-server. |

`target_available()` does a cheap `/v1/models` probe so unreachable gateways
report `UNREACHABLE` rather than wasting a chat-completion budget.

### Deliberately excluded

| Candidate | Why excluded |
|---|---|
| `agentgateway` | MCP tool-federation proxy, not a chat-completions gateway. Wrong tool category. |
| `llm-chain` / `llmclient` | HTTP clients, not engines. Same as `reqwest` for our purposes — no separate row needed. |
| `higgsfield-oss` (multi-node training framework) | Not an inference runtime. Only relevant if we fine-tune PaddleOCR-VL ourselves, which is out of scope. |

## Scoring — three columns

| Column | Source |
|---|---|
| **Quality** | text CER (alphanumeric-lowercase) + graph-fidelity (reading-order agreement, table-cell agreement, native-page preservation) |
| **Latency** | wall-clock seconds + bytes/sec + (optional) `guidellm`-driven TTFT/ITL per OpenAI-compat target |
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

All three return `null` when the reference has no rows of the relevant kind.

### TTFT/ITL lane (`--ttft-requests N`)

Mirrors `ai-gateway/benches/ttft.rs`: drives `guidellm run --backend
kind=openai_http,target=<url>,model=<model>` as a subprocess per target,
writes raw JSON to `<out-dir>/ttft_raw/guidellm_<target>.json`, parses TTFT
+ ITL + tokens/sec back into the report's `ttft_lane` field. **Requires
`guidellm` installed (`pip install guidellm`) and a working Python 3.10+
on the host.** Skip with `--ttft-requests 0` (default) on cold runs.

The inspector's three-column table gets a fifth column — TTFT p50/p95/p99 —
which the upstream `ai-gateway/benches/ttft.rs` already collects.

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

## Quickstart — host-native (CPU smoke)

```sh
# Build (no candle — that's opt-in)
cargo build --release --manifest-path ocr/Cargo.toml

# Run against the smoke fixture; reports "no targets available" until you
# stand up at least one OpenAI-compat gateway on the documented ports.
cargo run --release --manifest-path ocr/Cargo.toml -- \
  --fixtures ocr/fixtures \
  --out ocr/results/ocr-inference-2026-09-27.json \
  --warmup 1 --samples 3 --iterations 2

# TTFT/ITL lane (requires `pip install guidellm`)
cargo run --release --manifest-path ocr/Cargo.toml -- \
  --fixtures ocr/fixtures \
  --out ocr/results/ocr-inference-2026-09-27.json \
  --ttft-requests 50
```

## Candle (in-process Rust inference)

```sh
# Qwen2.5-0.5B-Instruct (~1 GB RAM, CPU). Weights download to ~/.cache/huggingface
# on first run; subsequent runs are offline. Override with CANDLE_MODEL_ID env var.
cargo run --release --features candle --manifest-path ocr/Cargo.toml -- \
  --fixtures ocr/fixtures \
  --out ocr/results/candle-2026-09-27.json \
  --only candle-qwen2.5-vl
```

The text-completion stub reads the extracted PDF text (not the rendered
page) and produces a markdown outline. It is **not visual OCR**. The
Qwen2.5-VL vision-tower port — the actual VLM-side of this benchmark — is
deferred because `candle-transformers` 0.9 ships no `qwen2_5_vl` module.
Tracking issue: implement `candle_qwen2_5_vl::vision_tower` from the
published Qwen2.5-VL config (windowed attention, m-RoPE for image
positions, multi-resolution input), then wire it through the existing
Qwen2 language model forward.

## Docker

### Alpine (`Dockerfile.alpine`, ~50 MB final image)

Multi-stage build on `rust:1.85-alpine` → `alpine:3.20`. No GPU. Brings
up `ai-gateway` + `mock_upstream` + the bench via `docker-compose.alpine.yml`.
Mock-upstream only — no real OCR upstream, no Candle (the alpine image
omits the candle feature).

```sh
docker build -f ocr/Dockerfile.alpine -t superdocs-bench-ocr:alpine ocr
docker compose -f ocr/docker-compose.alpine.yml up --abort-on-container-exit \
  --exit-code-from bench
```

### Slim NVIDIA (`Dockerfile.cuda-slim`, ~250 MB final image)

Based on `nvidia/cuda:12.4.1-cudnn-runtime-ubuntu22.04` (slim, no
compiler). Targets the 3060 (sm_86, Ampere); any CUDA 12.x host works.

```sh
docker build -f ocr/Dockerfile.cuda-slim -t superdocs-bench-ocr:cuda-slim ocr
docker compose -f ocr/docker-compose.gpu.yml up --abort-on-container-exit \
  --exit-code-from bench
```

The compose stack runs `vllm/vllm-openai:unlimited-ocr` (SGLang serving
PaddleOCR-VL-1.6 via the published docker image) and `vllm/vllm-openai:latest`
side-by-side. The bench targets both on `--ttft-requests 50`. GPU
access via `deploy.resources.reservations.devices` requires Docker Compose
v2.20+ and `nvidia-container-toolkit` on the host.

### Building both binaries

The alpine compose runs `ai-gateway` natively (its private-git-deps make
it inconvenient to dockerize). Build the ai-gateway binary first:

```sh
cargo build --release --manifest-path ../ai-gateway/Cargo.toml --bin hub
```

The GPU compose pulls pre-built images from Docker Hub; only the bench
binary needs to be in `ocr/target/release/` before the slim build runs.

## Native install (any Linux host)

```sh
# From the repo root, with the bench binary already built:
doas ocr/contrib/install-native.sh
# or: switch to root and run it directly.
```

Idempotent. Installs:
- `/opt/superdocuments-bench/bin/superdocuments-bench-ocr` (the binary)
- `/opt/superdocuments-bench/fixtures/` (the test corpus)
- `/var/log/superdocs-bench/` (results dir)
- `/etc/systemd/system/superdocuments-bench-ocr.service` (oneshot — wire
  a `superdocuments-bench-ocr.timer` if you want scheduled runs)

Override `OPENAI_BASE_URL` via `systemctl edit` to point at the local
gateway.

## Status

| Layer | Status |
|---|---|
| Skeleton + 12 targets + CER + 3-dim graph-fidelity scoring | ✅ implemented |
| TTFT/ITL lane via `guidellm` | ✅ implemented (`--ttft-requests N`) |
| Candle text-completion stub (Qwen2.5-0.5B-Instruct) | ✅ implemented behind `--features candle` |
| `liteparse` subprocess target | ✅ wired (gated on `lit` being on PATH) |
| `parse-bench` subprocess target | ✅ wired (gated on `parse-bench` being on PATH) |
| `marker` subprocess target | ✅ wired (gated on `marker_single` being on PATH) |
| `docling` subprocess target | ✅ wired (gated on `docling` being on PATH) |
| `surya-ocr` subprocess target | ✅ wired (gated on `surya_ocr` being on PATH) |
| `Dockerfile.alpine` + `docker-compose.alpine.yml` | ✅ written, smoke builds clean on this host |
| `Dockerfile.cuda-slim` + `docker-compose.gpu.yml` | ✅ written, builds on this host (no GPU here) |
| `contrib/install-native.sh` + systemd unit | ✅ written |
| Adversarial fixture corpus | ⏳ pending — `webfold-bench/bench/ocr-vs-incumbents/make_adversarial.py` already generates the same set we want here |
| `candle-qwen2.5-vl` vision-tower port | ⏳ deferred — needs custom architecture in `candle-transformers`; multi-week |
| GPU SGLang / vLLM serving PaddleOCR-VL / GLM-OCR / olmOCR | ⏳ blocked on GPU host (this host has no NVIDIA) |
| Full `parse-bench` 5-dimension scoring against `text_content.jsonl` / `table.jsonl` rules | ⏳ follow-up — wire parse-bench's published rule files into our JSON output |
| `nougat` / `olmocr` direct CLI targets | ⏳ skipped — both expect server/workspace patterns, not a single-file CLI. `sglang-ocr` and `vllm-ocr` cover the inference-side comparison. |

## What this is NOT

- **Not a Tesseract / Poppler / `pdftotext` benchmark.** Those live in
  `webfold-bench/bench/ocr-vs-incumbents/`. Different layer (CPU baseline vs
  inference serving).
- **Not a quality model leaderboard.** We're not claiming PaddleOCR-VL beats
  Tesseract — we're measuring what happens when each runs at production
  scale, with the same fixture set, against the same graph substrate.
- **Not a Rust-native PaddleOCR-VL port.** That's the multi-week candle port
  above. If/when it lands, it's one more target in this table.
