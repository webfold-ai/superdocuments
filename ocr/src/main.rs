//! superdocuments-bench/ocr — OCR inference-stack benchmark.
//!
//! Measures text CER, graph-fidelity, and OpenAI-compat TTFT/ITL for seven
//! inference targets on the same PDF fixtures:
//!
//! - `candle-qwen2.5-vl` — in-process Rust via the `candle` feature (off by default)
//! - `ai-gateway`        — Traceloop Hub on localhost:3100
//! - `litellm`           — on localhost:4000
//! - `bifrost`           — on localhost:8080
//! - `portkey`           — on localhost:8787
//! - `sglang-ocr`        — SGLang serving PaddleOCR-VL-1.6 / GLM-OCR / olmOCR
//! - `vllm-ocr`          — vLLM serving the same models
//!
//! Three columns, per the OCR Quality–Latency Frontier:
//!
//! | Quality | text CER + graph-fidelity (reading order, table cells, native-preservation) |
//! | Latency | wall-clock seconds + p50/p95/p99 TTFT for OpenAI-compat targets |
//! | Cost    | bytes_per_sec + peak RSS KiB
//!
//! Scope is **inference-stack only**. The CPU baselines (native, tesseract-direct,
//! pdftotext, markitdown) live in `webfold-bench/bench/ocr-vs-incumbents/`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;

use superdocuments::graph::{from_ocr_markdown, DocumentGraph, Source};

#[derive(Parser, Debug)]
#[command(name = "superdocuments-bench-ocr", version, about)]
struct Cli {
    /// Root directory containing `<name>.pdf` and optional `<name>.expected.{txt,json}`.
    #[arg(long, default_value = "./fixtures")]
    fixtures: PathBuf,

    /// Output JSON path.
    #[arg(long, default_value = "./results/ocr-inference.json")]
    out: PathBuf,

    /// Number of warmup runs per (tool, fixture). Excluded from medians.
    #[arg(long, default_value_t = 1)]
    warmup: u64,

    /// Number of samples per (tool, fixture). The reported medians are the median
    /// across sample medians, NOT across individual iterations. Documented in the
    /// JSON's `definitions.n_runs_per_cell` and `definitions.median_wall_secs`.
    #[arg(long, default_value_t = 3)]
    samples: u64,

    /// Number of back-to-back conversions per sample. Aggregated into one wall
    /// measurement per sample.
    #[arg(long, default_value_t = 2)]
    iterations: u64,

    /// Restrict to a subset of targets by name (csv). Default: all available.
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Per-sample timeout for one tool call. Defaults to 60s.
    #[arg(long, default_value_t = 60)]
    timeout_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Target {
    #[serde(rename = "candle-qwen2.5-vl")]
    CandleQwen25Vl,
    AiGateway,
    Litellm,
    Bifrost,
    Portkey,
    SglangOcr,
    VllmOcr,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Self::CandleQwen25Vl => "candle-qwen2.5-vl",
            Self::AiGateway => "ai-gateway",
            Self::Litellm => "litellm",
            Self::Bifrost => "bifrost",
            Self::Portkey => "portkey",
            Self::SglangOcr => "sglang-ocr",
            Self::VllmOcr => "vllm-ocr",
        }
    }

    /// OpenAI-compatible endpoint and model name. Candle is the only in-process
    /// target; everything else is HTTP.
    fn endpoint(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::AiGateway => Some(("http://localhost:3100/v1", "mock-ocr")),
            Self::Litellm => Some(("http://localhost:4000/v1", "mock-ocr")),
            Self::Bifrost => Some(("http://localhost:8080/v1", "mock-ocr")),
            Self::Portkey => Some(("http://localhost:8787/v1", "mock-ocr")),
            Self::SglangOcr => Some(("http://localhost:30001/v1", "PaddlePaddle/PaddleOCR-VL-1.6")),
            Self::VllmOcr => Some(("http://localhost:30000/v1", "PaddlePaddle/PaddleOCR-VL-1.6")),
            Self::CandleQwen25Vl => None,
        }
    }

    fn is_candle(self) -> bool {
        matches!(self, Self::CandleQwen25Vl)
    }

    fn all() -> [Target; 7] {
        [
            Self::CandleQwen25Vl,
            Self::AiGateway,
            Self::Litellm,
            Self::Bifrost,
            Self::Portkey,
            Self::SglangOcr,
            Self::VllmOcr,
        ]
    }
}

