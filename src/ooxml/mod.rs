//! Shared pieces of the OOXML readers (DOCX, PPTX, XLSX).
//!
//! Elements and attributes are matched by *local* name, so `w:p`, `p` and `ns0:p` are the same
//! element: the prefix is whatever the producing tool chose, and only the namespace URI is
//! fixed. Matching on `w:`/`a:`/`p:` literally, as the older readers did, reads nothing from a
//! file written by a tool that binds a different prefix.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::Cursor;
use zip::ZipArchive;

use crate::document::{entry, text};

pub(crate) mod docx;
pub(crate) mod omml;
pub(crate) mod pptx;
pub(crate) mod numfmt;
pub(crate) mod xlsx;

/// An attribute by local name: `val` finds `w:val`, `val` and `ns0:val`.
pub(crate) fn attr_local(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .map(|a| text(a.value.as_ref()))
}

/// Every attribute with this local name. `<p:sldId id="257" r:id="rId9"/>` has two `id`s, and
/// only the one in the relationships namespace is a relationship id: callers pick the one that
/// matches a relationship.
pub(crate) fn attrs_local(tag: &BytesStart<'_>, name: &str) -> Vec<String> {
    tag.attributes()
        .flatten()
        .filter(|a| a.key.local_name().as_ref() == name)
        .map(|a| text(a.value.as_ref()))
        .collect()
}

/// One `Relationship` of a `.rels` part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rel {
    pub id: String,
    /// The last path segment of the relationship type: `hyperlink`, `officeDocument`, `slide`...
    pub kind: String,
    pub target: String,
    pub external: bool,
}

