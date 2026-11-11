//! TeX text to Unicode, shared by `bibtex` and `latex`.
//!
//! `decode` removes grouping braces and turns accent commands (`\"o`, `{\'e}`, `\c{c}`), letter
//! commands (`\ss`, `\o`, `\ae`), text symbols (`\&`, `\%`, `\textendash`) and formatting
//! wrappers (`\emph{x}` keeps `x`) into Unicode. Unknown control words are kept as written, so
//! nothing is invented. Only the `Ligatures::Tex` mode maps `--`, `---`, ``` `` ``` and `''` and
//! `~`, which TeX typesets as text. BibTeX values use `Ligatures::None`.
//!
//! The table covers Latin letters with the common accents. A combination outside the table
//! falls back to the base letter followed by the Unicode combining mark, which is still correct
//! Unicode for the same text.

/// Whether TeX ligatures and the non-breaking tilde are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ligatures {
    None,
    Tex,
}

/// Decodes TeX text to Unicode. Never panics; unknown input passes through.
pub fn decode(input: &str, ligatures: Ligatures) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' | '}' => i += 1,
            '\\' => i = control(&chars, i, &mut out),
            '~' if ligatures == Ligatures::Tex => {
                out.push('\u{a0}');
                i += 1;
            }
            '-' if ligatures == Ligatures::Tex && starts(&chars, i, "---") => {
                out.push('\u{2014}');
                i += 3;
            }
            '-' if ligatures == Ligatures::Tex && starts(&chars, i, "--") => {
                out.push('\u{2013}');
                i += 2;
            }
            '`' if ligatures == Ligatures::Tex && starts(&chars, i, "``") => {
                out.push('\u{201c}');
                i += 2;
            }
            '\'' if ligatures == Ligatures::Tex && starts(&chars, i, "''") => {
                out.push('\u{201d}');
                i += 2;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Returns the precomposed character for an accent command applied to a base letter, when the
/// table has one.
pub fn accented(accent: char, base: char) -> Option<char> {
    let (bases, outs) = accent_table(accent)?;
    bases
        .chars()
        .zip(outs.chars())
        .find(|(b, _)| *b == base)
        .map(|(_, o)| o)
}

/// The Unicode combining mark for a TeX accent command, or `None` for an unknown accent.
pub fn combining_mark(accent: char) -> Option<char> {
    Some(match accent {
        '\'' => '\u{301}',
        '`' => '\u{300}',
        '^' => '\u{302}',
        '"' => '\u{308}',
        '~' => '\u{303}',
        '=' => '\u{304}',
        '.' => '\u{307}',
        'u' => '\u{306}',
        'v' => '\u{30c}',
        'H' => '\u{30b}',
        'c' => '\u{327}',
        'k' => '\u{328}',
        'r' => '\u{30a}',
        'd' => '\u{323}',
        'b' => '\u{331}',
        _ => return None,
    })
}

/// Unicode for a TeX letter or text-symbol command without an argument (`ss`, `ldots`).
pub fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "ss" => "\u{df}",
        "o" => "\u{f8}",
        "O" => "\u{d8}",
        "ae" => "\u{e6}",
        "AE" => "\u{c6}",
        "oe" => "\u{153}",
        "OE" => "\u{152}",
        "aa" => "\u{e5}",
        "AA" => "\u{c5}",
        "l" => "\u{142}",
        "L" => "\u{141}",
        "i" => "\u{131}",
        "j" => "\u{237}",
        "ldots" | "dots" | "textellipsis" => "\u{2026}",
        "textendash" | "ndash" => "\u{2013}",
        "textemdash" | "mdash" => "\u{2014}",
        "textquoteleft" | "lq" => "\u{2018}",
        "textquoteright" | "rq" => "\u{2019}",
        "textquotedblleft" => "\u{201c}",
        "textquotedblright" => "\u{201d}",
        "textasciitilde" => "~",
        "textasciicircum" => "^",
        "textbackslash" => "\\",
        "textless" => "<",
        "textgreater" => ">",
        "copyright" | "textcopyright" => "\u{a9}",
        "texttrademark" => "\u{2122}",
        "textregistered" => "\u{ae}",
        "S" => "\u{a7}",
        "P" => "\u{b6}",
        "pounds" | "textsterling" => "\u{a3}",
        "euro" => "\u{20ac}",
        "textdegree" => "\u{b0}",
        "textbullet" => "\u{2022}",
        "dag" => "\u{2020}",
        "ddag" => "\u{2021}",
        "guillemotleft" => "\u{ab}",
        "guillemotright" => "\u{bb}",
        "LaTeX" => "LaTeX",
        "TeX" => "TeX",
        _ => return None,
    })
}

