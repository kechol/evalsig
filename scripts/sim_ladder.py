"""The Ladder's defaults (D-015): how often an adaptive agent passes the gate without a
real improvement, and how often a real improvement passes and gets its value back, by
holdout size n, budget B and step factor k.

Every submission is judged on the paired accuracy difference to the baseline, item by
item d in {-1, 0, 1}, with a one-sided normal bound at level alpha/B; it passes when
the bound is above 0 (metrics spec §11.1).

- Adaptive attack (no real improvement; Blum and Hardt 2015): in each of the first
  B - 1 rounds the agent submits a random candidate whose d is +1 or -1 with
  probability psi/2 each, so it has no real advantage. It keeps the candidates the
  gate's answers point to, and in the last round submits their item-by-item majority,
  which is still no better on the population (d stays symmetric). With full feedback
  the agent sees each unrounded difference and keeps those above 0. With the Ladder it
  sees a value only on a new best: the lower bound of the difference to the best at
  alpha/B is above 0 and the gap is at least eta (metrics spec §11.3). A false pass is
  any pass within the budget.
- Real improvement: one candidate better by delta on the population, submitted once.
  It passes at the power of the test at alpha/B; under the Ladder it gets its value
  back only when it is also a new best over the baseline by at least eta.

eta = max(k * sqrt(psi_hat / n), 1 / n) with psi_hat = max(0.05, 4/n, min(1, 2 * e_val)).
The bounds use the normal approximation; the gate's own intervals differ in detail,
not in how the rates move with n, B and k.

    uv run --project scripts scripts/sim_ladder.py [--sims N]
"""

import argparse
import itertools
import math

import numpy as np
from scipy import stats

ALPHA = 0.05
E_VAL = 0.1
PSI = 2 * E_VAL
DELTA = 0.05


def eta(k, n):
    psi_hat = max(0.05, 4 / n, min(1.0, 2 * E_VAL))
    return max(k * math.sqrt(psi_hat / n), 1 / n)


def lower_bound(d, z):
    sd = d.std(ddof=1)
    return d.mean() - z * sd / math.sqrt(len(d))


def random_candidate(rng, n, delta=0.0):
    u = rng.random(n)
    up, down = (PSI + delta) / 2, (PSI - delta) / 2
    return np.where(u < up, 1.0, np.where(u < up + down, -1.0, 0.0))


def new_best(d, best, z, step):
    over = d - best
    return lower_bound(over, z) > 0 and over.mean() >= step


def attack(rng, n, budget, k, ladder):
    z = stats.norm.ppf(1 - ALPHA / budget)
    step = eta(k, n)
    best = np.zeros(n)
    kept = []
    for _ in range(budget - 1):
        d = random_candidate(rng, n)
        if lower_bound(d, z) > 0:
            return True
        if ladder:
            if new_best(d, best, z, step):
                best = d
                kept.append(d)
        elif d.mean() > 0:
            kept.append(d)
    if not kept:
        return False
    final = np.sign(np.sum(kept, axis=0))
    return lower_bound(final, z) > 0


def improvement(rng, n, budget, k):
    z = stats.norm.ppf(1 - ALPHA / budget)
    d = random_candidate(rng, n, DELTA)
    passed = lower_bound(d, z) > 0
    return passed, passed and new_best(d, np.zeros(n), z, eta(k, n))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sims", type=int, default=4000)
    ap.add_argument("--seed", type=int, default=20261103)
    a = ap.parse_args()
    rng = np.random.default_rng(a.seed)
    print(f"alpha {ALPHA}, e_val {E_VAL}, discordance {PSI}, real improvement {DELTA}, "
          f"{a.sims} runs per cell\n")
    print("| n | B | k | eta | false pass, full feedback | false pass, Ladder "
          "| real improvement passes | ... and gets its value back |")
    print("|---:|---:|---:|---:|---:|---:|---:|---:|")
    for n, budget, k in itertools.product((300, 1000), (5, 10, 20), (1, 2, 3)):
        full = np.mean([attack(rng, n, budget, k, False) for _ in range(a.sims)])
        lad = np.mean([attack(rng, n, budget, k, True) for _ in range(a.sims)])
        real = np.array([improvement(rng, n, budget, k) for _ in range(a.sims)])
        print(f"| {n} | {budget} | {k} | {eta(k, n):.3f} | {full:.3f} | {lad:.3f} "
              f"| {real[:, 0].mean():.3f} | {real[:, 1].mean():.3f} |")


if __name__ == "__main__":
    main()
