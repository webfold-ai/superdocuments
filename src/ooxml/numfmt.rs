//! Spreadsheet number formats: what a cell with a stored number *shows*.
//!
//! `format_value(value, code, date1904)` applies an Excel number-format code (`General`, `0.0`,
//! `#,##0.00`, `0%`, `mmm-yyyy`, `d/mm/yyyy h:mm AM/PM`, `[h]:mm`, sectioned `pos;neg;zero;text`
//! codes, quoted and escaped literals) to a number. Anything it does not implement (conditions,
//! fractions `# ?/?`, scientific `E+`, several digit runs, eras, locale-dependent built-ins)
//! returns `None`, and the caller keeps its previous output. Excel keeps 15 significant digits of
//! a stored double, so a value is rounded to 15 digits before it is shown ("General" included):
//! `4.0999999999999996` is `4.1`.

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The format code of a built-in number format id, when it does not depend on the user's locale.
pub(crate) fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        49 => "@",
        _ => return None,
    })
}

/// Apply a number format to a number. `None`: not supported, keep the caller's rendering.
pub(crate) fn format_value(value: f64, code: &str, date1904: bool) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let sections = split_sections(code);
    let n = sections.len();
    let (section, negative_shown_by_format) = if n == 1 {
        (sections[0].as_str(), false)
    } else if value < 0.0 {
        (sections[1].as_str(), true)
    } else if value == 0.0 && n >= 3 {
        (sections[2].as_str(), false)
    } else {
        (sections[0].as_str(), false)
    };
    let toks = tokenize(section)?;
    let is_date = toks.iter().any(Tok::is_date);
    let magnitude = if negative_shown_by_format {
        value.abs()
    } else {
        value
    };
    if is_date {
        if magnitude < 0.0 {
            return None;
        }
        return format_date(&toks, magnitude, date1904);
    }
    format_number(&toks, magnitude)
}

/// Split a code on `;` outside quotes and brackets; at most four sections are meaningful.
fn split_sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut quoted, mut bracket, mut escaped) = (false, false, false);
    for c in code.chars() {
        let last = out.last_mut().unwrap();
        if escaped {
            last.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' | '_' | '*' if !quoted => {
                escaped = true;
                last.push(c);
            }
            '"' => {
                quoted = !quoted;
                last.push(c);
            }
            '[' if !quoted => {
                bracket = true;
                last.push(c);
            }
            ']' if !quoted => {
                bracket = false;
                last.push(c);
            }
            ';' if !quoted && !bracket => out.push(String::new()),
            _ => last.push(c),
        }
    }
    out.truncate(4);
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    /// A digit placeholder: `0`, `#` or `?`.
    Digit(char),
    Point,
    Comma,
    Percent,
    General,
    /// y, m, d, h, s runs and their length; `A` is AM/PM (length 0 = `A/P`, 2 = `AM/PM`).
    Date(char, usize),
    /// `[h]`, `[m]`, `[s]`: elapsed units.
    Elapsed(char),
    Text,
}

impl Tok {
    fn is_date(&self) -> bool {
        matches!(self, Tok::Date(..) | Tok::Elapsed(_))
    }
}

