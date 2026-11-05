//! AcroForm repair, on this crate's own PDF parser (no `lopdf`).
//!
//! Some PDFs carry form widgets (`/Subtype /Widget` annotations on their pages) but have no
//! `/AcroForm` in the catalog, so a reader sees no form. The repair finds the orphaned widget
//! fields and appends an incremental update (ISO 32000-1 §7.5.6) that adds an `/AcroForm`
//! dictionary and a new revision of the catalog that points at it. The bytes of the original file
//! are not touched, so earlier revisions and any signatures over them stay intact.
//!
//! The idea comes from `run-llama/liteparse` `acroform_repair.rs` (Apache-2.0). The PDFium
//! verification step of that code is not carried: without a geometry backend the repair is not
//! verified against a render.
//!
//! Use [`repair_acroform`] on bytes, or [`repair_acroform_at`] to read a path and write the result
//! next to it as `<name>.repaired.pdf`. Both are safe on healthy PDFs.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use super::parser::{Dict, Document, Object};

/// Outcome of an AcroForm repair attempt: `Repaired(bytes)` carries the fixed PDF, `NotNeeded`
/// says the input is healthy, `Skipped(reason)` documents why the file was left alone.
pub enum AcroformRepair {
    /// Repair succeeded; `bytes` is the incrementally-updated PDF.
    Repaired(Vec<u8>),
    /// Input PDF already had a `/AcroForm`; nothing to do.
    NotNeeded,
    /// No repair was attempted, with a human-readable reason.
    Skipped(&'static str),
}

impl AcroformRepair {
    pub fn is_repaired(&self) -> bool {
        matches!(self, Self::Repaired(_))
    }
}

fn key(name: &str) -> String {
    name.to_string()
}

/// Best-effort repair for PDFs whose page annotations contain form widgets but whose catalog
/// omits `/AcroForm`. Returns the repaired bytes as `Repaired`; `NotNeeded` if the catalog already
/// has an `/AcroForm`; `Skipped` if the PDF is encrypted, has no widget fields, cannot be parsed,
/// or ends in a cross-reference stream (a classic table is not appended to such a file).
pub fn repair_acroform(bytes: &[u8]) -> AcroformRepair {
    // Cheap pre-flight: a PDF with no /Widget bytes anywhere has nothing to do.
    if !bytes.windows(b"/Widget".len()).any(|w| w == b"/Widget") {
        return AcroformRepair::Skipped("no /Widget annotations present");
    }
    let doc = match Document::parse(bytes) {
        Ok(d) => d,
        Err(_) => return AcroformRepair::Skipped("PDF failed to parse"),
    };
    if doc.trailer.get(&key("Encrypt")).is_some() {
        return AcroformRepair::Skipped("encrypted PDF");
    }
    let (root_num, root_gen) = match doc.trailer.get(&key("Root")) {
        Some(Object::Ref(n, g)) => (*n, *g),
        _ => return AcroformRepair::Skipped("no /Root in the trailer"),
    };
    let catalog = match doc.ref_object(root_num, root_gen) {
        Some(Object::Dict(c)) => c,
        _ => return AcroformRepair::Skipped("catalog not a dict"),
    };
    if catalog.get(&key("AcroForm")).is_some() {
        return AcroformRepair::NotNeeded;
    }
    let fields = match collect_orphaned_fields(&doc) {
        Some(f) => f,
        None => return AcroformRepair::Skipped("no orphaned widget fields"),
    };
    match append_update(bytes, &doc, root_num, root_gen, catalog, &fields) {
        Ok(repaired) => AcroformRepair::Repaired(repaired),
        Err(reason) => AcroformRepair::Skipped(reason),
    }
}

/// Convenience: read a path, run [`repair_acroform`], write the result next to the original as
/// `<name>.repaired.pdf`. Returns the path of the written file on success, or `Ok(None)` if the
/// input is healthy or was skipped.
pub fn repair_acroform_at(path: impl AsRef<Path>) -> io::Result<Option<std::path::PathBuf>> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    match repair_acroform(&bytes) {
        AcroformRepair::Repaired(bytes) => {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("document");
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let out_path = parent.join(format!("{stem}.repaired.pdf"));
            fs::write(&out_path, bytes)?;
            Ok(Some(out_path))
        }
        AcroformRepair::NotNeeded => Ok(None),
        AcroformRepair::Skipped(reason) => {
            eprintln!("superdocuments::pdf::acroform: skipped ({reason})");
            Ok(None)
        }
    }
}

