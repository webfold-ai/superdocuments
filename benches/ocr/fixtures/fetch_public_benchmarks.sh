#!/usr/bin/env bash
# Fetch small representative subsets of public OCR/document-parsing benchmarks.
#
# Sources:
#   - opendatalab/OmniDocBench        (CVPR 2025; 1,651 pages, 10 doc types, English/CN)
#   - llamaindex/ParseBench          (LlamaIndex; 2,078 pages; 5 dimensions)
#   - getomni-ai/ocr-benchmark       (multi-LLM JSON-extraction OCR benchmark)
#
# All three are public, ungated, and have commercial-friendly dataset licenses
# (OmniDocBench is research-only for the *benchmark* but the dataset itself is
# downloadable; ParseBench is Apache-2.0; getomni is MIT). We download a small
# sample of each (~100 MB total) so the fixture corpus stays portable.
#
# Usage:
#   ./fetch_public_benchmarks.sh [--out=DIR] [--omnidoc N] [--parsebench N] [--getomni N]
#
# Defaults: 30 OmniDocBench pages, 5 ParseBench docs, 5 getomni images.
# After download, run the bench with:
#   cargo run --release --manifest-path ../Cargo.toml -- \
#     --fixtures ./fetched --out ./results/public-bench-2026-09-27.json

set -euo pipefail

OUT="${OUT_DIR:-$(cd "$(dirname "$0")" && pwd)/fetched}"
OMNIDOC_N="${OMNIDOC_N:-30}"
PARSEBENCH_N="${PARSEBENCH_N:-5}"
GETOMNI_N="${GETOMNI_N:-5}"

mkdir -p "$OUT"/{omnidocbench/{images,annotations},parsebench/{docs,annotations},getomni}

# HF base URL + raw download helper (no auth required for public datasets).
hf() {
    local repo="$1" file="$2"
    echo "  https://huggingface.co/datasets/$repo/resolve/main/$file"
}

curl_s() {
    # curl with a browser-like UA so HF doesn't 403 us; resume on failure.
    curl -fL --retry 3 --retry-delay 2 \
        -H 'User-Agent: Mozilla/5.0 (compatible; superdocuments-bench-ocr)' \
        "$1" -o "$2"
}

echo "==> Output dir: $OUT"
echo "    OmniDocBench pages: $OMNIDOC_N"
echo "    ParseBench docs:    $PARSEBENCH_N"
echo "    getomni images:     $GETOMNI_N"
echo

# ── OmniDocBench ────────────────────────────────────────────────────────────
echo "==> OmniDocBench (opendatalab/OmniDocBench)"
curl_s "$(hf opendatalab/OmniDocBench README.md)" "$OUT/omnidocbench/README.md" || true
# Pull a small slice of the annotations JSON — full file is 42 MB and we only
# need a handful of entries. Streaming + grep the first N matching image paths
# is the cheapest way. Since HF doesn't support range queries on raw content,
# download the whole file then trim.
if [ ! -f "$OUT/omnidocbench/OmniDocBench.json" ]; then
    echo "  downloading OmniDocBench.json (42 MB) — will be trimmed afterwards"
    curl_s "https://huggingface.co/datasets/opendatalab/OmniDocBench/resolve/main/OmniDocBench.json" \
        "$OUT/omnidocbench/OmniDocBench.json"
fi

# Pick N image basenames (stratified across doc types: 3 per major prefix).
echo "  selecting $OMNIDOC_N image paths"
OMNIDOC_OUT="$OUT/omnidocbench"
OMNIDOC_TARGET="$OMNIDOC_N"
export OMNIDOC_OUT OMNIDOC_TARGET
python3 - <<'PY' > "$OUT/omnidocbench/selected.txt"
import json, os
target = int(os.environ["OMNIDOC_TARGET"])
out_dir = os.environ["OMNIDOC_OUT"]
data = json.load(open(os.path.join(out_dir, "OmniDocBench.json")))
seen = {}
out = []
for entry in data:
    img = entry.get("page_info", {}).get("image_path", "")
    if not img: continue
    prefix = img.split("_")[0]
    if seen.get(prefix, 0) >= 3: continue
    seen[prefix] = seen.get(prefix, 0) + 1
    out.append(f"images/{img}")
    if len(out) >= target: break
