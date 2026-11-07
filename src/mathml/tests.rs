//! MathML converter tests. The inputs follow the shape LaTeXML and MathJax write, with and
//! without the TeX annotation.

use super::*;

fn tex(markup: &str) -> String {
    to_tex(markup).tex
}

fn wrap(body: &str) -> String {
    format!("<math xmlns=\"http://www.w3.org/1998/Math/MathML\">{body}</math>")
}

#[test]
fn tex_annotation_is_returned_unchanged() {
    let markup = "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" alttext=\"x^2\"><semantics><mrow><msup><mi>x</mi><mn>2</mn></msup></mrow><annotation encoding=\"application/x-tex\">x^{2} + \\alpha</annotation></semantics></math>";
    let c = to_tex(markup);
    assert_eq!(c.tex, "x^{2} + \\alpha");
    assert!(c.from_annotation);
    assert!(c.warnings.is_empty());
}

#[test]
fn annotation_can_be_ignored() {
    let markup = "<math><semantics><mi>x</mi><annotation encoding=\"application/x-tex\">X</annotation></semantics></math>";
    let c = to_tex_with(
        markup,
        &Options {
            prefer_annotation: false,
            ..Options::default()
        },
    );
    assert_eq!(c.tex, "x");
    assert!(!c.from_annotation);
}

#[test]
fn scripts_fractions_and_roots() {
    assert_eq!(tex(&wrap("<msup><mi>x</mi><mn>2</mn></msup>")), "x^2");
    assert_eq!(
        tex(&wrap(
            "<msub><mi>a</mi><mrow><mi>i</mi><mo>+</mo><mn>1</mn></mrow></msub>"
        )),
        "a_{i + 1}"
    );
    assert_eq!(
        tex(&wrap("<msubsup><mi>x</mi><mi>i</mi><mn>2</mn></msubsup>")),
        "x_i^2"
    );
    assert_eq!(
        tex(&wrap("<mfrac><mi>a</mi><msqrt><mi>b</mi></msqrt></mfrac>")),
        "\\frac{a}{\\sqrt{b}}"
    );
    assert_eq!(
        tex(&wrap("<mroot><mi>x</mi><mn>3</mn></mroot>")),
        "\\sqrt[3]{x}"
    );
    assert_eq!(
        tex(&wrap(
            "<mfrac linethickness=\"0\"><mi>n</mi><mi>k</mi></mfrac>"
        )),
        "\\binom{n}{k}"
    );
}

#[test]
fn big_operators_take_limits_as_scripts() {
    let markup = wrap(
        "<munderover><mo>∑</mo><mrow><mi>i</mi><mo>=</mo><mn>1</mn></mrow><mi>n</mi></munderover>",
    );
    assert_eq!(tex(&markup), "\\sum_{i = 1}^n");
    let markup = wrap("<munder><mo>∫</mo><mn>0</mn></munder>");
    assert_eq!(tex(&markup), "\\int_0");
}

#[test]
fn over_and_under_with_accents_and_other_bases() {
    assert_eq!(
        tex(&wrap("<mover><mi>x</mi><mo>¯</mo></mover>")),
        "\\bar{x}"
    );
    assert_eq!(
        tex(&wrap("<mover><mi>x</mi><mo>→</mo></mover>")),
        "\\vec{x}"
    );
    assert_eq!(
        tex(&wrap("<mover><mi>y</mi><mi>z</mi></mover>")),
        "\\overset{z}{y}"
    );
    assert_eq!(
        tex(&wrap("<munder><mi>y</mi><mi>z</mi></munder>")),
        "\\underset{z}{y}"
    );
    assert_eq!(
        tex(&wrap(
            "<munderover><mi>y</mi><mi>a</mi><mi>b</mi></munderover>"
        )),
        "\\underset{a}{\\overset{b}{y}}"
    );
}

#[test]
fn identifiers_map_greek_functions_and_words() {
    assert_eq!(tex(&wrap("<mi>α</mi>")), "\\alpha");
    assert_eq!(tex(&wrap("<mi>Ω</mi>")), "\\Omega");
    assert_eq!(tex(&wrap("<mi>sin</mi>")), "\\sin");
    assert_eq!(tex(&wrap("<mi>ab</mi>")), "\\mathrm{ab}");
    assert_eq!(tex(&wrap("<mi mathvariant=\"bold\">v</mi>")), "\\mathbf{v}");
    assert_eq!(tex(&wrap("<mi>ℝ</mi>")), "\\mathbb{R}");
    assert_eq!(tex(&wrap("<mi>{</mi>")), "\\{");
}

#[test]
fn operators_numbers_text_and_spaces() {
    assert_eq!(tex(&wrap("<mi>x</mi><mo>≤</mo><mn>3.5</mn>")), "x \\le 3.5");
    assert_eq!(tex(&wrap("<mo>−</mo><mi>b</mi>")), "- b");
    assert_eq!(tex(&wrap("<mo>&lt;</mo>")), "<");
    assert_eq!(tex(&wrap("<mo>&amp;</mo>")), "\\&");
    assert_eq!(tex(&wrap("<mtext>if x</mtext>")), "\\text{if x}");
    assert_eq!(
        tex(&wrap("<mi>a</mi><mspace width=\"1em\"/><mi>b</mi>")),
        "a \\quad b"
    );
    assert_eq!(tex(&wrap("<mi>a</mi><mspace/><mi>b</mi>")), "a \\, b");
    // Function application is invisible.
    assert_eq!(tex(&wrap("<mi>f</mi><mo>⁡</mo><mi>x</mi>")), "f x");
}

