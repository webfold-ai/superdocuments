#!/usr/bin/env python3
"""Seeded generators for the non-LaTeX corpus: Markdown seeds, hand-written HTML and RST,
BibTeX/BibLaTeX files, and raw OOXML (DOCX) built with the standard library only.

All text is written for this repository (Apache-2.0).
"""
import io
import random
import zipfile
from xml.sax.saxutils import escape

from gen_latex import WORDS, NAMES, KEYS, sent, para


# ---- Markdown seeds ----------------------------------------------------------------------

def m_list(rng):
    return "\n".join("- " + sent(rng, 5) for _ in range(rng.randint(2, 5)))


def m_olist(rng):
    return "\n".join("%d. %s" % (i + 1, sent(rng, 5)) for i in range(rng.randint(2, 5)))


def m_nested(rng):
    return "- %s\n    - %s\n    - %s\n- %s" % (sent(rng, 4), sent(rng, 3), sent(rng, 3), sent(rng, 4))


def m_table(rng):
    cols = rng.randint(2, 4)
    al = ["---", ":---:", "---:", ":---"]
    hdr = "| " + " | ".join(rng.choice(WORDS).capitalize() for _ in range(cols)) + " |"
    sep = "|" + "|".join(rng.choice(al) for _ in range(cols)) + "|"
    rows = ["| " + " | ".join(str(round(rng.uniform(1, 99), 1)) for _ in range(cols)) + " |" for _ in range(rng.randint(2, 5))]
    return "\n".join([hdr, sep] + rows)


def m_code(rng):
    lang = rng.choice(["python", "rust", "", "sh"])
    return "```%s\n%s = %d\nprint(%s)\n```" % (lang, rng.choice(WORDS), rng.randint(1, 99), rng.choice(WORDS))


def m_math(rng):
    return rng.choice(["$$\n\\int_0^1 x^2\\,dx = \\frac13\n$$", "Inline math $a^2+b^2=c^2$ and $\\sum_i x_i$ together.",
                       "$$E = mc^2$$"])


def m_footnote(rng):
    n = rng.randint(1, 99)
    return "%s[^n%d]\n\n[^n%d]: %s" % (sent(rng, 7), n, n, sent(rng, 5))


def m_links(rng):
    return "See [%s](https://example.org/%s) and <https://example.org/x> and ![%s](img/%s.png)." % (
        rng.choice(WORDS), rng.choice(WORDS), rng.choice(WORDS), rng.choice(WORDS))


def m_quote(rng):
    return "> " + sent(rng, 10) + "\n>\n> " + sent(rng, 6)


def m_emph(rng):
    return "Some *%s* and **%s** and `%s` and ~~%s~~ text, plus H~2~O and x^2^." % tuple(rng.choice(WORDS) for _ in range(4))


def m_hr(rng):
    return "---"


def m_deflist(rng):
    return "%s\n:   %s\n\n%s\n:   %s" % (rng.choice(WORDS), sent(rng, 4), rng.choice(WORDS), sent(rng, 4))


def m_task(rng):
    return "- [x] %s\n- [ ] %s" % (sent(rng, 3), sent(rng, 3))


def m_html(rng):
    return "<div class=\"note\">%s</div>" % sent(rng, 4)


MD_BLOCKS = [m_list, m_olist, m_nested, m_table, m_code, m_math, m_footnote, m_links, m_quote, m_emph, m_hr, m_deflist,
             m_task]
# Blocks that every output writer (docx, odt, epub, rst) can carry.
MD_PORTABLE = [m_list, m_olist, m_nested, m_table, m_code, m_math, m_footnote, m_links, m_quote, m_emph, m_hr]


def md_seed(rng, i, portable=True):
    blocks = MD_PORTABLE if portable else MD_BLOCKS
    out = ["---\ntitle: %s\nauthor: %s\n---\n" % (sent(rng, 3).rstrip("."), rng.choice(NAMES))]
    for s in range(rng.randint(2, 4)):
        out.append("# " + sent(rng, 3).rstrip("."))
        out.append(para(rng, 1))
        for _ in range(rng.randint(2, 4)):
            out.append(rng.choice(blocks)(rng))
            out.append(para(rng, 1))
        if rng.random() < 0.6:
            out.append("## " + sent(rng, 3).rstrip("."))
            out.append(para(rng, 1))
            out.append(rng.choice(blocks)(rng))
        if rng.random() < 0.3:
            out.append("### " + sent(rng, 3).rstrip("."))
            out.append(para(rng, 1))
    return "\n\n".join(out) + "\n"


