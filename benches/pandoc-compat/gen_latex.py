#!/usr/bin/env python3
"""Deterministic LaTeX corpus generator for the Pandoc-compatibility score.

Everything here is written for this repository (Apache-2.0); nothing is copied from a
third-party document. A document is a seeded random composition of snippets from the
library below, so the corpus is reproducible: `gen_corpus.py` with the same seed writes the
same bytes. Generated documents are labelled "generated" in the manifest; the whole-paper
documents in `papers()` are hand-written and labelled "hand-written".
"""
import random

WORDS = (
    "adaptive agent algorithm analysis approach array baseline batch benchmark block bound "
    "cache channel cluster compiler complexity convergence corpus cost dataset decoder "
    "density distribution domain encoder entropy estimate experiment feature filter "
    "framework function gradient graph heuristic index kernel latency layer lattice "
    "learning matrix memory metric model network node objective operator optimizer "
    "parser pipeline policy prior protocol query queue ranking regression retrieval "
    "scheduler schema segment sequence signal solver sparse stream structure throughput "
    "token trace transform tree vector vocabulary window workload"
).split()
NAMES = ["Ada Lovelace", "Grace Hopper", "Donald Knuth", "Barbara Liskov", "Edsger Dijkstra",
         "Alan Turing", "Leslie Lamport", "Tony Hoare", "Margaret Hamilton", "John Backus"]
KEYS = ["lovelace1843", "hopper1952", "knuth1984", "liskov1987", "dijkstra1968", "turing1936",
        "lamport1978", "hoare1969", "hamilton1969", "backus1978"]


def sent(rng, n=None):
    n = n or rng.randint(7, 16)
    w = [rng.choice(WORDS) for _ in range(n)]
    w[0] = w[0].capitalize()
    return " ".join(w) + "."


def para(rng, k=None):
    return " ".join(sent(rng) for _ in range(k or rng.randint(2, 4)))


def inline_rich(rng):
    opts = [
        lambda: r"\emph{%s}" % rng.choice(WORDS),
        lambda: r"\textbf{%s}" % rng.choice(WORDS),
        lambda: r"\texttt{%s}" % rng.choice(WORDS),
        lambda: r"\textit{%s}" % rng.choice(WORDS),
        lambda: r"\textsc{%s}" % rng.choice(WORDS),
        lambda: r"$%s$" % rng.choice([r"x_i", r"\alpha+\beta", r"O(n\log n)", r"\sum_{i=1}^n i", r"\frac{a}{b}"]),
        lambda: r"\url{https://example.org/%s}" % rng.choice(WORDS),
        lambda: r"\href{https://example.org/%s}{%s}" % (rng.choice(WORDS), rng.choice(WORDS)),
        lambda: r"\footnote{%s}" % sent(rng, 6),
        lambda: r"\textsuperscript{%d}" % rng.randint(1, 9),
        lambda: r"``%s''" % rng.choice(WORDS),
        lambda: r"%d\%% of the %s" % (rng.randint(2, 98), rng.choice(WORDS)),
        lambda: r"R\&D",
        lambda: r"well-known --- and %s" % rng.choice(WORDS),
        lambda: r"pages 3--7",
    ]
    return rng.choice(opts)()


def rich_para(rng):
    parts = [sent(rng, 6), inline_rich(rng), sent(rng, 5), inline_rich(rng), sent(rng, 7)]
    return " ".join(parts)


# ---- block snippets -------------------------------------------------------------------

def s_itemize(rng):
    items = "\n".join(r"  \item " + (rich_para(rng) if rng.random() < 0.3 else sent(rng, 6))
                      for _ in range(rng.randint(2, 5)))
    return "\\begin{itemize}\n%s\n\\end{itemize}" % items


def s_enumerate(rng):
    items = "\n".join(r"  \item " + sent(rng, 6) for _ in range(rng.randint(2, 5)))
    return "\\begin{enumerate}\n%s\n\\end{enumerate}" % items


def s_nested_list(rng):
    return ("\\begin{itemize}\n  \\item %s\n  \\begin{enumerate}\n    \\item %s\n    \\item %s\n"
            "  \\end{enumerate}\n  \\item %s\n\\end{itemize}") % (
        sent(rng, 5), sent(rng, 4), sent(rng, 4), sent(rng, 5))


def s_description(rng):
    items = "\n".join(r"  \item[%s] %s" % (rng.choice(WORDS), sent(rng, 5)) for _ in range(rng.randint(2, 4)))
    return "\\begin{description}\n%s\n\\end{description}" % items


