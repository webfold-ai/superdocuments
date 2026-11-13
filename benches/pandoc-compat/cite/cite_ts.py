#!/usr/bin/env python3
"""Stress corpus from the CSL project's citeproc test suite (processor-tests/humans, 847 tests;
https://github.com/citation-style-language/test-suite, cloned 2026-11-13). The suite is a set of edge
cases: every test brings its own CSL style and a few items. Each test is turned into a Pandoc Markdown
document (its citation clusters, or `nocite: '@*'` for bibliography tests), a CSL file and a CSL-JSON file,
and Pandoc 3.1.3 (`--citeproc`) is the reference, exactly as in cite_bench.py. The suite's own expected
results are NOT used here: they are what citeproc-js prints, and Pandoc does not always agree with them.
Tests Pandoc cannot process are listed as excluded.

  cite_ts.py gen SUITE_DIR | oracle | run BIN | report
"""
import glob
import json
import os
import re
import subprocess
import sys
import concurrent.futures as cf

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import cite_bench as cb  # noqa: E402

OUT = os.path.join(HERE, "suite")
LABELS = {"page": "p.", "chapter": "chap.", "section": "sec.", "volume": "vol.", "figure": "fig.", "line": "l.",
          "note": "n.", "paragraph": "para.", "part": "pt.", "verse": "v.", "column": "col.", "folio": "fol.",
          "book": "bk.", "opus": "op.", "number": "no.", "issue": "no."}


def sections(text):
    out = {}
    for m in re.finditer(r">>=+ ([A-Z-]+) =+>>\n(.*?)\n?<<=+ \1 =+<<", text, re.S):
        out[m.group(1)] = m.group(2)
    return out


def md_item(it):
    if it.get("author-only"):
        return None
    s = ""
    if it.get("prefix"):
        s += re.sub(r"<[^>]+>", "", it["prefix"])
        if not s.endswith(" "):
            s += " "
    s += ("-" if it.get("suppress-author") else "") + "@" + str(it["id"])
    loc = it.get("locator")
    if loc:
        lab = LABELS.get(it.get("label", "page"), "p.")
        s += ", %s %s" % (lab, loc)
    if it.get("suffix"):
        suf = re.sub(r"<[^>]+>", "", it["suffix"])
        s += (" " if not suf.startswith((",", ";", ".")) else "") + suf
    return s


def cmd_gen(suite):
    os.makedirs(OUT, exist_ok=True)
    n = skipped = 0
    for path in sorted(glob.glob(os.path.join(suite, "processor-tests", "humans", "*.txt"))):
        name = os.path.basename(path)[:-4]
        s = sections(open(path, encoding="utf-8").read())
        try:
            items = json.loads(s["INPUT"])
            csl = s["CSL"]
        except Exception:
            skipped += 1
            continue
        if not isinstance(items, list) or not items:
            skipped += 1
            continue
        for i in items:
            if "id" in i:
                i["id"] = str(i["id"])
        mode = s.get("MODE", "citation").strip()
        clusters = []
        raw = s.get("CITATION-ITEMS") or s.get("CITATIONS")
        if raw and raw.strip():
            try:
                data = json.loads(raw)
            except Exception:
                skipped += 1
                continue
            for c in data:
                its = c if isinstance(c, list) else c.get("citationItems", []) if isinstance(c, dict) else []
                its = [i for i in its if isinstance(i, dict) and "id" in i]
                parts = [md_item(i) for i in its]
                if all(parts) and parts:
                    clusters.append("[" + "; ".join(parts) + "]")
        if not clusters and mode.startswith("citation"):
            # No citations given: every item is cited on its own, in order (what the suite's runner does).
            clusters = ["[@%s]" % i["id"] for i in items if "id" in i]
        doc = ""
        if mode.startswith("bibliography") or not clusters:
            doc += "---\nnocite: '@*'\n---\n\n"
        if clusters:
            doc += "\n\n".join(clusters) + "\n"
        else:
            doc += "x\n"
        open(os.path.join(OUT, name + ".md"), "w").write(doc)
        open(os.path.join(OUT, name + ".csl"), "w").write(csl)
        json.dump(items, open(os.path.join(OUT, name + ".json"), "w"), ensure_ascii=False)
        n += 1
    print("tests converted:", n, "skipped:", skipped)


