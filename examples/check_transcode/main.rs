//! Debug helper: what does Transcode's geometry_from_transcode actually
//! return for each fixture?
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from("/home/ctown/projects/superdocuments/tests/fixtures");
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    let mut entries: Vec<_> = entries
        .into_iter()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("pdf"))
        .collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        println!("{:30}", p.file_name().unwrap().to_string_lossy());
        let geom = superdocuments::pdf::geometry_from_transcode(&p).unwrap();
        let pages = geom.len();
        let total_spans: usize = geom.iter().map(|p| p.text_spans.len()).sum();
        let sample: Vec<String> = geom
            .iter()
            .flat_map(|p| p.text_spans.iter())
            .take(3)
            .map(|s| {
                format!(
                    "  ({:.0},{:.0}) {:?}",
                    s.x0,
                    s.y,
                    s.text.chars().take(40).collect::<String>()
                )
            })
            .collect();
        println!(
            "  pages={}, total_spans={}, sample={:?}",
            pages, total_spans, sample
        );
    }
}
