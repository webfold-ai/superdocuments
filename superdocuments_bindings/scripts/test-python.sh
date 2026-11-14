#!/usr/bin/env bash
# Build the Python extension and run its tests. Needs Python 3.9+ and a Rust toolchain; no maturin.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p superdocuments_bindings --features python
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
case "$(uname -s)" in
  Darwin) lib=libsuperdocuments_bindings.dylib ;;
  MINGW*|MSYS*|CYGWIN*) lib=superdocuments_bindings.dll ;;
  *) lib=libsuperdocuments_bindings.so ;;
esac
target="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')"
# abi3: one file works for every Python from 3.9.
ext=so; [ "$lib" = superdocuments_bindings.dll ] && ext=pyd
cp "$target/debug/$lib" "$out/superdocuments.abi3.$ext"
PYTHONPATH="$out" python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); sys.argv = ["t"]; import unittest; unittest.main(module=None, argv=["t","discover","-s","tests/python","-v"])' "$out"
