# Report — three-column shape + BLAKE3 locks

Every backend in every benchmark repo produces the same three-column
shape, plus a per-row scored NDJSON sidecar and a BLAKE3 lock.

## The three columns

Per `fold/AGENTS.md` `REG-1`: every claim carries quality, speed,
baseline.

| column | field | meaning |
|---|---|---|
| **quality** | `exact_match_accuracy` (decision-bench) / `accuracy` (agent-evals) / `top1` (memorybench) | headline metric, closed-form, no model in scoring path |
| **speed** | `mean_latency_us` / `p50_latency_us` / `p95_latency_us` | wall-clock per row, single-threaded |
| **baseline** | `baseline_backend` + `baseline_exact_match_accuracy` + `delta_vs_baseline_pp` | REG-4 reservoir or self-comparison; the "X beats floor by N pp" statement |

## Sidecar JSON shape

```json
{
  "backend": "verdict_engine",
  "split": "splits/intent/banking77.test.ndjson",
  "split_sha256": "<blake3 hex of the split file>",
  "exact_match_accuracy": 0.83,
  "set_f1": 0.86,
  "n": 3080,
  "mean_latency_us": 12.4,
  "p50_latency_us": 8,
  "p95_latency_us": 24,
  "baseline_backend": "reservoir_baseline",
  "baseline_exact_match_accuracy": 0.71,
  "delta_vs_baseline_pp": 12.0,
  "output_blake3": "<blake3 hex of the .scored.ndjson sidecar>",
  "generated_at": "2026-09-24T16:30:00Z"
}
```

`split_sha256` is BLAKE3 (field name kept for backwards
compatibility). Both hashes anchor the regression gate.

## Per-row scored NDJSON sidecar

`results/<split_stem>.<backend>.scored.ndjson` — one JSON object per
scored row:

```json
{"text": "...", "predicted": ["..."], "gold": ["..."], "exact_match": true, "confidence": 0.83, "latency_us": 12}
```

Use this for per-row drill-down — e.g., "why did the model miss
this one?".

## Regression gate (CI)

```sh
# In CI:
for split in splits/intent/*.ndjson; do
  for backend in reservoir guardfold-direct verdict-engine; do
    ./target/release/$backend --split "$split" --output "results/$(basename $split .ndjson).$backend.json"
    # Compare the locked BLAKE3 hash against the committed baseline:
    locked=$(jq -r .output_blake3 "results/$(basename $split .ndjson).$backend.json")
    if [ "$locked" != "$(cat results/locks/$(basename $split .ndjson).$backend.lock)" ]; then
      echo "REGRESSION: $backend on $split"
      exit 1
    fi
  done
done
```

## Markdown table — `compare_backends`

`decision-bench`'s `compare_backends` binary synthesises a
human-readable markdown table from a directory of report JSONs:

```markdown
| backend | quality (EM) | set-F1 | speed (mean µs) | speed (p95 µs) | baseline | delta (pp) |
|---------|--------------|--------|----------------:|---------------:|----------|-----------:|
| reservoir | 0.7100 | 0.7600 | 57000.2 | 58116 | self: 0.7100 | +0.0000 |
| guardfold-direct | 0.7800 | 0.8200 | 3.8 | 6 | reservoir: 0.7100 | +0.0700 |
| verdict-engine | 0.8300 | 0.8600 | 14.0 | 37 | reservoir: 0.7100 | +0.1200 |
```

`agent-evals` / `memorybench` print a similar table to stdout (no
synthesis step needed; one invocation per benchmark).

## Reading the published-leaderboard comparison

`decision-bench/results/published_compare.md` keeps the
hand-curated comparison of our closed-form numbers against published
leaderboards for each benchmark that has one. Anchor it once a
capture run completes; update whenever a new external-model backend
publishes a number.

The headline published comparison (Fast Decisions, Fastino's own
table, verbatim):

| Model | Type | Fast Decisions avg |
|---|---|---:|
| GLiNER2.5-Decide | 340M encoder | 60.1% |
| JevK5 | 4B-class Qwen3.5 decoder | 57.5% |
| SemIf | Qwen3.5-4B decoder | 56.4% |
| GLiFormer large-v1 | Single-pass encoder | 49.0% |
| Laya | 421M ModernBERT encoder | 46.6% |
| **CLM-8B** (new) | 8B Contrastive LM System One | **? — capture in `results/fast_decisions.clm-8b.json`** |
