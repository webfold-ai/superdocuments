//! MarathonDocBench lane: fetch | subset | run | score.
//!
//! A Rust port of the four Python scripts this lane started as (`fetch.py`, `make_subset_gt.py`, `run.py`,
//! `score_provisional.py`); see `marathondocbench/README.md` for what the lane measures and what it does not.
//!
//!   marathondocbench fetch  --out DIR [--ids A B | --shortest N | --all]
//!   marathondocbench subset --data DIR
//!   WEBFOLD_BIN=... marathondocbench run --data DIR --out DIR/pred [--systems webfold,webfold_default,tesseract] [--lang L] [--jobs 2]
//!   marathondocbench score  --data DIR --pred DIR/pred [--json out.json]
//!
//! The provisional score is a text-only character similarity (1 - normalised Levenshtein distance of whole-document
//! strings). It is NOT an MPDocBench metric.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use regex::Regex;
use serde_json::value::RawValue;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

const BASE: &str = "https://huggingface.co/datasets/baidu/MarathonDocBench/resolve/main/";

#[derive(Parser)]
#[command(name = "marathondocbench", about = "MarathonDocBench lane: fetch, subset, run, score")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download the ground truth and the page images of chosen documents into a cache directory.
    Fetch {
        #[arg(long)]
        out: String,
        /// named documents, e.g. --ids mp3lan_0101 mp3lan_0778
        #[arg(long, num_args = 1..)]
        ids: Vec<String>,
        /// the N documents with the fewest pages
        #[arg(long)]
        shortest: Option<usize>,
        /// all 100 documents, about 18,000 page images
        #[arg(long)]
        all: bool,
    },
    /// Cut MarathonDocBench.json down to the documents fetch selected.
    Subset {
        #[arg(long)]
        data: String,
    },
    /// Write one Markdown file per document per system.
    Run {
        #[arg(long)]
        data: String,
        #[arg(long)]
        out: String,
        #[arg(long, default_value = "webfold,webfold_default,tesseract")]
        systems: String,
        #[arg(long, default_value = "chi_sim+chi_tra+eng")]
        lang: String,
        /// accepted for compatibility with run.py, where it was never used
        #[arg(long, default_value_t = 3600)]
        timeout: i64,
        #[arg(long, default_value_t = 2)]
        jobs: usize,
        /// longest rendered page edge; webfold allows pages x edge^2 x 4 bytes up to 512 MiB per call
        #[arg(long, default_value_t = 1800)]
        dim: u32,
        /// pages per webfold call (its OCR allows at most 100 pages and 600 s per file)
        #[arg(long, default_value_t = 40)]
        chunk: usize,
    },
    /// Provisional text-only score of the predictions `run` wrote. Not an MPDocBench metric.
    Score {
        #[arg(long)]
        data: String,
        #[arg(long)]
        pred: String,
        #[arg(long)]
        json: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Fetch { out, ids, shortest, all } => fetch(&out, ids, shortest, all),
        Cmd::Subset { data } => subset(&data),
        Cmd::Run { data, out, systems, lang, timeout: _, jobs, dim, chunk } => run(RunArgs {
            webfold: std::env::var("WEBFOLD_BIN").unwrap_or_else(|_| "webfold".into()),
            data: expand(&data),
            out: expand(&out),
            systems,
            lang,
            jobs: jobs.max(1),
            dim,
            chunk: chunk.max(1),
        }),
        Cmd::Score { data, pred, json } => score(&data, &pred, json.as_deref()),
    }
}

