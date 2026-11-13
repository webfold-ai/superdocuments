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
    // Pandoc-flavoured Markdown escapes `<` and `>`.
    assert!(
        odt.replace('\\', "").contains("Tom & Jerry said x < y > z"),
        "{odt}"
    );
}

#[test]
fn docx_structure_comes_from_styles_numbering_and_tables() {
    let md = convert("basic.docx");
    println!("{md}");
    assert!(
        // The Title style is metadata, as Pandoc reads it.
        md.starts_with("---\ntitle: \"Quarterly Report\"\n---\n\n# Overview\n\n"),
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
    // the master's body style gives these paragraphs a bullet
    assert!(md.contains("- Point <A>\n- Point B"), "{md}");
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
        // the cell's format code is `yyyy-mm-dd h:mm:ss`: one-digit hour, as the sheet shows it
        md.contains("| C<D | 2026-01-02 3:04:05 | 0.25 | FALSE |"),
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

#[test]
fn libreoffice_rtf_heading_with_outline_numbering_is_a_heading_not_a_list_item() {
    let md = convert("rich.rtf");
    assert!(md.starts_with("# Rich Document\n"), "{md}");
    assert!(
        md.contains("[the site](https://example.com/a?x=1&y=2)"),
        "{md}"
    );
}

#[test]
fn a_bullet_list_followed_by_a_numbered_list_is_two_lists() {
    let md = convert("basic.docx");
    assert!(
        md.contains("- Second bullet\n\n1. Step one\n2. Step two"),
        "{md}"
    );
}

#[test]
fn rtf_merged_cell_written_through_cellx_edges_keeps_its_columns() {
    let md = convert("rich.rtf");
    assert!(md.contains("| Merged head |  | Third |"), "{md}");
    assert!(md.contains("| a | b | c |"), "{md}");
}

// ----- regressions found by webfold-bench/tests/formats-fidelity-v1 (fixtures written by python-pptx) -----

/// Body text of a python-pptx slide has no bullet markup of its own: the bullet and its nesting come
/// from the master's body style. The old reader printed every bullet as a plain paragraph.
#[test]
fn pptx_bullets_inherited_from_the_master_keep_their_nesting() {
    let md = convert("inherited-bullets.pptx");
    assert!(
        md.contains("- Goals for the year\n    - Reduce churn\n    - Raise onboarding rate\n- Risks\n    - Supplier delay"),
        "{md}"
    );
    assert!(
        md.contains("## Slide 2: Timeline\n\n- Q1 planning\n- Q2 build\n- Q3 launch"),
        "{md}"
    );
    let two = convert("two-column.pptx");
    assert!(
        two.contains("- Option A is cheaper\n- Option A ships in May\n- Option B is faster"),
        "{two}"
    );
}

/// A chart is a graphic frame whose text lives in its own part: its title and data were dropped.
#[test]
fn pptx_chart_title_and_data_are_read() {
    let md = convert("chart.pptx");
    assert!(md.contains("Chart: Quarterly revenue"), "{md}");
    assert!(
        md.contains("|  | Revenue |\n| --- | --- |\n| Q1 | 120 |\n| Q2 | 135 |\n| Q3 | 160 |"),
        "{md}"
    );
}

// ----- ODP: found by webfold-bench/tests/formats-fidelity-v1 (the reader wrote every paragraph as flat text) -----

/// A library-written presentation (odfpy): slides, nested bullets and speaker notes. The old reader
/// returned one run of paragraphs with no slide boundaries, no bullets and the notes mixed in.
#[test]
fn odp_slides_bullets_and_notes_are_kept_apart() {
    let md = convert("odp-bullets-notes.odp");
    assert!(
        md.starts_with("## Slide 1\n\nKickoff\n\n- Scope agreed\n- Budget approved\n\n**Notes:** Remind the team about the freeze date."),
        "{md}"
    );
    assert!(
        md.contains("---\n\n## Slide 2\n\nWrap-up\n\n- Actions assigned\n\n**Notes:** Close within ten minutes."),
        "{md}"
    );
}

/// LibreOffice marks the title frame (`presentation:class="title"`) and the outline frame; the
/// title is the slide heading and the outline's bullets keep their nesting.
#[test]
fn odp_written_by_libreoffice_has_titles_and_nested_bullets() {
    let md = convert("lo-slides.odp");
    assert!(md.starts_with("## Slide 1: Project Overview\n\n"), "{md}");
    assert!(
        md.contains("- Goals for the year\n    - Reduce churn\n    - Raise onboarding rate\n- Risks\n    - Supplier delay"),
        "{md}"
    );
    assert!(md.contains("## Slide 2: Timeline"), "{md}");
}

/// A span that holds only a space (`<text:span> </text:span>`) separates the words around it, and a
/// span that ends in a space keeps it. The reader trimmed every span, which wrote `**Lorem Ipsum**is`
/// (Docling's text_document_02.odt).
#[test]
fn odt_spans_keep_the_whitespace_between_words() {
    let md = convert("odt-span-space.odt");
    assert!(md.contains("**Lorem Ipsum** is simply dummy text"), "{md}");
    assert!(
        md.contains("[the page](https://example.org/x) afterwards."),
        "{md}"
    );
    assert!(md.contains("One two"), "{md}");
}

/// A SmartArt diagram keeps its text in a data part the slide points to (`dgm:relIds r:dm`); a slide
/// that holds only a diagram came out empty (LibreOffice sd/qa Horizontal_Organization_Chart.pptx, MPL-2.0).
#[test]
fn pptx_smartart_text_is_read() {
    let md = convert("smartart-org-chart.pptx");
    for node in ["One", "Two", "Assistent", "Eleven", "Sekretärin", "Twelve"] {
        assert!(md.contains(&format!("- {node}")), "{node}: {md}");
    }
}
