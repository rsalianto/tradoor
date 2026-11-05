//! Market / volume profile: VAH, VAL, POC for one session.
//!
//! Reconstructed from OHLCV candles by spreading each candle across the price
//! buckets its range covers — an approximation. A candle's volume is spread
//! evenly, when in reality it clustered somewhere inside the range, so the levels
//! are ZONES rather than tick-exact prices.

use crate::num::Within;
use crate::{Candle, ChartTimeframe};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileBasis {
    /// Each candle contributes its volume, spread across its range.
    Volume,
    /// Each candle contributes one unit per bucket it touches — time at price.
    Tpo,
}

impl ProfileBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Tpo => "tpo",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProfileOptions {
    pub bins: usize,
    pub basis: ProfileBasis,
    pub value_area_pct: f64,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self { bins: 50, basis: ProfileBasis::Volume, value_area_pct: 0.7 }
    }
}

#[derive(Clone, Debug)]
pub struct SessionProfile {
    pub session_start: i64,
    pub poc: f64,
    pub vah: f64,
    pub val: f64,
    pub high: f64,
    pub low: f64,
    /// Index 0 is the lowest price bucket.
    pub buckets: Vec<f64>,
    pub bucket_size: f64,
    pub bar_count: usize,
    pub basis: ProfileBasis,
}

pub fn session_profile(candles: &[Candle], session_start: i64, opts: &ProfileOptions) -> Option<SessionProfile> {
    if candles.is_empty() {
        return None;
    }
    let bins = opts.bins.max(1);
    let high = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    if !high.is_finite() || !low.is_finite() {
        return None;
    }
    // A session that never moved has one price; bucket_size would be 0.
    if high <= low {
        let total = match opts.basis {
            ProfileBasis::Tpo => candles.len() as f64,
            ProfileBasis::Volume => candles.iter().map(|c| c.volume).sum(),
        };
        return Some(SessionProfile {
            session_start,
            poc: high,
            vah: high,
            val: high,
            high,
            low,
            buckets: vec![total],
            bucket_size: 0.0,
            bar_count: candles.len(),
            basis: opts.basis,
        });
    }

    let bucket_size = (high - low) / bins as f64;
    let mut buckets = vec![0.0; bins];
    for c in candles {
        add_candle(&mut buckets, c, low, bucket_size, opts.basis);
    }
    let poc_index = arg_max(&buckets);
    let (low_index, high_index) = value_area(&buckets, poc_index, opts.value_area_pct);
    Some(SessionProfile {
        session_start,
        poc: low + (poc_index as f64 + 0.5) * bucket_size,
        // Outer edges of the bounding buckets: "price above VAH" means price left
        // the value area entirely, not that it entered its topmost bucket.
        vah: low + (high_index as f64 + 1.0) * bucket_size,
        val: low + low_index as f64 * bucket_size,
        high,
        low,
        buckets,
        bucket_size,
        bar_count: candles.len(),
        basis: opts.basis,
    })
}

fn add_candle(buckets: &mut [f64], candle: &Candle, low: f64, bucket_size: f64, basis: ProfileBasis) {
    let bins = buckets.len();
    let lo = candle.low.max(low);
    let hi = candle.high.min(low + bins as f64 * bucket_size);
    let index = |price: f64| ((price - low) / bucket_size).floor().within(0.0, (bins - 1) as f64) as usize;
    let (first, last) = (index(lo), index(hi));

    if basis == ProfileBasis::Tpo {
        for b in &mut buckets[first..=last] {
            *b += 1.0;
        }
        return;
    }
    let weight = candle.volume;
    if !(weight > 0.0) {
        return;
    }
    let span = hi - lo;
    if span <= 0.0 || first == last {
        buckets[first] += weight;
        return;
    }
    // Proportional to how much of the range falls in each bucket, so the POC does
    // not jump around as the bin count changes.
    for (k, b) in buckets.iter_mut().enumerate().take(last + 1).skip(first) {
        let bucket_low = low + k as f64 * bucket_size;
        let overlap = hi.min(bucket_low + bucket_size) - lo.max(bucket_low);
        if overlap > 0.0 {
            *b += weight * (overlap / span);
        }
    }
}

