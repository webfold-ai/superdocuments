//! In-process PDF-to-Markdown extraction via Transcode's parser, plus the legacy
//! `pdffold` process adapter for callers that still need it.
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub mod acroform;
pub mod geometry;
pub mod page_walker;
pub mod parser;
pub mod transcode_populator;

/// Bound parser input before scanning attacker-controlled bytes.
pub const MAX_PDF_BYTES: usize = 256 * 1024 * 1024;
const MAX_OCR_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_OCR_RENDER_BYTES: u64 = 512 * 1024 * 1024;

/// Read and parse a PDF file without spawning a process.
pub fn extract_file(path: impl AsRef<Path>) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_PDF_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF exceeds size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PDF_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    extract_markdown(&bytes)
}

/// Parse PDF bytes and return layout-aware Markdown without spawning a process.
///
/// The parser handles born-digital text PDFs and FlateDecode content streams. It
/// does not OCR image-only pages; those produce no text (they still get their
/// `<!-- page N -->` marker, so the hole is visible to a page-aware consumer).
/// Malformed input and parser panics are returned as errors rather than escaping
/// to the caller.
///
/// Every page's text is preceded by a `<!-- page N -->` marker — byte-identical to
/// the marker the OCR path emits — so downstream consumers recover page ranges with
/// [`split_page_markers`] without re-parsing the PDF. Use [`extract_markdown_flat`]
/// for the historical marker-less projection.
pub fn extract_markdown(bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > MAX_PDF_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF exceeds size limit",
        ));
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid PDF header",
        ));
    }
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        let document =
            crate::pdf::parser::Document::parse(bytes).map_err(|error| match error {
                crate::pdf::parser::PdfError::Parse(message) => message,
            })?;
        if document.object_count() == 0 || document.root() == 0 {
            return Err("PDF has no readable catalog".to_string());
        }
        Ok(document.markdown())
    }))
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PDF parser panicked"))?;
    parsed.map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
}

/// [`extract_markdown`] without the `<!-- page N -->` markers — the historical
/// single-blob projection, kept additive so whole-document comparisons keep working.
pub fn extract_markdown_flat(bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > MAX_PDF_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF exceeds size limit",
        ));
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid PDF header",
        ));
    }
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        let document =
            crate::pdf::parser::Document::parse(bytes).map_err(|error| match error {
                crate::pdf::parser::PdfError::Parse(message) => message,
            })?;
        if document.object_count() == 0 || document.root() == 0 {
            return Err("PDF has no readable catalog".to_string());
        }
        Ok(document.markdown_flat())
    }))
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PDF parser panicked"))?;
    parsed.map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
}

// ─── page splitting / merging (the shared page-anchor convention) ─────────────────────────────

/// One page's text, recovered from the `<!-- page N -->` markers that both the native
/// parser ([`extract_markdown`]) and the Poppler + Tesseract path
/// ([`tesseract_ocr`]) emit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageText {
    pub number: u32,
    /// The page body with its marker stripped. Empty for a page with no text layer.
    pub text: String,
}

/// Split marker-bearing markdown into its pages. Text appearing before the first marker
/// becomes page 1; a page with an empty body is still returned (that is how a scanned
/// page shows up). Marker numbers are taken verbatim, so the result is exactly the page
/// sequence the emitting parser walked; a trailing text run that is only whitespace does
/// not invent a page.
pub fn split_page_markers(markdown: &str) -> Vec<PageText> {
    let mut pages: Vec<PageText> = Vec::new();
    let mut current = String::new();
    let mut number: Option<u32> = None;
    for line in markdown.lines() {
        if let Some(marker) = crate::pdf::parser::parse_page_marker(line) {
            // A marker closes the page before it and opens the next one.
            if number.is_some() || !current.trim().is_empty() {
                pages.push(PageText {
                    number: number.unwrap_or(1),
                    text: std::mem::take(&mut current),
                });
            }
            number = Some(marker);
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if number.is_some() || !current.trim().is_empty() {
        pages.push(PageText {
            number: number.unwrap_or(1),
            text: current,
        });
    }
    pages
}

/// Re-join pages into marker-bearing markdown (the inverse of [`split_page_markers`]).
/// Pages with no text at all are skipped, matching the OCR path's convention of only
/// emitting a marker for a page it actually recovered text for.
pub fn merge_page_markers(pages: &[PageText]) -> String {
    let mut out = String::new();
    for page in pages {
        let text = page.text.trim();
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::pdf::parser::page_marker(page.number as usize));
        out.push_str(text);
    }
    out
}

/// Does this page's text carry a real (non-image) text layer? The per-page half of
/// [`has_embedded_text`] — same predicate, applied to one page instead of the document.
pub fn page_has_embedded_text(text: &str) -> bool {
    has_embedded_text(text)
}

/// Fill the pages that have no text layer from an injected per-page OCR callback, then
/// merge every page back in page order with its marker.
///
/// The callback receives the 1-based page number and returns that page's recognized
/// text. A page whose native text is already usable is never OCR'd — that is the whole
/// point of the per-page decision: a mixed born-digital + scanned PDF pays OCR only for
/// the scanned pages. If every OCR'd page fails, the first error is returned; a partial
/// failure keeps the pages that did come back (fail-soft per page, fail-closed overall).
pub fn merge_ocr_pages(
    pages: &[PageText],
    ocr_page: &mut impl FnMut(u32) -> io::Result<String>,
) -> io::Result<String> {
    let mut merged: Vec<PageText> = Vec::with_capacity(pages.len());
    let mut errors = 0usize;
    let mut attempted = 0usize;
    let mut first_error: Option<io::Error> = None;
    for page in pages {
        if page_has_embedded_text(&page.text) {
            merged.push(page.clone());
            continue;
        }
        attempted += 1;
        match ocr_page(page.number) {
            Ok(text) => merged.push(PageText {
                number: page.number,
                text,
            }),
            Err(error) => {
                errors += 1;
                if first_error.is_none() {
                    first_error = Some(error);
                }
                // Keep the page slot with whatever native text it had (usually nothing) so
                // page numbering downstream stays aligned with the PDF.
                merged.push(page.clone());
            }
        }
    }
    if attempted > 0 && errors == attempted {
        return Err(first_error.unwrap_or_else(|| {
            io::Error::other("every OCR page failed")
        }));
    }
    Ok(merge_page_markers(&merged))
}

/// Runtime limits for the optional Poppler + Tesseract OCR fallback.
#[derive(Clone, Debug)]
pub struct OcrOptions {
    /// Tesseract language codes, e.g. `eng` or `eng+deu`.
    pub languages: String,
    /// Maximum rendered pages. PDFs beyond this limit return an explicit truncation note.
    pub max_pages: u32,
    /// Maximum edge length of each rendered page in pixels.
    pub max_image_dimension: u32,
    /// Shared wall-clock budget for rendering and OCR.
    pub timeout: Duration,
    /// Optional observability sink — counters/gauges/histograms plus an append-only JSONL
    /// log. Both default to no-op so callers without metrics infrastructure are unaffected.
    pub config: OcrConfig,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            languages: "eng".into(),
            max_pages: 20,
            max_image_dimension: 2_400,
            timeout: Duration::from_secs(120),
            config: OcrConfig::default(),
        }
    }
}

/// Observability hooks for the OCR fallback. Counters/gauges/histograms and the JSONL log
/// are each optional and independent — pass an `OcrMetrics` to record into Prometheus-shaped
/// state, pass a `log_path` to append one JSON line per request, neither, or both.
///
/// The histogram bucket layout matches `DEFAULT_LATENCY_BOUNDS` (seconds) so callers that
/// already render Webfold's `Registry` get a consistent percentile shape end to end.
#[derive(Clone, Debug, Default)]
pub struct OcrConfig {
    pub metrics: Option<Arc<OcrMetrics>>,
    pub log_path: Option<PathBuf>,
}

/// Per-request OCR observability state. Cheap to clone (`Arc` over atomics + the inner
/// histogram mutexes), safe to share across threads. `render()` produces the Prometheus text
/// exposition format directly so the Webfold `/metrics` endpoint can splice it in.
#[derive(Debug)]
pub struct OcrMetrics {
    requests_total: AtomicU64,
    native_hits_total: AtomicU64,
    ocr_attempts_total: AtomicU64,
    ocr_success_total: AtomicU64,
    ocr_failure_total: AtomicU64,
    truncated_total: AtomicU64,
    rendered_pages_total: AtomicU64,
    rendered_bytes_total: AtomicU64,
    ocr_bytes_total: AtomicU64,
    inflight: AtomicI64,
    /// Peak working-set estimate in bytes: the largest sum of on-disk PNG sizes any
    /// in-flight request has rendered so far. Monotonic; resets are unnecessary for a peak
    /// gauge and would only hide regressions.
    peak_resident_bytes: AtomicI64,
    request_seconds: Mutex<HistogramState>,
    render_seconds: Mutex<HistogramState>,
    recognize_seconds: Mutex<HistogramState>,
}

#[derive(Debug)]
struct HistogramState {
    /// Cumulative per-bucket counts (`le`-style). One mutex per histogram so concurrent
    /// observations across histograms don't contend on the same lock.
    bucket_le: Vec<f64>,
    bucket_count: Vec<u64>,
    sum: f64,
    count: u64,
}

impl HistogramState {
    fn new(bounds: &[f64]) -> Self {
        Self {
            bucket_le: bounds.to_vec(),
            bucket_count: vec![0; bounds.len()],
            sum: 0.0,
            count: 0,
        }
    }

    fn observe(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        // Per-bucket counts are non-cumulative; Prometheus's cumulative shape is rendered
        // at output time. The first bucket whose bound is >= value gets the observation.
        // A value past every finite bound falls only into the implicit `+Inf` bucket.
        for (i, bound) in self.bucket_le.iter().enumerate() {
            if value <= *bound {
                self.bucket_count[i] += 1;
                break;
            }
        }
    }
}

/// Latency bucket layout, seconds. Matches Webfold's `DEFAULT_LATENCY_BOUNDS` so any
/// Prometheus panel that already covers HTTP latency lines up against the OCR histograms
/// without re-bucketing.
pub const OCR_LATENCY_BOUNDS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

