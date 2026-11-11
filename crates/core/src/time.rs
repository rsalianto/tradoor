//! UTC calendar arithmetic, without a date library.
//!
//! Every boundary in this app is UTC — weeks start on Monday, months on the 1st —
//! so all that is needed is days-since-epoch ⇄ civil date. These are Howard
//! Hinnant's algorithms: exact for the proleptic Gregorian calendar, integer
//! only, and small enough that the WASM build carries no timezone data.

pub const MINUTE: i64 = 60_000;
pub const HOUR: i64 = 60 * MINUTE;
pub const DAY: i64 = 24 * HOUR;
pub const WEEK: i64 = 7 * DAY;

/// The epoch (1970-01-01) is a Thursday; the first Monday is four days later.
/// Subtracting this before rounding to a week is what lands weeks on Monday.
pub const FIRST_MONDAY_OFFSET: i64 = 4 * DAY;

/// Days since the epoch for a civil date (`month` 1–12).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (`month` 1–12) for a count of days since the epoch.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// A UTC instant split into calendar fields. `month0` is zero-based, like
/// JavaScript's, because every formula ported from the TypeScript used it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month0: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub fn civil(ms: i64) -> Civil {
    let rem = ms.rem_euclid(DAY);
    let (year, month, day) = civil_from_days(ms.div_euclid(DAY));
    Civil {
        year,
        month0: month - 1,
        day,
        hour: (rem / HOUR) as u32,
        minute: (rem % HOUR / MINUTE) as u32,
        second: (rem % MINUTE / 1000) as u32,
    }
}

/// `Date.UTC(year, month0, day)`, including its month overflow: `month0 = 12`
/// is January of the next year and `-1` December of the previous one.
pub fn utc(year: i64, month0: i64, day: u32) -> i64 {
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) as u32 + 1;
    days_from_civil(y, m, day) * DAY
}

/// Day of the week, 0 = Sunday — `getUTCDay`.
pub fn weekday(ms: i64) -> u32 {
    (ms.div_euclid(DAY) + 4).rem_euclid(7) as u32
}

/// Whole weeks since the first Monday.
pub fn week_index(ms: i64) -> i64 {
    (ms - FIRST_MONDAY_OFFSET).div_euclid(WEEK)
}

/// Months since year 0.
pub fn month_index(ms: i64) -> i64 {
    let c = civil(ms);
    c.year * 12 + i64::from(c.month0)
}

pub fn year_of(ms: i64) -> i64 {
    civil(ms).year
}

/// `2026-09-14`.
pub fn iso_date(ms: i64) -> String {
    let c = civil(ms);
    format!("{:04}-{:02}-{:02}", c.year, c.month0 + 1, c.day)
}

/// `2026-09-14 13:05`, the resolution a candle boundary needs.
pub fn iso_minute(ms: i64) -> String {
    let c = civil(ms);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", c.year, c.month0 + 1, c.day, c.hour, c.minute)
}

/// Parse `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM` or `YYYY-MM-DDTHH:MM:SS`, optionally
/// ending in `Z`. UTC only: a string carrying any other offset is refused rather
/// than silently read as UTC.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = s.get(from..from + len)?;
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date = days_from_civil(year, month as u32, day as u32) * DAY;
    if b.len() == 10 {
        return Some(date);
    }
    if (b[10] != b'T' && b[10] != b' ') || b.len() < 16 || b[13] != b':' {
        return None;
    }
    let ms = date + num(11, 2)? * HOUR + num(14, 2)? * MINUTE;
    match b.len() {
        16 => Some(ms),
        19 if b[16] == b':' => Some(ms + num(17, 2)? * 1000),
        _ => None,
    }
}

/// The `n`th Sunday of a month (`0`: the last), at midnight UTC.
fn sunday(year: i64, month0: i64, n: i64) -> i64 {
    if n == 0 {
        let last = utc(year, month0 + 1, 1) - DAY;
        return last - i64::from(weekday(last)) * DAY;
    }
    let first = utc(year, month0, 1);
    first + ((7 - i64::from(weekday(first))) % 7 + 7 * (n - 1)) * DAY
}

/// Whether London keeps summer time on the day starting at `day`.
fn london_summer(day: i64) -> bool {
    let year = civil(day).year;
    day >= sunday(year, 2, 0) && day < sunday(year, 9, 0)
}

/// Whether New York keeps summer time on the day starting at `day`.
fn new_york_summer(day: i64) -> bool {
    let year = civil(day).year;
    day >= sunday(year, 2, 2) && day < sunday(year, 10, 1)
}

/// London's open and close on a day, in minutes past midnight UTC.
pub fn london(day: i64) -> (i64, i64) {
    let shift = if london_summer(day) { 60 } else { 0 };
    (8 * 60 - shift, 16 * 60 + 30 - shift)
}

/// Minutes New York is behind UTC on the day starting at `day`.
pub fn new_york_behind(day: i64) -> i64 {
    if new_york_summer(day) {
        4 * 60
    } else {
        5 * 60
    }
}

/// New York's open and close on a day, in minutes past midnight UTC.
pub fn new_york(day: i64) -> (i64, i64) {
    let behind = new_york_behind(day);
    (9 * 60 + 30 + behind, 16 * 60 + behind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_across_centuries() {
        for days in (-800_000..800_000).step_by(997) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
    }

    #[test]
    fn the_epoch_is_a_thursday() {
        assert_eq!(weekday(0), 4);
        assert_eq!(weekday(parse_iso("2026-09-14").unwrap()), 1, "2026-09-14 is a Monday");
    }

    #[test]
    fn utc_overflows_months_like_javascript() {
        assert_eq!(utc(2026, 12, 1), parse_iso("2027-01-01").unwrap());
        assert_eq!(utc(2026, -1, 1), parse_iso("2025-12-01").unwrap());
        assert_eq!(utc(2024, 1, 29), parse_iso("2024-02-29").unwrap());
    }

    #[test]
    fn parses_the_shapes_it_claims_to() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("1970-01-02T01:02Z"), Some(DAY + HOUR + 2 * MINUTE));
        assert_eq!(parse_iso("2026-09-14+07:00"), None);
        assert_eq!(parse_iso("2026-13-01"), None);
    }

    #[test]
    fn formats_minutes() {
        assert_eq!(iso_minute(parse_iso("2026-09-14T13:05:59Z").unwrap()), "2026-09-14 13:05");
    }
}
