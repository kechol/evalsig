"""Why rates of rare events (missing, disagreement and false-alarm rates) get exact
upper bounds rather than bootstrap ones (metrics spec §1).

At a true rate of 0.01 or 0.005 over n = 300 items, count how often the one-sided 99%
upper bound (α/B = 0.01) falls below the true rate, which in a gate condition such as
`missing_rate <= u` is a false pass: the percentile bootstrap against Clopper-Pearson.
With no event observed the bootstrap bound degenerates to 0.

    uv run --project scripts scripts/sim_rare_rate_bounds.py
"""

import numpy as np
from scipy.stats import beta

rng = np.random.default_rng(31)
n, Bt, D = 300, 2000, 20000
for p in (0.01, 0.005):
    fail_boot = fail_cp = zero = 0
    for _ in range(D):
        k = rng.binomial(n, p)
        zero += k == 0
        bs = rng.binomial(n, k / n, Bt) / n
        fail_boot += np.quantile(bs, 0.99) < p
        cp = 1.0 if k == n else beta.ppf(0.99, k + 1, n - k)
        fail_cp += cp < p
    print(
        f"true rate={p}: P(k=0)={zero / D:.3f}  P(boot 99% upper < true)={fail_boot / D:.3f}"
        f"  P(CP 99% upper < true)={fail_cp / D:.4f}"
    )
print("CP 99% upper at k=0, n=300 =", round(1 - 0.01 ** (1 / 300), 4), "; bootstrap upper at k=0 = 0.0")
