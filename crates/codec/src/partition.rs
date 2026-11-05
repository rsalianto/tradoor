//! The partition file.
//!
//! ```text
//! magic "TDC1" | interval u8 | price_scale u8 | volume_scale u8 | quote_scale u8
//! flags u8 (1 quote volume, 2 taker buy volume, 4 trade count) | count u32 LE
//! first_open i64 LE
//! steps    uvarint × (count-1)   bars between consecutive opens (1 unless a gap)
//! open     ivarint × count       open[i] - close[i-1]  (open[0] as is)
//! high     ivarint × count       high - max(open, close)
//! low      ivarint × count       min(open, close) - low
//! close    ivarint × count       close - open
//! volume   uvarint × count
//! [quote volume, taker buy volume, trades]  uvarint × count each, when flagged
//! ```
//!
//! Columnar rather than row by row: a column of near-identical small deltas is
//! what gzip compresses best.

use tradoor_core::time::{civil, utc};
use tradoor_core::{Candle, Interval};

use crate::decimal::{parse_decimal, rescale, to_f64};
use crate::varint::{put_i, put_u, Reader};

pub const MAGIC: &[u8; 4] = b"TDC1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    BadMagic,
    Truncated,
    TrailingBytes,
    UnknownInterval(u8),
    /// A field that is not a plain decimal, or does not fit in 64 bits.
    BadNumber(String),
    NotAscending(i64),
    Misaligned(i64),
    MixedIntervals,
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a tradoor partition (bad magic)"),
            Self::Truncated => write!(f, "partition is truncated"),
            Self::TrailingBytes => write!(f, "partition has trailing bytes"),
            Self::UnknownInterval(c) => write!(f, "unknown interval code {c}"),
            Self::BadNumber(s) => write!(f, "not a decimal number: {s}"),
            Self::NotAscending(t) => write!(f, "open times are not ascending at {t}"),
            Self::Misaligned(t) => write!(f, "open time {t} is not on an interval boundary"),
            Self::MixedIntervals => write!(f, "cannot merge partitions of different intervals"),
        }
    }
}

impl std::error::Error for CodecError {}

/// One kline as the exchange sent it: decimal strings, parsed exactly.
#[derive(Debug, Clone, Copy)]
pub struct RawCandle<'a> {
    pub open_time: i64,
    pub open: &'a str,
    pub high: &'a str,
    pub low: &'a str,
    pub close: &'a str,
    pub volume: &'a str,
    pub quote_volume: Option<&'a str>,
    pub taker_buy_volume: Option<&'a str>,
    pub trades: Option<u64>,
}

/// Candles of one interval as integer ticks. `price_scale` is the number of
/// decimals every price column is expressed in, and likewise for the volumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickCandles {
    pub interval: Interval,
    pub price_scale: u8,
    pub volume_scale: u8,
    pub quote_scale: u8,
    pub open_time: Vec<i64>,
    pub open: Vec<i64>,
    pub high: Vec<i64>,
    pub low: Vec<i64>,
    pub close: Vec<i64>,
    pub volume: Vec<u64>,
    pub quote_volume: Option<Vec<u64>>,
    pub taker_buy_volume: Option<Vec<u64>>,
    pub trades: Option<Vec<u64>>,
}

fn interval_code(i: Interval) -> u8 {
    Interval::ALL.iter().position(|x| *x == i).expect("every interval is listed") as u8
}

fn interval_from(code: u8) -> Result<Interval, CodecError> {
    Interval::ALL.get(usize::from(code)).copied().ok_or(CodecError::UnknownInterval(code))
}

fn month_index(ts: i64) -> i64 {
    let c = civil(ts);
    c.year * 12 + i64::from(c.month0)
}

impl TickCandles {
    pub fn empty(interval: Interval) -> Self {
        Self {
            interval,
            price_scale: 0,
            volume_scale: 0,
            quote_scale: 0,
            open_time: Vec::new(),
            open: Vec::new(),
            high: Vec::new(),
            low: Vec::new(),
            close: Vec::new(),
            volume: Vec::new(),
            quote_volume: None,
            taker_buy_volume: None,
            trades: None,
        }
    }

