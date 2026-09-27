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

`webfold-bench/bench/ocr-vs-incumbents/` measures CPU baselines (native PDF,
Tesseract fallback, `pdftotext`, `markitdown`, etc.). This repo's `ocr/`
bench complements it with **inference-stack** OCR: sglang/vllm-served
PaddleOCR-VL / GLM-OCR, candle-native Rust VLMs, and the OpenAI-compat
gateways (ai-gateway, litellm, bifrost, portkey) routed at those upstreams.

Both benches consume the same `superdocuments::graph::DocumentGraph`
substrate and the same fixture corpus; their scoring layers differ in
emphasis (CPU → CER; inference → CER + graph fidelity + TTFT/ITL).

## License

Private. Do not redistribute.
