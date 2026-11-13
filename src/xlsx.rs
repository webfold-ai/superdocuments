//! Bounded XLSX/XLSM reader extracted from Webfold's native worksheet parser.
use crate::Worksheet;

/// Parse a workbook into worksheet records in tab order, named as the workbook names them.
/// XLSX and macro-enabled XLSM share this container format. ZIP expansion and per-part sizes
/// are bounded before XML parsing. A package without `workbook.xml` falls back to the numbers
/// in the sheet file names (`Sheet 1`, ...).
pub fn read_workbook(data: &[u8]) -> Result<Vec<Worksheet>, String> {
    Ok(crate::ooxml::xlsx::read(data)?
        .into_iter()
        .enumerate()
        .map(|(index, sheet)| Worksheet {
            index,
            name: sheet.name,
            rows: sheet.rows,
        })
        .collect())
}

/// A cell's typed value, kept next to what the cell shows.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    /// Shared, inline or formula-result text.
    Text(String),
    /// A number whose style is not a date format: the value and the text exactly as stored
    /// (`4.0999999999999996`).
    Number {
        value: f64,
        raw: String,
    },
    Bool(bool),
    /// A serial date or time (a date number format): the serial and its ISO form (`2026-09-01`,
    /// `12:30:00` or both).
    DateTime {
        serial: f64,
        iso: String,
    },
    /// An error value such as `#DIV/0!`.
    Error(String),
    /// A formula with no stored result.
    Formula(String),
}

/// One cell: `display` is what the sheet shows (number format applied), `value` is the typed value.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedCell {
    pub display: String,
    pub value: CellValue,
}

impl TypedCell {
    pub(crate) fn empty() -> Self {
        Self {
            display: String::new(),
            value: CellValue::Empty,
        }
    }
}

/// A worksheet whose cells keep their typed values. Rows are as in [`Worksheet`]: cells at their
/// absolute column, rows with no content dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedWorksheet {
    pub index: usize,
    pub name: String,
    pub rows: Vec<Vec<TypedCell>>,
}

/// Like [`read_workbook`], with each cell's typed value as well as its display text.
pub fn read_workbook_typed(data: &[u8]) -> Result<Vec<TypedWorksheet>, String> {
    Ok(crate::ooxml::xlsx::read_typed(data)?
        .into_iter()
        .enumerate()
        .map(|(index, sheet)| TypedWorksheet {
            index,
            name: sheet.name,
            rows: sheet.rows,
        })
        .collect())
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
    use std::io::{Cursor, Write};

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
