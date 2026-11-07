"""Why compare does not control the error over a sequence of comparisons with LORD++
(metrics spec §7.2): with the default γ sequence the per-test level drops below 1% from
the first test, so a few hundred items cannot detect any realistic difference.

Prints LORD++'s α_t (Javanmard and Montanari 2018; Ramdas et al. 2017) for a target
q = 0.1 and initial wealth W0 = q/2 and q/10, with no rejection and with rejections at
tests 1 and 2, then the smallest paired accuracy difference detectable at 80% power
for a one-sided level α when the difference's standard error is 0.0183 (300 items, 10%
discordant).

    uv run --project scripts scripts/sim_lord.py
"""

import numpy as np
from scipy.stats import norm

q = 0.1
T = 60
j = np.arange(1, T + 1)
# The constant 0.07720838 makes the infinite sum of γ 1, as in onlineFDR.
gamma = 0.07720838 * np.log(np.maximum(j, 2)) / (j * np.exp(np.sqrt(np.log(j))))
print("gamma_1..5 =", np.round(gamma[:5], 5), " sum(1..60) =", gamma[:60].sum().round(4))


def lord_pp(rejections, w0):
    # α_t = γ_t·W0 + (q − W0)·γ_{t−τ1} + q·Σ_{j≥2} γ_{t−τj}
    out = []
    for t in range(1, T + 1):
        a = gamma[t - 1] * w0
        taus = [r for r in rejections if r < t]
        if taus:
            a += (q - w0) * gamma[t - taus[0] - 1]
            for r in taus[1:]:
                a += q * gamma[t - r - 1]
        out.append(a)
    return np.array(out)


for w0 in (q / 2, q / 10):
    a = lord_pp([], w0)
    print(
        f"W0={w0}: no rejections  α_1={a[0]:.5f} α_2={a[1]:.5f} α_3={a[2]:.5f}"
        f" α_5={a[4]:.5f} α_10={a[9]:.5f} α_20={a[19]:.5f}"
    )
    a = lord_pp([1, 2], w0)
    print(f"W0={w0}: rejected at 1,2  α_3={a[2]:.5f}")
for al in (0.05, 0.005, 0.0005, 0.0001):
    print(
        f"α={al}: z={norm.ppf(1 - al):.2f},"
        f" min detectable Δ (SE=0.0183, power 0.8) = {(norm.ppf(1 - al) + norm.ppf(0.8)) * 0.0183:.3f}"
    )