fn tokenize(section: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = section.chars().collect();
    let mut toks: Vec<Tok> = Vec::new();
    let mut i = 0;
    let push_lit = |toks: &mut Vec<Tok>, s: &str| match toks.last_mut() {
        Some(Tok::Lit(prev)) => prev.push_str(s),
        _ => toks.push(Tok::Lit(s.to_string())),
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                push_lit(&mut toks, &s);
            }
            '\\' => {
                i += 1;
                if let Some(c) = chars.get(i) {
                    push_lit(&mut toks, &c.to_string());
                }
            }
            '_' => {
                i += 1;
                push_lit(&mut toks, " ");
            }
            '*' => i += 1, // repeat-fill: shown once, as nothing
            '[' => {
                let end = chars[i..].iter().position(|c| *c == ']')? + i;
                let inner: String = chars[i + 1..end].iter().collect();
                let lower = inner.to_lowercase();
                match lower.as_str() {
                    "h" | "hh" => toks.push(Tok::Elapsed('h')),
                    "m" | "mm" => toks.push(Tok::Elapsed('m')),
                    "s" | "ss" => toks.push(Tok::Elapsed('s')),
                    "black" | "blue" | "cyan" | "green" | "magenta" | "red" | "white"
                    | "yellow" => {}
                    _ if lower.starts_with("color") => {}
                    _ if inner.starts_with('$') => {
                        // [$€-407]: the currency symbol; a bare locale ([$-409]) shows nothing.
                        let symbol: String = inner[1..].split('-').next().unwrap_or("").to_string();
                        // [$-F800] / [$-F400] are the system date formats: not reproducible here.
                        if inner[1..].starts_with("-F") || inner[1..].starts_with("-f") {
                            return None;
                        }
                        push_lit(&mut toks, &symbol);
                    }
                    // conditions ([>100]) and anything else
                    _ => return None,
                }
                i = end;
            }
            '0' | '#' | '?' => toks.push(Tok::Digit(c)),
            '.' => toks.push(Tok::Point),
            ',' => toks.push(Tok::Comma),
            '%' => toks.push(Tok::Percent),
            '@' => toks.push(Tok::Text),
            'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => return None,
            'G' if chars[i..].iter().take(7).collect::<String>() == "General" => {
                toks.push(Tok::General);
                i += "General".len() - 1;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let l = c.to_ascii_lowercase();
                let mut n = 1;
                while chars.get(i + n).map(|x| x.to_ascii_lowercase()) == Some(l) {
                    n += 1;
                }
                toks.push(Tok::Date(l, n));
                i += n - 1;
            }
            'a' | 'A' => {
                let rest: String = chars[i..].iter().take(5).collect::<String>().to_uppercase();
                if rest.starts_with("AM/PM") {
                    toks.push(Tok::Date('A', 2));
                    i += 4;
                } else if rest.starts_with("A/P") {
                    toks.push(Tok::Date('A', 0));
                    i += 2;
                } else {
                    return None;
                }
            }
            // eras, calendars
            'b' | 'B' | 'e' | 'E' | 'g' | 'G' | 'n' | 'N' => return None,
            '/' => {
                // `# ?/?` fractions: a slash between digit placeholders.
                let prev_digit = matches!(toks.last(), Some(Tok::Digit(_)));
                let next_digit = matches!(chars.get(i + 1), Some('0' | '#' | '?'));
                if prev_digit && next_digit {
                    return None;
                }
                push_lit(&mut toks, "/");
            }
            other => push_lit(&mut toks, &other.to_string()),
        }
        i += 1;
    }
    Some(toks)
}

// ── numbers ─────────────────────────────────────────────────────────────────────────────────

/// `value` rounded to 15 significant digits, as `(digits, exponent)`: the value is
/// `0.d1d2d3... * 10^exponent`. Zero is `([], 0)`.
fn digits15(value: f64) -> (Vec<u8>, i32) {
    let abs = value.abs();
    if abs == 0.0 {
        return (Vec::new(), 0);
    }
    let s = format!("{abs:.14e}");
    let (mant, exp) = s.split_once('e').unwrap();
    let digits: Vec<u8> = mant
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exp.parse::<i32>().unwrap() + 1)
}

/// `|value|` with `decimals` places, rounded half away from zero on its 15-digit form:
/// `(integer digits, fraction digits)`, the integer part without leading zeros.
fn fixed(value: f64, decimals: usize) -> (String, String) {
    let (digits, exp) = digits15(value);
    // positions: digit k has weight 10^(exp-1-k). Build integer and fraction digit strings.
    let mut int: Vec<u8> = Vec::new();
    let mut frac: Vec<u8> = Vec::new();
    for (k, d) in digits.iter().enumerate() {
        let weight = exp - 1 - k as i32;
        if weight >= 0 {
            int.push(*d);
        } else {
            let at = (-weight - 1) as usize;
            while frac.len() < at {
                frac.push(0);
            }
            frac.push(*d);
        }
    }
    // integer digits missing at the right (value larger than the 15 digits kept)
    if exp > digits.len() as i32 {
        int.extend(std::iter::repeat_n(0, (exp - digits.len() as i32) as usize));
    }
    let round_up = frac.get(decimals).is_some_and(|d| *d >= 5);
    frac.truncate(decimals);
    while frac.len() < decimals {
        frac.push(0);
    }
    if round_up {
        let mut carry = true;
        for d in frac.iter_mut().rev() {
            if *d == 9 {
                *d = 0;
            } else {
                *d += 1;
                carry = false;
                break;
            }
        }
        if carry {
            for d in int.iter_mut().rev() {
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                int.insert(0, 1);
            }
        }
    }
    let to_s = |v: &[u8]| v.iter().map(|d| (b'0' + d) as char).collect::<String>();
    (to_s(&int).trim_start_matches('0').to_string(), to_s(&frac))
}