def names():
    return sorted(os.path.basename(p)[:-3] for p in glob.glob(os.path.join(OUT, "*.md")))


def cmd_oracle():
    os.makedirs(os.path.join(OUT, "oracle"), exist_ok=True)

    def one(n):
        r = cb.pandoc_cmp(os.path.join(OUT, n + ".md"), os.path.join(OUT, n + ".json"), os.path.join(OUT, n + ".csl"))
        json.dump(r, open(os.path.join(OUT, "oracle", n + ".json"), "w"), ensure_ascii=False)

    with cf.ThreadPoolExecutor(3) as ex:
        list(ex.map(one, names()))


def cmd_run(binary):
    os.makedirs(os.path.join(OUT, "ours"), exist_ok=True)

    def one(n):
        r = subprocess.run([binary, "--bib", os.path.join(OUT, n + ".json"), "--csl", os.path.join(OUT, n + ".csl"),
                            os.path.join(OUT, n + ".md")], capture_output=True, timeout=60)
        dest = os.path.join(OUT, "ours", n + ".json")
        open(dest, "wb").write(r.stdout if r.returncode == 0 else json.dumps({"error": r.stderr.decode("utf-8", "replace")[:200]}).encode())

    with cf.ThreadPoolExecutor(3) as ex:
        list(ex.map(one, names()))


def cmd_report():
    by = {}
    ca = cm = ba = bm = nd = excl = failed = 0
    rows = []
    for n in names():
        ref = json.load(open(os.path.join(OUT, "oracle", n + ".json")))
        if ref is None:
            excl += 1
            continue
        try:
            ours = json.load(open(os.path.join(OUT, "ours", n + ".json")))
        except Exception:
            ours = {"error": "missing"}
        nd += 1
        fam = n.split("_")[0]
        d = by.setdefault(fam, [0, 0, 0, 0, 0])
        d[0] += 1
        rc, rb = ref["citations"], ref["bibliography"]
        if "error" in ours:
            failed += 1
            ca += len(rc); ba += len(rb); d[1] += len(rc); d[3] += len(rb)
            rows.append({"test": n, "error": ours["error"]})
            continue
        oc = [cb.normalize(x) for x in ours["citations"]]
        ob = [cb.normalize(x) for x in ours["bibliography"]]
        mc = sum(1 for i, x in enumerate(rc) if i < len(oc) and oc[i] == x)
        mb = sum(1 for i, x in enumerate(rb) if i < len(ob) and ob[i] == x)
        ca += len(rc); cm += mc; ba += len(rb); bm += mb
        d[1] += len(rc); d[2] += mc; d[3] += len(rb); d[4] += mb
        rows.append({"test": n, "cit_ref": rc, "cit_ours": oc, "bib_ref": rb, "bib_ours": ob})
    print("tests with a Pandoc result: %d (excluded: Pandoc failed %d); ours failed to run: %d" % (nd, excl, failed))
    print("citations exact %d/%d (%.1f%%), bibliography entries exact %d/%d (%.1f%%)" % (cm, ca, 100.0 * cm / max(ca, 1), bm, ba, 100.0 * bm / max(ba, 1)))
    ft = sum(1 for r in rows if "error" not in r and r["cit_ref"] == r["cit_ours"] and r["bib_ref"] == r["bib_ours"])
    print("tests where every citation and entry is exact: %d/%d (%.1f%%)" % (ft, nd, 100.0 * ft / max(nd, 1)))
    for fam, d in sorted(by.items(), key=lambda x: -x[1][0]):
        print("  %-14s tests %3d  citations %4d/%4d  entries %4d/%4d" % (fam, d[0], d[2], d[1], d[4], d[3]))
    json.dump(rows, open(os.path.join(cb.RESULTS, "cite-suite.json"), "w"), ensure_ascii=False, indent=0)


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[0] == "gen":
        cmd_gen(a[1])
    elif a[0] == "oracle":
        cmd_oracle()
    elif a[0] == "run":
        cmd_run(a[1])
    elif a[0] == "report":
        cmd_report()
