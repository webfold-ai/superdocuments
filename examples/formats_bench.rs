//! Conversion driver for `webfold-bench/tests/formats-fidelity-v1`: Markdown output and in-process
//! timing for the document formats (RTF, DOCX, PPTX, XLSX, ODF, EPUB, CSV).
//!
//! Usage:
//!   formats_bench md <file>              Markdown on stdout (extension picks the reader)
//!   formats_bench time <file> <runs>     one JSON line: per-run in-process nanoseconds of
//!                                        `convert_bytes` after three warm-up runs, bytes read
//!                                        from disk once and output length
//!   formats_bench batch <list> <runs>    the same for every path in <list> (one per line),
//!                                        one JSON line per file, all in one process

use std::time::Instant;

use superdocuments::document::convert_bytes;

fn ext_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string()
}

fn time_one(path: &str, runs: usize) -> String {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return format!("{{\"file\":{path:?},\"error\":{:?}}}", e.to_string()),
    };
    let ext = ext_of(path);
    let mut out_len = 0usize;
    for _ in 0..3 {
        match convert_bytes(&ext, &data) {
            Ok(s) => out_len = s.len(),
            Err(e) => return format!("{{\"file\":{path:?},\"error\":{e:?}}}"),
        }
    }
    let mut ns: Vec<u128> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        let r = convert_bytes(&ext, &data);
        let d = t.elapsed().as_nanos();
        std::hint::black_box(&r);
        ns.push(d);
    }
    let list: Vec<String> = ns.iter().map(|n| n.to_string()).collect();
    format!(
        "{{\"file\":{path:?},\"bytes\":{},\"out_bytes\":{out_len},\"ns\":[{}]}}",
        data.len(),
        list.join(",")
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("md") if args.len() >= 3 => {
            let data = std::fs::read(&args[2]).unwrap_or_else(|e| {
                eprintln!("cannot read {}: {e}", args[2]);
                std::process::exit(1);
            });
            match convert_bytes(&ext_of(&args[2]), &data) {
                Ok(s) => print!("{s}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            }
        }
        Some("time") if args.len() >= 4 => {
            println!("{}", time_one(&args[2], args[3].parse().unwrap_or(7)));
        }
        Some("batch") if args.len() >= 4 => {
            let list = std::fs::read_to_string(&args[2]).expect("list file");
            let runs: usize = args[3].parse().unwrap_or(7);
            for line in list.lines().filter(|l| !l.is_empty()) {
                println!("{}", time_one(line, runs));
            }
        }
        _ => {
            eprintln!("usage: formats_bench md <file> | time <file> <runs> | batch <list> <runs>");
            std::process::exit(64);
        }
    }
}
