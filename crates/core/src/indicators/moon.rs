//! When the moon is new and full, after Meeus, Astronomical Algorithms, ch. 49:
//! the mean lunation and the seven largest of its corrections, which keeps every
//! phase within a few minutes. A chart marks the bar a phase falls in, so that
//! is more than it can show.

const DAY_MS: f64 = 86_400_000.0;
/// The Julian day the Unix epoch falls on.
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
const SYNODIC_DAYS: f64 = 29.530_588_861;
/// The new moon Meeus counts lunations from, as a Julian day.
const FIRST_NEW_MOON: f64 = 2_451_550.097_66;
/// Dynamical time runs about 69 s ahead of UTC these years.
const DELTA_T_MS: f64 = 69_000.0;

/// The sine of an angle in degrees, from its series about the nearest quarter
/// turn: good to a billionth, far past what a minute of the moon's time asks,
/// and the library's weighs 8 kB of wasm.
fn sine(degrees: f64) -> f64 {
    let a = degrees.rem_euclid(360.0);
    let a = if a > 270.0 { a - 360.0 } else if a > 90.0 { 180.0 - a } else { a };
    let x = a.to_radians();
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))))
}

/// The instant of lunation `k`: new at a whole `k`, full halfway to the next.
fn phase_ms(k: f64) -> i64 {
    let t = k / 1236.85;
    let degrees = |a: f64, b: f64, c: f64| a + b * k + c * t * t;
    let sin = sine;
    let e = 1.0 - 0.002_516 * t - 0.000_007_4 * t * t;
    let m = degrees(2.5534, 29.105_356_70, -0.000_001_4);
    let moon = degrees(201.5643, 385.816_935_28, 0.010_758_2);
    let f = degrees(160.7108, 390.670_502_84, -0.001_611_8);
    let full = k.fract().abs() > 0.25;
    let w: [f64; 7] = if full { [-0.40614, 0.17302, 0.01614, 0.01043, 0.00734, -0.00515, 0.00209] } else { [-0.40720, 0.17241, 0.01608, 0.01039, 0.00739, -0.00514, 0.00208] };
    let corrections = w[0] * sin(moon) + w[1] * e * sin(m) + w[2] * sin(2.0 * moon) + w[3] * sin(2.0 * f) + w[4] * e * sin(moon - m) + w[5] * e * sin(moon + m) + w[6] * e * e * sin(2.0 * m);
    let jd = FIRST_NEW_MOON + SYNODIC_DAYS * k + 0.000_154_37 * t * t + corrections;
    ((jd - UNIX_EPOCH_JD) * DAY_MS - DELTA_T_MS) as i64
}

/// `(time, full)` of every new and full moon from `from` to `to`, oldest first.
pub fn moon_phases(from: i64, to: i64) -> Vec<(i64, bool)> {
    let lunation = |t: i64| (t as f64 / DAY_MS + UNIX_EPOCH_JD - FIRST_NEW_MOON) / SYNODIC_DAYS;
    // A half-lunation either side: a phase can fall that far off its mean.
    let mut half = ((lunation(from) - 0.5) * 2.0).floor() as i64;
    let last = ((lunation(to) + 0.5) * 2.0).ceil() as i64;
    let mut out = Vec::new();
    while half <= last {
        let at = phase_ms(half as f64 / 2.0);
        if at >= from && at <= to {
            out.push((at, half.rem_euclid(2) == 1));
        }
        half += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_iso;

    const HOUR: i64 = 3_600_000;

    fn near(at: i64, day: &str, minutes: i64) {
        let want = parse_iso(day).unwrap() + minutes * 60_000;
        assert!((at - want).abs() < 10 * 60_000, "{day}: off by {} s", (at - want) / 1000);
    }

    fn only(day: &str) -> (i64, bool) {
        let t = parse_iso(day).unwrap();
        let found = moon_phases(t, t + 24 * HOUR);
        assert_eq!(found.len(), 1, "{day}");
        found[0]
    }

    #[test]
    fn phases_fall_where_the_almanac_has_them() {
        let (new, full) = only("2000-01-06");
        assert!(!full);
        near(new, "2000-01-06", 18 * 60 + 14);
        let (eclipse, full) = only("2024-04-08");
        assert!(!full, "the April 2024 eclipse");
        near(eclipse, "2024-04-08", 18 * 60 + 21);
        let (moon, full) = only("2024-04-23");
        assert!(full);
        near(moon, "2024-04-23", 23 * 60 + 49);
        let (moon, full) = only("2025-11-05");
        assert!(full);
        near(moon, "2025-11-05", 13 * 60 + 19);
    }

    #[test]
    fn a_year_holds_about_a_dozen_of_each_and_they_take_turns() {
        let phases = moon_phases(parse_iso("2025-01-01").unwrap(), parse_iso("2026-01-01").unwrap());
        assert!((24..=26).contains(&phases.len()), "{}", phases.len());
        assert!(phases.windows(2).all(|w| w[1].0 > w[0].0 && w[0].1 != w[1].1));
        assert!(moon_phases(0, 0).is_empty());
    }
}
