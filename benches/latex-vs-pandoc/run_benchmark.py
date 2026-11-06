#!/usr/bin/env python3
"""Benchmark and agreement check: superdocuments LaTeX and BibTeX readers vs pandoc 3.x.

Usage: run_benchmark.py [--runs 20] [--out DIR]

Commands run (pandoc 3.1.3 on this machine):
  pandoc -f latex -t json <file.tex>        structure for the agreement check
  pandoc -f latex -t markdown <file.tex>    timing
  pandoc -f latex -t plain <file.tex>       timing
  pandoc -f bibtex -t csljson <file.bib>    structure and timing

Ours (examples/latex_bench.rs, built with `cargo build --locked --example latex_bench`):
  latex_bench counts <file.tex>, latex_bench markdown <file.tex>, latex_bench time <file.tex> N
  latex_bench bib-counts <file.bib>, latex_bench bib-csl <file.bib>, latex_bench bib-time <file.bib> N

Timing: the median wall-clock time of N runs of each command line (process start-up included),
run one after another on one core with `taskset` when available. Ours also reports the median
in-process time of parse plus Markdown rendering (no process start-up).

Agreement: each metric is compared between pandoc's JSON and our counts. Every disagreement is
written to disagreements.txt; the explanation for each is written separately, by hand, after
reading the source and the LaTeX or BibTeX rule that applies.
"""
import glob
import json
import os
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
TARGET = os.environ.get("CARGO_TARGET_DIR", "/home/user/sd-target-latex")
BENCH = os.path.join(TARGET, "debug", "examples", "latex_bench")


def arg(name, default):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


RUNS = int(arg("--runs", "20"))
OUT = arg("--out", HERE)
TASKSET = shutil.which("taskset")


PANDOC_GUARD = ["sh", "-c", 'ulimit -v 4000000; exec timeout 60 "$@"', "guard"]


def pandoc(args):
    """Runs pandoc under a memory ceiling and a timeout, so one bad input cannot exhaust the machine."""
    return PANDOC_GUARD + args


def timed(cmd):
    """Median wall time of RUNS runs, in milliseconds, and the last stdout."""
    if cmd and cmd[0] == "pandoc":
        cmd = pandoc(cmd)
    if TASKSET:
        cmd = [TASKSET, "-c", "0"] + cmd
    samples = []
    last = ""
    for _ in range(RUNS):
        t = time.perf_counter()
        p = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO)
        samples.append((time.perf_counter() - t) * 1000)
        last = p.stdout
        if p.returncode != 0:
            return None, p.stderr.strip()[:300]
    return statistics.median(samples), last


def describe_failure(p):
    """A short reason for a failed pandoc run: a kill or timeout code, or pandoc's own message."""
    if p.returncode in (124,):
        return "timeout after 60 s"
    if p.returncode in (137, 251):
        return f"exit {p.returncode}: out of memory under the 4 GB ceiling"
    return (p.stderr.strip() or f"exit {p.returncode}")[:300]


def pandoc_json(path, fmt):
    p = subprocess.run(pandoc(["pandoc", "-f", fmt, "-t", "json", path]), capture_output=True, text=True)
    if p.returncode != 0:
        return None, describe_failure(p)
    return json.loads(p.stdout), None


def walk(node, visit):
    if isinstance(node, dict):
        visit(node)
        for v in node.values():
            walk(v, visit)
    elif isinstance(node, list):
        for v in node:
            walk(v, visit)