/// Steidlmayer expansion: from the POC, compare the TWO buckets above against the
/// TWO below and take the heavier pair until `pct` of activity is enclosed. Ties
/// go upward. Returns `(low_index, high_index)`.
pub fn value_area(buckets: &[f64], poc_index: usize, pct: f64) -> (usize, usize) {
    let total: f64 = buckets.iter().sum();
    if total <= 0.0 {
        return (poc_index, poc_index);
    }
    let at = |i: i64| -> Option<f64> { usize::try_from(i).ok().and_then(|i| buckets.get(i).copied()) };
    let n = buckets.len() as i64;
    let (mut lo, mut hi) = (poc_index as i64, poc_index as i64);
    let target = total * pct;
    let mut acc = at(lo).unwrap_or(0.0);

    while acc < target && (lo > 0 || hi < n - 1) {
        let (up_a, up_b, down_a, down_b) = (at(hi + 1), at(hi + 2), at(lo - 1), at(lo - 2));
        let pair = |a: Option<f64>, b: Option<f64>| match a {
            Some(a) if a >= 0.0 => a + b.unwrap_or(0.0),
            _ => -1.0,
        };
        let (up_sum, down_sum) = (pair(up_a, up_b), pair(down_a, down_b));
        if up_sum < 0.0 && down_sum < 0.0 {
            break;
        }
        if up_sum >= down_sum {
            acc += up_a.unwrap_or(0.0);
            hi += 1;
            if let (true, Some(b)) = (acc < target, up_b) {
                acc += b;
                hi += 1;
            }
        } else {
            acc += down_a.unwrap_or(0.0);
            lo -= 1;
            if let (true, Some(b)) = (acc < target, down_b) {
                acc += b;
                lo -= 1;
            }
        }
    }
    (lo as usize, hi as usize)
}

/// Profile every session in `candles`. Sessions with fewer than `min_bars` are
/// dropped: three hourly candles of a 24h session are noise wearing a value
/// area's name.
/// Oldest-first candles into their sessions, oldest first.
pub(crate) fn by_session(candles: &[Candle], session: ChartTimeframe) -> Vec<(i64, Vec<Candle>)> {
    let mut groups: Vec<(i64, Vec<Candle>)> = Vec::new();
    for c in candles {
        let start = session.align(c.open_time);
        match groups.last_mut() {
            Some(group) if group.0 == start => group.1.push(*c),
            _ => groups.push((start, vec![*c])),
        }
    }
    groups
}

pub fn profile_by_session(
    candles: &[Candle],
    session: ChartTimeframe,
    opts: &ProfileOptions,
    min_bars: usize,
) -> Vec<(i64, SessionProfile)> {
    by_session(candles, session)
        .into_iter()
        .filter(|(_, g)| g.len() >= min_bars)
        .filter_map(|(start, g)| session_profile(&g, start, opts).map(|p| (start, p)))
        .collect()
}

/// The start of the session `back` sessions before the one containing `ts`.
/// `back >= 1` means the session has FULLY CLOSED as of `ts`.
pub fn session_start_back(ts: i64, session: ChartTimeframe, back: usize) -> i64 {
    let mut start = session.align(ts);
    for _ in 0..back {
        start = session.align(start - 1);
    }
    start
}