impl OcrMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            requests_total: AtomicU64::new(0),
            native_hits_total: AtomicU64::new(0),
            ocr_attempts_total: AtomicU64::new(0),
            ocr_success_total: AtomicU64::new(0),
            ocr_failure_total: AtomicU64::new(0),
            truncated_total: AtomicU64::new(0),
            rendered_pages_total: AtomicU64::new(0),
            rendered_bytes_total: AtomicU64::new(0),
            ocr_bytes_total: AtomicU64::new(0),
            inflight: AtomicI64::new(0),
            peak_resident_bytes: AtomicI64::new(0),
            request_seconds: Mutex::new(HistogramState::new(&OCR_LATENCY_BOUNDS)),
            render_seconds: Mutex::new(HistogramState::new(&OCR_LATENCY_BOUNDS)),
            recognize_seconds: Mutex::new(HistogramState::new(&OCR_LATENCY_BOUNDS)),
        })
    }

    fn begin_request(self: &Arc<Self>) -> RequestGuard {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        self.inflight.fetch_add(1, Ordering::Relaxed);
        RequestGuard {
            metrics: self.clone(),
            started: Instant::now(),
        }
    }

    fn record_native_hit(&self) {
        self.native_hits_total.fetch_add(1, Ordering::Relaxed);
    }

    fn record_render(&self, seconds: f64, pages: u64, bytes: u64) {
        self.rendered_pages_total
            .fetch_add(pages, Ordering::Relaxed);
        self.rendered_bytes_total
            .fetch_add(bytes, Ordering::Relaxed);
        // Monotonic peak of working-set bytes across in-flight requests.
        let mut current = self.peak_resident_bytes.load(Ordering::Relaxed);
        while (bytes as i64) > current {
            match self.peak_resident_bytes.compare_exchange_weak(
                current,
                bytes as i64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        self.render_seconds
            .lock()
            .expect("render histogram mutex poisoned")
            .observe(seconds);
    }

    fn record_recognize(&self, seconds: f64, bytes: u64) {
        self.ocr_bytes_total.fetch_add(bytes, Ordering::Relaxed);
        self.recognize_seconds
            .lock()
            .expect("recognize histogram mutex poisoned")
            .observe(seconds);
    }

    fn record_outcome(&self, success: bool, truncated: bool) {
        if success {
            self.ocr_success_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.ocr_failure_total.fetch_add(1, Ordering::Relaxed);
        }
        if truncated {
            self.truncated_total.fetch_add(1, Ordering::Relaxed);
        }
        self.ocr_attempts_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Render every metric in Prometheus text exposition format. Names are stable and
    /// sort-ordered for diffable output.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let counters: [(&str, u64); 6] = [
            (
                "superdocuments_ocr_requests_total",
                self.requests_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_native_hits_total",
                self.native_hits_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_attempts_total",
                self.ocr_attempts_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_success_total",
                self.ocr_success_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_failure_total",
                self.ocr_failure_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_truncated_total",
                self.truncated_total.load(Ordering::Relaxed),
            ),
        ];
        for (name, value) in counters {
            writeln!(out, "# TYPE {name} counter\n{name} {value}").unwrap();
        }
        let sum_counters: [(&str, u64); 3] = [
            (
                "superdocuments_ocr_rendered_pages_total",
                self.rendered_pages_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_rendered_bytes_total",
                self.rendered_bytes_total.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_output_bytes_total",
                self.ocr_bytes_total.load(Ordering::Relaxed),
            ),
        ];
        for (name, value) in sum_counters {
            writeln!(out, "# TYPE {name} counter\n{name} {value}").unwrap();
        }
        let gauges: [(&str, i64); 2] = [
            (
                "superdocuments_ocr_inflight",
                self.inflight.load(Ordering::Relaxed),
            ),
            (
                "superdocuments_ocr_peak_resident_bytes",
                self.peak_resident_bytes.load(Ordering::Relaxed),
            ),
        ];
        for (name, value) in gauges {
            writeln!(out, "# TYPE {name} gauge\n{name} {value}").unwrap();
        }
        let histograms = [
            ("superdocuments_ocr_request_seconds", &self.request_seconds),
            ("superdocuments_ocr_render_seconds", &self.render_seconds),
            (
                "superdocuments_ocr_recognize_seconds",
                &self.recognize_seconds,
            ),
        ];
        for (name, state) in histograms {
            writeln!(out, "# TYPE {name} histogram").unwrap();
            let state = state.lock().expect("histogram mutex poisoned");
            let mut cumulative = 0u64;
            for (bound, count) in state.bucket_le.iter().zip(state.bucket_count.iter()) {
                cumulative += count;
                writeln!(out, "{name}_bucket{{le=\"{bound}\"}} {cumulative}").unwrap();
            }
            writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {}", state.count).unwrap();
            writeln!(out, "{name}_sum {}", state.sum).unwrap();
            writeln!(out, "{name}_count {}", state.count).unwrap();
        }
        out
    }

    /// Snapshot the per-phase histogram sums and counts. Useful for benchmarks that
    /// want to attribute wall time to the request, render, and recognize phases without
    /// re-parsing the Prometheus text. Cheap; takes three mutexes briefly.
    pub fn snapshot(&self) -> PhaseSnapshot {
        let request = self
            .request_seconds
            .lock()
            .expect("request histogram mutex poisoned");
        let render = self
            .render_seconds
            .lock()
            .expect("render histogram mutex poisoned");
        let recognize = self
            .recognize_seconds
            .lock()
            .expect("recognize histogram mutex poisoned");
        PhaseSnapshot {
            requests_total: self.requests_total.load(Ordering::Relaxed),
            native_hits_total: self.native_hits_total.load(Ordering::Relaxed),
            ocr_attempts_total: self.ocr_attempts_total.load(Ordering::Relaxed),
            ocr_success_total: self.ocr_success_total.load(Ordering::Relaxed),
            ocr_failure_total: self.ocr_failure_total.load(Ordering::Relaxed),
            truncated_total: self.truncated_total.load(Ordering::Relaxed),
            rendered_pages_total: self.rendered_pages_total.load(Ordering::Relaxed),
            rendered_bytes_total: self.rendered_bytes_total.load(Ordering::Relaxed),
            ocr_bytes_total: self.ocr_bytes_total.load(Ordering::Relaxed),
            inflight: self.inflight.load(Ordering::Relaxed),
            peak_resident_bytes: self.peak_resident_bytes.load(Ordering::Relaxed),
            request_seconds_sum: request.sum,
            request_seconds_count: request.count,
            render_seconds_sum: render.sum,
            render_seconds_count: render.count,
            recognize_seconds_sum: recognize.sum,
            recognize_seconds_count: recognize.count,
        }
    }
}

/// Counters + per-phase sums/counts at a moment in time. Subtracting two snapshots
/// yields the work done between them — the pattern the OCR testbench uses to attribute
/// wall time to render vs recognize without instrumenting the runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseSnapshot {
    pub requests_total: u64,
    pub native_hits_total: u64,
    pub ocr_attempts_total: u64,
    pub ocr_success_total: u64,
    pub ocr_failure_total: u64,
    pub truncated_total: u64,
    pub rendered_pages_total: u64,
    pub rendered_bytes_total: u64,
    pub ocr_bytes_total: u64,
    pub inflight: i64,
    pub peak_resident_bytes: i64,
    pub request_seconds_sum: f64,
    pub request_seconds_count: u64,
    pub render_seconds_sum: f64,
    pub render_seconds_count: u64,
    pub recognize_seconds_sum: f64,
    pub recognize_seconds_count: u64,
}

impl PhaseSnapshot {
    /// Element-wise subtraction: `self - earlier`. Saturates at zero on unsigned fields.
    pub fn delta(&self, earlier: &Self) -> Self {
        Self {
            requests_total: self.requests_total.saturating_sub(earlier.requests_total),
            native_hits_total: self
                .native_hits_total
                .saturating_sub(earlier.native_hits_total),
            ocr_attempts_total: self
                .ocr_attempts_total
                .saturating_sub(earlier.ocr_attempts_total),
            ocr_success_total: self
                .ocr_success_total
                .saturating_sub(earlier.ocr_success_total),
            ocr_failure_total: self
                .ocr_failure_total
                .saturating_sub(earlier.ocr_failure_total),
            truncated_total: self.truncated_total.saturating_sub(earlier.truncated_total),
            rendered_pages_total: self
                .rendered_pages_total
                .saturating_sub(earlier.rendered_pages_total),
            rendered_bytes_total: self
                .rendered_bytes_total
                .saturating_sub(earlier.rendered_bytes_total),
            ocr_bytes_total: self.ocr_bytes_total.saturating_sub(earlier.ocr_bytes_total),
            inflight: self.inflight - earlier.inflight,
            peak_resident_bytes: self.peak_resident_bytes.max(earlier.peak_resident_bytes),
            request_seconds_sum: (self.request_seconds_sum - earlier.request_seconds_sum).max(0.0),
            request_seconds_count: self
                .request_seconds_count
                .saturating_sub(earlier.request_seconds_count),
            render_seconds_sum: (self.render_seconds_sum - earlier.render_seconds_sum).max(0.0),
            render_seconds_count: self
                .render_seconds_count
                .saturating_sub(earlier.render_seconds_count),
            recognize_seconds_sum: (self.recognize_seconds_sum - earlier.recognize_seconds_sum)
                .max(0.0),
            recognize_seconds_count: self
                .recognize_seconds_count
                .saturating_sub(earlier.recognize_seconds_count),
        }
    }
}

impl Default for OcrMetrics {
    fn default() -> Self {
        // `OcrMetrics::new()` returns `Arc<Self>` because the common case needs shared
        // ownership across the request guard, the metrics sink, and the call site. The
        // bare `Default` impl is here only so `Arc<OcrMetrics>: Default` continues to work
        // — callers should prefer `OcrMetrics::new()` directly.
        Self {
            requests_total: AtomicU64::new(0),
            native_hits_total: AtomicU64::new(0),
            ocr_attempts_total: AtomicU64::new(0),
            ocr_success_total: AtomicU64::new(0),
            ocr_failure_total: AtomicU64::new(0),
            truncated_total: AtomicU64::new(0),
            rendered_pages_total: AtomicU64::new(0),
            rendered_bytes_total: AtomicU64::new(0),
            ocr_bytes_total: AtomicU64::new(0),
            inflight: AtomicI64::new(0),
            peak_resident_bytes: AtomicI64::new(0),
            request_seconds: Mutex::new(HistogramState::new(&OCR_LATENCY_BOUNDS)),
            render_seconds: Mutex::new(HistogramState::new(&OCR_LATENCY_BOUNDS)),
            recognize_seconds: Mutex::new(HistogramState::new(&OCR_LATENCY_BOUNDS)),
        }
    }
}

