#!/usr/bin/env python3
"""Generate tests/fixtures/canonicalization-vectors.json.

Each vector gives an input as raw JSON text, the canonical bytes (RFC 8785 + NFC, see
evalsig_canon.py) and their sha256 and BLAKE3 references. `invalid` lists inputs that a
conforming implementation must reject. `files` gives snapshot-file hashes: canonical
lines sorted by the caller and joined with "\\n" (no trailing newline).

Usage: uv run --project scripts scripts/gen_canonicalization_vectors.py [--out PATH]
"""
import argparse
import json
import sys
from pathlib import Path

import evalsig_canon as c

OUT = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "canonicalization-vectors.json"

VALID = [
    ("key_order_and_whitespace", '{ "b": 1, "a": "é", "n": [1, 2] }'),
    ("nfc_of_string_value", '{"s": "e\\u0301"}'),
    ("nfc_of_object_key", '{"e\\u0301": true}'),
    ("utf16_key_order", '{"\\ufb01": 2, "\\ud83d\\ude00": 1, "z": 0, "\\u00e9": 3}'),
    ("string_escapes", '{"q": "\\"\\\\\\/\\b\\f\\n\\r\\t\\u0001\\u001f\\u007f"}'),
    ("non_ascii_is_not_escaped", '{"ja": "\\u8a55\\u4fa1", "emoji": "\\ud83d\\udcca"}'),
    ("literals", '{"t": true, "f": false, "z": null, "e": {}, "a": []}'),
    ("integers", '{"zero": 0, "neg": -42, "max_safe": 9007199254740991, "min_safe": -9007199254740991}'),
    ("float_shortest_form", '[1.0, 0.1, 0.30000000000000004, 123.456, -0.0, 1e21, 1e20, 1e-7, 1.5e-6, 0.000001, 5e-324, 1.7976931348623157e308]'),
    ("float_exponent_input", '[1E3, 2.50e-1, 1e+2]'),
    ("nested", '{"z": [{"b": [3, {"y": 1, "x": 2}], "a": null}], "a": {"c": {"b": {"a": 0}}}}'),
    ("record_judgment", '{"kind": "judgment", "run_id": "r1", "item_id": "item-0007", "gen_replicate": 0, "grade_replicate": 0, "value": 0.85, "value_semantics": "score"}'),
]

INVALID = [
    ("nan_literal", '{"x": NaN}', "not JSON"),
    ("infinity_literal", '{"x": Infinity}', "not JSON"),
    ("duplicate_key", '{"a": 1, "a": 2}', "duplicate key"),
    ("duplicate_key_after_nfc", '{"\\u00e9": 1, "e\\u0301": 2}', "duplicate key after NFC"),
    ("integer_outside_exact_range", '{"x": 9007199254740993}', "integer outside the IEEE 754 exact range"),
    ("lone_surrogate", '{"x": "\\ud800"}', "unpaired surrogate"),
]


def ledger_entry():
    """The first referee-ledger line of the specification's example (a split)."""
    def ref(s):
        return c.sha256_ref(s.encode())

    e = {"seq": 0, "ts": "2026-10-29T09:05:00Z", "actor": "user:alice", "kind": "split", "ledger": "referee",
         "inputs": [ref("baseline-snapshot-bytes")],
         "outputs": [ref("dev"), ref("val"), ref("holdout")],
         "summary": {"method": "time", "boundary": "2026-09-01", "seed": 20261029, "unit": "cluster"},
         "tool": "evalsig-proto 0", "prev": "genesis"}
    e["hash"] = c.sha256_ref(c.canonical(e))
    return e


def vector(name, text):
    obj = c.loads(text)
    b = c.canonical(obj)
    return {"name": name, "input": text, "canonical": b.decode("utf-8"),
            "sha256": c.sha256_ref(b), "blake3": c.blake3_ref(b)}


def rejected(text):
    try:
        obj = c.loads(text)
        c.canonical(obj).decode("utf-8")
    except (ValueError, TypeError, UnicodeError):
        return True
    return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=OUT)
    args = ap.parse_args()

    vectors = [vector(n, t) for n, t in VALID]
    entry = ledger_entry()
    body = c.canonical({k: v for k, v in entry.items() if k != "hash"})
    vectors.append({"name": "ledger_entry_without_hash",
                    "input": json.dumps(entry, ensure_ascii=False),
                    "canonical": body.decode("utf-8"),
                    "sha256": entry["hash"], "blake3": c.blake3_ref(body),
                    "note": "the entry hash is the hash of the entry without its `hash` key"})
    invalid = []
    for n, t, why in INVALID:
        assert rejected(t), n
        invalid.append({"name": n, "input": t, "reason": why})
    lines = [c.canonical(c.loads(t)) for t in (
        '{"item_id": "a", "kind": "item"}', '{"item_id": "b", "kind": "item", "stratum": {"lang": "ja"}}')]
    joined = b"\n".join(lines)
    files = [{"name": "two_item_lines", "lines": [x.decode("utf-8") for x in lines],
              "sha256": c.sha256_ref(joined), "blake3": c.blake3_ref(joined)}]
    doc = {"schema": "canonicalization-vectors/1",
           "rules": "RFC 8785 plus NFC normalization of all strings and keys. Integers must lie within "
                    "+/-(2^53-1). A ledger entry hash covers the entry without its `hash` key. "
                    "A file hash covers its canonical lines joined with LF and no trailing newline.",
           "vectors": vectors, "invalid": invalid, "files": files}
    args.out.write_text(json.dumps(doc, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {args.out} ({len(vectors)} vectors, {len(invalid)} invalid, {len(files)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
