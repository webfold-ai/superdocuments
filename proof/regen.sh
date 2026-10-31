#!/usr/bin/env bash
# Regenerate (or, with --check, verify) the Charon/Aeneas extractions of the loop-free kernels
# that production calls: proof/PdfMath.lean (proof/extract_pdf.rs, crate pdf_math) and
# proof/EpubMath.lean (proof/extract_epub.rs, crate epub_math), each adapted to the
# self-contained Constructive prelude exactly as webfold's proof/regen.sh does.
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

# The kernel surfaces the specifications fix; a change here is a specification change.
PDF_KERNELS="xref_be_step xref_row_len row_start xref_entry_type xref_offset_ok objstm_ref_ok objstm_header_ok objstm_span xref_prev_ok stream_extent predictor_kind png_geometry png_recon_byte"
EPUB_KERNELS="opf_child_role manifest_item_accept manifest_index_step spine_step href_byte_class pct_hex path_segment_kind epub_text_budget zip_entry_bound"

TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

# extract <source.rs> <crate> <kernels> <output-name.lean>
extract() {
    local source="$1" crate="$2" kernels="$3" output="$4"
    if [[ ! -f "$PROOF_DIR/$source" ]]; then
        echo "missing proof/$source (the kernels its specification names)" >&2
        exit 1
    fi
    local start=() k
    for k in $kernels; do start+=(--start-from "crate::$k"); done
    local work="$TMP/$crate"
    mkdir -p "$work"

    echo "== extracting constructive kernels of proof/$source =="
    "$CHARON" rustc --sysroot default --preset aeneas \
        --dest-file "$work/$crate.llbc" "${start[@]}" \
        -- --crate-type lib --crate-name "$crate" "$PROOF_DIR/$source"
    "$AENEAS" -backend lean -dest "$work" -warnings-as-errors -no-progress-bar \
        "$work/$crate.llbc"

    python3 - "$work/$output" "$crate" "$kernels" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
crate = sys.argv[2]
raw = path.read_text()
assert raw.count('public import Aeneas') == 1
assert raw.count('open Aeneas Aeneas.Std Result ControlFlow Error') == 1
body = raw.split('namespace ' + crate + '\n', 1)[1]
expected = set(sys.argv[3].split())
assert set(re.findall(r'^def (\w+)', body, re.M)) == expected, 'Kernel surface changed; review required'
assert not re.search(r'\b(sorry|axiom|opaque|unsafe|partial_fixpoint|noncomputable)\b|Result\.div', body), 'Unsupported extraction'
adapted = raw.replace('public import Aeneas', 'public import Constructive').replace(
    'open Aeneas Aeneas.Std Result ControlFlow Error', 'open WebfoldModel WebfoldModel.Std Result Error')
adapted = '\n'.join(line for line in adapted.split('\n') if not line.startswith('set_option linter.'))
assert adapted.split('namespace ' + crate + '\n', 1)[1] == body
path.write_text(adapted)
PY

    if [[ "$MODE" == --check ]]; then
        diff -u "$PROOF_DIR/$output" "$work/$output"
        echo "PASS: fresh Charon/Aeneas extraction of $source matches the constructive model."
    else
        cp "$work/$output" "$PROOF_DIR/$output"
    fi
}

extract extract_pdf.rs pdf_math "$PDF_KERNELS" PdfMath.lean
extract extract_epub.rs epub_math "$EPUB_KERNELS" EpubMath.lean
