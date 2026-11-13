//! Legacy Excel (`.xls`, BIFF) conversion. `lo-sheets.xls` was written by LibreOffice from a
//! generated workbook; `lo-sheets.xlsx` is LibreOffice's round trip of that same `.xls`, so the two
//! conversions must match. `date_1904.xls` and `date_1904.xlsx` are calamine's MIT-licensed test
//! files (see tests/fixtures/xls/README.md).

use std::fs;
use superdocuments::document::{convert_bytes, convert_streaming, DocumentCache};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(format!("tests/fixtures/xls/{name}")).expect("fixture")
}

#[test]
fn xls_matches_the_xlsx_export_of_the_same_workbook() {
    let xls = convert_bytes("xls", &fixture("lo-sheets.xls")).unwrap();
    let xlsx = convert_bytes("xlsx", &fixture("lo-sheets.xlsx")).unwrap();
    assert_eq!(xls, xlsx);
}

#[test]
fn xls_sheets_are_in_tab_order_with_values_formulas_and_errors() {
    let md = convert_bytes("xls", &fixture("lo-sheets.xls")).unwrap();
    // Tab order: Summary, Dates, Gaps Two. Name order would put Dates first.
    let summary = md.find("## Summary").expect(&md);
    let dates = md.find("## Dates").expect(&md);
    let gaps = md.find("## Gaps Two").expect(&md);
    assert!(summary < dates && dates < gaps, "{md}");
    assert!(md.contains("| Item | Value | Error |"), "{md}");
    assert!(md.contains("| Apples | 1 | #DIV/0! |"), "{md}");
    assert!(md.contains("| Pears | 2.5 | #N/A |"), "{md}");
    // A formula result (`=SUM(B2:B3)`) is the stored value, not the formula.
    assert!(md.contains("| Total | 3.5 |"), "{md}");
    assert!(md.contains("| Flag | TRUE |"), "{md}");
    // A pipe in a cell is escaped, as in the XLSX output.
    assert!(md.contains("| A \\| B |"), "{md}");
    // Date and time formats: a date, a time of day, a date-time and a date formula result.
    assert!(md.contains("| 2023-03-15 |"), "{md}");
    assert!(md.contains("| 12:00:00 |"), "{md}");
    assert!(md.contains("| 2023-03-15 12:00:00 |"), "{md}");
    assert!(md.contains("| 2023-03-16 |"), "{md}");
    // Column B alone starts at column 2: leading empty cells keep their position.
    assert!(md.contains("|  | x |  |"), "{md}");
    assert!(md.contains("|  |  | z |"), "{md}");
}

#[test]
fn xls_1904_epoch_is_applied_once() {
    let xls = convert_bytes("xls", &fixture("date_1904.xls")).unwrap();
    let xlsx = convert_bytes("xlsx", &fixture("date_1904.xlsx")).unwrap();
    // The third row is an elapsed-time cell (`[h]:mm:ss`). The XLSX reader applies the format code and
    // shows the hours (255:10:10); calamine tells the XLS reader only that it is a date-time, so it
    // prints the date. The rows that are dates agree, which is what this test is about.
    let rows = |md: &str| md.lines().take(5).map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(rows(&xls), rows(&xlsx));
    assert!(xls.contains("| 2021-01-01 | 15 |"), "{xls}");
    assert!(xls.contains("| 2021-01-02 | 16 |"), "{xls}");
    assert!(xlsx.contains("| 255:10:10 | 17 |"), "{xlsx}");
}

#[test]
fn xls_through_the_document_cache_matches_convert_bytes() {
    let data = fixture("lo-sheets.xls");
    let expected = convert_bytes("xls", &data).unwrap();
    let mut out = Vec::new();
    convert_streaming("xls", &data, &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), expected);

    let mut cache = DocumentCache::new();
    let (mut cold, mut warm) = (Vec::new(), Vec::new());
    cache.convert(7, "xls", &data, &mut cold).unwrap();
    cache.convert(7, "xls", &data, &mut warm).unwrap();
    assert_eq!(String::from_utf8(cold).unwrap(), expected);
    assert_eq!(String::from_utf8(warm).unwrap(), expected);
}

#[test]
fn garbage_and_damaged_xls_are_errors_not_aborts() {
    assert!(convert_bytes("xls", b"definitely not a workbook, just text").is_err());
    assert!(convert_bytes("xls", b"").is_err());
    let data = fixture("lo-sheets.xls");
    // Every truncation (sampled) and every byte overwritten with 0x00 or 0xff (sampled) must return
    // a result, an error or a partial conversion. A damaged file must not abort the process: the
    // CFB reader's allocations are bounded by the file, and panics in the BIFF reader are caught.
    for cut in (0..data.len()).step_by(13) {
        let _ = convert_bytes("xls", &data[..cut]);
    }
    for at in (0..data.len()).step_by(31) {
        for value in [0x00u8, 0xff] {
            let mut corrupt = data.clone();
            corrupt[at] = value;
            let _ = convert_bytes("xls", &corrupt);
        }
    }
}

/// An XLS cell is shown the way its number format shows it, as the XLSX reader does: currency with its
/// symbol, thousands separators and fixed decimals. The reader printed the stored number (`500`) (webfold-bench formats-fidelity-v1).
#[test]
fn xls_cells_use_their_number_format() {
    let md = convert_bytes("xls", &fixture("euses-cs101-bad-dunn-q1.xls")).unwrap();
    assert!(md.contains("$500.00"), "{md}");
    assert!(md.contains("$8.25"), "{md}");
}
