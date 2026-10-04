# PDF outline coverage — measurement on the 34-PDF PageIndex benchmark slice

**This is a coverage measurement on a 34-document slice, not a quality claim about
PDF outlines, and not a benchmark result.** It answers one question: how many of
these documents even carry a producer-authored bookmark tree — the LLM-free
structure source that PageIndex otherwise substitutes with LLM calls.

Worktree: `/home/ctown/orca/workspaces/superdocuments/w3-superdocuments-structure`
Corpus (READ ONLY, never copied): `…/w2-agent-evals-pageindex-bench/data/PageIndex-OSS-Benchmark/documents`
Reference trees (READ ONLY): `…/.pageindex/docs/*/tree.json`, `…/.pageindex/manifest.json`

## API added (additive; no existing signature changed)

`src/pdf/outline.rs`, registered as `superdocuments::pdf::outline`:

| Item | Purpose |
| --- | --- |
| `OutlineNode { title, level (1-based), start_page: Option<u32> (1-based), children }` | one bookmark |
| `OutlineError::{Parse(String), NoCatalog}` | malformed input; a document with no `/Outlines` is `Ok(vec![])`, not an error |
| `outline_tree(bytes) -> Result<Vec<OutlineNode>, OutlineError>` | walks catalog `/Outlines` → `/First` → `/Next`, recursing `/First` for children |
| `page_count(bytes) -> Result<u32, OutlineError>` | 1-based page count from the page tree |
| `to_pageindex_tree_json(&[OutlineNode], page_count) -> String` | PageIndex-shaped JSON |
| `to_pageindex_tree_json_reporting(...) -> PageIndexTree { json, unresolved_titles }` | same JSON **plus** the titles whose destination did not resolve (those inherit the previous start page) |
| `node_count`, `max_depth`, `resolved_counts` | coverage helpers used by the example |
| `decode_title`, `decode_pdf_text`, `write_json_string` | UTF-16BE-with-BOM / PDFDocEncoding decoding, hand-rolled JSON string escaping |
| `MAX_OUTLINE_DEPTH` (64), `MAX_OUTLINE_NODES` (20 000) | bounds |

Destination resolution covers `/Dest` arrays, `/A` GoTo actions, and **named**
destinations through both the catalog `/Dests` dictionary and the `/Names /Dests`
name tree (with `/Kids` recursion). Page numbers come from the page-tree order
(`lopdf::Document::get_pages`). Titles and destinations are frequently *indirect*
(`/Title 665 0 R`, `/D 1444 0 R`) — dereferencing both raised resolution on the
corpus from 0 % to ~100 % for outlined PDFs. Empty-password encryption is
attempted as a best effort.

Defensive behaviour: a **single shared visited set** across `/First`, `/Next` and
`/Parent` (so a `/Next` loop, a self-parented item, and a self-child all
terminate), a depth limit, a node-count limit, malformed input → `Err`, no
outlines → empty `Vec`, no panics on garbage bytes.

JSON is produced with a small hand-rolled encoder (no new dependency) and has a
unit test covering quotes, backslashes, `\n \r \t \b \f`, `\u0001`, non-BMP
(`😀`) and round-trips through `serde_json` in the test.

**One documented convention difference:** `end_index` is implemented as the task
specified — the page *before* the next node at the same or a shallower level
starts (inclusive last page). PageIndex's own `tree.json` files use an *exclusive*
`end_index` equal to the next node's `start_index`, i.e. one page later. Anyone
diffing the two shapes must account for that off-by-one.

## Commits (local only, never pushed)

| Hash | Contents |
| --- | --- |
| `0ed97c2` | library + in-module unit tests (`src/pdf/outline.rs`, `mod` declaration) |
| `74f1baa` | `examples/outline_coverage.rs` |

Unrelated churn from `cargo fmt` on pre-existing files and the `Cargo.lock`
`patch.unused` section was reverted; the tree is clean at `74f1baa`.

## Test counts

- `cargo test --lib pdf::outline` → **11 passed, 0 failed, 0 ignored** (142 filtered out)
- `cargo test --lib` → **149 passed, 0 failed, 4 ignored** (was 138 passed / 4 ignored → +11, all new)

