//! Module tests: fixtures with golden Markdown, structure checks, round trips, writer checks,
//! limits, equivalence with the previous `document.rs` converter, and seeded truncation and
//! mutation runs.
//!
//! Golden files live in `tests/fixtures/rtf/<name>.expected.md`. To regenerate them after an
//! intended output change, run `RTF_REGENERATE=1 cargo test --lib rtf::tests::golden`, then review
//! the diff.

use super::*;

const FIXTURES: &[&str] = &[
    "word-basic.rtf",
    "wordpad-sizes.rtf",
    "libreoffice-export.rtf",
    "cp1251-cyrillic.rtf",
    "cp1253-greek.rtf",
    "cp1255-hebrew.rtf",
    "cp932-japanese.rtf",
    "cp936-chinese.rtf",
    "unicode-surrogates.rtf",
    "mac-roman.rtf",
    "tables-nested.rtf",
    "hyperlinks.rtf",
    "lists.rtf",
    "notes-headers.rtf",
    "picture-bin.rtf",
    "headings-large-bold.rtf",
    "nested-groups.rtf",
    "malformed-unclosed.rtf",
    "malformed-truncated.rtf",
    "malformed-bin-overrun.rtf",
    "malformed-trailing.rtf",
    "malformed-unknown-codepage.rtf",
    "malformed-controls.rtf",
];

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/rtf/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn runs_text(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

fn block_text(block: &Block) -> String {
    match block {
        Block::Paragraph(p) => runs_text(&p.runs),
        Block::ListItem(item) => runs_text(&item.runs),
        Block::Table(table) => table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        cell.blocks
                            .iter()
                            .map(block_text)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Trimmed plain text of each top-level block.
fn texts(doc: &RtfDocument) -> Vec<String> {
    doc.blocks
        .iter()
        .map(|b| block_text(b).trim().to_string())
        .collect()
}

fn warnings_text(doc: &RtfDocument) -> Vec<String> {
    doc.warnings.iter().map(|w| w.message.clone()).collect()
}

fn parse_fixture(name: &str) -> RtfDocument {
    parse(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn markdown_of(data: &[u8]) -> String {
    convert(data).expect("convert")
}

// ----- golden output and structure ------------------------------------------------------------

#[test]
fn golden_markdown_for_every_fixture() {
    let regenerate = std::env::var_os("RTF_REGENERATE").is_some();
    for name in FIXTURES {
        let actual = markdown_of(&fixture(name));
        let golden = fixture_path(&format!("{}.expected.md", name.trim_end_matches(".rtf")));
        if regenerate {
            std::fs::write(&golden, &actual).expect("write golden");
        }
        let expected = std::fs::read_to_string(&golden)
            .unwrap_or_else(|e| panic!("{golden}: {e} (run with RTF_REGENERATE=1)"));
        assert_eq!(actual, expected, "golden mismatch for {name}");
    }
}

#[test]
fn word_export_metadata_headings_and_paragraphs() {
    let doc = parse_fixture("word-basic.rtf");
    assert_eq!(doc.metadata.title, "Quarterly Report \u{2013} Draft");
    assert_eq!(doc.metadata.author, "Jane O\u{2019}Brien");
    assert_eq!(doc.metadata.subject, "Fees");
    assert_eq!(doc.metadata.generator, "Microsoft Word 16.0");
    assert_eq!(doc.metadata.created, "2024-03-07T09:05");
    assert_eq!(doc.metadata.modified, "2024-03-09T17:40");
    assert!(doc.warnings.is_empty(), "{:?}", warnings_text(&doc));

    let t = texts(&doc);
    assert_eq!(t[0], "Summary");
    assert_eq!(
        doc.blocks[0],
        Block::Paragraph(Paragraph {
            runs: vec![Run {
                text: "Summary".into(),
                style: Style {
                    bold: true,
                    size_half_points: 32,
                    ..Style::default()
                },
            }],
            heading_level: Some(1),
            ..Paragraph::default()
        })
    );
    // Non-breaking space from \'a0 and \~ both survive; smart quotes come from \lquote and \rquote.
    assert_eq!(
        t[1],
        "1.\tThe parties agreed to the terms\u{a0}\u{a0} set out in \u{2018}Schedule A\u{2019}."
    );
    assert_eq!(t[2], "2.\tFees are \u{a3}500 \u{2013} payable on the 1st.");
    assert_eq!(
        t[3],
        "3.\tCaf\u{e9} owner\u{2019}s name: Jos\u{e9} M\u{fc}ller."
    );
    assert_eq!(
        t[4],
        "Section \u{a7} 12 \u{a9} 2024 ACME Corp.\nAddress line two"
    );
    assert_eq!(t[5], "Indented\ttabbed text.");
    let Block::Paragraph(last) = &doc.blocks[6] else {
        panic!("expected a paragraph");
    };
    assert_eq!(last.left_indent_twips, 720);
    let Block::Paragraph(first) = &doc.blocks[1] else {
        panic!("expected a paragraph");
    };
    assert_eq!(first.alignment, Alignment::Justify);
}

#[test]
fn codepages_decode_per_font_and_document() {
    assert_eq!(
        texts(&parse_fixture("cp1251-cyrillic.rtf"))[0],
        "Привет мир!"
    );
    assert_eq!(texts(&parse_fixture("cp1253-greek.rtf"))[0], "Αθήνα");
    assert_eq!(texts(&parse_fixture("cp1255-hebrew.rtf"))[0], "שלום");
    let japanese = texts(&parse_fixture("cp932-japanese.rtf"));
    assert_eq!(japanese[0], "こんにちは");
    assert_eq!(japanese[1], "Mixed 日本語text.");
    assert_eq!(texts(&parse_fixture("cp936-chinese.rtf"))[0], "中文");
    assert_eq!(
        texts(&parse_fixture("mac-roman.rtf"))[0],
        "Caf\u{e9} \u{c4} \u{201c}quoted\u{201d} \u{2022} \u{20ac}"
    );
}

#[test]
fn unicode_escapes_skip_fallbacks_and_join_surrogates() {
    let doc = parse_fixture("unicode-surrogates.rtf");
    let t = texts(&doc);
    // \u233 with \uc1 skips one fallback character; 荤 with \'80 is one euro sign.
    assert_eq!(t[0], "caf\u{e9} and \u{20ac}\u{20ac} euro.");
    // \uc2 skips two fallback units: the \'80 and the x.
    assert_eq!(t[1], "Two-byte fallbacks: \u{20ac} done.");
    // A surrogate pair given as two \u values becomes one character; a lone high surrogate
    // becomes U+FFFD.
    assert_eq!(t[2], "Emoji: \u{1f600} and a lone \u{fffd} here.");
    assert_eq!(
        t[3],
        "Curly \u{201c}quoted\u{201d} text, dash \u{2013}, ellipsis \u{2026}."
    );
}

#[test]
fn tables_nest_and_mark_the_header_row() {
    let doc = parse_fixture("tables-nested.rtf");
    assert_eq!(doc.blocks.len(), 2, "one table then one paragraph");
    let Block::Table(outer) = &doc.blocks[0] else {
        panic!("expected a table");
    };
    assert_eq!(outer.rows.len(), 3);
    assert!(outer.rows[0].header);
    assert!(!outer.rows[1].header);
    assert_eq!(outer.rows[0].cells.len(), 2);
    assert_eq!(block_text(&outer.rows[0].cells[0].blocks[0]), "Field");
    assert_eq!(outer.rows[1].cells.len(), 2);
    let Block::Table(inner) = &outer.rows[1].cells[1].blocks[0] else {
        panic!("expected a nested table in the second cell");
    };
    assert_eq!(inner.rows.len(), 2);
    assert_eq!(block_text(&inner.rows[0].cells[0].blocks[0]), "Claimant");
    assert_eq!(block_text(&inner.rows[1].cells[1].blocks[0]), "Beta plc");
    assert_eq!(texts(&doc)[1], "After the table.");
}

#[test]
fn hyperlink_fields_keep_their_targets() {
    let doc = parse_fixture("hyperlinks.rtf");
    let targets: Vec<&str> = doc.hyperlinks.iter().map(|h| h.target.as_str()).collect();
    assert_eq!(
        targets,
        vec![
            "https://example.test/docs?a=1&b=2",
            "#_Ref123",
            "mailto:jane@example.test",
        ]
    );
    let Block::Paragraph(p) = &doc.blocks[0] else {
        panic!("expected a paragraph");
    };
    let linked: Vec<(&str, Option<usize>)> = p
        .runs
        .iter()
        .map(|r| (r.text.as_str(), r.style.link))
        .collect();
    assert!(linked.contains(&("our site", Some(0))), "{linked:?}");
    // PAGE is a field with no HYPERLINK instruction, so its result is plain text.
    let Block::Paragraph(page) = &doc.blocks[3] else {
        panic!("expected a paragraph");
    };
    assert!(page.runs.iter().all(|r| r.style.link.is_none()));
}

#[test]
fn list_items_carry_level_order_and_marker() {
    let doc = parse_fixture("lists.rtf");
    let items: Vec<(u8, bool, String, String)> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::ListItem(i) => Some((i.level, i.ordered, i.marker.clone(), runs_text(&i.runs))),
            _ => None,
        })
        .collect();
    assert_eq!(items.len(), 5);
    assert_eq!(
        items[0],
        (0, false, "\u{2022}".into(), "First bullet".into())
    );
    assert_eq!(items[1].0, 1, "nested level from \\ilvl1");
    assert_eq!(items[3], (0, true, "1.".into(), "First number".into()));
    assert_eq!(items[4].2, "2.");
    assert_eq!(texts(&doc)[5], "Paragraph after the lists.");
}

#[test]
fn footnotes_are_collected_and_headers_are_optional() {
    let doc = parse_fixture("notes-headers.rtf");
    let numbers: Vec<u32> = doc.footnotes.iter().map(|f| f.number).collect();
    assert_eq!(numbers, vec![1, 2]);
    // The space after \chftn is the control word's delimiter; the second space is text.
    assert_eq!(
        block_text(&doc.footnotes[0].blocks[0]),
        " Footnote body text."
    );
    assert_eq!(
        doc.footnotes[1]
            .blocks
            .iter()
            .map(|b| block_text(b).trim().to_string())
            .collect::<Vec<_>>(),
        vec!["Second note, two", "paragraphs."]
    );
    assert!(
        doc.parts.is_empty(),
        "headers and footers are off by default"
    );
    assert_eq!(texts(&doc)[0], "Body with a note[1] and text after.");

    let options = Options {
        include_headers_footers: true,
        ..Options::default()
    };
    let with_parts = parse_with(&fixture("notes-headers.rtf"), &options).unwrap();
    let kinds: Vec<PartKind> = with_parts.parts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PartKind::Header, PartKind::Footer]);
    assert_eq!(
        block_text(&with_parts.parts[0].blocks[0]),
        "Confidential header"
    );

    let no_notes = Options {
        include_footnotes: false,
        ..Options::default()
    };
    let plain = parse_with(&fixture("notes-headers.rtf"), &no_notes).unwrap();
    assert!(plain.footnotes.is_empty());
    assert_eq!(texts(&plain)[0], "Body with a note and text after.");
}

#[test]
fn pictures_and_generator_destinations_add_no_text() {
    let doc = parse_fixture("picture-bin.rtf");
    assert_eq!(
        texts(&doc),
        vec!["Before picture", "Between pictures", "After the picture."]
    );
    assert!(doc.warnings.is_empty(), "{:?}", warnings_text(&doc));
    let gen = parse_fixture("word-basic.rtf");
    assert!(!texts(&gen).iter().any(|t| t.contains("Microsoft")));
}

#[test]
fn headings_come_from_levels_styles_and_large_bold_text() {
    let doc = parse_fixture("headings-large-bold.rtf");
    let levels: Vec<Option<u8>> = doc
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.heading_level,
            _ => None,
        })
        .collect();
    assert_eq!(
        levels,
        vec![Some(1), Some(2), None, Some(2), Some(3), None, None]
    );

    let plain = parse(&fixture("headings-large-bold.rtf")).unwrap();
    assert!(plain.blocks.iter().all(|b| match b {
        Block::Paragraph(p) => p.heading_level.is_some() || p.runs.iter().all(|r| r.style.bold),
        _ => true,
    }));

    let no_detection = Options {
        detect_headings: false,
        ..Options::default()
    };
    let off = parse_with(&fixture("headings-large-bold.rtf"), &no_detection).unwrap();
    let off_levels: Vec<Option<u8>> = off
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.heading_level,
            _ => None,
        })
        .collect();
    // Style and outline levels still apply; the size heuristic does not.
    assert_eq!(off_levels[0], None);
    assert_eq!(off_levels[1], None);
    assert_eq!(off_levels[3], Some(2));
    assert_eq!(off_levels[4], Some(3));
}

