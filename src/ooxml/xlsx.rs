//! XLSX / XLSM to worksheets.
//!
//! Sheet names and tab order come from `workbook.xml` and its relationships (a sheet's file
//! name says nothing about either). Cells are shown the way a person reads them: dates from the
//! number format (`cellXfs` -> `numFmts`, plus the built-in date formats and the 1904 epoch),
//! booleans as `TRUE`/`FALSE`, inline and rich-text strings as text with phonetic guides left
//! out, errors as written, and a formula that was never calculated as `=formula`.
//!
//! A number is shown the way its format shows it. Plain formats (a fixed number of decimals, a
//! thousands separator, a percentage, a currency symbol or other text around the number: `0.00`,
//! `#,##0.0`, `0.0%`, `$#,##0.00`, `"EUR "0`) go through `format_number` below. Everything else
//! `numfmt` can read (General, sections, date and time codes such as `mmm-yyyy`) is applied
//! after Excel's rounding of a stored double to 15 digits. A format neither implements
//! (scientific, fractions, conditions) leaves the number as stored and a date in ISO form.
//! `read_typed` returns each cell's typed value next to its display.

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use super::{attr_local, attrs_local, main_part, read_rels, rels_path_of, require_root, resolve};
use crate::document::{archive, entry, push_ref, text};
use crate::xlsx::{CellValue, TypedCell};

/// The most cells a single worksheet may allocate. `r="XFD1048576"` asks for 17 billion.
const MAX_CELLS: usize = 20_000_000;

pub(crate) struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

pub(crate) struct TypedSheet {
    pub name: String,
    pub rows: Vec<Vec<TypedCell>>,
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
pub(super) fn civil_date(days: i64) -> (i64, u32, u32) {
    civil(days)
}

/// How each cell style (`s` attribute) formats a number.
#[derive(Default)]
pub(crate) struct Formats {
    /// Per `cellXfs` index: `(is_date, has_date_part, has_time_part)`.
    xfs: Vec<(bool, bool, bool)>,
    /// Per `cellXfs` index: the format code, when it is custom or a locale-independent built-in.
    codes: Vec<Option<String>>,
    /// Per `cellXfs` index: the plain number format, when it has one.
    numbers: Vec<Option<NumberFormat>>,
    pub date1904: bool,
}

/// A plain number format: what `format_number` can reproduce exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NumberFormat {
    min_decimals: usize,
    max_decimals: usize,
    min_integer_digits: usize,
    thousands: bool,
    percent: bool,
    prefix: String,
    suffix: String,
}

/// The built-in number formats that are plain (ids 1-4, 9, 10).
fn builtin_number_format(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        _ => return None,
    })
}

