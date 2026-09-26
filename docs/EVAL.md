# Eval — adding adapters, evaluating, recording results

Every benchmark across the four repos follows the **adapter pattern**:
one small struct + impl + append to the registry. No `Cargo.toml`
change, no new bin, no new crate.

## Adding a benchmark to `agent-evals`

```rust
// agent-evals/src/harness/adapters.rs

pub struct MyBenchAdapter;
impl Adapter for MyBenchAdapter {
    fn name(&self) -> &'static str { "my_bench" }
    fn family(&self) -> &'static str { "code" }
    fn n_domains(&self) -> usize { 0 } // 0 = no bucketing
    fn process(&self, row: &serde_json::Value) -> Outcome {
        let candidate = row.get("prediction").and_then(Value::as_str).unwrap_or("");
        let gold = row.get("answer").and_then(Value::as_str).unwrap_or("");
        Outcome {
            correct: substring_correct(candidate, gold),
            scored: true,
            ..Default::default()
        }
    }
}
```

Then in `pub fn all()`:

```rust
pub fn all() -> Vec<&'static dyn Adapter> {
    vec![
        // ... existing ...
        &MyBenchAdapter,
    ]
}
```

That's it. `cargo build --release && ./target/release/agent_evals --list`
will show the new adapter under its family. Run it with:

```sh
./target/release/agent_evals --benchmark=my_bench --dataset=my_bench.test.ndjson
```

## Adding a benchmark to `memorybench`

Same pattern in `crates/memorybench-harness/src/adapters.rs`:

```rust
pub struct MyMemoryBenchAdapter;
impl Adapter for MyMemoryBenchAdapter {
    fn name(&self) -> &'static str { "my_memory_bench" }
    fn family(&self) -> &'static str { "long_context" }
    fn process(&self, row: &Value) -> Outcome {
        let candidate = row.get("prediction").and_then(Value::as_str).unwrap_or("");
        let gold = row.get("answer").and_then(Value::as_str).unwrap_or("");
        Outcome {
            top1_hit: !gold.is_empty() && candidate.to_lowercase().contains(&gold.to_lowercase()),
            graded: 0.0,
            latency_ms: 0.0,
        }
    }
}
```

## Adding a backend to `decision-bench`

Two paths:

### Closed-form backend (new model under test)

```rust
// decision-bench/src/<my_backend>.rs + bin/<my_backend>.rs
//
// Reuse the harness helpers in src/lib.rs:
//   decision_bench::captures::load_split
//   decision_bench::judge::{exact_match, set_f1}
//   decision_bench::report::{aggregate, sha256_hex, write_scored_ndjson, blake3_hex}
//   decision_bench::schema::ScoredRow
//
// Pattern: load split → 80/20 cut → train on train → predict on test →
// build ScoredRow per row → write sidecar JSON with the three columns.
```

### Capture-based backend (external model)

```python
# decision-bench/captures/capture_<model>.py
#
# Reads NDJSON, runs the model once per row, writes capture NDJSON.
# The Rust `score_capture` bin reads the capture and scores it
# against the gold split.
```

Follow the protocol in `decision-bench/captures/README.md`.

## Recording results

Three-column results land in:

- `agent-evals` — stdout (one row per benchmark invocation)
- `memorybench` — stdout (one row per benchmark invocation)
- `decision-bench` — `results/<split_stem>.<backend>.json` +
  `results/<split_stem>.<backend>.scored.ndjson` +
  `results/<split_stem>.compare.md` (markdown table)

**Commit convention**: every result JSON is committed. The BLAKE3
lock means a regression is detected by `git diff` showing a hash
change. CI runs `--dry-run --limit 200` to gate PRs; the full run
is the bench leaderboard.

## Adding a benchmark to the published-leaderboard comparison

`decision-bench/results/published_compare.md` is the hand-curated
comparison of our closed-form numbers against published
leaderboards. When you add a benchmark that has a published
leaderboard (Banking77, MMLU, BFCL, etc), add a row:

```markdown
| Banking77 | 77-intent banking queries | accuracy (closed-form) | closed-form: 0.71, GLiNER2.5-Decide: ?, Laya: ? | tbd | closed-form beats REG-4 reservoir by N pp |
```

The `?` entries are filled in once a capture is run; the row is
the regression gate.

## Eval checklist (per benchmark)

- [ ] NDJSON converted from the public dataset (HuggingFace, paper
  download, etc), one row per example, schema per `splits/fast_decisions/schema.json`
  for decision-bench or the per-adapter `process()` signature
  for agent-evals / memorybench.
- [ ] Adapter implemented + appended to `all()`.
- [ ] Adapter appears in `--list`.
- [ ] First dry-run produces sensible accuracy (compare to published
  floor; flag in `swot.md` if the closed-form is below the reservoir
  baseline).
- [ ] Captures run for external-model backends (decision-bench).
- [ ] Three-column report committed to `results/`.
- [ ] Published-leaderboard row added (if applicable).
