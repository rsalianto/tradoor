//! The exchanges' public trades: where they come from, how to ask, and how to
//! read one. Two things here listen — the alert watcher for the price, the flow
//! collector for which side was the aggressor — so the wire talk lives in one
//! place and both see the same trade.

use serde_json::Value;
use tradoor_venues::{Kind, Venue};

/// The public streams each venue's trades come on, best first. A venue with
/// more than one is tried in turn: an address that answers but then says
/// nothing is as useless as one that refuses, and which is which moves with the
/// network — the page carries the same lists for the same reason.
pub fn stream_urls(venue: Venue) -> &'static [&'static str] {
    match venue {
        Venue::BinanceSpot => &["wss://stream.binance.com:9443/ws", "wss://data-stream.binance.vision/ws"],
        Venue::BinancePerp => &["wss://fstream.binance.com/market/ws", "wss://fstream.binance.com/ws"],
        Venue::BybitSpot => &["wss://stream.bybit.com/v5/public/spot"],
        Venue::BybitPerp => &["wss://stream.bybit.com/v5/public/linear"],
        Venue::OkxSpot | Venue::OkxPerp => &["wss://ws.okx.com/ws/v5/public"],
        Venue::HyperliquidPerp => &["wss://api.hyperliquid.xyz/ws"],
        Venue::BitgetSpot | Venue::BitgetPerp => &["wss://ws.bitget.com/v2/ws/public"],
        Venue::KrakenSpot => &["wss://ws.kraken.com/v2"],
        Venue::KrakenPerp => &["wss://futures.kraken.com/ws/v1"],
        Venue::LighterPerp => &["wss://mainnet.zklighter.elliot.ai/stream"],
        // Worked out here; nothing trades on it.
        Venue::Index => &[],
    }
}

/// Whether a venue calls its markets something other than we do, so the names
/// asked for and the names answered with have to be translated. Binance and
/// Bybit use ours; the rest have their own — `BTC-USDT-SWAP`, `BTC`, `BTC/USD`,
/// `PI_XBTUSD`, or a market number.
pub fn renames(venue: Venue) -> bool {
    !matches!(venue, Venue::BinanceSpot | Venue::BinancePerp | Venue::BybitSpot | Venue::BybitPerp)
}

/// What to send to ask for these markets' trades, in the exchange's own names.
/// Most take one frame for the lot; Hyperliquid takes one subscription each.
pub fn subscribe(venue: Venue, symbols: &[String]) -> Vec<String> {
    match venue {
        Venue::BybitSpot | Venue::BybitPerp => {
            let args: Vec<String> = symbols.iter().map(|s| format!("\"publicTrade.{s}\"")).collect();
            vec![format!("{{\"op\":\"subscribe\",\"args\":[{}]}}", args.join(","))]
        }
        Venue::OkxSpot | Venue::OkxPerp => {
            let args: Vec<String> = symbols.iter().map(|s| format!("{{\"channel\":\"trades\",\"instId\":\"{s}\"}}")).collect();
            vec![format!("{{\"op\":\"subscribe\",\"args\":[{}]}}", args.join(","))]
        }
        Venue::HyperliquidPerp => symbols.iter().map(|s| format!("{{\"method\":\"subscribe\",\"subscription\":{{\"type\":\"trades\",\"coin\":\"{s}\"}}}}")).collect(),
        Venue::BitgetSpot | Venue::BitgetPerp => {
            let inst = if venue.kind() == Kind::Perp { "USDT-FUTURES" } else { "SPOT" };
            let args: Vec<String> = symbols.iter().map(|s| format!("{{\"instType\":\"{inst}\",\"channel\":\"trade\",\"instId\":\"{s}\"}}")).collect();
            vec![format!("{{\"op\":\"subscribe\",\"args\":[{}]}}", args.join(","))]
        }
        Venue::KrakenSpot => vec![format!("{{\"method\":\"subscribe\",\"params\":{{\"channel\":\"trade\",\"symbol\":[{}]}}}}", quoted(symbols))],
        Venue::KrakenPerp => vec![format!("{{\"event\":\"subscribe\",\"feed\":\"trade\",\"product_ids\":[{}]}}", quoted(symbols))],
        // Lighter is asked with a slash and answers with a colon.
        Venue::LighterPerp => symbols.iter().map(|s| format!("{{\"type\":\"subscribe\",\"channel\":\"market_stats/{s}\"}}")).collect(),
        _ => {
            let params: Vec<String> = symbols.iter().map(|s| format!("\"{}@aggTrade\"", s.to_lowercase())).collect();
            vec![format!("{{\"method\":\"SUBSCRIBE\",\"params\":[{}],\"id\":1}}", params.join(","))]
        }
    }
}

