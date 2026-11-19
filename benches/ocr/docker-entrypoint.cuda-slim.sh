#!/bin/sh
# CUDA-slim entrypoint. Same shape as the alpine entrypoint: forwards CLI args
# to the binary, optionally runs the guidellm TTFT/ITL lane if installed.
set -eu

EXTRA=""
if [ "${GUIDELLM_RUN:-0}" = "1" ] && command -v guidellm >/dev/null 2>&1; then
    EXTRA="--ttft-requests=${GUIDELLM_REQUESTS:-50}"
fi

exec /bench/superdocuments-bench-ocr $EXTRA "$@"