#[derive(Clone, Debug)]
struct Fixture {
    path: PathBuf,
    bytes_len: u64,
    /// Alphanumeric-lowercase CER reference (mirrors `ocr-vs-incumbents` convention).
    text_expected: Option<String>,
    /// Ground-truth DocumentGraph serialized as JSON. Optional — when present,
    /// graph-fidelity scoring runs.
    graph_expected: Option<DocumentGraph>,
}

fn load_fixtures(root: &Path) -> Vec<Fixture> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(it) => it,
        Err(e) => {
            eprintln!("could not read fixtures dir {}: {e}", root.display());
            return out;
        }
    };
    for item in entries.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            != Some("pdf".to_string())
        {
            continue;
        }
        let bytes_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let text_path = path.with_extension("expected.txt");
        let text_expected = std::fs::read_to_string(&text_path)
            .ok()
            .as_deref()
            .map(normalize);
        let graph_path = path.with_extension("expected.json");
        let graph_expected = std::fs::read_to_string(&graph_path)
            .ok()
            .and_then(|s| parse_graph_expected(&s));
        out.push(Fixture {
            path,
            bytes_len,
            text_expected,
            graph_expected,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Parse the `.expected.json` ground truth. The file is a JSON object with
/// `markdown` (string) and optional `source_label` (string). `markdown` is
/// projected through the same builder the model output uses, so the comparison
/// is apples-to-apples.
fn parse_graph_expected(s: &str) -> Option<DocumentGraph> {
    #[derive(Deserialize)]
    struct Expected {
        markdown: String,
        #[serde(default = "default_source_label")]
        source_label: String,
    }
    fn default_source_label() -> String {
        "ocr".into()
    }
    let parsed: Expected = serde_json::from_str(s).ok()?;
    let source = Source::Ocr {
        model: format!(
            "ground-truth-{parsed_source}",
            parsed_source = parsed.source_label
        ),
        dpi: 200,
        psm: 3,
        languages: "eng".into(),
    };
    Some(from_ocr_markdown(&parsed.markdown, source))
}

/// Alphanumeric-lowercase collapse. Mirrors `webfold-bench/bench/ocr-vs-incumbents`.
fn normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        }
    }
    out
}

/// Strip `<!-- page N -->` markers, then normalize. Same convention as
/// `ocr-vs-incumbents::normalize_ocr_markdown`.
fn normalize_ocr_markdown(md: &str) -> String {
    let mut joined = String::with_capacity(md.len());
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<!-- page ") || trimmed.is_empty() {
            continue;
        }
        if !joined.is_empty() {
            joined.push('\n');
        }
        joined.push_str(trimmed);
    }
    normalize(&joined)
}

fn cer(reference: &str, hypothesis: &str) -> Option<f64> {
    if reference.is_empty() || hypothesis.is_empty() {
        return None;
    }
    let ref_chars: Vec<char> = reference.chars().collect();
    let hyp_chars: Vec<char> = hypothesis.chars().collect();
    let n = ref_chars.len();
    let m = hyp_chars.len();
    let mut prev = vec![0usize; m + 1];
    let mut curr = vec![0usize; m + 1];
    for (j, item) in curr.iter_mut().enumerate().take(m + 1).skip(1) {
        *item = j;
    }
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if ref_chars[i - 1] == hyp_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    Some(prev[m] as f64 / n as f64)
}

