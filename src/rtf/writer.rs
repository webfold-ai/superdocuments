//! RTF writer. Output is ASCII only: cp1252 characters are written as `\'hh`, other BMP
//! characters as `\uN?` (with `\uc1`), and characters beyond the BMP as surrogate pairs.
//!
//! The writer does not emit headers, footers or unreferenced footnotes, and it drops
//! characters below U+0020 other than tab and newline.

use super::model::{
    Alignment, Block, Footnote, ListItem, Metadata, Paragraph, RtfDocument, Run, Table,
};
use super::reader::large_bold_level;
use std::fmt::Write as _;

const PREAMBLE: &str = "{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1\\deflang1033\n\
{\\fonttbl{\\f0\\fswiss\\fcharset0 Arial;}}\n";

/// Font sizes (half-points) for heading styles 1 to 6.
const HEADING_SIZES: [u32; 6] = [36, 32, 28, 26, 24, 24];

/// Serialises a document as RTF.
pub(super) fn write(doc: &RtfDocument) -> Vec<u8> {
    let mut w = Writer {
        out: String::with_capacity(4096),
        doc,
    };
    w.out.push_str(PREAMBLE);
    w.stylesheet();
    w.info(&doc.metadata);
    for block in &doc.blocks {
        w.block(block, 0);
    }
    w.out.push_str("}\n");
    w.out.into_bytes()
}

struct Writer<'a> {
    out: String,
    doc: &'a RtfDocument,
}

