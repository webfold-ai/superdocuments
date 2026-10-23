# Calamine with Apeiron workbook maps

This local fork starts at Calamine `0af05f4f6030351e3b8a999ea0810c8618368776`
(v0.36.1). It keeps Calamine's parser and public API. XLSX and XLSB ZIP part
path caches and workbook relationship indexes use `apeiron::Map` with borrowed
`&str` lookups. XLS and ODS readers are unchanged controls. This is an initial
storage migration, not a rewrite of spreadsheet parsing or a claimed speedup.

The `apeiron` dependency resolves from the `apeiron` branch of Ptyktos/hashtrinity
(commit a2b5093). This crate lives in the superdocuments workspace as `calamine/`
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
