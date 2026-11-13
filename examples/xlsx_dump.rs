//! `xlsx_dump FILE.xlsx` prints every sheet as CSV of the cells' display text (number formats applied), one file
//! per sheet written as `<out-dir>/<sheet>.csv` when `--out DIR` is given, otherwise `## sheet` sections to stdout.
//! `--values` prints the typed value of each cell instead (`n:4.0999999999999996`, `d:46266`, `t:text`).
use superdocuments::xlsx::{read_workbook_typed, CellValue};

fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = args
        .iter()
        .find(|a| {
            !a.starts_with("--")
                && args
                    .iter()
                    .position(|b| b == *a)
                    .is_none_or(|i| i == 0 || args[i - 1] != "--out")
        })
        .expect("usage: xlsx_dump FILE [--values] [--out DIR]");
    let values = args.iter().any(|a| a == "--values");
    let out_dir = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1));
    let sheets = read_workbook_typed(&std::fs::read(file).expect("read")).expect("xlsx");
    for sheet in sheets {
        let mut text = String::new();
        for row in &sheet.rows {
            let cells: Vec<String> = row
                .iter()
                .map(|c| {
                    csv(&if values {
                        match &c.value {
                            CellValue::Number { raw, .. } => format!("n:{raw}"),
                            CellValue::DateTime { serial, .. } => format!("d:{serial}"),
                            CellValue::Text(t) => format!("t:{t}"),
                            CellValue::Empty => String::new(),
                            other => format!("{other:?}"),
                        }
                    } else {
                        c.display.clone()
                    })
                })
                .collect();
            text.push_str(&cells.join(","));
            text.push('\n');
        }
        match out_dir {
            Some(dir) => std::fs::write(format!("{dir}/{}.csv", sheet.name), text).expect("write"),
            None => println!("## {}\n{text}", sheet.name),
        }
    }
}
