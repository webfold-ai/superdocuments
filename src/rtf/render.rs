//! Plain text and Markdown output rendered from an `RtfDocument`.

use super::model::{
    Block, Cell, Hyperlink, ListItem, Options, Paragraph, RtfDocument, RtfError, Run, Table,
    TableStyle,
};

/// A rendered block. List items carry their kind (`Some(ordered)`), so consecutive items of the
/// same kind are separated by one newline and every other pair of blocks by a blank line.
type Chunk = Option<(Option<bool>, String)>;

struct Ctx<'a> {
    options: &'a Options,
    links: &'a [Hyperlink],
}

/// Plain text: paragraphs separated by blank lines, list items on consecutive lines, tables as
/// tab-separated or ` | `-separated rows, then headers, footers and footnotes.
pub(super) fn text(doc: &RtfDocument, options: &Options) -> Result<String, RtfError> {
    let ctx = Ctx {
        options,
        links: &doc.hyperlinks,
    };
    let mut chunks = Vec::new();
    for block in doc
        .blocks
        .iter()
        .chain(doc.parts.iter().flat_map(|p| p.blocks.iter()))
    {
        chunks.push(text_block(&ctx, block));
    }
    for note in &doc.footnotes {
        let body: Vec<String> = note
            .blocks
            .iter()
            .filter_map(|b| text_block(&ctx, b).map(|(_, t)| t))
            .collect();
        if !body.is_empty() {
            chunks.push(Some((
                None,
                format!("[{}] {}", note.number, body.join(" ")),
            )));
        }
    }
    join(chunks, options)
}

/// Markdown: headings as `#` lines, emphasis, strikethrough and links as Markdown syntax, list
/// items as `-` or `N.`, tables as pipe tables (or tab-separated rows with `TableStyle::Tabs`),
/// and footnotes as `[^n]:` definitions. Underline and font details are not represented.
pub(super) fn markdown(doc: &RtfDocument, options: &Options) -> Result<String, RtfError> {
    let ctx = Ctx {
        options,
        links: &doc.hyperlinks,
    };
    let mut chunks = Vec::new();
    for block in doc
        .blocks
        .iter()
        .chain(doc.parts.iter().flat_map(|p| p.blocks.iter()))
    {
        chunks.push(markdown_block(&ctx, block));
    }
    for note in &doc.footnotes {
        let body: Vec<String> = note
            .blocks
            .iter()
            .filter_map(|b| markdown_block(&ctx, b).map(|(_, t)| t))
            .collect();
        if !body.is_empty() {
            chunks.push(Some((
                None,
                format!("[^{}]: {}", note.number, body.join(" ")),
            )));
        }
    }
    join(chunks, options)
}

fn join(chunks: Vec<Chunk>, options: &Options) -> Result<String, RtfError> {
    let limit = options.limits.max_output_bytes;
    let mut out = String::new();
    let mut previous_kind: Option<Option<bool>> = None;
    for (kind, text) in chunks.into_iter().flatten() {
        if !out.is_empty() {
            let tight = matches!((previous_kind, kind), (Some(Some(a)), Some(b)) if a == b);
            out.push_str(if tight { "\n" } else { "\n\n" });
        }
        out.push_str(&text);
        previous_kind = Some(kind);
        if out.len() > limit {
            return Err(RtfError::OutputTooLarge { limit });
        }
    }
    Ok(out.trim_end().to_string())
}

// ----- plain text -----------------------------------------------------------------------------

fn text_block(ctx: &Ctx<'_>, block: &Block) -> Chunk {
    match block {
        Block::Paragraph(p) => {
            let text = plain_runs(&p.runs);
            let text = text.trim();
            (!text.is_empty()).then(|| (None, text.to_string()))
        }
        Block::ListItem(item) => {
            let text = plain_runs(&item.runs);
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let marker = if item.marker.is_empty() {
                "-"
            } else {
                item.marker.as_str()
            };
            let indent = "  ".repeat(usize::from(item.level));
            Some((Some(item.ordered), format!("{indent}{marker} {text}")))
        }
        Block::Table(table) => plain_table(ctx, table).map(|t| (None, t)),
    }
}

