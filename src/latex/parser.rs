//! The LaTeX parser. One loop reads tokens from a stack of sources: the document at the bottom,
//! and macro expansions or `\input` files pushed on top. Each construct is read either as tokens
//! (for content) or as raw text (for arguments, math and verbatim bodies).
//!
//! `body` returns when it meets a stop token for the caller: a closing brace, an `\end`, an
//! `\item`, a table cell boundary, or the end of the input. Environments and groups are built
//! from those returns, so an unmatched delimiter produces a warning and the parse goes on.

use std::collections::HashMap;
use std::rc::Rc;

use super::lexer::{Lexer, Tok};
use super::FileProvider;
use super::{
    BibItem, Block, Cell, Cite, Document, Figure, ImagePath, Inline, LabelInfo, List, ListItem,
    ListKind, MathBlock, MathRow, Options, Ref, Row, Section, Table, Theorem, Verbatim, Warning,
};
use crate::tex_text::{self, Ligatures};

#[derive(Debug, Clone)]
struct MacroDef {
    nargs: usize,
    default: Option<String>,
    body: String,
}

#[derive(Debug, Clone)]
struct EnvDef {
    nargs: usize,
    default: Option<String>,
    begin: String,
    end: String,
}

#[derive(Debug, Clone)]
struct TheoremDef {
    name: String,
    counter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stop {
    /// The top level, or an environment body.
    Top,
    /// Inside `{...}`.
    Group,
    /// Inside a list: stops at `\item`.
    Item,
    /// Inside a table cell: stops at `&` and `\\`.
    Cell,
    /// Inside `thebibliography`: stops at `\bibitem`.
    Bib,
    /// One argument token. Never consumes a closing brace.
    Single,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    Eof,
    Close,
    EnvEnd(String),
    Item,
    Amp,
    RowEnd,
    BibItem,
}

#[derive(Debug, Default)]
struct State {
    blocks: Vec<Block>,
    para: Vec<Inline>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Figure,
    Table,
}

#[derive(Default)]
struct FloatAcc {
    caption: Option<Vec<Inline>>,
    caption_text: Option<String>,
    label: Option<String>,
    number: Option<String>,
    starred: bool,
    images: Vec<ImagePath>,
    subs: Vec<Figure>,
    tikz: Option<String>,
    rows: Vec<Row>,
    columns: Option<String>,
}

enum FloatEnd {
    Env(String),
    Group,
}

pub(super) fn parse(source: &str, options: &Options, files: Option<&dyn FileProvider>) -> Document {
    let mut doc = Document::default();
    if source.len() > options.max_input_bytes {
        doc.warnings.push(Warning {
            offset: 0,
            message: format!(
                "source is {} bytes, over the {} byte limit; nothing read",
                source.len(),
                options.max_input_bytes
            ),
        });
        return doc;
    }
    let mut p = Parser {
        main: Lexer::new(Rc::from(source), 0, true),
        stack: Vec::new(),
        opts: options,
        files,
        doc,
        macros: HashMap::new(),
        envs: HashMap::new(),
        theorems: HashMap::new(),
        counters: HashMap::new(),
        sections: [0; 6],
        ctx: ("".into(), None),
        expansions: 0,
        expansion_bytes: 0,
        files_read: 0,
        depth: 0,
        preamble: source.contains("\\begin{document}"),
        colspan: 1,
        figure_ids: 0,
        in_author: false,
        warned_limits: false,
    };
    loop {
        let (blocks, end) = p.body(&Stop::Top);
        p.doc.blocks.extend(blocks);
        match end {
            Ended::Eof => break,
            Ended::EnvEnd(ref n) if n == "document" => break,
            _ => {}
        }
    }
    p.doc
}

struct Parser<'a> {
    main: Lexer,
    stack: Vec<Lexer>,
    opts: &'a Options,
    files: Option<&'a dyn FileProvider>,
    doc: Document,
    macros: HashMap<String, MacroDef>,
    envs: HashMap<String, EnvDef>,
    theorems: HashMap<String, TheoremDef>,
    counters: HashMap<String, u32>,
    /// Section counters for levels 1 to 5 (index 0 unused).
    sections: [u32; 6],
    /// The kind and number that a `\label` takes when it appears in running text.
    ctx: (String, Option<String>),
    expansions: usize,
    expansion_bytes: usize,
    files_read: usize,
    depth: usize,
    /// True before `\begin{document}`: running text there is dropped.
    preamble: bool,
    colspan: usize,
    figure_ids: usize,
    in_author: bool,
    warned_limits: bool,
}

