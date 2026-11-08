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

#[test]
fn docx_structure_comes_from_styles_numbering_and_tables() {
    let md = convert("basic.docx");
    println!("{md}");
    assert!(
        md.starts_with("# Quarterly Report\n\n# Overview\n\n"),
        "{md}"
    );
    assert!(md.contains("**Bold part** and *italic part*"), "{md}");
    assert!(md.contains("## Details"), "{md}");
    assert!(md.contains("- First bullet\n- Second bullet"), "{md}");
    assert!(md.contains("1. Step one\n2. Step two"), "{md}");
    assert!(md.contains("| Name | Value \\| pipe |"), "{md}");
    assert!(md.contains("| Line one<br>Line two | 7 |"), "{md}");
}

#[test]
fn libreoffice_docx_keeps_links_footnotes_nested_lists_and_merged_cells() {
    let md = convert("rich.docx");
    println!("{md}");
    assert!(md.starts_with("# Rich Document"), "{md}");
    assert!(
        md.contains("[the site](https://example.com/a?x=1&y=2)"),
        "{md}"
    );
    // The id is whatever the producer wrote (LibreOffice starts real notes at 2).
    let reference = md
        .split("note[^")
        .nth(1)
        .and_then(|r| r.split(']').next())
        .expect("reference");
    assert!(
        md.contains(&format!("[^{reference}]: Footnote body & text.")),
        "{md}"
    );
    assert!(md.contains("**strong**"), "{md}");
    assert!(md.contains("- Outer\n    - Inner"), "{md}");
    assert!(md.contains("| Merged head |  | Third |"), "{md}");
    assert!(md.contains("After table."), "{md}");
}

#[test]
fn pptx_follows_presentation_order_and_reads_notes_and_tables() {
    let md = convert("basic.pptx");
    println!("{md}");
    let second = md.find("Second slide").expect("second slide");
    let intro = md.find("Intro & Goals").expect("intro");
    assert!(
        second < intro,
        "slide order must follow the presentation, not the file names: {md}"
    );
    assert!(md.starts_with("## Slide 1: Second slide"), "{md}");
    assert!(md.contains("## Slide 2: Intro & Goals"), "{md}");
    assert!(md.contains("Point <A>\n\nPoint B"), "{md}");
    assert!(md.contains("**Notes:** Speaker note here"), "{md}");
    assert!(md.contains("| H1 | H2 |"), "{md}");
    assert!(md.contains("| a | b |"), "{md}");
}

#[test]
fn xlsx_uses_workbook_names_tab_order_dates_and_booleans() {
    let md = convert("basic.xlsx");
    println!("{md}");
    let costs = md.find("## Costs").expect("Costs sheet");
    let revenue = md.find("## Revenue").expect("Revenue sheet");
    assert!(costs < revenue, "tab order, not file number: {md}");
    assert!(!md.contains("Sheet 1"), "{md}");
    assert!(
        md.contains("| A & B | 2026-11-07 | 1234.5 | TRUE |"),
        "{md}"
    );
    assert!(
        md.contains("| C<D | 2026-01-02 03:04:05 | 0.25 | FALSE |"),
        "{md}"
    );
    assert!(md.contains("TRUE") && md.contains("FALSE"), "{md}");
}

#[test]
fn libreoffice_ods_and_xlsx_agree_on_the_same_data() {
    let sheets = superdocuments::xlsx::read_workbook(
        &std::fs::read("tests/fixtures/office/basic.xlsx").unwrap(),
    )
    .unwrap();
    assert_eq!(
        sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Costs", "Revenue"]
    );
}

#[test]
fn libreoffice_written_xlsx_and_pptx_read_the_same_content() {
    let sheets = convert("lo.xlsx");
    assert!(
        sheets.contains("## Revenue") && sheets.contains("A & B"),
        "{sheets}"
    );
    let slides = convert("lo.pptx");
    println!("{sheets}\n----\n{slides}");
    assert!(
        slides.contains("Intro & Goals") && slides.contains("Point <A>"),
        "{slides}"
    );
}
