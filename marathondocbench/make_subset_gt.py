#!/usr/bin/env python3
"""Cut MarathonDocBench.json down to the documents fetch.py selected, so the official scorer can run on a subset.

  python3 make_subset_gt.py --data DIR       # writes DIR/MarathonDocBench.subset.json
"""
import argparse, json, os

ap = argparse.ArgumentParser(); ap.add_argument("--data", required=True); a = ap.parse_args()
d = os.path.expanduser(a.data)
docs = set(json.load(open(f"{d}/selection.json"))["docs"])
gt = [e for e in json.load(open(f"{d}/MarathonDocBench.json")) if e["page_info"]["image_path"].replace(".pdf", "") in docs]
json.dump(gt, open(f"{d}/MarathonDocBench.subset.json", "w"), ensure_ascii=False)
print(f"{len(gt)} of 100 documents -> {d}/MarathonDocBench.subset.json")
