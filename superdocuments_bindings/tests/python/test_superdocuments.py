"""Tests for the `superdocuments` Python module, against real documents.

Run with scripts/test-python.sh, which builds the extension first. The Office, RTF and PDF inputs are
the repository's own fixtures (../tests/fixtures); the EPUB and the hostile inputs are built here.
"""

import io
import json
import pathlib
import time
import unittest
import zipfile

import superdocuments as sd

FIXTURES = pathlib.Path(__file__).resolve().parents[3] / "tests" / "fixtures"


def fixture(name):
    return (FIXTURES / name).read_bytes()


def make_epub():
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr(
            "META-INF/container.xml",
            '<?xml version="1.0"?><container version="1.0" '
            'xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles>'
            '<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>'
            "</rootfiles></container>",
        )
        z.writestr(
            "OEBPS/content.opf",
            '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" '
            'unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">'
            '<dc:identifier id="id">x</dc:identifier><dc:title>Fixture Book</dc:title></metadata>'
            '<manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>'
            '<spine><itemref idref="c1"/></spine></package>',
        )
        z.writestr(
            "OEBPS/c1.xhtml",
            '<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter One</h1>'
            "<p>Call me <b>Ishmael</b>.</p></body></html>",
        )
    return buf.getvalue()


class Convert(unittest.TestCase):
    def test_docx(self):
        md = sd.convert(fixture("office/basic.docx"), "docx")
        self.assertIn('title: "Quarterly Report"', md)
        self.assertIn("**Bold part**", md)
        self.assertIn("- First bullet", md)
        self.assertIn("| Name |", md)

    def test_xlsx_sheets_become_sections(self):
        md = sd.convert(fixture("office/basic.xlsx"), "xlsx")
        self.assertIn("## Costs", md)
        self.assertIn("## Revenue", md)
        self.assertIn("| A & B | 2026-11-07 | 1234.5 | TRUE |", md)

    def test_pptx_slides(self):
        md = sd.convert(fixture("office/basic.pptx"), "pptx")
        self.assertIn("## Slide 1:", md)
        self.assertIn("**Notes:** Speaker note here", md)

    def test_odt_and_ods(self):
        self.assertIn("# Overview", sd.convert(fixture("office/basic.odt"), "odt"))
        self.assertIn("## Costs", sd.convert(fixture("office/basic.ods"), "ods"))

    def test_rtf_with_a_link_and_a_footnote(self):
        md = sd.convert(fixture("office/rich.rtf"), "rtf")
        self.assertIn("[the site](https://example.com/a?x=1&y=2)", md)
        self.assertIn("Footnote body & text.", md)

    def test_legacy_xls(self):
        md = sd.convert(fixture("xls/lo-sheets.xls"), "xls")
        self.assertTrue(md.strip(), "an .xls fixture produced no text")
        self.assertIn("|", md)

    def test_epub_built_here(self):
        md = sd.convert(make_epub(), "epub")
        self.assertIn("Chapter One", md)
        self.assertIn("Ishmael", md)

    def test_csv_and_plain_text(self):
        md = sd.convert(b"name,qty\nbolt,3\nnut,5\n", "csv")
        self.assertIn("| name | qty |", md)
        self.assertIn("| bolt | 3 |", md)
        self.assertEqual(sd.convert(b"# just markdown", "md"), "# just markdown")

    def test_html(self):
        md = sd.convert(b"<html><body><h1>Title</h1><p>Body <b>text</b></p></body></html>", "html")
        self.assertIn("# Title", md)

    def test_the_extension_is_normalised(self):
        a = sd.convert(fixture("office/basic.docx"), "docx")
        self.assertEqual(a, sd.convert(fixture("office/basic.docx"), ".DOCX"))
        self.assertEqual(a, sd.convert(fixture("office/basic.docx"), " docx "))

    def test_unsupported_extension_is_named(self):
        with self.assertRaises(ValueError) as cm:
            sd.convert(b"MZ", "exe")
        self.assertIn("unsupported document extension: exe", str(cm.exception))


class Pdf(unittest.TestCase):
    def test_native_text_pdf(self):
        md = sd.convert(fixture("reportlab-invoice.pdf"), "pdf")
        self.assertIn("# Invoice Fixture 2025", md)
        self.assertIn("INV-2048", md)
        self.assertEqual(md, sd.pdf_to_markdown(fixture("reportlab-invoice.pdf")))

    def test_pages(self):
        pages = sd.pdf_pages(fixture("reportlab-invoice.pdf"))
        self.assertEqual(len(pages), 1)
        self.assertIn("Total amount: $1,234.50", pages[0])

    def test_an_image_only_pdf_has_no_ocr_text_and_does_not_shell_out(self):
        # No OCR in this API: a scan gives an image reference at most, never its recognised text.
        out = sd.convert(fixture("scanned-text.pdf"), "pdf")
        self.assertNotIn("Scanned fixture", out)
        self.assertNotIn("OCR works", out)


