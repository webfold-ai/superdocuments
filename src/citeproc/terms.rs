//! The English (en-US) terms a CSL style asks for, written out here so no locale file has to be
//! read at run time. Styles add or override terms in their own `<locale>` blocks; those are read
//! by `Style` and take precedence.
//!
//! Each line is `name|form|singular|plural` (`form` empty for `long`); a term with no plural has
//! the singular in both places.

pub(crate) const EN_US: &str = "\
accessed||accessed|
and||and|
and others||and others|
and|symbol|&|
anonymous||anonymous|
anonymous|short|anon.|
at||at|
available at||available at|
by||by|
circa||circa|
circa|short|c.|
cited||cited|
et-al||et al.|
forthcoming||forthcoming|
from||from|
ibid||ibid.|
in||in|
in press||in press|
internet||internet|
no date||no date|
no date|short|n.d.|
no-place||no place|
no-place|short|n.p.|
no-publisher||no publisher|
no-publisher|short|n.p.|
on||on|
online||online|
presented at||presented at the|
retrieved||retrieved|
reference||reference|references
reference|short|ref.|refs.
preprint||preprint|
article||preprint|
article-journal||journal article|
article-magazine||magazine article|
article-newspaper||newspaper article|
book||book|books
chapter||chapter|chapters
collection||archival collection|
dataset||dataset|
document||document|
entry||entry|
entry-dictionary||dictionary entry|
entry-encyclopedia||encyclopedia entry|
manuscript||manuscript|
manuscript|short|MS|MSS
pamphlet||pamphlet|
paper-conference||conference paper|
patent||patent|
report||report|
report|short|rep.|
review||review|
review-book||book review|
software||software|
speech||presentation|
thesis||thesis|
thesis|short|thes.|
webpage||webpage|
ad||AD|
bc||BC|
open-quote||\u{201c}|
close-quote||\u{201d}|
open-inner-quote||\u{2018}|
close-inner-quote||\u{2019}|
page-range-delimiter||\u{2013}|
colon||:|
comma||,|
semicolon||;|
ordinal||th|
ordinal-01||st|
ordinal-02||nd|
ordinal-03||rd|
ordinal-11||th|
ordinal-12||th|
ordinal-13||th|
long-ordinal-01||first|
long-ordinal-02||second|
long-ordinal-03||third|
long-ordinal-04||fourth|
long-ordinal-05||fifth|
long-ordinal-06||sixth|
long-ordinal-07||seventh|
long-ordinal-08||eighth|
long-ordinal-09||ninth|
long-ordinal-10||tenth|
act||act|acts
appendix||appendix|appendices
appendix|short|app.|apps.
article-locator||article|articles
article-locator|short|art.|arts.
canon||canon|canons
column||column|columns
column|short|col.|cols.
elocation||location|locations
elocation|short|loc.|locs.
equation||equation|equations
equation|short|eq.|eqq.
figure||figure|figures
figure|short|fig.|figs.
folio||folio|folios
folio|short|fol.|fols.
issue||issue|issues
issue|short|no.|nos.
line||line|lines
line|short|l.|ll.
note||note|notes
note|short|n.|nn.
opus||opus|opera
opus|short|op.|opp.
page||page|pages
page|short|p.|pp.
paragraph||paragraph|paragraphs
paragraph|short|para.|paras.
paragraph|symbol|\u{b6}|\u{b6}\u{b6}
part||part|parts
part|short|pt.|pts.
rule||rule|rules
scene||scene|scenes
section||section|sections
section|short|sec.|secs.
section|symbol|\u{a7}|\u{a7}\u{a7}
sub-verbo||sub verbo|sub verbis
sub-verbo|short|s.v.|s.vv.
supplement||supplement|supplements
supplement|short|supp.|supps.
table||table|tables
table|short|tbl.|tbls.
title-locator||title|titles
verse||verse|verses
verse|short|v.|vv.
volume||volume|volumes
volume|short|vol.|vols.
chapter|short|chap.|chaps.
chapter|symbol|c.|cc.
chapter-number||chapter|chapters
citation-number||citation|citations
collection-number||number|numbers
collection-number|short|no.|nos.
edition||edition|editions
edition|short|ed.|eds.
number||number|numbers
number|short|no.|nos.
number-of-pages||page|pages
number-of-pages|short|p.|pp.
number-of-volumes||volume|volumes
number-of-volumes|short|vol.|vols.
page-first||page|pages
printing||printing|printings
printing|short|ptg.|ptgs.
version||version|
version|short|v.|
chair||chair|chairs
collection-editor||editor|editors
collection-editor|short|ed.|eds.
compiler||compiler|compilers
compiler|short|comp.|comps.
contributor||contributor|contributors
curator||curator|curators
director||director|directors
director|short|dir.|dirs.
editor||editor|editors
editor|short|ed.|eds.
editor|verb|edited by|
editor|verb-short|ed. by|
editorial-director||editor|editors
editorial-director|short|ed.|eds.
editor-translator||editor & translator|editors & translators
editor-translator|short|ed. & trans.|eds. & trans.
editor-translator|verb|edited & translated by|
editortranslator||editor & translator|editors & translators
editortranslator|short|ed. & trans.|eds. & trans.
editortranslator|verb|edited & translated by|
illustrator||illustrator|illustrators
illustrator|short|ill.|ills.
narrator||narrator|narrators
organizer||organizer|organizers
performer||performer|performers
producer||producer|producers
producer|short|prod.|prods.
script-writer||writer|writers
translator||translator|translators
translator|short|trans.|
translator|verb|translated by|
translator|verb-short|trans. by|
collection-editor|verb|edited by|
compiler|verb|compiled by|
container-author|verb|by|
director|verb|directed by|
interviewer|verb|interview by|
recipient|verb|to|
reviewed-author|verb|by|
month-01||January|
month-02||February|
month-03||March|
month-04||April|
month-05||May|
month-06||June|
month-07||July|
month-08||August|
month-09||September|
month-10||October|
month-11||November|
month-12||December|
month-01|short|Jan.|
month-02|short|Feb.|
month-03|short|Mar.|
month-04|short|Apr.|
month-05|short|May|
month-06|short|June|
month-07|short|July|
month-08|short|Aug.|
month-09|short|Sept.|
month-10|short|Oct.|
month-11|short|Nov.|
month-12|short|Dec.|
season-01||Spring|
season-02||Summer|
season-03||Autumn|
season-04||Winter|
";

