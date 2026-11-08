### evalsig compare

**`improved`** — accuracy: difference +0.0533 (better is positive; 95% interval +0.0133 to +0.0933), so the candidate improved.

`triage-v1` → `triage-v2` on 300 paired items (0 unpaired)

| Metric | Baseline | Candidate | Difference (better +) | Interval | Verdict |
|---|---:|---:|---:|---|---|
| accuracy | 0.7583 | 0.8117 | +0.0533 | +0.0133 to +0.0933 | `improved` |
| hit_rate | 0.7268 | 0.7541 | +0.0273 | -0.0479 to +0.1041 | `indistinguishable` |
| false_alarm_rate | 0.2278 | 0.1631 | +0.0647 | +0.0195 to +0.1138 | `improved` |
| precision | 0.5938 | 0.6699 | +0.0762 | +0.0177 to +0.1386 | `improved` |

New hits 9 · lost hits 6.50 · new false alarms 7 · cleared false alarms 20.50

Comparison 1 on this split.

> **Warning** `missing_rate_differs`: system-side missing rates differ: 6/300 vs 0/300 (95% interval of the difference 0.003 to 0.043)


<sub>evalsig <VERSION> · 1 snapshot(s) · config blake3:7888712e78ee · seed 20261103</sub>
