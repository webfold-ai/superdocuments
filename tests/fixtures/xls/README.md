# Legacy XLS fixtures

| File | Source | Licence |
| --- | --- | --- |
| `lo-sheets.xls` | Written by LibreOffice (headless `--convert-to xls`) from a workbook generated with openpyxl: three sheets (`Summary`, `Dates`, `Gaps Two`) with numbers, a boolean, a formula, error values, date and time formats and leading empty cells. | Apache-2.0, as the rest of this repository |
| `lo-sheets.xlsx` | LibreOffice round trip of `lo-sheets.xls` to `.xlsx`, so formula results are cached values. Used only as the reference for `lo-sheets.xls`. | Apache-2.0 |
| `date_1904.xls` | Copied from `calamine/tests/date_1904.xls` (calamine 0.36.1). A workbook using the 1904 date epoch. | MIT (calamine, `calamine/Cargo.toml`) |
| `date_1904.xlsx` | Copied from `calamine/tests/date_1904.xlsx`. The same content as `date_1904.xls`. | MIT (calamine) |

`euses-cs101-bad-dunn-q1.xls` is a student spreadsheet from the EUSES corpus (Zenodo 581673, CC-BY-4.0): currency
(`$#,##0.00`) cells.
