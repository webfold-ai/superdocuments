//! `sd_convert FILE` prints the Markdown superdocuments reads from FILE (used by benches/pandoc-compat).
//! `sd_convert --csl FILE.bib` prints the CSL-JSON read from a BibTeX/BibLaTeX file (the bib track).
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().expect("usage: sd_convert [--csl] FILE");
    if first == "--csl" {
        let path = args.next().expect("usage: sd_convert --csl FILE.bib");
        let text = std::fs::read_to_string(&path).expect("read bib");
        let bib = superdocuments::bibtex::parse(&text);
        println!("{}", superdocuments::bibtex::csl_json(&bib));
        return;
    }
    let path = first;
    match superdocuments::document::convert_path(std::path::Path::new(&path)) {
        Ok(md) => {
            std::io::stdout().write_all(md.as_bytes()).expect("write stdout");
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
