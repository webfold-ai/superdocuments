//! A forgiving HTML parser that builds a tree. It is used by the document reader (`reader`),
//! which needs the structure of a whole document; the single-pass extractor in `extract` does
//! not build a tree. It handles void elements, raw-text elements, comments, character
//! references and the optional end tags of `p`, `li`, `dt`, `dd`, `tr`, `td` and `th`.

use super::entities::lookup_named_entity;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Text(String),
    Elem(Elem),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Elem {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Elem {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_whitespace().any(|x| x == class))
    }

    /// All text below this element, with `<br>` as a newline.
    pub fn text(&self) -> String {
        let mut s = String::new();
        collect_text(&self.children, &mut s);
        s
    }

    /// The element written back as markup (used to hand MathML to the MathML reader).
    pub fn to_markup(&self) -> String {
        let mut s = String::new();
        write_elem(self, &mut s);
        s
    }
}

pub(crate) fn collect_text(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Elem(e) if e.tag == "br" => out.push('\n'),
            Node::Elem(e) if matches!(e.tag.as_str(), "script" | "style") => {}
            Node::Elem(e) => collect_text(&e.children, out),
        }
    }
}

fn write_elem(e: &Elem, out: &mut String) {
    out.push('<');
    out.push_str(&e.tag);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        out.push_str(&v.replace('&', "&amp;").replace('"', "&quot;"));
        out.push('"');
    }
    out.push('>');
    for c in &e.children {
        match c {
            Node::Text(t) => out.push_str(&t.replace('&', "&amp;").replace('<', "&lt;")),
            Node::Elem(c) => write_elem(c, out),
        }
    }
    out.push_str("</");
    out.push_str(&e.tag);
    out.push('>');
}

pub(crate) fn is_void(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
            | "command"
            | "keygen"
    )
}

fn is_blocklike(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "details"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
            | "li"
            | "dd"
            | "dt"
            | "center"
    )
}

/// Elements that stop the search for an open element to close implicitly.
fn is_scope_boundary(tag: &str) -> bool {
    matches!(
        tag,
        "table"
            | "td"
            | "th"
            | "caption"
            | "div"
            | "blockquote"
            | "section"
            | "article"
            | "body"
            | "html"
            | "ul"
            | "ol"
            | "dl"
            | "figure"
            | "aside"
            | "nav"
            | "header"
            | "footer"
            | "main"
            | "li"
            | "dd"
            | "dt"
    )
}

