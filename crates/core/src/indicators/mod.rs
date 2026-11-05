//! Technical indicators with Pine Script semantics.
//!
//! Hand-written rather than taken from a TA library because the numbers MUST be
//! identical to TradingView's: many libraries use an EMA where Pine uses Wilder's
//! RMA, sample stdev where Pine uses population, or seed an EMA from the first
//! value instead of an SMA — and a chart that disagrees with the one being read
//! by hand cannot be trusted.
//!
//! Conventions: every output is as long as its input and index-aligned with it;
//! `None` marks warm-up; one `None` inside a window makes that bar `None` too.

pub mod cross;
pub mod momentum;
pub mod moon;
pub mod pivots;
pub mod profile;
pub mod series;
pub mod tpo;
pub mod volatility;

pub type Series = Vec<Option<f64>>;

pub use cross::{cross_over, cross_under};
pub use momentum::{macd, rsi, stoch_rsi, Macd, StochRsi};
pub use pivots::{pivot_highs, pivot_lows, Pivot};
pub use series::{change, combine, ema, exp_smooth, highest, lowest, rma, sma, stdev};
pub use volatility::{atr, bollinger, true_range, Bollinger};
