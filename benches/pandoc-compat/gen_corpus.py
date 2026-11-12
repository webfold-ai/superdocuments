#!/usr/bin/env python3
"""Writes the Pandoc-compatibility corpus and its manifest.

Usage: gen_corpus.py [--out DIR] [--lo-profile DIR]

Needs pandoc 3.1.3 (to write the DOCX/ODT/EPUB/RST/HTML variants of the Markdown seeds) and,
optionally, LibreOffice (`soffice`) for a second DOCX/ODT/EPUB producer. Every file records its
producer in the manifest, because a DOCX that Pandoc wrote is easier for Pandoc to read than one
that Word or LibreOffice wrote.

Seeds are fixed, so the corpus is byte-for-byte reproducible apart from zip timestamps in the
files Pandoc and LibreOffice write. The committed corpus is the one the published scores use.
"""
import json
import os
import random
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen_latex as L  # noqa: E402
import gen_other as O  # noqa: E402


def arg(name, default):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


OUT = arg("--out", os.path.join(HERE, "corpus"))
LO_PROFILE = arg("--lo-profile", "")
MANIFEST = []


def write(rel, data, **meta):
    path = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    mode = "wb" if isinstance(data, bytes) else "w"
    with open(path, mode, **({} if mode == "wb" else {"encoding": "utf-8"})) as f:
        f.write(data)
    return path


def add(cid, rel, fmt, producer, kind, reader, flags=None, entry=None):
    MANIFEST.append({"id": cid, "path": rel, "format": fmt, "producer": producer, "kind": kind,
                     "pandoc_from": reader, "pandoc_flags": flags or [], "cwd": entry or ""})


def pandoc(args, data=None):
    return subprocess.run(["pandoc"] + args, input=data, capture_output=True, check=True).stdout


