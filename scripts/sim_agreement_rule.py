"""Why system–human agreement is compared under the same rule as human–human
agreement (metrics spec §9.1): against the raters' majority or soft share, a system no
better than any rater looks better than the raters do against each other.

2,000 items with 40% positives, three raters and the system, each correct with
probability 0.85 independently. Prints κ between two raters, κ of the system against
the majority of three, the mean κ of the system against each rater, and accuracy
against the soft reference for the system and for a rater whose own label is in the
reference, beside the ceiling mean(max(s, 1 - s)).

    uv run --project scripts scripts/sim_agreement_rule.py
"""

import numpy as np

rng = np.random.default_rng(8)
n, R = 2000, 3
acc = 0.85


def kappa(x, y):
    po = (x == y).mean()
    pe = x.mean() * y.mean() + (1 - x.mean()) * (1 - y.mean())
    return (po - pe) / (1 - pe)


def noisy(t):
    flip = rng.binomial(1, 1 - acc, len(t))
    return np.where(flip == 1, 1 - t, t)


truth = rng.binomial(1, 0.4, n)
labels = np.stack([noisy(truth) for _ in range(R)], 1)
system = noisy(truth)
majority = (labels.sum(1) >= 2).astype(int)
print(f"κ(rater1, rater2)        = {kappa(labels[:, 0], labels[:, 1]):.3f}")
print(f"κ(system, majority of 3) = {kappa(system, majority):.3f}")
print(f"κ(system, rater j) mean  = {np.mean([kappa(system, labels[:, j]) for j in range(R)]):.3f}")
s = labels.mean(1)
print(
    f"accuracy on the soft reference: system = {np.mean(system * s + (1 - system) * (1 - s)):.3f},"
    f"  rater1 (in the reference) = {np.mean(labels[:, 0] * s + (1 - labels[:, 0]) * (1 - s)):.3f},"
    f"  ceiling mean(max(s, 1-s)) = {np.maximum(s, 1 - s).mean():.3f}"
)
