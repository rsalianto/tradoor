//! Chart timeframes, exchange intervals, and folding one into the other.
//!
//! Boundaries must match TradingView exactly: all arithmetic is UTC, weeks start
//! on MONDAY, months on the 1st. Every chart timeframe maps to an exchange kline
//! interval, folded `group` bars at a time where the exchange has no native one.
//! Binance's native `3d` is deliberately NOT used: its buckets are anchored to the
//! epoch, so a 3-day bar would start on a different weekday from the 2-day bars
//! beside it. Folding every day multiple from `1d` with one anchoring rule is the
//! only way they agree with each other.

use crate::candle::Candle;
use crate::time::{civil, utc, DAY, FIRST_MONDAY_OFFSET, HOUR, MINUTE, WEEK};

/// An interval an exchange publishes klines at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Interval {
    M1,
    M5,
    M15,
    M30,
    H1,
    H2,
    H4,
    H6,
    H12,
    D1,
    W1,
    Mo1,
}

impl Interval {
    pub const ALL: [Interval; 12] = [
        Self::M1, Self::M5, Self::M15, Self::M30, Self::H1, Self::H2,
        Self::H4, Self::H6, Self::H12, Self::D1, Self::W1, Self::Mo1,
    ];

    /// Binance's spelling, which is also ours.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H2 => "2h",
            Self::H4 => "4h",
            Self::H6 => "6h",
            Self::H12 => "12h",
            Self::D1 => "1d",
            Self::W1 => "1w",
            Self::Mo1 => "1M",
        }
    }

    /// Case-sensitive on purpose: `1m` and `1M` differ only in case, and one is
    /// 43,200 times the other.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.as_str() == s)
    }

    /// Bybit takes minutes as a number and the rest as a letter.
    pub fn bybit(self) -> &'static str {
        match self {
            Self::M1 => "1",
            Self::M5 => "5",
            Self::M15 => "15",
            Self::M30 => "30",
            Self::H1 => "60",
            Self::H2 => "120",
            Self::H4 => "240",
            Self::H6 => "360",
            Self::H12 => "720",
            Self::D1 => "D",
            Self::W1 => "W",
            Self::Mo1 => "M",
        }
    }

    /// Length in milliseconds, or `None` for the month, which has no fixed one.
    pub fn ms(self) -> Option<i64> {
        Some(match self {
            Self::M1 => MINUTE,
            Self::M5 => 5 * MINUTE,
            Self::M15 => 15 * MINUTE,
            Self::M30 => 30 * MINUTE,
            Self::H1 => HOUR,
            Self::H2 => 2 * HOUR,
            Self::H4 => 4 * HOUR,
            Self::H6 => 6 * HOUR,
            Self::H12 => 12 * HOUR,
            Self::D1 => DAY,
            Self::W1 => WEEK,
            Self::Mo1 => return None,
        })
    }

    /// The start of the period `ts` belongs to.
    pub fn align(self, ts: i64) -> i64 {
        match self {
            Self::Mo1 => {
                let c = civil(ts);
                utc(c.year, i64::from(c.month0), 1)
            }
            Self::W1 => (ts - FIRST_MONDAY_OFFSET).div_euclid(WEEK) * WEEK + FIRST_MONDAY_OFFSET,
            fixed => {
                let ms = fixed.ms().expect("fixed interval");
                ts.div_euclid(ms) * ms
            }
        }
    }

    /// The instant the period that opened at `open_time` stops accepting trades.
    pub fn end(self, open_time: i64) -> i64 {
        match self {
            Self::Mo1 => {
                let c = civil(open_time);
                utc(c.year, i64::from(c.month0) + 1, 1)
            }
            fixed => open_time + fixed.ms().expect("fixed interval"),
        }
    }
}

/// A timeframe the chart can be read at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChartTimeframe {
    M1,
    M5,
    M15,
    M30,
    H1,
    H2,
    H4,
    H6,
    H12,
    D1,
    D2,
    D3,
    W1,
    W2,
    Mo1,
    Mo3,
    Mo6,
    Mo12,
}

