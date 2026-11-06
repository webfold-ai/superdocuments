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
    // \and and \\ both separate entries: two authors and an affiliation line.
    let authors: Vec<String> = doc.authors.iter().map(|a| text_of(a)).collect();
    assert_eq!(authors, ["Ada Smith", "Bo Chen", "Example University"]);
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
        },
    );
    assert!(rendered.contains("(Knuth, 1984)"), "{rendered}");
    assert!(
        rendered.contains("Lovelace and Babbage, 1843")
            || rendered.contains("Lovelace and Babbage"),
        "{rendered}"
    );
}
