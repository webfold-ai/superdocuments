//! Reads an arXiv e-print (the bytes served at `/e-print/<id>`) without touching the network.
//!
//! The caller supplies the bytes. `read_eprint` unpacks them in memory with safety limits
//! (`archive`), chooses the main TeX file, parses it with `latex` and resolves `\input` and
//! `\include` against the archive, collects bibliography entries from `.bib` files (or from
//! `.bbl` files and `thebibliography` items when there are no `.bib` files), and resolves each
//! figure's `\includegraphics` candidates to the archive's bytes, with the format detected from
//! the magic number.
//!
//! The library does not read the network, the file system, or the clock. Problems that do not
//! stop the read (non-UTF-8 source, missing images, duplicate bibliography keys) are returned in
//! `Source::warnings`.

mod archive;

#[cfg(test)]
mod tests;

pub(crate) use archive::crc32;
pub use archive::{Entry, Error as ArchiveError, Limits as ArchiveLimits};

use crate::bibtex::{self, Bibliography, Entry as BibEntry, Field};
use crate::latex::{self, BibItem, Document, Figure, FileProvider};

/// Limits for reading an e-print: archive bounds and LaTeX bounds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub archive: ArchiveLimits,
    pub latex: latex::Options,
}

/// The image formats the figure resolver recognises from the first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Pdf,
    Eps,
    /// Present in the archive, but not one of the formats above.
    Other,
}

/// Detects an image format from its magic number. EPS is recognised by `%!PS` in the first
/// kilobyte, which covers the DOS binary header some EPS files start with.
pub fn detect_image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(b"%PDF-") {
        ImageFormat::Pdf
    } else if bytes.starts_with(b"%!PS")
        || bytes.starts_with(&[0xc5, 0xd0, 0xd3, 0xc6])
        || bytes[..bytes.len().min(1024)]
            .windows(4)
            .any(|w| w == b"%!PS")
    {
        ImageFormat::Eps
    } else {
        ImageFormat::Other
    }
}

/// A parsed e-print.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The archive path of the main TeX file.
    pub main_file: String,
    pub document: Document,
    pub bibliography: Bibliography,
    /// Every regular file in the archive, by path.
    pub files: Vec<String>,
    /// The figures of the main document, in source order, with their image bytes.
    pub figures: Vec<FigureAssets>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FigureAssets {
    pub id: usize,
    pub env: String,
    pub label: Option<String>,
    pub number: Option<String>,
    pub caption_text: Option<String>,
    pub images: Vec<ImageAsset>,
    pub subfigures: Vec<FigureAssets>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageAsset {
    /// The path as written in `\includegraphics`.
    pub declared: String,
    /// The file that was found, or `None` when no candidate exists in the archive.
    pub file: Option<ImageFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFile {
    pub path: String,
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Archive(ArchiveError),
    /// The archive has no TeX file.
    NoTexFile,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Archive(e) => write!(f, "{e}"),
            Error::NoTexFile => write!(f, "the archive contains no TeX file"),
        }
    }
}

impl std::error::Error for Error {}

/// Reads an e-print with default limits.
pub fn read_eprint(data: &[u8]) -> Result<Source, Error> {
    read_eprint_with(data, &Options::default())
}

/// Reads an e-print with explicit limits.
pub fn read_eprint_with(data: &[u8], options: &Options) -> Result<Source, Error> {
    let entries = archive::unpack(data, &options.archive).map_err(Error::Archive)?;
    let mut warnings = Vec::new();
    // A gzip stream that is not a tar is taken as one TeX file. arXiv serves PDF-only
    // submissions the same way, and a PDF is not TeX.
    if let [only] = entries.as_slice() {
        if only.bytes.starts_with(b"%PDF-") {
            return Err(Error::NoTexFile);
        }
    }
    let main_index = select_main(&entries).ok_or(Error::NoTexFile)?;
    let main = &entries[main_index];
    let main_path = main.path.clone();
    let base = dir_of(&main_path);
    let text = decode_text(&main.bytes, &main_path, &mut warnings);
    let provider = ArchiveFiles {
        entries: &entries,
        base: base.clone(),
    };
    let document = latex::parse_with(&text, &options.latex, Some(&provider));
    for w in &document.warnings {
        warnings.push(format!("{main_path}:{}: {}", w.offset, w.message));
    }

    let bibliography = collect_bibliography(&entries, &document, &mut warnings);
    let figures: Vec<FigureAssets> = latex::figures(&document)
        .iter()
        .map(|f| resolve_figure(f, &entries, &base))
        .collect();
    for fig in &figures {
        note_missing_images(fig, &mut warnings);
    }
    Ok(Source {
        main_file: main_path,
        document,
        bibliography,
        files: entries.iter().map(|e| e.path.clone()).collect(),
        figures,
        warnings,
    })
}

/// Chooses the main file: a `.tex` file with both `\documentclass` and `\begin{document}`, then
/// one with `\begin{document}`, then one with `\documentclass`, then the largest `.tex` file.
/// Ties go to the larger file, then to the path order.
fn select_main(entries: &[Entry]) -> Option<usize> {
    let mut best: Option<(u8, std::cmp::Reverse<usize>, &str, usize)> = None;
    for (i, e) in entries.iter().enumerate() {
        if !e.path.ends_with(".tex") {
            continue;
        }
        let text = String::from_utf8_lossy(&e.bytes);
        let has_class = text.contains("\\documentclass");
        let has_body = text.contains("\\begin{document}");
        let tier = match (has_class, has_body) {
            (true, true) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (false, false) => 3,
        };
        let key = (tier, std::cmp::Reverse(e.bytes.len()), e.path.as_str(), i);
        if best
            .as_ref()
            .is_none_or(|b| (key.0, key.1, key.2) < (b.0, b.1, b.2))
        {
            best = Some(key);
        }
    }
    best.map(|b| b.3)
}

