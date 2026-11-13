//! Renders the benchmark PNGs embedded in README.md (`docs/img/*.png`).
//!
//! Every number below is copied from a published result table; the comment on
//! each chart names the source file so a reviewer can check the copy by eye.
//! Nothing here is measured at run time. Regenerate with `cargo run` from this
//! directory.
//!
//! Sources (private repos; the fixture corpora are public):
//! - `webfold-bench/bench/results/ocr-vs-incumbents-2026-10-14.md` — the
//!   eight-fixture PDF-to-text comparison (superdocuments `786e8bd`, tesseract
//!   5.3.4, pdftotext 24.02.0, MarkItDown 0.1.9-era). PDF charts below.
//! - `webfold-bench/bench/STRUCTURED-CONVERSION.md` Tracks A, B, E/F.6, F.7 —
//!   in-process latency, throughput and subprocess comparisons. The tracks
//!   were measured through `webfold`, which re-exports superdocuments'
//!   `html` and `document` modules; that code lives in this crate.

use std::error::Error;

use charton::prelude::*;

/// CER, alphanumeric-lowercase, lower is better, from the 2026-10-14 run.
/// Rows with no score (tool produced no text, or no reference) are absent:
/// an absent bar means no output. "superdocuments" is the shipped route
/// (native first, OCR fallback); tesseract produced no text on rotated-15deg.
fn chart_pdf_cer() -> Result<(), Box<dyn Error>> {
    let rows: &[(&str, f64, &str)] = &[
        ("reportlab-invoice", 0.000, "superdocuments"),
        ("reportlab-invoice", 0.000, "pdftotext"),
        ("reportlab-invoice", 0.000, "markitdown"),
        ("reportlab-invoice", 0.000, "tesseract"),
        ("rotated-15deg", 0.000, "superdocuments"),
        ("rotated-15deg", 0.678, "pdftotext"),
        ("rotated-15deg", 0.264, "markitdown"),
        ("scanned-text", 0.000, "superdocuments"),
        ("scanned-text", 0.000, "tesseract"),
        ("scanned-fuzzy", 0.023, "superdocuments"),
        ("scanned-fuzzy", 0.628, "pdftotext"),
        ("scanned-fuzzy", 0.628, "markitdown"),
        ("scanned-fuzzy", 0.023, "tesseract"),
        ("mixed-native-and-scanned", 0.119, "superdocuments"),
        ("mixed-native-and-scanned", 0.212, "pdftotext"),
        ("mixed-native-and-scanned", 0.212, "markitdown"),
        ("mixed-native-and-scanned", 0.098, "tesseract"),
        ("two-column", 0.027, "superdocuments"),
        ("two-column", 0.240, "pdftotext"),
        ("two-column", 0.027, "markitdown"),
        ("two-column", 0.027, "tesseract"),
        ("borderless-table", 0.000, "superdocuments"),
        ("borderless-table", 0.000, "pdftotext"),
        ("borderless-table", 0.000, "markitdown"),
        ("borderless-table", 0.000, "tesseract"),
        ("reportlab-ascii85", 0.000, "superdocuments"),
        ("reportlab-ascii85", 0.000, "pdftotext"),
        ("reportlab-ascii85", 0.000, "markitdown"),
        ("reportlab-ascii85", 0.000, "tesseract"),
    ];
    let fixture: Vec<&str> = rows.iter().map(|r| r.0).collect();
    let cer: Vec<f64> = rows.iter().map(|r| r.1).collect();
    let tool: Vec<&str> = rows.iter().map(|r| r.2).collect();

    chart!(fixture, cer, tool)?
        .mark_bar()?
        .encode((
            alt::x("fixture"),
            alt::y("cer").with_label("character error rate (lower is better)"),
            alt::color("tool"),
        ))?
        .configure_theme(|t| t.with_x_tick_label_angle(-40.0))
        .with_title("PDF to text, accuracy by tool (2026-10-14, 8 fixtures)")
        .save("../../docs/img/bench-pdf-cer.png")?;
    Ok(())
}

