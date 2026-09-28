# PDF regression fixtures

These fixtures contain authored sample text only (no external customer documents):

- `reportlab-invoice.pdf` — one-page ReportLab PDF with native text and a small invoice layout; exercises ordinary indirect objects and uncompressed content streams.
- `reportlab-ascii85.pdf` — same authored invoice with ReportLab's ASCII85+Flate stream filters; exercises the OCR fallback when the current native parser cannot decode that filter chain.
- `scanned-text.pdf` — one raster-only page rendered from the authored phrase “Scanned fixture: OCR works.”; contains no text layer and exercises Poppler + Tesseract.

The OCR integration test is ignored by default because it requires `pdftoppm` and `tesseract` on `PATH`. On the local one-page scan, Tesseract matched all 22 normalized alphanumeric characters (CER 0); `pdftotext` returned no text. End-to-end OCR was 3.13 s on this host, a smoke measurement rather than a corpus benchmark.

A non-bundled spot-check against CapTable's OpenOffice Writer 2.1 `dummy.pdf` produced the same normalized `dummypdffile` as Poppler `pdftotext` in 0.03 s. It is not copied here because its redistribution terms are unclear.

Run the OCR integration test with:

```sh
cargo test --manifest-path Cargo.toml --lib pdf::tests::tesseract_fallback_handles_raster_and_unsupported_filter_pdfs -- --ignored
```
