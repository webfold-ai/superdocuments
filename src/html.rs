//! Shared pure HTML-to-content conversion. Fetching, URL resolution, browser state and
//! crawler policy remain owned by Webfold or a consuming application.

/// Convert HTML bytes to Markdown using the native transcode HTML parser.
pub fn to_markdown(html: &[u8], base_url: &str) -> String {
    let html = String::from_utf8_lossy(html);
    to_markdown_str(&html, base_url)
}

/// Convert an already-decoded HTML string to Markdown.
pub fn to_markdown_str(html: &str, base_url: &str) -> String {
    let blocks = transcode::doc::html::to_blocks(html, base_url);
    transcode::doc::html::render(&blocks, None, 1).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_html_without_fetching_resources() {
        let markdown = to_markdown_str(
            "<html><body><h1>Title</h1><p>Hello <strong>world</strong></p></body></html>",
            "https://example.test/",
        );
        assert!(markdown.contains("Title"));
        assert!(markdown.contains("Hello"));
        assert!(markdown.contains("world"));
    }
}
