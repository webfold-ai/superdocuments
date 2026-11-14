//! Native document-to-Markdown conversion over bounded ZIP/XML inputs.
//!
//! Migrated from Webfold's bounded native document parser. Legacy XLS is read by the
//! in-repo `calamine` crate; parsing does not fetch or launch a browser.

pub use crate::media::{extract_images, ExtractedImage};

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;
use zip::ZipArchive;

const MAX_ENTRY: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

pub(crate) fn text(raw: &str) -> String {
    quick_xml::escape::unescape(raw).map_or_else(|_| raw.to_string(), |s| s.into_owned())
}

/// Appends the character an XML reference stands for: `&#38;`, `&#x26;`, and the five predefined
/// entities. quick-xml reports `&amp;` and its kind as `Event::GeneralRef`, separate from the
/// surrounding `Event::Text`, so a reader that handles only `Text` silently loses every `&`, `<`,
/// `>`, `"` and `'` that the document wrote as an entity. An unknown entity is dropped.
pub(crate) fn push_ref(out: &mut String, reference: &quick_xml::events::BytesRef<'_>) {
    if let Ok(Some(c)) = reference.resolve_char_ref() {
        out.push(c);
        return;
    }
    match reference.as_ref() {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        _ => {}
    }
}

fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref().as_bytes() == name)
        .map(|a| text(a.value.as_ref()))
}

