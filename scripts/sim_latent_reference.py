"""Bias and interval coverage of the `soft` and `latent` references (D-016).

A synthetic panel labels every item (a crossed design). Each item has a true class;
the system judges it with a fixed hit rate and false-alarm rate against that class.
Raters follow their own sensitivity and specificity, except on "hard" items, where
every rater says positive with probability one half whatever the class. The target is
the system's hit rate and false-alarm rate against the true class on the sample.

- soft:   H = sum(s*y) / sum(s), s the share of raters saying positive.
- latent: the same with s replaced by the Dawid-Skene posterior (binary EM from the
          majority share, re-estimated in every bootstrap resample).

The interval is a 95% percentile bootstrap over items. Output: a Markdown table of
the mean bias and coverage for each condition.

    uv run --project scripts scripts/sim_latent_reference.py [--sims N] [--boot B]
"""

import argparse
import itertools

import numpy as np

N_ITEMS = 300
PREVALENCE = 0.3
SYSTEM_H, SYSTEM_F = 0.80, 0.10
BASE_ACCURACY = 0.85
EM_ITERATIONS = 100
CLIP = 1e-3


def dawid_skene(labels):
    """Posterior P(positive) per item. labels: (..., items, raters) of 0/1."""
    q = labels.mean(axis=-1)
    for _ in range(EM_ITERATIONS):
        pi = q.mean(axis=-1, keepdims=True).clip(CLIP, 1 - CLIP)
        qe = q[..., None]
        se = ((qe * labels).sum(-2) / qe.sum(-2).clip(CLIP)).clip(CLIP, 1 - CLIP)
        sp = (((1 - qe) * (1 - labels)).sum(-2) / (1 - qe).sum(-2).clip(CLIP)).clip(CLIP, 1 - CLIP)
        se, sp = se[..., None, :], sp[..., None, :]
        log_pos = np.log(pi) + (labels * np.log(se) + (1 - labels) * np.log(1 - se)).sum(-1)
        log_neg = np.log(1 - pi) + (labels * np.log(1 - sp) + (1 - labels) * np.log(sp)).sum(-1)
        q = 1 / (1 + np.exp(log_neg - log_pos))
    return q


def rates(share, judged):
    """Hit rate and false-alarm rate against a (soft) reference share."""
    h = (share * judged).sum(-1) / share.sum(-1)
    f = ((1 - share) * judged).sum(-1) / (1 - share).sum(-1)
    return h, f


def one_dataset(rng, raters, bias, hard_share):
    truth = rng.random(N_ITEMS) < PREVALENCE
    judged = np.where(truth, rng.random(N_ITEMS) < SYSTEM_H, rng.random(N_ITEMS) < SYSTEM_F).astype(float)
    # Raters alternate lenient (higher sensitivity) and strict (higher specificity).
    tilt = np.where(np.arange(raters) % 2 == 0, bias, -bias)
    se, sp = BASE_ACCURACY + tilt, BASE_ACCURACY - tilt
    u = rng.random((N_ITEMS, raters))
    labels = np.where(truth[:, None], u < se, u >= sp)
    hard = rng.random(N_ITEMS) < hard_share
    labels[hard] = rng.random((hard.sum(), raters)) < 0.5
    return truth.astype(float), judged, labels.astype(float)


def simulate(raters, bias, hard_share, sims, boot, seed):
    rng = np.random.default_rng(seed)
    out = {(m, k): [] for m in ("soft", "latent") for k in ("h_bias", "f_bias", "h_cover", "f_cover")}
    for _ in range(sims):
        truth, judged, labels = one_dataset(rng, raters, bias, hard_share)
        target_h, target_f = rates(truth, judged)
        draw = rng.integers(0, N_ITEMS, (boot, N_ITEMS))
        for mode in ("soft", "latent"):
            share = labels.mean(-1) if mode == "soft" else dawid_skene(labels)
            h, f = rates(share, judged)
            boot_share = labels[draw].mean(-1) if mode == "soft" else dawid_skene(labels[draw])
            bh, bf = rates(boot_share, judged[draw])
            for name, point, reps, target in (("h", h, bh, target_h), ("f", f, bf, target_f)):
                lo, hi = np.nanquantile(reps, [0.025, 0.975])
                out[(mode, f"{name}_bias")].append(point - target)
                out[(mode, f"{name}_cover")].append(lo <= target <= hi)
    return {k: float(np.mean(v)) for k, v in out.items()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sims", type=int, default=200)
    ap.add_argument("--boot", type=int, default=200)
    ap.add_argument("--seed", type=int, default=20261103)
    a = ap.parse_args()
    print(f"N = {N_ITEMS} items, prevalence {PREVALENCE}, system H {SYSTEM_H} F {SYSTEM_F}, "
          f"rater accuracy {BASE_ACCURACY} ± bias, {a.sims} datasets, {a.boot} resamples\n")
    print("| raters/item | bias | hard share | soft H bias | latent H bias | soft F bias | latent F bias "
          "| soft H cover | latent H cover | soft F cover | latent F cover |")
    print("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for i, (raters, bias, hard) in enumerate(itertools.product((3, 5), (0.0, 0.1), (0.0, 0.2))):
        r = simulate(raters, bias, hard, a.sims, a.boot, a.seed + i)
        print(f"| {raters} | {bias} | {hard} | {r['soft', 'h_bias']:+.3f} | {r['latent', 'h_bias']:+.3f} "
              f"| {r['soft', 'f_bias']:+.3f} | {r['latent', 'f_bias']:+.3f} | {r['soft', 'h_cover']:.2f} "
              f"| {r['latent', 'h_cover']:.2f} | {r['soft', 'f_cover']:.2f} | {r['latent', 'f_cover']:.2f} |")


if __name__ == "__main__":
    main()