/// The labels a locator can carry, in the abbreviations Pandoc's citation syntax accepts
/// (`p.`, `pp.`, `chap.`, `sec.`, `\u{a7}`...), mapped to the CSL locator name.
pub(crate) const LOCATOR_LABELS: &[(&str, &str)] = &[
    ("act", "act"),
    ("acts", "act"),
    ("appendix", "appendix"),
    ("app.", "appendix"),
    ("book", "book"),
    ("bk.", "book"),
    ("bks.", "book"),
    ("chapter", "chapter"),
    ("chapters", "chapter"),
    ("chap.", "chapter"),
    ("chaps.", "chapter"),
    ("column", "column"),
    ("col.", "column"),
    ("cols.", "column"),
    ("figure", "figure"),
    ("figures", "figure"),
    ("fig.", "figure"),
    ("figs.", "figure"),
    ("folio", "folio"),
    ("fol.", "folio"),
    ("fols.", "folio"),
    ("issue", "issue"),
    ("number", "number"),
    ("no.", "number"),
    ("nos.", "number"),
    ("line", "line"),
    ("lines", "line"),
    ("l.", "line"),
    ("ll.", "line"),
    ("note", "note"),
    ("notes", "note"),
    ("n.", "note"),
    ("nn.", "note"),
    ("opus", "opus"),
    ("op.", "opus"),
    ("opp.", "opus"),
    ("page", "page"),
    ("pages", "page"),
    ("p.", "page"),
    ("pp.", "page"),
    ("paragraph", "paragraph"),
    ("paragraphs", "paragraph"),
    ("para.", "paragraph"),
    ("paras.", "paragraph"),
    ("\u{b6}", "paragraph"),
    ("\u{b6}\u{b6}", "paragraph"),
    ("part", "part"),
    ("parts", "part"),
    ("pt.", "part"),
    ("pts.", "part"),
    ("section", "section"),
    ("sections", "section"),
    ("sec.", "section"),
    ("secs.", "section"),
    ("\u{a7}", "section"),
    ("\u{a7}\u{a7}", "section"),
    ("sub verbo", "sub-verbo"),
    ("s.v.", "sub-verbo"),
    ("s.vv.", "sub-verbo"),
    ("verse", "verse"),
    ("verses", "verse"),
    ("v.", "verse"),
    ("vv.", "verse"),
    ("volume", "volume"),
    ("volumes", "volume"),
    ("vol.", "volume"),
    ("vols.", "volume"),
];