# ---- hand-written HTML -------------------------------------------------------------------

HTML_DOCS = {
"html-article": """<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><title>Field notes</title></head><body>
<header><h1>Field notes on caching</h1><p class="byline">By Ruth Ng</p></header>
<nav><ul><li><a href="/">Home</a></li><li><a href="/about">About</a></li></ul></nav>
<main><article>
<h2>Why cache</h2>
<p>A <em>cache</em> trades <strong>memory</strong> for latency. See the <a href="https://example.org/cache">overview</a>.</p>
<ul><li>Fast reads</li><li>Stale data<ul><li>Expiry</li><li>Invalidation</li></ul></li></ul>
<h2>Numbers</h2>
<table><thead><tr><th>Tier</th><th>Latency</th></tr></thead><tbody><tr><td>L1</td><td>1 ns</td></tr><tr><td>RAM</td><td>100 ns</td></tr></tbody></table>
<pre><code>get(key) -&gt; value</code></pre>
<blockquote><p>There are only two hard things in computer science.</p></blockquote>
<p>Footnote-like text<sup>1</sup>.</p>
</article></main><footer><p>Copyright 2026</p></footer></body></html>""",
"html-recipe": """<html><body><h1>Flatbread</h1><h2>Ingredients</h2><ol><li>2 cups flour</li><li>1 cup water</li><li>1 tsp salt</li></ol>
<h2>Method</h2><p>Mix. <br>Rest for <b>30 minutes</b>. Roll thin and cook on a hot pan.</p>
<img src="bread.jpg" alt="A flatbread on a plate"><hr><p><i>Serves four.</i></p></body></html>""",
"html-defs": """<html><body><h1>Glossary</h1><dl><dt>Latency</dt><dd>Time to first byte.</dd><dt>Throughput</dt><dd>Bytes per second.</dd></dl>
<h3>Code</h3><p>Use <code>cargo test</code> to run tests.</p><pre>line one
line two
  indented</pre></body></html>""",
"html-table-span": """<html><body><h1>Report</h1><table border="1"><tr><th rowspan="2">Region</th><th colspan="2">Sales</th></tr>
<tr><th>Q1</th><th>Q2</th></tr><tr><td>North</td><td>10</td><td>12</td></tr><tr><td>South</td><td>7</td><td>9</td></tr></table>
<p>Totals are in thousands.</p></body></html>""",
"html-math": """<html><body><h1>Math</h1><p>The area is <span class="math inline">\\(\\pi r^2\\)</span>.</p>
<p>Display: <span class="math display">\\[\\int_0^1 x\\,dx = \\tfrac12\\]</span></p>
<math xmlns="http://www.w3.org/1998/Math/MathML"><mfrac><mn>1</mn><mn>2</mn></mfrac></math></body></html>""",
"html-nested": """<html><body><div><h1>Main</h1><div><p>One</p><div><p>Two <a href="#x">link</a></p></div></div></div>
<section><h2>Sub</h2><p>Three</p></section><script>var x = 1;</script><style>p{color:red}</style></body></html>""",
"html-entities": """<html><body><h1>Symbols &amp; entities</h1><p>&copy; 2026 &mdash; caf&eacute; &lt;tag&gt; &#8364;5 &nbsp;spaced.</p>
<p>A line<br/>break and a <abbr title="HyperText">HTML</abbr> term.</p></body></html>""",
"html-form-page": """<html><body><h1>Contact</h1><form><label>Name <input type="text"></label><button>Send</button></form>
<p>Email us at <a href="mailto:hi@example.org">hi@example.org</a>.</p></body></html>""",
"html-figure": """<html><body><h1>Figures</h1><figure><img src="a.png" alt="Chart"><figcaption>Figure 1: A chart.</figcaption></figure>
<p>Text after.</p><ul><li><p>Loose item one</p></li><li><p>Loose item two</p></li></ul></body></html>""",
"html-headings": """<html><body><h1>A</h1><p>a</p><h2>B</h2><p>b</p><h3>C</h3><p>c</p><h4>D</h4><p>d</p><h5>E</h5><p>e</p><h6>F</h6><p>f</p></body></html>""",
"html-ordered": """<html><body><ol start="3"><li>three</li><li>four</li></ol><ol type="a"><li>alpha</li><li>beta</li></ol>
<ul><li>x</li><li>y</li></ul></body></html>""",
"html-inline": """<html><body><p>Mix of <b>bold</b>, <strong>strong</strong>, <i>ital</i>, <em>em</em>, <u>under</u>, <s>strike</s>,
<sub>sub</sub>, <sup>sup</sup>, <code>code</code>, <kbd>kbd</kbd>, <q>quoted</q> and <mark>marked</mark> text.</p></body></html>""",
}


