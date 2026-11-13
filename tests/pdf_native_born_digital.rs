//! Born-digital fixtures must come out of the native parser. `extract_file` never spawns OCR, so an
//! empty result here is a native-parser failure that the production route would hide behind a
//! 1 to 2.6 s Tesseract fallback (see webfold-bench `ocr-vs-incumbents-2026-10-14`: native returned
//! nothing for two-column, borderless-table and reportlab-ascii85).
//! Every word of the fixture's `.expected.txt` must be present in the native output, in order.
//! HTML tags are not words and thousands commas are dropped.

use std::path::Path;

fn words(s: &str) -> Vec<String> {
    let mut text = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if in_tag || c == ',' => {}
            c if c.is_alphanumeric() => text.extend(c.to_lowercase()),
            _ => text.push(' '),
        }
    }
    text.split_whitespace().map(str::to_string).collect()
}

fn assert_native(name: &str) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let native = words(
        &superdocuments::pdf::extract_file(fixtures.join(format!("{name}.pdf")))
            .unwrap_or_default(),
    );
    assert!(!native.is_empty(), "{name}: native parser returned no text");
    let expected = std::fs::read_to_string(fixtures.join(format!("{name}.expected.txt"))).unwrap();
    let mut at = 0;
    for word in words(&expected) {
        match native[at..].iter().position(|w| *w == word) {
            Some(i) => at += i + 1,
            None => panic!("{name}: expected word {word:?} missing or out of order"),
        }
    }
}

#[test]
fn two_column_is_read_natively() {
    assert_native("two-column");
}

#[test]
fn borderless_table_is_read_natively() {
    assert_native("borderless-table");
}

#[test]
fn ascii85_stream_is_read_natively() {
    assert_native("reportlab-ascii85");
}

#[test]
fn invoice_is_read_natively() {
    assert_native("reportlab-invoice");
}
