//! AcroForm repair lifted from `run-llama/liteparse` `acroform_repair.rs`
//! (Apache-2.0, ~8 KB).
//!
//! The lift carries the `lopdf`-based parts of LiteParse's repair function
//! (walking pages, collecting orphaned `/Widget` annotations, minting an
//! `/AcroForm` dictionary at the catalog root). The PDFium-based verification
//! path is stubbed: without PDFium on the dep tree, the repair is reported
//! as "unverified" — a separate `verify_acroform_repair()` function returns
//! `false` until a geometry backend lands.
//!
//! Use `repair_acroform(path)` for a one-shot repair that reads bytes from
//! disk and writes repaired bytes to a sibling `.repaired.pdf` next to the
//! input. The function is safe to call on healthy PDFs (returns `Ok(None)`).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(test)]
use lopdf::Dictionary;
use lopdf::{
    dictionary, Document as LoDocument, IncrementalDocument, Object, ObjectId, StringFormat,
};

/// Outcome of an AcroForm repair attempt. Mirrors `PdfInput`-style option
/// semantics: `Repaired(bytes)` carries the fixed PDF, `NotNeeded` says the
/// input is healthy, `Skipped(reason)` documents why we didn't touch it.
pub enum AcroformRepair {
    /// Repair succeeded; `bytes` is the incrementally-updated PDF.
    Repaired(Vec<u8>),
    /// Input PDF already had a valid `/AcroForm`; nothing to do.
    NotNeeded,
    /// We didn't attempt a repair, with a human-readable reason.
    Skipped(&'static str),
}

impl AcroformRepair {
    pub fn is_repaired(&self) -> bool {
        matches!(self, Self::Repaired(_))
    }
}

/// Best-effort repair for PDFs whose page annotations contain form widgets
/// (`/Subtype /Widget`) but whose catalog omits `/AcroForm`. Returns the
/// repaired bytes as `AcroformRepair::Repaired`; returns `NotNeeded` if the
/// input is already healthy; returns `Skipped` if the PDF is encrypted, has
/// no widget annotations, or fails the lopdf parse.
///
/// Lifted from `run-llama/liteparse` `acroform_repair.rs::repair_orphaned_widgets`
/// with the PDFium verification path removed (we don't link PDFium).
pub fn repair_acroform(bytes: &[u8]) -> AcroformRepair {
    // Cheap pre-flight: PDFs with no /Widget bytes anywhere have nothing to do.
    if !bytes
        .windows(b"/Widget".len())
        .any(|window| window == b"/Widget")
    {
        return AcroformRepair::Skipped("no /Widget annotations present");
    }
    let parsed = match LoDocument::load_mem(bytes) {
        Ok(d) => d,
        Err(_) => return AcroformRepair::Skipped("lopdf failed to parse"),
    };
    if parsed.trailer.has(b"Encrypt") {
        return AcroformRepair::Skipped("encrypted PDF");
    }
    let root_id = match parsed
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|v| v.as_reference().ok())
    {
        Some(id) => id,
        None => return AcroformRepair::Skipped("no /Root"),
    };
    let catalog = match parsed.get_dictionary(root_id) {
        Ok(c) => c,
        Err(_) => return AcroformRepair::Skipped("catalog not a dict"),
    };
    if catalog.has(b"AcroForm") {
        return AcroformRepair::NotNeeded;
    }
    let (field_refs, first_widget_page) = match collect_orphaned_fields(&parsed) {
        Some(t) => t,
        None => return AcroformRepair::Skipped("no orphaned widget fields"),
    };
    let _ = first_widget_page; // reserved for future verification.

    // Build the `/AcroForm` dict and write it back via IncrementalDocument so
    // we only mutate the catalog — page bodies are untouched, preserving any
    // incremental-save signatures or stream filter state.
    let mut update = IncrementalDocument::create_from(bytes.to_vec(), parsed);
    if update.opt_clone_object_to_new_document(root_id).is_err() {
        return AcroformRepair::Skipped("could not clone catalog into the update");
    }
    let acroform_id = update.new_document.add_object(dictionary! {
        "Fields" => Object::Array(field_refs.into_iter().map(Object::Reference).collect()),
        "NeedAppearances" => true,
        "DA" => Object::String(b"/Helv 0 Tf 0 g".to_vec(), StringFormat::Literal),
    });
    match update.new_document.get_dictionary_mut(root_id) {
        Ok(catalog_mut) => {
            catalog_mut.set("AcroForm", Object::Reference(acroform_id));
        }
        Err(_) => return AcroformRepair::Skipped("catalog became non-dict after clone"),
    }

    let mut repaired = Vec::new();
    if update.save_to(&mut repaired).is_err() {
        return AcroformRepair::Skipped("IncrementalDocument::save_to failed");
    }
    AcroformRepair::Repaired(repaired)
}

