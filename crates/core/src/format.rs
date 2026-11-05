//! Price precision and formatting for axis labels and readouts.
//!
//! Crypto prices span 0.00000285 to 63,749 in one list, so precision is read off
//! the instrument's own traded prices rather than pinned.

pub const MAX_PRICE_DECIMALS: usize = 8;

/// A decimal number, `-12.5` or `1e-7`, as a page writes one. The standard parser
/// brings a 10 kB table to the wasm for digits nobody sends here: to 15 significant
/// digits this reads the same to the last bit, and no number sent has more.
pub fn parse_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let negative = *b.first()? == b'-';
    let mut i = usize::from(negative || b[0] == b'+');
    let (mut digits, mut scale, mut mantissa, mut dot) = (0, 0i32, 0.0f64, false);
    while let Some(c) = b.get(i) {
        match c {
            b'0'..=b'9' => {
                mantissa = mantissa * 10.0 + f64::from(c - b'0');
                digits += 1;
                scale -= i32::from(dot);
            }
            b'.' if !dot => dot = true,
            _ => break,
        }
        i += 1;
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() {
        if b[i] != b'e' && b[i] != b'E' {
            return None;
        }
        scale += s.get(i + 1..)?.parse::<i32>().ok()?;
    }
    let v = if scale < 0 { mantissa / 10f64.powi(-scale) } else { mantissa * 10f64.powi(scale) };
    Some(if negative { -v } else { v })
}

/// `v` to `decimals` places, as `{v:.decimals$}` writes it: a tie goes to the even
/// digit. The standard printer weighs some 20 kB of wasm with its tables, to get
/// digits past what a double holds right, and nothing here asks for those.
pub fn fixed(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return (if v.is_nan() { "NaN" } else if v > 0.0 { "inf" } else { "-inf" }).to_string();
    }
    let a = v.abs();
    let sign = if v.is_sign_negative() { "-" } else { "" };
    let scale = 10f64.powi(decimals as i32);
    let whole = a.trunc();
    // The product and what rounding it dropped (Dekker), so a figure just under
    // a half is not taken for one: 1.115 is below 1.115 as far as a double goes.
    let f = a - whole;
    let split = |x: f64| {
        let c = 134_217_729.0 * x;
        let high = c - (c - x);
        (high, x - high)
    };
    let ((fh, fl), (sh, sl)) = (split(f), split(scale));
    let product = f * scale;
    let lost = ((fh * sh - product) + fh * sl + fl * sh) + fl * sl;
    let mut part = product.floor();
    let past_half = (product - (part + 0.5)) + lost;
    if past_half > 0.0 || (past_half == 0.0 && (if decimals == 0 { whole } else { part }) % 2.0 == 1.0) {
        part += 1.0;
    }
    let (mut int, mut part) = (whole as u64, part as u64);
    if part >= scale as u64 {
        int += 1;
        part = 0;
    }
    if decimals == 0 { format!("{sign}{int}") } else { format!("{sign}{int}.{part:0decimals$}") }
}

/// `v` with the decimals it carries: `50`, `2.5`.
pub fn plain(v: f64) -> String {
    fixed(v, decimals_of(v))
}

/// Decimals a number carries in its shortest round-trip form: `1.2e-7` reads as
/// `0.00000012` → 8.
pub fn decimals_of(v: f64) -> usize {
    if !v.is_finite() {
        return 0;
    }
    let a = v.abs();
    (0..17).find(|d| {
        let scale = 10f64.powi(*d);
        (a * scale).round() / scale == a
    }).map_or(17, |d| d as usize)
}

/// The precision an instrument trades at, from PRICES ONLY — an indicator carries
/// seventeen decimals of float noise and would push everything to the cap. Two
/// decimals is the floor.
pub fn price_precision(values: impl IntoIterator<Item = f64>) -> usize {
    let mut digits = 2;
    for v in values {
        if v.is_finite() {
            digits = digits.max(decimals_of(v));
            if digits >= MAX_PRICE_DECIMALS {
                return MAX_PRICE_DECIMALS;
            }
        }
    }
    digits
}

fn bucket(abs: f64) -> usize {
    match abs {
        a if a == 0.0 || a >= 1000.0 => 2,
        a if a >= 1.0 => 4,
        a if a >= 0.01 => 5,
        a if a >= 0.0001 => 6,
        _ => MAX_PRICE_DECIMALS,
    }
}

/// Decimals for one quoted price when the instrument's precision is unknown.
pub fn price_decimals(v: f64) -> usize {
    if !v.is_finite() {
        return 2;
    }
    MAX_PRICE_DECIMALS.min(bucket(v.abs()).max(decimals_of(v)))
}

/// `62,685.10`. `None` digits falls back to `price_decimals`, which is only safe
/// for a traded price.
pub fn format_price(v: f64, digits: Option<usize>) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    let d = digits.unwrap_or_else(|| price_decimals(v));
    let text = fixed(v.abs(), d);
    let (int, frac) = text.split_once('.').map_or((text.as_str(), None), |(i, f)| (i, Some(f)));
    let mut grouped = String::with_capacity(text.len() + int.len() / 3 + 1);
    if v < 0.0 {
        grouped.push('-');
    }
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if let Some(frac) = frac {
        grouped.push('.');
        grouped.push_str(frac);
    }
    grouped
}

/// `27.4M`, `1.2B`.
pub fn format_turnover(v: f64) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    match v.abs() {
        a if a >= 1e12 => fixed(v / 1e12, 1) + "T",
        a if a >= 1e9 => fixed(v / 1e9, 1) + "B",
        a if a >= 1e6 => fixed(v / 1e6, 1) + "M",
        a if a >= 1e3 => fixed(v / 1e3, 0) + "K",
        _ => fixed(v, 0),
    }
}

