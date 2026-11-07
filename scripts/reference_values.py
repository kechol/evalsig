#!/usr/bin/env python3
"""Generate tests/fixtures/reference-values.json, the reference values for the Rust metrics.

Every case names its source in "source":
  scipy      - the scipy.stats implementation, written independently of evalsig
  closed     - a closed-form expression computed here (same formulas as the metrics spec)
  literature - a published value, so that an error shared by two implementations is still caught

Bootstrap results are not included: random number generators differ between languages, so
the values cannot be matched. Property tests cover the bootstrap instead.

Usage: uv run --project scripts scripts/reference_values.py [--out PATH]
"""
import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np
import scipy
from scipy import stats

OUT = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "reference-values.json"
Z95 = stats.norm.ppf(0.975)
Z95_ONE = stats.norm.ppf(0.95)


def r(x):
    # Full double precision: rounding to fewer digits lets a last-bit library difference
    # flip a stored digit, which would exceed the regeneration tolerance.
    return None if x is None else float(x)


def wilson(k, n, z):
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def loglinear(k, n):
    return (k + 0.5) / (n + 1)


def proportion_cases():
    out = []
    for k, n in [(0, 10), (0, 268), (0, 300), (3, 20), (14, 42), (50, 100), (99, 100), (100, 100)]:
        bt = stats.binomtest(k, n)
        w = bt.proportion_ci(confidence_level=0.95, method="wilson")
        cp = bt.proportion_ci(confidence_level=0.95, method="exact")
        wl, wh = wilson(k, n, Z95)
        assert abs(wl - w.low) < 1e-12 and abs(wh - w.high) < 1e-12, (k, n)
        cp_upper_one = 1.0 if k == n else stats.beta.ppf(0.95, k + 1, n - k)
        jl = 0.0 if k == 0 else stats.beta.ppf(0.025, k + 0.5, n - k + 0.5)
        jh = 1.0 if k == n else stats.beta.ppf(0.975, k + 0.5, n - k + 0.5)
        out.append({
            "input": {"k": k, "n": n},
            "source": f"scipy {scipy.__version__} (binomtest.proportion_ci, beta.ppf)",
            "expected": {
                "wilson95": [r(w.low), r(w.high)],
                "clopper_pearson95": [r(cp.low), r(cp.high)],
                "jeffreys95": [r(jl), r(jh)],
                "wilson_one_sided95_upper": r(wilson(k, n, Z95_ONE)[1]),
                "clopper_pearson_one_sided95_upper": r(cp_upper_one),
            },
        })
    return out


def sdt_cases():
    out = []
    for kh, npos, kf, nneg in [(40, 50, 5, 50), (50, 50, 0, 50), (10, 50, 10, 50), (30, 40, 8, 60)]:
        h, f = loglinear(kh, npos), loglinear(kf, nneg)
        zh, zf = stats.norm.ppf(h), stats.norm.ppf(f)
        out.append({
            "input": {"k_hit": kh, "n_pos": npos, "k_fa": kf, "n_neg": nneg},
            "source": f"closed (log-linear correction) + scipy {scipy.__version__} norm.ppf",
            "expected": {"H_raw": r(kh / npos), "F_raw": r(kf / nneg), "H_ll": r(h), "F_ll": r(f),
                         "dprime": r(zh - zf), "c": r(-(zh + zf) / 2)},
        })
    return out


def ordinal_cases():
    """Ordinal output with k levels. hist counts run from the strongest level. AUC is Mann-Whitney (ties 0.5)."""
    out = []
    for pos, neg in [([20, 10, 5, 5], [2, 8, 10, 40]), ([5, 5, 5, 5], [5, 5, 5, 5]), ([30, 0, 0, 0], [1, 2, 3, 24])]:
        k = len(pos)
        xs = np.repeat(np.arange(k, 0, -1), pos)  # stronger levels get larger values
        ys = np.repeat(np.arange(k, 0, -1), neg)
        u = stats.mannwhitneyu(xs, ys, alternative="two-sided", method="asymptotic").statistic
        auc = u / (len(xs) * len(ys))
        npos, nneg = sum(pos), sum(neg)
        pts, zpts = [], []
        for i in range(1, k):
            kh, kf = sum(pos[:i]), sum(neg[:i])
            pts.append([r(kf / nneg), r(kh / npos)])
            zpts.append((stats.norm.ppf(loglinear(kf, nneg)), stats.norm.ppf(loglinear(kh, npos))))
        zf, zh = np.array([p[0] for p in zpts]), np.array([p[1] for p in zpts])
        b, a = np.polyfit(zf, zh, 1)
        out.append({
            "input": {"hist_pos": pos, "hist_neg": neg},
            "source": f"scipy {scipy.__version__} mannwhitneyu + numpy polyfit (z-ROC with log-linear correction)",
            "expected": {"auc": r(auc), "roc_points_FH": pts, "zroc_slope_b": r(b), "zroc_intercept_a": r(a),
                         "d_a": r(a * math.sqrt(2 / (1 + b * b)))},
        })
    return out


def _psi(x, y):
    return 1.0 if x > y else 0.5 if x == y else 0.0


def _delong_components(pos, neg):
    """Structural components by explicit pairs (DeLong, DeLong & Clarke-Pearson 1988)."""
    v10 = np.array([np.mean([_psi(x, y) for y in neg]) for x in pos])
    v01 = np.array([np.mean([_psi(x, y) for x in pos]) for y in neg])
    return v10, v01


def _midrank(x):
    order = np.argsort(x, kind="mergesort")
    xs = x[order]
    ranks = np.empty(len(x))
    i = 0
    while i < len(x):
        j = i
        while j < len(x) and xs[j] == xs[i]:
            j += 1
        ranks[i:j] = 0.5 * (i + j - 1) + 1
        i = j
    out = np.empty(len(x))
    out[order] = ranks
    return out


