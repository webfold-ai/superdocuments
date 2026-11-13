---
title: "Blockwise Sparse Attention with Provable Error Bounds"
author:
- "Ada Smith"
- "Bo Chen"
- "Chidi Okafor"
date: "November 2026"
abstract: |
  We analyse a blockwise sparsity pattern for self-attention, prove an error bound against dense attention, and measure the speed-up on long inputs.
---

# Introduction

Self-attention costs $O(n^2)$ in the sequence length $n$. We restrict each query to a set of blocks and show that the output error is bounded by the discarded softmax mass. Our contributions are:

1. a block pattern with $O(n\sqrt{n})$ cost;
2. Theorem [1](#thm:bound), an error bound;
3. an evaluation on three tasks (Table [1](#tab:main)).

# Setup {#sec:setup}

Let $Q,K,V\in\mathbb{R}^{n\times d}$. Dense attention is

$$
A = \mathrm{softmax}\!\left(\frac{QK^\top}{\sqrt d}\right)V . \tag{1}
$$

## Blocks

Partition $\{1,\dots,n\}$ into blocks of size $b$. A *mask* $M\in\{0,1\}^{n\times n}$ keeps entry $(i,j)$ iff block$(j)$ is selected for block$(i)$.

# Analysis

**Lemma 1**. *Let $p_i$ be the softmax row. If the selected blocks hold mass at least $1-\epsilon$ then $\left\lVert p_i - \tilde p_i \right\rVert_1 \le 2\epsilon$.*

*Proof.* The renormalised row $\tilde p_i$ differs from $p_i$ only by the discarded mass $\epsilon$ and the rescaling of the kept mass. Summing the absolute differences gives the claim.

**Theorem 1**. *Under Lemma [1](#lem:mass), $\left\lVert A-\tilde A \right\rVert_{\infty} \le 2\epsilon \max_j \left\lVert V_j \right\rVert$.*

# Experiments

| Model | Task A | Task B | Speed |
| --- | --- | --- | --- |
| Dense | 81.2 | 77.4 | 1.0$\times$ |
| Blockwise | 80.9 | 77.1 | 3.4$\times$ |

: Accuracy (%) and relative speed. {#tab:main}

The results in Table [1](#tab:main) follow Equation [(1)](#eq:dense).[^1]

# Acknowledgements

We thank the reviewers.

[^1]: Code: <https://example.org/blocks>.