impl Parser<'_> {
    // ----- token and raw access ------------------------------------------------------------

    fn warn(&mut self, offset: usize, message: impl Into<String>) {
        if self.doc.warnings.len() >= self.opts.max_warnings {
            self.doc.suppressed_warnings += 1;
            return;
        }
        self.doc.warnings.push(Warning {
            offset,
            message: message.into(),
        });
    }

    fn next_tok(&mut self) -> Option<(Tok, usize)> {
        loop {
            if self.stack.is_empty() {
                return self.main.next_tok();
            }
            let n = self.stack.len();
            match self.stack[n - 1].next_tok() {
                Some(t) => return Some(t),
                None => {
                    self.stack.pop();
                }
            }
        }
    }

    /// The source that raw reads should use: the top expansion, or the main source.
    fn top(&mut self) -> &mut Lexer {
        while self.stack.last().is_some_and(|l| l.at_end()) {
            self.stack.pop();
        }
        let n = self.stack.len();
        if n == 0 {
            &mut self.main
        } else {
            &mut self.stack[n - 1]
        }
    }

    fn at_main(&self) -> bool {
        self.stack.is_empty()
    }

    fn count(&mut self, key: &str) -> u32 {
        let c = self.counters.entry(key.to_string()).or_insert(0);
        *c += 1;
        *c
    }

    fn push_label(&mut self, name: &str, kind: &str, number: Option<String>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.doc.labels.push(LabelInfo {
            name: name.to_string(),
            kind: kind.to_string(),
            number,
        });
    }

    fn record_label(&mut self, name: &str) {
        let (kind, number) = self.ctx.clone();
        let kind = if kind.is_empty() {
            "label".to_string()
        } else {
            kind
        };
        self.push_label(name, &kind, number);
    }

    // ----- the main loop --------------------------------------------------------------------

    /// Reads tokens until a stop for `stop`, returning the blocks and how the body ended.
    fn body(&mut self, stop: &Stop) -> (Vec<Block>, Ended) {
        if self.depth >= self.opts.max_depth {
            let at = self.top().offset();
            self.warn(at, "nesting limit reached; the group is skipped");
            self.skip_group_tokens();
            return (Vec::new(), Ended::Close);
        }
        self.depth += 1;
        let mut st = State::default();
        let end = loop {
            let Some((tok, at)) = self.next_tok() else {
                break Ended::Eof;
            };
            if let Some(end) = self.step(tok, at, stop, &mut st) {
                break end;
            }
        };
        self.depth -= 1;
        self.flush(&mut st);
        (st.blocks, end)
    }

    /// Consumes tokens up to the `}` that closes the current group, counting nested groups.
    fn skip_group_tokens(&mut self) {
        let mut depth = 0usize;
        while let Some((tok, _)) = self.next_tok() {
            match tok {
                Tok::Open => depth += 1,
                Tok::Close => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }

    fn flush(&mut self, st: &mut State) {
        while matches!(st.para.last(), Some(Inline::Text(t)) if t.trim().is_empty()) {
            st.para.pop();
        }
        if !st.para.is_empty() {
            st.blocks
                .push(Block::Paragraph(std::mem::take(&mut st.para)));
        }
        st.para.clear();
    }

    fn push_text(st: &mut State, s: &str) {
        if s.is_empty() || (s == " " && st.para.is_empty()) {
            return;
        }
        // Runs of spaces collapse to one, as in TeX.
        if s == " " {
            if let Some(Inline::Text(t)) = st.para.last() {
                if t.ends_with(' ') {
                    return;
                }
            }
        }
        if let Some(Inline::Text(t)) = st.para.last_mut() {
            t.push_str(s);
        } else {
            st.para.push(Inline::Text(s.to_string()));
        }
    }

    /// Adds the blocks of a group to the current state. Paragraphs join the running text.
    fn absorb(&mut self, st: &mut State, blocks: Vec<Block>) {
        let mut joined_one = false;
        for b in blocks {
            match b {
                Block::Paragraph(inl) => {
                    // Paragraphs inside a group join the running text. A second paragraph
                    // inside the same group is separated from the first by a space.
                    if joined_one {
                        st.para.push(Inline::Text(" ".into()));
                    }
                    joined_one = true;
                    st.para.extend(inl);
                }
                other => {
                    self.flush(st);
                    st.blocks.push(other);
                }
            }
        }
    }

    fn step(&mut self, tok: Tok, at: usize, stop: &Stop, st: &mut State) -> Option<Ended> {
        match tok {
            Tok::Text(s) => {
                if !self.preamble || *stop != Stop::Top {
                    let t = tex_text::decode(&s, Ligatures::Tex);
                    Self::push_text(st, &t);
                }
            }
            Tok::Space => {
                if !self.preamble || *stop != Stop::Top {
                    Self::push_text(st, " ");
                }
            }
            Tok::Par => self.flush(st),
            Tok::Open => {
                let (blocks, _end) = self.body(&Stop::Group);
                self.absorb(st, blocks);
            }
            Tok::Close => {
                if matches!(stop, Stop::Group | Stop::Single) {
                    return Some(Ended::Close);
                }
                self.warn(at, "unmatched } ignored");
            }
            Tok::Dollar => {
                let (tex, found) = self.top().until_raw("$", true);
                if !found {
                    self.warn(at, "unterminated inline math");
                }
                st.para.push(Inline::Math {
                    tex: tex.trim().to_string(),
                    display: false,
                });
            }
            Tok::DDollar => {
                let (tex, found) = self.top().until_raw("$$", true);
                if !found {
                    self.warn(at, "unterminated display math");
                }
                self.flush(st);
                let row = MathRow {
                    tex: tex.trim().to_string(),
                    number: None,
                    label: None,
                };
                st.blocks.push(Block::Math(MathBlock {
                    env: "$$".into(),
                    rows: vec![row],
                }));
            }
            Tok::Amp => {
                if *stop == Stop::Cell {
                    return Some(Ended::Amp);
                }
                self.warn(at, "& outside a table is kept as text");
                st.para.push(Inline::Text("&".into()));
            }
            Tok::Param(n) => st.para.push(Inline::Text(format!("#{n}"))),
            Tok::Verb(s) => st.para.push(Inline::Code(s)),
            Tok::Sym(c) => return self.sym(c, at, stop, st),
            Tok::Ctrl(name) => return self.control(&name, at, stop, st),
        }
        None
    }

    fn sym(&mut self, c: char, at: usize, stop: &Stop, st: &mut State) -> Option<Ended> {
        match c {
            '\\' => {
                if *stop == Stop::Cell {
                    return Some(Ended::RowEnd);
                }
                if self.top().peek_byte() == Some(b'[') {
                    let _ = self.top().optional_raw();
                }
                st.para.push(Inline::Break);
            }
            '[' => {
                let (tex, found) = self.top().until_raw("\\]", true);
                if !found {
                    self.warn(at, "unterminated display math");
                }
                self.flush(st);
                st.blocks.push(Block::Math(MathBlock {
                    env: "\\[".into(),
                    rows: vec![MathRow {
                        tex: tex.trim().to_string(),
                        number: None,
                        label: None,
                    }],
                }));
            }
            '(' => {
                let (tex, found) = self.top().until_raw("\\)", true);
                if !found {
                    self.warn(at, "unterminated inline math");
                }
                st.para.push(Inline::Math {
                    tex: tex.trim().to_string(),
                    display: false,
                });
            }
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => Self::push_text(st, &c.to_string()),
            ' ' | ',' | ';' | ':' => Self::push_text(st, " "),
            '!' | '-' | '/' | '@' | '>' | '<' | '\n' => {}
            '\'' | '`' | '^' | '"' | '~' | '=' | '.' => {
                let arg = self.accent_arg();
                let text = tex_text::decode(&format!("\\{c}{{{arg}}}"), Ligatures::None);
                Self::push_text(st, &text);
            }
            other => Self::push_text(st, &other.to_string()),
        }
        None
    }

    /// The argument of an accent command: a group, or the next character.
    fn accent_arg(&mut self) -> String {
        match self.top().peek_byte() {
            Some(b'{') => self.top().group_raw().unwrap_or_default(),
            Some(_) => self.top().single_raw().unwrap_or_default(),
            None => String::new(),
        }
    }

    /// The content of an argument: a group, or a single token. Returns no inlines when the
    /// next thing is a closing brace or the end of the input.
    fn arg_inlines(&mut self) -> Vec<Inline> {
        match self.top().peek_byte() {
            None | Some(b'}') => Vec::new(),
            Some(b'{') => match self.next_tok() {
                Some((Tok::Open, _)) => {
                    let (blocks, _) = self.body(&Stop::Group);
                    flatten(blocks)
                }
                _ => Vec::new(),
            },
            Some(_) => {
                let Some((tok, at)) = self.next_tok() else {
                    return Vec::new();
                };
                let mut st = State::default();
                let _ = self.step(tok, at, &Stop::Single, &mut st);
                self.flush(&mut st);
                flatten(st.blocks)
            }
        }
    }

    /// Parses a string as running text, with the document's macros, in isolation.
    fn isolated_inlines(&mut self, text: &str) -> Vec<Inline> {
        let saved_main = std::mem::replace(&mut self.main, Lexer::new(Rc::from(text), 0, true));
        let saved_stack = std::mem::take(&mut self.stack);
        let saved_preamble = self.preamble;
        self.preamble = false;
        let (blocks, _) = self.body(&Stop::Top);
        self.preamble = saved_preamble;
        self.main = saved_main;
        self.stack = saved_stack;
        flatten(blocks)
    }

    // ----- commands -------------------------------------------------------------------------

    fn control(&mut self, name: &str, at: usize, stop: &Stop, st: &mut State) -> Option<Ended> {
        if let Some(def) = self.macros.get(name).cloned() {
            self.expand_macro(name, &def, at);
            return None;
        }
        match name {
            "begin" => self.begin_env(at, st),
            "end" => {
                let env = self
                    .top()
                    .group_raw()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if let Some(def) = self.envs.get(&env).cloned() {
                    self.push_expansion(&def.end, at);
                    return None;
                }
                Some(Ended::EnvEnd(env))
            }
            "item" => {
                if *stop == Stop::Item {
                    return Some(Ended::Item);
                }
                self.warn(at, "\\item outside a list is ignored");
                None
            }
            "bibitem" => {
                if *stop == Stop::Bib {
                    return Some(Ended::BibItem);
                }
                self.warn(at, "\\bibitem outside thebibliography is ignored");
                None
            }
            "section" | "subsection" | "subsubsection" | "paragraph" | "subparagraph"
            | "chapter" => {
                let block = self.section(name);
                self.flush(st);
                st.blocks.push(Block::Section(block));
                None
            }
            "label" => {
                let label = self.top().group_raw().unwrap_or_default();
                self.record_label(&label);
                None
            }
            "caption" => {
                let inl = self.arg_inlines();
                self.flush(st);
                st.blocks.push(Block::Paragraph(inl));
                None
            }
            "includegraphics" => {
                let image = self.image_path();
                st.para.push(Inline::Image(image));
                None
            }
            "footnote" | "footnotetext" | "thanks" => {
                if self.in_author && name == "thanks" {
                    let _ = self.arg_inlines();
                    return None;
                }
                let inl = self.arg_inlines();
                st.para.push(Inline::Footnote(inl));
                None
            }
            "footnotemark" => {
                let _ = self.top().optional_raw();
                None
            }
            "emph" | "textit" | "textsl" => {
                let inl = self.arg_inlines();
                st.para.push(Inline::Emph(inl));
                None
            }
            "textbf" => {
                let inl = self.arg_inlines();
                st.para.push(Inline::Strong(inl));
                None
            }
            "texttt" => {
                let inl = self.arg_inlines();
                st.para.push(Inline::Mono(inl));
                None
            }
            "textsc" => {
                let inl = self.arg_inlines();
                st.para.push(Inline::SmallCaps(inl));
                None
            }
            "underline" | "textrm" | "textsf" | "textup" | "textnormal" | "mbox" | "text"
            | "textsuperscript" | "textsubscript" | "hbox" | "ensuremath" | "mathrm" | "mathbf"
            | "mathit" | "mathsf" | "mathtt" | "mathcal" | "boldsymbol" | "fbox" => {
                let inl = self.arg_inlines();
                st.para.extend(inl);
                None
            }
            "bf" | "bfseries" | "it" | "itshape" | "em" | "sl" | "slshape" | "tt" | "ttfamily"
            | "sc" | "scshape" => {
                let (blocks, _) = self.body(&Stop::Group);
                let inl = flatten(blocks);
                st.para.push(match name {
                    "bf" | "bfseries" => Inline::Strong(inl),
                    "tt" | "ttfamily" => Inline::Mono(inl),
                    "sc" | "scshape" => Inline::SmallCaps(inl),
                    _ => Inline::Emph(inl),
                });
                if *stop == Stop::Group {
                    return Some(Ended::Close);
                }
                None
            }
            "rm" | "normalfont" | "upshape" | "sf" | "sffamily" | "rmfamily" => {
                let (blocks, _) = self.body(&Stop::Group);
                st.para.extend(flatten(blocks));
                if *stop == Stop::Group {
                    return Some(Ended::Close);
                }
                None
            }
            "cite" | "citep" | "citet" | "citealp" | "citealt" | "citeauthor" | "citeyear"
            | "citeyearpar" | "nocite" | "parencite" | "textcite" | "autocite" | "footcite"
            | "citetext" => {
                let cite = self.cite(name);
                st.para.push(Inline::Cite(cite));
                None
            }
            "ref" | "eqref" | "autoref" | "pageref" | "cref" | "Cref" | "vref" | "nameref" => {
                let label = self
                    .top()
                    .group_raw()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                st.para.push(Inline::Ref(Ref {
                    command: name.to_string(),
                    label,
                }));
                None
            }
            "url" => {
                let url = self
                    .top()
                    .group_raw()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                st.para.push(Inline::Link {
                    url: url.clone(),
                    text: vec![Inline::Text(url)],
                });
                None
            }
            "href" => {
                let url = self
                    .top()
                    .group_raw()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let text = self.arg_inlines();
                st.para.push(Inline::Link { url, text });
                None
            }
            "hyperlink" => {
                let _ = self.top().group_raw();
                let text = self.arg_inlines();
                st.para.extend(text);
                None
            }
            "hyperref" => {
                let _ = self.top().optional_raw();
                let text = self.arg_inlines();
                st.para.extend(text);
                None
            }
            "par" => {
                self.flush(st);
                None
            }
            "newline" => {
                st.para.push(Inline::Break);
                None
            }
            "tabularnewline" => {
                if *stop == Stop::Cell {
                    return Some(Ended::RowEnd);
                }
                st.para.push(Inline::Break);
                None
            }
            "quad" | "qquad" | "enspace" | "and" | "AND" => {
                if (name == "and" || name == "AND") && self.in_author {
                    return None;
                }
                Self::push_text(st, " ");
                None
            }
            "title" => {
                let inl = self.arg_inlines();
                self.doc.title = Some(inl);
                None
            }
            "author" => {
                self.author();
                None
            }
            "date" => {
                let raw = self.top().group_raw().unwrap_or_default();
                let text = tex_text::decode(&raw, Ligatures::None);
                self.doc.date = Some(text.trim().to_string());
                None
            }
            "newcommand" | "renewcommand" | "providecommand" => {
                self.define_command(name, at);
                None
            }
            "def" => {
                self.define_plain(at);
                None
            }
            "DeclareMathOperator" => {
                let _ = self.top().eat(b'*');
                let target = self.command_name_arg();
                let text = self.top().group_raw().unwrap_or_default();
                if let Some(target) = target {
                    self.macros.insert(
                        target,
                        MacroDef {
                            nargs: 0,
                            default: None,
                            body: format!("\\operatorname{{{text}}}"),
                        },
                    );
                }
                None
            }
            "newenvironment" | "renewenvironment" => {
                self.define_env();
                None
            }
            "newtheorem" => {
                self.define_theorem();
                None
            }
            "input" | "include" | "subfile" => {
                self.include(name, at);
                None
            }
            "bibliography" | "addbibresource" => {
                let _ = self.top().optional_raw();
                let raw = self.top().group_raw().unwrap_or_default();
                for item in raw.split(',') {
                    let item = item.trim();
                    if !item.is_empty() {
                        self.doc.bibliography_files.push(item.to_string());
                    }
                }
                None
            }
            "printbibliography" | "bibliographystyle" => {
                let _ = self.top().optional_raw();
                let _ = self.top().group_raw();
                None
            }
            "newcounter" => {
                let _ = self.top().group_raw();
                let _ = self.top().optional_raw();
                None
            }
            "parbox" => {
                let _ = self.top().optional_raw();
                let _ = self.top().group_raw();
                let inl = self.arg_inlines();
                st.para.extend(inl);
                None
            }
            "raisebox" => {
                let _ = self.top().group_raw();
                let _ = self.top().optional_raw();
                let _ = self.top().optional_raw();
                let inl = self.arg_inlines();
                st.para.extend(inl);
                None
            }
            "textcolor" => {
                let _ = self.top().group_raw();
                let inl = self.arg_inlines();
                st.para.extend(inl);
                None
            }
            "multicolumn" => {
                let n = self.top().group_raw().unwrap_or_default();
                let _ = self.top().group_raw();
                self.colspan = n.trim().parse::<usize>().unwrap_or(1).max(1);
                let inl = self.arg_inlines();
                st.para.extend(inl);
                None
            }
            "LaTeX" | "TeX" => {
                Self::push_text(st, name);
                None
            }
            "noindent" | "indent" | "centering" | "raggedright" | "raggedleft" | "maketitle"
            | "tableofcontents" | "listoffigures" | "listoftables" | "newpage" | "clearpage"
            | "cleardoublepage" | "appendix" | "bigskip" | "medskip" | "smallskip" | "hfill"
            | "vfill" | "hline" | "toprule" | "midrule" | "bottomrule" | "small"
            | "footnotesize" | "scriptsize" | "tiny" | "normalsize" | "large" | "Large"
            | "LARGE" | "huge" | "Huge" | "sloppy" | "fussy" | "frenchspacing" | "raggedbottom"
            | "flushbottom" | "relax" | "protect" | "ignorespaces" | "unskip" | "selectfont"
            | "linebreak" | "nolinebreak" | "nopagebreak" | "pagebreak" | "smallbreak"
            | "medbreak" | "bigbreak" | "par@" => {
                if name == "linebreak" || name == "pagebreak" {
                    let _ = self.top().optional_raw();
                }
                if name == "linebreak" {
                    st.para.push(Inline::Break);
                }
                None
            }
            _ => {
                if name.chars().count() == 1 {
                    if let Some(c) = name.chars().next() {
                        if tex_text::combining_mark(c).is_some() {
                            self.accent_control(c, st);
                            return None;
                        }
                    }
                }
                if let Some(sym) = tex_text::symbol(name) {
                    Self::push_text(st, sym);
                    return None;
                }
                if tex_text::is_transparent(name) {
                    return None;
                }
                if let Some((opts, mands)) = drop_spec(name) {
                    let _ = self.top().eat(b'*');
                    for _ in 0..opts {
                        let _ = self.top().optional_raw();
                    }
                    for _ in 0..mands {
                        let _ = self.top().group_raw();
                    }
                    return None;
                }
                self.warn(
                    at,
                    format!("unknown command \\{name}; its arguments are kept as text"),
                );
                None
            }
        }
    }

    /// A letter accent such as `\c{c}` or `\v{s}`.
    fn accent_control(&mut self, c: char, st: &mut State) {
        let arg = self.accent_arg();
        let text = tex_text::decode(&format!("\\{c}{{{arg}}}"), Ligatures::None);
        Self::push_text(st, &text);
    }

    fn cite(&mut self, name: &str) -> Cite {
        let _ = self.top().eat(b'*');
        let first = self.top().optional_raw();
        let (pre, post) = match first {
            Some(a) => match self.top().optional_raw() {
                Some(b) => (Some(a), Some(b)),
                None => (None, Some(a)),
            },
            None => (None, None),
        };
        let raw = self.top().group_raw().unwrap_or_default();
        let keys = raw
            .split(',')
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect();
        let decode = |s: String| tex_text::decode(s.trim(), Ligatures::Tex);
        Cite {
            command: name.to_string(),
            pre: pre.map(decode).filter(|s| !s.is_empty()),
            post: post.map(decode).filter(|s| !s.is_empty()),
            keys,
        }
    }

    fn image_path(&mut self) -> ImagePath {
        let _ = self.top().eat(b'*');
        let options = self
            .top()
            .optional_raw()
            .map(|raw| parse_options(&raw))
            .unwrap_or_default();
        let declared = self
            .top()
            .group_raw()
            .unwrap_or_default()
            .trim()
            .to_string();
        ImagePath {
            candidates: image_candidates(&declared),
            declared,
            options,
        }
    }

    fn section(&mut self, name: &str) -> Section {
        let starred = self.top().eat(b'*');
        let _ = self.top().optional_raw();
        let level: u8 = match name {
            "chapter" | "section" => 1,
            "subsection" => 2,
            "subsubsection" => 3,
            "paragraph" => 4,
            _ => 5,
        };
        let number = if starred || level > 3 {
            None
        } else {
            let l = usize::from(level);
            self.sections[l] += 1;
            for deeper in (l + 1)..self.sections.len() {
                self.sections[deeper] = 0;
            }
            Some(
                self.sections[1..=l]
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join("."),
            )
        };
        self.ctx = ("section".into(), number.clone());
        let before = self.doc.labels.len();
        let title = self.arg_inlines();
        let label = self.doc.labels.get(before).map(|l| l.name.clone());
        Section {
            level,
            starred,
            title,
            number,
            label,
        }
    }

    fn author(&mut self) {
        let raw = self.top().group_raw().unwrap_or_default();
        let saved = self.in_author;
        self.in_author = true;
        for piece in split_authors(&raw) {
            let inl = self.isolated_inlines(piece.trim());
            if !inl.is_empty() {
                self.doc.authors.push(inl);
            }
        }
        self.in_author = saved;
    }

    fn command_name_arg(&mut self) -> Option<String> {
        let raw = if self.top().peek_byte() == Some(b'{') {
            self.top().group_raw()
        } else {
            self.top().ctrl_name_raw()
        };
        raw.map(|s| s.trim().trim_start_matches('\\').to_string())
            .filter(|s| !s.is_empty())
    }

    fn define_command(&mut self, command: &str, at: usize) {
        let Some(name) = self.command_name_arg() else {
            self.warn(at, format!("\\{command} without a command name"));
            return;
        };
        let nargs_raw = self.top().optional_raw();
        let nargs = match nargs_raw {
            Some(s) => match s.trim().parse::<usize>() {
                Ok(n) if n <= 9 => n,
                _ => {
                    self.warn(at, format!("bad argument count for \\{name}; using 0"));
                    0
                }
            },
            None => 0,
        };
        let default = if nargs > 0 {
            self.top().optional_raw()
        } else {
            None
        };
        let body = self.top().group_raw().unwrap_or_default();
        if command == "providecommand" && self.macros.contains_key(&name) {
            return;
        }
        self.macros.insert(
            name,
            MacroDef {
                nargs,
                default,
                body,
            },
        );
    }

    fn define_plain(&mut self, at: usize) {
        let Some(name) = self
            .top()
            .ctrl_name_raw()
            .map(|s| s.trim_start_matches('\\').to_string())
        else {
            self.warn(at, "\\def without a command name");
            return;
        };
        let params = self.top().text_until_brace();
        if params.chars().any(|c| c != '#' && !c.is_ascii_digit()) {
            self.warn(
                at,
                format!("delimited parameters in \\def\\{name} are not supported"),
            );
        }
        let nargs = params.matches('#').count().min(9);
        let body = self.top().group_raw().unwrap_or_default();
        self.macros.insert(
            name,
            MacroDef {
                nargs,
                default: None,
                body,
            },
        );
    }

    fn define_env(&mut self) {
        let name = self
            .top()
            .group_raw()
            .unwrap_or_default()
            .trim()
            .to_string();
        let nargs = self
            .top()
            .optional_raw()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0)
            .min(9);
        let default = if nargs > 0 {
            self.top().optional_raw()
        } else {
            None
        };
        let begin = self.top().group_raw().unwrap_or_default();
        let end = self.top().group_raw().unwrap_or_default();
        self.envs.insert(
            name,
            EnvDef {
                nargs,
                default,
                begin,
                end,
            },
        );
    }

    fn define_theorem(&mut self) {
        let name = self
            .top()
            .group_raw()
            .unwrap_or_default()
            .trim()
            .to_string();
        let shared = self.top().optional_raw();
        let title = self.top().group_raw().unwrap_or_default();
        let _ = self.top().optional_raw();
        let counter = shared
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| name.clone());
        self.theorems.insert(
            name,
            TheoremDef {
                name: title,
                counter,
            },
        );
    }

    fn include(&mut self, command: &str, at: usize) {
        let name = self
            .top()
            .group_raw()
            .unwrap_or_default()
            .trim()
            .to_string();
        let text = match self.files {
            Some(files) if self.files_read < self.opts.max_files => {
                self.files_read += 1;
                files
                    .read_tex(&name)
                    .or_else(|| files.read_tex(&format!("{name}.tex")))
            }
            _ => None,
        };
        match text {
            Some(t) if t.len() <= self.opts.max_input_bytes => {
                if self.stack.len() >= self.opts.max_depth {
                    self.warn(at, format!("\\{command} nesting limit reached"));
                    return;
                }
                self.stack.push(Lexer::new(Rc::from(t), at, false));
            }
            Some(_) => self.warn(at, format!("\\{command}{{{name}}} is over the size limit")),
            None => {
                self.warn(
                    at,
                    format!("\\{command}{{{name}}} not resolved; no file provider"),
                );
                self.doc.inputs.push(name);
            }
        }
    }

    fn expand_macro(&mut self, name: &str, def: &MacroDef, at: usize) {
        self.expansions += 1;
        if self.expansions > self.opts.max_expansions {
            if !self.warned_limits {
                self.warned_limits = true;
                self.warn(
                    at,
                    "macro expansion limit reached; further expansions are dropped",
                );
            }
            return;
        }
        let mut args: Vec<String> = Vec::new();
        let mut mandatory = def.nargs;
        if let Some(d) = &def.default {
            let opt = self.top().optional_raw();
            args.push(opt.unwrap_or_else(|| d.clone()));
            mandatory = mandatory.saturating_sub(1);
        }
        for _ in 0..mandatory {
            args.push(self.top().single_raw().unwrap_or_default());
        }
        let expanded = substitute(&def.body, &args);
        self.expansion_bytes += expanded.len();
        if self.expansion_bytes > self.opts.max_expansion_bytes {
            if !self.warned_limits {
                self.warned_limits = true;
                self.warn(at, format!("expansion size limit reached at \\{name}"));
            }
            return;
        }
        self.push_expansion(&expanded, at);
    }

    fn push_expansion(&mut self, text: &str, at: usize) {
        if self.stack.len() >= self.opts.max_depth {
            self.warn(at, "expansion depth limit reached");
            return;
        }
        self.stack.push(Lexer::new(Rc::from(text), at, false));
    }

    // ----- environments ---------------------------------------------------------------------

    fn begin_env(&mut self, at: usize, st: &mut State) -> Option<Ended> {
        let Some(name) = self.top().group_raw().map(|s| s.trim().to_string()) else {
            self.warn(at, "\\begin without an environment name");
            return None;
        };
        if let Some(def) = self.envs.get(&name).cloned() {
            let mut args = Vec::new();
            let mut mandatory = def.nargs;
            if let Some(d) = &def.default {
                args.push(self.top().optional_raw().unwrap_or_else(|| d.clone()));
                mandatory = mandatory.saturating_sub(1);
            }
            for _ in 0..mandatory {
                args.push(self.top().single_raw().unwrap_or_default());
            }
            let text = substitute(&def.begin, &args);
            self.push_expansion(&text, at);
            return None;
        }
        match name.as_str() {
            "document" => {
                self.preamble = false;
            }
            "abstract" => {
                let blocks = self.env_body(&name);
                match self.doc.abstract_body.as_mut() {
                    Some(existing) => existing.extend(blocks),
                    None => self.doc.abstract_body = Some(blocks),
                }
            }
            "itemize" | "enumerate" | "description" => {
                let _ = self.top().optional_raw();
                let list = self.list(&name);
                self.flush(st);
                st.blocks.push(Block::List(list));
            }
            "tabular" | "tabular*" | "tabularx" | "longtable" => {
                let table = self.table_env(&name);
                self.flush(st);
                st.blocks.push(Block::Table(table));
            }
            "table" | "table*" => {
                let block = self.float(&name, Kind::Table, at);
                self.flush(st);
                st.blocks.push(block);
            }
            "figure" | "figure*" | "wrapfigure" | "wrapfigure*" | "subfigure" | "subfigure*" => {
                let block = self.float(&name, Kind::Figure, at);
                self.flush(st);
                st.blocks.push(block);
            }
            "tikzpicture" => {
                let _ = self.top().optional_raw();
                let (tex, found) = self.top().until_raw("\\end{tikzpicture}", false);
                if !found {
                    self.warn(at, "unterminated tikzpicture");
                }
                self.flush(st);
                let fig = self.make_figure(
                    "tikzpicture",
                    FloatAcc {
                        tikz: Some(tex),
                        ..FloatAcc::default()
                    },
                    at,
                );
                st.blocks.push(Block::Figure(fig));
            }
            "center" | "flushleft" | "flushright" | "minipage" | "small" | "footnotesize"
            | "scriptsize" | "normalsize" | "large" => {
                if name == "minipage" {
                    let _ = self.top().optional_raw();
                    let _ = self.top().group_raw();
                    let _ = self.top().optional_raw();
                    let _ = self.top().optional_raw();
                }
                let blocks = self.env_body(&name);
                self.flush(st);
                st.blocks.extend(blocks);
            }
            "quote" | "quotation" | "verse" => {
                let blocks = self.env_body(&name);
                self.flush(st);
                st.blocks.push(Block::Quote(blocks));
            }
            "verbatim" | "Verbatim" | "BVerbatim" | "alltt" | "lstlisting" | "minted" => {
                let mut language = None;
                if name == "minted" {
                    let _ = self.top().optional_raw();
                    language = self.top().group_raw().map(|s| s.trim().to_string());
                } else {
                    if let Some(opts) = self.top().optional_raw() {
                        language = parse_options(&opts)
                            .into_iter()
                            .find(|(k, _)| k == "language")
                            .map(|(_, v)| v);
                    }
                }
                let end = format!("\\end{{{name}}}");
                let (text, found) = self.top().until_raw(&end, false);
                if !found {
                    self.warn(at, format!("unterminated {name}"));
                }
                let text = text.strip_prefix('\n').unwrap_or(&text).to_string();
                self.flush(st);
                st.blocks.push(Block::Verbatim(Verbatim {
                    env: name,
                    language,
                    text,
                }));
            }
            "comment" => {
                let _ = self.top().until_raw("\\end{comment}", false);
            }
            "equation" | "equation*" | "align" | "align*" | "alignat" | "alignat*" | "gather"
            | "gather*" | "multline" | "multline*" | "eqnarray" | "eqnarray*" | "displaymath" => {
                let block = self.math_env(&name);
                self.flush(st);
                st.blocks.push(Block::Math(block));
            }
            "math" => {
                let (tex, found) = self.top().until_raw("\\end{math}", true);
                if !found {
                    self.warn(at, "unterminated math environment");
                }
                st.para.push(Inline::Math {
                    tex: tex.trim().to_string(),
                    display: false,
                });
            }
            "thebibliography" => {
                let _ = self.top().group_raw();
                self.bibliography_env(at);
            }
            "proof" => {
                let title = self.top().optional_raw().map(|t| self.isolated_inlines(&t));
                let before = self.doc.labels.len();
                let saved = self.ctx.clone();
                self.ctx = ("theorem".into(), None);
                let body = self.env_body(&name);
                self.ctx = saved;
                let label = self.doc.labels.get(before).map(|l| l.name.clone());
                self.flush(st);
                st.blocks.push(Block::Theorem(Theorem {
                    env: name,
                    name: "Proof".into(),
                    title,
                    number: None,
                    label,
                    body,
                }));
            }
            _ => {
                if let Some(def) = self.theorems.get(&name).cloned() {
                    let base = name.trim_end_matches('*').to_string();
                    let starred = name.ends_with('*');
                    let title = self.top().optional_raw().map(|t| self.isolated_inlines(&t));
                    let number = if starred {
                        None
                    } else {
                        Some(self.count(&def.counter).to_string())
                    };
                    let before = self.doc.labels.len();
                    let saved = self.ctx.clone();
                    self.ctx = ("theorem".into(), number.clone());
                    let body = self.env_body(&name);
                    self.ctx = saved;
                    let label = self.doc.labels.get(before).map(|l| l.name.clone());
                    self.flush(st);
                    st.blocks.push(Block::Theorem(Theorem {
                        env: base,
                        name: def.name,
                        title,
                        number,
                        label,
                        body,
                    }));
                } else {
                    self.warn(
                        at,
                        format!("unknown environment {name}; its content is kept"),
                    );
                    let blocks = self.env_body(&name);
                    self.flush(st);
                    st.blocks.extend(blocks);
                }
            }
        }
        None
    }

    /// Body of an environment whose `\end` has not been read. Stops at its own `\end`.
    fn env_body(&mut self, name: &str) -> Vec<Block> {
        let mut out = Vec::new();
        loop {
            let (blocks, end) = self.body(&Stop::Top);
            out.extend(blocks);
            match end {
                Ended::EnvEnd(ref n) if n == name => break,
                Ended::EnvEnd(n) => {
                    let at = self.top().offset();
                    self.warn(at, format!("\\end{{{n}}} does not match \\begin{{{name}}}"));
                }
                Ended::Eof => {
                    let at = self.top().offset();
                    self.warn(at, format!("\\begin{{{name}}} is never closed"));
                    break;
                }
                _ => {}
            }
        }
        out
    }

    fn list(&mut self, env: &str) -> List {
        let kind = match env {
            "itemize" => ListKind::Itemize,
            "enumerate" => ListKind::Enumerate,
            _ => ListKind::Description,
        };
        let mut items: Vec<ListItem> = Vec::new();
        let mut current: Option<ListItem> = None;
        loop {
            let (blocks, end) = self.body(&Stop::Item);
            match current.as_mut() {
                Some(cur) => cur.blocks.extend(blocks),
                None => {
                    if !blocks.is_empty() {
                        let at = self.top().offset();
                        self.warn(at, "text before the first \\item is kept outside the list");
                    }
                }
            }
            match end {
                Ended::Item => {
                    if let Some(done) = current.take() {
                        items.push(done);
                    }
                    let label = if self.top().peek_byte() == Some(b'[') {
                        self.top()
                            .optional_raw()
                            .map(|raw| self.isolated_inlines(&raw))
                    } else {
                        None
                    };
                    current = Some(ListItem {
                        label,
                        blocks: Vec::new(),
                    });
                }
                Ended::EnvEnd(ref n) if n == env => break,
                Ended::Eof => break,
                other => {
                    let at = self.top().offset();
                    self.warn(at, format!("unexpected {other:?} inside {env}"));
                }
            }
        }
        if let Some(done) = current.take() {
            items.push(done);
        }
        List { kind, items }
    }

    fn table_env(&mut self, name: &str) -> Table {
        if matches!(name, "tabularx" | "tabular*") {
            let _ = self.top().group_raw();
        }
        if matches!(name, "tabular" | "tabular*" | "longtable") {
            let _ = self.top().optional_raw();
        }
        let columns = self.top().group_raw();
        let rows = self.table_rows(name);
        Table {
            env: name.to_string(),
            caption: None,
            caption_text: None,
            label: None,
            number: None,
            columns,
            rows,
        }
    }

    fn table_rows(&mut self, name: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut row: Vec<Cell> = Vec::new();
        loop {
            self.colspan = 1;
            let (blocks, end) = self.body(&Stop::Cell);
            let content = flatten(blocks);
            let colspan = std::mem::replace(&mut self.colspan, 1);
            match end {
                Ended::Amp => row.push(Cell { content, colspan }),
                Ended::RowEnd => {
                    row.push(Cell { content, colspan });
                    if self.top().peek_byte() == Some(b'[') {
                        let _ = self.top().optional_raw();
                    }
                    rows.push(Row {
                        cells: std::mem::take(&mut row),
                    });
                }
                Ended::EnvEnd(ref n) if n == name => {
                    if !content.is_empty() || colspan > 1 || !row.is_empty() {
                        row.push(Cell { content, colspan });
                    }
                    if !row.is_empty() {
                        rows.push(Row { cells: row });
                    }
                    break;
                }
                Ended::Eof => {
                    if !content.is_empty() || !row.is_empty() {
                        row.push(Cell { content, colspan });
                        rows.push(Row { cells: row });
                    }
                    break;
                }
                other => {
                    let at = self.top().offset();
                    self.warn(at, format!("unexpected {other:?} inside {name}"));
                }
            }
        }
        rows
    }

    fn skip_float_args(&mut self, name: &str) {
        if name.starts_with("wrapfigure") {
            let _ = self.top().optional_raw();
            let _ = self.top().group_raw();
            let _ = self.top().optional_raw();
            let _ = self.top().group_raw();
        } else if name.starts_with("subfigure") {
            let _ = self.top().optional_raw();
            let _ = self.top().group_raw();
        } else {
            let _ = self.top().optional_raw();
        }
    }

    /// A `figure` or `table` environment, or a subfigure environment at the top level.
    fn float(&mut self, name: &str, kind: Kind, at: usize) -> Block {
        self.skip_float_args(name);
        let mut acc = FloatAcc::default();
        self.float_body(name, kind, &mut acc, FloatEnd::Env(name.to_string()));
        match kind {
            Kind::Table => {
                let table = self.finish_table(name, acc, at);
                Block::Table(table)
            }
            Kind::Figure => {
                let fig = self.finish_figure(name, acc, at);
                Block::Figure(fig)
            }
        }
    }

    fn float_body(&mut self, env: &str, kind: Kind, acc: &mut FloatAcc, end: FloatEnd) {
        while let Some((tok, at)) = self.next_tok() {
            match tok {
                Tok::Close if matches!(end, FloatEnd::Group) => break,
                Tok::Open => self.float_body(env, kind, acc, FloatEnd::Group),
                Tok::Ctrl(name) => match name.as_str() {
                    "end" => {
                        let n = self
                            .top()
                            .group_raw()
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        if matches!(&end, FloatEnd::Env(e) if *e == n) {
                            break;
                        }
                    }
                    "caption" => self.caption_into(env, kind, acc),
                    "label" => {
                        let label = self.top().group_raw().unwrap_or_default();
                        if acc.label.is_none() {
                            acc.label = Some(label.trim().to_string());
                        }
                    }
                    "includegraphics" => {
                        let image = self.image_path();
                        acc.images.push(image);
                    }
                    "subfloat" => {
                        let caption = self.top().optional_raw();
                        let label = self.top().optional_raw();
                        if self.top().peek_byte() == Some(b'{') {
                            let _ = self.next_tok();
                            let mut sub = FloatAcc::default();
                            if let Some(c) = caption {
                                sub.caption_text = Some(c.clone());
                                sub.caption = Some(self.isolated_inlines(&c));
                            }
                            if let Some(l) = label {
                                sub.label = Some(l.trim().to_string());
                            }
                            self.float_body("subfloat", Kind::Figure, &mut sub, FloatEnd::Group);
                            let fig = self.finish_sub(sub, "subfloat", at);
                            acc.subs.push(fig);
                        }
                    }
                    "begin" => {
                        let n = self
                            .top()
                            .group_raw()
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        match n.as_str() {
                            "subfigure" | "subfigure*" | "subfloat" => {
                                self.skip_float_args(&n);
                                let mut sub = FloatAcc::default();
                                self.float_body(
                                    &n,
                                    Kind::Figure,
                                    &mut sub,
                                    FloatEnd::Env(n.clone()),
                                );
                                let fig = self.finish_sub(sub, &n, at);
                                acc.subs.push(fig);
                            }
                            "tabular" | "tabular*" | "tabularx" | "longtable" => {
                                let table = self.table_env(&n);
                                acc.rows.extend(table.rows);
                                if table.columns.is_some() {
                                    acc.columns = table.columns;
                                }
                            }
                            "tikzpicture" => {
                                let _ = self.top().optional_raw();
                                let (tex, _) = self.top().until_raw("\\end{tikzpicture}", false);
                                acc.tikz = Some(tex);
                            }
                            _ => {}
                        }
                    }
                    other => {
                        // Macros expand into the token stream; other commands contribute nothing.
                        let mut st = State::default();
                        let _ = self.step(Tok::Ctrl(other.to_string()), at, &Stop::Single, &mut st);
                    }
                },
                _ => {}
            }
        }
    }

    fn caption_into(&mut self, env: &str, kind: Kind, acc: &mut FloatAcc) {
        let starred = self.top().eat(b'*');
        let _ = self.top().optional_raw();
        let base = match kind {
            Kind::Figure => "figure",
            Kind::Table => "table",
        };
        let is_sub = env.starts_with("subfig") || env == "subfloat";
        let number = if starred {
            None
        } else if is_sub {
            let n = self.count("subfigure");
            Some(letter(n))
        } else {
            Some(self.count(base).to_string())
        };
        let kind_name = if is_sub { "subfigure" } else { base };
        self.ctx = (kind_name.to_string(), number.clone());
        let inl = self.arg_inlines();
        acc.caption_text = Some(plain(&inl));
        acc.caption = Some(inl);
        acc.number = number;
        acc.starred = starred;
    }

    /// A subfigure. Its label is recorded by the parent figure, once its number is known.
    fn finish_sub(&mut self, acc: FloatAcc, env: &str, at: usize) -> Figure {
        self.make_figure(env, acc, at)
    }

    fn make_figure(&mut self, env: &str, acc: FloatAcc, at: usize) -> Figure {
        self.figure_ids += 1;
        let end = self.top().offset();
        Figure {
            id: self.figure_ids,
            env: env.to_string(),
            starred: acc.starred,
            label: acc.label,
            number: acc.number,
            caption: acc.caption,
            caption_text: acc.caption_text,
            image_paths: acc.images,
            subfigures: acc.subs,
            tikz: acc.tikz,
            source_span: (at, end.max(at)),
        }
    }

    fn finish_figure(&mut self, env: &str, mut acc: FloatAcc, at: usize) -> Figure {
        // Subfigure numbers are the parent number followed by the letter.
        let parent = acc.number.clone();
        for sub in &mut acc.subs {
            if let (Some(p), Some(l)) = (parent.as_ref(), sub.number.as_ref()) {
                sub.number = Some(format!("{p}{l}"));
            }
        }
        let label = acc.label.clone();
        let number = acc.number.clone();
        for sub in &acc.subs {
            if let Some(sub_label) = &sub.label {
                let sub_label = sub_label.clone();
                self.push_label(&sub_label, "subfigure", sub.number.clone());
            }
        }
        if let Some(l) = &label {
            self.push_label(l, "figure", number);
        }
        self.make_figure(env, acc, at)
    }

    fn finish_table(&mut self, env: &str, acc: FloatAcc, at: usize) -> Table {
        let label = acc.label.clone();
        let number = acc.number.clone();
        if let Some(l) = &label {
            self.push_label(l, "table", number.clone());
        }
        let _ = at;
        Table {
            env: env.to_string(),
            caption: acc.caption,
            caption_text: acc.caption_text,
            label,
            number,
            columns: acc.columns,
            rows: acc.rows,
        }
    }

    fn math_env(&mut self, name: &str) -> MathBlock {
        let end = format!("\\end{{{name}}}");
        let (raw, found) = self.top().until_raw(&end, true);
        if !found {
            let at = self.top().offset();
            self.warn(at, format!("unterminated {name}"));
        }
        let base = name.trim_end_matches('*');
        let starred = name.ends_with('*');
        let numbered = !starred
            && matches!(
                base,
                "equation" | "align" | "alignat" | "gather" | "multline" | "eqnarray"
            );
        let splits = matches!(base, "align" | "alignat" | "gather" | "eqnarray");
        let pieces = if splits { split_rows(&raw) } else { vec![raw] };
        let mut rows = Vec::new();
        for piece in pieces {
            let (tex, label, nonumber, tag) = clean_row(&piece);
            if tex.trim().is_empty() && label.is_none() {
                continue;
            }
            let number = if let Some(t) = tag {
                Some(t)
            } else if numbered && !nonumber {
                Some(self.count("equation").to_string())
            } else {
                None
            };
            if let Some(l) = &label {
                self.push_label(l, "equation", number.clone());
            }
            rows.push(MathRow { tex, number, label });
        }
        MathBlock {
            env: name.to_string(),
            rows,
        }
    }

    fn bibliography_env(&mut self, at: usize) {
        let mut items: Vec<BibItem> = Vec::new();
        let mut current: Option<(String, Option<String>, Vec<Inline>, usize)> = None;
        let mut current_start = self.top().offset();
        loop {
            let (blocks, end) = self.body(&Stop::Bib);
            if let Some(cur) = current.as_mut() {
                cur.2.extend(flatten(blocks));
            }
            match end {
                Ended::BibItem => {
                    if let Some(done) = current.take() {
                        items.push(self.finish_item(done, current_start));
                    }
                    current_start = self.top().offset();
                    let label = self.top().optional_raw().map(|s| s.trim().to_string());
                    let key = self
                        .top()
                        .group_raw()
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    current = Some((key, label, Vec::new(), current_start));
                }
                Ended::EnvEnd(ref n) if n == "thebibliography" => break,
                Ended::Eof => break,
                other => {
                    let pos = self.top().offset();
                    self.warn(pos, format!("unexpected {other:?} inside thebibliography"));
                }
            }
        }
        if let Some(done) = current.take() {
            items.push(self.finish_item(done, current_start));
        }
        let _ = at;
        self.doc.bibitems.extend(items);
    }

    fn finish_item(
        &mut self,
        item: (String, Option<String>, Vec<Inline>, usize),
        start: usize,
    ) -> BibItem {
        let end = self.top().offset();
        let raw = if self.at_main() && start <= end {
            self.main.source_slice(start, end)
        } else {
            String::new()
        };
        BibItem {
            key: item.0,
            label: item.1,
            text: item.2,
            raw,
        }
    }
}