def _fast_delong(pos, neg):
    """The midrank algorithm of Sun & Xu (2014), as a second implementation."""
    m, n = len(pos), len(neg)
    tx, ty = _midrank(np.array(pos, float)), _midrank(np.array(neg, float))
    tz = _midrank(np.array(list(pos) + list(neg), float))
    auc = (tz[:m].sum() - m * (m + 1) / 2) / (m * n)
    v10 = (tz[:m] - tx) / n
    v01 = 1 - (tz[m:] - ty) / m
    return auc, v10, v01


def delong_cases():
    """AUC variance and interval (logit scale, delta method) and the paired comparison of
    two scores on the same items, by two independent implementations that must agree."""
    out = []
    singles = [
        ([0.9, 0.8, 0.8, 0.7, 0.6, 0.55, 0.4], [0.7, 0.5, 0.45, 0.3, 0.3, 0.2, 0.1, 0.05]),
        ([3, 3, 2, 2, 2, 1], [2, 1, 1, 1, 0, 0, 0]),
    ]
    for pos, neg in singles:
        v10, v01 = _delong_components(pos, neg)
        auc = v10.mean()
        fauc, fv10, fv01 = _fast_delong(pos, neg)
        assert abs(auc - fauc) < 1e-12 and np.allclose(v10, fv10) and np.allclose(v01, fv01)
        var = np.var(v10, ddof=1) / len(pos) + np.var(v01, ddof=1) / len(neg)
        se_logit = math.sqrt(var) / (auc * (1 - auc))
        lo, hi = (1 / (1 + math.exp(-(math.log(auc / (1 - auc)) + s * Z95 * se_logit))) for s in (-1, 1))
        out.append({
            "input": {"pos": pos, "neg": neg},
            "source": "independent pair-sum and Sun & Xu midrank implementations (agree to 1e-12)",
            "expected": {"auc": r(auc), "variance": r(var), "logit95": [r(lo), r(hi)]},
        })
    labels = [1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0]
    a = [0.9, 0.85, 0.7, 0.6, 0.6, 0.3, 0.65, 0.4, 0.35, 0.3, 0.2, 0.1, 0.05]
    b = [0.8, 0.5, 0.75, 0.4, 0.7, 0.55, 0.6, 0.6, 0.2, 0.45, 0.1, 0.3, 0.2]
    comps = []
    for s in (a, b):
        pos = [x for x, l in zip(s, labels) if l]
        neg = [x for x, l in zip(s, labels) if not l]
        comps.append(_delong_components(pos, neg))
        f = _fast_delong(pos, neg)
        assert np.allclose(comps[-1][0], f[1]) and np.allclose(comps[-1][1], f[2])
    (a10, a01), (b10, b01) = comps
    m, n = len(a10), len(a01)
    s10 = np.cov(np.vstack([a10, b10]), ddof=1)
    s01 = np.cov(np.vstack([a01, b01]), ddof=1)
    s = s10 / m + s01 / n
    diff = b10.mean() - a10.mean()
    var_diff = s[0, 0] + s[1, 1] - 2 * s[0, 1]
    z = diff / math.sqrt(var_diff)
    out.append({
        "input": {"labels": labels, "a": a, "b": b},
        "source": "independent pair-sum implementation; covariance of structural components",
        "expected": {"auc_a": r(a10.mean()), "auc_b": r(b10.mean()), "difference": r(diff),
                     "variance_difference": r(var_diff), "z": r(z),
                     "p_two_sided": r(2 * stats.norm.sf(abs(z))),
                     "difference95": [r(diff - Z95 * math.sqrt(var_diff)), r(diff + Z95 * math.sqrt(var_diff))]},
    })
    return out


def calibration_cases():
    """Brier score and 10 equal-width reliability bins (a score of 1.0 falls in the last)."""
    p = [0.05, 0.12, 0.18, 0.33, 0.35, 0.51, 0.66, 0.71, 0.88, 0.95, 1.0, 0.0]
    y = [0, 0, 1, 0, 1, 1, 0, 1, 1, 1, 1, 0]
    brier = float(np.mean([(pi - yi) ** 2 for pi, yi in zip(p, y)]))
    bins = []
    for b in range(10):
        members = [(pi, yi) for pi, yi in zip(p, y) if min(int(pi * 10), 9) == b]
        if members:
            bins.append({"bin": b, "n": len(members), "mean_score": r(np.mean([m[0] for m in members])),
                         "positive_share": r(np.mean([m[1] for m in members]))})
    return [{"input": {"scores": p, "labels": y}, "source": "closed (definition)",
             "expected": {"brier": r(brier), "bins": bins}}]


def cost_cases():
    """Expected cost EC = pi*sum_j P(j|pos)C[1][j] + (1-pi)*sum_j P(j|neg)C[0][j], with levels
    counted from the weakest. A threshold t calls levels >= t positive; with a 2x2 matrix
    its EC is pi*((1-H)C10 + H*C11) + (1-pi)*(F*C01 + (1-F)*C00). Cost-ratio sensitivity
    sets C_FP = 1, C_FN = r and finds the cheapest threshold at both ends of [r_lo, r_hi]."""
    pos, neg = [5, 5, 10, 20], [40, 10, 8, 2]   # weakest level first
    pi = 0.12
    matrix = [[0, 0.2, 1, 3], [4, 2, 0.5, 0]]
    npos, nneg = sum(pos), sum(neg)
    ec = pi * sum(c * w / npos for c, w in zip(matrix[1], pos)) + (1 - pi) * sum(c * w / nneg for c, w in zip(matrix[0], neg))

    def threshold_ec(t, c2):
        h = sum(pos[t:]) / npos
        f = sum(neg[t:]) / nneg
        return pi * ((1 - h) * c2[1][0] + h * c2[1][1]) + (1 - pi) * (f * c2[0][1] + (1 - f) * c2[0][0])

    binary = [[0, 1], [5, 0]]
    ecs = [threshold_ec(t, binary) for t in range(1, len(pos))]
    best = 1 + int(np.argmin(ecs))
    def best_at(r):
        vals = [threshold_ec(t, [[0, 1], [r, 0]]) for t in range(1, len(pos))]
        return 1 + int(np.argmin(vals))
    out = [{
        "input": {"pos_by_level": pos, "neg_by_level": neg, "prevalence": pi, "matrix": matrix,
                  "binary_matrix": binary, "ratio_range": [1, 20]},
        "source": "closed (expected cost over the empirical ROC thresholds)",
        "expected": {"expected_cost": r(ec), "threshold_costs": [r(x) for x in ecs], "optimal_threshold": best,
                     "optimal_at_low": best_at(1), "optimal_at_high": best_at(20)},
    }]
    a, b = (0.8, 0.1), (0.9, 0.2)
    def arm_ec(hf, ratio):
        h, f = hf
        return pi * (1 - h) * ratio + (1 - pi) * f
    lo, hi = arm_ec(a, 1) - arm_ec(b, 1), arm_ec(a, 20) - arm_ec(b, 20)
    out.append({
        "input": {"a": list(a), "b": list(b), "prevalence": pi, "ratio_range": [1, 20]},
        "source": "closed (difference in expected cost of two arms at both ends of the cost-ratio range)",
        "expected": {"difference_at_low": r(lo), "difference_at_high": r(hi), "sign_changes": (lo > 0) != (hi > 0)},
    })
    return out


