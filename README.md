# superdocuments

> **Status: research build.** This code is not production-ready. It is being hardened toward a production release, and interfaces, formats and results may change without notice.

Document parsing and conversion for Rust. Converts DOCX, DOCM, PPTX, XLSX, XLSM, XLSB, ODT, ODP, ODS, EPUB, RTF, CSV, HTML and plain text to Markdown, and extracts text from PDF.

## What is in the crate

- `document`: `convert_bytes`, `convert_path` and `convert_streaming` pick a converter from the file extension. Supported: `docx`/`docm`, `pptx`, `xlsx`/`xlsm`, `xlsb`, `odt`/`odp`, `ods`, `epub`, `rtf`, `csv`, `html`/`htm`, `txt`/`md`/`markdown`. `convert_cached` and `fingerprint_bytes` provide a fold cache keyed on input content.
- `html`: HTML to Markdown, with structured blocks and extracted links. Navigation and script subtrees are skipped on a tag stack.
- `pdf`: in-process PDF text extraction to Markdown, with bounded input. Scanned or text-less pages can fall back to external Tesseract and Poppler binaries when `extract_file_with_ocr` is used.
- `epub`, `xlsx`, `markdown` (parsed with `hashtrinity::codec::syntax`) and `graph`.
- Optional features: `framefold` (renderer, `frame_renderer`), and `media-codecs` / `images` (re-export of the Transcode codecs). Both depend on git repositories listed below.

## Known gaps

- Legacy binary `.xls` is not converted here. `convert_bytes` returns an error for it.
- PDF coverage is limited. Mixed born-digital and scanned pages, unusual PDF filters, and encrypted or object-stream variants need more fixtures and tests.
- HTML parity fixtures and the public API are still being settled.
- Not verified in the public-release change: the test suite was not run in the environment that prepared it, because the git dependencies below are not reachable from there.

## Dependencies

Some dependencies are git repositories rather than crates.io packages:

- `hashtrinity` (`github.com/Ptyktos/hashtrinity`), always required.
- `framefold` and `transcode` (`github.com/Ptyktos/...`), only for the optional features.
- `apeiron`, which comes through the `hashtrinity` repository at a pinned revision.

A clean build needs access to these repositories. Cargo resolves optional dependencies into `Cargo.lock`, so `framefold` and `transcode` must be reachable even when their features are off.

## Build and test

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

The minimum supported Rust version is 1.85 (`rust-version` in `Cargo.toml`).

## Licence

The crate is licensed under the Apache License, Version 2.0 (`LICENSE`). The `calamine/` sub-crate is MIT (`calamine/Cargo.toml`), and its upstream authorship is retained there.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and [SECURITY.md](SECURITY.md). Architecture and format notes are in [docs/](docs/).
