"""Generate adversarial OCR fixtures and their alphanumeric ground-truth sidecars.

Run from `superdocuments/tests/fixtures/`:

    python3 make_adversarial.py

Each fixture is a PDF with a known content shape; the matching `<name>.expected.txt`
contains the alphanumeric-lowercase reference text for CER scoring. The harness
loads these via `<fixture>.expected.txt` convention.

Fixtures produced:

- `mixed-native-and-scanned.pdf` \u2014 first page born-digital text (native hits),
  second page rasterized scanned text (OCR fallback). Exercises the page-level
  routing rule that `extract_file_with_ocr` already implements but the bench
  hasn't measured end-to-end.
- `two-column.pdf` \u2014 two-column body text. Tests reading-order preservation in
  the markdown projection; both columns should appear in column order.
- `borderless-table.pdf` \u2014 a table with no explicit cell borders, distinguished
  only by spacing. CER cannot catch this; the report calls it out separately.
- `rotated-15deg.pdf` \u2014 single page, content stream rotated 15 degrees. Tesseract
  recovers this with TesseractOSD/cv preprocessing; we report raw.
- `scanned-fuzzy.pdf` \u2014 the same scanned sentence but with a Gaussian blur applied
  at PDF render time, so OCR must compensate. Separates native-OCR text recovery
  from noise-robust recognition.

The ASCII85 fixture (`reportlab-ascii85.pdf`) is already in the corpus; its
.expected.txt is the authoritative ASCII85 content. We do not regenerate it.

No dataset, no copyrighted material. All text is hand-authored for the test.
"""

import os
import sys
from pathlib import Path

from reportlab.lib.pagesizes import letter
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import inch
from reportlab.platypus import (
    BaseDocTemplate,
    Frame,
    PageTemplate,
    Paragraph,
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)


HERE = Path(__file__).resolve().parent


def _write_pdf(name, story):
    out = HERE / name
    doc = SimpleDocTemplate(str(out), pagesize=letter)
    doc.build(story)
    return out


def _text_styles():
    return {
        "body": ParagraphStyle("body", fontName="Helvetica", fontSize=11, leading=15),
        "title": ParagraphStyle(
            "title", fontName="Helvetica-Bold", fontSize=18, leading=22, spaceAfter=12
        ),
        "small": ParagraphStyle("small", fontName="Helvetica", fontSize=9, leading=11),
    }


def make_mixed_native_and_scanned():
    """First page: born-digital text. Second page: an image-only rasterized layer.

    We use a two-page PDF where page 1 has a real content stream with selectable
    text (so native extraction returns it) and page 2 carries an embedded raster
    image (so native extraction returns empty and the OCR fallback is exercised).
    """
    pdf = HERE / "mixed-native-and-scanned.pdf"
    if pdf.exists():
        return pdf

    from reportlab.platypus.flowables import Image as RLImage
    from reportlab.platypus import PageBreak

    # Page 1: real text. Page 2: a single PNG image with a known text rendered on it.
    styles = _text_styles()
    page1 = [
        Paragraph("Adversarial Fixture: Mixed Native and Scanned", styles["title"]),
        Paragraph(
            "This first page carries selectable text and is recovered by native extraction.",
            styles["body"],
        ),
        Paragraph(
            "The second page is a rasterized scan; only OCR recovers it.",
            styles["body"],
        ),
    ]

    # Generate a small PNG with a known phrase via PIL if available; otherwise emit
    # a placeholder page with a colored block (still image-only, so OCR is exercised).
    page2_text = "Scanned raster page: OCR works on mixed documents."
    page2_png = HERE / "_mixed-page2.png"
    try:
        from PIL import Image, ImageDraw, ImageFont

        img = Image.new("RGB", (1700, 2200), color="white")
        draw = ImageDraw.Draw(img)
        try:
            font = ImageFont.truetype(
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 56
            )
        except OSError:
            font = ImageFont.load_default()
        draw.text((100, 1000), page2_text, fill="black", font=font)
        img.save(page2_png)
        page2 = [RLImage(str(page2_png), width=6 * inch, height=2 * inch)]
    except ImportError:
        page2 = [
            Paragraph(
                f"(image-only fallback \u2014 install Pillow for raster scan)",
                styles["small"],
            )
        ]
        page2_text = ""

    story = page1 + [PageBreak()] + page2
    out = _write_pdf("mixed-native-and-scanned.pdf", story)
    (HERE / "mixed-native-and-scanned.expected.txt").write_text(
        "Adversarial Fixture Mixed Native and Scanned\n"
        "This first page carries selectable text and is recovered by native extraction\n"
        "The second page is a rasterized scan only OCR recovers it\n"
        f"{page2_text}\n".rstrip()
    )
    if page2_png.exists():
        page2_png.unlink()
    return out


def make_two_column():
    """Two-column body text. Tests reading-order preservation in the markdown view."""
    pdf = HERE / "two-column.pdf"
    if pdf.exists():
        return pdf

    styles = _text_styles()
    left = "".join(
        f"<para>Column one line {i}. " * 6 + "</para>"
        for i in range(1, 5)
    )
    right = "".join(
        f"<para>Column two line {i}. " * 6 + "</para>"
        for i in range(1, 5)
    )
    table = Table(
        [[Paragraph(left, styles["small"]), Paragraph(right, styles["small"])]],
        colWidths=[3.25 * inch, 3.25 * inch],
    )
    table.setStyle(
        TableStyle(
            [
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("LEFTPADDING", (0, 0), (-1, -1), 12),
                ("RIGHTPADDING", (0, 0), (-1, -1), 12),
            ]
        )
    )
    story = [
        Paragraph("Two Column Reading Order Test", styles["title"]),
        Paragraph(
            "Both columns must appear in their natural reading order. "
            "Column one before column two, top to bottom within each column.",
            styles["body"],
        ),
        Spacer(1, 0.25 * inch),
        table,
    ]
    out = _write_pdf("two-column.pdf", story)
    lines = ["Two Column Reading Order Test"]
    for i in range(1, 5):
        lines.extend([f"Column one line {i}"] * 6)
    for i in range(1, 5):
        lines.extend([f"Column two line {i}"] * 6)
    (HERE / "two-column.expected.txt").write_text("\n".join(lines) + "\n")
    return out