def _katz(a, n1, b, n0, z):
    aa, bb, m1, m0 = (a, b, n1, n0) if a and b else (a + 0.5, b + 0.5, n1 + 1, n0 + 1)
    lr = (aa / m1) / (bb / m0)
    se = math.sqrt(1 / aa - 1 / m1 + 1 / bb - 1 / m0)
    return lr, se, (lr * math.exp(-z * se), lr * math.exp(z * se))


def lr_bh_cases():
    """Several features at once: p-values of log LR = 0 by the Katz SE, Benjamini-Hochberg
    rejections R at alpha, and intervals at the FCR-adjusted level 1 - max(R, 1)*alpha/m
    (Benjamini & Yekutieli 2005), beside the unadjusted ones."""
    feats = [(12, 50, 3, 50), (30, 40, 30, 60), (7, 20, 0, 80), (20, 50, 18, 50)]
    alpha, m = 0.05, len(feats)
    ps = []
    for a, n1, b, n0 in feats:
        lr, se, _ = _katz(a, n1, b, n0, Z95)
        ps.append(2 * stats.norm.sf(abs(math.log(lr)) / se))
    order = sorted(range(m), key=lambda i: ps[i])
    rejected = 0
    for rank, i in enumerate(order, 1):
        if ps[i] <= rank * alpha / m:
            rejected = rank
    level = 1 - max(rejected, 1) * alpha / m
    z_adj = stats.norm.ppf(1 - (1 - level) / 2)
    adjusted = [list(map(r, _katz(a, n1, b, n0, z_adj)[2])) for a, n1, b, n0 in feats]
    return [{
        "input": {"features": [list(f) for f in feats], "alpha": alpha},
        "source": "closed (Katz log intervals; BH step-up; Benjamini & Yekutieli 2005 FCR level)",
        "expected": {"p_values": [r(p) for p in ps], "rejected": rejected, "adjusted_level": r(level),
                     "adjusted95": adjusted},
    }]


def agreement_ordinal_cases():
    """Quadratic weighted kappa 1 - sum(w*po)/sum(w*pe) with w = (a-b)^2/(Q-1)^2, and
    Spearman's rho (scipy.stats.spearmanr, midranks for ties)."""
    table = np.array([[10, 3, 1], [2, 12, 4], [0, 3, 15]], dtype=float)
    q = table.shape[0]
    po = table / table.sum()
    pe = np.outer(po.sum(axis=1), po.sum(axis=0))
    w = np.array([[(a - b) ** 2 / (q - 1) ** 2 for b in range(q)] for a in range(q)])
    kw = 1 - (w * po).sum() / (w * pe).sum()
    x = [1, 2, 2, 3, 4, 4, 5, 6, 7, 7]
    y = [2, 1, 3, 3, 5, 4, 6, 6, 9, 8]
    rho = stats.spearmanr(x, y).statistic
    return [{
        "input": {"table": table.astype(int).tolist(), "x": x, "y": y},
        "source": f"closed (quadratic weights); scipy {scipy.__version__} spearmanr",
        "expected": {"quadratic_weighted_kappa": r(kw), "spearman_rho": r(rho)},
    }]


def ppi_mean(y, f, f_unlabelled, alpha, lam=None):
    """PPI++ for a mean in the conventions of ppi_py (ppi_mean_pointestimate / ppi_mean_ci):
    lambda = Cov_n(y, f) / ((1 + n/N) Var_{n+N}(f)) clipped to [0, 1] with the population
    covariance and the sample variance; standard errors from population variances."""
    y, f, fu = (np.asarray(a, dtype=float) for a in (y, f, f_unlabelled))
    n, big_n = len(y), len(fu)
    if lam is None:
        cov = np.mean((y - y.mean()) * (f - f.mean()))
        var_f = np.var(np.concatenate([f, fu]), ddof=1)
        lam = float(np.clip(cov / ((1 + n / big_n) * var_f), 0, 1))
    est = lam * fu.mean() + (y - lam * f).mean()
    se = math.sqrt((lam * fu).std() ** 2 / big_n + (y - lam * f).std() ** 2 / n)
    z = stats.norm.ppf(1 - alpha / 2)
    return est, se, lam, (est - z * se, est + z * se)


def ppi_cases():
    """A grader agreeing with humans on about 80% of items; 60 human labels, 240 grader-only."""
    rng = np.random.default_rng(20261103)
    truth = (rng.random(300) < 0.4).astype(int)
    grader = np.where(rng.random(300) < 0.8, truth, 1 - truth)
    y, f, fu = truth[:60], grader[:60], grader[60:]
    out = []
    for lam in (None, 0.0, 1.0):
        est, se, used, ci = ppi_mean(y, f, fu, 0.05, lam)
        out.append({
            "input": {"y": y.tolist(), "f": f.tolist(), "f_unlabelled": fu.tolist(), "level": 0.95, "lambda": lam},
            "source": "closed (PPI++ mean in the conventions of ppi_py ppi_mean_pointestimate, ppi_mean_ci and _calc_lam_glm, read at commit 3d1f0c6)",
            "expected": {"estimate": r(est), "se": r(se), "lambda": r(used), "ci95": [r(ci[0]), r(ci[1])]},
        })
    return out


