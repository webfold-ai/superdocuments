//! Reading a CSL style (1.0.x): the macros, the locale overrides and the option attributes the
//! processor inherits from `style`, `citation` and `bibliography`.

use std::collections::HashMap;

use super::terms;
use crate::xmltree::{self, XElem};

#[derive(Debug, Clone)]
pub struct Style {
    pub(crate) root: XElem,
    pub(crate) macros: HashMap<String, XElem>,
    /// (name, form) -> (singular, plural)
    pub(crate) terms: HashMap<(String, String), (String, String)>,
    /// `<date form="text|numeric">` overrides from the style's own locale blocks.
    pub(crate) date_formats: HashMap<String, XElem>,
    pub title: String,
    pub class_in_text: bool,
    pub(crate) calls_year_suffix: bool,
}

impl Style {
    /// Reads a style from its XML. Fails when the XML is not a CSL `style`.
    pub fn parse(xml: &str) -> Result<Style, String> {
        let root = xmltree::parse(xml.as_bytes())?;
        if root.name != "style" {
            return Err("not a CSL style (root element is not <style>)".into());
        }
        let mut macros = HashMap::new();
        for m in root.elems().filter(|e| e.name == "macro") {
            if let Some(n) = m.attr("name") {
                macros.insert(n.to_string(), m.clone());
            }
        }
        let mut t: HashMap<(String, String), (String, String)> = HashMap::new();
        for line in terms::EN_US.lines() {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() >= 4 {
                let plural = if p[3].is_empty() { p[2] } else { p[3] };
                t.insert((p[0].to_string(), p[1].to_string()), (p[2].to_string(), plural.to_string()));
            }
        }
        let mut date_formats = HashMap::new();
        // Locale blocks without an `xml:lang`, or in English, apply; others are ignored.
        for loc in root.elems().filter(|e| e.name == "locale") {
            let lang = loc.attr("xml:lang").unwrap_or("");
            if !(lang.is_empty() || lang == "en" || lang == "en-US") {
                continue;
            }
            for sec in loc.elems() {
                if sec.name == "terms" {
                    for term in sec.elems().filter(|e| e.name == "term") {
                        let name = term.attr("name").unwrap_or("").to_string();
                        let form = term.attr("form").unwrap_or("long").to_string();
                        let form = if form == "long" { String::new() } else { form };
                        let single = term.find("single").map(text_of);
                        let multi = term.find("multiple").map(text_of);
                        let plain = text_of(term);
                        let (s, m) = match (single, multi) {
                            (Some(s), Some(m)) => (s, m),
                            (Some(s), None) => (s.clone(), s),
                            _ => (plain.clone(), plain),
                        };
                        t.insert((name, form), (s, m));
                    }
                } else if sec.name == "date" {
                    if let Some(f) = sec.attr("form") {
                        date_formats.insert(f.to_string(), sec.clone());
                    }
                }
            }
        }
        let title = root.find("info").and_then(|i| i.find("title")).map(text_of).unwrap_or_default();
        let class_in_text = root.attr("class") != Some("note");
        let calls_year_suffix = mentions_variable(&root, "year-suffix");
        Ok(Style { root, macros, terms: t, date_formats, title, class_in_text, calls_year_suffix })
    }

    pub(crate) fn citation(&self) -> Option<&XElem> {
        self.root.find("citation")
    }

    pub(crate) fn bibliography(&self) -> Option<&XElem> {
        self.root.find("bibliography")
    }

    /// A term, in the requested form with the usual fall-back (`verb-short` -> `verb` ->
    /// long, `symbol` -> `short` -> long).
    pub(crate) fn term(&self, name: &str, form: &str, plural: bool) -> Option<String> {
        let chain: &[&str] = match form {
            "verb-short" => &["verb-short", "verb", ""],
            "verb" => &["verb", ""],
            "symbol" => &["symbol", "short", ""],
            "short" => &["short", ""],
            _ => &[""],
        };
        for f in chain {
            if let Some((s, p)) = self.terms.get(&(name.to_string(), (*f).to_string())) {
                return Some(if plural { p.clone() } else { s.clone() });
            }
        }
        None
    }

    /// An option attribute looked up on the element, then the given parents, then `style`.
    pub(crate) fn inherited<'a>(&'a self, attr: &str, chain: &[&'a XElem]) -> Option<&'a str> {
        for e in chain {
            if let Some(v) = e.attr(attr) {
                return Some(v);
            }
        }
        self.root.attr(attr)
    }
}

pub(crate) fn text_of(e: &XElem) -> String {
    let mut s = String::new();
    for c in &e.children {
        match c {
            xmltree::XNode::Text(t) => s.push_str(t),
            xmltree::XNode::Elem(x) => s.push_str(&text_of(x)),
        }
    }
    s.trim().to_string()
}

fn mentions_variable(e: &XElem, var: &str) -> bool {
    if e.attr("variable").is_some_and(|v| v.split_whitespace().any(|x| x == var)) {
        return true;
    }
    e.elems().any(|c| mentions_variable(c, var))
}
