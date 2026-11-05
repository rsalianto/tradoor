//! Coinbase Exchange's spot markets, named `BTC-USD`. Spot alone: its public
//! API lists no perpetual a chart could read.

use serde_json::Value;

use crate::catalog::{change, num, product, tick_decimals, unique, Product};
use crate::{shape, Venue, VenueError};

pub const REST: &str = "https://api.exchange.coinbase.com";
/// Quotes a pair is listed under here.
const QUOTES: [&str; 4] = ["USD", "USDT", "USDC", "EUR"];

pub fn products_url() -> String {
    format!("{REST}/products")
}

pub fn stats_url() -> String {
    format!("{REST}/products/stats")
}

/// Pairs that are online and trading. A day's volume is in coins.
pub fn parse_spot(products: &str, stats: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let stats: Value = match stats {
        Some(s) => serde_json::from_str(s)?,
        None => Value::Null,
    };
    let products: Value = serde_json::from_str(products)?;
    let list = products
        .as_array()
        .ok_or_else(|| shape("coinbase products"))?
        .iter()
        .filter(|p| p["status"] == "online" && p["trading_disabled"] != true)
        .filter_map(|p| {
            let (id, base, quote) = (p["id"].as_str()?, p["base_currency"].as_str()?, p["quote_currency"].as_str()?);
            if !QUOTES.contains(&quote) {
                return None;
            }
            let day = &stats[id]["stats_24hour"];
            let last = num(day.get("last"));
            let turnover = num(day.get("volume")).zip(last).map(|(v, l)| v * l);
            product(Venue::CoinbaseSpot, base, quote, id, tick_decimals(p["quote_increment"].as_str()), turnover, last, change(num(day.get("open")), last))
        })
        .collect();
    Ok(unique(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured 2026-11-05, trimmed.
    #[test]
    fn pairs_that_trade_with_the_days_stats() {
        let products = r#"[
            {"id":"BTC-USD","base_currency":"BTC","quote_currency":"USD","quote_increment":"0.01","status":"online","trading_disabled":false},
            {"id":"MASK-GBP","base_currency":"MASK","quote_currency":"GBP","quote_increment":"0.001","status":"online","trading_disabled":false},
            {"id":"OLD-USD","base_currency":"OLD","quote_currency":"USD","quote_increment":"0.01","status":"delisted","trading_disabled":true}]"#;
        let stats = r#"{"BTC-USD":{"stats_30day":{"volume":"168444.18"},"stats_24hour":{"open":"78490.26","high":"81130.79","low":"77897.7","last":"80178.31","volume":"10041.29"}}}"#;
        let p = parse_spot(products, Some(stats)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["coinbase-spot:BTCUSD"]);
        assert_eq!((p[0].native.as_deref(), p[0].price_decimals, p[0].last_price), (Some("BTC-USD"), Some(2), Some(80178.31)));
        assert_eq!(p[0].quote_volume_24h, Some(10041.29 * 80178.31));
        assert!((p[0].change_pct.unwrap() - 2.1506).abs() < 0.001);
        assert_eq!(parse_spot(products, None).unwrap()[0].last_price, None, "the list stands without its stats");
    }
}
