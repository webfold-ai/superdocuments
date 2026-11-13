//! `cite_run --bib FILE --csl STYLE.csl DOC.md` formats the citations of a Pandoc Markdown document
//! and its bibliography, and prints them as JSON (`citations`, `bibliography`) in the comparison
//! form used by benches/pandoc-compat/cite (formatting marked with private-use characters).
use superdocuments::citeproc::{self, markdown, Processor, Style};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let bib = get("--bib").expect("--bib FILE");
    let csl = get("--csl").expect("--csl FILE");
    let doc = args.last().expect("DOC.md").clone();
    let bib_text = std::fs::read_to_string(&bib).expect("read bib");
    let refs = if bib.ends_with(".json") {
        citeproc::references_from_json(&bib_text).expect("csl-json")
    } else {
        let parsed = superdocuments::bibtex::parse(&bib_text);
        citeproc::references_from_json(&superdocuments::bibtex::csl_json_pandoc(&parsed, true).to_string()).expect("csl-json")
    };
    let style = Style::parse(&std::fs::read_to_string(&csl).expect("read csl")).expect("parse csl");
    let md = std::fs::read_to_string(&doc).expect("read doc");
    let p = Processor::new(style, refs).with_pandoc_names(args.iter().any(|a| a == "--pandoc-names"));
    let found = markdown::parse_citations(&md);
    let cites: Vec<_> = found.iter().map(|f| f.citation.clone()).collect();
    let done = p.process(&cites, &[]);
    let (c, b) = done.comparison(p.style());
    println!("{}", serde_json::json!({ "citations": c, "bibliography": b }));
}
