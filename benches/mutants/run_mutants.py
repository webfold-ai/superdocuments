#!/usr/bin/env python3
"""Runs the hand-made mutants in mutants.py against the module's unit tests.

Usage: run_mutants.py <bibtex|latex> [--only ID,ID]

For each mutant: apply one exact replacement (the selector must match once), run
`cargo test --locked --lib <module>` with a timeout, restore the original text, and record one
of: DETECTED (a test failed), DETECTED-TIMEOUT (the tests did not finish), NOT-VIABLE (the
mutant does not compile), SURVIVED (all tests passed). The file is checked against git after
each mutant, so a restore failure cannot go unnoticed.
"""
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import mutants  # noqa: E402

TEST_TIMEOUT = 420


def cargo_test(module):
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", "/home/user/sd-target-latex")
    try:
        p = subprocess.run(
            ["cargo", "test", "--locked", "--lib", module],
            cwd=REPO, env=env, capture_output=True, text=True, timeout=TEST_TIMEOUT,
        )
        return p.returncode, p.stdout + p.stderr, False
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return 1, out, True


def main():
    module = sys.argv[1]
    only = None
    if "--only" in sys.argv:
        only = set(sys.argv[sys.argv.index("--only") + 1].split(","))
    table = mutants.BIBTEX if module == "bibtex" else mutants.LATEX
    results = []
    for mid, path, old, new, desc in table:
        if only and mid not in only:
            continue
        full = os.path.join(REPO, path)
        with open(full) as f:
            original = f.read()
        if original.count(old) != 1:
            results.append((mid, "BAD-SELECTOR", desc, ""))
            continue
        started = time.time()
        try:
            with open(full, "w") as f:
                f.write(original.replace(old, new, 1))
            code, out, timed_out = cargo_test(module)
        finally:
            with open(full, "w") as f:
                f.write(original)
        check = subprocess.run(["git", "diff", "--quiet", "--", path], cwd=REPO)
        if check.returncode != 0:
            print(f"RESTORE CHECK FAILED for {path}", file=sys.stderr)
            sys.exit(3)
        elapsed = time.time() - started
        failing = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
        if timed_out:
            status = "DETECTED-TIMEOUT"
        elif "could not compile" in out or re.search(r"^error(\[E\d+\])?:", out, re.M) and "test result" not in out:
            status = "NOT-VIABLE"
        elif code != 0:
            status = "DETECTED"
        else:
            status = "SURVIVED"
        detail = ", ".join(failing[:4]) if failing else ""
        results.append((mid, status, desc, f"{elapsed:.0f}s {detail}".strip()))
        print(f"{mid} {status:17} {desc} [{elapsed:.0f}s] {detail}", flush=True)
    counts = {}
    for _, status, _, _ in results:
        counts[status] = counts.get(status, 0) + 1
    print("SUMMARY", module, counts, flush=True)
    out_path = os.path.join(HERE, f"results-{module}.txt")
    with open(out_path, "w") as f:
        for mid, status, desc, detail in results:
            f.write(f"{mid}\t{status}\t{desc}\t{detail}\n")
        f.write(f"SUMMARY\t{counts}\n")


if __name__ == "__main__":
    main()
