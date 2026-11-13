//! The page rule on its own inputs (see `superdocuments::pdf::page_profile`).

mod common;
use common::*;


#[test]
fn page_rule_on_geometry_and_text() {
    use superdocuments::pdf::page_profile::{page_ocr_verdicts, pages_needing_ocr, OcrReason};
    let text = dense_text();
    let pdf = build(&[
        page(FULL_PAGE, Some(STAMP)),             // 1: scan + stamp
        page(FULL_PAGE, None),                    // 2: scan, no text
        page(Some((60, 60)), Some("Short")),      // 3: crest, small coverage
        page(FULL_PAGE, Some(&text)),             // 4: scan with a full text layer
        page(None, Some(STAMP)),                  // 5: no image: a short born-digital page
        Page {                                    // 6: small page, same image size: sparse by area
            size: Some((200, 200)),
            image: Some((200, 200)),
            text: Some(STAMP.to_string()),
        },
        page(Some((290, 842)), Some(STAMP)),      // 7: image covers 49% of the page
    ]);
    let v = page_ocr_verdicts(&pdf).unwrap();
    let reasons: Vec<Option<OcrReason>> = v.iter().map(|p| p.reason).collect();
    assert_eq!(
        reasons,
        vec![
            Some(OcrReason::ScannedImage),
            Some(OcrReason::NoText),
            None,
            None,
            None,
            None, // 200x200 pt page: 46 characters is 1.15 per 1000 pt^2, above the threshold
            None,
        ],
        "{v:#?}"
    );
    assert!((v[0].image_coverage - 1.0).abs() < 1e-9);
    assert!(v[6].image_coverage > 0.48 && v[6].image_coverage < 0.5, "{v:?}");
    assert_eq!(pages_needing_ocr(&pdf).unwrap(), vec![1, 2]);
}

#[test]
fn media_box_is_inherited_from_the_pages_node() {
    // pages carry no /MediaBox of their own: the A4 box of the /Pages node applies
    let pdf = build(&[page(Some((595, 842)), Some(STAMP))]);
    let v = superdocuments::pdf::page_profile::page_ocr_verdicts(&pdf).unwrap();
    assert!((v[0].image_coverage - 1.0).abs() < 1e-9, "{v:?}");
    // 54 alphanumerics on A4 (501 pt^2/1000): 0.11 per 1000 pt^2
    assert!(v[0].text_density < 0.2, "{v:?}");
}

