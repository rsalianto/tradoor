//! The exchanges' public trades: where they come from, how to ask, and how to
//! read one. Two things here listen — the alert watcher for the price, the flow
//! collector for which side was the aggressor — so the wire talk lives in one
//! place and both see the same trade.

use std::collections::HashMap;

use serde_json::{json, Value};
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
        Venue::GateSpot => &["wss://api.gateio.ws/ws/v4/"],
        Venue::GatePerp => &["wss://fx-ws.gateio.ws/v4/ws/usdt"],
        Venue::AsterSpot => &["wss://sstream.asterdex.com/ws"],
        Venue::AsterPerp => &["wss://fstream.asterdex.com/ws"],
        Venue::HtxSpot => &["wss://api.huobi.pro/ws"],
        Venue::HtxPerp => &["wss://api.hbdm.com/linear-swap-ws"],
        Venue::BitfinexSpot | Venue::BitfinexPerp => &["wss://api-pub.bitfinex.com/ws/2"],
        Venue::DeribitPerp => &["wss://www.deribit.com/ws/api/v2"],
        Venue::CoinbaseSpot => &["wss://ws-feed.exchange.coinbase.com"],
        Venue::BitstampSpot | Venue::BitstampPerp => &["wss://ws.bitstamp.net"],
        // An index is worked out here: nothing trades on it.
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
        Venue::GateSpot | Venue::GatePerp => {
            let channel = if venue == Venue::GateSpot { "spot.trades" } else { "futures.trades" };
            vec![format!("{{\"time\":{},\"channel\":\"{channel}\",\"event\":\"subscribe\",\"payload\":[{}]}}", tradoor_store::now_ms() / 1000, quoted(symbols))]
        }
        Venue::HtxSpot | Venue::HtxPerp => symbols.iter().map(|s| format!("{{\"sub\":\"market.{s}.trade.detail\",\"id\":\"{s}\"}}")).collect(),
        Venue::BitfinexSpot | Venue::BitfinexPerp => symbols.iter().map(|s| format!("{{\"event\":\"subscribe\",\"channel\":\"trades\",\"symbol\":\"{s}\"}}")).collect(),
        Venue::DeribitPerp => {
            let channels: Vec<String> = symbols.iter().map(|s| format!("\"trades.{s}.100ms\"")).collect();
            vec![format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"public/subscribe\",\"params\":{{\"channels\":[{}]}}}}", channels.join(","))]
        }
        // The heartbeat is what keeps a socket on a quiet product from looking dead.
        Venue::CoinbaseSpot => vec![format!("{{\"type\":\"subscribe\",\"product_ids\":[{}],\"channels\":[\"matches\",\"heartbeat\"]}}", quoted(symbols))],
        Venue::BitstampSpot | Venue::BitstampPerp => symbols.iter().map(|s| format!("{{\"event\":\"bts:subscribe\",\"data\":{{\"channel\":\"live_trades_{s}\"}}}}")).collect(),
        // Binance, and Aster, which speaks as Binance does.
        _ => {
            let params: Vec<String> = symbols.iter().map(|s| format!("\"{}@aggTrade\"", s.to_lowercase())).collect();
            vec![format!("{{\"method\":\"SUBSCRIBE\",\"params\":[{}],\"id\":1}}", params.join(","))]
        }
    }
}

