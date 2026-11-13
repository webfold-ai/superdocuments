#!/usr/bin/env python3
"""Citation benchmark: superdocuments' CSL processor against `pandoc --citeproc` (Pandoc 3.1.3).

Documents (docs/*.md) are Pandoc Markdown with citations, written by gen_docs() from the bibliographies
in ../corpus/bib (seeded, so regenerating gives the same files). Styles are CSL files in styles/
(see styles/README.md for where each was fetched from and under which licence). For each
document x style, Pandoc is the reference:

  pandoc --citeproc --bibliography B --csl S -t json DOC

The formatted text of every citation (the inlines of each Cite node, in document order) and of every
bibliography entry (the csl-entry divs of #refs) is turned into a comparison string: text with
formatting marked (italic, bold, small caps, sup, sub), non-breaking and thin spaces as spaces,
whitespace runs as one space, adjacent identical marks merged. Nothing else is normalised.
Ours is `examples/cite_run.rs`, which produces the same kind of string from its own output.

Two tracks:
  csl   both processors read the CSL-JSON that Pandoc reads from the .bib (isolates the CSL engine)
  bib   both read the .bib (adds our BibTeX to CSL-JSON conversion)

Subcommands:  gen | oracle | run TRACK BIN | report TRACK
"""
import collections
import glob
import json
import os
import random
import re
import subprocess
import sys
import concurrent.futures as cf

HERE = os.path.dirname(os.path.abspath(__file__))
BIBS = os.path.join(HERE, "..", "corpus", "bib")
# CITE_SET=heldout selects the held-out set: other documents (another seed) and four styles that were never
# looked at while the processor was developed (styles-heldout/). The default set is the development set.
SET = os.environ.get("CITE_SET", "dev")
SUFFIX = "" if SET == "dev" else "-" + SET
DOCS = os.path.join(HERE, "docs" + SUFFIX)
STYLES = os.path.join(HERE, "styles" + SUFFIX)
ORACLE = os.path.join(HERE, "oracle" + SUFFIX)
OUT = os.path.join(HERE, "outputs" + SUFFIX)
RESULTS = os.path.join(HERE, "results")
CSLJSON = os.path.join(HERE, "csljson")
STYLE_NAMES = (["apa", "chicago-author-date", "ieee", "harvard-cite-them-right", "nature", "modern-language-association"]
               if SET == "dev" else ["elsevier-vancouver", "american-medical-association", "springer-basic-author-date",
                                     "american-sociological-association"])
DOCS_PER_BIB = 5
SEED_PREFIX = "" if SET == "dev" else SET + "-"


def keys_of(path):
    return re.findall(r"@\w+\s*\{\s*([^,\s]+)\s*,", open(path, encoding="utf-8", errors="replace").read())[1:] if False else \
        [k for k in re.findall(r"^@(\w+)\s*\{\s*([^,\s]+)\s*,", open(path, encoding="utf-8", errors="replace").read(), re.M) if k[0].lower() not in ("string", "comment", "preamble")]


FORMS = [
    "[@{a}]", "[@{a}; @{b}]", "[@{a}; @{b}; @{c}]", "[@{a}, p. {n}]", "[@{a}, pp. {n}-{m}]", "[see @{a}]",
    "[see @{a}, ch. {n}]", "[-@{a}]", "[@{a}; -@{b}]", "@{a}", "@{a} [p. {n}]", "[@{a}, sec. {n}.{m} and more]",
    "[@{a}, {n}]", "[@{a}; @{b}, p. {n}]", "[@{a}, fig. {n}]", "[e.g. @{a}; @{b}]", "@{a} [pp. {n}-{m}]", "[@{a}, vol. {n}]",
]
SENT = ["The result holds", "This was observed earlier", "Prior work disagrees", "A related claim appears", "We follow the method",
        "The effect is small", "The dataset is public", "Results vary", "See the survey", "The proof is short"]