print("\n".join(out))
PY

mapfile -t OMNIDOC_IMAGES < "$OUT/omnidocbench/selected.txt"
echo "  downloading $(wc -l < "$OUT/omnidocbench/selected.txt") PNG pages"
for img in "${OMNIDOC_IMAGES[@]}"; do
    [ -z "$img" ] && continue
    # `img` looks like `images/page-X.png`; strip the `images/` prefix and
    # re-add it under our own output root.
    rel="${img#images/}"
    mkdir -p "$OUT/omnidocbench/images"
    curl_s "https://huggingface.co/datasets/opendatalab/OmniDocBench/resolve/main/$img" \
        "$OUT/omnidocbench/images/$rel"
done

# ── ParseBench ─────────────────────────────────────────────────────────────
echo
echo "==> ParseBench (llamaindex/ParseBench)"
curl_s "$(hf llamaindex/ParseBench README.md)" "$OUT/parsebench/README.md" || true
# Pull the JSONL files (small enough to download whole; we slice later).
for j in text_content text_formatting table chart layout; do
    if [ ! -f "$OUT/parsebench/annotations/${j}.jsonl" ]; then
        echo "  downloading ${j}.jsonl"
        curl_s "https://huggingface.co/datasets/llamaindex/ParseBench/resolve/main/${j}.jsonl" \
            "$OUT/parsebench/annotations/${j}.jsonl"
    fi
done

# Pick $PARSEBENCH_N PDFs across categories.
echo "  selecting $PARSEBENCH_N PDFs across text/table/layout/chart"
PARSEBENCH_OUT="$OUT/parsebench"
PARSEBENCH_TARGET="$PARSEBENCH_N"
export PARSEBENCH_OUT PARSEBENCH_TARGET
python3 - <<'PY' > "$OUT/parsebench/selected.txt"
import json, os
target = int(os.environ["PARSEBENCH_TARGET"])
out_dir = os.environ["PARSEBENCH_OUT"]
selected = []
seen_categories = set()
# Try one PDF per JSONL source (table, layout, chart); fall back to text_dense.
for j, n_each in [("table", 1), ("layout", 1), ("chart", 1), ("text_content", 2)]:
    src = os.path.join(out_dir, "annotations", f"{j}.jsonl")
    if not os.path.exists(src): continue
    picked_in_cat = 0
    with open(src) as f:
        for line in f:
            if len(selected) >= target: break
            row = json.loads(line)
            pdf = row.get("pdf", "")
            cat = pdf.split("/")[1] if "/" in pdf else ""
            if cat not in seen_categories and cat == j and picked_in_cat < n_each:
                seen_categories.add(cat)
                selected.append(pdf)
                picked_in_cat += 1
                if picked_in_cat >= n_each: break
print("\n".join(selected))
PY

mapfile -t PARSEBENCH_DOCS < "$OUT/parsebench/selected.txt"
echo "  downloading $(wc -l < "$OUT/parsebench/selected.txt") PDFs"
for pdf in "${PARSEBENCH_DOCS[@]}"; do
    [ -z "$pdf" ] && continue
    # `pdf` looks like `docs/table/0000027_page1.pdf`; we want the file to
    # land at `$OUT/parsebench/docs/table/0000027_page1.pdf`, so strip the
    # leading `docs/` (the parent dir is already `$OUT/parsebench/docs/`).
    rel="${pdf#docs/}"
    mkdir -p "$OUT/parsebench/docs/$(dirname "$rel")"
    curl_s "https://huggingface.co/datasets/llamaindex/ParseBench/resolve/main/$pdf" \
        "$OUT/parsebench/docs/$rel"
done

# Slice the text_content.jsonl down to only the selected PDFs (saves a few MB
# of fixture footprint; keeps the rest of the rules for ad-hoc queries).
PARSEBENCH_OUT2="$OUT/parsebench"
export PARSEBENCH_OUT2
python3 - <<'PY'
import json, os
out_dir = os.environ["PARSEBENCH_OUT2"]
selected = set()
sel_file = os.path.join(out_dir, "selected.txt")
if os.path.exists(sel_file):
    for line in open(sel_file):
        s = line.strip()
        if s: selected.add(s)