fn expand(p: &str) -> PathBuf {
    if p == "~" || p.starts_with("~/") {
        if let Ok(h) = std::env::var("HOME") {
            return PathBuf::from(h).join(p.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    PathBuf::from(p)
}

// ---------------------------------------------------------------------------------------------------------------
// JSON output in the layout Python's json.dump(..., indent=1) produced, keys in insertion order.

enum J {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    fn dump(&self, level: usize, out: &mut String) {
        let pad = |n: usize| " ".repeat(n);
        match self {
            J::Null => out.push_str("null"),
            J::Int(i) => out.push_str(&i.to_string()),
            J::Float(f) => out.push_str(&format!("{f:?}")),
            J::Str(s) => out.push_str(&serde_json::to_string(s).unwrap()),
            J::Arr(v) if v.is_empty() => out.push_str("[]"),
            J::Arr(v) => {
                out.push_str("[\n");
                for (i, x) in v.iter().enumerate() {
                    out.push_str(&pad(level + 1));
                    x.dump(level + 1, out);
                    out.push_str(if i + 1 < v.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(level));
                out.push(']');
            }
            J::Obj(v) if v.is_empty() => out.push_str("{}"),
            J::Obj(v) => {
                out.push_str("{\n");
                for (i, (k, x)) in v.iter().enumerate() {
                    out.push_str(&pad(level + 1));
                    out.push_str(&serde_json::to_string(k).unwrap());
                    out.push_str(": ");
                    x.dump(level + 1, out);
                    out.push_str(if i + 1 < v.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(level));
                out.push('}');
            }
        }
    }
    fn to_file(&self, path: &Path) -> Result<()> {
        let mut s = String::new();
        self.dump(0, &mut s);
        fs::write(path, s).with_context(|| format!("write {}", path.display()))
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let b = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&b).with_context(|| format!("parse {}", path.display()))
}

struct Selection {
    docs: Vec<String>,
    pages: HashMap<String, u64>,
}

fn read_selection(data: &Path) -> Result<Selection> {
    let v = read_json(&data.join("selection.json"))?;
    let docs = v["docs"].as_array().ok_or_else(|| anyhow!("selection.json: no docs"))?.iter().filter_map(|d| d.as_str().map(String::from)).collect();
    let pages = v["pages"].as_object().map(|m| m.iter().filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n))).collect()).unwrap_or_default();
    Ok(Selection { docs, pages })
}

// ---------------------------------------------------------------------------------------------------------------
// fetch

fn http_client() -> Result<reqwest::blocking::Client> {
    let mut b = reqwest::blocking::Client::builder().timeout(Duration::from_secs(120));
    // an HTTPS proxy that re-signs traffic hands its CA over through one of these variables; urllib (and so fetch.py) honoured them
    for var in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"] {
        if let Ok(p) = std::env::var(var) {
            if let Ok(pem) = fs::read(&p) {
                for c in reqwest::Certificate::from_pem_bundle(&pem).unwrap_or_default() {
                    b = b.add_root_certificate(c);
                }
            }
        }
    }
    Ok(b.build()?)
}

fn get(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<()> {
    if fs::metadata(dest).map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(());
    }
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d)?;
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let tries = 4;
    for i in 0..tries {
        let attempt = (|| -> Result<()> {
            let mut r = client.get(url).send()?.error_for_status()?;
            let mut f = fs::File::create(&part)?;
            r.copy_to(&mut f)?;
            f.flush()?;
            fs::rename(&part, dest)?;
            Ok(())
        })();
        match attempt {
            Ok(()) => return Ok(()),
            Err(e) if i == tries - 1 => return Err(e.context(format!("GET {url}"))),
            Err(_) => std::thread::sleep(Duration::from_secs(1 << i)),
        }
    }
    unreachable!()
}

