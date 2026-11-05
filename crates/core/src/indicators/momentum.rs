//! RSI, Stochastic RSI, MACD — Pine Script definitions.

use crate::num::Within;
use super::series::{change, combine, ema, highest, lowest, rma, sma};
use super::Series;

/// `ta.rsi`. Wilder smoothing; the first value lands at index `len`, because the
/// change series only starts on the second bar.
pub fn rsi(src: &[Option<f64>], len: usize) -> Series {
    let diff = change(src, 1);
    let gains: Series = diff.iter().map(|d| d.map(|d| d.max(0.0))).collect();
    let losses: Series = diff.iter().map(|d| d.map(|d| (-d).max(0.0))).collect();
    let (avg_gain, avg_loss) = (rma(&gains, len), rma(&losses, len));
    (0..src.len())
        .map(|i| match (avg_gain[i], avg_loss[i]) {
            // Verbatim from Pine: down == 0 ? 100 : up == 0 ? 0 : 100 - 100/(1 + up/down).
            // On perfectly flat price TradingView yields 100, not 50.
            (Some(_), Some(l)) if l == 0.0 => Some(100.0),
            (Some(g), Some(_)) if g == 0.0 => Some(0.0),
            (Some(g), Some(l)) => Some(100.0 - 100.0 / (1.0 + g / l)),
            _ => None,
        })
        .collect()
}

pub struct StochRsi {
    pub raw: Series,
    pub k: Series,
    pub d: Series,
}

/// TradingView's "Stochastic RSI": the stochastic is taken over the RSI SERIES.
pub fn stoch_rsi(src: &[Option<f64>], rsi_len: usize, stoch_len: usize, smooth_k: usize, smooth_d: usize) -> StochRsi {
    let r = rsi(src, rsi_len);
    let (hi, lo) = (highest(&r, stoch_len), lowest(&r, stoch_len));
    let raw: Series = (0..src.len())
        .map(|i| match (r[i], hi[i], lo[i]) {
            // A flat RSI has no range; the value is undefined, not 0 or 100.
            (Some(v), Some(h), Some(l)) if h != l => Some(100.0 * (v - l) / (h - l)),
            _ => None,
        })
        .collect();
    // Bounded 0..100 by definition; residual float error is clamped so it cannot
    // cross a threshold it mathematically cannot reach.
    let clamp = |s: Series| -> Series { s.into_iter().map(|v| v.map(|v| v.within(0.0, 100.0))).collect() };
    let k = clamp(sma(&raw, smooth_k));
    let d = clamp(sma(&k, smooth_d));
    StochRsi { raw, k, d }
}

pub struct Macd {
    pub macd: Series,
    pub signal: Series,
    pub histogram: Series,
}

pub fn macd(src: &[Option<f64>], fast_len: usize, slow_len: usize, signal_len: usize) -> Macd {
    let line = combine(&ema(src, fast_len), &ema(src, slow_len), |f, s| f - s);
    let signal = ema(&line, signal_len);
    let histogram = combine(&line, &signal, |m, s| m - s);
    Macd { macd: line, signal, histogram }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{assert_close, some, synthetic_closes, warmup_length, EPS};

    fn pine_rsi(src: &[f64], len: usize) -> Series {
        let ch: Series = (0..src.len()).map(|i| (i > 0).then(|| src[i] - src[i - 1])).collect();
        let up = rma(&ch.iter().map(|c| c.map(|c| c.max(0.0))).collect::<Vec<_>>(), len);
        let down = rma(&ch.iter().map(|c| c.map(|c| -c.min(0.0))).collect::<Vec<_>>(), len);
        (0..src.len())
            .map(|i| match (up[i], down[i]) {
                (Some(_), Some(d)) if d == 0.0 => Some(100.0),
                (Some(u), Some(_)) if u == 0.0 => Some(0.0),
                (Some(u), Some(d)) => Some(100.0 - 100.0 / (1.0 + u / d)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_fixture_never_pins_rsi() {
        let closes = synthetic_closes(500, 100.0);
        let longest = closes.windows(2).fold((0, 0), |(best, run), w| {
            let run = if w[1] > w[0] { run + 1 } else { 0 };
            (best.max(run), run)
        });
        assert!(longest.0 < 14);
        assert!(rsi(&some(&closes), 14).into_iter().flatten().all(|v| v != 0.0 && v != 100.0));
    }

    #[test]
    fn rsi_hand_computed() {
        let out = rsi(&some(&[10.0, 11.0, 10.5, 12.0, 11.5, 13.0, 12.5]), 3);
        assert_eq!(&out[..3], &[None, None, None]);
        assert_close(out[3], 83.33333333333333, EPS);
        assert_close(out[4], 66.66666666666667, EPS);
        assert_close(out[5], 82.45614035087719, EPS);
        assert_close(out[6], 66.66666666666667, EPS);
        assert_eq!(warmup_length(&rsi(&some(&synthetic_closes(100, 100.0)), 14)), 14);
    }

    #[test]
    fn rsi_matches_the_pine_transcription() {
        let closes = synthetic_closes(400, 100.0);
        for len in [2, 7, 14, 21] {
            let (mine, reference) = (rsi(&some(&closes), len), pine_rsi(&closes, len));
            for i in 0..closes.len() {
                match reference[i] {
                    None => assert_eq!(mine[i], None),
                    Some(v) => assert_close(mine[i], v, 1e-9),
                }
            }
        }
        assert_close(rsi(&some(&[1., 2., 3., 4., 5., 6., 7., 8.]), 3)[7], 100.0, EPS);
        assert_close(rsi(&some(&[8., 7., 6., 5., 4., 3., 2., 1.]), 3)[7], 0.0, EPS);
    }

    #[test]
    fn stoch_rsi_warmups_and_bounds() {
        let closes = some(&synthetic_closes(500, 100.0));
        let s = stoch_rsi(&closes, 14, 14, 3, 3);
        assert_eq!((warmup_length(&s.raw), warmup_length(&s.k), warmup_length(&s.d)), (27, 29, 31));
        assert!(s.k.iter().chain(&s.d).flatten().all(|v| (0.0..=100.0).contains(v)));
        assert_eq!(s.raw[31..].iter().filter(|v| v.is_none()).count(), 0);
        let expected_k = sma(&s.raw, 3);
        for i in 0..closes.len() {
            match expected_k[i] {
                None => assert_eq!(s.k[i], None),
                Some(v) => assert_close(s.k[i], v, 1e-9),
            }
        }
        let rising: Vec<f64> = (0..60).map(|i| 100.0 + i as f64).collect();
        assert_eq!(stoch_rsi(&some(&rising), 14, 14, 3, 3).raw[50], None);
    }

    #[test]
    fn macd_warmups_stack() {
        let closes = some(&synthetic_closes(200, 100.0));
        let m = macd(&closes, 12, 26, 9);
        assert_eq!((warmup_length(&m.macd), warmup_length(&m.signal), warmup_length(&m.histogram)), (25, 33, 33));
        assert_close(m.histogram[150], m.macd[150].unwrap() - m.signal[150].unwrap(), EPS);
    }
}
