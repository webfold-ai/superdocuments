//! Office Math (OMML) to TeX, for the formulas Word writes: fractions, scripts, radicals,
//! n-ary operators, delimiters, functions, accents, limits, matrices and plain runs.

use crate::xmltree::{XElem, XNode};

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn child<'a>(e: &'a XElem, name: &str) -> Option<&'a XElem> {
    e.elems().find(|c| local(&c.name) == name)
}

fn arg(e: &XElem, name: &str) -> String {
    child(e, name).map(content).unwrap_or_default()
}

fn prop<'a>(e: &'a XElem, group: &str, name: &str) -> Option<&'a str> {
    child(e, group)
        .and_then(|g| child(g, name))
        .and_then(|c| c.attr("m:val").or_else(|| c.attr("val")))
}

fn braced(s: String) -> String {
    format!("{{{s}}}")
}

fn text_run(e: &XElem) -> String {
    let mut s = String::new();
    for c in &e.children {
        match c {
            XNode::Text(t) => s.push_str(t),
            XNode::Elem(x) if local(&x.name) == "t" => {
                for n in &x.children {
                    if let XNode::Text(t) = n {
                        s.push_str(t);
                    }
                }
            }
            _ => {}
        }
    }
    let plain = child(e, "rPr").and_then(|p| child(p, "nor")).is_some()
        || child(e, "rPr")
            .and_then(|p| child(p, "sty"))
            .and_then(|x| x.attr("m:val").or_else(|| x.attr("val")))
            == Some("p");
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\u{2212}' => out.push('-'),
            '\u{00d7}' => out.push_str("\\times "),
            '\u{00b1}' => out.push_str("\\pm "),
            '\u{2264}' => out.push_str("\\leq "),
            '\u{2265}' => out.push_str("\\geq "),
            '\u{2260}' => out.push_str("\\neq "),
            '\u{221e}' => out.push_str("\\infty "),
            '\u{03b1}' => out.push_str("\\alpha "),
            '\u{03b2}' => out.push_str("\\beta "),
            '\u{03b3}' => out.push_str("\\gamma "),
            '\u{03b4}' => out.push_str("\\delta "),
            '\u{03b5}' => out.push_str("\\epsilon "),
            '\u{03bb}' => out.push_str("\\lambda "),
            '\u{03bc}' => out.push_str("\\mu "),
            '\u{03c0}' => out.push_str("\\pi "),
            '\u{03c3}' => out.push_str("\\sigma "),
            '\u{03c9}' => out.push_str("\\omega "),
            '\u{0394}' => out.push_str("\\Delta "),
            '\u{03a3}' => out.push_str("\\Sigma "),
            '\u{2192}' => out.push_str("\\rightarrow "),
            '\u{2208}' => out.push_str("\\in "),
            '\u{2211}' => out.push_str("\\sum "),
            '\u{222b}' => out.push_str("\\int "),
            '\u{221a}' => out.push_str("\\sqrt "),
            c => out.push(c),
        }
    }
    if plain && !out.trim().is_empty() {
        format!("\\mathrm{{{}}}", out.trim())
    } else {
        out
    }
}

/// TeX for the children of an OMML element, concatenated.
pub(crate) fn content(e: &XElem) -> String {
    let mut out = String::new();
    for c in e.elems() {
        out.push_str(&node(c));
    }
    out
}

