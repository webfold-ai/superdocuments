//! Page-level OCR routing: a page whose images dominate it and whose only native text is a stamp
//! is read by OCR, not by its stamp.
//!
//! The Federal Register of Legislation's scanned ordinances (Census and Statistics Ordinance 1961
//! and two amending ordinances, 10/5/3 pages) have one full-page image per page and a single line
//! of real text per page, "Federal Register of Legislative Instruments F2006B00…". The native
//! route used to return that line and nothing else; OCR only ran for pages with no text at all.
//!
//! The tests build small PDFs in memory. The end-to-end ones put `pdftoppm` and `tesseract`
//! stand-ins on PATH, so what is checked is the routing (which pages are rendered, whose text
//! reaches the output) and needs no OCR engine. `real_scan_fixture_is_read_by_ocr` uses the real
//! tools and the real Register page when they are installed.

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
mod common;
use common::*;
use superdocuments::pdf::{self, OcrOptions};

/// PATH is process-wide: tests that replace it take this lock.
static PATH_LOCK: Mutex<()> = Mutex::new(());

// ── a stand-in OCR toolchain ────────────────────────────────────────────────────────────────

struct Toolchain {
    dir: PathBuf,
    log: PathBuf,
    saved_path: Option<std::ffi::OsString>,
}

