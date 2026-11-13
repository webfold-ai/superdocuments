//! `pdf_page_routes FILE.pdf...` prints, per file, one JSON line with the pages the page rule routes to OCR
//! (no OCR is run): the page numbers, reason, native alphanumeric characters, image coverage and text density.
use superdocuments::pdf::page_profile::{page_ocr_verdicts, OcrReason};

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("read pdf");
        match page_ocr_verdicts(&bytes) {
            Ok(verdicts) => {
                let flagged: Vec<String> = verdicts
                    .iter()
                    .filter_map(|v| {
                        v.reason.map(|r| {
                            let why = match r {
                                OcrReason::NoText => "no_text",
                                OcrReason::ScannedImage => "scanned_image",
                            };
                            format!(
                                "{{\"page\":{},\"reason\":\"{why}\",\"text_chars\":{},\"image_coverage\":{:.3},\"text_density\":{:.3}}}",
                                v.page, v.text_chars, v.image_coverage, v.text_density
                            )
                        })
                    })
                    .collect();
                println!(
                    "{{\"file\":{path:?},\"pages\":{},\"flagged\":[{}]}}",
                    verdicts.len(),
                    flagged.join(",")
                );
            }
            Err(e) => println!("{{\"file\":{path:?},\"error\":{:?}}}", e.to_string()),
        }
    }
}