Test coverage of the required cases: 2-level outline with direct `/Dest`,
`/A` GoTo, catalog `/Dests` named destination **and** `/Names /Dests` name-tree
destination (one test, all four paths), cyclic `/Next` chain (terminates, each
node once), self-referential `/First`+`/Next` (terminates), no `/Outlines`
(empty `Vec`), UTF-16BE-BOM title (`Ünïcode`), missing-outline-object,
malformed bytes, PDFDocEncoding trademark sign, unpaired surrogate, JSON
escaping, `end_index`/inheritance/unresolved reporting, depth limit, parseable
JSON shape.

## Coverage measurement — per-PDF table (34 PDFs)

```
file                                             pages has_outline  nodes  depth resolved%
0b85477387a9d0cc33fca0f4becaa0e5.pdf                16      false      0      0      0.0%
2310.05634v2.pdf                                    24       true     37      2    100.0%
2312.04350v3.pdf                                    28       true     44      3    100.0%
2312.09390v1.pdf                                    49       true     48      3    100.0%
2312.10997v5.pdf                                    21       true     49      3    100.0%
2401.18059v1.pdf                                    23       true     23      2    100.0%
379f44022bb27aa53efd5d322c7b57bf.pdf                17       true     48      3    100.0%
698bba535087fa9a7f9009e172a7f763.pdf                20      false      0      0      0.0%
7c3f6204b3241f142f0f8eb8e1fefe7a.pdf                15      false      0      0      0.0%
8dfc21ec151fb9d3578fc32d5c4e5df9.pdf                18       true      8      1    100.0%
936c0e2c2e6c8e0c07c51bfaf7fd0a83.pdf                15      false      0      0      0.0%
ACTIVISIONBLIZZARD_2019_10K.pdf                    198      false      0      0      0.0%
ADOBE_2015_10K.pdf                                 116       true     76      4    100.0%
AMAZON_2017_10K.pdf                                 85      false      0      0      0.0%
BESTBUY_2023_10K.pdf                                75      false      0      0      0.0%
COSTCO_2021_10K.pdf                                 76      false      0      0      0.0%
Campaign_038_Introducing_AC_Whitepaper_v5e.pdf      28       true      1      1    100.0%
NETFLIX_2015_10K.pdf                                72      false      0      0      0.0%
NYU_graduate.pdf                                    30       true      2      2     50.0%
SAO-StudentSupport_Guidebook-Content.pdf            44      false      0      0      0.0%
User_Manual_1500S_Classic_EN.pdf                   108       true    163      4    100.0%
a4f3ced0696009fec3179f493e4f28c4.pdf                17      false      0      0      0.0%
a5879805d70c854ea4361e43a84e3bb2.pdf                15      false      0      0      0.0%
camry_ebrochure.pdf                                 26      false      0      0      0.0%
e639029d16094ea71d964e2fb953952b.pdf                20      false      0      0      0.0%
e79deb02a0c0e87511080836c5d4347b.pdf                17      false      0      0      0.0%
f1f5242528411b262be447e61e2eb10f.pdf                20      false      0      0      0.0%
f8d3a162ab9507e021d83dd109118b60.pdf                17      false      0      0      0.0%
fdac8d1e9ef56519371df7e6532df27d.pdf                20      false      0      0      0.0%
mi_phone.pdf                                        37      false      0      0      0.0%
mmdetection-readthedocs-io-en-v2.18.0.pdf          468       true    398      4    100.0%
owners-manual-2170416.pdf                            32       true     44      2    100.0%
stereo_headset.pdf                                   0      false      0      0      0.0%
watch_d.pdf                                         27       true     86      3    100.0%
```

### Summary

```
pdfs                              34
with at least one outline node     14   (41.2 %)
with at least 3 outline nodes      12   (35.3 %)
median node count (all pdfs)       0
median node count (>=3 nodes)      48
median percent nodes resolved      0.0 %   (over all 34; 99.3 % over the 14 outlined)
```

Reading of the numbers, on this slice only:

- **Roughly 4 documents in 10 have no outline at all.** All the big 10-Ks here
  (ACTIVISIONBLIZZARD, AMAZON, BESTBUY, COSTCO, NETFLIX) carry **no** `/Outlines`
  entry — an outline-based structure source covers nothing for them, whatever the
  quality of the extraction.
- Where an outline exists it is **large and shallow**: 23–398 nodes, depth 2–4,
  median 48 nodes among the ≥3-node documents.
- Destination resolution is **essentially total** where an outline exists
  (100 % for 13 of 14; NYU_graduate.pdf at 50 % with 2 nodes). The unresolved
  remainder, if any, is reported in `PageIndexTree::unresolved_titles` rather
  than silently inherited.
