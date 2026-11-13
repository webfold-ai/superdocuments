#!/bin/sh
# Usage: it.sh BIN   rebuild-free: runs the csl track and prints the report
cd "$(dirname "$0")" && python3 cite_bench.py run csl "$1" && python3 cite_bench.py report csl