def s_tabular(rng):
    cols = rng.randint(2, 4)
    spec = "".join(rng.choice("lcr") for _ in range(cols))
    rows = []
    hdr = " & ".join(rng.choice(WORDS).capitalize() for _ in range(cols))
    rows.append(hdr + r" \\ \hline")
    for _ in range(rng.randint(2, 5)):
        rows.append(" & ".join(str(round(rng.uniform(1, 99), 1)) if rng.random() < 0.7 else rng.choice(WORDS)
                               for _ in range(cols)) + r" \\")
    rule = "|" + "|".join(spec) + "|" if rng.random() < 0.4 else spec
    return "\\begin{tabular}{%s}\n\\hline\n%s\n\\hline\n\\end{tabular}" % (rule, "\n".join(rows))


def s_table_float(rng):
    cols = rng.randint(2, 4)
    spec = "l" + "c" * (cols - 1)
    hdr = " & ".join(rng.choice(WORDS).capitalize() for _ in range(cols))
    body = "\n".join(
        " & ".join([rng.choice(WORDS)] + [str(round(rng.uniform(1, 99), 1)) for _ in range(cols - 1)]) + r" \\"
        for _ in range(rng.randint(2, 5)))
    lab = "tab:%s%d" % (rng.choice(WORDS), rng.randint(1, 99))
    return ("\\begin{table}[h]\n\\centering\n\\caption{%s}\n\\label{%s}\n\\begin{tabular}{%s}\n\\toprule\n%s \\\\\n"
            "\\midrule\n%s\n\\bottomrule\n\\end{tabular}\n\\end{table}") % (sent(rng, 5).rstrip("."), lab, spec, hdr, body)


def s_multicol(rng):
    return ("\\begin{tabular}{lcc}\n\\hline\n & \\multicolumn{2}{c}{Scores} \\\\\n Model & A & B \\\\\n\\hline\n"
            "%s & %.1f & %.1f \\\\\n%s & %.1f & %.1f \\\\\n\\hline\n\\end{tabular}") % (
        rng.choice(WORDS), rng.uniform(1, 9), rng.uniform(1, 9), rng.choice(WORDS), rng.uniform(1, 9), rng.uniform(1, 9))


def s_figure(rng):
    lab = "fig:%s%d" % (rng.choice(WORDS), rng.randint(1, 99))
    return ("\\begin{figure}[ht]\n\\centering\n\\includegraphics[width=0.%d\\textwidth]{figs/%s}\n"
            "\\caption{%s}\n\\label{%s}\n\\end{figure}") % (rng.randint(4, 9), rng.choice(WORDS), sent(rng, 6).rstrip("."), lab)


def s_equation(rng):
    eqs = [r"E = mc^2", r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}", r"\nabla \cdot \mathbf{E} = \frac{\rho}{\varepsilon_0}",
           r"f(x) = \sum_{k=0}^{n} \binom{n}{k} x^k", r"\hat{y} = \mathrm{softmax}(Wx + b)",
           r"\mathcal{L}(\theta) = -\sum_i \log p_\theta(y_i \mid x_i)"]
    return "\\begin{equation}\n%s\n\\label{eq:e%d}\n\\end{equation}" % (rng.choice(eqs), rng.randint(1, 99))


def s_align(rng):
    return ("\\begin{align}\na &= b + c \\\\\nd &= e - f \\\\\n\\end{align}")


def s_display_bracket(rng):
    return "\\[\n  %s\n\\]" % rng.choice([r"\sum_{i=1}^{n} x_i^2", r"\lim_{n\to\infty} \frac{1}{n} = 0", r"a^2 + b^2 = c^2"])


def s_dollars(rng):
    return "$$ %s $$" % rng.choice([r"x^2 + y^2 = z^2", r"\frac{d}{dx} e^x = e^x"])


def s_theorem(rng):
    env = rng.choice(["theorem", "lemma", "definition", "proposition", "corollary", "remark"])
    opt = "[%s]" % rng.choice(WORDS).capitalize() if rng.random() < 0.4 else ""
    out = "\\begin{%s}%s\n%s\n\\end{%s}" % (env, opt, sent(rng, 10), env)
    if env in ("theorem", "lemma", "proposition") and rng.random() < 0.6:
        out += "\n\\begin{proof}\n%s\n\\end{proof}" % sent(rng, 9)
    return out


