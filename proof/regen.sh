#!/usr/bin/env bash
# Regenerate (or, with --check, verify) proof/PdfMath.lean: the Charon/Aeneas extraction of
# the loop-free PDF kernels in proof/extract_pdf.rs, adapted to the self-contained
# Constructive prelude exactly as webfold's proof/regen.sh does.
set -euo pipefail

PROOF_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
ROOT_DIR="$(cd -- "$PROOF_DIR/.." && pwd)"
cd "$ROOT_DIR"

CHARON="${WEBFOLD_CHARON:-charon}"
AENEAS="${WEBFOLD_AENEAS:-aeneas}"
MODE="${1:-write}"
if [[ "$MODE" != write && "$MODE" != --check ]]; then
    echo "usage: proof/regen.sh [--check]" >&2
    exit 2
fi
if [[ ! -f "$PROOF_DIR/extract_pdf.rs" ]]; then
    echo "missing proof/extract_pdf.rs (the kernels PdfSpec names)" >&2
    exit 1
fi

# The kernel surface PdfSpec fixes; a change here is a specification change.
KERNELS="xref_be_step xref_row_len row_start xref_entry_type xref_offset_ok objstm_ref_ok objstm_header_ok objstm_span xref_prev_ok stream_extent predictor_kind png_geometry png_recon_byte"

TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

START=()
for k in $KERNELS; do START+=(--start-from "crate::$k"); done

echo "== extracting constructive PDF kernels =="
"$CHARON" rustc --sysroot default --preset aeneas \
    --dest-file "$TMP/pdf_math.llbc" "${START[@]}" \
    -- --crate-type lib --crate-name pdf_math "$PROOF_DIR/extract_pdf.rs"
"$AENEAS" -backend lean -dest "$TMP" -warnings-as-errors -no-progress-bar \
    "$TMP/pdf_math.llbc"

python3 - "$TMP/PdfMath.lean" "$KERNELS" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
raw = path.read_text()
assert raw.count('public import Aeneas') == 1
assert raw.count('open Aeneas Aeneas.Std Result ControlFlow Error') == 1
body = raw.split('namespace pdf_math\n', 1)[1]
expected = set(sys.argv[2].split())
assert set(re.findall(r'^def (\w+)', body, re.M)) == expected, 'Kernel surface changed; review required'
assert not re.search(r'\b(sorry|axiom|opaque|unsafe|partial_fixpoint|noncomputable)\b|Result\.div', body), 'Unsupported extraction'
adapted = raw.replace('public import Aeneas', 'public import Constructive').replace(
    'open Aeneas Aeneas.Std Result ControlFlow Error', 'open WebfoldModel WebfoldModel.Std Result Error')
adapted = '\n'.join(line for line in adapted.split('\n') if not line.startswith('set_option linter.'))
assert adapted.split('namespace pdf_math\n', 1)[1] == body
path.write_text(adapted)
PY

if [[ "$MODE" == --check ]]; then
    diff -u "$PROOF_DIR/PdfMath.lean" "$TMP/PdfMath.lean"
    echo "PASS: fresh Charon/Aeneas extraction matches the constructive model."
else
    cp "$TMP/PdfMath.lean" "$PROOF_DIR/PdfMath.lean"
fi