pub(crate) fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.find(';').filter(|&e| e <= 32);
        let Some(e) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..e];
        let replacement: Option<String> = if let Some(num) = name.strip_prefix('#') {
            let code = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(h, 16).ok()
            } else {
                num.parse::<u32>().ok()
            };
            code.and_then(char::from_u32).map(|c| c.to_string())
        } else {
            lookup_named_entity(name).map(str::to_string)
        };
        match replacement {
            Some(r) => {
                out.push_str(&r);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    /// Open elements; index 0 is a synthetic root.
    stack: Vec<Elem>,
}

/// Parses markup into a list of top-level nodes.
pub(crate) fn parse(src: &str) -> Vec<Node> {
    let mut p = Parser {
        src,
        pos: 0,
        stack: vec![Elem::default()],
    };
    p.run();
    while p.stack.len() > 1 {
        p.pop();
    }
    p.stack.pop().map(|r| r.children).unwrap_or_default()
}

impl Parser<'_> {
    fn pop(&mut self) {
        if self.stack.len() > 1 {
            if let Some(e) = self.stack.pop() {
                if let Some(top) = self.stack.last_mut() {
                    top.children.push(Node::Elem(e));
                }
            }
        }
    }

    fn push_text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        let top = self.stack.last_mut().expect("root");
        if let Some(Node::Text(last)) = top.children.last_mut() {
            last.push_str(t);
        } else {
            top.children.push(Node::Text(t.to_string()));
        }
    }

    fn run(&mut self) {
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            let Some(lt) = self.src[self.pos..].find('<').map(|i| i + self.pos) else {
                let t = decode_entities(&self.src[self.pos..]);
                self.push_text(&t);
                break;
            };
            if lt > self.pos {
                let t = decode_entities(&self.src[self.pos..lt]);
                self.push_text(&t);
            }
            self.pos = lt;
            let rest = &self.src[lt..];
            if rest.starts_with("<!--") {
                self.pos = rest.find("-->").map_or(self.src.len(), |e| lt + e + 3);
            } else if rest.starts_with("<![CDATA[") {
                let end = rest.find("]]>").map_or(self.src.len(), |e| lt + e);
                let t = self.src[lt + 9..end].to_string();
                self.push_text(&t);
                self.pos = (end + 3).min(self.src.len());
            } else if rest.starts_with("<!") || rest.starts_with("<?") {
                self.pos = rest.find('>').map_or(self.src.len(), |e| lt + e + 1);
            } else if rest.starts_with("</") {
                self.end_tag();
            } else if rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                self.start_tag();
            } else {
                self.push_text("<");
                self.pos += 1;
            }
        }
    }

    fn end_tag(&mut self) {
        let rest = &self.src[self.pos + 2..];
        let end = rest.find('>').unwrap_or(rest.len());
        let name = rest[..end]
            .trim()
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        self.pos += 2 + end + 1;
        self.pos = self.pos.min(self.src.len());
        if let Some(idx) = self.stack.iter().rposition(|e| e.tag == name) {
            if idx > 0 {
                while self.stack.len() > idx {
                    self.pop();
                }
            }
        }
    }

    fn start_tag(&mut self) {
        let bytes = self.src.as_bytes();
        let mut i = self.pos + 1;
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let tag = self.src[name_start..i].to_ascii_lowercase();
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut self_close = false;
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'>' => {
                    i += 1;
                    break;
                }
                b'/' => {
                    self_close = true;
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let ns = i;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !matches!(bytes[i], b'=' | b'>' | b'/')
            {
                i += 1;
            }
            let name = self.src[ns..i].to_ascii_lowercase();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut value = String::new();
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    let q = bytes[i];
                    i += 1;
                    let vs = i;
                    while i < bytes.len() && bytes[i] != q {
                        i += 1;
                    }
                    value = decode_entities(&self.src[vs..i]);
                    i = (i + 1).min(bytes.len());
                } else {
                    let vs = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&self.src[vs..i]);
                }
            }
            if !name.is_empty() && !attrs.iter().any(|(k, _)| *k == name) {
                attrs.push((name, value));
            }
        }
        self.pos = i;
        self.implied_ends(&tag);
        let elem = Elem {
            tag: tag.clone(),
            attrs,
            children: Vec::new(),
        };
        if is_void(&tag) || self_close && !matches!(tag.as_str(), "script" | "style") {
            if let Some(top) = self.stack.last_mut() {
                top.children.push(Node::Elem(elem));
            }
            return;
        }
        if matches!(tag.as_str(), "script" | "style" | "textarea" | "title") {
            let close = format!("</{tag}");
            let lower = self.src[self.pos..].to_ascii_lowercase();
            let end = lower.find(&close).map_or(self.src.len(), |e| self.pos + e);
            let raw = &self.src[self.pos..end];
            let mut elem = elem;
            if tag == "title" || tag == "textarea" {
                elem.children.push(Node::Text(decode_entities(raw)));
            } else {
                elem.children.push(Node::Text(raw.to_string()));
            }
            if let Some(top) = self.stack.last_mut() {
                top.children.push(Node::Elem(elem));
            }
            self.pos = self.src[end..]
                .find('>')
                .map_or(self.src.len(), |e| end + e + 1);
            return;
        }
        self.stack.push(elem);
    }

    /// Closes the elements whose end tag the markup left out because `tag` begins here.
    fn implied_ends(&mut self, tag: &str) {
        let find = |stack: &[Elem], names: &[&str], stop: &dyn Fn(&str) -> bool| -> Option<usize> {
            for (i, e) in stack.iter().enumerate().skip(1).rev() {
                if names.contains(&e.tag.as_str()) {
                    return Some(i);
                }
                if stop(&e.tag) {
                    return None;
                }
            }
            None
        };
        let target = match tag {
            "li" => find(&self.stack, &["li"], &|t| matches!(t, "ul" | "ol" | "menu")),
            "dt" | "dd" => find(&self.stack, &["dt", "dd"], &|t| t == "dl"),
            "tr" => find(&self.stack, &["tr"], &|t| t == "table"),
            "td" | "th" => find(&self.stack, &["td", "th"], &|t| matches!(t, "tr" | "table")),
            "thead" | "tbody" | "tfoot" => find(
                &self.stack,
                &["thead", "tbody", "tfoot", "tr", "td", "th"],
                &|t| t == "table",
            ),
            "option" => find(&self.stack, &["option"], &|t| t == "select"),
            t if is_blocklike(t) => find(&self.stack, &["p"], &|t| is_scope_boundary(t)),
            _ => None,
        };
        if let Some(idx) = target {
            while self.stack.len() > idx {
                self.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match n {
                Node::Elem(e) => Some(e.tag.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn optional_end_tags_close_siblings() {
        let nodes = parse("<ul><li>a<li>b</ul><p>x<p>y");
        assert_eq!(tags(&nodes), ["ul", "p", "p"]);
        let Node::Elem(ul) = &nodes[0] else { panic!() };
        assert_eq!(tags(&ul.children), ["li", "li"]);
    }

    #[test]
    fn decodes_entities_and_skips_comments() {
        let nodes =
            parse("<p>a &amp; b &eacute; &#8364;<!-- no --></p><script>if (a<b) {}</script>");
        let Node::Elem(p) = &nodes[0] else { panic!() };
        assert_eq!(p.text(), "a & b é €");
        let Node::Elem(s) = &nodes[1] else { panic!() };
        assert_eq!(s.text(), "if (a<b) {}");
    }

    #[test]
    fn table_cells_without_end_tags() {
        let nodes = parse("<table><tr><td>1<td>2<tr><td>3<td>4</table>");
        let Node::Elem(t) = &nodes[0] else { panic!() };
        assert_eq!(tags(&t.children), ["tr", "tr"]);
    }
}
