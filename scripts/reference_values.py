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


def krippendorff_alpha_nominal(units):
    """units: values per unit (None for missing). Alpha from the coincidence matrix."""
    vals = sorted({v for u in units for v in u if v is not None})
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
    do = o.sum() - np.trace(o)
    de = (np.outer(nc, nc).sum() - (nc * nc).sum()) / (n - 1)
    return 1 - do / de


def agreement_cases():
    # Example from Krippendorff (2011) "Computing Krippendorff's Alpha-Reliability":
    # 4 coders x 12 units with missing values; nominal alpha = 0.743.
    A = [1, 2, 3, 3, 2, 1, 4, 1, 2, None, None, None]
    B = [1, 2, 3, 3, 2, 2, 4, 1, 2, 5, None, 3]
    C = [None, 3, 3, 3, 2, 3, 4, 2, 2, 5, 1, None]
    D = [1, 2, 3, 3, 2, 4, 4, 1, 2, 5, 1, None]
    units = list(zip(A, B, C, D))
    alpha = krippendorff_alpha_nominal(units)
    assert round(alpha, 3) == 0.743, alpha
    return [
        {"input": {"confusion": [[20, 5], [10, 15]]}, "source": "closed (po = 0.7, pe = 0.5, so kappa = 0.4 by hand)",
         "expected": {"cohen_kappa": r(cohen_kappa([[20, 5], [10, 15]]))}},
        {"input": {"coders": {"A": A, "B": B, "C": C, "D": D}}, "source": "literature (Krippendorff 2011 example; nominal alpha = 0.743)",
         "expected": {"krippendorff_alpha_nominal": r(alpha), "literature_value_3dp": 0.743}},
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
            "feature_likelihood_ratio": lr_cases(),
            "mcnemar": mcnemar_cases(),
            "agreement": agreement_cases(),
            "agreement_prevalence_robust": agreement_extra_cases(),
            "estimand_at_k": estimand_cases(),
            "misc": misc_cases(),
            "score": score_cases(),
            "paired_difference": paired_cases(),
            "unpaired_difference": newcombe_cases(),
        },
    }
    out.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    n = sum(len(v) for v in doc["cases"].values())
    ka = doc["cases"]["agreement"][1]["expected"]["krippendorff_alpha_nominal"]
    print(f"wrote {out} ({n} cases). krippendorff alpha = {ka}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
