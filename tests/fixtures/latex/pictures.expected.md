# Pictures

*TikZ picture, not rendered.*

```latex
\begin{tikzpicture}[scale=0.8, every node/.style={draw}]
  \node (a) at (0,0) {A}; % a comment with \end{tikzpicture} in it
  \node (b) at (2,0) {B};
  \draw[->] (a) -- (b);
\end{tikzpicture}
```

*TikZ picture, not rendered.*

```latex
\begin{tikzpicture}
  \draw (0,0) circle (1);
\end{tikzpicture}
```

**Figure 1:** Two diagrams in one float.

*forest picture, not rendered.*

```latex
\begin{forest}
  [S [NP [she]] [VP [runs]]]
\end{forest}
```

**Figure 2:** A parse tree.

**Figure 3:** Circuit and line.

*circuitikz picture, not rendered.*

```latex
\begin{circuitikz} \draw (0,0) to[R] (2,0); \end{circuitikz}
```

**(3a)** A resistor.

*LaTeX picture, not rendered.*

```latex
\begin{picture}(100,50)
  \put(0,0){\line(1,0){100}}
\end{picture}
```

**(3b)** A line.

**Table 1:** A table drawn as a picture.

*pgfpicture picture, not rendered.*

```latex
\begin{pgfpicture}
  \pgfpathmoveto{\pgfpointorigin}
\end{pgfpicture}
```

Figures 1, 2 and 3a; Table 1.