/// The relationships in `rels_path`. A package without that part has none.
pub(crate) fn read_rels(
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    rels_path: &str,
) -> Result<Vec<Rel>, String> {
    if zip.by_name(rels_path).is_err() {
        return Ok(Vec::new());
    }
    let xml = entry(zip, rels_path)?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Empty(tag) | Event::Start(tag)
                if tag.local_name().as_ref() == "Relationship" =>
            {
                let kind = attr_local(&tag, "Type").unwrap_or_default();
                out.push(Rel {
                    id: attr_local(&tag, "Id").unwrap_or_default(),
                    kind: kind.rsplit('/').next().unwrap_or("").to_owned(),
                    target: attr_local(&tag, "Target").unwrap_or_default(),
                    external: attr_local(&tag, "TargetMode")
                        .is_some_and(|m| m.eq_ignore_ascii_case("External")),
                });
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// The `.rels` part that belongs to `part`: `word/document.xml` has `word/_rels/document.xml.rels`.
pub(crate) fn rels_path_of(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolves a relationship `target` against the part that holds it, inside the package. `None`
/// when the result would leave the package.
pub(crate) fn resolve(base_part: &str, target: &str) -> Option<String> {
    if target.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = if let Some(rooted) = target.strip_prefix('/') {
        rooted.split('/').collect()
    } else {
        let mut p: Vec<&str> = base_part.split('/').collect();
        p.pop(); // the file name
        p.extend(target.split('/'));
        p
    };
    let mut out: Vec<&str> = Vec::with_capacity(parts.len());
    for part in parts.drain(..) {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            other => out.push(other),
        }
    }
    (!out.is_empty()).then(|| out.join("/"))
}

/// The image relationships of `base_part`, by relationship id, resolved to package paths
/// (`ppt/slides/slide1.xml` with `../media/image1.png` gives `ppt/media/image1.png`). External
/// targets and targets that leave the package are left out.
pub(crate) fn image_refs(rels: &[Rel], base_part: &str) -> HashMap<String, String> {
    rels.iter()
        .filter(|r| r.kind == "image" && !r.external)
        .filter_map(|r| resolve(base_part, &r.target).map(|path| (r.id.clone(), path)))
        .collect()
}

/// The file name of a package path: `media/image1.png` for `word/media/image1.png`.
pub(crate) fn media_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Alt text as it goes inside Markdown image brackets: whitespace collapsed to single spaces and
/// square brackets escaped.
pub(crate) fn markdown_alt(alt: &str) -> String {
    alt.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

/// The package's main part: the `officeDocument` relationship of `_rels/.rels`, else `default`.
/// A relationship that points outside the package is an error.
pub(crate) fn main_part(
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    default: &str,
) -> Result<String, String> {
    let rels = read_rels(zip, "_rels/.rels")?;
    if let Some(rel) = rels
        .iter()
        .find(|r| r.kind == "officeDocument" && !r.external)
    {
        return resolve("", &rel.target)
            .ok_or_else(|| "officeDocument target escapes package".to_owned());
    }
    if zip.by_name(default).is_ok() {
        Ok(default.to_owned())
    } else {
        Err("missing officeDocument relationship".to_owned())
    }
}

/// One inline piece of a paragraph.
pub(crate) enum Seg {
    Text {
        text: String,
        bold: bool,
        italic: bool,
        /// `MARK_*` bits: strikeout, superscript, subscript, inline code.
        marks: u8,
        href: Option<String>,
    },
    Br,
    Raw(String),
}

pub(crate) const MARK_STRIKE: u8 = 1;
pub(crate) const MARK_SUP: u8 = 2;
pub(crate) const MARK_SUB: u8 = 4;
pub(crate) const MARK_CODE: u8 = 8;

fn emphasise(text: &str, bold: bool, italic: bool, marks: u8) -> String {
    let core = text.trim();
    if core.is_empty() || (!bold && !italic && marks == 0) {
        return text.to_owned();
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let tail = &text[text.trim_end().len()..];
    let mut core = core.to_owned();
    if marks & MARK_CODE != 0 {
        let ticks = if core.contains('`') { "``" } else { "`" };
        core = format!("{ticks}{core}{ticks}");
    }
    if marks & MARK_SUP != 0 {
        core = format!("^{}^", core.replace(' ', "\\ "));
    }
    if marks & MARK_SUB != 0 {
        core = format!("~{}~", core.replace(' ', "\\ "));
    }
    if marks & MARK_STRIKE != 0 {
        core = format!("~~{core}~~");
    }
    let mark = match (bold, italic) {
        (true, true) => "***",
        (true, false) => "**",
        (false, true) => "*",
        _ => "",
    };
    format!("{lead}{mark}{core}{mark}{tail}")
}

fn link_url(url: &str) -> String {
    url.replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
}

pub(crate) fn render(segs: &[Seg], line_break: &str, emphasis: bool) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < segs.len() {
        match &segs[index] {
            Seg::Br => {
                out.push_str(line_break);
                index += 1;
            }
            Seg::Raw(raw) => {
                out.push_str(raw);
                index += 1;
            }
            Seg::Text { href, .. } => {
                let mut inner = String::new();
                let mut end = index;
                while let Some(Seg::Text {
                    text,
                    bold,
                    italic,
                    marks,
                    href: h,
                }) = segs.get(end)
                {
                    if h != href {
                        break;
                    }
                    inner.push_str(&emphasise(text, *bold && emphasis, *italic && emphasis, if emphasis { *marks } else { 0 }));
                    end += 1;
                }
                match href {
                    Some(url) if !inner.trim().is_empty() => {
                        let lead = &inner[..inner.len() - inner.trim_start().len()];
                        let tail = &inner[inner.trim_end().len()..];
                        out.push_str(&format!(
                            "{lead}[{}]({}){tail}",
                            inner.trim(),
                            link_url(url)
                        ));
                    }
                    _ => out.push_str(&inner),
                }
                index = end;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_joins_against_the_holding_part_and_stays_inside() {
        assert_eq!(
            resolve("word/document.xml", "media/a.png").as_deref(),
            Some("word/media/a.png")
        );
        assert_eq!(
            resolve("word/document.xml", "../customXml/i.xml").as_deref(),
            Some("customXml/i.xml")
        );
        assert_eq!(
            resolve("ppt/presentation.xml", "/ppt/slides/slide1.xml").as_deref(),
            Some("ppt/slides/slide1.xml")
        );
        assert_eq!(
            resolve("", "word/document.xml").as_deref(),
            Some("word/document.xml")
        );
        assert_eq!(resolve("word/document.xml", "../../x"), None);
        assert_eq!(resolve("word/document.xml", ""), None);
    }

    #[test]
    fn rels_part_names() {
        assert_eq!(
            rels_path_of("word/document.xml"),
            "word/_rels/document.xml.rels"
        );
        assert_eq!(rels_path_of("workbook.xml"), "_rels/workbook.xml.rels");
    }
}