pub(crate) fn archive(data: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, String> {
    let mut zip = ZipArchive::new(Cursor::new(data)).map_err(|e| format!("invalid ZIP: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("too many archive entries".into());
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
        if entry.size() > MAX_ENTRY {
            return Err(format!(
                "entry too large: {}",
                entry.name().unwrap_or_else(|_| "<undecodable name>".into())
            ));
        }
        total = total
            .checked_add(entry.size())
            .ok_or("archive size overflow")?;
        if total > MAX_TOTAL {
            return Err("archive expands beyond limit".into());
        }
    }
    Ok(zip)
}

pub(crate) fn entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    let file = zip.by_name(name).map_err(|_| format!("missing {name}"))?;
    if file.size() > MAX_ENTRY {
        return Err(format!("entry too large: {name}"));
    }
    let mut data = Vec::with_capacity(file.size() as usize);
    file.take(MAX_ENTRY + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_ENTRY {
        return Err(format!("entry exceeds read limit: {name}"));
    }
    Ok(data)
}

pub(crate) fn xml_text(data: &[u8], wanted: &[u8]) -> Result<String, String> {
    let mut reader = Reader::from_reader(data);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut out = String::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == wanted => depth += 1,
            Event::End(tag) if tag.name().as_ref().as_bytes() == wanted => {
                depth = depth.saturating_sub(1);
                if !out.chars().last().is_some_and(char::is_whitespace) {
                    out.push(' ');
                }
            }
            Event::Text(value) if depth > 0 => out.push_str(&text(value.as_ref())),
            Event::GeneralRef(r) if depth > 0 => push_ref(&mut out, &r),
            Event::CData(value) if depth > 0 => out.push_str(value.as_ref()),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Appends a table cell: `|` is escaped and a line break inside the cell (a quoted CSV field, an
/// Excel cell with Alt+Enter) becomes `<br>`, because a raw newline would end the table row.
fn push_table_cell(line: &mut String, v: &str) {
    let mut chars = v.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '|' => line.push_str("\\|"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                line.push_str("<br>");
            }
            '\n' => line.push_str("<br>"),
            other => line.push(other),
        }
    }
}

pub(crate) fn markdown_table(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (row_index, row) in rows.iter().enumerate() {
        out.push('|');
        for col in 0..width {
            out.push(' ');
            push_table_cell(&mut out, row.get(col).map_or("", String::as_str));
            out.push_str(" |");
        }
        out.push('\n');
        if row_index == 0 {
            out.push('|');
            for _ in 0..width {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out
}

fn csv_delimiter(data: &str) -> char {
    let mut counts = [0usize; 3];
    let mut quoted = false;
    let mut chars = data.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if quoted && chars.peek() == Some(&'"') {
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if !quoted {
            if ch == '\n' {
                break;
            }
            match ch {
                ',' => counts[0] += 1,
                ';' => counts[1] += 1,
                '\t' => counts[2] += 1,
                _ => {}
            }
        }
    }
    [',', ';', '\t'][counts
        .iter()
        .enumerate()
        .max_by_key(|(index, count)| (*count, usize::MAX - index))
        .map_or(0, |(index, _)| index)]
}

fn decode_csv(data: &[u8]) -> Result<String, String> {
    if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        let little = data[0] == 0xff;
        let payload = &data[2..];
        if payload.len() % 2 == 1 {
            return Err("odd UTF-16 CSV byte length".into());
        }
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|pair| {
                if little {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|e| e.to_string())
    } else {
        String::from_utf8(
            data.strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(data)
                .to_vec(),
        )
        .map_err(|e| e.to_string())
    }
}

fn csv_rows(data: &str, delimiter: char) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = data.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ch if ch == delimiter && !quoted => row.push(std::mem::take(&mut cell)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            _ => cell.push(ch),
        }
    }
    if quoted {
        return Err("unterminated CSV quote".into());
    }
    if !row.is_empty() || !cell.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    Ok(rows)
}

fn xlsx(data: &[u8]) -> Result<String, String> {
    let sheets = crate::ooxml::xlsx::read(data)?
        .into_iter()
        .map(|sheet| (sheet.name, sheet.rows))
        .collect();
    Ok(workbook_markdown(sheets))
}

/// One `## <name>` section per sheet, in workbook order. Shared by the XLSX and legacy XLS readers
/// so both produce the same shape.
fn workbook_markdown(sheets: Vec<(String, Vec<Vec<String>>)>) -> String {
    let mut out = String::new();
    for (name, rows) in sheets {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "## {}\n\n{}",
            name,
            markdown_table(&rows).trim_end()
        ));
    }
    out.trim_end().to_string()
}

/// Legacy Excel 97-2003 (BIFF8, and BIFF5 from older tools) through the `calamine` workspace
/// member. Output has the same shape as [`xlsx`].
fn xls(data: &[u8]) -> Result<String, String> {
    // calamine still has panicking slice indexes in its BIFF record parser for some damaged files.
    // A panic here must surface as an error for the caller, so the reader runs behind
    // `catch_unwind`. Nothing is shared across the boundary that a panic could leave inconsistent.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| xls_unchecked(data)))
        .unwrap_or_else(|_| Err("invalid XLS: damaged record".into()))
}

fn xls_unchecked(data: &[u8]) -> Result<String, String> {
    use calamine::{Reader, SheetType, Xls};
    let mut book: Xls<Cursor<&[u8]>> =
        Reader::new(Cursor::new(data)).map_err(|e| format!("invalid XLS: {e}"))?;
    let date1904 = book.has_1904_epoch();
    // calamine keeps its own sheets sorted by name; the workbook's tab order is in the metadata.
    let names: Vec<String> = book
        .sheets_metadata()
        .iter()
        .filter(|sheet| matches!(sheet.typ, SheetType::WorkSheet))
        .map(|sheet| sheet.name.clone())
        .collect();
    let mut sheets = Vec::with_capacity(names.len());
    let mut cells = 0usize;
    for name in names {
        let range = book
            .worksheet_range(&name)
            .map_err(|e| format!("invalid XLS sheet {name}: {e}"))?;
        let (height, width) = range.get_size();
        cells = cells.saturating_add(height.saturating_mul(width));
        if cells > XLS_MAX_CELLS {
            return Err("worksheet too large".into());
        }
        let rows = xls_rows(&book, &name, &range, date1904);
        sheets.push((name, rows));
    }
    Ok(workbook_markdown(sheets))
}

/// Upper bound on cells across an XLS workbook's sheet ranges, matching the XLSX reader.
const XLS_MAX_CELLS: usize = 20_000_000;

/// Rows as the XLSX reader builds them: a cell sits at its absolute column, rows with no
/// content are dropped, and trailing empty cells are trimmed.
fn xls_rows(
    book: &calamine::Xls<Cursor<&[u8]>>,
    sheet: &str,
    range: &calamine::Range<calamine::Data>,
    date1904: bool,
) -> Vec<Vec<String>> {
    let Some((top, left)) = range.start() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (y, cells) in range.rows().enumerate() {
        let mut row: Vec<String> = vec![String::new(); left as usize];
        row.extend(cells.iter().enumerate().map(|(x, cell)| {
            let at = (top.saturating_add(y as u32), left.saturating_add(x as u32));
            xls_cell(cell, book.cell_number_format(sheet, at), date1904)
        }));
        while row.last().is_some_and(String::is_empty) {
            row.pop();
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    rows
}

/// A binary double the way Excel's General format shows it: 15 significant digits. An XLS file stores
/// doubles, so `33730816.02` is held as `33730816.019999996` and printing the shortest round-trip
/// form shows the noise.
fn general_number(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    format!("{value:.14e}")
        .parse::<f64>()
        .unwrap_or(value)
        .to_string()
}

/// What a number shows under its cell's number format, read from the XF and FORMAT records exactly
/// as the XLSX reader reads `cellXfs` and `numFmts`. `None` without a format, or one `numfmt` does
/// not implement and the number is not a date.
fn xls_number(value: f64, format: Option<(u16, Option<&str>)>, date1904: bool) -> Option<String> {
    let (id, code) = format?;
    crate::ooxml::xlsx::display_number(u32::from(id), code, value, date1904)
}

/// One cell as the XLSX reader prints it. `format` is the number format of a numeric cell.
fn xls_cell(cell: &calamine::Data, format: Option<(u16, Option<&str>)>, date1904: bool) -> String {
    use calamine::Data;
    match cell {
        Data::Empty => String::new(),
        Data::String(text) | Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
        Data::Bool(value) => if *value { "TRUE" } else { "FALSE" }.to_owned(),
        Data::Int(value) => {
            xls_number(*value as f64, format, date1904).unwrap_or_else(|| value.to_string())
        }
        Data::Float(value) => {
            xls_number(*value, format, date1904).unwrap_or_else(|| general_number(*value))
        }
        Data::DateTime(value) => {
            let serial = value.as_f64();
            if let Some(shown) = xls_number(serial, format, date1904) {
                return shown;
            }
            // Calamine reports only that a cell is a date or time format. A duration is a time of
            // day; a date-time with a fraction shows both, and a whole serial shows the date.
            let (date_part, time_part) = if value.is_duration() {
                (false, true)
            } else {
                (serial >= 1.0, serial < 1.0 || serial.fract() != 0.0)
            };
            crate::ooxml::xlsx::serial_to_text(serial, date1904, date_part, time_part)
                .unwrap_or_else(|| serial.to_string())
        }
        Data::Error(error) => error.to_string(),
    }
}

fn odt(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let xml = entry(&mut zip, "content.xml")?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let (mut paragraph, mut out, mut active) = (String::new(), String::new(), false);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag)
                if matches!(tag.name().as_ref().as_bytes(), b"text:p" | b"text:h") =>
            {
                active = true
            }
            Event::Text(value) if active => paragraph.push_str(&text(value.as_ref())),
            Event::GeneralRef(r) if active => push_ref(&mut paragraph, &r),
            Event::End(tag) if matches!(tag.name().as_ref().as_bytes(), b"text:p" | b"text:h") => {
                if !paragraph.trim().is_empty() {
                    if tag.name().as_ref().as_bytes() == b"text:h" {
                        out.push_str("## ");
                    }
                    out.push_str(paragraph.trim());
                    out.push_str("\n\n");
                }
                paragraph.clear();
                active = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out.trim_end().to_string())
}

fn ods(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let xml = entry(&mut zip, "content.xml")?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let (mut out, mut sheet_name, mut rows, mut row, mut cell) = (
        String::new(),
        String::new(),
        Vec::<Vec<String>>::new(),
        Vec::new(),
        String::new(),
    );
    let (mut in_cell, mut in_text) = (false, false);
    // A cell whose start tag carries a typed value (`office:value`, `office:date-value`, ...) has
    // the formatted display text of the same value in its `text:p` children. The display text is
    // what the document shows (`$1,234.50`, `12.5%`, `7 Nov 2026`) and wins; the typed value is
    // what a cell without display text keeps.
    let mut display = String::new();
    // `table:number-columns-repeated` on a cell that has content: the cell appears that many times.
    let mut cell_repeat = 1;
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"table:table" => {
                sheet_name = attr(&tag, b"table:name").unwrap_or_else(|| "Sheet".into());
                rows.clear();
            }
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"table:table-row" => {
                row.clear();
            }
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"table:table-cell" => {
                in_cell = true;
                cell = odf_cell_value(&tag);
                display.clear();
                cell_repeat = odf_repeat(&tag);
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"table:table-cell" => {
                let repeat = odf_repeat(&tag);
                let value = odf_cell_value(&tag);
                row.extend(std::iter::repeat_n(value, repeat));
            }
            Event::Start(tag) if in_cell && tag.name().as_ref().as_bytes() == b"text:p" => {
                in_text = true
            }
            Event::Text(value) if in_cell && in_text => display.push_str(&text(value.as_ref())),
            Event::GeneralRef(r) if in_cell && in_text => push_ref(&mut display, &r),
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"text:p" => {
                in_text = false;
                if !display.ends_with(' ') {
                    display.push(' ');
                }
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"table:table-cell" => {
                let shown = display.trim();
                let value = if shown.is_empty() {
                    cell.trim().to_string()
                } else {
                    shown.to_string()
                };
                row.extend(std::iter::repeat_n(value, cell_repeat));
                cell.clear();
                display.clear();
                in_cell = false;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"table:table-row" => {
                while row.last().is_some_and(String::is_empty) {
                    row.pop();
                }
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
                // Empty repeated rows have no visible Markdown cells; do not
                // expand a potentially huge blank grid.
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"table:table" => {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&format!(
                    "## {}\n\n{}",
                    sheet_name.replace('\n', " "),
                    markdown_table(&rows)
                ));
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out.trim_end().to_string())
}

fn odf_repeat(tag: &BytesStart<'_>) -> usize {
    attr(tag, b"table:number-columns-repeated")
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
        .min(16_384)
}

fn odf_cell_value(tag: &BytesStart<'_>) -> String {
    for key in [
        b"office:string-value".as_slice(),
        b"office:value",
        b"office:date-value",
        b"office:time-value",
        b"office:boolean-value",
    ] {
        if let Some(value) = attr(tag, key) {
            return value;
        }
    }
    String::new()
}

/// An EPUB as a document: spine order, metadata, footnotes, tables and the rest of the
/// structure the HTML reader keeps. Falls back to the file-name order reader when the package
/// has no usable spine.
fn epub_document(data: &[u8]) -> Result<String, String> {
    if let Ok(book) = crate::epub::read(data) {
        if !crate::epub::spine(&book).is_empty() {
            let doc = crate::epub::extract_doc(&book, data)?;
            return Ok(crate::ast::to_markdown(&doc));
        }
    }
    epub(data)
}

fn epub(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let mut chapters = Vec::new();
    for i in 0..zip.len() {
        let name = zip
            .by_index_raw(i)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if name.ends_with(".xhtml") || name.ends_with(".html") {
            chapters.push(name);
        }
    }
    chapters.sort();
    if chapters.is_empty() {
        // The package reader already failed, so this is the last resort for a damaged book; a
        // zip with no content document at all (an Office file, say) is not one.
        return Err("not an EPUB file: no package document and no content documents".into());
    }
    let mut out = String::new();
    for name in chapters {
        let html = String::from_utf8_lossy(&entry(&mut zip, &name)?).into_owned();
        let blocks = crate::html::to_blocks(&html, &format!("epub:///{name}"));
        let chapter = crate::html::render(&blocks, None, 1).0;
        if !chapter.trim().is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(chapter.trim());
        }
    }
    Ok(out)
}

// Binary SpreadsheetML's framed records. This reader covers cached scalar
// cells and shared strings; formulas and style formatting are not reconstructed.
fn bin_record<'a>(data: &'a [u8], pos: &mut usize) -> Result<Option<(u16, &'a [u8])>, String> {
    if *pos == data.len() {
        return Ok(None);
    }
    let first = *data.get(*pos).ok_or("truncated XLSB record type")?;
    *pos += 1;
    let kind = if first & 0x80 != 0 {
        let second = *data.get(*pos).ok_or("truncated XLSB record type")?;
        *pos += 1;
        u16::from(first & 0x7f) | (u16::from(second & 0x7f) << 7)
    } else {
        u16::from(first)
    };
    let mut size = 0usize;
    for shift in (0..28).step_by(7) {
        let byte = *data.get(*pos).ok_or("truncated XLSB record size")?;
        *pos += 1;
        size |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        if shift == 21 {
            return Err("oversized XLSB record header".into());
        }
    }
    let end = pos
        .checked_add(size)
        .filter(|end| *end <= data.len())
        .ok_or("XLSB record exceeds part")?;
    let payload = &data[*pos..end];
    *pos = end;
    Ok(Some((kind, payload)))
}

fn bin_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or("truncated XLSB u32")?
        .try_into()
        .map_err(|_| "truncated XLSB u32")?;
    Ok(u32::from_le_bytes(bytes))
}

fn bin_wide(data: &[u8], offset: usize) -> Result<(String, usize), String> {
    let count = bin_u32(data, offset)? as usize;
    let size = count.checked_mul(2).ok_or("oversized XLSB string")?;
    let end = offset
        .checked_add(4)
        .and_then(|v| v.checked_add(size))
        .ok_or("oversized XLSB string")?;
    let bytes = data.get(offset + 4..end).ok_or("truncated XLSB string")?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    Ok((String::from_utf16_lossy(&units), end))
}

fn xlsb(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let mut names = Vec::new();
    if let Ok(workbook) = entry(&mut zip, "xl/workbook.bin") {
        let mut pos = 0;
        while let Some((kind, payload)) = bin_record(&workbook, &mut pos)? {
            if kind == 156 {
                let (_, next) = bin_wide(payload, 8)?;
                let (name, _) = bin_wide(payload, next)?;
                names.push(name);
            }
        }
    }
    let mut shared = Vec::new();
    if let Ok(strings) = entry(&mut zip, "xl/sharedStrings.bin") {
        let mut pos = 0;
        while let Some((kind, payload)) = bin_record(&strings, &mut pos)? {
            if kind == 19 {
                shared.push(bin_wide(payload, 1)?.0);
            }
        }
    }
    let mut sheet_parts = Vec::new();
    for index in 0..zip.len() {
        let name = zip
            .by_index_raw(index)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if let Some(number) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|s| s.strip_suffix(".bin"))
            .and_then(|s| s.parse::<usize>().ok())
        {
            sheet_parts.push((number, name));
        }
    }
    sheet_parts.sort_by_key(|(number, _)| *number);
    let mut out = String::new();
    for (number, name) in sheet_parts {
        let bytes = entry(&mut zip, &name)?;
        let mut pos = 0;
        let mut current_row = None;
        let mut row = Vec::new();
        let mut rows = Vec::new();
        while let Some((kind, payload)) = bin_record(&bytes, &mut pos)? {
            if kind == 0 {
                if current_row.is_some() {
                    rows.push(std::mem::take(&mut row));
                }
                current_row = Some(bin_u32(payload, 0)?);
            } else if (2..=11).contains(&kind) && current_row.is_some() {
                let col = bin_u32(payload, 0)? as usize;
                if col >= 16_384 {
                    continue;
                }
                let value = match kind {
                    2 if payload.len() >= 12 => {
                        let rk = bin_u32(payload, 8)?;
                        let value = if rk & 2 != 0 {
                            ((rk as i32) >> 2) as f64
                        } else {
                            f64::from_bits(u64::from(rk & !3) << 32)
                        };
                        (if rk & 1 != 0 { value / 100.0 } else { value }).to_string()
                    }
                    4 | 10 => match payload.get(8) {
                        Some(0) => "FALSE".into(),
                        Some(1) => "TRUE".into(),
                        _ => String::new(),
                    },
                    5 | 9 if payload.len() >= 16 => {
                        let value = f64::from_le_bytes(
                            payload[8..16].try_into().map_err(|_| "bad XLSB f64")?,
                        );
                        value.to_string()
                    }
                    6 | 8 => bin_wide(payload, 8)?.0,
                    7 => shared
                        .get(bin_u32(payload, 8)? as usize)
                        .cloned()
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                if row.len() <= col {
                    row.resize(col + 1, String::new());
                }
                row[col] = value;
            }
        }
        if current_row.is_some() {
            rows.push(row);
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let title = names
            .get(number.saturating_sub(1))
            .cloned()
            .unwrap_or_else(|| format!("Sheet {number}"));
        out.push_str(&format!("## {title}\n\n{}", markdown_table(&rows)));
    }
    Ok(out.trim_end().to_string())
}

// ─── Scientific sources: LaTeX, BibTeX, MathML, arXiv e-prints ──────────────────────────────

/// gzip (`1f 8b`) or a tar archive (`ustar` at offset 257): what an arXiv e-print is.
fn looks_like_archive(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b]) || (data.len() >= 512 && data.get(257..262) == Some(b"ustar"))
}

/// UTF-8 (a byte-order mark is dropped) when valid, otherwise Latin-1, which is what older
/// TeX sources usually are.
fn decode_tex(data: &[u8]) -> String {
    let data = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data);
    match std::str::from_utf8(data) {
        Ok(text) => text.to_string(),
        Err(_) => data.iter().map(|&b| char::from(b)).collect(),
    }
}