/// The text of a siunitx unit argument: `\kilo\gram\per\second\squared` gives `kg/s²`, and
/// `m.s^{-1}` gives `m·s⁻¹`. A unit macro that is not in the table keeps its name.
pub fn si_unit(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                let start = i + 1;
                let mut end = start;
                while chars.get(end).is_some_and(|c| c.is_ascii_alphabetic()) {
                    end += 1;
                }
                if end == start {
                    // `\%` and other escaped characters.
                    if let Some(&c) = chars.get(start) {
                        out.push(c);
                    }
                    i = start + 1;
                    continue;
                }
                let name: String = chars[start..end].iter().collect();
                i = end;
                match name.as_str() {
                    "per" => out.push('/'),
                    "squared" | "square" => out.push('\u{b2}'),
                    "cubed" | "cubic" => out.push('\u{b3}'),
                    "tothe" | "raiseto" => {}
                    _ => out.push_str(si_symbol(&name).unwrap_or(&name)),
                }
            }
            '^' => {
                i += 1;
                let mut exponent = String::new();
                if chars.get(i) == Some(&'{') {
                    i += 1;
                    while let Some(&c) = chars.get(i) {
                        i += 1;
                        if c == '}' {
                            break;
                        }
                        exponent.push(c);
                    }
                } else if let Some(&c) = chars.get(i) {
                    exponent.push(c);
                    i += 1;
                }
                out.push_str(&superscript(&exponent));
            }
            '.' | '~' => {
                out.push('\u{b7}');
                i += 1;
            }
            '{' | '}' => i += 1,
            c if c.is_whitespace() => i += 1,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The text of a siunitx number: `1.5e-3` gives `1.5 × 10⁻³`, and `+-` or `\pm` gives `±`.
pub fn si_number(raw: &str) -> String {
    let text = raw
        .trim()
        .replace("\\pm", "\u{b1}")
        .replace("+-", "\u{b1}")
        .replace("\\,", "")
        .replace(['{', '}'], "");
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(pos) = text.find(['e', 'E']) else {
        return text
            .replace(" \u{b1} ", "\u{a0}\u{b1}\u{a0}")
            .replace('-', "\u{2212}");
    };
    let (mantissa, exponent) = (&text[..pos], &text[pos + 1..]);
    let exponent_ok = !exponent.is_empty()
        && exponent
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '+');
    if !exponent_ok {
        return text;
    }
    let power = format!("10{}", superscript(exponent.trim_start_matches('+')));
    let mantissa = mantissa.replace('-', "\u{2212}");
    if mantissa.is_empty() {
        power
    } else {
        format!("{mantissa}\u{a0}\u{d7}\u{a0}{power}")
    }
}

/// Digits and signs as Unicode superscripts; anything else is written after `^`.
fn superscript(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        out.push(match c {
            '0' => '\u{2070}',
            '1' => '\u{b9}',
            '2' => '\u{b2}',
            '3' => '\u{b3}',
            '4'..='9' => char::from_u32(0x2070 + (c as u32 - '0' as u32)).unwrap_or(c),
            '-' => '\u{207b}',
            '+' => '\u{207a}',
            _ => return format!("^{text}"),
        });
    }
    out
}

/// The symbol of a siunitx prefix or unit macro.
fn si_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // prefixes
        "yocto" => "y",
        "zepto" => "z",
        "atto" => "a",
        "femto" => "f",
        "pico" => "p",
        "nano" => "n",
        "micro" => "\u{b5}",
        "milli" => "m",
        "centi" => "c",
        "deci" => "d",
        "deca" | "deka" => "da",
        "hecto" => "h",
        "kilo" => "k",
        "mega" => "M",
        "giga" => "G",
        "tera" => "T",
        "peta" => "P",
        "exa" => "E",
        "kibi" => "Ki",
        "mebi" => "Mi",
        "gibi" => "Gi",
        "tebi" => "Ti",
        // units
        "meter" | "metre" => "m",
        "gram" => "g",
        "kilogram" => "kg",
        "second" => "s",
        "ampere" => "A",
        "kelvin" => "K",
        "mole" => "mol",
        "candela" => "cd",
        "hertz" => "Hz",
        "newton" => "N",
        "pascal" => "Pa",
        "joule" => "J",
        "watt" => "W",
        "coulomb" => "C",
        "volt" => "V",
        "farad" => "F",
        "ohm" => "\u{3a9}",
        "siemens" => "S",
        "weber" => "Wb",
        "tesla" => "T",
        "henry" => "H",
        "lumen" => "lm",
        "lux" => "lx",
        "becquerel" => "Bq",
        "gray" => "Gy",
        "sievert" => "Sv",
        "radian" => "rad",
        "steradian" => "sr",
        "celsius" | "degreeCelsius" => "\u{b0}C",
        "degree" => "\u{b0}",
        "percent" => "%",
        "minute" => "min",
        "hour" => "h",
        "day" => "d",
        "liter" | "litre" => "L",
        "tonne" => "t",
        "electronvolt" => "eV",
        "angstrom" => "\u{c5}",
        "bar" => "bar",
        "decibel" => "dB",
        "bit" => "bit",
        "byte" => "B",
        "arcminute" => "\u{2032}",
        "arcsecond" => "\u{2033}",
        "astronomicalunit" => "au",
        "dalton" => "Da",
        _ => return None,
    })
}

