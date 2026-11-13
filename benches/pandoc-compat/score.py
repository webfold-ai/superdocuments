#!/usr/bin/env python3
"""Fidelity scorer: how closely a converter's Markdown matches what Pandoc 3.1.3 reads.

Reference: `pandoc -f <reader> -t json <input>` for each corpus item (flags pinned in
corpus/manifest.json). Candidate: the converter's Markdown, read back by Pandoc with ONE fixed
reader (`-f markdown -t json`) so both sides go through the same feature extractor. Pandoc is a
dependency of this scorer, never of superdocuments at runtime.

Sub-scores per document, each in [0, 1]:
  structure  LCS-F1 of the ordered block sequence (headings by level, paragraphs, list and list
             items, tables, code blocks, display math, quotes, figures, rules, definition lists)
  features   micro-F1 of counted features (heading levels, list items, table rows and cells,
             code blocks with their text, inline and display math, footnotes, links with targets,
             images, emphasis, strong, inline code, sub/superscript, strikeout, citations)
  text       bag-of-words F1 over lower-cased word tokens (math, raw markup and citation text
             excluded; footnote text included)
  math       micro-F1 over whitespace-stripped math strings (reported separately; only for
             documents where either side has math)
  fidelity   mean of structure, features and text

A converter that fails on a document scores 0 on it. A document Pandoc cannot read is listed as
an exclusion, not scored.

Subcommands:
  oracle                     write oracle/<id>.json from Pandoc
  convert TOOL -- CMD...     run CMD on each document ({path} replaced) into outputs/TOOL/<id>.md
  score TOOL                 score outputs/TOOL against the oracle -> results/TOOL.json
  report TOOL [TOOL...]      write RESULTS.md-style tables to stdout
"""
import collections
import json
import os
import re
import subprocess
import sys
import concurrent.futures as cf

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = os.environ.get("PC_CORPUS", os.path.join(HERE, "corpus"))
ORACLE = os.path.join(HERE, "oracle")
OUTPUTS = os.environ.get("PC_OUTPUTS", os.path.join(HERE, "outputs"))
RESULTS = os.path.join(HERE, "results")
REREAD = ["-f", "markdown-raw_tex", "-t", "json"]
TIMEOUT = 90


def manifest():
    with open(os.path.join(CORPUS, "manifest.json")) as f:
        return json.load(f)["items"]


# ---- feature extraction from a Pandoc JSON AST ------------------------------------------

WORD = re.compile(r"\w+", re.UNICODE)


def inl_text(inls, out, feat):
    for i in inls:
        t = i["t"]
        c = i.get("c")
        if t == "Str":
            out.append(c)
        elif t in ("Space", "SoftBreak", "LineBreak"):
            out.append(" ")
        elif t in ("Emph", "Underline", "SmallCaps"):
            if t == "Emph":
                feat["emph"] += 1
            inl_text(c, out, feat)
        elif t == "Strong":
            feat["strong"] += 1
            inl_text(c, out, feat)
        elif t == "Strikeout":
            feat["strike"] += 1
            inl_text(c, out, feat)
        elif t == "Superscript":
            feat["sup"] += 1
            inl_text(c, out, feat)
        elif t == "Subscript":
            feat["sub"] += 1
            inl_text(c, out, feat)
        elif t in ("Span",):
            inl_text(c[1], out, feat)
        elif t == "Quoted":
            inl_text(c[1], out, feat)
        elif t == "Cite":
            feat["cite"] += len(c[0])
        elif t == "Code":
            feat["code_inline"] += 1
            out.append(" " + c[1] + " ")
        elif t == "Math":
            feat["math_display" if c[0]["t"] == "DisplayMath" else "math_inline"] += 1
            feat["MATH:" + re.sub(r"\s+", "", c[1])] += 0  # tracked separately by walk()
        elif t == "Link":
            feat["link"] += 1
            tgt = c[2][0]
            if tgt and not tgt.startswith("#"):
                feat["linktarget:" + tgt] += 1
            inl_text(c[1], out, feat)
        elif t == "Image":
            feat["image"] += 1
            inl_text(c[1], out, feat)
        elif t == "Note":
            feat["footnote"] += 1
            walk(c, None, out, feat, None, nested=True)
        elif t == "RawInline":
            pass