/// Walk every page, find `/Subtype /Widget` annotations, and return the form field each belongs
/// to, deduplicated, in page order. A widget's field is its `/Parent`, or the widget itself when it
/// has no parent but carries `/FT`. `None` when the pages hold no such field.
fn collect_orphaned_fields(doc: &Document) -> Option<Vec<(u32, u16)>> {
    let mut seen: HashSet<(u32, u16)> = HashSet::new();
    let mut fields: Vec<(u32, u16)> = Vec::new();
    for page_num in doc.page_numbers() {
        let Some(Object::Dict(page)) = doc.object(page_num) else {
            continue;
        };
        let Some(annots) = page.get(&key("Annots")).and_then(|a| doc.resolve(a)) else {
            continue;
        };
        let Object::Array(annots) = annots else {
            continue;
        };
        for entry in annots {
            let Object::Ref(n, g) = entry else {
                continue;
            };
            let Some(Object::Dict(widget)) = doc.ref_object(*n, *g) else {
                continue;
            };
            let is_widget =
                matches!(widget.get(&key("Subtype")), Some(Object::Name(t)) if t == "Widget");
            if !is_widget {
                continue;
            }
            let field = match widget.get(&key("Parent")) {
                Some(Object::Ref(pn, pg)) => Some((*pn, *pg)),
                _ if widget.get(&key("FT")).is_some() => Some((*n, *g)),
                _ => None,
            };
            if let Some(field) = field {
                if seen.insert(field) {
                    fields.push(field);
                }
            }
        }
    }
    if fields.is_empty() {
        None
    } else {
        Some(fields)
    }
}

/// The byte offset that the last `startxref` of `bytes` names.
fn last_startxref(bytes: &[u8]) -> Option<usize> {
    let marker = b"startxref";
    let at = bytes.windows(marker.len()).rposition(|w| w == marker)? + marker.len();
    let digits: Vec<u8> = bytes[at..]
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

/// A non-negative integer entry of the trailer.
fn trailer_uint(doc: &Document, name: &str) -> Option<u64> {
    match doc.trailer.get(&key(name)) {
        Some(Object::Int(i)) if *i >= 0 => Some(*i as u64),
        _ => None,
    }
}

/// Serialise `o` in PDF syntax. Streams are not written (the objects rewritten here have none);
/// a non-finite real has no PDF form. Both give `None`.
fn write_object(out: &mut Vec<u8>, o: &Object) -> Option<()> {
    match o {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(true) => out.extend_from_slice(b"true"),
        Object::Bool(false) => out.extend_from_slice(b"false"),
        Object::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Object::Real(f) => {
            if !f.is_finite() {
                return None;
            }
            out.extend_from_slice(format!("{f}").as_bytes());
        }
        Object::Str(s) => {
            // Hex form: no escaping of parentheses, backslashes or line ends.
            out.push(b'<');
            for b in s {
                out.extend_from_slice(format!("{b:02X}").as_bytes());
            }
            out.push(b'>');
        }
        Object::Name(n) => write_name(out, n),
        Object::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_object(out, item)?;
            }
            out.push(b']');
        }
        Object::Dict(d) => write_dict(out, d)?,
        Object::Ref(n, g) => out.extend_from_slice(format!("{n} {g} R").as_bytes()),
        Object::Stream(..) => return None,
    }
    Some(())
}

/// A name: `/` then each byte, with delimiters, `#`, white space and non-printables as `#XX`.
fn write_name(out: &mut Vec<u8>, name: &str) {
    out.push(b'/');
    for &b in name.as_bytes() {
        let regular = (0x21..=0x7e).contains(&b) && !b"()<>[]{}/%#".contains(&b);
        if regular {
            out.push(b);
        } else {
            out.extend_from_slice(format!("#{b:02X}").as_bytes());
        }
    }
}

fn write_dict(out: &mut Vec<u8>, d: &Dict) -> Option<()> {
    out.extend_from_slice(b"<<");
    for (k, v) in d.iter() {
        out.push(b' ');
        write_name(out, k);
        out.push(b' ');
        write_object(out, v)?;
    }
    out.extend_from_slice(b" >>");
    Some(())
}

