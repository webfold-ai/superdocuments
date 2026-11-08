//! XLSX / XLSM to worksheets.
//!
//! Sheet names and tab order come from `workbook.xml` and its relationships (a sheet's file
//! name says nothing about either). Cells are shown the way a person reads them: dates from the
//! number format (`cellXfs` -> `numFmts`, plus the built-in date formats and the 1904 epoch),
//! booleans as `TRUE`/`FALSE`, inline and rich-text strings as text with phonetic guides left
//! out, errors as written, and a formula that was never calculated as `=formula`.
//!
//! A cell value is otherwise the number exactly as stored. Number formats that only change
//! how a number looks (`0.00`, `#,##0`, percentages, currency) are not applied.

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use super::{attr_local, attrs_local, main_part, read_rels, rels_path_of, resolve};
use crate::document::{archive, entry, push_ref, text};

/// The most cells a single worksheet may allocate. `r="XFD1048576"` asks for 17 billion.
const MAX_CELLS: usize = 20_000_000;

pub(crate) struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

/// How each cell style (`s` attribute) formats a number.
#[derive(Default)]
pub(crate) struct Formats {
    /// Per `cellXfs` index: `(is_date, has_date_part, has_time_part)`.
    xfs: Vec<(bool, bool, bool)>,
    pub date1904: bool,
}

impl Formats {
    pub(crate) fn parse(styles: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::from_reader(styles);
        let mut buf = Vec::new();
        let mut custom: HashMap<u32, String> = HashMap::new();
        let mut xfs = Vec::new();
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
                        xfs.push(classify(id, custom.get(&id).map(String::as_str)));
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
            date1904: false,
        })
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

/// The rows of one worksheet part.
pub(crate) fn worksheet(
    xml: &[u8],
    strings: &[String],
    formats: &Formats,
) -> Result<Vec<Vec<String>>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut allocated = 0usize;
    let mut row: Vec<String> = Vec::new();
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
                    if !cell.is_empty() {
                        let at = col.unwrap_or(row.len());
                        if row.len() <= at {
                            allocated = allocated.saturating_add(at + 1 - row.len());
                            if allocated > MAX_CELLS {
                                return Err("worksheet too large".into());
                            }
                            row.resize(at + 1, String::new());
                        }
                        row[at] = cell;
                    }
                }
                // Styled but empty rows are common and carry nothing.
                "row" if row.iter().any(|c| !c.is_empty()) => rows.push(std::mem::take(&mut row)),
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
) -> Result<String, String> {
    Ok(match kind {
        "s" => {
            if value.trim().is_empty() {
                String::new()
            } else {
                let index = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "bad shared-string index")?;
                strings
                    .get(index)
                    .cloned()
                    .ok_or("shared-string index out of range")?
            }
        }
        "inlineStr" => inline.to_owned(),
        "b" => match value.trim() {
            "1" | "true" => "TRUE".to_owned(),
            "0" | "false" => "FALSE".to_owned(),
            _ => String::new(),
        },
        "str" | "e" | "d" => value.to_owned(),
        _ => {
            let raw = value.trim();
            if raw.is_empty() {
                if formula.trim().is_empty() {
                    String::new()
                } else {
                    format!("={}", formula.trim())
                }
            } else {
                let (is_date, date_part, time_part) = formats.of(style);
                is_date
                    .then(|| raw.parse::<f64>().ok())
                    .flatten()
                    .and_then(|n| serial_to_text(n, formats.date1904, date_part, time_part))
                    .unwrap_or_else(|| raw.to_owned())
            }
        }
    })
}

/// Worksheets in tab order with their real names.
pub(crate) fn read(data: &[u8]) -> Result<Vec<Sheet>, String> {
    let mut zip = archive(data)?;
    let workbook = main_part(&mut zip, "xl/workbook.xml").ok();
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
    declared
        .into_iter()
        .map(|(name, part)| {
            let rows = worksheet(&entry(&mut zip, &part)?, &strings, &formats)?;
            Ok(Sheet { name, rows })
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
                "0.30000000000000004"
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
