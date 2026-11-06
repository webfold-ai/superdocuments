//! BibTeX module tests: behaviour, golden files, round trips, limits, and seeded truncation and
//! mutation runs.
//!
//! Golden files live in `tests/fixtures/bibtex/<name>.expected.bib` (the writer output) and
//! `<name>.expected.json` (the CSL-JSON). To regenerate them after an intended change, run
//! `BIBTEX_REGENERATE=1 cargo test --lib bibtex::tests::golden`, then review the diff.

use super::*;

const FIXTURES: &[&str] = &[
    "basic",
    "strings-accents",
    "crossref-biblatex",
    "malformed",
    "names-and-pages",
];

fn fixture_text(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/bibtex/{name}.bib",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap_or_default()
}

fn same_content(a: &Bibliography, b: &Bibliography) -> bool {
    a.entries == b.entries
        && a.strings == b.strings
        && a.preambles == b.preambles
        && a.comments == b.comments
}

fn entry<'a>(bib: &'a Bibliography, key: &str) -> &'a Entry {
    bib.lookup(key).unwrap_or_else(|| panic!("no entry {key}"))
}

fn warnings_text(bib: &Bibliography) -> Vec<String> {
    bib.warnings.iter().map(|w| w.message.clone()).collect()
}

#[test]
fn entry_types_keys_and_delimiters() {
    let bib = parse(&fixture_text("basic"));
    let keys: Vec<&str> = bib.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "lovelace1843",
            "knuth1984",
            "hopper1952",
            "nokey_fields_only",
            "thesis2001"
        ]
    );
    // Upper-case type names are lower-cased; parentheses are accepted as delimiters.
    assert_eq!(entry(&bib, "hopper1952").entry_type, "inproceedings");
    assert!(bib.warnings.is_empty(), "{:?}", warnings_text(&bib));
}

#[test]
fn field_names_are_case_insensitive_and_last_comma_is_optional() {
    let bib = parse(&fixture_text("basic"));
    let hopper = entry(&bib, "hopper1952");
    assert_eq!(hopper.value("title"), Some("The Education of a Computer"));
    assert_eq!(
        hopper.value("BOOKTITLE"),
        Some("Proceedings of the 1952 ACM National Meeting")
    );
    let misc = entry(&bib, "nokey_fields_only");
    assert_eq!(misc.value("note"), Some("Online"));
}

#[test]
fn braces_and_quotes_nest_and_keep_their_inner_text() {
    let bib = parse(&fixture_text("basic"));
    let lovelace = entry(&bib, "lovelace1843");
    assert_eq!(
        lovelace.value("title"),
        Some("Notes on the Analytical Engine")
    );
    assert_eq!(
        lovelace.field("title").map(|f| f.raw.as_str()),
        Some("Notes on the {Analytical Engine}")
    );
    assert_eq!(
        lovelace.value("journal"),
        Some("Taylor's Scientific Memoirs")
    );
    assert_eq!(lovelace.value("pages"), Some("691--731"));
    let knuth = entry(&bib, "knuth1984");
    assert_eq!(knuth.value("title"), Some("The TeXbook"));
}

#[test]
fn string_macros_concatenation_and_month_names() {
    let bib = parse(&fixture_text("strings-accents"));
    assert_eq!(bib.strings.len(), 3);
    let schneider = entry(&bib, "schneider2020");
    // `booktitle = "Proc. of " # "the Workshop"` is concatenated.
    assert_eq!(schneider.value("booktitle"), Some("Proc. of the Workshop"));
    // A @string used as a value expands to its raw text.
    assert_eq!(schneider.value("publisher"), Some("ACM Press, New York"));
    // Built-in month macros are expanded, and `month = {jan} # "uary"` gives January.
    assert_eq!(schneider.value("month"), Some("September"));
    let mueller = entry(&bib, "mueller2019");
    assert_eq!(mueller.value("month"), Some("January"));
    assert_eq!(
        mueller.value("journal"),
        Some("Journal of Machine Learning Research")
    );
}

