#!/usr/bin/env python3
"""Map the records of a custom evaluation harness to evalsig's canonical format (records/1).

A harness that uses neither Inspect nor promptfoo can still use evalsig by writing
canonical records. The input of this example is a log of two independent runs of a
ticket triage (one line per "ticket x task x two judgments (a, b)") and a JSON file of
final human decisions as {task: {ticket ID: bool}}. See run-log.sample.jsonl and
labels.sample.json (synthetic data) for the shapes.

How it maps:
  run       ... value semantics `decision` (system decisions compared with human
                labels); the output type is a 4-level ordinal
  item      ... one item per (task, ticket ID); the ticket ID is the cluster
                (the tasks of one ticket are correlated, so they are resampled and
                split together)
  judgment  ... runs a and b become generation replicates 0 and 1; the 4-level
                conclusion maps to the ordinal output
  features  ... rules that apply become features, fired=true when they fired
  missing   ... a line without one run's output makes that run missing, with a reason
  label     ... the final human decision is the reference label

Usage: python3 convert_runlog.py run-log.sample.jsonl labels.sample.json --run-id triage-v1 > records.jsonl
Standard library only. `examples/mapping.toml` declares the same mapping.
"""
import argparse
import json
import sys

LEVELS = {"low": 0, "normal": 1, "high": 2, "urgent": 3}
MISSING_REASON = {"provider_error": "error", "timeout": "timeout"}


def records(run_log_lines, labels, run_id, system_version, none_as):
    yield {"record": "run", "run_id": run_id, "system": {"id": "triage-pipeline", "version": system_version},
           "value_semantics": "decision", "output_type": "ordinal", "levels": len(LEVELS)}
    seen = set()
    for line in run_log_lines:
        row = json.loads(line)
        task, target = row["task"], row["ticket_id"]
        item_id = f"{task}:{target}"
        if item_id not in seen:
            seen.add(item_id)
            yield {"record": "item", "item_id": item_id, "cluster": target, "strata": {"task": task}}
            if target in labels.get(task, {}):
                yield {"record": "label", "item_id": item_id, "rater_id": "human:final", "source": "human",
                       "value": bool(labels[task][target]), "version": 1}
        missing = row.get("missing", {})
        for replicate, run in enumerate(("a", "b")):
            base = {"record": "judgment", "item_id": item_id, "run_id": run_id, "gen_replicate": replicate,
                    "output_type": "ordinal", "levels": len(LEVELS)}
            if run in missing:
                reason = MISSING_REASON.get(missing[run], "other")
                yield {**base, "missing": {"reason": reason, "detail": f"run {run}: {missing[run]}"}}
                continue
            out = row["runs"][run]
            level = out["level"]
            if level == "none":
                if none_as == "missing":
                    yield {**base, "missing": {"reason": "invalid_output", "detail": "no conclusion"}}
                    continue
                level = "low"
            feats = [{"feature_id": r["rule_id"], "fired": bool(r.get("fired"))}
                     for r in out.get("rules", []) if r.get("applies")]
            yield {**base, "value": LEVELS[level], "features": feats}


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("run_log")
    ap.add_argument("labels")
    ap.add_argument("--run-id", required=True)
    ap.add_argument("--system-version", default="unknown")
    ap.add_argument("--none-as", choices=["low", "missing"], default="low",
                    help="what a missing conclusion (none) becomes; low matches the harness default")
    a = ap.parse_args(argv)
    with open(a.labels, encoding="utf-8") as f:
        labels = json.load(f)
    with open(a.run_log, encoding="utf-8") as f:
        lines = [line for line in f if line.strip()]
    for r in records(lines, labels, a.run_id, a.system_version, a.none_as):
        sys.stdout.write(json.dumps(r, ensure_ascii=False) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
