#!/usr/bin/env python3
"""Verify an evalsig store without evalsig: ledger chains, object names and saved reports.

An independent implementation of the hashing rules (SPEC §9 "Verification from other
languages"), so a third party can check evalsig's evidence and CI can catch a Rust
change that alters a hash:

- ledger lines: `seq` counts from 0, `prev` names the previous line's `hash` (the first
  names `genesis`), and `hash` is the hash of the canonical line without `hash`, using
  the algorithm the line's own hash names (blake3 or sha256);
- objects: `objects/blake3/xx/rest.zst` decompresses to bytes whose BLAKE3 is `xxrest`,
  and the objects the ledger names (snapshots, provenance, configuration) are canonical
  JSON lines;
- reports: `reports/<hex>.json` carries results whose canonical BLAKE3 is `<hex>` and
  its `provenance.output`, and names a stored configuration and stored snapshots.

Usage: uv run --project scripts scripts/verify_store.py <.evalsig dir | ledger.jsonl>...
Exits 0 when everything matches, 1 with one line per problem otherwise.
"""
import sys
from pathlib import Path

import zstandard

from evalsig_canon import blake3_ref, canonical, loads, sha256_ref

GENESIS = "genesis"
ENTRY_FIELDS = ("seq", "ts", "actor", "kind", "ledger", "inputs", "outputs", "summary", "tool", "prev", "hash")
ENTRY_DEFAULTS = {"inputs": [], "outputs": [], "summary": {}}


def entry_hash(entry: dict) -> str:
    """The hash of the line without `hash`, with the defaults Rust fills in, using the
    algorithm the line's own hash names."""
    body = {**ENTRY_DEFAULTS, **{k: v for k, v in entry.items() if k != "hash"}}
    ref = sha256_ref if str(entry.get("hash", "")).startswith("sha256:") else blake3_ref
    return ref(canonical(body))


def verify_ledger(path: Path, problems: list) -> list:
    """Check the chain; returns the entries that parse."""
    prev, entries = GENESIS, []
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
        entries.append(e)
    return entries


def canonical_problem(data: bytes) -> str | None:
    """Why JSON content is not in canonical form; None for canonical or non-JSON content.
    Objects hold one JSON document per line (snapshots have several lines)."""
    raws = data.split(b"\n")
    try:
        docs = [loads(raw.decode("utf-8")) for raw in raws]
    except (UnicodeDecodeError, ValueError):
        return None
    for n, (v, raw) in enumerate(zip(docs, raws), 1):
        try:
            if canonical(v) != raw:
                return f"line {n} is not canonical"
        except (TypeError, ValueError) as err:
            return f"line {n} cannot be canonicalized ({err})"
    return None


def verify_objects(root: Path, records: set, problems: list) -> set:
    """Check every object's name; `records` are the objects that hold evalsig's own JSON
    (snapshots, provenance, configuration), which must also be canonical. Content bodies
    are kept as they came."""
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
        if blake3_ref(data) != name:
            problems.append(f"object {name}: content hashes to {blake3_ref(data)}")
            continue
        why = canonical_problem(data) if name in records else None
        if why:
            problems.append(f"object {name}: {why}")
    return names


def verify_reports(root: Path, objects: set, problems: list) -> int:
    paths = sorted(root.glob("*.json"))
    for path in paths:
        try:
            report = loads(path.read_text(encoding="utf-8"))
            out = blake3_ref(canonical(report["results"]))
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
            ledger = arg / "ledger.jsonl"
            if ledger.is_file():
                entries = verify_ledger(ledger, problems)
            else:
                problems.append(f"{arg}: no ledger.jsonl")
                entries = []
            named = {h for e in entries for h in [*e.get("inputs", []), *e.get("outputs", [])]}
            objects = verify_objects(arg / "objects", named, problems)
            reports = verify_reports(arg / "reports", objects, problems)
            print(f"{arg}: {len(entries)} ledger entries, {len(objects)} objects, {reports} reports")
        else:
            print(f"{arg}: {len(verify_ledger(arg, problems))} ledger entries")
    for p in problems:
        print(p)
    print("ok" if not problems else f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