    pub fn len(&self) -> usize {
        self.open_time.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open_time.is_empty()
    }

    /// Parse klines exactly. Scales are the finest precision any row carries;
    /// an optional column is kept only when every row has it.
    pub fn from_raw(interval: Interval, rows: &[RawCandle<'_>]) -> Result<Self, CodecError> {
        let dec = |s: &str| parse_decimal(s).ok_or_else(|| CodecError::BadNumber(s.to_string()));
        let mut parsed = Vec::with_capacity(rows.len());
        let (mut ps, mut vs, mut qs) = (0u8, 0u8, 0u8);
        let (mut has_quote, mut has_taker, mut has_trades) = (!rows.is_empty(), !rows.is_empty(), !rows.is_empty());
        for r in rows {
            let prices = [dec(r.open)?, dec(r.high)?, dec(r.low)?, dec(r.close)?];
            let volume = dec(r.volume)?;
            let quote = r.quote_volume.map(dec).transpose()?;
            let taker = r.taker_buy_volume.map(dec).transpose()?;
            ps = prices.iter().map(|p| p.1).fold(ps, u8::max);
            vs = vs.max(volume.1).max(taker.map_or(0, |t| t.1));
            qs = qs.max(quote.map_or(0, |q| q.1));
            has_quote &= quote.is_some();
            has_taker &= taker.is_some();
            has_trades &= r.trades.is_some();
            parsed.push((r.open_time, prices, volume, quote, taker, r.trades));
        }

        let mut out = Self { price_scale: ps, volume_scale: vs, quote_scale: qs, ..Self::empty(interval) };
        let (mut quote_col, mut taker_col, mut trades_col) = (Vec::new(), Vec::new(), Vec::new());
        let scaled = |(m, s): (i64, u8), to: u8, raw: &str| rescale(m, s, to).ok_or_else(|| CodecError::BadNumber(raw.to_string()));
        let unsigned = |v: i64, raw: &str| u64::try_from(v).map_err(|_| CodecError::BadNumber(raw.to_string()));
        for (r, (t, prices, volume, quote, taker, trades)) in rows.iter().zip(parsed) {
            out.open_time.push(t);
            out.open.push(scaled(prices[0], ps, r.open)?);
            out.high.push(scaled(prices[1], ps, r.high)?);
            out.low.push(scaled(prices[2], ps, r.low)?);
            out.close.push(scaled(prices[3], ps, r.close)?);
            out.volume.push(unsigned(scaled(volume, vs, r.volume)?, r.volume)?);
            if has_quote {
                let raw = r.quote_volume.unwrap_or_default();
                quote_col.push(unsigned(scaled(quote.expect("column present"), qs, raw)?, raw)?);
            }
            if has_taker {
                let raw = r.taker_buy_volume.unwrap_or_default();
                taker_col.push(unsigned(scaled(taker.expect("column present"), vs, raw)?, raw)?);
            }
            if has_trades {
                trades_col.push(trades.expect("column present"));
            }
        }
        out.quote_volume = has_quote.then_some(quote_col);
        out.taker_buy_volume = has_taker.then_some(taker_col);
        out.trades = has_trades.then_some(trades_col);
        out.validate_times()?;
        Ok(out)
    }

    fn validate_times(&self) -> Result<(), CodecError> {
        for (i, &t) in self.open_time.iter().enumerate() {
            if self.interval.align(t) != t {
                return Err(CodecError::Misaligned(t));
            }
            if i > 0 && t <= self.open_time[i - 1] {
                return Err(CodecError::NotAscending(t));
            }
        }
        Ok(())
    }

    fn steps_between(&self, a: i64, b: i64) -> Result<u64, CodecError> {
        if b <= a {
            return Err(CodecError::NotAscending(b));
        }
        let steps = match self.interval.ms() {
            Some(ms) if (b - a) % ms == 0 => (b - a) / ms,
            Some(_) => return Err(CodecError::Misaligned(b)),
            None => month_index(b) - month_index(a),
        };
        Ok(steps as u64)
    }

    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        self.validate_times()?;
        let n = self.len();
        let mut buf = Vec::with_capacity(24 + n * 10);
        buf.extend_from_slice(MAGIC);
        buf.push(interval_code(self.interval));
        buf.extend_from_slice(&[self.price_scale, self.volume_scale, self.quote_scale]);
        buf.push(u8::from(self.quote_volume.is_some()) | u8::from(self.taker_buy_volume.is_some()) << 1 | u8::from(self.trades.is_some()) << 2);
        buf.extend_from_slice(&(n as u32).to_le_bytes());
        buf.extend_from_slice(&self.open_time.first().copied().unwrap_or(0).to_le_bytes());
        for w in self.open_time.windows(2) {
            put_u(&mut buf, self.steps_between(w[0], w[1])?);
        }
        for i in 0..n {
            put_i(&mut buf, self.open[i] - if i == 0 { 0 } else { self.close[i - 1] });
        }
        for i in 0..n {
            put_i(&mut buf, self.high[i] - self.open[i].max(self.close[i]));
        }
        for i in 0..n {
            put_i(&mut buf, self.open[i].min(self.close[i]) - self.low[i]);
        }
        for i in 0..n {
            put_i(&mut buf, self.close[i] - self.open[i]);
        }
        for col in [Some(&self.volume), self.quote_volume.as_ref(), self.taker_buy_volume.as_ref(), self.trades.as_ref()].into_iter().flatten() {
            for &v in col {
                put_u(&mut buf, v);
            }
        }
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader::new(bytes);
        let t = CodecError::Truncated;
        if r.take(4).ok_or(CodecError::BadMagic)? != MAGIC {
            return Err(CodecError::BadMagic);
        }
        let interval = interval_from(r.u8().ok_or(t.clone())?)?;
        let (price_scale, volume_scale, quote_scale, flags) = (r.u8().ok_or(t.clone())?, r.u8().ok_or(t.clone())?, r.u8().ok_or(t.clone())?, r.u8().ok_or(t.clone())?);
        let n = r.u32_le().ok_or(t.clone())? as usize;
        let first = r.i64_le().ok_or(t.clone())?;
        // Every row costs at least 5 bytes, so a count the file cannot hold is
        // rejected before it becomes a huge allocation.
        if n.saturating_mul(5) > bytes.len() {
            return Err(CodecError::Truncated);
        }

        let mut out = Self { price_scale, volume_scale, quote_scale, ..Self::empty(interval) };
        let mut time = first;
        for i in 0..n {
            if i > 0 {
                let steps = r.u().ok_or(t.clone())? as i64;
                time = match interval.ms() {
                    Some(ms) => time + steps * ms,
                    None => {
                        let c = civil(time);
                        utc(c.year, i64::from(c.month0) + steps, 1)
                    }
                };
            }
            out.open_time.push(time);
        }
        let mut signed = || (0..n).map(|_| r.i().ok_or(CodecError::Truncated)).collect::<Result<Vec<i64>, _>>();
        let (open_deltas, highs, lows, closes) = (signed()?, signed()?, signed()?, signed()?);
        // Undo the deltas in row order: an open depends on the previous close.
        for i in 0..n {
            let open = open_deltas[i] + if i == 0 { 0 } else { out.close[i - 1] };
            let close = open + closes[i];
            out.open.push(open);
            out.close.push(close);
            out.high.push(highs[i] + open.max(close));
            out.low.push(open.min(close) - lows[i]);
        }
        let mut column = |present: bool| -> Result<Option<Vec<u64>>, CodecError> {
            if !present {
                return Ok(None);
            }
            (0..n).map(|_| r.u().ok_or(CodecError::Truncated)).collect::<Result<Vec<_>, _>>().map(Some)
        };
        out.volume = column(true)?.expect("volume is always present");
        out.quote_volume = column(flags & 1 != 0)?;
        out.taker_buy_volume = column(flags & 2 != 0)?;
        out.trades = column(flags & 4 != 0)?;
        if !r.is_empty() {
            return Err(CodecError::TrailingBytes);
        }
        Ok(out)
    }

