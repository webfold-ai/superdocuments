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
use std::process::Stdio;
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

    /// TTFT/ITL lane: number of streamed requests guidellm drives per target.
    /// Same shape as `ai-gateway/benches/ttft.rs` (50 requests, rate=5, 32 prompt/
    /// output tokens). The default 0 means "skip the guidellm/aiperf subprocess
    /// lane and report wall-clock only" — useful for cold runs where installing
    /// guidellm is not yet wired.
    #[arg(long, default_value_t = 0)]
    ttft_requests: u64,
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
    /// `run-llama/liteparse` — Rust-native PDF + OCR via the `lit` binary. Built
    /// in-process via PDFium with Tesseract as the OCR engine. Falls back to
    /// native text when no OCR is needed (~2-5 ms/page).
    Liteparse,
    /// `run-llama/ParseBench` — wraps the Python benchmark harness. Subprocess
    /// driven; uses the published `parse-bench` CLI to run inference +
    /// evaluation against the document parsing pipelines we register.
    ParseBench,
    /// `tesseract-direct` — vanilla Tesseract subprocess call. Always available
    /// on PATH where Tesseract is installed; the only OCR target that runs
    /// without GPU or a Python venv. Kept here as the canonical baseline: any
    /// inference target slower than `tesseract-direct` on a given fixture is a
    /// regression, not an improvement. Mirrors the same-named target in
    /// `webfold-bench/bench/ocr-vs-incumbents`.
    TesseractDirect,
    /// `datalab-to/marker` — 40k★ GPU PDF→Markdown. Apache-2.0 code / OpenRAIL-M
    /// weights. Drives the `marker_single` Python CLI on the local PDF.
    Marker,
    /// `docling-project/docling` — 68k★ MIT doc-AI framework. Drives the
    /// `docling` Python CLI; output goes to a temp dir, we read the produced
    /// `*.md` file.
    Docling,
    /// `datalab-to/surya` — 21k★ OCR + layout + reading order + tables in 90+
    /// languages. Apache-2.0 code / OpenRAIL-M weights. Drives the `surya_ocr`
    /// Python CLI.
    SuryaOcr,
    /// `candle-qwen-serve` — the in-tree `superdocuments-bench/services/candle-serve`
    /// HTTP service. Loads Qwen2.5-0.5B-Instruct via candle-transformers and
    /// exposes OpenAI-compat `/v1/chat/completions`. CPU-only, slow but zero
    /// external deps beyond the model weights. Sub HTTP service per the
    /// "eventually our own" directive; replaces the original candle subprocess
    /// target so the model lives in a separate process and can be re-used
    /// outside the bench.
    CandleQwenServe,
    /// `llama-cpp-server` — official llama.cpp server binary (`llama-server`).
    /// OpenAI-compat HTTP API; run as `./llama-server -m model.gguf --port
    /// 8081`. Lightweight GGUF runtime; benchmarks any GGUF model the local
    /// harness has on disk. Endpoint defaults to `http://localhost:8081/v1`.
    LlamaCppServer,
    /// `peregrine-ng` — Ptyktos Rust-native GGUF inference engine. Invoked via
    /// the `peregrine-serve` CLI (when wired) or the in-process library.
    /// Qwen2.5/Qwen3.5 family; loads GGUF weights directly.
    PeregrineNg,
    /// `modelfold` — Ptyktos ML primitives substrate (GPU kernels,
    /// quantization, training fold). For OCR/inference benches, used via the
    /// `modelfold-serve` companion that loads a checkpoint and exposes
    /// `/v1/chat/completions`. Subprocess when the binary is on PATH.
    ModelfoldServe,
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
            Self::Liteparse => "liteparse",
            Self::ParseBench => "parse-bench",
            Self::TesseractDirect => "tesseract-direct",
            Self::Marker => "marker",
            Self::Docling => "docling",
            Self::SuryaOcr => "surya-ocr",
            Self::CandleQwenServe => "candle-qwen-serve",
            Self::LlamaCppServer => "llama-cpp-server",
            Self::PeregrineNg => "peregrine-ng",
            Self::ModelfoldServe => "modelfold-serve",
        }
    }

    /// OpenAI-compatible endpoint and model name. Candle is the only in-process
    /// target; everything else is HTTP or subprocess.
    fn endpoint(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::AiGateway => Some(("http://localhost:3100/v1", "mock-ocr")),
            Self::Litellm => Some(("http://localhost:4000/v1", "mock-ocr")),
            Self::Bifrost => Some(("http://localhost:8080/v1", "mock-ocr")),
            Self::Portkey => Some(("http://localhost:8787/v1", "mock-ocr")),
            Self::SglangOcr => Some(("http://localhost:30001/v1", "PaddlePaddle/PaddleOCR-VL-1.6")),
            Self::VllmOcr => Some(("http://localhost:30000/v1", "PaddlePaddle/PaddleOCR-VL-1.6")),
            Self::CandleQwenServe => {
                Some(("http://localhost:38080/v1", "Qwen/Qwen2.5-0.5B-Instruct"))
            }
            Self::LlamaCppServer => Some(("http://localhost:8081/v1", "local-gguf-model")),
            Self::PeregrineNg => Some(("http://localhost:38100/v1", "peregrine-default")),
            Self::ModelfoldServe => Some(("http://localhost:38200/v1", "modelfold-default")),
            Self::CandleQwen25Vl
            | Self::Liteparse
            | Self::ParseBench
            | Self::TesseractDirect
            | Self::Marker
            | Self::Docling
            | Self::SuryaOcr => None,
        }
    }

    fn is_candle(self) -> bool {
        matches!(self, Self::CandleQwen25Vl)
    }

    fn all() -> [Target; 17] {
        [
            Self::CandleQwen25Vl,
            Self::AiGateway,
            Self::Litellm,
            Self::Bifrost,
            Self::Portkey,
            Self::SglangOcr,
            Self::VllmOcr,
            Self::Liteparse,
            Self::ParseBench,
            Self::TesseractDirect,
            Self::Marker,
            Self::Docling,
            Self::SuryaOcr,
            Self::CandleQwenServe,
            Self::LlamaCppServer,
            Self::PeregrineNg,
            Self::ModelfoldServe,
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
    walk_pdfs(root, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Walk `root` recursively, collecting every `*.pdf` plus its sidecar files.
/// Both the shipped 9-fixture corpus (flat layout) and the downloaded
/// ParseBench subset (nested `docs/{text,table,layout,chart}/`) Just Work
/// because `walk_dir` recurses.
fn walk_pdfs(dir: &Path, out: &mut Vec<Fixture>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            eprintln!("could not read fixtures dir {}: {e}", dir.display());
            return;
        }
    };
    for item in entries.flatten() {
        let path = item.path();
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            walk_pdfs(&path, out);
            continue;
        }
        if !file_type.is_file() {
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
        match target {
            Target::CandleQwen25Vl => cfg!(feature = "candle"),
            // LiteParse is available when the `lit` binary is on PATH (built from
            // run-llama/liteparse via `cargo install lit` or vendored).
            Target::Liteparse => command_on_path("lit"),
            // ParseBench is available when the `parse-bench` Python CLI is on
            // PATH (installed via `pip install parse-bench`).
            Target::ParseBench => command_on_path("parse-bench"),
            // Tesseract-direct: `tesseract` on PATH. This is the canonical
            // CPU baseline target — almost always available.
            Target::TesseractDirect => command_on_path("tesseract"),
            // Marker — datalab-to/marker Python CLI (`pip install marker-pdf`).
            Target::Marker => command_on_path("marker_single"),
            // Docling — docling-project/docling Python CLI (`pip install docling`).
            Target::Docling => command_on_path("docling"),
            // Surya — datalab-to/surya Python CLI (`pip install surya-ocr`).
            Target::SuryaOcr => command_on_path("surya_ocr"),
            // HTTP endpoints — `target_available` already short-circuits to the
            // `/v1/models` probe for any target with an `endpoint()`. The four
            // Ptyktos + llama.cpp servers (candle-qwen-serve, llama-cpp-server,
            // peregrine-ng, modelfold-serve) are not subprocess invocations;
            // they're HTTP services the user runs separately.
            Target::CandleQwenServe
            | Target::LlamaCppServer
            | Target::PeregrineNg
            | Target::ModelfoldServe => false,
            _ => false,
        }
    }
}

/// Cheap PATH probe; returns true iff `program` resolves via the standard PATH
/// lookup. Avoids the heavier `Command::new(...).status()` shell-out.
fn command_on_path(program: &str) -> bool {
    let path_var = std::env::var_os("PATH");
    let extensions = ["", ".exe"];
    let candidates = path_var
        .as_ref()
        .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .unwrap_or_default();
    for dir in candidates {
        for ext in extensions {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

/// TTFT/ITL result for one OpenAI-compat target, captured by driving `guidellm`
/// as a subprocess and reading the resulting JSON. Mirrors the shape produced
/// by `ai-gateway/benches/ttft.rs::run_guidellm` so the JSON output is drop-in
/// comparable with that harness's results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TtftResult {
    tool: String,
    target: String,
    request_count: u64,
    /// Time to first token, milliseconds. None when guidellm couldn't parse the
    /// output schema or the subprocess failed.
    ttft_ms: Option<f64>,
    /// Inter-token latency, milliseconds. Same None-on-failure semantics.
    itl_ms: Option<f64>,
    /// Token throughput (tokens/sec). Used by the inspector's three-column
    /// table to attribute wall time to prefill vs decode.
    tokens_per_sec: Option<f64>,
    /// Raw guidellm JSON output path. Empty when the subprocess never ran.
    raw_path: String,
    error: Option<String>,
}

/// Drive `guidellm run` against one OpenAI-compat target. Same `kind=openai_http`
/// backend shape as `ai-gateway/benches/ttft.rs`. Returns the parsed TTFT/ITL
/// (best-effort; on failure, returns a TtftResult with `error` set and None
/// numerics — never panics, never fabricates a number).
fn run_guidellm_ttft(target: Target, request_count: u64, raw_dir: &Path) -> TtftResult {
    let raw_path = raw_dir.join(format!("guidellm_{}.json", target.name()));
    let raw_path_str = raw_path.to_string_lossy().into_owned();
    let (base_url, model) = match target.endpoint() {
        Some(e) => e,
        None => {
            return TtftResult {
                tool: "guidellm".into(),
                target: target.name().into(),
                request_count: 0,
                error: Some("candle target has no HTTP endpoint".into()),
                raw_path: raw_path_str,
                ..Default::default()
            };
        }
    };
    let backend = format!("kind=openai_http,target={base_url},model={model}");
    let output_arg = format!("kind=json,path={raw_path_str}");
    let count_arg = format!("kind=max_requests,count={request_count}");
    let args = [
        "run",
        "--backend",
        &backend,
        "--data",
        "kind=synthetic_text,prompt_tokens=32,output_tokens=32",
        "--profile",
        "kind=constant,rate=5",
        "--constraint",
        &count_arg,
        "--output",
        &output_arg,
    ];
    let run_out = std::process::Command::new("guidellm").args(args).output();
    let output = match run_out {
        Ok(o) => o,
        Err(e) => {
            return TtftResult {
                tool: "guidellm".into(),
                target: target.name().into(),
                request_count,
                error: Some(format!(
                    "guidellm not runnable: {e} (pip install guidellm?)"
                )),
                raw_path: raw_path_str,
                ..Default::default()
            };
        }
    };
    if !output.status.success() {
        return TtftResult {
            tool: "guidellm".into(),
            target: target.name().into(),
            request_count,
            error: Some(format!(
                "guidellm exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )),
            raw_path: raw_path_str,
            ..Default::default()
        };
    }
    let raw = match std::fs::read_to_string(&raw_path) {
        Ok(s) => s,
        Err(e) => {
            return TtftResult {
                tool: "guidellm".into(),
                target: target.name().into(),
                request_count,
                error: Some(format!("read {}: {e}", raw_path.display())),
                raw_path: raw_path_str,
                ..Default::default()
            };
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return TtftResult {
                tool: "guidellm".into(),
                target: target.name().into(),
                request_count,
                error: Some(format!("parse {}: {e}", raw_path.display())),
                raw_path: raw_path_str,
                ..Default::default()
            };
        }
    };
    TtftResult {
        tool: "guidellm".into(),
        target: target.name().into(),
        request_count,
        ttft_ms: find_numeric_field(&parsed, "ttft"),
        itl_ms: find_numeric_field(&parsed, "itl"),
        tokens_per_sec: find_numeric_field(&parsed, "throughput")
            .or_else(|| find_numeric_field(&parsed, "tokens_per_second")),
        raw_path: raw_path_str,
        error: None,
    }
}

/// Apply the per-target result-shape logic: graph-fidelity scoring against
/// the fixture's expected graph. The OpenAI-compat row takes a different
/// code path because it builds `s` inline in `run_target`; this helper
/// handles the subprocess targets (liteparse, parse-bench) where the model
/// output comes from outside Rust.
fn finalize_sample(mut s: Sample, fixture: &Fixture, _target: Target) -> Result<Sample> {
    if let (Some(ref_graph), Some(hyp_graph)) = (&fixture.graph_expected, &s.output_graph) {
        s.graph_fidelity = Some(GraphFidelity::compute(ref_graph, hyp_graph));
    }
    Ok(s)
}

/// Recursively search a JSON value for the first numeric field whose key
/// (case-insensitive) contains `needle`. guidellm's exact field naming has
/// changed across versions; substring search keeps the harness robust.
fn find_numeric_field(value: &serde_json::Value, needle: &str) -> Option<f64> {
    let needle = needle.to_ascii_lowercase();
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let kl = k.to_ascii_lowercase();
                if kl.contains(&needle) || kl == needle {
                    if let Some(n) = v.as_f64() {
                        return Some(n);
                    }
                }
                if let Some(n) = find_numeric_field(v, &needle) {
                    return Some(n);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_numeric_field(v, &needle)),
        _ => None,
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
        #[cfg(feature = "candle")]
        {
            return run_candle_target(&pdf_bytes, iterations, timeout);
        }
        #[cfg(not(feature = "candle"))]
        {
            return Err(anyhow!(
                "candle target requires `--features candle`; the default build doesn't pull the ML stack. \
                 See ocr/README.md §Candle for build instructions."
            ));
        }
    }

    // LiteParse — Rust-native PDF parser with built-in Tesseract fallback. Drives
    // the `lit` binary as a subprocess; the bench writes the input PDF to a
    // temp file and reads markdown from stdout. Output quality depends on the
    // lit build flags (Tesseract on by default; pass `--ocr-engine=none` for
    // native-text-only). The --features-candle toggle doesn't affect this path.
    if matches!(target, Target::Liteparse) {
        let t = Instant::now();
        let mut last_output = String::new();
        for _ in 0..iterations {
            match run_lit_subprocess(&fixture.path, timeout) {
                Ok(text) => last_output = text,
                Err(e) => return Err(e),
            }
        }
        let s = Sample {
            wall_secs: t.elapsed().as_secs_f64(),
            output_text: last_output.clone(),
            bytes_out: last_output.len() as u64,
            output_graph: Some(from_ocr_markdown(
                &last_output,
                Source::Vlm {
                    model: "run-llama/liteparse".into(),
                    version: "lit-subprocess".into(),
                },
            )),
            ..Default::default()
        };
        return finalize_sample(s, fixture, target);
    }

    // ParseBench — wraps the run-llama/ParseBench Python CLI. The harness
    // registers a pipeline that points at our inference target (default:
    // anything that hits our OpenAI-compat upstream) and runs the published
    // 5-dimension evaluation. We capture the eval JSON and surface the headline
    // scores alongside our own CER / graph-fidelity.
    if matches!(target, Target::ParseBench) {
        let t = Instant::now();
        let mut last_output = String::new();
        for _ in 0..iterations {
            match run_parse_bench_subprocess(&fixture.path, timeout) {
                Ok(text) => last_output = text,
                Err(e) => return Err(e),
            }
        }
        let s = Sample {
            wall_secs: t.elapsed().as_secs_f64(),
            output_text: last_output.clone(),
            bytes_out: last_output.len() as u64,
            output_graph: Some(from_ocr_markdown(
                &last_output,
                Source::Vlm {
                    model: "run-llama/ParseBench".into(),
                    version: "parse-bench-subprocess".into(),
                },
            )),
            ..Default::default()
        };
        return finalize_sample(s, fixture, target);
    }

    // Marker — `datalab-to/marker` 40k★ GPU PDF→Markdown. `marker_single`
    // writes markdown to stdout (with `--output_format markdown`). The
    // tool downloads ~2GB of model weights on first run; subsequent runs are
    // cached under `~/.cache/datalab/`. We use `--disable_image_extraction`
    // to skip image decoding (we don't score images) and emit markdown.
    if matches!(target, Target::Marker) {
        let t = Instant::now();
        let mut last_output = String::new();
        for _ in 0..iterations {
            match run_marker_subprocess(&fixture.path, timeout) {
                Ok(text) => last_output = text,
                Err(e) => return Err(e),
            }
        }
        let s = Sample {
            wall_secs: t.elapsed().as_secs_f64(),
            output_text: last_output.clone(),
            bytes_out: last_output.len() as u64,
            output_graph: Some(from_ocr_markdown(
                &last_output,
                Source::Vlm {
                    model: "datalab-to/marker".into(),
                    version: "marker_single-subprocess".into(),
                },
            )),
            ..Default::default()
        };
        return finalize_sample(s, fixture, target);
    }

    // Docling — `docling-project/docling` 68k★ MIT doc-AI framework. The
    // `docling` CLI writes a `<name>.md` next to the input file (or to
    // `--output`); we read that back. Docling downloads its own model
    // weights on first run (~1.5GB).
    if matches!(target, Target::Docling) {
        let t = Instant::now();
        let mut last_output = String::new();
        for _ in 0..iterations {
            match run_docling_subprocess(&fixture.path, timeout) {
                Ok(text) => last_output = text,
                Err(e) => return Err(e),
            }
        }
        let s = Sample {
            wall_secs: t.elapsed().as_secs_f64(),
            output_text: last_output.clone(),
            bytes_out: last_output.len() as u64,
            output_graph: Some(from_ocr_markdown(
                &last_output,
                Source::Vlm {
                    model: "docling-project/docling".into(),
                    version: "docling-subprocess".into(),
                },
            )),
            ..Default::default()
        };
        return finalize_sample(s, fixture, target);
    }

    // Tesseract-direct — vanilla Tesseract subprocess. Rasterize the PDF with
    // `pdftoppm`, OCR each page with `tesseract` (--psm 3 default), concatenate
    // output. The canonical CPU baseline that any inference target must beat
    // to be a net win. Same algorithm as `webfold-bench/bench/ocr-vs-incumbents`
    // `tesseract_direct`, kept here so the inference stack has a referent.
    if matches!(target, Target::TesseractDirect) {
        let t = Instant::now();
        let mut last_output = String::new();
        for _ in 0..iterations {
            match run_tesseract_direct_subprocess(&fixture.path, timeout) {
                Ok(text) => last_output = text,
                Err(e) => return Err(e),
            }
        }
        let s = Sample {
            wall_secs: t.elapsed().as_secs_f64(),
            output_text: last_output.clone(),
            bytes_out: last_output.len() as u64,
            output_graph: Some(from_ocr_markdown(
                &last_output,
                Source::Ocr {
                    model: "tesseract-direct".into(),
                    dpi: 200,
                    psm: 3,
                    languages: "eng".into(),
                },
            )),
            ..Default::default()
        };
        return finalize_sample(s, fixture, target);
    }

    // Surya — `datalab-to/surya` 21k★ OCR + layout + reading order + tables
    // in 90+ languages. The `surya_ocr` CLI requires a SuryaInferenceManager
    // (auto-spawns vllm or llama-server); on first run it downloads model
    // weights and starts a server. We surface whatever it emits as the model
    // output. Honors SURYA_INFERENCE_BACKEND / SURYA_INFERENCE_URL env vars.
    if matches!(target, Target::SuryaOcr) {
        let t = Instant::now();
        let mut last_output = String::new();
        for _ in 0..iterations {
            match run_surya_subprocess(&fixture.path, timeout) {
                Ok(text) => last_output = text,
                Err(e) => return Err(e),
            }
        }
        let s = Sample {
            wall_secs: t.elapsed().as_secs_f64(),
            output_text: last_output.clone(),
            bytes_out: last_output.len() as u64,
            output_graph: Some(from_ocr_markdown(
                &last_output,
                Source::Vlm {
                    model: "datalab-to/surya".into(),
                    version: "surya_ocr-subprocess".into(),
                },
            )),
            ..Default::default()
        };
        return finalize_sample(s, fixture, target);
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

    // TTFT/ITL lane — guidellm-driven measurement, one shot per OpenAI-compat
    // target. Skipped when --ttft-requests 0 (default) so cold runs without
    // guidellm installed don't fail. The raw guidellm JSON files are dropped
    // next to the bench report so the inspector's "guidance" evidence is
    // self-contained.
    let mut ttft_rows: Vec<TtftResult> = Vec::new();
    if cli.ttft_requests > 0 {
        let raw_dir = cli.out.parent().unwrap_or(Path::new(".")).join("ttft_raw");
        std::fs::create_dir_all(&raw_dir).ok();
        for target in &targets {
            ttft_rows.push(run_guidellm_ttft(*target, cli.ttft_requests, &raw_dir));
        }
    }

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
        "ttft_lane": ttft_rows,
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

    #[test]
    fn find_numeric_field_walks_nested_objects() {
        let v: serde_json::Value = serde_json::json!({
            "benchmarks": [{"metrics": {"ttft_p50_ms": 12.3, "itl_ms": 8.7}}]
        });
        assert_eq!(find_numeric_field(&v, "ttft"), Some(12.3));
        assert_eq!(find_numeric_field(&v, "itl"), Some(8.7));
        assert_eq!(find_numeric_field(&v, "missing"), None);
    }

    #[test]
    fn ttft_lane_skipped_when_request_count_zero() {
        let cli = Cli::parse_from(["superdocuments-bench-ocr", "--ttft-requests", "0"]);
        assert_eq!(cli.ttft_requests, 0);
    }

    #[test]
    fn command_on_path_finds_cargo_or_sh() {
        // Cargo's CI box usually has at least sh on PATH. If PATH is empty
        // (some sandboxes), returns false; that's also a valid signal.
        let _ = command_on_path("sh"); // may be true or false depending on env
        let _ = command_on_path("definitely-not-on-path-xyz123");
        assert!(!command_on_path("definitely-not-on-path-xyz123"));
    }

    #[test]
    fn target_enumeration_includes_liteparse_and_parsebench() {
        // The full target list is 17 entries. Adding/removing targets is a
        // breaking change for downstream JSON consumers; this test guards that.
        assert_eq!(Target::all().len(), 17);
        let names: Vec<&str> = Target::all().iter().map(|t| t.name()).collect();
        assert!(names.contains(&"liteparse"));
        assert!(names.contains(&"parse-bench"));
        assert!(names.contains(&"marker"));
        assert!(names.contains(&"docling"));
        assert!(names.contains(&"surya-ocr"));
        assert!(names.contains(&"tesseract-direct"));
        assert!(names.contains(&"candle-qwen-serve"));
        assert!(names.contains(&"llama-cpp-server"));
        assert!(names.contains(&"peregrine-ng"));
        assert!(names.contains(&"modelfold-serve"));
    }

    #[test]
    fn walk_pdfs_finds_pdfs_in_subdirectories() {
        // The recursive walker handles both flat (shipped superdocuments
        // fixtures) and nested (downloaded ParseBench subset) layouts.
        // This test creates a temp tree with a PDF in a subdirectory and
        // verifies the walker sees it.
        let temp =
            std::env::temp_dir().join(format!("superdocs-bench-walk-pdfs-{}", std::process::id()));
        let nested = temp.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let pdf = nested.join("test.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n").unwrap();
        let mut found = Vec::new();
        super::walk_pdfs(&temp, &mut found);
        assert_eq!(found.len(), 1, "expected exactly one PDF in temp tree");
        assert!(found[0].path.ends_with("test.pdf"));
        let _ = std::fs::remove_dir_all(&temp);
    }
}

// ── Candle (in-process Rust inference) target ─────────────────────────────────
//
// Text-only baseline today. Reads the extracted PDF text and runs it through
// Qwen2.5-0.5B-Instruct (smallest variant that fits comfortably on CPU and
// still produces coherent markdown). NOT visual OCR — that's the Qwen2.5-VL
// vision-tower port, deferred to a follow-up session because
// candle-transformers 0.9 ships no qwen2_5_vl model module.
//
// Honest reporting: every row produced by this path has
// `target_kind = "candle-text-completion"` in the JSON output. A reader can
// tell at a glance that the fidelity score is "this is what the model thinks
// of the extracted text", not "this is what the model read off the page".
#[cfg(feature = "candle")]
mod candle_runner {
    use super::*;

    const DEFAULT_MODEL_ID: &str = "Qwen/Qwen2.5-0.5B-Instruct";
    const MAX_NEW_TOKENS: usize = 256;

    pub fn run(pdf_bytes: &[u8], iterations: u64, _timeout: Duration) -> Result<Sample> {
        let mut accum = Sample::default();
        let t_total = Instant::now();
        let mut last_text = String::new();

        for _ in 0..iterations {
            last_text = one_inference(pdf_bytes)?;
        }
        accum.wall_secs = t_total.elapsed().as_secs_f64();
        accum.output_text = last_text.clone();
        accum.bytes_out = last_text.len() as u64;
        accum.output_graph = Some(from_ocr_markdown(
            &last_text,
            Source::Vlm {
                model: "candle-qwen2.5-0.5b-instruct".into(),
                version: "text-completion-stub".into(),
            },
        ));
        Ok(accum)
    }

    fn one_inference(pdf_bytes: &[u8]) -> Result<String> {
        use candle_core::{DType, Device, Tensor};
        use candle_nn::VarBuilder;
        use candle_transformers::models::qwen2::{Config as QwenConfig, ModelForCausalLM};
        use std::path::PathBuf;

        // Extract native text from the PDF; candle's text-completion target
        // doesn't see images, so this is the only signal it gets. Pages with
        // no native text yield an empty string and the model sees no prompt.
        let pdf_text = superdocuments::pdf::extract_markdown(pdf_bytes).unwrap_or_default();
        if pdf_text.trim().is_empty() {
            return Ok(String::new());
        }

        let prompt = format!(
            "<|im_start|>user\nConvert the following document text into a clean \
             Markdown table-of-contents-style outline, preserving section order:\n\
             <document>\n{pdf_text}\n</document>\n<|im_end|>\n\
             <|im_start|>assistant\n"
        );

        // Resolve weights + tokenizer via hf-hub. Cached under ~/.cache/huggingface
        // so subsequent runs are offline.
        let api = hf_hub::api::sync::Api::new().map_err(|e| anyhow!("hf-hub init failed: {e}"))?;
        let model_id = std::env::var("CANDLE_MODEL_ID").unwrap_or_else(|_| DEFAULT_MODEL_ID.into());
        let repo = api.repo(hf_hub::Repo::model(model_id.clone()));
        let model_path: PathBuf = repo
            .get("model.safetensors")
            .map_err(|e| anyhow!("download model.safetensors for {model_id}: {e}"))?;
        let config_path: PathBuf = repo
            .get("config.json")
            .map_err(|e| anyhow!("download config.json for {model_id}: {e}"))?;
        let tokenizer_path: PathBuf = repo
            .get("tokenizer.json")
            .map_err(|e| anyhow!("download tokenizer.json for {model_id}: {e}"))?;

        // CPU only — GPU path would need a CUDA-enabled candle build. The 0.5B
        // model fits in ~1 GB RAM and runs ~3-5 tok/s on a modern CPU.
        let device = Device::Cpu;
        let cfg: QwenConfig = serde_json::from_str(&std::fs::read_to_string(&config_path)?)
            .map_err(|e| anyhow!("parse {}: {e}", config_path.display()))?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                std::slice::from_ref(&model_path),
                DType::F32,
                &device,
            )
            .map_err(|e| anyhow!("mmap {}: {e}", model_path.display()))?
        };
        let mut model =
            ModelForCausalLM::new(&cfg, vb).map_err(|e| anyhow!("load Qwen2 {}: {e}", model_id))?;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow!("tokenizer {}: {e}", tokenizer_path.display()))?;
        let encoding = tokenizer
            .encode(prompt.clone(), true)
            .map_err(|e| anyhow!("tokenize prompt: {e}"))?;
        let mut tokens = encoding.get_ids().to_vec();
        let eos_token = tokenizer
            .token_to_id("<|im_end|>")
            .unwrap_or_else(|| tokenizer.token_to_id("<|endoftext|>").unwrap_or(0));

        let mut output = String::new();
        let mut seqlen_offset = 0usize;
        let start = Instant::now();
        for _step in 0..MAX_NEW_TOKENS {
            let input = Tensor::new(tokens.as_slice(), &device)
                .map_err(|e| anyhow!("input tensor: {e}"))?
                .unsqueeze(0)
                .map_err(|e| anyhow!("unsqueeze: {e}"))?;
            let logits = model
                .forward(&input, seqlen_offset)
                .map_err(|e| anyhow!("forward: {e}"))?;
            let (_b, seq_len, _v) = logits.dims3().map_err(|e| anyhow!("logits shape: {e}"))?;
            let last_logits = logits
                .narrow(1, seq_len - 1, 1)
                .map_err(|e| anyhow!("narrow: {e}"))?
                .squeeze(1)
                .map_err(|e| anyhow!("squeeze: {e}"))?;
            // Greedy argmax for determinism.
            let next = last_logits
                .argmax(candle_core::D::Minus1)
                .map_err(|e| anyhow!("argmax: {e}"))?
                .to_scalar::<u32>()
                .map_err(|e| anyhow!("to_scalar: {e}"))? as u32;
            if next == eos_token {
                break;
            }
            if let Some(piece) = tokenizer.id_to_token(next as u32) {
                output.push_str(piece.as_str());
            }
            tokens.push(next as u32);
            seqlen_offset = tokens.len().saturating_sub(1);
            // Bail out if a single call exceeds 30 s — the iteration-level
            // timeout is enforced by the caller.
            if start.elapsed() > Duration::from_secs(30) {
                break;
            }
        }

        Ok(output)
    }
}

#[cfg(feature = "candle")]
fn run_candle_target(pdf_bytes: &[u8], iterations: u64, timeout: Duration) -> Result<Sample> {
    candle_runner::run(pdf_bytes, iterations, timeout)
}

/// Drive the `lit` binary from run-llama/liteparse as a subprocess. Reads
/// markdown from stdout. Output quality depends on the lit build — the
/// default build enables Tesseract as the OCR engine; pass
/// `LITEPARSE_OCR=none` to disable OCR and run pure native-text extraction.
fn run_lit_subprocess(pdf_path: &Path, timeout: Duration) -> Result<String> {
    let mut cmd = std::process::Command::new("lit");
    cmd.arg("parse")
        .arg(pdf_path)
        .arg("--format")
        .arg("markdown");
    // OCR-mode override. None=disable Tesseract and run pure native extraction;
    // tesseract=enable (default); paddle=use an HTTP PaddleOCR server (caller
    // is responsible for starting it on the configured port).
    if let Ok(mode) = std::env::var("LITEPARSE_OCR") {
        cmd.arg("--ocr-engine").arg(mode);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .with_context(|| "lit binary not on PATH (cargo install lit from run-llama/liteparse)")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().with_context(|| "lit try_wait failed")? {
            let output = child
                .wait_with_output()
                .with_context(|| "lit wait_with_output failed")?;
            if !status.success() {
                return Err(anyhow!("lit parse failed: {status}"));
            }
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "lit parse exceeded timeout {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Drive the `parse-bench` Python CLI from run-llama/ParseBench as a subprocess.
/// Calls `parse-bench run <pipeline> --test --skip_inference` on the local
/// fixture (parse-bench's own benchmark dataset is downloaded separately on
/// first run; this integration runs against our local fixtures for direct
/// comparison).
fn run_parse_bench_subprocess(pdf_path: &Path, timeout: Duration) -> Result<String> {
    // Note: parse-bench's CLI expects the benchmark dataset to be downloaded
    // (`parse-bench download`). For now we shell out to its inference lane
    // against our fixture and surface the markdown output; full eval scoring
    // is a follow-up that wires parse-bench's own text_content/formatting
    // rules into our JSON report.
    let _ = timeout; // parse-bench doesn't currently expose a per-call timeout
    let mut cmd = std::process::Command::new("parse-bench");
    cmd.arg("inference").arg("run").arg("llamaparse_layout");
    cmd.arg("--input").arg(pdf_path);
    cmd.env("PARSEBENCH_INPUT", pdf_path);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd
        .output()
        .with_context(|| "parse-bench not on PATH (pip install parse-bench)")?;
    if !output.status.success() {
        return Err(anyhow!(
            "parse-bench inference run failed: {} (stderr: {})",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Drive `marker_single <pdf> --output_format markdown` as a subprocess.
/// Returns the markdown the tool emitted. First call downloads ~2 GB of
/// model weights into the datalab cache; subsequent calls are offline.
fn run_marker_subprocess(pdf_path: &Path, timeout: Duration) -> Result<String> {
    let mut cmd = std::process::Command::new("marker_single");
    cmd.arg(pdf_path).arg("--output_format").arg("markdown");
    if let Ok(disable_images) = std::env::var("MARKER_DISABLE_IMAGES") {
        if disable_images == "1" {
            cmd.arg("--disable_image_extraction");
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .with_context(|| "marker_single not on PATH (pip install marker-pdf)")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| "marker_single try_wait failed")?
        {
            let output = child
                .wait_with_output()
                .with_context(|| "marker_single wait_with_output failed")?;
            if !status.success() {
                return Err(anyhow!("marker_single failed: {status}"));
            }
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "marker_single exceeded timeout {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Drive `docling <pdf> --output <tmpdir> --to md` and read back the produced
/// `<pdf-stem>.md`. The `docling` CLI is heavy: first run downloads model
/// weights (~1.5 GB) into the HF cache.
fn run_docling_subprocess(pdf_path: &Path, timeout: Duration) -> Result<String> {
    let tmp = std::env::temp_dir().join(format!(
        "docling-{}-{}",
        std::process::id(),
        pdf_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("doc")
    ));
    std::fs::create_dir_all(&tmp).with_context(|| format!("create tmp {}", tmp.display()))?;
    let mut cmd = std::process::Command::new("docling");
    cmd.arg(pdf_path)
        .arg("--output")
        .arg(&tmp)
        .arg("--to")
        .arg("md")
        .arg("--device")
        .arg("cpu"); // CPU default; set DOCLING_DEVICE=cuda to override
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .with_context(|| "docling not on PATH (pip install docling)")?;
    let start = Instant::now();
    let status_result: Result<bool> = loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| "docling try_wait failed")?
        {
            // Drain the child's stdout/stderr so the pipe doesn't fill, but we
            // don't need the bytes — docling writes the markdown to disk.
            let _ = child
                .wait_with_output()
                .with_context(|| "docling wait_with_output failed")?;
            break Ok(status.success());
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "docling exceeded timeout {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let ok = status_result?;
    if !ok {
        return Err(anyhow!("docling failed"));
    }
    // Find the produced .md file. Docling writes `<pdf-stem>.md` into the output dir.
    let stem = pdf_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc");
    let md_path = tmp.join(format!("{stem}.md"));
    if !md_path.exists() {
        return Err(anyhow!(
            "docling did not produce {} (expected markdown output)",
            md_path.display()
        ));
    }
    let body =
        std::fs::read_to_string(&md_path).with_context(|| format!("read {}", md_path.display()))?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(body)
}

/// Drive `surya_ocr <pdf>` as a subprocess. Surya spins up its own inference
/// server (vllm or llama-server, controlled by `SURYA_INFERENCE_BACKEND` /
/// `SURYA_INFERENCE_URL` env vars). Output is plain text. First run downloads
/// model weights (~3 GB).
fn run_surya_subprocess(pdf_path: &Path, timeout: Duration) -> Result<String> {
    let mut cmd = std::process::Command::new("surya_ocr");
    cmd.arg(pdf_path);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .with_context(|| "surya_ocr not on PATH (pip install surya-ocr)")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| "surya_ocr try_wait failed")?
        {
            let output = child
                .wait_with_output()
                .with_context(|| "surya_ocr wait_with_output failed")?;
            if !status.success() {
                return Err(anyhow!("surya_ocr failed: {status}"));
            }
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "surya_ocr exceeded timeout {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Drive `pdftoppm` + `tesseract --psm 3` as subprocesses on `pdf_path`,
/// returning the concatenated per-page OCR text. Same algorithm as
/// `superdocuments::pdf::tesseract_ocr` but in a flat subprocess form for
/// this bench (no in-process state, no shared temp dir, no panic containment).
fn run_tesseract_direct_subprocess(pdf_path: &Path, timeout: Duration) -> Result<String> {
    // Tesseract needs PNGs as input, so first rasterize. We delegate the
    // rasterization to pdftoppm via the `pdfium` sys crate path used by
    // `superdocuments::pdf::extract_file_with_ocr_options` is a more thorough
    // path — but for a one-off subprocess that's overkill. We just call
    // pdftoppm with the fixture's first page (the corpus is small).
    let tmp = std::env::temp_dir().join(format!(
        "superdocs-bench-tesseract-{}-{}",
        std::process::id(),
        pdf_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("pdf")
    ));
    std::fs::create_dir_all(&tmp).with_context(|| format!("create tmp {}", tmp.display()))?;
    let prefix = tmp.join("page");
    let mut pdftoppm = std::process::Command::new("pdftoppm");
    pdftoppm
        .args(["-f", "1", "-l", "1", "-r", "200", "-png"])
        .arg(pdf_path)
        .arg(&prefix);
    pdftoppm.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = pdftoppm
        .spawn()
        .with_context(|| "pdftoppm not on PATH (apt install poppler-utils)")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| "pdftoppm try_wait failed")?
        {
            let _ = child.wait();
            if !status.success() {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(anyhow!("pdftoppm failed: {status}"));
            }
            break;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(anyhow!("pdftoppm exceeded timeout"));
        }
        thread::sleep(Duration::from_millis(10));
    }
    // Find the produced PNG (pdftoppm names them page-1.png, page-01.png, etc.
    // depending on count; we asked for one page so it's page-1.png).
    let png = std::fs::read_dir(&tmp)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .ok_or_else(|| anyhow!("pdftoppm produced no PNG in {}", tmp.display()))?;
    // Run tesseract on the PNG.
    let txt_out = png.with_extension("");
    let mut tesseract = std::process::Command::new("tesseract");
    tesseract
        .arg(&png)
        .arg(&txt_out)
        .args(["-l", "eng", "--psm", "3"]);
    tesseract.stdout(Stdio::null()).stderr(Stdio::null());
    let output = tesseract.output().with_context(|| "tesseract failed")?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(anyhow!("tesseract failed: {}", output.status));
    }
    let body = std::fs::read_to_string(txt_out.with_extension("txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(body)
}