#[test]
fn accents_and_symbols_become_unicode_with_raw_kept() {
    let bib = parse(&fixture_text("strings-accents"));
    let schneider = entry(&bib, "schneider2020");
    let authors = schneider.authors();
    assert_eq!(authors.names[0].family, "Schneider");
    assert_eq!(authors.names[0].given, "Jürgen");
    assert_eq!(authors.names[1].family, "Małgorzata");
    assert_eq!(authors.names[1].given, "Nowak");
    assert_eq!(
        schneider.value("title"),
        Some("Café au lait: the Çase of ß")
    );
    assert_eq!(
        schneider.field("title").map(|f| f.raw.as_str()),
        Some("Caf{\\'e} au lait: the \\c{C}ase of {\\ss}")
    );
    let mueller = entry(&bib, "mueller2019");
    assert_eq!(mueller.value("title"), Some("Erdős numbers & % rates"));
    assert_eq!(mueller.value("url"), Some("https://example.org/a_b"));
    let quoted = entry(&bib, "quoted2018");
    // BibTeX does not apply TeX ligatures, so `` and '' stay as written.
    assert_eq!(
        quoted.value("title"),
        Some("A ``quoted'' title with a – dash")
    );
}

#[test]
fn preamble_and_comment_are_kept_as_raw_text() {
    let bib = parse(&fixture_text("strings-accents"));
    assert_eq!(bib.preambles, ["\\newcommand{\\noop}[1]{#1}"]);
    assert_eq!(bib.comments.len(), 1);
    assert!(bib.comments[0].contains("ignored by BibTeX"));
}

#[test]
fn names_follow_bibtex_rules() {
    let bib = parse(&fixture_text("names-and-pages"));
    let e = entry(&bib, "names1");
    let list = e.authors();
    assert_eq!(list.names[0].prefix, "van");
    assert_eq!(list.names[0].family, "Beethoven");
    assert_eq!(list.names[1].prefix, "de la");
    assert_eq!(list.names[1].family, "Vallee Poussin");
    assert_eq!(list.names[2].suffix, "Jr");
    assert_eq!(list.names[2].given, "John Q.");
    assert_eq!(list.names[3].family, "Álvarez y Muñoz");
    assert_eq!(list.names[3].given, "Ana");
    assert_eq!(list.names[4].prefix, "van");
    assert_eq!(list.names[4].family, "Gogh");
    assert!(!list.others);
}

#[test]
fn pages_ranges_are_parsed() {
    let bib = parse(&fixture_text("names-and-pages"));
    let pages = entry(&bib, "names1").pages();
    assert_eq!(pages.len(), 4);
    assert_eq!(
        (pages[0].start.as_str(), pages[0].end.as_deref()),
        ("12", Some("34"))
    );
    assert_eq!(pages[2].end, None);
    assert_eq!(pages[3].start, "e123");
}

#[test]
fn years_months_dates_doi_urls_and_arxiv_ids() {
    let bib = parse(&fixture_text("crossref-biblatex"));
    let vaswani = entry(&bib, "vaswani2017");
    assert_eq!(vaswani.year(), Some(2017));
    assert_eq!(vaswani.month(), Some(6));
    assert_eq!(vaswani.arxiv_id().as_deref(), Some("1706.03762"));
    assert_eq!(vaswani.doi().as_deref(), Some("10.5555/3295222.3295349"));
    let lecun = entry(&bib, "lecun2015");
    assert_eq!(lecun.arxiv_id().as_deref(), Some("1502.00001"));
    assert_eq!(
        lecun.url().as_deref(),
        Some("https://www.nature.com/articles/nature14539")
    );
    let (start, end) = super::parse_date("2015/2016-05").unwrap_or((
        Date {
            year: 0,
            month: None,
            day: None,
        },
        None,
    ));
    assert_eq!(start.year, 2015);
    assert_eq!(end.map(|d| d.year), Some(2016));
}

#[test]
fn crossref_inherits_missing_fields() {
    let bib = parse(&fixture_text("crossref-biblatex"));
    let vaswani = entry(&bib, "vaswani2017");
    assert_eq!(vaswani.value("editor"), None, "own entry only");
    assert_eq!(
        bib.resolved_value(vaswani, "publisher").as_deref(),
        Some("Curran Associates")
    );
    assert_eq!(
        bib.resolved_value(vaswani, "booktitle"),
        None,
        "the parent's title is not booktitle"
    );
    assert_eq!(
        bib.resolved_value(vaswani, "title").as_deref(),
        Some("Attention Is All You Need")
    );
}