/// Parses the first section of a format code. `None` when the code is not a plain number format.
fn parse_number_format(code: &str) -> Option<NumberFormat> {
    let mut spec = String::new();
    let (mut prefix, mut suffix) = (String::new(), String::new());
    let mut seen_spec = false;
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        let literal = match c {
            ';' => break,
            '"' => {
                let mut text = String::new();
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    text.push(q);
                }
                text
            }
            '\\' => chars.next().map(String::from).unwrap_or_default(),
            '_' => {
                chars.next();
                " ".to_owned()
            }
            '*' => {
                chars.next();
                String::new()
            }
            '[' => {
                let mut inner = String::new();
                for q in chars.by_ref() {
                    if q == ']' {
                        break;
                    }
                    inner.push(q);
                }
                match inner.strip_prefix('$') {
                    Some(currency) => currency.split('-').next().unwrap_or("").to_owned(),
                    None if inner.chars().all(|q| q.is_ascii_alphabetic()) => String::new(), // [Red]
                    None => return None,
                }
            }
            '0' | '#' | '?' | ',' | '.' | '%' => {
                spec.push(c);
                seen_spec = true;
                continue;
            }
            // scientific, fractions, General, text and date codes are not plain number formats
            c if c.is_ascii_alphabetic() || c == '/' || c == '@' => return None,
            other => other.to_string(),
        };
        if seen_spec {
            suffix.push_str(&literal);
        } else {
            prefix.push_str(&literal);
        }
    }
    if !spec.chars().any(|c| matches!(c, '0' | '#' | '?')) {
        return None;
    }
    let percent = spec.contains('%');
    let spec = spec.replace('%', "");
    let (int_part, dec_part) = match spec.split_once('.') {
        Some((i, d)) => (i.to_owned(), d.to_owned()),
        None => (spec.clone(), String::new()),
    };
    // a comma after the last digit placeholder scales by 1000
    if int_part.ends_with(',') && dec_part.is_empty() || dec_part.contains(',') {
        return None;
    }
    if dec_part.chars().any(|c| c == ',' || c == '.') || int_part.contains('.') {
        return None;
    }
    // Mandatory digits: in the integer part everything from the leftmost `0` to the end (`0#` shows
    // 1 as `01`), in the decimals everything up to the last `0` or `?`.
    let placeholders: Vec<char> = int_part.chars().filter(|c| *c != ',').collect();
    let min_integer_digits = placeholders
        .iter()
        .position(|c| *c == '0')
        .map_or(0, |first| placeholders.len() - first);
    let min_decimals = dec_part.rfind(['0', '?']).map_or(0, |last| last + 1);
    Some(NumberFormat {
        min_decimals,
        max_decimals: dec_part.len(),
        min_integer_digits,
        thousands: int_part.contains(','),
        percent,
        prefix,
        suffix,
    })
}

/// A number as its plain format shows it. `None` when the value is not finite.
fn format_number(value: f64, format: &NumberFormat) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let value = if format.percent { value * 100.0 } else { value };
    let mut text = format!("{:.*}", format.max_decimals, value.abs());
    if format.max_decimals > format.min_decimals {
        while text.ends_with('0')
            && text.len() - text.find('.').unwrap_or(text.len()) - 1 > format.min_decimals
        {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    let (integer, decimals) = match text.split_once('.') {
        Some((i, d)) => (i.to_owned(), Some(d.to_owned())),
        None => (text, None),
    };
    let integer = format!(
        "{integer:0>width$}",
        width = format.min_integer_digits.max(1)
    );
    let integer = if format.thousands {
        let mut grouped = String::new();
        for (i, c) in integer.chars().enumerate() {
            if i > 0 && (integer.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        grouped
    } else {
        integer
    };
    let negative = value < 0.0
        && format!("{:.*}", format.max_decimals, value.abs())
            .chars()
            .any(|c| c.is_ascii_digit() && c != '0');
    Some(format!(
        "{}{}{}{}{}{}",
        if negative { "-" } else { "" },
        format.prefix,
        integer,
        decimals.map(|d| format!(".{d}")).unwrap_or_default(),
        if format.percent { "%" } else { "" },
        format.suffix
    ))
}

impl Formats {
    pub(crate) fn parse(styles: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::from_reader(styles);
        let mut buf = Vec::new();
        let mut custom: HashMap<u32, String> = HashMap::new();
        let mut xfs = Vec::new();
        let mut codes = Vec::new();
        let mut numbers = Vec::new();
        let mut in_cell_xfs = false;
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attr_local(&tag, "numFmtId").and_then(|v| v.parse().ok()),
                            attr_local(&tag, "formatCode"),
                        ) {
                            custom.insert(id, code);
                        }
                    }
                    "cellXfs" => in_cell_xfs = true,
                    "xf" if in_cell_xfs => {
                        let id: u32 = attr_local(&tag, "numFmtId")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        let code = custom.get(&id).map(String::as_str);
                        let classified = classify(id, code);
                        numbers.push(if classified.0 {
                            None
                        } else {
                            code.or_else(|| builtin_number_format(id))
                                .and_then(parse_number_format)
                        });
                        xfs.push(classified);
                        codes.push(
                            custom
                                .get(&id)
                                .cloned()
                                .or_else(|| super::numfmt::builtin(id).map(str::to_owned)),
                        );
                    }
                    _ => {}
                },
                Event::End(tag) if tag.local_name().as_ref() == "cellXfs" => in_cell_xfs = false,
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(Self {
            xfs,
            codes,
            numbers,
            date1904: false,
        })
    }

    /// What the cell style shows for a number; `None` when the style has no usable format code.
    fn display(&self, style: Option<usize>, value: f64) -> Option<String> {
        // A plain number format first; the other formats `numfmt` reads second.
        if let Some(format) = self.number(style) {
            if let Some(shown) = format_number(value, format) {
                return Some(shown);
            }
        }
        // No style (or style 0 with no code) is the General format.
        let code = match style {
            Some(s) => self.codes.get(s)?.as_deref()?,
            None => "General",
        };
        super::numfmt::format_value(value, code, self.date1904)
    }

    fn number(&self, style: Option<usize>) -> Option<&NumberFormat> {
        style
            .and_then(|s| self.numbers.get(s))
            .and_then(Option::as_ref)
    }

    fn of(&self, style: Option<usize>) -> (bool, bool, bool) {
        style
            .and_then(|s| self.xfs.get(s))
            .copied()
            .unwrap_or((false, false, false))
    }
}

