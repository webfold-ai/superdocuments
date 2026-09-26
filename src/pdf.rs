//! In-process PDF-to-Markdown extraction via Transcode's parser, plus the legacy
//! `pdffold` process adapter for callers that still need it.
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
/// does not OCR image-only pages; those produce no text. Malformed input and parser
/// panics are returned as errors rather than escaping to the caller.
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
            transcode::doc::pdf::Document::parse(bytes).map_err(|error| match error {
                transcode::doc::pdf::PdfError::Parse(message) => message,
            })?;
        if document.object_count() == 0 || document.root() == 0 {
            return Err("PDF has no readable catalog".to_string());
        }
        Ok(document.markdown())
    }))
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PDF parser panicked"))?;
    parsed.map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
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
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            languages: "eng".into(),
            max_pages: 20,
            max_image_dimension: 2_400,
            timeout: Duration::from_secs(120),
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
    let native = extract_file(path);
    if let Ok(markdown) = &native {
        if has_embedded_text(markdown) {
            return Ok(markdown.clone());
        }
    }
    match tesseract_ocr(path, options) {
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

/// Force OCR for every page using the installed Poppler and Tesseract executables.
pub fn tesseract_ocr(path: impl AsRef<Path>, options: &OcrOptions) -> io::Result<String> {
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
    run_bounded(&mut render, deadline)?;

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
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rendered PDF pages exceed the disk-space limit",
        ));
    }
    if pages.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Poppler rendered no PDF pages",
        ));
    }

    let rendered_limit_reached = pages.len() == options.max_pages as usize;
    let mut output = String::new();
    for (number, image) in pages {
        let base = temp.0.join(format!("ocr-{number}"));
        let mut ocr = Command::new("tesseract");
        ocr.arg(image)
            .arg(&base)
            .arg("-l")
            .arg(&options.languages)
            .args(["--psm", "3"]);
        run_bounded(&mut ocr, deadline)?;
        let text = read_bounded_text(base.with_extension("txt"), 4 * 1024 * 1024)?;
        if !text.trim().is_empty() {
            if output.len().saturating_add(text.len()) > MAX_OCR_TOTAL_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "total OCR output exceeds size limit",
                ));
            }
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(&format!("<!-- page {number} -->\n{}", text.trim()));
        }
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
            extract_markdown(include_bytes!("../tests/fixtures/reportlab-invoice.pdf")).unwrap();
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
}
