---
title: "Notes on Sorting"
author:
- "Ruth Ng"
---

# Comparison sorts

**Definition 1** (Comparison sort). A sort that orders its input only by comparing pairs of elements.

**Key point.** Any comparison sort needs $\Omega(n\log n)$ comparisons in the worst case.

## Merge sort

Merge sort runs in $\mathcal{O}\!\left(n\log n\right)$ time and uses $\mathcal{O}\!\left(n\right)$ extra space.

```
def merge_sort(xs):
    if len(xs) <= 1:
        return xs
    mid = len(xs) // 2
    return merge(merge_sort(xs[:mid]), merge_sort(xs[mid:]))
```

**Example 1**. Sorting $[3,1,2]$ takes three comparisons.

# Counting sorts

Counting sort
:   stable, $\mathcal{O}\!\left(n+k\right)$.

Radix sort
:   stable, $\mathcal{O}\!\left(d(n+k)\right)$.

| Algorithm | Time | Space |
| --- | --- | --- |
| Merge | $n\log n$ | $n$ |
| Counting | $n+k$ | $k$ |
