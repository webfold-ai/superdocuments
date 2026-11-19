# Calamine with Apeiron workbook maps

This is **our** crate, not a vendored copy of upstream `calamine`. It keeps
calamine's parser and public API, and its storage is the shared substrate: XLSX and
XLSB ZIP part-path caches and workbook relationship indexes are
`apeiron::freq::Table` with borrowed `&str` lookups, on `zip` 9 and `quick-xml`
0.42. XLS and ODS readers are unchanged controls.

History: it was imported from `calamine-apeiron` (`claude/zip9`, `3ad21be`) as the
`calamine/` workspace member, and has been re-based on the substrate revisions the
rest of this workspace shares since.

Two things that need stating because they look like upstream's and are not:

- **Version.** This tree is versioned `0.38.0` because RUSTSEC-2026-0436 is keyed to
  the calamine release line and only clears at `>= 0.38.0`. That number is not a
  claim that upstream's 0.37/0.38 API changes are here — they are not.
- **The CFB/OLE2 fix** (`src/cfb.rs`) *is* upstream's `b8ec4f0` (`tafia/calamine#762`),
  applied verbatim, with upstream's two crafted fixtures and tests now in
  `tests/test.rs`. It is the one place where upstream's code was carried in whole,
  so it is cited rather than paraphrased.

The optional `chrono`/`dates` features were removed (RUSTSEC-2026-0543: chrono is
unmaintained with no patched release, and nothing here enabled the feature while
Cargo locked it into every audit). See `Changelog.md`.

One addition to the XLS reader: `Xls::cell_number_format(sheet, (row, col))` returns the number format id
and custom format code (XF and FORMAT records) of a numeric cell, so a caller can apply the same number
format as for XLSX. Nothing else in the parser changed.

The `apeiron` dependency resolves from `webfold-ai/apeiron` (`apeiron::freq::Table`). This code lives in the superdocuments workspace as `calamine/`
rather than as a redistributable standalone package. The upstream Calamine source is MIT; Apeiron has its own
license in that checkout, which must be reviewed before external distribution.

Validation:

```bash
cargo test --lib
cargo test --test test
../spbench/webfold-structured/run.sh calamine \
  --fixtures /tmp/webfold-upstreams/calamine/tests \
  --out ../webfold-bench/bench/results/calamine-local.json \
  --warmup 2 --samples 7 --iterations 20
```

The last command compiles the pinned upstream and this fork into one Rust
runner. It compares ordered sheet names, range bounds and every cell value,
then times both tools with alternating execution order in hot and fresh-process
cold paths. A fixture that fails identically on both sides is logged as an
exclusion. Any output difference aborts the run.