/// What keeps a socket open where the exchange asks for it. Binance does not.
pub fn ping(venue: Venue) -> Option<&'static str> {
    match venue {
        Venue::BybitSpot | Venue::BybitPerp => Some("{\"op\":\"ping\"}"),
        // OKX and Bitget want the bare word, and close a socket that says
        // nothing for thirty seconds.
        Venue::OkxSpot | Venue::OkxPerp | Venue::BitgetSpot | Venue::BitgetPerp => Some("ping"),
        Venue::HyperliquidPerp | Venue::KrakenSpot => Some("{\"method\":\"ping\"}"),
        Venue::KrakenPerp => Some("{\"event\":\"ping\"}"),
        _ => None,
    }
}

fn quoted(symbols: &[String]) -> String {
    symbols.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(",")
}

/// One trade as both readers need it: what it cost, how much, and whether the
/// aggressor was buying — the taker's side, which is what flow counts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Print<'a> {
    pub symbol: &'a str,
    pub price: f64,
    pub qty: f64,
    pub buy: bool,
    pub at: i64,
}

/// A time an exchange writes as a string of milliseconds.
fn millis(v: &Value) -> i64 {
    v.as_str().and_then(|t| t.parse().ok()).or_else(|| v.as_i64()).unwrap_or_default()
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.parse().ok(),
        _ => v.as_f64(),
    }
    .filter(|p: &f64| p.is_finite() && *p > 0.0)
}

