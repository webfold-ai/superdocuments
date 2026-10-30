# MarathonDocBench

[MarathonDocBench](https://huggingface.co/datasets/baidu/MarathonDocBench) (Baidu ERNIE team, Apache-2.0 annotations) is a benchmark for **hundred-page document parsing**: 100 documents
of 100 to 300 pages (about 18,000 page images, mixed Chinese and English) with layout annotations for text, titles, tables, formulas and figures, reading order, and cross-page
continuations. It is scored with the MPDocBench-Parse toolkit on eight metrics (TextEdit, T-TextEdit, TableTEDS, T-TableTEDS, FormulaCDM, ReadOrderEdit, FigureF1, HeadingTEDS).
The published leaderboard has end-to-end vision-language models at 5 to 77 on the mean of the eight metrics.

This lane puts the superdocuments / webfold OCR path on it. The dataset ships **page images, not PDFs**, so the input is an image-only PDF built from the pages, which is
what the PDF parser's OCR fallback (`pdftoppm` + `tesseract`) exists for.

## What is here

| File | Role |
|---|---|
| `fetch.py` | download the ground truth and the page images of chosen documents (`--shortest N`, `--ids`, `--all`); nothing is committed, the page images are for academic and research use only |
| `run.py` | write one Markdown file per document per system: `webfold` (chunked, all languages), `webfold_default` (`webfold pdf` as shipped), `tesseract` (the incumbent it wraps) |
| `score_provisional.py` | a text-only character similarity. **Not an MPDocBench metric** |
| `make_subset_gt.py`, `score_official.md` | how to run the official toolkit on a subset or on all 100 documents |
| `results/` | the timings and provisional scores below |

## Result, 2026-10-30: two documents, text only, provisional score

The two shortest documents (`mp3lan_0105`, a Chinese book with many equations, 103 pages; `mp3lan_0299`, an English nursing thesis with 54 tables, 103 pages). One shared 4-core VM, one run,
two documents at a time, Tesseract 5.3.4 with `chi_sim+chi_tra+eng`. Similarity is 1 minus the normalised character-level Levenshtein distance between the ground-truth text (text, titles, captions,
table cell text, in reading order) and the prediction, whitespace and markup removed.

| system | document | seconds | predicted chars (ground truth) | similarity |
|---|---|---:|---:|---:|
| tesseract | mp3lan_0105 | 298 | 67,104 (65,704) | 0.750 |
| webfold, chunked | mp3lan_0105 | 362 | 67,041 | 0.713 |
| webfold, as shipped | mp3lan_0105 | 49 | 13,374 | 0.058 |
| tesseract | mp3lan_0299 | 92 | 71,859 (71,861) | 0.779 |
| webfold, chunked | mp3lan_0299 | 119 | 70,809 | 0.784 |
| webfold, as shipped | mp3lan_0299 | 25 | 16,100 | 0.196 |

**What it says.** Given enough settings, webfold's OCR fallback reads the text about as well as bare Tesseract on these two documents (0.713 vs 0.750 and 0.784 vs 0.779; the gap is a few points
either way and two documents cannot say which is better). As shipped it reads a fifth of it, because of limits below. Neither system produces layout, so the official table, heading, formula and
figure metrics would be near zero for both; **this is not a leaderboard number** and must not be compared with the table above. A hypothesis for webfold's lower score on the Chinese
book, not tested: its pages are 1812 px tall and webfold rendered them at a longest edge of 1800 px, a slight downscale Tesseract did not get.

## Findings about the code under test

* `webfold pdf` on an image-only PDF returned only `![](img_N.png)` placeholders and never reached OCR, because the fallback ran only when the extracted Markdown was empty. Fixed in webfold:
  the fallback now runs when nothing but image placeholders came out.
* The OCR defaults (`superdocuments::pdf::OcrOptions`) are English only, 20 pages, a longest page edge of 2400 px and 120 s. Hard limits in superdocuments: at most 100 pages, at most 600 s,
  and pages x edge^2 x 4 bytes within 512 MiB (about 22 pages at 2400 px, 40 pages at 1800 px). A 100 to 300 page document therefore **cannot be OCR'd in one call**; this lane splits it into 40-page
  chunks and joins them. Webfold gained `--ocr-lang`, `--ocr-max-pages`, `--ocr-max-dim` and `--ocr-timeout` (or `WEBFOLD_OCR_*`) to make that possible; before, none of it was reachable from the CLI.
* `webfold pdf` as shipped drops everything after page 20 with no error, so a hundred-page scan comes back as a fifth of a document. It does add a truncation note to the output for some limits; whether it does here was not checked.
* Tesseract spins OpenMP threads; with more concurrent processes than cores a single page took minutes instead of half a second. The lane sets `OMP_THREAD_LIMIT=1` and two jobs.
* No layout. The fallback returns text with `<!-- page N -->` markers: no headings, tables, formulas or figure tags, which is most of what MarathonDocBench scores.

## Not done

* The official MPDocBench-Parse scorer was **not run**: it executes code cloned from GitHub plus a patch downloaded from the dataset repository, and the environment this was built in declined to run unreviewed external code.
  `score_official.md` has the exact steps. Until then there is no comparable number.
* Two of 100 documents. The full set is about 18,000 pages; at the measured 1.1 to 3.5 s per page that is 5 to 17 hours per system on one core.
* No vision-language model baseline, and no layout-aware path (the superdocuments `DocumentGraph` is the place to start).
* Registering the lane in `ptyktos-evals` (`registry.toml`) and an spbench entry point: neither repository was available in this session.
