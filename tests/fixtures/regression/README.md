# PDF regression set (G18)

Files the xref, object-stream and predictor defects were found on. Each is committed
unmodified; the sha256 is the one recorded in `pdf-vs-poppler-2026-10-12.json`.

| file | sha256 | role |
|---|---|---|
| `doc.pdf` | a5c746bae4bf00e51a14eb078e64670bd70feeab5d068b33b16d6ba92b9dafbf | DoD 2: `/XRef /W [1 2 2]`, `/ObjStm /N 17 /First 114` |
| `paper-html2pdf.pdf` | fef737150c29e42a7d099c5dc135369d4d31bec489707db59e694ece91ef7f97 | DoD 2 (paper.pdf, sha fef737); DoD 1 word spacing |
| `paper-syndesi.pdf` | 2b531dc2451568a55fac0192fa3eeeb36030a23c1e2318aa45a2ce67b5a48062 | DoD 2 (paper.pdf, sha 2b531d); DoD 1 word spacing |
| `research-agent-loop.pdf` | fa413fe76f9d8f304542db1fb85b002cf4492e0060eb5f08e5ac6bcc04fa04c9 | DoD 2 |
| `webfold-benchmarks.pdf` | 13154b665ea20530a28d4e8b5b42a76fd1de4b707e866801f87c7b4472c8a090 | DoD 2 |
| `smoke.pdf` | 24b1d307f3d29b7d73404940e3fe4a85ecd27802b61768061256019ffd92a0ee | DoD 3: `smoke.expected.txt` is `pdftotext smoke.pdf` (154 bytes) |

`smoke.expected.txt` is the poppler line set the smoke test compares against.
