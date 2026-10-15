> **Status: NOT RE-RUN on the final tree.** This table was measured on an earlier build (`run-d`), before the
> OCR-merge, ToUnicode and generation-check edits. The harness could not be rebuilt because free disk was at
> the 700 MB floor. Re-run `webfold-pdf-vs-poppler` against this tree and replace the table before relying on it.
> In particular, the "No OCR runs" wording below predates the OCR merge and must be re-checked.

# PDF text agreement with poppler (G18, DoD 1)

Measured by `webfold-pdf-vs-poppler` (the `bench/pdf-vs-poppler` harness): `superdocuments::pdf::extract_file` against `pdftotext` on each file, scored as word-set Jaccard similarity. This measures how alike the two outputs are, not accuracy, because the corpus has no ground-truth transcripts. The rule is: a file that poppler extracts non-empty text from must reach 0.95, or carry an explanation below, keyed by sha256.

Run: `draft from run-d`

| file | sha256 | poppler chars | superdocuments chars | agreement | status |
|---|---|---:|---:|---:|---|
| borderless-table.pdf | 40bc8d5c60d259b91f69bba9792e0d511f8fee5925df105218085395c3f88eeb | 227 | 225 | 1.000 | pass |
| mixed-native-and-scanned.pdf | 7591b90a7a404e6ef02268923e4f871b24f62cd133d5e7583e256a23469397c3 | 188 | 204 | 0.893 | explained |
| reportlab-ascii85.pdf | 1ff2fb5ec10a2e1b19ef8d9e1120eec02edb4800a9cc4b8e7c6ad81afad51852 | 196 | 276 | 0.909 | explained |
| reportlab-invoice.pdf | 881e3b32fd68cd26c51e7e1d0c2760274f7848ab631a777059d647c9be9c18eb | 144 | 145 | 1.000 | pass |
| rotated-15deg.pdf | c8ae94aab72a0b1049cfbad4444442f244829672b669c52a8992793f566199e3 | 122 | 147 | 0.043 | explained |
| scanned-fuzzy.pdf | 0c9024f1f3302df0456e1fcdf67289fc3b79c41f5e50608f99132d926341bf53 | 21 | 37 | 0.500 | explained |
| scanned-text.pdf | 2ef910e0979b9ec585e873162d02ccd80d5d0c63f3dcfc5dd78fc67106fdcb4a | 1 | 16 | n/a | not scored: poppler extracts 1 character, so there is no poppler text to agree with |
| smoke.pdf | 24b1d307f3d29b7d73404940e3fe4a85ecd27802b61768061256019ffd92a0ee | 154 | 155 | 1.000 | pass |
| two-column.pdf | f75a08c66c1f2efd5606650b76b17ff64d735ba51e550922f11f87017aba24fd | 1066 | 1070 | 1.000 | pass |
| memory_comparison.pdf | f7abe00144aa93ac0ad5292e3b21b361c6ceec1143cf295b84755990ee401e7b | 187 | 187 | 1.000 | pass |
| output_size.pdf | 9dc0fa19eeeef2702a7efc9dfc369b5f4f7386d92e2e7b60024bbf30f8f409bc | 304 | 335 | 1.000 | pass |
| pdffold_gap.pdf | 416fd421632846fde65b5000dac5069b07ba2c572bef7aada6516834876d4b99 | 278 | 284 | 1.000 | pass |
| pipeline_complexity.pdf | d45f7adfe2c08e9861492bba80b0f636db34059f4193fa5b246d4b35a97800e6 | 407 | 392 | 1.000 | pass |
| pipeline_diagram.pdf | 25b8101bcba7c39767b57ebdbdcfd64b6b01a8778a5c30d10b0450f265e7631c | 344 | 342 | 1.000 | pass |
| pipeline_speed.pdf | 3e958fab8b840498aaf0a5df2081f49e993245a1f804aad2f3c93d6c525908ae | 288 | 289 | 1.000 | pass |
| single_page_timing.pdf | ac8f156a7a18185ced95abb90f7d21139452e77ac94f2c4c3561d74ef91603a3 | 397 | 418 | 1.000 | pass |
| paper-html2pdf.pdf | fef737150c29e42a7d099c5dc135369d4d31bec489707db59e694ece91ef7f97 | 37366 | 38448 | 0.973 | pass |
| research-agent-loop-pretty.pdf | b6def9edf5b9715f3896a57843c43745812e261a643eb30d9beb6ef7a5bb9356 | 428 | 467 | 0.942 | explained |
| research-agent-loop.pdf | fa413fe76f9d8f304542db1fb85b002cf4492e0060eb5f08e5ac6bcc04fa04c9 | 1144 | 1130 | 1.000 | pass |
| webfold-benchmarks-pretty.pdf | 4aa6043f937146e0fba4c1f3899f2531fcb3ddb107dcfb887e3386e4f03e74d0 | 572 | 592 | 0.963 | pass |
| webfold-benchmarks.pdf | 13154b665ea20530a28d4e8b5b42a76fd1de4b707e866801f87c7b4472c8a090 | 1267 | 1263 | 1.000 | pass |
| paper-syndesi.pdf | 2b531dc2451568a55fac0192fa3eeeb36030a23c1e2318aa45a2ce67b5a48062 | 48484 | 48240 | 0.990 | pass |
| doc.pdf | a5c746bae4bf00e51a14eb078e64670bd70feeab5d068b33b16d6ba92b9dafbf | 387 | 389 | 1.000 | pass |

## Explanations below 0.95

- **mixed-native-and-scanned.pdf** (`7591b90a7a404e6ef02268923e4f871b24f62cd133d5e7583e256a23469397c3`): Page 2 is a scanned image with no text layer. `extract_file` does not OCR, so the page is the image marker `![](img_5.png)`, and the marker's tokens (`img`, `png`, `5`) are counted as words. Page 1's native text matches poppler.
- **reportlab-ascii85.pdf** (`1ff2fb5ec10a2e1b19ef8d9e1120eec02edb4800a9cc4b8e7c6ad81afad51852`): The invoice table is emitted as HTML markup (`<table>`, `<tr>`, `<td>`), and the tag names `table`, `tr` and `td` are counted as words. The cell text matches poppler.
- **rotated-15deg.pdf** (`c8ae94aab72a0b1049cfbad4444442f244829672b669c52a8992793f566199e3`): The page is set at 15 degrees. Poppler emits the rotated glyphs one per line and in scrambled order. The superdocuments output reads the sentences, and matches the committed `rotated-15deg.expected.txt` word for word.
- **scanned-fuzzy.pdf** (`0c9024f1f3302df0456e1fcdf67289fc3b79c41f5e50608f99132d926341bf53`): A scanned image-only page. Poppler finds only the heading. The superdocuments output has the same heading plus the image marker `![](img_4.png)`, so the marker's tokens are counted. No OCR runs in `extract_file`.
- **research-agent-loop-pretty.pdf** (`b6def9edf5b9715f3896a57843c43745812e261a643eb30d9beb6ef7a5bb9356`): Figure image markers (`![](img_13.png)`) are counted as the tokens `img`, `png` and `13`. The text otherwise agrees with poppler.

Files that agree at 0.95 or more are not listed. The `paper.pdf` corpus files are committed as `tests/fixtures/regression/paper-html2pdf.pdf` and `paper-syndesi.pdf`; both pass at 0.95 or above.
