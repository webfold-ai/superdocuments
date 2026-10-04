//! Shared pure HTML-to-content conversion. Fetching, browser state and crawler policy remain
//! owned by Webfold or a consuming application.
//!
//! `extract` is the single-pass HTML-to-Markdown extractor (moved here from Transcode's
//! `doc::html`, which is not part of the GitHub Transcode). Its block API is re-exported so
//! Webfold can expose it unchanged: `to_blocks`, `render`, `Template`, `links`, `resolve`,
//! `chunk_markdown`.

mod entities;
pub mod extract;

pub use extract::{
    chunk_markdown, insert_with, links, norm_sig, render, resolve, to_blocks,
    to_blocks_with_headings, Block, BlockWithHeadings, ChunkSeed, Template, TextChunk,
};

/// One content block from an HTML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlBlock {
    pub markdown: String,
    pub signature: u64,
    pub links: Vec<String>,
}

/// Parsed HTML content in both block and Markdown forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlDocument {
    pub blocks: Vec<HtmlBlock>,
    pub markdown: String,
}

/// Parse an already-decoded HTML string without fetching resources.
pub fn parse(html: &str, base_url: &str) -> HtmlDocument {
    let raw_blocks = to_blocks(html, base_url);
    let markdown = render(&raw_blocks, None, 1).0;
    let blocks = raw_blocks
        .into_iter()
        .map(|block| HtmlBlock {
            markdown: block.md,
            signature: block.sig,
            links: block.links,
        })
        .collect();
    HtmlDocument { blocks, markdown }
}

/// The same parse as [`parse`], with each block's ancestor heading path attached — the
/// `h1`/`h2`… breadcrumbs in force where the block was emitted. Additive: nothing about
/// `parse`'s output changes.
pub fn parse_with_headings(html: &str, base_url: &str) -> (HtmlDocument, Vec<Vec<String>>) {
    let document = parse(html, base_url);
    let paths: Vec<Vec<String>> = to_blocks_with_headings(html, base_url)
        .into_iter()
        .map(|entry| entry.heading_path)
        .collect();
    (document, paths)
}

/// Convert HTML bytes to Markdown.
pub fn to_markdown(html: &[u8], base_url: &str) -> String {
    let html = String::from_utf8_lossy(html);
    parse(&html, base_url).markdown
}

/// Convert an already-decoded HTML string to Markdown.
pub fn to_markdown_str(html: &str, base_url: &str) -> String {
    parse(html, base_url).markdown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks_and_renders_markdown_without_fetching() {
        let document = parse(
            "<html><body><h1>Title</h1><p>Hello <strong>world</strong></p></body></html>",
            "https://example.test/",
        );
        assert!(!document.blocks.is_empty());
        assert!(document.markdown.contains("Title"));
        assert!(document.markdown.contains("Hello"));
        assert!(document.markdown.contains("world"));
    }

    #[test]
    fn mismatched_close_tag_does_not_end_a_dropped_subtree() {
        // `</header>` has no matching open inside `<nav>`; it must not end the drop.
        let md = to_markdown_str(
            "<nav><a href=\"/x\">Menu</a></header><p>nav leak</p></nav><p>Body text</p>",
            "https://example.test/",
        );
        assert!(md.contains("Body text"), "{md}");
        assert!(!md.contains("nav leak"), "{md}");
        assert!(!md.contains("Menu"), "{md}");
    }

    #[test]
    fn nested_drop_subtrees_close_in_order() {
        let md = to_markdown_str(
            "<nav><form><p>inner</p></form><p>still nav</p></nav><p>Body</p>",
            "https://example.test/",
        );
        assert!(md.contains("Body"), "{md}");
        assert!(!md.contains("still nav"), "{md}");
        assert!(!md.contains("inner"), "{md}");
    }

    #[test]
    fn drop_tags_match_case_insensitively() {
        let md = to_markdown_str("<NAV><p>chrome</p></NAV><p>Body</p>", "https://example.test/");
        assert!(md.contains("Body"), "{md}");
        assert!(!md.contains("chrome"), "{md}");
    }

    #[test]
    fn parse_with_headings_returns_paths_alongside_the_same_document() {
        let (document, paths) = parse_with_headings(
            "<html><body><h1>Guide</h1><h2>Install</h2><p>Run it</p></body></html>",
            "https://example.test/",
        );
        assert_eq!(paths.len(), document.blocks.len());
        assert_eq!(paths[0], Vec::<String>::new());
        assert_eq!(paths[1], vec!["Guide".to_string()]);
        assert_eq!(
            paths[2],
            vec!["Guide".to_string(), "Install".to_string()]
        );
        assert!(document.markdown.contains("Run it"));
    }
}