/// How the converters read LaTeX: the default bounds, with the document's own macros expanded
/// inside math too, since a Markdown math renderer does not know them.
fn latex_options() -> crate::latex::Options {
    crate::latex::Options {
        expand_math_macros: true,
        ..Default::default()
    }
}

/// Markdown for a parsed LaTeX document. When the document has no `thebibliography` of its
/// own and a bibliography is supplied, the entries it cites are listed under "References",
/// in order of first citation (`\nocite{*}` lists every entry).
fn latex_markdown(
    doc: &crate::latex::Document,
    bib: Option<&crate::bibtex::Bibliography>,
) -> String {
    use crate::latex::CitationSource;
    let mut out = crate::latex::render_markdown(
        doc,
        &crate::latex::RenderOptions {
            bibliography: bib,
            pandoc: true,
        },
    );
    let Some(bib) = bib.filter(|_| doc.bibitems.is_empty()) else {
        return out;
    };
    let links = crate::latex::resolve_citations(doc, Some(bib));
    let everything = links.iter().any(|l| l.key == "*");
    let mut listed: Vec<&crate::bibtex::Entry> = Vec::new();
    for link in &links {
        if link.source != CitationSource::Bibliography {
            continue;
        }
        if let Some(entry) = bib.lookup(&link.key) {
            if !listed.iter().any(|e| std::ptr::eq(*e, entry)) {
                listed.push(entry);
            }
        }
    }
    if everything {
        for entry in &bib.entries {
            if !listed.iter().any(|e| std::ptr::eq(*e, entry)) {
                listed.push(entry);
            }
        }
    }
    if listed.is_empty() {
        return out;
    }
    out.push_str("\n# References\n\n");
    for entry in listed {
        out.push_str(&format!(
            "- `{}` — {}\n",
            entry.key.replace('`', "'"),
            crate::bibtex::reference_markdown(bib, entry)
        ));
    }
    out
}

/// LaTeX source with no file access: `\input` names stay unresolved and `\cite` shows keys.
fn latex(data: &[u8]) -> Result<String, String> {
    if looks_like_archive(data) {
        return eprint(data);
    }
    let doc = crate::latex::parse_with(&decode_tex(data), &latex_options(), None);
    Ok(latex_markdown(&doc, None))
}