/// RAII guard for a single OCR request. Drops the inflight gauge and records total
/// latency on drop; sub-phases record into their own histograms explicitly.
struct RequestGuard {
    metrics: Arc<OcrMetrics>,
    started: Instant,
}

impl RequestGuard {
    fn finish(self, outcome: RequestOutcome) -> Arc<OcrMetrics> {
        let elapsed = self.started.elapsed().as_secs_f64();
        self.metrics
            .request_seconds
            .lock()
            .expect("request histogram mutex poisoned")
            .observe(elapsed);
        self.metrics.inflight.fetch_sub(1, Ordering::Relaxed);
        let metrics = self.metrics.clone();
        let _ = outcome; // recorded through `record_outcome` from the call site
        std::mem::forget(self); // avoid running Drop twice; fields are intentionally inert
        metrics
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        // Safety net for early-return paths that don't call `finish`. The histogram for
        // total latency is best-effort here; sub-phases are still observed explicitly.
        self.metrics.inflight.fetch_sub(1, Ordering::Relaxed);
        let elapsed = self.started.elapsed().as_secs_f64();
        let mut state = self
            .metrics
            .request_seconds
            .lock()
            .expect("request histogram mutex poisoned");
        state.observe(elapsed);
    }
}

struct RequestOutcome;

/// One JSONL line per request. Written from a small inline encoder to avoid a `serde_json`
/// dependency for what is six numbers and three booleans. Fields are stable; new ones go on
/// the end. `event` is `request_started`, `request_finished`, or `flamegraph` (one per phase,
/// for `flamegraph --flamechart` consumers that expect stage timings).
#[derive(Debug)]
enum LogEvent<'a> {
    Started {
        path: &'a Path,
        bytes: u64,
        languages: &'a str,
        max_pages: u32,
        max_image_dimension: u32,
        timeout_ms: u64,
    },
    Finished {
        outcome: &'a str,
        used_ocr: bool,
        truncated: bool,
        pages_rendered: u64,
        bytes_rendered: u64,
        bytes_ocr: u64,
        elapsed_ms: u64,
        render_ms: u64,
        recognize_ms: u64,
        inflight: u32,
    },
    Flame {
        stage: &'a str,
        elapsed_us: u64,
        pages: u32,
    },
}

fn write_log_line(path: &Path, event: LogEvent<'_>) -> io::Result<()> {
    let mut line = String::with_capacity(256);
    // Event name is written as the value of `event` directly (not via `push_str`,
    // which expects to be the *first field after* the opening brace). Subsequent
    // fields are appended with a leading comma.
    line.push_str("{\"event\":\"");
    match &event {
        LogEvent::Started { .. } => line.push_str("request_started"),
        LogEvent::Finished { .. } => line.push_str("request_finished"),
        LogEvent::Flame { .. } => line.push_str("flamegraph"),
    }
    line.push('"');

    match event {
        LogEvent::Started {
            path: input_path,
            bytes,
            languages,
            max_pages,
            max_image_dimension,
            timeout_ms,
        } => {
            push_str_value(&mut line, "path", &input_path.display().to_string());
            push_u64(&mut line, "bytes", bytes);
            push_str_value(&mut line, "languages", languages);
            push_u64(&mut line, "max_pages", u64::from(max_pages));
            push_u64(
                &mut line,
                "max_image_dimension",
                u64::from(max_image_dimension),
            );
            push_u64(&mut line, "timeout_ms", timeout_ms);
        }
        LogEvent::Finished {
            outcome,
            used_ocr,
            truncated,
            pages_rendered,
            bytes_rendered,
            bytes_ocr,
            elapsed_ms,
            render_ms,
            recognize_ms,
            inflight,
        } => {
            push_str_value(&mut line, "outcome", outcome);
            push_bool(&mut line, "used_ocr", used_ocr);
            push_bool(&mut line, "truncated", truncated);
            push_u64(&mut line, "pages_rendered", pages_rendered);
            push_u64(&mut line, "bytes_rendered", bytes_rendered);
            push_u64(&mut line, "bytes_ocr", bytes_ocr);
            push_u64(&mut line, "elapsed_ms", elapsed_ms);
            push_u64(&mut line, "render_ms", render_ms);
            push_u64(&mut line, "recognize_ms", recognize_ms);
            push_u64(&mut line, "inflight", u64::from(inflight));
        }
        LogEvent::Flame {
            stage,
            elapsed_us,
            pages,
        } => {
            push_str_value(&mut line, "stage", stage);
            push_u64(&mut line, "elapsed_us", elapsed_us);
            push_u64(&mut line, "pages", u64::from(pages));
        }
    }
    line.push_str("}\n");

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

/// Append `,"key":"value"` to a JSON object line. Caller is responsible for emitting the
/// leading `{` and the `event` field.
fn push_str_value(out: &mut String, key: &str, value: &str) {
    out.push(',');
    out.push('"');
    push_json_string(out, key);
    out.push_str("\":\"");
    push_json_string(out, value);
    out.push('"');
}

fn push_u64(out: &mut String, key: &str, value: u64) {
    out.push(',');
    out.push('"');
    push_json_string(out, key);
    out.push_str("\":");
    let _ = write!(out, "{value}");
}

fn push_bool(out: &mut String, key: &str, value: bool) {
    out.push(',');
    out.push('"');
    push_json_string(out, key);
    out.push_str("\":");
    out.push_str(if value { "true" } else { "false" });
}

fn push_json_string(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

impl OcrOptions {
    fn validate(&self) -> io::Result<()> {
        if self.languages.is_empty()
            || self.languages.len() > 64
            || !self
                .languages
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'+')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid OCR language list",
            ));
        }
        if !(1..=100).contains(&self.max_pages) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "OCR page limit must be 1..=100",
            ));
        }
        if !(256..=4_000).contains(&self.max_image_dimension) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "OCR image dimension must be 256..=4000",
            ));
        }
        let render_bound = u64::from(self.max_pages)
            .saturating_mul(u64::from(self.max_image_dimension))
            .saturating_mul(u64::from(self.max_image_dimension))
            .saturating_mul(4);
        if render_bound > MAX_OCR_RENDER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "OCR page and pixel limits exceed the rendered-data budget",
            ));
        }
        if self.timeout.is_zero() || self.timeout > Duration::from_secs(600) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "OCR timeout must be 1..=600 seconds",
            ));
        }
        Ok(())
    }
}

/// Extract native PDF text first; when the PDF has no readable text layer, OCR it
/// with `pdftoppm` and `tesseract` from `PATH`. Install Poppler utilities and
/// Tesseract (plus the requested language data) to enable this fallback.
pub fn extract_file_with_ocr(path: impl AsRef<Path>) -> io::Result<String> {
    extract_file_with_ocr_options(path, &OcrOptions::default())
}

/// Same as [`extract_file_with_ocr`] with explicit language, page, pixel, and time limits.
pub fn extract_file_with_ocr_options(
    path: impl AsRef<Path>,
    options: &OcrOptions,
) -> io::Result<String> {
    let path = path.as_ref();
    let guard = options
        .config
        .metrics
        .as_ref()
        .map(|metrics| metrics.begin_request());
    let started = Instant::now();
    let input_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if let Some(log_path) = options.config.log_path.as_deref() {
        let _ = write_log_line(
            log_path,
            LogEvent::Started {
                path,
                bytes: input_bytes,
                languages: &options.languages,
                max_pages: options.max_pages,
                max_image_dimension: options.max_image_dimension,
                timeout_ms: options.timeout.as_millis() as u64,
            },
        );
    }
    let timings: Arc<Mutex<PhaseTimings>> = Arc::new(Mutex::new(PhaseTimings::default()));
    let timings_for_observer = Arc::clone(&timings);
    let metrics_for_observer = options.config.metrics.clone();
    let result = extract_file_with_ocr_inner(
        path,
        options,
        guard.as_ref().map(|g| g.metrics.clone()),
        move |stage, seconds, count, bytes| {
            let mut t = timings_for_observer
                .lock()
                .expect("phase timings mutex poisoned");
            match stage {
                "render_done" => {
                    t.render_seconds = seconds;
                    t.pages_rendered = count;
                    t.bytes_rendered = bytes;
                    if let Some(metrics) = metrics_for_observer.as_ref() {
                        metrics.record_render(seconds, count, bytes);
                    }
                }
                "page_recognize" => {
                    t.recognize_seconds += seconds;
                    if let Some(metrics) = metrics_for_observer.as_ref() {
                        metrics.record_recognize(seconds, bytes);
                    }
                }
                "recognize_done" => {
                    t.recognize_seconds = seconds;
                }
                _ => {}
            }
        },
        Arc::clone(&timings),
    );
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if let Some(guard) = guard {
        let _ = guard.finish(RequestOutcome);
    }
    if let Some(log_path) = options.config.log_path.as_deref() {
        let inflight = options
            .config
            .metrics
            .as_ref()
            .map(|m| m.inflight.load(Ordering::Relaxed))
            .unwrap_or(0);
        let timings_snapshot = timings
            .lock()
            .expect("phase timings mutex poisoned")
            .clone();
        let (outcome, used_ocr, truncated) = match &result {
            Ok(markdown) if has_embedded_text(markdown) => {
                if !timings_snapshot.used_ocr {
                    ("native", false, false)
                } else {
                    ("ocr", true, timings_snapshot.truncated)
                }
            }
            Ok(_) => ("ocr", true, timings_snapshot.truncated),
            Err(_) => (
                if timings_snapshot.used_ocr {
                    "ocr_error"
                } else {
                    "native_error"
                },
                timings_snapshot.used_ocr,
                false,
            ),
        };
        // `error` is borrowed but unused here — capture kind+message for the log line.
        let _ = result.as_ref().err();
        let _ = outcome;
        let inflight_u32 = inflight.max(0) as u32;
        let event = LogEvent::Finished {
            outcome: match &result {
                Ok(markdown) if has_embedded_text(markdown) && !timings_snapshot.used_ocr => {
                    "native"
                }
                Ok(_) => "ocr",
                Err(_) => "error",
            },
            used_ocr,
            truncated,
            pages_rendered: timings_snapshot.pages_rendered,
            bytes_rendered: timings_snapshot.bytes_rendered,
            bytes_ocr: timings_snapshot.bytes_ocr,
            elapsed_ms,
            render_ms: (timings_snapshot.render_seconds * 1000.0) as u64,
            recognize_ms: (timings_snapshot.recognize_seconds * 1000.0) as u64,
            inflight: inflight_u32,
        };
        let _ = write_log_line(log_path, event);
    }
    result
}

