//! tradoor-venues — what each exchange's public API looks like.
//!
//! URLs and parsers only. The server binaries do the fetching, and keeping the
//! shapes here means every parser is tested against a real response without a
//! network.
#![forbid(unsafe_code)]

pub mod aster;
pub mod binance;
pub mod bitfinex;
pub mod bitget;
pub mod bitstamp;
pub mod bybit;
pub mod catalog;
pub mod coinbase;
pub mod deribit;
pub mod gate;
pub mod htx;
pub mod hyperliquid;
pub mod kline;
pub mod kraken;
pub mod lighter;
pub mod market;
pub mod okx;

pub use catalog::Product;
pub use kline::OwnedKline;
pub use market::{Exchange, Kind, MarketRef, Venue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenueError {
    Json(String),
    /// The response parsed but is not the shape this parser knows.
    Shape(String),
    /// The exchange answered with its own error.
    Api { code: i64, message: String },
    Unsupported(String),
}

impl std::fmt::Display for VenueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
            Self::Shape(e) => write!(f, "unexpected response shape: {e}"),
            Self::Api { code, message } => write!(f, "exchange error {code}: {message}"),
            Self::Unsupported(e) => write!(f, "unsupported: {e}"),
        }
    }
}

impl std::error::Error for VenueError {}

impl From<serde_json::Error> for VenueError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

pub(crate) fn shape(what: impl Into<String>) -> VenueError {
    VenueError::Shape(what.into())
}