/// Median wall time per conversion, milliseconds, log scale (0.1 ms to 2.6 s),
/// same run as the CER chart. superdocuments hits native speed (~0.1 ms) on
/// born-digital files and pays tesseract-class OCR cost on scans.
fn chart_pdf_latency() -> Result<(), Box<dyn Error>> {
    let rows: &[(&str, f64, &str)] = &[
        ("reportlab-invoice", 0.1, "superdocuments"),
        ("reportlab-invoice", 15.4, "pdftotext"),
        ("reportlab-invoice", 1548.0, "markitdown"),
        ("reportlab-invoice", 1086.0, "tesseract"),
        ("rotated-15deg", 0.1, "superdocuments"),
        ("rotated-15deg", 15.9, "pdftotext"),
        ("rotated-15deg", 1473.0, "markitdown"),
        ("rotated-15deg", 1018.0, "tesseract"),
        ("scanned-text", 1014.0, "superdocuments"),
        ("scanned-text", 16.2, "pdftotext"),
        ("scanned-text", 1478.0, "markitdown"),
        ("scanned-text", 967.0, "tesseract"),
        ("scanned-fuzzy", 1264.0, "superdocuments"),
        ("scanned-fuzzy", 16.7, "pdftotext"),
        ("scanned-fuzzy", 1479.0, "markitdown"),
        ("scanned-fuzzy", 1121.0, "tesseract"),
        ("mixed-native-and-scanned", 2606.0, "superdocuments"),
        ("mixed-native-and-scanned", 18.2, "pdftotext"),
        ("mixed-native-and-scanned", 1481.0, "markitdown"),
        ("mixed-native-and-scanned", 2164.0, "tesseract"),
        ("two-column", 2311.0, "superdocuments"),
        ("two-column", 16.4, "pdftotext"),
        ("two-column", 1781.0, "markitdown"),
        ("two-column", 2166.0, "tesseract"),
        ("borderless-table", 1378.0, "superdocuments"),
        ("borderless-table", 18.2, "pdftotext"),
        ("borderless-table", 1497.0, "markitdown"),
        ("borderless-table", 1201.0, "tesseract"),
        ("reportlab-ascii85", 1275.0, "superdocuments"),
        ("reportlab-ascii85", 19.1, "pdftotext"),
        ("reportlab-ascii85", 1450.0, "markitdown"),
        ("reportlab-ascii85", 1203.0, "tesseract"),
    ];
    let fixture: Vec<&str> = rows.iter().map(|r| r.0).collect();
    let ms: Vec<f64> = rows.iter().map(|r| r.1).collect();
    let tool: Vec<&str> = rows.iter().map(|r| r.2).collect();

    chart!(fixture, ms, tool)?
        .mark_bar()?
        .encode((
            alt::x("fixture"),
            alt::y("ms").with_scale(Scale::Log).with_label("median per call, ms, log scale (lower is better)"),
            alt::color("tool"),
        ))?
        .configure_theme(|t| t.with_x_tick_label_angle(-40.0))
        .with_title("PDF to text, latency by tool (2026-10-14, 8 fixtures)")
        .save("../../docs/img/bench-pdf-latency.png")?;
    Ok(())
}

/// Track A refresh (2026-09-30) and Track B (2026-09-24): hot median of
/// fixture medians, µs, CPU-pinned i9-14900KF. Track A values are candidate-B
/// range midpoints (4.127–4.251 µs; 17.506–17.889 µs for the control).
/// Sources: results/parity-refresh-2026-09-30.json, results/document-rust-cold-hot-2026-09-24.json.
fn chart_hot_latency() -> Result<(), Box<dyn Error>> {
    let series = [
        "HTML, 54 fixtures", "HTML, 54 fixtures", "Documents, 45 fixtures", "Documents, 45 fixtures",
    ];
    let microseconds = [4.2, 17.7, 27.7, 155.5];
    let tool = [
        "superdocuments", "firecrawl html-extractor", "superdocuments", "anydoc",
    ];

    chart!(series, microseconds, tool)?
        .mark_bar()?
        .encode((
            alt::x("series"),
            alt::y("microseconds").with_label("hot median per fixture, µs (lower is better)"),
            alt::color("tool"),
        ))?
        .with_title("In-process conversion, hot median (2026-09-24/30)")
        .save("../../docs/img/bench-hot-latency.png")?;
    Ok(())
}

/// Track F.7 (2026-09-25): single-call subprocess latency on the 17-fixture
/// corpus, log scale (5.8 ms to 49.8 s). pandoc 3.1.11.1, MarkItDown 0.1.8;
/// MarkItDown is dominated by Python startup in this shape.
/// Source: results/document-vs-incumbents-2026-09-25.json.
fn chart_subprocess_latency() -> Result<(), Box<dyn Error>> {
    let tool = ["webfold convert (superdocuments)", "pandoc", "markitdown"];
    let ms = [5.8, 37.0, 49_800.0];

    chart!(tool, ms)?
        .mark_bar()?
        .encode((
            alt::x("tool"),
            alt::y("ms").with_scale(Scale::Log).with_label("per call, ms, log scale (lower is better)"),
        ))?
        .with_title("Subprocess per call, CI shape (2026-09-25, 17 fixtures)")
        .save("../../docs/img/bench-subprocess-latency.png")?;
    Ok(())
}

/// Track E/F.6 (2026-09-25): sustained docs/sec, high_repeat workload,
/// no-cache conversion vs anydoc. Cached configs (15.5M-26.6M docs/sec at 64
/// workers) are off this linear scale; the README table carries them.
/// Source: results/document-throughput-2026-09-25.json.
fn chart_throughput() -> Result<(), Box<dyn Error>> {
    // Workers as categorical labels: a bar mark needs a Discrete X scale.
    let workers = ["1", "4", "16", "64"];
    let mut x = workers.to_vec();
    x.extend(workers.to_vec());
    let y: Vec<f64> = vec![
        24_302.0, 84_024.0, 127_832.0, 169_282.0, // superdocuments convert_bytes
        2_093.0, 7_484.0, 18_262.0, 23_948.0, // anydoc
    ];
    let tool: Vec<&str> = vec![
        "superdocuments", "superdocuments", "superdocuments", "superdocuments",
        "anydoc", "anydoc", "anydoc", "anydoc",
    ];

    chart!(x, y, tool)?
        .mark_bar()?
        .encode((
            alt::x("x").with_label("concurrent workers"),
            alt::y("y").with_label("docs/sec, sum across workers (higher is better)"),
            alt::color("tool"),
        ))?
        .with_title("Sustained throughput, no cache (2026-09-25)")
        .save("../../docs/img/bench-throughput.png")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    chart_pdf_cer()?;
    chart_pdf_latency()?;
    chart_hot_latency()?;
    chart_subprocess_latency()?;
    chart_throughput()?;
    println!("wrote bench-pdf-cer.png, bench-pdf-latency.png, bench-hot-latency.png, bench-subprocess-latency.png, bench-throughput.png into docs/img/");
    Ok(())
}
