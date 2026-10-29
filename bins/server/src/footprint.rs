//! Volume by price inside each minute — the numbers a footprint chart is made
//! of: what the buyers took at each level, and what the sellers did.
//!
//! The same prints the flow collector reads, kept by price rather than summed:
//! a trade lands in the level its price falls in, and a minute is a column of
//! levels. A month of them is kept.
//!
//! The levels are packed into one blob a minute rather than stored a row each.
//! A minute of BTC spans some forty dollars, so a dollar level is forty rows a
//! minute — two million a month for one market, where the blob is twelve
//! megabytes for the same thing. The layout is plain: the lowest level of the
//! bar is the row's `base`, and the blob is, per level, the gap from the level
//! before it and the two volumes, all as varints of thousandths of a coin.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{sleep, sleep_until, Instant};
use tokio_tungstenite::tungstenite::Message;
use tradoor_codec::varint;
use tradoor_store::now_ms;
use tradoor_venues::{MarketRef, Venue};

use crate::api::{ApiError, ApiResult, AppState};
use crate::catalog::Catalog;
use crate::dns::Doh;
use crate::trades::{self, Print};

/// The markets recorded, and what one price level is worth on each. A level is
/// the finest a reader would ever want: a dollar on BTC is some forty levels a
/// minute, a tenth on ETH about twenty. Coarser groupings are made by adding
/// levels together, which cannot be undone — so the finest is what is kept.
const MARKETS: [(&str, f64); 8] = [
    ("binance-spot:BTCUSDT", 1.0),
    ("binance-perp:BTCUSDT", 1.0),
    ("bybit-spot:BTCUSDT", 1.0),
    ("bybit-perp:BTCUSDT", 1.0),
    ("binance-spot:ETHUSDT", 0.1),
    ("binance-perp:ETHUSDT", 0.1),
    ("bybit-spot:ETHUSDT", 0.1),
    ("bybit-perp:ETHUSDT", 0.1),
];
/// Minutes kept: a month, as the panel offers.
const KEPT_MS: i64 = 30 * 24 * 3_600_000;
/// How often finished minutes are written down.
const FLUSH: Duration = Duration::from_secs(10);
/// Volumes are kept as thousandths of a coin: a varint of an integer packs far
/// smaller than a float, and a thousandth of a coin is below any exchange's
/// smallest trade.
const SCALE: f64 = 1_000.0;
const RETRY_SECS: [u64; 5] = [1, 3, 5, 10, 30];
const SILENCE: Duration = Duration::from_secs(90);
const NO_TRADES: Duration = Duration::from_secs(150);
const PING: Duration = Duration::from_secs(15);
/// Minutes one request may ask for: a day of them at once, no more.
const MAX_MINUTES: i64 = 1_440;

/// The markets this server records, in the order the chart offers them.
pub fn markets() -> Vec<String> {
    MARKETS.iter().map(|(market, _)| (*market).to_string()).collect()
}

/// What one price level is worth on a market, or nothing where it is not recorded.
pub fn tick_of(market: &str) -> Option<f64> {
    MARKETS.iter().find(|(m, _)| *m == market).map(|(_, tick)| *tick)
}

/// One minute of one market — or an hour of it, kept as history: the levels it
/// traded at, and how many prints.
#[derive(Default)]
pub(crate) struct Bar {
    /// Level (price in ticks, rounded down) to the coins taken at it, each way.
    pub(crate) levels: BTreeMap<i64, (f64, f64)>,
    pub(crate) trades: i64,
}

impl Bar {
    /// The blob a row carries: the lowest level, then each level as the gap
    /// from the one before it and the two volumes.
    pub(crate) fn pack(&self) -> (i64, Vec<u8>) {
        let mut bytes = Vec::with_capacity(self.levels.len() * 4);
        let base = self.levels.keys().next().copied().unwrap_or(0);
        let mut last = base;
        for (level, (buy, sell)) in &self.levels {
            varint::put_u(&mut bytes, (level - last) as u64);
            varint::put_u(&mut bytes, (buy * SCALE).round().max(0.0) as u64);
            varint::put_u(&mut bytes, (sell * SCALE).round().max(0.0) as u64);
            last = *level;
        }
        (base, bytes)
    }
}

