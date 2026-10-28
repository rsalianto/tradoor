//! Liquidations for a few tickers, collected on every perpetual exchange and
//! kept for a day, so the panel's 1H to 24H totals are whole the moment it opens.
//!
//! One WebSocket per exchange carries every ticker. Hits go down a channel to a
//! single writer, which inserts them in batches and trims the table to 24 hours.
//! None of it is on the request path: an exchange that is down only shows in the
//! status the API reports next to the totals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{sleep, sleep_until, Instant};
use tokio_tungstenite::tungstenite::Message;

use crate::api::{ApiError, ApiResult, AppState};

pub const DAY_MS: i64 = 86_400_000;
/// The totals the panel shows, shortest first.
const WINDOWS: [(&str, i64); 4] = [("1h", 3_600_000), ("4h", 14_400_000), ("12h", 43_200_000), ("24h", DAY_MS)];
/// Seconds between reconnects, growing while an exchange stays unreachable.
const RETRY_SECS: [u64; 5] = [1, 3, 5, 10, 30];
/// Rows the panel's tape opens with.
const RECENT_ROWS: i64 = 200;
/// Hits waiting to be written before the writer stops waiting for the second.
const BATCH: usize = 500;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A ticker as the server takes it: `BTCUSDT`, the same spelling on every exchange.
pub fn is_ticker(s: &str) -> bool {
    (2..=30).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub venue: &'static str,
    pub symbol: String,
    /// Epoch milliseconds, as the exchange stamped it.
    pub time: i64,
    pub price: f64,
    pub qty: f64,
    /// A long position was closed out.
    pub long: bool,
}

/// What an exchange's stream needs looked up before it is subscribed to:
/// contract sizes, where a liquidation is counted in contracts, and Lighter's
/// market numbers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lookup {
    /// Coins per contract, by the exchange's symbol.
    pub sizes: HashMap<String, f64>,
    /// Lighter's market id by coin, and back.
    pub markets: HashMap<String, u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prepare {
    Nothing,
    OkxSizes,
    GateSizes,
    LighterMarkets,
}

/// An exchange's liquidation stream: where it is and how it speaks.
#[derive(Clone, Copy)]
pub struct Venue {
    pub id: &'static str,
    pub url: &'static str,
    subscribe: fn(&[String], &Lookup) -> Vec<String>,
    parse: fn(&str, &Lookup) -> Vec<Hit>,
    /// An answer the exchange expects to a frame of its own — its ping.
    reply: fn(&str) -> Option<String>,
    /// A frame to send this often, for exchanges that close a socket nobody talks on.
    ping: Option<(fn() -> String, Duration)>,
    /// Reconnect when not a single frame has arrived for this long.
    silence: Duration,
    prepare: Prepare,
    /// Frames arrive gzipped, as binary.
    gzip: bool,
}

const fn venue(id: &'static str, url: &'static str, subscribe: fn(&[String], &Lookup) -> Vec<String>, parse: fn(&str, &Lookup) -> Vec<Hit>) -> Venue {
    Venue { id, url, subscribe, parse, reply: no_reply, ping: None, silence: Duration::from_secs(600), prepare: Prepare::Nothing, gzip: false }
}

fn no_reply(_: &str) -> Option<String> {
    None
}

const fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// Every exchange with a public feed of its liquidations — the set rsalianto
/// follows, and Deribit and Kraken, whose trade feeds mark them. Hyperliquid
/// publishes none. Where a feed carries every market, the hits are matched to
/// the followed tickers by coin.
pub const VENUES: [Venue; 12] = [
    // Binance pings every three minutes and drops a socket after a day.
    venue("binance-perp", "wss://fstream.binance.com/market/ws", binance_subscribe, parse_binance_venue),
    // Bybit closes a socket that has not pinged in twenty seconds.
    Venue { ping: Some((|| r#"{"op":"ping"}"#.into(), secs(15))), silence: secs(60), ..venue("bybit-perp", "wss://stream.bybit.com/v5/public/linear", bybit_subscribe, parse_bybit_venue) },
    Venue { ping: Some((|| "ping".into(), secs(25))), silence: secs(90), prepare: Prepare::OkxSizes, ..venue("okx-perp", "wss://ws.okx.com/ws/v5/public", okx_subscribe, parse_okx) },
    Venue { ping: Some((|| "ping".into(), secs(25))), silence: secs(90), ..venue("bitget-perp", "wss://ws.bitget.com/v3/ws/public", bitget_subscribe, parse_bitget) },
    Venue { ping: Some((gate_ping, secs(18))), silence: secs(60), prepare: Prepare::GateSizes, ..venue("gate-perp", "wss://fx-ws.gateio.ws/v4/ws/usdt", gate_subscribe, parse_gate) },
    Venue { reply: htx_reply, silence: secs(60), gzip: true, ..venue("htx-perp", "wss://api.hbdm.com/linear-swap-notification", htx_subscribe, parse_htx) },
    Venue { ping: Some((|| "ping".into(), secs(20))), silence: secs(90), ..venue("bitmex-perp", "wss://www.bitmex.com/realtime", bitmex_subscribe, parse_bitmex) },
    venue("bitfinex-perp", "wss://api-pub.bitfinex.com/ws/2", bitfinex_subscribe, parse_bitfinex),
    Venue { ping: Some((|| r#"{"type":"ping"}"#.into(), secs(30))), silence: secs(120), prepare: Prepare::LighterMarkets, ..venue("lighter-perp", "wss://mainnet.zklighter.elliot.ai/stream", lighter_subscribe, parse_lighter) },
    venue("aster-perp", "wss://fstream.asterdex.com/ws", binance_subscribe, parse_aster),
    Venue { reply: deribit_reply, silence: secs(90), ..venue("deribit-perp", "wss://www.deribit.com/ws/api/v2", deribit_subscribe, parse_deribit) },
    Venue { silence: secs(120), ..venue("kraken-perp", "wss://futures.kraken.com/ws/v1", kraken_subscribe, parse_kraken) },
];

/// The coin a ticker trades: `BTCUSDT` is BTC.
pub fn coin_of(ticker: &str) -> &str {
    ["USDT", "USDC", "USD"].iter().find_map(|q| ticker.strip_suffix(q)).filter(|c| !c.is_empty()).unwrap_or(ticker)
}

fn binance_subscribe(tickers: &[String], _: &Lookup) -> Vec<String> {
    let params: Vec<String> = tickers.iter().map(|t| format!("{}@forceOrder", t.to_lowercase())).collect();
    vec![json!({ "method": "SUBSCRIBE", "params": params, "id": 1 }).to_string()]
}

/// One topic a frame: Bybit refuses a whole frame over one topic it does not know.
fn bybit_subscribe(tickers: &[String], _: &Lookup) -> Vec<String> {
    tickers.iter().map(|t| json!({ "op": "subscribe", "args": [format!("allLiquidation.{t}")] }).to_string()).collect()
}

/// OKX and Bitget send every market's liquidations on one channel.
fn okx_subscribe(_: &[String], _: &Lookup) -> Vec<String> {
    vec![json!({ "op": "subscribe", "args": [{ "channel": "liquidation-orders", "instType": "SWAP" }] }).to_string()]
}

fn bitget_subscribe(_: &[String], _: &Lookup) -> Vec<String> {
    vec![json!({ "op": "subscribe", "args": [{ "instType": "usdt-futures", "topic": "liquidation" }] }).to_string()]
}

fn gate_subscribe(tickers: &[String], _: &Lookup) -> Vec<String> {
    let contracts: Vec<String> = tickers.iter().map(|t| format!("{}_USDT", coin_of(t))).collect();
    vec![json!({ "time": now_ms() / 1000, "channel": "futures.public_liquidates", "event": "subscribe", "payload": contracts }).to_string()]
}

/// Gate's ping carries the time; one without it is dropped, and so is the socket.
fn gate_ping() -> String {
    json!({ "time": now_ms() / 1000, "channel": "futures.ping" }).to_string()
}

fn htx_subscribe(tickers: &[String], _: &Lookup) -> Vec<String> {
    tickers.iter().map(|t| json!({ "op": "sub", "topic": format!("public.{}-USDT.liquidation_orders", coin_of(t)) }).to_string()).collect()
}

/// HTX pings with `{"op":"ping","ts":…}` and closes a socket that does not answer.
fn htx_reply(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    (v["op"] == "ping").then(|| json!({ "op": "pong", "ts": v["ts"] }).to_string())
}

/// BitMEX's liquidation table covers every instrument.
fn bitmex_subscribe(_: &[String], _: &Lookup) -> Vec<String> {
    vec![json!({ "op": "subscribe", "args": ["liquidation"] }).to_string()]
}

fn bitfinex_subscribe(_: &[String], _: &Lookup) -> Vec<String> {
    vec![json!({ "event": "subscribe", "channel": "status", "key": "liq:global" }).to_string()]
}

fn lighter_subscribe(tickers: &[String], lookup: &Lookup) -> Vec<String> {
    tickers.iter().filter_map(|t| lookup.markets.get(coin_of(t))).map(|id| json!({ "type": "subscribe", "channel": format!("trade/{id}") }).to_string()).collect()
}

fn deribit_instruments(tickers: &[String]) -> Vec<String> {
    tickers.iter().map(|t| coin_of(t)).filter(|c| matches!(*c, "BTC" | "ETH")).map(|c| format!("trades.{c}-PERPETUAL.100ms")).collect()
}

/// Deribit drops a socket that does not answer its heartbeat's test requests.
fn deribit_subscribe(tickers: &[String], _: &Lookup) -> Vec<String> {
    vec![
        json!({ "jsonrpc": "2.0", "id": 1, "method": "public/set_heartbeat", "params": { "interval": 30 } }).to_string(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "public/subscribe", "params": { "channels": deribit_instruments(tickers) } }).to_string(),
    ]
}

fn deribit_reply(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    (v["method"] == "heartbeat" && v["params"]["type"] == "test_request").then(|| json!({ "jsonrpc": "2.0", "id": 3, "method": "public/test", "params": {} }).to_string())
}

/// Kraken spells bitcoin XBT.
fn kraken_product(coin: &str) -> String {
    format!("PF_{}USD", if coin == "BTC" { "XBT" } else { coin })
}

fn kraken_subscribe(tickers: &[String], _: &Lookup) -> Vec<String> {
    let products: Vec<String> = tickers.iter().map(|t| kraken_product(coin_of(t))).collect();
    vec![json!({ "event": "subscribe", "feed": "trade", "product_ids": products }).to_string(), json!({ "event": "subscribe", "feed": "heartbeat" }).to_string()]
}

/// Exchanges send numbers as strings; a number is taken as well.
fn num(v: &Value) -> Option<f64> {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()).filter(|n: &f64| n.is_finite())
}

/// Binance's `forceOrder`: one order a message. `p` is the limit the order was
/// pushed through the book at, a percent or more off the market; `ap` is what
/// it filled at, and is the price kept. A forced SELL closes a long.
pub fn parse_binance(text: &str) -> Vec<Hit> {
    parse_force_order(text, "binance-perp")
}

fn parse_binance_venue(text: &str, _: &Lookup) -> Vec<Hit> {
    parse_binance(text)
}

/// Aster is a Binance clone down to the stream.
fn parse_aster(text: &str, _: &Lookup) -> Vec<Hit> {
    parse_force_order(text, "aster-perp")
}

fn parse_force_order(text: &str, venue: &'static str) -> Vec<Hit> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    if v["e"] != "forceOrder" {
        return Vec::new();
    }
    let o = &v["o"];
    let price = num(&o["ap"]).filter(|p| *p > 0.0).or_else(|| num(&o["p"]));
    match (o["s"].as_str(), o["T"].as_i64(), price, num(&o["q"])) {
        (Some(symbol), Some(time), Some(price), Some(qty)) if price > 0.0 && qty > 0.0 => {
            vec![Hit { venue, symbol: symbol.to_string(), time, price, qty, long: o["S"] == "SELL" }]
        }
        _ => Vec::new(),
    }
}

/// Bybit's `allLiquidation`: a batch a message. `S` is the side of the position
/// that was closed, so `Buy` is a long.
fn parse_bybit_venue(text: &str, _: &Lookup) -> Vec<Hit> {
    parse_bybit(text)
}

pub fn parse_bybit(text: &str) -> Vec<Hit> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    if !v["topic"].as_str().is_some_and(|t| t.starts_with("allLiquidation.")) {
        return Vec::new();
    }
    let Some(rows) = v["data"].as_array() else { return Vec::new() };
    rows.iter()
        .filter_map(|l| match (l["s"].as_str(), l["T"].as_i64(), num(&l["p"]), num(&l["v"])) {
            (Some(symbol), Some(time), Some(price), Some(qty)) if price > 0.0 && qty > 0.0 => {
                Some(Hit { venue: "bybit-perp", symbol: symbol.to_string(), time, price, qty, long: l["S"] == "Buy" })
            }
            _ => None,
        })
        .collect()
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// A hit, when it has a price and a size.
fn hit(venue: &'static str, symbol: impl Into<String>, time: Option<i64>, price: Option<f64>, qty: Option<f64>, long: bool) -> Option<Hit> {
    match (time, price, qty) {
        (Some(time), Some(price), Some(qty)) if price > 0.0 && qty > 0.0 => Some(Hit { venue, symbol: symbol.into(), time, price, qty, long }),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i64> {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_i64()).or_else(|| v.as_f64().map(|f| f as i64))
}

/// OKX's `liquidation-orders`: every swap, sizes in contracts. A `sell` closes
/// a long; `bkPx` is the bankruptcy price. Only USDT swaps, named by coin.
pub fn parse_okx(text: &str, lookup: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["arg"]["channel"] != "liquidation-orders" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for pair in v["data"].as_array().into_iter().flatten() {
        let Some(inst) = pair["instId"].as_str() else { continue };
        let (Some(coin), Some(&size)) = (inst.strip_suffix("-USDT-SWAP"), lookup.sizes.get(inst)) else { continue };
        for d in pair["details"].as_array().into_iter().flatten() {
            out.extend(hit("okx-perp", coin, int(&d["ts"]), num(&d["bkPx"]), num(&d["sz"]).map(|c| c * size), d["side"] == "sell"));
        }
    }
    out
}

/// Bitget's `liquidation`: every USDT perpetual; `amount` is in USDT and a
/// `buy` closes a long.
pub fn parse_bitget(text: &str, _: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["arg"]["topic"] != "liquidation" {
        return Vec::new();
    }
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let price = num(&d["price"])?;
            hit("bitget-perp", d["symbol"].as_str()?, int(&d["ts"]), Some(price), num(&d["amount"]).map(|usd| usd / price), d["side"] == "buy")
        })
        .collect()
}

/// Gate's `futures.public_liquidates`: `size` is signed contracts — negative
/// is a forced sell, closing a long.
pub fn parse_gate(text: &str, lookup: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["channel"] != "futures.public_liquidates" || v["event"] != "update" {
        return Vec::new();
    }
    let items: Vec<&Value> = match &v["result"] {
        Value::Array(a) => a.iter().collect(),
        one @ Value::Object(_) => vec![one],
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|d| {
            let contract = d["contract"].as_str()?;
            let coin = contract.strip_suffix("_USDT")?;
            let size = num(&d["size"])?;
            let per = lookup.sizes.get(contract)?;
            // `time_ms` when present; `time` is seconds on some pushes and milliseconds on others.
            let time = int(&d["time_ms"]).or_else(|| int(&d["time"]).map(|t| if t >= 1_000_000_000_000 { t } else { t * 1000 }));
            hit("gate-perp", coin, time, num(&d["price"]), Some(size.abs() * per), size < 0.0)
        })
        .collect()
}

/// HTX's liquidation notification: `amount` in coins, a `sell` closes a long.
pub fn parse_htx(text: &str, _: &Lookup) -> Vec<Hit> {
    let v = json(text);
    let Some(coin) = v["topic"].as_str().and_then(|t| t.strip_prefix("public.")).and_then(|t| t.strip_suffix("-USDT.liquidation_orders")) else { return Vec::new() };
    v["data"].as_array().into_iter().flatten().filter_map(|d| hit("htx-perp", coin, int(&d["created_at"]), num(&d["price"]), num(&d["amount"]), d["direction"] == "sell")).collect()
}

/// BitMEX's liquidation table. XBTUSD is inverse — a contract is a dollar — so
/// its size is converted to coins; the other contracts are left out. It sends
/// no time, so the hit is stamped on arrival.
pub fn parse_bitmex(text: &str, _: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["table"] != "liquidation" || v["action"] != "insert" {
        return Vec::new();
    }
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["symbol"] == "XBTUSD")
        .filter_map(|d| {
            let price = num(&d["price"])?;
            hit("bitmex-perp", "BTC", Some(now_ms()), Some(price), num(&d["leavesQty"]).map(|usd| usd / price), d["side"] == "Sell")
        })
        .collect()
}

/// Bitfinex's global liquidation feed: `[chan, ["pos", id, mts, _, symbol,
/// amount, basePrice, _, isMatch, isMarketSold, _, priceAcquired]]`, or a
/// list of those. Only a position's first announcement counts; the snapshot
/// sent on subscribing is older than a minute and left out. A positive amount
/// was a long. Perpetuals are `tBTCF0:USTF0`.
pub fn parse_bitfinex(text: &str, _: &Lookup) -> Vec<Hit> {
    let v = json(text);
    let Some(payload) = v.get(1) else { return Vec::new() };
    let rows: Vec<&Value> = match payload.get(0) {
        Some(Value::Array(_)) => payload.as_array().into_iter().flatten().collect(),
        Some(Value::String(_)) => vec![payload],
        _ => Vec::new(),
    };
    let now = now_ms();
    rows.into_iter()
        .filter(|a| a[0] == "pos" && !truthy(&a[8]) && !truthy(&a[10]) && !truthy(&a[11]))
        .filter_map(|a| {
            let coin = a[4].as_str()?.strip_prefix('t')?.strip_suffix("F0:USTF0")?;
            let amount = num(&a[5])?;
            let time = int(&a[2]).filter(|t| now - t < 60_000);
            hit("bitfinex-perp", coin, time, num(&a[6]), Some(amount.abs()), amount > 0.0)
        })
        .collect()
}

fn truthy(v: &Value) -> bool {
    !(v.is_null() || v == &json!(0) || v == &json!(false))
}

/// Lighter's trade channel carries `liquidation_trades` apart from the rest.
/// The liquidated account is the taker: a position that was short before is a
/// short closed out.
pub fn parse_lighter(text: &str, lookup: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["type"] != "update/trade" {
        return Vec::new();
    }
    v["liquidation_trades"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = t["market_id"].as_u64()? as u32;
            let coin = lookup.markets.iter().find(|(_, m)| **m == id)?.0.clone();
            // Milliseconds, or seconds on older pushes.
            let time = int(&t["timestamp"]).map(|ts| if ts >= 1_000_000_000_000 { ts } else { ts * 1000 });
            hit("lighter-perp", coin, time, num(&t["price"]), num(&t["size"]), num(&t["taker_position_size_before"]).is_some_and(|p| p > 0.0))
        })
        .collect()
}

