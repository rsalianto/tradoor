//! HTX spot, named `btcusdt`, and USDT perpetuals, named `BTC-USDT`. Both are
//! `BTCUSDT` here.

use std::collections::HashMap;

use serde_json::Value;

use crate::catalog::{change, num, product, tick_decimals, unique, Product};
use crate::{shape, Venue, VenueError};

pub const SPOT_REST: &str = "https://api.huobi.pro";
pub const PERP_REST: &str = "https://api.hbdm.com";
/// Quotes a spot pair is listed under here.
const SPOT_QUOTES: [&str; 2] = ["usdt", "usdc"];

pub fn spot_symbols_url() -> String {
    format!("{SPOT_REST}/v2/settings/common/symbols")
}

pub fn spot_tickers_url() -> String {
    format!("{SPOT_REST}/market/tickers")
}

pub fn contracts_url() -> String {
    format!("{PERP_REST}/linear-swap-api/v1/swap_contract_info?business_type=swap")
}

pub fn perp_tickers_url() -> String {
    format!("{PERP_REST}/linear-swap-ex/market/detail/batch_merged?business_type=swap")
}

fn stats(body: Option<&str>, rows: &str, key: &str) -> Result<HashMap<String, Value>, VenueError> {
    let Some(body) = body else { return Ok(HashMap::new()) };
    let v: Value = serde_json::from_str(body)?;
    Ok(v[rows].as_array().map(|rows| rows.iter().filter_map(|r| Some((r[key].as_str()?.to_string(), r.clone()))).collect()).unwrap_or_default())
}

/// Online pairs in USDT or USDC. A ticker's `vol` is the quote's turnover and
/// its `open` the day's.
pub fn parse_spot(symbols: &str, tickers: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let stats = stats(tickers, "data", "symbol")?;
    let symbols: Value = serde_json::from_str(symbols)?;
    let list = symbols["data"]
        .as_array()
        .ok_or_else(|| shape("htx symbols"))?
        .iter()
        .filter(|s| s["state"] == "online")
        .filter_map(|s| {
            let (native, base, quote) = (s["sc"].as_str()?, s["bc"].as_str()?, s["qc"].as_str()?);
            if !SPOT_QUOTES.contains(&quote) {
                return None;
            }
            let t = stats.get(native);
            let last = t.and_then(|t| num(t.get("close")));
            product(Venue::HtxSpot, &base.to_ascii_uppercase(), &quote.to_ascii_uppercase(), native, s["tpp"].as_u64().map(|d| d as u8), t.and_then(|t| num(t.get("vol"))), last, change(t.and_then(|t| num(t.get("open"))), last))
        })
        .collect();
    Ok(unique(list))
}

/// Swaps that are trading (`contract_status` 1).
pub fn parse_perps(contracts: &str, tickers: Option<&str>) -> Result<Vec<Product>, VenueError> {
    let stats = stats(tickers, "ticks", "contract_code")?;
    let contracts: Value = serde_json::from_str(contracts)?;
    let list = contracts["data"]
        .as_array()
        .ok_or_else(|| shape("htx contracts"))?
        .iter()
        .filter(|c| c["contract_status"] == 1 && c["contract_type"] == "swap")
        .filter_map(|c| {
            let code = c["contract_code"].as_str()?;
            let (base, quote) = code.split_once('-')?;
            let t = stats.get(code);
            let last = t.and_then(|t| num(t.get("close")));
            // The tick comes as a number; its plain spelling gives the decimals.
            let tick = c.get("price_tick").and_then(|t| num(Some(t))).map(|t| format!("{t}"));
            product(Venue::HtxPerp, base, quote, code, tick_decimals(tick.as_deref()), t.and_then(|t| num(t.get("trade_turnover"))), last, change(t.and_then(|t| num(t.get("open"))), last))
        })
        .collect();
    Ok(unique(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured 2026-11-05, trimmed.
    #[test]
    fn spot_is_named_in_lower_case() {
        let symbols = r#"{"status":"ok","data":[
            {"sc":"btcusdt","bc":"btc","qc":"usdt","state":"online","tpp":2},
            {"sc":"ethbtc","bc":"eth","qc":"btc","state":"online","tpp":6},
            {"sc":"oldusdt","bc":"old","qc":"usdt","state":"offline","tpp":4}]}"#;
        let tickers = r#"{"data":[{"symbol":"btcusdt","open":78000.0,"close":80253.17,"vol":203732585.05}]}"#;
        let p = parse_spot(symbols, Some(tickers)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["htx-spot:BTCUSDT"]);
        assert_eq!((p[0].base.as_str(), p[0].native.as_deref(), p[0].price_decimals, p[0].quote_volume_24h), ("BTC", Some("btcusdt"), Some(2), Some(203732585.05)));
        assert!((p[0].change_pct.unwrap() - 2.8886).abs() < 0.001);
    }

    #[test]
    fn perps_keep_the_swaps_that_trade() {
        let contracts = r#"{"status":"ok","data":[
            {"symbol":"BTC","contract_code":"BTC-USDT","price_tick":0.100000000000000000,"contract_status":1,"contract_type":"swap"},
            {"symbol":"GONE","contract_code":"GONE-USDT","price_tick":0.1,"contract_status":5,"contract_type":"swap"}]}"#;
        let tickers = r#"{"status":"ok","ticks":[{"contract_code":"BTC-USDT","open":"80000","close":"80800","trade_turnover":"661726768.13"}]}"#;
        let p = parse_perps(contracts, Some(tickers)).unwrap();
        assert_eq!(p.iter().map(|p| p.market.as_str()).collect::<Vec<_>>(), ["htx-perp:BTCUSDT"]);
        assert_eq!((p[0].native.as_deref(), p[0].price_decimals, p[0].quote_volume_24h, p[0].change_pct), (Some("BTC-USDT"), Some(1), Some(661726768.13), Some(1.0)));
    }
}