def bt_loglik(x, comparisons, arms, ties):
    """Davidson-Bradley-Terry log-likelihood with theta_0 = 0; x = (theta_1..theta_{m-1}[, log nu])."""
    theta = np.concatenate([[0.0], x[:arms - 1]])
    total = 0.0
    for i, j, s in comparisons:
        logits = [theta[i], theta[j]]
        if ties:
            logits.append(x[-1] + (theta[i] + theta[j]) / 2)
        lse = np.logaddexp.reduce(logits)
        o = 2 if s == 0.5 else (1 if s == 1 else 0)
        total += logits[o] - lse
    return total


def bt_newton(comparisons, arms):
    ties = any(s == 0.5 for _, _, s in comparisons)
    dim = arms - 1 + int(ties)

    def coef(a):
        v = np.zeros(dim)
        if a > 0:
            v[a - 1] = 1.0
        return v

    x = np.zeros(dim)
    for _ in range(200):
        g = np.zeros(dim)
        h = np.zeros((dim, dim))
        for i, j, s in comparisons:
            vs = [coef(i), coef(j)]
            if ties:
                vt = (coef(i) + coef(j)) / 2
                vt[-1] = 1.0
                vs.append(vt)
            logits = np.array([v @ x for v in vs])
            p = np.exp(logits - np.logaddexp.reduce(logits))
            o = 2 if s == 0.5 else (1 if s == 1 else 0)
            mu = sum(pk * v for pk, v in zip(p, vs))
            g += vs[o] - mu
            h += sum(pk * np.outer(v, v) for pk, v in zip(p, vs)) - np.outer(mu, mu)
        step = np.linalg.solve(h, g)
        x = x + step
        if np.max(np.abs(step)) < 1e-13:
            break
    theta = np.concatenate([[0.0], x[:arms - 1]])
    return theta - theta.mean(), (math.exp(x[-1]) if ties else None), x, ties


def bradley_terry_cases():
    """Davidson-Bradley-Terry by Newton's method; checked against scipy BFGS on the same likelihood."""
    from scipy.optimize import minimize
    rng = np.random.default_rng(20261104)
    true = np.array([0.0, 0.4, 0.9, -0.5])
    out = []
    for with_ties in (True, False):
        comparisons = []
        for _ in range(120):
            i, j = rng.choice(4, 2, replace=False)
            a, b = math.exp(true[i]), math.exp(true[j])
            c = 0.6 * math.sqrt(a * b) if with_ties else 0.0
            u = rng.random() * (a + b + c)
            s = 0.0 if u < a else (1.0 if u < a + b else 0.5)
            comparisons.append((int(i), int(j), s))
        strength, nu, x, ties = bt_newton(comparisons, 4)
        alt = minimize(lambda z: -bt_loglik(z, comparisons, 4, ties), np.zeros_like(x), method="BFGS", options={"gtol": 1e-10})
        assert np.max(np.abs(alt.x - x)) < 1e-5, (alt.x, x)
        out.append({
            "input": {"arms": 4, "comparisons": [[i, j, s] for i, j, s in comparisons]},
            "source": "closed (Davidson-Bradley-Terry MLE by Newton's method; agrees with scipy BFGS within 1e-5)",
            "expected": {"log_strength": [r(v) for v in strength], "tie": None if nu is None else r(nu)},
        })
    return out


def prpl_eb_cs(xs, alpha, c=0.5):
    """Predictable plug-in empirical-Bernstein confidence sequence for a mean in [0, 1]
    (Waudby-Smith & Ramdas 2024, Theorem 2), intersected over time and with [0, 1]."""
    log_term = math.log(2 / alpha)
    mu_prev, var_prev = 0.5, 0.25
    sum_x = sum_sq = 0.0
    lam_sum = lam_x = pen = 0.0
    lo, hi = 0.0, 1.0
    out = []
    for k, x in enumerate(xs):
        t = k + 1
        lam = min(math.sqrt(2 * log_term / (var_prev * t * math.log(1 + t))), c)
        v = 4 * (x - mu_prev) ** 2
        psi = (-math.log(1 - lam) - lam) / 4
        lam_sum += lam
        lam_x += lam * x
        pen += v * psi
        centre, radius = lam_x / lam_sum, (log_term + pen) / lam_sum
        lo, hi = max(lo, centre - radius), min(hi, centre + radius)
        out.append((lo, max(hi, lo)))
        sum_x += x
        mu = (0.5 + sum_x) / (t + 1)
        sum_sq += (x - mu) ** 2
        mu_prev, var_prev = mu, (0.25 + sum_sq) / (t + 1)
    return out


def confidence_sequence_cases():
    rng = np.random.default_rng(20261105)
    out = []
    for name, xs in (("bernoulli_0.7", (rng.random(200) < 0.7).astype(float)),
                     ("beta_2_5", rng.beta(2, 5, 200))):
        xs = [r(v) for v in xs]
        cs = prpl_eb_cs(xs, 0.05)
        out.append({
            "input": {"name": name, "x": xs, "level": 0.95},
            "source": "closed (Waudby-Smith & Ramdas 2024 PrPl-EB, Theorem 2, c = 1/2)",
            "expected": {f"t{t}": [r(cs[t - 1][0]), r(cs[t - 1][1])] for t in (1, 10, 50, 200)},
        })
    return out