    pub fn to_candles(&self) -> Vec<Candle> {
        (0..self.len())
            .map(|i| Candle {
                open_time: self.open_time[i],
                open: to_f64(self.open[i], self.price_scale),
                high: to_f64(self.high[i], self.price_scale),
                low: to_f64(self.low[i], self.price_scale),
                close: to_f64(self.close[i], self.price_scale),
                volume: to_f64(self.volume[i] as i64, self.volume_scale),
                quote_volume: self.quote_volume.as_ref().map(|q| to_f64(q[i] as i64, self.quote_scale)),
                taker_buy: self.taker_buy_volume.as_ref().map(|q| to_f64(q[i] as i64, self.volume_scale)),
            })
            .collect()
    }

    fn rescaled(&self, price_scale: u8, volume_scale: u8, quote_scale: u8) -> Result<Self, CodecError> {
        let bad = || CodecError::BadNumber("value overflows when rescaled".to_string());
        let p = |v: &[i64]| v.iter().map(|&x| rescale(x, self.price_scale, price_scale).ok_or_else(bad)).collect::<Result<Vec<_>, _>>();
        let u = |v: &[u64], from: u8, to: u8| v.iter().map(|&x| rescale(x as i64, from, to).map(|y| y as u64).ok_or_else(bad)).collect::<Result<Vec<_>, _>>();
        Ok(Self {
            interval: self.interval,
            price_scale,
            volume_scale,
            quote_scale,
            open_time: self.open_time.clone(),
            open: p(&self.open)?,
            high: p(&self.high)?,
            low: p(&self.low)?,
            close: p(&self.close)?,
            volume: u(&self.volume, self.volume_scale, volume_scale)?,
            quote_volume: self.quote_volume.as_deref().map(|q| u(q, self.quote_scale, quote_scale)).transpose()?,
            taker_buy_volume: self.taker_buy_volume.as_deref().map(|q| u(q, self.volume_scale, volume_scale)).transpose()?,
            trades: self.trades.clone(),
        })
    }

