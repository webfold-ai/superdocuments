//! LaTeX module tests: behaviour, golden Markdown and text, figures, citations, limits, and
//! seeded truncation and mutation runs.
//!
//! Golden files live in `tests/fixtures/latex/<name>.expected.md` and `<name>.expected.txt`.
//! To regenerate them after an intended output change, run
//! `LATEX_REGENERATE=1 cargo test --lib latex::tests::golden`, then review the diff.

use super::*;

const FIXTURES: &[&str] = &[
    "article-basic",
    "math",
    "lists",
    "tables",
    "citations",
    "figures",
    "macros",
    "accents",
    "verbatim",
    "unknown-and-malformed",
    "sections-numbering",
    "quotes-minipage",
    "fragment",
    "main-with-input",
    "pictures",
    "starred",
    "booktabs",
    "references",
    "paper-short",
];

fn fixture_text(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/latex/{name}.tex",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap_or_default()
}

fn parsed(name: &str) -> Document {
    parse(&fixture_text(name))
}

fn md(doc: &Document) -> String {
    render_markdown(doc, &RenderOptions::default())
}

fn warnings(doc: &Document) -> String {
    doc.warnings
        .iter()
        .map(|w| w.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn sections(doc: &Document) -> Vec<&Section> {
    doc.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Section(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn text_of(inl: &[Inline]) -> String {
    super::parser::plain(inl)
}

fn all_math(doc: &Document) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    fn walk(blocks: &[Block], out: &mut Vec<(String, bool)>) {
        for b in blocks {
            match b {
                Block::Math(m) => {
                    for r in &m.rows {
                        out.push((r.tex.clone(), true));
                    }
                }
                Block::Paragraph(inl) => {
                    for i in inl {
                        if let Inline::Math { tex, display } = i {
                            out.push((tex.clone(), *display));
                        }
                    }
                }
                Block::Section(s) => {
                    for i in &s.title {
                        if let Inline::Math { tex, display } = i {
                            out.push((tex.clone(), *display));
                        }
                    }
                }
                Block::List(l) => {
                    for item in &l.items {
                        walk(&item.blocks, out);
                    }
                }
                Block::Quote(v) => walk(v, out),
                Block::Theorem(t) => walk(&t.body, out),
                _ => {}
            }
        }
    }
    walk(&doc.blocks, &mut out);
    out
}

// ----- behaviour -------------------------------------------------------------------------------

#[test]
fn title_authors_date_and_abstract() {
    let doc = parsed("article-basic");
    assert_eq!(
        text_of(doc.title.as_deref().unwrap_or(&[])),
        "On Sparse Recovery"
    );
    // \and separates authors; the \\ inside the second block is an affiliation line.
    let authors: Vec<String> = doc.authors.iter().map(|a| text_of(a)).collect();
    assert_eq!(authors, ["Ada Smith", "Bo Chen Example University"]);
    assert_eq!(doc.date.as_deref(), Some("2024"));
    let abs = doc.abstract_body.clone().unwrap_or_default();
    assert_eq!(abs.len(), 1);
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

#[test]
fn sections_levels_numbers_and_starred() {
    let doc = parsed("article-basic");
    let s = sections(&doc);
    let shape: Vec<(u8, bool, Option<String>)> = s
        .iter()
        .map(|x| (x.level, x.starred, x.number.clone()))
        .collect();
    assert_eq!(
        shape,
        vec![
            (1, false, Some("1".into())),
            (2, false, Some("1.1".into())),
            (4, false, None),
            (1, true, None),
        ]
    );
    assert!(md(&doc).contains("## Introduction"));
    assert!(md(&doc).contains("### Setting"));
}

#[test]
fn footnote_and_emphasis_are_inline() {
    let doc = parsed("article-basic");
    let para = match &doc.blocks[1] {
        Block::Paragraph(p) => p.clone(),
        other => panic!("expected a paragraph, got {other:?}"),
    };
    assert!(para.iter().any(|i| matches!(i, Inline::Footnote(_))));
    let rendered = md(&doc);
    assert!(rendered.contains("**basic**"), "{rendered}");
    assert!(rendered.contains("[^1]: Funded by a grant."), "{rendered}");
}

#[test]
fn math_is_kept_as_tex_with_numbers_and_labels() {
    let doc = parsed("math");
    let maths = all_math(&doc);
    let inline: Vec<&str> = maths
        .iter()
        .filter(|m| !m.1)
        .map(|m| m.0.as_str())
        .collect();
    assert_eq!(inline, ["a+b", "c^2"]);
    let display: Vec<&str> = maths.iter().filter(|m| m.1).map(|m| m.0.as_str()).collect();
    assert!(display.contains(&"E = mc^2"), "{display:?}");
    assert!(display.iter().any(|m| m.contains("\\sum_{i=1}^n")));
    assert!(display.iter().any(|m| m.contains("\\int_0^1")));
    // Labels are removed from the TeX and recorded with their numbers.
    assert!(display.iter().all(|m| !m.contains("\\label")));
    assert_eq!(doc.label_number("eq:energy"), Some("1"));
    assert_eq!(doc.label_number("eq:f"), Some("2"));
    // `\nonumber` on the middle row does not use a number, so eq:h is 3.
    assert_eq!(doc.label_number("eq:h"), Some("3"));
    // \tag{$\star$} sets the number and does not advance the counter.
    assert!(display.iter().any(|m| m.contains("c=d")));
    let rendered = md(&doc);
    assert!(rendered.contains("as in (1)."), "{rendered}");
    assert!(rendered.contains("Equation 3 and (2)."), "{rendered}");
}

#[test]
fn lists_nest_and_carry_labels() {
    let doc = parsed("lists");
    let lists: Vec<&List> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::List(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0].kind, ListKind::Itemize);
    assert_eq!(lists[0].items.len(), 3);
    let nested = lists[0].items[1].blocks.iter().find_map(|b| match b {
        Block::List(l) => Some(l),
        _ => None,
    });
    assert_eq!(
        nested.map(|l| (l.kind.clone(), l.items.len())),
        Some((ListKind::Enumerate, 2))
    );
    assert_eq!(lists[1].kind, ListKind::Description);
    let label = lists[1].items[0].label.as_ref().map(|l| text_of(l));
    assert_eq!(label.as_deref(), Some("Term A"));
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

#[test]
fn tables_have_captions_labels_cells_and_spans() {
    let doc = parsed("tables");
    let tables: Vec<&Table> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 2);
    let acc = tables[0];
    assert_eq!(acc.caption_text.as_deref(), Some("Accuracy by method."));
    assert_eq!(acc.label.as_deref(), Some("tab:acc"));
    assert_eq!(acc.number.as_deref(), Some("1"));
    assert_eq!(acc.rows.len(), 3);
    assert_eq!(acc.rows[0].cells.len(), 3);
    assert_eq!(text_of(&acc.rows[2].cells[1].content), "79.5");
    let summary = tables[1];
    assert_eq!(summary.rows[0].cells.len(), 1);
    assert_eq!(summary.rows[0].cells[0].colspan, 2);
    assert_eq!(summary.rows[1].cells.len(), 2);
    assert!(md(&doc).contains("Table 1:"));
    assert!(md(&doc).contains("As Table\u{a0}1 shows"));
}

#[test]
fn citations_link_to_thebibliography_and_report_unresolved_keys() {
    let doc = parsed("citations");
    assert_eq!(doc.bibitems.len(), 2);
    assert_eq!(doc.bibitems[0].key, "lovelace1843");
    assert_eq!(doc.bibitems[0].label.as_deref(), Some("Lov43"));
    assert_eq!(doc.bibitems[1].label, None);
    assert!(
        doc.bibitems[1].raw.contains("TeXbook"),
        "{:?}",
        doc.bibitems[1].raw
    );
    let links = resolve_citations(&doc, None);
    let sources: Vec<(String, CitationSource)> = links
        .iter()
        .map(|l| (l.key.clone(), l.source.clone()))
        .collect();
    assert_eq!(
        sources,
        vec![
            ("lovelace1843".to_string(), CitationSource::Thebibliography),
            ("knuth1984".to_string(), CitationSource::Thebibliography),
            ("hopper1952".to_string(), CitationSource::Unresolved),
            ("missing2000".to_string(), CitationSource::Unresolved),
        ]
    );
    let rendered = md(&doc);
    assert!(rendered.contains("(Lov43)"), "{rendered}");
    assert!(rendered.contains("missing2000"), "{rendered}");
}

#[test]
fn citations_use_a_supplied_bibliography() {
    let doc = parsed("citations");
    let bib = crate::bibtex::parse(
        "@article{hopper1952, author={Grace Hopper and Alan Turing}, title={T}, year={1952}}",
    );
    let rendered = render_markdown(
        &doc,
        &RenderOptions {
            bibliography: Some(&bib),
            ..Default::default()
        },
    );
    assert!(rendered.contains("Hopper and Turing, 1952"), "{rendered}");
    let links = resolve_citations(&doc, Some(&bib));
    let hopper = links.iter().find(|l| l.key == "hopper1952");
    assert_eq!(
        hopper.map(|l| l.source.clone()),
        Some(CitationSource::Bibliography)
    );
}

#[test]
fn figures_carry_captions_labels_paths_subfigures_and_tikz() {
    let doc = parsed("figures");
    let figs = super::figures(&doc);
    let shape: Vec<(String, Option<String>, Option<String>, usize)> = figs
        .iter()
        .map(|f| {
            (
                f.env.clone(),
                f.label.clone(),
                f.number.clone(),
                f.subfigures.len(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            (
                "figure".into(),
                Some("fig:loss".into()),
                Some("1".into()),
                0
            ),
            ("figure*".into(), None, None, 0),
            (
                "figure".into(),
                Some("fig:pair".into()),
                Some("2".into()),
                2
            ),
            ("figure".into(), None, Some("3".into()), 0),
        ]
    );
    let loss = &figs[0];
    assert_eq!(
        text_of(loss.caption.as_deref().unwrap_or(&[])),
        "Training loss over time."
    );
    assert_eq!(loss.image_paths[0].declared, "plots/loss");
    assert_eq!(
        loss.image_paths[0].candidates,
        [
            "plots/loss.pdf",
            "plots/loss.png",
            "plots/loss.jpg",
            "plots/loss.jpeg",
            "plots/loss.eps",
            "plots/loss"
        ]
    );
    assert_eq!(
        loss.image_paths[0].options,
        vec![("width".to_string(), "0.8\\linewidth".to_string())]
    );
    assert_eq!(figs[1].image_paths[0].candidates, ["images/arch.pdf"]);
    assert_eq!(
        figs[1].caption_text.as_deref(),
        Some("Architecture without a number.")
    );
    let pair = &figs[2];
    assert_eq!(pair.subfigures[0].label.as_deref(), Some("fig:left"));
    assert_eq!(pair.subfigures[0].number.as_deref(), Some("2a"));
    assert_eq!(pair.subfigures[1].number.as_deref(), Some("2b"));
    assert_eq!(pair.subfigures[1].image_paths[0].declared, "a/right.png");
    let tikz = &figs[3];
    assert!(tikz.tikz.as_deref().unwrap_or("").contains("\\draw (0,0)"));
    assert!(tikz.image_paths.is_empty());
    assert_eq!(doc.label_number("fig:left"), Some("2a"));
    assert_eq!(doc.label_number("fig:loss"), Some("1"));
    assert!(doc.unresolved_references().is_empty());
}

#[test]
fn bare_includegraphics_in_text_is_an_inline_image_not_a_figure() {
    let doc = parsed("figures");
    let mut found = false;
    for b in &doc.blocks {
        if let Block::Paragraph(p) = b {
            if p.iter()
                .any(|i| matches!(i, Inline::Image(img) if img.declared == "icon"))
            {
                found = true;
            }
        }
    }
    assert!(found);
    assert_eq!(super::figures(&doc).len(), 4);
}

#[test]
fn macros_expand_in_text_and_stay_tex_in_math() {
    let doc = parsed("macros");
    let rendered = md(&doc);
    let plain_text = render_text(&doc, &RenderOptions::default());
    assert!(plain_text.contains("[a|b]"), "{plain_text}");
    let maths = all_math(&doc);
    assert!(
        maths.iter().any(|(tex, _)| tex.contains("\\norm{x}")),
        "{maths:?}"
    );
    assert!(maths
        .iter()
        .any(|(tex, _)| tex == "\\R^n" || tex.contains("\\R^n")));
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    // Theorem and lemma share a counter.
    let theorems: Vec<&Theorem> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Theorem(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(
        theorems.len(),
        2,
        "theorem and lemma; the remarkbox is a quotation"
    );
    let quotes = doc
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Quote(_)))
        .count();
    assert_eq!(quotes, 1);
    assert_eq!(theorems[0].number.as_deref(), Some("1"));
    assert_eq!(
        theorems[0].title.as_ref().map(|t| text_of(t)).as_deref(),
        Some("Main")
    );
    assert_eq!(theorems[0].label.as_deref(), Some("thm:main"));
    assert_eq!(theorems[1].number.as_deref(), Some("2"));
    assert!(
        rendered.replace('\u{a0}', " ").contains("See Theorem 1."),
        "{rendered}"
    );
}

#[test]
fn macro_with_optional_argument_uses_the_default() {
    let doc = parse("\\newcommand{\\hi}[2][Dear]{#1, #2!}\n\\hi{Ada} and \\hi[Hi]{Bo}");
    assert_eq!(text_of(&flatten_doc(&doc)), "Dear, Ada! and Hi, Bo!");
}

fn flatten_doc(doc: &Document) -> Vec<Inline> {
    let mut out = Vec::new();
    for b in &doc.blocks {
        if let Block::Paragraph(p) = b {
            out.extend(p.clone());
        }
    }
    out
}

#[test]
fn accents_and_symbols_become_unicode() {
    let doc = parsed("accents");
    let rendered = md(&doc);
    assert!(rendered.contains("Café and Schön"), "{rendered}");
    assert!(
        rendered.contains("Erdős, Şahin, Ørsted, Straße, Łukasz"),
        "{rendered}"
    );
    assert!(
        rendered.contains("–") && rendered.contains("—"),
        "{rendered}"
    );
    assert!(rendered.contains("“quotes”"), "{rendered}");
    assert!(
        rendered.contains("&")
            && rendered.contains("%")
            && rendered.contains("$")
            && rendered.contains("#")
    );
    assert!(rendered.contains("…"));
    assert!(rendered.contains("école") && rendered.contains("naïve") && rendered.contains("Čapek"));
    assert!(rendered.contains("ñ") && rendered.contains("ā"));
    assert!(rendered.contains('\u{a0}'), "tilde is a non-breaking space");
}

#[test]
fn verbatim_bodies_are_raw_and_keep_their_language() {
    let doc = parsed("verbatim");
    let rendered = md(&doc);
    assert!(
        rendered.contains("Use `a{b}$` for literal text."),
        "{rendered}"
    );
    let verbs: Vec<&Verbatim> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Verbatim(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(verbs.len(), 3);
    assert!(verbs[0].text.contains("x = 1 % not a comment"));
    assert!(verbs[0].text.contains("\\emph{not parsed}"));
    assert_eq!(verbs[1].language.as_deref(), Some("Python"));
    assert!(verbs[1].text.contains("return x  # kept"));
    assert_eq!(verbs[2].language.as_deref(), Some("haskell"));
}

#[test]
fn unknown_commands_and_environments_warn_and_keep_content() {
    let doc = parsed("unknown-and-malformed");
    let w = warnings(&doc);
    assert!(w.contains("unknown command \\mysterymacro"), "{w}");
    assert!(w.contains("unknown environment weirdenv"), "{w}");
    assert!(w.contains("unknown command \\unknowncmd"), "{w}");
    assert!(
        !w.contains("hspace"),
        "hspace is a known argument-dropping command: {w}"
    );
    let rendered = md(&doc);
    assert!(
        rendered.contains("This uses kept argument and  spacing.")
            || rendered.contains("kept argument"),
        "{rendered}"
    );
    assert!(rendered.contains("Inside weirdenv."));
    assert_eq!(doc.inputs, vec!["missing-part".to_string()]);
}

#[test]
fn unterminated_math_warns_and_stops_at_the_end() {
    let doc = parse("Before $x + y and the rest of the text.");
    assert!(warnings(&doc).contains("unterminated inline math"));
    let doc = parse("\\begin{equation} a = b");
    assert!(warnings(&doc).contains("unterminated equation"));
}

#[test]
fn sections_numbering_and_reference_resolution() {
    let doc = parsed("sections-numbering");
    let s = sections(&doc);
    let numbers: Vec<Option<String>> = s.iter().map(|x| x.number.clone()).collect();
    assert_eq!(
        numbers,
        vec![
            Some("1".into()),
            Some("1.1".into()),
            Some("1.1.1".into()),
            Some("1.2".into()),
            Some("2".into()),
            None,
            Some("2.1".into()),
        ]
    );
    assert_eq!(doc.label_number("sec:one"), Some("1"));
    assert_eq!(doc.label_number("sec:two"), Some("2"));
    assert_eq!(doc.unresolved_references(), vec!["sec:missing".to_string()]);
    let shown = md(&doc).replace('\u{a0}', " ");
    assert!(shown.contains("See Section 1 and Section ??."), "{shown}");
}

#[test]
fn quotes_centres_and_minipages_keep_their_text() {
    let doc = parsed("quotes-minipage");
    let quotes = doc
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Quote(_)))
        .count();
    assert_eq!(quotes, 1);
    let rendered = md(&doc);
    assert!(
        rendered.contains("> A quoted paragraph that wraps."),
        "{rendered}"
    );
    assert!(
        rendered.contains("Centered text with *style*."),
        "{rendered}"
    );
    assert!(rendered.contains("Left aligned."));
    assert!(rendered.contains("Inside a minipage."));
}

#[test]
fn fragments_without_a_document_environment_are_read_whole() {
    let doc = parsed("fragment");
    assert_eq!(sections(&doc).len(), 2);
    let lists: Vec<&List> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::List(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].items.len(), 2);
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    assert!(
        !md(&doc).contains("(a)"),
        "the optional argument of enumerate is consumed"
    );
}

#[test]
fn input_files_are_resolved_only_with_a_provider() {
    struct Files;
    impl FileProvider for Files {
        fn read_tex(&self, name: &str) -> Option<String> {
            (name == "sec-intro.tex").then(|| fixture_text_dir("inputs/sec-intro.tex"))
        }
    }
    let source = fixture_text("main-with-input");
    let without = parse(&source);
    assert_eq!(without.inputs, vec!["sec-intro".to_string()]);
    assert!(warnings(&without).contains("not resolved"));
    let with = parse_with(&source, &Options::default(), Some(&Files));
    assert!(with.inputs.is_empty());
    let titles: Vec<String> = sections(&with).iter().map(|s| text_of(&s.title)).collect();
    assert_eq!(titles, ["Main", "Included intro"]);
    assert!(md(&with).contains("Text from an included file with $z$."));
}

fn fixture_text_dir(rel: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/latex/{rel}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_default()
}

#[test]
fn expansion_and_depth_limits_stop_runaway_macros() {
    let looping = "\\def\\a{\\a x}\\a";
    let doc = parse_with(
        looping,
        &Options {
            max_expansions: 50,
            ..Options::default()
        },
        None,
    );
    assert!(
        warnings(&doc).contains("macro expansion limit"),
        "{}",
        warnings(&doc)
    );
    let deep = "\\def\\b{\\b}\\b";
    let doc = parse_with(
        deep,
        &Options {
            max_depth: 8,
            ..Options::default()
        },
        None,
    );
    assert!(!doc.warnings.is_empty());
    let big = "\\def\\c{0123456789}\\c\\c\\c";
    let doc = parse_with(
        big,
        &Options {
            max_expansion_bytes: 12,
            ..Options::default()
        },
        None,
    );
    assert!(
        warnings(&doc).contains("expansion size limit"),
        "{}",
        warnings(&doc)
    );
}

#[test]
fn input_size_and_warning_limits() {
    let doc = parse_with(
        &fixture_text("article-basic"),
        &Options {
            max_input_bytes: 10,
            ..Options::default()
        },
        None,
    );
    assert!(doc.blocks.is_empty());
    assert!(warnings(&doc).contains("over the 10 byte limit"));
    let noisy = "\\foo ".repeat(50);
    let doc = parse_with(
        &noisy,
        &Options {
            max_warnings: 5,
            ..Options::default()
        },
        None,
    );
    assert_eq!(doc.warnings.len(), 5);
    assert!(doc.suppressed_warnings > 0);
}

#[test]
fn every_renderer_runs_on_every_fixture() {
    for name in FIXTURES {
        let doc = parsed(name);
        let _ = render_text(&doc, &RenderOptions::default());
        let _ = md(&doc);
        let _ = render_html(&doc, &RenderOptions::default());
        let _ = super::figures(&doc);
        let _ = resolve_citations(&doc, None);
    }
}

#[test]
fn html_output_escapes_and_wraps_math() {
    let doc = parse("A <b> & $x<y$ and \\[a<b\\].");
    let html = render_html(&doc, &RenderOptions::default());
    assert!(html.contains("&lt;b&gt; &amp;"), "{html}");
    assert!(
        html.contains("<span class=\"math inline\">\\(x&lt;y\\)</span>"),
        "{html}"
    );
    assert!(
        html.contains("<div class=\"math display\">\\[a&lt;b\\]</div>"),
        "{html}"
    );
}

// ----- golden files ----------------------------------------------------------------------------

#[test]
fn golden_files_for_every_fixture() {
    let regenerate = std::env::var("LATEX_REGENERATE").is_ok();
    for name in FIXTURES {
        let doc = parsed(name);
        let markdown = md(&doc);
        let text = render_text(&doc, &RenderOptions::default());
        let base = format!("{}/tests/fixtures/latex/{name}", env!("CARGO_MANIFEST_DIR"));
        if regenerate {
            std::fs::write(format!("{base}.expected.md"), &markdown).unwrap_or_default();
            std::fs::write(format!("{base}.expected.txt"), &text).unwrap_or_default();
            continue;
        }
        let want_md = std::fs::read_to_string(format!("{base}.expected.md")).unwrap_or_default();
        let want_txt = std::fs::read_to_string(format!("{base}.expected.txt")).unwrap_or_default();
        assert_eq!(markdown, want_md, "{name}.expected.md");
        assert_eq!(text, want_txt, "{name}.expected.txt");
    }
}

// ----- seeded truncation and mutation ----------------------------------------------------------

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Any input must parse and render without panicking or hanging.
fn exercise(src: &str) {
    let options = Options {
        max_expansions: 2_000,
        max_depth: 32,
        ..Options::default()
    };
    let doc = parse_with(src, &options, None);
    let _ = render_markdown(&doc, &RenderOptions::default());
    let _ = render_text(&doc, &RenderOptions::default());
    let _ = render_html(&doc, &RenderOptions::default());
    let _ = super::figures(&doc);
}

#[test]
fn truncated_fixtures_never_panic_or_hang() {
    for name in FIXTURES {
        let text = fixture_text(name);
        let len = text.len();
        for cut in (0..=len).step_by(7).chain(len.saturating_sub(40)..=len) {
            if text.is_char_boundary(cut) {
                exercise(&text[..cut]);
            }
        }
    }
}

#[test]
fn mutated_fixtures_never_panic_or_hang() {
    const ALPHABET: &[u8] = b"{}[]$\\%&#~^_'\"`\n a1\\begin\\end";
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    for name in FIXTURES {
        let text = fixture_text(name);
        for _ in 0..120 {
            let mut bytes = text.clone().into_bytes();
            for _ in 0..1 + rng.below(5) {
                if bytes.is_empty() {
                    break;
                }
                let at = rng.below(bytes.len());
                bytes[at] = ALPHABET[rng.below(ALPHABET.len())];
            }
            if let Ok(s) = String::from_utf8(bytes) {
                exercise(&s);
            }
        }
    }
}

#[test]
fn random_fragments_never_panic_or_hang() {
    const PIECES: &[&str] = &[
        "\\begin{itemize}",
        "\\end{itemize}",
        "\\item",
        "\\section{",
        "}",
        "{",
        "$",
        "$$",
        "\\[",
        "\\]",
        "\\\\",
        "&",
        "\\begin{tabular}{ll}",
        "\\end{tabular}",
        "\\caption{",
        "\\label{x}",
        "\\newcommand{\\z}[1]{#1#1}",
        "\\z{",
        "\\begin{figure}",
        "\\end{figure}",
        "\\includegraphics{a}",
        "\\cite{k}",
        "\\bibitem{k}",
        "\\verb|",
        "%",
        "\n\n",
        "word ",
        "\\def\\q#1{\\q}",
        "\\q{",
        "\\begin{verbatim}",
        "\\end{verbatim}",
        "\\'",
        "é",
    ];
    let mut rng = Xorshift(0x0123_4567_89AB_CDEF);
    for _ in 0..400 {
        let mut s = String::new();
        for _ in 0..(1 + rng.below(60)) {
            s.push_str(PIECES[rng.below(PIECES.len())]);
        }
        exercise(&s);
    }
}

#[test]
fn bibtex_round_trip_into_citations_is_stable() {
    let bib = crate::bibtex::parse(
        &std::fs::read_to_string(format!(
            "{}/tests/fixtures/bibtex/basic.bib",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_default(),
    );
    let doc = parse("\\cite{knuth1984} and \\citet{lovelace1843}.");
    let rendered = render_markdown(
        &doc,
        &RenderOptions {
            bibliography: Some(&bib),
            ..Default::default()
        },
    );
    assert!(rendered.contains("(Knuth, 1984)"), "{rendered}");
    assert!(
        rendered.contains("Lovelace and Babbage, 1843")
            || rendered.contains("Lovelace and Babbage"),
        "{rendered}"
    );
}

#[test]
fn newline_after_control_word_is_swallowed_and_comments_end_runs() {
    // TeX skips the line break after a control word, and a comment eats its newline.
    let text = render_text(
        &parse("\\ldots\nand word%hidden\nnext"),
        &RenderOptions::default(),
    );
    assert!(text.contains("…and"), "{text}");
    assert!(text.contains("wordnext"), "{text}");
}

#[test]
fn standalone_tikzpicture_options_are_not_code() {
    let doc = parse("\\begin{tikzpicture}[scale=2]\\draw (0,0);\\end{tikzpicture}");
    let figs = super::figures(&doc);
    assert_eq!(figs.len(), 1);
    assert!(figs[0]
        .tikz
        .as_deref()
        .unwrap_or("")
        .trim_start()
        .starts_with("\\draw"));
}

#[test]
fn citation_notes_are_decoded() {
    let doc = parse("See \\citep[p.~2][p.~3]{k}.");
    let cite = doc.blocks.iter().find_map(|b| match b {
        Block::Paragraph(p) => p.iter().find_map(|i| match i {
            Inline::Cite(c) => Some(c.clone()),
            _ => None,
        }),
        _ => None,
    });
    let cite = cite.unwrap_or(crate::latex::Cite {
        command: String::new(),
        pre: None,
        post: None,
        keys: vec![],
    });
    assert_eq!(cite.pre, Some("p.\u{a0}2".to_string()));
    assert_eq!(cite.post, Some("p.\u{a0}3".to_string()));
}

#[test]
fn figures_inside_theorems_are_collected() {
    let doc = parse("\\newtheorem{thm}{Theorem}\n\\begin{thm}\n\\begin{figure}\\caption{Inside.}\\end{figure}\n\\end{thm}");
    assert_eq!(super::figures(&doc).len(), 1);
}

// ----- \input bounds -----------------------------------------------------------------------------

struct MapFiles(Vec<(&'static str, &'static str)>);

impl FileProvider for MapFiles {
    fn read_tex(&self, name: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| (*text).to_string())
    }
}

#[test]
fn an_input_cycle_through_a_macro_is_not_followed() {
    // `loop.tex` reaches itself through a macro defined in the main file.
    let files = MapFiles(vec![("loop.tex", "LOOP-BODY \\again")]);
    let doc = parse_with(
        "\\newcommand{\\again}{\\input{loop}}\nstart \\again{} end",
        &Options::default(),
        Some(&files),
    );
    let text = render_text(&doc, &RenderOptions::default());
    assert_eq!(text.matches("LOOP-BODY").count(), 1, "{text}");
    assert!(text.trim_end().ends_with("end"), "{text}");
    assert!(
        warnings(&doc).contains("includes itself"),
        "{}",
        warnings(&doc)
    );
    // The same file may be read twice in a row: that is not a cycle.
    let files = MapFiles(vec![("part.tex", "PART")]);
    let doc = parse_with(
        "\\input{part} \\input{part}",
        &Options::default(),
        Some(&files),
    );
    let text = render_text(&doc, &RenderOptions::default());
    assert_eq!(text.matches("PART").count(), 2, "{text}");
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

#[test]
fn a_missing_input_says_whether_a_provider_was_asked() {
    let without = parse("\\input{gone}");
    assert!(
        warnings(&without).contains("not resolved; no file provider"),
        "{}",
        warnings(&without)
    );
    let with = parse_with(
        "\\input{gone}",
        &Options::default(),
        Some(&MapFiles(vec![])),
    );
    assert!(
        warnings(&with).contains("not resolved; the file provider has no such file"),
        "{}",
        warnings(&with)
    );
    assert_eq!(with.inputs, ["gone"]);
}

// ----- picture environments ----------------------------------------------------------------------

fn figure_blocks(doc: &Document) -> Vec<&Figure> {
    doc.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Figure(f) => Some(f),
            _ => None,
        })
        .collect()
}

#[test]
fn a_picture_keeps_its_whole_source_with_options_caption_and_label() {
    let doc = parse(
        "\\begin{figure}\n  \\begin{tikzpicture}[scale=2, auto]\n    \\draw (0,0) -- (1,1);\n  \
         \\end{tikzpicture}\n  \\caption{A diagram.}\\label{fig:d}\n\\end{figure}\nSee \\ref{fig:d}.",
    );
    let figs = figure_blocks(&doc);
    assert_eq!(figs.len(), 1);
    assert_eq!(
        figs[0].pictures,
        vec![Picture {
            env: "tikzpicture".into(),
            source:
                "\\begin{tikzpicture}[scale=2, auto]\n  \\draw (0,0) -- (1,1);\n\\end{tikzpicture}"
                    .into(),
        }]
    );
    assert_eq!(figs[0].caption_text.as_deref(), Some("A diagram."));
    assert_eq!(figs[0].label.as_deref(), Some("fig:d"));
    assert_eq!(doc.label_number("fig:d"), Some("1"));
    assert_eq!(
        md(&doc),
        "*TikZ picture, not rendered.*\n\n```latex\n\\begin{tikzpicture}[scale=2, auto]\n  \
         \\draw (0,0) -- (1,1);\n\\end{tikzpicture}\n```\n\n**Figure 1:** A diagram.\n\nSee 1.\n"
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

#[test]
fn every_picture_environment_is_kept_as_source_not_read_as_text() {
    for env in [
        "tikzpicture",
        "pgfpicture",
        "picture",
        "circuitikz",
        "forest",
        "pspicture",
        "tikzcd",
        "asy",
    ] {
        let body = "[S [NP & she] $ # % \\unknowncommand{x}]";
        let src = format!("Before.\n\\begin{{{env}}}\n{body}\n\\end{{{env}}}\nAfter.");
        let doc = parse(&src);
        assert!(warnings(&doc).is_empty(), "{env}: {}", warnings(&doc));
        let figs = figure_blocks(&doc);
        assert_eq!(figs.len(), 1, "{env}");
        assert_eq!(figs[0].env, env);
        let expected = format!("\\begin{{{env}}}\n{body}\n\\end{{{env}}}");
        assert_eq!(figs[0].pictures.len(), 1, "{env}");
        assert_eq!(figs[0].pictures[0].source, expected, "{env}");
        let rendered = md(&doc);
        assert!(
            rendered.contains(&format!("```latex\n{expected}\n```")),
            "{env}: {rendered}"
        );
        // The body appears once, inside the fence, and the text around it is intact.
        assert_eq!(rendered.matches("[S [NP").count(), 1, "{env}: {rendered}");
        assert!(rendered.starts_with("Before.\n"), "{env}: {rendered}");
        assert!(rendered.trim_end().ends_with("After."), "{env}: {rendered}");
        let text = render_text(&doc, &RenderOptions::default());
        assert!(text.contains("picture, not rendered]"), "{env}: {text}");
        let html = render_html(&doc, &RenderOptions::default());
        assert!(
            html.contains("<pre><code class=\"language-latex\">\\begin{"),
            "{env}: {html}"
        );
    }
}

#[test]
fn several_pictures_nested_pictures_and_comments_are_all_kept() {
    let doc = parsed("pictures");
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    let figs = super::figures(&doc);
    let envs: Vec<&str> = figs.iter().map(|f| f.env.as_str()).collect();
    assert_eq!(envs, ["figure", "figure", "figure*", "pgfpicture"]);
    // Two pictures in one float; a commented-out \end does not end the first.
    assert_eq!(figs[0].pictures.len(), 2);
    assert!(figs[0].pictures[0]
        .source
        .contains("\\draw[->] (a) -- (b);"));
    assert!(figs[0].pictures[0]
        .source
        .starts_with("\\begin{tikzpicture}[scale=0.8, every node/.style={draw}]\n"));
    assert!(figs[0].pictures[1].source.contains("circle (1)"));
    // The older field still gives the first tikzpicture's body.
    assert!(figs[0]
        .tikz
        .as_deref()
        .unwrap_or("")
        .contains("\\node (b) at (2,0) {B};"));
    assert_eq!(figs[1].pictures[0].env, "forest");
    assert_eq!(figs[1].number.as_deref(), Some("2"));
    // Pictures inside subfigures stay with their subfigure.
    let subs = &figs[2].subfigures;
    assert_eq!(subs[0].pictures[0].env, "circuitikz");
    assert_eq!(subs[0].number.as_deref(), Some("3a"));
    assert_eq!(subs[1].pictures[0].env, "picture");
    assert!(subs[1].pictures[0]
        .source
        .starts_with("\\begin{picture}(100,50)\n"));
    // A picture in a table float follows the table as a picture of its own.
    let after_table = doc
        .blocks
        .iter()
        .position(|b| matches!(b, Block::Table(t) if t.label.as_deref() == Some("tab:pic")));
    let next = after_table.and_then(|i| doc.blocks.get(i + 1));
    assert!(
        matches!(next, Some(Block::Figure(f)) if f.pictures[0].env == "pgfpicture"),
        "{next:?}"
    );
    assert_eq!(doc.label_number("fig:r"), Some("3a"));
    assert_eq!(doc.label_number("tab:pic"), Some("1"));

    let nested = parse(
        "\\begin{tikzpicture}\\node{\\begin{tikzpicture}\\draw (0,0);\\end{tikzpicture}};\
         \\draw (1,1);\\end{tikzpicture} tail",
    );
    let figs = figure_blocks(&nested);
    assert_eq!(figs.len(), 1);
    assert!(figs[0].pictures[0]
        .source
        .ends_with("\\draw (1,1);\\end{tikzpicture}"));
    assert!(md(&nested).trim_end().ends_with("tail"));
}

#[test]
fn an_unterminated_picture_warns() {
    let doc = parse("\\begin{forest} [a [b]]");
    assert!(
        warnings(&doc).contains("unterminated forest"),
        "{}",
        warnings(&doc)
    );
    assert_eq!(figure_blocks(&doc)[0].pictures[0].env, "forest");
}

#[test]
fn a_fence_is_longer_than_the_backticks_in_the_picture() {
    let doc = parse("\\begin{asy}\nlabel(\"```\");\n\\end{asy}");
    let rendered = md(&doc);
    assert!(rendered.contains("````latex\n"), "{rendered}");
    assert!(rendered.contains("\n````\n"), "{rendered}");
}

#[test]
fn floats_do_not_drop_what_their_block_cannot_hold() {
    // An image in a table float, a tabular in a figure float, a listing in a figure float,
    // and threeparttable notes.
    let doc = parse(
        "\\begin{table}\\caption{As image}\\includegraphics{tab.png}\\end{table}\n\
         \\begin{figure}\\begin{tabular}{ll} a & b \\\\ c & d \\\\ \\end{tabular}\\caption{As rows}\\end{figure}\n\
         \\begin{figure}\n\\begin{lstlisting}[language=Python]\nx = 1  # 50% & $y$\n\\end{lstlisting}\n\\caption{Code}\\end{figure}\n\
         \\begin{table}\\begin{threeparttable}\\caption{Notes}\\begin{tabular}{l} v \\\\ \\end{tabular}\
         \\begin{tablenotes}\\item First note. \\item Second note.\\end{tablenotes}\\end{threeparttable}\\end{table}",
    );
    let rendered = md(&doc);
    assert!(
        rendered.contains("**Table 1:** As image\n\n![](tab.png)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("**Figure 1:** As rows\n\n| a | b |\n| --- | --- |\n| c | d |"),
        "{rendered}"
    );
    assert!(
        rendered.contains("**Figure 2:** Code\n\n```\nx = 1  # 50% & $y$\n```"),
        "{rendered}"
    );
    assert!(
        rendered.contains("| v |\n| --- |\n\n- First note.\n- Second note."),
        "{rendered}"
    );
    let listing = doc.blocks.iter().find_map(|b| match b {
        Block::Verbatim(v) => Some(v),
        _ => None,
    });
    assert_eq!(listing.and_then(|v| v.language.as_deref()), Some("Python"));
}

// ----- starred environments ----------------------------------------------------------------------

/// The structure of a document with everything a star may change taken out: environment
/// names and numbers.
fn structure(doc: &Document) -> Vec<String> {
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Section(s) => out.push(format!("section {}", text_of(&s.title))),
                Block::Paragraph(p) => out.push(format!("paragraph {}", text_of(p))),
                Block::List(l) => out.push(format!("list of {}", l.items.len())),
                Block::Table(t) => out.push(format!(
                    "table caption={:?} label={:?} rows={:?}",
                    t.caption_text,
                    t.label,
                    t.rows
                        .iter()
                        .map(|r| r
                            .cells
                            .iter()
                            .map(|c| text_of(&c.content))
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                )),
                Block::Math(m) => out.push(format!(
                    "math {:?}",
                    m.rows
                        .iter()
                        .map(|r| (r.tex.as_str(), r.label.as_deref()))
                        .collect::<Vec<_>>()
                )),
                Block::Verbatim(v) => out.push(format!("verbatim {}", v.text)),
                Block::Figure(f) => out.push(format!(
                    "figure caption={:?} label={:?} images={:?} subfigures={:?}",
                    f.caption_text,
                    f.label,
                    f.image_paths
                        .iter()
                        .map(|i| i.declared.as_str())
                        .collect::<Vec<_>>(),
                    f.subfigures
                        .iter()
                        .map(|s| (s.caption_text.clone(), s.label.clone()))
                        .collect::<Vec<_>>()
                )),
                Block::Theorem(t) => {
                    out.push(format!("theorem {} label={:?}", t.name, t.label));
                    walk(&t.body, out);
                }
                Block::Quote(inner) => walk(inner, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    out
}

#[test]
fn starred_environments_have_the_structure_of_their_unstarred_forms() {
    let float = "\\centering\\includegraphics[width=\\linewidth]{img/a}\n\
                 \\caption{A caption.}\\label{x:one}";
    let subfigures = "\\begin{subfigure}{0.5\\textwidth}\\includegraphics{l}\\caption{Left.}\\label{x:l}\\end{subfigure}\n\
                      \\begin{subfigure}{0.5\\textwidth}\\includegraphics{r}\\caption{Right.}\\end{subfigure}\n\
                      \\caption{Both.}\\label{x:one}";
    let table = "\\caption{A table.}\\label{x:one}\n\\begin{tabular}{lr} a & 1 \\\\ b & 2 \\\\ \\end{tabular}";
    let cases: &[(&str, &str, &str)] = &[
        ("figure", "[t]", float),
        ("figure", "", subfigures),
        ("sidewaysfigure", "", float),
        ("table", "[ht]", table),
        ("sidewaystable", "", table),
        ("equation", "", "E = mc^2 \\label{x:one}"),
        ("align", "", "a &= b \\label{x:one} \\\\ c &= d"),
        ("gather", "", "a = b \\label{x:one} \\\\ c = d"),
        ("multline", "", "a + b \\\\ = c \\label{x:one}"),
        ("alignat", "{2}", "a &= b &\\quad c &= d \\label{x:one}"),
        ("flalign", "", "a &= b & \\label{x:one}"),
        ("xalignat", "{2}", "a &= b & c &= d \\label{x:one}"),
        ("eqnarray", "", "a &=& b \\label{x:one} \\\\ c &=& d"),
    ];
    for (env, args, body) in cases {
        let plain = parse(&format!(
            "Before.\n\\begin{{{env}}}{args}\n{body}\n\\end{{{env}}}\nAfter."
        ));
        let starred = parse(&format!(
            "Before.\n\\begin{{{env}*}}{args}\n{body}\n\\end{{{env}*}}\nAfter."
        ));
        assert!(warnings(&plain).is_empty(), "{env}: {}", warnings(&plain));
        assert!(
            warnings(&starred).is_empty(),
            "{env}*: {}",
            warnings(&starred)
        );
        assert_eq!(structure(&plain), structure(&starred), "{env} and {env}*");
        // Three blocks: the text before, the environment, the text after.
        assert_eq!(structure(&plain).len(), 3, "{env}: {:?}", structure(&plain));
        assert_eq!(plain.labels.len(), starred.labels.len(), "{env}");
        assert!(
            plain.labels.iter().any(|l| l.name == "x:one"),
            "{env}: {:?}",
            plain.labels
        );
        let is_math = matches!(plain.blocks[1], Block::Math(_));
        let starred_number = starred.label_number("x:one");
        if is_math {
            // A starred display is not numbered; the unstarred one is.
            assert_eq!(plain.label_number("x:one"), Some("1"), "{env}");
            assert_eq!(starred_number, None, "{env}*");
        } else {
            // A starred float spans the columns: it is numbered like any other float.
            assert_eq!(plain.label_number("x:one"), Some("1"), "{env}");
            assert_eq!(starred_number, Some("1"), "{env}*");
        }
    }
}

#[test]
fn a_starred_figure_is_a_figure_with_its_subfigures() {
    // The case behind the two recorded differences from pandoc, which reads figure* as a Div.
    let doc = parsed("starred");
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    let figs = super::figures(&doc);
    let shape: Vec<(&str, Option<&str>, usize)> = figs
        .iter()
        .map(|f| (f.env.as_str(), f.number.as_deref(), f.subfigures.len()))
        .collect();
    assert_eq!(shape, [("figure*", Some("1"), 2), ("figure", Some("2"), 0)]);
    assert_eq!(figs[0].subfigures[1].number.as_deref(), Some("1b"));
    assert_eq!(doc.label_number("fig:wide"), Some("1"));
    assert_eq!(doc.label_number("fig:wide-b"), Some("1b"));
    assert_eq!(doc.label_number("tab:wide"), Some("1"));
    assert_eq!(doc.label_number("eq:plain"), Some("1"));
    assert_eq!(doc.label_number("eq:starred"), None);
    let tables = doc
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Table(t) if t.env == "table*" && t.rows.len() == 2))
        .count();
    assert_eq!(tables, 1);
}

#[test]
fn the_column_argument_of_alignat_is_not_math() {
    let doc = parse("\\begin{alignat}{2} a &= b \\\\ c &= d \\end{alignat}");
    let rows: Vec<(String, bool)> = all_math(&doc);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].0, "a &= b");
    assert!(rows.iter().all(|(tex, _)| !tex.contains("{2}")), "{rows:?}");
}

#[test]
fn theorems_work_starred_and_without_a_declaration() {
    let doc = parse(
        "\\newtheorem*{mainthm}{Main Theorem}\n\\newtheorem{prop}{Proposition}\n\
         \\begin{mainthm} Unnumbered. \\end{mainthm}\n\
         \\begin{prop}\\label{p} Numbered. \\end{prop}\n\
         \\begin{lemma}\\label{l} Undeclared lemma. \\end{lemma}\n\
         \\begin{lemma} Second. \\end{lemma}\n\
         \\begin{definition*} Starred and undeclared. \\end{definition*}",
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    let theorems: Vec<(&str, Option<&str>)> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Theorem(t) => Some((t.name.as_str(), t.number.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(
        theorems,
        [
            ("Main Theorem", None),
            ("Proposition", Some("1")),
            ("Lemma", Some("1")),
            ("Lemma", Some("2")),
            ("Definition", None),
        ]
    );
    let rendered = md(&doc);
    assert!(
        rendered.contains("**Main Theorem.** Unnumbered."),
        "{rendered}"
    );
    assert!(rendered.contains("**Lemma 2.** Second."), "{rendered}");
    assert!(
        !rendered.contains('*') || !rendered.contains("mainthmMain"),
        "{rendered}"
    );
}

// ----- macros ------------------------------------------------------------------------------------

#[test]
fn environments_opened_and_closed_by_macros_end_where_the_macro_says() {
    // The common shorthand of older papers. The document must go on after each one.
    let doc = parse(
        "\\newcommand{\\be}{\\begin{equation}}\n\\newcommand{\\ee}{\\end{equation}}\n\
         \\def\\bea{\\begin{eqnarray}}\n\\def\\eea{\\end{eqnarray}}\n\
         \\newenvironment{eq}{\\begin{equation}}{\\end{equation}}\n\
         First.\n\\be x = 1 \\label{eq:x} \\ee\nSecond \\eqref{eq:x}.\n\
         \\bea a &=& b \\\\ c &=& d \\eea\nThird.\n\
         \\begin{eq} y = 2 \\end{eq}\nFourth, with \\eeprom unknown.",
    );
    let shape = structure(&doc);
    assert_eq!(
        shape,
        [
            "paragraph First.",
            "math [(\"x = 1\", Some(\"eq:x\"))]",
            "paragraph Second eq:x.",
            "math [(\"a &=& b\", None), (\"c &=& d\", None)]",
            "paragraph Third.",
            "math [(\"y = 2\", None)]",
            "paragraph Fourth, with unknown.",
        ]
    );
    assert_eq!(doc.label_number("eq:x"), Some("1"));
    // \eeprom is another command, not \ee followed by text.
    assert!(warnings(&doc).contains("\\eeprom"), "{}", warnings(&doc));
    let rendered = md(&doc);
    assert!(rendered.contains("Second (1)."), "{rendered}");
    assert!(rendered.contains("y = 2 \\tag{4}"), "{rendered}");
}

#[test]
fn starred_definitions_define_the_command() {
    let doc = parse(
        "\\newcommand*{\\one}{ONE}\\renewcommand*{\\one}{UNO}\\providecommand*{\\two}{TWO}\n\
         \\newcommand*{\\wrap}[1]{<#1>}\\DeclareRobustCommand{\\three}{THREE}\n\
         \\DeclareMathOperator*{\\argmax}{arg\\,max}\\DeclareMathOperator{\\tr}{tr}\n\
         \\one{} \\two{} \\wrap{x} \\three{} $\\argmax_x \\tr A$",
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    assert_eq!(
        render_text(&doc, &RenderOptions::default()).trim(),
        "UNO TWO <x> THREE \\argmax_x \\tr A"
    );
    let expanded = parse_with(
        "\\DeclareMathOperator*{\\argmax}{arg\\,max}\\DeclareMathOperator{\\tr}{tr}$\\argmax_x \\tr A$",
        &Options {
            expand_math_macros: true,
            ..Options::default()
        },
        None,
    );
    assert_eq!(
        all_math(&expanded),
        [(
            "\\operatorname*{arg\\,max}_x \\operatorname{tr} A".to_string(),
            false
        )]
    );
}

fn math_expanded(source: &str) -> Vec<String> {
    let doc = parse_with(
        source,
        &Options {
            expand_math_macros: true,
            ..Options::default()
        },
        None,
    );
    all_math(&doc).into_iter().map(|(tex, _)| tex).collect()
}

#[test]
fn math_macros_expand_only_when_asked() {
    let defs = "\\newcommand{\\R}{\\mathbb{R}}\\newcommand{\\norm}[1]{\\left\\lVert #1 \\right\\rVert}\n\
                \\newcommand{\\vv}[1]{\\vec #1}\\newcommand{\\E}[2][x]{\\mathbb{E}_{#1}\\left[#2\\right]}\n\
                \\def\\half{\\frac{1}{2}}\\newcommand{\\RR}{\\R\\times\\R}\\newcommand{\\eps}{\\varepsilon}\n\
                \\newcommand{\\gr}[1]{\\alpha#1}\\DeclarePairedDelimiter{\\abs}{\\lvert}{\\rvert}\n";
    let source = format!(
        "{defs}$x \\in \\R^n$, $2^\\R$, $\\norm{{x_i}}$, $\\vv a$, $\\E{{f}} + \\E[y]{{g}}$, \
         $\\half\\RR$, $\\eps x \\epsilon \\Rightarrow$, $\\gr{{b}}\\gr{{2}}$, $\\abs{{x}} + \\abs*{{y}}$\n\\[ \\norm{{\\norm{{y}}}} \\]\n\
         \\begin{{align}} \\R &= \\R \\label{{eq:r}} \\\\ \\half &= 0.5 \\end{{align}}"
    );
    // By default math is exactly what the author wrote.
    let raw: Vec<String> = all_math(&parse(&source))
        .into_iter()
        .map(|(tex, _)| tex)
        .collect();
    assert_eq!(raw[0], "x \\in \\R^n");
    assert_eq!(raw[2], "\\norm{x_i}");
    // With the option, no macro of the document is left in the math.
    let expanded = math_expanded(&source);
    assert_eq!(
        expanded,
        [
            "x \\in \\mathbb{R}^n",
            // After ^ the expansion is braced, so it is still one superscript.
            "2^{\\mathbb{R}}",
            "\\left\\lVert x_i \\right\\rVert",
            // `\vec #1` with the argument `a` keeps the space that separates the letters.
            "\\vec a",
            "\\mathbb{E}_{x}\\left[f\\right] + \\mathbb{E}_{y}\\left[g\\right]",
            "\\frac{1}{2}\\mathbb{R}\\times\\mathbb{R}",
            // \epsilon and \Rightarrow are not \eps and \R followed by letters.
            "\\varepsilon x \\epsilon \\Rightarrow",
            // `\alpha#1` with the argument `b` must not become `\alphab`.
            "\\alpha b\\alpha2",
            "\\lvert x \\rvert + \\left\\lvert y \\right\\rvert",
            "\\left\\lVert \\left\\lVert y \\right\\rVert \\right\\rVert",
            "\\mathbb{R} &= \\mathbb{R}",
            "\\frac{1}{2} &= 0.5",
        ]
    );
}

#[test]
fn runaway_math_macros_stop_at_the_limits() {
    // A macro that expands to itself, and one that doubles each time.
    let doc = parse_with(
        "\\newcommand{\\loop}{\\loop x}\\newcommand{\\dbl}[1]{\\dbl{#1#1}} $\\loop$ $\\dbl{ab}$ after",
        &Options {
            expand_math_macros: true,
            max_expansions: 500,
            max_expansion_bytes: 20_000,
            ..Options::default()
        },
        None,
    );
    let maths = all_math(&doc);
    assert_eq!(maths.len(), 2);
    assert!(maths.iter().all(|(tex, _)| tex.len() < 200_000), "bounded");
    assert!(
        render_text(&doc, &RenderOptions::default())
            .trim_end()
            .ends_with("after"),
        "the text after the math is still read"
    );
}

#[test]
fn xspace_adds_a_space_only_before_a_word() {
    let doc = parse(
        "\\newcommand{\\sys}{\\textsc{FoldNet}\\xspace}\
         \\sys is fast. We use \\sys, then \\sys. \\sys (ours) and \\sys's \\sys{} \\textbf{\\sys}",
    );
    assert_eq!(
        render_text(&doc, &RenderOptions::default()).trim(),
        "FoldNet is fast. We use FoldNet, then FoldNet. FoldNet (ours) and FoldNet's FoldNet FoldNet"
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

#[test]
fn ensuremath_in_text_is_math() {
    let doc = parse(
        "\\newcommand{\\Rn}{\\ensuremath{\\mathbb{R}^n}}A point of \\Rn{} and $x \\in \\Rn$.",
    );
    assert_eq!(
        all_math(&doc),
        [
            ("\\mathbb{R}^n".to_string(), false),
            ("x \\in \\Rn".to_string(), false)
        ]
    );
    assert_eq!(
        md(&doc).trim(),
        "A point of $\\mathbb{R}^n$ and $x \\in \\Rn$."
    );
    assert_eq!(
        math_expanded("\\newcommand{\\Rn}{\\ensuremath{\\mathbb{R}^n}}$x \\in \\Rn$"),
        ["x \\in \\mathbb{R}^n"]
    );
}

#[test]
fn false_conditionals_are_skipped_and_switches_are_followed() {
    let doc = parse(
        "Shown. \\iffalse HIDDEN-A \\ifx\\a\\b nested \\fi still hidden \\fi Visible.\n\
         \\iftrue KEPT \\else HIDDEN-B \\fi\n\
         \\newif\\ifdraft \\ifdraft HIDDEN-C \\else FINAL \\fi\n\
         \\drafttrue \\ifdraft DRAFT \\else HIDDEN-D \\fi End.",
    );
    assert_eq!(
        render_text(&doc, &RenderOptions::default()).trim(),
        "Shown. Visible. KEPT FINAL DRAFT End."
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    // Other conditionals are not evaluated: the first branch is kept, and that is said.
    let doc = parse("\\ifdefined\\foo first \\else second \\fi tail");
    assert_eq!(
        render_text(&doc, &RenderOptions::default()).trim(),
        "first tail"
    );
    assert!(
        warnings(&doc).contains("not evaluated"),
        "{}",
        warnings(&doc)
    );
}

#[test]
fn let_copies_a_macro_of_the_document() {
    let doc = parse("\\newcommand{\\old}{VALUE}\\let\\new\\old \\let\\other=\\old \\new{} \\other{} \\let\\x=\\relax done");
    assert_eq!(
        render_text(&doc, &RenderOptions::default()).trim(),
        "VALUE VALUE done"
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

// ----- references --------------------------------------------------------------------------------

#[test]
fn named_references_say_what_they_point_to() {
    let doc = parsed("references");
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    assert_eq!(doc.unresolved_references(), ["sec:missing", "fig:missing"]);
    let text = render_text(&doc, &RenderOptions::default()).replace('\u{a0}', " ");
    for expected in [
        "cref: section 1, fig. 1, table 1, eq. (1), lemma 1, algorithm 1, appendix A.",
        "Cref: Section 1, Figure 1, Table 1, Equation (1), Lemma 1, Algorithm 1, Appendix A.",
        "autoref: section 1, Figure 1, Table 1, Equation 1, Lemma 1, Algorithm 1, Appendix A.",
        "plain: 1, 1, 1, (1), 1, 1, A.",
        "several: figs. 1 and 1a, Sections 1, 1.1 and A, eqs. (1) and (2).",
        "subfigures: (a) and (b) of Figure 1; 1b.",
        "missing: ??, sections 1 and ??, ??.",
        "starred: 1.",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn appendix_sections_are_lettered() {
    let doc = parse(
        "\\section{Intro}\\label{s}\\appendix\\section{Proofs}\\label{a}\\subsection{Lemma}\\label{a1}\
         \\section{Data}\\label{b}",
    );
    let numbers: Vec<Option<&str>> = sections(&doc).iter().map(|s| s.number.as_deref()).collect();
    assert_eq!(numbers, [Some("1"), Some("A"), Some("A.1"), Some("B")]);
    let kinds: Vec<&str> = doc.labels.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(kinds, ["section", "appendix", "appendix", "appendix"]);
    assert_eq!(doc.label_number("a1"), Some("A.1"));
}

// ----- tables ------------------------------------------------------------------------------------

#[test]
fn booktabs_rules_and_row_spans_do_not_leak_into_cells() {
    let doc = parsed("booktabs");
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    let table = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no table"));
    let rows: Vec<Vec<(String, usize)>> = table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| (text_of(&c.content), c.colspan))
                .collect()
        })
        .collect();
    let cell = |s: &str| (s.to_string(), 1usize);
    assert_eq!(
        rows,
        vec![
            vec![
                cell(""),
                ("CASP14".to_string(), 2),
                ("CAMEO".to_string(), 2)
            ],
            vec![
                cell("Method"),
                cell("TM"),
                cell("GDT"),
                cell("TM"),
                cell("GDT")
            ],
            vec![
                cell("Baseline"),
                cell("0.71"),
                cell("65.2"),
                cell("0.80"),
                cell("77.1")
            ],
            vec![
                cell(""),
                cell("0.72"),
                cell("66.0"),
                cell("0.81"),
                cell("77.9")
            ],
            vec![
                cell("Ours (large)"),
                cell("0.85"),
                cell("80.1"),
                cell("0.88"),
                cell("84.3")
            ],
        ]
    );
    let rendered = md(&doc);
    // The spanned header keeps CAMEO over its own two columns.
    assert!(
        rendered.contains("|  | CASP14 |  | CAMEO |  |\n| --- | --- | --- | --- | --- |\n| Method | TM | GDT | TM | GDT |"),
        "{rendered}"
    );
    for leaked in ["lr", "2-3", "2\\*", "gray", "1.5pt", "0.5em"] {
        assert!(!rendered.contains(leaked), "{leaked} leaked: {rendered}");
    }
    let html = render_html(&doc, &RenderOptions::default());
    assert!(
        html.contains(
            "<tr><td></td><td colspan=\"2\">CASP14</td><td colspan=\"2\">CAMEO</td></tr>"
        ),
        "{html}"
    );
    let text = render_text(&doc, &RenderOptions::default());
    assert!(text.contains("\tCASP14\t\tCAMEO\t\n"), "{text:?}");
}

#[test]
fn a_line_break_inside_a_cell_does_not_break_the_row() {
    let doc = parse(
        "\\begin{tabular}{ll} a & \\makecell{first \\\\ second} \\\\ b & c \\\\ \\end{tabular}",
    );
    assert_eq!(
        md(&doc),
        "| a | first <br>second |\n| --- | --- |\n| b | c |\n"
    );
    assert!(render_text(&doc, &RenderOptions::default()).contains("a\tfirst second\n"));
}

// ----- units, citations, algorithms ----------------------------------------------------------------

#[test]
fn siunitx_quantities_keep_their_number_and_unit() {
    let doc = parse(
        "\\SI{3.5}{\\kilo\\hertz}, \\qty{9.81}{\\meter\\per\\second\\squared}, \\si{\\micro\\meter}, \
         \\SI{20}{\\percent}, \\SI{-4}{\\celsius}, \\num{1.5e-3}, \\num{2 +- 0.1}, \
         \\SIrange{1}{5}{\\milli\\second}, \\unit{kg.m^{-1}}, \\ang{45}, \\SI{2}{\\furlong}.",
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    assert_eq!(
        render_text(&doc, &RenderOptions::default()).trim(),
        "3.5\u{a0}kHz, 9.81\u{a0}m/s\u{b2}, \u{b5}m, 20%, \u{2212}4\u{b0}C, 1.5\u{a0}\u{d7}\u{a0}10\u{207b}\u{b3}, \
         2\u{a0}\u{b1}\u{a0}0.1, 1\u{2013}5\u{a0}ms, kg\u{b7}m\u{207b}\u{b9}, 45\u{b0}, 2\u{a0}furlong."
    );
}

#[test]
fn citation_commands_of_natbib_and_biblatex_are_citations() {
    let bib = crate::bibtex::parse(
        "@article{k, author={Knuth, Donald}, year=1984, title={T}}\n\
         @article{l, author={Lamport, Leslie and Knuth, Donald}, year=1986, title={L}}",
    );
    let doc = parse(
        "\\Citet{k} \\newcite{l} \\citeauthor{k} \\citeyear{k} \\citealp{k} \\citep*{k,l} \
         \\Textcite{k} \\autocite[p.~2]{l} \\nocite{k}\\nocite{*}end",
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    let rendered = render_text(
        &doc,
        &RenderOptions {
            bibliography: Some(&bib),
            ..Default::default()
        },
    );
    assert_eq!(
        rendered.replace('\u{a0}', " ").trim(),
        "Knuth, 1984 Lamport and Knuth, 1986 Knuth 1984 Knuth, 1984 (Knuth, 1984; Lamport and Knuth, 1986) \
         Knuth, 1984 (Lamport and Knuth, 1986, p. 2) end"
    );
}

#[test]
fn algorithms_are_numbered_and_kept_as_source() {
    let doc = parse(
        "\\begin{algorithm}[t]\n\\caption{Training loop}\\label{alg:train}\n\\begin{algorithmic}[1]\n\
         \\State $\\theta \\gets \\theta_0$\n\\For{$t = 1$ to $T$}\n  \\State update $\\theta$\n\\EndFor\n\
         \\end{algorithmic}\n\\end{algorithm}\n\
         \\begin{algorithm}\\caption{Second}\\label{alg:two}\\KwIn{x}\\end{algorithm}\n\
         Algorithms~\\ref{alg:train} and \\ref{alg:two}; \\cref{alg:two}.",
    );
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
    assert_eq!(doc.label_number("alg:train"), Some("1"));
    assert_eq!(doc.label_number("alg:two"), Some("2"));
    assert_eq!(
        md(&doc).replace('\u{a0}', " "),
        "**Algorithm 1:** Training loop\n\n```latex\n\\begin{algorithmic}[1]\n\
         \\State $\\theta \\gets \\theta_0$\n\\For{$t = 1$ to $T$}\n  \\State update $\\theta$\n\\EndFor\n\
         \\end{algorithmic}\n```\n\n**Algorithm 2:** Second\n\n```latex\n\\KwIn{x}\n```\n\n\
         Algorithms 1 and 2; algorithm 2.\n"
    );
}

#[test]
fn optional_arguments_of_title_author_footnote_and_caption_are_not_text() {
    let doc = parse(
        "\\title[Short]{The Long Title}\\author[A. L.]{Ada Lovelace}\n\\begin{document}\n\
         Body\\footnote[3]{A note.} text.\n\n\\caption[Short]{A loose caption.}\n\\end{document}",
    );
    assert_eq!(
        doc.title.as_deref().map(text_of).as_deref(),
        Some("The Long Title")
    );
    assert_eq!(doc.authors.len(), 1);
    assert_eq!(text_of(&doc.authors[0]), "Ada Lovelace");
    let rendered = md(&doc);
    assert!(
        !rendered.contains("Short") && !rendered.contains("A. L."),
        "{rendered}"
    );
    assert!(rendered.contains("Body[^1] text."), "{rendered}");
    assert!(rendered.contains("[^1]: A note."), "{rendered}");
    assert!(rendered.contains("\nA loose caption.\n"), "{rendered}");
    assert!(warnings(&doc).is_empty(), "{}", warnings(&doc));
}

#[test]
fn the_command_form_of_the_abstract_is_the_abstract() {
    let doc = parse(
        "\\title{T}\\abstract{We show $x$.}\n\\begin{document}\\maketitle Body.\\end{document}",
    );
    let abstract_text = doc
        .abstract_body
        .as_ref()
        .map(|blocks| {
            structure(&Document {
                blocks: blocks.clone(),
                ..Document::default()
            })
        })
        .unwrap_or_default();
    assert_eq!(abstract_text, ["paragraph We show x."]);
    assert!(
        md(&doc).contains("## Abstract\n\nWe show $x$."),
        "{}",
        md(&doc)
    );
}

#[test]
fn damaged_fixtures_never_panic_or_hang_with_math_expansion_and_conversion() {
    // The same truncations and mutations as above, through the paths the converters use:
    // macros expanded inside math, and `document::convert_bytes`.
    const ALPHABET: &[u8] = b"{}[]$\\%&#~^_*'\"`\n a1\\begin\\end\\newcommand\\fi";
    let options = Options {
        max_expansions: 2_000,
        max_depth: 32,
        expand_math_macros: true,
        ..Options::default()
    };
    let run = |src: &str| {
        let doc = parse_with(src, &options, None);
        let _ = render_markdown(&doc, &RenderOptions::default());
        let _ = render_html(&doc, &RenderOptions::default());
        let _ = crate::document::convert_bytes("tex", src.as_bytes());
    };
    let mut rng = Xorshift(0xD1B5_4A32_D192_ED03);
    for name in FIXTURES {
        let text = fixture_text(name);
        let len = text.len();
        for cut in (0..=len).step_by(11).chain(len.saturating_sub(30)..=len) {
            if text.is_char_boundary(cut) {
                run(&text[..cut]);
            }
        }
        for _ in 0..80 {
            let mut bytes = text.clone().into_bytes();
            for _ in 0..1 + rng.below(6) {
                if bytes.is_empty() {
                    break;
                }
                let at = rng.below(bytes.len());
                bytes[at] = ALPHABET[rng.below(ALPHABET.len())];
            }
            if let Ok(s) = String::from_utf8(bytes) {
                run(&s);
            }
        }
    }
    // Multi-byte text next to every construct that is read by byte offset.
    for src in [
        "\\newcommand{\\é}[1]{é#1é}$\\é{ü}\\é",
        "\\newcommand{\\a}[2][é]{#1ü#2}$\\a[ñ{]ø}{å}\\a ü$ \\a",
        "\\begin{alignat}{é} ü &= å \\end{alignat}",
        "\\begin{algorithm}\\caption[é]{ü}\\label{å}ø\\end{algorithm}",
        "\\begin{tikzpicture}[é={ü]}]ñ\\end{tikzpicture}",
        "\\begin{tikzpicture}[é",
        "\\SI{é}{\\ü^{å}} \\num{1eé} \\si{^}",
        "\\cmidrule(é \\multirow{é}{ü}{å} \\iffalse é",
        "$\\ensuremath{é",
        "$x^\\é$ \\newcommand{\\é}{ü}$x^\\é_\\é\\",
    ] {
        run(src);
    }
}

// ----- Pandoc layout (RenderOptions::pandoc), checked against Pandoc 3.1.3's output ---------------

fn pandoc_md(src: &str) -> String {
    let doc = parse(src);
    render_markdown(
        &doc,
        &RenderOptions {
            pandoc: true,
            ..Default::default()
        },
    )
}

#[test]
fn pandoc_layout_puts_front_matter_in_yaml_and_levels_follow_the_top_division() {
    let out = pandoc_md(
        "\\title{T}\\author{A \\and B}\\begin{document}\\maketitle\\section{S}\\label{s}\\paragraph{p}x\\end{document}",
    );
    assert!(
        out.starts_with("---\ntitle: \"T\"\nauthor:\n- \"A\"\n- \"B\"\n---\n\n# S {#s}\n"),
        "{out}"
    );
    assert!(out.contains("\n#### p\n"), "{out}");
    // Pandoc: \chapter is level 1, \section 2, \paragraph 5; \part above them shifts all by one.
    let book = pandoc_md("\\begin{document}\\chapter{C}\\section{S}\\paragraph{p}\\end{document}");
    assert!(
        book.contains("# C\n") && book.contains("## S\n") && book.contains("##### p\n"),
        "{book}"
    );
    let part = pandoc_md("\\begin{document}\\part{P}\\section{S}\\end{document}");
    assert!(part.contains("# P\n") && part.contains("### S\n"), "{part}");
}

#[test]
fn pandoc_layout_writes_pandoc_citations() {
    let out = pandoc_md(
        "\\begin{document}A \\cite{a} B \\cite{a,b} C \\citep[see][p.~4]{a,b} D \\citet{a} E \\citet{a,b} F \\citeyear{a} G \\nocite{z}\\end{document}",
    );
    let out = out.replace('\u{a0}', " ");
    assert!(
        out.contains("A [@a] B [@a; @b] C [see @a; @b p. 4] D @a E @a [@b] F [-@a] G"),
        "{out}"
    );
}

#[test]
fn pandoc_layout_keeps_description_lists_as_definition_lists() {
    let out = pandoc_md(
        "\\begin{document}\\begin{description}\\item[t] d\\end{description}\\end{document}",
    );
    assert!(out.contains("t\n:   d\n"), "{out}");
}