def obuchowski_cases():
    """Clustered AUC variance (Obuchowski 1997): DeLong components summed per cluster."""
    rng = np.random.default_rng(20261106)
    pos, neg = [], []
    for c in range(12):
        shift = rng.normal(0, 0.5)
        for _ in range(rng.integers(1, 4)):
            pos.append((round(float(rng.normal(1 + shift, 1)), 3), c))
        for _ in range(rng.integers(1, 4)):
            neg.append((round(float(rng.normal(shift, 1)), 3), c))
    psi = lambda x, y: 1.0 if x > y else (0.5 if x == y else 0.0)
    m, n, k = len(pos), len(neg), 12
    v10 = [sum(psi(x, y) for y, _ in neg) / n for x, _ in pos]
    v01 = [sum(psi(x, y) for x, _ in pos) / m for y, _ in neg]
    a = sum(v10) / m
    c10, c01, mk, nk = np.zeros(k), np.zeros(k), np.zeros(k), np.zeros(k)
    for v, (_, c) in zip(v10, pos):
        c10[c] += v
        mk[c] += 1
    for v, (_, c) in zip(v01, neg):
        c01[c] += v
        nk[c] += 1
    d10, d01 = c10 - mk * a, c01 - nk * a
    s10 = k / ((k - 1) * m) * (d10 ** 2).sum()
    s01 = k / ((k - 1) * n) * (d01 ** 2).sum()
    s11 = k / (k - 1) * (d10 * d01).sum()
    var = s10 / m + s01 / n + 2 * s11 / (m * n)
    return [{
        "input": {"pos": [[s, c] for s, c in pos], "neg": [[s, c] for s, c in neg]},
        "source": "closed (Obuchowski 1997 clustered DeLong variance)",
        "expected": {"auc": r(a), "variance": r(var)},
    }]


def gate_rule_cases():
    """Each gate rule passing and failing (SPEC §8.3, D-034): bounds come from intervals.
    The difference rules read the difference oriented so that positive is better and are
    strict; better_by also needs the point difference to reach min_effect."""
    def judge(rule, value, min_effect, e):
        m = min_effect or 0.0
        if rule == "lower_bound_at_least":
            return e["lo"] >= value
        if rule == "upper_bound_at_most":
            return e["hi"] <= value
        if rule == "better_by":
            return e["lo"] > value and e["point"] >= m
        if rule == "not_worse_than":
            return e["lo"] > -value
        if rule == "pass":
            return e["verdict"]
    rows = [
        ("lower_bound_at_least", 0.7, None, {"kind": "value", "point": 0.8, "lo": 0.72, "hi": 0.88}),
        ("lower_bound_at_least", 0.7, None, {"kind": "value", "point": 0.8, "lo": 0.68, "hi": 0.9}),
        ("upper_bound_at_most", 0.05, None, {"kind": "value", "point": 0.02, "lo": 0.0, "hi": 0.04}),
        ("upper_bound_at_most", 0.05, None, {"kind": "value", "point": 0.03, "lo": 0.0, "hi": 0.06}),
        ("better_by", 0.0, 0.02, {"kind": "delta", "point": 0.05, "lo": 0.01, "hi": 0.09}),
        ("better_by", 0.0, 0.02, {"kind": "delta", "point": 0.015, "lo": 0.001, "hi": 0.03}),
        ("better_by", 0.0, None, {"kind": "delta", "point": 0.04, "lo": -0.01, "hi": 0.09}),
        # A candidate no different from the baseline: a zero-width interval at 0.
        ("better_by", 0.0, None, {"kind": "delta", "point": 0.0, "lo": 0.0, "hi": 0.0}),
        ("not_worse_than", 0.05, None, {"kind": "delta", "point": 0.0, "lo": -0.03, "hi": 0.03}),
        ("not_worse_than", 0.05, None, {"kind": "delta", "point": -0.02, "lo": -0.07, "hi": 0.03}),
        ("pass", 0.0, None, {"kind": "verdict", "verdict": True}),
        ("pass", 0.0, None, {"kind": "verdict", "verdict": False}),
    ]
    out = []
    for rule, value, me, e in rows:
        out.append({"input": {"rule": rule, "value": value, "min_effect": me, "evidence": e},
                    "source": "closed (SPEC §8.3 rule definitions)", "expected": {"passed": judge(rule, value, me, e)}})
    # Missing rate: 6 of 300 judgments, one-sided at 1 - 0.05/5 (exact upper bound).
    k, n, level = 6, 300, 1 - 0.05 / 5
    out.append({"input": {"missing_rate": {"missing": k, "judgments": n, "one_sided_level": level}},
                "source": f"scipy {scipy.__version__} beta.ppf (Clopper-Pearson upper bound)",
                "expected": {"upper": r(stats.beta.ppf(level, k + 1, n - k))}})
    return out


def lr_cases():
    """Feature likelihood ratio LR = P(fire|pos)/P(fire|neg) with the Katz log interval.
    If a or b is zero, 0.5 is added to all four cells of the 2x2 table (Haldane-Anscombe; denominators +1)."""
    out = []
    for a, n1, b, n0 in [(12, 50, 3, 50), (30, 40, 30, 60), (0, 40, 5, 60), (7, 20, 0, 80)]:
        aa, bb, m1, m0 = (a, b, n1, n0) if a and b else (a + 0.5, b + 0.5, n1 + 1, n0 + 1)
        lr = (aa / m1) / (bb / m0)
        se = math.sqrt(1 / aa - 1 / m1 + 1 / bb - 1 / m0)
        exp = {"lr": r(lr), "katz95": [r(lr * math.exp(-Z95 * se)), r(lr * math.exp(Z95 * se))]}
        src = "closed (Katz 1978 log method; zero cells get +0.5 on all four cells)"
        if a and b:
            rr = stats.contingency.relative_risk(a, n1, b, n0)
            ci = rr.confidence_interval(0.95)
            assert abs(rr.relative_risk - lr) < 1e-12 and abs(ci.low - exp["katz95"][0]) < 1e-9, (a, b)
            src += f"; checked against scipy {scipy.__version__} contingency.relative_risk"
        out.append({"input": {"hit_pos": a, "n_pos": n1, "hit_neg": b, "n_neg": n0}, "source": src, "expected": exp})
    return out


