//! arXiv source tests: archive safety, main-file selection, input inlining, bibliography and figure
//! assets. Fixtures live in `tests/fixtures/arxiv/`. They were generated with Python's `tarfile`
//! and `gzip` (real DEFLATE output) from sources written for this test; the JPEG is a stub with
//! the right magic number, not a decodable image.

use super::archive::test_support::{gzip_stored, tar};
use super::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/arxiv/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_default()
}

/// Declared paths of every inline `\includegraphics` in running text.
fn inline_images(doc: &latex::Document) -> Vec<String> {
    fn walk(blocks: &[latex::Block], out: &mut Vec<String>) {
        for b in blocks {
            if let latex::Block::Paragraph(inl) = b {
                for i in inl {
                    if let latex::Inline::Image(p) = i {
                        out.push(p.declared.clone());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    out
}

#[test]
fn paper_archive_gives_main_file_document_bibliography_and_figures() {
    let source = read_eprint(&fixture("paper.tar.gz")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(source.main_file, "main.tex");
    // \input{sections/intro} was inlined from the archive.
    let sections: Vec<String> = source
        .document
        .blocks
        .iter()
        .filter_map(|b| match b {
            latex::Block::Section(s) => Some(latex::plain_text(&s.title)),
            _ => None,
        })
        .collect();
    assert_eq!(sections, ["Introduction", "Method"]);
    assert!(source.document.inputs.is_empty());
    // Bibliography from refs.bib, with the BibLaTeX-style fields parsed.
    let keys: Vec<&str> = source
        .bibliography
        .entries
        .iter()
        .map(|e| e.key.as_str())
        .collect();
    assert_eq!(keys, ["vaswani2017", "knuth1984"]);
    let vaswani = source.bibliography.lookup("vaswani2017");
    assert_eq!(
        vaswani.map(|e| e.authors().names[1].family.clone()),
        Some("Shazeer".to_string())
    );
    assert_eq!(
        vaswani.and_then(|e| e.arxiv_id()).as_deref(),
        Some("1706.03762")
    );
    // Figures: the PDF and the PNG resolve to archive bytes, the EPS is detected by its header.
    assert_eq!(source.figures.len(), 2);
    let plot = &source.figures[0];
    assert_eq!(plot.label.as_deref(), Some("fig:plot"));
    assert_eq!(plot.number.as_deref(), Some("1"));
    let file = plot.images[0]
        .file
        .clone()
        .unwrap_or_else(|| panic!("plot image not found"));
    assert_eq!(file.path, "figs/plot.pdf");
    assert_eq!(file.format, ImageFormat::Pdf);
    assert_eq!(
        file.bytes,
        fixture_bytes_of_archive("paper.tar.gz", "figs/plot.pdf")
    );
    assert!(inline_images(&source.document).contains(&"figs/logo".to_string()));
    let diagram = &source.figures[1];
    let eps = diagram.images[0]
        .file
        .clone()
        .unwrap_or_else(|| panic!("eps not found"));
    assert_eq!(eps.format, ImageFormat::Eps);
    assert!(source.files.contains(&"figs/logo.png".to_string()));
    assert!(source.warnings.is_empty(), "{:?}", source.warnings);
}

/// The bytes of one archive member, for comparison with the figure assets.
fn fixture_bytes_of_archive(archive: &str, member: &str) -> Vec<u8> {
    let entries =
        archive::unpack(&fixture(archive), &archive::Limits::default()).unwrap_or_default();
    entries
        .into_iter()
        .find(|e| e.path == member)
        .map(|e| e.bytes)
        .unwrap_or_default()
}

#[test]
fn cited_keys_resolve_against_the_archive_bibliography() {
    let source = read_eprint(&fixture("paper.tar.gz")).unwrap_or_else(|e| panic!("{e}"));
    let links = latex::resolve_citations(&source.document, Some(&source.bibliography));
    let found: Vec<String> = links
        .iter()
        .filter(|l| l.source != latex::CitationSource::Unresolved)
        .map(|l| l.key.clone())
        .collect();
    assert_eq!(found, ["vaswani2017", "knuth1984"]);
}

#[test]
fn a_gzip_of_one_tex_file_is_read_as_main_tex() {
    let source = read_eprint(&fixture("single-file.tex.gz")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(source.main_file, "main.tex");
    assert_eq!(source.files, ["main.tex"]);
    assert!(source.bibliography.entries.is_empty());
    // The \bibliography{refs} target is not in the archive, and nothing there is missing.
    assert_eq!(source.document.bibliography_files, ["refs"]);
}

#[test]
fn bbl_only_archives_give_entries_with_raw_text() {
    let source = read_eprint(&fixture("bbl-only.tar.gz")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(source.bibliography.entries.len(), 1);
    let entry = &source.bibliography.entries[0];
    assert_eq!(entry.key, "vaswani2017");
    assert_eq!(entry.entry_type, "misc");
    let raw = entry.value("raw").unwrap_or_default();
    assert!(raw.contains("Attention is all you need"), "{raw}");
    assert!(entry.value("note").unwrap_or_default().contains("NeurIPS"));
    assert_eq!(entry.value("label"), Some("Vaswani et~al.(2017)"));
    let links = latex::resolve_citations(&source.document, Some(&source.bibliography));
    assert_eq!(links[0].source, latex::CitationSource::Bibliography);
}

#[test]
fn latin1_source_is_decoded_with_a_warning() {
    let source = read_eprint(&fixture("latin1.tar.gz")).unwrap_or_else(|e| panic!("{e}"));
    let text = latex::render_text(&source.document, &latex::RenderOptions::default());
    assert!(text.contains("Café notes."), "{text}");
    assert!(
        source.warnings.iter().any(|w| w.contains("Latin-1")),
        "{:?}",
        source.warnings
    );
}

#[test]
fn traversal_and_links_are_refused() {
    assert!(matches!(
        read_eprint(&fixture("traversal.tar.gz")),
        Err(Error::Archive(ArchiveError::UnsafePath(_)))
    ));
    assert!(matches!(
        read_eprint(&fixture("symlink.tar.gz")),
        Err(Error::Archive(ArchiveError::Link(_)))
    ));
}

#[test]
fn compression_bomb_is_refused_by_ratio() {
    let bomb = fixture("bomb.tar.gz");
    assert!(bomb.len() < 64 << 10, "the bomb fixture is small");
    // The inflate ceiling is the compressed size times the ratio limit, so the bomb fails inside
    // the decoder. The error says so.
    let err = read_eprint(&bomb);
    assert!(
        matches!(&err, Err(Error::Archive(ArchiveError::Corrupt(m))) if m.contains("ratio")),
        "{err:?}"
    );
}

#[test]
fn main_file_prefers_documentclass_with_body_then_size() {
    let archive = tar(&[
        ("big-fragment.tex", "x".repeat(4000).as_bytes(), b'0'),
        (
            "main.tex",
            b"\\documentclass{article}\\begin{document}Hi\\end{document}",
            b'0',
        ),
    ]);
    let source = read_eprint(&gzip_stored(&archive)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(source.main_file, "main.tex");
    let archive = tar(&[
        ("long.tex", "y".repeat(4000).as_bytes(), b'0'),
        ("paper.tex", b"\\begin{document}Body\\end{document}", b'0'),
    ]);
    let source = read_eprint(&gzip_stored(&archive)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(source.main_file, "paper.tex");
}

#[test]
fn archives_without_tex_are_an_error() {
    let archive = tar(&[("figs/only.png", b"\x89PNG\r\n\x1a\nxx", b'0')]);
    assert_eq!(
        read_eprint(&gzip_stored(&archive)).map(|s| s.main_file),
        Err(Error::NoTexFile)
    );
}

#[test]
fn inputs_resolve_relative_to_the_main_file_directory() {
    let archive = tar(&[
        (
            "paper/main.tex",
            b"\\documentclass{article}\\begin{document}\\input{part}\\end{document}",
            b'0',
        ),
        (
            "paper/part.tex",
            b"\\section{Part}\\label{sec:part}Text.",
            b'0',
        ),
    ]);
    let source = read_eprint(&gzip_stored(&archive)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(source.main_file, "paper/main.tex");
    assert_eq!(source.document.label_number("sec:part"), Some("1"));
    assert!(source.document.inputs.is_empty());
}

#[test]
fn missing_images_are_reported_not_invented() {
    let archive = tar(&[(
        "main.tex",
        b"\\documentclass{article}\\begin{document}\\begin{figure}\\includegraphics{gone}\\caption{x}\\end{figure}\\end{document}",
        b'0',
    )]);
    let source = read_eprint(&gzip_stored(&archive)).unwrap_or_else(|e| panic!("{e}"));
    assert!(source.figures[0].images[0].file.is_none());
    assert!(
        source.warnings.iter().any(|w| w.contains("gone")),
        "{:?}",
        source.warnings
    );
}

#[test]
fn image_formats_are_detected_by_magic_number() {
    assert_eq!(
        detect_image_format(b"\x89PNG\r\n\x1a\nrest"),
        ImageFormat::Png
    );
    assert_eq!(
        detect_image_format(&[0xff, 0xd8, 0xff, 0xe0]),
        ImageFormat::Jpeg
    );
    assert_eq!(detect_image_format(b"%PDF-1.7\n"), ImageFormat::Pdf);
    assert_eq!(
        detect_image_format(b"%!PS-Adobe-3.0 EPSF"),
        ImageFormat::Eps
    );
    assert_eq!(
        detect_image_format(&[0xc5, 0xd0, 0xd3, 0xc6, 0, 0]),
        ImageFormat::Eps
    );
    assert_eq!(detect_image_format(b"GIF89a"), ImageFormat::Other);
    assert_eq!(detect_image_format(b""), ImageFormat::Other);
}

#[test]
fn truncated_and_mutated_archives_never_panic() {
    let original = fixture("paper.tar.gz");
    for cut in (0..original.len()).step_by(37) {
        let _ = read_eprint(&original[..cut]);
    }
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for _ in 0..200 {
        let mut bytes = original.clone();
        for _ in 0..3 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = (state as usize) % bytes.len();
            bytes[at] = (state >> 8) as u8;
        }
        let _ = read_eprint(&bytes);
    }
}

#[test]
fn a_gzipped_pdf_is_not_read_as_tex() {
    // arXiv serves a PDF-only submission as a gzip stream of the PDF.
    let pdf = gzip_stored(b"%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n");
    assert_eq!(
        read_eprint(&pdf).map(|s| s.main_file),
        Err(Error::NoTexFile)
    );
    assert_eq!(
        crate::document::convert_bytes("gz", &pdf).unwrap_err(),
        "the archive contains no TeX file"
    );
    // A gzip stream of TeX is still one TeX file.
    let tex = gzip_stored(b"\\section{Kept} Body with %PDF- in the text.");
    assert_eq!(
        read_eprint(&tex).map(|s| s.main_file).as_deref(),
        Ok("main.tex")
    );
}
