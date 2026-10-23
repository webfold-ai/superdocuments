//! Bounded XLSX/XLSM reader extracted from Webfold's native worksheet parser.
use crate::Worksheet;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::io::{Cursor, Read};
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

/// Parse a workbook into ordered worksheet records. XLSX and macro-enabled XLSM share this
/// container format. ZIP expansion and per-part sizes are bounded before XML parsing.
pub fn read_workbook(data: &[u8]) -> Result<Vec<Worksheet>, String> {
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
    sheets
        .into_iter()
        .enumerate()
        .map(|(index, (number, path))| {
            let xml = entry(&mut zip, &path)?;
            Ok(Worksheet {
                index,
                name: format!("Sheet {number}"),
                rows: worksheet(&xml, &strings)?,
            })
        })
        .collect()
}

/// Read one worksheet by zero-based workbook-order index.
pub fn read_sheet(data: &[u8], index: usize) -> Result<Worksheet, String> {
    read_workbook(data)?
        .into_iter()
        .nth(index)
        .ok_or_else(|| format!("worksheet index {index} out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn workbook() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("xl/sharedStrings.xml", "<sst><si><t>Name</t></si><si><t>Ada</t></si></sst>"),
            ("xl/worksheets/sheet1.xml", "<worksheet><sheetData><row><c r=\"A1\" t=\"s\"><v>0</v></c></row><row><c r=\"A2\" t=\"s\"><v>1</v></c><c r=\"C2\"><v>42</v></c></row></sheetData></worksheet>"),
            ("xl/worksheets/sheet2.xml", "<worksheet><sheetData><row><c r=\"A1\"><v>second</v></c></row></sheetData></worksheet>"),
        ] { zip.start_file(name, opts).unwrap(); zip.write_all(body.as_bytes()).unwrap(); }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn parses_shared_and_sparse_cells_in_order() {
        let sheet = read_sheet(&workbook(), 0).unwrap();
        assert_eq!(sheet.name, "Sheet 1");
        assert_eq!(sheet.rows, vec![vec!["Name"], vec!["Ada", "", "42"]]);
        let second = read_sheet(&workbook(), 1).unwrap();
        assert_eq!((second.index, second.name.as_str()), (1, "Sheet 2"));
        assert_eq!(second.rows, vec![vec!["second"]]);
        assert!(read_sheet(&workbook(), 2).is_err());
    }

    #[test]
    fn rejects_invalid_or_over_expanding_workbooks() {
        assert!(read_workbook(b"not a zip").is_err());
    }
}
