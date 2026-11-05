//! RFC 3339 timestamps reduced to an instant, for ordering and as-of selection.

use std::fmt;

/// An instant as nanoseconds since 1970-01-01T00:00:00Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(i128);

/// A calendar period for trends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    /// Calendar day.
    Day,
    /// Week.
    Week,
    /// Calendar month.
    Month,
}

/// A string that is not an RFC 3339 timestamp.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an RFC 3339 timestamp: {0:?}")]
pub struct TimestampError(pub String);

impl Instant {
    /// Parse `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`, the JSON Schema `date-time`
    /// format. Lowercase `t` and `z` are accepted; leap seconds are not.
    pub fn parse(s: &str) -> Result<Instant, TimestampError> {
        parse(s.as_bytes()).ok_or_else(|| TimestampError(s.to_owned()))
    }

    /// Nanoseconds since the Unix epoch.
    pub fn as_unix_nanos(self) -> i128 {
        self.0
    }

    /// The instant at this many nanoseconds since the Unix epoch.
    pub fn from_unix_nanos(nanos: i128) -> Self {
        Instant(nanos)
    }

    /// The UTC calendar period holding this instant: `2026-11-03` (day), `2026-W45`
    /// (ISO 8601 week, Monday first) or `2026-11` (month). Labels of one kind sort in
    /// time order.
    pub fn period(self, p: Period) -> String {
        let days = self.0.div_euclid(86_400 * 1_000_000_000) as i64;
        let (y, m, d) = civil_from_days(days);
        match p {
            Period::Day => format!("{y:04}-{m:02}-{d:02}"),
            Period::Month => format!("{y:04}-{m:02}"),
            Period::Week => {
                // 1970-01-01 was a Thursday; the ISO year is that of the week's Thursday.
                let weekday = (days + 3).rem_euclid(7);
                let thursday = days - weekday + 3;
                let (iso_year, _, _) = civil_from_days(thursday);
                let week = (thursday - days_from_civil(iso_year, 1, 1)) / 7 + 1;
                format!("{iso_year:04}-W{week:02}")
            }
        }
    }

    /// `YYYY-MM-DDTHH:MM:SSZ` in UTC, truncated to whole seconds.
    pub fn to_rfc3339(self) -> String {
        let secs = self.0.div_euclid(1_000_000_000) as i64;
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        let (y, m, d) = civil_from_days(days);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            rem / 3_600,
            rem % 3_600 / 60,
            rem % 60
        )
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ns", self.0)
    }
}

fn digits(b: &[u8], n: usize) -> Option<i64> {
    let s = b.get(..n)?;
    s.iter().try_fold(0i64, |acc, c| {
        c.is_ascii_digit().then(|| acc * 10 + i64::from(c - b'0'))
    })
}

fn parse(b: &[u8]) -> Option<Instant> {
    let year = digits(b, 4)?;
    let month = digits(b.get(5..)?, 2)?;
    let day = digits(b.get(8..)?, 2)?;
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    if !matches!(b.get(10), Some(b'T' | b't')) {
        return None;
    }
    let hour = digits(b.get(11..)?, 2)?;
    let minute = digits(b.get(14..)?, 2)?;
    let second = digits(b.get(17..)?, 2)?;
    if b.get(13) != Some(&b':') || b.get(16) != Some(&b':') {
        return None;
    }
    let mut i = 19;
    let mut nanos: i64 = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            if i - start < 9 {
                nanos = nanos * 10 + i64::from(b[i] - b'0');
            }
            i += 1;
        }
        let n = i - start;
        if n == 0 {
            return None;
        }
        for _ in n..9 {
            nanos *= 10;
        }
    }
    let offset_minutes = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let h = digits(&b[i + 1..], 2)?;
            let m = digits(&b[i + 4..], 2)?;
            if h > 23 || m > 59 {
                return None;
            }
            let o = h * 60 + m;
            if *sign == b'-' { -o } else { o }
        }
        _ => return None,
    };
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(Instant(
        i128::from(secs) * 1_000_000_000 + i128::from(nanos),
    ))
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: &str) -> i128 {
        Instant::parse(s).unwrap().as_unix_nanos() / 1_000_000_000
    }

    #[test]
    fn parses_known_instants() {
        assert_eq!(secs("1970-01-01T00:00:00Z"), 0);
        assert_eq!(secs("2026-10-29T09:05:00Z"), 1_793_264_700);
        assert_eq!(secs("2000-02-29T12:00:00+09:00"), 951_793_200);
        assert_eq!(secs("1969-12-31T23:59:59Z"), -1);
        assert_eq!(
            Instant::parse("2026-01-01T00:00:00.5Z")
                .unwrap()
                .as_unix_nanos()
                % 1_000_000_000,
            500_000_000
        );
    }

    #[test]
    fn formats_round_trip() {
        for s in [
            "1970-01-01T00:00:00Z",
            "2000-02-29T03:00:00Z",
            "2026-11-03T08:37:45Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert_eq!(Instant::parse(s).unwrap().to_rfc3339(), s);
        }
        assert_eq!(
            Instant::parse("2026-11-03T17:37:45+09:00")
                .unwrap()
                .to_rfc3339(),
            "2026-11-03T08:37:45Z"
        );
    }

    #[test]
    fn offsets_order_correctly() {
        let a = Instant::parse("2026-08-03T09:00:00+09:00").unwrap();
        let b = Instant::parse("2026-08-03T01:00:00Z").unwrap();
        assert!(a < b);
    }

    #[test]
    fn rejects_malformed() {
        for s in [
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-01-01",
            "2026-01-01T00:00:00",
            "2026-01-01T00:00:00.Z",
            "2026-01-01T25:00:00Z",
            "2026-01-01T00:00:00+0900",
            "2026-01-01T00:00:00Zjunk",
            "2026-01-01 00:00:00Z",
            "2026-06-30T23:59:60Z",
        ] {
            assert!(Instant::parse(s).is_err(), "{s}");
        }
    }

    #[test]
    fn periods_follow_iso_weeks_across_year_ends() {
        let at = |s: &str| Instant::parse(s).unwrap();
        assert_eq!(at("2026-11-03T12:00:00Z").period(Period::Day), "2026-11-03");
        assert_eq!(at("2026-11-03T12:00:00Z").period(Period::Month), "2026-11");
        assert_eq!(at("2026-11-03T12:00:00Z").period(Period::Week), "2026-W45");
        // 2027-01-01 is a Friday: still week 53 of 2026. 2024-12-30 is in 2025-W01.
        assert_eq!(at("2027-01-01T00:00:00Z").period(Period::Week), "2026-W53");
        assert_eq!(at("2024-12-30T00:00:00Z").period(Period::Week), "2025-W01");
        assert_eq!(
            at("2026-11-03T00:30:00+09:00").period(Period::Day),
            "2026-11-02"
        );
    }
}