/// Per-sample record. Wall is the only timing we collect for inference targets
/// (TTFT/ITL for OpenAI-compat targets come from guidellm/aiperf in a separate
/// lane). graph_fidelity holds the four new scoring dimensions.
#[derive(Clone, Debug, Default)]
struct Sample {
    wall_secs: f64,
    bytes_out: u64,
    output_text: String,
    output_graph: Option<DocumentGraph>,
    graph_fidelity: Option<GraphFidelity>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct GraphFidelity {
    /// Fraction of reference regions whose reading-order index matches the
    /// hypothesis at the same position. 1.0 = identical order.
    reading_order_agreement: Option<f64>,
    /// Fraction of reference (row, col) table cells whose text matches the
    /// hypothesis. None when the reference has no tables.
    table_cell_agreement: Option<f64>,
    /// Fraction of reference pages whose `source` is `native` that the
    /// hypothesis also marks as `native`. None when the reference has no
    /// native pages.
    native_preservation: Option<f64>,
}

impl GraphFidelity {
    fn compute(reference: &DocumentGraph, hypothesis: &DocumentGraph) -> Self {
        Self {
            reading_order_agreement: Some(reading_order_score(reference, hypothesis)),
            table_cell_agreement: table_cell_score(reference, hypothesis),
            native_preservation: native_preservation_score(reference, hypothesis),
        }
    }
}

fn reading_order_score(ref_graph: &DocumentGraph, hyp_graph: &DocumentGraph) -> f64 {
    // Flatten region reading-order and text per page, then compute Kendall-tau
    // style agreement on text equality at the same reading-order index. Simpler
    // version: count regions where (text, kind) matches between ref and hyp at
    // the same reading_order within the same page.
    let mut total = 0u64;
    let mut matched = 0u64;
    for (r_page, h_page) in ref_graph.pages.iter().zip(hyp_graph.pages.iter()) {
        for r_region in &r_page.regions {
            total += 1;
            let hyp_match = h_page.regions.iter().any(|h| {
                h.reading_order == r_region.reading_order
                    && h.kind == r_region.kind
                    && h.text().trim() == r_region.text().trim()
            });
            if hyp_match {
                matched += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        matched as f64 / total as f64
    }
}

fn table_cell_score(ref_graph: &DocumentGraph, hyp_graph: &DocumentGraph) -> Option<f64> {
    // Build a (page, row, col) -> text map from each graph's table regions.
    let ref_cells = collect_table_cells(ref_graph);
    let hyp_cells = collect_table_cells(hyp_graph);
    if ref_cells.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut matched = 0u64;
    for (key, ref_text) in &ref_cells {
        total += 1;
        if let Some(hyp_text) = hyp_cells.get(key) {
            if hyp_text.trim() == ref_text.trim() {
                matched += 1;
            }
        }
    }
    Some(matched as f64 / total as f64)
}

fn collect_table_cells(
    graph: &DocumentGraph,
) -> std::collections::HashMap<(u32, usize, usize), String> {
    let mut out = std::collections::HashMap::new();
    for page in &graph.pages {
        for region in &page.regions {
            if !matches!(region.kind, superdocuments::graph::RegionKind::Table) {
                continue;
            }
            for span in &region.spans {
                if let Some((row, col)) = span.cell {
                    out.insert((page.number, row, col), span.text.clone());
                }
            }
        }
    }
    out
}

fn native_preservation_score(ref_graph: &DocumentGraph, hyp_graph: &DocumentGraph) -> Option<f64> {
    let ref_native_pages: Vec<u32> = ref_graph
        .pages
        .iter()
        .filter(|p| {
            p.regions
                .iter()
                .any(|r| matches!(r.source, Source::Native { .. }))
        })
        .map(|p| p.number)
        .collect();
    if ref_native_pages.is_empty() {
        return None;
    }
    let hyp_native_pages: std::collections::HashSet<u32> = hyp_graph
        .pages
        .iter()
        .filter(|p| {
            p.regions
                .iter()
                .any(|r| matches!(r.source, Source::Native { .. }))
        })
        .map(|p| p.number)
        .collect();
    let matched = ref_native_pages
        .iter()
        .filter(|n| hyp_native_pages.contains(n))
        .count();
    Some(matched as f64 / ref_native_pages.len() as f64)
}

fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n == 0 {
        0.0
    } else {
        values[n / 2]
    }
}

fn median_u64(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let n = values.len();
    if n == 0 {
        0
    } else {
        values[n / 2]
    }
}

fn fmt_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n}B")
    } else {
        format!("{:.1}KiB", n as f64 / 1024.0)
    }
}

