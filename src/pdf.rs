//! In-process PDF-to-Markdown extraction via Transcode's parser, plus the legacy
//! `pdffold` process adapter for callers that still need it.
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::process::Command;

/// Bound parser input before scanning attacker-controlled bytes.
pub const MAX_PDF_BYTES: usize = 256 * 1024 * 1024;

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
    fn process_adapter_returns_stdout_and_surfaces_failures() {
        let mut success = Command::new("sh");
        success.args(["-c", "printf converted"]);
        assert_eq!(run(&mut success).unwrap(), "converted");

        let mut failure = Command::new("sh");
        failure.args(["-c", "printf failed >&2; exit 7"]);
        assert_eq!(run(&mut failure).unwrap_err().to_string(), "failed");
    }
}