def walk(blocks, seq, out, feat, maths, nested=False, in_item=False):
    """Appends skeleton tokens to seq (None inside notes), text to out and counts to feat."""
    for b in blocks:
        t = b["t"]
        c = b.get("c")
        s = seq if not nested else None

        def emit(tok):
            if s is not None:
                s.append(tok)

        if t == "Header":
            emit("H%d" % c[0])
            feat["h%d" % c[0]] += 1
            inl_text(c[2], out, feat)
            out.append(" ")
        elif t in ("Para", "Plain"):
            if len(c) == 1 and c[0]["t"] == "Math" and c[0]["c"][0]["t"] == "DisplayMath":
                emit("MATH")
            elif len(c) == 1 and c[0]["t"] == "Image":
                emit("FIG")
            elif not in_item and t == "Para":
                emit("P")
            elif not in_item and t == "Plain":
                emit("P")
            inl_text(c, out, feat)
            out.append(" ")
        elif t == "BulletList" or t == "OrderedList":
            emit("UL" if t == "BulletList" else "OL")
            items = c if t == "BulletList" else c[1]
            for it in items:
                emit("LI")
                feat["ul_item" if t == "BulletList" else "ol_item"] += 1
                walk(it, seq, out, feat, maths, nested, in_item=True)
        elif t == "DefinitionList":
            emit("DL")
            for term, defs in c:
                emit("LI")
                feat["dl_item"] += 1
                inl_text(term, out, feat)
                out.append(" ")
                for d in defs:
                    walk(d, seq, out, feat, maths, nested, in_item=True)
        elif t == "CodeBlock":
            emit("CODE")
            feat["code_block"] += 1
            txt = c[1]
            feat["CODE:" + re.sub(r"\s+", " ", txt).strip()] += 1
            out.append(" " + txt + " ")
        elif t == "BlockQuote":
            emit("QUOTE")
            feat["quote"] += 1
            walk(c, seq, out, feat, maths, nested, in_item)
        elif t == "Table":
            emit("TABLE")
            feat["table"] += 1
            head, bodies, foot = c[3], c[4], c[5]
            rows = list(head[1])
            for body in bodies:
                rows += list(body[2]) + list(body[3])
            rows += list(foot[1])
            for r in rows:
                feat["row"] += 1
                for cell in r[1]:
                    feat["cell"] += 1
                    walk(cell[4], None, out, feat, maths, True)
            caption = c[1]
            if caption[1]:
                walk(caption[1], None, out, feat, maths, True)
        elif t == "Figure":
            emit("FIG")
            walk(c[2], None, out, feat, maths, True)
            if c[1][1]:
                walk(c[1][1], None, out, feat, maths, True)
        elif t == "HorizontalRule":
            emit("HR")
            feat["hr"] += 1
        elif t == "Div":
            walk(c[1], seq, out, feat, maths, nested, in_item)
        elif t == "LineBlock":
            emit("P")
            for ln in c:
                inl_text(ln, out, feat)
                out.append(" ")
        elif t == "RawBlock":
            pass


def collect_math(blocks, maths):
    """Every Math string in the document, whitespace-stripped (including notes and tables)."""
    def visit(o):
        if isinstance(o, dict):
            if o.get("t") == "Math":
                maths[re.sub(r"\s+", "", o["c"][1])] += 1
                return
            for v in o.values():
                visit(v)
        elif isinstance(o, list):
            for v in o:
                visit(v)
    visit(blocks)


def meta_text(v, out, feat, seq):
    """Title, author, date and abstract of the document, counted as text (and an abstract as a block)."""
    t, c = v["t"], v.get("c")
    if t == "MetaInlines":
        inl_text(c, out, feat)
        out.append(" ")
    elif t == "MetaBlocks":
        walk(c, seq, out, feat, None, nested=True)
    elif t == "MetaString":
        out.append(c + " ")
    elif t == "MetaList":
        for x in c:
            meta_text(x, out, feat, seq)
    elif t == "MetaMap":
        for x in c.values():
            meta_text(x, out, feat, seq)


