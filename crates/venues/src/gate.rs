//! Gate spot and USDT perpetuals. Both name a market `BTC_USDT`; here it is
//! `BTCUSDT`.

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::{num, product, tick_decimals, unique, Product};
use crate::{shape, Venue, VenueError};

pub const REST: &str = "https://api.gateio.ws/api/v4";
/// Quotes a spot pair is listed under here.
const SPOT_QUOTES: [&str; 3] = ["USDT", "USDC", "USD"];
/// Gate lists two thousand spot pairs, half of them barely traded: dollars a
/// day below which a pair stays out of a search everyone downloads.
const SPOT_MIN_TURNOVER: f64 = 10_000.0;

pub fn spot_pairs_url() -> String {
    format!("{REST}/spot/currency_pairs")
}

pub fn spot_tickers_url() -> String {
    format!("{REST}/spot/tickers")
}

pub fn contracts_url() -> String {
    format!("{REST}/futures/usdt/contracts")
}

pub fn perp_tickers_url() -> String {
    format!("{REST}/futures/usdt/tickers")
}

fn by_name(body: Option<&str>, key: &str) -> Result<HashMap<String, Value>, VenueError> {
    let Some(body) = body else { return Ok(HashMap::new()) };
    let rows: Value = serde_json::from_str(body)?;
    Ok(rows.as_array().map(|rows| rows.iter().filter_map(|r| Some((r[key].as_str()?.to_string(), r.clone()))).collect()).unwrap_or_default())
}

/// Tradable spot pairs in the dollar and its stablecoins, with turnover enough
/// to be worth listing. Without the ticker table every one is kept.
pub fn parse_spot(pairs: &str, tickers: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let stats = by_name(tickers, "currency_pair")?;
    let pairs: Value = serde_json::from_str(pairs)?;
    let list = pairs
        .as_array()
        .ok_or_else(|| shape("gate pairs"))?
        .iter()
        .filter(|p| p["trade_status"] == "tradable")
        .filter_map(|p| {
            let (id, base, quote) = (p["id"].as_str()?, p["base"].as_str()?, p["quote"].as_str()?);
            if !SPOT_QUOTES.contains(&quote) {
                return None;
            }
            let t = stats.get(id);
            let turnover = t.and_then(|t| num(t.get("quote_volume")));
            if !stats.is_empty() && turnover.unwrap_or(0.0) < SPOT_MIN_TURNOVER {
                return None;
            }
            product(Venue::GateSpot, base, quote, id, p["precision"].as_u64().map(|d| d as u8), turnover, t.and_then(|t| num(t.get("last"))), t.and_then(|t| num(t.get("change_percentage"))))
        })
        .collect();
    Ok(unique(list))
}

/// Perpetuals that are trading and not on their way out.
pub fn parse_perps(contracts: &str, tickers: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let stats = by_name(tickers, "contract")?;
    let contracts: Value = serde_json::from_str(contracts)?;
    let list = contracts
        .as_array()
        .ok_or_else(|| shape("gate contracts"))?
        .iter()
        .filter(|c| c["status"] == "trading" && c["in_delisting"] != true)
        .filter_map(|c| {
            let name = c["name"].as_str()?;
            let (base, quote) = name.rsplit_once('_')?;
            let t = stats.get(name);
            product(Venue::GatePerp, base, quote, name, tick_decimals(c["order_price_round"].as_str()), t.and_then(|t| num(t.get("volume_24h_quote"))), t.and_then(|t| num(t.get("last"))), t.and_then(|t| num(t.get("change_percentage"))))
        })
        .collect();
    Ok(unique(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured 2026-11-05, trimmed.
    #[test]
    fn spot_keeps_traded_dollar_pairs_under_gates_name() {
        let pairs = r#"[
            {"id":"BTC_USDT","base":"BTC","quote":"USDT","precision":1,"trade_status":"tradable"},
            {"id":"RATS_USDT","base":"RATS","quote":"USDT","precision":9,"trade_status":"tradable"},
            {"id":"ETH_BTC","base":"ETH","quote":"BTC","precision":5,"trade_status":"tradable"},
            {"id":"OLD_USDT","base":"OLD","quote":"USDT","precision":4,"trade_status":"untradable"}]"#;
        let tickers = r#"[
            {"currency_pair":"BTC_USDT","last":"80323.3","change_percentage":"2.27","quote_volume":"1086408721.16"},
            {"currency_pair":"RATS_USDT","last":"0.000014921","change_percentage":"0","quote_volume":"120.5"},
            {"currency_pair":"ETH_BTC","last":"0.03","change_percentage":"0","quote_volume":"900000"}]"#;
        let p = parse_spot(pairs, Some(tickers)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["gate-spot:BTCUSDT"], "the thin pair, the coin pair and the closed one stay out");
        assert_eq!((p[0].native.as_deref(), p[0].price_decimals, p[0].last_price, p[0].change_pct), (Some("BTC_USDT"), Some(1), Some(80323.3), Some(2.27)));
        assert_eq!(parse_spot(pairs, None).unwrap().len(), 2, "no ticker table, no turnover to go by");
    }

    #[test]
    fn perps_read_the_contract_and_its_quote_turnover() {
        let contracts = r#"[
            {"name":"BTC_USDT","order_price_round":"0.1","status":"trading","in_delisting":false},
            {"name":"1000BONK_USDT","order_price_round":"0.000001","status":"trading","in_delisting":false},
            {"name":"GONE_USDT","order_price_round":"0.1","status":"suspend","in_delisting":false},
            {"name":"LEAVING_USDT","order_price_round":"0.1","status":"trading","in_delisting":true}]"#;
        let tickers = r#"[{"contract":"BTC_USDT","last":"80309.4","change_percentage":"2.30","volume_24h_quote":"7785996417"}]"#;
        let p = parse_perps(contracts, Some(tickers)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["gate-perp:BTCUSDT", "gate-perp:1000BONKUSDT"]);
        assert_eq!((p[0].base.as_str(), p[0].quote.as_str(), p[0].price_decimals, p[0].quote_volume_24h), ("BTC", "USDT", Some(1), Some(7785996417.0)));
    }
}
