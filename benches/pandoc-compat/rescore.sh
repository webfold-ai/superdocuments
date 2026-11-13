#!/bin/sh
# Usage: rescore.sh TOOL BIN   -- converts the corpus with BIN, scores it against the Pandoc oracle
cd "$(dirname "$0")" && rm -rf outputs/$1 && PC_JOBS=${PC_JOBS:-2} python3 score.py convert $1 -- $2 '{path}' | tail -1 && python3 score.py score $1 | tail -1
