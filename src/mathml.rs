//! MathML to TeX.
//!
//! `to_tex` reads a `<math>` element (namespace prefixes are ignored) and returns TeX. When the
//! element holds `<semantics>` with an `<annotation encoding="application/x-tex">` (what
//! LaTeXML and MathJax write), that TeX is returned unchanged, because it is the author's
//! source. Otherwise the presentation markup is converted:
//!
//! - `mi`, `mn`, `mo`, `mtext`, `mspace`: identifiers, numbers, operators, text and spaces.
//!   Greek letters and common symbols map to their TeX commands. Multi-letter identifiers
//!   such as `sin` become `\sin`; other words become `\mathrm{...}`.
//! - `mrow`, `mstyle`, `mpadded`, `mphantom`, `semantics` (first child only), `menclose`.
//! - `msub`, `msup`, `msubsup`, `mover`, `munder`, `munderover`: `_`, `^`, `\overset`,
//!   `\underset`. Large operators (`∑`, `∫`, ...) take limits as `_` and `^`.
//! - `mfrac` (`\frac`, or `\binom` when `linethickness="0"`), `msqrt`, `mroot`.
//! - `mtable`, `mtr`, `mtd`: `array` with one `c` column per cell.
//! - `mfenced`: `\left` and `\right` with the `open`, `close` and `separators` attributes.
//!
//! Anything else keeps its text and adds a warning. Unmapped characters are kept as Unicode,
//! which MathJax and most TeX-to-HTML converters accept. Depth and node limits stop runaway
//! input. The result is never a panic; malformed XML gives a warning and whatever could be read.

use std::fmt::Write as _;

/// Bounds and choices for conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Use the TeX annotation when present. Default true.
    pub prefer_annotation: bool,
    pub max_depth: usize,
    pub max_nodes: usize,
    /// Largest input, in bytes.
    pub max_bytes: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            prefer_annotation: true,
            max_depth: 64,
            max_nodes: 100_000,
            max_bytes: 4 << 20,
        }
    }
}

/// The result of a conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub tex: String,
    /// True when the TeX came from the annotation rather than from the presentation markup.
    pub from_annotation: bool,
    /// Elements, characters and structures that were not converted exactly.
    pub warnings: Vec<String>,
}

/// Converts MathML with default options.
pub fn to_tex(mathml: &str) -> Converted {
    to_tex_with(mathml, &Options::default())
}

/// Converts MathML with explicit options.
pub fn to_tex_with(mathml: &str, options: &Options) -> Converted {
    let mut out = Converted {
        tex: String::new(),
        from_annotation: false,
        warnings: Vec::new(),
    };
    if mathml.len() > options.max_bytes {
        out.warnings.push(format!(
            "MathML is {} bytes, over the {} byte limit",
            mathml.len(),
            options.max_bytes
        ));
        return out;
    }
    let root = match parse_xml(mathml, options.max_depth, options.max_nodes) {
        Ok(root) => root,
        Err(e) => {
            out.warnings.push(format!("malformed MathML: {e}"));
            return out;
        }
    };
    let math = find_math(&root).unwrap_or(&root);
    if options.prefer_annotation {
        if let Some(tex) = tex_annotation(math) {
            out.tex = tex;
            out.from_annotation = true;
            return out;
        }
    }
    let mut conv = Converter {
        warnings: Vec::new(),
        depth: 0,
        max_depth: options.max_depth,
    };
    out.tex = conv.node(math).trim().to_string();
    out.warnings = conv.warnings;
    out
}

// ----- XML ------------------------------------------------------------------------------------

/// An open element while parsing: name, attributes, children.
type Frame = (String, Vec<(String, String)>, Vec<Node>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
    Text(String),
}

impl Node {
    fn name(&self) -> Option<&str> {
        match self {
            Node::Element { name, .. } => Some(name),
            Node::Text(_) => None,
        }
    }

    fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Node::Element { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str()),
            Node::Text(_) => None,
        }
    }

    fn children(&self) -> &[Node] {
        match self {
            Node::Element { children, .. } => children,
            Node::Text(_) => &[],
        }
    }

    /// The text content, descendants included.
    fn text(&self) -> String {
        match self {
            Node::Text(t) => t.clone(),
            Node::Element { children, .. } => children.iter().map(Node::text).collect(),
        }
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Finds the first `math` element in document order.
fn find_math(node: &Node) -> Option<&Node> {
    if node.name().is_some_and(|n| local(n) == "math") {
        return Some(node);
    }
    node.children().iter().find_map(find_math)
}

/// The TeX annotation of a `semantics` element, if there is one.
fn tex_annotation(math: &Node) -> Option<String> {
    for child in math.children() {
        if child.name().is_some_and(|n| local(n) == "semantics") {
            for ann in child.children() {
                if ann.name().is_some_and(|n| local(n) == "annotation") {
                    let enc = ann.attr("encoding").unwrap_or("");
                    if enc.contains("tex") {
                        let text = ann.text().trim().to_string();
                        if !text.is_empty() {
                            return Some(text);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parses the markup into a tree. Declarations, processing instructions, comments and
/// doctypes are skipped. Entities are decoded. Mismatched end tags close the open elements up
/// to the matching one, and are ignored when no element matches.
fn parse_xml(src: &str, max_depth: usize, max_nodes: usize) -> Result<Node, String> {
    let bytes = src.as_bytes();
    let mut stack: Vec<Frame> = vec![(String::new(), Vec::new(), Vec::new())];
    let mut i = 0usize;
    let mut nodes = 0usize;
    let mut text_start = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        flush_text(&src[text_start..i], &mut stack);
        if src[i..].starts_with("<!--") {
            i = src[i..]
                .find("-->")
                .map(|e| i + e + 3)
                .ok_or("unterminated comment")?;
        } else if src[i..].starts_with("<![CDATA[") {
            let start = i + 9;
            let end = src[start..]
                .find("]]>")
                .map(|e| start + e)
                .ok_or("unterminated CDATA")?;
            push_text(&mut stack, &src[start..end]);
            i = end + 3;
        } else if src[i..].starts_with("<!") || src[i..].starts_with("<?") {
            i = src[i..]
                .find('>')
                .map(|e| i + e + 1)
                .ok_or("unterminated declaration")?;
        } else if src[i..].starts_with("</") {
            let end = src[i..]
                .find('>')
                .map(|e| i + e)
                .ok_or("unterminated end tag")?;
            let name = src[i + 2..end].trim().to_string();
            close_to(&mut stack, &name)?;
            i = end + 1;
        } else {
            let end = find_tag_end(src, i).ok_or("unterminated tag")?;
            let inner = &src[i + 1..end];
            let self_closing = inner.trim_end().ends_with('/');
            let inner = if self_closing {
                inner.trim_end().trim_end_matches('/')
            } else {
                inner
            };
            let (name, attrs) = parse_tag(inner)?;
            nodes += 1;
            if nodes > max_nodes {
                return Err("node limit reached".into());
            }
            if self_closing {
                let node = Node::Element {
                    name,
                    attrs,
                    children: Vec::new(),
                };
                if let Some(top) = stack.last_mut() {
                    top.2.push(node);
                }
            } else {
                if stack.len() > max_depth {
                    return Err("depth limit reached".into());
                }
                stack.push((name, attrs, Vec::new()));
            }
            i = end + 1;
        }
        text_start = i;
    }
    flush_text(&src[text_start..], &mut stack);
    while stack.len() > 1 {
        let (name, attrs, children) = stack.pop().ok_or("stack")?;
        if let Some(top) = stack.last_mut() {
            top.2.push(Node::Element {
                name,
                attrs,
                children,
            });
        }
    }
    let (_, _, children) = stack.pop().ok_or("stack")?;
    children
        .into_iter()
        .find(|n| matches!(n, Node::Element { .. }))
        .ok_or_else(|| "no element".to_string())
}

fn flush_text(raw: &str, stack: &mut [Frame]) {
    if !raw.is_empty() {
        push_text(stack, raw);
    }
}

fn push_text(stack: &mut [Frame], raw: &str) {
    let decoded = decode_entities(raw);
    if decoded.trim().is_empty() {
        return;
    }
    if let Some(top) = stack.last_mut() {
        top.2.push(Node::Text(decoded));
    }
}

/// The index of the `>` that ends the tag starting at `start`, skipping quoted attributes.
fn find_tag_end(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut quote: Option<u8> = None;
    for (k, &b) in bytes.iter().enumerate().skip(start) {
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some(k),
                _ => {}
            },
        }
    }
    None
}

fn parse_tag(inner: &str) -> Result<(String, Vec<(String, String)>), String> {
    let inner = inner.trim_start();
    let name_end = inner
        .find(|c: char| c.is_whitespace())
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_string();
    if name.is_empty() {
        return Err("empty tag name".into());
    }
    let mut attrs = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let eq = match rest.find('=') {
            Some(e) => e,
            None => break,
        };
        let key = rest[..eq].trim().to_string();
        rest = rest[eq + 1..].trim_start();
        let quote = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => q,
            _ => break,
        };
        let close = match rest[1..].find(quote) {
            Some(c) => c + 1,
            None => break,
        };
        let value = decode_entities(&rest[1..close]);
        if !key.is_empty() {
            attrs.push((key, value));
        }
        rest = rest[close + 1..].trim_start();
    }
    Ok((name, attrs))
}

fn close_to(stack: &mut Vec<Frame>, name: &str) -> Result<(), String> {
    let Some(pos) = stack
        .iter()
        .rposition(|(n, _, _)| !n.is_empty() && n == name)
    else {
        return Ok(());
    };
    while stack.len() > pos {
        let (name, attrs, children) = stack.pop().ok_or("stack")?;
        if let Some(top) = stack.last_mut() {
            top.2.push(Node::Element {
                name,
                attrs,
                children,
            });
        }
    }
    Ok(())
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';') {
            Some(semi) if semi <= 12 => {
                let name = &after[..semi];
                let decoded = match name {
                    "amp" => Some("&".to_string()),
                    "lt" => Some("<".to_string()),
                    "gt" => Some(">".to_string()),
                    "quot" => Some("\"".to_string()),
                    "apos" => Some("'".to_string()),
                    _ => name.strip_prefix('#').and_then(|num| {
                        let code = match num.strip_prefix(['x', 'X']) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok(),
                            None => num.parse::<u32>().ok(),
                        }?;
                        char::from_u32(code).map(|c| c.to_string())
                    }),
                };
                match decoded {
                    Some(d) => {
                        out.push_str(&d);
                        rest = &after[semi + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = after;
                    }
                }
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ----- conversion -----------------------------------------------------------------------------

struct Converter {
    warnings: Vec<String>,
    depth: usize,
    max_depth: usize,
}

const BIG_OPERATORS: &[&str] = &[
    "\\sum",
    "\\prod",
    "\\int",
    "\\oint",
    "\\bigcup",
    "\\bigcap",
    "\\bigvee",
    "\\bigwedge",
    "\\coprod",
    "\\lim",
    "\\max",
    "\\min",
    "\\sup",
    "\\inf",
    "\\bigoplus",
    "\\bigotimes",
];

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "log", "ln", "exp", "lim", "max", "min", "sup", "inf", "det", "dim", "ker", "arg", "deg",
    "gcd", "hom", "Pr",
];

fn greek(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' => "\\varepsilon",
        'ϵ' => "\\epsilon",
        'ζ' => "\\zeta",
        'η' => "\\eta",
        'θ' => "\\theta",
        'ι' => "\\iota",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' => "\\mu",
        'ν' => "\\nu",
        'ξ' => "\\xi",
        'π' => "\\pi",
        'ρ' => "\\rho",
        'σ' => "\\sigma",
        'τ' => "\\tau",
        'υ' => "\\upsilon",
        'φ' => "\\varphi",
        'ϕ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'Γ' => "\\Gamma",
        'Δ' => "\\Delta",
        'Θ' => "\\Theta",
        'Λ' => "\\Lambda",
        'Ξ' => "\\Xi",
        'Π' => "\\Pi",
        'Σ' => "\\Sigma",
        'Υ' => "\\Upsilon",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Ω' => "\\Omega",
        _ => return None,
    })
}

/// TeX for a Unicode operator or symbol. `None` when there is no mapping.
fn symbol(c: char) -> Option<&'static str> {
    Some(match c {
        '∞' => "\\infty",
        '∑' => "\\sum",
        '∏' => "\\prod",
        '∫' => "\\int",
        '∮' => "\\oint",
        '∂' => "\\partial",
        '∇' => "\\nabla",
        '≤' | '⩽' => "\\le",
        '≥' | '⩾' => "\\ge",
        '≠' => "\\neq",
        '≈' => "\\approx",
        '≡' => "\\equiv",
        '∼' => "\\sim",
        '≃' => "\\simeq",
        '∝' => "\\propto",
        '±' => "\\pm",
        '∓' => "\\mp",
        '×' => "\\times",
        '÷' => "\\div",
        '·' | '⋅' => "\\cdot",
        '∘' => "\\circ",
        '∈' => "\\in",
        '∉' => "\\notin",
        '⊂' => "\\subset",
        '⊆' => "\\subseteq",
        '∪' => "\\cup",
        '∩' => "\\cap",
        '∧' => "\\land",
        '∨' => "\\lor",
        '¬' => "\\neg",
        '∀' => "\\forall",
        '∃' => "\\exists",
        '→' => "\\to",
        '←' => "\\leftarrow",
        '↔' => "\\leftrightarrow",
        '⇒' => "\\Rightarrow",
        '⇔' => "\\Leftrightarrow",
        '↦' => "\\mapsto",
        '↑' => "\\uparrow",
        '↓' => "\\downarrow",
        '⟨' => "\\langle",
        '⟩' => "\\rangle",
        '⌊' => "\\lfloor",
        '⌋' => "\\rfloor",
        '⌈' => "\\lceil",
        '⌉' => "\\rceil",
        '‖' | '∥' => "\\|",
        '∅' => "\\emptyset",
        'ℝ' => "\\mathbb{R}",
        'ℕ' => "\\mathbb{N}",
        'ℤ' => "\\mathbb{Z}",
        'ℂ' => "\\mathbb{C}",
        'ℚ' => "\\mathbb{Q}",
        '⊗' => "\\otimes",
        '⊕' => "\\oplus",
        '∖' => "\\setminus",
        '≪' => "\\ll",
        '≫' => "\\gg",
        '⊥' => "\\perp",
        '∣' => "\\mid",
        '…' => "\\ldots",
        '⋯' => "\\cdots",
        '′' => "'",
        '−' => "-",
        '⁢' | '⁡' | '⁣' | '⁤' => "",
        '⋁' => "\\bigvee",
        '⋀' => "\\bigwedge",
        '⋂' => "\\bigcap",
        '⋃' => "\\bigcup",
        '∗' => "*",
        _ => return None,
    })
}

fn is_big_operator(tex: &str) -> bool {
    BIG_OPERATORS.contains(&tex)
}

/// Wraps a TeX fragment as one argument: as is when it is a single token, else in braces.
fn arg(tex: &str) -> String {
    let t = tex.trim();
    let single_char = t.chars().count() == 1;
    let single_command =
        t.starts_with('\\') && t.len() > 1 && t[1..].chars().all(|c| c.is_ascii_alphabetic());
    if single_char || single_command {
        t.to_string()
    } else if t.is_empty() {
        "{}".to_string()
    } else {
        format!("{{{t}}}")
    }
}

impl Converter {
    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn node(&mut self, node: &Node) -> String {
        match node {
            Node::Text(t) => {
                self.warn(format!("text outside a token element: {:?}", t.trim()));
                escape_text_token(t.trim())
            }
            Node::Element { .. } => {
                if self.depth > self.max_depth {
                    self.warn("depth limit reached; the rest is dropped".into());
                    return String::new();
                }
                self.depth += 1;
                let out = self.element(node);
                self.depth -= 1;
                out
            }
        }
    }

    fn children_tex(&mut self, node: &Node) -> String {
        let mut out = String::new();
        for c in node.children() {
            let piece = self.node(c);
            if !piece.is_empty() {
                if !out.is_empty() && !out.ends_with(' ') && !piece.starts_with(' ') {
                    out.push(' ');
                }
                out.push_str(&piece);
            }
        }
        out
    }

    /// The TeX of the children that are element children, in order.
    fn elements(node: &Node) -> Vec<&Node> {
        node.children()
            .iter()
            .filter(|c| matches!(c, Node::Element { .. }))
            .collect()
    }

    fn element(&mut self, node: &Node) -> String {
        let name = node.name().unwrap_or("").to_string();
        let name = local(&name).to_string();
        match name.as_str() {
            "math" | "mrow" | "mstyle" | "mpadded" | "mphantom" | "merror" | "maction" => {
                if name == "mphantom" {
                    return format!("\\phantom{{{}}}", self.children_tex(node));
                }
                self.children_tex(node)
            }
            "semantics" => {
                let first = Self::elements(node).into_iter().next();
                match first {
                    Some(n) => self.node(n),
                    None => String::new(),
                }
            }
            "annotation" | "annotation-xml" => String::new(),
            "mi" => self.identifier(node),
            "mn" => node.text().trim().to_string(),
            "mo" => self.operator(node),
            "mtext" => {
                let text = node.text();
                if text.trim().is_empty() {
                    String::new()
                } else {
                    format!("\\text{{{}}}", escape_braces(text.trim()))
                }
            }
            "mspace" => {
                let width = node.attr("width").unwrap_or("");
                match width {
                    "" => "\\,".to_string(),
                    w if w.starts_with('0') => String::new(),
                    _ => "\\quad".to_string(),
                }
            }
            "msub" | "msup" | "msubsup" | "munder" | "mover" | "munderover" => {
                self.scripts(node, &name)
            }
            "mfrac" => {
                let parts = Self::elements(node);
                if parts.len() < 2 {
                    self.warn("mfrac with fewer than two children".into());
                    return self.children_tex(node);
                }
                let num = self.node(parts[0]);
                let den = self.node(parts[1]);
                let cmd = if node.attr("linethickness") == Some("0") {
                    "\\binom"
                } else {
                    "\\frac"
                };
                format!("{cmd}{{{num}}}{{{den}}}")
            }
            "msqrt" => format!("\\sqrt{{{}}}", self.children_tex(node)),
            "mroot" => {
                let parts = Self::elements(node);
                if parts.len() < 2 {
                    self.warn("mroot with fewer than two children".into());
                    return self.children_tex(node);
                }
                let base = self.node(parts[0]);
                let index = self.node(parts[1]);
                format!("\\sqrt[{index}]{{{base}}}")
            }
            "mtable" => self.table(node),
            "mtr" | "mlabeledtr" => {
                let cells: Vec<String> = Self::elements(node)
                    .into_iter()
                    .map(|c| self.node(c))
                    .collect();
                cells.join(" & ")
            }
            "mtd" => self.children_tex(node),
            "mfenced" => self.fenced(node),
            "menclose" => {
                let body = self.children_tex(node);
                if node.attr("notation").is_some_and(|n| n.contains("box")) {
                    format!("\\boxed{{{body}}}")
                } else {
                    body
                }
            }
            other => {
                self.warn(format!(
                    "unsupported MathML element <{other}>; its content is kept"
                ));
                self.children_tex(node)
            }
        }
    }

    fn identifier(&mut self, node: &Node) -> String {
        let text = node.text();
        let t = text.trim();
        let variant = node.attr("mathvariant").unwrap_or("");
        if t.is_empty() {
            return String::new();
        }
        let mut chars = t.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if let Some(g) = greek(c) {
                return wrap_variant(g, variant);
            }
            if let Some(s) = symbol(c) {
                return s.to_string();
            }
            return wrap_variant(&escape_text_token(t), variant);
        }
        if FUNCTIONS.contains(&t) {
            return format!("\\{t}");
        }
        format!("\\mathrm{{{}}}", escape_braces(t))
    }

    fn operator(&mut self, node: &Node) -> String {
        let text = node.text();
        let t = text.trim();
        let mut chars = t.chars();
        match (chars.next(), chars.next()) {
            (None, _) => String::new(),
            (Some(c), None) => match symbol(c) {
                Some(s) => s.to_string(),
                None if c.is_ascii() => escape_op(c),
                None => {
                    self.warn(format!(
                        "unmapped operator U+{:04X} kept as Unicode",
                        c as u32
                    ));
                    c.to_string()
                }
            },
            _ => {
                let mut out = String::new();
                for c in t.chars() {
                    match symbol(c) {
                        Some(s) => out.push_str(s),
                        None => out.push(c),
                    }
                }
                out
            }
        }
    }

    fn scripts(&mut self, node: &Node, kind: &str) -> String {
        let parts = Self::elements(node);
        let need = match kind {
            "msubsup" | "munderover" => 3,
            _ => 2,
        };
        if parts.len() < need {
            self.warn(format!("<{kind}> with too few children"));
            return self.children_tex(node);
        }
        let base_node = parts[0];
        let base = self.node(base_node);
        let (second, third) = (self.node(parts[1]), parts.get(2).map(|p| self.node(p)));
        let base_is_big = is_big_operator(base.trim());
        match kind {
            "msub" => format!("{}_{}", arg(&base), arg(&second)),
            "msup" => format!("{}^{}", arg(&base), arg(&second)),
            "msubsup" => format!(
                "{}_{}^{}",
                arg(&base),
                arg(&second),
                arg(&third.unwrap_or_default())
            ),
            "munder" => {
                if base_is_big {
                    format!("{}_{}", base.trim(), arg(&second))
                } else if parts[1].attr("accentunder") == Some("true") {
                    format!("\\underline{{{base}}}")
                } else {
                    format!("\\underset{{{second}}}{{{base}}}")
                }
            }
            "mover" => {
                if base_is_big {
                    format!("{}^{}", base.trim(), arg(&second))
                } else {
                    match accent_command(&parts[1].text()) {
                        Some(cmd) => format!("{cmd}{{{base}}}"),
                        None => format!("\\overset{{{second}}}{{{base}}}"),
                    }
                }
            }
            _ => {
                let over = third.unwrap_or_default();
                if base_is_big {
                    format!("{}_{}^{}", base.trim(), arg(&second), arg(&over))
                } else {
                    format!("\\underset{{{second}}}{{\\overset{{{over}}}{{{base}}}}}")
                }
            }
        }
    }

    fn table(&mut self, node: &Node) -> String {
        let rows: Vec<Vec<String>> = Self::elements(node)
            .into_iter()
            .map(|row| {
                let cells: Vec<&Node> = if row
                    .name()
                    .is_some_and(|n| local(n) == "mtr" || local(n) == "mlabeledtr")
                {
                    Self::elements(row)
                } else {
                    vec![row]
                };
                cells.into_iter().map(|c| self.node(c)).collect()
            })
            .collect();
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return String::new();
        }
        let body: Vec<String> = rows.iter().map(|r| r.join(" & ")).collect();
        format!(
            "\\begin{{array}}{{{}}} {} \\end{{array}}",
            "c".repeat(cols),
            body.join(" \\\\ ")
        )
    }

    fn fenced(&mut self, node: &Node) -> String {
        let open = node.attr("open").unwrap_or("(");
        let close = node.attr("close").unwrap_or(")");
        let seps: Vec<char> = node.attr("separators").unwrap_or(",").chars().collect();
        let parts: Vec<String> = Self::elements(node)
            .into_iter()
            .map(|c| self.node(c))
            .collect();
        let mut inner = String::new();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                let sep = seps.get(i - 1).or(seps.last()).copied().unwrap_or(',');
                // A comma binds to the left operand; other separators are spaced on both sides.
                let text = escape_op(sep);
                if matches!(sep, ',' | ';') {
                    inner.push_str(&format!("{text} "));
                } else {
                    inner.push_str(&format!(" {text} "));
                }
            }
            inner.push_str(p);
        }
        format!("\\left{} {} \\right{}", fence(open), inner, fence(close))
    }
}

