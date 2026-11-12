//! `sd_convert FILE` prints the Markdown superdocuments reads from FILE (used by benches/pandoc-compat).
use std::io::Write;

fn main() {
    let path = std::env::args().nth(1).expect("usage: sd_convert FILE");
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
