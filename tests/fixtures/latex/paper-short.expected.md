# Sparse Folding for Protein Structure

Ada Lovelace, Alan Turing

March 2024

## Abstract

We introduce FoldNet, which improves accuracy by 12% on CASP14. Code: <https://example.org/foldnet>.

## Introduction

Structure prediction (jumper2021; baek2021) is hard. Following jumper2021, FoldNet minimises the loss in Eq. (1) over $x \in \R^n$ with $\norm{x} \le 1$[^1]. Schrödinger, Erdős and Poincaré are cited “often” — see [this paper](https://arxiv.org/abs/1706.03762). Section 2 has the details, Table 1 the numbers and Figure 1 the model.

## Method

### Objective

We solve

$$
\theta^\star = \argmin_{\theta} \sum_{i=1}^{n} \norm{f_\theta(x_i) - y_i}^2 . \tag{1}
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

**Theorem 1 (Convergence).** If $\eta$ is small, (2) converges.

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

**Figure 1:** The architecture of FoldNet: (a) encoder, (b) decoder.

**(1a)** Encoder.

![](figs/encoder)

**(1b)** Decoder.

![](figs/decoder.pdf)

Theorem 1 and Figure 1b conclude the section.

[^1]: The unit ball, e.g., in $\R^3$.
