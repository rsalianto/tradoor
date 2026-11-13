//! Lighter's trades, for its footprint. Its stream gives each trade on a
//! `trade/{market}` channel (the server's other Lighter feed carries only the
//! last price), and its REST pages back through them a hundred at a time from
//! the newest — some ten seconds of BTC a page, with no way to start at a time.
//!
//! So the day before the stream began is read back once, in the background, a
//! page a second: only the trades older than the first one the stream gave, so
//! none is counted twice or missed, and only back to the newest minute already
//! kept, so a restart reads its own gap and not the day again.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::time::sleep;
use tradoor_store::now_ms;

use crate::footprint::{tick_of, unpack, Bar, Footprint};

const REST: &str = "https://mainnet.zklighter.elliot.ai/api/v1/trades";
/// How far back the first fill reads.
const FILL_MS: i64 = 24 * 3_600_000;
/// Between two pages: Lighter is asked gently, the relay's browsers ask it too.
const PACE: Duration = Duration::from_secs(1);
/// Pages between writing down what is done so far.
const WRITE_EVERY: usize = 60;
/// Failures in a row before the fill stops; it waits longer after each.
const TRIES: u32 = 8;

/// One trade: its id, price, coins, whether the taker bought, and when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trade {
    pub id: i64,
    pub price: f64,
    pub qty: f64,
    pub buy: bool,
    pub at: i64,
}