def mcnemar_cases():
    out = []
    for b, c in [(0, 0), (1, 9), (5, 15), (10, 12), (3, 0), (40, 60)]:
        n = b + c
        if n == 0:
            p, mid = 1.0, 1.0
        else:
            p = stats.binomtest(b, n, 0.5).pvalue
            m = min(b, c)
            mid = min(1.0, 2 * (stats.binom.cdf(m, n, 0.5) - 0.5 * stats.binom.pmf(m, n, 0.5)))
        out.append({"input": {"b": b, "c": c}, "source": f"scipy {scipy.__version__} binomtest (exact binomial)",
                    "expected": {"exact_two_sided_p": r(p), "mid_p_two_sided": r(mid)}})
    return out


def cohen_kappa(table):
    t = np.asarray(table, dtype=float)
    n = t.sum()
    po = np.trace(t) / n
    pe = (t.sum(0) * t.sum(1)).sum() / n / n
    return (po - pe) / (1 - pe)


def krippendorff_alpha(units, level="nominal"):
    """units: values per unit (None for missing). Alpha = 1 - (n-1) sum(o*d) / sum(nc*nk*d)
    from the coincidence matrix, with the nominal, ordinal or interval difference d."""
    vals = sorted({v for u in units if sum(v is not None for v in u) >= 2 for v in u if v is not None})
    idx = {v: i for i, v in enumerate(vals)}
    o = np.zeros((len(vals), len(vals)))
    for u in units:
        xs = [v for v in u if v is not None]
        m = len(xs)
        if m < 2:
            continue
        for i, x in enumerate(xs):
            for j, y in enumerate(xs):
                if i != j:
                    o[idx[x], idx[y]] += 1 / (m - 1)
    nc = o.sum(1)
    n = nc.sum()
    q = len(vals)
    d = np.zeros((q, q))
    for c in range(q):
        for k in range(q):
            if level == "nominal":
                d[c, k] = float(c != k)
            elif level == "ordinal":
                lo, hi = min(c, k), max(c, k)
                d[c, k] = (nc[lo:hi + 1].sum() - (nc[c] + nc[k]) / 2) ** 2
            else:
                d[c, k] = (vals[c] - vals[k]) ** 2
    return 1 - (n - 1) * (o * d).sum() / (np.outer(nc, nc) * d).sum()

def agreement_cases():
    # Example from Krippendorff (2011) "Computing Krippendorff's Alpha-Reliability":
    # 4 coders x 12 units with missing values; alpha = 0.743 nominal, 0.815 ordinal,
    # 0.849 interval.
    A = [1, 2, 3, 3, 2, 1, 4, 1, 2, None, None, None]
    B = [1, 2, 3, 3, 2, 2, 4, 1, 2, 5, None, 3]
    C = [None, 3, 3, 3, 2, 3, 4, 2, 2, 5, 1, None]
    D = [1, 2, 3, 3, 2, 4, 4, 1, 2, 5, 1, None]
    units = list(zip(A, B, C, D))
    alpha = krippendorff_alpha(units)
    ordinal = krippendorff_alpha(units, "ordinal")
    interval = krippendorff_alpha(units, "interval")
    assert (round(alpha, 3), round(ordinal, 3), round(interval, 3)) == (0.743, 0.815, 0.849)
    return [
        {"input": {"confusion": [[20, 5], [10, 15]]}, "source": "closed (po = 0.7, pe = 0.5, so kappa = 0.4 by hand)",
         "expected": {"cohen_kappa": r(cohen_kappa([[20, 5], [10, 15]]))}},
        {"input": {"coders": {"A": A, "B": B, "C": C, "D": D}}, "source": "literature (Krippendorff 2011 example; alpha = 0.743 nominal, 0.815 ordinal, 0.849 interval)",
         "expected": {"krippendorff_alpha_nominal": r(alpha), "literature_value_3dp": 0.743,
                      "krippendorff_alpha_ordinal": r(ordinal), "krippendorff_alpha_interval": r(interval)}},
    ]


def misc_cases():
    w = [1, 1, 2, 2, 4]
    out = [{"input": {"weights": w}, "source": "closed (Kish effective sample size (sum w)^2 / sum w^2)",
            "expected": {"kish_n_eff": r(sum(w) ** 2 / sum(x * x for x in w))}}]
    for c_fp, c_fn, prev in [(1, 3, 0.1), (1, 10, 0.1), (1, 3, 0.5)]:
        out.append({
            "input": {"cost_fp": c_fp, "cost_fn": c_fn, "prevalence": prev},
            "source": "closed (cost-minimising decision: posterior and likelihood ratio thresholds)",
            "expected": {"posterior_threshold": r(c_fp / (c_fp + c_fn)),
                         "lr_threshold": r((c_fp / c_fn) * (1 - prev) / prev)},
        })
    for u in [0.05, 0.01]:
        out.append({
            "input": {"upper_bound": u, "confidence_one_sided": 0.95},
            "source": "closed (items needed for a one-sided upper bound below u with zero events)",
            "expected": {"n_exact": math.ceil(math.log(0.05) / math.log(1 - u)),
                         "n_wilson": math.ceil(Z95_ONE ** 2 * (1 - u) / u)},
        })
    return out


def at_k_closed(c, K, k, rule):
    """Value of a k-run rule for an item positive in c of K replicates, drawing k without replacement."""
    tot = math.comb(K, k)
    if rule == "any":
        return 1 - math.comb(K - c, k) / tot
    if rule == "all":
        return math.comb(c, k) / tot
    s = 0.0
    for j in range(k + 1):
        w = math.comb(c, j) * math.comb(K - c, k - j) / tot
        s += w if 2 * j > k else (0.5 * w if 2 * j == k else 0.0)
    return s


def at_k_brute(c, K, k, rule):
    """The same value by enumerating every k-subset (an independent check of the closed form)."""
    from itertools import combinations
    obs = [1] * c + [0] * (K - c)
    vals = []
    for sub in combinations(range(K), k):
        j = sum(obs[i] for i in sub)
        if rule == "any":
            vals.append(1.0 if j > 0 else 0.0)
        elif rule == "all":
            vals.append(1.0 if j == k else 0.0)
        else:
            vals.append(1.0 if 2 * j > k else (0.5 if 2 * j == k else 0.0))
    return sum(vals) / len(vals)