def main():
    if os.path.isdir(OUT):
        shutil.rmtree(OUT)
    # ---- LaTeX: 100 generated, 6 hand-written papers, 12 projects -----------------------
    for i in range(1, 101):
        rng = random.Random(1000 + i)
        rel = "latex/gen-%03d.tex" % i
        write(rel, L.document(rng, i))
        add("latex-gen-%03d" % i, rel, "latex", "python generator (seeded)", "generated", "latex")
    bibrng = random.Random(77)
    for name, src in L.papers().items():
        rel = "latex/%s.tex" % name
        write(rel, src)
        add("latex-%s" % name, rel, "latex", "hand-written", "hand-written", "latex")
    for name, files in L.project_files().items():
        for fp, text in files.items():
            write("latex-projects/%s/%s" % (name, fp), text)
        write("latex-projects/%s/refs.bib" % name, O.bib_file(random.Random(hash_seed(name)), 10))
        add("latex-%s" % name, "latex-projects/%s/main.tex" % name, "latex", "python generator (seeded), \\input/\\include project",
            "generated", "latex", entry="latex-projects/%s" % name)

    # ---- BibTeX / BibLaTeX ---------------------------------------------------------------
    for i in range(1, 26):
        rng = random.Random(2000 + i)
        biblatex = i > 13
        rel = "bib/bib-%02d.bib" % i
        write(rel, O.bib_file(rng, biblatex=biblatex))
        add("bib-%02d" % i, rel, "bibtex", "python generator (seeded)", "generated", "bibtex" if not biblatex else "biblatex")

    # ---- Markdown seeds and the formats derived from them ----------------------------------
    for i in range(1, 41):
        rng = random.Random(3000 + i)
        md = O.md_seed(rng, i)
        rel = "markdown/md-%02d.md" % i
        write(rel, md)
        add("markdown-%02d" % i, rel, "markdown", "python generator (seeded)", "generated", "markdown")
        src = os.path.join(OUT, rel)
        if i <= 30:
            html = pandoc(["-f", "markdown", "-t", "html5", "-s", "--metadata", "title=Seed %d" % i, src])
            write("html/seed-%02d.html" % i, html)
            add("html-seed-%02d" % i, "html/seed-%02d.html" % i, "html", "pandoc 3.1.3 writer from a seed", "generated", "html")
        if i <= 22:
            write("rst/seed-%02d.rst" % i, pandoc(["-f", "markdown", "-t", "rst", src]))
            add("rst-seed-%02d" % i, "rst/seed-%02d.rst" % i, "rst", "pandoc 3.1.3 writer from a seed", "generated", "rst")
        if i <= 20:
            write("docx/pandoc-%02d.docx" % i, pandoc(["-f", "markdown", "-t", "docx", src]))
            add("docx-pandoc-%02d" % i, "docx/pandoc-%02d.docx" % i, "docx", "pandoc 3.1.3 writer from a seed", "generated", "docx",
                ["--track-changes=accept"])
        if i <= 15:
            write("odt/pandoc-%02d.odt" % i, pandoc(["-f", "markdown", "-t", "odt", src]))
            add("odt-pandoc-%02d" % i, "odt/pandoc-%02d.odt" % i, "odt", "pandoc 3.1.3 writer from a seed", "generated", "odt")
        if i <= 15:
            write("epub/pandoc-%02d.epub" % i, pandoc(["-f", "markdown", "-t", "epub3", src]))
            add("epub-pandoc-%02d" % i, "epub/pandoc-%02d.epub" % i, "epub", "pandoc 3.1.3 writer from a seed", "generated", "epub")

    # ---- hand-written HTML and RST -------------------------------------------------------------
    for name, src in O.HTML_DOCS.items():
        write("html/%s.html" % name, src)
        add("html-hand-" + name[5:], "html/%s.html" % name, "html", "hand-written", "hand-written", "html")
    for name, src in O.RST_DOCS.items():
        write("rst/%s.rst" % name, src)
        add("rst-hand-" + name[4:], "rst/%s.rst" % name, "rst", "hand-written", "hand-written", "rst")

    # ---- raw DOCX (not written by Pandoc) -----------------------------------------------------
    for name, (data, producer) in O.raw_docx_docs().items():
        write("docx/%s.docx" % name, data)
        flags = ["--track-changes=accept"]
        add("docx-" + name, "docx/%s.docx" % name, "docx", producer, "hand-written", "docx", flags)

    # ---- LibreOffice as a second producer: HTML -> DOCX/ODT/EPUB -------------------------------------
    soffice = shutil.which("soffice")
    if soffice and LO_PROFILE:
        lo_dir = os.path.join(OUT, "_lo")
        os.makedirs(lo_dir, exist_ok=True)
        conv = {"docx": "docx:MS Word 2007 XML", "odt": "odt:writer8", "epub": "epub:EPUB"}
        sources = ["seed-%02d" % i for i in range(21, 31)] + [
            "html-" + n for n in ("article", "recipe", "defs", "table-span", "nested", "figure", "headings", "ordered", "inline")]
        for ext, flt in conv.items():
            outdir = os.path.join(OUT, ext)
            os.makedirs(outdir, exist_ok=True)
            for stem in sources:
                src = os.path.join(OUT, "html", stem + ".html")
                subprocess.run([soffice, "-env:UserInstallation=file://" + LO_PROFILE, "--headless", "--convert-to", flt,
                                "--outdir", lo_dir, src], capture_output=True, timeout=180)
                made = os.path.join(lo_dir, "%s.%s" % (stem, ext))
                if os.path.exists(made):
                    name = stem.replace("seed-", "lo-").replace("html-", "lo-")
                    shutil.move(made, os.path.join(outdir, "%s.%s" % (name, ext)))
                    flags = ["--track-changes=accept"] if ext == "docx" else []
                    add("%s-%s" % (ext, name), "%s/%s.%s" % (ext, name, ext), ext,
                        "LibreOffice Writer (convert-to %s) from HTML" % flt, "generated", ext, flags)
        shutil.rmtree(lo_dir, ignore_errors=True)
    elif not soffice:
        print("soffice not found: the LibreOffice-produced files are skipped", file=sys.stderr)

    # shared bibliography for the paper-style documents
    write("latex/refs.bib", O.bib_file(random.Random(55), 10))
    with open(os.path.join(OUT, "manifest.json"), "w") as f:
        json.dump({"pandoc": "3.1.3", "seed_note": "all seeds are fixed in gen_corpus.py / gen_latex.py / gen_other.py",
                   "items": MANIFEST}, f, indent=1)
    by = {}
    for m in MANIFEST:
        by[m["format"]] = by.get(m["format"], 0) + 1
    print(len(MANIFEST), "items", by)


def hash_seed(name):
    return sum(ord(c) * (i + 1) for i, c in enumerate(name))


if __name__ == "__main__":
    main()
