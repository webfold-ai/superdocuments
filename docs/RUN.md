# Run — Ptyktos benchmark suite

Every benchmark across the four repos follows the same three-step
recipe:

1. **Get the data** — fetch + convert the public benchmark into
   our NDJSON schema (`{"text": ..., "labels": [...]}`).
2. **Run the backend** — closed-form via the unified harness, or
   capture + score for external models.
3. **Read the report** — three-column JSON, optionally synthesised
   into a markdown table.

## Step 1 — get the data

### agent-evals / memorybench (unified harness)

Both repos speak the same NDJSON shape. Convert a public benchmark
into NDJSON once, then run `--benchmark=<name>`:

```sh
# HuggingFace datasets example — Banking77 (decision-bench splits/).
python3 -c "
from datasets import load_dataset
ds = load_dataset('PolyAI/banking77', split='test')
with open('splits/intent/banking77.test.ndjson', 'w') as f:
    for row in ds:
        text = row['text']
        label_name = ds.features['label'].int2str(row['label'])
        f.write(json.dumps({'text': text, 'labels': [label_name]}) + '\n')
"

# HuggingFace datasets example — MMLU (agent-evals harness).
python3 -c "
from datasets import load_dataset
ds = load_dataset('cais/mmlu', 'all', split='test')
with open('mmlu.test.ndjson', 'w') as f:
    for row in ds:
        text = row['question']
        # MMLU answers are 0..3 → A..D
        letter = ['A','B','C','D'][row['answer']]
        f.write(json.dumps({
            'text': text,
            'choices': row['choices'],
            'answer': letter,
            'subject': row['subject'],
        }) + '\n')
"
```

Field-name conventions per benchmark live in
`src/harness/adapters.rs` in the agent-evals crate and
`crates/memorybench-harness/src/adapters.rs` in memorybench — see
the matching adapter's `process()` to know which JSON fields are
read.

### decision-bench (per-task)

Each capture script in `captures/capture_*.py` reads a split from
`splits/<task>/` and writes a per-row capture NDJSON. See
`captures/README.md` for the protocol.

## Step 2 — run the backend

### agent-evals

```sh
# Single benchmark:
./target/release/agent_evals --benchmark=mmlu    --dataset=mmlu.test.ndjson

# Dry run on the first 200 rows (CI gate):
./target/release/agent_evals --benchmark=mmlu    --dataset=mmlu.test.ndjson --limit 200 --dry-run
```

### memorybench

```sh
# Single benchmark (BEIR datasets, long-context, multi-hop, long-memory):
./target/release/memorybench --benchmark=beir/scifact --dataset=beir_scifact.jsonl
./target/release/memorybench --benchmark=longbench   --dataset=longbench.jsonl
./target/release/memorybench --benchmark=longmemeval --dataset=longmemeval.jsonl
```

### decision-bench

Three closed-form backends, all of which take `--split` and `--output`:

```sh
./target/release/reservoir_baseline --split  splits/intent/banking77.test.ndjson --output results/banking77.reservoir.json
./target/release/guardfold_direct   --split  splits/intent/banking77.test.ndjson --output results/banking77.guardfold.json
./target/release/verdict_engine     --split  splits/intent/banking77.test.ndjson --output results/banking77.verdict.json
```

For the external-model backends (GLiNER2.5-Decide / CLM-8B / Laya /
SemIf / etc), the recipe is **capture then score**:

```sh
# Capture (one-shot, offline):
python3 captures/capture_clm8b.py \
    --split splits/intent/banking77.test.ndjson \
    --out   captures/banking77.clm8b.jsonl

# Score:
./target/release/score_capture \
    --model   clm-8b \
    --capture captures/banking77.clm8b.jsonl \
    --split   splits/intent/banking77.test.ndjson \
    --output  results/banking77.clm8b.json
```

`compare_backends` orchestrates all of the above into one
`results/<task>.compare.md` table:

```sh
./target/release/compare_backends \
    --split         splits/intent/banking77.test.ndjson \
    --backends      reservoir,guardfold-direct,verdict-engine,gliner25-decide,laya,clm-8b \
    --captures-dir  captures/ \
    --results-dir   results/ \
    --output        results/banking77.compare.md
```

## Step 3 — read the report

Every backend writes a sidecar JSON with the three columns:

```json
{
  "backend": "verdict_engine",
  "split": "splits/intent/banking77.test.ndjson",
  "split_sha256": "<blake3 hex>",
  "exact_match_accuracy": 0.83,
  "set_f1": 0.86,
  "n": 3080,
  "mean_latency_us": 12.4,
  "p50_latency_us": 8,
  "p95_latency_us": 24,
  "baseline_backend": "reservoir_baseline",
  "baseline_exact_match_accuracy": 0.71,
  "delta_vs_baseline_pp": 12.0,
  "output_blake3": "<blake3 hex>",
  "generated_at": "2026-09-24T16:30:00Z"
}
```

Three-column rule (`REG-1` in `fold/AGENTS.md`):
- **quality** = `exact_match_accuracy`
- **speed**   = `mean_latency_us` / `p50` / `p95`
- **baseline** = `baseline_backend` + `baseline_exact_match_accuracy` +
  `delta_vs_baseline_pp`

The `output_blake3` is the BLAKE3 of the per-row scored NDJSON —
regression-gate lock. A bench re-run without changing scores
produces the same hash; a changed model or split produces a new
hash and the gate fails. (`split_sha256` is also BLAKE3 — the field
name is kept for backwards compatibility.)

See `REPORT.md` for the full report schema + per-benchmark
leaderboard comparison sheet.