/// Converts a body into inlines: paragraphs are joined by a space, and display math becomes
/// `Inline::Math { display: true }`. Other blocks contribute their inline content where they
/// have one.
pub(super) fn flatten(blocks: Vec<Block>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(inl) => {
                if !out.is_empty() {
                    out.push(Inline::Text(" ".into()));
                }
                out.extend(inl);
            }
            Block::Math(m) => {
                let tex = m
                    .rows
                    .iter()
                    .map(|r| r.tex.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(Inline::Math { tex, display: true });
            }
            Block::Quote(inner) => out.extend(flatten(inner)),
            Block::List(list) => {
                for item in list.items {
                    out.extend(flatten(item.blocks));
                }
            }
            _ => {}
        }
    }
    out
}

/// Plain text of inline content, for captions and titles.
pub(super) fn plain(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inl in inlines {
        match inl {
            Inline::Text(t) | Inline::Code(t) => s.push_str(t),
            Inline::Emph(v)
            | Inline::Strong(v)
            | Inline::Mono(v)
            | Inline::SmallCaps(v)
            | Inline::Footnote(v) => s.push_str(&plain(v)),
            Inline::Link { text, .. } => s.push_str(&plain(text)),
            Inline::Math { tex, .. } => s.push_str(tex),
            Inline::Ref(r) => s.push_str(&r.label),
            Inline::Cite(c) => s.push_str(&c.keys.join(", ")),
            Inline::Break => s.push(' '),
            Inline::Image(_) => {}
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn letter(n: u32) -> String {
    let idx = (n.max(1) - 1) % 26;
    char::from(b'a' + idx as u8).to_string()
}

/// Replaces `#1` to `#9` in a macro body. `##` gives `#`.
fn substitute(body: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '#' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('#') => {
                chars.next();
                out.push('#');
            }
            Some(d @ '1'..='9') => {
                let idx = (d as usize) - ('0' as usize);
                if idx <= args.len() {
                    chars.next();
                    out.push_str(&args[idx - 1]);
                } else {
                    out.push('#');
                }
            }
            _ => out.push('#'),
        }
    }
    out
}

/// Splits `\author` at `\and` and `\AND`, outside braces. A `\\` is a line break inside one
/// author's block (an affiliation), so it stays in that author's text.
fn split_authors(raw: &str) -> Vec<&str> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'\\' if depth == 0 => {
                let rest = &bytes[i..];
                let word_end = |n: usize| rest.get(n).is_none_or(|b| !b.is_ascii_alphabetic());
                if rest.starts_with(b"\\and") && word_end(4) {
                    out.push(&raw[start..i]);
                    i += 4;
                    start = i;
                    continue;
                }
                if rest.starts_with(b"\\AND") && word_end(4) {
                    out.push(&raw[start..i]);
                    i += 4;
                    start = i;
                    continue;
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&raw[start..]);
    out
}

/// Splits an alignment body at `\\` outside braces and outside `\begin`/`\end` pairs.
fn split_rows(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let mut rows = Vec::new();
    let mut depth = 0usize;
    let mut env_depth = 0i64;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'\\' => {
                let rest = &bytes[i..];
                if rest.starts_with(b"\\begin{") {
                    env_depth += 1;
                } else if rest.starts_with(b"\\end{") {
                    env_depth -= 1;
                } else if rest.starts_with(b"\\\\") && depth == 0 && env_depth <= 0 {
                    rows.push(raw[start..i].to_string());
                    i += 2;
                    if bytes.get(i) == Some(&b'[') {
                        while i < bytes.len() && bytes[i] != b']' {
                            i += 1;
                        }
                        i += 1;
                    }
                    start = i;
                    continue;
                } else {
                    // Skip the escaped character so that `\\` pairs are not misread.
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    rows.push(raw[start.min(raw.len())..].to_string());
    rows
}

/// Removes `\label{..}`, `\nonumber` and `\notag` from one row. Returns the TeX, the first
/// label, whether the row is unnumbered, and the `\tag` value if any.
fn clean_row(row: &str) -> (String, Option<String>, bool, Option<String>) {
    let mut tex = row.to_string();
    let mut label = None;
    let mut nonumber = false;
    let mut tag = None;
    while let Some(pos) = tex.find("\\label{") {
        let open = pos + "\\label".len();
        match matching_brace(&tex, open) {
            Some(close) => {
                let name = tex[open + 1..close].trim().to_string();
                if label.is_none() {
                    label = Some(name);
                }
                tex.replace_range(pos..=close, "");
            }
            None => break,
        }
    }
    for word in ["\\nonumber", "\\notag"] {
        if tex.contains(word) {
            nonumber = true;
            tex = tex.replace(word, "");
        }
    }
    if let Some(pos) = tex.find("\\tag") {
        let mut open = pos + "\\tag".len();
        if tex.as_bytes().get(open) == Some(&b'*') {
            open += 1;
        }
        if let Some(close) = matching_brace(&tex, open) {
            tag = Some(tex[open + 1..close].trim().to_string());
        }
    }
    (tex.trim().to_string(), label, nonumber, tag)
}

/// Index of the `}` matching the `{` at `open`, when `tex[open]` is `{`.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `key=value, key2=value2` option lists.
fn parse_options(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut parts = Vec::new();
    for (i, c) in raw.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&raw[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&raw[start..]);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((k, v)) => out.push((k.trim().to_string(), v.trim().to_string())),
            None => out.push((part.to_string(), String::new())),
        }
    }
    out
}

const IMAGE_EXTENSIONS: [&str; 5] = ["pdf", "png", "jpg", "jpeg", "eps"];

/// The files graphicx tries for a path: the path itself when it already has an image extension,
/// otherwise the path with each known extension, in graphicx's order.
pub(super) fn image_candidates(path: &str) -> Vec<String> {
    let base = path.rsplit('/').next().unwrap_or(path);
    let ext = base.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext {
        Some(e) if IMAGE_EXTENSIONS.contains(&e.as_str()) => vec![path.to_string()],
        _ => IMAGE_EXTENSIONS
            .iter()
            .map(|e| format!("{path}.{e}"))
            .chain(std::iter::once(path.to_string()))
            .collect(),
    }
}

/// Commands whose arguments are dropped: (optional arguments, mandatory arguments).
fn drop_spec(name: &str) -> Option<(usize, usize)> {
    Some(match name {
        "usepackage" | "RequirePackage" | "documentclass" | "captionsetup" => (1, 1),
        "usetikzlibrary" | "pgfplotsset" | "graphicspath" | "geometry" | "hypersetup"
        | "pagestyle" | "thispagestyle" | "setstretch" | "linespread" | "includeonly"
        | "newlength" | "lstset" | "tcbuselibrary" | "setcitestyle" | "sisetup"
        | "theoremstyle" | "hyphenation" | "color" | "hspace" | "vspace" | "cline"
        | "fontfamily" | "fontseries" | "fontshape" => (0, 1),
        "setlength" | "addtolength" | "setcounter" | "addtocounter" | "fontsize" => (0, 2),
        "definecolor" => (0, 3),
        "colorlet" | "rule" | "pdfbookmark" => (1, 2),
        "phantom" | "vphantom" | "hphantom" => (0, 1),
        "numberwithin" => (1, 2),
        _ => return None,
    })
}