def features(ast):
    seq, out, feat, maths = [], [], collections.Counter(), collections.Counter()
    meta = ast.get("meta", {})
    for key in ("title", "author", "date", "abstract"):
        if key in meta:
            seq.append(key.upper())
            meta_text(meta[key], out, feat, seq)
    walk(ast["blocks"], seq, out, feat, maths)
    collect_math(ast["blocks"], maths)
    collect_math(meta, maths)
    text = " ".join(out)
    toks = collections.Counter(w.lower() for w in WORD.findall(text))
    feat = collections.Counter({k: v for k, v in feat.items() if v and not k.startswith("MATH:")})
    return {"seq": seq, "feat": dict(feat), "tokens": dict(toks), "math": dict(maths)}


# ---- comparison --------------------------------------------------------------------------

def lcs_f1(a, b):
    if not a and not b:
        return 1.0
    if not a or not b:
        return 0.0
    prev = [0] * (len(b) + 1)
    for x in a:
        cur = [0]
        for j, y in enumerate(b):
            cur.append(prev[j] + 1 if x == y else max(prev[j + 1], cur[j]))
        prev = cur
    l = prev[-1]
    return 2.0 * l / (len(a) + len(b))


def bag_f1(a, b):
    ta, tb = sum(a.values()), sum(b.values())
    if ta == 0 and tb == 0:
        return 1.0
    inter = sum(min(v, b.get(k, 0)) for k, v in a.items())
    return 2.0 * inter / (ta + tb)


def compare(ref, cand):
    sc = {
        "structure": lcs_f1(ref["seq"], cand["seq"]),
        "features": bag_f1(ref["feat"], cand["feat"]),
        "text": bag_f1(ref["tokens"], cand["tokens"]),
    }
    sc["fidelity"] = (sc["structure"] + sc["features"] + sc["text"]) / 3
    if ref["math"] or cand["math"]:
        sc["math"] = bag_f1(ref["math"], cand["math"])
    return sc


# ---- Pandoc ----------------------------------------------------------------------------------

def run_pandoc(args, cwd=None, data=None):
    cmd = ["sh", "-c", 'ulimit -v 4000000; exec timeout %d "$@"' % TIMEOUT, "guard", "pandoc"] + args
    return subprocess.run(cmd, cwd=cwd, input=data, capture_output=True)


def pandoc_ast(item):
    path = os.path.join(CORPUS, item["path"])
    cwd = os.path.join(CORPUS, item["cwd"]) if item.get("cwd") else None
    if cwd:
        path = os.path.relpath(path, cwd)
    args = ["-f", item["pandoc_from"]] + item["pandoc_flags"] + ["-t", "json", path]
    return run_pandoc(args, cwd=cwd)


def cmd_oracle():
    os.makedirs(ORACLE, exist_ok=True)
    items = manifest()

    def one(item):
        r = pandoc_ast(item)
        dest = os.path.join(ORACLE, item["id"] + ".json")
        if r.returncode != 0:
            rec = {"id": item["id"], "pandoc_error": r.stderr.decode("utf-8", "replace").strip()[:300]}
        else:
            rec = {"id": item["id"], **features(json.loads(r.stdout))}
        with open(dest, "w") as f:
            json.dump(rec, f, sort_keys=True)
        return item["id"], "pandoc_error" in rec

    with cf.ThreadPoolExecutor(8) as ex:
        res = list(ex.map(one, items))
    print("oracle: %d items, %d pandoc errors" % (len(res), sum(e for _, e in res)))
    for i, e in res:
        if e:
            print("  pandoc cannot read", i)


# ---- running a converter -------------------------------------------------------------------------

def cmd_convert(tool, cmd):
    out = os.path.join(OUTPUTS, tool)
    os.makedirs(out, exist_ok=True)
    items = manifest()

    def one(item):
        path = os.path.abspath(os.path.join(CORPUS, item["path"]))
        argv = [a.replace("{path}", path) for a in cmd]
        dest = os.path.join(out, item["id"] + ".md")
        try:
            r = subprocess.run(argv, capture_output=True, timeout=TIMEOUT)
            ok = r.returncode == 0
            with open(dest, "wb") as f:
                f.write(r.stdout if ok else b"")
            with open(dest + ".status", "w") as f:
                f.write("ok" if ok else "error: " + r.stderr.decode("utf-8", "replace").strip()[:300])
        except subprocess.TimeoutExpired:
            with open(dest, "wb"):
                pass
            with open(dest + ".status", "w") as f:
                f.write("error: timeout")

    with cf.ThreadPoolExecutor(int(os.environ.get("PC_JOBS", "4"))) as ex:
        list(ex.map(one, items))
    print("converted", len(items), "with", tool)