#[test]
fn nested_groups_read_to_the_bottom() {
    let doc = parse_fixture("nested-groups.rtf");
    assert_eq!(texts(&doc), vec!["deep text"]);
    assert!(doc.warnings.is_empty());
}

#[test]
fn malformed_inputs_warn_and_keep_what_they_can() {
    let unclosed = parse_fixture("malformed-unclosed.rtf");
    assert_eq!(texts(&unclosed), vec!["Hello", "bold text"]);
    assert!(warnings_text(&unclosed)
        .iter()
        .any(|w| w.starts_with("truncated group")));

    let truncated = parse_fixture("malformed-truncated.rtf");
    assert_eq!(texts(&truncated), vec!["Caf"]);
    assert!(warnings_text(&truncated)
        .iter()
        .any(|w| w == "invalid or truncated hex escape"));

    let overrun = parse_fixture("malformed-bin-overrun.rtf");
    assert_eq!(texts(&overrun), vec!["Before"]);
    assert!(warnings_text(&overrun)
        .iter()
        .any(|w| w.starts_with("truncated binary data")));

    let trailing = parse_fixture("malformed-trailing.rtf");
    assert_eq!(texts(&trailing), vec!["Body text"]);
    assert!(warnings_text(&trailing)
        .iter()
        .any(|w| w.starts_with("ignored content after the end")));

    let codepage = parse_fixture("malformed-unknown-codepage.rtf");
    let warnings = warnings_text(&codepage);
    assert!(warnings.contains(&"unknown code page 99999, used cp1252".to_string()));
    assert!(warnings.contains(&"code page 1361 is not supported, used cp1252".to_string()));
    assert_eq!(texts(&codepage)[0], "\u{c8}t\u{c8} \u{e49}");

    let controls = parse_fixture("malformed-controls.rtf");
    assert!(warnings_text(&controls)
        .iter()
        .any(|w| w == "\\u without a code point"));
    assert_eq!(texts(&controls)[0], "zz bad");
}