fn fetch(out: &str, ids: Vec<String>, shortest: Option<usize>, all: bool) -> Result<()> {
    let out = expand(out);
    let client = http_client()?;
    get(&client, &format!("{BASE}MarathonDocBench.json"), &out.join("MarathonDocBench.json"))?;
    get(&client, &format!("{BASE}images/metadata.jsonl"), &out.join("images/metadata.jsonl"))?;
    let mut pages: HashMap<String, u64> = HashMap::new();
    for line in fs::read_to_string(out.join("images/metadata.jsonl"))?.lines().filter(|l| !l.trim().is_empty()) {
        let m: Value = serde_json::from_str(line)?;
        let (Some(id), Some(n)) = (m["doc_id"].as_str(), m["num_pages"].as_u64()) else { bail!("bad metadata line: {line}") };
        pages.entry(id.to_string()).or_insert(n);
    }
    let ids: Vec<String> = if all {
        let mut v: Vec<_> = pages.keys().cloned().collect();
        v.sort();
        v
    } else if !ids.is_empty() {
        ids
    } else if let Some(n) = shortest.filter(|n| *n > 0) {
        let mut v: Vec<(&String, &u64)> = pages.iter().collect();
        v.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
        v.into_iter().take(n).map(|(d, _)| d.clone()).collect()
    } else {
        bail!("choose --ids, --shortest N or --all");
    };
    let mut jobs = Vec::new();
    for d in &ids {
        let n = *pages.get(d).ok_or_else(|| anyhow!("unknown document {d}"))?;
        for p in 1..=n {
            jobs.push((format!("{BASE}images/{d}/page_{p}.png"), out.join(format!("images/{d}/page_{p}.png"))));
        }
    }
    let next = AtomicUsize::new(0);
    let errors: Mutex<Vec<anyhow::Error>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((url, dest)) = jobs.get(i) else { break };
                if let Err(e) = get(&client, url, dest) {
                    errors.lock().unwrap().push(e);
                    next.store(jobs.len(), Ordering::SeqCst);
                    break;
                }
            });
        }
    });
    if let Some(e) = errors.into_inner().unwrap().into_iter().next() {
        return Err(e);
    }
    J::Obj(vec![
        ("docs".into(), J::Arr(ids.iter().cloned().map(J::Str).collect())),
        ("pages".into(), J::Obj(ids.iter().map(|d| (d.clone(), J::Int(pages[d] as i64))).collect())),
    ])
    .to_file(&out.join("selection.json"))?;
    println!("{} documents, {} page images in {}", ids.len(), jobs.len(), out.display());
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// subset

fn gt_doc_id(entry: &Value) -> String {
    entry["page_info"]["image_path"].as_str().unwrap_or("").replace(".pdf", "")
}

fn subset(data: &str) -> Result<()> {
    let d = expand(data);
    let docs = read_selection(&d)?.docs;
    let raw = fs::read_to_string(d.join("MarathonDocBench.json"))?;
    let all: Vec<Box<RawValue>> = serde_json::from_str(&raw)?;
    let mut keep: Vec<&str> = Vec::new();
    for e in &all {
        let v: Value = serde_json::from_str(e.get())?;
        if docs.contains(&gt_doc_id(&v)) {
            keep.push(e.get());
        }
    }
    // the entries are copied verbatim, so key order and number formatting survive; only the whitespace between entries is ours
    fs::write(d.join("MarathonDocBench.subset.json"), format!("[{}]", keep.join(", ")))?;
    println!("{} of 100 documents -> {}", keep.len(), d.join("MarathonDocBench.subset.json").display());
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// run: page PNGs -> image-only PDFs

/// Decode a PNG to 8-bit RGB the way Pillow's `.convert("RGB")` does (alpha dropped, grey replicated).
fn load_rgb(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut dec = png::Decoder::new(BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().with_context(|| format!("decode {}", path.display()))?;
    let mut buf = vec![0u8; r.output_buffer_size().ok_or_else(|| anyhow!("image too large: {}", path.display()))?];
    let info = r.next_frame(&mut buf).with_context(|| format!("decode {}", path.display()))?;
    let px = &buf[..info.buffer_size()];
    let n = (info.width as usize) * (info.height as usize);
    let mut rgb = Vec::with_capacity(n * 3);
    match info.color_type {
        png::ColorType::Rgb => rgb.extend_from_slice(px),
        png::ColorType::Rgba => px.chunks_exact(4).for_each(|p| rgb.extend_from_slice(&p[..3])),
        png::ColorType::Grayscale => px.iter().for_each(|&g| rgb.extend_from_slice(&[g, g, g])),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).for_each(|p| rgb.extend_from_slice(&[p[0], p[0], p[0]])),
        png::ColorType::Indexed => bail!("{}: palette image was not expanded", path.display()),
    }
    Ok((info.width, info.height, rgb))
}

struct PdfPage {
    w: u32,
    h: u32,
    /// zlib-compressed RGB samples
    data: Vec<u8>,
}

fn pdf_page(w: u32, h: u32, rgb: &[u8]) -> Result<PdfPage> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(rgb)?;
    Ok(PdfPage { w, h, data: z.finish()? })
}