def s_verbatim(rng):
    return "\\begin{verbatim}\nfn %s(x: i32) -> i32 {\n    x + %d\n}\n\\end{verbatim}" % (rng.choice(WORDS), rng.randint(1, 9))


def s_lstlisting(rng):
    return "\\begin{lstlisting}[language=Python]\ndef %s(x):\n    return x * %d\n\\end{lstlisting}" % (rng.choice(WORDS), rng.randint(2, 9))


def s_quote(rng):
    return "\\begin{quote}\n%s\n\\end{quote}" % sent(rng, 12)


def s_footnotes(rng):
    return "%s\\footnote{%s} %s" % (sent(rng, 8), sent(rng, 6), sent(rng, 6))


def s_labels(rng):
    return "See Section~\\ref{sec:a} and Equation~\\eqref{eq:e1}. %s" % sent(rng, 6)


def s_cites(rng):
    k = rng.sample(KEYS, 3)
    forms = [r"\cite{%s}" % k[0], r"\cite{%s,%s}" % (k[0], k[1]), r"\citep{%s}" % k[0],
             r"\citet{%s}" % k[0], r"\citep[see][p.~12]{%s}" % k[1], r"\citeauthor{%s}" % k[2],
             r"\citeyear{%s}" % k[0], r"\parencite{%s}" % k[1], r"\textcite{%s}" % k[0],
             r"\autocite{%s}" % k[2], r"\cite[Chapter 2]{%s}" % k[1], r"\citep{%s,%s,%s}" % tuple(k)]
    return "%s %s. %s %s." % (sent(rng, 5).rstrip("."), rng.choice(forms), sent(rng, 4).rstrip("."), rng.choice(forms))


def s_abstract_like(rng):
    return "\\paragraph{%s.} %s" % (rng.choice(WORDS).capitalize(), para(rng, 2))


def s_siunitx(rng):
    return "The device draws \\SI{%d}{\\milli\\ampere} at \\SI{%.1f}{\\volt}, i.e.\\ about \\num{%d} units." % (
        rng.randint(1, 900), rng.uniform(1, 12), rng.randint(100, 99999))


def s_cleveref(rng):
    return "As shown in \\cref{sec:a} and \\cref{eq:e1}, %s" % sent(rng, 6)


def s_hrule(rng):
    return "\\bigskip\n\\hrulefill\n%s" % sent(rng, 5)


def s_minipage(rng):
    return "\\begin{minipage}{0.5\\textwidth}\n%s\n\\end{minipage}" % sent(rng, 8)


def s_center(rng):
    return "\\begin{center}\n%s\n\\end{center}" % sent(rng, 5)


def s_algorithm(rng):
    return ("\\begin{algorithm}\n\\caption{%s}\n\\begin{algorithmic}\n\\State $x \\gets 0$\n\\For{$i = 1$ to $n$}\n"
            "\\State $x \\gets x + i$\n\\EndFor\n\\end{algorithmic}\n\\end{algorithm}") % sent(rng, 3).rstrip(".")


def s_special(rng):
    return ("Tom \\& Jerry cost \\$5 at 10\\%% off; use C\\# or the \\_underscore\\_. "
            "Caf\\'e na\\\"ive \\c{c}a r\\'esum\\'e. Em---dash, en--dash, ellipsis\\ldots{} %s") % sent(rng, 4)


def s_comment(rng):
    return "%% a comment that must vanish\n%s %% trailing comment\n%s" % (sent(rng, 5), sent(rng, 4))


def s_newcmd_use(rng):
    return "The \\sys{} reads \\kv{%s}{%d} and \\todoitem{%s}." % (rng.choice(WORDS), rng.randint(1, 9), rng.choice(WORDS))


def s_footnote_math(rng):
    return "Note that $\\alpha_{%d}$ holds\\footnote{Where $\\alpha$ is the rate, see \\url{https://example.org}.} for all $n$." % rng.randint(1, 9)


def s_href_list(rng):
    return "Visit \\url{https://example.org/a_b} or \\href{mailto:a@example.org}{write us}."


def s_ordered_alpha(rng):
    return "\\begin{enumerate}[label=(\\alph*)]\n  \\item %s\n  \\item %s\n\\end{enumerate}" % (sent(rng, 4), sent(rng, 4))


BLOCKS = [s_itemize, s_enumerate, s_nested_list, s_description, s_tabular, s_table_float, s_multicol,
          s_figure, s_equation, s_align, s_display_bracket, s_dollars, s_theorem, s_verbatim,
          s_lstlisting, s_quote, s_footnotes, s_labels, s_cites, s_abstract_like, s_siunitx,
          s_cleveref, s_hrule, s_minipage, s_center, s_algorithm, s_special, s_comment, s_newcmd_use,
          s_footnote_math, s_href_list, s_ordered_alpha]

