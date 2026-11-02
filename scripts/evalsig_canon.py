"""Canonical JSON bytes and content hashes, as specified for evalsig objects and ledgers.

Rules: RFC 8785 (JSON Canonicalization Scheme) plus Unicode NFC normalization of every
string, including object keys. NaN and infinities are rejected; evalsig stores them as
`missing` values instead.

Standard library only, so that a third party can verify evalsig hashes without
installing anything. BLAKE3 needs the optional `blake3` package.
"""
import hashlib
import json
import math
import unicodedata
from decimal import Decimal


def _number(x) -> str:
    if isinstance(x, int):
        if abs(x) > 2**53 - 1:
            raise ValueError(f"integer {x} is outside the IEEE 754 exact range")
        return str(x)
    if not math.isfinite(x):
        raise ValueError("NaN and infinities have no JSON representation")
    if x == 0:
        return "0"
    # ECMAScript Number::toString: shortest round-trip digits, then choose notation.
    digits, exp = _shortest_digits(abs(x))
    sign = "-" if x < 0 else ""
    k = len(digits)
    n = exp + k  # position of the decimal point relative to the digit string
    if k <= n <= 21:
        return sign + digits + "0" * (n - k)
    if 0 < n <= 21:
        return sign + digits[:n] + "." + digits[n:]
    if -6 < n <= 0:
        return sign + "0." + "0" * (-n) + digits
    e = n - 1
    mant = digits[0] + ("." + digits[1:] if k > 1 else "")
    return sign + mant + "e" + ("+" if e >= 0 else "-") + str(abs(e))


def _shortest_digits(x: float):
    """Return (digits, exponent) with x == int(digits) * 10**exponent, digits minimal."""
    t = Decimal(repr(x)).normalize().as_tuple()
    return "".join(map(str, t.digits)), t.exponent


def _string(s: str) -> str:
    # json.dumps escapes exactly what RFC 8785 requires: quote, backslash, the short
    # escapes and other controls below U+0020 as lowercase \u00xx; nothing else.
    return json.dumps(unicodedata.normalize("NFC", s), ensure_ascii=False)


def _encode(v) -> str:
    if v is None:
        return "null"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if isinstance(v, (int, float)):
        return _number(v)
    if isinstance(v, str):
        return _string(v)
    if isinstance(v, (list, tuple)):
        return "[" + ",".join(_encode(x) for x in v) + "]"
    if isinstance(v, dict):
        items = {}
        for k, x in v.items():
            if not isinstance(k, str):
                raise TypeError("object keys must be strings")
            nk = unicodedata.normalize("NFC", k)
            if nk in items:
                raise ValueError(f"duplicate key after NFC normalization: {nk!r}")
            items[nk] = x
        keys = sorted(items, key=lambda k: k.encode("utf-16-be"))
        return "{" + ",".join(_string(k) + ":" + _encode(items[k]) for k in keys) + "}"
    raise TypeError(f"unsupported type {type(v).__name__}")


def canonical(obj) -> bytes:
    return _encode(obj).encode("utf-8")


def sha256_ref(b: bytes) -> str:
    return "sha256:" + hashlib.sha256(b).hexdigest()


def blake3_ref(b: bytes) -> str:
    import blake3  # optional dependency

    return "blake3:" + blake3.blake3(b).hexdigest()


def loads(text: str):
    """Parse JSON keeping integers as int; reject NaN, infinities and duplicate keys."""

    def constant(name):
        raise ValueError(f"{name} is not JSON")

    def pairs(ps):
        d = {}
        for k, v in ps:
            if k in d:
                raise ValueError(f"duplicate key {k!r}")
            d[k] = v
        return d

    return json.loads(text, object_pairs_hook=pairs, parse_constant=constant)
