//! Outline-coverage measurement: how many PDFs even HAVE a usable outline?
//!
//! This is a MEASUREMENT over whatever directory of PDFs you point it at — not a
//! quality claim about anything. It answers one question per file: does the
//! document carry its own bookmark structure, how big is it, how deep, and what
//! fraction of the nodes actually resolved to a start page.
//!
//! Usage: `cargo run --example outline_coverage -- <dir-of-pdfs> [manifest.json]`
//!
//! With a second argument (a PageIndex `manifest.json`), the run also prints the
//! five documents with the most outline nodes alongside the first ten titles of
//! the matching PageIndex `tree.json`, for a side-by-side look at what the
//! producer's outline says versus what the LLM-based tree says.

use std::path::{Path, PathBuf};

use superdocuments::pdf::outline::{self, OutlineNode};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: outline_coverage <dir-of-pdfs> [pageindex-manifest.json]");
        std::process::exit(2);
    };
    let manifest = args.next().map(PathBuf::from);
    let dir = PathBuf::from(&dir);

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("pdf"))
        .collect();
    paths.sort();

    println!(
        "{:<48} {:>5} {:>10} {:>6} {:>6} {:>9}",
        "file", "pages", "has_outline", "nodes", "depth", "resolved%"
    );

    let mut rows: Vec<Row> = Vec::new();
    for path in &paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::read(path).expect("read pdf");
        match (outline::page_count(&bytes), outline::outline_tree(&bytes)) {
            (Ok(pages), Ok(nodes)) => {
                let (resolved, total) = outline::resolved_counts(&nodes);
                let row = Row {
                    name: name.clone(),
                    pages,
                    nodes: outline::node_count(&nodes),
                    depth: outline::max_depth(&nodes),
                    resolved_pct: percent(resolved, total),
                    tree: nodes,
                };
                println!(
                    "{:<48} {:>5} {:>10} {:>6} {:>6} {:>8.1}%",
                    name,
                    row.pages,
                    row.nodes > 0,
                    row.nodes,
                    row.depth,
                    row.resolved_pct
                );
                rows.push(row);
            }
            (page_result, tree_result) => {
                // A PDF lopdf cannot load still counts as a document in the
                // denominator; it just has no outline.
                let message = match (page_result, tree_result) {
                    (Err(error), _) | (_, Err(error)) => error.to_string(),
                    (Ok(_), Ok(_)) => unreachable!(),
                };
                println!(
                    "{:<48} {:>5} {:>10} {:>6} {:>6} {:>9}  ({message})",
                    name, "?", false, 0, 0, "n/a"
                );
                rows.push(Row {
                    name,
                    pages: 0,
                    nodes: 0,
                    depth: 0,
                    resolved_pct: 0.0,
                    tree: Vec::new(),
                });
            }
        }
    }

    let with_outline = rows.iter().filter(|row| row.nodes > 0).count();
    let usable: Vec<&Row> = rows.iter().filter(|row| row.nodes >= 3).collect();
    let median_nodes = median(usable.iter().map(|row| row.nodes as f64));
    let median_resolved = median(rows.iter().map(|row| row.resolved_pct));
    let median_all_nodes = median(rows.iter().map(|row| row.nodes as f64));

    println!();
    println!("SUMMARY (measurement over this directory of PDFs — not a quality claim)");
    println!("  pdfs                              {}", rows.len());
    println!("  with at least one outline node     {with_outline}");
    println!("  with at least 3 outline nodes      {}", usable.len());
    println!("  median node count (all pdfs)       {median_all_nodes:.1}");
    println!("  median node count (>=3 nodes)      {median_nodes:.1}");
    println!("  median percent nodes resolved      {median_resolved:.1}%");

    if let Some(manifest) = manifest {
        compare_with_pageindex(&rows, &manifest);
    }
}

struct Row {
    name: String,
    pages: u32,
    nodes: usize,
    depth: u32,
    resolved_pct: f64,
    tree: Vec<OutlineNode>,
}

fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut sorted = values.collect::<Vec<f64>>();
    if sorted.is_empty() {
        return 0.0;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN counts"));
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

/// Side-by-side listing only: the outline's top-level titles next to the first
/// ten titles of the matching PageIndex tree. No scoring — this is for a human
/// to look at.
fn compare_with_pageindex(rows: &[Row], manifest: &Path) {
    let base = manifest.parent().unwrap_or(Path::new("."));
    let Ok(text) = std::fs::read_to_string(manifest) else {
        eprintln!("could not read manifest {}", manifest.display());
        return;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
        eprintln!("manifest {} is not JSON", manifest.display());
        return;
    };
    let Some(docs) = manifest.get("docs").and_then(|docs| docs.as_object()) else {
        eprintln!("manifest has no `docs` object");
        return;
    };
    let mut by_name: Vec<(String, PathBuf)> = docs
        .iter()
        .filter_map(|(id, doc)| {
            let name = doc.get("name")?.as_str()?.to_string();
            Some((name, base.join("docs").join(id).join("tree.json")))
        })
        .collect();
    by_name.sort();

    let mut top: Vec<&Row> = rows.iter().collect();
    top.sort_by(|a, b| b.nodes.cmp(&a.nodes).then_with(|| a.name.cmp(&b.name)));
    let top = top.into_iter().take(5).collect::<Vec<_>>();

    println!();
    println!("TOP-LEVEL TITLES: PDF outline vs first 10 PageIndex titles");
    for row in top {
        println!();
        println!(
            "=== {} ({} outline nodes, depth {})",
            row.name, row.nodes, row.depth
        );
        let Some((_, tree_path)) = by_name.iter().find(|(name, _)| *name == row.name) else {
            println!("    (no PageIndex tree.json for this document)");
            continue;
        };
        let pageindex_titles: Vec<String> = std::fs::read_to_string(tree_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|tree| tree.as_array().cloned())
            .map(|nodes| {
                nodes
                    .iter()
                    .take(10)
                    .map(|node| {
                        node.get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("?")
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let outline_titles = row
            .tree
            .iter()
            .map(|node| node.title.clone())
            .collect::<Vec<_>>();
        let width = outline_titles.len().max(pageindex_titles.len()).max(1);
        println!(
            "    {:<44} | PageIndex (first {} shown)",
            "PDF outline (top level)",
            pageindex_titles.len().min(10)
        );
        for index in 0..width.min(20) {
            println!(
                "    {:<44} | {}",
                truncate(
                    outline_titles.get(index).map(String::as_str).unwrap_or(""),
                    44
                ),
                truncate(
                    pageindex_titles
                        .get(index)
                        .map(String::as_str)
                        .unwrap_or(""),
                    60
                )
            );
        }
        if outline_titles.len() > 20 {
            println!(
                "    … ({} top-level outline titles total)",
                outline_titles.len()
            );
        }
    }
}

fn truncate(text: &str, width: usize) -> String {
    let cleaned = text.replace(['\n', '\r'], " ");
    if cleaned.chars().count() <= width {
        return cleaned;
    }
    let head = cleaned
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    format!("{head}…")
}