/// Formatting commands whose argument is kept and whose name is dropped.
pub fn is_transparent(name: &str) -> bool {
    matches!(
        name,
        "emph"
            | "textit"
            | "textbf"
            | "textsc"
            | "texttt"
            | "textrm"
            | "textsf"
            | "textsl"
            | "textup"
            | "textmd"
            | "textnormal"
            | "mbox"
            | "text"
            | "underline"
            | "relax"
            | "protect"
            | "bf"
            | "it"
            | "em"
            | "sc"
            | "rm"
            | "sf"
            | "tt"
            | "normalfont"
    )
}

fn accent_table(accent: char) -> Option<(&'static str, &'static str)> {
    Some(match accent {
        '\'' => ("aeiouyAEIOUYcCnNsSzZlLrRgG", "áéíóúýÁÉÍÓÚÝćĆńŃśŚźŹĺĹŕŔǵǴ"),
        '`' => ("aeiouAEIOUnN", "àèìòùÀÈÌÒÙǹǸ"),
        '^' => ("aeiouAEIOUcCgGhHjJsSwWyY", "âêîôûÂÊÎÔÛĉĈĝĜĥĤĵĴŝŜŵŴŷŶ"),
        '"' => ("aeiouyAEIOUY", "äëïöüÿÄËÏÖÜŸ"),
        '~' => ("anoiueANOIUE", "ãñõĩũẽÃÑÕĨŨẼ"),
        '=' => ("aeiouAEIOU", "āēīōūĀĒĪŌŪ"),
        'u' => ("aeguAEGU", "ăĕğŭĂĔĞŬ"),
        '.' => ("cegzCEGIZ", "ċėġżĊĖĠİŻ"),
        'v' => ("cdenrstzCDENRSTZ", "čďěňřšťžČĎĚŇŘŠŤŽ"),
        'r' => ("auAU", "åůÅŮ"),
        'H' => ("ouOU", "őűŐŰ"),
        'c' => ("cCsStT", "çÇşŞţŢ"),
        'k' => ("aAeEiIuU", "ąĄęĘįĮųŲ"),
        _ => return None,
    })
}

fn starts(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(j, p)| chars.get(at + j) == Some(&p))
}

/// Handles one backslash sequence starting at `at` (the backslash). Returns the next index.
fn control(chars: &[char], at: usize, out: &mut String) -> usize {
    let len = chars.len();
    let j = at + 1;
    let Some(&c) = chars.get(j) else {
        out.push('\\');
        return j;
    };
    if c.is_ascii_alphabetic() {
        let mut k = j;
        while k < len && chars[k].is_ascii_alphabetic() {
            k += 1;
        }
        let name: String = chars[j..k].iter().collect();
        let first = name.chars().next().unwrap_or(' ');
        if name.len() == 1 && combining_mark(first).is_some() {
            return accent_command(chars, k, first, out);
        }
        let mut after = k;
        while after < len && chars[after] == ' ' {
            after += 1;
        }
        if let Some(sym) = symbol(&name) {
            out.push_str(sym);
            return after;
        }
        if is_transparent(&name) {
            return after;
        }
        out.push('\\');
        out.push_str(&name);
        return k;
    }
    match c {
        '\'' | '`' | '^' | '"' | '~' | '=' | '.' => accent_command(chars, j + 1, c, out),
        '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
            out.push(c);
            j + 1
        }
        ' ' | ',' | ';' | '\\' => {
            out.push(' ');
            j + 1
        }
        '!' | '-' | '/' | ':' => j + 1,
        _ => {
            out.push(c);
            j + 1
        }
    }
}

