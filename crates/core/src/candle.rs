//! One bar of trading.

/// `open_time` is epoch milliseconds UTC, aligned to the boundary of the
/// interval the candle belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Candle {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Turnover in the quote asset, when the exchange reports it.
    pub quote_volume: Option<f64>,
    /// What market buyers took, in the base asset, when the exchange reports it.
    pub taker_buy: Option<f64>,
}