#[derive(Clone, Debug, Default)]
struct PhaseTimings {
    used_ocr: bool,
    truncated: bool,
    pages_rendered: u64,
    bytes_rendered: u64,
    bytes_ocr: u64,
    render_seconds: f64,
    recognize_seconds: f64,
}

fn extract_file_with_ocr_inner(
    path: &Path,
    options: &OcrOptions,
    metrics: Option<Arc<OcrMetrics>>,
    mut observer: impl FnMut(&str, f64, u64, u64),
    timings: Arc<Mutex<PhaseTimings>>,
) -> io::Result<String> {
    let native = extract_file(path);
    if let Ok(markdown) = &native {
        if has_embedded_text(markdown) {
            // Per-page decision. A PDF can be born-digital on some pages and a scan on
            // others; the old whole-document check short-circuited on the first alnum
            // line and never OCR'd the scanned pages. If every page already has a text
            // layer this is the unchanged native fast path; if only some pages lack text
            // we OCR exactly those and merge; if every page lacks text we fall through
            // to the whole-document render below (one pdftoppm pass is cheaper than N).
            let pages = split_page_markers(markdown);
            let missing: Vec<u32> = pages
                .iter()
                .filter(|page| !page_has_embedded_text(&page.text))
                .map(|page| page.number)
                .collect();
            if missing.is_empty() {
                if let Some(metrics) = metrics.as_ref() {
                    metrics.record_native_hit();
                }
                return Ok(markdown.clone());
            }
            if missing.len() < pages.len() {
                let ocr_pages = missing.len() as u64;
                {
                    let mut t = timings.lock().expect("phase timings mutex poisoned");
                    t.used_ocr = true;
                    t.pages_rendered = ocr_pages;
                }
                let mut recognize_seconds = 0.0f64;
                let merged = {
                    let mut per_page = |number: u32| {
                        let started = Instant::now();
                        let result = ocr_page_text(path, options, number);
                        let seconds = started.elapsed().as_secs_f64();
                        recognize_seconds += seconds;
                        if let Some(metrics) = metrics.as_ref() {
                            metrics.record_recognize(seconds, 0);
                        }
                        observer("page_recognize", seconds, 1, 0);
                        result
                    };
                    merge_ocr_pages(&pages, &mut per_page)
                };
                let mut t = timings.lock().expect("phase timings mutex poisoned");
                t.recognize_seconds = recognize_seconds;
                t.bytes_ocr = merged.as_ref().map(|m| m.len() as u64).unwrap_or(0);
                drop(t);
                observer("recognize_done", recognize_seconds, ocr_pages, 0);
                if merged.is_ok() {
                    if let Some(metrics) = metrics.as_ref() {
                        metrics.record_outcome(true, false);
                    }
                }
                return merged.map_err(|ocr_error| match &native {
                    Err(native_error) => io::Error::new(
                        ocr_error.kind(),
                        format!("PDF parser failed ({native_error}); OCR fallback failed ({ocr_error})"),
                    ),
                    Ok(_) => ocr_error,
                });
            }
            if let Some(metrics) = metrics.as_ref() {
                metrics.record_native_hit();
            }
        }
    }
    timings
        .lock()
        .expect("phase timings mutex poisoned")
        .used_ocr = true;
    let ocr_result = tesseract_ocr_with_observer(path, options, &mut observer);
    let mut t = timings.lock().expect("phase timings mutex poisoned");
    if let Ok(markdown) = &ocr_result {
        if markdown.contains("OCR stopped at the configured limit of") {
            t.truncated = true;
        }
        if let Some(last_brace) = markdown.rfind("<!-- page ") {
            let tail = &markdown[last_brace..];
            t.pages_rendered = tail.matches("<!-- page ").count() as u64;
        }
        t.bytes_ocr = markdown.len() as u64;
    }
    drop(t);
    match ocr_result {
        Ok(markdown) => Ok(markdown),
        Err(ocr_error) => match native {
            Err(native_error) => Err(io::Error::new(
                ocr_error.kind(),
                format!("PDF parser failed ({native_error}); OCR fallback failed ({ocr_error})"),
            )),
            Ok(_) => Err(ocr_error),
        },
    }
}

/// OCR exactly one page: `pdftoppm -f N -l N` renders just that page, then Tesseract reads it.
/// Same executables, same limits and same `<!-- page N -->` output convention as
/// [`tesseract_ocr`], but scoped to a single page so a mixed born-digital + scanned PDF
/// pays the render/recognize cost only on the pages that have no text layer.
fn ocr_page_text(path: &Path, options: &OcrOptions, page: u32) -> io::Result<String> {
    options.validate()?;
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_PDF_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF exceeds size limit",
        ));
    }
    if page == 0 || page > options.max_pages {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "page outside the configured OCR page range",
        ));
    }
    let deadline = Instant::now() + options.timeout;
    let temp = OcrTempDir::create()?;
    let prefix = temp.0.join(format!("page{page}"));
    let mut render = Command::new("pdftoppm");
    render
        .args(["-f", &page.to_string(), "-l", &page.to_string()])
        .arg("-scale-to")
        .arg(options.max_image_dimension.to_string())
        .arg("-png")
        .arg(path)
        .arg(&prefix);
    run_bounded(&mut render, deadline)?;
    let image = std::fs::read_dir(&temp.0)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension().and_then(|s| s.to_str()) == Some("png")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix(&format!("page{page}")))
                    .map(|rest| rest.trim_start_matches('-'))
                    .and_then(|digits| digits.parse::<u32>().ok())
                    == Some(page)
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Poppler rendered no image for page {page}"),
            )
        })?;
    let rendered_bytes = image.metadata().map(|m| m.len()).unwrap_or(0);
    if rendered_bytes > MAX_OCR_RENDER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rendered PDF pages exceed the disk-space limit",
        ));
    }
    let base = temp.0.join(format!("ocr-{page}"));
    let mut ocr = Command::new("tesseract");
    ocr.arg(&image)
        .arg(&base)
        .arg("-l")
        .arg(&options.languages)
        .args(["--psm", "3"]);
    run_bounded(&mut ocr, deadline)?;
    let text = read_bounded_text(base.with_extension("txt"), 4 * 1024 * 1024)?;
    if text.len() > MAX_OCR_TOTAL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "total OCR output exceeds size limit",
        ));
    }
    Ok(text)
}

/// Force OCR for every page using the installed Poppler and Tesseract executables.
pub fn tesseract_ocr(path: impl AsRef<Path>, options: &OcrOptions) -> io::Result<String> {
    tesseract_ocr_with_observer(path, options, |_, _, _, _| {})
}

/// Lower-level entry point: `tesseract_ocr` plus a per-phase observer for observability
/// sinks. Observer stages:
///   - `"render_done"` once, with `(seconds, pages, bytes)` after page enumeration
///   - `"page_recognize"` once per page, with `(seconds, ocr_text_bytes)`
///   - `"recognize_done"` once, with `(total_seconds, pages)` after the per-page loop
pub fn tesseract_ocr_with_observer(
    path: impl AsRef<Path>,
    options: &OcrOptions,
    mut observer: impl FnMut(&str, f64, u64, u64),
) -> io::Result<String> {
    options.validate()?;
    let input = path.as_ref();
    let metadata = std::fs::metadata(input)?;
    if metadata.len() > MAX_PDF_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF exceeds size limit",
        ));
    }
    let deadline = Instant::now() + options.timeout;
    let temp = OcrTempDir::create()?;
    let prefix = temp.0.join("page");
    let mut render = Command::new("pdftoppm");
    render
        .args(["-f", "1", "-l"])
        .arg(options.max_pages.to_string())
        .arg("-scale-to")
        .arg(options.max_image_dimension.to_string())
        .arg("-png")
        .arg(input)
        .arg(&prefix);
    let render_started = Instant::now();
    let render_outcome = run_bounded(&mut render, deadline);
    let render_seconds = render_started.elapsed().as_secs_f64();
    if let Err(error) = render_outcome {
        if let Some(metrics) = options.config.metrics.as_ref() {
            metrics.record_outcome(false, false);
        }
        return Err(error);
    }

    let mut pages = Vec::new();
    for entry in std::fs::read_dir(&temp.0)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let Some(number) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("page-"))
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        pages.push((number, path));
    }
    pages.sort_by_key(|(number, _)| *number);
    let rendered_bytes = pages.iter().try_fold(0u64, |total, (_, path)| {
        path.metadata()
            .map(|metadata| total.saturating_add(metadata.len()))
    })?;
    if rendered_bytes > MAX_OCR_RENDER_BYTES {
        if let Some(metrics) = options.config.metrics.as_ref() {
            metrics.record_outcome(false, false);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rendered PDF pages exceed the disk-space limit",
        ));
    }
    if pages.is_empty() {
        if let Some(metrics) = options.config.metrics.as_ref() {
            metrics.record_outcome(false, false);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Poppler rendered no PDF pages",
        ));
    }

    observer(
        "render_done",
        render_seconds,
        pages.len() as u64,
        rendered_bytes,
    );
    if let Some(log_path) = options.config.log_path.as_deref() {
        let _ = write_log_line(
            log_path,
            LogEvent::Flame {
                stage: "render",
                elapsed_us: (render_seconds * 1_000_000.0) as u64,
                pages: pages.len() as u32,
            },
        );
    }

    let rendered_limit_reached = pages.len() == options.max_pages as usize;
    let mut output = String::new();
    let mut recognize_seconds_total = 0.0f64;
    let mut pages_kept = 0u64;
    for (number, image) in pages {
        let base = temp.0.join(format!("ocr-{number}"));
        let mut ocr = Command::new("tesseract");
        ocr.arg(&image)
            .arg(&base)
            .arg("-l")
            .arg(&options.languages)
            .args(["--psm", "3"]);
        let page_started = Instant::now();
        let page_outcome = run_bounded(&mut ocr, deadline);
        let page_seconds = page_started.elapsed().as_secs_f64();
        if let Err(error) = page_outcome {
            if let Some(metrics) = options.config.metrics.as_ref() {
                metrics.record_outcome(false, false);
            }
            return Err(error);
        }
        let text = read_bounded_text(base.with_extension("txt"), 4 * 1024 * 1024)?;
        let text_len = text.len() as u64;
        recognize_seconds_total += page_seconds;
        observer("page_recognize", page_seconds, 1, text_len);
        if let Some(log_path) = options.config.log_path.as_deref() {
            let _ = write_log_line(
                log_path,
                LogEvent::Flame {
                    stage: "page_recognize",
                    elapsed_us: (page_seconds * 1_000_000.0) as u64,
                    pages: 1,
                },
            );
        }
        if !text.trim().is_empty() {
            if output.len().saturating_add(text.len()) > MAX_OCR_TOTAL_BYTES {
                if let Some(metrics) = options.config.metrics.as_ref() {
                    metrics.record_outcome(false, false);
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "total OCR output exceeds size limit",
                ));
            }
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(&format!("<!-- page {number} -->\n{}", text.trim()));
            pages_kept += 1;
        }
    }
    observer("recognize_done", recognize_seconds_total, pages_kept, 0);
    if let Some(log_path) = options.config.log_path.as_deref() {
        let _ = write_log_line(
            log_path,
            LogEvent::Flame {
                stage: "recognize",
                elapsed_us: (recognize_seconds_total * 1_000_000.0) as u64,
                pages: pages_kept as u32,
            },
        );
    }
    if let Some(metrics) = options.config.metrics.as_ref() {
        metrics.record_outcome(true, rendered_limit_reached);
    }
    if rendered_limit_reached {
        output.push_str(&format!(
            "\n\n[OCR stopped at the configured limit of {} pages; later pages may be omitted]",
            options.max_pages
        ));
    }
    Ok(output)
}

