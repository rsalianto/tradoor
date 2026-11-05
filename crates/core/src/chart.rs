//! What the chart draws besides the reader's own studies: the EMA 12/25 ribbon
//! and its crosses, EMA 200, the prior-session market profile, and TPO sessions.

use crate::indicators::profile::{profile_by_session, session_start_back, ProfileBasis, ProfileOptions};
use crate::indicators::{cross_over, cross_under, ema, Series};
use crate::{Candle, ChartTimeframe, Interval};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cross {
    /// Index into the candles — the bar the cross completed on.
    pub index: usize,
    pub bull: bool,
}

pub struct EngineSeries {
    pub ema12: Series,
    pub ema25: Series,
    pub ema200: Series,
    /// Raw EMA 12/25 crossings, by the same strict definition `ta.crossover` uses.
    pub crosses: Vec<Cross>,
}

pub fn engine_series(candles: &[Candle]) -> EngineSeries {
    engine_series_with(candles, (12, 25, 200))
}

/// The same with the reader's lengths: the ribbon's fast and slow, and the long average.
pub fn engine_series_with(candles: &[Candle], (fast, slow, long): (usize, usize, usize)) -> EngineSeries {
    let close: Series = candles.iter().map(|c| Some(c.close)).collect();
    let (ema12, ema25) = (ema(&close, fast), ema(&close, slow));
    let (up, down) = (cross_over(&ema12, &ema25), cross_under(&ema12, &ema25));
    let crosses = (0..candles.len())
        .filter_map(|i| match (up[i], down[i]) {
            (Some(true), _) => Some(Cross { index: i, bull: true }),
            (_, Some(true)) => Some(Cross { index: i, bull: false }),
            _ => None,
        })
        .collect();
    EngineSeries { ema200: ema(&close, long), ema12, ema25, crosses }
}

/// Which session a timeframe's profile describes, and which candles build it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileSpec {
    pub session: ChartTimeframe,
    pub from: Interval,
}

/// Intraday charts profile the prior DAY from hourly bars, daily ones the prior
/// WEEK from dailies, weekly ones the prior MONTH from dailies — the same ladder
/// the rules engine used. Monthly and up have no session above them.
pub fn profile_spec_for(tf: ChartTimeframe) -> Option<ProfileSpec> {
    use ChartTimeframe::*;
    match tf {
        M1 | M5 | M15 | M30 | H1 | H2 | H4 | H6 | H12 => Some(ProfileSpec { session: D1, from: Interval::H1 }),
        D1 | D2 | D3 => Some(ProfileSpec { session: W1, from: Interval::D1 }),
        W1 | W2 => Some(ProfileSpec { session: Mo1, from: Interval::D1 }),
        Mo1 | Mo3 | Mo6 | Mo12 => None,
    }
}

/// A week holds 5–7 daily bars and a holiday week 4; the rules asked for 4.
pub const PROFILE_MIN_BARS: usize = 4;

pub struct ProfileOverlay {
    pub vah: Series,
    pub val: Series,
    pub poc: Series,
    /// Bars with a level. Usually partial: the source candles rarely reach as
    /// far back as the chart does, and a missing level is `None`, never a stale
    /// value carried back through history.
    pub covered: usize,
}

/// Prior-session levels, index-aligned with `candles`. The level on bar i comes
/// from the session BEFORE the one bar i sits in, so it can never move.
pub fn profile_overlay(candles: &[Candle], source: &[Candle], spec: ProfileSpec, basis: ProfileBasis) -> ProfileOverlay {
    let n = candles.len();
    let mut out = ProfileOverlay { vah: vec![None; n], val: vec![None; n], poc: vec![None; n], covered: 0 };
    if source.is_empty() {
        return out;
    }
    let profiles = profile_by_session(source, spec.session, &ProfileOptions { basis, ..Default::default() }, PROFILE_MIN_BARS);
    for (i, c) in candles.iter().enumerate() {
        if let Some((_, p)) = profiles.binary_search_by_key(&session_start_back(c.open_time, spec.session, 1), |e| e.0).ok().map(|at| &profiles[at]) {
            out.vah[i] = Some(p.vah);
            out.val[i] = Some(p.val);
            out.poc[i] = Some(p.poc);
            out.covered += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crosses_are_found_by_index() {
        let closes: Vec<f64> = (0..120).map(|i| 100.0 + ((i as f64) / 9.0).sin() * 10.0).collect();
        let candles = crate::fixtures::candles_from_closes(&closes, 1.0);
        let s = engine_series(&candles);
        assert!(!s.crosses.is_empty());
        for c in &s.crosses {
            let (f, sl) = (s.ema12[c.index].unwrap(), s.ema25[c.index].unwrap());
            assert_eq!(c.bull, f > sl);
        }
        assert_eq!(crate::fixtures::warmup_length(&s.ema200), 120);
    }
}