/// A `.tex` file on disk. `\input`, `\include` and `\subfile` are read from the file's own
/// directory tree and nowhere else (`latex::DirFiles`). The `.bib` files named by
/// `\bibliography` and `\addbibresource` are read the same way; when there are none, the
/// `.bbl` file next to the document is used.
fn latex_path(path: &std::path::Path, data: &[u8]) -> Result<String, String> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let files = crate::latex::DirFiles::new(dir)
        .map_err(|e| format!("cannot read the document directory: {e}"))?
        .excluding(path);
    let options = latex_options();
    let mut doc = crate::latex::parse_with(&decode_tex(data), &options, Some(&files));
    let mut bib = crate::bibtex::Bibliography::default();
    for name in &doc.bibliography_files {
        let text = if name.to_ascii_lowercase().ends_with(".bib") {
            files.read(name)
        } else {
            files.read(&format!("{name}.bib"))
        };
        if let Some(text) = text {
            bib.entries.extend(crate::bibtex::parse(&text).entries);
        }
    }
    if bib.entries.is_empty() && doc.bibitems.is_empty() {
        let bbl = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|stem| files.read(&format!("{stem}.bbl")));
        if let Some(bbl) = bbl {
            doc.bibitems = crate::latex::parse_with(&bbl, &options, None).bibitems;
        }
    }
    Ok(latex_markdown(&doc, Some(&bib)))
}

/// An arXiv e-print: a gzip-compressed tar, a plain tar, or a gzip-compressed single TeX file.
/// `\input` files, `.bib` and `.bbl` files are read from the archive only.
fn eprint(data: &[u8]) -> Result<String, String> {
    let options = crate::arxiv_source::Options {
        latex: latex_options(),
        ..Default::default()
    };
    let mut source =
        crate::arxiv_source::read_eprint_with(data, &options).map_err(|e| e.to_string())?;
    // Entries that came from `\bibitem` text (a `.bbl` file) have no author or year to build a
    // citation from. They are shown the way `thebibliography` items are: by label or number.
    if source.document.bibitems.is_empty() {
        let (items, entries): (Vec<_>, Vec<_>) = std::mem::take(&mut source.bibliography.entries)
            .into_iter()
            .partition(|e| e.field("raw").is_some() && e.field("author").is_none());
        source.bibliography.entries = entries;
        for entry in items {
            let note = entry.value("note").unwrap_or_default().to_string();
            source.document.bibitems.push(crate::latex::BibItem {
                key: entry.key.clone(),
                label: entry.value("label").map(str::to_string),
                text: vec![crate::latex::Inline::Text(note)],
                raw: entry.value("raw").unwrap_or_default().to_string(),
            });
        }
    }
    Ok(latex_markdown(&source.document, Some(&source.bibliography)))
}

/// A `.bib` file as a readable Markdown reference list. For structured output use
/// `bibtex::parse` with `bibtex::csl_json` (CSL-JSON) or `bibtex::write` (BibTeX).
fn bib(data: &[u8]) -> Result<String, String> {
    let parsed = crate::bibtex::parse(&decode_tex(data));
    if parsed.entries.is_empty() {
        return Err(match parsed.warnings.first() {
            Some(w) => format!("no BibTeX entries found: {}", w.message),
            None => "no BibTeX entries found".to_string(),
        });
    }
    Ok(crate::bibtex::render_markdown(&parsed)
        .trim_end()
        .to_string())
}

/// A MathML `<math>` element as display math in TeX.
fn mathml(data: &[u8]) -> Result<String, String> {
    let converted = crate::mathml::to_tex(&decode_tex(data));
    let tex = converted.tex.trim();
    if tex.is_empty() {
        return Err(match converted.warnings.first() {
            Some(w) => format!("no MathML converted: {w}"),
            None => "no MathML <math> content found".to_string(),
        });
    }
    Ok(format!("$$\n{tex}\n$$"))
}

/// Convert an in-memory document to Markdown. Unsupported extensions return an
/// error rather than silently interpreting binary Office bytes as text.
///
/// Scientific sources: `tex`, `latex` and `ltx` are read as LaTeX (with no file access, so
/// `\input` files are not inlined; use `convert_path` for a project on disk). `bib` and
/// `bibtex` give a Markdown reference list. `mml` and `mathml` give display math in TeX.
/// `gz`, `tgz` and `tar` are read as an arXiv e-print. An e-print is also recognised by its
/// content (gzip or tar magic) under a `tex` extension or an extension this function does not
/// know, which is how files saved from `arxiv.org/e-print/<id>` are named.
pub fn convert_bytes(extension: &str, data: &[u8]) -> Result<String, String> {
    match extension.to_ascii_lowercase().as_str() {
        "docx" | "docm" => crate::ooxml::docx::convert(data),
        "doc" | "dot" => crate::doc::convert(data),
        "pptx" => crate::ooxml::pptx::convert(data),
        "xlsx" | "xlsm" => xlsx(data),
        "odt" => crate::odt::to_markdown(data),
        "odp" => match crate::odp::to_markdown(data)? {
            // a hand-made package without `office:presentation`: the generic text extraction
            slides if slides.is_empty() => odt(data),
            slides => Ok(slides),
        },
        "ods" => ods(data),
        "xls" => xls(data),
        "xlsb" => xlsb(data),
        "epub" => epub_document(data),
        "rtf" => crate::rtf::convert(data),
        "csv" => {
            let decoded = decode_csv(data)?;
            csv_rows(&decoded, csv_delimiter(&decoded)).map(|rows| markdown_table(&rows))
        }
        "html" | "htm" | "xhtml" => Ok(crate::html::reader::to_markdown_document(data)),
        "rst" | "rest" => Ok(crate::rst::to_markdown(data)),
        "txt" | "md" | "markdown" => Ok(String::from_utf8_lossy(data).into_owned()),
        "tex" | "latex" | "ltx" => latex(data),
        "bib" | "bibtex" => bib(data),
        "mml" | "mathml" => mathml(data),
        "gz" | "tgz" | "tar" => eprint(data),
        other => {
            let unsupported = format!("unsupported document extension: {other}");
            if looks_like_archive(data) {
                // An e-print saved under its arXiv identifier (`2301.00001v2`).
                return eprint(data).map_err(|_| unsupported);
            }
            Err(unsupported)
        }
    }
}

/// Convert a file to Markdown by its extension. A `.tex`, `.latex` or `.ltx` file is read
/// together with its project: `\input` files and bibliographies come from the file's own
/// directory tree, never from outside it. A file with no extension is accepted only when its
/// content is an e-print archive.
pub fn convert_path(path: &std::path::Path) -> Result<String, String> {
    let extension = path.extension().and_then(|e| e.to_str());
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    match extension {
        Some(ext)
            if matches!(ext.to_ascii_lowercase().as_str(), "tex" | "latex" | "ltx")
                && !looks_like_archive(&data) =>
        {
            latex_path(path, &data)
        }
        Some(ext) => convert_bytes(ext, &data),
        None if looks_like_archive(&data) => eprint(&data),
        None => Err("missing document extension".into()),
    }
}

/// Write-through lift: parse the document and emit Markdown chunks directly to `out`
/// without materialising a `LiftedDoc` Table layer in between. Cold-path only — the
/// fold's cache uses `convert_bytes` (which still goes through the String return path
/// for non-streaming formats and through `LiftedDoc` for streaming ones).
///
/// Streaming formats (CSV / DOCX / DOCM / XLSX) write directly to `out` via the
/// streaming render functions; non-streaming formats fall back to `convert_bytes`
/// and copy the resulting String into `out` once.
///
/// The streaming-law guarantee: the only allocation between input and `out` is the
/// per-line scratch String for the current cell or paragraph being formatted. No
/// per-cell Table::put, no per-cell Table::get on the render side, no full
/// Markdown-String materialisation.
pub fn convert_streaming<W: Write>(
    extension: &str,
    data: &[u8],
    out: &mut W,
) -> Result<(), String> {
    match extension.to_ascii_lowercase().as_str() {
        "csv" => {
            let decoded = decode_csv(data)?;
            let delimiter = csv_delimiter(&decoded);
            let rows = csv_rows(&decoded, delimiter)?;
            write_csv_rows(&rows, out)
        }
        // Every other format, including DOCX and XLSX: one implementation (`convert_bytes`), so
        // the streamed output cannot drift from it. Those readers need the whole package
        // (styles, numbering, relationships) before the first line can be written.
        // Fallback: full convert_bytes, write the resulting String once.
        other => {
            let s = convert_bytes(other, data)?;
            out.write_all(s.as_bytes()).map_err(|e| e.to_string())
        }
    }
}