# ---- hand-written RST --------------------------------------------------------------------

RST_DOCS = {
"rst-basic": """Project Title
=============

A short paragraph with *emphasis*, **strong** text and ``inline literal``.

Section One
-----------

* bullet one
* bullet two

  * nested a
  * nested b

1. first
2. second
3. third

Subsection
~~~~~~~~~~

Plain text with a `link <https://example.org>`_ and an anonymous__ one.

__ https://example.org/anon
""",
"rst-code": """Code samples
============

Run this::

    cargo test
    cargo build

.. code-block:: python

    def f(x):
        return x + 1

.. code:: rust

    fn main() {}
""",
"rst-table-grid": """Tables
======

+-------+-------+
| A     | B     |
+=======+=======+
| 1     | 2     |
+-------+-------+
| 3     | 4     |
+-------+-------+

=====  =====
Name   Value
=====  =====
x      10
y      20
=====  =====
""",
"rst-admonitions": """Notes
=====

.. note::

   This is a note with *emphasis*.

.. warning::

   Careful here.

.. tip:: A one-line tip.

.. image:: logo.png
   :alt: Logo
   :width: 100
""",
"rst-fields": """:Author: Ruth Ng
:Date: 2026-11-01

Title
=====

:param x: the first value
:returns: the sum

Term
    Definition of the term.

Another
    Second definition.
""",
"rst-footnotes": """Footnotes
=========

A claim [1]_ and another [#]_ with a citation [CIT2002]_.

.. [1] First footnote.
.. [#] Auto-numbered footnote.
.. [CIT2002] A citation reference.
""",
"rst-math": """Math
====

Inline :math:`a^2 + b^2 = c^2` and a block:

.. math::

   \\int_0^1 x^2 dx = \\frac{1}{3}
""",
"rst-quote-line": """Quotes
======

    A block quote with indentation.

| line one
| line two
| line three

----

Final paragraph.
""",
"rst-links": """Links
=====

.. _target:

Section with target
-------------------

See target_ and `Example`_ and :ref:`target`.

.. _Example: https://example.org
""",
"rst-toc": """Document
========

.. contents::

First
-----

Text.

Second
------

More text.

.. toctree::
   :maxdepth: 2
""",
"rst-substitution": """Subs
====

The |name| is here.

.. |name| replace:: Frobnitz

.. raw:: html

   <b>raw html</b>

.. comment: this is a comment

Done.
""",
"rst-roles": """Roles
=====

Use :emphasis:`this`, :strong:`that`, :code:`x = 1`, :sub:`sub` and :sup:`sup`.

.. versionadded:: 2.0
   New thing.

.. seealso:: Other docs.
""",
}


# ---- BibTeX / BibLaTeX -------------------------------------------------------------------

TITLES = ["Notes on the Analytical Engine", "The Education of a Computer", "The Art of Computer Programming",
          "Data Abstraction and Hierarchy", "Go To Statement Considered Harmful", "On Computable Numbers",
          "Time, Clocks, and the Ordering of Events", "An Axiomatic Basis for Computer Programming",
          "The Apollo Guidance Computer", "Can Programming Be Liberated from the von Neumann Style"]
JOURNALS = ["Proceedings of the London Mathematical Society", "Communications of the ACM", "Journal of the ACM",
            "ACM Computing Surveys", "IEEE Transactions on Software Engineering"]
PUBL = ["Addison-Wesley", "Springer", "MIT Press", "Cambridge University Press"]
BIB_AUTHORS = [("Lovelace", "Ada"), ("Hopper", "Grace"), ("Knuth", "Donald E."), ("Liskov", "Barbara"),
               ("Dijkstra", "Edsger W."), ("Turing", "Alan M."), ("Lamport", "Leslie"), ("Hoare", "C. A. R."),
               ("Hamilton", "Margaret"), ("Backus", "John"), ("van Rossum", "Guido"), ("de la Cruz", "Maria"),
               ("Smith", "John Jr."), ("O'Brien", "Flann"), ("M\\\"uller", "Hans"), ("Garc\\'ia", "Luis")]


