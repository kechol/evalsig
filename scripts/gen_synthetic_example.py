#!/usr/bin/env python3
"""Generate examples/synthetic/records.jsonl: a fictional binary triage task large
enough for intervals and verdicts.

300 items in 60 clusters of 5, one or two human raters per item, and two runs of a
classifier: a baseline and a candidate that fixes some of the baseline's errors and
makes a few new ones. Everything is drawn from a fixed seed, so the file is the same
on every run. Standard library only.

Usage: python3 scripts/gen_synthetic_example.py [--out PATH]
"""
import argparse
import json
import random
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "examples" / "synthetic" / "records.jsonl"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=OUT)
    args = ap.parse_args()
    rng = random.Random(20261103)
    lines = [
        {"record": "run", "run_id": "triage-v1", "system": {"id": "triage", "version": "prompt-v1"},
         "value_semantics": "decision", "output_type": "binary",
         "grader": {"id": "rule-check", "version": "1", "kind": "rule"}},
        {"record": "run", "run_id": "triage-v2", "system": {"id": "triage", "version": "prompt-v2"},
         "value_semantics": "decision", "output_type": "binary",
         "grader": {"id": "rule-check", "version": "1", "kind": "rule"}},
    ]
    for i in range(300):
        item = f"t-{i:04d}"
        cluster = f"thread-{i // 5:03d}"
        truth = rng.random() < 0.3
        lines.append({"record": "item", "item_id": item, "cluster": cluster,
                      "strata": {"channel": "email" if i % 3 else "chat"}})
        lines.append({"record": "label", "item_id": item, "rater_id": "rater:a", "source": "human",
                      "value": truth, "version": 1})
        if i % 4 == 0:
            disagree = rng.random() < 0.1
            lines.append({"record": "label", "item_id": item, "rater_id": "rater:b", "source": "human",
                          "value": truth != disagree, "version": 1})
        base_right = rng.random() < 0.80
        base = truth if base_right else not truth
        if base_right:
            cand = base if rng.random() < 0.96 else not base
        else:
            cand = truth if rng.random() < 0.55 else base
        for run, value in (("triage-v1", base), ("triage-v2", cand)):
            if run == "triage-v1" and i % 50 == 7:
                lines.append({"record": "judgment", "item_id": item, "run_id": run, "gen_replicate": 0,
                              "output_type": "binary", "missing": {"reason": "timeout", "detail": "no reply in 30s"}})
            else:
                lines.append({"record": "judgment", "item_id": item, "run_id": run, "gen_replicate": 0,
                              "output_type": "binary", "value": value})
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("".join(json.dumps(x, separators=(",", ":")) + "\n" for x in lines), encoding="utf-8")
    print(f"wrote {args.out} ({len(lines)} records)")


if __name__ == "__main__":
    main()