fn arg_max(values: &[f64]) -> usize {
    let mut best = 0;
    for (i, v) in values.iter().enumerate().skip(1) {
        if *v > values[best] {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{DAY, HOUR};

    fn session(bars: &[(f64, f64, f64)], start: i64) -> Vec<Candle> {
        bars.iter()
            .enumerate()
            .map(|(i, &(low, high, volume))| Candle {
                open_time: start + i as i64 * HOUR,
                open: low,
                high,
                low,
                close: high,
                volume,
                quote_volume: None,
                taker_buy: None,
            })
            .collect()
    }

    fn humped(center: f64, bars: usize, start: i64) -> Vec<Candle> {
        let rows: Vec<_> = (0..bars)
            .map(|i| {
                let offset = i as f64 - bars as f64 / 2.0;
                let low = center + offset * 0.5;
                (low, low + 0.5, 1000.0 * (-(offset * offset) / 8.0).exp())
            })
            .collect();
        session(&rows, start)
    }

    #[test]
    fn empty_and_flat_sessions() {
        assert!(session_profile(&[], 0, &ProfileOptions::default()).is_none());
        let p = session_profile(&session(&[(100.0, 100.0, 500.0), (100.0, 100.0, 500.0)], 0), 0, &ProfileOptions::default()).unwrap();
        assert_eq!((p.poc, p.vah, p.val, p.bucket_size), (100.0, 100.0, 100.0, 0.0));
    }

    #[test]
    fn poc_sits_on_the_hump_inside_the_value_area() {
        let p = session_profile(&humped(100.0, 24, 0), 0, &ProfileOptions::default()).unwrap();
        assert!((p.poc - 100.0).abs() <= p.bucket_size * 2.0);
        assert!(p.val <= p.poc && p.poc <= p.vah);
        assert!(p.val >= p.low && p.vah <= p.high);
        let total: f64 = p.buckets.iter().sum();
        let inside: f64 = p
            .buckets
            .iter()
            .enumerate()
            .filter(|(k, _)| {
                let mid = p.low + (*k as f64 + 0.5) * p.bucket_size;
                mid >= p.val && mid <= p.vah
            })
            .map(|(_, b)| b)
            .sum();
        let share = inside / total;
        assert!((0.7..0.92).contains(&share), "share {share}");
    }

    #[test]
    fn wider_pct_is_wider_and_bases_differ() {
        let candles = humped(100.0, 24, 0);
        let narrow = session_profile(&candles, 0, &ProfileOptions { value_area_pct: 0.5, ..Default::default() }).unwrap();
        let wide = session_profile(&candles, 0, &ProfileOptions { value_area_pct: 0.9, ..Default::default() }).unwrap();
        assert!(wide.vah - wide.val > narrow.vah - narrow.val);

        let candles = session(&[(100.0, 101.0, 10.0), (100.0, 101.0, 10.0), (100.0, 101.0, 10.0), (110.0, 111.0, 5000.0), (110.0, 111.0, 5000.0), (110.0, 111.0, 5000.0)], 0);
        let by_volume = session_profile(&candles, 0, &ProfileOptions::default()).unwrap();
        let by_tpo = session_profile(&candles, 0, &ProfileOptions { basis: ProfileBasis::Tpo, ..Default::default() }).unwrap();
        assert!(by_volume.poc > 105.0 && by_tpo.poc < 105.0);

        let p = session_profile(&session(&[(100.0, 200.0, 1000.0)], 0), 0, &ProfileOptions { bins: 10, ..Default::default() }).unwrap();
        assert!(p.buckets.iter().filter(|b| **b > 0.0).count() >= 9);
    }

    #[test]
    fn sessions_group_and_thin_ones_drop() {
        let mut candles = humped(100.0, 24, 0);
        candles.extend(humped(110.0, 24, DAY));
        let profiles = profile_by_session(&candles, ChartTimeframe::D1, &ProfileOptions::default(), 5);
        assert_eq!(profiles.len(), 2);
        assert!(profiles[1].0 == DAY && profiles[1].1.poc > profiles[0].1.poc);

        let mut candles = humped(100.0, 24, 0);
        candles.extend(humped(110.0, 2, DAY));
        let profiles = profile_by_session(&candles, ChartTimeframe::D1, &ProfileOptions::default(), 5);
        assert_eq!(profiles.iter().map(|p| p.0).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn back_one_is_the_prior_closed_session() {
        let mid_day2 = 2 * DAY + 13 * HOUR;
        assert_eq!(session_start_back(mid_day2, ChartTimeframe::D1, 1), DAY);
        assert_eq!(session_start_back(mid_day2, ChartTimeframe::D1, 2), 0);
        let first_monday = 4 * DAY;
        assert_eq!(session_start_back(first_monday + 3 * DAY, ChartTimeframe::W1, 1), first_monday - 7 * DAY);
    }
}