/// The directory part of an archive path, with a trailing `/`, or empty for the root.
fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..=i].to_string(),
        None => String::new(),
    }
}

/// UTF-8 when valid. Otherwise Latin-1, which is what older e-prints usually are.
fn decode_text(bytes: &[u8], path: &str, warnings: &mut Vec<String>) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            warnings.push(format!("{path} is not UTF-8; decoded as Latin-1"));
            bytes.iter().map(|&b| char::from(b)).collect()
        }
    }
}

/// Finds an archive file by a path relative to the main file's directory, then by the path as
/// written. Names that would leave the archive are never matched.
fn find_entry<'a>(entries: &'a [Entry], base: &str, name: &str) -> Option<&'a Entry> {
    let name = name.trim();
    let candidates = [format!("{base}{name}"), name.to_string()];
    for candidate in candidates {
        let Ok(path) = archive::validate_path(&candidate) else {
            continue;
        };
        if let Some(e) = entries.iter().find(|e| e.path == path) {
            return Some(e);
        }
    }
    None
}

struct ArchiveFiles<'a> {
    entries: &'a [Entry],
    base: String,
}

impl FileProvider for ArchiveFiles<'_> {
    fn read_tex(&self, name: &str) -> Option<String> {
        let found = find_entry(self.entries, &self.base, name).or_else(|| {
            if name.ends_with(".tex") {
                None
            } else {
                find_entry(self.entries, &self.base, &format!("{name}.tex"))
            }
        })?;
        let mut ignored = Vec::new();
        Some(decode_text(&found.bytes, &found.path, &mut ignored))
    }
}

fn collect_bibliography(
    entries: &[Entry],
    document: &Document,
    warnings: &mut Vec<String>,
) -> Bibliography {
    let mut bib = Bibliography::default();
    for e in entries.iter().filter(|e| e.path.ends_with(".bib")) {
        let text = decode_text(&e.bytes, &e.path, warnings);
        let parsed = bibtex::parse(&text);
        for w in &parsed.warnings {
            warnings.push(format!("{}:{}: {}", e.path, w.offset, w.message));
        }
        bib.entries.extend(parsed.entries);
        bib.strings.extend(parsed.strings);
        bib.preambles.extend(parsed.preambles);
        bib.comments.extend(parsed.comments);
    }
    if !bib.entries.is_empty() {
        return bib;
    }
    if !document.bibitems.is_empty() {
        bib.entries = bibitems_as_entries(&document.bibitems);
        return bib;
    }
    for e in entries.iter().filter(|e| e.path.ends_with(".bbl")) {
        let text = decode_text(&e.bytes, &e.path, warnings);
        let parsed = latex::parse(&text);
        bib.entries.extend(bibitems_as_entries(&parsed.bibitems));
    }
    bib
}

/// `\bibitem` items as bibliography entries: type `misc`, with `label` (if any), `raw` (the
/// source text) and `note` (the plain text) fields.
fn bibitems_as_entries(items: &[BibItem]) -> Vec<BibEntry> {
    items
        .iter()
        .map(|item| {
            let mut fields = Vec::new();
            if let Some(label) = &item.label {
                fields.push(Field {
                    name: "label".into(),
                    raw: label.clone(),
                    value: label.clone(),
                });
            }
            fields.push(Field {
                name: "raw".into(),
                raw: item.raw.clone(),
                value: item.raw.clone(),
            });
            let plain = latex::plain_text(&item.text);
            fields.push(Field {
                name: "note".into(),
                raw: plain.clone(),
                value: plain,
            });
            BibEntry {
                entry_type: "misc".into(),
                key: item.key.clone(),
                fields,
            }
        })
        .collect()
}

fn note_missing_images(fig: &FigureAssets, warnings: &mut Vec<String>) {
    for image in &fig.images {
        if image.file.is_none() {
            warnings.push(format!("image {} is not in the archive", image.declared));
        }
    }
    for sub in &fig.subfigures {
        note_missing_images(sub, warnings);
    }
}

fn resolve_figure(fig: &Figure, entries: &[Entry], base: &str) -> FigureAssets {
    let images = fig
        .image_paths
        .iter()
        .map(|p| ImageAsset {
            declared: p.declared.clone(),
            file: p.candidates.iter().find_map(|c| {
                find_entry(entries, base, c).map(|e| ImageFile {
                    path: e.path.clone(),
                    format: detect_image_format(&e.bytes),
                    bytes: e.bytes.clone(),
                })
            }),
        })
        .collect();
    FigureAssets {
        id: fig.id,
        env: fig.env.clone(),
        label: fig.label.clone(),
        number: fig.number.clone(),
        caption_text: fig.caption_text.clone(),
        images,
        subfigures: fig
            .subfigures
            .iter()
            .map(|s| resolve_figure(s, entries, base))
            .collect(),
    }
}
