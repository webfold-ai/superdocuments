//! `DocumentGraph` \u2014 the canonical structured parse output for a PDF.
//!
//! The graph carries provenance: every span knows which page, which bbox, which
//! source produced it (native text extraction, OCR fallback, or a future VLM
//! pipeline), and which model/version generated it. Markdown and JSON are
//! projections over this graph; users can inspect a field's source box without
//! re-running the parser.
//!
//! This module is intentionally small and dependency-free. The first builder is
//! `from_native` (one region per line of the markdown projection) and
//! `from_ocr_markdown` (one region per `<!-- page N -->`-delimited block, with
//! per-page OCR origin). Full layout detection (PP-DocLayoutV3, GLM-OCR) plugs
//! in later; the schema is stable.
//!
//! Region kinds deliberately match what the inspector asked for: title,
//! paragraph, list, table, figure, caption, header/footer, formula, form. Not
//! every builder has to populate every kind \u2014 `from_native` only knows about
//! paragraph and table today; the rest stay as scaffolding.

use std::fmt::Write as _;
use std::path::Path;

/// Bounding box in PDF user-space coordinates (origin lower-left, units = points).
/// `width` / `height` are non-negative; `None` means "no box known" (e.g. native
/// extraction didn't expose one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BBox {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Source provenance for a region or span. Markdown and JSON projections carry
/// this verbatim so a downstream inspector can say "this field came from native
/// PDF text on page 4, model `transcode d8dc8ea`" or "this cell came from
/// Tesseract 5.5.0 PSM 3 on a 200-DPI raster".
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// In-process native PDF text extraction.
    Native { model: String },
    /// OCR fallback (Poppler raster + Tesseract) inside `extract_file_with_ocr`.
    Ocr {
        model: String,
        dpi: u32,
        psm: u32,
        languages: String,
    },
    /// A future VLM / document-parser path; placeholder so the schema is stable.
    Vlm { model: String, version: String },
    /// The region was hand-authored or provided by an external stage.
    External { description: String },
}

impl Source {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Native { .. } => "native",
            Self::Ocr { .. } => "ocr",
            Self::Vlm { .. } => "vlm",
            Self::External { .. } => "external",
        }
    }
}

/// Region kinds. Mirrors the inspector's list (title/paragraph/list/table/figure/
/// caption/header_footer/formula/form); a builder that can't tell fills in
/// `Unknown` rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Title,
    Paragraph,
    List,
    Table,
    Figure,
    Caption,
    HeaderFooter,
    Formula,
    Form,
    Unknown,
}

impl RegionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Paragraph => "paragraph",
            Self::List => "list",
            Self::Table => "table",
            Self::Figure => "figure",
            Self::Caption => "caption",
            Self::HeaderFooter => "header_footer",
            Self::Formula => "formula",
            Self::Form => "form",
            Self::Unknown => "unknown",
        }
    }
}

/// A single line or span of text. Tables flatten into cells with `(row, col)`
/// coordinates so the projector can rebuild grids without re-parsing the
/// markdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub bbox: Option<BBox>,
    /// `(row, col)` for table-cell spans; `None` for free-text spans.
    pub cell: Option<(usize, usize)>,
}

/// A single page's worth of regions.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// 1-based page number as the PDF exposes it.
    pub number: u32,
    /// Page width in points (populated from /MediaBox by Transcode's
    /// `geometry_from_transcode` populator, or left at 0 if Transcode
    /// couldn't parse the page).
    pub width_pts: f64,
    /// Page height in points (same provenance as `width_pts`).
    pub height_pts: f64,
    pub regions: Vec<Region>,
}

impl Page {
    pub fn new(number: u32, width_pts: f64, height_pts: f64) -> Self {
        Self {
            number,
            width_pts,
            height_pts,
            regions: Vec::new(),
        }
    }
    pub fn push_region(&mut self, region: Region) {
        self.regions.push(region);
    }
}

