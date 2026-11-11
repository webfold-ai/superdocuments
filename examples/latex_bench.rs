//! Structured counts and timings for the LaTeX and BibTeX readers, used by
//! `benches/latex-vs-pandoc/run_benchmark.py`.
//!
//! Usage:
//!   latex_bench counts <file.tex>        JSON counts (structure, math, figures, citations, ...)
//!   latex_bench markdown <file.tex>      Markdown on stdout (for timing the CLI)
//!   latex_bench convert <file>           Markdown from `document::convert_path` (a .tex project,
//!                                        a .bib file, MathML, or an arXiv e-print archive)
//!   latex_bench plain <file.tex>         plain text on stdout
//!   latex_bench time <file.tex> <runs>   median in-process nanoseconds for parse + Markdown
//!   latex_bench bib-counts <file.bib>    JSON counts for BibTeX
//!   latex_bench bib-csl <file.bib>       CSL-JSON on stdout
//!   latex_bench bib-time <file.bib> <runs>

use std::time::Instant;

use serde_json::{json, Value};
use superdocuments::bibtex;
use superdocuments::latex::{self, Block, Inline, RenderOptions};

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    })
}

fn walk_inlines(inls: &[Inline], f: &mut dyn FnMut(&Inline)) {
    for inl in inls {
        f(inl);
        match inl {
            Inline::Emph(v)
            | Inline::Strong(v)
            | Inline::Mono(v)
            | Inline::SmallCaps(v)
            | Inline::Footnote(v) => walk_inlines(v, f),
            Inline::Link { text, .. } => walk_inlines(text, f),
            _ => {}
        }
    }
}

fn walk_blocks(blocks: &[Block], counts: &mut Counts) {
    for b in blocks {
        match b {
            Block::Section(s) => {
                counts.sections += 1;
                walk_inlines(&s.title, &mut |i| counts.inline(i));
            }
            Block::Paragraph(inl) => walk_inlines(inl, &mut |i| counts.inline(i)),
            Block::List(l) => {
                counts.lists += 1;
                counts.list_items += l.items.len();
                for item in &l.items {
                    walk_blocks(&item.blocks, counts);
                }
            }
            Block::Table(t) => {
                counts.tables += 1;
                if let Some(c) = &t.caption {
                    counts.tables_with_caption += 1;
                    walk_inlines(c, &mut |i| counts.inline(i));
                }
                for row in &t.rows {
                    for cell in &row.cells {
                        walk_inlines(&cell.content, &mut |i| counts.inline(i));
                    }
                }
            }
            Block::Math(m) => {
                counts.display_math += 1;
                counts.display_rows += m.rows.len();
            }
            Block::Verbatim(_) => counts.verbatim += 1,
            Block::Figure(f) => {
                counts.figures_nested += 1;
                count_figure(f, counts);
            }
            Block::Theorem(t) => walk_blocks(&t.body, counts),
            Block::Quote(v) => walk_blocks(v, counts),
        }
    }
}

fn count_figure(f: &latex::Figure, counts: &mut Counts) {
    if f.caption.is_some() {
        counts.figures_with_caption += 1;
    }
    counts
        .figure_captions
        .push(f.caption_text.clone().unwrap_or_default());
    if let Some(c) = &f.caption {
        walk_inlines(c, &mut |i| counts.inline(i));
    }
    counts.image_paths += f.image_paths.len();
    for sub in &f.subfigures {
        count_figure(sub, counts);
    }
}

#[derive(Default)]
struct Counts {
    sections: usize,
    display_math: usize,
    display_rows: usize,
    inline_math: usize,
    lists: usize,
    list_items: usize,
    tables: usize,
    tables_with_caption: usize,
    verbatim: usize,
    figures_nested: usize,
    figures_with_caption: usize,
    figure_captions: Vec<String>,
    image_paths: usize,
    cite_commands: usize,
    cite_keys: Vec<String>,
    refs: usize,
    footnotes: usize,
    links: usize,
}

impl Counts {
    fn inline(&mut self, inl: &Inline) {
        match inl {
            Inline::Math { display: false, .. } => self.inline_math += 1,
            Inline::Cite(c) => {
                self.cite_commands += 1;
                self.cite_keys.extend(c.keys.iter().cloned());
            }
            Inline::Ref(_) => self.refs += 1,
            Inline::Footnote(_) => self.footnotes += 1,
            Inline::Link { .. } => self.links += 1,
            _ => {}
        }
    }
}