# ---- scoring --------------------------------------------------------------------------------------

def cmd_score(tool):
    items = manifest()
    results = []

    def one(item):
        with open(os.path.join(ORACLE, item["id"] + ".json")) as f:
            ref = json.load(f)
        row = {"id": item["id"], "format": item["format"], "producer": item["producer"], "kind": item["kind"]}
        if "pandoc_error" in ref:
            row["excluded"] = "pandoc cannot read: " + ref["pandoc_error"][:80]
            return row
        dest = os.path.join(OUTPUTS, tool, item["id"] + ".md")
        status = open(dest + ".status").read() if os.path.exists(dest + ".status") else "missing"
        row["status"] = status
        if status != "ok":
            row["scores"] = {"structure": 0.0, "features": 0.0, "text": 0.0, "fidelity": 0.0}
            row["failed"] = True
            return row
        r = run_pandoc(REREAD, data=open(dest, "rb").read())
        if r.returncode != 0:
            row["scores"] = {"structure": 0.0, "features": 0.0, "text": 0.0, "fidelity": 0.0}
            row["failed"] = True
            row["status"] = "output not readable by pandoc"
            return row
        cand = features(json.loads(r.stdout))
        row["scores"] = compare(ref, cand)
        row["detail"] = diff_detail(ref, cand)
        return row

    with cf.ThreadPoolExecutor(8) as ex:
        results = list(ex.map(one, items))
    os.makedirs(RESULTS, exist_ok=True)
    with open(os.path.join(RESULTS, tool + ".json"), "w") as f:
        json.dump(results, f, indent=0, sort_keys=True)
    print("scored", len(results), "for", tool)


def diff_detail(ref, cand):
    """Feature keys whose counts differ (code and link-target keys abbreviated)."""
    d = {}
    for k in set(ref["feat"]) | set(cand["feat"]):
        a, b = ref["feat"].get(k, 0), cand["feat"].get(k, 0)
        if a != b:
            d[k[:60]] = [a, b]
    return d


# ---- bibliography track: CSL-JSON agreement -------------------------------------------------------

def canon(v):
    if isinstance(v, str):
        return re.sub(r"\s+", " ", v).strip()
    if isinstance(v, list):
        return [canon(x) for x in v]
    if isinstance(v, dict):
        return {k: canon(x) for k, x in v.items()}
    return v


def csl_fields(entries):
    out = {}
    for e in entries:
        for k, v in e.items():
            if k == "id":
                continue
            out[(e["id"], k)] = json.dumps(canon(v), sort_keys=True, ensure_ascii=False)
    return out


def cmd_bib(tool, cmd):
    """Entry-level and field-level agreement of CSL-JSON read from each .bib file."""
    rows = []
    for item in manifest():
        if item["format"] != "bibtex":
            continue
        path = os.path.abspath(os.path.join(CORPUS, item["path"]))
        ref = run_pandoc(["-f", item["pandoc_from"], "-t", "csljson", path])
        if ref.returncode != 0:
            rows.append({"id": item["id"], "excluded": "pandoc cannot read"})
            continue
        argv = [a.replace("{path}", path) for a in cmd]
        r = subprocess.run(argv, capture_output=True, timeout=TIMEOUT)
        try:
            ours = json.loads(r.stdout) if r.returncode == 0 else None
        except ValueError:
            ours = None
        pref = json.loads(ref.stdout)
        a = csl_fields(pref)
        b = csl_fields(ours or [])
        ids_a = {e["id"] for e in pref}
        ids_b = {e["id"] for e in ours or []}
        tp = sum(1 for k, v in a.items() if b.get(k) == v)
        exact = 0
        diff = collections.Counter()
        for i in ids_a:
            fa = {k[1]: v for k, v in a.items() if k[0] == i}
            fb = {k[1]: v for k, v in b.items() if k[0] == i}
            if fa == fb:
                exact += 1
            else:
                for k in set(fa) | set(fb):
                    if fa.get(k) != fb.get(k):
                        diff[k] += 1
        rows.append({"id": item["id"], "format": "bibtex", "entries": len(ids_a), "entries_found": len(ids_a & ids_b),
                     "entries_exact": exact, "fields_pandoc": len(a), "fields_ours": len(b), "fields_equal": tp,
                     "field_f1": 2.0 * tp / (len(a) + len(b)) if (a or b) else 1.0, "diff": dict(diff), "failed": ours is None})
    os.makedirs(RESULTS, exist_ok=True)
    with open(os.path.join(RESULTS, tool + "-bib.json"), "w") as f:
        json.dump(rows, f, indent=0, sort_keys=True)
    ok = [r for r in rows if "excluded" not in r]
    ent = sum(r["entries"] for r in ok)
    ex = sum(r["entries_exact"] for r in ok)
    tp = sum(r["fields_equal"] for r in ok)
    na, nb = sum(r["fields_pandoc"] for r in ok), sum(r["fields_ours"] for r in ok)
    dsum = collections.Counter()
    for r in ok:
        dsum.update(r["diff"])
    print("bib %s: %d files, %d entries, exact entries %d (%.1f%%), field F1 %.4f (%d/%d/%d)" % (
        tool, len(ok), ent, ex, 100.0 * ex / max(ent, 1), 2.0 * tp / max(na + nb, 1), tp, na, nb))
    print("  differing fields:", dict(dsum.most_common(12)))


