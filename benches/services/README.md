# services/

Sub HTTP services for the `ocr/` inference targets that
don't ship their own server.

| Service | Status | Drives target |
|---|---|---|
| `candle-serve` | **Stub** — deterministic echo response | `candle-qwen-serve` |

`candle-serve` currently returns a deterministic markdown echo of the user's
last message. The endpoint shapes (`GET /v1/models`, `POST
/v1/chat/completions`) match OpenAI's contract so the bench's
`http_chat_completion` path consumes the response without changes.
The real Qwen2.5-VL model load + autoregressive decode is a follow-up:
candle-transformers 0.9's `quantized_qwen2::ModelWeights::from_gguf`
constructor and the per-tok forward loop are the remaining work.

## Build

```sh
cargo build --release --manifest-path services/candle-serve/Cargo.toml
./target/release/candle-serve    # listens on 0.0.0.0:38080 by default
```

## Test

```sh
# Stub mode (default): echoes the user message wrapped as a markdown heading.
curl -fsS -X POST http://localhost:38080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"test","messages":[{"role":"user","content":"hello"}]}'

# Models endpoint (used by the bench's `target_available()` probe).
curl -fsS http://localhost:38080/v1/models
```

## Config

| Env var | Default | Effect |
|---|---|---|
| `CANDLE_BIND` | `0.0.0.0:38080` | listen address |
| `CANDLE_MODEL_ID` | `Qwen/Qwen2.5-0.5B-Instruct` | id reported via `/v1/models` |
| `CANDLE_STUB` | `1` | `1` = echo stub, `0` = 503 ("model loading not yet implemented") |

## Future services

The bench also has targets for `llama-cpp-server`, `peregrine-ng`,
and `modelfold-serve`. Those are upstream services we shell out to, not
custom binaries — `cargo install lit` (LiteParse), the official llama.cpp
release, the `peregrine-serve` binary, and `modelfold-serve` respectively.
This directory is reserved for services Ptyktos owns.