/// `(is_date, has_date_part, has_time_part)` for a number format.
fn classify(id: u32, code: Option<&str>) -> (bool, bool, bool) {
    match (id, code) {
        (14..=17, None) | (27..=31, None) | (34..=36, None) | (50..=58, None) => {
            (true, true, false)
        }
        (18..=21, None) | (45..=47, None) => (true, false, true),
        (22, None) => (true, true, true),
        (_, None) => (false, false, false),
        (_, Some(code)) => {
            // Drop quoted text, [colour]/[condition] blocks and escaped characters.
            let mut plain = String::new();
            let mut chars = code.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        for q in chars.by_ref() {
                            if q == '"' {
                                break;
                            }
                        }
                    }
                    '[' => {
                        let mut inner = String::new();
                        for q in chars.by_ref() {
                            if q == ']' {
                                break;
                            }
                            inner.push(q);
                        }
                        // [h] [mm] [ss] are elapsed-time markers.
                        if matches!(
                            inner.to_lowercase().as_str(),
                            "h" | "hh" | "m" | "mm" | "s" | "ss"
                        ) {
                            plain.push('h');
                        }
                    }
                    '\\' | '_' | '*' => {
                        chars.next();
                    }
                    other => plain.push(other.to_ascii_lowercase()),
                }
            }
            let date = plain.contains('y') || plain.contains('d');
            let time = plain.contains('h') || plain.contains('s');
            let month = plain.contains('m') && !plain.contains(['0', '#', '?']);
            let is_date = date || time || month;
            (is_date, date || month && !time, time)
        }
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// A serial date number as `YYYY-MM-DD`, `HH:MM:SS` or both. `None` for a value that is not a
/// plausible serial (negative, not finite, or beyond year 9999).
pub(crate) fn serial_to_text(
    serial: f64,
    date1904: bool,
    date_part: bool,
    time_part: bool,
) -> Option<String> {
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut seconds = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if seconds >= 86_400 {
        days += 1;
        seconds -= 86_400;
    }
    // Excel counts a non-existent 1900-02-29, so serials from 61 on are offset by one more day.
    let epoch = if date1904 {
        -24_107 // 1904-01-01
    } else if days < 61 {
        -25_568 // 1899-12-31
    } else {
        -25_569 // 1899-12-30
    };
    let (year, month, day) = civil(days + epoch);
    let date = format!("{year:04}-{month:02}-{day:02}");
    let time = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    );
    Some(match (date_part, time_part) {
        (_, false) => date,
        (false, true) if days == 0 => time,
        (_, true) => format!("{date} {time}"),
    })
}