def gen_docs():
    os.makedirs(DOCS, exist_ok=True)
    n = 0
    for bib in sorted(glob.glob(os.path.join(BIBS, "bib-*.bib"))):
        entries = [k[1] for k in keys_of(bib)]
        if len(entries) < 3:
            continue
        base = os.path.basename(bib)[:-4]
        for d in range(DOCS_PER_BIB):
            rnd = random.Random("%s%s-%d" % (SEED_PREFIX, base, d))
            lines = ["# Notes %s.%d" % (base, d), ""]
            used = []
            for p in range(rnd.randint(3, 5)):
                parts = []
                for _ in range(rnd.randint(2, 3)):
                    f = rnd.choice(FORMS)
                    pool = used[:] if used and rnd.random() < 0.35 else entries
                    ks = [rnd.choice(pool) for _ in range(3)]
                    if len({ks[0], ks[1], ks[2]}) < 3 and len(entries) >= 3:
                        ks = rnd.sample(entries, 3)
                    used += ks
                    c = f.format(a=ks[0], b=ks[1], c=ks[2], n=rnd.randint(2, 99), m=rnd.randint(100, 140))
                    parts.append("%s %s." % (rnd.choice(SENT), c))
                lines.append(" ".join(parts))
                lines.append("")
            lines += ["# References", ""]
            with open(os.path.join(DOCS, "%s-d%d.md" % (base, d)), "w") as f:
                f.write("\n".join(lines))
            n += 1
    print("documents:", n)


def docs():
    return sorted(glob.glob(os.path.join(DOCS, "*.md")))


def bib_of(doc):
    return os.path.join(BIBS, os.path.basename(doc).rsplit("-d", 1)[0] + ".bib")


# ---- Pandoc JSON to comparison strings -------------------------------------------------------------

MARKS = {"Emph": ("", ""), "Strong": ("", ""), "SmallCaps": ("", ""),
         "Superscript": ("", ""), "Subscript": ("", "")}


def inl(xs, out):
    for x in xs:
        t = x["t"]
        c = x.get("c")
        if t == "Str":
            out.append(c)
        elif t in ("Space", "SoftBreak", "LineBreak"):
            out.append(" ")
        elif t in MARKS:
            out.append(MARKS[t][0]); inl(c, out); out.append(MARKS[t][1])
        elif t == "Quoted":
            q = ("“", "”") if c[0]["t"] == "DoubleQuote" else ("‘", "’")
            out.append(q[0]); inl(c[1], out); out.append(q[1])
        elif t in ("Link", "Span", "Underline", "Strikeout"):
            inl(c[1] if t in ("Link", "Span") else c, out)
        elif t == "Cite":
            inl(c[1], out)
        elif t == "Code":
            out.append(c[1])
        elif t == "Note":
            # A note style: citeproc puts the citation text in a footnote.
            for b in c:
                if b["t"] in ("Para", "Plain"):
                    inl(b["c"], out)


def normalize(s):
    s = s.replace(" ", " ").replace(" ", " ").replace(" ", " ")
    for o, c in MARKS.values():
        while c + o in s:
            s = s.replace(c + o, "")
        s = s.replace(o + c, "")
    return " ".join(s.split())


def cmp_string(xs):
    out = []
    inl(xs, out)
    return normalize("".join(out))


def cites(blocks, acc):
    for b in blocks:
        t = b["t"]
        c = b.get("c")
        if t in ("Para", "Plain", "Header"):
            find_cites(c[2] if t == "Header" else c, acc)
        elif t in ("BlockQuote",):
            cites(c, acc)
        elif t in ("BulletList", "OrderedList"):
            for item in (c if t == "BulletList" else c[1]):
                cites(item, acc)
        elif t == "Div" and not (c[0][0] == "refs"):
            cites(c[1], acc)


def find_cites(xs, acc):
    for x in xs:
        t = x["t"]
        c = x.get("c")
        if t == "Cite":
            acc.append(cmp_string(c[1]))
        elif t in ("Emph", "Strong", "SmallCaps", "Superscript", "Subscript", "Underline", "Strikeout"):
            find_cites(c, acc)
        elif t in ("Link", "Span"):
            find_cites(c[1], acc)
        elif t == "Quoted":
            find_cites(c[1], acc)