fn plain_table(ctx: &Ctx<'_>, table: &Table) -> Option<String> {
    let separator = match ctx.options.table_style {
        TableStyle::Pipes => " | ",
        TableStyle::Tabs => "\t",
    };
    let lines: Vec<String> = table
        .rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = row.cells.iter().map(|c| plain_cell(ctx, c)).collect();
            cells.join(separator)
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn plain_cell(ctx: &Ctx<'_>, cell: &Cell) -> String {
    let parts: Vec<String> = cell
        .blocks
        .iter()
        .filter_map(|b| text_block(ctx, b).map(|(_, t)| t))
        .collect();
    parts.join(" ").replace('\n', " ")
}

fn plain_runs(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

// ----- Markdown -------------------------------------------------------------------------------

fn markdown_block(ctx: &Ctx<'_>, block: &Block) -> Chunk {
    match block {
        Block::Paragraph(p) => markdown_paragraph(ctx, p),
        Block::ListItem(item) => markdown_list(ctx, item),
        Block::Table(table) => markdown_table(ctx, table).map(|t| (None, t)),
    }
}

fn markdown_paragraph(ctx: &Ctx<'_>, p: &Paragraph) -> Chunk {
    match p.heading_level {
        Some(level) => {
            // A heading is already emphasised by its level, so bold is not repeated.
            let text = markdown_runs_with(ctx, &p.runs, false);
            let text = text.trim().replace("\\\n", " ");
            if text.is_empty() {
                return None;
            }
            let level = usize::from(level.clamp(1, 6));
            Some((None, format!("{} {}", "#".repeat(level), text)))
        }
        None => {
            let text = markdown_runs(ctx, &p.runs);
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            Some((None, escape_line_start(text)))
        }
    }
}

/// Escapes a leading `#`, `>`, `-`, `+`, `*` or `N.` so that the paragraph is not read as a
/// heading, quote or list.
fn escape_line_start(text: &str) -> String {
    let bytes = text.as_bytes();
    match bytes.first() {
        Some(b'#') | Some(b'>') => format!("\\{text}"),
        Some(b'-') | Some(b'+') | Some(b'*') if bytes.get(1) == Some(&b' ') => {
            format!("\\{text}")
        }
        Some(first) if first.is_ascii_digit() => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            match bytes.get(digits) {
                Some(b'.') | Some(b')') if bytes.get(digits + 1) == Some(&b' ') => {
                    format!("{}\\{}", &text[..digits], &text[digits..])
                }
                _ => text.to_string(),
            }
        }
        _ => text.to_string(),
    }
}

fn markdown_list(ctx: &Ctx<'_>, item: &ListItem) -> Chunk {
    let text = markdown_runs(ctx, &item.runs);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let indent = "    ".repeat(usize::from(item.level));
    let marker = if item.ordered {
        let number = item
            .marker
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse::<u32>()
            .unwrap_or(1);
        format!("{number}.")
    } else {
        "-".to_string()
    };
    Some((
        Some(item.ordered),
        format!("{indent}{marker} {}", text.replace("\\\n", "<br>")),
    ))
}

fn markdown_table(ctx: &Ctx<'_>, table: &Table) -> Option<String> {
    if table.rows.is_empty() {
        return None;
    }
    if ctx.options.table_style == TableStyle::Tabs {
        return plain_table(ctx, table);
    }
    // A pipe table cannot hold a table: when one cell contains another table the whole table is
    // written as HTML, which Markdown allows and which keeps every cell in its own element.
    if has_nested_table(table) {
        return Some(html_table(ctx, table));
    }
    let rows: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|c| markdown_cell(ctx, c)).collect())
        .collect();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (index, row) in rows.iter().enumerate() {
        let mut cells = row.clone();
        cells.resize(columns, String::new());
        lines.push(format!("| {} |", cells.join(" | ")));
        if index == 0 {
            lines.push(format!("|{}|", vec![" --- "; columns].join("|")));
        }
    }
    Some(lines.join("\n"))
}

