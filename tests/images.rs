//! Image extraction from DOCX, PPTX, XLSX, ODT and EPUB packages, and the image references the
//! DOCX and PPTX Markdown writes. Each package is built here with the `zip` crate.

use std::io::{Cursor, Write};

use superdocuments::document::{convert_bytes, extract_images};

/// A 1x1 transparent PNG, 67 bytes.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// Builds a ZIP package from `(name, bytes)` entries, in order.
fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .expect("start entry");
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish");
    }
    buf
}

const CONTENT_TYPES_DOCX_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const DOCX_DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image2.png"/>
</Relationships>"#;

/// Two pictures in body order: `rId6` is written first in the document, `rId5` second, so the
/// reading order is by position in the document, not by file name. `rId5` has a `descr` with
/// square brackets, which must be escaped in Markdown.
const DOCX_DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
 xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
 xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
 xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
<w:body>
<w:p><w:r><w:t>Before</w:t></w:r></w:p>
<w:p><w:r><w:drawing><wp:inline><wp:docPr id="1" name="Picture 2" descr="Second [picture]"/>
<a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rId6"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>
</wp:inline></w:drawing></w:r></w:p>
<w:p><w:r><w:t>Middle</w:t></w:r></w:p>
<w:p><w:r><w:drawing><wp:inline><wp:docPr id="2" name="Picture 1" descr="First photo"/>
<a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rId5"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>
</wp:inline></w:drawing></w:r></w:p>
</w:body>
</w:document>"#;

fn docx(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", CONTENT_TYPES_DOCX_RELS.as_bytes()),
        ("word/document.xml", DOCX_DOCUMENT.as_bytes()),
        (
            "word/_rels/document.xml.rels",
            DOCX_DOCUMENT_RELS.as_bytes(),
        ),
        ("word/media/image1.png", PNG_1X1),
        ("word/media/image2.png", PNG_1X1),
    ];
    entries.extend_from_slice(extra);
    package(&entries)
}

#[test]
fn docx_images_follow_document_order_with_alt_and_reference() {
    let data = docx(&[("word/media/unused.png", PNG_1X1)]);
    let images = extract_images(&data, "docx").expect("images");
    let names: Vec<&str> = images.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "word/media/image2.png",
            "word/media/image1.png",
            "word/media/unused.png"
        ]
    );
    let first = &images[0];
    assert_eq!(first.mime, "image/png");
    assert_eq!((first.width, first.height), (Some(1), Some(1)));
    assert_eq!(first.bytes, PNG_1X1);
    assert_eq!(first.referenced_from.as_deref(), Some("word/document.xml"));
    assert_eq!(first.alt.as_deref(), Some("Second [picture]"));
    assert_eq!(images[1].alt.as_deref(), Some("First photo"));
    assert_eq!(images[2].referenced_from, None);
    assert_eq!(images[2].alt, None);
}

#[test]
fn docx_markdown_places_each_picture_where_its_drawing_is() {
    let md = convert_bytes("docx", &docx(&[])).expect("markdown");
    let before = md.find("Before").expect(&md);
    let second = md
        .find("![Second \\[picture\\]](media/image2.png)")
        .expect(&md);
    let middle = md.find("Middle").expect(&md);
    let first = md.find("![First photo](media/image1.png)").expect(&md);
    assert!(before < second && second < middle && middle < first, "{md}");
}

#[test]
fn docx_drawing_without_a_picture_keeps_its_description_as_text() {
    let document = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">
<w:body><w:p><w:r><w:drawing><wp:inline><wp:docPr id="3" name="Chart" descr="Sales by quarter"/></wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#;
    let data = package(&[
        ("_rels/.rels", CONTENT_TYPES_DOCX_RELS.as_bytes()),
        ("word/document.xml", document.as_bytes()),
    ]);
    let md = convert_bytes("docx", &data).expect("markdown");
    assert!(md.contains("Sales by quarter"), "{md}");
    assert!(!md.contains("!["), "{md}");
    assert!(extract_images(&data, "docx").expect("images").is_empty());
}

#[test]
fn entry_names_with_parent_segments_are_refused() {
    let data = docx(&[("../evil.png", PNG_1X1)]);
    assert!(extract_images(&data, "docx").is_err());
    assert!(
        convert_bytes("docx", &data).is_ok(),
        "text conversion is unchanged"
    );
}

#[test]
fn files_that_are_not_images_are_left_out() {
    let data = docx(&[("word/media/notes.bin", b"plain bytes, not a picture")]);
    let names: Vec<String> = extract_images(&data, "docx")
        .expect("images")
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert!(!names.iter().any(|n| n.ends_with("notes.bin")), "{names:?}");
}

const PRESENTATION: &str = r#"<?xml version="1.0"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<p:sldIdLst><p:sldId id="256" r:id="rId3"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide3.xml"/>
</Relationships>"#;

const SLIDE3: &str = r#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<p:cSld><p:spTree>
<p:sp><p:txBody><a:p><a:r><a:t>Slide three text</a:t></a:r></a:p></p:txBody></p:sp>
<p:pic><p:nvPicPr><p:cNvPr id="4" name="Logo" descr="Company logo"/></p:nvPicPr>
<p:blipFill><a:blip r:embed="rId2"/></p:blipFill></p:pic>
</p:spTree></p:cSld></p:sld>"#;