impl Writer<'_> {
    fn stylesheet(&mut self) {
        self.out.push_str("{\\stylesheet{\\s0 Normal;}");
        for (index, size) in HEADING_SIZES.iter().enumerate() {
            let level = index + 1;
            let _ = write!(
                self.out,
                "{{\\s{level}\\outlinelevel{index}\\sbasedon0\\snext0\\b\\fs{size} heading {level};}}"
            );
        }
        self.out.push_str("}\n");
    }

    fn info(&mut self, md: &Metadata) {
        let fields = [
            ("title", &md.title),
            ("subject", &md.subject),
            ("author", &md.author),
            ("keywords", &md.keywords),
            ("doccomm", &md.comments),
        ];
        let has_dates = parse_date(&md.created).is_some() || parse_date(&md.modified).is_some();
        if fields.iter().all(|(_, v)| v.is_empty()) && md.generator.is_empty() && !has_dates {
            return;
        }
        self.out.push_str("{\\info");
        for (name, value) in fields {
            if !value.is_empty() {
                let _ = write!(self.out, "{{\\{name} ");
                self.text(value);
                self.out.push('}');
            }
        }
        if !md.generator.is_empty() {
            self.out.push_str("{\\*\\generator ");
            self.text(&md.generator);
            self.out.push_str(";}");
        }
        for (name, value) in [("creatim", &md.created), ("revtim", &md.modified)] {
            if let Some([yr, mo, dy, hr, min]) = parse_date(value) {
                let _ = write!(
                    self.out,
                    "{{\\{name}\\yr{yr}\\mo{mo}\\dy{dy}\\hr{hr}\\min{min}}}"
                );
            }
        }
        self.out.push('}');
    }

    /// `depth` is zero outside tables, and the table nesting level inside a cell.
    fn block(&mut self, block: &Block, depth: usize) {
        match block {
            Block::Paragraph(p) => self.paragraph(p, depth),
            Block::ListItem(item) => self.list_item(item, depth),
            Block::Table(table) => self.table(table, depth),
        }
    }

    fn para_start(&mut self, depth: usize) {
        self.out.push_str("\\pard ");
        if depth > 0 {
            let _ = write!(self.out, "\\intbl\\itap{depth} ");
        }
    }

    fn paragraph(&mut self, p: &Paragraph, depth: usize) {
        self.para_start(depth);
        match p.heading_level {
            Some(level) => {
                let _ = write!(self.out, "\\s{} ", level.clamp(1, 6));
            }
            // Outline level 9 is body text. Without it, a large bold paragraph would be read
            // back as a heading by the size heuristic.
            None if large_bold_level(&p.runs).is_some() => self.out.push_str("\\outlinelevel9 "),
            None => {}
        }
        match p.alignment {
            Alignment::Left => {}
            Alignment::Center => self.out.push_str("\\qc "),
            Alignment::Right => self.out.push_str("\\qr "),
            Alignment::Justify => self.out.push_str("\\qj "),
        }
        if p.left_indent_twips != 0 {
            let _ = write!(self.out, "\\li{} ", p.left_indent_twips);
        }
        if p.first_line_indent_twips != 0 {
            let _ = write!(self.out, "\\fi{} ", p.first_line_indent_twips);
        }
        self.runs(&p.runs);
        self.out.push_str("\\par\n");
    }

    fn list_item(&mut self, item: &ListItem, depth: usize) {
        self.para_start(depth);
        let _ = write!(self.out, "\\ls1 \\ilvl{} ", item.level);
        let marker = if !item.marker.is_empty() {
            item.marker.clone()
        } else if item.ordered {
            "1.".to_string()
        } else {
            "\u{2022}".to_string()
        };
        self.out.push_str("{\\listtext ");
        self.text(&marker);
        self.out.push_str("\\tab }");
        self.runs(&item.runs);
        self.out.push_str("\\par\n");
    }

    /// Rows are closed with `\row` at the top level and `\nestrow` inside a cell.
    fn table(&mut self, table: &Table, depth: usize) {
        for row in &table.rows {
            self.out.push_str("\\trowd ");
            if row.header {
                self.out.push_str("\\trhdr ");
            }
            for index in 0..row.cells.len() {
                let _ = write!(self.out, "\\cellx{} ", (index + 1) * 1000);
            }
            for cell in &row.cells {
                for block in &cell.blocks {
                    self.block(block, depth + 1);
                }
                self.out.push_str(if depth == 0 {
                    "\\cell\n"
                } else {
                    "\\nestcell\n"
                });
            }
            self.out
                .push_str(if depth == 0 { "\\row\n" } else { "\\nestrow\n" });
        }
    }

    fn runs(&mut self, runs: &[Run]) {
        // Borrow the document through a copied reference so that `self` stays mutable below.
        let doc = self.doc;
        for run in runs {
            if let Some(number) = run.style.footnote {
                if let Some(note) = doc.footnotes.iter().find(|f| f.number == number) {
                    self.write_footnote(note);
                    continue;
                }
            }
            if run.text.is_empty() {
                continue;
            }
            let style = &run.style;
            self.out.push('{');
            if style.bold {
                self.out.push_str("\\b ");
            }
            if style.italic {
                self.out.push_str("\\i ");
            }
            if style.underline {
                self.out.push_str("\\ul ");
            }
            if style.strike {
                self.out.push_str("\\strike ");
            }
            if style.size_half_points > 0 {
                let _ = write!(self.out, "\\fs{} ", style.size_half_points);
            }
            let link = style.link.and_then(|i| doc.hyperlinks.get(i));
            match link {
                Some(target) => {
                    let url = target.target.replace('"', "%22");
                    self.out.push_str("{\\field{\\*\\fldinst{HYPERLINK \"");
                    self.text(&url);
                    self.out.push_str("\"}}{\\fldrslt ");
                    self.text(&run.text);
                    self.out.push_str("}}");
                }
                None => self.text(&run.text),
            }
            self.out.push('}');
        }
    }

    fn write_footnote(&mut self, note: &Footnote) {
        self.out
            .push_str("{\\super\\chftn}{\\footnote\\pard\\plain\\chftn ");
        for block in &note.blocks {
            self.block(block, 0);
        }
        self.out.push('}');
    }

    /// Writes text with RTF escapes. Only ASCII is produced.
    fn text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\\' => self.out.push_str("\\\\"),
                '{' => self.out.push_str("\\{"),
                '}' => self.out.push_str("\\}"),
                '\t' => self.out.push_str("\\tab "),
                '\n' => self.out.push_str("\\line "),
                '\r' => {}
                c if (c as u32) < 0x20 => {}
                c if c.is_ascii() => self.out.push(c),
                c => self.non_ascii(c),
            }
        }
    }

    fn non_ascii(&mut self, c: char) {
        let code = c as u32;
        let byte = if (0xA0..=0xFF).contains(&code) {
            Some(code as u8)
        } else {
            cp1252_byte(c)
        };
        match byte {
            Some(b) => {
                let _ = write!(self.out, "\\'{b:02x}");
            }
            None => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(self.out, "\\u{}?", *unit as i16);
                }
            }
        }
    }
}

/// The cp1252 byte for a character outside Latin-1, if there is one.
fn cp1252_byte(c: char) -> Option<u8> {
    let text = c.to_string();
    let (bytes, _, had_errors) = encoding_rs::WINDOWS_1252.encode(&text);
    if had_errors || bytes.len() != 1 {
        None
    } else {
        Some(bytes[0])
    }
}

/// Parses `YYYY-MM-DDTHH:MM` into its numeric fields.
fn parse_date(value: &str) -> Option<[i32; 5]> {
    if value.len() < 16 {
        return None;
    }
    let year = value.get(0..4)?.parse().ok()?;
    let month = value.get(5..7)?.parse().ok()?;
    let day = value.get(8..10)?.parse().ok()?;
    let hour = value.get(11..13)?.parse().ok()?;
    let minute = value.get(14..16)?.parse().ok()?;
    if year <= 0 {
        return None;
    }
    Some([year, month, day, hour, minute])
}
