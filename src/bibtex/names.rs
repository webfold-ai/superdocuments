//! Name, page, date and identifier parsing for BibTeX and BibLaTeX values.
//!
//! Names follow BibTeX's rules. The name list splits on `and` at brace depth zero, and `others`
//! is recorded as `NameList::others`. A name is either `First von Last`, `von Last, First`, or
//! `von Last, Jr, First`. The von part is the run of lower-case words before the last word; a
//! word that starts with a brace is not lower case. A name wrapped in braces, such as an
//! institution, is a `literal` name in `family`. Components are decoded to Unicode.

use crate::tex_text::{self, Ligatures};

/// One person or institution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Name {
    pub given: String,
    pub family: String,
    /// The von part, such as "van der".
    pub prefix: String,
    /// The Jr part, such as "Jr." or "III".
    pub suffix: String,
    /// True for a braced institutional name, which is kept whole in `family`.
    pub literal: bool,
}

/// A parsed `author` or `editor` list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameList {
    pub names: Vec<Name>,
    /// True when the list ends with `and others`.
    pub others: bool,
}

/// A page or range. `end` is `None` for a single page or article number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRange {
    pub start: String,
    pub end: Option<String>,
}

/// A calendar date. Components are optional for BibLaTeX dates such as `2020` or `2020-05`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

/// Parses an `and`-separated name list from the raw (brace-preserving) field text.
pub fn parse_names(raw: &str) -> NameList {
    let mut list = NameList::default();
    for segment in split_top_level_and(raw) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if segment.eq_ignore_ascii_case("others") {
            list.others = true;
            continue;
        }
        if let Some(name) = parse_name(segment) {
            list.names.push(name);
        }
    }
    list
}

/// Parses one name from raw text. Returns `None` for an empty string.
pub fn parse_name(raw: &str) -> Option<Name> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if is_wrapped_in_braces(raw) {
        return Some(Name {
            family: decode(&raw[1..raw.len() - 1]),
            literal: true,
            ..Name::default()
        });
    }
    let parts = split_top_level(raw, ',');
    let mut name = Name::default();
    match parts.len() {
        1 => {
            let words = words(parts[0]);
            let (first, von, last) = split_words(&words);
            name.given = join_decoded(&first);
            name.prefix = join_decoded(&von);
            name.family = join_decoded(&last);
        }
        2 => {
            // "von Last, First": the von part can only be at the start.
            let words = words(parts[0]);
            let (_, von, last) = split_von_last(&words);
            name.prefix = join_decoded(&von);
            name.family = join_decoded(&last);
            name.given = decode(parts[1].trim());
        }
        _ => {
            let words = words(parts[0]);
            let (_, von, last) = split_von_last(&words);
            name.prefix = join_decoded(&von);
            name.family = join_decoded(&last);
            name.suffix = decode(parts[1].trim());
            let rest = parts[2..].join(", ");
            name.given = decode(rest.trim());
        }
    }
    Some(name)
}

/// Splits a page field on commas and parses each range. `--`, `-`, en dash and em dash all
/// separate a range.
pub fn parse_pages(value: &str) -> Vec<PageRange> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let split = ["--", "\u{2013}", "\u{2014}", "-"]
            .iter()
            .find_map(|sep| part.find(sep).map(|i| (i, sep.len())));
        match split {
            Some((i, len)) => {
                let start = part[..i].trim();
                let end = part[i + len..].trim();
                out.push(PageRange {
                    start: start.to_string(),
                    end: (!end.is_empty()).then(|| end.to_string()),
                });
            }
            None => out.push(PageRange {
                start: part.to_string(),
                end: None,
            }),
        }
    }
    out
}

/// A four-digit year, optionally followed by a letter suffix (`2020a`).
pub fn parse_year(value: &str) -> Option<i32> {
    let v = value.trim();
    let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
    let rest = &v[digits.len()..];
    if digits.len() != 4 || !rest.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    digits.parse().ok()
}

