# Setup — Ptyktos benchmark suite

The Ptyktos benchmark suite spans four repos, all of which share a
common substrate (`memoryfold`, `modelfold`, `hashtrinity`).

## Repositories

| repo | purpose | harness shape |
|---|---|---|
| `Ptyktos/agent-evals` | agents / code / reasoning / safety / vision / medical / financial / ops | one binary, 110+ adapters (`./target/release/agent_evals --list`) |
| `Ptyktos/memorybench` | memory / retrieval / long-context | one binary + 30 adapters (`./target/release/memorybench --list`) |
| `Ptyktos/decision-bench` | decision / routing / intent / guardrails / NLI / LLM-as-judge | three binaries (`reservoir_baseline`, `guardfold_direct`, `verdict_engine`) + capture-based for external models |
| `Ptyktos/superdocuments` | shared document parsing — also the home of this guide | n/a |

(Plus the legacy `guardfold/benches/jev-as-judge/` for the JEV
reproduction, and the autofold draft worktree at
`ptyktos/drafts/autofold-vs-auto/standalone/autofold-standalone/`
which vendored its own substrate — runs standalone.)

## Prerequisites

```sh
# Rust 1.78+ (memorybench pins 1.78; agent-evals / decision-bench work on stable)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable && rustup update

# Optional but recommended:
# - A CUDA-capable GPU + CUDA 12.x toolchain for the capture-based
#   external-model backends (GLiNER2.5-Decide / CLM-8B / JevK5 / etc).
#   CPU is supported for the headline Fastino GLiNER family (340M / 287M).
# - Python 3.10+ for the capture scripts in
#   decision-bench/captures/.

# HuggingFace token (only if you're going to run capture scripts that
# pull gated models — JevK5, SemIf, etc):
export HF_TOKEN=...

# OpenJev / TypeSafe API keys (only if you're going to capture those
# backends — most are open-weight and free):
export OPENJEV_API_KEY=...   # optional
export TYPESAFE_API_KEY=...  # optional; publish-only fallback exists
```

## Clone + build

```sh
mkdir -p ~/projects && cd ~/projects
git clone https://github.com/Ptyktos/agent-evals.git
git clone https://github.com/Ptyktos/memorybench.git
git clone https://github.com/Ptyktos/decision-bench.git
git clone https://github.com/Ptyktos/superdocuments.git

cd agent-evals && cargo build --release && cd ..
cd memorybench && cargo build --release && cd ..
cd decision-bench && cargo build --release && cd ..
cd superdocuments && cargo build --release && cd ..
```

(`decision-bench` and `agent-evals` are standalone crates;
`memorybench` is a Cargo workspace — `cargo build --release` at the
workspace root builds all member crates including the new
`memorybench-harness` unified binary.)

## Verify

```sh
# agent-evals: should print 56+ adapters grouped by family.
~/projects/agent-evals/target/release/agent_evals --list

# memorybench: should print 30 adapters grouped by family.
~/projects/memorybench/target/release/memorybench --list

# decision-bench: three closed-form backends + score_capture.
ls ~/projects/decision-bench/target/release/{reservoir_baseline,guardfold_direct,verdict_engine,score_capture,compare_backends}

# superdocuments: cargo test runs the in-tree doc-parse unit tests.
cd ~/projects/superdocuments && cargo test
```

## Layout convention

Every benchmark repo uses the same `docs/` shape:

```
docs/
  SETUP.md     ← this file (and a per-repo pointer to it)
  RUN.md       ← benchmark run guide
  EVAL.md      ← adding adapters, evaluating, recording results
  REPORT.md    ← how to read the three-column reports + BLAKE3 locks
```

If a repo's `docs/` is missing the canonical `RUN.md` / `EVAL.md` /
`REPORT.md`, they live here in superdocuments and the per-repo
README points back at them. (Ptyktos convention — superdocuments is
the documentation home for the bench suite.)

## Related work — peer agents and benchmarks

These projects complement the Ptyktos bench suite; not benchmarks we
adopt directly, but peer agent implementations whose eval surfaces
are worth knowing about.

- **[synthetic-sciences/openscience](https://github.com/synthetic-sciences/openscience)** —
  the open-source AI workbench for scientific research. Bundled
  skills across biology / chemistry / physics / ML, connectors to
  ChEMBL / UniProt / PubMed / arXiv. The companion
  `evals/science-harness/` ships a research-agent eval layer that
  wraps Harbor 0.22.0 and runs against Terminal-Bench science tasks,
  BixBench3, ResearchClawBench, and DrugDiscoveryBench.
