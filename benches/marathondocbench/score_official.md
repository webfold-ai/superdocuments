# Scoring with the official MPDocBench-Parse toolkit

The leaderboard numbers (TextEdit, T-TextEdit, TableTEDS, T-TableTEDS, FormulaCDM, ReadOrderEdit, FigureF1, HeadingTEDS, and their mean) come from the
MPDocBench-Parse toolkit, which MarathonDocBench reuses. This lane produces the predictions it reads; **it has not been run through the toolkit**: the
toolkit executes code cloned from GitHub (Tongyi-Zhiwen/Qwen-Doc) and a patch downloaded from the dataset repository, and the environment this lane was built in
refused to run unreviewed external code. Run it where you have reviewed that code.

```bash
git clone https://github.com/Tongyi-Zhiwen/Qwen-Doc.git && cd Qwen-Doc/MPDocBench
python3 -m venv .venv && . .venv/bin/activate
pip install numpy pyyaml tqdm pillow python-Levenshtein rapidfuzz beautifulsoup4 lxml apted scipy pandas pylatexenc mmeval html5lib regex   # no requirements.txt ships in that directory
# the dataset's accelerated drop-in for three evaluator files (metric-identical, adds parallelism and a match timeout):
for f in dataset/end2end_dataset.py metrics/cal_metric.py utils/data_preprocess.py; do
  curl -sL https://huggingface.co/datasets/baidu/MarathonDocBench/resolve/main/utils/mpdocbench_patch/$f -o $f; done

/path/to/superdocuments/benches/ocr/target/release/marathondocbench subset --data DATA     # for a subset run
# in configs/end2end.yaml: ground_truth.data_path -> DATA/MarathonDocBench.subset.json (or MarathonDocBench.json for all 100),
#                          prediction.data_path    -> DATA/pred/<system>
MPDOC_EVAL_NPROC=4 MPDOC_MATCH_TIMEOUT=1800 MPDOC_DOC_BUDGET=0 MPDOC_TEDS_NPROC=4 MPDOC_TEDS_TIMEOUT=300 \
    python pdf_validation.py --config ./configs/end2end.yaml
```

FormulaCDM needs Node.js, ImageMagick and TeX Live (`metrics/cdm/README.md`); without them leave that metric out and say so. Figure tags
(`![](pageN_x1_y1_x2_y2.jpg)`), real Markdown headings and Markdown/HTML tables are part of the score; neither system here emits them, so
HeadingTEDS, FigureF1 and the table metrics are expected to be at or near zero for text-only OCR, and that is a finding about the systems, not the scorer.