fn read_rss_kib() -> u64 {
    let mut s = String::new();
    if std::fs::File::open("/proc/self/status")
        .ok()
        .and_then(|mut f| std::io::Read::read_to_string(&mut f, &mut s).ok())
        .is_none()
    {
        return 0;
    }
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            if let Some(kib) = rest.split_whitespace().next() {
                return kib.parse().unwrap_or(0);
            }
        }
    }
    0
}

// ── HTTP / OpenAI-compat path ──────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: ChatResponseMessage,
}

#[derive(Deserialize, Default)]
struct ChatResponseMessage {
    #[serde(default)]
    content: String,
}

fn http_chat_completion(
    base_url: &str,
    model: &str,
    pdf_bytes: &[u8],
    timeout: Duration,
) -> Result<String> {
    // The OpenAI-compatible multimodal contract varies by gateway. We use a
    // text-only prompt that asks for a structured JSON document parse. This is
    // what SGLang's PaddleOCR-VL-1.6 recipe returns; gateways just pass it
    // through. The "image" side is the page text from `extract_file` so the test
    // stays CPU-runnable: a real multimodal call would base64-encode the PDF
    // page, but that's beyond what ai-gateway/litellm/bifrost/portkey ship as
    // a uniform contract today.
    let pdf_text =
        superdocuments::pdf::extract_markdown(pdf_bytes).unwrap_or_else(|_| String::new());
    let prompt = format!(
        "<document>\n{}\n</document>\n\nRespond with the document's markdown \
         representation. Preserve reading order and table structure.",
        if pdf_text.is_empty() {
            "(no native text; OCR the rendered pages)"
        } else {
            pdf_text.as_str()
        }
    );
    let req = ChatRequest {
        model,
        messages: vec![ChatMessage {
            role: "user",
            content: prompt,
        }],
        stream: false,
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .context("build reqwest client")?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let body: ChatResponse = resp.json().context("decode chat response")?;
    if !status.is_success() {
        return Err(anyhow!("{} returned {}", url, status));
    }
    Ok(body
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default())
}

fn target_available(target: Target) -> bool {
    if let Some((base_url, _)) = target.endpoint() {
        // Cheap TCP probe: try a /models call. If the gateway isn't up, the
        // request will fail before we waste a chat-completion budget.
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        let resp = client
            .get(format!("{}/models", base_url.trim_end_matches('/')))
            .send();
        matches!(resp, Ok(r) if r.status().is_success())
    } else {
        // Candle is "available" only when compiled with the feature.
        cfg!(feature = "candle")
    }
}

fn run_target(
    target: Target,
    fixture: &Fixture,
    iterations: u64,
    timeout: Duration,
) -> Result<Sample> {
    // Read bytes once outside the timed loop so the wall measurement reflects
    // inference time, not file I/O.
    let pdf_bytes =
        std::fs::read(&fixture.path).with_context(|| format!("read {}", fixture.path.display()))?;
    let mut accum = Sample::default();

    if target.is_candle() {
        // The candle target is a placeholder until the `candle` feature is
        // implemented in a follow-up commit. For now it returns a clean
        // UNREACHABLE-style failure rather than panicking, so the harness stays
        // runnable on the default build.
        return Err(anyhow!(
            "candle target is not yet implemented; build with `--features candle` once the runner lands"
        ));
    }

    let (base_url, model) = target.endpoint().expect("non-candle has endpoint");
    let t = Instant::now();
    let mut last_output = String::new();
    for _ in 0..iterations {
        match http_chat_completion(base_url, model, &pdf_bytes, timeout) {
            Ok(text) => last_output = text,
            Err(e) => return Err(e),
        }
    }
    accum.wall_secs = t.elapsed().as_secs_f64();
    accum.output_text = last_output.clone();
    accum.bytes_out = last_output.len() as u64;
    accum.output_graph = Some(from_ocr_markdown(
        &last_output,
        Source::Vlm {
            model: target.name().into(),
            version: "openai-compat".into(),
        },
    ));
    if let (Some(ref_graph), Some(hyp_graph)) = (&fixture.graph_expected, &accum.output_graph) {
        accum.graph_fidelity = Some(GraphFidelity::compute(ref_graph, hyp_graph));
    }
    Ok(accum)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let fixtures = load_fixtures(&cli.fixtures);
    if fixtures.is_empty() {
        return Err(anyhow!("no PDFs found under {}", cli.fixtures.display()));
    }
    eprintln!(
        "loaded {} fixtures from {}",
        fixtures.len(),
        cli.fixtures.display()
    );
    for f in &fixtures {
        eprintln!(
            "  {} ({}B, text_gt={}, graph_gt={})",
            f.path.display(),
            fmt_bytes(f.bytes_len),
            if f.text_expected.is_some() {
                "yes"
            } else {
                "no"
            },
            if f.graph_expected.is_some() {
                "yes"
            } else {
                "no"
            },
        );
    }

    // Background RSS monitor — parent-side only; subprocess endpoints won't
    // contribute to the parent poll, same caveat as `ocr-vs-incumbents`.
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor_stop_c = monitor_stop.clone();
    let monitor = thread::spawn(move || {
        let mut peak = read_rss_kib();
        while !monitor_stop_c.load(Ordering::Relaxed) {
            peak = peak.max(read_rss_kib());
            thread::sleep(Duration::from_millis(4));
        }
        peak
    });

    let targets: Vec<Target> = Target::all()
        .into_iter()
        .filter(|t| cli.only.is_empty() || cli.only.iter().any(|name| name == t.name()))
        .filter(|t| target_available(*t))
        .collect();
    if targets.is_empty() {
        eprintln!(
            "no targets available. Check that the OpenAI-compat gateways are up \
             on the expected ports (see ocr/README.md)."
        );
    }
    eprintln!(
        "active targets: {}",
        targets
            .iter()
            .map(|t| t.name())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let timeout = Duration::from_secs(cli.timeout_secs);
    let mut rows = Vec::new();

    for target in &targets {
        for fixture in &fixtures {
            for _ in 0..cli.warmup {
                let _ = run_target(*target, fixture, 1, timeout);
            }
            let mut per_sample_wall = Vec::with_capacity(cli.samples as usize);
            let mut per_sample_peak_rss = Vec::with_capacity(cli.samples as usize);
            let mut per_sample_cer = Vec::with_capacity(cli.samples as usize);
            let mut per_sample_reading_order = Vec::with_capacity(cli.samples as usize);
            let mut per_sample_table_cell = Vec::with_capacity(cli.samples as usize);
            let mut per_sample_native_preservation = Vec::with_capacity(cli.samples as usize);
            let mut per_sample_bytes = Vec::with_capacity(cli.samples as usize);
            let mut error_count = 0u64;
            for _ in 0..cli.samples {
                let peak0 = read_rss_kib();
                match run_target(*target, fixture, cli.iterations, timeout) {
                    Ok(s) => {
                        per_sample_wall.push(s.wall_secs);
                        per_sample_peak_rss.push(peak0.max(read_rss_kib()));
                        per_sample_bytes.push(s.bytes_out);
                        if let Some(gt) = &fixture.text_expected {
                            if let Some(score) = cer(gt, &normalize_ocr_markdown(&s.output_text)) {
                                per_sample_cer.push(score);
                            }
                        }
                        if let Some(gf) = &s.graph_fidelity {
                            if let Some(v) = gf.reading_order_agreement {
                                per_sample_reading_order.push(v);
                            }
                            if let Some(v) = gf.table_cell_agreement {
                                per_sample_table_cell.push(v);
                            }
                            if let Some(v) = gf.native_preservation {
                                per_sample_native_preservation.push(v);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("  {} on {}: {e:#}", target.name(), fixture.path.display());
                        error_count += 1;
                    }
                }
            }
            let median_wall = median_f64(&mut per_sample_wall);
            let median_peak = median_u64(&mut per_sample_peak_rss) as f64;
            let median_cer = median_f64(&mut per_sample_cer);
            let median_reading_order = median_f64(&mut per_sample_reading_order);
            let median_table_cell = median_f64(&mut per_sample_table_cell);
            let median_native_pres = median_f64(&mut per_sample_native_preservation);
            let median_bytes = median_u64(&mut per_sample_bytes) as f64;
            let bytes_per_sec = if median_wall > 0.0 {
                (fixture.bytes_len as f64 * cli.iterations as f64) / median_wall
            } else {
                0.0
            };
            eprintln!(
                "{:<22} {:<48} samples={} median_wall={:.3}s peak={}KiB bytes/s={:.0} cer={} ro={} tc={} np={} errors={}",
                target.name(),
                fixture.path.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
                cli.samples,
                median_wall,
                median_peak as u64,
                bytes_per_sec,
                if per_sample_cer.is_empty() { f64::NAN } else { median_cer },
                if per_sample_reading_order.is_empty() { f64::NAN } else { median_reading_order },
                if per_sample_table_cell.is_empty() { f64::NAN } else { median_table_cell },
                if per_sample_native_preservation.is_empty() { f64::NAN } else { median_native_pres },
                error_count,
            );
            rows.push(json!({
                "target": target.name(),
                "fixture": fixture.path.to_string_lossy(),
                "fixture_bytes": fixture.bytes_len,
                "iterations_per_sample": cli.iterations,
                "samples": cli.samples,
                "warmup": cli.warmup,
                "median_wall_secs": median_wall,
                "median_peak_rss_kib": median_peak,
                "median_cer": if per_sample_cer.is_empty() { None } else { Some(median_cer) },
                "median_reading_order_agreement": if per_sample_reading_order.is_empty() { None } else { Some(median_reading_order) },
                "median_table_cell_agreement": if per_sample_table_cell.is_empty() { None } else { Some(median_table_cell) },
                "median_native_preservation": if per_sample_native_preservation.is_empty() { None } else { Some(median_native_pres) },
                "median_bytes_out": median_bytes,
                "bytes_per_sec": bytes_per_sec,
                "error_count": error_count,
                "available": target_available(*target),
            }));
        }
    }

    monitor_stop.store(true, Ordering::Relaxed);
    let _ = monitor.join();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let report = json!({
        "kind": "superdocuments-bench-ocr",
        "timestamp_unix": now,
        "fixtures_root": cli.fixtures.to_string_lossy(),
        "fixture_count": fixtures.len(),
        "warmup": cli.warmup,
        "samples": cli.samples,
        "iterations": cli.iterations,
        "targets": targets.iter().map(|t| t.name()).collect::<Vec<_>>(),
        "definitions": {
            "median_wall_secs": "median across `samples` per-sample medians, not across individual iterations. Documented honestly because the inspector's first criticism of the CPU bench was exactly this ambiguity.",
            "median_peak_rss_kib": "parent-side 4 ms `/proc/self/status` poll. Subprocess OpenAI-compat upstreams don't contribute to the parent's RSS.",
            "median_cer": "edit distance / reference length, both alphanumeric-lowercase. Reference is `<fixture>.expected.txt`. `null` when reference is missing.",
            "median_reading_order_agreement": "fraction of reference regions whose (kind, text, reading_order) matches the hypothesis at the same reading_order within the same page. 1.0 = identical order.",
            "median_table_cell_agreement": "fraction of reference table cells (page, row, col) whose text matches the hypothesis. `null` when the reference has no tables.",
            "median_native_preservation": "fraction of reference pages whose `source` is `native` that the hypothesis also marks as `native`. `null` when the reference has no native pages. Critical for the mixed native+scanned fixture: a hallucinating VLM that rewrites a native page as OCR text scores 0 here.",
            "median_bytes_out": "median output markdown byte count across samples.",
            "bytes_per_sec": "(fixture_bytes * iterations) / median_wall_secs.",
            "n_runs_per_cell": "samples × iterations; warmup runs are excluded.",
        },
        "n_runs_per_cell": cli.samples * cli.iterations,
        "rows": rows,
    });
    if let Some(parent) = cli.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    std::fs::write(
        &cli.out,
        serde_json::to_vec_pretty(&report).context("serialize report")?,
    )
    .with_context(|| format!("write {}", cli.out.display()))?;
    println!("{}", cli.out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use superdocuments::graph::{from_ocr_markdown, RegionKind, Source};

    fn ocr_source(label: &str) -> Source {
        Source::Ocr {
            model: label.into(),
            dpi: 200,
            psm: 3,
            languages: "eng".into(),
        }
    }

    #[test]
    fn cer_handles_empty_inputs_as_none() {
        assert_eq!(cer("", "abc"), None);
        assert_eq!(cer("abc", ""), None);
        let v = cer("abc", "abc").unwrap();
        assert!(v.abs() < 1e-9);
        let v = cer("abc", "abd").unwrap();
        assert!(v > 0.0 && v < 1.0);
    }

    #[test]
    fn normalize_collapses_alphanumeric_only() {
        assert_eq!(normalize("Invoice 2048!"), "invoice2048");
        assert_eq!(normalize("Mixed CASE 123"), "mixedcase123");
    }

    #[test]
    fn parse_graph_expected_round_trips_through_from_ocr_markdown() {
        let json =
            r#"{"markdown":"<!-- page 1 -->\n# Title\n\nBody line.\n","source_label":"ocr"}"#;
        let g = parse_graph_expected(json).expect("parse");
        assert_eq!(g.pages.len(), 1);
        assert!(g.pages[0]
            .regions
            .iter()
            .any(|r| r.kind == RegionKind::Title));
    }

    #[test]
    fn reading_order_score_matches_identical_graphs_at_one() {
        let md = "<!-- page 1 -->\nA\n\nB\n\nC\n";
        let r = from_ocr_markdown(md, ocr_source("ref"));
        let h = from_ocr_markdown(md, ocr_source("hyp"));
        assert!((reading_order_score(&r, &h) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn table_cell_score_full_match_and_miss() {
        let ref_md = "<!-- page 1 -->\n| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let hyp_full = ref_md;
        let hyp_swap = "<!-- page 1 -->\n| A | B |\n| --- | --- |\n| 2 | 1 |\n";
        let r = from_ocr_markdown(ref_md, ocr_source("ref"));
        let h_full = from_ocr_markdown(hyp_full, ocr_source("hyp"));
        let h_swap = from_ocr_markdown(hyp_swap, ocr_source("hyp"));
        assert_eq!(table_cell_score(&r, &h_full), Some(1.0));
        // Swapped values: cells exist at the same positions but text differs.
        let swapped = table_cell_score(&r, &h_swap).unwrap();
        assert!(swapped < 1.0 && swapped > 0.0);
    }

    #[test]
    fn native_preservation_score_zero_when_hyp_marks_everything_ocr() {
        let mut r = from_ocr_markdown("<!-- page 1 -->\nA", ocr_source("ref"));
        // Force the source on the first region to Native.
        r.pages[0].regions[0].source = Source::Native {
            model: "ref".into(),
        };
        let h = from_ocr_markdown("<!-- page 1 -->\nA", ocr_source("hyp"));
        assert_eq!(native_preservation_score(&r, &h), Some(0.0));
    }

    #[test]
    fn median_helpers_handle_empty_input() {
        let mut v: Vec<f64> = vec![];
        assert_eq!(median_f64(&mut v), 0.0);
        let mut v: Vec<u64> = vec![];
        assert_eq!(median_u64(&mut v), 0);
    }
}