- `stereo_headset.pdf` is encrypted (empty-password AES). lopdf reads its
  catalog, which has **no `/Outlines`** — so it is genuinely outline-less, not a
  parse failure. Its page count reads as 0 because the page tree is inside the
  encrypted object stream; that affects only the reported page count, not the
  outline verdict.
- `Campaign_038_…pdf` has an outline of exactly one node and `NYU_graduate.pdf`
  two — below any useful threshold, hence the "≥3 nodes" statistic.

## Top-level titles vs PageIndex (5 largest outlines; listing only, no scoring)

### mmdetection-readthedocs-io-en-v2.18.0.pdf — 398 outline nodes, depth 4
| PDF outline (top level) | PageIndex (first 10) |
| --- | --- |
| Prerequisites | Preface |
| Installation | Prerequisites |
| Verification | Installation |
| Benchmark and Model Zoo | Verification |
| 1: Inference and train with existing models… | Benchmark and Model Zoo |
| 2: Train with customized datasets | 1: Inference and train with existing models and standard da… |
| 3: Train with customized models and standar… | 2: Train with customized datasets |
| Tutorial 1: Learn about Configs | 3: Train with customized models and standard datasets |
| Tutorial 2: Customize Datasets | Tutorial 1: Learn about Configs |
| Tutorial 3: Customize Data Pipelines | Tutorial 2: Customize Datasets |
| Tutorial 4: Customize Models | |
| Tutorial 5: Customize Runtime Settings | |
| … (43 top-level outline titles total) | |

### User_Manual_1500S_Classic_EN.pdf — 163 outline nodes, depth 4
| PDF outline (top level) | PageIndex (first 10) |
| --- | --- |
| Table of Contents | Preface |
| 1 Safety | WMF 1500 S Coffee Machine Introduction and Safety Informati… |
| 2 Introduction | Conditions for usage and installation |
| 3 Operation | Table of Contents |
| 4 Software | 1 Safety |
| 5 Other settings | 2 Introduction |
| 6 Care | 3 Operation |
| 7 HACCP cleaning schedule | 4 Software |
| 8 Maintenance and descaling | 5 Other settings |
| 9 Messages and instructions | 6 Care |
| 10 Safety and warranty | |
| Appendix: Technical data | |
| Appendix: Accessories and spare parts | |
| Index | |

### watch_d.pdf — 86 outline nodes, depth 3
| PDF outline (top level) | PageIndex (first 6) |
| --- | --- |
| Contents | Preface |
| Getting Started | Contents |
| Blood Pressure Management | Getting Started |
| Care for Health | Blood Pressure Management |
| Assistant | Care for Health |
| | Assistant |

### ADOBE_2015_10K.pdf — 76 outline nodes, depth 4
| PDF outline (top level) | PageIndex (first 9) |
| --- | --- |
| Cover Page | Cover Page |
| Part I | Part I |
| Part II. | Part II. |
| Part III. | Part III. |
| Part IV. | Part IV. |
| Signatures | Signatures |
| Summary of Trademarks | Summary of Trademarks |
| Index to Exhibits | Index to Exhibits |
| Stock Performance Graph | Stock Performance Graph |

### 2312.10997v5.pdf — 49 outline nodes, depth 3
| PDF outline (top level) | PageIndex (first 9) |
| --- | --- |
| Introduction | Introduction |
| Overview of RAG | Overview of RAG |
| Retrieval | Retrieval |
| Generation | Generation |
| Augmentation process in RAG | Augmentation process in RAG |
| Task and Evaluation | Task and Evaluation |
| Discussion and Future Prospects | Discussion and Future Prospects |
| Conclusion | Conclusion |
| References | References |

Observation, offered as an observation only: on every one of the five, the
outline's top-level titles are the *same strings* PageIndex's LLM-derived tree
carries (PageIndex simply prepends a "Preface" node for the front matter). That
is consistent with PageIndex deriving its tree from the document's own headings,
and it says nothing yet about the ~65 % of documents with no outline, nor about
depth/structure agreement — a title-agreement score would be the next
measurement, not this one.

## Reproduce

```
cargo run --example outline_coverage -- \
  /home/ctown/orca/workspaces/agent-evals/w2-agent-evals-pageindex-bench/data/PageIndex-OSS-Benchmark/documents \
  /home/ctown/orca/workspaces/agent-evals/w2-agent-evals-pageindex-bench/.pageindex/manifest.json
```
