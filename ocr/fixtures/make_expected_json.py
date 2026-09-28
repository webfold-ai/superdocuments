"""Generate DocumentGraph ground-truth JSON sidecars for every PDF in this
directory that has only an `.expected.txt` (alphanumeric CER reference).

The bench (`superdocuments::graph::DocumentGraph`) reads each fixture's
`.expected.json` and projects it through `from_ocr_markdown` to build the
ground-truth graph. The JSON's `markdown` string is structured to mirror
the per-fixture CER ground truth plus `<!-- page N -->` markers so the
graph-fidelity scoring (reading order, table cells, native-preservation)
has a coherent reference to compare against.

Each fixture's structure is documented inline below — the script picks
the right structure by name. New fixtures need a small entry added here.
"""

import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent

# Per-fixture structure: (title, body_paragraphs, tables). Body is a list of
# either strings (paragraphs) or list-of-lists (tables — one list per row).
STRUCTURES = {
    "smoke": (
        "Inference Benchmark Smoke",
        [
            "A two-line fixture that every inference target must recover.",
            "Line two: the second line is what tests reading-order agreement.",
        ],
        [],
    ),
    "reportlab-invoice": (
        "Invoice Fixture 2025",
        [
            "Prepared for Example Research Ltd. Invoice number: INV-2048.",
            "Payment is due within thirty days. Total amount: $1,234.50.",
        ],
        [],
    ),
    "reportlab-ascii85": (
        "Invoice Fixture 2025",
        [
            "Prepared for Example Research Ltd. Invoice number: INV-2048.",
            "Payment is due within thirty days. Total amount: $1,234.50.",
        ],
        [
            [
                ["Item", "Quantity", "Price"],
                ["Document review", "2", "$617.25"],
            ]
        ],
    ),
    "scanned-text": (
        None,  # The fixture has no selectable text; only the OCR layer recovers it.
        [
            "Scanned fixture: OCR works.",
        ],
        [],
    ),
    "mixed-native-and-scanned": (
        "Adversarial Fixture: Mixed Native and Scanned",
        [
            "This first page carries selectable text and is recovered by native extraction.",
            "The second page is a rasterized scan; only OCR recovers it.",
        ],
        [],
    ),
    "two-column": (
        "Two Column Reading Order Test",
        [
            "Both columns must appear in their natural reading order. Column one "
            "before column two, top to bottom within each column.",
        ],
        [],
    ),
    "borderless-table": (
        "Borderless Table Test",
        [
            "Cells are distinguished by alignment only — no visible borders. "
            "Markdown output should still recover rows and cells in order.",
        ],
        [
            [
                ["Item", "Quantity", "Price"],
                ["Document review", "2", "$617.25"],
                ["Translation", "1", "$150.00"],
            ]
        ],
    ),
    "rotated-15deg": (
        "Rotation Tilt Test",
        [
            "This page is rotated fifteen degrees from the page MediaBox. "
            "Tesseract without orientation detection will see skewed glyphs.",
        ],
        [],
    ),
    "scanned-fuzzy": (
        "Fuzzy Scanned Page",
        [
            "Fuzzy scan: OCR is robust to noise.",
        ],
        [],
    ),
}


def build_markdown(name: str) -> str:
    title, paragraphs, tables = STRUCTURES[name]
    out_lines: list[str] = ["<!-- page 1 -->"]
    if title:
        out_lines.append(f"# {title}")
        out_lines.append("")
    for p in paragraphs:
        out_lines.append(p)
        out_lines.append("")
    for table in tables:
        for row_idx, row in enumerate(table):
            cells = " | ".join(c.replace("|", "\\|") for c in row)
            out_lines.append(f"| {cells} |")
            if row_idx == 0:
                out_lines.append("|" + "|".join(" --- " for _ in row) + "|")
        out_lines.append("")
    return "\n".join(out_lines).rstrip() + "\n"


def main() -> None:
    written: list[str] = []
    skipped: list[str] = []
    for pdf_path in sorted(HERE.glob("*.pdf")):
        name = pdf_path.stem
        if name not in STRUCTURES:
            skipped.append(name)
            continue
        json_path = HERE / f"{name}.expected.json"
        md = build_markdown(name)
        json_path.write_text(
            json.dumps({"markdown": md, "source_label": "ocr"}, ensure_ascii=False)
        )
        written.append(json_path.name)
    print(f"wrote {len(written)} expected.json sidecars:")
    for w in written:
        print(f"  {w}")
    if skipped:
        print(f"skipped (no STRUCTURES entry): {skipped}")


if __name__ == "__main__":
    main()
