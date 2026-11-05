# EPUB

`superdocuments::epub` is the substrate's spine-driven EPUB reader. It
reads the OCF (Open Container Format) container, indexes the package
manifest in an `apeiron::Map`, and walks the spine one chapter at a
time. Each chapter is read and converted separately, but `extract_text`
accumulates the whole result in one `String`, capped at 64 MiB.

This document is the *substrate* half of the EPUB story. The webfold
client side lives in `Ptyktos/webfold/docs/EPUB-WALK.md`.

## What is in scope

- Reading the EPUB container (META-INF/container.xml → OPF package).
- Resolving the spine (canonical reading order) against the manifest.
- Extracting each spine entry as Markdown via the existing
  `superdocuments::html::to_markdown` pipeline.
- Bounded input and `Err(String)` errors. The module has no panic containment.

## What is NOT in scope

- CSS, embedded fonts, images, scripting, DRM. The extract is a typed
  text spine; if a caller needs a chapter's media, they re-open the
  EPUB with their own reader.
- PDF or DOCX (those are the EPUB viewer's sibling download formats
  and are handled by their own modules).
- Network acquisition. The substrate takes bytes; the fetch is the
  caller's job (`webfold fetch` in the Ptyktos stack).

## Public surface

```rust
pub struct Book {
    manifest: Map<String, ManifestItem>,  // private
    spine:    Vec<SpineItem>,             // private
    opf_path: String,                     // recorded
    title:    Option<String>,             // from OPF metadata
}

pub struct ManifestItem { pub id: String, pub href: String, pub media_type: String }
pub struct SpineItem    { pub id: String, pub href: String }

pub fn read(bytes: &[u8]) -> Result<Book, String>;
pub fn extract_text(book: &Book, bytes: &[u8]) -> Result<String, String>;
pub fn spine(book: &Book) -> &[SpineItem];
pub fn title(book: &Book) -> Option<&str>;
pub fn manifest_get<'a>(book: &'a Book, id: &str) -> Option<&'a ManifestItem>;
```

`read` and `extract_text` are intentionally separate. The caller
passes the original bytes to `read`; `extract_text` reopens the ZIP
from the same bytes and walks the spine. This keeps the `Book` value
cheap to clone and store (it holds the manifest index and the spine,
not the file bytes) and lets a caller re-walk with a different
strategy (e.g. a chapter-by-chapter API) without re-parsing the
container.

## How the walk works

```
                 ┌────────────────────────────────────┐
META-INF/        │ read(bytes)                        │
  container.xml ─┤   1. open ZIP                       │
                  │   2. read META-INF/container.xml   │
                  │      → OPF path                    │
                  │   3. read OPF                      │
                  │      → manifest (id → item)        │
                  │      → spine idref list (in order) │
                  │      → title (from <metadata>)     │
                  │   4. resolve each spine idref      │
                  │      against the OPF directory    │
                  └────────────┬───────────────────────┘
                               ▼
                         Book { manifest, spine, title }
                               │
                               ▼
            ┌──────────────────────────────────────┐
            │ extract_text(book, bytes)            │
            │   for each spine entry (in order):    │
            │     1. read the entry from the ZIP    │
            │     2. superdocuments::html::         │
            │        to_markdown(&entry, "")        │
            │     3. append to output               │
            │   4. return the full Markdown text     │
            └──────────────────────────────────────┘
```

## Why the manifest is `apeiron::Map<String, ManifestItem>`

The id → item lookup runs once per spine entry. `apeiron::Map<String, _>`
is the table the local `calamine/` fork also uses for its XLSX and XLSB
path caches and relationship indexes (`calamine/APEIRON-FORK.md`). No
benchmark comparing it with `HashMap` is recorded in this repository.

`apeiron::Map` also exposes a borrowed-key `get_at<Q: Locate<K>>(&self, q: Q)`
that accepts `&str` directly. The public `manifest_get` API uses that,
so a caller iterating ids does not allocate on the lookup path.