def bib_entry(rng, key, kind, biblatex=False):
    n = rng.randint(1, 3)
    auths = rng.sample(BIB_AUTHORS, n)
    sep = " and "
    author = sep.join("%s, %s" % a for a in auths)
    if rng.random() < 0.3:
        author = sep.join("%s %s" % (a[1], a[0]) for a in auths)
    year = rng.randint(1936, 2024)
    f = [("author", author), ("title", rng.choice(TITLES)), ("year", str(year))]
    if kind == "article":
        f += [("journal" if not biblatex else "journaltitle", rng.choice(JOURNALS)), ("volume", str(rng.randint(1, 60))),
              ("number", str(rng.randint(1, 12))), ("pages", "%d--%d" % (rng.randint(1, 300), rng.randint(301, 400)))]
        if rng.random() < 0.5:
            f.append(("doi", "10.%d/%s.%d" % (rng.randint(1000, 9999), rng.choice(WORDS), rng.randint(10, 99))))
    elif kind in ("book", "inbook"):
        f += [("publisher", rng.choice(PUBL)), ("address" if not biblatex else "location", rng.choice(["Boston", "New York", "London"]))]
        if rng.random() < 0.4:
            f.append(("edition", rng.choice(["2", "Second", "3"])))
    elif kind in ("inproceedings", "conference"):
        f += [("booktitle", "Proceedings of the %s Symposium" % rng.choice(WORDS).capitalize()),
              ("pages", "%d-%d" % (rng.randint(1, 100), rng.randint(101, 200)))]
    elif kind == "phdthesis":
        f += [("school", "University of %s" % rng.choice(["Cambridge", "Oxford", "Zurich"]))]
    elif kind == "techreport":
        f += [("institution", "Lab %s" % rng.choice(WORDS).capitalize()), ("number", "TR-%d" % rng.randint(1, 99))]
    elif kind == "misc":
        f += [("howpublished", "\\url{https://example.org/%s}" % rng.choice(WORDS)), ("note", "Accessed 2024")]
    if rng.random() < 0.25:
        f.append(("url", "https://example.org/%s" % rng.choice(WORDS)))
    if rng.random() < 0.2:
        f.append(("abstract", sent(rng, 12)))
    body = ",\n".join("  %s = {%s}" % (k, v) for k, v in f)
    return "@%s{%s,\n%s\n}\n" % (kind, key, body)


def bib_file(rng, n=None, biblatex=False):
    n = n or rng.randint(4, 10)
    kinds = ["article", "book", "inproceedings", "phdthesis", "techreport", "misc"]
    out = []
    if rng.random() < 0.3:
        out.append("@string{acm = {Association for Computing Machinery}}\n")
    for i in range(n):
        key = (KEYS[i] if i < len(KEYS) else "ref%d" % i)
        out.append(bib_entry(rng, key, rng.choice(kinds), biblatex))
    return "\n".join(out)


# ---- raw DOCX ----------------------------------------------------------------------------

W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'


def _run(t, rpr=""):
    return '<w:r>%s<w:t xml:space="preserve">%s</w:t></w:r>' % ("<w:rPr>%s</w:rPr>" % rpr if rpr else "", escape(t))


def _p(runs, style=None, num=None):
    ppr = ""
    if style:
        ppr += '<w:pStyle w:val="%s"/>' % style
    if num:
        ppr += '<w:numPr><w:ilvl w:val="%d"/><w:numId w:val="%d"/></w:numPr>' % num
    return "<w:p>%s%s</w:p>" % ("<w:pPr>%s</w:pPr>" % ppr if ppr else "", runs)


