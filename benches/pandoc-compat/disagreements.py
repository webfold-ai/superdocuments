#!/usr/bin/env python3
"""Lists every document the scorer found a difference in (results/sd-after.json), with the features that
differ, and the cause. The causes are rules over (format, feature, source): each rule is a claim
that a person checked on the documents it covers (see DISAGREEMENTS.md for what each cause means).
A difference no rule explains is printed as UNCLASSIFIED and makes the script exit 1.
Usage: disagreements.py > DISAGREEMENTS-TABLE.md   (stdout is a Markdown table)"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
rows = json.load(open(os.path.join(HERE, "results", "sd-after.json")))
manifest = {i["id"]: i for i in json.load(open(os.path.join(HERE, "corpus", "manifest.json")))["items"]}


def source(i):
    p = os.path.join(HERE, "corpus", manifest[i]["path"])
    try:
        return open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        return ""


def cause(r, feat, a, b):
    """a = Pandoc's count, b = ours. Returns a cause code (see DISAGREEMENTS.md)."""
    f = r["format"]
    base = feat.split(":")[0]
    if base == "cell":
        return "X1"  # a spanned cell: Markdown pipe tables repeat it, Pandoc counts it once
    if f == "latex":
        src = source(r["id"])
        if "algorithmic" in src and base in ("CODE", "code_block", "math_inline", "strong", "emph", "math_display"):
            return "L1"
        if re.search(r"\\(bibliography|addbibresource)\{", src) or "thebibliography" in src:
            if base in ("ul_item", "code_inline", "emph", "link", "linktarget", "h1", "strong"):
                return "L2"
        if base == "sup" and "siunitx" in src:
            return "L3"
        if base in ("footnote", "strong", "emph"):
            return "L4"
    if f == "docx" and base == "quote":
        return "D1"
    if f == "docx" and base == "sup":
        return "D2"
    if f == "odt" and base in ("hr", "CODE", "code_block", "code_inline"):
        return "O1"
    if f == "odt" and base in ("quote", "emph", "row", "linktarget", "link", "sup"):
        return "O2"
    if f == "epub" and base == "emph":
        return "E1"
    if f == "html" and base in ("math_display", "math_inline", "code_inline"):
        return "H1"
    return "UNCLASSIFIED"


bad = 0
print("| Document | Format | Fidelity | Differences (Pandoc count, ours) | Cause |")
print("|---|---|---|---|---|")
for r in sorted(rows, key=lambda r: (r["format"], r["id"])):
    if "excluded" in r or r["format"] == "bibtex" or not r.get("detail"):
        continue
    parts, causes = [], set()
    for k, (a, b) in sorted(r["detail"].items()):
        c = cause(r, k, a, b)
        causes.add(c)
        parts.append("%s %d/%d" % (k[:40].replace("|", "/"), a, b))
    bad += "UNCLASSIFIED" in causes
    print("| %s | %s | %.3f | %s | %s |" % (r["id"], r["format"], r["scores"]["fidelity"], "; ".join(parts), ", ".join(sorted(causes))))
sys.exit(1 if bad else 0)
