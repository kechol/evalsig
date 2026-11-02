//! Canonical JSON bytes (D-005): RFC 8785 plus NFC normalization of every string and
//! key. The same rules are implemented by `scripts/evalsig_canon.py`; the shared
//! vectors in `tests/fixtures/canonicalization-vectors.json` keep the two in step.

use std::fmt::{self, Write as _};

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Largest integer magnitude an IEEE 754 double represents exactly.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CanonicalError {
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("duplicate key {0:?}")]
    DuplicateKey(String),
    #[error("duplicate key after NFC normalization: {0:?}")]
    DuplicateKeyAfterNfc(String),
    #[error("integer {0} is outside the IEEE 754 exact range")]
    IntegerOutOfRange(String),
    #[error("NaN and infinities have no JSON representation")]
    NonFinite,
}

/// A JSON value ready for canonical encoding: object keys are unique after NFC and
/// numbers are either exact integers or finite doubles.
#[derive(Debug, Clone, PartialEq)]
pub enum Canonical {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Canonical>),
    Object(Vec<(String, Canonical)>),
}

impl Canonical {
    /// Parse JSON text, rejecting duplicate keys (also after NFC normalization),
    /// integers beyond ±(2^53 − 1), NaN and infinities.
    pub fn parse(text: &str) -> Result<Canonical, CanonicalError> {
        let mut de = serde_json::Deserializer::from_str(text);
        let v = Canonical::deserialize(&mut de).map_err(|e| from_json_error(&e))?;
        de.end().map_err(|e| from_json_error(&e))?;
        v.check()?;
        Ok(v)
    }

    /// Convert any serializable value (a record, a configuration, a ledger entry).
    pub fn from_serialize<T: Serialize + ?Sized>(value: &T) -> Result<Canonical, CanonicalError> {
        let text = serde_json::to_string(value).map_err(|e| from_json_error(&e))?;
        Canonical::parse(&text)
    }

    fn check(&self) -> Result<(), CanonicalError> {
        match self {
            Canonical::Float(f) if !f.is_finite() => Err(CanonicalError::NonFinite),
            Canonical::Array(xs) => xs.iter().try_for_each(Canonical::check),
            Canonical::Object(kv) => {
                let mut keys: Vec<String> = kv.iter().map(|(k, _)| nfc(k)).collect();
                keys.sort_unstable();
                if let Some(w) = keys.windows(2).find(|w| w[0] == w[1]) {
                    return Err(CanonicalError::DuplicateKeyAfterNfc(w[0].clone()));
                }
                kv.iter().try_for_each(|(_, v)| v.check())
            }
            _ => Ok(()),
        }
    }

    /// The canonical UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        self.write(&mut out);
        out.into_bytes()
    }

    fn write(&self, out: &mut String) {
        match self {
            Canonical::Null => out.push_str("null"),
            Canonical::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Canonical::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Canonical::Float(f) => write_number(*f, out),
            Canonical::String(s) => write_string(&nfc(s), out),
            Canonical::Array(xs) => {
                out.push('[');
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    x.write(out);
                }
                out.push(']');
            }
            Canonical::Object(kv) => {
                let mut entries: Vec<(String, &Canonical)> =
                    kv.iter().map(|(k, v)| (nfc(k), v)).collect();
                entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Canonical bytes of JSON text.
pub fn canonicalize(text: &str) -> Result<Vec<u8>, CanonicalError> {
    Canonical::parse(text).map(|v| v.to_bytes())
}

/// Canonical bytes of a serializable value.
pub fn to_canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    Canonical::from_serialize(value).map(|v| v.to_bytes())
}

/// Round a number written to an output envelope to 10 decimal places (metrics spec
/// §0), so that `verify` compares values that do not depend on the last bits.
pub fn round_output(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let rounded: f64 = format!("{x:.10}").parse().expect("formatted float parses");
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn from_json_error(e: &serde_json::Error) -> CanonicalError {
    let msg = e.to_string();
    for prefix in ["duplicate key ", "integer "] {
        if let Some(rest) = msg.strip_prefix(prefix) {
            let value = rest.split(" at line").next().unwrap_or(rest).to_owned();
            return if prefix == "integer " {
                CanonicalError::IntegerOutOfRange(value)
            } else {
                CanonicalError::DuplicateKey(value)
            };
        }
    }
    CanonicalError::Json(msg)
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// ECMAScript Number::toString of a finite double.
fn write_number(x: f64, out: &mut String) {
    if x == 0.0 {
        out.push('0');
        return;
    }
    // `{:e}` prints the shortest digits that round-trip, as d.ddddeN.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().expect("exponent is an integer") + 1;
    if x < 0.0 {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(digits);
    } else {
        let e = n - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "e{}{}", if e >= 0 { '+' } else { '-' }, e.abs());
    }
}

impl<'de> Deserialize<'de> for Canonical {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(CanonicalVisitor)
    }
}

struct CanonicalVisitor;

impl<'de> Visitor<'de> for CanonicalVisitor {
    type Value = Canonical;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Canonical, E> {
        Ok(Canonical::Null)
    }

    fn visit_bool<E>(self, b: bool) -> Result<Canonical, E> {
        Ok(Canonical::Bool(b))
    }

    fn visit_i64<E: de::Error>(self, i: i64) -> Result<Canonical, E> {
        if i.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(E::custom(format_args!("integer {i}")));
        }
        Ok(Canonical::Int(i))
    }

    fn visit_u64<E: de::Error>(self, u: u64) -> Result<Canonical, E> {
        if u > MAX_SAFE_INTEGER {
            return Err(E::custom(format_args!("integer {u}")));
        }
        Ok(Canonical::Int(u as i64))
    }

    fn visit_f64<E>(self, f: f64) -> Result<Canonical, E> {
        Ok(Canonical::Float(f))
    }

    fn visit_str<E>(self, s: &str) -> Result<Canonical, E> {
        Ok(Canonical::String(s.to_owned()))
    }

    fn visit_string<E>(self, s: String) -> Result<Canonical, E> {
        Ok(Canonical::String(s))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Canonical, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element()? {
            out.push(v);
        }
        Ok(Canonical::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Canonical, A::Error> {
        let mut out: Vec<(String, Canonical)> = Vec::new();
        while let Some(k) = map.next_key::<String>()? {
            if out.iter().any(|(e, _)| *e == k) {
                return Err(de::Error::custom(format_args!("duplicate key {k:?}")));
            }
            let v = map.next_value()?;
            out.push((k, v));
        }
        Ok(Canonical::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_use_the_ecmascript_form() {
        let cases = [
            (1.0, "1"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-7, "1e-7"),
            (1.5e-6, "0.0000015"),
            (123.456, "123.456"),
            (-0.0, "0"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (-2.5e-10, "-2.5e-10"),
            (0.000001, "0.000001"),
        ];
        for (x, want) in cases {
            let mut s = String::new();
            write_number(x, &mut s);
            assert_eq!(s, want, "{x}");
        }
    }

    #[test]
    fn output_rounding() {
        assert_eq!(round_output(0.123_456_789_012_34), 0.123_456_789_0);
        assert_eq!(round_output(-0.000_000_000_04), 0.0);
        assert_eq!(round_output(2.0), 2.0);
    }
}
