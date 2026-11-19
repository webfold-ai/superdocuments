#!/bin/sh
# Alpine entrypoint for the OCR bench. Forwards CLI args to the binary.
# If GUIDELLM_RUN=1 is set in the environment, also runs the guidellm TTFT/ITL
# lane with --ttft-requests=50 (matches ai-gateway/benches/ttft.rs default).
set -eu

EXTRA=""
if [ "${GUIDELLM_RUN:-0}" = "1" ] && [ -x "$(command -v guidellm 2>/dev/null || true)" ]; then
    EXTRA="--ttft-requests=${GUIDELLM_REQUESTS:-50}"
fi

exec /bench/superdocuments-bench-ocr $EXTRA "$@"