def estimand_cases():
    out = []
    for c, K in [(0, 5), (2, 5), (3, 5), (5, 5), (4, 10), (7, 10)]:
        exp = {}
        for rule in ("majority", "any", "all"):
            for k in (1, 2, 3, K):
                a, b = at_k_closed(c, K, k, rule), at_k_brute(c, K, k, rule)
                assert abs(a - b) < 1e-12, (c, K, k, rule)
                exp[f"{rule}@{k}"] = r(a)
        out.append({"input": {"positives": c, "replicates": K},
                    "source": "closed (hypergeometric sum; checked by enumerating every k-subset)", "expected": exp})
    return out


def agreement_extra_cases():
    """Gwet's AC1 and positive/negative agreement (two raters, binary). Table [[a, b], [c, d]]; a = both positive."""
    out = []
    for t in [[[40, 9], [6, 45]], [[5, 5], [5, 85]], [[1, 4], [3, 92]]]:
        (a, b), (c, d) = t
        n = a + b + c + d
        pa = (a + d) / n
        pi = ((a + b) + (a + c)) / (2 * n)  # share of positives across both raters
        pe = 2 * pi * (1 - pi)
        out.append({"input": {"table": t},
                    "source": "closed (Gwet 2008 AC1, positive and negative agreement)",
                    "expected": {"cohen_kappa": r(cohen_kappa(t)), "gwet_ac1": r((pa - pe) / (1 - pe)),
                                 "positive_agreement": r(2 * a / (2 * a + b + c)),
                                 "negative_agreement": r(2 * d / (2 * d + b + c))}})
    return out


def score_cases():
    """Score-type metrics (value_semantics = score): the weighted mean of per-item
    scores, the weighted share at or above t, and pass@k from c passing replicates
    out of n (the unbiased estimator 1 - C(n-c, k) / C(n, k))."""
    out = []
    for scores, weights, t in [
        ([0.9, 0.4, 0.75, 0.2, 1.0], [1, 1, 1, 1, 1], 0.5),
        ([0.9, 0.4, 0.75, 0.2, 1.0], [2, 1, 1, 3, 1], 0.75),
        ([0.0, 0.0, 1.0], [1, 1, 1], 1.0),
    ]:
        w = np.array(weights, dtype=float)
        x = np.array(scores, dtype=float)
        out.append({"input": {"scores": scores, "weights": weights, "t": t},
                    "source": "closed (weighted mean and weighted share at or above t)",
                    "expected": {"mean_score": r(float(np.sum(w * x) / np.sum(w))),
                                 "rate_at_least": r(float(np.sum(w * (x >= t)) / np.sum(w)))}})
    for c, n, k in [(0, 10, 1), (3, 10, 1), (3, 10, 5), (10, 10, 3), (1, 20, 10)]:
        exact = 1 - math.comb(n - c, k) / math.comb(n, k)
        out.append({"input": {"passing": c, "replicates": n, "k": k},
                    "source": "closed (Chen et al. 2021 unbiased pass@k)",
                    "expected": {"pass_at_k": r(exact)}})
    return out


def tango_interval(b, c, n, z=Z95):
    """Tango (1998) score interval for p1 - p2 of paired proportions, where b counts
    pairs positive only in arm 1 and c only in arm 2. The constrained maximum-likelihood
    estimate of the (0, 1) cell is found numerically here (a root of the score function),
    independently of the closed form used by the Rust implementation."""
    from scipy.optimize import brentq

    rest = n - b - c

    def q21(delta):
        # The log-likelihood is concave in q on (max(0, -delta), (1 - delta) / 2): find
        # where its derivative crosses zero, or take the boundary it points to.
        lo = max(0.0, -delta)
        hi = (1.0 - delta) / 2.0

        w = hi - lo

        # Parametrised by t = q - lo in (0, w), so that q + delta, q and hi - q keep
        # their precision near either end.
        def score(t):
            v = 0.0
            if b:
                v += b / (t + max(delta, 0.0))
            if c:
                v += c / (lo + t)
            if rest:
                v -= rest / (w - t)
            return v

        a, z_ = w * 1e-13, w * (1.0 - 1e-13)
        if score(a) <= 0.0:
            return lo
        if score(z_) >= 0.0:
            return hi
        return lo + brentq(score, a, z_, xtol=1e-300, rtol=4 * np.finfo(float).eps, maxiter=500)

    def stat(delta):
        q = q21(delta)
        var = n * (2.0 * q + delta * (1.0 - delta))
        return (b - c - n * delta) / math.sqrt(var)

    eps = 1e-12
    lower = -1.0 if stat(-1.0 + eps) <= z else brentq(lambda d: stat(d) - z, -1.0 + eps, (b - c) / n, xtol=1e-15)
    upper = 1.0 if stat(1.0 - eps) >= -z else brentq(lambda d: stat(d) + z, (b - c) / n, 1.0 - eps, xtol=1e-15)
    return lower, upper


def paired_cases():
    out = []
    for b, c, n in [(5, 15, 100), (10, 12, 60), (0, 9, 40), (3, 0, 25), (40, 60, 1000)]:
        lo, hi = tango_interval(b, c, n)
        out.append({"input": {"b": b, "c": c, "n": n},
                    "source": f"scipy {scipy.__version__} (Tango 1998 score interval; constrained MLE as a numerical root)",
                    "expected": {"difference": r((b - c) / n), "tango95": [r(lo), r(hi)]}})
    return out


def newcombe_cases():
    """Newcombe (1998) method 10 for the difference of two independent proportions."""
    out = []
    for k1, n1, k2, n2 in [(56, 70, 48, 80), (9, 10, 3, 10), (6, 7, 2, 7), (5, 56, 0, 29), (0, 10, 0, 20)]:
        p1, p2 = k1 / n1, k2 / n2
        l1, u1 = wilson(k1, n1, Z95)
        l2, u2 = wilson(k2, n2, Z95)
        d = p1 - p2
        lo = d - math.sqrt((p1 - l1) ** 2 + (u2 - p2) ** 2)
        hi = d + math.sqrt((u1 - p1) ** 2 + (p2 - l2) ** 2)
        case = {"input": {"k1": k1, "n1": n1, "k2": k2, "n2": n2},
                "source": "closed (Newcombe 1998 method 10 from Wilson intervals)",
                "expected": {"newcombe95": [r(lo), r(hi)]}}
        if (k1, n1, k2, n2) == (56, 70, 48, 80):
            case["source"] = "literature (Newcombe 1998, Table II example (a): 0.0524 to 0.3339)"
            case["expected"]["literature_4dp"] = [0.0524, 0.3339]
            assert round(lo, 4) == 0.0524 and round(hi, 4) == 0.3339, (lo, hi)
        out.append(case)
    return out