fn has_embedded_text(markdown: &str) -> bool {
    markdown.lines().any(|line| {
        let line = line.trim();
        !line.starts_with("![](") && line.chars().any(char::is_alphanumeric)
    })
}

struct OcrTempDir(PathBuf);

impl OcrTempDir {
    fn create() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..16 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("superdocuments-ocr-{}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Err(error) =
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                        {
                            let _ = std::fs::remove_dir_all(&path);
                            return Err(error);
                        }
                    }
                    return Ok(Self(path));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate OCR temp directory",
        ))
    }
}

impl Drop for OcrTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_bounded(command: &mut Command, deadline: Instant) -> io::Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!("{program} exited with {status}")))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{program} timed out"),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_bounded_text(path: PathBuf, limit: u64) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OCR output exceeds size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OCR output exceeds size limit",
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Convert a PDF path using `PDFFOLD_BIN`, `PDFFOLD_MANIFEST`, or `pdffold` on `PATH`.
pub fn to_markdown(path: impl AsRef<Path>) -> io::Result<String> {
    to_markdown_with_manifest(path, None)
}

/// Convert a PDF path while retaining an optional legacy Cargo-manifest fallback.
/// Explicit `PDFFOLD_BIN` takes priority, then `manifest`, then `PDFFOLD_MANIFEST`,
/// and finally an installed `pdffold` executable.
pub fn to_markdown_with_manifest(
    path: impl AsRef<Path>,
    manifest: Option<&Path>,
) -> io::Result<String> {
    let path = path.as_ref();
    if let Ok(bin) = std::env::var("PDFFOLD_BIN") {
        return run(Command::new(bin).arg(path));
    }
    let manifest = manifest
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("PDFFOLD_MANIFEST").map(Into::into));
    if let Some(manifest) = manifest {
        let mut command = Command::new("cargo");
        command
            .args(["run", "--quiet", "--example", "md", "--manifest-path"])
            .arg(manifest)
            .arg("--")
            .arg(path);
        return run(&mut command);
    }
    run(Command::new("pdffold").arg(path))
}