fn node(e: &XElem) -> String {
    match local(&e.name) {
        "r" => text_run(e),
        "f" => {
            let kind = prop(e, "fPr", "type");
            let (n, d) = (braced(arg(e, "num")), braced(arg(e, "den")));
            match kind {
                Some("noBar") => format!("\\binom{n}{d}"),
                Some("lin") => format!("{}/{}", arg(e, "num"), arg(e, "den")),
                _ => format!("\\frac{n}{d}"),
            }
        }
        "sSup" => format!("{{{}}}^{}", arg(e, "e"), braced(arg(e, "sup"))),
        "sSub" => format!("{{{}}}_{}", arg(e, "e"), braced(arg(e, "sub"))),
        "sSubSup" => format!(
            "{{{}}}_{}^{}",
            arg(e, "e"),
            braced(arg(e, "sub")),
            braced(arg(e, "sup"))
        ),
        "sPre" => format!(
            "{{}}_{}^{}{{{}}}",
            braced(arg(e, "sub")),
            braced(arg(e, "sup")),
            arg(e, "e")
        ),
        "rad" => {
            let deg = arg(e, "deg");
            if deg.trim().is_empty() || prop(e, "radPr", "degHide") == Some("1") {
                format!("\\sqrt{}", braced(arg(e, "e")))
            } else {
                format!("\\sqrt[{deg}]{}", braced(arg(e, "e")))
            }
        }
        "nary" => {
            let chr = prop(e, "naryPr", "chr").unwrap_or("\u{222b}");
            let op = match chr {
                "\u{2211}" => "\\sum",
                "\u{220f}" => "\\prod",
                "\u{222c}" => "\\iint",
                "\u{222e}" => "\\oint",
                "\u{22c3}" => "\\bigcup",
                "\u{22c2}" => "\\bigcap",
                _ => "\\int",
            };
            let (sub, sup) = (arg(e, "sub"), arg(e, "sup"));
            let mut s = op.to_string();
            if !sub.trim().is_empty() {
                s.push_str(&format!("_{}", braced(sub)));
            }
            if !sup.trim().is_empty() {
                s.push_str(&format!("^{}", braced(sup)));
            }
            format!("{s} {}", arg(e, "e"))
        }
        "d" => {
            let beg = prop(e, "dPr", "begChr").unwrap_or("(");
            let end = prop(e, "dPr", "endChr").unwrap_or(")");
            let sep = prop(e, "dPr", "sepChr").unwrap_or("|");
            let parts: Vec<String> = e
                .elems()
                .filter(|c| local(&c.name) == "e")
                .map(content)
                .collect();
            let tex = |c: &str| match c {
                "" => ".".to_string(),
                "{" => "\\{".into(),
                "}" => "\\}".into(),
                "\u{27e8}" => "\\langle".into(),
                "\u{27e9}" => "\\rangle".into(),
                other => other.to_string(),
            };
            format!("\\left{} {} \\right{}", tex(beg), parts.join(sep), tex(end))
        }
        "func" => {
            let name = arg(e, "fName");
            let known = [
                "sin", "cos", "tan", "log", "ln", "exp", "lim", "max", "min", "det", "sec", "csc",
                "cot",
            ];
            let n = name.trim().to_string();
            let f = if known.contains(&n.as_str()) {
                format!("\\{n}")
            } else {
                name
            };
            format!("{f} {}", arg(e, "e"))
        }
        "acc" => {
            let chr = prop(e, "accPr", "chr").unwrap_or("\u{302}");
            let cmd = match chr {
                "\u{303}" | "~" => "\\tilde",
                "\u{304}" | "\u{af}" => "\\bar",
                "\u{307}" => "\\dot",
                "\u{308}" => "\\ddot",
                "\u{20d7}" | "\u{2192}" => "\\vec",
                _ => "\\hat",
            };
            format!("{cmd}{}", braced(arg(e, "e")))
        }
        "bar" => {
            if prop(e, "barPr", "pos") == Some("bot") {
                format!("\\underline{}", braced(arg(e, "e")))
            } else {
                format!("\\overline{}", braced(arg(e, "e")))
            }
        }
        "limLow" => format!("\\underset{}{}", braced(arg(e, "lim")), braced(arg(e, "e"))),
        "limUpp" => format!("\\overset{}{}", braced(arg(e, "lim")), braced(arg(e, "e"))),
        "m" => {
            let rows: Vec<String> = e
                .elems()
                .filter(|c| local(&c.name) == "mr")
                .map(|r| {
                    r.elems()
                        .filter(|c| local(&c.name) == "e")
                        .map(content)
                        .collect::<Vec<_>>()
                        .join(" & ")
                })
                .collect();
            format!("\\begin{{matrix}} {} \\end{{matrix}}", rows.join(" \\\\ "))
        }
        "eqArr" => {
            let rows: Vec<String> = e
                .elems()
                .filter(|c| local(&c.name) == "e")
                .map(content)
                .collect();
            format!(
                "\\begin{{aligned}} {} \\end{{aligned}}",
                rows.join(" \\\\ ")
            )
        }
        "groupChr" | "box" | "borderBox" | "phant" => arg(e, "e"),
        "oMath" | "e" | "num" | "den" | "sup" | "sub" | "lim" | "fName" | "deg" => content(e),
        "rPr" | "ctrlPr" | "fPr" | "dPr" | "naryPr" | "accPr" | "barPr" | "radPr" | "sSupPr"
        | "sSubPr" | "funcPr" | "mPr" | "eqArrPr" | "limLowPr" | "limUppPr" => String::new(),
        _ => content(e),
    }
}

/// TeX for an `oMath` or `oMathPara` element.
pub(crate) fn to_tex(e: &XElem) -> String {
    let body = if local(&e.name) == "oMathPara" {
        e.elems()
            .filter(|c| local(&c.name) == "oMath")
            .map(content)
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        content(e)
    };
    // One space between tokens is enough for TeX; keep the text readable.
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}