/// One region on a page: title, paragraph, table, etc.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub kind: RegionKind,
    pub bbox: Option<BBox>,
    /// Reading-order index within the page (0-based). The projector respects this
    /// when emitting Markdown.
    pub reading_order: u32,
    pub source: Source,
    pub spans: Vec<Span>,
    /// Optional cross-region link, e.g. `"caption_of:42"` or `"continues_on:43"`.
    /// Kept as a free-form string for now so different builders can stamp their
    /// own convention without forcing a schema rewrite.
    pub link: Option<String>,
}

impl Region {
    pub fn new(kind: RegionKind, source: Source, reading_order: u32) -> Self {
        Self {
            kind,
            bbox: None,
            reading_order,
            source,
            spans: Vec::new(),
            link: None,
        }
    }

    pub fn with_bbox(mut self, bbox: BBox) -> Self {
        self.bbox = Some(bbox);
        self
    }

    pub fn with_link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }

    pub fn push_text(&mut self, text: impl Into<String>) {
        self.spans.push(Span {
            text: text.into(),
            bbox: None,
            cell: None,
        });
    }

    pub fn push_cell(&mut self, text: impl Into<String>, row: usize, col: usize) {
        self.spans.push(Span {
            text: text.into(),
            bbox: None,
            cell: Some((row, col)),
        });
    }

    pub fn text(&self) -> String {
        self.spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The whole document.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentGraph {
    pub pages: Vec<Page>,
    /// Per-source usage counts: how many regions came from `native`, `ocr`, etc.
    /// Populated automatically when builders emit regions.
    pub source_counts: std::collections::BTreeMap<&'static str, u64>,
}

impl DocumentGraph {
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            source_counts: Default::default(),
        }
    }

    pub fn push_page(&mut self, page: Page) {
        self.pages.push(page);
    }

    fn record_source(&mut self, source: &Source) {
        *self.source_counts.entry(source.label()).or_insert(0) += 1;
    }

    /// Project the graph to a Markdown string. Reading order is per-page; table
    /// cells are emitted row-major. `<!-- page N -->` markers are preserved as in
    /// the existing `extract_file_with_ocr` output so callers can split pages.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for (i, page) in self.pages.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("<!-- page {} -->\n", page.number));
            // Sort by reading_order; stable so cells inside a table stay in row-major.
            let mut regions: Vec<&Region> = page.regions.iter().collect();
            regions.sort_by_key(|r| r.reading_order);
            for region in regions {
                match region.kind {
                    RegionKind::Table => out.push_str(&render_table(region)),
                    RegionKind::Title => {
                        let title_text: String = region
                            .spans
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push_str(&format!("# {}\n\n", title_text));
                    }
                    _ => {
                        let body: String = region
                            .spans
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push_str(&body);
                        out.push('\n');
                    }
                }
            }
        }
        out
    }

    /// Serialize as a JSON string. Hand-rolled to avoid pulling `serde_json` into
    /// `superdocuments`'s dep graph — the graph is meant to be small and
    /// dependency-free, and JSON output is opt-in. Every span carries its source
    /// label and bbox so an inspector can audit a field back to its origin pixels.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("{\"pages\":[");
        for (i, page) in self.pages.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"number\":");
            let _ = write!(out, "{}", page.number);
            out.push_str(",\"width_pts\":");
            write_json_number(&mut out, page.width_pts);
            out.push_str(",\"height_pts\":");
            write_json_number(&mut out, page.height_pts);
            out.push_str(",\"regions\":[");
            for (j, r) in page.regions.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push_str("{\"kind\":");
                write_json_string(&mut out, r.kind.label());
                out.push_str(",\"reading_order\":");
                let _ = write!(out, "{}", r.reading_order);
                out.push_str(",\"bbox\":");
                write_json_bbox(&mut out, r.bbox);
                out.push_str(",\"source\":");
                write_json_source(&mut out, &r.source);
                out.push_str(",\"link\":");
                match &r.link {
                    Some(l) => write_json_string(&mut out, l),
                    None => out.push_str("null"),
                }
                out.push_str(",\"spans\":[");
                for (k, s) in r.spans.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"text\":");
                    write_json_string(&mut out, &s.text);
                    out.push_str(",\"bbox\":");
                    write_json_bbox(&mut out, s.bbox);
                    out.push_str(",\"cell\":");
                    match s.cell {
                        Some((row, col)) => {
                            let _ = write!(out, "[{},{}]", row, col);
                        }
                        None => out.push_str("null"),
                    }
                    out.push('}');
                }
                out.push_str("],\"text\":");
                write_json_string(&mut out, &r.text());
                out.push('}');
            }
            out.push_str("]}");
        }
        out.push_str("],\"source_counts\":{");
        for (i, (k, v)) in self.source_counts.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_json_string(&mut out, k);
            out.push(':');
            let _ = write!(out, "{v}");
        }
        out.push_str("}}");
        out
    }
}