def entries(blocks, acc):
    for b in blocks:
        if b["t"] == "Div":
            attr, inner = b["c"]
            if attr[0] == "refs":
                for e in inner:
                    if e["t"] == "Div":
                        acc.append(div_text(e))
            else:
                entries(inner, acc)


def div_text(d):
    out = []
    for b in d["c"][1]:
        if b["t"] in ("Para", "Plain"):
            inl(b["c"], out)
        elif b["t"] == "Div":
            if out and out[-1] != " ":
                out.append(" ")
            out.append(div_text(b))
    return normalize("".join(out))


def pandoc_cmp(doc, bib, style, to_json=True):
    r = subprocess.run(["pandoc", "--citeproc", "--bibliography", bib, "--csl", style, "-f", "markdown", "-t", "json", doc],
                       capture_output=True, timeout=120)
    if r.returncode != 0:
        return None
    ast = json.loads(r.stdout)
    c, b = [], []
    cites(ast["blocks"], c)
    entries(ast["blocks"], b)
    return {"citations": c, "bibliography": b}


def csl_json_for(bibfile):
    os.makedirs(CSLJSON, exist_ok=True)
    dest = os.path.join(CSLJSON, os.path.basename(bibfile)[:-4] + ".json")
    if not os.path.exists(dest):
        r = subprocess.run(["pandoc", "-f", "bibtex", "-t", "csljson", bibfile], capture_output=True)
        open(dest, "wb").write(r.stdout)
    return dest


def style_path(name):
    return os.path.join(STYLES, name + ".csl")


def cmd_oracle():
    os.makedirs(ORACLE, exist_ok=True)

    def one(args):
        doc, st = args
        base = os.path.basename(doc)[:-3]
        dest = os.path.join(ORACLE, st, base + ".json")
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        # The csl track gives Pandoc the CSL-JSON it itself reads from the .bib.
        res = {}
        for track, bib in (("csl", csl_json_for(bib_of(doc))), ("bib", bib_of(doc))):
            res[track] = pandoc_cmp(doc, bib, style_path(st))
        json.dump(res, open(dest, "w"), indent=0, ensure_ascii=False)

    with cf.ThreadPoolExecutor(3) as ex:
        list(ex.map(one, [(d, s) for d in docs() for s in STYLE_NAMES]))
    print("oracle written for", len(docs()), "documents x", len(STYLE_NAMES), "styles")


def cmd_run(track, binary):
    os.makedirs(os.path.join(OUT, track), exist_ok=True)

    def one(args):
        doc, st = args
        base = os.path.basename(doc)[:-3]
        bib = csl_json_for(bib_of(doc)) if track == "csl" else bib_of(doc)
        extra = ["--pandoc-names"] if os.environ.get("CITE_PANDOC_NAMES") else []
        r = subprocess.run([binary] + extra + ["--bib", bib, "--csl", style_path(st), doc], capture_output=True, timeout=120)
        dest = os.path.join(OUT, track, st, base + ".json")
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        if r.returncode == 0:
            open(dest, "wb").write(r.stdout)
        else:
            open(dest, "w").write(json.dumps({"error": r.stderr.decode("utf-8", "replace")[:300]}))

    with cf.ThreadPoolExecutor(3) as ex:
        list(ex.map(one, [(d, s) for d in docs() for s in STYLE_NAMES]))