/// A large figure to four significant-ish digits: `5.327M`, `106.2K`, `12.35`.
pub fn format_compact(v: f64) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    let (scaled, unit) = match v.abs() {
        a if a >= 1e12 => (v / 1e12, "T"),
        a if a >= 1e9 => (v / 1e9, "B"),
        a if a >= 1e6 => (v / 1e6, "M"),
        a if a >= 1e3 => (v / 1e3, "K"),
        _ => return fixed(v, 2),
    };
    let decimals = if scaled.abs() >= 100.0 { 1 } else if scaled.abs() >= 10.0 { 2 } else { 3 };
    fixed(scaled, decimals) + unit
}

/// Time left until a bar closes: `04:12`, `2:05:33`, or `3d 04h`.
pub fn format_countdown(ms: i64) -> String {
    if ms <= 0 {
        return "00:00".to_string();
    }
    let total = ms / 1000;
    let (days, hours, minutes, seconds) = (total / 86_400, total % 86_400 / 3600, total % 3600 / 60, total % 60);
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixed_writes_what_the_standard_printer_writes() {
        use super::{fixed, plain};
        let mut x = 12345u32;
        let mut values = vec![0.0, -0.0, 0.5, 1.5, 2.5, 0.25, 0.75, 0.125, 1.005, 1.115, 0.999, 9.995, 99.95, 999.5, 80515.1, 62685.1, 0.00000012, 1e-9, 1234567.891, 1e15, -0.001, -2.5, 0.05, 0.15, 0.35];
        for _ in 0..4000 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let digits = (x >> 8) % 1_000_000;
            // Prices as exchanges quote them, and figures worked out from them.
            values.push(f64::from(digits) / 10f64.powi((x % 7) as i32));
            values.push(f64::from(digits) * 1.000_123_7 / 10f64.powi((x % 9) as i32) - 300.0);
        }
        for v in values {
            for d in 0..=8 {
                assert_eq!(fixed(v, d), format!("{v:.d$}"), "{v} to {d}");
            }
        }
        for v in [50.0, 2.5, 0.3, 1000.0, 255.0, 0.25, 14.0, 0.0] {
            assert_eq!(plain(v), v.to_string());
        }
        assert_eq!((fixed(f64::NAN, 2), fixed(f64::INFINITY, 2)), ("NaN".to_string(), "inf".to_string()));
    }

    #[test]
    fn numbers_read_as_the_standard_parser_reads_them() {
        use super::parse_number;
        for s in ["0", "1", "-1", "+2.5", "0.1", "0.3", "1.15", "80515.10", ".5", "5.", "1e3", "1E3", "1e-7", "1.5e-8", "-2.5e2", "0.00000012", "123456789.123456", "1000000000", "999999999999999"] {
            assert_eq!(parse_number(s), s.parse::<f64>().ok(), "{s}");
        }
        for s in ["", "-", "+", ".", "abc", "1.2.3", "1e", "1e+", "12px", "--1", "nan", "inf", " 1"] {
            assert_eq!(parse_number(s), None, "{s}");
        }
    }

    #[test]
    fn compact_figures() {
        assert_eq!(super::format_compact(5_327_000.0), "5.327M");
        assert_eq!(super::format_compact(106_188.6), "106.2K");
        assert_eq!(super::format_compact(12_345_678_901.0), "12.35B");
        assert_eq!(super::format_compact(12.345), "12.35");
    }

    use super::*;

    #[test]
    fn decimals_see_through_exponents() {
        assert_eq!((decimals_of(62685.12), decimals_of(0.2479), decimals_of(0.0062015), decimals_of(100.0)), (2, 4, 7, 0));
        assert_eq!((decimals_of(0.00000012), decimals_of(1e-7), decimals_of(2.85e-6)), (8, 7, 8));
    }

    #[test]
    fn precision_floors_caps_and_skips_gaps() {
        assert_eq!(price_precision([0.24, 0.2479, 0.248]), 4);
        assert_eq!(price_precision([100.0, 200.0]), 2);
        assert_eq!(price_precision([0.2612345678901234]), MAX_PRICE_DECIMALS);
        assert_eq!(price_precision([f64::NAN, 0.0062015]), 7);
    }

    #[test]
    fn prices_format_like_the_original() {
        assert_eq!(format_price(0.0062015, Some(7)), "0.0062015");
        assert_eq!(format_price(0.006, Some(7)), "0.0060000");
        assert_eq!(format_price(62685.1, Some(2)), "62,685.10");
        assert_eq!(format_price(0.2612345678901234, Some(4)), "0.2612");
        assert_eq!(format_price(0.0062015, None), "0.0062015");
        assert_eq!(format_price(0.24, None), "0.24000");
        assert_eq!(format_price(62685.1, None), "62,685.10");
        assert_eq!(format_price(0.2612345678901234, None), "0.26123457");
        assert_eq!(format_price(f64::NAN, None), "—");
        assert_eq!(format_price(-1234567.5, Some(1)), "-1,234,567.5");
    }

    #[test]
    fn countdown() {
        assert_eq!(format_countdown(12_000), "00:12");
        assert_eq!(format_countdown(72_000), "01:12");
        assert_eq!(format_countdown(59 * 60_000 + 59_000), "59:59");
        assert_eq!(format_countdown(2 * 3_600_000 + 5 * 60_000 + 33_000), "2:05:33");
        assert_eq!(format_countdown(3 * 86_400_000 + 4 * 3_600_000), "3d 04h");
        assert_eq!((format_countdown(0), format_countdown(-5_000)), ("00:00".to_string(), "00:00".to_string()));
    }
}