class Worksheets(unittest.TestCase):
    def test_structured_rows(self):
        sheets = json.loads(sd.worksheets(fixture("office/basic.xlsx")))
        self.assertEqual([s["name"] for s in sheets], ["Costs", "Revenue"])
        self.assertEqual([s["index"] for s in sheets], [0, 1])
        self.assertEqual(sheets[0]["rows"], [["x", "y"], ["1", "2"]])
        self.assertEqual(sheets[1]["rows"][1][0], "A & B")


class Hostile(unittest.TestCase):
    """A bad document is a ValueError, and never a crash or a hang."""

    def check(self, data, ext):
        start = time.monotonic()
        with self.assertRaises(ValueError):
            sd.convert(data, ext)
        self.assertLess(time.monotonic() - start, 20, f"{ext} took too long to reject")

    def test_empty_input(self):
        for ext in ("docx", "xlsx", "pptx", "odt", "ods", "epub", "xls", "xlsb", "rtf", "pdf"):
            with self.subTest(ext=ext):
                self.check(b"", ext)

    def test_truncated_archives(self):
        for name, ext in (("office/basic.docx", "docx"), ("office/basic.xlsx", "xlsx"), ("office/basic.pptx", "pptx"), ("office/basic.odt", "odt")):
            data = fixture(name)
            for cut in (4, len(data) // 2, len(data) - 10):
                with self.subTest(name=name, cut=cut):
                    self.check(data[:cut], ext)

    def test_wrong_extension_with_a_header_check_is_an_error(self):
        docx = fixture("office/basic.docx")
        for ext in ("xls", "pdf", "rtf", "odt", "ods"):
            with self.subTest(ext=ext):
                self.check(docx, ext)

    def test_a_valid_package_of_another_format_is_an_error_not_empty_markdown(self):
        # Every Office package handed to the converter of a different OOXML format, or to the
        # EPUB converter, is refused: empty Markdown would look like a document that is empty.
        fixtures = {
            "docx": "office/basic.docx",
            "xlsx": "office/basic.xlsx",
            "pptx": "office/basic.pptx",
        }
        for real, path in fixtures.items():
            data = fixture(path)
            self.assertTrue(sd.convert(data, real).strip(), f"{path} is empty as {real}")
            for wrong in ("docx", "xlsx", "pptx", "epub"):
                if wrong == real:
                    continue
                with self.subTest(real=real, wrong=wrong):
                    with self.assertRaises(ValueError):
                        sd.convert(data, wrong)
        with self.assertRaises(ValueError):
            sd.worksheets(fixture("office/basic.docx"))

    def test_random_bytes(self):
        junk = bytes((i * 131 + 7) % 256 for i in range(4096))
        for ext in ("docx", "xlsx", "pptx", "odt", "epub", "xls", "xlsb", "pdf"):
            with self.subTest(ext=ext):
                self.check(junk, ext)

    def test_a_zip_bomb_is_refused_quickly(self):
        # 400 MB of zeros deflates to about 400 KB: the expansion limit must stop it.
        buf = io.BytesIO()
        with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("[Content_Types].xml", "<Types/>")
            z.writestr("word/document.xml", b"\0" * (400 * 1024 * 1024))
        bomb = buf.getvalue()
        self.assertLess(len(bomb), 5 * 1024 * 1024)
        start = time.monotonic()
        try:
            sd.convert(bomb, "docx")
        except ValueError:
            pass  # rejected: the expected outcome
        else:
            self.fail("a 400 MB expansion was accepted")
        self.assertLess(time.monotonic() - start, 20)

    def test_worksheets_on_junk(self):
        with self.assertRaises(ValueError):
            sd.worksheets(b"not a zip")


class Meta(unittest.TestCase):
    def test_extensions_and_version(self):
        exts = sd.extensions()
        for e in ("docx", "xlsx", "pptx", "epub", "rtf", "csv", "pdf"):
            self.assertIn(e, exts)
        self.assertRegex(sd.version(), r"^\d+\.\d+\.\d+")


if __name__ == "__main__":
    unittest.main()
