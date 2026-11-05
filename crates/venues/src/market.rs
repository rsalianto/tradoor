//! Venues and market references.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Exchange {
    Binance,
    Bybit,
    Okx,
    Bitget,
    Hyperliquid,
    Kraken,
    Lighter,
    Gate,
    Aster,
    Htx,
    Bitfinex,
    Deribit,
    Coinbase,
    Bitstamp,
    /// Series this server works out itself — the crypto market's total
    /// capitalisation and its parts, and indices such as the VIX — rather
    /// than markets anyone trades.
    Index,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Spot,
    Perp,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Perp => "perp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Venue {
    BinanceSpot,
    BinancePerp,
    BybitSpot,
    BybitPerp,
    OkxSpot,
    OkxPerp,
    BitgetSpot,
    BitgetPerp,
    HyperliquidPerp,
    KrakenSpot,
    KrakenPerp,
    LighterPerp,
    GateSpot,
    GatePerp,
    AsterSpot,
    AsterPerp,
    HtxSpot,
    HtxPerp,
    BitfinexSpot,
    BitfinexPerp,
    DeribitPerp,
    CoinbaseSpot,
    BitstampSpot,
    BitstampPerp,
    Index,
}

impl Venue {
    pub const ALL: [Venue; 25] = [
        Self::BinanceSpot,
        Self::BinancePerp,
        Self::BybitSpot,
        Self::BybitPerp,
        Self::OkxSpot,
        Self::OkxPerp,
        Self::BitgetSpot,
        Self::BitgetPerp,
        Self::HyperliquidPerp,
        Self::KrakenSpot,
        Self::KrakenPerp,
        Self::LighterPerp,
        Self::GateSpot,
        Self::GatePerp,
        Self::AsterSpot,
        Self::AsterPerp,
        Self::HtxSpot,
        Self::HtxPerp,
        Self::BitfinexSpot,
        Self::BitfinexPerp,
        Self::DeribitPerp,
        Self::CoinbaseSpot,
        Self::BitstampSpot,
        Self::BitstampPerp,
        Self::Index,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::BinanceSpot => "binance-spot",
            Self::BinancePerp => "binance-perp",
            Self::BybitSpot => "bybit-spot",
            Self::BybitPerp => "bybit-perp",
            Self::OkxSpot => "okx-spot",
            Self::OkxPerp => "okx-perp",
            Self::BitgetSpot => "bitget-spot",
            Self::BitgetPerp => "bitget-perp",
            Self::HyperliquidPerp => "hyperliquid-perp",
            Self::KrakenSpot => "kraken-spot",
            Self::KrakenPerp => "kraken-perp",
            Self::LighterPerp => "lighter-perp",
            Self::GateSpot => "gate-spot",
            Self::GatePerp => "gate-perp",
            Self::AsterSpot => "aster-spot",
            Self::AsterPerp => "aster-perp",
            Self::HtxSpot => "htx-spot",
            Self::HtxPerp => "htx-perp",
            Self::BitfinexSpot => "bitfinex-spot",
            Self::BitfinexPerp => "bitfinex-perp",
            Self::DeribitPerp => "deribit-perp",
            Self::CoinbaseSpot => "coinbase-spot",
            Self::BitstampSpot => "bitstamp-spot",
            Self::BitstampPerp => "bitstamp-perp",
            Self::Index => "index",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BinanceSpot => "Binance Spot",
            Self::BinancePerp => "Binance Perp",
            Self::BybitSpot => "Bybit Spot",
            Self::BybitPerp => "Bybit Perp",
            Self::OkxSpot => "OKX Spot",
            Self::OkxPerp => "OKX Perp",
            Self::BitgetSpot => "Bitget Spot",
            Self::BitgetPerp => "Bitget Perp",
            Self::HyperliquidPerp => "Hyperliquid Perp",
            Self::KrakenSpot => "Kraken Spot",
            Self::KrakenPerp => "Kraken Perp",
            Self::LighterPerp => "Lighter Perp",
            Self::GateSpot => "Gate Spot",
            Self::GatePerp => "Gate Perp",
            Self::AsterSpot => "Aster Spot",
            Self::AsterPerp => "Aster Perp",
            Self::HtxSpot => "HTX Spot",
            Self::HtxPerp => "HTX Perp",
            Self::BitfinexSpot => "Bitfinex Spot",
            Self::BitfinexPerp => "Bitfinex Perp",
            Self::DeribitPerp => "Deribit Perp",
            Self::CoinbaseSpot => "Coinbase Spot",
            Self::BitstampSpot => "Bitstamp Spot",
            Self::BitstampPerp => "Bitstamp Perp",
            Self::Index => "Index",
        }
    }

    pub fn exchange(self) -> Exchange {
        match self {
            Self::BinanceSpot | Self::BinancePerp => Exchange::Binance,
            Self::BybitSpot | Self::BybitPerp => Exchange::Bybit,
            Self::OkxSpot | Self::OkxPerp => Exchange::Okx,
            Self::BitgetSpot | Self::BitgetPerp => Exchange::Bitget,
            Self::HyperliquidPerp => Exchange::Hyperliquid,
            Self::KrakenSpot | Self::KrakenPerp => Exchange::Kraken,
            Self::LighterPerp => Exchange::Lighter,
            Self::GateSpot | Self::GatePerp => Exchange::Gate,
            Self::AsterSpot | Self::AsterPerp => Exchange::Aster,
            Self::HtxSpot | Self::HtxPerp => Exchange::Htx,
            Self::BitfinexSpot | Self::BitfinexPerp => Exchange::Bitfinex,
            Self::DeribitPerp => Exchange::Deribit,
            Self::CoinbaseSpot => Exchange::Coinbase,
            Self::BitstampSpot | Self::BitstampPerp => Exchange::Bitstamp,
            Self::Index => Exchange::Index,
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            // An index is no perpetual: nothing is funded or liquidated on it.
            Self::BinanceSpot | Self::BybitSpot | Self::OkxSpot | Self::BitgetSpot | Self::KrakenSpot | Self::GateSpot | Self::AsterSpot | Self::HtxSpot | Self::BitfinexSpot | Self::CoinbaseSpot | Self::BitstampSpot | Self::Index => Kind::Spot,
            _ => Kind::Perp,
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id() == id)
    }
}