fn has_nested_table(table: &Table) -> bool {
    table
        .rows
        .iter()
        .flat_map(|r| r.cells.iter())
        .flat_map(|c| c.blocks.iter())
        .any(|b| matches!(b, Block::Table(_)))
}

/// An HTML table, nested tables included. No blank lines, so a Markdown reader keeps it as one
/// HTML block.
fn html_table(ctx: &Ctx<'_>, table: &Table) -> String {
    let mut out = String::from("<table>\n");
    for row in &table.rows {
        out.push_str("<tr>");
        let tag = if row.header { "th" } else { "td" };
        for cell in &row.cells {
            out.push_str(&format!("<{tag}>"));
            let mut previous_text = false;
            for block in &cell.blocks {
                let (html, is_text) = match block {
                    Block::Paragraph(p) => (html_runs(ctx, &p.runs), true),
                    Block::ListItem(item) => (
                        format!("<ul><li>{}</li></ul>", html_runs(ctx, &item.runs)),
                        false,
                    ),
                    Block::Table(inner) => (html_table(ctx, inner).trim_end().to_string(), false),
                };
                if html.is_empty() {
                    continue;
                }
                if previous_text && is_text {
                    out.push_str("<br>");
                }
                out.push_str(&html);
                previous_text = is_text;
            }
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>");
    out
}

fn html_runs(ctx: &Ctx<'_>, runs: &[Run]) -> String {
    let mut out = String::new();
    for run in runs {
        if let Some(number) = run.style.footnote {
            out.push_str(&format!("[^{number}]"));
            continue;
        }
        let mut text = escape_html(run.text.trim_matches(|c| c == '\n')).replace('\n', "<br>");
        if text.trim().is_empty() {
            out.push_str(&text);
            continue;
        }
        if run.style.strike {
            text = format!("<del>{text}</del>");
        }
        if run.style.italic {
            text = format!("<em>{text}</em>");
        }
        if run.style.bold {
            text = format!("<strong>{text}</strong>");
        }
        if let Some(target) = run.style.link.and_then(|i| ctx.links.get(i)) {
            text = format!("<a href=\"{}\">{text}</a>", escape_html(&target.target));
        }
        out.push_str(&text);
    }
    out.trim().to_string()
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn markdown_cell(ctx: &Ctx<'_>, cell: &Cell) -> String {
    let parts: Vec<String> = cell
        .blocks
        .iter()
        .map(|b| markdown_cell_block(ctx, b))
        .filter(|p| !p.is_empty())
        .collect();
    parts.join("<br>")
}

fn markdown_cell_block(ctx: &Ctx<'_>, block: &Block) -> String {
    match block {
        Block::Paragraph(p) => markdown_runs(ctx, &p.runs).trim().replace("\\\n", "<br>"),
        Block::ListItem(item) => {
            let marker = if item.ordered { "1." } else { "-" };
            format!(
                "{marker} {}",
                markdown_runs(ctx, &item.runs)
                    .trim()
                    .replace("\\\n", "<br>")
            )
        }
        // Markdown tables cannot nest, so a nested table is flattened: rows separated by `<br>`,
        // cells by ` / `.
        Block::Table(table) => table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| markdown_cell(ctx, c))
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .collect::<Vec<_>>()
            .join("<br>"),
    }
}

/// Renders runs. Adjacent runs with the same formatting and link share one set of markers,
/// and whitespace stays outside them.
fn markdown_runs(ctx: &Ctx<'_>, runs: &[Run]) -> String {
    markdown_runs_with(ctx, runs, true)
}

fn markdown_runs_with(ctx: &Ctx<'_>, runs: &[Run], keep_bold: bool) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < runs.len() {
        let run = &runs[index];
        if let Some(number) = run.style.footnote {
            out.push_str(&format!("[^{number}]"));
            index += 1;
            continue;
        }
        let style = &run.style;
        let bold = keep_bold && style.bold;
        let key = (bold, style.italic, style.strike, style.link);
        let mut text = String::new();
        let mut end = index;
        while end < runs.len() {
            let other = &runs[end].style;
            let other_bold = keep_bold && other.bold;
            if other.footnote.is_some()
                || (other_bold, other.italic, other.strike, other.link) != key
            {
                break;
            }
            text.push_str(&runs[end].text);
            end += 1;
        }
        out.push_str(&styled(ctx, &text, key));
        index = end;
    }
    out
}

type Key = (bool, bool, bool, Option<usize>);

fn styled(ctx: &Ctx<'_>, text: &str, key: Key) -> String {
    let (bold, italic, strike, link) = key;
    let core_start = text.len() - text.trim_start().len();
    let core_end = text.trim_end().len();
    if core_start >= core_end {
        return escape_markdown(text);
    }
    let lead = escape_markdown(&text[..core_start]);
    let trail = escape_markdown(&text[core_end..]);
    let mut wrapped = escape_markdown(&text[core_start..core_end]);
    if strike {
        wrapped = format!("~~{wrapped}~~");
    }
    match (bold, italic) {
        (true, true) => wrapped = format!("***{wrapped}***"),
        (true, false) => wrapped = format!("**{wrapped}**"),
        (false, true) => wrapped = format!("*{wrapped}*"),
        (false, false) => {}
    }
    if let Some(target) = link.and_then(|i| ctx.links.get(i)) {
        wrapped = format!("[{wrapped}]({})", link_destination(&target.target));
    }
    format!("{lead}{wrapped}{trail}")
}

/// A link destination with spaces or parentheses goes in angle brackets.
fn link_destination(url: &str) -> String {
    if url
        .chars()
        .any(|c| matches!(c, ' ' | '(' | ')' | '<' | '>'))
    {
        let escaped = url
            .replace('<', "%3C")
            .replace('>', "%3E")
            .replace(' ', "%20");
        format!("<{}>", escaped.replace('(', "%28").replace(')', "%29"))
    } else {
        url.to_string()
    }
}

/// Escapes characters that Markdown would read as markup. Newlines become hard breaks.
pub(super) fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push_str("\\\n"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' => {
                out.push('\\');
                out.push(c);
            }
            '&' if chars
                .peek()
                .is_some_and(|n| n.is_alphanumeric() || *n == '#') =>
            {
                out.push_str("\\&");
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_characters() {
        assert_eq!(escape_markdown("a*b_c[d]"), "a\\*b\\_c\\[d\\]");
        assert_eq!(escape_markdown("x\ny"), "x\\\ny");
        assert_eq!(escape_markdown("A & B"), "A & B");
        assert_eq!(escape_markdown("&amp;"), "\\&amp;");
    }

    #[test]
    fn line_start_escapes() {
        assert_eq!(escape_line_start("# not a heading"), "\\# not a heading");
        assert_eq!(escape_line_start("- item"), "\\- item");
        assert_eq!(escape_line_start("-x"), "-x");
        assert_eq!(escape_line_start("2. two"), "2\\. two");
        assert_eq!(escape_line_start("2019 was"), "2019 was");
    }

    #[test]
    fn whitespace_stays_outside_emphasis() {
        let ctx = Ctx {
            options: &Options::default(),
            links: &[],
        };
        assert_eq!(
            styled(&ctx, "bold ", (true, false, false, None)),
            "**bold** "
        );
        assert_eq!(styled(&ctx, " x ", (false, true, false, None)), " *x* ");
    }

    #[test]
    fn link_destinations_with_spaces_are_bracketed() {
        assert_eq!(link_destination("https://a.test/x"), "https://a.test/x");
        assert_eq!(link_destination("a b"), "<a%20b>");
    }
}