/// Deribit marks a liquidation on the trade itself: `T` when the taker was
/// liquidated, `M` the maker. The perpetual is inverse — `amount` is dollars.
/// A taker selling, or a maker on the buy side of a sell, was long.
pub fn parse_deribit(text: &str, _: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["method"] != "subscription" {
        return Vec::new();
    }
    v["params"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let liquidated = t["liquidation"].as_str()?;
            let coin = t["instrument_name"].as_str()?.strip_suffix("-PERPETUAL")?;
            let price = num(&t["price"])?;
            let sold = t["direction"] == "sell";
            let long = if liquidated.contains('T') { sold } else { !sold };
            hit("deribit-perp", coin, int(&t["timestamp"]), Some(price), num(&t["amount"]).map(|usd| usd / price), long)
        })
        .collect()
}

/// Kraken's trade feed types each trade; a `liquidation` is the forced order
/// taking liquidity, so a `sell` closes a long. Sizes are in coins.
pub fn parse_kraken(text: &str, _: &Lookup) -> Vec<Hit> {
    let v = json(text);
    if v["feed"] != "trade" || v["type"] != "liquidation" {
        return Vec::new();
    }
    let Some(coin) = v["product_id"].as_str().and_then(|p| p.strip_prefix("PF_")).and_then(|p| p.strip_suffix("USD")) else { return Vec::new() };
    let coin = if coin == "XBT" { "BTC" } else { coin };
    hit("kraken-perp", coin, int(&v["time"]), num(&v["price"]), num(&v["qty"]), v["side"] == "sell").into_iter().collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueState {
    pub venue: &'static str,
    /// `connecting`, `connected` or `retrying`.
    pub state: &'static str,
    /// When it entered that state, epoch milliseconds.
    pub since: i64,
    pub error: Option<String>,
}

/// The running collection: what it follows, since when, and how each exchange is.
pub struct Collector {
    pub tickers: Vec<String>,
    pub started: i64,
    states: Mutex<Vec<VenueState>>,
    /// Exchange names looked up over HTTPS, past networks that block them.
    doh: Option<Arc<crate::dns::Doh>>,
    /// For the contract sizes some exchanges need looked up first.
    http: Option<reqwest::Client>,
    /// Every hit as it arrives, for the live feed.
    pub live: broadcast::Sender<(String, Hit)>,
}

impl Collector {
    /// Starts following `tickers` on every exchange. Returns at once; the work
    /// runs on the runtime for as long as the server does.
    pub fn start(pool: deadpool_postgres::Pool, tickers: Vec<String>, doh: Arc<crate::dns::Doh>) -> Arc<Self> {
        let started = now_ms();
        let states = VENUES.iter().map(|v| VenueState { venue: v.id, state: "connecting", since: started, error: None }).collect();
        let http = reqwest::Client::builder().dns_resolver(doh.resolver()).timeout(Duration::from_secs(20)).build().ok();
        let collector = Arc::new(Self { tickers, started, states: Mutex::new(states), doh: Some(doh), http, live: broadcast::channel(1024).0 });
        let (tx, rx) = mpsc::channel(8_192);
        tokio::spawn(write(pool, rx));
        for venue in VENUES {
            tokio::spawn(follow(venue, venue.url.to_string(), collector.clone(), tx.clone()));
        }
        tracing::info!(tickers = ?collector.tickers, "collecting liquidations");
        collector
    }

    pub fn states(&self) -> Vec<VenueState> {
        self.states.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set(&self, venue: &str, state: &'static str, error: Option<String>) {
        let Ok(mut states) = self.states.lock() else { return };
        if let Some(s) = states.iter_mut().find(|s| s.venue == venue) {
            if s.state != state {
                s.since = now_ms();
            }
            s.state = state;
            s.error = error;
        }
    }
}

type Tx = mpsc::Sender<(String, Hit)>;

/// Keeps one exchange connected for as long as the writer is there to take hits.
async fn follow(venue: Venue, url: String, collector: Arc<Collector>, tx: Tx) {
    let mut attempt = 0;
    while !tx.is_closed() {
        collector.set(venue.id, "connecting", None);
        let result = session(&venue, &url, &collector, &tx).await;
        let error = match result {
            Ok(true) => {
                attempt = 0;
                "the exchange closed the connection".to_string()
            }
            Ok(false) => return,
            Err(e) => e.to_string(),
        };
        tracing::warn!(venue = venue.id, error = %error, "liquidation stream lost");
        collector.set(venue.id, "retrying", Some(error));
        sleep(Duration::from_secs(RETRY_SECS[attempt.min(RETRY_SECS.len() - 1)])).await;
        attempt += 1;
    }
}

/// One connection. `Ok(true)` when the exchange ended it and it is worth
/// reconnecting; `Ok(false)` when the writer has gone and so should we.
async fn session(venue: &Venue, url: &str, collector: &Collector, tx: &Tx) -> anyhow::Result<bool> {
    let connect = async {
        match &collector.doh {
            Some(doh) => doh.connect_ws(url).await,
            None => Ok(tokio_tungstenite::connect_async(url).await?.0),
        }
    };
    let lookup = prepare(venue.prepare, collector).await?;
    let mut ws = tokio::time::timeout(Duration::from_secs(15), connect).await.map_err(|_| anyhow::anyhow!("connecting timed out"))??;
    for frame in (venue.subscribe)(&collector.tickers, &lookup) {
        ws.send(Message::text(frame)).await?;
    }
    collector.set(venue.id, "connected", None);

    let mut heard = Instant::now();
    let mut ping = venue.ping.map(|(_, every)| tokio::time::interval(every));
    loop {
        tokio::select! {
            frame = ws.next() => {
                let Some(frame) = frame else { return Ok(true) };
                heard = Instant::now();
                let text = match frame? {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) if venue.gzip => match tradoor_codec::partition::gunzip(&bytes) {
                        Ok(raw) => String::from_utf8_lossy(&raw).into_owned(),
                        Err(_) => continue,
                    },
                    Message::Close(_) => return Ok(true),
                    _ => continue,
                };
                if let Some(answer) = (venue.reply)(&text) {
                    ws.send(Message::text(answer)).await?;
                }
                for hit in (venue.parse)(&text, &lookup) {
                    // Spelled as asked for, or by coin where one feed carries every market.
                    let Some(ticker) = collector.tickers.iter().find(|t| **t == hit.symbol || coin_of(t) == hit.symbol) else { continue };
                    let _ = collector.live.send((ticker.clone(), hit.clone()));
                    if tx.send((ticker.clone(), hit)).await.is_err() {
                        return Ok(false);
                    }
                }
            }
            () = tick(&mut ping) => {
                if let Some((frame, _)) = venue.ping {
                    ws.send(Message::text(frame())).await?;
                }
            }
            () = sleep_until(heard + venue.silence) => {
                anyhow::bail!("nothing heard for {} seconds", venue.silence.as_secs());
            }
        }
    }
}

/// Contract sizes and market numbers, fetched before subscribing — fresh on
/// every connection, so a new listing is picked up on the next reconnect.
async fn prepare(what: Prepare, collector: &Collector) -> anyhow::Result<Lookup> {
    let mut lookup = Lookup::default();
    let Some(http) = collector.http.as_ref().filter(|_| what != Prepare::Nothing) else { return Ok(lookup) };
    let get = |url: &'static str| async move { anyhow::Ok(serde_json::from_str::<Value>(&http.get(url).send().await?.error_for_status()?.text().await?)?) };
    match what {
        Prepare::OkxSizes => lookup.sizes = parse_okx_sizes(&get("https://www.okx.com/api/v5/public/instruments?instType=SWAP").await?),
        Prepare::GateSizes => lookup.sizes = parse_gate_sizes(&get("https://api.gateio.ws/api/v4/futures/usdt/contracts").await?),
        Prepare::LighterMarkets => lookup.markets = parse_lighter_markets(&get("https://mainnet.zklighter.elliot.ai/api/v1/orderBooks").await?),
        Prepare::Nothing => {}
    }
    Ok(lookup)
}

/// OKX linear swaps' `ctVal`, the coins in a contract.
pub fn parse_okx_sizes(v: &Value) -> HashMap<String, f64> {
    v["data"].as_array().into_iter().flatten().filter(|i| i["ctType"] == "linear").filter_map(|i| Some((i["instId"].as_str()?.to_string(), num(&i["ctVal"])?))).collect()
}

/// Gate's `quanto_multiplier`, the coins in a contract.
pub fn parse_gate_sizes(v: &Value) -> HashMap<String, f64> {
    v.as_array().into_iter().flatten().filter_map(|c| Some((c["name"].as_str()?.to_string(), num(&c["quanto_multiplier"])?))).collect()
}

/// Lighter's perpetual markets by coin.
pub fn parse_lighter_markets(v: &Value) -> HashMap<String, u32> {
    v["order_books"].as_array().into_iter().flatten().filter(|o| o["market_type"] == "perp").filter_map(|o| Some((o["symbol"].as_str()?.to_string(), o["market_id"].as_u64()? as u32))).collect()
}

async fn tick(interval: &mut Option<tokio::time::Interval>) {
    match interval {
        Some(i) => {
            i.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// The one writer: hits in batches of up to a second, and a trim to 24 hours
/// every minute.
async fn write(pool: deadpool_postgres::Pool, mut rx: mpsc::Receiver<(String, Hit)>) {
    let mut batch = Vec::with_capacity(BATCH);
    let mut flush = tokio::time::interval(Duration::from_secs(1));
    let mut trim = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            got = rx.recv() => match got {
                Some(hit) => {
                    batch.push(hit);
                    if batch.len() >= BATCH {
                        insert(&pool, &mut batch).await;
                    }
                }
                None => {
                    insert(&pool, &mut batch).await;
                    return;
                }
            },
            _ = flush.tick() => insert(&pool, &mut batch).await,
            _ = trim.tick() => {
                if let Err(e) = trim_to_a_day(&pool).await {
                    tracing::error!(error = %e, "trimming liquidations");
                }
            }
        }
    }
}

/// Writes the batch in one statement. A failed batch is logged and dropped:
/// holding it while the database is away would only grow without bound.
async fn insert(pool: &deadpool_postgres::Pool, batch: &mut Vec<(String, Hit)>) {
    if batch.is_empty() {
        return;
    }
    let rows = std::mem::take(batch);
    let result = async {
        let client = pool.get().await.map_err(|e| e.to_string())?;
        let tickers: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
        let venues: Vec<&str> = rows.iter().map(|(_, h)| h.venue).collect();
        let symbols: Vec<&str> = rows.iter().map(|(_, h)| h.symbol.as_str()).collect();
        let times: Vec<i64> = rows.iter().map(|(_, h)| h.time).collect();
        let prices: Vec<f64> = rows.iter().map(|(_, h)| h.price).collect();
        let qtys: Vec<f64> = rows.iter().map(|(_, h)| h.qty).collect();
        let usds: Vec<f64> = rows.iter().map(|(_, h)| h.price * h.qty).collect();
        let longs: Vec<bool> = rows.iter().map(|(_, h)| h.long).collect();
        client
            .execute(
                "INSERT INTO liquidation (ticker, venue, symbol, time, price, qty, usd, long)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::int8[], $5::float8[], $6::float8[], $7::float8[], $8::bool[])",
                &[&tickers, &venues, &symbols, &times, &prices, &qtys, &usds, &longs],
            )
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, rows = rows.len(), "writing liquidations");
    }
}

async fn trim_to_a_day(pool: &deadpool_postgres::Pool) -> Result<(), String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let gone = client.execute("DELETE FROM liquidation WHERE time < $1", &[&(now_ms() - DAY_MS)]).await.map_err(|e| e.to_string())?;
    if gone > 0 {
        tracing::debug!(rows = gone, "trimmed liquidations older than a day");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowTotal {
    pub window: String,
    pub long_usd: f64,
    pub short_usd: f64,
    pub longs: i64,
    pub shorts: i64,
}

/// A ticker's four totals as of `now`, every exchange together.
async fn totals(client: &tokio_postgres::Client, ticker: &str, now: i64) -> Result<Vec<WindowTotal>, tokio_postgres::Error> {
    let rows = client
        .query(
            "SELECT w.name,
                    COALESCE(SUM(l.usd) FILTER (WHERE l.long), 0),
                    COALESCE(SUM(l.usd) FILTER (WHERE NOT l.long), 0),
                    COUNT(l.id) FILTER (WHERE l.long),
                    COUNT(l.id) FILTER (WHERE NOT l.long)
             FROM UNNEST($2::text[], $3::int8[]) AS w(name, span)
             LEFT JOIN liquidation l ON l.ticker = $1 AND l.time >= $4 - w.span AND l.time <= $4
             GROUP BY w.name, w.span
             ORDER BY w.span",
            &[&ticker, &WINDOWS.map(|w| w.0).to_vec(), &WINDOWS.map(|w| w.1).to_vec(), &now],
        )
        .await?;
    Ok(rows.iter().map(|r| WindowTotal { window: r.get(0), long_usd: r.get(1), short_usd: r.get(2), longs: r.get(3), shorts: r.get(4) }).collect())
}

/// `GET /api/liquidations/{ticker}`: the 1H, 4H, 12H and 24H totals across
/// every exchange, how each exchange's connection is, and the latest hits.
pub async fn stats(State(state): State<AppState>, Path(ticker): Path<String>) -> ApiResult<Json<Value>> {
    if !is_ticker(&ticker) {
        return Err(ApiError::bad_request("a ticker is letters and digits, like BTCUSDT"));
    }
    let Some(collector) = state.liquidations.as_ref().filter(|c| c.tickers.contains(&ticker)) else {
        let tickers = state.liquidations.as_ref().map(|c| c.tickers.clone()).unwrap_or_default();
        return Ok(Json(json!({ "ticker": ticker, "tracked": false, "tickers": tickers })));
    };

    let now = now_ms();
    let client = state.pool.get().await?;
    let windows = totals(&client, &ticker, now).await?;
    let oldest: Option<i64> = client.query_one("SELECT MIN(time) FROM liquidation WHERE ticker = $1", &[&ticker]).await?.get(0);
    let recent = client.query("SELECT venue, time, price, qty, usd, long FROM liquidation WHERE ticker = $1 ORDER BY time DESC, id DESC LIMIT $2", &[&ticker, &RECENT_ROWS]).await?;

    Ok(Json(json!({
        "ticker": ticker,
        "tracked": true,
        "now": now,
        // Hits are known from here on: the older of the first row kept and the
        // moment this server started listening.
        "coverageFrom": oldest.map_or(collector.started, |o| o.min(collector.started)),
        "windows": windows,
        "venues": collector.states(),
        "recent": recent.iter().map(|r| json!({
            "venue": r.get::<_, String>(0),
            "time": r.get::<_, i64>(1),
            "price": r.get::<_, f64>(2),
            "qty": r.get::<_, f64>(3),
            "usd": r.get::<_, f64>(4),
            "long": r.get::<_, bool>(5),
        })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/liquidations/{ticker}/live`: every hit on every exchange as it
/// arrives, as server-sent events — one `hit` event each, and a `venues` event
/// with the exchanges' connections every ten seconds.
pub async fn live(State(state): State<AppState>, headers: axum::http::HeaderMap, Path(ticker): Path<String>) -> Result<axum::response::Response, ApiError> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use axum::response::IntoResponse;
    let collector = state.liquidations.as_ref().filter(|c| c.tickers.contains(&ticker)).ok_or_else(|| ApiError::not_found(format!("{ticker} is not followed on this server")))?.clone();
    // A stream a chart, a handful an address; the place is freed when the stream ends.
    let held = match crate::limits::client(&headers) {
        Some(ip) => match state.limits.hold("live", &ip, crate::limits::LIVE_STREAMS) {
            Some(held) => Some(held),
            None => return Ok(crate::limits::too_many("live streams open")),
        },
        None => None,
    };
    let rx = collector.live.subscribe();
    let venues = tokio::time::interval(Duration::from_secs(10));
    let stream = futures_util::stream::unfold((rx, venues, collector, ticker, held), |(mut rx, mut venues, collector, ticker, held)| async move {
        loop {
            tokio::select! {
                got = rx.recv() => match got {
                    Ok((t, hit)) if t == ticker => {
                        let data = json!({ "venue": hit.venue, "time": hit.time, "price": hit.price, "qty": hit.qty, "usd": hit.price * hit.qty, "long": hit.long });
                        return Some((Ok::<Event, std::convert::Infallible>(Event::default().event("hit").data(data.to_string())), (rx, venues, collector, ticker, held)));
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                _ = venues.tick() => {
                    let data = serde_json::to_string(&collector.states()).unwrap_or_default();
                    return Some((Ok(Event::default().event("venues").data(data)), (rx, venues, collector, ticker, held)));
                }
            }
        }
    });
    // nginx would otherwise hold the stream back in its buffer.
    Ok(([("x-accel-buffering", "no")], Sse::new(stream).keep_alive(KeepAlive::default())).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    // Captured from fstream.binance.com, 2026-09-16.
    const FORCE_ORDER: &str = r#"{"e":"forceOrder","E":1789531696996,"o":{"s":"BTCUSDT","S":"BUY","o":"LIMIT","f":"IOC","q":"0.706","p":"76781.0","ap":"75363.5","X":"FILLED","l":"0.193","z":"0.706","T":1789531695983}}"#;

    #[test]
    fn binance_keeps_the_fill_price_and_a_forced_sell_is_a_long() {
        let hits = parse_binance(FORCE_ORDER);
        assert_eq!(hits, [Hit { venue: "binance-perp", symbol: "BTCUSDT".into(), time: 1789531695983, price: 75363.5, qty: 0.706, long: false }]);
        let sell = FORCE_ORDER.replace(r#""S":"BUY""#, r#""S":"SELL""#);
        assert!(parse_binance(&sell)[0].long, "a forced sell closes a long");
        // An order not filled yet has no average price; its limit stands in.
        let unfilled = FORCE_ORDER.replace(r#""ap":"75363.5""#, r#""ap":"0""#);
        assert_eq!(parse_binance(&unfilled)[0].price, 76781.0);
        assert!(parse_binance(r#"{"result":null,"id":1}"#).is_empty(), "a subscribe answer is not a hit");
    }

    #[test]
    fn bybit_reads_a_batch_and_buy_is_a_long() {
        let hits = parse_bybit(r#"{"topic":"allLiquidation.ETHUSDT","type":"snapshot","ts":1789532479754,"data":[{"T":1789532479393,"s":"ETHUSDT","S":"Buy","v":"8.5","p":"2401.3"},{"T":1789532479400,"s":"ETHUSDT","S":"Sell","v":"0","p":"2401.0"},{"T":1789532479500,"s":"ETHUSDT","S":"Sell","v":"1.25","p":"2400.9"}]}"#);
        assert_eq!(hits.len(), 2, "a zero-size row is not a hit");
        assert!(hits[0].long && !hits[1].long);
        assert_eq!((hits[1].price, hits[1].qty, hits[1].time), (2400.9, 1.25, 1789532479500));
        assert!(parse_bybit(r#"{"success":true,"ret_msg":"pong","op":"ping"}"#).is_empty());
    }

    #[test]
    fn subscriptions_ask_for_every_ticker() {
        let tickers = ["BTCUSDT".to_string(), "SOLUSDT".to_string()];
        let none = Lookup::default();
        assert_eq!(binance_subscribe(&tickers, &none), [r#"{"id":1,"method":"SUBSCRIBE","params":["btcusdt@forceOrder","solusdt@forceOrder"]}"#]);
        assert_eq!(bybit_subscribe(&tickers, &none).len(), 2, "one Bybit topic a frame");
        assert!(bybit_subscribe(&tickers, &none)[1].contains("allLiquidation.SOLUSDT"));
    }

    // Captured 2026-09-23 unless marked otherwise.
    #[test]
    fn okx_counts_contracts_in_coins_and_names_the_coin() {
        let text = r#"{"arg":{"channel":"liquidation-orders","instType":"SWAP"},"data":[{"details":[{"bkLoss":"0","bkPx":"0.068886","ccy":"","posSide":"long","side":"sell","sz":"22","ts":"1790139669517"}],"instFamily":"MUBARAK-USDT","instId":"MUBARAK-USDT-SWAP","instType":"SWAP","uly":"MUBARAK-USDT"}]}"#;
        let lookup = Lookup { sizes: HashMap::from([("MUBARAK-USDT-SWAP".to_string(), 10.0)]), ..Lookup::default() };
        assert_eq!(parse_okx(text, &lookup), [Hit { venue: "okx-perp", symbol: "MUBARAK".into(), time: 1790139669517, price: 0.068886, qty: 220.0, long: true }]);
        assert!(parse_okx(text, &Lookup::default()).is_empty(), "no contract size, no hit");
        let sizes = parse_okx_sizes(&serde_json::from_str(r#"{"data":[{"instId":"BTC-USDT-SWAP","ctType":"linear","ctVal":"0.01"},{"instId":"BTC-USD-SWAP","ctType":"inverse","ctVal":"100"}]}"#).unwrap());
        assert_eq!(sizes, HashMap::from([("BTC-USDT-SWAP".to_string(), 0.01)]));
    }

    #[test]
    fn bitget_amount_is_dollars() {
        let hits = parse_bitget(r#"{"action":"update","arg":{"instType":"usdt-futures","topic":"liquidation"},"data":[{"symbol":"XAGUSDT","side":"buy","price":"66.04535931559","amount":"198.13607794677","ts":"1790141913473"}],"ts":1790141914550}"#, &Lookup::default());
        assert_eq!((hits[0].symbol.as_str(), hits[0].long, hits[0].time), ("XAGUSDT", true, 1790141913473));
        assert!((hits[0].qty * hits[0].price - 198.136).abs() < 0.001);
        assert!(parse_bitget("pong", &Lookup::default()).is_empty());
    }

    #[test]
    fn gate_sizes_are_signed_contracts() {
        // Shape from Gate's documentation.
        let lookup = Lookup { sizes: HashMap::from([("BTC_USDT".to_string(), 0.0001)]), ..Lookup::default() };
        let hits = parse_gate(r#"{"channel":"futures.public_liquidates","event":"update","time":1790141905,"result":[{"price":86800.1,"size":-1200,"time_ms":1790141905119,"contract":"BTC_USDT"}]}"#, &lookup);
        assert_eq!((hits[0].symbol.as_str(), hits[0].time, hits[0].price, hits[0].long), ("BTC", 1790141905119, 86800.1, true));
        assert!((hits[0].qty - 0.12).abs() < 1e-12);
        assert!(gate_ping().contains("futures.ping") && gate_ping().contains("time"));
    }

    #[test]
    fn htx_answers_its_ping_and_reads_coins() {
        assert_eq!(htx_reply(r#"{"op":"ping","ts":"1790141909985"}"#).as_deref(), Some(r#"{"op":"pong","ts":"1790141909985"}"#));
        assert_eq!(htx_reply(r#"{"op":"notify"}"#), None);
        // Shape from HTX's documentation.
        let hits = parse_htx(r#"{"op":"notify","topic":"public.BTC-USDT.liquidation_orders","ts":1790141909985,"data":[{"contract_code":"BTC-USDT","symbol":"BTC","direction":"sell","offset":"close","volume":12,"amount":0.012,"trade_turnover":1041.6,"price":86800,"created_at":1790141909000}]}"#, &Lookup::default());
        assert_eq!(hits, [Hit { venue: "htx-perp", symbol: "BTC".into(), time: 1790141909000, price: 86800.0, qty: 0.012, long: true }]);
    }

    #[test]
    fn bitmex_counts_dollars_on_the_inverse_contract() {
        let hits = parse_bitmex(r#"{"table":"liquidation","action":"insert","data":[{"orderID":"a","symbol":"XBTUSD","side":"Buy","price":86800,"leavesQty":8680},{"orderID":"b","symbol":"ETHUSD","side":"Sell","price":2700,"leavesQty":5}]}"#, &Lookup::default());
        assert_eq!(hits.len(), 1, "only the inverse bitcoin contract");
        assert_eq!((hits[0].symbol.as_str(), hits[0].qty, hits[0].long), ("BTC", 0.1, false));
        assert!(parse_bitmex(r#"{"table":"liquidation","action":"partial","data":[]}"#, &Lookup::default()).is_empty());
    }

    #[test]
    fn bitfinex_skips_the_old_snapshot_and_repeats() {
        let old = r#"[33170,[["pos",194168355,1790140803684,null,"tARBF0:USTF0",-550,0.243825454545,null,0,1,null,null]]]"#;
        assert!(parse_bitfinex(old, &Lookup::default()).is_empty(), "the snapshot is older than a minute");
        let now = now_ms();
        let fresh = format!(r#"[33170,["pos",194168356,{now},null,"tBTCF0:USTF0",0.5,86800,null,0,1,null,null]]"#);
        let hits = parse_bitfinex(&fresh, &Lookup::default());
        assert_eq!((hits[0].symbol.as_str(), hits[0].qty, hits[0].long), ("BTC", 0.5, true));
        let matched = format!(r#"[33170,["pos",194168356,{now},null,"tBTCF0:USTF0",0.5,86800,null,1,1,null,86790]]"#);
        assert!(parse_bitfinex(&matched, &Lookup::default()).is_empty(), "the follow-up once it is matched");
        assert!(parse_bitfinex(r#"[33170,"hb"]"#, &Lookup::default()).is_empty());
    }

    #[test]
    fn lighter_reads_the_taker_position_before() {
        let text = r#"{"channel":"trade:1","liquidation_trades":[{"trade_id":31627652389,"type":"liquidation","market_id":1,"market_kind":"perps","size":"0.00521","price":"87249.8","usd_amount":"454.571458","is_maker_ask":true,"timestamp":1790138063949,"taker_position_size_before":"-0.00521"}],"trades":[],"type":"update/trade"}"#;
        let lookup = Lookup { markets: HashMap::from([("BTC".to_string(), 1)]), ..Lookup::default() };
        assert_eq!(parse_lighter(text, &lookup), [Hit { venue: "lighter-perp", symbol: "BTC".into(), time: 1790138063949, price: 87249.8, qty: 0.00521, long: false }]);
        assert!(parse_lighter(&text.replace("update/trade", "subscribed/trade"), &lookup).is_empty(), "the snapshot on subscribing");
        assert_eq!(lighter_subscribe(&["BTCUSDT".to_string(), "DOGEUSDT".to_string()], &lookup), [r#"{"channel":"trade/1","type":"subscribe"}"#]);
    }

    #[test]
    fn deribit_marks_the_liquidated_side() {
        let trade = |liq: &str, dir: &str| format!(r#"{{"jsonrpc":"2.0","method":"subscription","params":{{"channel":"trades.BTC-PERPETUAL.100ms","data":[{{"timestamp":1790141905728,"price":86800.0,"amount":8680.0,"direction":"{dir}","instrument_name":"BTC-PERPETUAL","liquidation":"{liq}"}}]}}}}"#);
        let taker_sold = parse_deribit(&trade("T", "sell"), &Lookup::default());
        assert_eq!((taker_sold[0].symbol.as_str(), taker_sold[0].qty, taker_sold[0].long), ("BTC", 0.1, true));
        assert!(!parse_deribit(&trade("M", "sell"), &Lookup::default())[0].long, "the maker bought: a short closed out");
        let plain = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.BTC-PERPETUAL.100ms","data":[{"timestamp":1790141905728,"price":86819.0,"amount":10.0,"direction":"sell","instrument_name":"BTC-PERPETUAL"}]}}"#;
        assert!(parse_deribit(plain, &Lookup::default()).is_empty(), "an ordinary trade");
        assert!(deribit_reply(r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"type":"test_request"}}"#).is_some());
    }

    #[test]
    fn kraken_keeps_only_liquidation_trades() {
        let fill = r#"{"feed":"trade","product_id":"PF_XBTUSD","uid":"x","side":"sell","type":"fill","time":1790141853843,"qty":0.02,"price":86800,"seq":1}"#;
        assert!(parse_kraken(fill, &Lookup::default()).is_empty());
        let hits = parse_kraken(&fill.replace("\"fill\"", "\"liquidation\""), &Lookup::default());
        assert_eq!(hits, [Hit { venue: "kraken-perp", symbol: "BTC".into(), time: 1790141853843, price: 86800.0, qty: 0.02, long: true }]);
        assert!(parse_kraken(r#"{"feed":"trade_snapshot","product_id":"PF_SOLUSD","trades":[]}"#, &Lookup::default()).is_empty());
    }

    #[test]
    fn coins_of_tickers() {
        assert_eq!([coin_of("BTCUSDT"), coin_of("ETHUSDC"), coin_of("SOLUSD"), coin_of("USDT")], ["BTC", "ETH", "SOL", "USDT"]);
    }

    #[test]
    fn tickers_are_plain_symbols() {
        assert!(is_ticker("BTCUSDT") && is_ticker("1000PEPEUSDT"));
        assert!(!is_ticker("btcusdt") && !is_ticker("BTC-USDT") && !is_ticker("B") && !is_ticker("DROP TABLE"));
    }

    /// The write, the trim and the totals against a real database — the SQL and
    /// the array bindings are what this checks. Needs DATABASE_URL with the
    /// schema applied; run with `cargo test -p tradoor-server -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a Postgres at DATABASE_URL"]
    async fn hits_are_written_totalled_and_trimmed_to_a_day() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = deadpool_postgres::Config { url: Some(url), ..Default::default() }.create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls).unwrap();
        const T: &str = "ZZTESTUSDT";
        let client = pool.get().await.unwrap();
        client.execute("DELETE FROM liquidation WHERE ticker = $1", &[&T]).await.unwrap();

        let now = now_ms();
        let hit = |venue: &'static str, ago: i64, price: f64, qty: f64, long: bool| (T.to_string(), Hit { venue, symbol: T.into(), time: now - ago, price, qty, long });
        let mut batch = vec![
            hit("binance-perp", 10 * 60_000, 100.0, 2.0, true),     // $200 long, 10 minutes ago
            hit("bybit-perp", 2 * 3_600_000, 50.0, 3.0, false),     // $150 short, 2 hours ago
            hit("bybit-perp", 11 * 3_600_000, 10.0, 1.0, true),     // $10 long, 11 hours ago
            hit("binance-perp", 25 * 3_600_000, 1000.0, 1.0, true), // a day and an hour ago
        ];
        insert(&pool, &mut batch).await;
        assert!(batch.is_empty(), "the batch is taken");

        let before = totals(&client, T, now).await.unwrap();
        assert_eq!(before.iter().map(|w| w.window.as_str()).collect::<Vec<_>>(), ["1h", "4h", "12h", "24h"]);
        assert_eq!((before[0].long_usd, before[0].short_usd, before[0].longs), (200.0, 0.0, 1), "1h: the recent long only");
        assert_eq!((before[1].long_usd, before[1].short_usd, before[1].shorts), (200.0, 150.0, 1), "4h: both exchanges together");
        assert_eq!(before[2].long_usd, 210.0, "12h");
        assert_eq!(before[3].long_usd, 210.0, "24h leaves out the day-old hit");

        trim_to_a_day(&pool).await.unwrap();
        let left: i64 = client.query_one("SELECT COUNT(*) FROM liquidation WHERE ticker = $1", &[&T]).await.unwrap().get(0);
        client.execute("DELETE FROM liquidation WHERE ticker = $1", &[&T]).await.unwrap();
        assert_eq!(left, 3, "the trim removed only what is older than a day");
    }

    /// A whole session against a stand-in exchange: it subscribes, passes on a
    /// hit for a followed ticker, ignores one for another, and reports the
    /// exchange closing the socket.
    #[tokio::test]
    async fn a_session_subscribes_and_passes_hits_on() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let exchange = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let subscribe = ws.next().await.unwrap().unwrap().into_text().unwrap();
            ws.send(Message::text(r#"{"result":null,"id":1}"#)).await.unwrap();
            ws.send(Message::text(FORCE_ORDER.replace("BTCUSDT", "DOGEUSDT"))).await.unwrap();
            ws.send(Message::text(FORCE_ORDER)).await.unwrap();
            ws.close(None).await.unwrap();
            subscribe.to_string()
        });

        let collector = Collector {
            tickers: vec!["BTCUSDT".into()],
            started: now_ms(),
            states: Mutex::new(vec![VenueState { venue: "binance-perp", state: "connecting", since: 0, error: None }]),
            doh: None,
            http: None,
            live: broadcast::channel(8).0,
        };
        let (tx, mut rx) = mpsc::channel(8);
        let reconnect = session(&VENUES[0], &url, &collector, &tx).await.unwrap();

        assert!(reconnect, "the exchange closed it, so it is worth reconnecting");
        assert_eq!(exchange.await.unwrap(), r#"{"id":1,"method":"SUBSCRIBE","params":["btcusdt@forceOrder"]}"#);
        let (ticker, hit) = rx.try_recv().expect("the BTCUSDT hit came through");
        assert_eq!((ticker.as_str(), hit.price), ("BTCUSDT", 75363.5));
        assert!(rx.try_recv().is_err(), "DOGEUSDT is not followed");
        assert_eq!(collector.states()[0].state, "connected");
    }
}