/// A month name (full or three letters, with or without a dot) or a number from 1 to 12.
pub fn parse_month(value: &str) -> Option<u8> {
    let v = decode(value)
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if let Ok(n) = v.parse::<u8>() {
        return (1..=12).contains(&n).then_some(n);
    }
    const NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    NAMES
        .iter()
        .position(|m| *m == v || (v.len() == 3 && m.starts_with(&v)))
        .map(|i| i as u8 + 1)
}

/// A BibLaTeX date: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, optionally with a time after `T`, and an
/// optional range after `/`. Returns the start date and the end date when there is a range.
pub fn parse_date(value: &str) -> Option<(Date, Option<Date>)> {
    let v = value.trim();
    match v.split_once('/') {
        Some((a, b)) => Some((parse_one_date(a)?, Some(parse_one_date(b)?))),
        None => Some((parse_one_date(v)?, None)),
    }
}

/// Normalises a DOI: removes a `doi:` or resolver prefix and trailing punctuation. Returns
/// `None` unless what remains starts with `10.` and contains a `/` and no spaces.
pub fn normalize_doi(value: &str) -> Option<String> {
    let mut v = decode(value).trim().to_string();
    let lower = v.to_ascii_lowercase();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi.org/",
        "doi:",
    ] {
        if lower.starts_with(prefix) {
            v = v[prefix.len()..].to_string();
            break;
        }
    }
    let v = v.trim().trim_end_matches(['.', ',', ';']).to_string();
    let valid = v.starts_with("10.")
        && v.contains('/')
        && !v.chars().any(char::is_whitespace)
        && v.len() > "10./".len();
    valid.then_some(v)
}

/// Normalises a URL: removes `\url{}` wrapping and TeX escapes, then accepts only `http`,
/// `https`, `ftp` and `mailto`. The scheme is lower-cased.
pub fn normalize_url(value: &str) -> Option<String> {
    let mut v = value.trim();
    if let Some(rest) = v.strip_prefix("\\url") {
        v = rest.trim();
    }
    let v = decode(v);
    let v = v.trim();
    if v.chars().any(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = v.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https" | "ftp" | "mailto") || rest.is_empty() {
        return None;
    }
    Some(format!("{scheme}:{rest}"))
}