def pandoc_counts(doc):
    c = {"header": 0, "display_math": 0, "inline_math": 0, "bullet": 0, "ordered": 0, "definition": 0,
         "bullet_items": 0, "ordered_items": 0, "definition_items": 0, "table": 0, "figure": 0,
         "figure_with_caption": 0, "figure_captions": [], "citations": 0, "note": 0, "link": 0,
         "image": 0, "plain_text_refs": 0}

    def visit(d):
        t = d.get("t")
        cc = d.get("c")
        if t == "Header":
            c["header"] += 1
        elif t == "Math" and isinstance(cc, list):
            if cc[0].get("t") == "DisplayMath":
                c["display_math"] += 1
            else:
                c["inline_math"] += 1
        elif t == "BulletList":
            c["bullet"] += 1
            c["bullet_items"] += len(cc)
        elif t == "OrderedList":
            c["ordered"] += 1
            c["ordered_items"] += len(cc[1])
        elif t == "DefinitionList":
            c["definition"] += 1
            c["definition_items"] += len(cc)
        elif t == "Table":
            c["table"] += 1
        elif t == "Figure":
            c["figure"] += 1
            caption_blocks = cc[1][1] if isinstance(cc[1], list) and len(cc[1]) > 1 else []
            text = json.dumps(caption_blocks)
            if '"t": "Str"' in text or '"t":"Str"' in text:
                c["figure_with_caption"] += 1
            c["figure_captions"].append(text[:0])
        elif t == "Cite":
            c["citations"] += len(cc[0])
        elif t == "Note":
            c["note"] += 1
        elif t == "Link":
            c["link"] += 1
        elif t == "Image":
            c["image"] += 1
    walk(doc.get("blocks", []), visit)
    return c


def meta_has(doc, key):
    v = doc.get("meta", {}).get(key)
    if not v:
        return False
    return json.dumps(v) not in ('{"t": "MetaList", "c": []}', '{"t":"MetaList","c":[]}', '{"t": "MetaInlines", "c": []}')


def run_latex(path, name):
    ours = json.loads(subprocess.run([BENCH, "counts", path], capture_output=True, text=True, cwd=REPO).stdout or "{}")
    pdoc, err = pandoc_json(path, "latex")
    row = {"file": name, "pandoc_error": err, "ours_counts": ours}
    if pdoc is None:
        return row, []
    pc = pandoc_counts(pdoc)
    row["pandoc_counts"] = {k: v for k, v in pc.items() if k != "figure_captions"}
    pairs = [
        ("sections", ours.get("sections"), pc["header"]),
        ("display_math", ours.get("display_math"), pc["display_math"]),
        ("inline_math", ours.get("inline_math"), pc["inline_math"]),
        ("lists", ours.get("lists"), pc["bullet"] + pc["ordered"] + pc["definition"]),
        ("list_items", ours.get("list_items"), pc["bullet_items"] + pc["ordered_items"] + pc["definition_items"]),
        ("tables", ours.get("tables"), pc["table"]),
        ("figures_top_level", ours.get("figures_top_level"), pc["figure"]),
        ("figures_with_caption", ours.get("figures_with_caption"), pc["figure_with_caption"]),
        ("cite_keys", len(ours.get("cite_keys", [])), pc["citations"]),
        ("footnotes", ours.get("footnotes"), pc["note"]),
        ("links_and_resolved_refs", ours.get("links", 0) + ours.get("refs", 0), pc["link"]),
        ("title", 1 if ours.get("title") else 0, 1 if meta_has(pdoc, "title") else 0),
        ("abstract", 1 if ours.get("abstract") else 0, 1 if meta_has(pdoc, "abstract") else 0),
        ("authors", ours.get("authors"), len(pdoc.get("meta", {}).get("author", {}).get("c", []))
            if isinstance(pdoc.get("meta", {}).get("author"), dict) else 0),
    ]
    diffs = []
    for metric, o, p in pairs:
        if o != p:
            diffs.append({"file": name, "metric": metric, "ours": o, "pandoc": p})
    return row, diffs


def run_bib(path, name):
    ours = json.loads(subprocess.run([BENCH, "bib-counts", path], capture_output=True, text=True, cwd=REPO).stdout or "{}")
    p = subprocess.run(pandoc(["pandoc", "-f", "bibtex", "-t", "csljson", path]), capture_output=True, text=True)
    row = {"file": name, "ours_entries": ours.get("entries"), "pandoc_error": None if p.returncode == 0 else describe_failure(p)}
    diffs = []
    if p.returncode != 0:
        return row, diffs
    pandoc_entries = json.loads(p.stdout)
    row["pandoc_entries"] = len(pandoc_entries)
    if len(pandoc_entries) != ours.get("entries"):
        diffs.append({"file": name, "metric": "entries", "ours": ours.get("entries"), "pandoc": len(pandoc_entries)})
    by_key = {e["key"]: e for e in ours.get("per_entry", [])}
    for pe in pandoc_entries:
        oe = by_key.get(pe.get("id"))
        if oe is None:
            diffs.append({"file": name, "metric": "key", "ours": None, "pandoc": pe.get("id")})
            continue
        pauthors = pe.get("author", [])
        def typographic(s):
            return s.replace("\u2019", "'") if isinstance(s, str) else s
        pdoi = pe.get("DOI")
        if isinstance(pdoi, str):
            pdoi = pdoi.replace("https://doi.org/", "")
        checks = [
            ("authors", oe["authors"], len(pauthors)),
            ("first_family", typographic(oe["first_family"]),
             typographic((pauthors[0].get("family") or pauthors[0].get("literal")) if pauthors else None)),
            ("year", oe["year"], (pe.get("issued", {}).get("date-parts", [[None]])[0][0] if pe.get("issued") else None)),
            ("pages", oe["pages"] > 0, "page" in pe),
            ("doi", oe["doi"], pdoi),
        ]
        for metric, o, pv in checks:
            if o != pv:
                diffs.append({"file": f"{name}:{pe.get('id')}", "metric": metric, "ours": o, "pandoc": pv})
    return row, diffs