#[test]
fn crossref_cycles_stop() {
    let bib = parse(&fixture_text("crossref-biblatex"));
    let a = entry(&bib, "cycle_a");
    assert_eq!(bib.resolved_value(a, "missing-field"), None);
}

#[test]
fn csl_json_maps_types_names_dates_and_inheritance() {
    let bib = parse(&fixture_text("crossref-biblatex"));
    let csl = csl_json(&bib);
    let items = csl.as_array().cloned().unwrap_or_default();
    let vaswani = items
        .iter()
        .find(|v| v["id"] == "vaswani2017")
        .cloned()
        .unwrap_or_default();
    assert_eq!(vaswani["type"], "paper-conference");
    assert_eq!(vaswani["publisher"], "Curran Associates");
    assert_eq!(
        vaswani["issued"]["date-parts"],
        serde_json::json!([[2017, 6, 12]])
    );
    assert_eq!(vaswani["DOI"], "10.5555/3295222.3295349");
    assert_eq!(vaswani["author"][0]["family"], "Vaswani");
    assert_eq!(vaswani["author"][0]["given"], "Ashish");
    assert_eq!(vaswani["number"], "arXiv:1706.03762");
    let ac = items
        .iter()
        .find(|v| v["id"] == "ac_ronym")
        .cloned()
        .unwrap_or_default();
    assert_eq!(ac["author"][0]["literal"], "World Health Organization");
    assert_eq!(ac["author"][1]["family"], "Zola");
    assert_eq!(ac["author"][2]["literal"], "et al.");
    let csl_basic = csl_json(&parse(&fixture_text("basic")));
    let lovelace = csl_basic[0].clone();
    assert_eq!(lovelace["page"], "691-731");
    assert_eq!(lovelace["issue"], "2");
    assert_eq!(lovelace["container-title"], "Taylor's Scientific Memoirs");
    assert_eq!(
        lovelace["issued"]["date-parts"],
        serde_json::json!([[1843]])
    );
}

#[test]
fn malformed_input_warns_and_keeps_what_it_can() {
    let bib = parse(&fixture_text("malformed"));
    let keys: Vec<&str> = bib.entries.iter().map(|e| e.key.as_str()).collect();
    assert!(keys.contains(&"ok1"), "{keys:?}");
    assert!(
        keys.contains(&"ok_last"),
        "entries after damage are read: {keys:?}"
    );
    let warnings = warnings_text(&bib).join("\n");
    assert!(warnings.contains("duplicate key ok1"), "{warnings}");
    assert!(
        warnings.contains("undefined macro undefinedmacro"),
        "{warnings}"
    );
    assert!(warnings.contains("unterminated"), "{warnings}");
    assert!(
        warnings.contains("expected a value") || warnings.contains("expected name"),
        "{warnings}"
    );
    // The first `ok1` is kept with its own value.
    assert_eq!(entry(&bib, "ok1").value("title"), Some("Fine"));
    // The last entry has no closing delimiter: it is kept with the fields read.
    assert_eq!(entry(&bib, "unclosed").value("title"), Some("never closes"));
}

#[test]
fn write_then_parse_gives_the_same_content() {
    for name in FIXTURES {
        let original = parse(&fixture_text(name));
        let written = write(&original);
        let again = parse(&written);
        assert!(
            same_content(&original, &again),
            "round trip changed {name}; written:\n{written}"
        );
        if original.warnings.is_empty() {
            assert!(
                again.warnings.is_empty(),
                "{name}: {:?}",
                warnings_text(&again)
            );
        }
        // A second write is byte-identical to the first.
        assert_eq!(write(&again), written, "{name}");
    }
}

#[test]
fn golden_files_for_every_fixture() {
    let regenerate = std::env::var("BIBTEX_REGENERATE").is_ok();
    for name in FIXTURES {
        let bib = parse(&fixture_text(name));
        let bib_out = write(&bib);
        let json_out = serde_json::to_string_pretty(&csl_json(&bib)).unwrap_or_default() + "\n";
        let base = format!(
            "{}/tests/fixtures/bibtex/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        if regenerate {
            std::fs::write(format!("{base}.expected.bib"), &bib_out).unwrap_or_default();
            std::fs::write(format!("{base}.expected.json"), &json_out).unwrap_or_default();
            continue;
        }
        let want_bib = std::fs::read_to_string(format!("{base}.expected.bib")).unwrap_or_default();
        let want_json =
            std::fs::read_to_string(format!("{base}.expected.json")).unwrap_or_default();
        assert_eq!(bib_out, want_bib, "{name}.expected.bib");
        assert_eq!(json_out, want_json, "{name}.expected.json");
    }
}

