//! TPO (letter chart) sessions.
//!
//! Unlike `session_profile`, rows are a fixed `row_size` anchored to a SHARED
//! `grid_low`, so a row index is the same price band in every session — a TPO
//! chart is read across sessions, and rows that shift under each profile make
//! that impossible. The value area is computed from the letter counts so it
//! agrees with what is on screen.

use std::collections::BTreeMap;

use super::profile::value_area;
use crate::{Candle, ChartTimeframe};

const LETTERS: &[u8; 52] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// A–Z then a–z, wrapping past 52 rather than failing.
pub fn tpo_letter(i: usize) -> char {
    LETTERS[i % LETTERS.len()] as char
}

#[derive(Clone, Debug)]
pub struct TpoRow {
    pub index: i64,
    pub price: f64,
    /// Indices into the session's periods, chronological.
    pub periods: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct TpoSession {
    pub session_start: i64,
    pub session_end: i64,
    /// Open time of each sub-period; its letter is `tpo_letter(i)`.
    pub periods: Vec<i64>,
    /// Occupied rows only, ascending. A gap here IS the unfilled gap.
    pub rows: Vec<TpoRow>,
    pub row_size: f64,
    pub high: f64,
    pub low: f64,
    pub poc: f64,
    pub vah: f64,
    pub val: f64,
    pub single_prints: Vec<i64>,
    pub initial_balance: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug)]
pub struct TpoOptions {
    pub row_size: f64,
    pub grid_low: f64,
    pub value_area_pct: f64,
    pub initial_balance_periods: usize,
    pub min_periods: usize,
}

impl TpoOptions {
    pub fn new(row_size: f64, grid_low: f64) -> Self {
        Self { row_size, grid_low, value_area_pct: 0.7, initial_balance_periods: 2, min_periods: 4 }
    }
}

/// Row height from the data's own range, aiming for `target_rows` rows.
pub fn suggest_row_size(candles: &[Candle], target_rows: usize) -> f64 {
    let high = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    if !high.is_finite() || !low.is_finite() || high <= low {
        return 0.0;
    }
    (high - low) / target_rows.max(1) as f64
}

fn row_index(price: f64, grid_low: f64, row_size: f64) -> i64 {
    ((price - grid_low) / row_size).floor() as i64
}

pub fn tpo_session(period_candles: &[Candle], session_start: i64, session_end: i64, opts: &TpoOptions) -> Option<TpoSession> {
    let TpoOptions { row_size, grid_low, .. } = *opts;
    if period_candles.is_empty() || !(row_size > 0.0) {
        return None;
    }
    let high = period_candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let low = period_candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    if !high.is_finite() || !low.is_finite() {
        return None;
    }

    let mut by_row: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (p, c) in period_candles.iter().enumerate() {
        for r in row_index(c.low, grid_low, row_size)..=row_index(c.high, grid_low, row_size) {
            by_row.entry(r).or_default().push(p);
        }
    }
    let rows: Vec<TpoRow> = by_row
        .into_iter()
        .map(|(index, periods)| TpoRow { index, price: grid_low + index as f64 * row_size, periods })
        .collect();
    let (first, last) = (rows.first()?.index, rows.last()?.index);

    // The expansion needs contiguous counts; holes are zeros so a gap stops the
    // area growing instead of being skipped over.
    let mut counts = vec![0.0; (last - first + 1) as usize];
    for row in &rows {
        counts[(row.index - first) as usize] = row.periods.len() as f64;
    }
    // The most-traded row; of several, the one nearest the range's middle.
    let most = counts.iter().copied().fold(0.0, f64::max);
    let middle = (counts.len() - 1) as f64 / 2.0;
    let poc_offset = (0..counts.len()).filter(|i| counts[*i] == most).min_by(|a, b| (*a as f64 - middle).abs().total_cmp(&(*b as f64 - middle).abs()).then(b.cmp(a))).unwrap_or(0);
    let (low_index, high_index) = value_area(&counts, poc_offset, opts.value_area_pct);

    let initial_balance = (opts.initial_balance_periods > 0 && period_candles.len() >= opts.initial_balance_periods).then(|| {
        let ib = &period_candles[..opts.initial_balance_periods];
        (ib.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max), ib.iter().map(|c| c.low).fold(f64::INFINITY, f64::min))
    });

    Some(TpoSession {
        session_start,
        session_end,
        periods: period_candles.iter().map(|c| c.open_time).collect(),
        single_prints: single_prints(&rows),
        rows,
        row_size,
        high,
        low,
        poc: grid_low + (first as f64 + poc_offset as f64 + 0.5) * row_size,
        vah: grid_low + (first as f64 + high_index as f64 + 1.0) * row_size,
        val: grid_low + (first as f64 + low_index as f64) * row_size,
        initial_balance,
    })
}

/// Rows touched once and never revisited, read inward from each extreme.
fn single_prints(rows: &[TpoRow]) -> Vec<i64> {
    let mut out: Vec<i64> = rows.iter().rev().take_while(|r| r.periods.len() == 1).map(|r| r.index).collect();
    for r in rows.iter().take_while(|r| r.periods.len() == 1) {
        if !out.contains(&r.index) {
            out.push(r.index);
        }
    }
    out.sort_unstable();
    out
}

pub fn tpo_by_session(candles: &[Candle], session: ChartTimeframe, opts: &TpoOptions) -> Vec<TpoSession> {
    let mut groups: BTreeMap<i64, Vec<Candle>> = BTreeMap::new();
    for c in candles {
        groups.entry(session.align(c.open_time)).or_default().push(*c);
    }
    groups
        .into_iter()
        .filter(|(_, g)| g.len() >= opts.min_periods)
        .filter_map(|(start, g)| tpo_session(&g, start, session.end(start), opts))
        .collect()
}
