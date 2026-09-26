//! Shared pure HTML-to-content conversion. Fetching, URL resolution, browser state and
//! crawler policy remain owned by Webfold or a consuming application.

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
    let transcode_blocks = transcode::doc::html::to_blocks(html, base_url);
    let markdown = transcode::doc::html::render(&transcode_blocks, None, 1).0;
    let blocks = transcode_blocks
        .into_iter()
        .map(|block| HtmlBlock {
            markdown: block.md,
            signature: block.sig,
            links: block.links,
        })
        .collect();
    HtmlDocument { blocks, markdown }
}

/// Convert HTML bytes to Markdown using the native transcode HTML parser.
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
}
