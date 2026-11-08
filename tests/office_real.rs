//! Conversion of real files written by python-docx, python-pptx, openpyxl and LibreOffice. The
//! fixtures are small and regenerable; each test states what the file contains.

use std::fs;
use superdocuments::document::convert_bytes;

fn convert(name: &str) -> String {
    let ext = name.rsplit('.').next().expect("extension");
    let data = fs::read(format!("tests/fixtures/office/{name}")).expect("fixture");
    convert_bytes(ext, &data).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// `&`, `<` and `>` are written as entity references in XML; they must survive conversion.
#[test]
fn entity_references_are_kept_in_every_xml_format() {
    let docx = convert("basic.docx");
    assert!(docx.contains("Tom & Jerry said x < y > z"), "{docx}");
    let pptx = convert("basic.pptx");
    assert!(pptx.contains("Intro & Goals"), "{pptx}");
    assert!(pptx.contains("Point <A>"), "{pptx}");
    let xlsx = convert("basic.xlsx");
    assert!(xlsx.contains("A & B"), "{xlsx}");
    assert!(xlsx.contains("C<D"), "{xlsx}");
    let ods = convert("basic.ods");
    assert!(ods.contains("A & B"), "{ods}");
    assert!(ods.contains("C<D"), "{ods}");
    let odt = convert("basic.odt");
    assert!(odt.contains("Tom & Jerry said x < y > z"), "{odt}");
}
