//! A small XML tree built from quick-xml events, for the readers whose formats nest deeply
//! (OpenDocument text). Names are kept as written, prefix included (`text:p`, `table:table`).
//! Character data keeps its whitespace; entities and character references are resolved.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum XNode {
    Text(String),
    Elem(XElem),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct XElem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XNode>,
}

impl XElem {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn elems(&self) -> impl Iterator<Item = &XElem> {
        self.children.iter().filter_map(|c| match c {
            XNode::Elem(e) => Some(e),
            XNode::Text(_) => None,
        })
    }

    pub fn find(&self, name: &str) -> Option<&XElem> {
        self.elems().find(|e| e.name == name)
    }

    /// The first descendant with this name, depth first.
    pub fn descendant(&self, name: &str) -> Option<&XElem> {
        for e in self.elems() {
            if e.name == name {
                return Some(e);
            }
            if let Some(d) = e.descendant(name) {
                return Some(d);
            }
        }
        None
    }

    /// Markup for this element, as written (used to hand a nested `math` element to the
    /// MathML reader).
    pub fn to_markup(&self) -> String {
        let mut s = String::new();
        write(self, &mut s);
        s
    }
}

fn write(e: &XElem, out: &mut String) {
    out.push('<');
    out.push_str(&e.name);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        out.push_str(&v.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;"));
        out.push('"');
    }
    out.push('>');
    for c in &e.children {
        match c {
            XNode::Text(t) => out.push_str(&t.replace('&', "&amp;").replace('<', "&lt;")),
            XNode::Elem(c) => write(c, out),
        }
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push('>');
}

/// Parses a document into its root element. Deeper than 512 levels is refused.
pub(crate) fn parse(xml: &[u8]) -> Result<XElem, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut stack: Vec<XElem> = vec![XElem::default()];
    loop {
        match reader.read_event_into(&mut buf).map_err(|e| e.to_string())? {
            Event::Start(t) => {
                if stack.len() > 512 {
                    return Err("XML nested too deeply".into());
                }
                stack.push(elem(&t));
            }
            Event::Empty(t) => {
                let e = elem(&t);
                if let Some(top) = stack.last_mut() {
                    top.children.push(XNode::Elem(e));
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let e = stack.pop().unwrap_or_default();
                    if let Some(top) = stack.last_mut() {
                        top.children.push(XNode::Elem(e));
                    }
                }
            }
            Event::Text(t) => {
                let s = crate::document::text(t.as_ref());
                push_text(&mut stack, &s);
            }
            Event::CData(c) => {
                let s = c.as_ref().to_string();
                push_text(&mut stack, &s);
            }
            Event::GeneralRef(r) => {
                let mut s = String::new();
                crate::document::push_ref(&mut s, &r);
                push_text(&mut stack, &s);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    while stack.len() > 1 {
        let e = stack.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.children.push(XNode::Elem(e));
        }
    }
    let root = stack.pop().unwrap_or_default();
    root.children
        .into_iter()
        .find_map(|c| match c {
            XNode::Elem(e) => Some(e),
            XNode::Text(_) => None,
        })
        .ok_or_else(|| "no root element".to_string())
}

fn push_text(stack: &mut [XElem], s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(top) = stack.last_mut() {
        if let Some(XNode::Text(last)) = top.children.last_mut() {
            last.push_str(s);
        } else {
            top.children.push(XNode::Text(s.to_string()));
        }
    }
}

fn elem(t: &quick_xml::events::BytesStart<'_>) -> XElem {
    XElem {
        name: t.name().as_ref().to_string(),
        attrs: t
            .attributes()
            .flatten()
            .map(|a| {
                (
                    a.key.as_ref().to_string(),
                    crate::document::text(a.value.as_ref()),
                )
            })
            .collect(),
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_entities() {
        let root = parse(b"<a x=\"1\"><b>t &amp; u</b><c/></a>").unwrap();
        assert_eq!(root.attr("x"), Some("1"));
        let b = root.find("b").unwrap();
        assert_eq!(b.children, vec![XNode::Text("t & u".into())]);
        assert!(root.find("c").is_some());
        assert_eq!(root.to_markup(), "<a x=\"1\"><b>t &amp; u</b><c></c></a>");
    }
}