/// What asks the stream for a market's trades.
pub fn subscribe(market_id: &str) -> String {
    format!(r#"{{"type":"subscribe","channel":"trade/{market_id}"}}"#)
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

/// The trades in a list as either the stream or REST gives it. The maker was the ask: the taker bought.
pub fn trades(list: &Value) -> Vec<Trade> {
    let Some(list) = list.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|t| Some(Trade { id: t["trade_id"].as_i64()?, price: num(&t["price"])?, qty: num(&t["size"])?, buy: t["is_maker_ask"].as_bool()?, at: t["timestamp"].as_i64()? }))
        .filter(|t| t.price > 0.0 && t.qty > 0.0)
        .collect()
}

/// A stream frame's new trades. The snapshot sent on subscribing is older ones, left to the fill.
pub fn live(frame: &Value) -> Vec<Trade> {
    if frame["type"] == "update/trade" { trades(&frame["trades"]) } else { Vec::new() }
}

/// A page's trades older than `first` and not before `from`, added to their minutes.
/// True once the page reaches past `from`: nothing older is wanted.
fn take(bars: &mut BTreeMap<i64, Bar>, tick: f64, page: &[Trade], first: i64, from: i64) -> bool {
    let mut past = false;
    for t in page {
        if t.at < from {
            past = true;
            continue;
        }
        if t.id < first {
            bars.entry(t.at - t.at.rem_euclid(60_000)).or_default().add((t.price / tick).floor() as i64, t.qty, t.buy);
        }
    }
    past
}

/// Reads back what came before the stream, once. `first` is the stream's first trade.
pub async fn fill(it: Arc<Footprint>, market: String, market_id: String, first: Trade) {
    let Some(tick) = tick_of(&market) else { return };
    let start = first.at - first.at.rem_euclid(60_000);
    let held = match it.pool().get().await {
        Ok(client) => client
            .query_one("SELECT max(minute) FROM footprint_minute WHERE market = $1 AND minute < $2", &[&market, &start])
            .await
            .ok()
            .and_then(|r| r.get::<_, Option<i64>>(0)),
        Err(e) => {
            tracing::warn!(error = %e, "lighter footprint fill: the database");
            return;
        }
    };
    let from = held.map_or(start - FILL_MS, |m| m + 60_000).max(start - FILL_MS);
    if from >= start + 60_000 {
        return;
    }
    tracing::info!(market, minutes = (start - from) / 60_000, "lighter footprint fill: reading back");
    let http = reqwest::Client::builder().dns_resolver(it.doh().resolver()).timeout(Duration::from_secs(20)).build().unwrap_or_default();
    let (mut bars, mut cursor, mut pages, mut failed) = (BTreeMap::<i64, Bar>::new(), None::<String>, 0usize, 0u32);
    let reached = loop {
        sleep(PACE).await;
        let mut url = format!("{REST}?market_id={market_id}&sort_by=timestamp&limit=100");
        if let Some(c) = &cursor {
            url += "&cursor=";
            url += c;
        }
        let page: anyhow::Result<Value> = async { Ok(serde_json::from_slice(&http.get(&url).send().await?.error_for_status()?.bytes().await?)?) }.await;
        let page = match page {
            Ok(p) => {
                failed = 0;
                p
            }
            Err(e) => {
                failed += 1;
                if failed >= TRIES {
                    tracing::warn!(error = %e, pages, "lighter footprint fill: stopped");
                    break false;
                }
                sleep(Duration::from_secs(5 << failed.min(5))).await;
                continue;
            }
        };
        let list = trades(&page["trades"]);
        pages += 1;
        let past = take(&mut bars, tick, &list, first.id, from);
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if past || cursor.is_none() || list.is_empty() {
            break past;
        }
        // What is newer than the oldest minute seen so far is whole: written down as it goes.
        if pages.is_multiple_of(WRITE_EVERY) {
            let oldest = bars.keys().next().copied();
            let whole: Vec<i64> = bars.keys().copied().filter(|m| Some(*m) != oldest && *m != start).collect();
            let done: Vec<(i64, Bar)> = whole.into_iter().filter_map(|m| bars.remove(&m).map(|b| (m, b))).collect();
            if let Err(e) = write(&it, &market, tick, &done).await {
                tracing::warn!(error = %e, "lighter footprint fill: writing");
            }
        }
    };
    // Stopped short of `from`, the oldest minute was not read to its start.
    if let Some(oldest) = bars.keys().next().copied().filter(|_| !reached) {
        bars.remove(&oldest);
    }
    let begun = bars.remove(&start);
    let rest: Vec<(i64, Bar)> = std::mem::take(&mut bars).into_iter().collect();
    let minutes = rest.len();
    if let Err(e) = write(&it, &market, tick, &rest).await {
        tracing::warn!(error = %e, "lighter footprint fill: writing");
    }
    // The minute the stream began in holds its trades from then on: the rest of it joins them.
    if let Some(begun) = begun {
        while now_ms() < start + 60_000 + 30_000 {
            sleep(Duration::from_secs(5)).await;
        }
        if let Err(e) = join(&it, &market, tick, start, begun).await {
            tracing::warn!(error = %e, "lighter footprint fill: the first minute");
        }
    }
    tracing::info!(market, pages, minutes, reached, "lighter footprint fill: done");
}

/// Minutes no one has written yet: one the stream holds is left as it is.
async fn write(it: &Footprint, market: &str, tick: f64, bars: &[(i64, Bar)]) -> anyhow::Result<()> {
    if bars.is_empty() {
        return Ok(());
    }
    let client = it.pool().get().await?;
    for (minute, bar) in bars {
        let (base, bytes) = bar.pack();
        client
            .execute(
                "INSERT INTO footprint_minute (market, minute, tick, base, levels, trades) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (market, minute) DO NOTHING",
                &[&market, minute, &tick, &base, &bytes, &bar.trades],
            )
            .await?;
    }
    Ok(())
}

/// The fill's part of a minute added to what the stream wrote of it.
async fn join(it: &Footprint, market: &str, tick: f64, minute: i64, mut bar: Bar) -> anyhow::Result<()> {
    let client = it.pool().get().await?;
    if let Some(row) = client.query_opt("SELECT base, levels, trades FROM footprint_minute WHERE market = $1 AND minute = $2", &[&market, &minute]).await? {
        for (level, buy, sell) in unpack(row.get(0), row.get(1)) {
            let at = bar.levels.entry(level).or_insert((0.0, 0.0));
            (at.0, at.1) = (at.0 + buy, at.1 + sell);
        }
        bar.trades += row.get::<_, i64>(2);
    }
    let (base, bytes) = bar.pack();
    client
        .execute(
            "INSERT INTO footprint_minute (market, minute, tick, base, levels, trades) VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (market, minute) DO UPDATE SET tick = EXCLUDED.tick, base = EXCLUDED.base, levels = EXCLUDED.levels, trades = EXCLUDED.trades",
            &[&market, &minute, &tick, &base, &bytes, &bar.trades],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trade(id: i64, price: f64, qty: f64, buy: bool, at: i64) -> Trade {
        Trade { id, price, qty, buy, at }
    }

    #[test]
    fn the_stream_and_rest_give_trades_alike_and_the_snapshot_is_left_out() {
        let row = json!({ "trade_id": 41, "price": "68923.6", "size": "0.00717", "is_maker_ask": true, "timestamp": 1_794_544_292_660i64 });
        assert_eq!(trades(&json!([row])), vec![trade(41, 68_923.6, 0.00717, true, 1_794_544_292_660)], "the maker was the ask: the taker bought");
        assert_eq!(live(&json!({ "type": "update/trade", "channel": "trade:1", "trades": [row] })).len(), 1);
        assert!(live(&json!({ "type": "subscribed/trade", "channel": "trade:1", "trades": [row] })).is_empty(), "older trades, for the fill");
        assert!(trades(&json!([{ "trade_id": 1, "price": "0", "size": "1", "is_maker_ask": false, "timestamp": 1 }])).is_empty());
        assert_eq!(subscribe("1"), r#"{"type":"subscribe","channel":"trade/1"}"#);
    }

    #[test]
    fn a_page_adds_only_what_the_stream_did_not_see_and_says_when_it_is_past_the_start() {
        let minute = 1_800_000_000_000 - 1_800_000_000_000i64.rem_euclid(60_000);
        let mut bars = BTreeMap::new();
        // Newest first, as Lighter pages them: 30 the stream already has, then two before it.
        let page = [trade(30, 100.4, 1.0, true, minute + 50_000), trade(29, 100.6, 2.0, false, minute + 40_000), trade(28, 101.0, 0.5, true, minute - 1_000)];
        assert!(!take(&mut bars, 1.0, &page, 30, minute - 60_000));
        assert_eq!(bars[&minute].levels.get(&100), Some(&(0.0, 2.0)), "29 alone in its minute: 30 is the stream's");
        assert_eq!(bars[&(minute - 60_000)].trades, 1);
        // Reaching back past where it was to stop.
        assert!(take(&mut bars, 1.0, &[trade(27, 99.0, 1.0, true, minute - 61_000)], 30, minute - 60_000));
        assert_eq!(bars.len(), 2, "the trade before the start is not kept");
    }
}