/// What keeps a socket open where the exchange asks for it, or shows it is still
/// answering where a market may say nothing for minutes. Binance does not need it.
pub fn ping(venue: Venue) -> Option<String> {
    let word = match venue {
        Venue::BybitSpot | Venue::BybitPerp => "{\"op\":\"ping\"}",
        // OKX and Bitget want the bare word, and close a socket that says
        // nothing for thirty seconds.
        Venue::OkxSpot | Venue::OkxPerp | Venue::BitgetSpot | Venue::BitgetPerp => "ping",
        Venue::HyperliquidPerp | Venue::KrakenSpot => "{\"method\":\"ping\"}",
        Venue::KrakenPerp => "{\"event\":\"ping\"}",
        // Gate leaves a ping without the time unanswered.
        Venue::GateSpot => return Some(format!("{{\"time\":{},\"channel\":\"spot.ping\"}}", tradoor_store::now_ms() / 1000)),
        Venue::GatePerp => return Some(format!("{{\"time\":{},\"channel\":\"futures.ping\"}}", tradoor_store::now_ms() / 1000)),
        Venue::AsterSpot | Venue::AsterPerp => "{\"method\":\"LIST_SUBSCRIPTIONS\",\"id\":0}",
        Venue::DeribitPerp => "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"public/test\"}",
        Venue::BitstampSpot | Venue::BitstampPerp => "{\"event\":\"bts:heartbeat\"}",
        _ => return None,
    };
    Some(word.to_string())
}

/// Whether a venue's frames arrive gzipped, as binary.
pub fn gzipped(venue: Venue) -> bool {
    matches!(venue, Venue::HtxSpot | Venue::HtxPerp)
}

/// The answer to a frame of the exchange's own. HTX pings, and closes a socket
/// that does not answer.
pub fn reply(venue: Venue, text: &str) -> Option<String> {
    if !gzipped(venue) || !text.starts_with("{\"ping\"") {
        return None;
    }
    let frame: Value = serde_json::from_str(text).ok()?;
    Some(json!({ "pong": frame.get("ping")? }).to_string())
}

/// Markets one socket can follow: Bitfinex allows 25 channels on a connection,
/// and the rest of a longer list would simply be refused.
pub fn most_symbols(venue: Venue) -> usize {
    if matches!(venue, Venue::BitfinexSpot | Venue::BitfinexPerp) { 25 } else { usize::MAX }
}

/// Bitfinex numbers a subscription when it answers, and sends only that number
/// after, so this keeps which is which and puts the name back. Every trade comes
/// twice, `te` and then `tu`, and the last few on subscribing; only `te` passes,
/// so a price is heard once and an old one not at all.
#[derive(Default)]
pub struct Channels(HashMap<i64, String>);