def _alt_scores(x, others, scale):
    if scale == "nominal":
        return sum(1 for o in others if o == x) / len(others)
    return -math.sqrt(sum((x - o) ** 2 for o in others) / len(others))


def alt_test(humans, system, scale, eps, q=0.05, min_items=30):
    """The alternative annotator test of D-017, after the authors' reference procedure
    (Calderon, Reichart and Dror 2025): per human rater j, on the items j and another
    human labelled, W = 1 when one's agreement with the other humans is at least the
    other labeller's (ties count for both); scipy's one-sided one-sample t-test of
    W_j - W_sys against eps, then scipy's Benjamini-Yekutieli adjustment."""
    tested, skipped = [], {}
    for j in sorted(humans):
        d, ws = [], []
        for item in sorted(humans[j]):
            if item not in system:
                continue
            others = [humans[k][item] for k in sorted(humans) if k != j and item in humans[k]]
            if not others:
                continue
            sj, ss = _alt_scores(humans[j][item], others, scale), _alt_scores(system[item], others, scale)
            d.append(float(sj >= ss) - float(ss >= sj))
            ws.append(float(ss >= sj))
        if len(d) < min_items:
            skipped[j] = len(d)
            continue
        p = stats.ttest_1samp(d, popmean=eps, alternative="less").pvalue
        tested.append({"rater": j, "items": len(d), "mean_difference": r(np.mean(d)),
                       "p_value": r(p), "advantage": r(np.mean(ws))})
    if tested:
        adj = stats.false_discovery_control([t["p_value"] for t in tested], method="by")
        for t, a in zip(tested, adj):
            t["adjusted_p_value"] = r(a)
            t["system_wins"] = bool(a <= q)
    omega = sum(t["system_wins"] for t in tested) / len(tested) if tested else None
    rho = r(np.mean([t["advantage"] for t in tested])) if tested else None
    verdict = "insufficient_data" if len(tested) < 2 else ("human_parity" if omega >= 0.5 else "below_human")
    return {"tested": tested, "skipped": skipped, "winning_rate": omega,
            "advantage_probability": rho, "verdict": verdict}


def alt_test_cases():
    """Synthetic panels (the authors' data carry no licence): 4 raters label 60 items
    each, and the system is either a fifth rater of the same quality or a weaker one."""
    rng = np.random.default_rng(20261106)
    out = []

    def panel(n_items, n_raters, levels, accuracy, system_accuracy):
        truth = rng.integers(0, levels, n_items)
        def label(acc):
            noise = rng.integers(0, levels, n_items)
            keep = rng.random(n_items) < acc
            return np.where(keep, truth, noise)
        humans = {f"human:{chr(97 + k)}": {f"i{i:03d}": float(v) for i, v in enumerate(label(accuracy))}
                  for k in range(n_raters)}
        system = {f"i{i:03d}": float(v) for i, v in enumerate(label(system_accuracy))}
        return humans, system

    for name, items, raters, levels, acc, sys_acc, scale in [
        ("binary, system as good as a rater", 60, 4, 2, 0.8, 0.8, "nominal"),
        ("binary, weaker system", 60, 4, 2, 0.8, 0.45, "nominal"),
        ("four ordered levels, system as good as a rater", 60, 4, 4, 0.7, 0.75, "graded"),
        ("two raters", 60, 2, 2, 0.85, 0.85, "nominal"),
        ("too few shared items", 20, 3, 2, 0.8, 0.8, "nominal"),
    ]:
        humans, system = panel(items, raters, levels, acc, sys_acc)
        out.append({"name": name,
                    "input": {"humans": humans, "system": system, "scale": scale, "epsilon": 0.1},
                    "source": "scipy (ttest_1samp alternative=less, false_discovery_control method=by)",
                    "expected": alt_test(humans, system, scale, 0.1)})
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=OUT)
    out = ap.parse_args().out
    doc = {
        "schema": "reference-values/1",
        "generated_with": {"python": sys.version.split()[0], "scipy": scipy.__version__, "numpy": np.__version__},
        "tolerance": {"abs": 1e-9, "note": "Rust tests require agreement within this absolute tolerance, allowing for differences in normal quantile implementations"},
        "cases": {
            "proportion_interval": proportion_cases(),
            "sdt_binary": sdt_cases(),
            "ordinal_roc": ordinal_cases(),
            "delong": delong_cases(),
            "calibration": calibration_cases(),
            "cost": cost_cases(),
            "feature_likelihood_ratio": lr_cases(),
            "feature_likelihood_ratio_bh": lr_bh_cases(),
            "mcnemar": mcnemar_cases(),
            "agreement": agreement_cases(),
            "agreement_prevalence_robust": agreement_extra_cases(),
            "agreement_ordinal": agreement_ordinal_cases(),
            "ppi_mean": ppi_cases(),
            "bradley_terry": bradley_terry_cases(),
            "confidence_sequence": confidence_sequence_cases(),
            "auc_clustered": obuchowski_cases(),
            "gate_rules": gate_rule_cases(),
            "estimand_at_k": estimand_cases(),
            "misc": misc_cases(),
            "score": score_cases(),
            "paired_difference": paired_cases(),
            "unpaired_difference": newcombe_cases(),
            "alt_test": alt_test_cases(),
        },
    }
    out.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    n = sum(len(v) for v in doc["cases"].values())
    ka = doc["cases"]["agreement"][1]["expected"]["krippendorff_alpha_nominal"]
    print(f"wrote {out} ({n} cases). krippendorff alpha = {ka}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