/// Append the incremental update: a new `/AcroForm` object, a new revision of the catalog that
/// names it, a cross-reference table with those two entries, and a trailer that chains to the
/// previous section with `/Prev`.
fn append_update(
    original: &[u8],
    doc: &Document,
    root_num: u32,
    root_gen: u16,
    catalog: &Dict,
    fields: &[(u32, u16)],
) -> Result<Vec<u8>, &'static str> {
    let prev = last_startxref(original).ok_or("no startxref")?;
    // A classic table is appended only to a file whose newest section is a classic table.
    let at_prev = original.get(prev..).ok_or("startxref outside the file")?;
    let at_prev = at_prev
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(4)
        .collect::<Vec<u8>>();
    if at_prev != b"xref" {
        return Err("newest cross-reference section is a stream");
    }
    let highest = doc.objects.iter().map(|(k, _)| *k).max().unwrap_or(0) as u64;
    let size = trailer_uint(doc, "Size").unwrap_or(0).max(highest + 1);
    let acro_num = u32::try_from(size).map_err(|_| "object numbers exhausted")?;
    let new_size = size + 1;

    let mut out = original.to_vec();
    if !matches!(out.last(), Some(b'\n' | b'\r')) {
        out.push(b'\n');
    }

    let acro_off = out.len();
    out.extend_from_slice(format!("{acro_num} 0 obj\n<< /Fields [").as_bytes());
    for (i, (n, g)) in fields.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        out.extend_from_slice(format!("{n} {g} R").as_bytes());
    }
    out.extend_from_slice(b"] /NeedAppearances true /DA (/Helv 0 Tf 0 g) >>\nendobj\n");

    let root_off = out.len();
    out.extend_from_slice(format!("{root_num} {root_gen} obj\n").as_bytes());
    let mut new_catalog = catalog.clone();
    new_catalog.put(key("AcroForm"), Object::Ref(acro_num, 0));
    write_dict(&mut out, &new_catalog).ok_or("catalog holds a stream or a non-finite real")?;
    out.extend_from_slice(b"\nendobj\n");

    let xref_off = out.len();
    out.extend_from_slice(b"xref\n");
    let mut entries = [(root_num, root_gen, root_off), (acro_num, 0u16, acro_off)];
    entries.sort_by_key(|e| e.0);
    for (num, gen, off) in entries {
        out.extend_from_slice(format!("{num} 1\n{off:010} {gen:05} n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {new_size} /Root {root_num} {root_gen} R /Prev {prev}")
            .as_bytes(),
    );
    for name in ["Info", "ID"] {
        if let Some(v) = doc.trailer.get(&key(name)) {
            out.push(b' ');
            write_name(&mut out, name);
            out.push(b' ');
            write_object(&mut out, v).ok_or("trailer entry could not be written")?;
        }
    }
    out.extend_from_slice(format!(" >>\nstartxref\n{xref_off}\n%%EOF\n").as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A classic-table PDF from `(object number, body)` pairs, with `trailer_extra` after `/Root`.
    fn build(objects: &[(u32, String)], trailer_extra: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (num, body) in objects {
            offsets.push((*num, out.len()));
            out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let size = objects.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for n in 1..size {
            let off = offsets.iter().find(|(m, _)| *m == n).map_or(0, |(_, o)| *o);
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {size} /Root 1 0 R{trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        out
    }

    /// Catalog 1, pages 2, page 3, widget 4; the catalog names an `/AcroForm` (object 5) or not.
    fn form_pdf(include_acroform: bool) -> Vec<u8> {
        let catalog = if include_acroform {
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>"
        } else {
            "<< /Type /Catalog /Pages 2 0 R >>"
        };
        let mut objects = vec![
            (1, catalog.to_string()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>"
                    .to_string(),
            ),
            (
                4,
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T (customer_name) /V (Ada Lovelace) \
                 /Rect [10 10 200 35] /F 4 >>"
                    .to_string(),
            ),
        ];
        if include_acroform {
            objects.push((5, "<< /Fields [4 0 R] /NeedAppearances true >>".to_string()));
        }
        build(&objects, "")
    }

    #[test]
    fn repair_orphaned_widgets_emits_acroform_dict() {
        let bytes = form_pdf(false);
        let AcroformRepair::Repaired(repaired) = repair_acroform(&bytes) else {
            panic!("a PDF with an orphaned widget is repaired");
        };
        // The original bytes are a prefix of the result: an incremental update, nothing rewritten.
        assert!(repaired.starts_with(&bytes));
        // The repaired file parses, and its catalog now names an /AcroForm with the widget.
        let doc = Document::parse(&repaired).expect("repaired PDF parses");
        let Some(Object::Dict(catalog)) = doc.object(doc.root()) else {
            panic!("catalog is a dict");
        };
        let Some(Object::Ref(acro, 0)) = catalog.get(&"AcroForm".to_string()) else {
            panic!("catalog names an /AcroForm");
        };
        let Some(Object::Dict(acroform)) = doc.object(*acro) else {
            panic!("/AcroForm is a dict");
        };
        assert_eq!(
            acroform.get(&"Fields".to_string()),
            Some(&Object::Array(vec![Object::Ref(4, 0)]))
        );
        assert_eq!(
            acroform.get(&"NeedAppearances".to_string()),
            Some(&Object::Bool(true))
        );
        // The rest of the catalog survives the new revision.
        assert_eq!(catalog.get(&"Pages".to_string()), Some(&Object::Ref(2, 0)));
    }

    #[test]
    fn a_repaired_file_is_not_repaired_again() {
        let bytes = form_pdf(false);
        let AcroformRepair::Repaired(repaired) = repair_acroform(&bytes) else {
            panic!("first repair");
        };
        assert!(matches!(
            repair_acroform(&repaired),
            AcroformRepair::NotNeeded
        ));
    }

    #[test]
    fn healthy_acroform_is_not_needed() {
        assert!(matches!(
            repair_acroform(&form_pdf(true)),
            AcroformRepair::NotNeeded
        ));
    }

    #[test]
    fn no_widgets_at_all_is_skipped() {
        let bytes = b"%PDF-1.4\nno widgets here\n%%EOF\n".to_vec();
        assert!(matches!(
            repair_acroform(&bytes),
            AcroformRepair::Skipped(_)
        ));
    }

    #[test]
    fn encrypted_pdf_is_skipped() {
        let mut bytes = form_pdf(false);
        let at = bytes
            .windows(b"/Root 1 0 R".len())
            .rposition(|w| w == b"/Root 1 0 R")
            .unwrap();
        bytes.splice(at..at, b"/Encrypt << /Filter /Standard >> ".iter().copied());
        // The splice moves the trailer but not the offsets before it; startxref is unchanged.
        assert!(matches!(
            repair_acroform(&bytes),
            AcroformRepair::Skipped("encrypted PDF")
        ));
    }

    #[test]
    fn widget_without_field_data_is_skipped() {
        // A widget with neither /Parent nor /FT names no field.
        let objects = vec![
            (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".to_string(),
            ),
            (4, "<< /Type /Annot /Subtype /Widget >>".to_string()),
        ];
        assert!(matches!(
            repair_acroform(&build(&objects, "")),
            AcroformRepair::Skipped("no orphaned widget fields")
        ));
    }

    #[test]
    fn widgets_sharing_a_parent_yield_one_field() {
        let objects = vec![
            (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>".to_string(),
            ),
            (4, "<< /Subtype /Widget /Parent 6 0 R >>".to_string()),
            (5, "<< /Subtype /Widget /Parent 6 0 R >>".to_string()),
            (6, "<< /FT /Btn /T (group) >>".to_string()),
        ];
        let AcroformRepair::Repaired(repaired) = repair_acroform(&build(&objects, "")) else {
            panic!("repaired");
        };
        let doc = Document::parse(&repaired).unwrap();
        let Some(Object::Dict(catalog)) = doc.object(doc.root()) else {
            panic!("catalog");
        };
        let Some(Object::Ref(acro, _)) = catalog.get(&"AcroForm".to_string()) else {
            panic!("AcroForm");
        };
        let Some(Object::Dict(acroform)) = doc.object(*acro) else {
            panic!("AcroForm dict");
        };
        assert_eq!(
            acroform.get(&"Fields".to_string()),
            Some(&Object::Array(vec![Object::Ref(6, 0)]))
        );
    }

    #[test]
    fn names_are_written_with_escapes() {
        let mut out = Vec::new();
        write_name(&mut out, "A B/C#D");
        assert_eq!(out, b"/A#20B#2FC#23D");
    }

    #[test]
    fn widget_byte_window_finds_marker() {
        let bytes = b"prefix /Widget suffix";
        assert!(bytes.windows(7).any(|w| w == b"/Widget"));
    }

    #[test]
    fn widget_byte_window_rejects_clean_text_without_marker() {
        let bytes = b"plain text without widget annotations at all";
        assert!(!bytes.windows(7).any(|w| w == b"/Widget"));
    }

    #[test]
    fn widget_byte_window_rejects_single_byte_off_match() {
        // "Widget" without the leading slash is a near-miss the scan must reject.
        let bytes = b"plain Widget annotation";
        assert!(!bytes.windows(7).any(|w| w == b"/Widget"));
    }
}
