#!/usr/bin/env python3
"""Verify an evalsig store without evalsig: ledger chains, object names and saved reports.

An independent implementation of the hashing rules (SPEC §9 "Verification from other
languages"), so a third party can check evalsig's evidence and CI can catch a Rust
change that alters a hash:

- ledger lines: `seq` counts from 0, `prev` names the previous line's `hash` (the first
  names `genesis`), and `hash` is the hash of the canonical line without `hash`, using
  the algorithm the line's own hash names (blake3 or sha256);
- objects: `objects/blake3/xx/rest.zst` decompresses to bytes whose BLAKE3 is `xxrest`,
  and JSON objects (a document, or JSON lines such as snapshots) are already canonical;
- reports: `reports/<hex>.json` carries results whose canonical BLAKE3 is `<hex>` and
  its `provenance.output`, and names a stored configuration and stored snapshots.

Usage: uv run --project scripts scripts/verify_store.py <.evalsig dir | ledger.jsonl>...
Exits 0 when everything matches, 1 with one line per problem otherwise.
"""
import hashlib
import sys
from pathlib import Path

import blake3
import zstandard

from evalsig_canon import canonical, loads

GENESIS = "genesis"
ENTRY_FIELDS = ("seq", "ts", "actor", "kind", "ledger", "inputs", "outputs", "summary", "tool", "prev", "hash")
ENTRY_DEFAULTS = {"inputs": [], "outputs": [], "summary": {}}


def ref(algorithm: str, data: bytes) -> str:
    if algorithm == "sha256":
        return "sha256:" + hashlib.sha256(data).hexdigest()
    return "blake3:" + blake3.blake3(data).hexdigest()


def entry_hash(entry: dict) -> str:
    algorithm = "sha256" if str(entry.get("hash", "")).startswith("sha256:") else "blake3"
    body = {**ENTRY_DEFAULTS, **{k: v for k, v in entry.items() if k != "hash"}}
    return ref(algorithm, canonical(body))


def verify_ledger(path: Path, problems: list) -> int:
    prev = GENESIS
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    for i, line in enumerate(lines):
        where = f"{path}: line {i + 1}"
        try:
            e = loads(line)
        except ValueError as err:
            problems.append(f"{where}: unparsable ({err})")
            prev = None
            continue
        unknown = sorted(set(e) - set(ENTRY_FIELDS))
        if unknown:
            problems.append(f"{where}: unknown fields {unknown}")
        if e.get("seq") != i:
            problems.append(f"{where}: seq is {e.get('seq')}, expected {i}")
        if e.get("prev") != prev:
            problems.append(f"{where}: prev does not name the previous line's hash")
        try:
            if entry_hash(e) != e.get("hash"):
                problems.append(f"{where}: hash does not match the content")
        except (TypeError, ValueError) as err:
            problems.append(f"{where}: cannot canonicalize ({err})")
        prev = e.get("hash")
    return len(lines)


def canonical_problem(data: bytes) -> str | None:
    """Why JSON content is not in canonical form; None for canonical or non-JSON content."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return None
    try:
        docs = [(loads(text), data)]
    except ValueError:
        try:
            docs = [(loads(line), raw) for line, raw in zip(text.split("\n"), data.split(b"\n"))]
        except ValueError:
            return None
    for n, (v, raw) in enumerate(docs, 1):
        try:
            if canonical(v) != raw:
                return f"line {n} is not canonical"
        except (TypeError, ValueError) as err:
            return f"line {n} cannot be canonicalized ({err})"
    return None


def verify_objects(root: Path, problems: list) -> set:
    names = set()
    dctx = zstandard.ZstdDecompressor()
    for path in sorted((root / "blake3").glob("*/*.zst")):
        name = "blake3:" + path.parent.name + path.name.removesuffix(".zst")
        names.add(name)
        try:
            with dctx.stream_reader(path.read_bytes()) as r:
                data = r.read()
        except zstandard.ZstdError as err:
            problems.append(f"object {name}: cannot decompress ({err})")
            continue
        if ref("blake3", data) != name:
            problems.append(f"object {name}: content hashes to {ref('blake3', data)}")
            continue
        why = canonical_problem(data)
        if why:
            problems.append(f"object {name}: {why}")
    return names


def verify_reports(root: Path, objects: set, problems: list) -> int:
    paths = sorted(root.glob("*.json"))
    for path in paths:
        try:
            report = loads(path.read_text(encoding="utf-8"))
            out = ref("blake3", canonical(report["results"]))
        except (ValueError, KeyError, TypeError) as err:
            problems.append(f"report {path.name}: unreadable ({err})")
            continue
        prov = report.get("provenance", {})
        if out != "blake3:" + path.stem or prov.get("output") != out:
            problems.append(f"report {path.name}: results hash to {out}")
        for h in [prov.get("config"), *prov.get("snapshots", [])]:
            if h not in objects:
                problems.append(f"report {path.name}: names {h}, which is not stored")
    return len(paths)


def main(args: list) -> int:
    if not args:
        print(__doc__, file=sys.stderr)
        return 2
    problems = []
    for arg in map(Path, args):
        if arg.is_dir():
            objects = verify_objects(arg / "objects", problems)
            reports = verify_reports(arg / "reports", objects, problems)
            entries = verify_ledger(arg / "ledger.jsonl", problems) if (arg / "ledger.jsonl").exists() else 0
            print(f"{arg}: {entries} ledger entries, {len(objects)} objects, {reports} reports")
        else:
            print(f"{arg}: {verify_ledger(arg, problems)} ledger entries")
    for p in problems:
        print(p)
    print("ok" if not problems else f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
