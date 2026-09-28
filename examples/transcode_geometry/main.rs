//! Smoke test that proves the Transcode-backed geometry populator wires
//! real bboxes into `from_pdf_path`'s output. Iterates every shipped
//! fixture and prints the per-page counts.

use std::fs;
use std::path::PathBuf;

fn main() {
    let fixtures = PathBuf::from("/home/ctown/projects/superdocuments/tests/fixtures");
    println!("== superdocuments transcode-geometry smoke ==");
    let Ok(entries) = fs::read_dir(&fixtures) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("pdf") {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        print!("{:<40}", name);
        match superdocuments::graph::from_pdf_path(&p) {
            Ok(g) => {
                let pages = g.pages.len();
                let mut total_items = 0usize;
                let mut with_bbox = 0usize;
                let mut total_regions = 0usize;
                let mut with_pwh = 0usize;
                for page in &g.pages {
                    if page.width_pts > 0.0 && page.height_pts > 0.0 {
                        with_pwh += 1;
                    }
                    total_regions += page.regions.len();
                    for region in &page.regions {
                        for span in &region.spans {
                            total_items += 1;
                            if span.bbox.is_some() {
                                with_bbox += 1;
                            }
                        }
                    }
                }
                println!(
                    " pages={} regions={} text_items={} bbox={} w×h={}",
                    pages, total_regions, total_items, with_bbox, with_pwh
                );
            }
            Err(e) => println!(" ERROR: {e}"),
        }
    }
}
