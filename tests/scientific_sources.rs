//! LaTeX, BibTeX, MathML and arXiv e-prints through the public conversion API
//! (`document::convert_bytes`, `document::convert_path`) and the bounded file provider
//! (`latex::DirFiles`). Each test states what the input contains.

use std::fs;
use std::path::{Path, PathBuf};

use superdocuments::document::{convert_bytes, convert_path, fingerprint_bytes, DocumentCache};
use superdocuments::latex::{self, DirFiles, FileProvider};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

fn fixture_bytes(rel: &str) -> Vec<u8> {
    fs::read(fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A directory under the build's temporary directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("scientific-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        Self(dir)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{}: {e}", parent.display()));
        }
        fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

// ----- convert_bytes ---------------------------------------------------------------------------

#[test]
fn tex_bytes_convert_to_markdown_under_every_latex_extension() {
    let source = br"\documentclass{article}
\title{Routing}
\begin{document}
\maketitle
\section{One}\label{sec:one}
Inline $a^2$ and a reference to Section~\ref{sec:one}.
\begin{equation}\label{eq:e} E = mc^2 \end{equation}
\end{document}";
    for ext in ["tex", "TEX", "latex", "ltx"] {
        let md = convert_bytes(ext, source).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert!(md.starts_with("# Routing\n"), "{ext}: {md}");
        assert!(md.contains("## One"), "{ext}: {md}");
        assert!(md.contains("Inline $a^2$"), "{ext}: {md}");
        assert!(
            md.replace('\u{a0}', " ").contains("Section 1."),
            "{ext}: {md}"
        );
        assert!(md.contains("E = mc^2 \\tag{1}"), "{ext}: {md}");
    }
}

#[test]
fn latin1_tex_bytes_are_decoded_not_rejected() {
    // 0xE9 is e-acute in Latin-1 and is not valid UTF-8 on its own.
    let md = convert_bytes("tex", b"Caf\xe9 notes.").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(md.trim(), "Caf\u{e9} notes.");
}

#[test]
fn tex_bytes_have_no_file_access_so_inputs_stay_unresolved() {
    // The same main file that `convert_path` inlines below: with bytes only, nothing is read.
    let md = convert_bytes("tex", &fixture_bytes("latex/project/main.tex"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(md.contains("# A Project in Several Files"), "{md}");
    assert!(!md.contains("Introduction"), "{md}");
    assert!(!md.contains("References"), "{md}");
}

#[test]
fn bib_bytes_give_a_readable_markdown_reference_list() {
    let md = convert_bytes("bib", &fixture_bytes("latex/project/refs.bib"))
        .unwrap_or_else(|e| panic!("{e}"));
    let lines: Vec<&str> = md.lines().collect();
    assert_eq!(lines.len(), 3, "{md}");
    // @string expansion, three authors, booktitle in italics, volume, year.
    assert_eq!(
        lines[0],
        "- `vaswani2017` \u{2014} Ashish Vaswani, Noam Shazeer and Niki Parmar (2017). \
         Attention Is All You Need. *Advances in Neural Information Processing Systems* 30."
    );
    // A parenthesis-delimited entry with a quoted-string title and a bare number year.
    assert_eq!(
        lines[1],
        "- `knuth1984` \u{2014} Donald E. Knuth (1984). The TeXbook. Addison-Wesley."
    );
    assert!(lines[2].contains("*Journal of Unused Results*"), "{md}");
    assert_eq!(
        convert_bytes("bibtex", &fixture_bytes("latex/project/refs.bib")).ok(),
        Some(md)
    );
}

#[test]
fn bib_markdown_links_a_doi_and_follows_crossref() {
    let source = br#"@inproceedings{child, author = {Hopper, Grace}, title = {A Compiler},
  pages = {10--12}, doi = {https://doi.org/10.1000/ABC}, crossref = {proc}}
@proceedings{proc, booktitle = {Proceedings of the Meeting}, year = 1952,
  editor = {Turing, Alan and others}}"#;
    let md = convert_bytes("bib", source).unwrap_or_else(|e| panic!("{e}"));
    let lines: Vec<&str> = md.lines().collect();
    assert_eq!(
        lines[0],
        "- `child` \u{2014} Grace Hopper (1952). A Compiler. *Proceedings of the Meeting*, \
         10\u{2013}12. <https://doi.org/10.1000/ABC>"
    );
    assert!(
        lines[1].contains("Alan Turing et al. (eds.) (1952)."),
        "{md}"
    );
}

#[test]
fn bib_bytes_without_entries_are_an_error_not_an_empty_document() {
    let err = convert_bytes("bib", b"this is not a bibliography").unwrap_err();
    assert!(err.contains("no BibTeX entries"), "{err}");
}

#[test]
fn csl_json_stays_reachable_next_to_the_markdown_route() {
    let text = String::from_utf8(fixture_bytes("latex/project/refs.bib")).unwrap_or_default();
    let bib = superdocuments::bibtex::parse(&text);
    let csl = superdocuments::bibtex::csl_json(&bib);
    assert_eq!(csl[0]["id"], "vaswani2017");
    assert_eq!(csl[0]["author"][1]["family"], "Shazeer");
    assert_eq!(csl[1]["issued"]["date-parts"][0][0], 1984);
}

#[test]
fn mathml_bytes_give_display_math() {
    let presentation = br#"<math xmlns="http://www.w3.org/1998/Math/MathML"><mfrac><mi>a</mi><mi>b</mi></mfrac></math>"#;
    for ext in ["mml", "mathml"] {
        let md = convert_bytes(ext, presentation).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(md, "$$\n\\frac{a}{b}\n$$", "{ext}");
    }
    let annotated = br#"<math><semantics><mi>x</mi><annotation encoding="application/x-tex">x^{2}+1</annotation></semantics></math>"#;
    assert_eq!(
        convert_bytes("mml", annotated).ok().as_deref(),
        Some("$$\nx^{2}+1\n$$")
    );
    assert!(convert_bytes("mml", b"<math></math>").is_err());
    assert!(convert_bytes("mml", b"not xml at all <").is_err());
}

#[test]
fn eprint_tarball_is_read_by_extension_and_by_content() {
    let data = fixture_bytes("arxiv/paper.tar.gz");
    let by_extension = convert_bytes("gz", &data).unwrap_or_else(|e| panic!("{e}"));
    // \input{sections/intro} comes from the archive, and so does refs.bib.
    assert!(by_extension.contains("Introduction"), "{by_extension}");
    assert!(by_extension.contains("Method"), "{by_extension}");
    assert!(by_extension.contains("Vaswani"), "{by_extension}");
    assert!(by_extension.contains("References"), "{by_extension}");
    for ext in ["tgz", "tex", "00001v2", "eprint"] {
        assert_eq!(
            convert_bytes(ext, &data).as_ref(),
            Ok(&by_extension),
            "extension {ext}"
        );
    }
}

#[test]
fn gzipped_single_tex_file_is_an_eprint() {
    let data = fixture_bytes("arxiv/single-file.tex.gz");
    let md = convert_bytes("gz", &data).unwrap_or_else(|e| panic!("{e}"));
    let direct = superdocuments::arxiv_source::read_eprint(&data).unwrap_or_else(|e| panic!("{e}"));
    let title = direct
        .document
        .title
        .as_deref()
        .map(latex::plain_text)
        .unwrap_or_default();
    assert!(!title.is_empty(), "the fixture has a title");
    assert!(md.contains(&title), "{md}");
    assert_eq!(convert_bytes("tex", &data).ok(), Some(md));
}

#[test]
fn bbl_only_eprint_cites_by_label_and_lists_the_items() {
    let md = convert_bytes("gz", &fixture_bytes("arxiv/bbl-only.tar.gz"))
        .unwrap_or_else(|e| panic!("{e}"));
    // The natbib label `Vaswani et~al.(2017)` is shown as an author-year citation, not as the
    // key with "n.d.".
    assert!(md.contains("(Vaswani et al., 2017)"), "{md}");
    assert!(!md.contains("n.d."), "{md}");
    assert!(md.contains("References"), "{md}");
    assert!(md.contains("Attention is all you need"), "{md}");
}

#[test]
fn unknown_extensions_and_broken_archives_are_errors() {
    assert_eq!(
        convert_bytes("xyz", b"plain bytes").unwrap_err(),
        "unsupported document extension: xyz"
    );
    // gzip magic followed by rubbish: named as an archive it reports the archive error, and
    // under an unknown extension it stays an unsupported extension.
    let broken = [0x1f, 0x8b, 8, 0, 1, 2, 3];
    assert!(convert_bytes("gz", &broken).is_err());
    assert_eq!(
        convert_bytes("xyz", &broken).unwrap_err(),
        "unsupported document extension: xyz"
    );
    assert!(convert_bytes("tar", b"not a tar").is_err());
}

#[test]
fn document_cache_accepts_the_scientific_formats() {
    let tex = br"\section{Cached} Body.";
    let mut cache = DocumentCache::new();
    let mut cold = Vec::new();
    cache
        .convert(fingerprint_bytes(tex), "tex", tex, &mut cold)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut warm = Vec::new();
    cache
        .convert(fingerprint_bytes(tex), "tex", tex, &mut warm)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(cold, warm);
    assert_eq!(
        String::from_utf8_lossy(&cold),
        convert_bytes("tex", tex).unwrap_or_default()
    );
    assert_eq!((cache.misses(), cache.hits()), (1, 1));

    let eprint = fixture_bytes("arxiv/paper.tar.gz");
    let mut out = Vec::new();
    cache
        .convert(fingerprint_bytes(&eprint), "2301.00001", &eprint, &mut out)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        String::from_utf8_lossy(&out),
        convert_bytes("gz", &eprint).unwrap_or_default()
    );
}

// ----- convert_path ----------------------------------------------------------------------------

#[test]
fn convert_path_reads_the_project_next_to_the_tex_file() {
    let md = convert_path(&fixture("latex/project/main.tex")).unwrap_or_else(|e| panic!("{e}"));
    assert!(md.starts_with("# A Project in Several Files\n"), "{md}");
    // \input{sections/intro}, \input{sections/method.tex}, and a file nested two levels down.
    assert!(md.contains("## Introduction"), "{md}");
    assert!(md.contains("## Method"), "{md}");
    assert!(md.contains("A detail from a nested file"), "{md}");
    // A reference from one input file to a label in another.
    assert!(
        md.replace('\u{a0}', " ").contains("Section 1 motivates"),
        "{md}"
    );
    // \bibliography{refs}: citations use refs.bib, and only the cited entries are listed.
    assert!(md.contains("(Vaswani et al., 2017)"), "{md}");
    assert!(md.contains("## References"), "{md}");
    assert!(
        md.contains("- `vaswani2017` \u{2014} Ashish Vaswani"),
        "{md}"
    );
    assert!(
        md.contains("- `knuth1984` \u{2014} Donald E. Knuth (1984)"),
        "{md}"
    );
    assert!(!md.contains("Never Cited"), "{md}");
    let order = (
        md.find("`vaswani2017`").unwrap_or(usize::MAX),
        md.find("`knuth1984`").unwrap_or(0),
    );
    assert!(order.0 < order.1, "references follow citation order: {md}");
}

#[test]
fn convert_path_falls_back_to_the_bbl_next_to_the_document() {
    let md =
        convert_path(&fixture("latex/project-bbl/paper.tex")).unwrap_or_else(|e| panic!("{e}"));
    assert!(md.contains("(Vaswani et al., 2017)"), "{md}");
    assert!(md.contains("# References"), "{md}");
    assert!(md.contains("Attention is all you need."), "{md}");
    assert!(md.contains("*The TeXbook*"), "{md}");
    // The second item has no label: it is cited by its number.
    assert!(md.contains("(2)"), "{md}");
    assert!(md.contains("[2] D."), "{md}");
}

#[test]
fn convert_path_reads_an_eprint_saved_without_a_document_extension() {
    let scratch = Scratch::new("eprint-names");
    let data = fixture_bytes("arxiv/paper.tar.gz");
    let expected = convert_bytes("gz", &data).unwrap_or_else(|e| panic!("{e}"));
    for name in ["2301.00001v2", "eprint", "paper.tar.gz", "source.tex"] {
        let path = scratch.0.join(name);
        fs::write(&path, &data).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(convert_path(&path).as_ref(), Ok(&expected), "{name}");
    }
    let plain = scratch.write("README", "just text");
    assert_eq!(
        convert_path(&plain).unwrap_err(),
        "missing document extension"
    );
}

/// `outside.tex` sits next to the root directory, not in it. Nothing may read it.
fn project_with_a_file_outside(name: &str) -> (Scratch, PathBuf) {
    let scratch = Scratch::new(name);
    scratch.write("outside.tex", "SECRET-OUTSIDE-SENTINEL");
    scratch.write("root/inside.tex", "inside text");
    let root = scratch.0.join("root");
    (scratch, root)
}

#[test]
fn input_cannot_escape_the_document_directory() {
    let (scratch, root) = project_with_a_file_outside("escape");
    let outside = scratch.0.join("outside.tex");
    let absolute = outside.to_string_lossy().replace('\\', "/");
    let main = scratch.write(
        "root/main.tex",
        &format!(
            "Start. \\input{{../outside}} \\input{{../outside.tex}} \
             \\input{{sub/../../outside}} \\include{{{absolute}}} \
             \\input{{inside}} End."
        ),
    );
    let md = convert_path(&main).unwrap_or_else(|e| panic!("{e}"));
    assert!(!md.contains("SECRET-OUTSIDE-SENTINEL"), "{md}");
    assert!(md.contains("inside text"), "{md}");
    assert!(md.contains("Start.") && md.contains("End."), "{md}");

    // The same through the provider, where the refusals and unresolved names are visible.
    let files = DirFiles::new(&root).unwrap_or_else(|e| panic!("{e}"));
    let source = fs::read_to_string(&main).unwrap_or_default();
    let doc = latex::parse_with(&source, &latex::Options::default(), Some(&files));
    assert_eq!(
        doc.inputs,
        [
            "../outside",
            "../outside.tex",
            "sub/../../outside",
            absolute.as_str()
        ]
    );
    let refused = files.refused();
    assert!(
        refused
            .iter()
            .any(|r| r.starts_with("../outside:") && r.contains("leaves the root")),
        "{refused:?}"
    );
    assert!(
        refused.iter().any(|r| r.contains("absolute paths")),
        "{refused:?}"
    );
    assert!(
        refused.iter().any(|r| r.starts_with("sub/../../outside")),
        "{refused:?}"
    );
    // A `..` that stays inside the root is fine.
    assert_eq!(
        files.read("sub/../inside.tex").as_deref(),
        Some("inside text")
    );
    assert_eq!(
        files.read_tex("./inside.tex").as_deref(),
        Some("inside text")
    );
    // A missing file is not a refusal.
    let before = files.refused().len();
    assert_eq!(files.read("no-such-file.tex"), None);
    assert_eq!(files.refused().len(), before);
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_document_directory_is_not_followed() {
    let (scratch, root) = project_with_a_file_outside("symlink");
    std::os::unix::fs::symlink(scratch.0.join("outside.tex"), root.join("link.tex"))
        .unwrap_or_else(|e| panic!("{e}"));
    std::os::unix::fs::symlink(&scratch.0, root.join("up")).unwrap_or_else(|e| panic!("{e}"));
    std::os::unix::fs::symlink(root.join("inside.tex"), root.join("alias.tex"))
        .unwrap_or_else(|e| panic!("{e}"));
    let main = scratch.write(
        "root/main.tex",
        "\\input{link} \\input{up/outside} \\input{alias} done",
    );
    let md = convert_path(&main).unwrap_or_else(|e| panic!("{e}"));
    assert!(!md.contains("SECRET-OUTSIDE-SENTINEL"), "{md}");
    // A link that stays inside the root is read.
    assert!(md.contains("inside text"), "{md}");

    let files = DirFiles::new(&root).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(files.read("link.tex"), None);
    assert_eq!(files.read("up/outside.tex"), None);
    let refused = files.refused();
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(
        refused
            .iter()
            .all(|r| r.contains("outside the root directory")),
        "{refused:?}"
    );
}

#[test]
fn a_bibliography_outside_the_document_directory_is_not_read() {
    let scratch = Scratch::new("bib-escape");
    scratch.write(
        "refs.bib",
        "@misc{k, author={Outside, Olga}, title={SECRET-BIB-SENTINEL}, year=2020}",
    );
    let main = scratch.write("root/main.tex", "\\cite{k} text \\bibliography{../refs}");
    let md = convert_path(&main).unwrap_or_else(|e| panic!("{e}"));
    assert!(!md.contains("SECRET-BIB-SENTINEL"), "{md}");
    assert!(!md.contains("Outside"), "{md}");
    assert!(md.contains("(k)"), "the key is shown unresolved: {md}");
}

#[test]
fn files_that_include_themselves_are_read_once() {
    let scratch = Scratch::new("cycle");
    // main -> a -> b -> a (cycle), main -> self (the main document is never an input),
    // main -> tail, whose last token includes itself.
    scratch.write("a.tex", "ALPHA-BODY \\input{b}");
    scratch.write("b.tex", "BETA-BODY \\input{a.tex}");
    scratch.write("tail.tex", "TAIL-BODY \\input{tail}");
    let main = scratch.write(
        "main.tex",
        "MAIN-BODY \\input{a} \\input{main} \\input{./tail} end",
    );
    let md = convert_path(&main).unwrap_or_else(|e| panic!("{e}"));
    for body in ["MAIN-BODY", "ALPHA-BODY", "BETA-BODY", "TAIL-BODY"] {
        assert_eq!(occurrences(&md, body), 1, "{body} in {md}");
    }
    assert!(md.trim_end().ends_with("end"), "{md}");

    let files = DirFiles::new(&scratch.0)
        .unwrap_or_else(|e| panic!("{e}"))
        .excluding(&main);
    let source = fs::read_to_string(&main).unwrap_or_default();
    let doc = latex::parse_with(&source, &latex::Options::default(), Some(&files));
    let cycles = doc
        .warnings
        .iter()
        .filter(|w| w.message.contains("includes itself"))
        .count();
    assert_eq!(cycles, 2, "{:?}", doc.warnings);
    assert_eq!(doc.inputs, ["main"]);
    assert!(
        files.refused().iter().any(|r| r.contains("main document")),
        "{:?}",
        files.refused()
    );
}

#[test]
fn nested_inputs_stop_at_the_depth_limit() {
    let scratch = Scratch::new("depth");
    // f0 inputs f1 inputs f2 ... to f39. Each file marks itself before going deeper.
    for i in 0..40 {
        scratch.write(
            &format!("f{i}.tex"),
            &format!("<L{i}> \\input{{f{}}}", i + 1),
        );
    }
    let main = scratch.write("main.tex", "\\input{f0}");
    let md = convert_path(&main).unwrap_or_else(|e| panic!("{e}"));
    assert!(md.contains("L0") && md.contains("L31"), "{md}");
    assert!(!md.contains("L32"), "32 files are open at most: {md}");
}

#[test]
fn dir_files_enforces_size_limits() {
    let scratch = Scratch::new("sizes");
    scratch.write("small.tex", "0123456789");
    scratch.write("big.tex", &"x".repeat(200));
    let files = DirFiles::with_limits(&scratch.0, 100, 25).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(files.read("big.tex"), None, "over the per-file limit");
    assert_eq!(files.read("small.tex").as_deref(), Some("0123456789"));
    assert_eq!(files.read("small.tex").as_deref(), Some("0123456789"));
    // 20 of the 25 bytes are used: a third read would pass the total.
    assert_eq!(files.read("small.tex"), None);
    let refused = files.refused();
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(refused[0].contains("over the size limit"), "{refused:?}");
    assert!(refused[1].contains("total size limit"), "{refused:?}");

    assert!(DirFiles::new(scratch.0.join("small.tex")).is_err());
    assert!(DirFiles::new(scratch.0.join("missing-dir")).is_err());
}

#[test]
fn a_subfile_that_is_a_whole_document_does_not_end_the_main_one() {
    let scratch = Scratch::new("subfile");
    scratch.write(
        "chapter.tex",
        "\\documentclass[main]{subfiles}\nPREAMBLE-JUNK\n\\begin{document}\n\
         \\section{Chapter}\nChapter text.\n\\end{document}\n",
    );
    let main = scratch.write(
        "main.tex",
        "\\documentclass{book}\n\\begin{document}\nBefore.\n\\subfile{chapter}\n\n\
         After the subfile.\n\\end{document}\n",
    );
    let md = convert_path(&main).unwrap_or_else(|e| panic!("{e}"));
    assert!(md.contains("# Chapter"), "{md}");
    assert!(md.contains("Chapter text."), "{md}");
    assert!(md.contains("After the subfile."), "{md}");
    assert!(!md.contains("PREAMBLE-JUNK"), "{md}");
}

// ----- a short paper, end to end -----------------------------------------------------------------

#[test]
fn convert_bytes_expands_the_documents_macros_inside_math() {
    let source = br"\newcommand{\R}{\mathbb{R}}\newcommand{\norm}[1]{\lVert #1 \rVert}
Let $x \in \R^n$ with $\norm{x} \le 1$.
\[ \norm{x}^2 = \sum_i x_i^2 \]";
    let md = convert_bytes("tex", source).unwrap_or_else(|e| panic!("{e}"));
    assert!(md.contains("$x \\in \\mathbb{R}^n$"), "{md}");
    assert!(md.contains("$\\lVert x \\rVert \\le 1$"), "{md}");
    assert!(
        md.contains("$$\n\\lVert x \\rVert^2 = \\sum_i x_i^2\n$$"),
        "{md}"
    );
    assert!(!md.contains("\\R") && !md.contains("\\norm"), "{md}");
    // The structured reader keeps math as written unless asked.
    let doc = latex::parse(std::str::from_utf8(source).unwrap_or_default());
    let raw = latex::render_markdown(&doc, &latex::RenderOptions::default());
    assert!(raw.contains("$x \\in \\R^n$"), "{raw}");
}

/// A short paper with an abstract, sections, a labelled equation and its reference, a booktabs
/// table with a spanned header, a starred figure with subfigures, a theorem, nested lists, a
/// footnote, and citations resolved against the `.bib` file next to it.
#[test]
fn a_short_paper_converts_to_the_expected_markdown() {
    let md = convert_path(&fixture("latex/paper-short.tex")).unwrap_or_else(|e| panic!("{e}"));
    let golden = fixture("latex/paper-short.converted.md");
    if std::env::var("LATEX_REGENERATE").is_ok() {
        fs::write(&golden, &md).unwrap_or_else(|e| panic!("{e}"));
    }
    let want = fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(md, want, "tests/fixtures/latex/paper-short.converted.md");
    // The points the golden file is there to hold, stated one by one.
    let flat = md.replace('\u{a0}', " ");
    for expected in [
        "# Sparse Folding for Protein Structure\n",
        "## Abstract\n\nWe introduce FoldNet, which improves accuracy by 12% on CASP14.",
        "Structure prediction (Jumper et al., 2021; Baek and DiMaio, 2021) is hard.",
        "Following Jumper et al., 2021, FoldNet minimises the loss in Eq. (1)",
        "over $x \\in \\mathbb{R}^n$ with $\\lVert x \\rVert \\le 1$[^1].",
        "Schr\u{f6}dinger, Erd\u{151}s and Poincar\u{e9} are cited \u{201c}often\u{201d} \u{2014} see [this paper](https://arxiv.org/abs/1706.03762).",
        "Section 2 has the details, Table 1 the numbers and Figure 1 the model.",
        "### Objective",
        "\\theta^\\star = \\operatorname*{arg\\,min}_{\\theta} \\sum_{i=1}^{n} \\lVert f_\\theta(x_i) - y_i \\rVert^2 . \\tag{1}",
        "**Theorem 1 (Convergence).** If $\\eta$ is small, (2) converges.",
        "|  | TM-score |  |\n| --- | --- | --- |\n| Method | CASP14 | CAMEO |",
        "| FoldNet | **0.85** | **0.88** |",
        "**Figure 1:** The architecture of FoldNet: (a) encoder, (b) decoder.",
        "**(1b)** Decoder.\n\n![](figs/decoder.pdf)",
        "Theorem 1 and Figure 1b conclude the section.",
        "[^1]: The unit ball, e.g., in $\\mathbb{R}^3$.",
        "## References\n\n- `jumper2021` \u{2014} John Jumper, Richard Evans and Alexander Pritzel (2021). \
         Highly accurate protein structure prediction with AlphaFold. *Nature* 596, 583\u{2013}589. \
         <https://doi.org/10.1038/s41586-021-03819-2>",
        "- `baek2021` \u{2014} Minkyung Baek and Frank DiMaio (2021). \
         Accurate prediction of protein structures and interactions. *Science*.",
    ] {
        assert!(flat.contains(expected), "missing {expected:?} in:\n{md}");
    }
}