# ---- report ---------------------------------------------------------------------------------------

def mean(xs):
    xs = list(xs)
    return sum(xs) / len(xs) if xs else float("nan")


def summarize(rows):
    by = collections.defaultdict(list)
    for r in rows:
        if "excluded" not in r and r["format"] != "bibtex":
            by[r["format"]].append(r)
    out = {}
    for fmt, rs in sorted(by.items()):
        failed = sum(1 for r in rs if r.get("failed"))
        ok = [r for r in rs if not r.get("failed")]
        o = {"n": len(rs), "failed": failed}
        for k in ("structure", "features", "text", "fidelity"):
            o[k] = mean(r["scores"][k] for r in rs)
            o[k + "_converted"] = mean(r["scores"][k] for r in ok)
        m = [r["scores"]["math"] for r in rs if "math" in r["scores"]]
        o["math"], o["math_n"] = mean(m), len(m)
        o["exact"] = sum(1 for r in rs if r["scores"]["structure"] == 1 and r["scores"]["features"] == 1 and r["scores"]["text"] == 1)
        out[fmt] = o
    allrows = [r for r in rows if "excluded" not in r and r["format"] != "bibtex"]
    out["ALL"] = {"n": len(allrows), "failed": sum(1 for r in allrows if r.get("failed")),
                  **{k: mean(r["scores"][k] for r in allrows) for k in ("structure", "features", "text", "fidelity")}}
    return out


def cmd_report(tools):
    sums = {}
    excl = {}
    for t in tools:
        rows = json.load(open(os.path.join(RESULTS, t + ".json")))
        sums[t] = summarize(rows)
        excl[t] = [r for r in rows if "excluded" in r]
    fmts = sorted({f for s in sums.values() for f in s if f != "ALL"}) + ["ALL"]
    print("| Format | n | " + " | ".join("%s fidelity (struct / feat / text)" % t for t in tools) + " |")
    print("|---|---|" + "---|" * len(tools))
    for f in fmts:
        n = max(s.get(f, {}).get("n", 0) for s in sums.values())
        cells = []
        for t in tools:
            o = sums[t].get(f)
            cells.append("-" if not o else "%.3f (%.3f / %.3f / %.3f)%s" % (
                o["fidelity"], o["structure"], o["features"], o["text"], " %d failed" % o["failed"] if o["failed"] else ""))
        print("| %s | %d | %s |" % (f, n, " | ".join(cells)))
    print()
    for t in tools:
        if excl[t]:
            print("%s: %d excluded (Pandoc cannot read): %s" % (t, len(excl[t]), ", ".join(r["id"] for r in excl[t])))


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        print(__doc__)
    elif a[0] == "oracle":
        cmd_oracle()
    elif a[0] == "convert":
        cmd_convert(a[1], a[a.index("--") + 1:])
    elif a[0] == "score":
        cmd_score(a[1])
    elif a[0] == "bib":
        cmd_bib(a[1], a[a.index("--") + 1:])
    elif a[0] == "report":
        cmd_report(a[1:])
