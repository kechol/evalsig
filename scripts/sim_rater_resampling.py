"""Why raters are not resampled (metrics spec §9.3): in a crossed design, where the
same three raters label every item and differ systematically in how lenient they are,
resampling raters does not recover the variance over new raters.

Item i has a latent positiveness z_i ~ N(0, 1); rater r labels it positive when
z_i + noise > bias_r, with bias_r ~ N(0, 0.6); the system's judgment is z_i + noise > 0.
The metric is the soft hit rate H = Σ s_i·y_i / Σ s_i with s_i the raters' positive
share. The true standard errors come from new items with new raters, and from new items
with the raters fixed. Three bootstraps run on data with fixed raters: items only,
items plus raters redrawn independently within each item, and items plus one redraw of
the rater set per replicate.

    uv run --project scripts scripts/sim_rater_resampling.py
"""

import numpy as np

rng = np.random.default_rng(3)
n, R = 300, 3
tau = 0.6


def gen(new_raters=True, bias=None):
    z = rng.normal(0, 1, n)
    if bias is None or new_raters:
        bias = rng.normal(0, tau, R)
    labels = (z[:, None] + rng.normal(0, 0.7, (n, R)) > bias[None, :]).astype(int)
    y = (z + rng.normal(0, 0.7, n) > 0).astype(int)
    return labels, labels.mean(1), y, bias


def h_soft(s, y):
    return (s * y).sum() / s.sum()


ests_new = np.array([h_soft(*gen()[1:3]) for _ in range(20000)])
fixed_bias = rng.normal(0, tau, R)
ests_fixed = np.array([h_soft(*gen(False, fixed_bias)[1:3]) for _ in range(20000)])
print(f"true SE (new items + new raters) = {ests_new.std():.5f}")
print(f"true SE (new items, raters fixed)= {ests_fixed.std():.5f}")

D, Bt = 300, 1000
se = {"item": [], "item+rater(per-item)": [], "item+rater(block)": []}
for _ in range(D):
    labels, s, y, _ = gen(False, fixed_bias)
    idx = rng.integers(0, n, (Bt, n))
    lb, yb = labels[idx], y[idx]
    sb = lb.mean(2)
    se["item"].append(((sb * yb).sum(1) / sb.sum(1)).std())
    ridx = rng.integers(0, R, (Bt, n, R))
    sr = np.take_along_axis(lb, ridx, 2).mean(2)
    se["item+rater(per-item)"].append(((sr * yb).sum(1) / sr.sum(1)).std())
    rblk = rng.integers(0, R, (Bt, R))
    sk = np.take_along_axis(lb, rblk[:, None, :].repeat(n, 1), 2).mean(2)
    se["item+rater(block)"].append(((sk * yb).sum(1) / sk.sum(1)).std())
for k, v in se.items():
    print(
        f"{k:24s} boot SE = {np.mean(v):.5f}"
        f"  (vs new-raters {np.mean(v) / ests_new.std():.2f}x, vs fixed {np.mean(v) / ests_fixed.std():.2f}x)"
    )
