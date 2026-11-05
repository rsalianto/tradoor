//! Where axis labels go.

use tradoor_core::time::{civil, DAY};

use crate::scale::{PriceScale, ScaleMode};

pub fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let pow = 10f64.powi(raw.log10().floor() as i32);
    [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().map(|m| m * pow).find(|s| *s >= raw * (1.0 - 1e-9)).unwrap_or(10.0 * pow)
}

/// Round numbers spaced about `gap` pixels apart. On a log axis they are placed
/// evenly in log space and rounded to two significant figures.
pub fn price_ticks(scale: &PriceScale, gap: f64) -> Vec<f64> {
    let (p_top, p_bottom) = (scale.price(scale.top), scale.price(scale.top + scale.height));
    let (lo, hi) = (p_bottom.min(p_top), p_bottom.max(p_top));
    let count = (scale.height / gap).floor().max(1.0);
    let mut out = Vec::new();
    match scale.mode {
        ScaleMode::Normal => {
            let step = nice_step((hi - lo) / count);
            let mut v = (lo / step).ceil() * step;
            while v <= hi && out.len() < 64 {
                out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
                v += step;
            }
        }
        ScaleMode::Log => {
            let (tlo, thi) = (scale.transform(lo.max(1e-12)), scale.transform(hi.max(1e-12)));
            for k in 0..=count as usize {
                let p = scale.untransform(tlo + (thi - tlo) * k as f64 / count);
                let mag = 10f64.powi(p.log10().floor() as i32 - 1);
                let rounded = (p / mag).round() * mag;
                if rounded >= lo && rounded <= hi && out.last().is_none_or(|last: &f64| (rounded - last).abs() > mag * 0.5) {
                    out.push(rounded);
                }
            }
        }
    }
    out
}

/// Decimals a tick step needs to be told apart from its neighbours.
pub fn step_decimals(ticks: &[f64]) -> usize {
    let step = ticks.windows(2).map(|w| (w[1] - w[0]).abs()).fold(f64::INFINITY, f64::min);
    if !step.is_finite() || step <= 0.0 {
        return 2;
    }
    ((-step.log10()).ceil().max(0.0) as usize).min(8)
}

/// How significant a bar boundary is: a new year outranks a new month, and so on
/// down to minutes. Labels are placed most significant first.
pub fn boundary_weight(prev: Option<i64>, t: i64) -> u8 {
    let Some(prev) = prev else { return 7 };
    let (a, b) = (civil(prev), civil(t));
    if a.year != b.year {
        7
    } else if a.month0 != b.month0 {
        6
    } else if a.day != b.day {
        5
    } else if a.hour != b.hour {
        if b.hour % 6 == 0 { 4 } else { 3 }
    } else if b.minute % 30 == 0 {
        2
    } else if b.minute % 15 == 0 {
        1
    } else {
        0
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub fn tick_label(t: i64, weight: u8) -> String {
    let c = civil(t);
    match weight {
        7 => format!("{}", c.year),
        6 => MONTHS[c.month0 as usize].to_string(),
        5 => format!("{}", c.day),
        _ => format!("{:02}:{:02}", c.hour, c.minute),
    }
}

/// The crosshair's time: minutes on intraday bars, the date otherwise.
pub fn crosshair_label(t: i64, bar_ms: Option<i64>, monthly: bool) -> String {
    let c = civil(t);
    let yy = c.year.rem_euclid(100);
    if monthly {
        return format!("{} '{yy:02}", MONTHS[c.month0 as usize]);
    }
    let day = format!("{} {:02} {} '{yy:02}", DAYS[tradoor_core::time::weekday(t) as usize], c.day, MONTHS[c.month0 as usize]);
    match bar_ms {
        Some(ms) if ms < DAY => format!("{day} {:02}:{:02}", c.hour, c.minute),
        _ => day,
    }
}

/// Pick labels most significant first, never closer than `gap` pixels.
pub fn pick_time_labels(candidates: &[(u8, f32, i64)], gap: f32) -> Vec<(f32, String)> {
    let mut taken: Vec<f32> = Vec::new();
    let mut out = Vec::new();
    // Candidates stand in order along the axis, so taking each weight in turn
    // needs no sort, and there are only a few weights.
    let heaviest = candidates.iter().map(|c| c.0).max().unwrap_or(0);
    for &(weight, x, t) in (0..=heaviest).rev().flat_map(|w| candidates.iter().filter(move |c| c.0 == w)) {
        let pos = taken.partition_point(|v| *v < x);
        let clear = taken.get(pos).is_none_or(|n| n - x >= gap) && (pos == 0 || x - taken[pos - 1] >= gap);
        if clear {
            taken.insert(pos, x);
            out.push((x, tick_label(t, weight)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradoor_core::time::parse_iso;

    #[test]
    fn steps_are_round() {
        assert_eq!(nice_step(3.2), 5.0);
        assert_eq!(nice_step(0.00018), 0.0002);
        assert_eq!(nice_step(21.0), 25.0);
    }

    #[test]
    fn ticks_cover_the_visible_range() {
        let mut s = PriceScale::new(ScaleMode::Normal, 0.0, 400.0, (0.0, 0.0));
        s.fit(64_010.0, 64_950.0);
        let ticks = price_ticks(&s, 48.0);
        assert!(ticks.len() >= 4 && ticks.len() <= 9, "{ticks:?}");
        assert!(ticks.windows(2).all(|w| w[1] > w[0]));
        let mut l = PriceScale::new(ScaleMode::Log, 0.0, 400.0, (0.0, 0.0));
        l.fit(1.0, 100_000.0);
        assert!(price_ticks(&l, 48.0).len() >= 5);
        assert_eq!(step_decimals(&[0.001, 0.002]), 3);
    }

    #[test]
    fn boundaries_rank_and_labels_space_out() {
        let t = |s| parse_iso(s).unwrap();
        assert_eq!(boundary_weight(Some(t("2025-12-31T23:00Z")), t("2026-01-01")), 7);
        assert_eq!(boundary_weight(Some(t("2026-08-31T23:00Z")), t("2026-09-01")), 6);
        assert_eq!(boundary_weight(Some(t("2026-09-01T11:00Z")), t("2026-09-01T12:00Z")), 4);
        assert_eq!(tick_label(t("2026-09-01"), 6), "Sep");
        let picked = pick_time_labels(&[(5, 100.0, t("2026-09-02")), (6, 120.0, t("2026-09-01")), (3, 200.0, t("2026-09-01T13:00Z"))], 60.0);
        assert_eq!(picked.iter().map(|p| p.1.as_str()).collect::<Vec<_>>(), ["Sep", "13:00"]);
        assert_eq!(crosshair_label(t("2026-09-14T13:05Z"), Some(60_000), false), "Mon 14 Sep '26 13:05");
    }
}
