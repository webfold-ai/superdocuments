//! Renders the three benchmark SVGs embedded in README.md (`docs/img/*.svg`).
//!
//! Every number below is copied from a published result table in
//! `webfold-bench/bench/STRUCTURED-CONVERSION.md` (private repo; the fixture
//! corpora are public). The comment on each constant names the track and the
//! result file the number comes from, so a reviewer can check the copy by eye.
//! Nothing here is measured at run time: re-running the bench does not happen
//! in this crate. Regenerate with `cargo run` from this directory.
//!
//! Attribution note: the tracks were measured through `webfold`, which
//! re-exports superdocuments' `html` and `document` modules; superdocuments
//! is where that code lives now (superdocuments-bench README, "How this
//! relates to existing bench work"). Measurement dates are kept per chart.

use std::error::Error;

use charton::prelude::*;

/// Track A refresh (2026-09-30) and Track B (2026-09-24): hot median of
/// fixture medians, in microseconds, CPU-pinned i9-14900KF, logical CPU 8.
/// Track A values are the paired A/B candidate-B range midpoints
/// (4.127–4.251 µs; 17.506–17.889 µs for the Firecrawl control).
/// Sources: results/parity-refresh-2026-09-30.json (Track A),
/// results/document-rust-cold-hot-2026-09-24.json (Track B).
fn chart_hot_latency() -> Result<(), Box<dyn Error>> {
    let series = [
        "HTML (54 fixtures)", "HTML (54 fixtures)", //
        "Documents (45 fixtures)", "Documents (45 fixtures)",
    ];
    let microseconds = [4.2, 17.7, 27.7, 155.5];
    let tool = [
        "superdocuments (via webfold)", "Firecrawl html-extractor", //
        "superdocuments (via webfold)", "anydoc",
    ];

    chart!(series, microseconds, tool)?
        .mark_bar()?
        .encode((
            alt::x("series"),
            alt::y("microseconds").with_label("hot median per fixture (µs)"),
            alt::color("tool"),
        ))?
        .with_title("In-process conversion, hot median latency (lower is better)")
        .save("../../docs/img/bench-hot-latency.svg")?;
    Ok(())
}

/// Track D (2026-09-25): DocumentCache warm-path speedup over convert_bytes,
/// median across the firecrawl/anydoc streaming fixtures of each format
/// family. Per-fixture range 3.5x-11.8x (CSV 3.5-4.9x, XLSX 3.7-4.3x,
/// DOCX 3.7-11.8x).
/// Source: results/document-cache-cold-hot-2026-09-25.json.
fn chart_cache_speedup() -> Result<(), Box<dyn Error>> {
    let format = ["CSV", "XLSX", "DOCX"];
    let speedup = [3.6, 4.0, 5.1];

    chart!(format, speedup)?
        .mark_bar()?
        .configure_bar(|b| b.with_stroke("black").with_stroke_width(1.0).with_width(0.5))
        .encode((
            alt::x("format"),
            alt::y("speedup").with_label("speedup over convert_bytes (x)"),
        ))?
        .with_title("Repeat-read cache, median warm-path speedup (higher is better)")
        .save("../../docs/img/bench-cache-speedup.svg")?;
    Ok(())
}

/// Track E/F.6 (2026-09-25): sustained throughput, docs/sec summed across
/// workers, high_repeat workload, no-cache convert_bytes vs anydoc. The
/// fold/cache configs (15.5M-26.6M docs/sec at 64 workers) are off this
/// linear scale; the README table carries them with their caveats.
/// Source: results/document-throughput-2026-09-25.json.
fn chart_throughput() -> Result<(), Box<dyn Error>> {
    // Workers as categorical labels: a bar mark needs a Discrete X scale.
    let workers = ["1", "4", "16", "64"];
    let mut x = workers.to_vec();
    x.extend(workers.to_vec());
    let y: Vec<f64> = vec![
        24_302.0, 84_024.0, 127_832.0, 169_282.0, // webfold convert_bytes
        2_093.0, 7_484.0, 18_262.0, 23_948.0, // anydoc
    ];
    let tool: Vec<&str> = vec![
        "superdocuments (via webfold)", "superdocuments (via webfold)", //
        "superdocuments (via webfold)", "superdocuments (via webfold)", "anydoc",
        "anydoc", "anydoc", "anydoc",
    ];

    chart!(x, y, tool)?
        .mark_bar()?
        .encode((
            alt::x("x").with_label("concurrent workers"),
            alt::y("y").with_label("docs/sec (sum across workers)"),
            alt::color("tool"),
        ))?
        .with_title("Sustained throughput, no-cache conversion (higher is better)")
        .save("../../docs/img/bench-throughput.svg")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    chart_hot_latency()?;
    chart_cache_speedup()?;
    chart_throughput()?;
    println!("wrote bench-hot-latency.svg, bench-cache-speedup.svg, bench-throughput.svg into docs/img/");
    Ok(())
}