/// A `\left`/`\right` delimiter for a fence character.
fn fence(c: &str) -> String {
    match c {
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "⟨" => "\\langle".into(),
        "⟩" => "\\rangle".into(),
        "‖" | "∥" => "\\|".into(),
        "⌊" => "\\lfloor".into(),
        "⌋" => "\\rfloor".into(),
        "⌈" => "\\lceil".into(),
        "⌉" => "\\rceil".into(),
        "" => ".".into(),
        other => other.to_string(),
    }
}

fn accent_command(over: &str) -> Option<&'static str> {
    Some(match over.trim() {
        "^" | "ˆ" | "̂" => "\\hat",
        "¯" | "ˉ" | "‾" => "\\bar",
        "→" | "⃗" => "\\vec",
        "˜" | "̃" | "~" => "\\tilde",
        "˙" => "\\dot",
        "¨" => "\\ddot",
        _ => return None,
    })
}

fn wrap_variant(tex: &str, variant: &str) -> String {
    match variant {
        "bold" => format!("\\mathbf{{{tex}}}"),
        "normal" => format!("\\mathrm{{{tex}}}"),
        "sans-serif" => format!("\\mathsf{{{tex}}}"),
        "monospace" => format!("\\mathtt{{{tex}}}"),
        "script" => format!("\\mathcal{{{tex}}}"),
        "fraktur" => format!("\\mathfrak{{{tex}}}"),
        "double-struck" => format!("\\mathbb{{{tex}}}"),
        _ => tex.to_string(),
    }
}

/// Characters that are special in math mode, escaped when they appear as operators.
fn escape_op(c: char) -> String {
    match c {
        '{' | '}' | '#' | '$' | '%' | '&' | '_' | '^' | '~' => format!("\\{c}"),
        '\\' => "\\backslash".into(),
        c => c.to_string(),
    }
}

/// Text in an identifier or token: TeX specials are escaped.
fn escape_text_token(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '{' | '}' | '#' | '$' | '%' | '&' | '_' | '^' | '~' => {
                let _ = write!(out, "\\{c}");
            }
            '\\' => out.push_str("\\backslash "),
            c => out.push(c),
        }
    }
    out
}

fn escape_braces(s: &str) -> String {
    s.replace('{', "\\{").replace('}', "\\}")
}

#[cfg(test)]
mod tests;
