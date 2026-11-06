//! Canonical JSON bytes (D-005): RFC 8785 plus NFC normalization of every string and
//! key. The same rules are implemented by `scripts/evalsig_canon.py`; the shared
//! vectors in `tests/fixtures/canonicalization-vectors.json` keep the two in step.

use std::borrow::Cow;
use std::fmt::{self, Write as _};

use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

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

impl From<serde_json::Error> for CanonicalError {
    fn from(e: serde_json::Error) -> Self {
        CanonicalError::Json(e.to_string())
    }
}

/// A JSON value in canonical form: strings and keys are NFC, object entries are
/// unique and sorted by UTF-16 code units, numbers are exact integers or finite
/// doubles. Values are only built through [`Canonical::parse`] and
/// [`Canonical::from_value`], which establish these invariants.
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

/// The parse tree before the canonical rules are applied: duplicate keys are kept
/// and integers are wide, so violations come back as typed errors.
enum Raw {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    String(String),
    Array(Vec<Raw>),
    Object(Vec<(String, Raw)>),
}

impl Canonical {
    /// Parse JSON text, rejecting duplicate keys (also after NFC normalization),
    /// integers beyond ±(2^53 − 1), NaN and infinities.
    pub fn parse(text: &str) -> Result<Canonical, CanonicalError> {
        // serde_json turns integer literals beyond u64 into doubles; catch them from
        // the text so every integer literal follows the same range rule.
        if let Some(lit) = huge_integer_literal(text) {
            return Err(CanonicalError::IntegerOutOfRange(lit.to_owned()));
        }
        let mut de = serde_json::Deserializer::from_str(text);
        let raw = Raw::deserialize(&mut de)?;
        de.end()?;
        Canonical::from_raw(raw)
    }

    /// Convert any serializable value (a record, a configuration, a ledger entry).
    pub fn from_serialize<T: Serialize + ?Sized>(value: &T) -> Result<Canonical, CanonicalError> {
        Canonical::from_value(serde_json::to_value(value)?)
    }

    /// Convert a parsed JSON value. Its keys are already unique; NFC may still merge
    /// two of them.
    pub fn from_value(v: serde_json::Value) -> Result<Canonical, CanonicalError> {
        use serde_json::Value as J;
        Ok(match v {
            J::Null => Canonical::Null,
            J::Bool(b) => Canonical::Bool(b),
            J::Number(n) => match (n.as_i64(), n.as_u64()) {
                (Some(i), _) => int(i128::from(i))?,
                (None, Some(u)) => int(i128::from(u))?,
                (None, None) => float(n.as_f64().ok_or(CanonicalError::NonFinite)?)?,
            },
            J::String(s) => Canonical::String(nfc(&s).into_owned()),
            J::Array(xs) => Canonical::Array(
                xs.into_iter()
                    .map(Canonical::from_value)
                    .collect::<Result<_, _>>()?,
            ),
            J::Object(m) => object(
                m.into_iter()
                    .map(|(k, v)| Ok((k, Canonical::from_value(v)?))),
            )?,
        })
    }

    fn from_raw(raw: Raw) -> Result<Canonical, CanonicalError> {
        Ok(match raw {
            Raw::Null => Canonical::Null,
            Raw::Bool(b) => Canonical::Bool(b),
            Raw::Int(i) => int(i)?,
            Raw::Float(f) => float(f)?,
            Raw::String(s) => Canonical::String(nfc(&s).into_owned()),
            Raw::Array(xs) => Canonical::Array(
                xs.into_iter()
                    .map(Canonical::from_raw)
                    .collect::<Result<_, _>>()?,
            ),
            Raw::Object(kv) => {
                let mut seen = std::collections::HashSet::with_capacity(kv.len());
                if let Some((k, _)) = kv.iter().find(|(k, _)| !seen.insert(k.as_str())) {
                    return Err(CanonicalError::DuplicateKey(k.clone()));
                }
                object(
                    kv.into_iter()
                        .map(|(k, v)| Ok((k, Canonical::from_raw(v)?))),
                )?
            }
        })
    }