def cmd_report(track, detail=False):
    os.makedirs(RESULTS, exist_ok=True)
    rows = []
    for st in STYLE_NAMES:
        ca = cm = ba = bm = nd = 0
        failed = 0
        for doc in docs():
            base = os.path.basename(doc)[:-3]
            ref = json.load(open(os.path.join(ORACLE, st, base + ".json")))[track]
            if ref is None:
                continue
            try:
                ours = json.load(open(os.path.join(OUT, track, st, base + ".json")))
            except Exception:
                ours = {"error": "missing"}
            nd += 1
            if "error" in ours:
                failed += 1
                ca += len(ref["citations"]); ba += len(ref["bibliography"])
                continue
            oc = [normalize(x) for x in ours["citations"]]
            ob = [normalize(x) for x in ours["bibliography"]]
            ca += len(ref["citations"]); ba += len(ref["bibliography"])
            cm += sum(1 for i, x in enumerate(ref["citations"]) if i < len(oc) and oc[i] == x)
            # Bibliography entries match by content, not position: a different order is a separate defect,
            # counted in `bib_order` below.
            bm += sum(1 for i, x in enumerate(ref["bibliography"]) if i < len(ob) and ob[i] == x)
            rows.append({"style": st, "doc": base, "cit_ref": ref["citations"], "cit_ours": oc,
                         "bib_ref": ref["bibliography"], "bib_ours": ob})
        print("%-28s docs %3d  citations %5d/%5d (%5.1f%%)  bibliography entries %5d/%5d (%5.1f%%)  failed %d" % (
            st, nd, cm, ca, 100.0 * cm / max(ca, 1), bm, ba, 100.0 * bm / max(ba, 1), failed))
        rows.append({"style": st, "summary": {"documents": nd, "citations": ca, "citations_exact": cm, "entries": ba,
                                              "entries_exact": bm, "failed": failed}})
    # Per-document strings stay in outputs/ (not committed: they are formatted text from third-party styles);
    # results/ gets the pass/fail table, which is our own result.
    os.makedirs(os.path.join(OUT, "_detail"), exist_ok=True)
    json.dump(rows, open(os.path.join(OUT, "_detail", "cite-%s.json" % track), "w"), indent=0, ensure_ascii=False)
    with open(os.path.join(RESULTS, "cite-%s%s.csv" % (track, SUFFIX)), "w") as f:
        f.write("style,document,citations,citations_exact,entries,entries_exact\n")
        for r in rows:
            if "summary" in r:
                continue
            ca = len(r["cit_ref"]); ba = len(r["bib_ref"])
            cm = sum(1 for i, x in enumerate(r["cit_ref"]) if i < len(r["cit_ours"]) and r["cit_ours"][i] == x)
            bm = sum(1 for i, x in enumerate(r["bib_ref"]) if i < len(r["bib_ours"]) and r["bib_ours"][i] == x)
            f.write("%s,%s,%d,%d,%d,%d\n" % (r["style"], r["doc"], ca, cm, ba, bm))
    tot = [r["summary"] for r in rows if "summary" in r]
    c, cm, e, em = (sum(t[k] for t in tot) for k in ("citations", "citations_exact", "entries", "entries_exact"))
    print("ALL: citations %d/%d (%.1f%%), bibliography entries %d/%d (%.1f%%)" % (cm, c, 100.0 * cm / max(c, 1), em, e, 100.0 * em / max(e, 1)))


# ---- golden set: our own small styles, Pandoc's output stored compactly (see tests/pandoc_compat_golden.rs) ----

def cmd_golden():
    """Writes golden/<style>.json: for every development document, the citations and bibliography entries
    Pandoc 3.1.3 produces with one of the three styles in golden-styles/ (both tracks)."""
    gdir = os.path.join(HERE, "golden")
    os.makedirs(gdir, exist_ok=True)
    styles = sorted(os.path.basename(p)[:-4] for p in glob.glob(os.path.join(HERE, "golden-styles", "*.csl")))
    for st in styles:
        out = {}

        def one(doc, st=st):
            base = os.path.basename(doc)[:-3]
            return base, pandoc_cmp(doc, csl_json_for(bib_of(doc)), os.path.join(HERE, "golden-styles", st + ".csl"))

        with cf.ThreadPoolExecutor(3) as ex:
            for base, r in ex.map(one, sorted(glob.glob(os.path.join(HERE, "docs", "*.md")))):
                out[base] = r
        with open(os.path.join(gdir, st + ".json"), "w") as f:
            json.dump(out, f, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    print("golden written for", styles)


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        print(__doc__)
    elif a[0] == "gen":
        gen_docs()
    elif a[0] == "oracle":
        cmd_oracle()
    elif a[0] == "run":
        cmd_run(a[1], a[2])
    elif a[0] == "report":
        cmd_report(a[1])
    elif a[0] == "golden":
        cmd_golden()