/// Reads the argument of an accent command (`{e}`, `{\i}`, or a bare letter) and writes the
/// combined character. Returns the index after the argument.
fn accent_command(chars: &[char], from: usize, accent: char, out: &mut String) -> usize {
    let len = chars.len();
    let mut k = from;
    while k < len && chars[k] == ' ' {
        k += 1;
    }
    let (base, next) = if chars.get(k) == Some(&'{') {
        match matching_close(chars, k) {
            Some(end) => {
                let inner: String = chars[k + 1..end].iter().collect();
                let inner = inner.trim();
                let base = if inner == "\\i" {
                    Some("i".to_string())
                } else if inner.chars().count() == 1 {
                    inner.chars().next().map(|c| c.to_string())
                } else {
                    None
                };
                (base, end + 1)
            }
            None => (None, len),
        }
    } else if let Some(&b) = chars.get(k) {
        if b.is_alphabetic() {
            (Some(b.to_string()), k + 1)
        } else {
            (None, k)
        }
    } else {
        (None, k)
    };
    match base {
        Some(b) => {
            let mut bc = b.chars();
            let base_char = bc.next().unwrap_or(' ');
            match accented(accent, base_char) {
                Some(p) => out.push(p),
                None => {
                    out.push_str(&b);
                    if let Some(m) = combining_mark(accent) {
                        out.push(m);
                    }
                }
            }
        }
        None => {
            if let Some(m) = combining_mark(accent) {
                out.push(m);
            }
        }
    }
    next
}

/// Index of the `}` that closes the `{` at `open`, or `None`.
fn matching_close(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (k, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_and_braced_accents_become_unicode() {
        assert_eq!(decode("M\\\"uller", Ligatures::None), "Müller");
        assert_eq!(decode("{\\'E}cole", Ligatures::None), "École");
        assert_eq!(decode("Caf{\\'e}", Ligatures::None), "Café");
        assert_eq!(decode("fa\\c{c}ade", Ligatures::None), "façade");
        assert_eq!(decode("Erd\\H{o}s", Ligatures::None), "Erdős");
        assert_eq!(decode("\\v{S}ev\\v{c}enko", Ligatures::None), "Ševčenko");
        assert_eq!(decode("Dvo\\v{r}\\'{a}k", Ligatures::None), "Dvořák");
    }

    #[test]
    fn accent_on_dotless_i_and_bare_letters() {
        assert_eq!(decode("\\'{\\i}", Ligatures::None), "í");
        assert_eq!(decode("\\~n", Ligatures::None), "ñ");
    }

    #[test]
    fn unknown_accent_combination_uses_combining_mark() {
        // No table entry for a macron on a consonant: base plus U+0304.
        assert_eq!(decode("\\={x}", Ligatures::None), "x\u{304}");
    }

    #[test]
    fn letter_symbols_and_text_commands() {
        assert_eq!(decode("Stra{\\ss}e", Ligatures::None), "Straße");
        assert_eq!(decode("{\\O}rsted", Ligatures::None), "Ørsted");
        assert_eq!(decode("Malm\\o", Ligatures::None), "Malmø");
        assert_eq!(decode("\\L{}ukasz", Ligatures::None), "Łukasz");
        assert_eq!(decode("\\AA{}ngstr\\\"om", Ligatures::None), "Ångström");
        assert_eq!(
            decode("A \\& B \\% C \\$1", Ligatures::None),
            "A & B % C $1"
        );
        assert_eq!(decode("p\\textendash{}q", Ligatures::None), "p–q");
        assert_eq!(
            decode("\\emph{x} and \\textbf{y}", Ligatures::None),
            "x and y"
        );
    }

    #[test]
    fn unknown_control_words_are_kept_as_written() {
        assert_eq!(decode("\\foo{bar}", Ligatures::None), "\\foobar");
    }

    #[test]
    fn tex_ligatures_only_in_tex_mode() {
        assert_eq!(decode("1--2", Ligatures::None), "1--2");
        assert_eq!(decode("1--2", Ligatures::Tex), "1\u{2013}2");
        assert_eq!(decode("a---b", Ligatures::Tex), "a\u{2014}b");
        assert_eq!(decode("``q''", Ligatures::Tex), "\u{201c}q\u{201d}");
        assert_eq!(decode("a~b", Ligatures::Tex), "a\u{a0}b");
        assert_eq!(decode("a~b", Ligatures::None), "a~b");
    }

    #[test]
    fn malformed_input_never_panics() {
        for s in [
            "\\", "\\\"", "\\'{", "\\c", "{", "}", "\\{", "\\'{\\", "\\H",
        ] {
            let _ = decode(s, Ligatures::Tex);
        }
    }

    #[test]
    fn accent_tables_are_balanced() {
        for a in [
            '\'', '`', '^', '"', '~', '=', 'u', '.', 'v', 'r', 'H', 'c', 'k',
        ] {
            let (b, o) = accent_table(a).unwrap_or(("", ""));
            assert_eq!(b.chars().count(), o.chars().count(), "accent {a}");
        }
    }
}
