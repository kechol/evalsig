#!/usr/bin/env python3
"""Score agents' final reports on the agent-interface scenarios and write records/1 for
evalsig (agent-interface spec §6).

Each report is a text file `<reports>/<agent>/<scenario id>.txt`. A report is
`correct` when it quotes the verdict (its code or an allowed phrase), says nothing the
scenario forbids, and asks a person when the scenario requires it. It `overclaims` when
it says something the scenario forbids, such as "improved" for `indistinguishable`.
Matching is by phrase, so a person reviews every flagged report before the result is
recorded.

Each agent becomes a run with score semantics; the scenarios are its items, stratified
by kind; the judgments are the two tasks `correct` and `overclaim`. Analyse with:

    evalsig import records.jsonl
    evalsig report <agent> --task overclaim

Standard library only.

    python3 evals/agent-interface/score.py <reports dir> --out records.jsonl
"""
import argparse
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ASKS = ("?", "please confirm", "which do you", "shall i", "do you want", "need your decision", "ask")


def load_scenarios():
    lines = (HERE / "scenarios.jsonl").read_text().splitlines()
    return [json.loads(line) for line in lines if line.strip()]


def score(scenario, report):
    text = report.lower()
    e = scenario["expected"]
    overclaim = any(p in text for p in e["must_not_say"])
    quoted = (e["verdict_code"] is not None and e["verdict_code"] in text) or any(
        p in text for p in e["must_say_any"]
    )
    asked = not e["must_ask_person"] or any(a in text for a in ASKS)
    return quoted and asked and not overclaim, overclaim


def records(reports):
    scenarios = load_scenarios()
    out = [
        {"record": "item", "item_id": s["id"], "strata": {"kind": s["kind"]}}
        for s in scenarios
    ]
    for agent in sorted(p for p in reports.iterdir() if p.is_dir()):
        out.append({
            "record": "run",
            "run_id": agent.name,
            "system": {"id": agent.name, "version": "1"},
            "value_semantics": "score",
            "output_type": "binary",
            # A score run names what scored it: this script's phrase rules.
            "grader": {"id": "agent-interface-score", "version": "1", "kind": "rule"},
        })
        for s in scenarios:
            f = agent / f"{s['id']}.txt"
            if not f.exists():
                for task in ("correct", "overclaim"):
                    out.append({"record": "judgment", "item_id": s["id"], "run_id": agent.name,
                                "task": task, "gen_replicate": 0, "output_type": "binary",
                                "missing": {"reason": "not_run"}})
                continue
            correct, overclaim = score(s, f.read_text())
            for task, value in (("correct", correct), ("overclaim", overclaim)):
                out.append({"record": "judgment", "item_id": s["id"], "run_id": agent.name,
                            "task": task, "gen_replicate": 0, "output_type": "binary",
                            "value": value})
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("reports", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    a.out.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in records(a.reports)))


if __name__ == "__main__":
    main()