## Why spine-driven, not TOC-driven

The table of contents is **navigation metadata** attached to a book. It
is the human-facing index. The spine is the **canonical reading
order** — what a renderer walks. For an extractor the spine is the
right list to walk; the TOC is a separate problem (linking ids to
spine positions) that this module deliberately does not solve.

`rbook` and `danigm/epub-rs` both expose a `spine()` method with the
same shape. We follow the same convention.

## Bounded input

- `MAX_TOTAL` = 256 MiB (the whole EPUB)
- `MAX_ENTRY` = 64 MiB per ZIP entry
- `MAX_ENTRIES` = 8192 entries
- `MAX_XHTML_BYTES` = 16 MiB per spine chapter
- `MAX_TEXT_BYTES` = 64 MiB of rendered text

A book that exceeds the text cap is refused with a clear error rather
than a partial render. The walker does not catch this; the caller
sees `Err(String)` from `extract_text`. Silently truncating would be
a worse failure mode than refusing the book — the alternative is
"chapter 47 of 360 appeared; the rest is missing", which a caller
that wants completeness cannot detect from the outside.

A broken spine idref (an `<itemref idref="…">` that has no matching
`<item id="…">` in the manifest) is logged on stderr and skipped.
A missing spine entry (the package is consistent but the ZIP is
missing one chapter) is an error from `extract_text` that names the
missing file. The walker does not panic on either case.

## Tests

Seven unit tests in `src/epub.rs`:

| test | pins |
|---|---|
| `read_parses_manifest_and_spine_in_order` | The spine order and href resolution against the OPF directory. |
| `manifest_get_is_o1_lookups_by_id` | The `apeiron::Map` id lookup is a real `get_at` call. |
| `extract_text_walks_spine_in_order` | A two-chapter fixture comes out with chapter 1 before chapter 2 in the rendered text. |
| `spine_with_broken_idref_is_skipped_not_panicked` | A package whose spine lists a non-existent id drops the chapter and reads the rest. |
| `extract_text_refuses_a_missing_spine_entry` | A consistent package whose ZIP is missing one chapter surfaces a clean error that names the file. |
| `opf_dirname_strips_the_filename_only` | Path helper. |
| `join_href_resolves_against_opf_dir` | Path helper, with the backslash and absolute-href edge cases. |

## What we did not adopt

- **`danigm/epub-rs`** (`epub` crate on crates.io). A reasonable
  reader, but adding a new crate dep for a 200-line reader that uses
  the same `quick-xml` + `zip` the crate already has is a wash. The
  xlsx.rs pattern (own parser, no extra crate) is the shape used here.
- **`rbook`**. Cleaner document model, but same shape as `danigm/epub-rs`
  and same observation: we don't need its full DocC model for an
  extractor.
- **`epub-stream`**. Designed for bounded memory on constrained
  devices, SAX-style XML, tokenization. Worth a look for a future
  SAX rewrite if the in-memory `quick-xml` reader ever shows up in a
  profile, but the current `quick-xml` reader is already streaming per
  spine entry and we don't have a memory profile that says otherwise.
- A separate **Tan**/Tantivy index. The EPUB module is read-only and
  short. No second-store index.

## Where the EPUB came from

The 2026-10-07 VoIP regulatory dogfood record
(`Ptyktos/webfold/bench/dogfood/2026-10-07-voip-regulatory/dogfood-webfold.md`)
flagged two gaps (Gap #1+#2) about `legislation.gov.au` `*/text` URLs
returning only the EPUB TOC. The owner's correction to the originating
task made it clear the gap was general, not legislation-specific: any
site that publishes an EPUB needs the same walk. This module is the
substrate change; the client wiring lives in
`Ptyktos/webfold/src/main.rs` and is documented in
`Ptyktos/webfold/docs/EPUB-WALK.md`.