    /// The value as `serde_json::Value`, for code that deserializes typed structures
    /// from input it has already parsed here.
    pub fn into_json(self) -> serde_json::Value {
        use serde_json::Value as J;
        match self {
            Canonical::Null => J::Null,
            Canonical::Bool(b) => J::Bool(b),
            Canonical::Int(i) => J::from(i),
            Canonical::Float(f) => serde_json::Number::from_f64(f).map_or(J::Null, J::Number),
            Canonical::String(s) => J::String(s),
            Canonical::Array(xs) => J::Array(xs.into_iter().map(Canonical::into_json).collect()),
            Canonical::Object(kv) => {
                J::Object(kv.into_iter().map(|(k, v)| (k, v.into_json())).collect())
            }
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
            Canonical::String(s) => write_string(s, out),
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
                out.push('{');
                for (i, (k, v)) in kv.iter().enumerate() {
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

/// The first integer literal (no fraction, no exponent) outside strings whose
/// magnitude exceeds u64, which serde_json would silently read as a double.
fn huge_integer_literal(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            match c {
                b'\\' => i += 1,
                b'"' => in_str = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        if c == b'-' || c.is_ascii_digit() {
            let start = i;
            while i < b.len() && matches!(b[i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                i += 1;
            }
            let lit = &text[start..i];
            let digits = lit.trim_start_matches('-');
            if !digits.is_empty()
                && digits.bytes().all(|d| d.is_ascii_digit())
                && digits.len() >= 20
                && digits.parse::<u64>().is_err()
            {
                return Some(lit);
            }
            continue;
        }
        i += 1;
    }
    None
}

fn int(i: i128) -> Result<Canonical, CanonicalError> {
    if i.unsigned_abs() > u128::from(MAX_SAFE_INTEGER) {
        return Err(CanonicalError::IntegerOutOfRange(i.to_string()));
    }
    Ok(Canonical::Int(i as i64))
}

fn float(f: f64) -> Result<Canonical, CanonicalError> {
    if f.is_finite() {
        Ok(Canonical::Float(f))
    } else {
        Err(CanonicalError::NonFinite)
    }
}

/// NFC-normalize keys, reject keys that NFC merges, and sort by UTF-16 code units.
fn object(
    entries: impl Iterator<Item = Result<(String, Canonical), CanonicalError>>,
) -> Result<Canonical, CanonicalError> {
    let mut kv: Vec<(String, Canonical)> = entries
        .map(|e| e.map(|(k, v)| (nfc(&k).into_owned(), v)))
        .collect::<Result<_, _>>()?;
    kv.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    if let Some(w) = kv.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(CanonicalError::DuplicateKeyAfterNfc(w[0].0.clone()));
    }
    Ok(Canonical::Object(kv))
}

/// Canonical bytes of JSON text.
pub fn canonicalize(text: &str) -> Result<Vec<u8>, CanonicalError> {
    Canonical::parse(text).map(|v| v.to_bytes())
}

/// The BLAKE3 reference of a value's canonical bytes, the name evalsig gives a JSON
/// document (a result, a configuration, an approval, a purged value).
pub fn hash<T: Serialize + ?Sized>(value: &T) -> Result<crate::hash::ContentHash, CanonicalError> {
    Ok(crate::hash::ContentHash::blake3(&to_canonical_bytes(
        value,
    )?))
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

fn nfc(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || is_nfc_quick(s.chars()) == IsNormalized::Yes {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.nfc().collect())
    }
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

impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(RawVisitor)
    }
}

struct RawVisitor;

impl<'de> Visitor<'de> for RawVisitor {
    type Value = Raw;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Raw, E> {
        Ok(Raw::Null)
    }

    fn visit_bool<E>(self, b: bool) -> Result<Raw, E> {
        Ok(Raw::Bool(b))
    }

    fn visit_i64<E>(self, i: i64) -> Result<Raw, E> {
        Ok(Raw::Int(i128::from(i)))
    }

    fn visit_u64<E>(self, u: u64) -> Result<Raw, E> {
        Ok(Raw::Int(i128::from(u)))
    }

    fn visit_f64<E>(self, f: f64) -> Result<Raw, E> {
        Ok(Raw::Float(f))
    }

    fn visit_str<E>(self, s: &str) -> Result<Raw, E> {
        Ok(Raw::String(s.to_owned()))
    }

    fn visit_string<E>(self, s: String) -> Result<Raw, E> {
        Ok(Raw::String(s))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Raw, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element()? {
            out.push(v);
        }
        Ok(Raw::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Raw, A::Error> {
        let mut out = Vec::new();
        while let Some(k) = map.next_key::<String>()? {
            out.push((k, map.next_value()?));
        }
        Ok(Raw::Object(out))
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
    fn integer_literals_beyond_u64_are_out_of_range() {
        assert!(matches!(
            Canonical::parse(r#"{"w":18446744073709551616}"#),
            Err(CanonicalError::IntegerOutOfRange(_))
        ));
        assert!(Canonical::parse(r#"{"w":1e20,"s":"18446744073709551616"}"#).is_ok());
        assert!(Canonical::parse("[-0.5, 12, -3]").is_ok());
    }

    #[test]
    fn output_rounding() {
        assert_eq!(round_output(0.123_456_789_012_34), 0.123_456_789_0);
        assert_eq!(round_output(-0.000_000_000_04), 0.0);
        assert_eq!(round_output(2.0), 2.0);
    }
}