/// A minimal PDF: one page per image, the page the size of the image in pixels (points at 72 dpi, as Pillow wrote it),
/// the image a FlateDecode DeviceRGB XObject painted over the whole page.
fn write_pdf(pages: &[PdfPage]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 3 + 3 * i)).collect();
    obj(&mut out, format!("<< /Type /Pages /Count {} /Kids [{}] >>", pages.len(), kids.join(" ")).as_bytes());
    for (i, p) in pages.iter().enumerate() {
        let (content, image) = (4 + 3 * i, 5 + 3 * i);
        obj(
            &mut out,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {content} 0 R >>", p.w, p.h).as_bytes(),
        );
        let c = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", p.w, p.h);
        obj(&mut out, format!("<< /Length {} >>\nstream\n{c}\nendstream", c.len()).as_bytes());
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            p.w,
            p.h,
            p.data.len()
        )
        .into_bytes();
        body.extend_from_slice(&p.data);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

struct RunArgs {
    webfold: String,
    data: PathBuf,
    out: PathBuf,
    systems: String,
    lang: String,
    jobs: usize,
    dim: u32,
    chunk: usize,
}

impl RunArgs {
    fn tmp(&self) -> PathBuf {
        self.out.join("_tmp")
    }
}

/// An image-only PDF of pages first..=last (1-based) written to the tmp directory.
fn pdf_of(doc: &str, a: &RunArgs, first: usize, last: usize) -> Result<PathBuf> {
    let mut pages = Vec::new();
    for p in first..=last {
        let (w, h, rgb) = load_rgb(&a.data.join(format!("images/{doc}/page_{p}.png")))?;
        pages.push(pdf_page(w, h, &rgb)?);
    }
    let path = a.tmp().join(format!("{doc}.{first}-{last}.pdf"));
    fs::write(&path, write_pdf(&pages))?;
    Ok(path)
}

/// webfold marks pages `<!-- page N -->` counting from the chunk's first page; make N the document's page number.
fn renumber(md: &str, offset: usize) -> String {
    let re = Regex::new(r"<!-- page (\d+) -->").unwrap();
    re.replace_all(md, |c: &regex::Captures| format!("<!-- page {} -->", c[1].parse::<usize>().unwrap_or(0) + offset)).into_owned()
}

/// Run a command with a wall-clock limit; returns (exit code, stdout). Like subprocess.run(timeout=...), a timeout is an error.
fn run_capture(cmd: &mut Command, secs: u64) -> Result<(i32, String)> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("spawn")?;
    let mut so = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(secs) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("command timed out after {secs}s");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = reader.join().map_err(|_| anyhow!("reader thread panicked"))?;
    Ok((status.code().unwrap_or(-1), String::from_utf8_lossy(&out).into_owned()))
}

struct DocRun {
    md: String,
    pages: usize,
    secs: f64,
    rc: i32,
}

