#!/usr/bin/env python3
"""Runs MarkItDown (Python API, one process) over the corpus into outputs/markitdown/.

Usage (inside a virtualenv with `markitdown[docx,pptx,xlsx]` installed):
  python run_markitdown.py CORPUS_DIR OUTPUT_DIR

The Python API is used rather than 2,000 command-line starts; the output is the same call the
`markitdown` command makes. A file MarkItDown refuses is recorded as an error, which the scorer
counts as 0 for that document and reports as "failed".
"""
import json
import os
import sys

from markitdown import MarkItDown

corpus, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
md = MarkItDown()
items = json.load(open(os.path.join(corpus, "manifest.json")))["items"]
for item in items:
    path = os.path.join(corpus, item["path"])
    dest = os.path.join(out, item["id"] + ".md")
    try:
        text = md.convert(path).text_content
        status = "ok"
    except Exception as e:  # noqa: BLE001 - any refusal is a recorded failure
        text, status = "", "error: %s: %s" % (type(e).__name__, str(e)[:200])
    with open(dest, "w", encoding="utf-8") as f:
        f.write(text)
    with open(dest + ".status", "w") as f:
        f.write(status)
print("markitdown converted", len(items))
