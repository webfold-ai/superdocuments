# PDF regression fixtures

These fixtures contain authored sample text only (no external customer documents):

- `reportlab-invoice.pdf` — one-page ReportLab PDF with native text and a small invoice layout; exercises ordinary indirect objects and uncompressed content streams.
- `reportlab-ascii85.pdf` — same authored invoice with ReportLab's ASCII85+Flate stream filters.
- `scanned-text.pdf` — one raster-only page rendered from the authored phrase “Scanned fixture: OCR works.”; contains no text layer and exercises Poppler + Tesseract.

The OCR integration test is ignored by default because it requires `pdftoppm` and `tesseract` on `PATH`.

Run the OCR integration test with:

```sh
cargo test --manifest-path Cargo.toml --lib pdf::tests::tesseract_fallback_handles_raster_and_unsupported_filter_pdfs -- --ignored
```