impl Channels {
    pub fn named(&mut self, venue: Venue, text: String) -> String {
        if !matches!(venue, Venue::BitfinexSpot | Venue::BitfinexPerp) {
            return text;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&text) else { return text };
        if frame["event"] == "subscribed" {
            if let (Some(id), Some(symbol)) = (frame["chanId"].as_i64(), frame["symbol"].as_str()) {
                self.0.insert(id, symbol.to_string());
            }
        } else if frame[1] == "te" {
            if let Some(symbol) = frame[0].as_i64().and_then(|id| self.0.get(&id)) {
                return json!({ "symbol": symbol, "trade": frame[2] }).to_string();
            }
        }
        String::new()
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

/// A time an exchange writes as a string of milliseconds, which Gate takes to six decimals.
fn millis(v: &Value) -> i64 {
    v.as_str().and_then(|t| t.parse::<f64>().ok()).map(|t| t as i64).or_else(|| v.as_i64()).unwrap_or_default()
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
        Venue::GateSpot => {
            let r = &frame["result"];
            if frame["channel"] != "spot.trades" || frame["event"] != "update" {
                return;
            }
            let (Some(symbol), Some(price), Some(qty)) = (r["currency_pair"].as_str(), num(&r["price"]), num(&r["amount"])) else { return };
            into.push(Print { symbol, price, qty, buy: r["side"] == "buy", at: millis(&r["create_time_ms"]) });
        }
        Venue::GatePerp => {
            if frame["channel"] != "futures.trades" || frame["event"] != "update" {
                return;
            }
            let Some(rows) = frame["result"].as_array() else { return };
            for r in rows {
                let (Some(symbol), Some(price), Some(size)) = (r["contract"].as_str(), num(&r["price"]), r["size"].as_f64()) else { continue };
                // In contracts; a negative size is a sale. It is the price that is read here.
                into.push(Print { symbol, price, qty: size.abs(), buy: size > 0.0, at: millis(&r["create_time_ms"]) });
            }
        }
        Venue::HtxSpot | Venue::HtxPerp => {
            let Some(symbol) = frame["ch"].as_str().and_then(|c| c.strip_prefix("market.")).and_then(|c| c.strip_suffix(".trade.detail")) else { return };
            let Some(rows) = frame["tick"]["data"].as_array() else { return };
            for r in rows {
                let (Some(price), Some(qty)) = (num(&r["price"]), num(&r["amount"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["direction"] == "buy", at: millis(&r["ts"]) });
            }
        }
        // As `Channels` leaves it: `[id, time, amount, price]`, a negative amount a sale.
        Venue::BitfinexSpot | Venue::BitfinexPerp => {
            let t = &frame["trade"];
            let (Some(symbol), Some(price), Some(amount)) = (frame["symbol"].as_str(), num(&t[3]), t[2].as_f64()) else { return };
            into.push(Print { symbol, price, qty: amount.abs(), buy: amount > 0.0, at: millis(&t[1]) });
        }
        Venue::DeribitPerp => {
            if !frame["params"]["channel"].as_str().is_some_and(|c| c.starts_with("trades.")) {
                return;
            }
            let Some(rows) = frame["params"]["data"].as_array() else { return };
            for r in rows {
                let (Some(symbol), Some(price), Some(qty)) = (r["instrument_name"].as_str(), num(&r["price"]), num(&r["amount"])) else { continue };
                into.push(Print { symbol, price, qty, buy: r["direction"] == "buy", at: millis(&r["timestamp"]) });
            }
        }
        Venue::CoinbaseSpot => {
            if frame["type"] != "match" {
                return;
            }
            let (Some(symbol), Some(price), Some(qty)) = (frame["product_id"].as_str(), num(&frame["price"]), num(&frame["size"])) else { return };
            // `side` is the maker's, so the taker bought when it is a sell.
            into.push(Print { symbol, price, qty, buy: frame["side"] == "sell", at: 0 });
        }
        Venue::BitstampSpot | Venue::BitstampPerp => {
            if frame["event"] != "trade" {
                return;
            }
            let Some(symbol) = frame["channel"].as_str().and_then(|c| c.strip_prefix("live_trades_")) else { return };
            let d = &frame["data"];
            let (Some(price), Some(qty)) = (num(&d["price"]), num(&d["amount"])) else { return };
            // `type` 0 is a buy. Spot's `timestamp` is in seconds and a perpetual's in milliseconds.
            into.push(Print { symbol, price, qty, buy: d["type"] == 0, at: millis(&d["microtimestamp"]) / 1000 });
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
    timed_prints(venue, text).into_iter().map(|(symbol, price, qty, buy, _)| (symbol, price, qty, buy)).collect()
}

/// The same with each trade's time, 0 where the frame's is not read.
pub fn timed_prints(venue: Venue, text: &str) -> Vec<(String, f64, f64, bool, i64)> {
    let Ok(frame) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut buffer = Vec::new();
    read_prints(venue, &frame, &mut buffer);
    buffer.iter().map(|p| (p.symbol.to_string(), p.price, p.qty, p.buy, p.at)).collect()
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
        assert_eq!(ping(Venue::OkxPerp).as_deref(), Some("ping"), "OKX wants the bare word");
        assert!(renames(Venue::OkxPerp) && renames(Venue::HyperliquidPerp) && !renames(Venue::BinancePerp));
    }

    // Captured 2026-11-05, trimmed.
    #[test]
    fn gate_says_a_sale_two_ways() {
        let spot = r#"{"time":1793894074,"channel":"spot.trades","event":"update","result":{"id":227221453,"create_time_ms":"1793894074848.662000","side":"sell","currency_pair":"BTC_USDT","amount":"0.380575","price":"75588.6"}}"#;
        assert_eq!(timed_prints(Venue::GateSpot, spot), [("BTC_USDT".to_string(), 75_588.6, 0.380575, false, 1_793_894_074_848)]);
        let perp = r#"{"time":1793894074,"channel":"futures.trades","event":"update","result":[{"id":869573784,"size":6881,"create_time_ms":1793893971399,"price":"75651.6","contract":"BTC_USDT"},{"id":869573785,"size":-20,"create_time_ms":1793893971400,"price":"75651.5","contract":"BTC_USDT"}]}"#;
        let read = prints(Venue::GatePerp, perp);
        assert_eq!((read[0].2, read[0].3), (6881.0, true));
        assert_eq!((read[1].2, read[1].3), (20.0, false), "a negative size is a sale");
        assert!(prints(Venue::GateSpot, r#"{"channel":"spot.trades","event":"subscribe","result":{"status":"success"}}"#).is_empty());
        assert!(prints(Venue::GatePerp, r#"{"channel":"futures.pong","event":"","result":null}"#).is_empty());
        let ask = subscribe(Venue::GatePerp, &["BTC_USDT".to_string()]);
        assert!(ask[0].contains(r#""channel":"futures.trades","event":"subscribe","payload":["BTC_USDT"]"#), "{ask:?}");
        assert!(ping(Venue::GateSpot).unwrap().contains("\"time\":17"), "a ping without the time goes unanswered");
    }

    #[test]
    fn htx_is_answered_and_read_by_its_channel() {
        let spot = r#"{"ch":"market.btcusdt.trade.detail","ts":1,"tick":{"id":9,"ts":1,"data":[{"id":1,"ts":1793894074053,"tradeId":103668665080,"amount":6.75E-4,"price":75595.79,"direction":"buy"}]}}"#;
        assert_eq!(timed_prints(Venue::HtxSpot, spot), [("btcusdt".to_string(), 75_595.79, 6.75e-4, true, 1_793_894_074_053)]);
        let perp = r#"{"ch":"market.BTC-USDT.trade.detail","ts":1,"tick":{"id":9,"ts":1,"data":[{"amount":132,"quantity":0.132,"ts":1,"id":1,"price":75578.5,"direction":"sell"}]}}"#;
        assert_eq!(prints(Venue::HtxPerp, perp), [("BTC-USDT".to_string(), 75_578.5, 132.0, false)]);
        assert!(prints(Venue::HtxPerp, r#"{"ch":"market.BTC-USDT.depth.step0","tick":{"bids":[]}}"#).is_empty());
        assert_eq!(reply(Venue::HtxPerp, r#"{"ping":1793894075239}"#).as_deref(), Some(r#"{"pong":1793894075239}"#));
        assert_eq!(reply(Venue::HtxPerp, r#"{"id":"a","status":"ok"}"#), None);
        assert_eq!(reply(Venue::BinancePerp, r#"{"ping":1}"#), None);
        assert_eq!(subscribe(Venue::HtxSpot, &["btcusdt".to_string()]), [r#"{"sub":"market.btcusdt.trade.detail","id":"btcusdt"}"#]);
        assert!(gzipped(Venue::HtxSpot) && !gzipped(Venue::GateSpot));
    }

    #[test]
    fn bitfinex_gets_its_names_back_and_each_trade_once() {
        let mut channels = Channels::default();
        let venue = Venue::BitfinexSpot;
        assert_eq!(channels.named(venue, r#"{"event":"subscribed","channel":"trades","chanId":118,"symbol":"tBTCUSD","pair":"BTCUSD"}"#.to_string()), "");
        assert_eq!(channels.named(venue, "[118,[[2006727740,1793894074662,0.1,70000]]]".to_string()), "", "the last few on subscribing are old");
        assert_eq!(channels.named(venue, "[118,\"hb\"]".to_string()), "");
        assert_eq!(channels.named(venue, "[118,\"tu\",[2006727741,1793894076402,0.03,75620]]".to_string()), "", "the second telling of a trade");
        assert_eq!(channels.named(venue, "[999,\"te\",[1,2,3,4]]".to_string()), "", "a channel never named");
        let trade = channels.named(venue, "[118,\"te\",[2006727741,1793894076402,-0.03,75620]]".to_string());
        assert_eq!(timed_prints(venue, &trade), [("tBTCUSD".to_string(), 75_620.0, 0.03, false, 1_793_894_076_402)]);
        assert_eq!(channels.named(Venue::OkxSpot, "pong".to_string()), "pong", "everyone else passes through");
        assert_eq!(most_symbols(venue), 25);
    }

    #[test]
    fn deribit_coinbase_bitstamp_and_aster_are_read_too() {
        let deribit = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.BTC-PERPETUAL.100ms","data":[{"timestamp":1793894074575,"price":75502.5,"direction":"buy","instrument_name":"BTC-PERPETUAL","amount":1320.0}]}}"#;
        assert_eq!(timed_prints(Venue::DeribitPerp, deribit), [("BTC-PERPETUAL".to_string(), 75_502.5, 1320.0, true, 1_793_894_074_575)]);
        assert!(prints(Venue::DeribitPerp, r#"{"jsonrpc":"2.0","id":0,"result":{"version":"1"}}"#).is_empty());

        let coinbase = r#"{"type":"match","trade_id":1,"side":"sell","size":"0.0008","price":"75479","product_id":"BTC-USD","time":"2026-11-05T15:54:35.753678Z"}"#;
        assert_eq!(prints(Venue::CoinbaseSpot, coinbase), [("BTC-USD".to_string(), 75_479.0, 0.0008, true)], "a maker selling is a taker buying");
        assert!(prints(Venue::CoinbaseSpot, &coinbase.replace("\"match\"", "\"last_match\"")).is_empty(), "the one it opens with is old");
        assert!(prints(Venue::CoinbaseSpot, r#"{"type":"heartbeat","product_id":"BTC-USD"}"#).is_empty());

        let spot = r#"{"data":{"id":667973520,"timestamp":"1793894075","amount":0.007044,"price":75482.9,"type":1,"microtimestamp":"1793894075583000"},"channel":"live_trades_btcusd","event":"trade"}"#;
        assert_eq!(timed_prints(Venue::BitstampSpot, spot), [("btcusd".to_string(), 75_482.9, 0.007044, false, 1_793_894_075_583)]);
        let perp = r#"{"data":{"id":2020552592496541696,"type":0,"amount":0.00026,"price":75537,"microtimestamp":"1793894078103000","timestamp":"1793894078103"},"channel":"live_trades_btcusd-perp","event":"trade"}"#;
        assert_eq!(timed_prints(Venue::BitstampPerp, perp), [("btcusd-perp".to_string(), 75_537.0, 0.00026, true, 1_793_894_078_103)]);
        assert!(prints(Venue::BitstampSpot, r#"{"event":"bts:heartbeat","channel":"","data":{"status":"success"}}"#).is_empty());

        let aster = r#"{"e":"aggTrade","E":1,"a":1,"s":"BTCUSDT","p":"75568.1","q":"0.331","T":1793894074800,"m":false}"#;
        assert_eq!(prints(Venue::AsterPerp, aster), [("BTCUSDT".to_string(), 75_568.1, 0.331, true)]);
        assert_eq!(subscribe(Venue::AsterSpot, &["BTCUSDT".to_string()]), [r#"{"method":"SUBSCRIBE","params":["btcusdt@aggTrade"],"id":1}"#]);
    }

    #[test]
    fn every_venue_but_the_index_has_somewhere_to_listen() {
        for venue in Venue::ALL {
            assert_eq!(stream_urls(venue).is_empty(), venue == Venue::Index, "{}", venue.id());
        }
    }
}
