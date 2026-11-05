//! tradoor-core — the maths behind the chart.
//!
//! Ported from rsalianto's `lib/trading/core`. The same crate runs on the server
//! (backfill, recorder) and inside the browser worker as WASM, so a number drawn
//! on the chart and a number computed anywhere else come from one
//! implementation. Nothing here does I/O or reads a clock: `now` is always
//! passed in, which is what keeps every function testable.
#![forbid(unsafe_code)]

pub mod candle;
pub mod chart;
pub mod format;
pub mod num;
pub mod sort;
pub mod indicators;
pub mod live;
pub mod studies;
pub mod time;
pub mod timeframe;

pub use candle::Candle;
pub use indicators::Series;
pub use timeframe::{ChartTimeframe, Interval};

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::Candle;

    pub const EPS: f64 = 1e-6;

    #[track_caller]
    pub fn assert_close(actual: Option<f64>, expected: f64, eps: f64) {
        let actual = actual.unwrap_or_else(|| panic!("got None, expected {expected}"));
        let diff = (actual - expected).abs();
        assert!(diff <= eps, "got {actual}, expected {expected} (diff {diff})");
    }

    pub fn warmup_length(series: &[Option<f64>]) -> usize {
        series.iter().take_while(|v| v.is_none()).count()
    }

    /// A deterministic, oscillating price series. The fast sine dominates the
    /// drift, so there is never a long run of rising bars that would pin RSI at
    /// 100 and make Stoch RSI undefined.
    pub fn synthetic_closes(n: usize, seed: f64) -> Vec<f64> {
        let mut price = seed;
        (0..n)
            .map(|i| {
                let i = i as f64;
                price += (i / 2.5).sin() * 3.0 + (i / 19.0).sin() * 1.2 + 0.03;
                format!("{price:.8}").parse().unwrap()
            })
            .collect()
    }

    pub fn some(values: &[f64]) -> Vec<Option<f64>> {
        values.iter().copied().map(Some).collect()
    }

    pub fn candles_from_closes(closes: &[f64], volume: f64) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| Candle {
                open_time: i as i64 * crate::time::DAY,
                open: if i == 0 { c } else { closes[i - 1] },
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                volume,
                quote_volume: Some(volume * c),
                taker_buy: None,
            })
            .collect()
    }
}
