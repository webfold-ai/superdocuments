#!/usr/bin/env python3
"""A provisional, text-only score for predictions made by run.py. It is NOT an MPDocBench metric.

  python3 score_provisional.py --data DIR --pred DIR/pred [--json out.json]

For each document and system: the ground-truth text (text, titles, captions, notes, table cell text, in reading `order`) and the prediction (page
markers, image tags and Markdown markup removed) are lower-cased, NFKC-normalised and stripped of whitespace; the score is the character-level
Levenshtein similarity of the two whole-document strings (1 = identical). It ignores layout, so tables, headings, formulas and figures are only
as good as the text they contain, and it is order-sensitive, so a reading-order mistake costs. Use it to see whether a system reads the text at all.
The official scorer (see README.md, `score_official.md`) is what a leaderboard number needs.
"""
import argparse, html, json, os, re, unicodedata
from rapidfuzz.distance import Levenshtein

TEXT_KINDS = {"text_block", "title", "figure_caption", "figure_footnote", "table_caption", "table_footnote", "header", "footer", "page_number"}


def norm(s):
    return re.sub(r"\s+", "", unicodedata.normalize("NFKC", s)).lower()


def strip_tags(h):
    return html.unescape(re.sub(r"<[^>]+>", " ", h or ""))


def gt_text(entry):
    items = [d for d in entry["layout_dets"] if not d.get("ignore")]
    items.sort(key=lambda d: (d.get("order") is None, d.get("order", 0)))
    out = []
    for d in items:
        k = d["category_type"]
        if k in TEXT_KINDS:
            out.append(d.get("text", ""))
        elif k == "table":
            out.append(strip_tags(d.get("html", "")))
    return norm("\n".join(out))


def pred_text(md):
    md = re.sub(r"<!--.*?-->", " ", md, flags=re.S)
    md = re.sub(r"!\[[^\]]*\]\([^)]*\)", " ", md)
    md = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", md)
    md = re.sub(r"^[ \t]*(#{1,6}|>|[-*+])[ \t]+", "", md, flags=re.M)
    md = md.replace("|", " ")
    md = re.sub(r"[*_`~]+", "", md)
    return norm(md)


def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--data", required=True); ap.add_argument("--pred", required=True); ap.add_argument("--json")
    a = ap.parse_args()
    data, pred = os.path.expanduser(a.data), os.path.expanduser(a.pred)
    gt = {e["page_info"]["image_path"].replace(".pdf", ""): e for e in json.load(open(f"{data}/MarathonDocBench.json"))}
    docs = json.load(open(f"{data}/selection.json"))["docs"]
    systems = sorted(d for d in os.listdir(pred) if os.path.isdir(f"{pred}/{d}") and not d.startswith("_"))
    timings = json.load(open(f"{pred}/timings.json")) if os.path.exists(f"{pred}/timings.json") else {}
    res = {}
    print("| system | document | pages | GT chars | predicted chars | similarity | seconds |")
    print("|---|---|---:|---:|---:|---:|---:|")
    for s in systems:
        for d in docs:
            p = f"{pred}/{s}/{d}.md"
            g = gt_text(gt[d])
            q = pred_text(open(p).read()) if os.path.exists(p) else ""
            sim = Levenshtein.normalized_similarity(g, q) if g or q else 1.0
            t = timings.get(s, {}).get(d, {})
            res.setdefault(s, {})[d] = {"similarity": round(sim, 4), "gt_chars": len(g), "pred_chars": len(q), "seconds": t.get("seconds")}
            print(f"| {s} | {d} | {t.get('pages', '')} | {len(g)} | {len(q)} | {sim:.3f} | {t.get('seconds', '')} |")
    if a.json: json.dump(res, open(a.json, "w"), indent=1)


main()