/// Where a chart timeframe's candles come from: an exchange interval, and how
/// many of those bars make one of ours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub interval: Interval,
    pub group: u32,
}

impl ChartTimeframe {
    /// Shortest first — the order they are drawn in and the arrow keys step.
    pub const ALL: [ChartTimeframe; 18] = [
        Self::M1, Self::M5, Self::M15, Self::M30,
        Self::H1, Self::H2, Self::H4, Self::H6, Self::H12,
        Self::D1, Self::D2, Self::D3,
        Self::W1, Self::W2,
        Self::Mo1, Self::Mo3, Self::Mo6, Self::Mo12,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H2 => "2h",
            Self::H4 => "4h",
            Self::H6 => "6h",
            Self::H12 => "12h",
            Self::D1 => "1d",
            Self::D2 => "2d",
            Self::D3 => "3d",
            Self::W1 => "1w",
            Self::W2 => "2w",
            Self::Mo1 => "1M",
            Self::Mo3 => "3M",
            Self::Mo6 => "6M",
            Self::Mo12 => "12M",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tf| tf.as_str() == s)
    }

    /// TradingView's button spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::D1 => "D",
            Self::D2 => "2D",
            Self::D3 => "3D",
            Self::W1 => "W",
            Self::W2 => "2W",
            Self::Mo1 => "M",
            other => other.as_str(),
        }
    }

    pub fn source(self) -> Source {
        let (interval, group) = match self {
            Self::M1 => (Interval::M1, 1),
            Self::M5 => (Interval::M5, 1),
            Self::M15 => (Interval::M15, 1),
            Self::M30 => (Interval::M30, 1),
            Self::H1 => (Interval::H1, 1),
            Self::H2 => (Interval::H2, 1),
            Self::H4 => (Interval::H4, 1),
            Self::H6 => (Interval::H6, 1),
            Self::H12 => (Interval::H12, 1),
            Self::D1 => (Interval::D1, 1),
            Self::D2 => (Interval::D1, 2),
            Self::D3 => (Interval::D1, 3),
            Self::W1 => (Interval::W1, 1),
            Self::W2 => (Interval::W1, 2),
            Self::Mo1 => (Interval::Mo1, 1),
            Self::Mo3 => (Interval::Mo1, 3),
            Self::Mo6 => (Interval::Mo1, 6),
            Self::Mo12 => (Interval::Mo1, 12),
        };
        Source { interval, group }
    }

    /// The kline stream that carries this timeframe's forming bar: its own, or
    /// for a folded one — the exchange publishes nothing shaped like it — the
    /// days its history is kept at, combined into the bar as they stream.
    pub fn live_interval(self) -> Interval {
        let source = self.source();
        if source.group == 1 { source.interval } else { self.history_base() }
    }

    /// The resolution stored history is kept at for this timeframe. Everything
    /// else is folded from it, so a week built from stored days lands on the same
    /// Monday boundary as the exchange's own weekly kline.
    pub fn history_base(self) -> Interval {
        match self {
            Self::M1 | Self::M5 | Self::M15 | Self::M30 => Interval::M1,
            Self::H1 | Self::H2 | Self::H4 | Self::H6 | Self::H12 => Interval::H1,
            _ => Interval::D1,
        }
    }

    /// The start of the period `ts` belongs to. Anchored to the calendar, never to
    /// the data: a 2-day bar must not shift when the window scrolls.
    pub fn align(self, ts: i64) -> i64 {
        let Source { interval, group } = self.source();
        let group = i64::from(group);
        match interval {
            Interval::Mo1 => {
                let c = civil(ts);
                let months = c.year * 12 + i64::from(c.month0);
                let start = months.div_euclid(group) * group;
                utc(start.div_euclid(12), start.rem_euclid(12), 1)
            }
            Interval::W1 => {
                let weeks = (ts - FIRST_MONDAY_OFFSET).div_euclid(WEEK);
                weeks.div_euclid(group) * group * WEEK + FIRST_MONDAY_OFFSET
            }
            fixed => {
                let ms = fixed.ms().expect("fixed interval") * group;
                ts.div_euclid(ms) * ms
            }
        }
    }

    /// The instant a period stops accepting trades — the next period's start.
    pub fn end(self, open_time: i64) -> i64 {
        let Source { interval, group } = self.source();
        let group = i64::from(group);
        match interval {
            Interval::Mo1 => {
                let c = civil(open_time);
                utc(c.year, i64::from(c.month0) + group, 1)
            }
            Interval::W1 => open_time + group * WEEK,
            fixed => open_time + fixed.ms().expect("fixed interval") * group,
        }
    }
}

