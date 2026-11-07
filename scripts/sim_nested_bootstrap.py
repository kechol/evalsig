"""Why the bootstrap resamples only the top level (metrics spec §8): resampling the
generation replicates inside each resampled item as well counts the within-item
variance twice.

A single-shot rate over n = 300 items with K = 3 replicates each: item i's chance
p_i ~ Beta(4, 1) (mean 0.8, wide spread between items), its count c_i ~ Binomial(K,
p_i), and the estimate mean(c_i / K). The true standard error comes from new items with
new replicates; the bootstraps run on one data set at a time and are compared with it
by mean standard error and by the coverage of a two-sided 95% percentile interval.

    uv run --project scripts scripts/sim_nested_bootstrap.py
"""

import numpy as np

rng = np.random.default_rng(1)
n, K = 300, 3
a, b = 4.0, 1.0


def gen():
    p = rng.beta(a, b, n)
    return rng.binomial(K, p)


R = 40000
ests = np.array([(gen() / K).mean() for _ in range(R)])
true_var = ests.var()
theta = a / (a + b)

D, Bt = 400, 1000
item_only_var, nested_var = [], []
cov_item, cov_nested = 0, 0
for _ in range(D):
    c = gen()
    y = c / K
    idx = rng.integers(0, n, (Bt, n))
    bo = y[idx].mean(1)
    item_only_var.append(bo.var())
    lo, hi = np.quantile(bo, [0.025, 0.975])
    cov_item += lo <= theta <= hi
    # Nested: after drawing the items, redraw each item's K replicates from its own
    # observed share, that is Binomial(K, c_i / K).
    cn = rng.binomial(K, c[idx] / K)
    bn = (cn / K).mean(1)
    nested_var.append(bn.var())
    lo, hi = np.quantile(bn, [0.025, 0.975])
    cov_nested += lo <= theta <= hi

print(f"true SE            = {np.sqrt(true_var):.5f}")
print(
    f"item-only boot SE  = {np.sqrt(np.mean(item_only_var)):.5f}"
    f"  (ratio {np.sqrt(np.mean(item_only_var) / true_var):.3f})  coverage95 = {cov_item / D:.3f}"
)
print(
    f"nested boot SE     = {np.sqrt(np.mean(nested_var)):.5f}"
    f"  (ratio {np.sqrt(np.mean(nested_var) / true_var):.3f})  coverage95 = {cov_nested / D:.3f}"
)
p = rng.beta(a, b, 10**6)
print(f"between var(p) = {p.var():.4f}, mean within p(1-p)/K = {(p * (1 - p) / K).mean():.4f}")
