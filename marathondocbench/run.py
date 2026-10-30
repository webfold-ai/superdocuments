#!/usr/bin/env python3
"""Produce one Markdown file per MarathonDocBench document, for each system, ready for the MPDocBench-Parse scorer.

  WEBFOLD_BIN=... python3 run.py --data DIR --out DIR/pred [--systems webfold,webfold_default,tesseract] [--lang chi_sim+chi_tra+eng] [--jobs 4]

Input is the page images fetch.py downloaded. Systems:
  webfold          page images wrapped into image-only PDFs of --chunk pages, each through `webfold pdf --ocr-lang L --ocr-max-pages N --ocr-max-dim D --ocr-timeout 600`
                   (the OCR fallback: native PDF text first, then pdftoppm + tesseract), chunks joined in page order
  webfold_default  the same PDF with `webfold pdf` as shipped: English only, 20 pages, 120 s
  tesseract        `tesseract <page.png> - -l L` on every page, joined in page order (the incumbent the OCR fallback wraps)

Neither system produces layout: no headings, tables, formulas or figure tags, only text. The scorer counts that as it finds it.
Per-document wall time and page count go to <out>/timings.json. Needs tesseract (with the language packs), poppler's pdftoppm and Pillow.
"""
import argparse, concurrent.futures as cf, json, os, re, subprocess, sys, time
from PIL import Image


def pdf_of(doc, data, tmp, first, last):
    """An image-only PDF of pages first..last (1-based, inclusive)."""
    ims = [Image.open(f"{data}/images/{doc}/page_{p}.png").convert("RGB") for p in range(first, last + 1)]
    path = f"{tmp}/{doc}.{first}-{last}.pdf"
    ims[0].save(path, save_all=True, append_images=ims[1:])
    return path


def renumber(md, offset):
    """webfold marks pages `<!-- page N -->` counting from the chunk's first page; make N the document's page number."""
    return re.sub(r"<!-- page (\d+) -->", lambda m: f"<!-- page {int(m.group(1)) + offset} -->", md)


def run_webfold(doc, a, default):
    t = time.time()
    pages = json.load(open(f"{a.data}/selection.json"))["pages"][doc]
    env = dict(os.environ, OMP_THREAD_LIMIT="1")
    if default:  # `webfold pdf` as shipped, on the whole document
        pdf = pdf_of(doc, a.data, a.tmp, 1, pages)
        r = subprocess.run([a.webfold, "pdf", pdf], capture_output=True, text=True, env=env, timeout=1800)
        os.remove(pdf)
        return (r.stdout if r.returncode == 0 else ""), pages, time.time() - t, r.returncode
    # superdocuments' OCR refuses more than 100 pages or 600 s per file, so a long document goes through in chunks
    out, rc = [], 0
    for first in range(1, pages + 1, a.chunk):
        last = min(pages, first + a.chunk - 1)
        pdf = pdf_of(doc, a.data, a.tmp, first, last)
        r = subprocess.run([a.webfold, "pdf", pdf, "--ocr-lang", a.lang, "--ocr-max-pages", str(a.chunk), "--ocr-max-dim", str(a.dim), "--ocr-timeout", "600"],
                           capture_output=True, text=True, env=env, timeout=1800)
        os.remove(pdf)
        rc |= r.returncode
        out.append(renumber(r.stdout, first - 1) if r.returncode == 0 else f"<!-- pages {first}-{last}: webfold exit {r.returncode} -->")
    return "\n\n".join(out), pages, time.time() - t, rc


def run_tesseract(doc, a):
    t = time.time()
    pages = json.load(open(f"{a.data}/selection.json"))["pages"][doc]
    out, rc = [], 0
    env = dict(os.environ, OMP_THREAD_LIMIT="1")
    for p in range(1, pages + 1):
        r = subprocess.run(["tesseract", f"{a.data}/images/{doc}/page_{p}.png", "-", "-l", a.lang], capture_output=True, text=True, env=env)
        rc |= r.returncode
        out.append(r.stdout)
    return "\n\n".join(out), pages, time.time() - t, rc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--systems", default="webfold,webfold_default,tesseract")
    ap.add_argument("--lang", default="chi_sim+chi_tra+eng")
    ap.add_argument("--timeout", type=int, default=3600)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--dim", type=int, default=1800, help="longest rendered page edge; webfold allows pages x edge^2 x 4 bytes up to 512 MiB per call")
    ap.add_argument("--chunk", type=int, default=40, help="pages per webfold call (its OCR allows at most 100 pages and 600 s per file)")
    a = ap.parse_args()
    a.webfold = os.environ.get("WEBFOLD_BIN", "webfold")
    a.data, a.out = os.path.expanduser(a.data), os.path.expanduser(a.out)
    a.tmp = f"{a.out}/_tmp"
    os.makedirs(a.tmp, exist_ok=True)
    docs = json.load(open(f"{a.data}/selection.json"))["docs"]
    timings = {}
    for system in a.systems.split(","):
        os.makedirs(f"{a.out}/{system}", exist_ok=True)

        def one(doc):
            if system == "tesseract":
                return doc, run_tesseract(doc, a)
            return doc, run_webfold(doc, a, default=(system == "webfold_default"))

        with cf.ThreadPoolExecutor(a.jobs) as ex:
            for doc, (md, pages, secs, rc) in ex.map(one, docs):
                open(f"{a.out}/{system}/{doc}.md", "w").write(md)
                timings.setdefault(system, {})[doc] = {"pages": pages, "seconds": round(secs, 1), "exit": rc, "chars": len(md)}
                print(f"{system:16s} {doc}: {pages} pages, {secs:.0f}s, exit {rc}, {len(md)} chars", flush=True)
    json.dump(timings, open(f"{a.out}/timings.json", "w"), indent=1)


main()