fn parse_one_date(text: &str) -> Option<Date> {
    let text = text.trim();
    let text = text.split('T').next().unwrap_or(text);
    let mut parts = text.split('-');
    let year = parts.next()?;
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = year.parse().ok()?;
    let month = match parts.next() {
        Some(m) => Some(parse_num_in(m, 1, 12)?),
        None => None,
    };
    let day = match parts.next() {
        Some(d) => Some(parse_num_in(d, 1, 31)?),
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(Date { year, month, day })
}

fn parse_num_in(text: &str, lo: u8, hi: u8) -> Option<u8> {
    if text.len() != 2 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u8 = text.parse().ok()?;
    (lo..=hi).contains(&n).then_some(n)
}

fn decode(s: &str) -> String {
    tex_text::decode(s, Ligatures::None)
}

fn join_decoded(words: &[&str]) -> String {
    decode(&words.join(" "))
}

fn is_wrapped_in_braces(s: &str) -> bool {
    if !(s.starts_with('{') && s.ends_with('}')) {
        return false;
    }
    // The first brace must close at the last character.
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i == s.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Splits on `sep` at brace depth zero.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Splits a name list on the word `and` (any case) surrounded by whitespace, at brace depth
/// zero.
fn split_top_level_and(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            c if depth == 0 && c.is_ascii_whitespace() => {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let is_and = bytes.len() >= j + 3
                    && bytes[j..j + 3].eq_ignore_ascii_case(b"and")
                    && bytes.get(j + 3).is_some_and(|b| b.is_ascii_whitespace());
                if is_and && j > i {
                    out.push(&s[start..i]);
                    start = j + 3;
                    i = start;
                    continue;
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// Whitespace-separated words, with a braced group kept as one word.
fn words(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        match c {
            '{' => {
                depth += 1;
                start.get_or_insert(i);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                start.get_or_insert(i);
            }
            c if c.is_whitespace() && depth == 0 => {
                if let Some(st) = start.take() {
                    out.push(&s[st..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

/// Splits name words into (first, von, last) by BibTeX's rules.
fn split_words<'a>(words: &[&'a str]) -> (Vec<&'a str>, Vec<&'a str>, Vec<&'a str>) {
    let n = words.len();
    if n <= 1 {
        return (
            Vec::new(),
            Vec::new(),
            words.first().map(|w| vec![*w]).unwrap_or_default(),
        );
    }
    let last_index = n - 1;
    let Some(von_start) = (0..last_index).find(|&i| is_lower_word(words[i])) else {
        return (
            words[..last_index].to_vec(),
            Vec::new(),
            vec![words[last_index]],
        );
    };
    let mut von_end = von_start;
    while von_end < last_index && is_lower_word(words[von_end]) {
        von_end += 1;
    }
    (
        words[..von_start].to_vec(),
        words[von_start..von_end].to_vec(),
        words[von_end..].to_vec(),
    )
}

/// Splits the "von Last" part of a comma form into (empty, von, last). The von part is the
/// run of lower-case words at the start, never including the last word.
fn split_von_last<'a>(words: &[&'a str]) -> (Vec<&'a str>, Vec<&'a str>, Vec<&'a str>) {
    let n = words.len();
    if n <= 1 {
        return (Vec::new(), Vec::new(), words.to_vec());
    }
    let mut von_end = 0;
    while von_end < n - 1 && is_lower_word(words[von_end]) {
        von_end += 1;
    }
    (
        Vec::new(),
        words[..von_end].to_vec(),
        words[von_end..].to_vec(),
    )
}

/// A word whose first character is a lower-case letter. A word that starts with a brace is not
/// lower case, which keeps `{\"o}` and `{Jr.}` out of the von part.
fn is_lower_word(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(raw: &str) -> Name {
        parse_name(raw).unwrap_or_default()
    }

    #[test]
    fn first_last_order() {
        let n = one("Ada Lovelace");
        assert_eq!((n.given.as_str(), n.family.as_str()), ("Ada", "Lovelace"));
        assert_eq!(n.prefix, "");
    }

    #[test]
    fn von_part_in_first_last_order() {
        let n = one("Ludwig van Beethoven");
        assert_eq!(n.given, "Ludwig");
        assert_eq!(n.prefix, "van");
        assert_eq!(n.family, "Beethoven");
        let n = one("Charles Louis Xavier Joseph de la Vallee Poussin");
        assert_eq!(n.given, "Charles Louis Xavier Joseph");
        assert_eq!(n.prefix, "de la");
        assert_eq!(n.family, "Vallee Poussin");
    }

    #[test]
    fn last_first_and_suffix_forms() {
        let n = one("Lovelace, Ada");
        assert_eq!((n.family.as_str(), n.given.as_str()), ("Lovelace", "Ada"));
        let n = one("van der Berg, Jan");
        assert_eq!((n.prefix.as_str(), n.family.as_str()), ("van der", "Berg"));
        let n = one("King, Jr, Martin Luther");
        assert_eq!(n.suffix, "Jr");
        assert_eq!(n.family, "King");
        assert_eq!(n.given, "Martin Luther");
    }

    #[test]
    fn comma_form_von_must_start_the_name() {
        let n = one("{\\'A}lvarez y Mu{\\~n}oz, Ana");
        assert_eq!(n.prefix, "");
        assert_eq!(n.family, "Álvarez y Muñoz");
        let n = one("Wang de la, Li");
        assert_eq!(n.family, "Wang de la");
    }

    #[test]
    fn braced_institution_is_literal() {
        let n = one("{World Health Organization}");
        assert!(n.literal);
        assert_eq!(n.family, "World Health Organization");
        assert_eq!(n.given, "");
    }

    #[test]
    fn accents_inside_names_are_decoded() {
        let n = one("{\\\"O}zt{\\\"u}rk, Ay{\\c{s}}e");
        assert_eq!(n.family, "Öztürk");
        assert_eq!(n.given, "Ayşe");
    }

    #[test]
    fn name_lists_split_on_and_and_flag_others() {
        let list = parse_names("Smith, Jane and {ACME Labs} and others");
        assert_eq!(list.names.len(), 2);
        assert!(list.others);
        assert!(list.names[1].literal);
        // "and" inside braces does not split.
        let list = parse_names("{Bread and Butter Co}");
        assert_eq!(list.names.len(), 1);
        // Case-insensitive "AND".
        assert_eq!(parse_names("A B AND C D").names.len(), 2);
    }

    #[test]
    fn pages_ranges_and_article_numbers() {
        let p = parse_pages("12--34, 56-60, 7, e123, xii+200");
        assert_eq!(p[0].start, "12");
        assert_eq!(p[0].end.as_deref(), Some("34"));
        assert_eq!(p[1].end.as_deref(), Some("60"));
        assert_eq!(p[2].end, None);
        assert_eq!(p[3].start, "e123");
        assert_eq!(p[4].start, "xii+200");
        let p = parse_pages("1\u{2013}9");
        assert_eq!(p[0].end.as_deref(), Some("9"));
    }

    #[test]
    fn years_and_months() {
        assert_eq!(parse_year(" 2020 "), Some(2020));
        assert_eq!(parse_year("2020a"), Some(2020));
        assert_eq!(parse_year("20"), None);
        assert_eq!(parse_month("January"), Some(1));
        assert_eq!(parse_month("sep."), Some(9));
        assert_eq!(parse_month("12"), Some(12));
        assert_eq!(parse_month("13"), None);
        assert_eq!(parse_month("Smarch"), None);
    }

    #[test]
    fn biblatex_dates() {
        let (d, end) = parse_date("2020-05-03").unwrap_or((
            Date {
                year: 0,
                month: None,
                day: None,
            },
            None,
        ));
        assert_eq!((d.year, d.month, d.day), (2020, Some(5), Some(3)));
        assert!(end.is_none());
        let (d, _) = parse_date("2019-11").unwrap_or((
            Date {
                year: 0,
                month: None,
                day: None,
            },
            None,
        ));
        assert_eq!((d.year, d.month, d.day), (2019, Some(11), None));
        let (_, end) = parse_date("2020/2021-02").unwrap_or((
            Date {
                year: 0,
                month: None,
                day: None,
            },
            None,
        ));
        assert_eq!(end.map(|e| e.year), Some(2021));
        assert!(parse_date("2020-13-01").is_none());
        assert!(parse_date("20-05").is_none());
        assert!(parse_date("2020-05-03-01").is_none());
    }

    #[test]
    fn dois_and_urls() {
        assert_eq!(
            normalize_doi("https://doi.org/10.1000/xyz123.").as_deref(),
            Some("10.1000/xyz123")
        );
        assert_eq!(normalize_doi("doi:10.1/a").as_deref(), Some("10.1/a"));
        assert_eq!(normalize_doi("not a doi"), None);
        assert_eq!(normalize_doi("10.1000 /x"), None);
        assert_eq!(
            normalize_url("\\url{HTTPS://example.org/a\\%20b}").as_deref(),
            Some("https://example.org/a%20b")
        );
        assert_eq!(normalize_url("javascript:alert(1)"), None);
        assert_eq!(normalize_url("https://a b"), None);
    }

    #[test]
    fn malformed_names_do_not_panic() {
        for s in [
            "",
            "{",
            "}",
            ",,,",
            "{a}}{b",
            "a,b,c,d,e",
            " and ",
            "{{{x}}",
        ] {
            let _ = parse_names(s);
            let _ = parse_name(s);
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn exact_length_and_prefix_boundaries() {
        assert_eq!(parse_year("20000"), None);
        assert_eq!(parse_date("2020-5-03"), None);
        assert_eq!(parse_date("2020-05-5"), None);
        assert_eq!(normalize_doi("1234/abc"), None);
        assert_eq!(normalize_url("mailto:a@b.org").as_deref(), Some("mailto:a@b.org"));
    }
}