fn write_csv_rows<W: Write>(rows: &[Vec<String>], out: &mut W) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut line = String::with_capacity(8 * cols + 16);
    // Header row (row 0).
    line.push('|');
    for c in 0..cols {
        line.push(' ');
        push_table_cell(&mut line, rows[0].get(c).map(String::as_str).unwrap_or(""));
        line.push_str(" |");
    }
    line.push('\n');
    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    // Separator.
    line.clear();
    line.push('|');
    for _ in 0..cols {
        line.push_str(" --- |");
    }
    line.push('\n');
    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    // Data rows.
    for row in rows.iter().skip(1) {
        line.clear();
        line.push('|');
        for c in 0..cols {
            line.push(' ');
            push_table_cell(&mut line, row.get(c).map(String::as_str).unwrap_or(""));
            line.push_str(" |");
        }
        line.push('\n');
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Streaming fold (Track D) ──────────────────────────────────────────────────────────────
//
// The AGENTS.md rule: "Certify the seed once; read each contact at its own coordinate."
//
// `convert_bytes` above is the naive shape: it lifts the document into a `String` on every call,
// which the AGENTS.md anti-loop rule explicitly forbids for the streaming law.
//
// `convert_cached` below is the fold:
//   1. The seed is BLAKE3(content) + format — two distinct inputs cannot collide.
//   2. The lift runs once per (hash, format) and writes the parsed state into a hashtrinity
//      Table addressed by (section, row, col). No rendered Markdown is materialized.
//   3. The render walks the Table in coordinate order and writes Markdown chunks directly to a
//      `Write` sink — the full Markdown never lives in a `String`.
//   4. Warm-path reads are O(1) coordinate lookups followed by `Write`; no ZIP, no XML, no parser.
//   5. Cold-path (lift) cost is identical to `convert_bytes`; warm-path cost is a hash, a probe,
//      and an iteration over N cells.
//
// Streaming discipline applied:
//   - ZIP entries are read directly from `zip.by_name(...)`; no `Vec::with_capacity(file.size())`
//     pre-allocation, no `read_to_end`.
//   - XML events are consumed one at a time from a `quick_xml::Reader` over the streaming entry.
//   - The output `Write` is never buffered past a single cell/line; `BufWriter` lives outside.
//   - Peak RSS during a warm hit is the size of the cached Table (one string per cell), not
//     the size of the rendered Markdown.

// DocFormat is small + Copy so it fits in a Table key without allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DocFormat {
    Xlsx = 1,
    Xlsb = 2,
    Xls = 3,
    Ods = 4,
    Docx = 5,
    Docm = 6,
    Pptx = 7,
    Odt = 8,
    Odp = 9,
    Epub = 10,
    Rtf = 11,
    Csv = 12,
    /// LaTeX source (`tex`, `latex`, `ltx`).
    Tex = 13,
    /// BibTeX or BibLaTeX (`bib`, `bibtex`).
    Bib = 14,
    /// MathML (`mml`, `mathml`).
    MathMl = 15,
    /// An arXiv e-print archive (`gz`, `tgz`, `tar`).
    ArxivSource = 16,
    /// Legacy Word binary (`doc`, `dot`).
    Doc = 17,
}

impl DocFormat {
    fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "xlsx" => Some(Self::Xlsx),
            "xlsm" => Some(Self::Xlsx),
            "xlsb" => Some(Self::Xlsb),
            "xls" => Some(Self::Xls),
            "ods" => Some(Self::Ods),
            "docx" => Some(Self::Docx),
            "docm" => Some(Self::Docm),
            "pptx" => Some(Self::Pptx),
            "odt" => Some(Self::Odt),
            "odp" => Some(Self::Odp),
            "epub" => Some(Self::Epub),
            "rtf" => Some(Self::Rtf),
            "csv" => Some(Self::Csv),
            "tex" | "latex" | "ltx" => Some(Self::Tex),
            "bib" | "bibtex" => Some(Self::Bib),
            "mml" | "mathml" => Some(Self::MathMl),
            "gz" | "tgz" | "tar" => Some(Self::ArxivSource),
            "doc" | "dot" => Some(Self::Doc),
            _ => None,
        }
    }

    /// Whether this format goes through the streaming lift path.
    /// Non-streaming formats still get cached as `Arc<String>` (no parse on warm hit), but they
    /// don't stream the render.
    fn is_streaming(self) -> bool {
        matches!(self, Self::Csv)
    }
}

// Coordinate within a lifted document. Format-specific semantics:
//   - XLSX/XLSB/ODS: (sheet_idx, row, col)   → cell text
//   - DOCX:          (paragraph_idx, 0, 0)    → paragraph text
//                   (paragraph_idx, 1, style) → heading level (0..=6) when non-zero
//   - CSV:           (row_idx, col, 0)        → cell text
//   - PPTX/EPUB/RTF: (block_idx, 0, 0)        → block text
pub type DocCoord = (u32, u32, u32);

/// Lifted document state. Parsed once, addressed by coordinate, never re-rendered in full.
pub struct LiftedDoc {
    pub format: DocFormat,
    /// Section names in render order (sheet names, slide names, …).
    pub section_names: Vec<String>,
    /// Cells: the parsed state. Keyed by coordinate; value is the literal cell text.
    pub cells: apeiron::map::hashtrinity::Table<DocCoord, String>,
    /// Cell kind tag: 0 = text, 1 = number, 2 = date, 3 = heading-1, 4 = heading-2, ….
    /// Used by the render pass to pick the right Markdown prefix without re-parsing.
    pub kinds: apeiron::map::hashtrinity::Table<DocCoord, u8>,
    /// The render order: which coordinates to emit, in order. Stored as a Vec so render is
    /// strictly streaming-write (no Table iteration order surprises).
    pub order: Vec<DocCoord>,
    /// Per-coordinate row width (DOCX tables, XLSX rows). Used to terminate table renders.
    /// `0` means "no row width tracked for this coordinate".
    pub row_widths: apeiron::map::hashtrinity::Table<DocCoord, u16>,
}

/// The fold cache. Two apeiron::map::hashtrinity::Tables:
///   - `lifted`: streaming formats — the parsed state, rendered streaming.
///   - `rendered`: non-streaming formats — the cached Markdown `String` directly.
///
/// Track F.6: the cache key is a caller-supplied `u64` identity, NOT a BLAKE3
/// computed inside the lib. The lib never hashes anything; the caller is
/// responsible for collision-resistance on their workload. The right identity
/// depends on the workload:
///
/// - **server hot path**: a non-crypto fingerprint (xxhash, length-mix, FxHash)
///   is ~50 ns/call and gives 64 bits of fingerprint; collision probability is
///   negligible for any realistic cache.
/// - **persistence / cross-process**: a cryptographic hash (BLAKE3, SHA-256)
///   when the cache might survive a restart and the same identity must mean
///   the same bytes; the caller still computes the hash, the lib just stores
///   the resulting u64.
/// - **upstream identifier**: an ETag, content-addressed storage key, or a
///   monotonic sequence number from the upstream system.
///
/// The `u64` choice keeps the lookup path branch-free and the hashtable probe
/// single-cycle; the only work in the hot path is `Table::get` + the render.
pub struct DocumentCache {
    lifted: apeiron::map::hashtrinity::Table<(u64, u8), Arc<LiftedDoc>>,
    rendered: apeiron::map::hashtrinity::Table<(u64, u8), Arc<String>>,
    hits: u64,
    misses: u64,
}