/// `binance-perp:BTCUSDT`. The venue is part of a market's identity: BTCUSDT
/// exists on many venues at many prices. The symbol is this app's spelling —
/// `[A-Z0-9_]`, the coin and the quote run together where the exchange writes
/// `BTC-USDT-SWAP` or `BTC/USD` — and the catalogue carries the exchange's own
/// name for it where the two differ.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MarketRef {
    pub venue: Venue,
    pub symbol: String,
}

fn valid_symbol(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

impl MarketRef {
    pub fn new(venue: Venue, symbol: &str) -> Option<Self> {
        valid_symbol(symbol).then(|| Self { venue, symbol: symbol.to_string() })
    }

    pub fn parse(s: &str) -> Option<Self> {
        let (venue, symbol) = s.split_once(':')?;
        Self::new(Venue::parse(venue)?, symbol)
    }

    /// Where the market's files live under the data directory. Safe to join onto
    /// a path: the symbol is validated to `[A-Z0-9_]`.
    pub fn dir(&self) -> String {
        format!("{}/{}", self.venue.id(), self.symbol)
    }
}

impl fmt::Display for MarketRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.venue.id(), self.symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_parse_and_refuse_paths() {
        let m = MarketRef::parse("binance-perp:BTCUSDT").unwrap();
        assert_eq!((m.venue, m.symbol.as_str(), m.to_string(), m.dir()), (Venue::BinancePerp, "BTCUSDT", "binance-perp:BTCUSDT".to_string(), "binance-perp/BTCUSDT".to_string()));
        assert!(MarketRef::parse("binance-perp:../etc").is_none());
        assert!(MarketRef::parse("binance-perp:btcusdt").is_none());
        assert_eq!(MarketRef::parse("kraken-spot:BTCUSD").unwrap().venue, Venue::KrakenSpot);
        assert_eq!(MarketRef::parse("index:TOTAL2").map(|m| m.dir()), Some("index/TOTAL2".to_string()));
        assert!(MarketRef::parse("kucoin-spot:BTCUSDT").is_none());
        assert_eq!(MarketRef::parse("gate-perp:BTCUSDT").map(|m| (m.venue.exchange(), m.venue.kind())), Some((Exchange::Gate, Kind::Perp)));
        assert_eq!(MarketRef::parse("coinbase-spot:BTCUSD").map(|m| m.venue.kind()), Some(Kind::Spot));
        for v in Venue::ALL {
            assert_eq!(Venue::parse(v.id()), Some(v));
        }
        assert!(MarketRef::parse("BTCUSDT").is_none());
    }
}
