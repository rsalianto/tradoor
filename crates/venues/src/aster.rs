//! Aster spot and perpetuals: Binance's API down to the shapes, on its own hosts
//! and with spot under `/api/v1`. `binance::parse_products` reads its lists.

use crate::{Kind, Venue};

pub const SPOT_REST: &str = "https://sapi.asterdex.com";
pub const PERP_REST: &str = "https://fapi.asterdex.com";

pub fn exchange_info_url(venue: Venue) -> String {
    match venue.kind() {
        Kind::Spot => format!("{SPOT_REST}/api/v1/exchangeInfo"),
        Kind::Perp => format!("{PERP_REST}/fapi/v1/exchangeInfo"),
    }
}

pub fn tickers_url(venue: Venue) -> String {
    match venue.kind() {
        Kind::Spot => format!("{SPOT_REST}/api/v1/ticker/24hr"),
        Kind::Perp => format!("{PERP_REST}/fapi/v1/ticker/24hr"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binance::parse_products;

    // Captured 2026-11-05, trimmed.
    #[test]
    fn binances_parser_reads_asters_lists() {
        let info = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"BTC","quoteAsset":"USDT","filters":[{"filterType":"PRICE_FILTER","tickSize":"0.1"}]},
            {"symbol":"SOONUSDT","status":"PENDING_TRADING","contractType":"","baseAsset":"SOON","quoteAsset":"USDT","filters":[]},
            {"symbol":"OLDUSDT","status":"SETTLING","contractType":"PERPETUAL","baseAsset":"OLD","quoteAsset":"USDT","filters":[]}]}"#;
        let tickers = r#"[{"symbol":"BTCUSDT","priceChangePercent":"2.147","lastPrice":"80271.0","quoteVolume":"576038035.56"}]"#;
        let p = parse_products(Venue::AsterPerp, info, Some(tickers)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["aster-perp:BTCUSDT"]);
        assert_eq!((p[0].native.as_deref(), p[0].price_decimals, p[0].last_price, p[0].change_pct), (None, Some(1), Some(80271.0), Some(2.147)), "spelled as this app spells it");
        let spot = r#"{"symbols":[{"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT","filters":[{"filterType":"PRICE_FILTER","tickSize":"0.01"}]}]}"#;
        assert_eq!(parse_products(Venue::AsterSpot, spot, None).unwrap()[0].market, "aster-spot:BTCUSDT");
        assert!(exchange_info_url(Venue::AsterSpot).contains("/api/v1/") && tickers_url(Venue::AsterPerp).contains("/fapi/v1/"));
    }
}
