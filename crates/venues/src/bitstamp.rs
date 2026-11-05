//! Bitstamp's spot markets and perpetuals, named `btcusd` and `btcusd-perp`. One
//! list and one set of tickers serve both.

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::{change, num, product, unique, Product};
use crate::{shape, Venue, VenueError};

pub const REST: &str = "https://www.bitstamp.net/api/v2";
/// Quotes a spot pair is listed under here. Every perpetual is in USD.
const QUOTES: [&str; 4] = ["USD", "USDT", "USDC", "EUR"];

pub fn markets_url() -> String {
    format!("{REST}/markets/")
}

pub fn tickers_url() -> String {
    format!("{REST}/ticker/")
}

/// Markets of `venue` that are trading. A day's volume is in coins, and its
/// change runs from `open_24`, the price a day ago: `open` is today's, from 00:00 UTC.
pub fn parse_products(venue: Venue, markets: &str, tickers: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let wanted = if venue == Venue::BitstampPerp { "PERPETUAL" } else { "SPOT" };
    let tickers: Value = match tickers {
        Some(s) => serde_json::from_str(s)?,
        None => Value::Null,
    };
    let tickers: HashMap<&str, &Value> = tickers.as_array().into_iter().flatten().filter_map(|t| Some((t["market"].as_str()?, t))).collect();
    let markets: Value = serde_json::from_str(markets)?;
    let list = markets
        .as_array()
        .ok_or_else(|| shape("bitstamp markets"))?
        .iter()
        .filter(|m| m["market_type"] == wanted && m["trading"] == "Enabled")
        .filter_map(|m| {
            let (native, base, quote) = (m["market_symbol"].as_str()?, m["base_currency"].as_str()?, m["counter_currency"].as_str()?);
            if !QUOTES.contains(&quote) {
                return None;
            }
            let day = m["name"].as_str().and_then(|name| tickers.get(name)).copied();
            let last = day.and_then(|t| num(t.get("last")));
            let turnover = day.and_then(|t| num(t.get("volume"))).zip(last).map(|(v, l)| v * l);
            let decimals = m["counter_decimals"].as_u64().and_then(|d| u8::try_from(d).ok());
            product(venue, base, quote, native, decimals, turnover, last, change(day.and_then(|t| num(t.get("open_24"))), last))
        })
        .collect();
    Ok(unique(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured 2026-11-05, trimmed.
    const MARKETS: &str = r#"[
        {"name":"BTC/USD","market_symbol":"btcusd","base_currency":"BTC","counter_currency":"USD","counter_decimals":2,"trading":"Enabled","market_type":"SPOT"},
        {"name":"ZK/EUR","market_symbol":"zkeur","base_currency":"ZK","counter_currency":"EUR","counter_decimals":5,"trading":"Disabled","market_type":"SPOT"},
        {"name":"ETH/BTC","market_symbol":"ethbtc","base_currency":"ETH","counter_currency":"BTC","counter_decimals":8,"trading":"Enabled","market_type":"SPOT"},
        {"name":"BTC/USD-PERP","market_symbol":"btcusd-perp","base_currency":"BTC","counter_currency":"USD","counter_decimals":0,"trading":"Enabled","market_type":"PERPETUAL","has_market_hours":false},
        {"name":"GOLD/USD-PERP","market_symbol":"goldusd-perp","base_currency":"GOLD","counter_currency":"USD","counter_decimals":1,"trading":"Enabled","market_type":"PERPETUAL","has_market_hours":true}]"#;
    const TICKERS: &str = r#"[
        {"open":"80423.32","last":"78555.00","volume":"2056.46582981","open_24":"79117.98","percent_change_24":"-0.71","market_type":"SPOT","market":"BTC/USD"},
        {"open":"80478","last":"78598","volume":"562.84433","open_24":"79152","percent_change_24":"-0.70","market_type":"PERPETUAL","market":"BTC/USD-PERP"}]"#;

    #[test]
    fn spot_and_perpetuals_come_apart_from_one_list() {
        let spot = parse_products(Venue::BitstampSpot, MARKETS, Some(TICKERS)).unwrap();
        assert_eq!(spot.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["bitstamp-spot:BTCUSD"], "not the disabled pair, nor one quoted in bitcoin");
        assert_eq!((spot[0].native.as_deref(), spot[0].price_decimals, spot[0].last_price), (Some("btcusd"), Some(2), Some(78555.0)));
        assert_eq!(spot[0].quote_volume_24h, Some(2056.46582981 * 78555.0));
        let perps = parse_products(Venue::BitstampPerp, MARKETS, Some(TICKERS)).unwrap();
        assert_eq!(perps.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["bitstamp-perp:BTCUSD", "bitstamp-perp:GOLDUSD"]);
        assert_eq!((perps[0].native.as_deref(), perps[0].kind), (Some("btcusd-perp"), "perp"));
    }

    #[test]
    fn change_runs_from_a_day_ago_not_from_midnight() {
        let spot = parse_products(Venue::BitstampSpot, MARKETS, Some(TICKERS)).unwrap();
        assert!((spot[0].change_pct.unwrap() + 0.7116).abs() < 0.001);
    }

    #[test]
    fn the_list_stands_without_its_tickers() {
        let perps = parse_products(Venue::BitstampPerp, MARKETS, None).unwrap();
        assert_eq!((perps.len(), perps[0].last_price, perps[1].price_decimals), (2, None, Some(1)));
    }
}