PACKAGE_SETS = [
    "amsmath,amssymb,amsthm",
    "amsmath,amsthm,graphicx,booktabs",
    "amsmath,amssymb,amsthm,natbib,hyperref,graphicx,booktabs,cleveref,siunitx,listings,enumitem,algorithm,algorithmic",
    "amsmath,amsthm,biblatex,hyperref,graphicx,booktabs,cleveref,siunitx,listings,enumitem,algorithm,algorithmic",
    "amsmath,amssymb,amsthm,graphicx,hyperref,cleveref,siunitx,listings,enumitem,algorithm,algorithmic,booktabs",
]

MACROS = r"""\newcommand{\sys}{\textsc{Frobnitz}}
\newcommand{\kv}[2]{\texttt{#1}=#2}
\newcommand{\todoitem}[1]{\emph{to do: #1}}
\newcommand{\R}{\mathbb{R}}
\DeclareMathOperator{\argmin}{arg\,min}
\newtheorem{theorem}{Theorem}
\newtheorem{lemma}[theorem]{Lemma}
\newtheorem{proposition}{Proposition}
\newtheorem{corollary}{Corollary}
\theoremstyle{definition}
\newtheorem{definition}{Definition}
\theoremstyle{remark}
\newtheorem{remark}{Remark}
"""


def document(rng, idx, with_macros=True):
    pk = PACKAGE_SETS[idx % len(PACKAGE_SETS)]
    cls = rng.choice(["article", "article", "report", "book"])
    opt = rng.choice(["", "[11pt]", "[a4paper,12pt]", "[twocolumn]"])
    lines = ["\\documentclass%s{%s}" % (opt, cls)]
    lines.append("\\usepackage{%s}" % pk)
    if with_macros:
        lines.append(MACROS)
    if rng.random() < 0.8:
        lines.append("\\title{%s}" % " ".join(w.capitalize() for w in rng.sample(WORDS, 4)))
        n = rng.randint(1, 3)
        lines.append("\\author{%s}" % " \\and ".join(rng.sample(NAMES, n)))
        lines.append("\\date{%s}" % rng.choice(["", "\\today", "March 2024"]))
    lines.append("\\begin{document}")
    if "title{" in "\n".join(lines):
        lines.append("\\maketitle")
    if rng.random() < 0.7:
        lines.append("\\begin{abstract}\n%s\n\\end{abstract}" % para(rng, 2))
    nsec = rng.randint(2, 4)
    top = "chapter" if cls in ("report", "book") else "section"
    sub = "section" if top == "chapter" else "subsection"
    for s in range(nsec):
        star = "*" if rng.random() < 0.12 else ""
        lines.append("\\%s%s{%s}\\label{sec:%s}" % (top, star, sent(rng, 3).rstrip("."), "a" if s == 0 else "s%d" % s))
        lines.append(para(rng, 2))
        for _ in range(rng.randint(2, 4)):
            lines.append(rng.choice(BLOCKS)(rng))
            lines.append(para(rng, 1))
        if rng.random() < 0.6:
            lines.append("\\%s{%s}" % (sub, sent(rng, 3).rstrip(".")))
            lines.append(rich_para(rng))
            lines.append(rng.choice(BLOCKS)(rng))
    lines.append("\\end{document}")
    return "\n\n".join(lines) + "\n"


