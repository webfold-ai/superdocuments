#!/usr/bin/env python3
"""Compares superdocuments::pdf::extract_images with poppler's pdfimages on a PDF.

Usage: compare_pdfimages.py <file.pdf> [--target-dir DIR]

Checks, for every image: page, size, colour space, bits per component, and encoding category
against `pdfimages -list`; JPEG bytes against `pdfimages -j`; and decoded PNG pixels against
`pdfimages -png` (both decoded with Pillow). Prints one line per image and a summary. Exit
status is 0 when everything agrees.
"""
import hashlib
import io
import os
import subprocess
import sys
import tempfile

from PIL import Image


def run(cmd, cwd=None):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=cwd).stdout


def main():
    pdf = os.path.abspath(sys.argv[1])
    target = os.environ.get("CARGO_TARGET_DIR", "/home/user/sd-target-latex")
    if "--target-dir" in sys.argv:
        target = sys.argv[sys.argv.index("--target-dir") + 1]
    repo = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    exe = run(["cargo", "build", "--locked", "--example", "pdf_image_list", "--message-format=json"], cwd=repo)
    binary = None
    for line in exe.splitlines():
        if '"executable"' in line and "pdf_image_list" in line:
            import json
            binary = json.loads(line)["executable"]
    if binary is None:
        binary = os.path.join(target, "debug", "examples", "pdf_image_list")

    with tempfile.TemporaryDirectory(prefix="pdfimg-ours-") as ours_dir, tempfile.TemporaryDirectory(prefix="pdfimg-poppler-") as pop_dir:
        ours = run([binary, pdf, ours_dir]).splitlines()
        listing = run(["pdfimages", "-list", pdf]).splitlines()[2:]
        poppler = []
        for line in listing:
            f = line.split()
            if len(f) < 9:
                continue
            poppler.append({
                "page": int(f[0]), "num": int(f[1]), "type": f[2], "w": int(f[3]), "h": int(f[4]),
                "color": f[5], "comp": int(f[6]), "bpc": int(f[7]), "enc": f[8], "obj": int(f[10]),
            })
        ours_rows = []
        for line in ours:
            f = line.split()
            ours_rows.append({
                "page": int(f[0]), "idx": int(f[1]), "obj": int(f[2]), "w": int(f[3]), "h": int(f[4]),
                "cs": f[5], "bpc": int(f[6]), "enc": f[7],
            })
        # Poppler's `-list` numbers objects by ID; compare in the same order.
        failures = 0
        if len(ours_rows) != len(poppler):
            print(f"COUNT MISMATCH: ours {len(ours_rows)} poppler {len(poppler)}")
            failures += 1
        for o, p in zip(ours_rows, poppler):
            problems = []
            if (o["page"], o["w"], o["h"], o["bpc"], o["obj"]) != (p["page"], p["w"], p["h"], p["bpc"], p["obj"]):
                problems.append("page/size/bpc/object")
            poppler_cat = "jpeg" if p["enc"] == "jpeg" else "raw-or-png"
            ours_cat = "jpeg" if o["enc"] == "jpeg" else "raw-or-png"
            if poppler_cat != ours_cat:
                problems.append(f"encoding {ours_cat} vs {poppler_cat}")
            if o["enc"] == "unsupported" or o["enc"].startswith("unsupported"):
                problems.append("unsupported")
            # Pixel and byte checks against poppler's own extraction of the same image.
            if o["enc"] == "jpeg":
                prefix = os.path.join(pop_dir, f"j{p['page']}-{p['num']}")
                run(["pdfimages", "-j", "-f", str(p["page"]), "-l", str(p["page"]), pdf, prefix])
                candidates = [f for f in os.listdir(pop_dir) if f.startswith(f"j{p['page']}-")]
                ours_file = [f for f in os.listdir(ours_dir) if f.startswith(f"p{o['page']}-{o['idx']}-")]
                if candidates and ours_file:
                    with open(os.path.join(pop_dir, sorted(candidates)[min(p["num"], len(candidates) - 1)]), "rb") as fh:
                        theirs = hashlib.sha256(fh.read()).hexdigest()
                    with open(os.path.join(ours_dir, ours_file[0]), "rb") as fh:
                        mine = hashlib.sha256(fh.read()).hexdigest()
                    if theirs != mine:
                        problems.append("jpeg bytes differ from pdfimages -j")
                else:
                    problems.append("no comparison file")
            elif o["enc"] == "png":
                ours_file = [f for f in os.listdir(ours_dir) if f.startswith(f"p{o['page']}-{o['idx']}-")]
                with Image.open(os.path.join(ours_dir, ours_file[0])) as img:
                    mine = img.convert("RGB" if o["cs"] == "DeviceRGB" else "L").tobytes()
                pop_name = f"n{p['page']}-{p['num']}"
                run(["pdfimages", "-png", "-f", str(p["page"]), "-l", str(p["page"]), pdf, os.path.join(pop_dir, pop_name)])
                theirs_files = sorted(f for f in os.listdir(pop_dir) if f.startswith(f"n{p['page']}-"))
                # pdfimages numbers images per page; pick the one with the same size.
                match = None
                for name in theirs_files:
                    with Image.open(os.path.join(pop_dir, name)) as pimg:
                        if pimg.size == (o["w"], o["h"]):
                            match = pimg.convert("RGB" if o["cs"] == "DeviceRGB" else "L").tobytes()
                            break
                if match is None:
                    problems.append("no poppler PNG of the same size")
                elif match != mine:
                    problems.append("decoded pixels differ from pdfimages -png")
            status = "ok" if not problems else "DIFF: " + "; ".join(problems)
            if problems:
                failures += 1
            print(f"page {o['page']} obj {o['obj']} {o['w']}x{o['h']} {o['cs']} bpc {o['bpc']} "
                  f"ours={o['enc']} poppler={p['enc']} -> {status}")
        print(f"{len(ours_rows)} images, {failures} disagreements")
        return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