    /// Combine partitions of one interval into one, ordered by time. Where two
    /// carry the same bar the LATER part wins — a fresh fetch replaces a stale one.
    pub fn merge(parts: &[TickCandles]) -> Result<Option<Self>, CodecError> {
        let Some(first) = parts.first() else { return Ok(None) };
        if parts.iter().any(|p| p.interval != first.interval) {
            return Err(CodecError::MixedIntervals);
        }
        let ps = parts.iter().map(|p| p.price_scale).max().unwrap_or(0);
        let vs = parts.iter().map(|p| p.volume_scale).max().unwrap_or(0);
        let qs = parts.iter().map(|p| p.quote_scale).max().unwrap_or(0);
        let scaled: Vec<Self> = parts.iter().map(|p| p.rescaled(ps, vs, qs)).collect::<Result<_, _>>()?;

        let mut rows: Vec<(i64, usize, usize)> = scaled.iter().enumerate().flat_map(|(p, part)| part.open_time.iter().enumerate().map(move |(i, &t)| (t, p, i))).collect();
        // The last part holding a bar wins it. A column stays if every bar that
        // survives has it: an empty part, or one the others overwrite whole, has
        // no say in it.
        rows.sort_by_key(|&(t, p, i)| (t, std::cmp::Reverse((p, i))));
        rows.dedup_by_key(|r| r.0);
        let all = |f: fn(&Self) -> bool| rows.iter().all(|r| f(&scaled[r.1]));
        let (has_quote, has_taker, has_trades) = (all(|p| p.quote_volume.is_some()), all(|p| p.taker_buy_volume.is_some()), all(|p| p.trades.is_some()));
        let mut out = Self { price_scale: ps, volume_scale: vs, quote_scale: qs, ..Self::empty(first.interval) };
        let (mut quote, mut taker, mut trades) = (Vec::new(), Vec::new(), Vec::new());
        for &(t, p, i) in &rows {
            let part = &scaled[p];
            out.open_time.push(t);
            out.open.push(part.open[i]);
            out.high.push(part.high[i]);
            out.low.push(part.low[i]);
            out.close.push(part.close[i]);
            out.volume.push(part.volume[i]);
            if has_quote {
                quote.push(part.quote_volume.as_ref().expect("checked")[i]);
            }
            if has_taker {
                taker.push(part.taker_buy_volume.as_ref().expect("checked")[i]);
            }
            if has_trades {
                trades.push(part.trades.as_ref().expect("checked")[i]);
            }
        }
        out.quote_volume = has_quote.then_some(quote);
        out.taker_buy_volume = has_taker.then_some(taker);
        out.trades = has_trades.then_some(trades);
        Ok(Some(out))
    }

