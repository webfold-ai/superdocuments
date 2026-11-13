#!/bin/sh
# Fetches the third-party inputs of the citation benchmark. They are NOT committed: the CSL styles are
# Creative Commons Attribution-ShareAlike 3.0 (each file says so in its <rights> element) and the CSL
# test-suite repository has no licence file of its own and embeds such styles in its fixtures.
# Everything is pinned to a commit and checked against corpora.sha256.
#
#   ./fetch_corpora.sh            styles/, styles-heldout/ (10 styles) and suite-src/ (845 test fixtures)
#   python3 cite_ts.py gen suite-src       turns the fixtures into suite/ (documents, styles, items)
#   python3 cite_bench.py oracle           Pandoc's reference output for the 125 x 6 development set
#   CITE_SET=heldout python3 cite_bench.py oracle      ... and the 125 x 4 held-out set
#   python3 cite_ts.py oracle              ... and the 845-test stress set
# Needs curl, git, sha256sum and (for the oracle commands) Pandoc 3.1.3.
set -eu
cd "$(dirname "$0")"
STYLES_COMMIT=af0aea3fffd44fe798cce9285598d0d5cf62ef23
SUITE_COMMIT=6eefc5b07c6969ab8999e48542acbcc131cba864
mkdir -p styles styles-heldout
for st in apa chicago-author-date ieee harvard-cite-them-right nature modern-language-association; do
  curl -fsS -o "styles/$st.csl" "https://raw.githubusercontent.com/citation-style-language/styles/$STYLES_COMMIT/$st.csl"
done
for st in elsevier-vancouver american-medical-association springer-basic-author-date american-sociological-association; do
  curl -fsS -o "styles-heldout/$st.csl" "https://raw.githubusercontent.com/citation-style-language/styles/$STYLES_COMMIT/$st.csl"
done
if [ ! -d suite-src/.git ]; then
  rm -rf suite-src
  GIT_LFS_SKIP_SMUDGE=1 git clone -q https://github.com/citation-style-language/test-suite suite-src
fi
git -C suite-src fetch -q origin "$SUITE_COMMIT"
git -C suite-src checkout -q "$SUITE_COMMIT"
sha256sum -c --quiet corpora.sha256
echo "fetched and verified: styles at $STYLES_COMMIT, test suite at $SUITE_COMMIT"