/// A packed minute read back: level, coins bought, coins sold.
pub fn unpack(base: i64, bytes: &[u8]) -> Vec<(i64, f64, f64)> {
    let mut out = Vec::new();
    let mut reader = varint::Reader::new(bytes);
    let mut level = base;
    while !reader.is_empty() {
        let (Some(gap), Some(buy), Some(sell)) = (reader.u(), reader.u(), reader.u()) else { break };
        level += gap as i64;
        out.push((level, buy as f64 / SCALE, sell as f64 / SCALE));
    }
    out
}

pub struct Footprint {
    pool: deadpool_postgres::Pool,
    doh: Arc<Doh>,
    catalog: Arc<Catalog>,
    /// Per market, the minutes not yet written down.
    open: Mutex<HashMap<String, BTreeMap<i64, Bar>>>,
}

impl Footprint {
    pub fn start(pool: deadpool_postgres::Pool, doh: Arc<Doh>, catalog: Arc<Catalog>) -> Arc<Self> {
        let it = Arc::new(Self { pool, doh, catalog, open: Mutex::new(HashMap::new()) });
        for market in markets() {
            tokio::spawn(follow(market, it.clone()));
        }
        tokio::spawn(keep(it.clone()));
        tracing::info!(markets = ?markets(), "recording footprint");
        it
    }

    /// One trade, counted to the level its price falls in.
    fn record(&self, market: &str, tick: f64, print: &Print<'_>, at: i64) {
        if !(print.price.is_finite() && print.price > 0.0 && print.qty.is_finite() && print.qty > 0.0) {
            return;
        }
        let Ok(mut open) = self.open.lock() else { return };
        let minute = at - at.rem_euclid(60_000);
        let bar = open.entry(market.to_string()).or_default().entry(minute).or_default();
        let level = (print.price / tick).floor() as i64;
        let at_level = bar.levels.entry(level).or_insert((0.0, 0.0));
        if print.buy {
            at_level.0 += print.qty;
        } else {
            at_level.1 += print.qty;
        }
        bar.trades += 1;
    }
}

/// Keeps one exchange connected for as long as the server runs.
async fn follow(market: String, it: Arc<Footprint>) {
    let Some(venue) = MarketRef::parse(&market).map(|m| m.venue) else { return };
    let urls = trades::stream_urls(venue);
    if urls.is_empty() {
        return;
    }
    let (mut attempt, mut at) = (0, 0);
    loop {
        let url = urls[at % urls.len()];
        if let Err(e) = session(&market, venue, url, &it).await {
            tracing::warn!(venue = venue.id(), url, error = %e, "footprint stream lost");
            sleep(Duration::from_secs(RETRY_SECS[attempt.min(RETRY_SECS.len() - 1)])).await;
            (attempt, at) = (attempt + 1, at + 1);
        } else {
            attempt = 0;
        }
    }
}