    /// Bars with `from <= open_time < to`.
    pub fn slice(&self, from: i64, to: i64) -> Self {
        let keep: Vec<usize> = (0..self.len()).filter(|&i| self.open_time[i] >= from && self.open_time[i] < to).collect();
        self.select(&keep)
    }

    fn select(&self, idx: &[usize]) -> Self {
        let pick_i = |v: &[i64]| idx.iter().map(|&i| v[i]).collect::<Vec<_>>();
        let pick_u = |v: &[u64]| idx.iter().map(|&i| v[i]).collect::<Vec<_>>();
        Self {
            interval: self.interval,
            price_scale: self.price_scale,
            volume_scale: self.volume_scale,
            quote_scale: self.quote_scale,
            open_time: pick_i(&self.open_time),
            open: pick_i(&self.open),
            high: pick_i(&self.high),
            low: pick_i(&self.low),
            close: pick_i(&self.close),
            volume: pick_u(&self.volume),
            quote_volume: self.quote_volume.as_deref().map(pick_u),
            taker_buy_volume: self.taker_buy_volume.as_deref().map(pick_u),
            trades: self.trades.as_deref().map(pick_u),
        }
    }

    /// Roll bars up into a coarser interval, exactly: extremes, first open, last
    /// close, summed volumes. Every bucket that has any bar is returned; whether
    /// a bucket is complete is the caller's question.
    pub fn aggregate(&self, to: Interval) -> Self {
        let mut out = Self { interval: to, price_scale: self.price_scale, volume_scale: self.volume_scale, quote_scale: self.quote_scale, ..Self::empty(to) };
        let (mut quote, mut taker, mut trades) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..self.len() {
            let bucket = to.align(self.open_time[i]);
            if out.open_time.last() == Some(&bucket) {
                let k = out.len() - 1;
                out.high[k] = out.high[k].max(self.high[i]);
                out.low[k] = out.low[k].min(self.low[i]);
                out.close[k] = self.close[i];
                out.volume[k] += self.volume[i];
                if let Some(q) = &self.quote_volume {
                    quote[k] += q[i];
                }
                if let Some(v) = &self.taker_buy_volume {
                    taker[k] += v[i];
                }
                if let Some(v) = &self.trades {
                    trades[k] += v[i];
                }
            } else {
                out.open_time.push(bucket);
                out.open.push(self.open[i]);
                out.high.push(self.high[i]);
                out.low.push(self.low[i]);
                out.close.push(self.close[i]);
                out.volume.push(self.volume[i]);
                if let Some(q) = &self.quote_volume {
                    quote.push(q[i]);
                }
                if let Some(v) = &self.taker_buy_volume {
                    taker.push(v[i]);
                }
                if let Some(v) = &self.trades {
                    trades.push(v[i]);
                }
            }
        }
        out.quote_volume = self.quote_volume.is_some().then_some(quote);
        out.taker_buy_volume = self.taker_buy_volume.is_some().then_some(taker);
        out.trades = self.trades.is_some().then_some(trades);
        out
    }