/// Excel's General format for a stored double: up to 15 significant digits, no trailing zeros.
fn general(value: f64) -> Option<String> {
    let abs = value.abs();
    if abs != 0.0 && !(1e-5..1e15).contains(&abs) {
        return None;
    }
    let (digits, exp) = digits15(value);
    if digits.is_empty() {
        return Some("0".into());
    }
    let mut digits = digits;
    while digits.last() == Some(&0) {
        digits.pop();
    }
    let ds: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let sign = if value < 0.0 { "-" } else { "" };
    let body = if exp <= 0 {
        format!("0.{}{}", "0".repeat((-exp) as usize), ds)
    } else if exp as usize >= ds.len() {
        format!("{}{}", ds, "0".repeat(exp as usize - ds.len()))
    } else {
        format!("{}.{}", &ds[..exp as usize], &ds[exp as usize..])
    };
    Some(format!("{sign}{body}"))
}

fn format_number(toks: &[Tok], value: f64) -> Option<String> {
    let percents = toks.iter().filter(|t| matches!(t, Tok::Percent)).count();
    // the single run of digit placeholders (with its separators)
    let first = toks
        .iter()
        .position(|t| matches!(t, Tok::Digit(_) | Tok::Point));
    let mut sign_prefix = "";
    let mut scaled = value;
    for _ in 0..percents {
        scaled *= 100.0;
    }
    if scaled < 0.0 {
        sign_prefix = "-";
    }
    let render_literals = |toks: &[Tok], num: &str| -> String {
        let mut out = String::new();
        let mut placed = false;
        for t in toks {
            match t {
                Tok::Lit(s) => out.push_str(s),
                Tok::Percent => out.push('%'),
                Tok::Digit(_) | Tok::Point | Tok::Comma if !placed => {
                    out.push_str(num);
                    placed = true;
                }
                Tok::Digit(_) | Tok::Point | Tok::Comma => {}
                Tok::General | Tok::Text if !placed => {
                    out.push_str(num);
                    placed = true;
                }
                Tok::General | Tok::Text => {}
                _ => {}
            }
        }
        out
    };
    let Some(first) = first else {
        // no placeholders: General / @ / pure literal sections
        if toks.iter().any(|t| matches!(t, Tok::General | Tok::Text)) {
            let g = general(scaled)?;
            return Some(render_literals(toks, &g));
        }
        return Some(render_literals(toks, ""));
    };
    // the run: placeholders, points and commas must be contiguous
    let mut end = first;
    while matches!(toks.get(end), Some(Tok::Digit(_) | Tok::Point | Tok::Comma)) {
        end += 1;
    }
    if toks[end..]
        .iter()
        .any(|t| matches!(t, Tok::Digit(_) | Tok::Point))
    {
        return None; // several digit runs (phone-number style masks)
    }
    let run = &toks[first..end];
    let point = run.iter().position(|t| matches!(t, Tok::Point));
    let (int_part, frac_part) = match point {
        Some(p) => (&run[..p], &run[p + 1..]),
        None => (run, &run[run.len()..]),
    };
    // trailing commas after the last placeholder scale by 1000 each
    let trailing = int_part
        .iter()
        .rev()
        .take_while(|t| matches!(t, Tok::Comma))
        .count()
        + if frac_part.is_empty() {
            0
        } else {
            frac_part
                .iter()
                .rev()
                .take_while(|t| matches!(t, Tok::Comma))
                .count()
        };
    let int_digits: Vec<char> = int_part
        .iter()
        .filter_map(|t| {
            if let Tok::Digit(c) = t {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    let frac_digits: Vec<char> = frac_part
        .iter()
        .filter_map(|t| {
            if let Tok::Digit(c) = t {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    if int_digits.contains(&'?') || frac_digits.contains(&'?') {
        return None;
    }
    let grouping = {
        let body = &int_part[..int_part.len()
            - int_part
                .iter()
                .rev()
                .take_while(|t| matches!(t, Tok::Comma))
                .count()];
        body.iter().any(|t| matches!(t, Tok::Comma))
    };
    let mut v = scaled.abs();
    for _ in 0..trailing {
        v /= 1000.0;
    }
    let (mut int_s, mut frac_s) = fixed(v, frac_digits.len());
    let zeros_required = int_digits.iter().filter(|c| **c == '0').count();
    while int_s.len() < zeros_required {
        int_s.insert(0, '0');
    }
    // '#' placeholders after the last '0' in the fraction drop trailing zeros
    let mut keep = frac_s.len();
    while keep > 0 && frac_digits[keep - 1] == '#' && frac_s.as_bytes()[keep - 1] == b'0' {
        keep -= 1;
    }
    frac_s.truncate(keep);
    if grouping {
        let mut grouped = String::new();
        for (k, ch) in int_s.chars().enumerate() {
            if k > 0 && (int_s.len() - k) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        int_s = grouped;
    }
    let mut num = int_s;
    if !frac_s.is_empty() || point.is_some() {
        num.push('.'); // Excel shows the point even when every optional decimal is dropped
        num.push_str(&frac_s);
    }
    Some(format!("{sign_prefix}{}", render_literals(toks, &num)))
}

// ── dates and times ─────────────────────────────────────────────────────────────────────────

fn format_date(toks: &[Tok], serial: f64, date1904: bool) -> Option<String> {
    if !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let has_ampm = toks.iter().any(|t| matches!(t, Tok::Date('A', _)));
    let fractional_seconds = toks.windows(2).any(|w| {
        matches!(w[0], Tok::Date('s', _)) && matches!(&w[1], Tok::Lit(s) if s.starts_with('.'))
    }) || toks.windows(3).any(|w| {
        matches!(w[0], Tok::Date('s', _))
            && matches!(w[1], Tok::Point)
            && matches!(w[2], Tok::Digit('0'))
    });
    if fractional_seconds {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if secs >= 86_400 {
        days += 1;
        secs -= 86_400;
    }
    let epoch = if date1904 {
        -24_107
    } else if days < 61 {
        -25_568
    } else {
        -25_569
    };
    let unix_days = days + epoch;
    let (year, month, day) = super::xlsx::civil_date(unix_days);
    let weekday = (unix_days + 4).rem_euclid(7) as usize;
    let (hour, minute, second) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let total_secs = days * 86_400 + secs;
    let mut out = String::new();
    // Minutes are `m` that follow an hour or precede seconds.
    let mut after_hour = false;
    for (k, t) in toks.iter().enumerate() {
        match t {
            Tok::Lit(s) => out.push_str(s),
            Tok::Date('y', n) => out.push_str(&if *n <= 2 {
                format!("{:02}", year.rem_euclid(100))
            } else {
                format!("{year:04}")
            }),
            Tok::Date('d', n) => match n {
                1 => out.push_str(&day.to_string()),
                2 => out.push_str(&format!("{day:02}")),
                3 => out.push_str(&DAYS[weekday][..3]),
                _ => out.push_str(DAYS[weekday]),
            },
            Tok::Date('h', n) => {
                let h = if has_ampm {
                    match hour % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    hour
                };
                out.push_str(&if *n == 1 {
                    h.to_string()
                } else {
                    format!("{h:02}")
                });
                after_hour = true;
            }
            Tok::Date('m', n) => {
                let next_is_seconds = toks[k + 1..]
                    .iter()
                    .find(|t| !matches!(t, Tok::Lit(_)))
                    .is_some_and(|t| matches!(t, Tok::Date('s', _)));
                if (after_hour || next_is_seconds) && *n <= 2 {
                    out.push_str(&if *n == 1 {
                        minute.to_string()
                    } else {
                        format!("{minute:02}")
                    });
                } else {
                    let name = MONTHS[(month - 1) as usize];
                    match n {
                        1 => out.push_str(&month.to_string()),
                        2 => out.push_str(&format!("{month:02}")),
                        3 => out.push_str(&name[..3]),
                        4 => out.push_str(name),
                        _ => out.push_str(&name[..1]),
                    }
                }
                after_hour = false;
            }
            Tok::Date('s', n) => {
                out.push_str(&if *n == 1 {
                    second.to_string()
                } else {
                    format!("{second:02}")
                });
                after_hour = false;
            }
            Tok::Date('A', n) => {
                let pm = hour >= 12;
                out.push_str(match (n, pm) {
                    (0, false) => "A",
                    (0, true) => "P",
                    (_, false) => "AM",
                    (_, true) => "PM",
                });
            }
            Tok::Elapsed('h') => {
                out.push_str(&(total_secs / 3600).to_string());
                after_hour = true;
            }
            Tok::Elapsed('m') => out.push_str(&(total_secs / 60).to_string()),
            Tok::Elapsed('s') => out.push_str(&total_secs.to_string()),
            Tok::Percent => out.push('%'),
            Tok::Comma => out.push(','),
            Tok::Point => out.push('.'),
            Tok::Digit(_) | Tok::General | Tok::Text | Tok::Date(..) | Tok::Elapsed(_) => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(value: f64, code: &str) -> Option<String> {
        format_value(value, code, false)
    }

    #[test]
    fn general_shows_fifteen_significant_digits() {
        // 4.1 and 4.0999999999999996 are the same double: the sheet XML stores the latter
        assert_eq!(f(4.1, "General").as_deref(), Some("4.1"));
        assert_eq!(f(0.30000000000000004, "General").as_deref(), Some("0.3"));
        assert_eq!(f(100.0, "General").as_deref(), Some("100"));
        assert_eq!(f(-0.000123, "General").as_deref(), Some("-0.000123"));
        assert_eq!(
            f(123456789012.0, "General").as_deref(),
            Some("123456789012")
        );
        assert_eq!(f(0.0, "General").as_deref(), Some("0"));
    }

    #[test]
    fn fixed_decimals_round_half_up_on_the_fifteen_digit_value() {
        assert_eq!(f(4.1, "0.0").as_deref(), Some("4.1"));
        assert_eq!(f(2.675, "0.00").as_deref(), Some("2.68"));
        assert_eq!(f(1.005, "0.00").as_deref(), Some("1.01"));
        assert_eq!(f(0.5, "0").as_deref(), Some("1"));
        assert_eq!(f(999.996, "0.00").as_deref(), Some("1000.00"));
        assert_eq!(f(0.05, "0.0").as_deref(), Some("0.1"));
        assert_eq!(f(5.0, "0.0#").as_deref(), Some("5.0"));
        assert_eq!(f(5.25, "0.0#").as_deref(), Some("5.25"));
    }

    #[test]
    fn thousands_percent_and_scaling() {
        assert_eq!(f(1234567.891, "#,##0.00").as_deref(), Some("1,234,567.89"));
        assert_eq!(f(0.0, "#,##0").as_deref(), Some("0"));
        assert_eq!(f(0.256, "0%").as_deref(), Some("26%"));
        assert_eq!(f(0.2567, "0.0%").as_deref(), Some("25.7%"));
        assert_eq!(f(1234567.0, "0.0,,").as_deref(), Some("1.2"));
        assert_eq!(f(7.0, "000").as_deref(), Some("007"));
        assert_eq!(f(0.5, "#.00").as_deref(), Some(".50"));
    }

    #[test]
    fn sections_literals_and_negatives() {
        assert_eq!(f(-1234.5, "#,##0.00").as_deref(), Some("-1,234.50"));
        assert_eq!(f(-5.0, "0;(0)").as_deref(), Some("(5)"));
        assert_eq!(f(5.0, "0;(0)").as_deref(), Some("5"));
        assert_eq!(f(0.0, "0;-0;\"nil\"").as_deref(), Some("nil"));
        assert_eq!(f(12.5, "\"$\"0.00").as_deref(), Some("$12.50"));
        assert_eq!(f(12.5, "[$€-407] 0.0").as_deref(), Some("€ 12.5"));
        assert_eq!(f(3.0, "0.0\" pts\"").as_deref(), Some("3.0 pts"));
    }

    #[test]
    fn dates_use_the_codes_the_sheet_uses() {
        // 2026-09-01 is serial 46266
        assert_eq!(f(46266.0, "mmm-yyyy").as_deref(), Some("Sep-2026"));
        assert_eq!(f(46266.0, "mmm-yy").as_deref(), Some("Sep-26"));
        assert_eq!(f(46266.0, "d/mm/yyyy").as_deref(), Some("1/09/2026"));
        assert_eq!(
            f(46266.0, "dddd, d mmmm yyyy").as_deref(),
            Some("Tuesday, 1 September 2026")
        );
        assert_eq!(f(46266.0, "yyyy-mm-dd").as_deref(), Some("2026-09-01"));
        assert_eq!(
            f(45000.75, "d-mmm-yy h:mm AM/PM").as_deref(),
            Some("15-Mar-23 6:00 PM")
        );
        assert_eq!(f(0.5, "h:mm:ss").as_deref(), Some("12:00:00"));
        assert_eq!(f(1.5, "[h]:mm").as_deref(), Some("36:00"));
        assert_eq!(f(0.25, "mm:ss").as_deref(), Some("00:00"));
        assert_eq!(f(45000.0, "mmmmm").as_deref(), Some("M"));
    }

    #[test]
    fn unsupported_codes_fall_back() {
        assert_eq!(f(1.5, "# ?/?"), None);
        assert_eq!(f(1.5, "0.00E+00"), None);
        assert_eq!(f(1.5, "[>100]0;0"), None);
        assert_eq!(f(1.5, "[$-F800]dddd, mmmm dd, yyyy"), None);
        assert_eq!(f(1.5, "(###) ###-####"), None);
        assert_eq!(f(f64::NAN, "0.0"), None);
    }
}
