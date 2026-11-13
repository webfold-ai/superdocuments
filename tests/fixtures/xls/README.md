# Legacy XLS fixtures

| File | Source | Licence |
| --- | --- | --- |
| `lo-sheets.xls` | Written by LibreOffice (headless `--convert-to xls`) from a workbook generated with openpyxl: three sheets (`Summary`, `Dates`, `Gaps Two`) with numbers, a boolean, a formula, error values, date and time formats and leading empty cells. | Apache-2.0, as the rest of this repository |
| `lo-sheets.xlsx` | LibreOffice round trip of `lo-sheets.xls` to `.xlsx`, so formula results are cached values. Used only as the reference for `lo-sheets.xls`. | Apache-2.0 |
| `date_1904.xls` | Copied from `calamine/tests/date_1904.xls` (calamine 0.36.1). A workbook using the 1904 date epoch. | MIT (calamine, `calamine/Cargo.toml`) |
| `date_1904.xlsx` | Copied from `calamine/tests/date_1904.xlsx`. The same content as `date_1904.xls`. | MIT (calamine) |
| `number-formats.xls` | Written by LibreOffice (headless `--convert-to xls`) from an openpyxl workbook with one number per row under the formats `0.0`, `0.00`, `#,##0.00`, `0%`, `0.0%`, `#,##0;(#,##0)`, `$#,##0.00`, `General`, `0`, `[h]:mm`, `mmm-yyyy` and `000` (including -0.04 and 2.675). | Apache-2.0 |
| `number-formats.xlsx` | LibreOffice round trip of `number-formats.xls` to `.xlsx`: the reference for the XLS reader. | Apache-2.0 |