for j in ["text_content", "text_formatting", "table", "chart", "layout"]:
    inp = os.path.join(out_dir, "annotations", f"{j}.jsonl")
    out = os.path.join(out_dir, "annotations", f"{j}-sliced.jsonl")
    if not os.path.exists(inp): continue
    with open(inp) as f, open(out, "w") as g:
        for line in f:
            row = json.loads(line)
            if row.get("pdf") in selected:
                g.write(line)
    os.replace(out, inp)
print("  sliced JSONL files down to selected PDFs only")
PY

# Generate `expected.txt` for each downloaded PDF by running the JSONL rule
# sentences through a flat alphanumeric-lowercase collapse (matches the bench's
# normalize() helper). Reads ALL five JSONL files — table/layout/chart JSONLs
# also carry text-level rules that we can use for CER scoring.
echo "  generating expected.txt sidecars"
export PARSEBENCH_OUT2
python3 - <<'PY'
import json, os
out_dir = os.environ["PARSEBENCH_OUT2"]
def normalize(s):
    return "".join(c.lower() for c in s if c.isalnum())

def extract_sentences(rule):
    """Pull the textual reference out of a ParseBench rule. Known shapes:
    - `bag_of_sentence`: dict of {sentence: weight} — values are the sentences.
    - `expected_markdown`: string of full markdown (table, headings, paragraphs).
    - `content` (layout rows): string of one block's text.
    - list: bare list of strings.
    Falls back to collecting any string-typed values found in the dict.
    """
    if isinstance(rule, list):
        return [str(x) for x in rule]
    if not isinstance(rule, dict):
        return []
    if "bag_of_sentence" in rule and isinstance(rule["bag_of_sentence"], dict):
        return list(rule["bag_of_sentence"].keys())
    out = []
    for k in ("expected_markdown", "expected_text", "markdown", "text", "content"):
        v = rule.get(k)
        if isinstance(v, str):
            out.append(v)
        elif isinstance(v, list):
            out.extend(str(x) for x in v)
    if out:
        return out
    # Last resort: collect any string-typed leaf value.
    for v in rule.values():
        if isinstance(v, str) and len(v) >= 4:
            out.append(v)
    return out

seen = set()
# For the layout category, accumulate per-PDF (pdf -> [(ro_index, content), ...])
# across all JSONL rows so the expected.txt matches the document's natural
# reading order, not row-id order.
layout_blocks: dict[str, list[tuple[int, str]]] = {}

for j in ["text_content", "text_formatting", "table", "layout"]:
    # `chart` is intentionally excluded: its rules encode numeric/value-pair
    # matches, not text, so its `expected.txt` would be junk. The harness
    # reports `text_gt=yes` for charts via CER anyway; chart truth has to be
    # evaluated by ParseBench's own `parse-bench run <pipeline>`, not by our
    # CER scorer.
    inp = os.path.join(out_dir, "annotations", f"{j}.jsonl")
    if not os.path.exists(inp): continue
    with open(inp) as f:
        for line in f:
            row = json.loads(line)
            pdf = row.get("pdf", "")  # e.g. "docs/text/text_dense__baoutou.pdf"
            rel = pdf[len("docs/"):] if pdf.startswith("docs/") else pdf
            full = os.path.join(out_dir, "docs", rel)
            if not os.path.exists(full): continue
            try:
                rule = json.loads(row["rule"])
            except Exception:
                rule = {}
            sentences = extract_sentences(rule)
            for top_key in ("expected_markdown", "expected_text", "content"):
                v = row.get(top_key)
                if isinstance(v, str):
                    sentences.append(v)
                elif isinstance(v, list):
                    sentences.extend(str(x) for x in v)
            if not sentences:
                # Layout rows where `content` is empty / missing still record
                # the (pdf, ro_index) so the final concatenation preserves reading
                # order — we just append an empty string for those slots.
                if j == "layout" and pdf:
                    layout_blocks.setdefault(pdf, []).append(
                        (row.get("ro_index", 0) or 0, "")
                    )
                continue
            base = os.path.splitext(os.path.basename(pdf))[0]
            target = os.path.join(os.path.dirname(full), base + ".expected.txt")
            if j == "layout":
                # Defer layout writes; we'll do them in a second pass with
                # the full reading-order concatenation.
                ro_index = row.get("ro_index", 0) or 0
                # Take only the first sentence (one block per row); rows with
                # multi-sentence content are uncommon in practice.
                layout_blocks.setdefault(pdf, []).append((ro_index, sentences[0]))
                continue
            new_body = "\n".join(normalize(s) for s in sentences) + "\n"
            key = base
            if key in seen:
                if os.path.exists(target):
                    with open(target) as g:
                        old_body = g.read()
                    if len(new_body) <= len(old_body):
                        continue
            seen.add(key)
            with open(target, "w") as g:
                g.write(new_body)