    #[cfg(feature = "gzip")]
    pub fn encode_gz(&self) -> Result<Vec<u8>, CodecError> {
        use std::io::Write;
        let raw = self.encode()?;
        let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(raw.len() / 3), flate2::Compression::best());
        gz.write_all(&raw).expect("writing to a Vec cannot fail");
        Ok(gz.finish().expect("writing to a Vec cannot fail"))
    }

    #[cfg(feature = "gzip")]
    pub fn decode_gz(bytes: &[u8]) -> Result<Self, CodecError> {
        Self::decode(&gunzip(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradoor_core::time::{parse_iso, DAY, HOUR, MINUTE};

    fn raw(t: i64, o: &'static str, h: &'static str, l: &'static str, c: &'static str, v: &'static str) -> RawCandle<'static> {
        RawCandle { open_time: t, open: o, high: h, low: l, close: c, volume: v, quote_volume: Some("1000.5"), taker_buy_volume: Some("0.25"), trades: Some(42) }
    }

    fn sample() -> TickCandles {
        let t0 = parse_iso("2026-09-13T00:00Z").unwrap();
        TickCandles::from_raw(Interval::M1, &[
            raw(t0, "64123.40", "64130.00", "64100.10", "64110.00", "12.345"),
            raw(t0 + MINUTE, "64110.00", "64200.00", "64105.00", "64190.50", "3.1"),
            // a gap of two minutes, and a price that falls
            raw(t0 + 4 * MINUTE, "64190.50", "64191.00", "63000.00", "63001.25", "100"),
        ])
        .unwrap()
    }

    #[test]
    fn scales_are_the_finest_precision_seen() {
        let s = sample();
        assert_eq!((s.price_scale, s.volume_scale, s.quote_scale), (2, 3, 1));
        assert_eq!(s.open[0], 6412340);
        assert_eq!(s.volume, vec![12345, 3100, 100000]);
        assert_eq!(s.taker_buy_volume, Some(vec![250, 250, 250]));
    }

    #[test]
    fn encode_decode_round_trips_exactly() {
        let s = sample();
        let bytes = s.encode().unwrap();
        assert_eq!(TickCandles::decode(&bytes).unwrap(), s);
        let candles = s.to_candles();
        assert_eq!(candles[2].low, 63000.0);
        assert_eq!(candles[0].volume, 12.345);
        assert!(TickCandles::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(TickCandles::decode(&extra), Err(CodecError::TrailingBytes));
    }

    #[test]
    fn months_encode_as_calendar_steps() {
        let rows: Vec<String> = (0..14).map(|m| tradoor_core::time::iso_date(utc(2025, m, 1))).collect();
        let raws: Vec<RawCandle> = rows.iter().map(|d| RawCandle { quote_volume: None, taker_buy_volume: None, trades: None, ..raw(parse_iso(d).unwrap(), "1", "2", "0.5", "1.5", "10") }).collect();
        let s = TickCandles::from_raw(Interval::Mo1, &raws).unwrap();
        assert_eq!(TickCandles::decode(&s.encode().unwrap()).unwrap(), s);
        assert_eq!(s.quote_volume, None);
    }

    #[test]
    fn rejects_bad_input() {
        let t0 = parse_iso("2026-09-13").unwrap();
        assert!(matches!(TickCandles::from_raw(Interval::M1, &[raw(t0 + 1, "1", "1", "1", "1", "1")]), Err(CodecError::Misaligned(_))));
        assert!(matches!(TickCandles::from_raw(Interval::M1, &[raw(t0, "1", "1", "1", "1", "1"), raw(t0, "1", "1", "1", "1", "1")]), Err(CodecError::NotAscending(_))));
        assert!(matches!(TickCandles::from_raw(Interval::M1, &[raw(t0, "1e3", "1", "1", "1", "1")]), Err(CodecError::BadNumber(_))));
        assert_eq!(TickCandles::decode(b"nope"), Err(CodecError::BadMagic));
    }

    #[test]
    fn merge_rescales_dedupes_and_later_wins() {
        let t0 = parse_iso("2026-09-13").unwrap();
        let a = TickCandles::from_raw(Interval::H1, &[raw(t0, "1.5", "2", "1", "1.5", "1"), raw(t0 + HOUR, "1.5", "2", "1", "1.5", "1")]).unwrap();
        let b = TickCandles::from_raw(Interval::H1, &[raw(t0 + HOUR, "1.55", "2.25", "1", "2.125", "2"), raw(t0 + 2 * HOUR, "2", "2", "2", "2", "3")]).unwrap();
        let m = TickCandles::merge(&[a, b]).unwrap().unwrap();
        assert_eq!(m.price_scale, 3);
        assert_eq!(m.len(), 3);
        assert_eq!(m.to_candles()[1].close, 2.125);
        // Volume shares its scale with taker volume ("0.25"), so 1 is stored as 100.
        assert_eq!(m.to_candles().iter().map(|c| c.volume).collect::<Vec<_>>(), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_column_survives_a_part_that_has_no_say_in_it() {
        let t0 = parse_iso("2026-09-13").unwrap();
        let full = TickCandles::from_raw(Interval::H1, &[raw(t0, "1", "2", "1", "1", "1"), raw(t0 + HOUR, "1", "2", "1", "1", "1")]).unwrap();
        let bare = |t: i64| RawCandle { quote_volume: None, taker_buy_volume: None, trades: None, ..raw(t, "1", "2", "1", "1", "1") };
        let empty = TickCandles::empty(Interval::H1);

        // Nothing on disk yet, then the exchange's bars: what a recent day is made of.
        let m = TickCandles::merge(&[empty, full.clone()]).unwrap().unwrap();
        assert!(m.taker_buy_volume.is_some() && m.quote_volume.is_some() && m.trades.is_some());
        assert_eq!(m.to_candles()[0].taker_buy, Some(0.25));

        // Older files without it, every bar overwritten by the later part.
        let old = TickCandles::from_raw(Interval::H1, &[bare(t0)]).unwrap();
        let m = TickCandles::merge(&[old, full.clone()]).unwrap().unwrap();
        assert!(m.taker_buy_volume.is_some());

        // A bar that keeps no figure of its own takes the column away, for all of them.
        let later = TickCandles::from_raw(Interval::H1, &[bare(t0 + 2 * HOUR)]).unwrap();
        let m = TickCandles::merge(&[full, later]).unwrap().unwrap();
        assert_eq!(m.len(), 3);
        assert!(m.taker_buy_volume.is_none());
        assert_eq!(m.to_candles()[0].taker_buy, None);
    }

    #[test]
    fn aggregation_is_exact() {
        let t0 = parse_iso("2026-09-13").unwrap();
        let rows: Vec<RawCandle> = (0..120).map(|i| raw(t0 + i * MINUTE, "0.1", "0.3", "0.1", "0.2", "0.1")).collect();
        let hours = TickCandles::from_raw(Interval::M1, &rows).unwrap().aggregate(Interval::H1);
        assert_eq!(hours.len(), 2);
        assert_eq!((hours.volume_scale, hours.volume.clone()), (2, vec![600, 600]));
        assert_eq!(hours.to_candles()[0].volume, 6.0, "0.1 × 60 summed in ticks is exactly 6, not 5.999999999999");
        assert_eq!(hours.trades, Some(vec![42 * 60, 42 * 60]));
        let days = hours.aggregate(Interval::D1);
        assert_eq!((days.len(), days.open_time[0]), (1, t0));
        assert_eq!(hours.slice(t0 + HOUR, t0 + DAY).len(), 1);
    }
}

/// Header facts about an encoded partition, read without decoding its prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub interval: Interval,
    pub rows: u32,
    pub first_open: i64,
    pub last_open: i64,
}

pub fn summarize(bytes: &[u8]) -> Result<Summary, CodecError> {
    let mut r = Reader::new(bytes);
    if r.take(4).ok_or(CodecError::BadMagic)? != MAGIC {
        return Err(CodecError::BadMagic);
    }
    let interval = interval_from(r.u8().ok_or(CodecError::Truncated)?)?;
    r.take(4).ok_or(CodecError::Truncated)?;
    let rows = r.u32_le().ok_or(CodecError::Truncated)?;
    let first_open = r.i64_le().ok_or(CodecError::Truncated)?;
    let mut last_open = first_open;
    for _ in 1..rows {
        let steps = r.u().ok_or(CodecError::Truncated)? as i64;
        last_open = match interval.ms() {
            Some(ms) => last_open + steps * ms,
            None => {
                let c = civil(last_open);
                utc(c.year, i64::from(c.month0) + steps, 1)
            }
        };
    }
    Ok(Summary { interval, rows, first_open, last_open })
}

/// Inflate a `.tdc.gz` file without decoding it.
#[cfg(feature = "gzip")]
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    use std::io::Read;
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut raw).map_err(|_| CodecError::Truncated)?;
    Ok(raw)
}