def main():
    subprocess.run(["cargo", "build", "--locked", "--example", "latex_bench"], cwd=REPO, check=True,
                   env=dict(os.environ, CARGO_TARGET_DIR=TARGET), capture_output=True)
    tex_files = sorted(glob.glob(os.path.join(REPO, "tests/fixtures/latex/*.tex")))
    # Golden outputs in the fixture directories are not inputs.
    bib_files = sorted(f for f in glob.glob(os.path.join(REPO, "tests/fixtures/bibtex/*.bib")) if ".expected." not in f)
    results = {"runs": RUNS, "pandoc": subprocess.run(["pandoc", "--version"], capture_output=True, text=True).stdout.splitlines()[0],
               "latex": [], "bib": []}
    all_diffs = []
    for path in tex_files:
        name = os.path.relpath(path, REPO)
        row, diffs = run_latex(path, name)
        all_diffs.extend(diffs)
        pm, _ = timed(["pandoc", "-f", "latex", "-t", "markdown", path])
        pp, _ = timed(["pandoc", "-f", "latex", "-t", "plain", path])
        om, _ = timed([BENCH, "markdown", path])
        ip = subprocess.run([BENCH, "time", path, str(RUNS)], capture_output=True, text=True, cwd=REPO).stdout.strip()
        row["timing_ms"] = {
            "pandoc_markdown_cli": pm, "pandoc_plain_cli": pp,
            "ours_markdown_cli": om, "ours_parse_render_in_process": int(ip) / 1e6 if ip.isdigit() else None,
        }
        results["latex"].append(row)
        print(f"{name}: diffs={len(diffs)} pandoc={pm and round(pm, 1)}ms ours_cli={om and round(om, 1)}ms "
              f"ours_inproc={row['timing_ms']['ours_parse_render_in_process'] and round(row['timing_ms']['ours_parse_render_in_process'], 3)}ms",
              flush=True)
    for path in bib_files:
        name = os.path.relpath(path, REPO)
        row, diffs = run_bib(path, name)
        all_diffs.extend(diffs)
        pm, _ = timed(["pandoc", "-f", "bibtex", "-t", "csljson", path])
        om, _ = timed([BENCH, "bib-csl", path])
        ip = subprocess.run([BENCH, "bib-time", path, str(RUNS)], capture_output=True, text=True, cwd=REPO).stdout.strip()
        row["timing_ms"] = {
            "pandoc_csljson_cli": pm, "ours_csljson_cli": om,
            "ours_parse_csl_in_process": int(ip) / 1e6 if ip.isdigit() else None,
        }
        results["bib"].append(row)
        print(f"{name}: diffs={len(diffs)} pandoc={pm and round(pm, 1)}ms ours_cli={om and round(om, 1)}ms", flush=True)
    results["disagreements"] = all_diffs
    with open(os.path.join(OUT, "results.json"), "w") as f:
        json.dump(results, f, indent=2, ensure_ascii=False)
    with open(os.path.join(OUT, "disagreements.txt"), "w") as f:
        for d in all_diffs:
            f.write(f"{d['file']}\t{d['metric']}\tours={d['ours']}\tpandoc={d['pandoc']}\n")
    print(f"{len(all_diffs)} disagreements written", flush=True)


if __name__ == "__main__":
    main()