fn latex_counts(src: &str) -> Value {
    let doc = latex::parse(src);
    let mut counts = Counts::default();
    walk_blocks(&doc.blocks, &mut counts);
    let figs = latex::figures(&doc);
    let bib_keys = latex::resolve_citations(&doc, None);
    json!({
        "title": doc.title.as_ref().map(|t| latex::plain_text(t)),
        "authors": doc.authors.len(),
        "abstract": doc.abstract_body.is_some(),
        "sections": counts.sections,
        "display_math": counts.display_math,
        "display_rows": counts.display_rows,
        "inline_math": counts.inline_math,
        "lists": counts.lists,
        "list_items": counts.list_items,
        "tables": counts.tables,
        "tables_with_caption": counts.tables_with_caption,
        "verbatim": counts.verbatim,
        "figures_top_level": figs.len(),
        "figures_with_caption": counts.figures_with_caption,
        "figure_captions": counts.figure_captions,
        "image_paths_in_figures": counts.image_paths,
        "cite_commands": counts.cite_commands,
        "cite_keys": counts.cite_keys,
        "cite_keys_unresolved": bib_keys.iter().filter(|k| k.source == latex::CitationSource::Unresolved).count(),
        "refs": counts.refs,
        "unresolved_refs": doc.unresolved_references().len(),
        "footnotes": counts.footnotes,
        "links": counts.links,
        "bibitems": doc.bibitems.len(),
        "labels": doc.labels.len(),
        "warnings": doc.warnings.len(),
    })
}

fn bib_counts(src: &str) -> Value {
    let bib = bibtex::parse(src);
    let entries: Vec<Value> = bib
        .entries
        .iter()
        .map(|e| {
            let authors = e.authors();
            json!({
                "key": e.key,
                "type": e.entry_type,
                "authors": authors.names.len(),
                "first_family": authors.names.first().map(|n| n.family.clone()),
                "year": bib
                    .resolved_value(e, "year")
                    .and_then(|y| bibtex::parse_year(&y))
                    .or_else(|| e.year()),
                "pages": e.pages().len(),
                "doi": e.doi(),
                "fields": e.fields.len(),
            })
        })
        .collect();
    json!({
        "entries": bib.entries.len(),
        "strings": bib.strings.len(),
        "preambles": bib.preambles.len(),
        "comments": bib.comments.len(),
        "warnings": bib.warnings.len(),
        "per_entry": entries,
    })
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, file) = match (args.first(), args.get(1)) {
        (Some(c), Some(f)) => (c.as_str(), f.as_str()),
        _ => {
            eprintln!("usage: latex_bench <counts|markdown|plain|time|bib-counts|bib-csl|bib-time> <file> [runs]");
            std::process::exit(2);
        }
    };
    if cmd == "convert" {
        // Through the conversion API: project files and bibliography from the file's own
        // directory, e-print archives by content, macros expanded inside math.
        match superdocuments::document::convert_path(std::path::Path::new(file)) {
            Ok(markdown) => println!("{markdown}"),
            Err(e) => {
                eprintln!("cannot convert {file}: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let src = read(file);
    match cmd {
        "counts" => println!(
            "{}",
            serde_json::to_string_pretty(&latex_counts(&src)).unwrap_or_default()
        ),
        "markdown" => {
            let doc = latex::parse(&src);
            print!(
                "{}",
                latex::render_markdown(&doc, &RenderOptions::default())
            );
        }
        "plain" => {
            let doc = latex::parse(&src);
            print!("{}", latex::render_text(&doc, &RenderOptions::default()));
        }
        "time" => {
            let runs: usize = args.get(2).and_then(|r| r.parse().ok()).unwrap_or(20);
            let mut samples = Vec::with_capacity(runs);
            for _ in 0..runs {
                let t = Instant::now();
                let doc = latex::parse(&src);
                let out = latex::render_markdown(&doc, &RenderOptions::default());
                std::hint::black_box(out);
                samples.push(t.elapsed().as_nanos());
            }
            println!("{}", median(samples));
        }
        "bib-counts" => println!(
            "{}",
            serde_json::to_string_pretty(&bib_counts(&src)).unwrap_or_default()
        ),
        "bib-csl" => {
            let bib = bibtex::parse(&src);
            println!(
                "{}",
                serde_json::to_string_pretty(&bibtex::csl_json(&bib)).unwrap_or_default()
            );
        }
        "bib-time" => {
            let runs: usize = args.get(2).and_then(|r| r.parse().ok()).unwrap_or(20);
            let mut samples = Vec::with_capacity(runs);
            for _ in 0..runs {
                let t = Instant::now();
                let bib = bibtex::parse(&src);
                std::hint::black_box(bibtex::csl_json(&bib));
                samples.push(t.elapsed().as_nanos());
            }
            println!("{}", median(samples));
        }
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    }
}
