//! Bitfinex spot, named `tBTCUST` or `tAAVE:UST`, and perpetuals, `tBTCF0:USTF0`.
//! `UST` is its name for USDT, and it has a few short names of its own for coins.

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::{num, product, unique, Product};
use crate::{shape, Kind, Venue, VenueError};

pub const REST: &str = "https://api-pub.bitfinex.com/v2";

pub fn pairs_url(venue: Venue) -> String {
    format!("{REST}/conf/pub:list:pair:{}", if venue.kind() == Kind::Perp { "futures" } else { "exchange" })
}

pub fn tickers_url() -> String {
    format!("{REST}/tickers?symbols=ALL")
}

/// The coin's usual name, where Bitfinex's differs (its `pub:map:currency:sym`).
fn coin(name: &str) -> &str {
    match name {
        "ALG" => "ALGO",
        "ATO" => "ATOM",
        "DSH" => "DASH",
        "IOT" => "IOTA",
        "OPX" => "OP",
        "UDC" => "USDC",
        "UST" => "USDT",
        "WBT" => "WBTC",
        other => other,
    }
}

/// A pair's two sides: `AAVE:UST`, or six letters run together, `BTCUST`.
fn sides(pair: &str) -> Option<(&str, &str)> {
    pair.split_once(':').or_else(|| (pair.len() == 6).then(|| pair.split_at(3)))
}

/// Pairs in the dollar and USDT, and USDT perpetuals, with the day's stats from
/// the ticker table: `[symbol, bid, bidSize, ask, askSize, change, changeRelative,
/// last, volume, high, low]`, the volume in coins. Bitfinex publishes no tick:
/// it quotes five significant figures.
pub fn parse_products(venue: Venue, pairs: &str, tickers: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let stats: HashMap<String, Value> = match tickers {
        Some(t) => serde_json::from_str::<Value>(t)?.as_array().map(|rows| rows.iter().filter_map(|r| Some((r[0].as_str()?.to_string(), r.clone()))).collect()).unwrap_or_default(),
        None => HashMap::new(),
    };
    let pairs: Value = serde_json::from_str(pairs)?;
    let list = pairs[0]
        .as_array()
        .ok_or_else(|| shape("bitfinex pairs"))?
        .iter()
        .filter_map(|p| {
            let pair = p.as_str()?;
            if pair.contains("TEST") {
                return None;
            }
            let (base, quote) = sides(pair)?;
            // A perpetual's sides end in F0: BTCF0:USTF0.
            let (base, quote) = if venue.kind() == Kind::Perp { (base.strip_suffix("F0")?, quote.strip_suffix("F0")?) } else { (base, quote) };
            let quote = coin(quote);
            if !matches!(quote, "USD" | "USDT") {
                return None;
            }
            let native = format!("t{pair}");
            let t = stats.get(&native);
            let last = t.and_then(|t| num(t.get(7)));
            let turnover = t.and_then(|t| num(t.get(8))).zip(last).map(|(v, l)| v * l);
            product(venue, coin(base), quote, &native, None, turnover, last, t.and_then(|t| num(t.get(6))).map(|c| c * 100.0))
        })
        .collect();
    Ok(unique(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured 2026-11-05, trimmed.
    const TICKERS: &str = r#"[["tBTCUST",80260,1.79,80272,3.48,1604,0.0203926,80260,1385.15,81522,78000],["tBTCF0:USTF0",80336,8.72,80350,5.9,1656,0.02104594,80341,648.86,81263,78054],["fUSD",0.0002,0.0002,120,19320535,0.0001,2,33853,0.00006,0.33,0.0002]]"#;

    #[test]
    fn spot_reads_both_spellings_and_usual_names() {
        let pairs = r#"[["AAVE:UST","ALGUSD","BTCUST","BTCEUR","ETHBTC","TESTBTC:TESTUSD","DOGE:BTC"]]"#;
        let p = parse_products(Venue::BitfinexSpot, pairs, Some(TICKERS)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["bitfinex-spot:AAVEUSDT", "bitfinex-spot:ALGOUSD", "bitfinex-spot:BTCUSDT"]);
        assert_eq!((p[2].base.as_str(), p[2].quote.as_str(), p[2].native.as_deref(), p[2].last_price), ("BTC", "USDT", Some("tBTCUST"), Some(80260.0)));
        assert_eq!(p[0].native.as_deref(), Some("tAAVE:UST"));
        assert_eq!(p[2].quote_volume_24h, Some(1385.15 * 80260.0));
        assert!((p[2].change_pct.unwrap() - 2.03926).abs() < 1e-9);
    }

    #[test]
    fn perpetuals_lose_their_suffix_and_keep_usdt() {
        let pairs = r#"[["BTCF0:USTF0","ETHF0:BTCF0","ALGF0:USTF0","TESTBTCF0:TESTUSDTF0"]]"#;
        let p = parse_products(Venue::BitfinexPerp, pairs, Some(TICKERS)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["bitfinex-perp:BTCUSDT", "bitfinex-perp:ALGOUSDT"]);
        assert_eq!((p[0].native.as_deref(), p[0].last_price), (Some("tBTCF0:USTF0"), Some(80341.0)));
        assert!(pairs_url(Venue::BitfinexPerp).ends_with("futures") && pairs_url(Venue::BitfinexSpot).ends_with("exchange"));
    }
}