fn run_webfold(doc: &str, a: &RunArgs, default: bool, pages: usize) -> Result<DocRun> {
    let t = Instant::now();
    if default {
        // `webfold pdf` as shipped, on the whole document
        let pdf = pdf_of(doc, a, 1, pages)?;
        let r = run_capture(Command::new(&a.webfold).arg("pdf").arg(&pdf).env("OMP_THREAD_LIMIT", "1"), 1800);
        let _ = fs::remove_file(&pdf);
        let (rc, so) = r?;
        return Ok(DocRun { md: if rc == 0 { so } else { String::new() }, pages, secs: t.elapsed().as_secs_f64(), rc });
    }
    // superdocuments' OCR refuses more than 100 pages or 600 s per file, so a long document goes through in chunks
    let (mut out, mut rc) = (Vec::new(), 0);
    let mut first = 1;
    while first <= pages {
        let last = pages.min(first + a.chunk - 1);
        let pdf = pdf_of(doc, a, first, last)?;
        let r = run_capture(
            Command::new(&a.webfold)
                .arg("pdf")
                .arg(&pdf)
                .args(["--ocr-lang", &a.lang, "--ocr-max-pages", &a.chunk.to_string(), "--ocr-max-dim", &a.dim.to_string(), "--ocr-timeout", "600"])
                .env("OMP_THREAD_LIMIT", "1"),
            1800,
        );
        let _ = fs::remove_file(&pdf);
        let (code, so) = r?;
        rc |= code;
        out.push(if code == 0 { renumber(&so, first - 1) } else { format!("<!-- pages {first}-{last}: webfold exit {code} -->") });
        first += a.chunk;
    }
    Ok(DocRun { md: out.join("\n\n"), pages, secs: t.elapsed().as_secs_f64(), rc })
}

