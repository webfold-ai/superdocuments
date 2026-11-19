"""Generate the OCR inference-stack smoke fixture.

Run from this directory:

    python3 make_smoke.py

Produces:

- `smoke.pdf` — two-paragraph text fixture using reportlab.
- `smoke.expected.txt` — alphanumeric-lowercase CER reference.
- `smoke.expected.json` — DocumentGraph ground truth (matches
  `superdocuments::graph::from_ocr_markdown` projection; the bench
  uses this for graph-fidelity scoring on inference outputs).

The `expected.json` file is written via `json.dumps` so embedded newlines
stay as the two-character `\n` escape — a hand-written heredoc often
emits raw newlines, which serde_json rejects.
"""

import json
from pathlib import Path

from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer

HERE = Path(__file__).resolve().parent


def main() -> None:
    pdf_path = HERE / "smoke.pdf"
    md = (
        "<!-- page 1 -->\n"
        "# Inference Benchmark Smoke\n"
        "\n"
        "A two-line fixture that every inference target must recover.\n"
        "\n"
        "Line two: the second line is what tests reading-order agreement.\n"
    )

    text_path = HERE / "smoke.expected.txt"
    graph_path = HERE / "smoke.expected.json"

    text_path.write_text(
        "Inference Benchmark Smoke\n"
        "A two-line fixture that every inference target must recover\n"
        "Line two the second line is what tests reading order agreement\n"
    )
    graph_path.write_text(
        json.dumps({"markdown": md, "source_label": "ocr"})
    )

    styles = getSampleStyleSheet()
    body = ParagraphStyle("body", parent=styles["Normal"], fontSize=11, leading=15)
    title = ParagraphStyle("title", parent=styles["Title"], fontSize=18, leading=22, spaceAfter=12)
    story = [
        Paragraph("Inference Benchmark Smoke", title),
        Paragraph(
            "A two-line fixture that every inference target must recover.",
            body,
        ),
        Spacer(1, 12),
        Paragraph(
            "Line two: the second line is what tests reading-order agreement.",
            body,
        ),
    ]
    SimpleDocTemplate(str(pdf_path)).build(story)

    print(f"wrote {pdf_path.name}, {text_path.name}, {graph_path.name}")


if __name__ == "__main__":
    main()