#[test]
fn limits_stop_reading_with_a_warning() {
    let text = fixture_text("basic");
    let small = Limits {
        max_input_bytes: 10,
        ..Limits::default()
    };
    let bib = parse_with(&text, &small);
    assert!(bib.entries.is_empty());
    assert!(warnings_text(&bib)[0].contains("byte limit"));

    let few = Limits {
        max_entries: 2,
        ..Limits::default()
    };
    let bib = parse_with(&text, &few);
    assert_eq!(bib.entries.len(), 2);
    assert!(warnings_text(&bib)
        .iter()
        .any(|m| m.contains("entry limit")));

    let fields = Limits {
        max_fields: 1,
        ..Limits::default()
    };
    let bib = parse_with(&text, &fields);
    assert!(warnings_text(&bib)
        .iter()
        .any(|m| m.contains("field limit")));

    let depth = Limits {
        max_depth: 3,
        ..Limits::default()
    };
    let deep = "@article{d, title = {{{{{{deep}}}}}}}";
    let bib = parse_with(deep, &depth);
    assert!(warnings_text(&bib).iter().any(|m| m.contains("nesting")));
    assert!(bib.entries.is_empty() || bib.entries[0].value("title").is_none());

    let value = Limits {
        max_value_bytes: 8,
        ..Limits::default()
    };
    let bib = parse_with("@article{v, title = {0123456789abcdef}}", &value);
    assert!(warnings_text(&bib)
        .iter()
        .any(|m| m.contains("value limit")));
}

#[test]
fn unterminated_and_empty_inputs_do_not_panic() {
    for s in [
        "",
        "@",
        "@article",
        "@article{",
        "@article{k, title = {",
        "@string{",
        "@comment{x",
    ] {
        let bib = parse(s);
        let _ = write(&bib);
        let _ = csl_json(&bib);
    }
}

// ----- seeded truncation and mutation ---------------------------------------------------------

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Any input must parse, write, export and round trip without panicking. A round trip that
/// changes the content is a failure.
fn exercise(text: &str) {
    let bib = parse(text);
    let written = write(&bib);
    let again = parse(&written);
    assert!(
        same_content(&bib, &again),
        "round trip changed the content of input:\n{text}\nwritten:\n{written}"
    );
    let _ = csl_json(&bib);
}

#[test]
fn truncated_fixtures_never_panic_and_round_trip() {
    for name in FIXTURES {
        let text = fixture_text(name);
        let len = text.len();
        let cuts = (0..=len).step_by(5).chain(len.saturating_sub(40)..=len);
        for cut in cuts {
            if text.is_char_boundary(cut) {
                exercise(&text[..cut]);
            }
        }
    }
}

#[test]
fn mutated_fixtures_never_panic_and_round_trip() {
    const ALPHABET: &[u8] = b"{}()\"#,=@\\'`~ \nabZ019-";
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    for name in FIXTURES {
        let text = fixture_text(name);
        for _ in 0..150 {
            let mut bytes = text.clone().into_bytes();
            for _ in 0..1 + rng.below(4) {
                if bytes.is_empty() {
                    break;
                }
                let at = rng.below(bytes.len());
                bytes[at] = ALPHABET[rng.below(ALPHABET.len())];
            }
            // Mutations keep ASCII, so the result is valid UTF-8.
            if let Ok(s) = String::from_utf8(bytes) {
                exercise(&s);
            }
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Xorshift(0x0123_4567_89AB_CDEF);
    for round in 0..300 {
        let mut s = String::from("@article{k, title = {");
        let length = rng.below(300) + round % 5;
        for _ in 0..length {
            let c = match rng.below(12) {
                0 => '{',
                1 => '}',
                2 => '"',
                3 => '#',
                4 => '@',
                5 => ',',
                6 => '\\',
                7 => '=',
                8 => 'é',
                _ => (b'a' + rng.below(26) as u8) as char,
            };
            s.push(c);
        }
        exercise(&s);
    }
}