fn run_tesseract(doc: &str, a: &RunArgs, pages: usize) -> Result<DocRun> {
    let t = Instant::now();
    let (mut out, mut rc) = (Vec::new(), 0);
    for p in 1..=pages {
        let img = a.data.join(format!("images/{doc}/page_{p}.png"));
        let r = Command::new("tesseract").arg(&img).args(["-", "-l", &a.lang]).env("OMP_THREAD_LIMIT", "1").stderr(Stdio::null()).output().context("spawn tesseract")?;
        rc |= r.status.code().unwrap_or(-1);
        out.push(String::from_utf8_lossy(&r.stdout).into_owned());
    }
    Ok(DocRun { md: out.join("\n\n"), pages, secs: t.elapsed().as_secs_f64(), rc })
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn run(a: RunArgs) -> Result<()> {
    fs::create_dir_all(a.tmp())?;
    let sel = read_selection(&a.data)?;
    let mut timings: Vec<(String, J)> = Vec::new();
    for system in a.systems.split(',') {
        fs::create_dir_all(a.out.join(system))?;
        let (tx, rx) = mpsc::channel::<(usize, Result<DocRun>)>();
        let next = AtomicUsize::new(0);
        let mut per_doc: Vec<(String, J)> = Vec::new();
        let mut failure: Option<anyhow::Error> = None;
        std::thread::scope(|s| {
            for _ in 0..a.jobs {
                let tx = tx.clone();
                let (a, sel, next) = (&a, &sel, &next);
                s.spawn(move || loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(doc) = sel.docs.get(i) else { break };
                    let r = match sel.pages.get(doc) {
                        None => Err(anyhow!("selection.json has no page count for {doc}")),
                        Some(&n) if system == "tesseract" => run_tesseract(doc, a, n as usize),
                        Some(&n) => run_webfold(doc, a, system == "webfold_default", n as usize),
                    };
                    if tx.send((i, r)).is_err() {
                        break;
                    }
                });
            }
            drop(tx);
            // results are consumed in document order, as ThreadPoolExecutor.map yielded them
            let mut pending: BTreeMap<usize, Result<DocRun>> = BTreeMap::new();
            let mut want = 0;
            while want < sel.docs.len() {
                while !pending.contains_key(&want) {
                    match rx.recv() {
                        Ok((i, r)) => {
                            pending.insert(i, r);
                        }
                        Err(_) => return,
                    }
                }
                let doc = &sel.docs[want];
                match pending.remove(&want).unwrap() {
                    Err(e) => {
                        failure = Some(e.context(format!("{system} {doc}")));
                        next.store(sel.docs.len(), Ordering::SeqCst);
                        return;
                    }
                    Ok(r) => {
                        if let Err(e) = fs::write(a.out.join(system).join(format!("{doc}.md")), &r.md) {
                            failure = Some(e.into());
                            next.store(sel.docs.len(), Ordering::SeqCst);
                            return;
                        }
                        let chars = r.md.chars().count();
                        per_doc.push((
                            doc.clone(),
                            J::Obj(vec![
                                ("pages".into(), J::Int(r.pages as i64)),
                                ("seconds".into(), J::Float(round1(r.secs))),
                                ("exit".into(), J::Int(r.rc as i64)),
                                ("chars".into(), J::Int(chars as i64)),
                            ]),
                        ));
                        println!("{:16} {}: {} pages, {:.0}s, exit {}, {} chars", system, doc, r.pages, r.secs, r.rc, chars);
                    }
                }
                want += 1;
            }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        timings.push((system.to_string(), J::Obj(per_doc)));
    }
    J::Obj(timings).to_file(&a.out.join("timings.json"))
}

// ---------------------------------------------------------------------------------------------------------------
// score

const TEXT_KINDS: [&str; 9] = ["text_block", "title", "figure_caption", "figure_footnote", "table_caption", "table_footnote", "header", "footer", "page_number"];

/// NFKC, whitespace removed, lower-cased. Whitespace is Python's `\s` for str: Unicode White_Space plus U+001C..U+001F.
fn norm(s: &str) -> Vec<char> {
    let n: String = s.nfkc().filter(|c| !(c.is_whitespace() || ('\x1c'..='\x1f').contains(c))).collect();
    n.to_lowercase().chars().collect()
}

fn strip_tags(h: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    html_escape::decode_html_entities(&re.replace_all(h, " ")).into_owned()
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn gt_text(entry: &Value) -> Result<Vec<char>> {
    let dets = entry["layout_dets"].as_array().ok_or_else(|| anyhow!("ground truth entry without layout_dets"))?;
    let mut items: Vec<&Value> = dets.iter().filter(|d| !truthy(d.get("ignore"))).collect();
    // (order is None, order): items without an order go last; the sort is stable like Python's
    let key = |d: &&Value| match d.get("order").and_then(|o| o.as_f64()) {
        Some(o) => (false, o),
        None => (true, 0.0),
    };
    items.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut out: Vec<String> = Vec::new();
    for d in items {
        let k = d["category_type"].as_str().unwrap_or("");
        if TEXT_KINDS.contains(&k) {
            out.push(d.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string());
        } else if k == "table" {
            out.push(strip_tags(d.get("html").and_then(|t| t.as_str()).unwrap_or("")));
        }
    }
    Ok(norm(&out.join("\n")))
}

fn pred_text(md: &str) -> Vec<char> {
    let comments = Regex::new(r"(?s)<!--.*?-->").unwrap();
    let images = Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap();
    let links = Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap();
    let lead = Regex::new(r"(?m)^[ \t]*(#{1,6}|>|[-*+])[ \t]+").unwrap();
    let marks = Regex::new(r"[*_`~]+").unwrap();
    let md = comments.replace_all(md, " ");
    let md = images.replace_all(&md, " ");
    let md = links.replace_all(&md, "${1}");
    let md = lead.replace_all(&md, "");
    let md = md.replace('|', " ");
    let md = marks.replace_all(&md, "");
    norm(&md)
}

/// Levenshtein distance (unit insert, delete, substitute) by Myers' bit-vector algorithm in Hyyro's block form:
/// the shorter string is the pattern, 64 of its characters per machine word, O(n * ceil(m / 64)).
pub fn levenshtein(a: &[char], b: &[char]) -> usize {
    let (pat, text) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let m = pat.len();
    if m == 0 {
        return text.len();
    }
    let words = (m + 63) / 64;
    // pattern-match masks, one row of `words` per distinct pattern character; row 0 is all zero (character not in pattern)
    let mut index: HashMap<char, usize> = HashMap::new();
    let mut peq: Vec<u64> = vec![0; words];
    for (i, &c) in pat.iter().enumerate() {
        let next = index.len() + 1;
        let row = *index.entry(c).or_insert(next);
        if peq.len() < (row + 1) * words {
            peq.resize((row + 1) * words, 0);
        }
        peq[row * words + i / 64] |= 1u64 << (i % 64);
    }
    let mut pv = vec![!0u64; words];
    let mut mv = vec![0u64; words];
    let last_bit = 1u64 << ((m - 1) % 64);
    let mut score = m as i64;
    for &c in text {
        let row = index.get(&c).copied().unwrap_or(0);
        let eqs = &peq[row * words..(row + 1) * words];
        let mut hin: i32 = 1; // the first row of the distance matrix grows by one per text character
        for w in 0..words {
            let (p, mvw) = (pv[w], mv[w]);
            let mut eq = eqs[w];
            let xv = eq | mvw;
            if hin < 0 {
                eq |= 1;
            }
            let xh = (((eq & p).wrapping_add(p)) ^ p) | eq;
            let mut ph = mvw | !(xh | p);
            let mut mh = p & xh;
            let top = if w == words - 1 { last_bit } else { 1u64 << 63 };
            let hout: i32 = if ph & top != 0 {
                1
            } else if mh & top != 0 {
                -1
            } else {
                0
            };
            ph <<= 1;
            mh <<= 1;
            if hin < 0 {
                mh |= 1;
            } else if hin > 0 {
                ph |= 1;
            }
            pv[w] = mh | !(xv | ph);
            mv[w] = ph & xv;
            hin = hout;
            if w == words - 1 {
                score += hout as i64;
            }
        }
    }
    score as usize
}

/// rapidfuzz's `Levenshtein.normalized_similarity`: 1 - distance / max(len a, len b).
pub fn normalized_similarity(a: &[char], b: &[char]) -> f64 {
    let max = a.len().max(b.len());
    if max == 0 {
        return 1.0;
    }
    1.0 - levenshtein(a, b) as f64 / max as f64
}

fn round4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

fn score(data: &str, pred: &str, json_out: Option<&str>) -> Result<()> {
    let (data, pred) = (expand(data), expand(pred));
    let gt_all = read_json(&data.join("MarathonDocBench.json"))?;
    let gt: HashMap<String, &Value> = gt_all.as_array().ok_or_else(|| anyhow!("MarathonDocBench.json is not a list"))?.iter().map(|e| (gt_doc_id(e), e)).collect();
    let docs = read_selection(&data)?.docs;
    let mut systems: Vec<String> = fs::read_dir(&pred)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('_'))
        .collect();
    systems.sort();
    let tpath = pred.join("timings.json");
    let timings = if tpath.exists() { read_json(&tpath)? } else { Value::Null };
    println!("| system | document | pages | GT chars | predicted chars | similarity | seconds |");
    println!("|---|---|---:|---:|---:|---:|---:|");
    let show = |v: Option<&Value>| match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::Number(n)) if n.is_f64() => format!("{:?}", n.as_f64().unwrap()),
        Some(v) => v.to_string(),
    };
    let mut res: Vec<(String, J)> = Vec::new();
    for s in &systems {
        let mut per_doc: Vec<(String, J)> = Vec::new();
        for d in &docs {
            let p = pred.join(s).join(format!("{d}.md"));
            let g = gt_text(gt.get(d).ok_or_else(|| anyhow!("{d} is not in the ground truth"))?)?;
            let q = if p.exists() { pred_text(&String::from_utf8_lossy(&fs::read(&p)?)) } else { Vec::new() };
            let sim = normalized_similarity(&g, &q);
            let t = &timings[s.as_str()][d.as_str()];
            println!("| {s} | {d} | {} | {} | {} | {sim:.3} | {} |", show(t.get("pages")), g.len(), q.len(), show(t.get("seconds")));
            per_doc.push((
                d.clone(),
                J::Obj(vec![
                    ("similarity".into(), J::Float(round4(sim))),
                    ("gt_chars".into(), J::Int(g.len() as i64)),
                    ("pred_chars".into(), J::Int(q.len() as i64)),
                    ("seconds".into(), t.get("seconds").and_then(|x| x.as_f64()).map(J::Float).unwrap_or(J::Null)),
                ]),
            ));
        }
        res.push((s.clone(), J::Obj(per_doc)));
    }
    if let Some(j) = json_out {
        J::Obj(res).to_file(&expand(j))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(a: &[char], b: &[char]) -> usize {
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i; b.len() + 1];
            for j in 1..=b.len() {
                cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
            }
            prev = cur;
        }
        prev[b.len()]
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    fn random(r: &mut Lcg, max_len: u64, alphabet: u64) -> Vec<char> {
        let n = r.next() % (max_len + 1);
        (0..n).map(|_| char::from_u32(0x4e00 + (r.next() % alphabet) as u32).unwrap()).collect()
    }

    #[test]
    fn levenshtein_matches_naive_dp() {
        let mut r = Lcg(42);
        for case in 0..3000 {
            // lengths straddle the 64 and 128 character word boundaries; small alphabets make matches dense
            let (max, alpha) = [(8, 3), (70, 4), (140, 2), (200, 26), (300, 5)][case % 5];
            let (a, b) = (random(&mut r, max, alpha), random(&mut r, max, alpha));
            assert_eq!(levenshtein(&a, &b), naive(&a, &b), "a={a:?} b={b:?}");
        }
    }

    #[test]
    fn levenshtein_edges() {
        let s = |x: &str| x.chars().collect::<Vec<_>>();
        assert_eq!(levenshtein(&s(""), &s("")), 0);
        assert_eq!(levenshtein(&s(""), &s("abc")), 3);
        assert_eq!(levenshtein(&s("kitten"), &s("sitting")), 3);
        let long: Vec<char> = "ab".repeat(64).chars().collect(); // exactly 128 = two full words
        assert_eq!(levenshtein(&long, &long), 0);
        assert_eq!(levenshtein(&long, &long[1..]), 1);
        assert_eq!(normalized_similarity(&s(""), &s("")), 1.0);
        assert_eq!(normalized_similarity(&s("abcd"), &s("abcx")), 0.75);
    }

    #[test]
    fn pdf_has_xref_and_one_page_per_image() {
        let p = pdf_page(2, 1, &[1, 2, 3, 4, 5, 6]).unwrap();
        let pdf = write_pdf(&[p, pdf_page(1, 1, &[9, 9, 9]).unwrap()]);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        assert!(text.contains("/MediaBox [0 0 2 1]"));
        let start: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[start..].starts_with(b"xref\n0 9\n"));
    }

    #[test]
    fn renumbers_pages() {
        assert_eq!(renumber("<!-- page 1 -->\nx\n<!-- page 12 -->", 40), "<!-- page 41 -->\nx\n<!-- page 52 -->");
    }

    #[test]
    fn prediction_markup_is_stripped() {
        let s: String = pred_text("<!-- page 1 -->\n# Title\n- item **bold** [link](http://x) ![](img_1.png) a|b").into_iter().collect();
        assert_eq!(s, "titleitembold" .to_string() + "linkab");
    }
}