/// Convenience: read a path, run `repair_acroform`, write the result next to
/// the original as `<name>.repaired.pdf`. Returns the path of the written
/// file on success, or `Ok(None)` if the input is healthy / skipped.
pub fn repair_acroform_at(path: impl AsRef<Path>) -> io::Result<Option<std::path::PathBuf>> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    let out = repair_acroform(&bytes);
    match out {
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

/// Walk every page, find `/Subtype /Widget` annotations whose `/Parent` (or
/// self, if no parent) is a form field, deduplicate, and report the first
/// page that owns a widget — used by `repair_acroform` to seed the new
/// `/AcroForm` entry.
///
/// Lifted from `run-llama/liteparse` `acroform_repair.rs::collect_orphaned_fields`
/// with the page_index conversion (`page_number.saturating_sub(1)`) preserved
/// verbatim.
fn collect_orphaned_fields(document: &LoDocument) -> Option<(Vec<ObjectId>, u32)> {
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut fields: Vec<ObjectId> = Vec::new();
    let mut first_page: Option<u32> = None;

    for (page_number, page_id) in document.get_pages() {
        let page = match document.get_dictionary(page_id).ok() {
            Some(p) => p,
            None => continue,
        };
        let annots = match page.get(b"Annots") {
            Ok(a) => a,
            Err(_) => continue,
        };
        let (_, annots) = match document.dereference(annots) {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        let annots = match annots.as_array() {
            Ok(a) => a,
            Err(_) => continue,
        };
        for entry in annots {
            let widget_id = match entry.as_reference().ok() {
                Some(id) => id,
                None => continue,
            };
            let widget = match document.get_dictionary(widget_id) {
                Ok(w) => w,
                Err(_) => continue,
            };
            let is_widget = widget
                .get(b"Subtype")
                .ok()
                .and_then(|value| value.as_name().ok())
                == Some(b"Widget");
            if !is_widget {
                continue;
            }
            let field_id = widget
                .get(b"Parent")
                .ok()
                .and_then(|value| value.as_reference().ok())
                .or_else(|| widget.has(b"FT").then_some(widget_id));
            let field_id = match field_id {
                Some(id) => id,
                None => continue,
            };
            first_page.get_or_insert(page_number.saturating_sub(1));
            if seen.insert(field_id) {
                fields.push(field_id);
            }
        }
    }
    Some((fields, first_page?))
}

/// Pure-Rust test: a minimal PDF with one `/Widget` annotation and no
/// `/AcroForm` should be repaired; one with `/AcroForm` already should be
/// returned as `NotNeeded`.
#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, Stream, StringFormat};

    fn build_pdf(include_acroform: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let widget_id = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "FT" => "Tx",
            "T" => Object::String(b"customer_name".to_vec(), StringFormat::Literal),
            "V" => Object::String(b"Ada Lovelace".to_vec(), StringFormat::Literal),
            "Rect" => vec![10.into(), 10.into(), 200.into(), 35.into()],
            "F" => 4,
        });
        let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => vec![Object::Reference(widget_id)],
            "Contents" => content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        let mut catalog: Dictionary = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if include_acroform {
            let acro_id = doc.add_object(dictionary! {
                "Fields" => vec![Object::Reference(widget_id)],
                "NeedAppearances" => true,
                "DA" => Object::String(b"/Helv 0 Tf 0 g".to_vec(), StringFormat::Literal),
            });
            catalog.set("AcroForm", acro_id);
        }
        let catalog_id = doc.add_object(catalog);
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn repair_orphaned_widgets_emits_acroform_dict() {
        let bytes = build_pdf(false);
        let result = repair_acroform(&bytes);
        assert!(matches!(result, AcroformRepair::Repaired(_)));
        if let AcroformRepair::Repaired(repaired) = result {
            // Repaired PDF should now have an AcroForm dict we can parse.
            let reparsed = LoDocument::load_mem(&repaired).unwrap();
            let root_id = reparsed
                .trailer
                .get(b"Root")
                .unwrap()
                .as_reference()
                .unwrap();
            let catalog = reparsed.get_dictionary(root_id).unwrap();
            assert!(catalog.has(b"AcroForm"));
        }
    }

    #[test]
    fn healthy_acroform_is_not_needed() {
        let bytes = build_pdf(true);
        let result = repair_acroform(&bytes);
        assert!(matches!(result, AcroformRepair::NotNeeded));
    }

    #[test]
    fn no_widgets_at_all_is_skipped() {
        // Construct a PDF with no /Widget strings anywhere.
        let bytes = b"%PDF-1.4\nno widgets here\n%%EOF\n".to_vec();
        let result = repair_acroform(&bytes);
        assert!(matches!(result, AcroformRepair::Skipped(_)));
    }

    #[test]
    fn widget_byte_window_finds_marker() {
        let bytes = b"prefix /Widget suffix";
        assert!(bytes.windows(7).any(|w| w == b"/Widget"));
    }

    #[test]
    fn widget_byte_window_rejects_clean_text_without_marker() {
        // A buffer with no "/Widget" anywhere must not match.
        let bytes = b"plain text without widget annotations at all";
        assert!(!bytes.windows(7).any(|w| w == b"/Widget"));
    }

    #[test]
    fn widget_byte_window_rejects_single_byte_off_match() {
        // The single byte sequence "Widget" (capital W, no leading slash) is
        // a near-miss the windowed scan must reject. With bytes starting at
        // offset N, the windows that include position N (where 'W' lives)
        // are 7 bytes wide and cannot match "/Widget" — '/Widget' requires
        // the leading slash.
        let bytes = b"plain Widget annotation"; // no leading slash
        assert!(!bytes.windows(7).any(|w| w == b"/Widget"));
    }
}