/// The trades in one frame. Anything else — a subscription answer, a heartbeat
/// — carries no trade and is skipped. The frame is parsed into `into` so the
/// borrowed symbols stay alive as long as the caller needs them.
pub fn read_prints<'a>(venue: Venue, frame: &'a Value, into: &mut Vec<Print<'a>>) {
    into.clear();
    match venue {
        Venue::OkxSpot | Venue::OkxPerp => {
            if frame["arg"]["channel"] != "trades" {
                return;
            }
            let Some(rows) = frame["data"].as_array() else { return };
            for r in rows {
                let (Some(symbol), Some(price), Some(qty)) = (r["instId"].as_str(), num(&r["px"]), num(&r["sz"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["side"] == "buy", at: millis(&r["ts"]) });
            }
        }
        Venue::HyperliquidPerp => {
            if frame["channel"] != "trades" {
                return;
            }
            let Some(rows) = frame["data"].as_array() else { return };
            for r in rows {
                let (Some(symbol), Some(price), Some(qty)) = (r["coin"].as_str(), num(&r["px"]), num(&r["sz"])) else { continue };
                // `B` is the aggressor buying, `A` taking the ask away.
                into.push(Print { symbol, price, qty, buy: r["side"] == "B", at: r["time"].as_i64().unwrap_or_default() });
            }
        }
        Venue::BitgetSpot | Venue::BitgetPerp => {
            if frame["arg"]["channel"] != "trade" {
                return;
            }
            let (Some(symbol), Some(rows)) = (frame["arg"]["instId"].as_str(), frame["data"].as_array()) else { return };
            for r in rows {
                let (Some(price), Some(qty)) = (num(&r["price"]), num(&r["size"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["side"] == "buy", at: millis(&r["ts"]) });
            }
        }
        Venue::KrakenSpot => {
            if frame["channel"] != "trade" {
                return;
            }
            let Some(rows) = frame["data"].as_array() else { return };
            for r in rows {
                let (Some(symbol), Some(price), Some(qty)) = (r["symbol"].as_str(), num(&r["price"]), num(&r["qty"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["side"] == "buy", at: 0 });
            }
        }
        Venue::KrakenPerp => {
            // One trade a frame, or a snapshot of the last few on subscribing.
            let rows: Vec<&Value> = match frame["feed"].as_str() {
                Some("trade") => vec![frame],
                Some("trade_snapshot") => frame["trades"].as_array().map(|r| r.iter().collect()).unwrap_or_default(),
                _ => return,
            };
            for r in rows {
                let (Some(symbol), Some(price), Some(qty)) = (r["product_id"].as_str().or_else(|| frame["product_id"].as_str()), num(&r["price"]), num(&r["qty"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["side"] == "buy", at: r["time"].as_i64().unwrap_or_default() });
            }
        }
        Venue::LighterPerp => {
            // Lighter streams the market rather than each trade: the last price
            // is there, which is what an alert reads; flow is not followed here.
            let Some(market) = frame["channel"].as_str().and_then(|c| c.strip_prefix("market_stats:")) else { return };
            let Some(price) = num(&frame["market_stats"]["last_trade_price"]) else { return };
            into.push(Print { symbol: market, price, qty: 0.0, buy: false, at: 0 });
        }
        Venue::BybitSpot | Venue::BybitPerp => {
            if !frame["topic"].as_str().is_some_and(|t| t.starts_with("publicTrade.")) {
                return;
            }
            let Some(rows) = frame["data"].as_array() else { return };
            for r in rows {
                let (Some(symbol), Some(price), Some(qty)) = (r["s"].as_str(), num(&r["p"]), num(&r["v"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["S"] == "Buy", at: r["T"].as_i64().unwrap_or_default() });
            }
        }
        _ => {
            if frame["e"] != "aggTrade" {
                return;
            }
            let (Some(symbol), Some(price), Some(qty)) = (frame["s"].as_str(), num(&frame["p"]), num(&frame["q"])) else { return };
            // `m` is whether the BUYER was the maker, so the taker bought when it is false.
            into.push(Print { symbol, price, qty, buy: frame["m"] == false, at: frame["T"].as_i64().unwrap_or_default() });
        }
    }
}

/// The trades in one frame, allocated — for a caller with no buffer to lend.
pub fn prints(venue: Venue, text: &str) -> Vec<(String, f64, f64, bool)> {
    let Ok(frame) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut buffer = Vec::new();
    read_prints(venue, &frame, &mut buffer);
    buffer.iter().map(|p| (p.symbol.to_string(), p.price, p.qty, p.buy)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trade_says_its_size_and_which_side_took_it() {
        let binance = r#"{"e":"aggTrade","E":1,"s":"BTCUSDT","p":"84250.10","q":"0.5","m":false,"T":7}"#;
        assert_eq!(prints(Venue::BinancePerp, binance), [("BTCUSDT".to_string(), 84_250.10, 0.5, true)], "the buyer took it");
        let sold = binance.replace("\"m\":false", "\"m\":true");
        assert_eq!(prints(Venue::BinancePerp, &sold)[0].3, false, "the buyer made it, so the seller took it");
        assert!(prints(Venue::BinancePerp, r#"{"result":null,"id":1}"#).is_empty());

        let bybit = r#"{"topic":"publicTrade.BTCUSDT","data":[{"s":"BTCUSDT","p":"84251.5","v":"0.2","S":"Buy","T":9},{"s":"BTCUSDT","p":"84252","v":"0.1","S":"Sell","T":9}]}"#;
        let read = prints(Venue::BybitSpot, bybit);
        assert_eq!(read.len(), 2);
        assert_eq!((read[0].2, read[0].3), (0.2, true));
        assert_eq!((read[1].2, read[1].3), (0.1, false));
        assert!(prints(Venue::BybitSpot, r#"{"success":true}"#).is_empty());
    }

    #[test]
    fn a_trade_carries_its_price_and_size() {
        let frame: Value = serde_json::from_str(r#"{"e":"aggTrade","s":"BTCUSDT","p":"80000","q":"0.25","m":false}"#).unwrap();
        let mut into = Vec::new();
        read_prints(Venue::BinancePerp, &frame, &mut into);
        assert_eq!((into[0].price, into[0].qty, into[0].buy), (80_000.0, 0.25, true));
    }

    #[test]
    fn okx_and_hyperliquid_are_read_in_their_own_names() {
        let okx = r#"{"arg":{"channel":"trades","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","px":"84100.5","sz":"3","side":"buy","ts":"1790000000000"}]}"#;
        assert_eq!(prints(Venue::OkxPerp, okx), [("BTC-USDT-SWAP".to_string(), 84_100.5, 3.0, true)]);
        assert_eq!(prints(Venue::OkxPerp, &okx.replace("\"side\":\"buy\"", "\"side\":\"sell\""))[0].3, false);
        assert!(prints(Venue::OkxPerp, r#"{"event":"subscribe","arg":{"channel":"trades","instId":"BTC-USDT-SWAP"}}"#).is_empty());
        assert!(prints(Venue::OkxPerp, "pong").is_empty(), "the answer to a ping is not a trade");

        let hl = r#"{"channel":"trades","data":[{"coin":"BTC","side":"B","px":"84120.0","sz":"0.5","time":1790000000000},{"coin":"BTC","side":"A","px":"84119.0","sz":"0.1","time":1790000000001}]}"#;
        let read = prints(Venue::HyperliquidPerp, hl);
        assert_eq!(read.len(), 2);
        assert_eq!((read[0].0.as_str(), read[0].3), ("BTC", true), "B is the aggressor buying");
        assert_eq!(read[1].3, false);
        assert!(prints(Venue::HyperliquidPerp, r#"{"channel":"subscriptionResponse","data":{}}"#).is_empty());
    }

    #[test]
    fn the_smaller_exchanges_are_read_too() {
        let bitget = r#"{"action":"snapshot","arg":{"instType":"USDT-FUTURES","channel":"trade","instId":"BTCUSDT"},"data":[{"ts":"1790000000000","price":"84100","size":"0.3","side":"buy"}]}"#;
        assert_eq!(prints(Venue::BitgetPerp, bitget), [("BTCUSDT".to_string(), 84_100.0, 0.3, true)]);
        assert!(prints(Venue::BitgetPerp, r#"{"event":"subscribe","arg":{"channel":"trade"}}"#).is_empty());

        let kraken = r#"{"channel":"trade","data":[{"symbol":"BTC/USD","side":"sell","price":84090.1,"qty":0.02,"timestamp":"2026-09-25T08:10:49.1Z"}]}"#;
        let read = prints(Venue::KrakenSpot, kraken);
        assert_eq!((read[0].0.as_str(), read[0].1, read[0].3), ("BTC/USD", 84_090.1, false));

        let futures = r#"{"feed":"trade","product_id":"PI_XBTUSD","side":"buy","price":84080.0,"qty":100,"time":1790000000000}"#;
        assert_eq!(prints(Venue::KrakenPerp, futures), [("PI_XBTUSD".to_string(), 84_080.0, 100.0, true)]);
        let snapshot = r#"{"feed":"trade_snapshot","product_id":"PI_XBTUSD","trades":[{"price":84070.0,"qty":5,"side":"sell","time":1}]}"#;
        assert_eq!(prints(Venue::KrakenPerp, snapshot).len(), 1, "the trades it opens with are trades too");
        assert!(prints(Venue::KrakenPerp, r#"{"feed":"heartbeat","time":1}"#).is_empty());

        // Lighter gives the market, not each trade: the last price is the alert's.
        let lighter = r#"{"type":"update/market_stats","channel":"market_stats:1","market_stats":{"last_trade_price":"84050.5"}}"#;
        let read = prints(Venue::LighterPerp, lighter);
        assert_eq!((read[0].0.as_str(), read[0].1), ("1", 84_050.5));
        assert!(prints(Venue::LighterPerp, r#"{"type":"connected"}"#).is_empty());
    }

    #[test]
    fn each_exchange_is_asked_in_its_own_words() {
        let symbols = ["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        assert_eq!(subscribe(Venue::BinanceSpot, &symbols), [r#"{"method":"SUBSCRIBE","params":["btcusdt@aggTrade","ethusdt@aggTrade"],"id":1}"#]);
        assert_eq!(subscribe(Venue::BybitPerp, &symbols), [r#"{"op":"subscribe","args":["publicTrade.BTCUSDT","publicTrade.ETHUSDT"]}"#]);
        let okx = ["BTC-USDT-SWAP".to_string()];
        assert_eq!(subscribe(Venue::OkxPerp, &okx), [r#"{"op":"subscribe","args":[{"channel":"trades","instId":"BTC-USDT-SWAP"}]}"#]);
        let coins = ["BTC".to_string(), "kPEPE".to_string()];
        assert_eq!(
            subscribe(Venue::HyperliquidPerp, &coins),
            [r#"{"method":"subscribe","subscription":{"type":"trades","coin":"BTC"}}"#, r#"{"method":"subscribe","subscription":{"type":"trades","coin":"kPEPE"}}"#],
            "Hyperliquid takes one subscription at a time"
        );

        assert_eq!(subscribe(Venue::KrakenSpot, &["BTC/USD".to_string()]), [r#"{"method":"subscribe","params":{"channel":"trade","symbol":["BTC/USD"]}}"#]);
        assert_eq!(subscribe(Venue::KrakenPerp, &["PI_XBTUSD".to_string()]), [r#"{"event":"subscribe","feed":"trade","product_ids":["PI_XBTUSD"]}"#]);
        assert_eq!(subscribe(Venue::BitgetSpot, &["BTCUSDT".to_string()]), [r#"{"op":"subscribe","args":[{"instType":"SPOT","channel":"trade","instId":"BTCUSDT"}]}"#]);
        assert!(subscribe(Venue::BitgetPerp, &["BTCUSDT".to_string()])[0].contains("USDT-FUTURES"), "a perpetual is another instrument type there");
        assert_eq!(subscribe(Venue::LighterPerp, &["1".to_string()]), [r#"{"type":"subscribe","channel":"market_stats/1"}"#], "asked with a slash, answered with a colon");
        assert!(stream_urls(Venue::BinancePerp).len() > 1, "an address that goes quiet is retried on the next one");
        assert!(ping(Venue::BybitSpot).is_some() && ping(Venue::BinanceSpot).is_none());
        assert_eq!(ping(Venue::OkxPerp), Some("ping"), "OKX wants the bare word");
        assert!(renames(Venue::OkxPerp) && renames(Venue::HyperliquidPerp) && !renames(Venue::BinancePerp));
    }
}