const SLIDE3_RELS: &str = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image7.png"/>
</Relationships>"#;

const SLIDE1: &str = r#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Slide one text</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;

fn pptx() -> Vec<u8> {
    package(&[
        (
            "_rels/.rels",
            CONTENT_TYPES_DOCX_RELS
                .replace("word/document.xml", "ppt/presentation.xml")
                .as_bytes(),
        ),
        ("ppt/presentation.xml", PRESENTATION.as_bytes()),
        (
            "ppt/_rels/presentation.xml.rels",
            PRESENTATION_RELS.as_bytes(),
        ),
        ("ppt/slides/slide1.xml", SLIDE1.as_bytes()),
        ("ppt/slides/slide3.xml", SLIDE3.as_bytes()),
        ("ppt/slides/_rels/slide3.xml.rels", SLIDE3_RELS.as_bytes()),
        ("ppt/media/image7.png", PNG_1X1),
    ])
}

#[test]
fn pptx_image_is_recorded_with_its_slide_and_alt_text() {
    let images = extract_images(&pptx(), "pptx").expect("images");
    assert_eq!(images.len(), 1);
    let logo = &images[0];
    assert_eq!(logo.name, "ppt/media/image7.png");
    assert_eq!(logo.mime, "image/png");
    assert_eq!((logo.width, logo.height), (Some(1), Some(1)));
    assert_eq!(
        logo.referenced_from.as_deref(),
        Some("ppt/slides/slide3.xml")
    );
    assert_eq!(logo.alt.as_deref(), Some("Company logo"));
}

#[test]
fn pptx_markdown_references_the_picture_in_its_slide() {
    let md = convert_bytes("pptx", &pptx()).expect("markdown");
    let text = md.find("Slide three text").expect(&md);
    let picture = md.find("![Company logo](media/image7.png)").expect(&md);
    // `sldIdLst` lists slide 3 first, so its text and picture come before slide 1's text.
    assert!(text < picture, "{md}");
    assert!(picture < md.find("Slide one text").expect(&md), "{md}");
}

const ODT_CONTENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0">
<office:body><office:text>
<draw:frame draw:name="img1"><svg:desc>A diagram &amp; key</svg:desc><draw:image xlink:href="Pictures/10000201.png"/></draw:frame>
</office:text></office:body></office:document-content>"#;

#[test]
fn odt_picture_with_its_frame_description() {
    let data = package(&[
        ("content.xml", ODT_CONTENT.as_bytes()),
        ("Pictures/10000201.png", PNG_1X1),
        ("Pictures/10000202.png", PNG_1X1),
    ]);
    let images = extract_images(&data, "odt").expect("images");
    assert_eq!(images[0].name, "Pictures/10000201.png");
    assert_eq!(images[0].referenced_from.as_deref(), Some("content.xml"));
    assert_eq!(images[0].alt.as_deref(), Some("A diagram & key"));
    assert_eq!(images[1].name, "Pictures/10000202.png");
    assert_eq!(images[1].referenced_from, None);
}

#[test]
fn xlsx_drawing_picture_is_found_through_the_drawing_relationships() {
    let drawing = r#"<?xml version="1.0"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Pic" descr="Chart sketch"/></xdr:nvPicPr>
<xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill></xdr:pic></xdr:wsDr>"#;
    let rels = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;
    let data = package(&[
        ("xl/drawings/drawing1.xml", drawing.as_bytes()),
        ("xl/drawings/_rels/drawing1.xml.rels", rels.as_bytes()),
        ("xl/media/image1.png", PNG_1X1),
    ]);
    let images = extract_images(&data, "xlsx").expect("images");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].name, "xl/media/image1.png");
    assert_eq!(
        images[0].referenced_from.as_deref(),
        Some("xl/drawings/drawing1.xml")
    );
    assert_eq!(images[0].alt.as_deref(), Some("Chart sketch"));
}

#[test]
fn epub_image_manifest_items_with_the_chapter_that_shows_them() {
    let container = r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles>
<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest>
<item id="ch1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
<item id="fig" href="images/fig.png" media-type="image/png"/>
<item id="spare" href="images/spare.png" media-type="image/png"/>
</manifest><spine><itemref idref="ch1"/></spine></package>"#;
    let chapter = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><p><img src="../images/fig.png" alt="Figure 1"/></p></body></html>"#;
    let data = package(&[
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/ch1.xhtml", chapter.as_bytes()),
        ("OEBPS/images/fig.png", PNG_1X1),
        ("OEBPS/images/spare.png", PNG_1X1),
    ]);
    let images = extract_images(&data, "epub").expect("images");
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].name, "OEBPS/images/fig.png");
    assert_eq!(
        images[0].referenced_from.as_deref(),
        Some("OEBPS/text/ch1.xhtml")
    );
    assert_eq!(images[0].alt.as_deref(), Some("Figure 1"));
    assert_eq!(images[1].name, "OEBPS/images/spare.png");
    assert_eq!(images[1].referenced_from, None);
}

#[test]
fn damaged_and_unsupported_inputs_are_errors_not_panics() {
    assert!(extract_images(b"not a zip", "docx").is_err());
    assert!(extract_images(&[], "docx").is_err());
    assert!(extract_images(&docx(&[]), "txt").is_err());
    let data = docx(&[]);
    for cut in (0..data.len()).step_by(37) {
        let _ = extract_images(&data[..cut], "docx");
    }
}
