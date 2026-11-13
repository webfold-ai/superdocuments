//! Typed cell values stay available next to the display text: `read_workbook_typed` returns the
//! number as stored (`4.0999999999999996`), the serial of a date cell with its ISO form, and so on,
//! while the Markdown shows what the sheet shows.

use std::io::{Cursor, Write};
use superdocuments::xlsx::{read_workbook_typed, CellValue};

fn workbook() -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, body) in [
        ("xl/styles.xml", r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="2"><numFmt numFmtId="164" formatCode="mmm\-yyyy"/><numFmt numFmtId="165" formatCode="0.0"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="165"/></cellXfs></styleSheet>"#),
        ("xl/sharedStrings.xml", r#"<sst><si><t>Series</t></si></sst>"#),
        ("xl/worksheets/sheet1.xml", r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1"><v>46266</v></c><c r="C1" s="2"><v>4.0999999999999996</v></c><c r="D1" t="b"><v>1</v></c><c r="E1" t="e"><v>#DIV/0!</v></c><c r="F1"><f>A1+1</f></c></row></sheetData></worksheet>"#),
    ] {
        z.start_file(name, opts).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn typed_cells_keep_value_and_display() {
    let sheets = read_workbook_typed(&workbook()).unwrap();
    let row = &sheets[0].rows[0];
    assert_eq!(row[0].value, CellValue::Text("Series".into()));
    assert_eq!(row[1].display, "Sep-2026");
    assert_eq!(row[1].value, CellValue::DateTime { serial: 46266.0, iso: "2026-09-01".into() });
    assert_eq!(row[2].display, "4.1");
    assert_eq!(row[2].value, CellValue::Number { value: 4.0999999999999996, raw: "4.0999999999999996".into() });
    assert_eq!(row[3].value, CellValue::Bool(true));
    assert_eq!(row[4].value, CellValue::Error("#DIV/0!".into()));
    assert_eq!(row[5].value, CellValue::Formula("A1+1".into()));
}