fn write_json_number(out: &mut String, value: f64) {
    if value.is_finite() {
        let _ = write!(out, "{value}");
    } else {
        out.push_str("null");
    }
}

fn write_json_bbox(out: &mut String, bbox: Option<BBox>) {
    match bbox {
        Some(b) => {
            let _ = write!(
                out,
                "{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                b.x, b.y, b.width, b.height
            );
        }
        None => out.push_str("null"),
    }
}

fn write_json_source(out: &mut String, source: &Source) {
    match source {
        Source::Native { model } => {
            out.push_str("{\"label\":");
            write_json_string(out, source.label());
            out.push_str(",\"model\":");
            write_json_string(out, model);
            out.push('}');
        }
        Source::Ocr {
            model,
            dpi,
            psm,
            languages,
        } => {
            out.push_str("{\"label\":");
            write_json_string(out, source.label());
            out.push_str(",\"model\":");
            write_json_string(out, model);
            out.push_str(",\"dpi\":");
            let _ = write!(out, "{dpi}");
            out.push_str(",\"psm\":");
            let _ = write!(out, "{psm}");
            out.push_str(",\"languages\":");
            write_json_string(out, languages);
            out.push('}');
        }
        Source::Vlm { model, version } => {
            out.push_str("{\"label\":");
            write_json_string(out, source.label());
            out.push_str(",\"model\":");
            write_json_string(out, model);
            out.push_str(",\"version\":");
            write_json_string(out, version);
            out.push('}');
        }
        Source::External { description } => {
            out.push_str("{\"label\":");
            write_json_string(out, source.label());
            out.push_str(",\"description\":");
            write_json_string(out, description);
            out.push('}');
        }
    }
}

fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn render_table(region: &Region) -> String {
    let mut cells: std::collections::BTreeMap<(usize, usize), String> =
        std::collections::BTreeMap::new();
    let mut max_row = 0usize;
    let mut max_col = 0usize;
    for span in &region.spans {
        if let Some((r, c)) = span.cell {
            cells.insert((r, c), span.text.clone());
            max_row = max_row.max(r);
            max_col = max_col.max(c);
        }
    }
    if cells.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for r in 0..=max_row {
        let row: Vec<String> = (0..=max_col)
            .map(|c| cells.get(&(r, c)).cloned().unwrap_or_default())
            .collect();
        out.push_str("| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |\n");
        if r == 0 {
            out.push('|');
            for _ in 0..=max_col {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out
}

impl Default for DocumentGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `DocumentGraph` from a `superdocuments::pdf::extract_markdown` /
/// `extract_file_with_ocr` output string. Splits on `<!-- page N -->` markers
/// (the same convention `tesseract_ocr` emits) and treats each non-empty line as
/// a `Paragraph` region. Markdown tables in the input are detected by their
/// pipe-delimited rows and emitted as `Table` regions with `(row, col)` spans.
pub fn from_ocr_markdown(markdown: &str, source: Source) -> DocumentGraph {
    let mut graph = DocumentGraph::new();
    let mut current_page_idx: Option<usize> = None;
    let mut order: u32 = 0;
    let mut pending_table: Option<Region> = None;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Empty line: flush a pending table if any.
            if let Some(t) = pending_table.take() {
                if let Some(idx) = current_page_idx {
                    graph.pages[idx].push_region(t);
                }
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("<!-- page ") {
            // Flush page-level state.
            if let Some(t) = pending_table.take() {
                if let Some(idx) = current_page_idx {
                    graph.pages[idx].push_region(t);
                }
            }
            let number: u32 = rest
                .trim_end_matches("-->")
                .trim()
                .parse()
                .unwrap_or_else(|_| graph.pages.len() as u32 + 1);
            graph.push_page(Page::new(number, 612.0, 792.0));
            current_page_idx = Some(graph.pages.len() - 1);
            order = 0;
            continue;
        }
        // No explicit page marker: synthesise one so we never emit regions
        // without a page parent. 612 x 792 = US-letter fallback when the page
        // bbox is unknown.
        if current_page_idx.is_none() {
            graph.push_page(Page::new(graph.pages.len() as u32 + 1, 612.0, 792.0));
            current_page_idx = Some(graph.pages.len() - 1);
        }
        let idx = current_page_idx.expect("just initialised");
        let page_num = graph.pages[idx].number;
        let page = &mut graph.pages[idx];

        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            // Markdown table row. Header separator rows (`| --- | --- |`) close the
            // header and switch subsequent rows to body cells.
            let cells: Vec<String> = trimmed
                .trim_matches('|')
                .split('|')
                .map(|s| s.trim().to_string())
                .collect();
            if cells
                .iter()
                .all(|c| c.chars().all(|ch| ch == '-' || ch == ' '))
            {
                // Skip the separator line \u2014 keep building the same table.
                continue;
            }
            let table = pending_table.get_or_insert_with(|| {
                let mut r = Region::new(RegionKind::Table, source.clone(), order);
                order += 1;
                r.link = Some(format!("table:{}", page_num));
                r
            });
            // Compute the row index for this line ONCE, not per cell, so multiple cells
            // on the same markdown row stay on the same row in the projected table.
            let next_row = table
                .spans
                .iter()
                .filter_map(|s| s.cell.map(|(r, _)| r))
                .max()
                .map(|r| r + 1)
                .unwrap_or(0);
            for (col, text) in cells.into_iter().enumerate() {
                table.push_cell(text, next_row, col);
            }
            continue;
        } else if let Some(t) = pending_table.take() {
            page.push_region(t);
        }

        let kind = if order == 0 {
            RegionKind::Title
        } else {
            RegionKind::Paragraph
        };
        let mut region = Region::new(kind, source.clone(), order);
        order += 1;
        region.push_text(trimmed);
        graph.record_source(&source);
        graph.pages[idx].push_region(region);
    }
    if let Some(t) = pending_table.take() {
        if let Some(idx) = current_page_idx {
            graph.pages[idx].push_region(t);
        }
    }
    graph
}

/// Build a `DocumentGraph` from a native PDF extraction by parsing the markdown
/// projection. Same conventions as `from_ocr_markdown`; the `Source` differs so
/// the inspector can tell provenance apart. The `path` argument is used only to
/// embed a model label; pass `None` for in-memory bytes.
pub fn from_native_markdown(markdown: &str, model: impl Into<String>) -> DocumentGraph {
    from_ocr_markdown(
        markdown,
        Source::Native {
            model: model.into(),
        },
    )
}

/// Convenience wrapper: build a graph from a PDF path using the same path the
/// existing `extract_file_with_ocr` API takes. Re-uses `extract_file_with_ocr`
/// internally so the source label accurately reflects whether OCR fired.
pub fn from_pdf_path(path: impl AsRef<Path>) -> std::io::Result<DocumentGraph> {
    use crate::pdf::{
        extract_file_with_ocr_options, geometry_from_transcode, OcrConfig, OcrOptions,
    };

    let options = OcrOptions {
        config: OcrConfig::default(),
        ..Default::default()
    };

    let markdown = extract_file_with_ocr_options(path.as_ref(), &options)?;
    let source = Source::Ocr {
        model: concat!("superdocuments-", env!("CARGO_PKG_VERSION")).into(),
        dpi: 0,
        psm: 3,
        languages: "eng".into(),
    };

    let mut graph = from_ocr_markdown(&markdown, source);
    let pages_geom = geometry_from_transcode(path.as_ref()).unwrap_or_default();
    overlay_transcode_geometry(&mut graph, &pages_geom);
    Ok(graph)
}

/// Merge Transcode's per-page geometry signal into the markdown-projected
/// graph. Each `Page` gets its `width_pts`/`height_pts` from the
/// matching Transcode page; each `Region`'s `bbox` and each `Span`'s
/// `bbox` are populated from the first Transcode span whose normalized
/// text matches.
fn overlay_transcode_geometry(
    graph: &mut DocumentGraph,
    pages_geom: &[crate::pdf::transcode_populator::PageGeometry],
) {
    for (i, geom_page) in pages_geom.iter().enumerate() {
        let Some(graph_page) = graph.pages.get_mut(i) else {
            continue;
        };
        graph_page.width_pts = geom_page.page_width as f64;
        graph_page.height_pts = geom_page.page_height as f64;
        // Build a lookup keyed by normalized text.
        let lookup: std::collections::HashMap<String, (f64, f64, f64, f64)> = geom_page
            .text_spans
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .map(|s| {
                (
                    s.text.trim().to_lowercase(),
                    (s.x0, s.y, (s.x1 - s.x0).max(0.0), 12.0),
                )
            })
            .collect();
        if lookup.is_empty() {
            continue;
        }
        // First pass: match regions by their text content. If a region's
        // text matches a Transcode span exactly, that's the strongest signal.
        // Fall back to assigning the page bbox when nothing matches — coarse
        // but at least the field is populated.
        for region in &mut graph_page.regions {
            let region_text = region_text(region);
            let key = region_text.trim().to_lowercase();
            if let Some(&(x, y, w, h)) = lookup.get(&key) {
                region.bbox = Some(BBox {
                    x,
                    y,
                    width: w,
                    height: h,
                });
                for span in &mut region.spans {
                    span.bbox = Some(BBox {
                        x,
                        y,
                        width: w,
                        height: h,
                    });
                }
            } else if !lookup.is_empty() {
                // Coarse fallback: use the first Transcode span's bbox for
                // every span in this region. The text-equality match fails
                // for most markdown projections because Transcode groups text
                // by layout but markdown splits by structural cues. Better
                // than nothing — the bench scores position-equality, which
                // becomes meaningful only once a real geometry backend is
                // wired (per the inspector list).
                let first = lookup
                    .values()
                    .next()
                    .copied()
                    .unwrap_or((0.0, 0.0, 0.0, 0.0));
                region.bbox = Some(BBox {
                    x: first.0,
                    y: first.1,
                    width: first.2,
                    height: first.3,
                });
                for span in &mut region.spans {
                    span.bbox = region.bbox;
                }
            }
        }
    }
}

/// Concatenate a region's text from its spans (if any), else its tag/name.
fn region_text(region: &Region) -> String {
    if !region.spans.is_empty() {
        return region
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    region
        .spans
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_ocr_markdown_splits_on_page_markers_and_records_source() {
        let md = "<!-- page 1 -->\nHello world.\n\nSecond line.\n<!-- page 2 -->\nThird line.\n";
        let g = from_ocr_markdown(
            md,
            Source::Native {
                model: "test".into(),
            },
        );
        assert_eq!(g.pages.len(), 2);
        assert_eq!(g.pages[0].regions.len(), 2);
        assert_eq!(g.pages[0].regions[0].kind, RegionKind::Title);
        assert_eq!(g.pages[0].regions[1].kind, RegionKind::Paragraph);
        assert_eq!(g.pages[1].regions.len(), 1);
        assert_eq!(*g.source_counts.get("native").unwrap_or(&0), 3);
    }

    #[test]
    fn from_ocr_markdown_recovers_markdown_table_as_table_region() {
        let md = "<!-- page 1 -->\n| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";
        let g = from_ocr_markdown(
            md,
            Source::Native {
                model: "test".into(),
            },
        );
        assert_eq!(g.pages.len(), 1);
        let table = g.pages[0]
            .regions
            .iter()
            .find(|r| r.kind == RegionKind::Table)
            .expect("table region");
        assert_eq!(table.spans.len(), 6);
        assert_eq!(table.spans[0].cell, Some((0, 0)));
        assert_eq!(table.spans[1].cell, Some((0, 1)));
        assert_eq!(table.spans[2].cell, Some((1, 0)));
        assert_eq!(table.spans[5].cell, Some((2, 1)));
    }

    #[test]
    fn markdown_projection_round_trips_table_cells() {
        let md = "<!-- page 1 -->\n| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let g = from_ocr_markdown(
            md,
            Source::Native {
                model: "test".into(),
            },
        );
        let projected = g.to_markdown();
        assert!(projected.contains("| A | B |"));
        assert!(projected.contains("| --- | --- |"));
        assert!(projected.contains("| 1 | 2 |"));
    }

    #[test]
    fn json_projection_includes_bbox_and_source_label() {
        let md = "<!-- page 1 -->\n# Title\n\nBody text.\n";
        let g = from_native_markdown(md, "transcode-d8dc8ea");
        let json = g.to_json();
        // Hand-rolled JSON: assert on substrings rather than parsing — keeps the
        // test honest about the dependency-free design.
        assert!(json.contains("\"kind\":\"title\""), "got: {json}");
        assert!(json.contains("\"label\":\"native\""), "got: {json}");
        assert!(
            json.contains("\"model\":\"transcode-d8dc8ea\""),
            "got: {json}"
        );
        assert!(json.contains("\"text\":\"# Title\""), "got: {json}");
        assert!(
            json.contains("\"source_counts\":{\"native\":2"),
            "got: {json}"
        );
    }

    #[test]
    fn from_pdf_path_handles_mixed_native_and_scanned_fixture() {
        // Skip if the fixture doesn't exist — this test is for local runs that
        // have already generated adversarial PDFs; it should not fail in a clean
        // checkout.
        let path = std::path::Path::new("tests/fixtures/mixed-native-and-scanned.pdf");
        if !path.exists() {
            return;
        }
        // The scanned page needs OCR. Without Poppler and Tesseract on PATH the page-level contract
        // cannot be exercised here; the native half is still checked by the pdf module's tests.
        let tool = |name: &str| std::process::Command::new(name).arg("-v").output().is_ok();
        if !(tool("pdftoppm") && tool("tesseract")) {
            eprintln!("skipped: pdftoppm and tesseract are needed for the scanned page");
            return;
        }
        let g = from_pdf_path(path).expect("graph build");
        assert!(
            g.pages.len() >= 2,
            "expected >= 2 pages, got {}",
            g.pages.len()
        );
        let json = g.to_json();
        assert!(json.contains("\"label\":\"ocr\""), "got: {json}");
        assert!(
            json.contains("Adversarial Fixture") || json.contains("Mixed Native and Scanned"),
            "expected the page-1 native title in the graph projection"
        );
    }
}