def papers():
    """Hand-written whole documents: name -> source."""
    out = {}
    out["paper-sparse-attention"] = r"""\documentclass[11pt]{article}
\usepackage{amsmath,amssymb,amsthm,graphicx,booktabs,hyperref}
\newtheorem{theorem}{Theorem}
\newtheorem{lemma}{Lemma}
\newcommand{\norm}[1]{\left\lVert #1 \right\rVert}
\newcommand{\E}{\mathbb{E}}
\title{Blockwise Sparse Attention with Provable Error Bounds}
\author{Ada Smith\thanks{Corresponding author.} \and Bo Chen \and Chidi Okafor}
\date{November 2026}
\begin{document}
\maketitle
\begin{abstract}
We analyse a blockwise sparsity pattern for self-attention, prove an error bound against dense attention, and measure the speed-up on long inputs.
\end{abstract}
\section{Introduction}
Self-attention costs $O(n^2)$ in the sequence length $n$. We restrict each query to a set of blocks and show that the output error is bounded by the discarded softmax mass. Our contributions are:
\begin{enumerate}
  \item a block pattern with $O(n\sqrt{n})$ cost;
  \item Theorem~\ref{thm:bound}, an error bound;
  \item an evaluation on three tasks (Table~\ref{tab:main}).
\end{enumerate}
\section{Setup}\label{sec:setup}
Let $Q,K,V\in\mathbb{R}^{n\times d}$. Dense attention is
\begin{equation}\label{eq:dense}
  A = \mathrm{softmax}\!\left(\frac{QK^\top}{\sqrt d}\right)V .
\end{equation}
\subsection{Blocks}
Partition $\{1,\dots,n\}$ into blocks of size $b$. A \emph{mask} $M\in\{0,1\}^{n\times n}$ keeps entry $(i,j)$ iff block$(j)$ is selected for block$(i)$.
\section{Analysis}
\begin{lemma}\label{lem:mass}
Let $p_i$ be the softmax row. If the selected blocks hold mass at least $1-\epsilon$ then $\norm{p_i - \tilde p_i}_1 \le 2\epsilon$.
\end{lemma}
\begin{proof}
The renormalised row $\tilde p_i$ differs from $p_i$ only by the discarded mass $\epsilon$ and the rescaling of the kept mass. Summing the absolute differences gives the claim.
\end{proof}
\begin{theorem}\label{thm:bound}
Under Lemma~\ref{lem:mass}, $\norm{A-\tilde A}_{\infty} \le 2\epsilon \max_j \norm{V_j}$.
\end{theorem}
\section{Experiments}
\begin{table}[h]
\centering
\caption{Accuracy (\%) and relative speed.}
\label{tab:main}
\begin{tabular}{lccc}
\toprule
Model & Task A & Task B & Speed \\
\midrule
Dense & 81.2 & 77.4 & 1.0$\times$ \\
Blockwise & 80.9 & 77.1 & 3.4$\times$ \\
\bottomrule
\end{tabular}
\end{table}
The results in Table~\ref{tab:main} follow Equation~\eqref{eq:dense}.\footnote{Code: \url{https://example.org/blocks}.}
\section*{Acknowledgements}
We thank the reviewers.
\end{document}
"""
    out["paper-notes-algorithms"] = r"""\documentclass{article}
\usepackage{amsmath,amsthm,enumitem,listings}
\newtheorem{definition}{Definition}
\newtheorem{example}{Example}
\newcommand{\Oh}[1]{\mathcal{O}\!\left(#1\right)}
\newenvironment{keypoint}{\par\noindent\textbf{Key point.}\ }{\par}
\title{Notes on Sorting}
\author{Ruth Ng}
\begin{document}
\maketitle
\tableofcontents
\section{Comparison sorts}
\begin{definition}[Comparison sort]
A sort that orders its input only by comparing pairs of elements.
\end{definition}
\begin{keypoint}Any comparison sort needs $\Omega(n\log n)$ comparisons in the worst case.\end{keypoint}
\subsection{Merge sort}
Merge sort runs in $\Oh{n\log n}$ time and uses $\Oh{n}$ extra space.
\begin{lstlisting}[language=Python]
def merge_sort(xs):
    if len(xs) <= 1:
        return xs
    mid = len(xs) // 2
    return merge(merge_sort(xs[:mid]), merge_sort(xs[mid:]))
\end{lstlisting}
\begin{example}
Sorting $[3,1,2]$ takes three comparisons.
\end{example}
\section{Counting sorts}
\begin{description}
  \item[Counting sort] stable, $\Oh{n+k}$.
  \item[Radix sort] stable, $\Oh{d(n+k)}$.
\end{description}
\begin{tabular}{|l|r|r|}
\hline
Algorithm & Time & Space \\ \hline
Merge & $n\log n$ & $n$ \\ \hline
Counting & $n+k$ & $k$ \\ \hline
\end{tabular}
\end{document}
"""
    out["paper-survey-cites"] = r"""\documentclass{article}
\usepackage{natbib}
\usepackage{hyperref}
\title{A Short Survey of Early Computing}
\author{Zoe Park}
\begin{document}
\maketitle
\section{Engines and programs}
The first published program~\citep{lovelace1843} targeted a mechanical engine.
\citet{hopper1952} built the first compiler, and \citet[chap.~2]{knuth1984} later formalised
literate programs. See also \citep[e.g.,][]{dijkstra1968,hoare1969}.
\section{Languages}
Structured programming~\citep{dijkstra1968} and abstract data types~\citep{liskov1987} shaped
languages. A survey by \citeauthor{backus1978} appeared in \citeyear{backus1978}.
\bibliographystyle{plainnat}
\bibliography{refs}
\end{document}
"""
    out["paper-biblatex"] = r"""\documentclass{article}
\usepackage[backend=biber,style=authoryear]{biblatex}
\addbibresource{refs.bib}
\begin{document}
\section{Background}
This follows \textcite{knuth1984} and \parencite[see][p.~10]{lamport1978}.
Several authors \autocite{hoare1969,liskov1987} agree.
\printbibliography
\end{document}
"""
    out["paper-physics-notes"] = r"""\documentclass[12pt]{article}
\usepackage{amsmath,amssymb,siunitx,graphicx}
\newcommand{\vect}[1]{\mathbf{#1}}
\newcommand{\dd}{\mathrm{d}}
\begin{document}
\section{Maxwell's equations}
In vacuum, with $\vect{E}$ the electric and $\vect{B}$ the magnetic field:
\begin{align}
\nabla\cdot\vect{E} &= \frac{\rho}{\varepsilon_0}, \label{eq:gauss}\\
\nabla\times\vect{B} &= \mu_0\vect{J} + \mu_0\varepsilon_0\frac{\partial\vect{E}}{\partial t}. \label{eq:ampere}
\end{align}
Equation~\eqref{eq:gauss} is Gauss's law. A charge of \SI{1.6e-19}{\coulomb} produces at \SI{1}{\metre} a field of about \SI{1.4e-9}{\volt\per\metre}.
\subsection{Units}
\begin{tabular}{lll}
\hline
Quantity & Symbol & Unit \\
\hline
Charge & $q$ & C \\
Field & $E$ & V/m \\
\hline
\end{tabular}
\begin{figure}
\centering
\includegraphics[width=6cm]{field-lines}
\caption{Field lines of a point charge.}
\end{figure}
\end{document}
"""
    out["paper-memo-lists"] = r"""\documentclass{article}
\usepackage[utf8]{inputenc}
\usepackage[T1]{fontenc}
\begin{document}
\section{Agenda}
\begin{enumerate}
\item Welcome
\item Review of last week
  \begin{itemize}
  \item Budget: \textbf{on track}
  \item Hiring: \emph{two open roles}
    \begin{itemize}
    \item Backend
    \item Design
    \end{itemize}
  \end{itemize}
\item Next steps
\end{enumerate}
\section{Decisions}
\begin{quote}
We ship on Friday unless the tests fail.
\end{quote}
Caf\'e stop at 3\,pm --- bring \$20.\footnote{Cash only.}
\end{document}
"""
    return out


