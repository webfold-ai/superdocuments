//! `pdf_markdown OUT_DIR file.pdf...` (OUT_DIR `-`: print to stdout): native-only markdown (`extract_markdown`, never OCR) for each
//! file, written to OUT_DIR/<stem>.md; a failure writes an empty file and is reported on stderr.
//! Used to iterate on native extraction quality without rebuilding the webfold binary.
use std::{env, fs, path::Path};

fn main() {
    let mut args = env::args().skip(1);
    let out = args
        .next()
        .expect("usage: pdf_markdown OUT_DIR file.pdf...");
    let to_stdout = out == "-";
    if !to_stdout {
        fs::create_dir_all(&out).unwrap();
    }
    for f in args {
        let stem = Path::new(&f)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let md = fs::read(&f).and_then(|b| superdocuments::pdf::extract_markdown(&b));
        if to_stdout {
            match md {
                Ok(s) => print!("{s}"),
                Err(e) => {
                    eprintln!("{f}: {e}");
                    std::process::exit(1);
                }
            }
            continue;
        }
        match md {
            Ok(s) => fs::write(Path::new(&out).join(format!("{stem}_pg1_repeat1.md")), s).unwrap(),
            Err(e) => {
                eprintln!("{f}: {e}");
                fs::write(Path::new(&out).join(format!("{stem}_pg1_repeat1.md")), "").unwrap();
            }
        }
    }
}