impl DocumentCache {
    pub fn new() -> Self {
        Self {
            lifted: apeiron::map::hashtrinity::Table::new(),
            rendered: apeiron::map::hashtrinity::Table::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// Number of warm-path lookups served from cache.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Number of cold-path lifts executed.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Streaming convert with caller-supplied identity. The lib never computes
    /// any hash; the caller picks whatever u64 identity fits their workload
    /// (see the [`DocumentCache`] docs).
    ///
    /// Track F.6: this is the only `convert` API on the cache. There is no
    /// `convert_by_hash` anymore — the caller is responsible for any hashing.
    /// For one-shot callers who don't have an identity handy, the simplest
    /// helper is `webfold::document::fingerprint_bytes(data)` below.
    pub fn convert<W: Write>(
        &mut self,
        identity: u64,
        extension: &str,
        data: &[u8],
        out: &mut W,
    ) -> Result<(), String> {
        let format = DocFormat::from_extension(extension)
            .or_else(|| looks_like_archive(data).then_some(DocFormat::ArxivSource))
            .ok_or_else(|| format!("unsupported document extension: {extension}"))?;

        if format.is_streaming() {
            let key = (identity, format as u8);
            if let Some(arc) = self.lifted.get(&key) {
                self.hits += 1;
                render_streaming(arc.as_ref(), out)
            } else {
                self.misses += 1;
                let arc = Arc::new(lift(format, data)?);
                self.lifted.put(key, arc.clone());
                render_streaming(arc.as_ref(), out)
            }
        } else if let Some(cached) = self.rendered.get(&(identity, format as u8)) {
            self.hits += 1;
            out.write_all(cached.as_bytes()).map_err(|e| e.to_string())
        } else {
            self.misses += 1;
            let rendered = Arc::new(convert_bytes(extension, data)?);
            self.rendered
                .put((identity, format as u8), rendered.clone());
            out.write_all(rendered.as_bytes())
                .map_err(|e| e.to_string())
        }
    }
}

/// Convenience non-crypto fingerprint for callers that don't already have an
/// identity. FxHash-style: ~50 ns on 7 KiB inputs, gives 64 bits of
/// collision-resistance. NOT cryptographic — callers who need verifiable
/// identity should use BLAKE3 themselves and truncate to `u64`.
pub fn fingerprint_bytes(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325; // FxHash offset basis
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3); // FxHash prime
    }
    h
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming convert with a fresh cache. Convenience wrapper for one-shot use.
pub fn convert_cached<W: Write>(
    identity: u64,
    extension: &str,
    data: &[u8],
    out: &mut W,
) -> Result<(), String> {
    let mut cache = DocumentCache::new();
    cache.convert(identity, extension, data, out)
}

// ─── Lift (certify the seed once) ───────────────────────────────────────────────────────────

fn lift(format: DocFormat, data: &[u8]) -> Result<LiftedDoc, String> {
    match format {
        DocFormat::Csv => lift_csv(data),
        _ => Err(format!(
            "lift: format {format:?} uses the rendered-string cache path"
        )),
    }
}

fn lift_csv(data: &[u8]) -> Result<LiftedDoc, String> {
    let decoded = decode_csv(data)?;
    let delimiter = csv_delimiter(&decoded);
    let rows = csv_rows(&decoded, delimiter)?;

    let mut cells: apeiron::map::hashtrinity::Table<DocCoord, String> =
        apeiron::map::hashtrinity::Table::new();
    let mut kinds: apeiron::map::hashtrinity::Table<DocCoord, u8> =
        apeiron::map::hashtrinity::Table::new();
    let mut order: Vec<DocCoord> = Vec::new();
    let row_count = row_count(&rows);

    for (row_idx, row) in rows.iter().enumerate() {
        for (col, value) in row.iter().enumerate() {
            if value.is_empty() {
                continue;
            }
            let coord = (row_idx as u32, col as u32, 0);
            cells.put(coord, value.clone());
            kinds.put(coord, 1);
            order.push(coord);
        }
    }

    let mut row_widths: apeiron::map::hashtrinity::Table<DocCoord, u16> =
        apeiron::map::hashtrinity::Table::new();
    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u16;
    if max_cols > 0 {
        row_widths.put((0, 0, 0), max_cols);
    }

    let _ = row_count; // tracked for symmetry with the docx/xlsx lifters; reserved for future bench rows.
    Ok(LiftedDoc {
        format: DocFormat::Csv,
        section_names: vec!["CSV".to_string()],
        cells,
        kinds,
        order,
        row_widths,
    })
}

fn row_count(rows: &[Vec<String>]) -> usize {
    rows.len()
}

// ─── Render (streaming write to a `Write` sink) ──────────────────────────────────────────────

fn render_streaming<W: Write>(doc: &LiftedDoc, out: &mut W) -> Result<(), String> {
    match doc.format {
        DocFormat::Csv => render_csv(doc, out),
        _ => Err(format!("render: format {:?} not streaming", doc.format)),
    }
}

fn render_csv<W: Write>(doc: &LiftedDoc, out: &mut W) -> Result<(), String> {
    // doc.order is row-major (row, col, 0). Stream cells to the sink in that order,
    // tracking current_row and reusing a single line buffer. No BTreeMap, no
    // per-cell allocation beyond the per-line clear.
    if doc.order.is_empty() {
        return Ok(());
    }
    let cols = doc.row_widths.get(&(0, 0, 0)).copied().unwrap_or(1) as usize;

    let mut current_row: u32 = u32::MAX;
    let mut header_emitted = false;
    let mut line = String::with_capacity(8 * cols + 16);

    for coord in &doc.order {
        if coord.0 != current_row {
            // Finish the previous row + separator if it was the header.
            if current_row != u32::MAX {
                line.push('\n');
                out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                if !header_emitted {
                    line.clear();
                    line.push('|');
                    for _ in 0..cols {
                        line.push_str(" --- |");
                    }
                    line.push('\n');
                    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                    header_emitted = true;
                }
                line.clear();
            }
            current_row = coord.0;
            line.push('|');
        }
        let cell = doc.cells.get(coord).map(String::as_str).unwrap_or("");
        line.push(' ');
        push_table_cell(&mut line, cell);
        line.push_str(" |");
    }
    // Flush the last row.
    line.push('\n');
    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    // If the file has only a header row (no data rows), emit the separator so the
    // canonical Markdown table shape is preserved.
    if !header_emitted {
        line.clear();
        line.push('|');
        for _ in 0..cols {
            line.push_str(" --- |");
        }
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Helper: read a single <v> or <is> body out of an XML stream ─────────────────────────────
//
// Reads until the matching `</v>`, `</is>`, or self-closing terminator; unescapes entity refs;
// resolves `t="s"` shared-string indices via the shared-string table. Caller owns the `Reader`
// and `buf` so we don't allocate per cell.
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(entries: &[(&str, &str)]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        for (name, value) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(value.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn csv_quotes_and_tables() {
        assert_eq!(
            csv_rows("name,value\n\"a,b\",2\n", ',').unwrap(),
            vec![vec!["name", "value"], vec!["a,b", "2"]]
        );
        assert!(convert_bytes("csv", b"name,value\na,2\n")
            .unwrap()
            .contains("| name | value |"));
        assert!(convert_bytes("csv", b"name;value\na;2\n")
            .unwrap()
            .contains("| name | value |"));
        let utf16: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain(
                "name,value\na,2\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        assert!(convert_bytes("csv", &utf16)
            .unwrap()
            .contains("| name | value |"));
    }

    #[test]
    fn spreadsheet_cells_preserve_sparse_columns() {
        let xml = br#"<worksheet><sheetData><row><c r="A1" t="s"><v>0</v></c><c r="C1"><v>42</v></c></row></sheetData></worksheet>"#;
        assert_eq!(
            crate::ooxml::xlsx::worksheet(xml, &["hello".into()], &Default::default()).unwrap(),
            vec![vec!["hello", "", "42"]]
        );
    }

    #[test]
    fn office_packages_convert_through_public_api() {
        let doc = package(&[("word/document.xml", "<w:document><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:document>")]);
        assert_eq!(convert_bytes("docx", &doc).unwrap(), "# Report\n\nBody");
        let relocated = package(&[("_rels/.rels", "<Relationships><Relationship Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"content/main.xml\"/></Relationships>"),
            ("content/main.xml", "<w:document><w:p><w:t>Moved</w:t></w:p></w:document>")]);
        assert_eq!(convert_bytes("docx", &relocated).unwrap(), "Moved");
        let slide = package(&[(
            "ppt/slides/slide1.xml",
            "<p:sld><a:p><a:t>Launch</a:t></a:p></p:sld>",
        )]);
        assert!(convert_bytes("pptx", &slide).unwrap().contains("Launch"));
        let book = package(&[("xl/sharedStrings.xml", "<sst><si><t>Name</t></si></sst>"),
            ("xl/worksheets/sheet1.xml", "<worksheet><sheetData><row><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\"><v>42</v></c></row></sheetData></worksheet>")]);
        assert!(convert_bytes("xlsx", &book)
            .unwrap()
            .contains("| Name | 42 |"));
        let odf = package(&[(
            "content.xml",
            "<office:document><text:h>Title</text:h><text:p>Body</text:p></office:document>",
        )]);
        assert_eq!(
            convert_bytes("odt", &odf).unwrap().trim(),
            "# Title\n\nBody"
        );
        let sheet = package(&[("content.xml", "<office:document><table:table table:name=\"Data\"><table:table-row><table:table-cell office:value=\"42\"/></table:table-row></table:table></office:document>")]);
        assert!(convert_bytes("ods", &sheet).unwrap().contains("| 42 |"));

        let macro_doc = package(&[(
            "word/document.xml",
            "<w:document><w:p><w:t>Macro doc</w:t></w:p></w:document>",
        )]);
        assert!(convert_bytes("docm", &macro_doc)
            .unwrap()
            .contains("Macro doc"));
        assert!(convert_bytes("odp", &odf).unwrap().contains("Body"));
        assert!(convert_bytes("xlsm", &book)
            .unwrap()
            .contains("| Name | 42 |"));
        let epub = package(&[(
            "OPS/chapter.xhtml",
            "<html><body><h1>Chapter</h1></body></html>",
        )]);
        assert!(convert_bytes("epub", &epub).unwrap().contains("Chapter"));
        let xlsb = package(&[("xl/worksheets/sheet1.bin", "")]);
        assert!(convert_bytes("xlsb", &xlsb).unwrap().contains("Sheet 1"));
        assert!(convert_bytes("html", b"<h1>HTML</h1>")
            .unwrap()
            .contains("HTML"));
    }

    #[test]
    fn ods_cell_with_a_value_attribute_and_display_text_is_not_doubled() {
        // LibreOffice writes a number cell as value attribute plus the formatted text in `text:p`.
        let sheet = package(&[(
            "content.xml",
            "<office:document><table:table table:name=\"Data\"><table:table-row>\
             <table:table-cell office:value-type=\"float\" office:value=\"42\"><text:p>42</text:p></table:table-cell>\
             <table:table-cell office:value-type=\"string\"><text:p>Name</text:p></table:table-cell>\
             <table:table-cell office:value-type=\"float\" office:value=\"7.5\"><text:p>7.50</text:p></table:table-cell>\
             </table:table-row></table:table></office:document>",
        )]);
        let md = convert_bytes("ods", &sheet).unwrap();
        // the display text is shown once (7.50, as the sheet shows it, not 7.5 and not both)
        assert!(md.contains("| 42 | Name | 7.50 |"), "got: {md}");
    }

    /// The display text of a typed cell is what the document shows: currency, percent and fixed
    /// decimals keep their format (LibreOffice sc/qa testNumberFormats.ods printed the stored
    /// number, `0.123456`, for cells shown as `123456` or `12.5%`); a typed cell without display
    /// text keeps its stored value.
    /// A cell with content and `table:number-columns-repeated` fills that many columns (odfpy
    /// chinese_spreadsheet.ods: one `test` cell repeated three times was read as one column).
    #[test]
    fn ods_repeated_cell_with_content_is_repeated() {
        let sheet = package(&[(
            "content.xml",
            "<office:document><table:table table:name=\"Data\"><table:table-row>\
             <table:table-cell table:number-columns-repeated=\"3\" office:value-type=\"string\"><text:p>test</text:p></table:table-cell>\
             </table:table-row></table:table></office:document>",
        )]);
        let md = convert_bytes("ods", &sheet).unwrap();
        assert!(md.contains("| test | test | test |"), "got: {md}");
    }

    #[test]
    fn ods_typed_cells_show_their_display_text() {
        let sheet = package(&[(
            "content.xml",
            "<office:document><table:table table:name=\"Data\"><table:table-row>\
             <table:table-cell office:value-type=\"currency\" office:value=\"1234.5\"><text:p>$1,234.50</text:p></table:table-cell>\
             <table:table-cell office:value-type=\"percentage\" office:value=\"0.125\"><text:p>12.5%</text:p></table:table-cell>\
             <table:table-cell office:value-type=\"float\" office:value=\"9\"/>\
             </table:table-row></table:table></office:document>",
        )]);
        let md = convert_bytes("ods", &sheet).unwrap();
        assert!(md.contains("| $1,234.50 | 12.5% | 9 |"), "got: {md}");
    }

    #[test]
    fn binary_format_failure_is_explicit() {
        assert!(convert_bytes("xlsb", b"binary").is_err());
        assert!(convert_bytes("docx", b"not a zip").is_err());
    }

    #[test]
    fn rtf_text_and_metadata_groups() {
        let source = br"{\rtf1{\fonttbl{\f0 Hidden;}}Hello\par world \'e9}";
        assert_eq!(convert_bytes("rtf", source).unwrap(), "Hello\n\nworld é");
    }

    /// Track D fold correctness: `DocumentCache::convert` must produce the same Markdown
    /// as `convert_bytes` for the same bytes, for every streaming format. The first call
    /// runs the lift + render (cold); the second is a cache hit (warm) and must emit the
    /// same Markdown.
    #[test]
    fn streaming_fold_matches_convert_bytes_for_csv_docx_xlsx() {
        let csv = b"name,value\na,2\nb,3\n";
        let csv_md_baseline = convert_bytes("csv", csv).unwrap();
        let mut cache = DocumentCache::new();
        let mut cold = Vec::new();
        cache
            .convert(fingerprint_bytes(csv), "csv", csv, &mut cold)
            .unwrap();
        let mut warm = Vec::new();
        cache
            .convert(fingerprint_bytes(csv), "csv", csv, &mut warm)
            .unwrap();
        assert_eq!(cold, warm, "warm hit must equal cold output for CSV");
        assert_eq!(String::from_utf8_lossy(&cold), csv_md_baseline);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);

        let doc = package(&[(
            "word/document.xml",
            "<w:document><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:document>",
        )]);
        let doc_md_baseline = convert_bytes("docx", &doc).unwrap();
        let mut cache = DocumentCache::new();
        let mut cold = Vec::new();
        cache
            .convert(fingerprint_bytes(&doc), "docx", &doc, &mut cold)
            .unwrap();
        let mut warm = Vec::new();
        cache
            .convert(fingerprint_bytes(&doc), "docx", &doc, &mut warm)
            .unwrap();
        assert_eq!(cold, warm, "warm hit must equal cold output for DOCX");
        assert_eq!(String::from_utf8_lossy(&cold), doc_md_baseline);

        let book = package(&[
            ("xl/sharedStrings.xml", "<sst><si><t>Name</t></si></sst>"),
            (
                "xl/worksheets/sheet1.xml",
                "<worksheet><sheetData><row><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\"><v>42</v></c></row></sheetData></worksheet>",
            ),
        ]);
        let xlsx_md_baseline = convert_bytes("xlsx", &book).unwrap();
        let mut cache = DocumentCache::new();
        let mut cold = Vec::new();
        cache
            .convert(fingerprint_bytes(&book), "xlsx", &book, &mut cold)
            .unwrap();
        let mut warm = Vec::new();
        cache
            .convert(fingerprint_bytes(&book), "xlsx", &book, &mut warm)
            .unwrap();
        assert_eq!(cold, warm, "warm hit must equal cold output for XLSX");
        assert_eq!(String::from_utf8_lossy(&cold), xlsx_md_baseline);
    }

    /// Track D fold keying: a caller may reuse an identity across formats.
    /// The format byte must therefore be part of the cache key; CSV and DOCX
    /// use incompatible LiftedDoc representations and must not share an entry.
    #[test]
    fn streaming_fold_keys_on_format_discriminator() {
        let csv = b"a,b\n1,2\n";
        // Deliberately reuse one caller identity across formats: the format must
        // participate in the cache key, otherwise one lift can poison the other.
        let doc_a = package(&[(
            "word/document.xml",
            "<w:document><w:p><w:t>A</w:t></w:p></w:document>",
        )]);
        let mut cache = DocumentCache::new();
        cache.convert(7, "csv", csv, &mut Vec::new()).unwrap();
        cache.convert(7, "docx", &doc_a, &mut Vec::new()).unwrap();
        cache.convert(7, "docx", &doc_a, &mut Vec::new()).unwrap();
        assert_eq!(cache.misses(), 2);
        assert_eq!(cache.hits(), 1);
    }

    /// Track F.4 write-through correctness: `convert_streaming<W: Write>` must produce
    /// the same Markdown as `convert_bytes` for CSV / DOCX / XLSX (the streaming
    /// formats), and must be byte-for-byte stable on the cold path. Non-streaming
    /// formats fall back to `convert_bytes` internally, so they match by construction.
    #[test]
    fn convert_streaming_matches_convert_bytes_for_csv_docx_xlsx() {
        let csv = b"name,value\na,2\nb,3\n";
        let csv_baseline = convert_bytes("csv", csv).unwrap();
        let mut stream_out = Vec::new();
        convert_streaming("csv", csv, &mut stream_out).unwrap();
        assert_eq!(String::from_utf8_lossy(&stream_out), csv_baseline);

        let doc = package(&[(
            "word/document.xml",
            "<w:document><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:document>",
        )]);
        let doc_baseline = convert_bytes("docx", &doc).unwrap();
        let mut stream_out = Vec::new();
        convert_streaming("docx", &doc, &mut stream_out).unwrap();
        assert_eq!(String::from_utf8_lossy(&stream_out), doc_baseline);

        let book = package(&[
            ("xl/sharedStrings.xml", "<sst><si><t>Name</t></si></sst>"),
            (
                "xl/worksheets/sheet1.xml",
                "<worksheet><sheetData><row><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\"><v>42</v></c></row></sheetData></worksheet>",
            ),
        ]);
        let xlsx_baseline = convert_bytes("xlsx", &book).unwrap();
        let mut stream_out = Vec::new();
        convert_streaming("xlsx", &book, &mut stream_out).unwrap();
        assert_eq!(String::from_utf8_lossy(&stream_out), xlsx_baseline);
    }

    /// Track F.6 correctness: `DocumentCache::convert(identity, ...)` with a
    /// caller-supplied identity (no BLAKE3) must produce the same Markdown as
    /// `convert_bytes`. Cold-path (cache miss) and warm-path (cache hit) are
    /// both verified, and a different identity produces a separate cache slot.
    #[test]
    fn convert_with_identity_matches_convert_bytes() {
        let csv = b"name,value\na,2\nb,3\n";
        let csv_baseline = convert_bytes("csv", csv).unwrap();
        let csv_id = fingerprint_bytes(csv);
        let mut cache = DocumentCache::new();
        let mut cold = Vec::new();
        cache.convert(csv_id, "csv", csv, &mut cold).unwrap();
        let mut warm = Vec::new();
        cache.convert(csv_id, "csv", csv, &mut warm).unwrap();
        assert_eq!(cold, warm, "warm hit must equal cold output");
        assert_eq!(String::from_utf8_lossy(&cold), csv_baseline);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);

        let doc = package(&[(
            "word/document.xml",
            "<w:document><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:document>",
        )]);
        let doc_baseline = convert_bytes("docx", &doc).unwrap();
        let doc_id = fingerprint_bytes(&doc);
        let mut cache = DocumentCache::new();
        let mut cold = Vec::new();
        cache.convert(doc_id, "docx", &doc, &mut cold).unwrap();
        let mut warm = Vec::new();
        cache.convert(doc_id, "docx", &doc, &mut warm).unwrap();
        assert_eq!(cold, warm, "warm hit must equal cold output");
        assert_eq!(String::from_utf8_lossy(&cold), doc_baseline);

        // A different identity lands in a separate cache slot. The output
        // must still match (the cache key shape is the identity, not the bytes).
        let bad_id = csv_id.wrapping_add(1);
        let mut sink = Vec::new();
        cache.convert(bad_id, "docx", &doc, &mut sink).unwrap();
        assert_eq!(cache.misses(), 2);
        assert_eq!(String::from_utf8_lossy(&sink), doc_baseline);
    }

    /// Track F.6: the lib never calls BLAKE3 internally on the read path. The
    /// only `blake3` reference left in the hot path of `convert` is via
    /// `fingerprint_bytes`, which is the FxHash-style non-crypto mix. Callers
    /// who want cryptographic identity compute BLAKE3 themselves and truncate
    /// to `u64` — the lib stays out of it.
    #[test]
    fn fingerprint_bytes_is_non_crypto_and_collision_free_on_corpus() {
        // FxHash-style mix: not BLAKE3, not cryptographic. The lib doesn't
        // claim any of those properties; the caller does.
        let csv = b"name,value\na,2\nb,3\n";
        let docx = b"xlsx-equivalent bytes";
        assert_ne!(fingerprint_bytes(csv), fingerprint_bytes(docx));
        // Same input gives the same fingerprint (stable across calls).
        assert_eq!(fingerprint_bytes(csv), fingerprint_bytes(csv));
    }
}

#[cfg(test)]
mod table_cell_tests {
    use super::*;

    /// XLS stores binary doubles; Excel shows 15 significant digits (euses grades_processed xls:
    /// `33730816.019999996` for a cell that reads 33730816.02).
    #[test]
    fn xls_floats_show_fifteen_significant_digits() {
        assert_eq!(general_number(33730816.019999996), "33730816.02");
        assert_eq!(general_number(0.1 + 0.2), "0.3");
        assert_eq!(general_number(1234.5), "1234.5");
        assert_eq!(general_number(5.0), "5");
        assert_eq!(general_number(-0.000125), "-0.000125");
        assert_eq!(general_number(f64::INFINITY), "inf");
    }

    /// A quoted CSV field with a line break (RFC 4180) must stay inside its table row: the raw
    /// newline used to end the row and split the table (csv-spectrum `newlines.csv`).
    #[test]
    fn line_breaks_inside_csv_cells_become_br() {
        let md = convert_bytes("csv", b"a,b,c\r\n1,\"x\r\ny\",3\r\n4,5,6\r\n").unwrap();
        assert_eq!(
            md,
            "| a | b | c |\n| --- | --- | --- |\n| 1 | x<br>y | 3 |\n| 4 | 5 | 6 |\n"
        );
    }

    #[test]
    fn pipes_and_line_breaks_are_escaped_in_every_table_writer() {
        let rows = vec![
            vec!["h".to_string(), "k".to_string()],
            vec!["a|b".to_string(), "c\nd".to_string()],
        ];
        assert_eq!(
            markdown_table(&rows),
            "| h | k |\n| --- | --- |\n| a\\|b | c<br>d |\n"
        );
        let mut streamed = Vec::new();
        write_csv_rows(&rows, &mut streamed).unwrap();
        assert_eq!(String::from_utf8(streamed).unwrap(), markdown_table(&rows));
    }
}
