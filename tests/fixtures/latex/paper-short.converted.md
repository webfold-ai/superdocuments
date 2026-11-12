---
title: "Sparse Folding for Protein Structure"
author:
- "Ada Lovelace"
- "Alan Turing"
date: "March 2024"
abstract: |
  We introduce FoldNet, which improves accuracy by 12% on CASP14. Code: <https://example.org/foldnet>.
---

# Introduction {#sec:intro}

Structure prediction [@jumper2021; @baek2021] is hard. Following @jumper2021, FoldNet minimises the loss in Eq. [(1)](#eq:loss) over $x \in \mathbb{R}^n$ with $\lVert x \rVert \le 1$[^1]. Schrödinger, Erdős and Poincaré are cited “often” — see [this paper](https://arxiv.org/abs/1706.03762). Section [2](#sec:method) has the details, Table [1](#tab:results) the numbers and Figure [1](#fig:arch) the model.

# Method {#sec:method}

## Objective

We solve

$$
\theta^\star = \operatorname*{arg\,min}_{\theta} \sum_{i=1}^{n} \lVert f_\theta(x_i) - y_i \rVert^2 . \tag{1}
$$

The update is

$$
\begin{aligned}
g_t &= \nabla_\theta \mathcal{L}(\theta_t),
\end{aligned} \tag{2}
$$

$$
\begin{aligned}
\theta_{t+1} &= \theta_t - \eta g_t .
\end{aligned}
$$

**Theorem 1 (Convergence)**. *If $\eta$ is small, [(2)](#eq:grad) converges.*

- Encoder with $d = 512$.
- Decoder:

  1. attention,
  2. refinement.

**Table 1:** Accuracy on two benchmarks.

|  | TM-score |  |
| --- | --- | --- |
| Method | CASP14 | CAMEO |
| Baseline | 0.71 | 0.80 |
| FoldNet | **0.85** | **0.88** |

**Figure 1:** The architecture of FoldNet: ([a](#fig:enc)) encoder, ([b](#fig:dec)) decoder.

![Encoder.](figs/encoder){#fig:enc}

![Decoder.](figs/decoder.pdf){#fig:dec}

Theorem [1](#thm:conv) and Figure [1b](#fig:dec) conclude the section.

[^1]: The unit ball, e.g., in $\mathbb{R}^3$.

# References

- `jumper2021` — John Jumper, Richard Evans and Alexander Pritzel (2021). Highly accurate protein structure prediction with AlphaFold. *Nature* 596, 583–589. <https://doi.org/10.1038/s41586-021-03819-2>
- `baek2021` — Minkyung Baek and Frank DiMaio (2021). Accurate prediction of protein structures and interactions. *Science*.