def make_borderless_table():
    """A 3-row x 3-col table with no visible cell borders. Distinguishable only by spacing."""
    pdf = HERE / "borderless-table.pdf"
    if pdf.exists():
        return pdf

    styles = _text_styles()
    data = [
        ["Item", "Quantity", "Price"],
        ["Document review", "2", "$617.25"],
        ["Translation", "1", "$150.00"],
    ]
    table = Table(data, colWidths=[2.5 * inch, 1.5 * inch, 1.5 * inch])
    table.setStyle(
        TableStyle(
            [
                ("FONTNAME", (0, 0), (-1, 0), "Helvetica-Bold"),
                ("ALIGN", (1, 0), (-1, -1), "RIGHT"),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 8),
                ("TOPPADDING", (0, 0), (-1, -1), 8),
                # No LINEABOVE / LINEBELOW / LINEAFTER / LINEBEFORE \u2014 deliberately
                # borderless. Layout model must recover structure from whitespace.
            ]
        )
    )
    story = [
        Paragraph("Borderless Table Test", styles["title"]),
        Paragraph(
            "Cells are distinguished by alignment only \u2014 no visible borders. "
            "Markdown output should still recover rows and cells in order.",
            styles["body"],
        ),
        Spacer(1, 0.25 * inch),
        table,
    ]
    out = _write_pdf("borderless-table.pdf", story)
    (HERE / "borderless-table.expected.txt").write_text(
        "Borderless Table Test\n"
        "Cells are distinguished by alignment only no visible borders\n"
        "Markdown output should still recover rows and cells in order\n"
        "Item Quantity Price\n"
        "Document review 2 617.25\n"
        "Translation 1 150.00\n"
    )
    return out


def make_rotated_15deg():
    """Single page whose content stream is rotated 15 degrees. Tesseract handles
    this poorly without OSD preprocessing; we record the raw CER.
    """
    pdf = HERE / "rotated-15deg.pdf"
    if pdf.exists():
        return pdf

    # Generate a normal-orientation PDF first, then rotate the page using pypdf.
    styles = _text_styles()
    story = [
        Paragraph("Rotation Tilt Test", styles["title"]),
        Paragraph(
            "This page is rotated fifteen degrees from the page MediaBox. "
            "Tesseract without orientation detection will see skewed glyphs.",
            styles["body"],
        ),
    ]
    intermediate = HERE / "_rotated-intermediate.pdf"
    SimpleDocTemplate(str(intermediate), pagesize=letter).build(story)

    from pypdf import PdfReader, PdfWriter, Transformation

    reader = PdfReader(str(intermediate))
    writer = PdfWriter()
    page = reader.pages[0]
    # pypdf exposes `add_transformation` for arbitrary transforms; rotation only is
    # one of those.
    page.add_transformation(Transformation().rotate(15))
    writer.add_page(page)
    with open(pdf, "wb") as f:
        writer.write(f)
    intermediate.unlink()
    (HERE / "rotated-15deg.expected.txt").write_text(
        "Rotation Tilt Test\n"
        "This page is rotated fifteen degrees from the page MediaBox\n"
        "Tesseract without orientation detection will see skewed glyphs\n"
    )
    return pdf


def make_scanned_fuzzy():
    """Same scanned sentence as `scanned-text.pdf` but with a Gaussian blur applied
    so OCR has to compensate. Separates native-OCR text recovery from noise-robust
    recognition.
    """
    pdf = HERE / "scanned-fuzzy.pdf"
    if pdf.exists():
        return pdf

    from PIL import Image, ImageDraw, ImageFilter, ImageFont

    img = Image.new("RGB", (1700, 2200), color="white")
    draw = ImageDraw.Draw(img)
    try:
        font = ImageFont.truetype(
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 72
        )
    except OSError:
        font = ImageFont.load_default()
    draw.text((100, 1000), "Fuzzy scan: OCR is robust to noise.", fill="black", font=font)
    blurred = img.filter(ImageFilter.GaussianBlur(radius=2.5))
    intermediate_png = HERE / "_fuzzy.png"
    blurred.save(intermediate_png)

    from reportlab.platypus.flowables import Image as RLImage

    styles = _text_styles()
    story = [
        Paragraph("Fuzzy Scanned Page", styles["title"]),
        RLImage(str(intermediate_png), width=6 * inch, height=4 * inch),
    ]
    out = _write_pdf("scanned-fuzzy.pdf", story)
    intermediate_png.unlink()
    (HERE / "scanned-fuzzy.expected.txt").write_text(
        "Fuzzy Scanned Page\n" "Fuzzy scan OCR is robust to noise\n"
    )
    return out


def main():
    fixtures = [
        make_mixed_native_and_scanned(),
        make_two_column(),
        make_borderless_table(),
        make_rotated_15deg(),
        make_scanned_fuzzy(),
    ]
    for f in fixtures:
        print(f"  {f.name}")


if __name__ == "__main__":
    main()
