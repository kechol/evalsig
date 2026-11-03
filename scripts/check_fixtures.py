#!/usr/bin/env python3
"""Regenerate the fixtures and docs/sample-size.md into a temporary directory and compare them
with the committed ones, and check that examples/custom-harness/records.out.jsonl is the
converter script's output.

Canonicalization vectors must match byte for byte. Reference values must match within
1e-12 (absolute); `generated_with` and `source` strings are allowed to differ, since they
record library versions.

Usage: uv run --project scripts scripts/check_fixtures.py
"""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

from gen_canonicalization_vectors import OUT as VECTORS
from reference_values import OUT as REFERENCE
from sample_size import OUT as SAMPLE_SIZE

HERE = Path(__file__).resolve().parent
TOL = 1e-12


def compare(a, b, path, errors):
    if isinstance(a, dict) and isinstance(b, dict):
        if a.keys() != b.keys():
            errors.append(f"{path}: keys differ")
            return
        for k in a:
            if k not in ("source", "generated_with"):
                compare(a[k], b[k], f"{path}.{k}", errors)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            errors.append(f"{path}: length {len(a)} != {len(b)}")
            return
        for i, (x, y) in enumerate(zip(a, b)):
            compare(x, y, f"{path}[{i}]", errors)
    elif isinstance(a, float) or isinstance(b, float):
        if not (isinstance(a, (int, float)) and isinstance(b, (int, float))) or abs(a - b) > TOL:
            errors.append(f"{path}: {a!r} != {b!r}")
    elif a != b:
        errors.append(f"{path}: {a!r} != {b!r}")


def main():
    errors = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for script, out in (("gen_canonicalization_vectors.py", VECTORS), ("reference_values.py", REFERENCE)):
            subprocess.run([sys.executable, str(HERE / script), "--out", str(tmp / out.name)], check=True,
                           stdout=subprocess.DEVNULL)
        if (tmp / VECTORS.name).read_bytes() != VECTORS.read_bytes():
            errors.append(f"{VECTORS.name}: bytes differ")
        compare(json.loads(REFERENCE.read_text(encoding="utf-8")),
                json.loads((tmp / REFERENCE.name).read_text(encoding="utf-8")), REFERENCE.name, errors)
        subprocess.run([sys.executable, str(HERE / "sample_size.py"), "--out", str(tmp / SAMPLE_SIZE.name)],
                       check=True, stdout=subprocess.DEVNULL)
        if (tmp / SAMPLE_SIZE.name).read_bytes() != SAMPLE_SIZE.read_bytes():
            errors.append(f"{SAMPLE_SIZE.name}: bytes differ")
    harness = HERE.parent / "examples" / "custom-harness"
    converted = subprocess.run(
        [sys.executable, "-I", str(harness / "convert_runlog.py"), str(harness / "run-log.sample.jsonl"),
         str(harness / "labels.sample.json"), "--run-id", "triage-v1"],
        check=True, capture_output=True).stdout
    if converted != (harness / "records.out.jsonl").read_bytes():
        errors.append("custom-harness/records.out.jsonl: differs from convert_runlog.py output")
    for e in errors:
        print(e)
    print("fixtures ok" if not errors else f"{len(errors)} difference(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
