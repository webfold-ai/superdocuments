//! Golden tests for the Pandoc-compatibility work. They need neither Pandoc nor the network.
//!
//! * Readers: every document in `benches/pandoc-compat/corpus` (377 inputs: LaTeX, DOCX, ODT, EPUB, HTML,
//!   reStructuredText, Markdown, BibTeX) is converted and compared with the Markdown stored in
//!   `benches/pandoc-compat/golden/<id>.md`. The stored files are superdocuments' own output as it
//!   was when the fidelity numbers in `benches/pandoc-compat/results` were measured; they are not
//!   Pandoc's output (Pandoc's is stored as ASTs in `oracle/` and is only used by the scorer).
//! * Citations: the processor is run on the 125 documents in `benches/pandoc-compat/cite/docs` with three
//!   small styles written for this repository (`cite/golden-styles/`, Apache-2.0) and compared with
//!   the strings `pandoc --citeproc` produced for the same inputs, stored in `cite/golden/<style>.json`
//!   (regenerate with `python3 cite_bench.py golden`, which needs Pandoc 3.1.3). The third-party CSL
//!   styles and the CSL test suite are not in the repository (see `cite/fetch_corpora.sh`); the
//!   measurements on them are in `benches/pandoc-compat/README.md`.
//!
//! `UPDATE_GOLDEN=1 cargo test --test pandoc_compat_golden` rewrites the stored files.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use superdocuments::citeproc::{self, markdown, normalize_comparison, Processor, Style};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/pandoc-compat")
}

fn update() -> bool {
    std::env::var("UPDATE_GOLDEN").is_ok()
}

#[test]
fn readers_match_the_stored_markdown() {
    let corpus = root().join("corpus");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(corpus.join("manifest.json")).unwrap()).unwrap();
    let items = manifest["items"].as_array().unwrap();
    assert!(items.len() >= 370, "the corpus has {} items", items.len());
    let mut bad = Vec::new();
    for item in items {
        let id = item["id"].as_str().unwrap();
        let path = corpus.join(item["path"].as_str().unwrap());
        let golden = root().join("golden").join(format!("{id}.md"));
        let got = superdocuments::document::convert_path(&path).unwrap_or_default();
        if update() {
            fs::write(&golden, &got).unwrap();
            continue;
        }
        let want =
            fs::read_to_string(&golden).unwrap_or_else(|_| panic!("missing golden file for {id}"));
        if got != want {
            bad.push(id.to_string());
        }
    }
    assert!(
        bad.is_empty(),
        "{} documents differ from their golden Markdown: {:?}",
        bad.len(),
        &bad[..bad.len().min(20)]
    );
}

fn normalized(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| normalize_comparison(s.as_str().unwrap()))
        .collect()
}

#[test]
fn citations_match_the_stored_pandoc_citeproc_output() {
    let cite = root().join("cite");
    let mut checked = 0;
    let mut bad = Vec::new();
    for style in ["lite-author-date", "lite-numeric", "lite-note"] {
        let golden: Value = serde_json::from_str(
            &fs::read_to_string(cite.join("golden").join(format!("{style}.json"))).unwrap(),
        )
        .unwrap();
        let csl = Style::parse(
            &fs::read_to_string(cite.join("golden-styles").join(format!("{style}.csl"))).unwrap(),
        )
        .unwrap();
        for (doc, reference) in golden.as_object().unwrap() {
            let bib = doc.rsplit_once("-d").unwrap().0;
            let items =
                fs::read_to_string(cite.join("csljson").join(format!("{bib}.json"))).unwrap();
            let p = Processor::new(csl.clone(), citeproc::references_from_json(&items).unwrap());
            let md = fs::read_to_string(cite.join("docs").join(format!("{doc}.md"))).unwrap();
            let (c, b) = markdown::comparison_for_document(&md, &p);
            let c: Vec<String> = c.iter().map(|s| normalize_comparison(s)).collect();
            let b: Vec<String> = b.iter().map(|s| normalize_comparison(s)).collect();
            if c != normalized(&reference["citations"]) {
                bad.push(format!("{style} {doc} citations"));
            }
            if b != normalized(&reference["bibliography"]) {
                bad.push(format!("{style} {doc} bibliography"));
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 375,
        "only {checked} document/style pairs checked"
    );
    assert!(
        bad.is_empty(),
        "{} differences from Pandoc's stored output: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}