/// Fold candles at `tf.source().interval` (or any finer base whose boundaries
/// nest inside it) into `tf`, oldest first.
///
/// The last bucket is dropped unless it has CLOSED as of `now`: the forming bar
/// is drawn from the live stream, and a bucket assembled from stale REST klines
/// would sit under it arguing. A bucket missing bars inside it is kept — a gap in
/// the tape is not a reason to hide the period.
pub fn fold(source: &[Candle], tf: ChartTimeframe, now: i64) -> Vec<Candle> {
    let mut out: Vec<Candle> = Vec::new();
    for candle in source {
        let start = tf.align(candle.open_time);
        let quote = candle.quote_volume.unwrap_or(candle.volume * candle.close);
        match out.last_mut() {
            Some(bucket) if bucket.open_time == start => {
                bucket.high = bucket.high.max(candle.high);
                bucket.low = bucket.low.min(candle.low);
                bucket.close = candle.close;
                bucket.volume += candle.volume;
                bucket.quote_volume = Some(bucket.quote_volume.unwrap_or(0.0) + quote);
                // A bar's buyers are known only if every bar in it says.
                bucket.taker_buy = bucket.taker_buy.zip(candle.taker_buy).map(|(a, b)| a + b);
            }
            _ => out.push(Candle {
                open_time: start,
                open: candle.open,
                high: candle.high,
                low: candle.low,
                close: candle.close,
                volume: candle.volume,
                quote_volume: Some(quote),
                taker_buy: candle.taker_buy,
            }),
        }
    }
    while out.last().is_some_and(|c| tf.end(c.open_time) > now) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folded_bars_buyers_are_known_only_if_every_bar_in_it_says() {
        let day = 86_400_000;
        let bar = |d: i64, taker_buy| Candle { open_time: d * day, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 10.0, quote_volume: None, taker_buy };
        let folded = fold(&[bar(0, Some(3.0)), bar(1, Some(4.0)), bar(2, Some(5.0)), bar(3, None)], ChartTimeframe::D2, 10 * day);
        assert_eq!(folded.iter().map(|c| c.taker_buy).collect::<Vec<_>>(), [Some(7.0), None]);
    }
    use crate::time::{iso_minute, parse_iso, weekday};

    fn t(s: &str) -> i64 {
        parse_iso(s).unwrap()
    }

    fn daily(days: &[&str]) -> Vec<Candle> {
        days.iter()
            .enumerate()
            .map(|(i, d)| {
                let i = i as f64;
                Candle { open_time: t(d), open: i, high: i + 1.0, low: i - 1.0, close: i, volume: 1.0, quote_volume: None, taker_buy: None }
            })
            .collect()
    }

    #[test]
    fn hours_anchor_to_the_epoch() {
        assert_eq!(iso_minute(ChartTimeframe::H2.align(t("2026-09-12T13:59:00Z"))), "2026-09-12 12:00");
        assert_eq!(iso_minute(ChartTimeframe::H2.align(t("2026-09-12T14:00:00Z"))), "2026-09-12 14:00");
    }

    #[test]
    fn day_multiples_count_whole_days_from_the_epoch() {
        let a = ChartTimeframe::D2.align(t("2026-09-10T00:00:00Z"));
        let b = ChartTimeframe::D2.align(t("2026-09-11T23:59:59Z"));
        assert_eq!(a, b);
        assert_eq!(a, t("2026-09-10"));
    }

    #[test]
    fn week_multiples_stay_on_monday() {
        assert_eq!(weekday(ChartTimeframe::W2.align(t("2026-09-12"))), 1);
        assert_eq!(weekday(ChartTimeframe::W1.align(t("2026-09-13T23:00Z"))), 1);
    }

    #[test]
    fn month_multiples_land_on_quarters_halves_and_years() {
        assert_eq!(ChartTimeframe::Mo3.align(t("2026-05-17")), t("2026-04-01"));
        assert_eq!(ChartTimeframe::Mo6.align(t("2026-05-17")), t("2026-01-01"));
        assert_eq!(ChartTimeframe::Mo12.align(t("2026-12-31T23:59:59Z")), t("2026-01-01"));
    }

    #[test]
    fn ends_are_read_off_the_calendar() {
        assert_eq!(ChartTimeframe::D2.end(t("2026-09-10")), t("2026-09-12"));
        assert_eq!(ChartTimeframe::Mo3.end(t("2026-01-01")), t("2026-04-01"));
        assert_eq!(Interval::Mo1.end(t("2026-08-01")), t("2026-09-01"));
    }

    #[test]
    fn folds_pairs_taking_first_open_last_close_and_extremes() {
        let out = fold(&daily(&["2026-09-10", "2026-09-11", "2026-09-12", "2026-09-13"]), ChartTimeframe::D2, t("2026-09-15"));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].open_time, t("2026-09-10"));
        assert_eq!((out[0].open, out[0].close, out[0].high, out[0].low, out[0].volume), (0.0, 1.0, 2.0, -1.0, 2.0));
    }

    #[test]
    fn the_forming_bucket_is_dropped() {
        let out = fold(&daily(&["2026-09-10", "2026-09-11", "2026-09-12"]), ChartTimeframe::D2, t("2026-09-13T10:00Z"));
        assert_eq!(out.len(), 1);
        let out = fold(&daily(&["2026-09-10", "2026-09-11", "2026-09-12"]), ChartTimeframe::D1, t("2026-09-12T10:00Z"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_bucket_missing_bars_is_kept_once_closed() {
        let out = fold(&daily(&["2026-09-10", "2026-09-13"]), ChartTimeframe::D2, t("2026-09-20"));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].volume, 1.0);
    }

    #[test]
    fn weekly_folded_from_days_matches_monday_boundaries() {
        let days: Vec<String> = (0..14).map(|i| crate::time::iso_date(t("2026-08-31") + i * DAY)).collect();
        let refs: Vec<&str> = days.iter().map(String::as_str).collect();
        let out = fold(&daily(&refs), ChartTimeframe::W1, t("2026-09-20"));
        assert_eq!(out.iter().map(|c| c.open_time).collect::<Vec<_>>(), vec![t("2026-08-31"), t("2026-09-07")]);
    }

    #[test]
    fn minutes_and_months_are_different_timeframes() {
        assert_ne!(ChartTimeframe::M1.source(), ChartTimeframe::Mo1.source());
        assert_eq!(ChartTimeframe::parse("1M"), Some(ChartTimeframe::Mo1));
        assert_eq!(ChartTimeframe::Mo12.source().group, 12);
        assert_eq!(ChartTimeframe::D3.live_interval(), Interval::D1);
        assert_eq!(ChartTimeframe::W1.live_interval(), Interval::W1);
        assert_eq!(ChartTimeframe::W2.live_interval(), Interval::D1, "built from the days its history is kept at, not from weeks");
        assert_eq!(ChartTimeframe::Mo3.live_interval(), Interval::D1);
    }

    #[test]
    fn the_countdown_boundary_is_the_period_now_is_in() {
        let now = t("2026-09-13T14:37:12Z");
        assert_eq!(ChartTimeframe::H4.end(ChartTimeframe::H4.align(now)), t("2026-09-13T16:00Z"));
        let now = t("2026-09-13T06:00Z");
        assert_eq!(ChartTimeframe::D2.end(ChartTimeframe::D2.align(now)), t("2026-09-14"));
        assert_eq!(ChartTimeframe::Mo1.end(ChartTimeframe::Mo1.align(t("2026-02-10"))), t("2026-03-01"));
    }
}