fn run(command: &mut Command) -> io::Result<String> {
    let output = command.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(error.trim().to_string()))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    fn pdf_with_stream(content: &[u8], filter: &str) -> Vec<u8> {
        let mut stream =
            format!("<< /Length {} {} >> stream\n", content.len(), filter).into_bytes();
        stream.extend_from_slice(content);
        stream.extend_from_slice(b"\nendstream");
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            stream,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }

    fn minimal_text_pdf() -> Vec<u8> {
        pdf_with_stream(b"BT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET", "")
    }

    /// Fixture PDF + the PDF page count its `pdfinfo` reports. The native parser must emit
    /// exactly one `<!-- page N -->` marker per page, 1..=pages, for every one of them.
    const PAGE_COUNTED_FIXTURES: &[(&str, u32)] = &[
        ("borderless-table.pdf", 1),
        ("mixed-native-and-scanned.pdf", 2),
        ("reportlab-ascii85.pdf", 1),
        ("reportlab-invoice.pdf", 1),
        ("rotated-15deg.pdf", 1),
        ("scanned-fuzzy.pdf", 1),
        ("scanned-text.pdf", 1),
        ("two-column.pdf", 1),
    ];

    fn fixture(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
    }

    #[test]
    fn native_parser_emits_one_page_marker_per_page_for_every_fixture() {
        for (name, expected_pages) in PAGE_COUNTED_FIXTURES {
            let markdown = extract_markdown(&fixture(name))
                .unwrap_or_else(|e| panic!("extract {name}: {e}"));
            let numbers: Vec<u32> = markdown
                .lines()
                .filter_map(crate::pdf::parser::parse_page_marker)
                .collect();
            let expected: Vec<u32> = (1..=*expected_pages).collect();
            assert_eq!(numbers, expected, "page markers for {name}: {markdown:?}");
            let marker_lines = markdown
                .lines()
                .filter(|line| line.starts_with("<!-- page "))
                .count();
            assert_eq!(marker_lines as u32, *expected_pages, "{name}");
            let split = split_page_markers(&markdown);
            assert_eq!(split.len(), *expected_pages as usize, "{name}");
            assert_eq!(
                split.iter().map(|p| p.number).collect::<Vec<_>>(),
                expected,
                "{name}"
            );
            // Round-trip: merging the split pages and re-splitting reproduces the page
            // sequence (merge skips pages with no text at all, like the OCR path does).
            let non_empty: Vec<u32> = split
                .iter()
                .filter(|p| page_has_embedded_text(&p.text))
                .map(|p| p.number)
                .collect();
            assert_eq!(
                split_page_markers(&merge_page_markers(&split))
                    .iter()
                    .map(|p| p.number)
                    .collect::<Vec<_>>(),
                non_empty,
                "{name}"
            );
            for page in &split {
                let again =
                    split_page_markers(&merge_page_markers(std::slice::from_ref(page)));
                if page_has_embedded_text(&page.text) {
                    assert_eq!(again.first().map(|p| p.number), Some(page.number), "{name}");
                } else {
                    assert!(
                        again.is_empty(),
                        "a text-less page is not re-emitted: {name} page {}",
                        page.number
                    );
                }
            }
        }
    }

    /// A two-page PDF: page 1 has a text layer, page 2 is a bare scan (no `/Contents`).
    fn mixed_native_and_scanned_pdf() -> Vec<u8> {
        let objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            {
                let body = b"BT /F1 12 Tf 72 720 Td (Native page one) Tj ET";
                let mut stream =
                    format!("<< /Length {} >> stream\n", body.len()).into_bytes();
                stream.extend_from_slice(body);
                stream.extend_from_slice(b"\nendstream");
                stream
            },
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }

    #[test]
    fn two_page_pdf_marks_every_page_and_per_page_ocr_fills_only_the_blank_one() {
        let pdf = mixed_native_and_scanned_pdf();
        let markdown = extract_markdown(&pdf).unwrap();
        let pages = split_page_markers(&markdown);
        assert_eq!(
            pages.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![1, 2],
            "{markdown:?}"
        );
        assert!(pages[0].text.contains("Native page one"), "{markdown:?}");
        assert!(
            !page_has_embedded_text(&pages[1].text),
            "page 2 is the scanned one: {:?}",
            pages[1].text
        );
        let mut asked: Vec<u32> = Vec::new();
        let merged = merge_ocr_pages(&pages, &mut |number| {
            asked.push(number);
            Ok(format!("OCR recovered page {number}"))
        })
        .unwrap();
        assert_eq!(asked, vec![2], "only the text-less page is OCR'd");
        let merged_pages = split_page_markers(&merged);
        assert_eq!(
            merged_pages.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(merged_pages[0].text.contains("Native page one"));
        assert!(merged_pages[1].text.contains("OCR recovered page 2"));
    }

    #[test]
    fn scanned_fixture_pages_carry_a_marker_with_no_text() {
        // A scanned page has no text layer: the marker must still be there so a downstream
        // page-aware consumer can see the hole (and the per-page OCR fallback can fill it).
        for name in ["scanned-text.pdf", "scanned-fuzzy.pdf"] {
            let markdown = extract_markdown(&fixture(name)).unwrap();
            let pages = split_page_markers(&markdown);
            assert_eq!(pages.len(), 1, "{name}");
            assert!(
                !page_has_embedded_text(&pages[0].text),
                "{name} should have no text layer, got {:?}",
                pages[0].text
            );
        }
    }

    #[test]
    fn extract_markdown_flat_drops_the_markers() {
        let markdown = extract_markdown_flat(&fixture("reportlab-invoice.pdf")).unwrap();
        assert!(!markdown.contains("<!-- page"), "{markdown:?}");
        assert!(markdown.contains("Invoice Fixture 2025"), "{markdown:?}");
        // Same text as the paged projection, minus the markers.
        let paged = extract_markdown(&fixture("reportlab-invoice.pdf")).unwrap();
        assert_eq!(
            markdown,
            paged.lines()
                .filter(|l| !l.starts_with("<!-- page "))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        );
    }

    #[test]
    fn split_and_merge_round_trip_mixed_native_and_scanned() {
        let markdown = extract_markdown(&fixture("mixed-native-and-scanned.pdf")).unwrap();
        let pages = split_page_markers(&markdown);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages.iter().map(|p| p.number).collect::<Vec<_>>(), vec![1, 2]);
        // The native parser cannot decode this adversarial fixture's text layer (it carries
        // no recoverable text on either page), so both pages take the OCR branch: the fixture
        // still proves the marker sequence is 1..=page_count, which is what downstream needs.
        let missing: Vec<u32> = pages
            .iter()
            .filter(|p| !page_has_embedded_text(&p.text))
            .map(|p| p.number)
            .collect();
        assert_eq!(missing, vec![1, 2]);
    }

    #[test]
    fn merge_ocr_pages_only_ocrs_the_scanned_pages_and_keeps_order() {
        let pages = split_page_markers(&extract_markdown(&mixed_native_and_scanned_pdf()).unwrap());
        let mut asked: Vec<u32> = Vec::new();
        let merged = merge_ocr_pages(&pages, &mut |number| {
            asked.push(number);
            Ok(format!("OCR text for page {number}"))
        })
        .unwrap();
        assert_eq!(asked, vec![2], "only the page with no text layer is OCR'd");
        let merged_pages = split_page_markers(&merged);
        assert_eq!(
            merged_pages.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(merged_pages[0].text.contains("Native page one"));
        assert!(merged_pages[1].text.contains("OCR text for page 2"));
    }

    #[test]
    fn merge_ocr_pages_is_fail_soft_per_page_and_fail_closed_overall() {
        let pages = vec![
            PageText {
                number: 1,
                text: String::new(),
            },
            PageText {
                number: 2,
                text: "native body\n".into(),
            },
            PageText {
                number: 3,
                text: String::new(),
            },
        ];
        // One page fails: the pages that came back survive, in order.
        let merged = merge_ocr_pages(&pages, &mut |number| {
            if number == 1 {
                Err(io::Error::new(io::ErrorKind::Other, "boom"))
            } else {
                Ok(format!("ocr {number}"))
            }
        })
        .unwrap();
        let out = split_page_markers(&merged);
        assert_eq!(out.iter().map(|p| p.number).collect::<Vec<_>>(), vec![2, 3]);
        assert!(out[1].text.contains("ocr 3"));

        // Every OCR'd page fails: the error surfaces.
        let all_scanned = vec![PageText {
            number: 1,
            text: String::new(),
        }];
        let error = merge_ocr_pages(&all_scanned, &mut |_| {
            Err(io::Error::new(io::ErrorKind::Other, "boom"))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    #[test]
    #[ignore = "requires pdftoppm and tesseract on PATH"]
    fn mixed_native_and_scanned_pdf_ocrs_only_the_scanned_page() {
        // The adversarial fixture's page 2 is a deliberately noisy scan: its OCR text is
        // lossy on this host (Tesseract returns "Senwrnrnea sester Page: OOF werk ..."
        // for "Scanned raster page: OCR works on mixed documents."). The same lossy text
        // comes out of the whole-document path, so the assertion here is the page STRUCTURE
        // — two pages, in order, both carrying text — not the noisy OCR characters.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed-native-and-scanned.pdf");
        let markdown = extract_file_with_ocr(path).unwrap();
        let pages = split_page_markers(&markdown);
        assert_eq!(
            pages.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![1, 2],
            "{markdown:?}"
        );
        assert!(pages[0].text.contains("selectable text"), "{markdown:?}");
        assert!(!pages[1].text.trim().is_empty(), "{markdown:?}");
    }

    #[test]
    #[ignore = "requires pdftoppm and tesseract on PATH"]
    fn per_page_ocr_matches_whole_document_ocr_quality_on_a_one_page_scan() {
        // Fair three-column check on the same held-out fixture (scanned-text.pdf):
        //   quality  = normalized OCR text vs the fixture's authored phrase
        //   baseline = the pre-existing whole-document `tesseract_ocr` path (same image,
        //              same Tesseract build, one pdftoppm pass over 1 page)
        // The per-page path must not be worse than the baseline on quality; it does one
        // pdftoppm invocation per page, which for a multi-page scan is the cost of the
        // scanned pages only.
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanned-text.pdf");
        let options = OcrOptions::default();
        let normalize = |text: &str| {
            text.chars()
                .filter(|ch| ch.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        let expected = normalize("Scanned fixture: OCR works.");

        let baseline_started = Instant::now();
        let baseline = tesseract_ocr(&pdf, &options).unwrap();
        let baseline_seconds = baseline_started.elapsed().as_secs_f64();
        // The baseline output carries the `<!-- page 1 -->` marker; compare page text only.
        let baseline_text: String = baseline
            .lines()
            .filter(|line| !line.starts_with("<!-- page "))
            .collect::<Vec<_>>()
            .join("\n");

        let per_page_started = Instant::now();
        let per_page = ocr_page_text(&pdf, &options, 1).unwrap();
        let per_page_seconds = per_page_started.elapsed().as_secs_f64();

        println!("baseline whole-doc OCR: {baseline_seconds:.2}s, per-page OCR: {per_page_seconds:.2}s");
        println!("baseline quality == expected: {}", normalize(&baseline_text) == expected);
        println!("per-page quality == expected: {}", normalize(&per_page) == expected);
        assert_eq!(normalize(&per_page), expected, "per-page OCR text: {per_page:?}");
        assert_eq!(
            normalize(&baseline_text),
            expected,
            "baseline whole-document OCR text: {baseline:?}"
        );
    }

    #[test]
    fn in_process_parser_extracts_text_and_rejects_invalid_input() {
        let markdown = extract_markdown(&minimal_text_pdf()).unwrap();
        assert!(markdown.contains("Hello PDF"), "{markdown:?}");
        assert!(extract_markdown(b"not a pdf").is_err());
    }

    #[test]
    fn in_process_parser_inflates_compressed_streams() {
        let compressed = [
            0x78, 0x9c, 0x73, 0x0a, 0x51, 0xd0, 0x77, 0x33, 0x54, 0x30, 0x34, 0x52, 0x08, 0x49,
            0x53, 0x30, 0x37, 0x02, 0x22, 0x03, 0x85, 0x90, 0x14, 0x05, 0x0d, 0xe7, 0xfc, 0xdc,
            0x82, 0xa2, 0xd4, 0xe2, 0xe2, 0xd4, 0x14, 0x85, 0x00, 0x17, 0x37, 0x4d, 0x85, 0x90,
            0x2c, 0x05, 0xd7, 0x10, 0x00, 0xfe, 0x21, 0x0b, 0xeb,
        ];
        let pdf = pdf_with_stream(&compressed, "/Filter /FlateDecode");
        let markdown = extract_markdown(&pdf).unwrap();
        assert!(markdown.contains("Compressed PDF"), "{markdown:?}");
    }

    #[test]
    fn real_reportlab_fixture_extracts_native_text() {
        let markdown =
            extract_markdown(include_bytes!("../../tests/fixtures/reportlab-invoice.pdf")).unwrap();
        assert!(markdown.contains("Invoice Fixture 2025"), "{markdown:?}");
        assert!(markdown.contains("INV-2048"), "{markdown:?}");
        assert!(markdown.contains("$1,234.50"), "{markdown:?}");
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reportlab-invoice.pdf");
        assert!(extract_file_with_ocr(path)
            .unwrap()
            .contains("Invoice Fixture 2025"));
    }

    #[test]
    #[ignore = "requires pdftoppm and tesseract on PATH"]
    fn tesseract_fallback_handles_raster_and_unsupported_filter_pdfs() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let scanned = extract_file_with_ocr(fixtures.join("scanned-text.pdf")).unwrap();
        let normalize = |text: &str| {
            text.lines()
                .filter(|line| !line.starts_with("<!-- page "))
                .flat_map(str::chars)
                .filter(|ch| ch.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        assert_eq!(
            normalize(&scanned),
            normalize("Scanned fixture: OCR works.")
        );

        let ascii85 = extract_file_with_ocr(fixtures.join("reportlab-ascii85.pdf")).unwrap();
        assert!(ascii85.contains("Invoice Fixture 2025"), "{ascii85:?}");
        assert!(ascii85.contains("INV-2048"), "{ascii85:?}");
    }

    #[test]
    fn malformed_pdf_and_ocr_limits_fail_closed() {
        assert!(extract_markdown(b"%PDF-1.4\\n").is_err());
        let mut options = OcrOptions::default();
        options.max_pages = 0;
        assert_eq!(
            options.validate().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let mut oversized_render = OcrOptions::default();
        oversized_render.max_pages = 100;
        oversized_render.max_image_dimension = 4_000;
        assert_eq!(
            oversized_render.validate().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let temp = OcrTempDir::create().unwrap();
        let oversized = temp.0.join("oversized.pdf");
        let file = std::fs::File::create(&oversized).unwrap();
        file.set_len(MAX_PDF_BYTES as u64 + 1).unwrap();
        drop(file);
        assert_eq!(
            extract_file(&oversized).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn ocr_child_process_respects_deadline() {
        let mut sleeper = Command::new("sleep");
        sleeper.arg("1");
        let error =
            run_bounded(&mut sleeper, Instant::now() + Duration::from_millis(30)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn process_adapter_returns_stdout_and_surfaces_failures() {
        let mut success = Command::new("sh");
        success.args(["-c", "printf converted"]);
        assert_eq!(run(&mut success).unwrap(), "converted");

        let mut failure = Command::new("sh");
        failure.args(["-c", "printf failed >&2; exit 7"]);
        assert_eq!(run(&mut failure).unwrap_err().to_string(), "failed");
    }

    #[test]
    fn ocr_metrics_record_native_hit_and_render_prometheus_text() {
        // Born-digital fixture: native path returns first, OCR fallback never runs. That
        // means render/recognize histograms stay empty and `native_hits_total` increments.
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let pdf = fixtures.join("reportlab-invoice.pdf");
        let metrics = OcrMetrics::new();
        let mut options = OcrOptions::default();
        options.config.metrics = Some(metrics.clone());

        let markdown = extract_file_with_ocr_options(&pdf, &options).unwrap();
        assert!(markdown.contains("Invoice Fixture 2025"));

        let text = metrics.render();
        assert!(text.contains("# TYPE superdocuments_ocr_requests_total counter"));
        assert!(text.contains("superdocuments_ocr_requests_total 1"));
        assert!(text.contains("superdocuments_ocr_native_hits_total 1"));
        assert!(text.contains("superdocuments_ocr_inflight 0"));
        // Native path: no rendered pages, no OCR attempts, no recognize histograms.
        assert!(text.contains("superdocuments_ocr_rendered_pages_total 0"));
        assert!(text.contains("superdocuments_ocr_attempts_total 0"));
        assert!(text.contains("# TYPE superdocuments_ocr_request_seconds histogram"));
    }

    #[test]
    fn ocr_metrics_record_render_and_recognize_when_observer_fires() {
        // Directly exercise the observer plumbing without spawning pdftoppm/tesseract.
        // The observer is what wires `record_render` / `record_recognize` from real
        // per-phase timings; if it regresses the metrics path is silent.
        let metrics = OcrMetrics::new();
        metrics.record_render(0.123, 3, 4_500);
        metrics.record_recognize(0.045, 200);
        metrics.record_recognize(0.050, 180);
        metrics.record_outcome(true, false);

        let text = metrics.render();
        assert!(text.contains("superdocuments_ocr_rendered_pages_total 3"));
        assert!(text.contains("superdocuments_ocr_rendered_bytes_total 4500"));
        assert!(text.contains("superdocuments_ocr_output_bytes_total 380"));
        assert!(text.contains("superdocuments_ocr_success_total 1"));
        assert!(text.contains("superdocuments_ocr_attempts_total 1"));
        // 0.123s lands in the `<=0.25` bucket; both recognize observations (0.045, 0.050)
        // are <= 0.1 so the cumulative `le=0.1` count is 2.
        assert!(text.contains("superdocuments_ocr_render_seconds_bucket{le=\"0.25\"} 1"));
        assert!(text.contains("superdocuments_ocr_recognize_seconds_bucket{le=\"0.1\"} 2"));
        // Peak working set reflects the single render call.
        assert!(text.contains("superdocuments_ocr_peak_resident_bytes 4500"));
    }

    #[test]
    fn ocr_metrics_peak_resident_bytes_is_monotonic_across_requests() {
        let metrics = OcrMetrics::new();
        metrics.record_render(0.01, 1, 1_000);
        metrics.record_render(0.01, 2, 5_000);
        metrics.record_render(0.01, 1, 500);
        // The gauge never decreases across renders, even when a later render is smaller.
        assert!(metrics
            .render()
            .contains("superdocuments_ocr_peak_resident_bytes 5000"));
    }

    #[test]
    fn ocr_jsonl_log_emits_started_and_finished_lines() {
        // Same born-digital fixture, same native path; the log file should receive both a
        // `request_started` and a `request_finished` line with outcome `native`.
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let pdf = fixtures.join("reportlab-invoice.pdf");
        let temp = OcrTempDir::create().unwrap();
        let log_path = temp.0.join("ocr.jsonl");
        let mut options = OcrOptions::default();
        options.config.log_path = Some(log_path.clone());

        let _ = extract_file_with_ocr_options(&pdf, &options).unwrap();
        let body = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert!(
            lines.len() >= 2,
            "expected at least started+finished, got: {body:?}"
        );
        assert!(lines[0].contains("\"event\":\"request_started\""));
        assert!(lines[0].contains("\"path\":\""));
        assert!(lines[0].contains("\"languages\":\"eng\""));
        assert!(lines[0].contains("\"max_pages\":20"));
        let finished = lines
            .iter()
            .find(|line| line.contains("\"event\":\"request_finished\""))
            .expect("finished line missing");
        assert!(finished.contains("\"outcome\":\"native\""));
        assert!(finished.contains("\"used_ocr\":false"));
        assert!(finished.contains("\"truncated\":false"));
        assert!(finished.contains("\"pages_rendered\":0"));
        assert!(finished.contains("\"bytes_rendered\":0"));
        assert!(finished.contains("\"bytes_ocr\":0"));
    }
}

// ── Geometry surface (LiteParse feature-parity: items 1, 2, 3, 5, 7, 9) ──────
//
// All entry points in this section are **shape-only**: they exist so the
// `superdocuments-bench/ocr` cross-stack comparison can score every target
// against the same JSON schema, but every geometry field is `None` until a
// geometry-aware PDF backend (PDFium, a Transcode patch exposing the Op tree,
// or a Rust-side geometry pass on `extract_markdown`'s output) is wired in.
//
// The function shapes match `run-llama/liteparse` upstream so swapping in a
// geometry backend later is a 1:1 port.

use self::geometry::{
    DocumentMetadata, LayoutBlock, LayoutCell, OutlineTarget, OutputFormat, ParsedPage,
};

/// Document-level provenance metadata. Stays `None` for most fields until
/// a backend that surfaces the PDF `/Info` dict is wired in.
pub fn document_metadata(path: impl AsRef<Path>) -> io::Result<DocumentMetadata> {
    let bytes = std::fs::read(path.as_ref())?;
    // TODO(items 9, 10): surface `creation_date`, `mod_date`, `is_encrypted`,
    // `signature_count`, etc. from Transcode's info-dict walker once the
    // upstream exposes them. Today we report only the file size we can read.
    Ok(DocumentMetadata {
        raw_file_size: Some(bytes.len() as u64),
        ..Default::default()
    })
}

/// Document outline (PDF bookmarks). Walks the catalog's `/Outlines` tree
/// via Transcode's object store; populated by `transcode_outline`.
pub fn outline(path: impl AsRef<Path>) -> io::Result<Vec<OutlineTarget>> {
    transcode_outline(path)
}

/// Cheap "does this page need OCR?" predicate. Implementation lives in
/// `page_walker` and uses byte-level heuristics over AcroForm presence +
/// content-stream token density. Without PDFium we can't compute
/// LiteParse's full image-coverage / vector-text / font-encoding signals,
/// so the result is a coarse `PageComplexityStats` populated with `NoText`
/// or `AnnotationText` reasons only. When a real PDF backend is available
/// the transcode-backed `complexity_from_transcode` populator returns richer
/// signals (SparseText + page-level thresholds).
pub use crate::pdf::page_walker::page_complexity;
pub use crate::pdf::page_walker::page_complexity_from_bytes;
pub use crate::pdf::transcode_populator::{
    complexity_from_transcode, geometry_from_transcode, transcode_extract_markdown_with_metadata,
    transcode_outline,
};

/// Default complexity function — prefers Transcode's per-page walker,
/// falls back to the byte-level heuristic when Transcode can't parse the
/// input (encrypted, malformed, etc).
pub fn complexity(
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<Vec<geometry::PageComplexityStats>> {
    match complexity_from_transcode(path.as_ref()) {
        Ok(stats) if !stats.is_empty() => Ok(stats),
        _ => Ok(vec![page_complexity(path)?]),
    }
}

/// Extract text + per-page metadata, optionally as a different output format.
///
/// `format=Markdown` (default) returns the same markdown string the existing
/// `extract_markdown` produces.
/// `format=Json` returns a `serde_json::Value` with `{ markdown, text, metadata,
/// outline, pages }`. `text` is the plain-text equivalent of the markdown;
/// `metadata` is `document_metadata(path)`; `outline` is `outline(path)`.
/// `format=Text` returns the plain text only.
///
/// Geometry fields (`pages[i].text_items`, `pages[i].blocks`, bbox) are
/// `None`/`[]` until a geometry backend lands; everything else is populated.
pub fn extract_with_format(
    path: impl AsRef<Path>,
    format: OutputFormat,
) -> io::Result<serde_json::Value> {
    let path_ref = path.as_ref();
    let bytes = std::fs::read(path_ref)?;
    let markdown = extract_markdown(&bytes)?;
    let text = markdown
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let metadata = document_metadata(path_ref).unwrap_or_default();
    let outline = outline(path_ref).unwrap_or_default();
    let page_count = markdown.matches("<!-- page").count().max(1) as u32;
    let pages: Vec<ParsedPage> = (0..page_count)
        .map(|i| ParsedPage {
            page_number: (i + 1) as usize,
            page_label: None,
            page_width: 0.0,
            page_height: 0.0,
            content_bounds: None,
            text: String::new(),
            markdown: String::new(),
            text_items: Vec::new(),
            blocks: Vec::new(),
            complexity: None,
        })
        .collect();
    let payload = match format {
        OutputFormat::Markdown => serde_json::Value::String(markdown),
        OutputFormat::Text => serde_json::Value::String(text),
        OutputFormat::Json => serde_json::json!({
            "markdown": markdown,
            "text": text,
            "metadata": metadata,
            "outline": outline,
            "pages": pages,
        }),
    };
    Ok(payload)
}

/// Block-level layout decomposition of one page. Pure-markdown
/// implementation: walks the lines `extract_markdown` produces and classifies
/// each non-blank line as heading / paragraph / list-item / table-row.
///
/// This is the markdown-text-only populator — it doesn't reconstruct
/// bounding boxes (those need PDFium) but it does populate every block's
/// `kind`, `text`, and table-cell structure so the harness's
/// `median_table_cell_agreement` score stops being 0/NaN on tabular docs.
///
/// Lifted in shape from `run-llama/liteparse` `projection.rs` (~228 KB of
/// upstream). We carry the markdown-level classifier and stub the XY-cut
/// bbox walker; future work can port the geometric half when a geometry
/// backend lands.
pub fn page_blocks(_page_index: usize, bytes: &[u8]) -> io::Result<Vec<LayoutBlock>> {
    let text = String::from_utf8_lossy(bytes);
    let mut blocks: Vec<LayoutBlock> = Vec::new();
    let mut list_depth: u8 = 0;
    // Table accumulator state — header_rows is the header candidate,
    // body_rows accumulates body cells, header_seen flips true once we
    // cross the separator row.
    let mut table_header: Option<Vec<String>> = None;
    let mut table_body: Vec<Vec<String>> = Vec::new();
    let mut table_separator_seen = false;
    let mut atx_count = 0u8;

    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            continue;
        }
        // Markdown ATX headings: `#`, `##`, ..., `######`. A bare `#` (no
        // space, no second `#`) is NOT a heading — it's just an inline `#`
        // (rare in PDFs but possible). The check is purely byte-level.
        if let Some(level) = atx_heading_level(line) {
            let body = line[level as usize + 1..].trim_start().to_string();
            flush_table(
                &mut blocks,
                &mut table_header,
                &mut table_body,
                &mut table_separator_seen,
            );
            blocks.push(LayoutBlock {
                kind: "heading".into(),
                text: Some(body),
                level: Some(level),
                bold: false,
                italic: false,
                ordered: None,
                marker: None,
                lines: None,
                lang: None,
                header: None,
                rows: None,
                header_rows: None,
                id: None,
                format: None,
                bbox: None,
            });
            atx_count = atx_count.saturating_add(1);
            list_depth = 0;
            continue;
        }
        // Markdown table rows: `| … |` or `|:-:|` style.
        if line.starts_with('|') && line.ends_with('|') && line.len() > 2 {
            if is_separator_row(line) {
                table_separator_seen = true;
                continue;
            }
            let cells = parse_table_row(line);
            if !table_separator_seen {
                // Header phase (or first header row): overwrite the candidate
                // so the last header row wins.
                table_header = Some(cells);
            } else {
                // Body phase.
                table_body.push(cells);
            }
            continue;
        }
        // Markdown list items: `-`, `*`, `+`, or `1. `.
        if let Some((ordered, marker, body)) = parse_list_item(line) {
            flush_table(
                &mut blocks,
                &mut table_header,
                &mut table_body,
                &mut table_separator_seen,
            );
            let depth = depth_of(line);
            list_depth = depth.max(list_depth);
            blocks.push(LayoutBlock {
                kind: "list_item".into(),
                text: Some(body.to_string()),
                level: Some(list_depth),
                bold: false,
                italic: false,
                ordered: Some(ordered),
                marker: Some(marker.to_string()),
                lines: None,
                lang: None,
                header: None,
                rows: None,
                header_rows: None,
                id: None,
                format: None,
                bbox: None,
            });
            continue;
        }
        // Fenced code blocks: ```
        if let Some(rest) = line.strip_prefix("```") {
            flush_table(
                &mut blocks,
                &mut table_header,
                &mut table_body,
                &mut table_separator_seen,
            );
            // Single-line variant: collect until matching closer.
            let lang = rest.trim().to_string();
            // We don't iterate the rest of the lines here — caller is
            // single-shot. Just emit a code block with this single line.
            let code = line.to_string();
            blocks.push(LayoutBlock {
                kind: "code".into(),
                text: None,
                level: None,
                bold: false,
                italic: false,
                ordered: None,
                marker: None,
                lines: Some(vec![code]),
                lang: if lang.is_empty() { None } else { Some(lang) },
                header: None,
                rows: None,
                header_rows: None,
                id: None,
                format: None,
                bbox: None,
            });
            list_depth = 0;
            continue;
        }
        // Default: paragraph line.
        flush_table(
            &mut blocks,
            &mut table_header,
            &mut table_body,
            &mut table_separator_seen,
        );
        blocks.push(LayoutBlock {
            kind: "paragraph".into(),
            text: Some(line.to_string()),
            level: None,
            bold: false,
            italic: false,
            ordered: None,
            marker: None,
            lines: None,
            lang: None,
            header: None,
            rows: None,
            header_rows: None,
            id: None,
            format: None,
            bbox: None,
        });
        list_depth = 0;
    }
    flush_table(
        &mut blocks,
        &mut table_header,
        &mut table_body,
        &mut table_separator_seen,
    );

    let _ = atx_count;
    Ok(blocks)
}

/// Flush a pending table to a single `LayoutBlock { kind: "table" }` and
/// reset the accumulator. Three cases:
///   * separator seen AND body rows exist — header is the candidate, body
///     is what was collected after the separator.
///   * separator NOT seen AND at least one row exists — no real header
///     separator was seen, so the first row IS the header and the rest are
///     body.
///   * separator seen but no body rows — header-only table.
fn flush_table(
    blocks: &mut Vec<LayoutBlock>,
    header: &mut Option<Vec<String>>,
    body: &mut Vec<Vec<String>>,
    separator_seen: &mut bool,
) {
    if header.is_none() && body.is_empty() {
        return;
    }
    let (header_cells, body_rows) = if *separator_seen {
        (header.take(), std::mem::take(body))
    } else if header.is_none() {
        // No separator and no header candidate either — first body row
        // becomes the header.
        let mut all = body.clone();
        if all.is_empty() {
            (None, Vec::new())
        } else {
            let first_body = all.remove(0);
            (Some(first_body), all)
        }
    } else if body.is_empty() {
        // Header-only table.
        (header.take(), Vec::new())
    } else {
        // Header candidate present but no separator row. The first body row
        // is actually the header; remaining rows are body.
        let mut all = body.clone();
        let first_body = all.remove(0);
        (Some(first_body), all)
    };
    let header_rows = if header_cells.is_some() { 1 } else { 0 };
    let header_layout: Option<Vec<LayoutCell>> = header_cells.as_ref().map(|cells| {
        cells
            .iter()
            .map(|text| LayoutCell {
                text: text.clone(),
                bbox: None,
                colspan: None,
                rowspan: None,
            })
            .collect()
    });
    let rows_layout: Option<Vec<Vec<LayoutCell>>> = if body_rows.is_empty() {
        None
    } else {
        Some(
            body_rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|text| LayoutCell {
                            text,
                            bbox: None,
                            colspan: None,
                            rowspan: None,
                        })
                        .collect()
                })
                .collect(),
        )
    };
    blocks.push(LayoutBlock {
        kind: "table".into(),
        text: None,
        level: None,
        bold: false,
        italic: false,
        ordered: None,
        marker: None,
        lines: None,
        lang: None,
        header: header_layout,
        rows: rows_layout,
        header_rows: Some(header_rows),
        id: None,
        format: None,
        bbox: None,
    });
    *separator_seen = false;
}

/// Detect ATX heading level (1–6) from a markdown line. Returns `None` for
/// non-heading lines. A bare `#` (e.g. `#1`) is NOT a heading — there must be
/// a space (or end of line) after the run of `#` characters.
fn atx_heading_level(line: &str) -> Option<u8> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b'#' {
        i += 1;
        if i > 6 {
            return None;
        }
    }
    if i == 0 {
        return None;
    }
    // After the run of `#`s, the next byte must be a space, a tab, or end-of-line.
    match bytes.get(i) {
        None => Some(i as u8),
        Some(b' ' | b'\t') => Some(i as u8),
        _ => None,
    }
}

/// Is this line a Markdown table separator (`|---|---|`)?
fn is_separator_row(line: &str) -> bool {
    if !line.starts_with('|') || !line.ends_with('|') {
        return false;
    }
    line[1..line.len() - 1].split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':') && cell.len() >= 3
    })
}

/// Parse a Markdown table row into cell strings. `|` cells.
fn parse_table_row(line: &str) -> Vec<String> {
    let inner = line.trim().trim_matches('|');
    inner
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// Parse a Markdown list-item line. Returns `(ordered, marker, body)`.
fn parse_list_item(line: &str) -> Option<(bool, String, String)> {
    let trimmed = line.trim_start();
    // Ordered list: one or more digits, then `.` or `)`, then a space, then body.
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    let mut saw_digit = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        saw_digit = true;
        i += 1;
    }
    if saw_digit && i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') {
        let marker_end = i + 1; // include the `.` or `)`
        let marker = trimmed[..marker_end].to_string();
        let body = trimmed[marker_end..].trim_start();
        return Some((true, marker, body.to_string()));
    }
    // Unordered list: `- `, `* `, `+ `.
    if let Some(stripped) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        let marker = trimmed.chars().next().unwrap().to_string();
        return Some((false, marker, stripped.to_string()));
    }
    None
}

/// Indent depth: count groups of 2 leading spaces (or one tab = 4 spaces).
/// Used to nest `list_item` blocks in markdown.
fn depth_of(line: &str) -> u8 {
    let leading = line
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum::<usize>();
    (leading / 2) as u8
}

#[cfg(test)]
mod page_blocks_tests {
    use super::*;

    fn blocks_of(text: &str) -> Vec<LayoutBlock> {
        page_blocks(0, text.as_bytes()).unwrap()
    }

    #[test]
    fn page_blocks_parses_atx_headings() {
        let md = "# Heading 1\n## Heading 2\n### Heading 3\nbody text\n";
        let blocks = blocks_of(md);
        let kinds: Vec<&str> = blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, vec!["heading", "heading", "heading", "paragraph"]);
        let levels: Vec<u8> = blocks.iter().take(3).filter_map(|b| b.level).collect();
        assert_eq!(levels, vec![1, 2, 3]);
    }

    #[test]
    fn page_blocks_parses_unordered_list() {
        let md = "- one\n- two\n- three\n";
        let blocks = blocks_of(md);
        assert_eq!(
            blocks.iter().map(|b| b.kind.as_str()).collect::<Vec<_>>(),
            vec!["list_item"; 3]
        );
        for b in &blocks {
            assert_eq!(b.ordered, Some(false));
            assert_eq!(b.marker.as_deref(), Some("-"));
            assert!(b.text.is_some());
        }
    }

    #[test]
    fn page_blocks_parses_ordered_list() {
        let md = "1. one\n2. two\n10. ten\n";
        let blocks = blocks_of(md);
        assert_eq!(
            blocks.iter().map(|b| b.kind.as_str()).collect::<Vec<_>>(),
            vec!["list_item"; 3]
        );
        let markers: Vec<&str> = blocks
            .iter()
            .map(|b| b.marker.as_deref().unwrap())
            .collect();
        assert_eq!(markers, vec!["1.", "2.", "10."]);
        for b in &blocks {
            assert_eq!(b.ordered, Some(true));
        }
    }

    #[test]
    fn page_blocks_parses_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n";
        let blocks = blocks_of(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "table");
        let header = blocks[0].header.as_ref().expect("header");
        assert_eq!(header.len(), 2);
        assert_eq!(header[0].text, "A");
        assert_eq!(header[1].text, "B");
        let rows = blocks[0].rows.as_ref().expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].text, "1");
        assert_eq!(rows[1][1].text, "4");
        assert_eq!(blocks[0].header_rows, Some(1));
    }

    #[test]
    fn page_blocks_distinguishes_separator_rows() {
        // `|---|---|` is the separator; the row before it is the header
        // (so it's stored as `header_rows: Some(1)`).
        let md = "| A | B |\n|---|---|---|\n| 1 | 2 |\n";
        let blocks = blocks_of(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "table");
        assert_eq!(blocks[0].rows.as_ref().unwrap().len(), 1);
        assert_eq!(blocks[0].header_rows, Some(1));
    }

    #[test]
    fn page_blocks_handles_mixed_block_types() {
        let md =
            "# Title\n\nparagraph one\n\n- item a\n- item b\n\n| X | Y |\n|---|---|\n| 1 | 2 |\n";
        let blocks = blocks_of(md);
        let kinds: Vec<&str> = blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["heading", "paragraph", "list_item", "list_item", "table"]
        );
    }

    #[test]
    fn page_blocks_empty_input_yields_empty_vec() {
        assert!(blocks_of("").is_empty());
        assert!(blocks_of("\n\n\n   \n").is_empty());
    }

    #[test]
    fn page_blocks_classifies_fenced_code_block() {
        let md = "```rust\nfn main() {}\n```\nafter\n";
        let blocks = blocks_of(md);
        assert_eq!(blocks[0].kind, "code");
        assert_eq!(blocks[0].lang.as_deref(), Some("rust"));
        assert_eq!(blocks[1].kind, "paragraph");
    }
}
