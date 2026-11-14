//! Defects the Australian public-documents demo (Federal Register of Legislation DOCX/EPUB, ABS CPI
//! workbooks) turned up in the converters' output. Each test builds the smallest package that has
//! the shape and states what a reader of the Markdown should see.

use std::io::{Cursor, Write};
use superdocuments::document::convert_bytes;

fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, body) in parts {
        z.start_file(*name, opts).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

// ── DOCX: the Coat of Arms is a Word.Picture.8 object ───────────────────────────────────────

fn docx_with_object(prog_id: &str) -> Vec<u8> {
    let document = format!(
        r##"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
 xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"
 xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>
<w:p><w:r><w:object w:dxaOrig="2146" w:dyaOrig="1561"><v:shape id="_x0000_i1025" type="#_x0000_t75">
<v:imagedata r:id="rId8" o:title=""/></v:shape>
<o:OLEObject Type="Embed" ProgID="{prog_id}" ShapeID="_x0000_i1025" DrawAspect="Content" r:id="rId9"/>
</w:object></w:r></w:p>
<w:p><w:r><w:t>Census and Statistics Act 1905</w:t></w:r></w:p></w:body></w:document>"##
    );
    zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.wmf"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="embeddings/oleObject1.bin"/></Relationships>"#,
        ),
        ("word/document.xml", &document),
    ])
}

#[test]
fn embedded_picture_object_leaves_no_placeholder_line() {
    let md = convert_bytes("docx", &docx_with_object("Word.Picture.8")).unwrap();
    assert!(
        !md.contains("Embedded object"),
        "stray placeholder in {md:?}"
    );
    assert!(md.contains("Census and Statistics Act 1905"), "{md:?}");
    assert!(!md.trim_start().starts_with("Embedded"), "{md:?}");
}

#[test]
fn embedded_documents_keep_their_attribution() {
    for prog_id in ["Excel.Sheet.12", "Word.Document.8", "AcroExch.Document"] {
        let md = convert_bytes("docx", &docx_with_object(prog_id)).unwrap();
        assert!(
            md.contains(&format!("Embedded object: {prog_id}")),
            "{prog_id}: {md:?}"
        );
    }
}

// ── EPUB: an anchor with an id and no text is not a link ────────────────────────────────────

fn epub(chapter_body: &str) -> Vec<u8> {
    let chapter = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>{chapter_body}</body></html>"#
    );
    zip(&[
        ("mimetype", "application/epub+zip"),
        (
            "META-INF/container.xml",
            r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        ),
        (
            "OEBPS/content.opf",
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Ordinance</dc:title><dc:identifier id="id">x</dc:identifier></metadata><manifest><item id="c1" href="c1.html" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#,
        ),
        ("OEBPS/c1.html", &chapter),
    ])
}

#[test]
fn empty_anchors_in_an_epub_do_not_become_empty_links() {
    // the Register's footnote markup: <a id="_ftnref1"></a><a href="#_ftn1"><span>[*]</span></a>
    let md = convert_bytes(
        "epub",
        &epub(
            r##"<p>This Ordinance may be cited as the Census Ordinance 1980.<a id="_ftnref1"></a><a href="#_ftn1" style="text-decoration:none"><span>[*]</span></a></p>
<div id="_ftn1"><p><a href="#_ftnref1"><span>[*]</span></a> Notes to the Ordinance.</p></div><p><a name="x"></a><a href=""></a>After.</p>"##,
        ),
    )
    .unwrap();
    assert!(!md.contains("[]("), "empty link in {md:?}");
    assert!(
        md.contains("This Ordinance may be cited as the Census Ordinance 1980."),
        "{md:?}"
    );
    assert!(
        md.contains("Notes to the Ordinance."),
        "link text kept: {md:?}"
    );
    assert!(md.contains("After."), "{md:?}");
}

#[test]
fn links_with_text_survive_in_an_epub() {
    let md = convert_bytes(
        "epub",
        &epub(r#"<p>See <a href="https://www.legislation.gov.au/">the Register</a>.</p>"#),
    )
    .unwrap();
    assert!(
        md.contains("[the Register](https://www.legislation.gov.au/)"),
        "{md:?}"
    );
}

// ── XLSX: a cell shows what its number format says ──────────────────────────────────────────

fn workbook() -> Vec<u8> {
    let styles = r##"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<numFmts count="3"><numFmt numFmtId="164" formatCode="mmm\-yyyy"/><numFmt numFmtId="165" formatCode="0.0"/><numFmt numFmtId="166" formatCode="#,##0.00"/></numFmts>
<cellXfs count="6"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="165"/><xf numFmtId="166"/><xf numFmtId="9"/><xf numFmtId="14"/></cellXfs></styleSheet>"##;
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" s="1"><v>46266</v></c><c r="B1" s="2"><v>4.0999999999999996</v></c><c r="C1"><v>0.30000000000000004</v></c><c r="D1" s="3"><v>1234567.891</v></c><c r="E1" s="4"><v>0.256</v></c></row>
<row r="2"><c r="A2" s="5"><v>46266</v></c><c r="B2" s="2"><v>-0.04</v></c><c r="C2"><v>100</v></c><c r="D2"><v>4.5</v></c></row>
</sheetData></worksheet>"#;
    zip(&[
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        ),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", sheet),
    ])
}

#[test]
fn cells_show_the_text_their_number_format_gives() {
    let md = convert_bytes("xlsx", &workbook()).unwrap();
    // row 1: mmm-yyyy date, "0.0" of 4.0999999999999996, General of 0.30000000000000004, #,##0.00, 0%
    assert!(
        md.contains("| Sep-2026 | 4.1 | 0.3 | 1,234,567.89 | 26% |"),
        "{md}"
    );
    // row 2: the locale-dependent built-in date format 14 keeps the ISO date; "0.0" of -0.04 (Excel shows the minus of a negative number that rounds to zero); General 100; General 4.5
    assert!(md.contains("| 2026-09-01 | -0.0 | 100 | 4.5 |"), "{md}");
}
