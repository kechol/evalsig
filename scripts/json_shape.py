#!/usr/bin/env python3
"""Print the structure of a JSON or JSONL document (key paths, types, example values) as a
Markdown table.

Used to record the formats of external harnesses from real outputs rather than from their
documentation. Arrays are followed through their first element only: the aim is the shape,
not a count.

Usage: python3 -I scripts/json_shape.py FILE [--jsonl] [--max-depth N]
"""
import argparse
import json
import sys


def walk(v, path, depth, max_depth, rows):
    t = type(v).__name__
    if isinstance(v, dict):
        rows.append((path or "$", "object", f"{len(v)} keys"))
        if depth < max_depth:
            for k, x in v.items():
                walk(x, f"{path}.{k}" if path else k, depth + 1, max_depth, rows)
    elif isinstance(v, list):
        rows.append((path or "$", "array", f"len={len(v)}"))
        if v and depth < max_depth:
            walk(v[0], f"{path}[0]", depth + 1, max_depth, rows)
    else:
        s = json.dumps(v, ensure_ascii=False)
        rows.append((path, t, s if len(s) <= 60 else s[:57] + "..."))


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--jsonl", action="store_true")
    ap.add_argument("--max-depth", type=int, default=6)
    a = ap.parse_args(argv)
    with open(a.file, encoding="utf-8") as f:
        doc = [json.loads(line) for line in f if line.strip()][0] if a.jsonl else json.load(f)
    rows = []
    walk(doc, "", 0, a.max_depth, rows)
    print("| Path | Type | Example |\n|---|---|---|")
    for p, t, ex in rows:
        ex = ex.replace("|", "\\|").replace("\n", " ")
        print(f"| `{p}` | {t} | {ex} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
