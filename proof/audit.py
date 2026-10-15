#!/usr/bin/env python3
"""Axiom audit for the superdocuments PDF and EPUB proofs (mirrors webfold's proof/audit.py).

Every authored declaration of the audited modules (private lemmas and instances
included) must have an empty transitive axiom set: no propext, Quot.sound,
Classical.choice or sorryAx. There is no axiom allowlist. Compilation failure,
a missing required declaration and a timeout all fail closed.

  proof/audit.py              full audit: spec, extraction (PdfMath) and refinement
                              (PdfProofs); fails while any of them is missing.
  proof/audit.py --spec-only  the specifications alone (PdfSpec, EpubSpec, Constructive).
"""
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent

# The loop-free kernels of proof/extract_pdf.rs (see the PdfSpec module docstring).
KERNELS = set('''xref_be_step xref_row_len row_start xref_entry_type xref_offset_ok
objstm_ref_ok objstm_header_ok objstm_span xref_prev_ok stream_extent predictor_kind
png_geometry png_recon_byte'''.split())

# Refinement theorems PdfProofs must provide: each kernel equals its field function,
# and the kernel-level statements of what must be impossible.
REFINES = {k + '_refines' for k in KERNELS} | set('''xref_kernel_be_step_no_wrap
xref_kernel_row_zero_refused xref_kernel_row_within_data xref_kernel_offset_outside_refused
xref_kernel_unknown_type_null objstm_kernel_index_at_n_refused
objstm_kernel_stream_not_direct_refused objstm_kernel_self_refused objstm_kernel_span_within
objstm_kernel_header_bounds_n xref_kernel_prev_revisit_refused xref_kernel_prev_outside_refused
stream_kernel_extent_within png_kernel_invalid_tag_refused png_kernel_recon_roundtrip
png_kernel_geometry_bits predictor_kernel_undefined_refused'''.split())

# The specification's own theorems (what must be computed and what must be impossible).
SPEC = set('''Arith.div_add_mod Arith.div_bounds
beAcc_lt beNat_lt beAcc_ge beFold_spec beFold_zero beFold_fits_8
rowLenFields_spec rowLenFields_zero_refused rowStart_within rowStart_iff rowAt_length
rowAt_append field_bytes_within entryOfRow_append
decodeEntry_code decode_w1_zero_in_use decode_unknown_null decode_gen_default
subsection_length_le subsection_rows sectionFrom_rows sectionEntries_rows
sectionEntries_bounded_by_data
lookup_newest lookup_older lookup_deleted_stays_deleted lookup_empty
entryLocation_direct resolve_direct_in_file resolve_direct_entry resolve_in_stream
resolve_no_nested_objstm resolve_free_absent resolve_null_absent resolve_compressed_gen
resolve_index_below_n objstmRefOk_iff offsetOk_iff
spanFields_within objstmHeader_bounds_n headerOk_header_fields objSpan_some_iff
objSpan_within objSpan_after_header objSpan_ordered objSpan_fields
prevOkFields_iff chain_nodup chain_in_file chain_length_le
streamExtent_within
predictorKind_cases rowBytes_bits pixelBytes_pos rowBytes_xref
paeth_mem paeth_lt predict_lt predict_none_iff addByte_lt addByte_eq_mod subByte_eq_mod
recon_filter_roundtrip recon_invalid_tag recon_lt reconFields_spec
reconRowGo_length reconRow_length reconRow_bytes pngDecode_rows unpredict_cases
examples'''.split())


# The EPUB specification's own theorems (G12; proof/EpubSpec.lean). Its kernels and their
# refinement theorems join KERNELS/REFINES once proof/extract_epub.rs is extracted.
EPUB_SPEC = set('''nth_lt childRole_cases childRole_item childRole_itemref childRole_leaf
roleFrom_ignored roleOf_item_iff roleOf_itemref_iff itemAccept_spec
firstIndex_sound firstIndex_least firstIndex_none firstIndex_of_mem countId_pos_of_mem
countId_two_of_two countId_two_witness countId_one_unique
resolveId_item_sound resolveId_item_unique resolveId_missing_iff resolveId_duplicate_iff
resolveId_item_complete indexStep_fields buildFrom_eq lookupIndex_build
resolveSpine_length resolveSpine_read_sound resolveSpine_missing resolveSpine_duplicate
resolveSpine_read_nodup resolveSpine_repeated spineCode_read codeAt_eq codeAt_not_corrupt
pathPart_clean hexVal_lt hexFields_spec pctDecode_plain pctDecode_length_le segKind_one
segKind_two removeDots_clean removeDots_plain removeDots_parent_at_root resolveHref_absolute
resolveHref_clean textBudget_accept textBudget_refuse renderGo_eq render_spec
entryBoundOk_spec entryBoundOk_of examples'''.split())


def required(full):
    names = {'PdfSpec.' + n for n in SPEC} | {'EpubSpec.' + n for n in EPUB_SPEC}
    if full:
        names |= {'pdf_math.' + n for n in KERNELS}
        names |= {'PdfProofs.' + n for n in REFINES}
    return names


def main():
    full = '--spec-only' not in sys.argv[1:]
    modules = ['Constructive', 'PdfSpec', 'EpubSpec'] + (['PdfMath', 'PdfProofs'] if full else [])
    targets = ['PdfSpec', 'EpubSpec'] + (['PdfMath', 'PdfProofs'] if full else [])
    subprocess.run(['lake', 'build'] + targets, cwd=ROOT, check=True, timeout=600)
    declarations = ', '.join('`' + n for n in sorted(required(full)))
    imports = '\n'.join('import ' + m for m in ['Lean'] + targets)
    program = imports + '''
open Lean Elab Command
run_cmd do
  let env ← getEnv
  let modules := #[MODULES]
  let required := #[ROOT_NAMES]
  let mut names : Array Name := #[]
  for (actual, _) in env.constants.toList do
    if let some idx := env.getModuleIdxFor? actual then
      if modules.contains env.header.moduleNames[idx.toNat]! then
        -- Lean records declaration ranges for authored declarations, instances,
        -- constructors and projections. Compiler-only auxiliaries without a
        -- source range are reached transitively if an audited root uses them.
        if (← findDeclarationRangesCore? actual).isSome then
          names := names.push actual
  unless names.size > 0 do throwError "No source declarations discovered"
  for name in required do
    let candidates := names.filter fun actual =>
      actual == name || privateToUserName? actual == some name
    unless candidates.size == 1 do throwError "Missing contracts: {name}"
  for actual in names do
    let axioms ← collectAxioms actual
    unless axioms.isEmpty do throwError "AXIOM AUDIT FAILED: {actual}: {axioms}"
  logInfo m!"AXIOM AUDIT PASS: {names.size} source declarations; empty transitive axiom sets (including sorryAx)."
'''.replace('MODULES', ', '.join('`' + m for m in modules)).replace('ROOT_NAMES', declarations)
    with tempfile.TemporaryDirectory(prefix='pdf-axioms-') as tmp:
        path = pathlib.Path(tmp) / 'Audit.lean'
        path.write_text(program)
        subprocess.run(['lake', 'env', 'lean', str(path)], cwd=ROOT, check=True, timeout=300)


if __name__ == '__main__':
    main()