#[test]
fn invalid_header_and_unbalanced_close_are_handled() {
    assert_eq!(parse(b"plain text"), Err(RtfError::InvalidHeader));
    assert_eq!(
        convert(b"plain text"),
        Err("invalid RTF header".to_string())
    );
    let doc = parse(b"  \r\n{\\rtf1 A}\n}").unwrap();
    assert_eq!(texts(&doc), vec!["A"]);
    // A stray brace after the document is reported, and the document is kept.
    let doc = parse(b"{\\rtf1 A}}").unwrap();
    assert_eq!(texts(&doc), vec!["A"]);
    assert!(warnings_text(&doc)
        .iter()
        .any(|w| w.starts_with("ignored content after the end")));
}

// ----- limits and options ---------------------------------------------------------------------

#[test]
fn exceeding_a_limit_is_an_explicit_error() {
    let deep = fixture("nested-groups.rtf");
    let shallow = Options {
        limits: Limits {
            max_depth: 10,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(
        parse_with(&deep, &shallow),
        Err(RtfError::TooDeep { limit: 10 })
    );

    let small_input = Options {
        limits: Limits {
            max_input_bytes: 8,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(
        parse_with(&deep, &small_input),
        Err(RtfError::InputTooLarge {
            size: deep.len(),
            limit: 8
        })
    );

    let small_output = Options {
        limits: Limits {
            max_output_bytes: 16,
            ..Limits::default()
        },
        ..Options::default()
    };
    let big = fixture("word-basic.rtf");
    assert_eq!(
        parse_with(&big, &small_output),
        Err(RtfError::OutputTooLarge { limit: 16 })
    );
}

#[test]
fn table_style_and_heading_options_change_rendering() {
    let tabs = Options {
        table_style: TableStyle::Tabs,
        ..Options::default()
    };
    let doc = parse_fixture("tables-nested.rtf");
    let md = to_markdown(&doc, &tabs).unwrap();
    assert!(md.starts_with("Field\tDetail\nParties\t"), "{md}");
    let text = to_text(&doc, &Options::default()).unwrap();
    assert!(text.starts_with("Field | Detail"), "{text}");
}

#[test]
fn plain_text_rendering_has_no_markup() {
    let text = to_text(&parse_fixture("word-basic.rtf"), &Options::default()).unwrap();
    assert!(text.starts_with("Summary\n\n1.\tThe parties"), "{text}");
    assert!(!text.contains("**"));
    assert!(!text.contains('#'));
}

// ----- writer and round trips -----------------------------------------------------------------

/// The parts of a document that the writer keeps. Warnings and header/footer parts are not
/// written, so they are cleared before comparison.
fn written_subset(mut doc: RtfDocument) -> RtfDocument {
    doc.warnings.clear();
    doc.parts.clear();
    doc
}

#[test]
fn writer_output_is_ascii_and_reads_back_with_the_same_structure() {
    for name in FIXTURES {
        let doc = parse_fixture(name);
        let bytes = to_rtf(&doc);
        assert!(bytes.is_ascii(), "{name}: writer output is not ASCII");
        let again = parse(&bytes).unwrap_or_else(|e| panic!("{name}: re-read failed: {e}"));
        assert_eq!(
            written_subset(again),
            written_subset(doc),
            "round trip changed the structure of {name}"
        );
    }
}

#[test]
fn writer_escapes_markup_and_uses_unicode_escapes_beyond_cp1252() {
    let text = "a\\b {c} \u{e9}\u{20ac}\u{1f600}\tx\ny";
    let doc = RtfDocument {
        blocks: vec![Block::Paragraph(Paragraph {
            runs: vec![Run {
                text: text.into(),
                style: Style::default(),
            }],
            ..Paragraph::default()
        })],
        ..RtfDocument::default()
    };
    let bytes = to_rtf(&doc);
    let source = String::from_utf8(bytes.clone()).unwrap();
    // Characters in cp1252 use \'hh (the euro sign is 0x80); others use \u with a fallback.
    assert!(
        source.contains("a\\\\b \\{c\\} \\'e9\\'80\\u-10179?\\u-8704?\\tab x\\line y"),
        "{source}"
    );
    assert!(source.contains("\\uc1"));
    let back = parse(&bytes).unwrap();
    assert_eq!(texts(&back)[0], text);
}

#[test]
fn markdown_to_rtf_round_trips_through_markdown() {
    let source = "# Title\n\nSome **bold** and *italic* text with [a link](https://example.test/).\n\n- one\n- two\n\n1. first\n2. second\n\n| a | b |\n| --- | --- |\n| 1 | 2 |";
    let bytes = markdown_to_rtf(source);
    assert!(bytes.is_ascii());
    let doc = parse(&bytes).unwrap();
    let md = to_markdown(&doc, &Options::default()).unwrap();
    assert_eq!(md, source);
}

#[test]
fn writer_declares_font_table_and_code_page() {
    let bytes = to_rtf(&RtfDocument::default());
    let source = String::from_utf8(bytes).unwrap();
    assert!(source.starts_with("{\\rtf1\\ansi\\ansicpg1252"));
    assert!(source.contains("{\\fonttbl{\\f0"));
    assert!(source.contains("{\\stylesheet"));
}

// ----- equivalence with the previous converter ------------------------------------------------

/// The `rtf()` function from `src/document.rs` before this change (commit a2dacab), copied here
/// so that the equivalence tests can compare against it.
#[allow(clippy::all)]
mod legacy {
    pub fn rtf(data: &[u8]) -> Result<String, String> {
        if !data.starts_with(b"{\\rtf") {
            return Err("invalid RTF header".into());
        }
        let mut out = String::new();
        let mut hidden = false;
        let mut stack = Vec::new();
        let mut i = 0usize;
        while i < data.len() {
            match data[i] {
                b'{' => {
                    stack.push(hidden);
                    i += 1;
                }
                b'}' => {
                    hidden = stack.pop().ok_or("unbalanced RTF group")?;
                    i += 1;
                }
                b'\\' => {
                    i += 1;
                    let Some(&next) = data.get(i) else {
                        break;
                    };
                    if matches!(next, b'\\' | b'{' | b'}') {
                        if !hidden {
                            out.push(next as char);
                        }
                        i += 1;
                    } else if next == b'\'' {
                        let hex = data.get(i + 1..i + 3).ok_or("truncated RTF hex escape")?;
                        let hex = std::str::from_utf8(hex).map_err(|e| e.to_string())?;
                        let byte = u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                        if !hidden {
                            out.push_str(&encoding_rs::WINDOWS_1252.decode(&[byte]).0);
                        }
                        i += 3;
                    } else if next == b'*' {
                        hidden = true;
                        i += 1;
                    } else if next.is_ascii_alphabetic() {
                        let start = i;
                        while data.get(i).is_some_and(u8::is_ascii_alphabetic) {
                            i += 1;
                        }
                        let word = &data[start..i];
                        let number_start = i;
                        if data.get(i) == Some(&b'-') {
                            i += 1;
                        }
                        while data.get(i).is_some_and(u8::is_ascii_digit) {
                            i += 1;
                        }
                        let number = std::str::from_utf8(&data[number_start..i])
                            .ok()
                            .and_then(|s| s.parse::<i32>().ok());
                        if data.get(i) == Some(&b' ') {
                            i += 1;
                        }
                        match word {
                            b"bin" => {
                                let count = number.ok_or("RTF bin length missing")?;
                                let count = usize::try_from(count)
                                    .map_err(|_| "negative RTF bin length")?;
                                i = i
                                    .checked_add(count)
                                    .filter(|end| *end <= data.len())
                                    .ok_or("truncated RTF binary run")?;
                            }
                            b"fonttbl" | b"colortbl" | b"stylesheet" | b"pict" | b"object"
                            | b"info" => hidden = true,
                            b"par" | b"line" if !hidden => out.push_str("\n\n"),
                            b"tab" if !hidden => out.push('\t'),
                            b"u" if !hidden => {
                                if let Some(value) = number {
                                    let codepoint = (value as i16 as u16) as u32;
                                    if let Some(ch) = char::from_u32(codepoint) {
                                        out.push(ch);
                                    }
                                }
                                if data.get(i) == Some(&b'?') {
                                    i += 1;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        if !hidden && next == b'~' {
                            out.push(' ');
                        }
                        i += 1;
                    }
                }
                b'\r' | b'\n' => i += 1,
                byte => {
                    if !hidden {
                        out.push(byte as char);
                    }
                    i += 1;
                }
            }
        }
        if !stack.is_empty() {
            return Err("unclosed RTF group".into());
        }
        Ok(out.trim().to_string())
    }
}

#[test]
fn plain_text_matches_the_previous_converter_on_inputs_it_handled() {
    let inputs: &[&[u8]] = &[
        br"{\rtf1\ansi Hello\par world \'e9}",
        br"{\rtf1\ansi plain {\b bold} text\par Second line.\par}",
        br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\colortbl ;\red0\green0\blue0;}Text with \tab tab.\par}",
        br"{\rtf1\ansi \'93Smart\'94 quotes \'96 dash\par}",
        br"{\rtf1\ansi\ansicpg1252 caf\'e9 \u233?\u8364? \par}",
        br"{\rtf1\ansi{\*\generator Test;}Visible{\*\unknown hidden}\par}",
        b"{\\rtf1\\ansi line one\r\nline two\\par}",
        br"{\rtf1\ansi{\pict\wmetafile8 \bin4 abcd}after\par}",
        br"{\rtf1\ansi Escapes: \\ \{ \} done.\par}",
    ];
    for input in inputs {
        let old = legacy::rtf(input).expect("legacy converter accepts this input");
        let new = to_text(&parse(input).unwrap(), &Options::default()).unwrap();
        assert_eq!(new, old, "input {:?}", String::from_utf8_lossy(input));
    }
}

#[test]
fn header_error_matches_the_previous_converter() {
    assert_eq!(
        legacy::rtf(b"not rtf").unwrap_err(),
        convert(b"not rtf").unwrap_err()
    );
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

/// Any input must produce `Ok` or `Err` and nothing else. The test harness reports a panic.
fn exercise(data: &[u8]) {
    if let Ok(doc) = parse(data) {
        let options = Options::default();
        let _ = to_markdown(&doc, &options);
        let _ = to_text(&doc, &options);
        let _ = to_rtf(&doc);
    }
}

#[test]
fn truncated_and_mutated_fixtures_never_panic_or_hang() {
    const ALPHABET: &[u8] = b"{}\\'*\r\n 0123456789abcdefzxu~-_|:\x00\xff\x80";
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    for name in FIXTURES {
        let data = fixture(name);
        let len = data.len();
        let cuts = (0..=len).step_by(7).chain(len.saturating_sub(64)..=len);
        for cut in cuts {
            exercise(&data[..cut]);
        }
        for _ in 0..200 {
            let mut mutated = data.clone();
            for _ in 0..1 + rng.below(4) {
                if mutated.is_empty() {
                    break;
                }
                let at = rng.below(mutated.len());
                mutated[at] = ALPHABET[rng.below(ALPHABET.len())];
            }
            exercise(&mutated);
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Xorshift(0x0123_4567_89AB_CDEF);
    for round in 0..300 {
        let mut data = b"{\\rtf1 ".to_vec();
        let length = rng.below(400) + round % 7;
        for _ in 0..length {
            data.push((rng.next() & 0xff) as u8);
        }
        exercise(&data);
    }
}