def project_files():
    """Whole projects with \\input / \\include / bibliography. name -> {relative path: text}."""
    projects = {}
    for n in range(1, 13):
        rng = random.Random(9000 + n)
        main = [r"\documentclass{article}", r"\usepackage{amsmath,amsthm,natbib}" if n % 2 else r"\usepackage{amsmath,amsthm}",
                r"\newcommand{\sys}{\textsc{Frobnitz}}", r"\newtheorem{theorem}{Theorem}",
                r"\title{Project %d}" % n, r"\author{%s}" % rng.choice(NAMES), r"\begin{document}", r"\maketitle"]
        files = {}
        parts = rng.randint(2, 4)
        for p in range(1, parts + 1):
            fname = "sections/part%d.tex" % p
            body = ["\\section{%s}" % sent(rng, 3).rstrip("."), para(rng, 2)]
            for _ in range(rng.randint(1, 3)):
                body.append(rng.choice([s_itemize, s_tabular, s_equation, s_theorem, s_footnotes, s_cites, s_figure])(rng))
                body.append(para(rng, 1))
            files[fname] = "\n\n".join(body) + "\n"
            main.append(("\\input{sections/part%d}" if p % 2 else "\\include{sections/part%d}") % p)
        if n % 3 == 0:
            files["macros.tex"] = "\\newcommand{\\extra}{extra text}\n"
            main.insert(3, "\\input{macros.tex}")
            main.append("Here is \\extra{}.")
        main += [r"\bibliographystyle{plainnat}" if n % 2 else r"\bibliographystyle{plain}", r"\bibliography{refs}", r"\end{document}"]
        files["main.tex"] = "\n".join(main) + "\n"
        projects["proj%02d" % n] = files
    return projects
