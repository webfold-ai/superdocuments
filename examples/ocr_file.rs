//! OCR a PDF through the shipped fallback and print the Markdown, with the limits on the command line.
//!
//!   cargo run --release --example ocr_file -- FILE.pdf [--lang chi_sim+eng] [--pages 300] [--dim 1800] [--timeout 3600] [--workers 4]
use std::time::{Duration, Instant};
use superdocuments::pdf::{extract_file_with_ocr_options, OcrOptions};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: ocr_file FILE.pdf [--lang L] [--pages N] [--dim PX] [--timeout SECS] [--workers N]");
        std::process::exit(2);
    };
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let mut o = OcrOptions::default();
    if let Some(v) = flag("--lang") {
        o.languages = v.clone();
    }
    if let Some(v) = flag("--pages").and_then(|v| v.parse().ok()) {
        o.max_pages = v;
    }
    if let Some(v) = flag("--dim").and_then(|v| v.parse().ok()) {
        o.max_image_dimension = v;
    }
    if let Some(v) = flag("--timeout").and_then(|v| v.parse().ok()) {
        o.timeout = Duration::from_secs(v);
    }
    if let Some(v) = flag("--workers").and_then(|v| v.parse().ok()) {
        o.workers = v;
    }
    let started = Instant::now();
    match extract_file_with_ocr_options(path, &o) {
        Ok(md) => {
            eprintln!("{} bytes in {:.1}s", md.len(), started.elapsed().as_secs_f64());
            println!("{md}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