def docx_zip(body, footnotes=None, comments=None, extra_rels="", numbering=True, endnotes=None):
    ct = ['<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">',
          '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>',
          '<Default Extension="xml" ContentType="application/xml"/>',
          '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>',
          '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>']
    rels = ['<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>']
    if numbering:
        ct.append('<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>')
        rels.append('<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>')
    if footnotes:
        ct.append('<Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>')
        rels.append('<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>')
    if endnotes:
        ct.append('<Override PartName="/word/endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/>')
        rels.append('<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes" Target="endnotes.xml"/>')
    if comments:
        ct.append('<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>')
        rels.append('<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>')
    rels.append('<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org/doc" TargetMode="External"/>')
    ct.append("</Types>")
    styles = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles %s>' % W +
              "".join('<w:style w:type="paragraph" w:styleId="Heading%d"><w:name w:val="heading %d"/></w:style>' % (i, i) for i in range(1, 7)) +
              '<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style>'
              '<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/></w:style>'
              '<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/></w:style></w:styles>')
    numxml = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering %s>'
              '<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="-"/></w:lvl>'
              '<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/></w:lvl></w:abstractNum>'
              '<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%%1."/></w:lvl></w:abstractNum>'
              '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>' % W)
    doc = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document %s><w:body>%s</w:body></w:document>' % (W, body)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", "".join(ct))
        z.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
        z.writestr("word/_rels/document.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">%s</Relationships>' % "".join(rels))
        z.writestr("word/document.xml", doc)
        z.writestr("word/styles.xml", styles)
        if numbering:
            z.writestr("word/numbering.xml", numxml)
        if footnotes:
            z.writestr("word/footnotes.xml", '<?xml version="1.0" encoding="UTF-8"?><w:footnotes %s>%s</w:footnotes>' % (W, footnotes))
        if endnotes:
            z.writestr("word/endnotes.xml", '<?xml version="1.0" encoding="UTF-8"?><w:endnotes %s>%s</w:endnotes>' % (W, endnotes))
        if comments:
            z.writestr("word/comments.xml", '<?xml version="1.0" encoding="UTF-8"?><w:comments %s>%s</w:comments>' % (W, comments))
    return buf.getvalue()


def _tbl(rows, merge_first=False):
    out = "<w:tbl><w:tblPr/><w:tblGrid>%s</w:tblGrid>" % "".join('<w:gridCol w:w="2000"/>' for _ in rows[0])
    for ri, row in enumerate(rows):
        out += "<w:tr>"
        for ci, cell in enumerate(row):
            tcpr = ""
            if merge_first and ri == 0 and ci == 0:
                tcpr = '<w:tcPr><w:gridSpan w:val="2"/></w:tcPr>'
            if merge_first and ri == 0 and ci == 1:
                continue
            out += "<w:tc>%s%s</w:tc>" % (tcpr, _p(_run(cell, "<w:b/>" if ri == 0 else "")))
        out += "</w:tr>"
    return out + "</w:tbl>"


def raw_docx_docs(seed=5):
    """name -> (bytes, producer note). Each exercises features Word writes that Pandoc's own
    DOCX writer does not (or writes differently)."""
    rng = random.Random(seed)
    docs = {}

    def S(n=7):
        return sent(rng, n)

    # headings, lists, hyperlinks, formatting
    body = (_p(_run("Quarterly Review"), "Title") + _p(_run("Overview"), "Heading1") + _p(_run(S()) + _run(" bold", "<w:b/>") +
            _run(" italic", "<w:i/>") + _run(" under", '<w:u w:val="single"/>') + _run(" strike", "<w:strike/>") + _run(" sup", '<w:vertAlign w:val="superscript"/>')) +
            _p(_run("Details"), "Heading2") + _p(_run(S()), None) + _p(_run("item one"), "ListParagraph", (0, 1)) +
            _p(_run("item two"), "ListParagraph", (0, 1)) + _p(_run("sub item"), "ListParagraph", (1, 1)) + _p(_run("step one"), "ListParagraph", (0, 2)) +
            _p(_run("step two"), "ListParagraph", (0, 2)) + _p('<w:hyperlink r:id="rId5">%s</w:hyperlink>' % _run("a hyperlink", "<w:u w:val=\"single\"/>")) +
            _p(_run("Deep"), "Heading3") + _p(_run(S())))
    docs["raw-structure"] = (docx_zip(body), "python zipfile, OOXML written for this corpus")
    # tables incl. merged header
    body = (_p(_run("Tables"), "Heading1") + _tbl([["Region", "Q1", "Q2"], ["North", "10", "12"], ["South", "7", "9"]]) + _p(_run(S())) +
            _tbl([["Merged header", "X"], ["a", "b", "c"]], merge_first=True) + _p(_run(S(5))))
    docs["raw-tables"] = (docx_zip(body, numbering=False), "python zipfile")
    # footnotes and endnotes
    fn = ('<w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>'
          '<w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>'
          '<w:footnote w:id="1"><w:p><w:r><w:footnoteRef/></w:r>%s</w:p></w:footnote>'
          '<w:footnote w:id="2"><w:p><w:r><w:footnoteRef/></w:r>%s</w:p></w:footnote>') % (_run(" " + S(6)), _run(" " + S(5)))
    body = (_p(_run("Notes"), "Heading1") + _p(_run(S()) + '<w:r><w:footnoteReference w:id="1"/></w:r>' + _run(" " + S(5)) + '<w:r><w:footnoteReference w:id="2"/></w:r>'))
    docs["raw-footnotes"] = (docx_zip(body, footnotes=fn, numbering=False), "python zipfile")
    en = ('<w:endnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:endnote>'
          '<w:endnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:endnote>'
          '<w:endnote w:id="1"><w:p><w:r><w:endnoteRef/></w:r>%s</w:p></w:endnote>') % _run(" " + S(6))
    body = _p(_run("Endnote document"), "Heading1") + _p(_run(S()) + '<w:r><w:endnoteReference w:id="1"/></w:r>')
    docs["raw-endnotes"] = (docx_zip(body, endnotes=en, numbering=False), "python zipfile")
    # tracked changes
    ins = '<w:ins w:id="10" w:author="Rev" w:date="2026-01-01T00:00:00Z">%s</w:ins>' % _run(" inserted words")
    dele = '<w:del w:id="11" w:author="Rev" w:date="2026-01-01T00:00:00Z"><w:r><w:delText xml:space="preserve"> removed words</w:delText></w:r></w:del>'
    body = (_p(_run("Tracked"), "Heading1") + _p(_run(S(6)) + ins + _run(" kept.") + dele) +
            _p('<w:del w:id="12" w:author="Rev" w:date="2026-01-01T00:00:00Z"><w:r><w:delText>A fully deleted paragraph of text.</w:delText></w:r></w:del>') +
            _p(_run(S(5))))
    docs["raw-tracked"] = (docx_zip(body, numbering=False), "python zipfile")
    docs["raw-tracked2"] = (docx_zip(
        _p(_run("Edits"), "Heading1") + _p(_run("Start") + ins + ins.replace('w:id="10"', 'w:id="20"') + dele + _run(" end.")), numbering=False), "python zipfile")
    # comments
    cm = ('<w:comment w:id="0" w:author="Rev" w:date="2026-01-01T00:00:00Z" w:initials="R"><w:p><w:r><w:t>Please check this claim.</w:t></w:r></w:p></w:comment>'
          '<w:comment w:id="1" w:author="Ed" w:date="2026-01-02T00:00:00Z" w:initials="E"><w:p><w:r><w:t>Agreed, cite a source.</w:t></w:r></w:p></w:comment>')
    body = (_p(_run("Review") , "Heading1") + _p('<w:commentRangeStart w:id="0"/>' + _run(S(6)) + '<w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r>' +
            '<w:commentRangeStart w:id="1"/>' + _run(" " + S(5)) + '<w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r>'))
    docs["raw-comments"] = (docx_zip(body, comments=cm, numbering=False), "python zipfile")
    docs["raw-comments-tracked"] = (docx_zip(
        _p(_run("Both"), "Heading1") + _p('<w:commentRangeStart w:id="0"/>' + _run(S(5)) + ins + '<w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r>' + dele),
        comments=cm, numbering=False), "python zipfile")
    # quote, code-ish, line breaks, tabs
    body = (_p(_run("Misc"), "Heading1") + _p(_run(S(9)), "Quote") + _p(_run("line one") + "<w:r><w:br/></w:r>" + _run("line two") + "<w:r><w:tab/></w:r>" + _run("tabbed")) +
            _p(_run("mono", '<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/>')) + _p(_run(S(5))))
    docs["raw-misc"] = (docx_zip(body, numbering=False), "python zipfile")
    # many headings
    body = "".join(_p(_run("Heading %d.%d" % (i, j)), "Heading%d" % j) + _p(_run(S())) for i in range(1, 3) for j in range(1, 5))
    docs["raw-headings"] = (docx_zip(body, numbering=False), "python zipfile")
    # inline math (OMML)
    omml = ('<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num>'
            '<m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>')
    body = _p(_run("Equation") , "Heading1") + '<w:p>%s%s%s</w:p>' % (_run("Ratio "), omml, _run(" is shown."))
    docs["raw-math"] = (docx_zip(body, numbering=False), "python zipfile")
    return docs