impl Toolchain {
    /// `pdftoppm` writes one tiny file per page; `tesseract` answers "OCR TEXT page N" for page N.
    fn install(name: &str, page_count: u32) -> Self {
        let dir = std::env::temp_dir().join(format!("sd-ocr-shim-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        let write = |file: &str, body: String| {
            let path = dir.join(file);
            std::fs::File::create(&path)
                .unwrap()
                .write_all(body.as_bytes())
                .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // pdftoppm -f F -l L -scale-to D -png INPUT PREFIX
        write(
            "pdftoppm",
            format!(
                "#!/bin/sh\nf=$2; l=$4; in=$8; prefix=$9\necho \"render $f-$l\" >> '{log}'\n\
                 n=$f\nwhile [ $n -le $l ] && [ $n -le {page_count} ]; do printf x > \"$prefix-$n.png\"; n=$((n+1)); done\n",
                log = log.display()
            ),
        );
        // tesseract IMAGE BASE -l LANG --psm 3
        write(
            "tesseract",
            format!(
                "#!/bin/sh\nimg=$1; base=$2\nn=${{img##*-}}; n=${{n%.png}}; n=$((n+0))\n\
                 echo \"ocr $n omp=$OMP_THREAD_LIMIT\" >> '{log}'\nprintf 'OCR TEXT page %s' \"$n\" > \"$base.txt\"\n",
                log = log.display()
            ),
        );
        let saved_path = std::env::var_os("PATH");
        let mut path = dir.clone().into_os_string();
        path.push(":");
        path.push(saved_path.clone().unwrap_or_default());
        std::env::set_var("PATH", path);
        Toolchain {
            dir,
            log,
            saved_path,
        }
    }

    fn pages_recognised(&self) -> Vec<u32> {
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        let mut pages: Vec<u32> = log
            .lines()
            .filter_map(|l| l.strip_prefix("ocr "))
            .filter_map(|n| n.split(' ').next()?.parse().ok())
            .collect();
        pages.sort_unstable();
        pages
    }
}

impl Drop for Toolchain {
    fn drop(&mut self) {
        match &self.saved_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write_pdf(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sd-routing-{}-{name}.pdf", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

// ── end to end: existing entry point, observable output ─────────────────────────────────────

#[test]
fn stamp_only_scan_pages_are_read_by_ocr() {
    let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Three full-page scans, each carrying only the Register's stamp (the 1981 ordinance's layout).
    let pdf = build(&[
        page(FULL_PAGE, Some(STAMP)),
        page(FULL_PAGE, Some(STAMP)),
        page(FULL_PAGE, Some(STAMP)),
    ]);
    let path = write_pdf("stamp-scan", &pdf);
    let tools = Toolchain::install("stamp-scan", 3);
    let md = pdf::extract_file_with_ocr_options(&path, &OcrOptions::default()).unwrap();
    for n in 1..=3 {
        assert!(
            md.contains(&format!("OCR TEXT page {n}")),
            "page {n} was not OCRed; output is {md:?}"
        );
    }
    assert_eq!(tools.pages_recognised(), vec![1, 2, 3]);
    // the stamp is kept (it is the only identifier on the page), after the page's own text
    assert!(md.contains(STAMP), "{md:?}");
    assert!(md.find("OCR TEXT page 1").unwrap() < md.find(STAMP).unwrap());
    let _ = std::fs::remove_file(path);
}

#[test]
fn tesseract_runs_with_one_openmp_thread_unless_the_caller_chose() {
    // Tesseract's OpenMP threads spin: on a busy host one page took minutes (the stamp fixture: 1.7 s with
    // one thread, over twelve minutes of CPU without). The caller's own OMP_THREAD_LIMIT is respected.
    let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pdf = build(&[page(FULL_PAGE, None)]);
    let path = write_pdf("omp", &pdf);
    let saved = std::env::var_os("OMP_THREAD_LIMIT");
    std::env::remove_var("OMP_THREAD_LIMIT");
    let tools = Toolchain::install("omp", 1);
    pdf::extract_file_with_ocr_options(&path, &OcrOptions::default()).unwrap();
    let log = std::fs::read_to_string(&tools.log).unwrap();
    assert!(log.contains("ocr 1 omp=1"), "unset: {log}");
    std::env::set_var("OMP_THREAD_LIMIT", "3");
    pdf::extract_file_with_ocr_options(&path, &OcrOptions::default()).unwrap();
    let log = std::fs::read_to_string(&tools.log).unwrap();
    assert!(log.contains("ocr 1 omp=3"), "caller's value kept: {log}");
    match saved {
        Some(v) => std::env::set_var("OMP_THREAD_LIMIT", v),
        None => std::env::remove_var("OMP_THREAD_LIMIT"),
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn mixed_document_ocrs_only_the_scanned_pages() {
    let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let born_digital = dense_text();
    let pdf = build(&[
        page(None, Some(&born_digital)),
        page(FULL_PAGE, Some(STAMP)),
        page(None, Some(&born_digital)),
        page(FULL_PAGE, None), // image only, no text at all
        page(Some((60, 60)), Some("Crest and short title")),
    ]);
    let path = write_pdf("mixed", &pdf);
    let tools = Toolchain::install("mixed", 5);
    let md = pdf::extract_file_with_ocr_options(&path, &OcrOptions::default()).unwrap();
    assert_eq!(tools.pages_recognised(), vec![2, 4], "{md:?}");
    assert!(md.contains("OCR TEXT page 2") && md.contains("OCR TEXT page 4"), "{md:?}");
    assert_eq!(md.matches("quick brown fox").count(), 10, "native pages 1 and 3 kept: {md:?}");
    assert!(md.contains("Crest and short title") && !md.contains("OCR TEXT page 5"), "{md:?}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn born_digital_document_never_starts_ocr() {
    let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let text = dense_text();
    // a crest, a short title, and a full-page scan that carries the publisher's complete text layer
    let pdf = build(&[
        page(Some((60, 60)), Some("Cover title")),
        page(FULL_PAGE, Some(&text)),
        page(None, Some(&text)),
    ]);
    let path = write_pdf("born-digital", &pdf);
    let tools = Toolchain::install("born-digital", 3);
    let md = pdf::extract_file_with_ocr_options(&path, &OcrOptions::default()).unwrap();
    assert!(tools.pages_recognised().is_empty(), "OCR ran: {md:?}");
    assert!(!md.contains("OCR TEXT"), "{md:?}");
    let _ = std::fs::remove_file(path);
}

// ── the real Register page, with the real tools ─────────────────────────────────────────────

fn have(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .is_ok()
}

#[test]
fn real_scan_fixture_is_read_by_ocr() {
    let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !(have("tesseract") && have("pdftoppm")) {
        eprintln!("skipped: pdftoppm and tesseract are not installed");
        return;
    }
    // Census and Statistics (Amendment) Ordinance 1981, page 1 of the Register's PDF: one 3310x4680
    // CCITT scan and the footer stamp as the only text. See tests/fixtures/au-register/README.md.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/au-register/census-amendment-ordinance-1981-p1.pdf");
    let native = pdf::extract_file(&fixture).unwrap();
    assert!(native.contains("F2006B00784") && !native.contains("Householder"), "{native:?}");
    let options = OcrOptions {
        timeout: std::time::Duration::from_secs(900),
        ..OcrOptions::default()
    };
    let md = pdf::extract_file_with_ocr_options(&fixture, &options).unwrap();
    for word in ["Census", "Statistics", "Ordinance", "Householder"] {
        assert!(md.contains(word), "OCR text lacks {word:?}: {md:?}");
    }
    assert!(md.contains("F2006B00784"), "stamp kept: {md:?}");
}