#[test]
fn tables_and_fences() {
    let table = wrap(
        "<mtable><mtr><mtd><mi>a</mi></mtd><mtd><mi>b</mi></mtd></mtr><mtr><mtd><mi>c</mi></mtd><mtd><mi>d</mi></mtd></mtr></mtable>",
    );
    assert_eq!(
        tex(&table),
        "\\begin{array}{cc} a & b \\\\ c & d \\end{array}"
    );
    assert_eq!(
        tex(&wrap("<mfenced><mi>x</mi><mi>y</mi></mfenced>")),
        "\\left( x, y \\right)"
    );
    assert_eq!(
        tex(&wrap(
            "<mfenced open=\"[\" close=\"]\" separators=\"|\"><mi>a</mi><mi>b</mi></mfenced>"
        )),
        "\\left[ a | b \\right]"
    );
    assert_eq!(
        tex(&wrap("<menclose notation=\"box\"><mi>q</mi></menclose>")),
        "\\boxed{q}"
    );
}

#[test]
fn unknown_elements_keep_their_content_with_a_warning() {
    let c = to_tex(&wrap("<mfoo><mi>x</mi></mfoo>"));
    assert_eq!(c.tex, "x");
    assert!(
        c.warnings.iter().any(|w| w.contains("<mfoo>")),
        "{:?}",
        c.warnings
    );
}

#[test]
fn unmapped_characters_are_kept_with_a_warning() {
    let c = to_tex(&wrap("<mo>⊞</mo>"));
    assert_eq!(c.tex, "⊞");
    assert!(
        c.warnings.iter().any(|w| w.contains("U+229E")),
        "{:?}",
        c.warnings
    );
}

#[test]
fn prefixes_comments_and_declarations_are_handled() {
    let markup = "<?xml version=\"1.0\"?><!DOCTYPE math><!-- note --><m:math xmlns:m=\"http://www.w3.org/1998/Math/MathML\"><m:mi>x</m:mi><m:mo>+</m:mo><m:mi>y</m:mi></m:math>";
    assert_eq!(tex(markup), "x + y");
    let cdata = "<math><mtext><![CDATA[a < b]]></mtext></math>";
    assert_eq!(tex(cdata), "\\text{a < b}");
}

#[test]
fn malformed_markup_gives_warnings_not_panics() {
    for s in [
        "",
        "<math>",
        "<math><mi>x</mi",
        "<math><mi>x</mi></mo></math>",
        "not xml at all",
        "<math><mi>&#xZZ;</mi></math>",
        "<math><!-- unterminated",
        "<math><![CDATA[ x",
        "</math>",
        "<<<>>>",
        "<math a=\"b></math>",
    ] {
        let c = to_tex(s);
        let _ = c.tex;
    }
    let c = to_tex("not xml at all");
    assert!(
        c.warnings
            .iter()
            .any(|w| w.contains("malformed") || w.contains("no element")),
        "{:?}",
        c.warnings
    );
}

#[test]
fn depth_and_size_limits_stop_deep_input() {
    let mut deep = String::new();
    for _ in 0..200 {
        deep.push_str("<mrow>");
    }
    deep.push_str("<mi>x</mi>");
    for _ in 0..200 {
        deep.push_str("</mrow>");
    }
    let c = to_tex_with(
        &wrap(&deep),
        &Options {
            max_depth: 16,
            ..Options::default()
        },
    );
    assert!(
        c.warnings.iter().any(|w| w.contains("limit")),
        "{:?}",
        c.warnings
    );
    let c = to_tex_with(
        &wrap("<mi>x</mi>"),
        &Options {
            max_bytes: 5,
            ..Options::default()
        },
    );
    assert!(c.warnings.iter().any(|w| w.contains("byte limit")));
    let c = to_tex_with(
        &wrap("<mi>x</mi>"),
        &Options {
            max_nodes: 1,
            ..Options::default()
        },
    );
    assert!(
        c.warnings.iter().any(|w| w.contains("node limit")),
        "{:?}",
        c.warnings
    );
}

#[test]
fn latexml_style_equation_round_trips_through_its_annotation() {
    let markup = r#"<math xmlns="http://www.w3.org/1998/Math/MathML" display="block" alttext="\sum _{i=1}^{n} i = \frac{n(n+1)}{2}"><semantics><mrow><mrow><munderover><mo movablelimits="false">∑</mo><mrow><mi>i</mi><mo>=</mo><mn>1</mn></mrow><mi>n</mi></munderover><mi>i</mi></mrow><mo>=</mo><mfrac><mrow><mi>n</mi><mo>⁢</mo><mrow><mo>(</mo><mrow><mi>n</mi><mo>+</mo><mn>1</mn></mrow><mo>)</mo></mrow></mrow><mn>2</mn></mfrac></mrow><annotation encoding="application/x-tex">\sum _{i=1}^{n} i = \frac{n(n+1)}{2}</annotation></semantics></math>"#;
    let c = to_tex(markup);
    assert_eq!(c.tex, "\\sum _{i=1}^{n} i = \\frac{n(n+1)}{2}");
    // The presentation markup of the same equation converts to the same structure.
    let presentation = markup
        .split("<annotation")
        .next()
        .unwrap_or("")
        .replace("<semantics>", "")
        .replace("</semantics>", "")
        + "</math>";
    let p = to_tex_with(
        &presentation,
        &Options {
            prefer_annotation: false,
            ..Options::default()
        },
    );
    // TeX ignores the spaces, so the converter's spacing is not part of the meaning.
    assert_eq!(p.tex, "\\sum_{i = 1}^n i = \\frac{n ( n + 1 )}{2}");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}
