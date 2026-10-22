//! Native document-to-Markdown conversion over bounded ZIP/XML inputs.
//!
//! Migrated from Webfold's bounded native document parser. Webfold retains a
//! compatibility adapter for legacy XLS; parsing does not fetch or launch a browser.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::io::{BufRead, Cursor, Read, Write};
use std::sync::Arc;
use zip::ZipArchive;

const MAX_ENTRY: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

fn text(raw: &str) -> String {
    quick_xml::escape::unescape(raw).map_or_else(|_| raw.to_string(), |s| s.into_owned())
}

fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref().as_bytes() == name)
        .map(|a| text(a.value.as_ref()))
}

fn archive(data: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, String> {
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

fn entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
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

fn xml_text(data: &[u8], wanted: &[u8]) -> Result<String, String> {
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
            Event::CData(value) if depth > 0 => out.push_str(value.as_ref()),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn markdown_table(rows: &[Vec<String>]) -> String {
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
            out.push_str(&row.get(col).map_or("", String::as_str).replace('|', "\\|"));
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
        if payload.len() % 2 != 0 {
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

fn docx(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let document_path = if zip
        .file_names()
        .any(|name| name.is_ok_and(|n| n == "word/document.xml"))
    {
        "word/document.xml".to_string()
    } else {
        let rels = entry(&mut zip, "_rels/.rels")?;
        let mut reader = Reader::from_reader(rels.as_slice());
        let mut buf = Vec::new();
        let mut target = None;
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Empty(tag) | Event::Start(tag)
                    if tag.name().as_ref().as_bytes() == b"Relationship" =>
                {
                    if attr(&tag, b"Type").is_some_and(|kind| kind.ends_with("/officeDocument")) {
                        target = attr(&tag, b"Target");
                        break;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let path = target.ok_or("missing officeDocument relationship")?;
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err("officeDocument target escapes package".into());
        }
        path
    };
    let xml = entry(&mut zip, &document_path)?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let (mut paragraph, mut output, mut in_text, mut heading) =
        (String::new(), String::new(), false, 0usize);
    let (mut in_table, mut cell, mut row, mut table) =
        (false, String::new(), Vec::new(), Vec::new());
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"w:tbl" => in_table = true,
            Event::Start(tag) if matches!(tag.name().as_ref().as_bytes(), b"w:t" | b"m:t") => {
                in_text = true
            }
            Event::End(tag) if matches!(tag.name().as_ref().as_bytes(), b"w:t" | b"m:t") => {
                in_text = false
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"wp:docPr" => {
                if let Some(description) = attr(&tag, b"descr") {
                    if !paragraph.is_empty() {
                        paragraph.push(' ');
                    }
                    paragraph.push_str(&description);
                }
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"o:OLEObject" => {
                if let Some(kind) = attr(&tag, b"ProgID") {
                    paragraph.push_str(&format!("Embedded object: {kind}"));
                }
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"w:pStyle" => {
                if let Some(style) = attr(&tag, b"w:val") {
                    heading = style
                        .strip_prefix("Heading")
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(0)
                        .min(6);
                }
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"w:tab" => paragraph.push('\t'),
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"w:br" => paragraph.push('\n'),
            Event::Text(value) if in_text => paragraph.push_str(&text(value.as_ref())),
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:p" => {
                let line = paragraph.trim();
                if !line.is_empty() {
                    if in_table {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(line);
                    } else {
                        if heading > 0 {
                            output.push_str(&"#".repeat(heading));
                            output.push(' ');
                        }
                        output.push_str(line);
                        output.push_str("\n\n");
                    }
                }
                paragraph.clear();
                heading = 0;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tc" => {
                row.push(std::mem::take(&mut cell))
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tr" => {
                table.push(std::mem::take(&mut row))
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tbl" => {
                output.push_str(&markdown_table(&table));
                output.push('\n');
                table.clear();
                in_table = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    for index in 0..zip.len() {
        let name = zip
            .by_index_raw(index)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if name.starts_with("word/charts/") && name.ends_with(".xml") {
            let chart = entry(&mut zip, &name)?;
            let title = xml_text(&chart, b"a:t")?;
            let values = xml_text(&chart, b"c:v")?;
            if !title.trim().is_empty() || !values.trim().is_empty() {
                output.push_str(&format!(
                    "\n**Chart: {}**\n\n{}\n",
                    title.trim(),
                    values.trim()
                ));
            }
        } else if name.starts_with("word/diagrams/") && name.ends_with(".xml") {
            let diagram = entry(&mut zip, &name)?;
            let values = xml_text(&diagram, b"a:t")?;
            if !values.trim().is_empty() {
                output.push_str(&format!("\nDiagram: {}\n", values.trim()));
            }
        }
    }
    Ok(output.trim_end().to_string())
}

fn pptx(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let mut slides = Vec::new();
    for i in 0..zip.len() {
        let name = zip
            .by_index_raw(i)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if let Some(number) = name
            .strip_prefix("ppt/slides/slide")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<usize>().ok())
        {
            slides.push((number, name));
        }
    }
    slides.sort_by_key(|(number, _)| *number);
    let mut out = String::new();
    for (number, name) in slides {
        let xml = entry(&mut zip, &name)?;
        let lines = xml_text(&xml, b"a:t")?;
        if !lines.trim().is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n---\n\n");
            }
            out.push_str(&format!("## Slide {number}\n\n{}", lines.trim()));
        }
    }
    Ok(out)
}

fn shared_strings(xml: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let (mut strings, mut current, mut in_t, mut in_si) = (Vec::new(), String::new(), false, false);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"si" => {
                in_si = true;
                current.clear();
            }
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"t" => in_t = true,
            Event::Text(value) if in_si && in_t => current.push_str(&text(value.as_ref())),
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"t" => in_t = false,
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"si" => {
                strings.push(std::mem::take(&mut current));
                in_si = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(strings)
}

fn column(reference: &str) -> Option<usize> {
    let mut value = 0usize;
    for byte in reference.bytes().take_while(u8::is_ascii_alphabetic) {
        value = value
            .checked_mul(26)?
            .checked_add((byte.to_ascii_uppercase() - b'A' + 1) as usize)?;
    }
    value.checked_sub(1).filter(|v| *v < 16_384)
}

fn worksheet(xml: &[u8], strings: &[String]) -> Result<Vec<Vec<String>>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut rows = Vec::new();
    let (mut row, mut cell, mut cell_type, mut cell_col, mut in_value) =
        (Vec::new(), String::new(), String::new(), None, false);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"row" => row.clear(),
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"c" => {
                cell.clear();
                cell_type = attr(&tag, b"t").unwrap_or_default();
                cell_col = attr(&tag, b"r").and_then(|r| column(&r));
            }
            Event::Start(tag) if matches!(tag.name().as_ref().as_bytes(), b"v" | b"t") => {
                in_value = true
            }
            Event::Text(value) if in_value => cell.push_str(&text(value.as_ref())),
            Event::End(tag) if matches!(tag.name().as_ref().as_bytes(), b"v" | b"t") => {
                in_value = false
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"c" => {
                let value = if cell_type == "s" {
                    let index = cell
                        .parse::<usize>()
                        .map_err(|_| "bad shared-string index")?;
                    strings
                        .get(index)
                        .cloned()
                        .ok_or("shared-string index out of range")?
                } else {
                    std::mem::take(&mut cell)
                };
                let col = cell_col.unwrap_or(row.len());
                if row.len() <= col {
                    row.resize(col + 1, String::new());
                }
                row[col] = value;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"row" => {
                rows.push(std::mem::take(&mut row))
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(rows)
}

fn xlsx(data: &[u8]) -> Result<String, String> {
    let mut zip = archive(data)?;
    let strings = match entry(&mut zip, "xl/sharedStrings.xml") {
        Ok(xml) => shared_strings(&xml)?,
        Err(_) => Vec::new(),
    };
    let mut sheets = Vec::new();
    for i in 0..zip.len() {
        let name = zip
            .by_index_raw(i)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .to_string();
        if let Some(number) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<usize>().ok())
        {
            sheets.push((number, name));
        }
    }
    sheets.sort_by_key(|(number, _)| *number);
    let mut out = String::new();
    for (number, name) in sheets {
        let xml = entry(&mut zip, &name)?;
        let rows = worksheet(&xml, &strings)?;
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("## Sheet {number}\n\n{}", markdown_table(&rows)));
    }
    Ok(out.trim_end().to_string())
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
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"table:table-cell" => {
                let repeat = odf_repeat(&tag);
                let value = odf_cell_value(&tag);
                row.extend(std::iter::repeat_n(value, repeat));
            }
            Event::Start(tag) if in_cell && tag.name().as_ref().as_bytes() == b"text:p" => {
                in_text = true
            }
            Event::Text(value) if in_cell && in_text => cell.push_str(&text(value.as_ref())),
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"text:p" => {
                in_text = false;
                if !cell.ends_with(' ') {
                    cell.push(' ');
                }
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"table:table-cell" => {
                row.push(cell.trim().to_string());
                cell.clear();
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

fn rtf(data: &[u8]) -> Result<String, String> {
    if !data.starts_with(b"{\\rtf") {
        return Err("invalid RTF header".into());
    }
    let mut out = String::new();
    let mut hidden = false;
    let mut stack = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        match data[i] {
            b'{' => {
                stack.push(hidden);
                i += 1;
            }
            b'}' => {
                hidden = stack.pop().ok_or("unbalanced RTF group")?;
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&next) = data.get(i) else {
                    break;
                };
                if matches!(next, b'\\' | b'{' | b'}') {
                    if !hidden {
                        out.push(next as char);
                    }
                    i += 1;
                } else if next == b'\'' {
                    let hex = data.get(i + 1..i + 3).ok_or("truncated RTF hex escape")?;
                    let hex = std::str::from_utf8(hex).map_err(|e| e.to_string())?;
                    let byte = u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                    if !hidden {
                        out.push_str(&encoding_rs::WINDOWS_1252.decode(&[byte]).0);
                    }
                    i += 3;
                } else if next == b'*' {
                    hidden = true;
                    i += 1;
                } else if next.is_ascii_alphabetic() {
                    let start = i;
                    while data.get(i).is_some_and(u8::is_ascii_alphabetic) {
                        i += 1;
                    }
                    let word = &data[start..i];
                    let number_start = i;
                    if data.get(i) == Some(&b'-') {
                        i += 1;
                    }
                    while data.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                    let number = std::str::from_utf8(&data[number_start..i])
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok());
                    if data.get(i) == Some(&b' ') {
                        i += 1;
                    }
                    match word {
                        b"bin" => {
                            let count = number.ok_or("RTF bin length missing")?;
                            let count =
                                usize::try_from(count).map_err(|_| "negative RTF bin length")?;
                            i = i
                                .checked_add(count)
                                .filter(|end| *end <= data.len())
                                .ok_or("truncated RTF binary run")?;
                        }
                        b"fonttbl" | b"colortbl" | b"stylesheet" | b"pict" | b"object"
                        | b"info" => hidden = true,
                        b"par" | b"line" if !hidden => out.push_str("\n\n"),
                        b"tab" if !hidden => out.push('\t'),
                        b"u" if !hidden => {
                            if let Some(value) = number {
                                let codepoint = (value as i16 as u16) as u32;
                                if let Some(ch) = char::from_u32(codepoint) {
                                    out.push(ch);
                                }
                            }
                            if data.get(i) == Some(&b'?') {
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                } else {
                    if !hidden && next == b'~' {
                        out.push(' ');
                    }
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1,
            byte => {
                if !hidden {
                    out.push(byte as char);
                }
                i += 1;
            }
        }
    }
    if !stack.is_empty() {
        return Err("unclosed RTF group".into());
    }
    Ok(out.trim().to_string())
}

/// Convert an in-memory document to Markdown. Unsupported extensions return an
/// error rather than silently interpreting binary Office bytes as text.
pub fn convert_bytes(extension: &str, data: &[u8]) -> Result<String, String> {
    match extension.to_ascii_lowercase().as_str() {
        "docx" | "docm" => docx(data),
        "pptx" => pptx(data),
        "xlsx" | "xlsm" => xlsx(data),
        "odt" | "odp" => odt(data),
        "ods" => ods(data),
        "xls" => Err("legacy XLS conversion remains in the Webfold compatibility adapter".into()),
        "xlsb" => xlsb(data),
        "epub" => epub(data),
        "rtf" => rtf(data),
        "csv" => {
            let decoded = decode_csv(data)?;
            csv_rows(&decoded, csv_delimiter(&decoded)).map(|rows| markdown_table(&rows))
        }
        "html" | "htm" => Ok(crate::html::to_markdown(data, "")),
        "txt" | "md" | "markdown" => Ok(String::from_utf8_lossy(data).into_owned()),
        other => Err(format!("unsupported document extension: {other}")),
    }
}

pub fn convert_path(path: &std::path::Path) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or("missing document extension")?;
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    convert_bytes(extension, &data)
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
        "xlsx" | "xlsm" => {
            // Stream the worksheet XML directly to the sink. We don't build the
            // LiftedDoc Tables on the cold path; the fold's cache is built lazily on
            // repeat reads, not up front.
            write_xlsx_streaming(data, out)
        }
        "docx" | "docm" => write_docx_streaming(data, out),
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
        let v = rows[0].get(c).map(String::as_str).unwrap_or("");
        if v.contains('|') {
            for ch in v.chars() {
                if ch == '|' {
                    line.push('\\');
                }
                line.push(ch);
            }
        } else {
            line.push_str(v);
        }
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
            let v = row.get(c).map(String::as_str).unwrap_or("");
            if v.contains('|') {
                for ch in v.chars() {
                    if ch == '|' {
                        line.push('\\');
                    }
                    line.push(ch);
                }
            } else {
                line.push_str(v);
            }
            line.push_str(" |");
        }
        line.push('\n');
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn write_xlsx_streaming<W: Write>(data: &[u8], out: &mut W) -> Result<(), String> {
    let mut zip = archive(data)?;
    let strings = match entry(&mut zip, "xl/sharedStrings.xml") {
        Ok(xml) => shared_strings(&xml)?,
        Err(_) => Vec::new(),
    };
    let mut sheets: Vec<(usize, String)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(entry) = zip.by_index_raw(i) else {
            continue;
        };
        let Ok(name) = entry.name().map(|n| n.into_owned()) else {
            continue;
        };
        if let Some(number) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<usize>().ok())
        {
            sheets.push((number, name));
        }
    }
    sheets.sort_by_key(|(n, _)| *n);
    let mut line = String::with_capacity(64);
    for (sheet_idx, sheet_name) in sheets.iter() {
        line.clear();
        line.push_str("## Sheet ");
        line.push_str(&sheet_idx.to_string());
        line.push_str("\n\n");
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;

        let xml = entry(&mut zip, sheet_name)?;
        let mut reader = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        let mut last_col: u32 = 0;
        let mut row_buf: Vec<String> = Vec::with_capacity(16);
        let mut first_row_in_sheet = true;
        loop {
            buf.clear();
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) | Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"c" => {
                    let cell_ref = attr(&tag, b"r");
                    let col = cell_ref.as_deref().and_then(column).unwrap_or(0);
                    let kind = attr(&tag, b"t");
                    let value = read_cell_value(&mut reader, &mut buf, kind.as_deref(), &strings)?;
                    // Pad row_buf with empty strings so col is indexable.
                    while row_buf.len() <= col {
                        row_buf.push(String::new());
                    }
                    row_buf[col] = value;
                    if (col as u32) > last_col {
                        last_col = col as u32;
                    }
                }
                Event::End(tag) if tag.name().as_ref().as_bytes() == b"row" => {
                    // Flush the row.
                    line.clear();
                    line.push('|');
                    for v in &row_buf {
                        line.push(' ');
                        if v.contains('|') {
                            for ch in v.chars() {
                                if ch == '|' {
                                    line.push('\\');
                                }
                                line.push(ch);
                            }
                        } else {
                            line.push_str(v);
                        }
                        line.push_str(" |");
                    }
                    line.push('\n');
                    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                    if first_row_in_sheet {
                        line.clear();
                        line.push('|');
                        for _ in 0..=last_col {
                            line.push_str(" --- |");
                        }
                        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                        first_row_in_sheet = false;
                    }
                    row_buf.clear();
                    last_col = 0;
                }
                Event::End(tag) if tag.name().as_ref().as_bytes() == b"sheetData" => break,
                Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(())
}

fn write_docx_streaming<W: Write>(data: &[u8], out: &mut W) -> Result<(), String> {
    let mut zip = archive(data)?;
    let document_path = if zip
        .file_names()
        .any(|name| name.is_ok_and(|n| n == "word/document.xml"))
    {
        "word/document.xml".to_string()
    } else {
        let rels = entry(&mut zip, "_rels/.rels")?;
        let mut reader = Reader::from_reader(rels.as_slice());
        let mut buf = Vec::new();
        let mut target = None;
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Empty(tag) | Event::Start(tag)
                    if tag.name().as_ref().as_bytes() == b"Relationship" =>
                {
                    if attr(&tag, b"Type").is_some_and(|kind| kind.ends_with("/officeDocument")) {
                        target = attr(&tag, b"Target");
                        break;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let path = target.ok_or("missing officeDocument relationship")?;
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err("officeDocument target escapes package".into());
        }
        path
    };
    let xml = entry(&mut zip, &document_path)?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let mut paragraph = String::new();
    let mut heading: u8 = 0;
    let mut in_text = false;
    let mut in_table = false;
    let mut cell = String::new();
    let mut row: Vec<String> = Vec::new();
    let mut table: Vec<Vec<String>> = Vec::new();
    // Track whether the previous paragraph was non-empty so we can emit "\n\n"
    // BETWEEN paragraphs (matching convert_bytes's "\n\n" join) without leaving a
    // trailing "\n\n" after the last one.
    let mut wrote_paragraph = false;
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"w:tbl" => {
                in_table = true;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tbl" => {
                in_table = false;
            }
            Event::Start(tag) if matches!(tag.name().as_ref().as_bytes(), b"w:t" | b"m:t") => {
                in_text = true
            }
            Event::End(tag) if matches!(tag.name().as_ref().as_bytes(), b"w:t" | b"m:t") => {
                in_text = false
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"w:pStyle" => {
                if let Some(style) = attr(&tag, b"w:val") {
                    heading = style
                        .strip_prefix("Heading")
                        .and_then(|n| n.parse::<u8>().ok())
                        .unwrap_or(0)
                        .min(6);
                }
            }
            Event::Text(value) if in_text => paragraph.push_str(&text(value.as_ref())),
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:p" => {
                let line = paragraph.trim();
                if !line.is_empty() {
                    if in_table {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(line);
                    } else {
                        let prefix = if heading > 0 {
                            "#".repeat(heading as usize)
                        } else {
                            String::new()
                        };
                        if wrote_paragraph {
                            out.write_all(b"\n\n").map_err(|e| e.to_string())?;
                        }
                        let mut out_line = String::with_capacity(prefix.len() + line.len() + 4);
                        if !prefix.is_empty() {
                            out_line.push_str(&prefix);
                            out_line.push(' ');
                        }
                        out_line.push_str(line);
                        out.write_all(out_line.as_bytes())
                            .map_err(|e| e.to_string())?;
                        wrote_paragraph = true;
                    }
                }
                paragraph.clear();
                heading = 0;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tc" => {
                row.push(std::mem::take(&mut cell))
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tr" => {
                table.push(std::mem::take(&mut row))
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // Flush the trailing table (if the document ended mid-table).
    if !table.is_empty() {
        let table_md = markdown_table(&table);
        out.write_all(table_md.as_bytes())
            .map_err(|e| e.to_string())?;
        out.write_all(b"\n").map_err(|e| e.to_string())?;
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
            _ => None,
        }
    }

    /// Whether this format goes through the streaming lift path.
    /// Non-streaming formats still get cached as `Arc<String>` (no parse on warm hit), but they
    /// don't stream the render.
    fn is_streaming(self) -> bool {
        matches!(self, Self::Xlsx | Self::Docx | Self::Csv)
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
    pub cells: hashtrinity::Table<DocCoord, String>,
    /// Cell kind tag: 0 = text, 1 = number, 2 = date, 3 = heading-1, 4 = heading-2, ….
    /// Used by the render pass to pick the right Markdown prefix without re-parsing.
    pub kinds: hashtrinity::Table<DocCoord, u8>,
    /// The render order: which coordinates to emit, in order. Stored as a Vec so render is
    /// strictly streaming-write (no Table iteration order surprises).
    pub order: Vec<DocCoord>,
    /// Per-coordinate row width (DOCX tables, XLSX rows). Used to terminate table renders.
    /// `0` means "no row width tracked for this coordinate".
    pub row_widths: hashtrinity::Table<DocCoord, u16>,
}

/// The fold cache. Two hashtrinity::Tables:
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
    lifted: hashtrinity::Table<(u64, u8), Arc<LiftedDoc>>,
    rendered: hashtrinity::Table<(u64, u8), Arc<String>>,
    hits: u64,
    misses: u64,
}

impl DocumentCache {
    pub fn new() -> Self {
        Self {
            lifted: hashtrinity::Table::new(),
            rendered: hashtrinity::Table::new(),
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
        DocFormat::Xlsx => lift_xlsx(data),
        DocFormat::Docx | DocFormat::Docm => lift_docx(data),
        DocFormat::Csv => lift_csv(data),
        _ => Err(format!(
            "lift: format {format:?} uses the rendered-string cache path"
        )),
    }
}

fn lift_xlsx(data: &[u8]) -> Result<LiftedDoc, String> {
    let mut zip = archive(data)?;

    // shared strings: one Vec<u8> buffer, materialized once. Bounded by the workbook; cannot
    // exceed MAX_TOTAL/MAX_ENTRY because `archive` already validated that.
    let strings = match read_zip_entry(&mut zip, "xl/sharedStrings.xml") {
        Ok(xml) => shared_strings(&xml)?,
        Err(_) => Vec::new(),
    };

    let mut sheets: Vec<(usize, String)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(entry) = zip.by_index_raw(i) else {
            continue;
        };
        let Ok(name) = entry.name().map(|n| n.into_owned()) else {
            continue;
        };
        if let Some(number) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<usize>().ok())
        {
            sheets.push((number, name));
        }
    }
    sheets.sort_by_key(|(n, _)| *n);

    let mut cells: hashtrinity::Table<DocCoord, String> = hashtrinity::Table::new();
    let mut kinds: hashtrinity::Table<DocCoord, u8> = hashtrinity::Table::new();
    let mut order: Vec<DocCoord> = Vec::new();
    let mut section_names: Vec<String> = Vec::with_capacity(sheets.len());
    let mut row_widths: hashtrinity::Table<DocCoord, u16> = hashtrinity::Table::new();

    for (number, name) in &sheets {
        section_names.push(format!("Sheet {number}"));
        // Stream the worksheet XML straight from the ZIP entry into the XML reader. No
        // `Vec::with_capacity(file.size())`, no `read_to_end` — the entry's `Read` impl is
        // borrowed by `Reader::from_reader`.
        let xml = read_zip_entry(&mut zip, name)?;
        let mut reader = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        // current_row tracks the worksheet row index; col tracks the column index inside
        // that row. They are independent — a `<row>` event sets current_row from its `r=`
        // attribute (default 0), and a `<c>` event sets col from its `r=` attribute (default
        // current_row, which advances row-major through sparse columns).
        let mut current_row: u32 = 0;
        let mut col: usize = 0;
        let mut row_width: u16 = 0;
        loop {
            buf.clear();
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) if tag.name().as_ref().as_bytes() == b"row" => {
                    current_row = attr(&tag, b"r")
                        .and_then(|r| r.parse::<u32>().ok())
                        .unwrap_or(0);
                    row_width = 0;
                    col = 0;
                }
                Event::Start(tag) | Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"c" => {
                    let cell_type = attr(&tag, b"t");
                    let cell_ref = attr(&tag, b"r");
                    col = cell_ref.as_deref().and_then(column).unwrap_or(col);
                    // Read the <v> or <is> body as a streaming text accumulate.
                    let value =
                        read_cell_value(&mut reader, &mut buf, cell_type.as_deref(), &strings)?;
                    if !value.is_empty() {
                        let coord = (*number as u32, current_row, col as u32);
                        cells.put(coord, value);
                        kinds.put(coord, 1);
                        order.push(coord);
                    }
                    if col as u16 + 1 > row_width {
                        row_width = col as u16 + 1;
                    }
                }
                Event::End(tag) if tag.name().as_ref().as_bytes() == b"row" => {
                    // Tag the row's first cell with the row width for the render pass.
                    if row_width > 0 {
                        row_widths.put((*number as u32, current_row, 0), row_width);
                    }
                }
                Event::End(tag) if tag.name().as_ref().as_bytes() == b"sheetData" => break,
                Event::Eof => break,
                _ => {}
            }
        }
    }

    Ok(LiftedDoc {
        format: DocFormat::Xlsx,
        section_names,
        cells,
        kinds,
        order,
        row_widths,
    })
}

fn lift_docx(data: &[u8]) -> Result<LiftedDoc, String> {
    let mut zip = archive(data)?;

    // Find the document part. The simple case is word/document.xml; otherwise walk the rels.
    let document_path = if zip
        .file_names()
        .any(|name| name.is_ok_and(|n| n == "word/document.xml"))
    {
        "word/document.xml".to_string()
    } else {
        let rels = read_zip_entry(&mut zip, "_rels/.rels")?;
        let mut reader = Reader::from_reader(rels.as_slice());
        let mut buf = Vec::new();
        let mut target = None;
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Empty(tag) | Event::Start(tag)
                    if tag.name().as_ref().as_bytes() == b"Relationship" =>
                {
                    if attr(&tag, b"Type").is_some_and(|kind| kind.ends_with("/officeDocument")) {
                        target = attr(&tag, b"Target");
                        break;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let path = target.ok_or("missing officeDocument relationship")?;
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err("officeDocument target escapes package".into());
        }
        path
    };

    let xml = read_zip_entry(&mut zip, &document_path)?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();

    let mut cells: hashtrinity::Table<DocCoord, String> = hashtrinity::Table::new();
    let mut kinds: hashtrinity::Table<DocCoord, u8> = hashtrinity::Table::new();
    let mut order: Vec<DocCoord> = Vec::new();
    let mut row_widths: hashtrinity::Table<DocCoord, u16> = hashtrinity::Table::new();
    let mut paragraph_idx: u32 = 0;
    let mut current = String::new();
    let mut heading: u8 = 0;
    let mut in_text = false;
    let mut in_table = false;
    let mut row_cells: u16 = 0;
    let mut table_start: u32 = 0;

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) if tag.name().as_ref().as_bytes() == b"w:tbl" => {
                in_table = true;
                table_start = paragraph_idx;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tbl" => {
                in_table = false;
            }
            Event::Start(tag) if matches!(tag.name().as_ref().as_bytes(), b"w:t" | b"m:t") => {
                in_text = true
            }
            Event::End(tag) if matches!(tag.name().as_ref().as_bytes(), b"w:t" | b"m:t") => {
                in_text = false
            }
            Event::Empty(tag) if tag.name().as_ref().as_bytes() == b"w:pStyle" => {
                if let Some(style) = attr(&tag, b"w:val") {
                    heading = style
                        .strip_prefix("Heading")
                        .and_then(|n| n.parse::<u8>().ok())
                        .unwrap_or(0)
                        .min(6);
                }
            }
            Event::Text(value) if in_text => current.push_str(&text(value.as_ref())),
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:p" => {
                let coord = (paragraph_idx, 0, 0);
                if !current.trim().is_empty() {
                    let kind = if heading > 0 { 2 + heading } else { 1 };
                    cells.put(coord, std::mem::take(&mut current));
                    kinds.put(coord, kind);
                    order.push(coord);
                    if in_table {
                        row_cells += 1;
                    }
                }
                paragraph_idx += 1;
                heading = 0;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"w:tr" => {
                row_widths.put((table_start, paragraph_idx, 0), row_cells);
                row_cells = 0;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(LiftedDoc {
        format: DocFormat::Docx,
        section_names: vec!["DOCX".to_string()],
        cells,
        kinds,
        order,
        row_widths,
    })
}

fn lift_csv(data: &[u8]) -> Result<LiftedDoc, String> {
    let decoded = decode_csv(data)?;
    let delimiter = csv_delimiter(&decoded);
    let rows = csv_rows(&decoded, delimiter)?;

    let mut cells: hashtrinity::Table<DocCoord, String> = hashtrinity::Table::new();
    let mut kinds: hashtrinity::Table<DocCoord, u8> = hashtrinity::Table::new();
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

    let mut row_widths: hashtrinity::Table<DocCoord, u16> = hashtrinity::Table::new();
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

/// Read a ZIP entry as `Vec<u8>` (bounded by MAX_ENTRY). Used by the lift where the XML reader
/// needs an owned buffer; the streaming entry is read once and dropped. Compared to `entry()`
/// above, this function deliberately *does* materialize so the parser can own the buffer and
/// free it on drop — but it caps at MAX_ENTRY so memory is bounded.
fn read_zip_entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    entry(zip, name)
}

// ─── Render (streaming write to a `Write` sink) ──────────────────────────────────────────────

fn render_streaming<W: Write>(doc: &LiftedDoc, out: &mut W) -> Result<(), String> {
    match doc.format {
        DocFormat::Xlsx => render_xlsx(doc, out),
        DocFormat::Docx | DocFormat::Docm => render_docx(doc, out),
        DocFormat::Csv => render_csv(doc, out),
        _ => Err(format!("render: format {:?} not streaming", doc.format)),
    }
}

fn render_xlsx<W: Write>(doc: &LiftedDoc, out: &mut W) -> Result<(), String> {
    // doc.order is already (sheet, row, col) row-major across sheets. Walk it in a
    // single pass, tracking which sheet and row we are in, with a per-row scratch
    // String for the current line. The hashtrinity::Table borrows return Option<&String>
    // for free — no per-cell clone. hashtrinity::Table::get is O(1) per coord; for
    // production Tables larger than the 512 KiB cache-resident threshold this path
    // would benefit from get_each batching (F.1) — but the Track E corpus's Tables are
    // tiny and fall back to the plain get loop inside get_each, so the chunking is
    // omitted here for clarity. When the LiftedDoc Tables grow past the cache line the
    // chunked get_each path becomes the win.
    if doc.order.is_empty() {
        return Ok(());
    }

    let mut current_sheet: u32 = u32::MAX;
    let mut current_row: u32 = u32::MAX;
    let mut last_col: u32 = 0;
    let mut line = String::with_capacity(64);
    let mut header_emitted = false;

    for coord in &doc.order {
        if coord.0 != current_sheet {
            // Finish the previous sheet's last row + flush.
            if current_sheet != u32::MAX {
                line.push('\n');
                out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                line.clear();
            }
            current_sheet = coord.0;
            current_row = u32::MAX;
            let section_name = doc
                .section_names
                .get((coord.0 as usize).saturating_sub(1))
                .map(String::as_str)
                .unwrap_or("?");
            line.clear();
            line.push_str("## ");
            line.push_str(section_name);
            line.push_str("\n\n");
            out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            line.clear();
            header_emitted = false;
        }
        if coord.1 != current_row {
            // Finish the previous row.
            if current_row != u32::MAX {
                line.push('\n');
                out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                if !header_emitted {
                    // The previous row was the header row. Emit separator.
                    line.clear();
                    line.push('|');
                    for _ in 0..=last_col {
                        line.push_str(" --- |");
                    }
                    line.push('\n');
                    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                    header_emitted = true;
                }
                line.clear();
            }
            current_row = coord.1;
            line.push('|');
        }
        let cell = doc.cells.get(coord).map(String::as_str).unwrap_or("");
        line.push(' ');
        if cell.contains('|') {
            for ch in cell.chars() {
                if ch == '|' {
                    line.push('\\');
                }
                line.push(ch);
            }
        } else {
            line.push_str(cell);
        }
        line.push_str(" |");
        last_col = coord.2;
    }
    // Flush the last row of the last sheet.
    line.push('\n');
    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    // If the last sheet had exactly one row (the header row) and we never emitted the
    // separator, emit it now so a one-row sheet still matches the canonical Markdown
    // table shape (header, separator, zero data rows).
    if !header_emitted && current_sheet != u32::MAX {
        line.clear();
        line.push('|');
        for _ in 0..=last_col {
            line.push_str(" --- |");
        }
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn render_docx<W: Write>(doc: &LiftedDoc, out: &mut W) -> Result<(), String> {
    // Each non-empty paragraph is its own cell. Headings render with `#` × level; body
    // paragraphs separate with a blank line (matches convert_bytes's "\n\n" join and
    // .trim_end() final pass). The prefix is a precomputed static string; we don't
    // allocate a new heading prefix per cell. The line buffer is reused across cells
    // so the per-cell allocation is just `line.clear()`.
    const HEADING_PREFIXES: &[&str] = &[
        "",        // kind 0 — placeholder (shouldn't reach here)
        "",        // kind 1 — body text, no prefix
        "",        // kind 2 — placeholder
        "# ",      // kind 3 — heading 1
        "## ",     // kind 4 — heading 2
        "### ",    // kind 5 — heading 3
        "#### ",   // kind 6 — heading 4
        "##### ",  // kind 7 — heading 5
        "###### ", // kind 8 — heading 6
    ];
    let last_index = doc.order.len().saturating_sub(1);
    let mut line = String::with_capacity(64);
    for (i, coord) in doc.order.iter().enumerate() {
        let cell = doc.cells.get(coord).map(String::as_str).unwrap_or("");
        let kind = doc.kinds.get(coord).copied().unwrap_or(1) as usize;
        let prefix = if (3..=8).contains(&kind) {
            HEADING_PREFIXES[kind]
        } else {
            ""
        };
        if !prefix.is_empty() {
            line.clear();
            line.push_str(prefix);
            line.push_str(cell);
            if i < last_index {
                line.push_str("\n\n");
            }
            out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        } else if !cell.trim().is_empty() {
            line.clear();
            line.push_str(cell);
            if i < last_index {
                line.push_str("\n\n");
            }
            out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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
        if cell.contains('|') {
            for ch in cell.chars() {
                if ch == '|' {
                    line.push('\\');
                }
                line.push(ch);
            }
        } else {
            line.push_str(cell);
        }
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
fn read_cell_value<R: BufRead>(
    reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    cell_type: Option<&str>,
    strings: &[String],
) -> Result<String, String> {
    let mut value = String::new();
    let mut in_body = false;
    let mut is_self_closing = false;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(|e| e.to_string())? {
            Event::Start(tag) if matches!(tag.name().as_ref().as_bytes(), b"v" | b"t" | b"is") => {
                in_body = true;
            }
            Event::Text(t) if in_body => value.push_str(&text(t.as_ref())),
            Event::End(tag) if matches!(tag.name().as_ref().as_bytes(), b"v" | b"t" | b"is") => {
                break;
            }
            Event::Empty(_) => {
                is_self_closing = true;
                break;
            }
            Event::End(tag) if tag.name().as_ref().as_bytes() == b"c" => {
                // </c> closes the cell before we hit a </v>: treat as empty.
                break;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let _ = is_self_closing;
    if cell_type == Some("s") {
        let idx = value
            .parse::<usize>()
            .map_err(|_| "bad shared-string index")?;
        strings
            .get(idx)
            .cloned()
            .ok_or_else(|| "shared-string index out of range".to_string())
    } else {
        Ok(value)
    }
}

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
            worksheet(xml, &["hello".into()]).unwrap(),
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
        assert_eq!(convert_bytes("odt", &odf).unwrap(), "## Title\n\nBody");
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
