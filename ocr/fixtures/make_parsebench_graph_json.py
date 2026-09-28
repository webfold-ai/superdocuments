"""Generate `expected.json` (DocumentGraph ground truth) for the ParseBench
PDFs downloaded by `fetch_public_benchmarks.sh`.

For each PDF that has an `expected.txt` sidecar, this script reads the
corresponding ParseBench JSONL rule file, extracts the *raw* markdown or
content text (not the alphanumeric-normalized version), and writes
`<name>.expected.json` so the bench's graph-fidelity scoring has a coherent
reference.

The convention: ParseBench's `text_content.jsonl` carries either
`bag_of_sentence` (one entry per expected sentence) or `expected_markdown`
(whole-document markdown). We project both shapes into a single `markdown`
string for `DocumentGraph::from_ocr_markdown`.

Run from the fixtures dir after `fetch_public_benchmarks.sh` completes:

    python3 make_parsebench_graph_json.py
"""

import json
import os
import sys
from pathlib import Path


def normalize_md_for_bench(text: str) -> str:
    """Light cleanup so `from_ocr_markdown` parses the JSONL markdown cleanly.

    ParseBench's `expected_markdown` for table rows is a raw HTML <table>;
    `from_ocr_markdown` already handles raw HTML tables. For text blocks we
    pass through unchanged. Page markers (`<!-- page N -->`) are prepended
    so the resulting graph has exactly one Page node.
    """
    return "<!-- page 1 -->\n" + text.strip() + "\n"


def normalize_layout_for_bench(blocks: list[tuple[int, str]]) -> str:
    """Project a list of (ro_index, content) pairs into a single markdown
    document with `<!-- page 1 -->` and one paragraph per block.
    """
    blocks = sorted(blocks, key=lambda b: b[0])
    paragraphs = [c.strip() for _, c in blocks if c and c.strip()]
    if not paragraphs:
        return ""
    return "<!-- page 1 -->\n" + "\n\n".join(paragraphs) + "\n"


def extract_table_markdown(jsonl_path: Path, pdf: str) -> str | None:
    """Find the `expected_markdown` value for `pdf` in the table JSONL."""
    if not jsonl_path.exists():
        return None
    with open(jsonl_path) as f:
        for line in f:
            row = json.loads(line)
            if row.get("pdf") != pdf:
                continue
            em = row.get("expected_markdown")
            if em:
                return em
    return None


def extract_layout_blocks(jsonl_path: Path, pdf: str) -> list[tuple[int, str]]:
    """Collect (ro_index, content) for every row of `pdf` in the layout JSONL."""
    out: list[tuple[int, str]] = []
    if not jsonl_path.exists():
        return out
    with open(jsonl_path) as f:
        for line in f:
            row = json.loads(line)
            if row.get("pdf") != pdf:
                continue
            ro_index = row.get("ro_index", 0) or 0
            try:
                rule = json.loads(row["rule"])
            except Exception:
                continue
            content = rule.get("content") if isinstance(rule, dict) else None
            # ParseBench's layout rule's content is itself a dict like
            # `{"text": "...", "type": "text"}` (or `{"text": "...", "type": "table"}`).
            text = None
            if isinstance(content, dict):
                text = content.get("text")
            elif isinstance(content, str):
                text = content
            if isinstance(text, str) and text.strip():
                out.append((ro_index, text))
    return out


def extract_text_sentences(jsonl_path: Path, pdf: str) -> str | None:
    """Find `bag_of_sentence` content for `pdf` in the text JSONLs."""
    if not jsonl_path.exists():
        return None
    with open(jsonl_path) as f:
        for line in f:
            row = json.loads(line)
            if row.get("pdf") != pdf:
                continue
            try:
                rule = json.loads(row["rule"])
            except Exception:
                continue
            if "bag_of_sentence" in rule and isinstance(rule["bag_of_sentence"], dict):
                return list(rule["bag_of_sentence"].keys())
    return None


def main() -> None:
    fetched = Path(__file__).resolve().parent / "fetched"
    docs = fetched / "parsebench" / "docs"
    annotations = fetched / "parsebench" / "annotations"

    table_jsonl = annotations / "table.jsonl"
    layout_jsonl = annotations / "layout.jsonl"
    text_content_jsonl = annotations / "text_content.jsonl"

    written = 0
    skipped = 0
    for pdf_path in sorted(docs.rglob("*.pdf")):
        # `pdf_path` is relative to `docs/`; the JSONL uses `docs/<cat>/<name>.pdf`
        rel = pdf_path.relative_to(docs)
        pdf_id = "docs/" + str(rel)
        expected_txt = pdf_path.with_suffix(".expected.txt")
        if not expected_txt.exists():
            skipped += 1
            continue

        md: str | None = None
        # Order matters: tables and layouts have specific structures that
        # outrank the generic text_content rules.
        table_md = extract_table_markdown(table_jsonl, pdf_id)
        if table_md:
            md = normalize_md_for_bench(table_md)
        else:
            layout_blocks = extract_layout_blocks(layout_jsonl, pdf_id)
            if layout_blocks:
                md = normalize_layout_for_bench(layout_blocks)
            else:
                sentences = extract_text_sentences(text_content_jsonl, pdf_id)
                if sentences:
                    md = (
                        "<!-- page 1 -->\n"
                        + "\n\n".join(sentences)
                        + "\n"
                    )
        if md is None:
            skipped += 1
            continue

        out_path = pdf_path.with_suffix(".expected.json")
        out_path.write_text(
            json.dumps({"markdown": md, "source_label": "parsebench"})
        )
        written += 1

    print(f"wrote {written} expected.json sidecars, skipped {skipped}")


if __name__ == "__main__":
    main()