# Layout pass: concatenate all (ro_index, content) per PDF in ro_index order.
for pdf, blocks in layout_blocks.items():
    rel = pdf[len("docs/"):] if pdf.startswith("docs/") else pdf
    full = os.path.join(out_dir, "docs", rel)
    if not os.path.exists(full): continue
    base = os.path.splitext(os.path.basename(pdf))[0]
    target = os.path.join(os.path.dirname(full), base + ".expected.txt")
    blocks.sort(key=lambda b: b[0])
    body = "\n".join(normalize(b[1]) for b in blocks) + "\n"
    if not body.strip():
        # No text content at all in this PDF; skip rather than write junk.
        continue
    key = base
    if key in seen:
        if os.path.exists(target):
            with open(target) as g:
                old_body = g.read()
            if len(body) <= len(old_body):
                continue
    seen.add(key)
    with open(target, "w") as g:
        g.write(body)

print(f"  wrote {len(seen)} expected.txt sidecars")
PY

# ── getomni-ai/ocr-benchmark ───────────────────────────────────────────────
echo
echo "==> getomni-ai/ocr-benchmark"
curl_s "$(hf getomni-ai/ocr-benchmark README.md)" "$OUT/getomni/README.md" || true
if [ ! -f "$OUT/getomni/metadata.jsonl" ]; then
    echo "  downloading test/metadata.jsonl"
    curl_s "https://huggingface.co/datasets/getomni-ai/ocr-benchmark/resolve/main/test/metadata.jsonl" \
        "$OUT/getomni/metadata.jsonl"
fi
echo "  selecting $GETOMNI_N images"
GETOMNI_OUT="$OUT/getomni"
GETOMNI_TARGET="$GETOMNI_N"
export GETOMNI_OUT GETOMNI_TARGET
python3 - <<'PY' > "$OUT/getomni/selected.txt"
import json, os
target = int(os.environ["GETOMNI_TARGET"])
out_dir = os.environ["GETOMNI_OUT"]
n = target
out = []
seen = set()
with open(os.path.join(out_dir, "metadata.jsonl")) as f:
    for line in f:
        if len(out) >= n: break
        try:
            row = json.loads(line)
        except Exception:
            continue
        img = row.get("image_path") or row.get("image") or row.get("file_name") or ""
        if not img or img in seen: continue
        seen.add(img)
        # The JSONL records full dataset-relative paths like `images/0.png`;
        # strip the `images/` prefix so the bash download loop can re-add it.
        if img.startswith("images/"):
            img = img[len("images/"):]
        out.append(img)
print("\n".join(out))
PY
mapfile -t GETOMNI_IMAGES < "$OUT/getomni/selected.txt"
echo "  downloading $(wc -l < "$OUT/getomni/selected.txt") images"
for img in "${GETOMNI_IMAGES[@]}"; do
    [ -z "$img" ] && continue
    # `img` already looks like `images/0.png` (the JSONL records the full
    # dataset-relative path); strip the leading `images/` so the saved
    # path doesn't double up to `images/images/...`.
    rel="${img#images/}"
    mkdir -p "$OUT/getomni/images/$(dirname "$rel")"
    curl_s "https://huggingface.co/datasets/getomni-ai/ocr-benchmark/resolve/main/test/$img" \
        "$OUT/getomni/images/$rel"
done

echo
echo "==> Done."
echo "    Total downloaded: $(du -sh "$OUT" | cut -f1)"
echo
echo "Run the bench against this corpus with:"
echo "    cargo run --release --manifest-path ../Cargo.toml -- \\"
echo "      --fixtures $OUT/parsebench/docs \\"
echo "      --out $OUT/../../results/parsebench-bench.json"
