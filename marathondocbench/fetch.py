#!/usr/bin/env python3
"""Download MarathonDocBench (baidu/MarathonDocBench on Hugging Face) into a cache directory, for chosen documents only.

  python3 fetch.py --out ~/.cache/marathondocbench --shortest 2          # the two shortest documents
  python3 fetch.py --out DIR --ids mp3lan_0101 mp3lan_0778               # named documents
  python3 fetch.py --out DIR --all                                       # all 100 documents, about 18,000 page images

The ground truth (`MarathonDocBench.json`, Apache-2.0) and the page images are fetched at run time and never committed here: the dataset card says the
images are derived from publicly accessible documents and are for academic and research use only.
"""
import argparse, concurrent.futures as cf, json, os, sys, time, urllib.request

BASE = "https://huggingface.co/datasets/baidu/MarathonDocBench/resolve/main/"


def get(url, dest, tries=4):
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        return
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    for i in range(tries):
        try:
            with urllib.request.urlopen(url, timeout=120) as r, open(dest + ".part", "wb") as f:
                while chunk := r.read(1 << 20):
                    f.write(chunk)
            os.replace(dest + ".part", dest)
            return
        except Exception as e:
            if i == tries - 1:
                raise
            time.sleep(2 ** i)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--ids", nargs="*")
    ap.add_argument("--shortest", type=int)
    ap.add_argument("--all", action="store_true")
    a = ap.parse_args()
    out = os.path.expanduser(a.out)
    get(BASE + "MarathonDocBench.json", f"{out}/MarathonDocBench.json")
    get(BASE + "images/metadata.jsonl", f"{out}/images/metadata.jsonl")
    pages = {}
    for line in open(f"{out}/images/metadata.jsonl"):
        m = json.loads(line)
        pages.setdefault(m["doc_id"], m["num_pages"])
    if a.all:
        ids = sorted(pages)
    elif a.ids:
        ids = a.ids
    elif a.shortest:
        ids = [d for d, _ in sorted(pages.items(), key=lambda kv: (kv[1], kv[0]))[: a.shortest]]
    else:
        sys.exit("choose --ids, --shortest N or --all")
    jobs = [(f"{BASE}images/{d}/page_{p}.png", f"{out}/images/{d}/page_{p}.png") for d in ids for p in range(1, pages[d] + 1)]
    with cf.ThreadPoolExecutor(8) as ex:
        list(ex.map(lambda j: get(*j), jobs))
    json.dump({"docs": ids, "pages": {d: pages[d] for d in ids}}, open(f"{out}/selection.json", "w"), indent=1)
    print(f"{len(ids)} documents, {len(jobs)} page images in {out}")


main()