/// Shared strings: plain `t` and rich-text runs, never the phonetic `rPh` text.
pub(crate) fn shared_strings(xml: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let (mut strings, mut current) = (Vec::new(), String::new());
    let (mut in_si, mut in_t, mut phonetic) = (false, false, 0usize);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) => match tag.local_name().as_ref() {
                "si" => {
                    in_si = true;
                    current.clear();
                }
                "rPh" => phonetic += 1,
                "t" if in_si && phonetic == 0 => in_t = true,
                _ => {}
            },
            Event::Empty(tag) if tag.local_name().as_ref() == "si" => strings.push(String::new()),
            Event::Text(value) if in_t => current.push_str(&text(value.as_ref())),
            Event::GeneralRef(r) if in_t => push_ref(&mut current, &r),
            Event::End(tag) => match tag.local_name().as_ref() {
                "t" => in_t = false,
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "si" => {
                    strings.push(std::mem::take(&mut current));
                    in_si = false;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(strings)
}

pub(crate) fn column(reference: &str) -> Option<usize> {
    let mut value = 0usize;
    let mut letters = 0;
    for byte in reference.bytes().take_while(u8::is_ascii_alphabetic) {
        letters += 1;
        value = value
            .checked_mul(26)?
            .checked_add((byte.to_ascii_uppercase() - b'A' + 1) as usize)?;
    }
    (letters > 0)
        .then_some(value)?
        .checked_sub(1)
        .filter(|v| *v < 16_384)
}

/// The rows of one worksheet part, as displayed (the tests compare against plain strings).
#[cfg(test)]
pub(crate) fn worksheet(
    xml: &[u8],
    strings: &[String],
    formats: &Formats,
) -> Result<Vec<Vec<String>>, String> {
    Ok(worksheet_typed(xml, strings, formats)?
        .into_iter()
        .map(|row| row.into_iter().map(|c| c.display).collect())
        .collect())
}

/// The rows of one worksheet part: each cell's display and typed value.
pub(crate) fn worksheet_typed(
    xml: &[u8],
    strings: &[String],
    formats: &Formats,
) -> Result<Vec<Vec<TypedCell>>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut rows: Vec<Vec<TypedCell>> = Vec::new();
    let mut allocated = 0usize;
    let mut row: Vec<TypedCell> = Vec::new();
    let mut kind = String::new();
    let mut style: Option<usize> = None;
    let mut col: Option<usize> = None;
    let (mut value, mut formula, mut inline) = (String::new(), String::new(), String::new());
    let (mut in_v, mut in_f, mut in_t, mut in_is, mut phonetic, mut in_c) =
        (false, false, false, false, 0usize, false);
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::Start(tag) => match tag.local_name().as_ref() {
                "row" => row.clear(),
                "c" => {
                    in_c = true;
                    kind = attr_local(&tag, "t").unwrap_or_default();
                    style = attr_local(&tag, "s").and_then(|v| v.parse().ok());
                    col = attr_local(&tag, "r").and_then(|r| column(&r));
                    value.clear();
                    formula.clear();
                    inline.clear();
                }
                "v" if in_c => in_v = true,
                "f" if in_c => in_f = true,
                "is" if in_c => in_is = true,
                "rPh" => phonetic += 1,
                "t" if in_is && phonetic == 0 => in_t = true,
                _ => {}
            },
            Event::Text(t) => {
                let s = text(t.as_ref());
                if in_v {
                    value.push_str(&s);
                } else if in_f {
                    formula.push_str(&s);
                } else if in_t {
                    inline.push_str(&s);
                }
            }
            Event::GeneralRef(r) => {
                let target = if in_v {
                    Some(&mut value)
                } else if in_f {
                    Some(&mut formula)
                } else if in_t {
                    Some(&mut inline)
                } else {
                    None
                };
                if let Some(target) = target {
                    push_ref(target, &r);
                }
            }
            Event::End(tag) => match tag.local_name().as_ref() {
                "v" => in_v = false,
                "f" => in_f = false,
                "t" => in_t = false,
                "is" => in_is = false,
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "c" => {
                    in_c = false;
                    let cell =
                        cell_text(&kind, style, &value, &inline, &formula, strings, formats)?;
                    if !cell.display.is_empty() {
                        let at = col.unwrap_or(row.len());
                        if row.len() <= at {
                            allocated = allocated.saturating_add(at + 1 - row.len());
                            if allocated > MAX_CELLS {
                                return Err("worksheet too large".into());
                            }
                            row.resize(at + 1, TypedCell::empty());
                        }
                        row[at] = cell;
                    }
                }
                // Styled but empty rows are common and carry nothing.
                "row" if row.iter().any(|c| !c.display.is_empty()) => {
                    rows.push(std::mem::take(&mut row))
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(rows)
}

fn cell_text(
    kind: &str,
    style: Option<usize>,
    value: &str,
    inline: &str,
    formula: &str,
    strings: &[String],
    formats: &Formats,
) -> Result<TypedCell, String> {
    let text_cell = |t: String| TypedCell {
        value: if t.is_empty() {
            CellValue::Empty
        } else {
            CellValue::Text(t.clone())
        },
        display: t,
    };
    Ok(match kind {
        "s" => {
            if value.trim().is_empty() {
                TypedCell::empty()
            } else {
                let index = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "bad shared-string index")?;
                text_cell(
                    strings
                        .get(index)
                        .cloned()
                        .ok_or("shared-string index out of range")?,
                )
            }
        }
        "inlineStr" => text_cell(inline.to_owned()),
        "b" => match value.trim() {
            "1" | "true" => TypedCell {
                display: "TRUE".to_owned(),
                value: CellValue::Bool(true),
            },
            "0" | "false" => TypedCell {
                display: "FALSE".to_owned(),
                value: CellValue::Bool(false),
            },
            _ => TypedCell::empty(),
        },
        "e" => TypedCell {
            display: value.to_owned(),
            value: if value.is_empty() {
                CellValue::Empty
            } else {
                CellValue::Error(value.to_owned())
            },
        },
        "str" | "d" => text_cell(value.to_owned()),
        _ => {
            let raw = value.trim();
            if raw.is_empty() {
                if formula.trim().is_empty() {
                    TypedCell::empty()
                } else {
                    TypedCell {
                        display: format!("={}", formula.trim()),
                        value: CellValue::Formula(formula.trim().to_owned()),
                    }
                }
            } else {
                let (is_date, date_part, time_part) = formats.of(style);
                let number = raw.parse::<f64>().ok();
                let iso = is_date
                    .then_some(number)
                    .flatten()
                    .and_then(|n| serial_to_text(n, formats.date1904, date_part, time_part));
                let shown = number.and_then(|n| formats.display(style, n));
                match (number, iso) {
                    (Some(serial), Some(iso)) => TypedCell {
                        display: shown.unwrap_or_else(|| iso.clone()),
                        value: CellValue::DateTime { serial, iso },
                    },
                    (Some(n), None) => TypedCell {
                        display: shown.unwrap_or_else(|| raw.to_owned()),
                        value: CellValue::Number {
                            value: n,
                            raw: raw.to_owned(),
                        },
                    },
                    (None, _) => TypedCell {
                        display: raw.to_owned(),
                        value: CellValue::Text(raw.to_owned()),
                    },
                }
            }
        }
    })
}

/// Worksheets in tab order with their real names.
pub(crate) fn read(data: &[u8]) -> Result<Vec<Sheet>, String> {
    Ok(read_typed(data)?
        .into_iter()
        .map(|sheet| Sheet {
            name: sheet.name,
            rows: sheet
                .rows
                .into_iter()
                .map(|row| row.into_iter().map(|c| c.display).collect())
                .collect(),
        })
        .collect())
}

/// Worksheets in tab order with typed cells.
pub(crate) fn read_typed(data: &[u8]) -> Result<Vec<TypedSheet>, String> {
    let mut zip = archive(data)?;
    let workbook = main_part(&mut zip, "xl/workbook.xml").ok();
    if let Some(workbook) = workbook.as_deref() {
        require_root(&mut zip, workbook, "workbook", "XLSX")?;
    }
    let mut declared: Vec<(String, String)> = Vec::new(); // (name, part)
    let mut formats = Formats::default();
    let mut strings_part = Some("xl/sharedStrings.xml".to_owned());
    let mut styles_part = Some("xl/styles.xml".to_owned());
    if let Some(workbook) = workbook.as_deref() {
        let rels = read_rels(&mut zip, &rels_path_of(workbook))?;
        let by_kind = |kind: &str| {
            rels.iter()
                .find(|r| r.kind == kind && !r.external)
                .and_then(|r| resolve(workbook, &r.target))
        };
        strings_part = by_kind("sharedStrings").or(strings_part);
        styles_part = by_kind("styles").or(styles_part);
        let xml = entry(&mut zip, workbook)?;
        let mut reader = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        let mut date1904 = false;
        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| e.to_string())?
            {
                Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                    "workbookPr" => {
                        date1904 =
                            attr_local(&tag, "date1904").is_some_and(|v| v == "1" || v == "true");
                    }
                    "sheet" => {
                        let ids = attrs_local(&tag, "id");
                        if let Some(part) = rels
                            .iter()
                            .find(|r| r.kind == "worksheet" && !r.external && ids.contains(&r.id))
                            .and_then(|r| resolve(workbook, &r.target))
                        {
                            declared.push((attr_local(&tag, "name").unwrap_or_default(), part));
                        }
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        formats.date1904 = date1904;
    }
    if let Some(part) = styles_part.filter(|p| zip.by_name(p).is_ok()) {
        let date1904 = formats.date1904;
        formats = Formats::parse(&entry(&mut zip, &part)?)?;
        formats.date1904 = date1904;
    }
    let strings = match strings_part.filter(|p| zip.by_name(p).is_ok()) {
        Some(part) => shared_strings(&entry(&mut zip, &part)?)?,
        None => Vec::new(),
    };
    if declared.is_empty() {
        // No usable workbook part: the numbers in the file names are all there is.
        let mut numbered = Vec::new();
        for i in 0..zip.len() {
            let name = zip
                .by_index_raw(i)
                .map_err(|e| e.to_string())?
                .name()
                .map_err(|e| e.to_string())?
                .to_string();
            if let Some(n) = name
                .strip_prefix("xl/worksheets/sheet")
                .and_then(|s| s.strip_suffix(".xml"))
                .and_then(|s| s.parse::<usize>().ok())
            {
                numbered.push((n, name));
            }
        }
        numbered.sort();
        declared = numbered
            .into_iter()
            .map(|(n, part)| (format!("Sheet {n}"), part))
            .collect();
    }
    if declared.is_empty() {
        // A workbook has at least one sheet (ECMA-376 Part 1, 18.2.20 `sheets`).
        return Err("not an XLSX file: the package has no worksheets".into());
    }
    declared
        .into_iter()
        .map(|(name, part)| {
            let rows = worksheet_typed(&entry(&mut zip, &part)?, &strings, &formats)?;
            Ok(TypedSheet { name, rows })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(serial: f64) -> String {
        serial_to_text(serial, false, true, false).expect("date")
    }

    #[test]
    fn serial_dates_match_excel() {
        assert_eq!(date(1.0), "1900-01-01");
        assert_eq!(date(59.0), "1900-02-28");
        assert_eq!(date(61.0), "1900-03-01");
        assert_eq!(date(45000.0), "2023-03-15");
        assert_eq!(
            serial_to_text(0.0, true, true, false).as_deref(),
            Some("1904-01-01")
        );
        assert_eq!(
            serial_to_text(45000.75, false, true, true).as_deref(),
            Some("2023-03-15 18:00:00")
        );
        assert_eq!(
            serial_to_text(0.5, false, false, true).as_deref(),
            Some("12:00:00")
        );
        assert_eq!(serial_to_text(-1.0, false, true, false), None);
        assert_eq!(serial_to_text(f64::NAN, false, true, false), None);
        // Rounding up to the next midnight carries into the date.
        assert_eq!(
            serial_to_text(45_000.999_999_99, false, true, true).as_deref(),
            Some("2023-03-16 00:00:00")
        );
    }

    #[test]
    fn date_formats_are_told_from_number_formats() {
        assert!(classify(14, None).0);
        assert!(classify(22, None).2);
        assert!(!classify(0, None).0);
        assert!(!classify(2, None).0);
        assert!(classify(164, Some("yyyy-mm-dd")).0);
        assert!(classify(165, Some("[$-409]d-mmm-yy;@")).0);
        assert!(classify(166, Some("hh:mm:ss")).2);
        assert!(classify(167, Some("[h]:mm")).2);
        assert!(!classify(168, Some("0.00")).0);
        assert!(
            !classify(169, Some("#,##0.00\"d\"")).0,
            "text in quotes is not a date token"
        );
        assert!(!classify(170, Some("0.0%")).0);
        assert!(!classify(171, Some("[Red]0.00")).0);
    }

    fn sheet(xml: &str, strings: &[&str]) -> Vec<Vec<String>> {
        let strings: Vec<String> = strings.iter().map(|s| (*s).to_owned()).collect();
        worksheet(xml.as_bytes(), &strings, &Formats::default()).expect("sheet")
    }

    #[test]
    fn cell_kinds() {
        let rows = sheet(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1" t="inlineStr"><is><r><t>in</t></r><r><t>line</t></r><rPh><t>ignored</t></rPh></is></c>
                 <c r="B1" t="b"><v>1</v></c>
                 <c r="C1" t="e"><v>#DIV/0!</v></c>
                 <c r="D1" t="str"><f>A1&amp;"x"</f><v>inlinex</v></c>
                 <c r="E1"><f>SUM(B2:B3)</f></c>
                 <c r="F1" s="9"/>
                 <c r="G1"><v>0.30000000000000004</v></c>
               </row><row r="2"><c r="A2" s="3"/></row></sheetData></worksheet>"#,
            &[],
        );
        assert_eq!(
            rows,
            vec![vec![
                "inline",
                "TRUE",
                "#DIV/0!",
                "inlinex",
                "=SUM(B2:B3)",
                "",
                // General shows 15 significant digits, as Excel does; the stored text stays
                // available through the typed API (`CellValue::Number::raw`)
                "0.3"
            ]]
        );
    }

    #[test]
    fn shared_strings_are_rich_text_without_phonetic_guides() {
        let strings = shared_strings(
            br#"<sst><si><t>plain</t></si><si><r><t>ri</t></r><r><rPr><b/></rPr><t>ch</t></r><rPh sb="0" eb="1"><t>reading</t></rPh></si><si/></sst>"#,
        )
        .unwrap();
        assert_eq!(strings, vec!["plain", "rich", ""]);
    }

    #[test]
    fn columns_past_xfd_and_huge_gaps_are_not_misplaced_or_unbounded() {
        let rows = sheet(
            r#"<worksheet><sheetData><row><c r="A1"><v>1</v></c><c r="XFE1"><v>2</v></c></row></sheetData></worksheet>"#,
            &[],
        );
        assert_eq!(
            rows,
            vec![vec!["1", "2"]],
            "a column beyond XFD is appended, not dropped"
        );
        let many = format!(
            "<worksheet><sheetData>{}</sheetData></worksheet>",
            (0..2000)
                .map(|r| format!(r#"<row><c r="XFD{r}"><v>1</v></c></row>"#))
                .collect::<String>()
        );
        let result = worksheet(many.as_bytes(), &[], &Formats::default());
        assert!(result.is_ok() || result.unwrap_err().contains("too large"));
    }

    #[test]
    fn style_sheet_marks_date_cells() {
        let formats = Formats::parse(
            br#"<styleSheet><numFmts><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts><cellXfs><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="14"/></cellXfs></styleSheet>"#,
        )
        .unwrap();
        let rows = worksheet(
            br#"<worksheet><sheetData><row><c r="A1" s="0"><v>45000</v></c><c r="B1" s="1"><v>45000</v></c><c r="C1" s="2"><v>45000</v></c></row></sheetData></worksheet>"#,
            &[],
            &formats,
        )
        .unwrap();
        assert_eq!(rows, vec![vec!["45000", "2023-03-15", "2023-03-15"]]);
    }
}

#[cfg(test)]
mod number_format_tests {
    use super::*;

    fn shown(code: &str, value: f64) -> Option<String> {
        format_number(value, &parse_number_format(code)?)
    }

    #[test]
    fn plain_number_formats_are_applied() {
        assert_eq!(shown("0.0%", 0.125).as_deref(), Some("12.5%"));
        assert_eq!(shown("0%", 0.5).as_deref(), Some("50%"));
        assert_eq!(shown("#,##0.00", 1234.5).as_deref(), Some("1,234.50"));
        assert_eq!(shown("#,##0", 1234567.0).as_deref(), Some("1,234,567"));
        assert_eq!(shown("0.00", 99.0).as_deref(), Some("99.00"));
        assert_eq!(shown("0.0#", 2.5).as_deref(), Some("2.5"));
        assert_eq!(shown("000", 7.0).as_deref(), Some("007"));
        // `0#` shows 1 as 01; a literal around the digits is kept (docling xlsx_07_gap_tolerance, LibreOffice shows 1.01)
        assert_eq!(shown("0#", 1.0).as_deref(), Some("01"));
        assert_eq!(shown("\\1\\.0#", 1.0).as_deref(), Some("1.01"));
        assert_eq!(shown("#,##0", 0.4).as_deref(), Some("0"));
        assert_eq!(shown("$#,##0.00", 1234.5).as_deref(), Some("$1,234.50"));
        assert_eq!(shown("\"EUR \"0.00", 3.0).as_deref(), Some("EUR 3.00"));
        assert_eq!(shown("[$€-407]#,##0.00", 10.0).as_deref(), Some("€10.00"));
        assert_eq!(shown("#,##0.00", -1234.5).as_deref(), Some("-1,234.50"));
        assert_eq!(shown("0.00;[Red]-0.00", 2.0).as_deref(), Some("2.00"));
    }

    #[test]
    fn other_formats_leave_the_number_alone() {
        for code in [
            "General",
            "0.00E+00",
            "# ?/?",
            "@",
            "yyyy-mm-dd",
            "#,##0,",
            "0.0,",
        ] {
            assert_eq!(parse_number_format(code), None, "{code}");
        }
    }

    #[test]
    fn cells_use_their_style_number_format() {
        let styles = br#"<styleSheet><numFmts><numFmt numFmtId="164" formatCode="0.0%"/><numFmt numFmtId="165" formatCode="yyyy-mm-dd"/></numFmts><cellXfs><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="4"/><xf numFmtId="165"/></cellXfs></styleSheet>"#;
        let formats = Formats::parse(styles).unwrap();
        let sheet = br#"<worksheet><sheetData><row r="1"><c r="A1" s="1"><v>0.125</v></c><c r="B1" s="2"><v>1234.5</v></c><c r="C1" s="3"><v>45000</v></c><c r="D1"><v>0.30000000000000004</v></c></row></sheetData></worksheet>"#;
        let rows = worksheet(sheet, &[], &formats).unwrap();
        assert_eq!(
            rows[0],
            // a General cell shows 15 significant digits, as Excel does (the stored text stays in
            // `CellValue::Number::raw`)
            ["12.5%", "1,234.50", "2023-03-15", "0.3"]
        );
    }
}