async fn session(market: &str, venue: Venue, url: &str, it: &Footprint) -> anyhow::Result<()> {
    let symbol = MarketRef::parse(market).map(|m| m.symbol).unwrap_or_default();
    let tick = tick_of(market).unwrap_or(1.0);
    let theirs = if trades::renames(venue) {
        it.catalog.native_names(venue, std::slice::from_ref(&symbol)).await.get(&symbol).cloned().unwrap_or_else(|| symbol.clone())
    } else {
        symbol.clone()
    };
    let connect = it.doh.connect_ws(url);
    let mut ws = tokio::time::timeout(Duration::from_secs(15), connect).await.map_err(|_| anyhow::anyhow!("connecting timed out"))??;
    for frame in trades::subscribe(venue, std::slice::from_ref(&theirs)) {
        ws.send(Message::text(frame)).await?;
    }
    tracing::info!(venue = venue.id(), symbol = %theirs, tick, "recording footprint");
    let (mut heard, mut traded) = (Instant::now(), Instant::now());
    let mut ping = tokio::time::interval(PING);
    loop {
        tokio::select! {
            frame = ws.next() => {
                let Some(frame) = frame else { return Ok(()) };
                heard = Instant::now();
                let text = match frame? {
                    Message::Text(text) => text.to_string(),
                    Message::Close(_) => return Ok(()),
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                let mut prints = Vec::new();
                trades::read_prints(venue, &value, &mut prints);
                let now = now_ms();
                for print in &prints {
                    if print.symbol != theirs {
                        continue;
                    }
                    traded = Instant::now();
                    it.record(market, tick, print, now);
                }
            }
            _ = ping.tick() => {
                if let Some(frame) = trades::ping(venue) {
                    ws.send(Message::text(frame)).await?;
                }
            }
            () = sleep_until(heard + SILENCE) => anyhow::bail!("nothing heard for {} seconds", SILENCE.as_secs()),
            () = sleep_until(traded + NO_TRADES) => anyhow::bail!("no trade in {} seconds", NO_TRADES.as_secs()),
        }
    }
}

/// Writes finished minutes down, and trims what is older than a month.
async fn keep(it: Arc<Footprint>) {
    let mut trimmed = Instant::now();
    loop {
        sleep(FLUSH).await;
        if let Err(e) = flush(&it).await {
            tracing::warn!(error = %e, "writing footprint minutes");
        }
        if trimmed.elapsed() >= Duration::from_secs(3_600) {
            trimmed = Instant::now();
            let cutoff = now_ms() - KEPT_MS;
            match it.pool.get().await {
                Ok(client) => {
                    if let Err(e) = client.execute("DELETE FROM footprint_minute WHERE minute < $1", &[&cutoff]).await {
                        tracing::warn!(error = %e, "trimming footprint minutes");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "trimming footprint minutes"),
            }
        }
    }
}

/// Every minute but the one still filling, written down and let go of.
async fn flush(it: &Footprint) -> anyhow::Result<()> {
    let now = now_ms();
    let filling = now - now.rem_euclid(60_000);
    let done: Vec<(String, i64, Bar)> = {
        let Ok(mut open) = it.open.lock() else { return Ok(()) };
        let mut out = Vec::new();
        for (market, bars) in open.iter_mut() {
            let finished: Vec<i64> = bars.keys().copied().filter(|minute| *minute < filling).collect();
            for minute in finished {
                if let Some(bar) = bars.remove(&minute) {
                    out.push((market.clone(), minute, bar));
                }
            }
        }
        out
    };
    if done.is_empty() {
        return Ok(());
    }
    let client = it.pool.get().await?;
    for (market, minute, bar) in &done {
        let (base, bytes) = bar.pack();
        let tick = tick_of(market).unwrap_or(1.0);
        client
            .execute(
                "INSERT INTO footprint_minute (market, minute, tick, base, levels, trades) VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (market, minute) DO UPDATE SET tick = EXCLUDED.tick, base = EXCLUDED.base, levels = EXCLUDED.levels, trades = EXCLUDED.trades",
                &[market, minute, &tick, &base, &bytes, &bar.trades],
            )
            .await?;
    }
    Ok(())
}

/// `GET /api/footprint/markets`: what this server records, and at what level.
pub async fn recorded() -> Json<Value> {
    Json(json!({
        "markets": MARKETS.iter().map(|(market, tick)| json!({ "market": market, "tick": tick })).collect::<Vec<_>>(),
        "keptDays": KEPT_MS / (24 * 3_600_000),
    }))
}

#[derive(Deserialize)]
pub struct Ask {
    market: String,
    from: i64,
    to: i64,
}

/// `GET /api/footprint?market=&from=&to=`: the minutes in that span, each as
/// its levels — price, coins bought, coins sold.
pub async fn bars(State(state): State<AppState>, Query(ask): Query<Ask>) -> ApiResult<Json<Value>> {
    let Some(tick) = tick_of(&ask.market) else {
        return Err(ApiError::bad_request("this server does not record that market's footprint"));
    };
    let (from, to) = (ask.from - ask.from.rem_euclid(60_000), ask.to);
    if to < from || (to - from) / 60_000 > MAX_MINUTES {
        return Err(ApiError::bad_request(format!("ask for at most {MAX_MINUTES} minutes at a time")));
    }
    let client = state.pool.get().await?;
    let rows = client
        .query(
            "SELECT minute, base, levels, trades FROM footprint_minute WHERE market = $1 AND minute >= $2 AND minute <= $3 ORDER BY minute",
            &[&ask.market, &from, &to],
        )
        .await?;
    let bars: Vec<Value> = rows
        .iter()
        .map(|r| {
            let levels = unpack(r.get::<_, i64>(1), r.get::<_, &[u8]>(2));
            json!({
                "at": r.get::<_, i64>(0),
                "trades": r.get::<_, i64>(3),
                // Price first, then what was bought and sold at it.
                "levels": levels.iter().map(|(level, buy, sell)| json!([*level as f64 * tick, buy, sell])).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(json!({ "market": ask.market, "tick": tick, "bars": bars })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print<'a>(symbol: &'a str, price: f64, qty: f64, buy: bool) -> Print<'a> {
        Print { symbol, price, qty, buy, at: 0 }
    }

    #[test]
    fn a_trade_lands_in_the_level_its_price_falls_in() {
        let it = Footprint {
            pool: deadpool_postgres::Config { url: Some("postgres://nobody@127.0.0.1:1/none".into()), ..Default::default() }
                .create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls)
                .unwrap(),
            doh: crate::dns::Doh::new().unwrap(),
            catalog: Arc::new(crate::catalog::Catalog::new("https://api.binance.com".into(), &crate::dns::Doh::new().unwrap()).unwrap()),
            open: Mutex::new(HashMap::new()),
        };
        let at = 1_800_000_000_000;
        it.record("binance-perp:BTCUSDT", 1.0, &print("BTCUSDT", 84_000.7, 0.5, true), at);
        it.record("binance-perp:BTCUSDT", 1.0, &print("BTCUSDT", 84_000.2, 0.25, false), at + 500);
        it.record("binance-perp:BTCUSDT", 1.0, &print("BTCUSDT", 84_003.0, 1.5, true), at + 1_000);
        // The next minute is a column of its own.
        it.record("binance-perp:BTCUSDT", 1.0, &print("BTCUSDT", 84_010.0, 2.0, true), at + 61_000);

        let open = it.open.lock().unwrap();
        let bars = open.get("binance-perp:BTCUSDT").unwrap();
        assert_eq!(bars.len(), 2, "two minutes, kept apart");
        let bar = bars.values().next().unwrap();
        assert_eq!(bar.trades, 3);
        assert_eq!(bar.levels.get(&84_000), Some(&(0.5, 0.25)), "both sides of the dollar they traded in");
        assert_eq!(bar.levels.get(&84_003), Some(&(1.5, 0.0)));

        // Packed and read back, it is the same column.
        let (base, bytes) = bar.pack();
        assert_eq!(base, 84_000);
        assert!(bytes.len() < 16, "two levels pack into a handful of bytes, not a row each: {}", bytes.len());
        let read = unpack(base, &bytes);
        assert_eq!(read, vec![(84_000, 0.5, 0.25), (84_003, 1.5, 0.0)]);
    }

    #[test]
    fn every_market_recorded_is_one_this_server_can_read() {
        assert_eq!(markets().len(), 8);
        assert!(markets().iter().all(|m| MarketRef::parse(m).is_some_and(|r| !trades::stream_urls(r.venue).is_empty())));
        assert_eq!(tick_of("binance-perp:BTCUSDT"), Some(1.0), "a dollar on BTC is some forty levels a minute");
        assert_eq!(tick_of("bybit-spot:ETHUSDT"), Some(0.1));
        assert_eq!(tick_of("okx-perp:BTCUSDT"), None, "and what is not recorded says so");
    }
}
