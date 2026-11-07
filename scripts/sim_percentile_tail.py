"""Why paired rates default to Tango's score test rather than the percentile bootstrap,
and why the bootstrap needs B >= 50/α_adj (metrics spec §7 and §8).

Under the null (the paired accuracy difference is 0) on n = 300 items, with a share ψ
of discordant items split evenly, count how often the one-sided lower bound of the
difference at level 1 - α is above 0, which is the real type I error: the percentile
bootstrap (2,000 resamples), Tango's score test (the bound of its interval), the exact
McNemar test and the Wald bound, first at ψ = 0.10 and then at the sparse ψ = 0.03 and
0.05. Then, on one fixed data set (15 and 15 discordant items), the Monte Carlo spread
of the bootstrap's α quantile across seeds at 2,000 and 20,000 resamples.

    uv run --project scripts scripts/sim_percentile_tail.py
"""

import numpy as np
from scipy.stats import binom, norm

n, Bt, D = 300, 2000, 20000


def mcnemar_rejects(b, c, a):
    return b + c > 0 and binom.sf(b - 1, b + c, 0.5) <= a


def score_rejects(b, c, a):
    # At a difference of 0, Tango's statistic is (b - c) / sqrt(b + c).
    return b + c > 0 and (b - c) / np.sqrt(b + c) > norm.ppf(1 - a)


rng = np.random.default_rng(5)
psi = 0.10
alphas = [0.05, 0.01, 0.0025]
hits = {a: 0 for a in alphas}
hits_mcnemar = {a: 0 for a in alphas}
hits_score = {a: 0 for a in alphas}
hits_wald = {a: 0 for a in alphas}
for _ in range(D):
    cnt = rng.multinomial(n, [psi / 2, psi / 2, 1 - psi])
    b, c = cnt[0], cnt[1]
    bs = rng.multinomial(n, cnt / n, Bt)
    delta = (bs[:, 0] - bs[:, 1]) / n
    for a in alphas:
        hits[a] += np.quantile(delta, a) > 0
        hits_mcnemar[a] += mcnemar_rejects(b, c, a)
        hits_score[a] += score_rejects(b, c, a)
        se = np.sqrt((b + c - (b - c) ** 2 / n) / n**2) if b + c > 0 else 0
        hits_wald[a] += ((b - c) / n - norm.ppf(1 - a) * se > 0) if se > 0 else False
print(f"n={n}, ψ={psi}, null. datasets={D}, B_boot={Bt}")
for a in alphas:
    print(
        f"nominal α={a:<7} percentile-boot L>0 rate={hits[a] / D:.4f}"
        f"  Tango score={hits_score[a] / D:.4f}  exact McNemar={hits_mcnemar[a] / D:.4f}"
        f"  Wald={hits_wald[a] / D:.4f}"
    )

b, c = 15, 15
cnt = np.array([b, c, n - b - c])
se_delta = np.sqrt((b + c) / n**2)
for a in (0.01, 0.0025):
    qs = np.array(
        [np.quantile((rng.multinomial(n, cnt / n, Bt)[:, :2] @ np.array([1, -1])) / n, a) for _ in range(2000)]
    )
    print(
        f"quantile {a}: mean={qs.mean():.4f} MC-SD={qs.std():.4f}"
        f"  (SE of Δ={se_delta:.4f}; MC-SD/SE = {qs.std() / se_delta:.2f})  B_boot=2000"
    )
    qs2 = np.array(
        [np.quantile((rng.multinomial(n, cnt / n, 20000)[:, :2] @ np.array([1, -1])) / n, a) for _ in range(500)]
    )
    print(f"quantile {a}: mean={qs2.mean():.4f} MC-SD={qs2.std():.4f}  B_boot=20000")

rng = np.random.default_rng(55)
for psi in (0.03, 0.05):
    hits = {a: 0 for a in (0.01, 0.0025)}
    mc = {a: 0 for a in (0.01, 0.0025)}
    sc = {a: 0 for a in (0.01, 0.0025)}
    for _ in range(D):
        cnt = rng.multinomial(n, [psi / 2, psi / 2, 1 - psi])
        b, c = cnt[0], cnt[1]
        bs = rng.multinomial(n, cnt / n, Bt)
        delta = (bs[:, 0] - bs[:, 1]) / n
        for a in hits:
            hits[a] += np.quantile(delta, a) > 0
            mc[a] += mcnemar_rejects(b, c, a)
            sc[a] += score_rejects(b, c, a)
    for a in hits:
        print(
            f"ψ={psi} nominal α={a}: percentile-boot L>0 rate={hits[a] / D:.4f}"
            f"  Tango score={sc[a] / D:.4f}  exact McNemar={mc[a] / D:.4f}"
        )
