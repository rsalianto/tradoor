//! Deribit's perpetuals: the inverse `BTC-PERPETUAL` and `ETH-PERPETUAL`, quoted
//! in the dollar, and the linear `SOL_USDC-PERPETUAL` and the like. Each has its
//! currency's list of stats, and only the three currencies below have any.

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::{decimals_of, num, product, unique, Product};
use crate::{shape, Venue, VenueError};

pub const REST: &str = "https://www.deribit.com/api/v2/public";

pub fn instruments_url() -> String {
    format!("{REST}/get_instruments?currency=any&kind=future")
}

/// `any` is refused here: BTC, ETH and USDC settle every perpetual.
pub fn summary_url(currency: &str) -> String {
    format!("{REST}/get_book_summary_by_currency?currency={currency}&kind=future")
}

/// Active perpetuals (a dated future expires, and its chart with it). `price_change`
/// is already a percent.
pub fn parse_perps(instruments: &str, summaries: &[Option<String>]) -> Result<Vec<Product>, VenueError> {
    let mut stats: HashMap<String, Value> = HashMap::new();
    for body in summaries.iter().flatten() {
        let v: Value = serde_json::from_str(body)?;
        for row in v["result"].as_array().into_iter().flatten() {
            if let Some(name) = row["instrument_name"].as_str() {
                stats.insert(name.to_string(), row.clone());
            }
        }
    }
    let v: Value = serde_json::from_str(instruments)?;
    let list = v["result"]
        .as_array()
        .ok_or_else(|| shape("deribit instruments"))?
        .iter()
        .filter(|i| i["settlement_period"] == "perpetual" && i["is_active"] == true)
        .filter_map(|i| {
            let name = i["instrument_name"].as_str()?;
            let t = stats.get(name);
            product(Venue::DeribitPerp, i["base_currency"].as_str()?, i["quote_currency"].as_str()?, name, num(i.get("tick_size")).and_then(decimals_of), t.and_then(|t| num(t.get("volume_usd"))), t.and_then(|t| num(t.get("last"))), t.and_then(|t| num(t.get("price_change"))))
        })
        .collect();
    Ok(unique(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured 2026-11-05, trimmed.
    #[test]
    fn perpetuals_only_with_stats_from_their_currency() {
        let instruments = r#"{"result":[
            {"instrument_name":"BTC-PERPETUAL","base_currency":"BTC","quote_currency":"USD","tick_size":0.5,"is_active":true,"settlement_period":"perpetual"},
            {"instrument_name":"SOL_USDC-PERPETUAL","base_currency":"SOL","quote_currency":"USDC","tick_size":0.001,"is_active":true,"settlement_period":"perpetual"},
            {"instrument_name":"BTC-29JAN27","base_currency":"BTC","quote_currency":"USD","tick_size":2.5,"is_active":true,"settlement_period":"month"},
            {"instrument_name":"OLD_USDC-PERPETUAL","base_currency":"OLD","quote_currency":"USDC","tick_size":0.1,"is_active":false,"settlement_period":"perpetual"}]}"#;
        let btc = r#"{"result":[{"instrument_name":"BTC-PERPETUAL","last":80278.5,"price_change":2.2604,"volume_usd":2242161710}]}"#.to_string();
        let usdc = r#"{"result":[{"instrument_name":"SOL_USDC-PERPETUAL","last":96.683,"price_change":3.4118,"volume_usd":688685020.82}]}"#.to_string();
        let p = parse_perps(instruments, &[Some(btc), None, Some(usdc)]).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["deribit-perp:BTCUSD", "deribit-perp:SOLUSDC"]);
        assert_eq!((p[0].native.as_deref(), p[0].price_decimals, p[0].change_pct), (Some("BTC-PERPETUAL"), Some(1), Some(2.2604)));
        assert_eq!((p[1].native.as_deref(), p[1].price_decimals, p[1].quote_volume_24h), (Some("SOL_USDC-PERPETUAL"), Some(3), Some(688685020.82)));
    }
}
