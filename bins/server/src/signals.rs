//! The Signal panel's screener. A minute after each four-hour bar closes, the
//! two hundred coins with the most turnover on Binance's and Bybit's USDT
//! perpetuals are read — five hundred closed bars each — and searched for three
//! drives either way (`drives`). A pattern is kept with the last scan that
//! still saw it: the panel lists what the latest scan saw, and the readers who
//! asked are told on Telegram, once, when a pattern turns.
//!
//! Between scans, two drives waiting for a third (`drives::setup`) are
//! weighed against live prices every fifteen seconds: price reaching their
//! line is a third drive as it happens, shown at once and told, to readers who
//! ask for it, on Telegram — hours before the bars that confirm it close.
//!
//! The record: a pattern that turned is entered at the close the scan read,
//! as a trade with three take-profits and a stop under the live plan
//! (`drives::levels`), its prices kept with it; and each
//! scan after weighs the bars since until it is closed or four days pass
//! (`drives::outcome`). Each pattern also has a limit entry, set by the scan
//! that first saw it half way back toward the third drive, recorded the same
//! way once filled (`drives::limit_entry`). The same rules
//! are replayed weekly — and whenever they change — over some 166 days of
//! every coin's bars, bar by bar and seeing nothing past the bar a scan would
//! have read, for what the record would have been.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tradoor_core::format::format_price;
use tradoor_core::{Candle, Interval};
use tradoor_store::now_ms;
use tradoor_venues::{binance, bybit, Exchange, MarketRef, OwnedKline};

use crate::api::{owner, ApiError, ApiResult, AppState};
use crate::catalog::Catalog;
use crate::dns::Doh;
use crate::drives::{self, Fill, Found, Hit, Plan, Point, Setup, Side};
use crate::telegram::{self, Telegram};

/// Coins screened, by a day's turnover.
pub(crate) const COINS: usize = 200;
/// Closed bars read per coin: enough for forty days of drives and their warm-up.
const BARS: u32 = 500;
/// Closed bars a replay reads per coin: as many as Bybit gives in one ask, some 166 days.
const HISTORY_BARS: u32 = 1000;
/// The rules' version. A change to `drives` bumps it, and the history is
/// replayed under the new rules after the next scan.
const VERSION: i32 = 10;
/// How old a replay may grow before it is run again.
const REPLAY_EVERY_MS: i64 = 7 * 24 * 3_600_000;
const STEP: i64 = 4 * 3_600_000;

/// The lengths of bar screened. Four hours is the list's own; a pattern on the
/// day's or the week's bars is the strong kind, listed with the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Frame {
    H4,
    D1,
    W1,
}

impl Frame {
    const ALL: [Frame; 3] = [Frame::H4, Frame::D1, Frame::W1];

    fn id(self) -> &'static str {
        match self {
            Frame::H4 => "4h",
            Frame::D1 => "1d",
            Frame::W1 => "1w",
        }
    }

    fn of(id: &str) -> Frame {
        match id {
            "1d" => Frame::D1,
            "1w" => Frame::W1,
            _ => Frame::H4,
        }
    }

    fn interval(self) -> Interval {
        match self {
            Frame::H4 => Interval::H4,
            Frame::D1 => Interval::D1,
            Frame::W1 => Interval::W1,
        }
    }

    /// A bar's length; the exchanges' weeks open on Monday, as the epoch's are counted from.
    fn step(self) -> i64 {
        match self {
            Frame::H4 => STEP,
            Frame::D1 => 24 * 3_600_000,
            Frame::W1 => 7 * 24 * 3_600_000,
        }
    }

    fn rules(self) -> drives::Rules {
        if self == Frame::H4 { drives::Rules::FOUR_HOURS } else { drives::Rules::LONGER }
    }
}

/// A scan's bars: by length, then by market.
type Held = HashMap<Frame, HashMap<String, Vec<Candle>>>;

fn held_of<'a>(held: &'a Held, frame: Frame, market: &str) -> Option<&'a Vec<Candle>> {
    held.get(&frame)?.get(market)
}
/// How long after a bar closes it is read: the exchanges take a moment to settle it.
const SETTLE_MS: i64 = 60_000;
/// Requests in flight at once.
const AT_ONCE: usize = 4;
/// Patterns and scans kept, for the record of how they went.
const KEPT_MS: i64 = 90 * 24 * 3_600_000;
const KIND: &str = "three-drives";
/// How often live prices are weighed against the setups' lines.
const TOUCH_EVERY: Duration = Duration::from_secs(15);
/// The score a turned pattern needs for Telegram to hear of it; the panel lists them all.
const TOLD_SCORE: i32 = 70;
/// Patterns named in one Telegram message; the rest are in the panel.
const TOLD_AT_MOST: usize = 12;
/// The score, before the turn adds its fifteen, a bearish pattern's limit
/// entry needs for Telegram to hear of it, with delta divergence: what bore
/// out in the replay.
const LIMIT_SCORE: i32 = 55;
/// The live record of limit entries counts the patterns first seen from this
/// bar on, the first scan after they began: those before it were in the
/// history the rule was chosen on.
const LIMIT_SINCE: i64 = 1_791_115_200_000;
/// How long a limit is followed: its two days, the trade's four, and some.
const LIMIT_FOLLOWED_MS: i64 = 10 * 24 * 3_600_000;

struct Screener {
    pool: deadpool_postgres::Pool,
    http: reqwest::Client,
    catalog: Arc<Catalog>,
    telegram: Option<Arc<Telegram>>,
    /// The site's address, for the link in a message that opens its chart.
    site: Option<String>,
}

pub fn start(pool: deadpool_postgres::Pool, doh: &Arc<Doh>, catalog: Arc<Catalog>, telegram: Option<Arc<Telegram>>, site: Option<String>) {
    let http = reqwest::Client::builder().dns_resolver(doh.resolver()).timeout(Duration::from_secs(20)).build().unwrap_or_default();
    let site = site.map(|s| s.trim_end_matches('/').to_string()).filter(|s| s.starts_with("http"));
    let screener = Arc::new(Screener { pool, http, catalog, telegram, site });
    tokio::spawn(screener.clone().run());
    tokio::spawn(screener.watch());
}

/// The close of the newest bar that has settled: what a scan now reads up to.
fn settled(now: i64) -> i64 {
    (now - SETTLE_MS).div_euclid(STEP) * STEP
}

impl Screener {
    async fn run(self: Arc<Self>) {
        // A moment after a start for the catalogue and the database.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            let bar = settled(now_ms());
            // A restart within the same four hours does not scan again, unless
            // the rules have changed since that scan.
            let done = match self.pool.get().await {
                Ok(client) => client.query_opt("SELECT bar, version FROM signal_scan ORDER BY bar DESC LIMIT 1", &[]).await.ok().flatten(),
                Err(_) => None,
            };
            if done.is_none_or(|d| d.get::<_, i64>(0) < bar || d.get::<_, Option<i32>>(1) != Some(VERSION)) {
                if let Err(e) = self.scan(bar).await {
                    tracing::warn!(error = %e, "signal scan");
                }
            }
            if self.replay_due().await {
                if let Err(e) = self.replay(bar).await {
                    tracing::warn!(error = %e, "signal replay");
                }
            }
            let wait = bar + STEP + SETTLE_MS - now_ms();
            tokio::time::sleep(Duration::from_millis(wait.max(1_000) as u64)).await;
        }
    }

    async fn scan(&self, bar: i64) -> anyhow::Result<()> {
        let started = now_ms();
        let markets = self.catalog.perp_leaders(COINS).await;
        if markets.is_empty() {
            bail!("the catalogue lists no perpetuals yet");
        }
        let jobs: Vec<(String, Frame)> = markets.iter().flat_map(|m| Frame::ALL.map(|frame| (m.clone(), frame))).collect();
        let results: Vec<(String, Frame, anyhow::Result<(Vec<Candle>, Option<Vec<f64>>)>)> = futures_util::stream::iter(jobs)
            .map(|(market, frame)| async move {
                let bars = self.bars(&market, frame, bar, BARS).await;
                (market, frame, bars)
            })
            .buffer_unordered(AT_ONCE)
            .collect()
            .await;
        let mut held: Held = HashMap::new();
        let mut patterns = Vec::new();
        let mut setups = Vec::new();
        for (market, frame, bars) in results {
            match bars {
                Ok((bars, taker)) => {
                    for side in [Side::Bear, Side::Bull] {
                        // A pattern found is ahead of a setup on the same side; setups are the four hours' alone.
                        if let Some(f) = drives::find_with(&bars, taker.as_deref(), side, frame.rules()) {
                            patterns.push((market.clone(), frame, f));
                        } else if frame == Frame::H4 {
                            if let Some(setup) = drives::setup(&bars, taker.as_deref(), side) {
                                setups.push((market.clone(), setup));
                            }
                        }
                    }
                    held.entry(frame).or_default().insert(market, bars);
                }
                Err(e) => tracing::debug!(%market, frame = frame.id(), error = %e, "signal scan: bars not read"),
            }
        }
        // Entered at the newest close there is: the four hours', whatever the pattern's length.
        let found: Vec<(String, Frame, Found, f64)> = patterns
            .into_iter()
            .map(|(market, frame, f)| {
                let price = held_of(&held, Frame::H4, &market).or_else(|| held_of(&held, frame, &market)).and_then(|b| b.last()).map_or(0.0, |c| c.close);
                (market, frame, f, price)
            })
            .collect();
        let read = held.get(&Frame::H4).map_or(0, HashMap::len);
        self.keep(bar, read, &found, &setups).await?;
        if let Err(e) = self.end(bar, &held).await {
            tracing::warn!(error = %e, "saying why patterns ended");
        }
        tracing::info!(markets = read, found = found.len(), setups = setups.len(), secs = (now_ms() - started) / 1000, "signal scan");
        if let Err(e) = self.settle(bar, &held).await {
            tracing::warn!(error = %e, "weighing the patterns that turned");
        }
        if let Err(e) = self.settle_touches(bar, &held).await {
            tracing::warn!(error = %e, "weighing the touches of a line");
        }
        if let Err(e) = self.settle_limits(bar, &held).await {
            tracing::warn!(error = %e, "weighing the limit entries");
        }
        if let Err(e) = self.tell_limits(bar).await {
            tracing::warn!(error = %e, "telling of the limit entries");
        }
        self.tell(bar).await
    }

    /// The patterns recent scans listed and this one does not, each with why:
    /// for the list's Ended rows.
    async fn end(&self, bar: i64, held: &Held) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let gone = client
            .query("SELECT id, market, side, third_at, timeframe FROM signal WHERE seen_bar < $1 AND seen_bar >= $1 - $2 AND ended_why IS NULL", &[&bar, &(6 * STEP)])
            .await?;
        for row in gone {
            let (id, market, third_at): (i64, String, i64) = (row.get(0), row.get(1), row.get(3));
            let side = if row.get::<_, String>(2) == "bull" { Side::Bull } else { Side::Bear };
            let frame = Frame::of(&row.get::<_, String>(4));
            // Its coin no longer among those screened; or still standing, an older pattern on the coin listed in its place.
            let why = match held_of(held, frame, &market) {
                None => "unread",
                Some(bars) => drives::ended(bars, side, third_at, frame.rules()).map_or("replaced", |e| e.id()),
            };
            client.execute("UPDATE signal SET ended_why = $2 WHERE id = $1", &[&id, &why]).await?;
        }
        Ok(())
    }

    /// Each pattern's limit entry, worked out again from its bars until it is
    /// decided: its price as the scan that first saw the pattern set it,
    /// whether price came back to it, and how the trade went.
    async fn settle_limits(&self, bar: i64, held: &Held) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let open = client
            .query(
                "SELECT id, market, side, drives, found_bar, timeframe FROM signal WHERE limit_outcome IS NULL
                     AND found_bar >= $1::bigint - $2::bigint * CASE timeframe WHEN '1w' THEN 42 WHEN '1d' THEN 6 ELSE 1 END",
                &[&bar, &LIMIT_FOLLOWED_MS],
            )
            .await?;
        for row in open {
            let (id, market, drives, found_bar): (i64, String, Value, i64) = (row.get(0), row.get(1), row.get(3), row.get(4));
            let side = if row.get::<_, String>(2) == "bull" { Side::Bull } else { Side::Bear };
            let frame = Frame::of(&row.get::<_, String>(5));
            let step = frame.step();
            let fetched;
            let bars = match held_of(held, frame, &market) {
                Some(bars) => bars,
                None => match self.bars(&market, frame, bar, BARS).await {
                    Ok((bars, _)) => {
                        fetched = bars;
                        &fetched
                    }
                    Err(_) => continue,
                },
            };
            let (Some(third_at), Some(extreme)) = (drives[2]["at"].as_i64(), drives[2]["price"].as_f64()) else { continue };
            // The newest bar closed when the pattern was first seen.
            let (Some(third), Some(seen)) = (bars.iter().position(|k| k.open_time == third_at), bars.iter().rposition(|k| k.open_time + step <= found_bar)) else { continue };
            // The first take-profit, which ends the wait for the limit if price gets there first.
            let Some(lv) = pattern_levels(side, extreme, &drives, bars) else { continue };
            let Some((limit, fill)) = drives::limit_entry(bars, side, third, extreme, lv.tp[0], seen) else { continue };
            let until = bars[seen].open_time + step * (1 + drives::LIMIT_BARS as i64);
            let (mut entry_at, mut entry, mut o) = (None, None, None);
            if let Fill::At { bar: from, price, within } = fill {
                let lv = pattern_levels(side, price, &drives, bars).unwrap_or(lv);
                (entry_at, entry, o) = (Some(bars[from - 1].open_time + step), Some(price), Some(drives::play(side, price, lv, &bars[from..], within)));
            }
            let outcome = match (fill, o) {
                (Fill::Missed, _) => Some("missed"),
                (_, Some(o)) if o.result != Hit::Open => Some(o.result.id()),
                _ => None,
            };
            // The trade's prices at its entry: the limit, or better where price opened past it.
            let levels = levels_json(pattern_levels(side, entry.unwrap_or(limit), &drives, bars).unwrap_or(lv));
            client
                .execute(
                    "UPDATE signal SET limit_price = $2, limit_until = $3, limit_levels = $4, limit_at = $5, limit_entry = $6, limit_outcome = $7,
                         limit_outcome_at = $8, limit_ret = $9, limit_best = $10, limit_worst = $11 WHERE id = $1",
                    &[&id, &limit, &until, &levels, &entry_at, &entry, &outcome, &outcome.map(|_| bar), &o.map(|o| o.ret), &o.map(|o| o.best), &o.map(|o| o.worst)],
                )
                .await?;
        }
        Ok(())
    }

    /// Readers who asked hear of the limit entries a scan set: the bearish
    /// patterns seen for the first time, scoring `LIMIT_SCORE` before the turn,
    /// with delta divergence.
    async fn tell_limits(&self, bar: i64) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let set = client
            .query(
                "UPDATE signal SET limit_told_at = $2 WHERE found_bar = $1 AND limit_price IS NOT NULL AND limit_told_at IS NULL AND side = 'bear'
                     AND delta_divergence AND score - CASE WHEN turned_at IS NULL THEN 0 ELSE 15 END >= $3
                 RETURNING market, score - CASE WHEN turned_at IS NULL THEN 0 ELSE 15 END, divergence, delta_divergence, price, limit_price, limit_until, limit_levels, limit_entry, id, timeframe",
                &[&bar, &now_ms(), &LIMIT_SCORE],
            )
            .await?;
        let Some(telegram) = self.telegram.as_ref().filter(|_| !set.is_empty()) else { return Ok(()) };
        let mut limits: Vec<Limit> = set
            .iter()
            .map(|r| Limit {
                market: r.get(0),
                score: r.get(1),
                divergence: r.get(2),
                delta_divergence: r.get::<_, Option<bool>>(3) == Some(true),
                price: r.get(4),
                limit: r.get(5),
                until: r.get(6),
                levels: levels_of(&r.get::<_, Option<Value>>(7)),
                filled: r.get::<_, Option<f64>>(8).is_some(),
                frame: Frame::of(&r.get::<_, String>(10)),
                link: self.pattern_link(r.get(9), &r.get::<_, String>(0), Frame::of(&r.get::<_, String>(10))),
            })
            .collect();
        limits.sort_by_key(|l| (l.frame == Frame::H4, -l.score));
        for reader in client.query("SELECT owner FROM signal_watch WHERE limits", &[]).await? {
            let owner: String = reader.get(0);
            telegram.send_to_owner(&owner, |offset| limit_text(&limits, bar, offset)).await;
        }
        Ok(())
    }

    /// Each pattern that turned and has not played out, against the bars since it was entered.
    async fn settle(&self, bar: i64, held: &Held) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let open = client
            .query("SELECT id, market, side, entry_at, entry, drives, levels, timeframe FROM signal WHERE outcome IS NULL AND entry_at IS NOT NULL", &[])
            .await?;
        for row in open {
            let (id, market, entry_at): (i64, String, i64) = (row.get(0), row.get(1), row.get(3));
            let side = if row.get::<_, String>(2) == "bull" { Side::Bull } else { Side::Bear };
            // Weighed on its own length's bars: a day's pattern over the days after it.
            let frame = Frame::of(&row.get::<_, String>(7));
            // A coin that has left the two hundred is still followed to its end.
            let fetched;
            let bars = match held_of(held, frame, &market) {
                Some(bars) => bars,
                None => match self.bars(&market, frame, bar, BARS).await {
                    Ok((bars, _)) => {
                        fetched = bars;
                        &fetched
                    }
                    Err(_) => continue,
                },
            };
            let after: Vec<Candle> = bars.iter().filter(|k| k.open_time >= entry_at).cloned().collect();
            let entry: f64 = row.get(4);
            // A trade entered before its prices were kept has them worked out from its bars.
            let Some(lv) = levels_of(&row.get::<_, Option<Value>>(6)).or_else(|| pattern_levels(side, entry, &row.get(5), bars)) else { continue };
            let o = drives::outcome(side, entry, lv, &after);
            let decided = o.result != Hit::Open;
            client
                .execute(
                    "UPDATE signal SET outcome = $2, outcome_at = $3, ret = $4, best = $5, worst = $6, target = $7, stop = $8, levels = $9 WHERE id = $1",
                    &[&id, &decided.then(|| o.result.id()), &decided.then_some(bar), &o.ret, &o.best, &o.worst, &lv.tp[0], &lv.sl, &levels_json(lv)],
                )
                .await?;
        }
        Ok(())
    }

    /// Every fifteen seconds, the latest scan's setups against live prices.
    async fn watch(self: Arc<Self>) {
        loop {
            tokio::time::sleep(TOUCH_EVERY).await;
            if let Err(e) = self.touches().await {
                tracing::warn!(error = %e, "weighing the setups against prices");
            }
        }
    }

    async fn touches(&self) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let Some(latest) = client.query_one("SELECT max(bar) FROM signal_scan", &[]).await?.get::<_, Option<i64>>(0) else { return Ok(()) };
        let rows = client
            .query("SELECT market, side, drives, atr, clear, until_at, pullback, deltas IS NOT NULL FROM signal_setup WHERE seen_bar = $1 AND touched_at IS NULL", &[&latest])
            .await?;
        if rows.is_empty() {
            return Ok(());
        }
        let setups: Vec<(String, Setup, bool)> =
            rows.iter().filter_map(|r| Some((r.get(0), setup_of(&r.get::<_, String>(1), &r.get(2), r.get(3), r.get(4), r.get(5), r.get(6))?, r.get(7)))).collect();
        let prices = self.prices(setups.iter().map(|(m, _, _)| m.as_str())).await;
        let now = now_ms();
        let mut reached = Vec::new();
        for (market, setup, measured) in setups {
            let Some(&price) = prices.get(&market) else { continue };
            if !setup.touched(price, now) {
                continue;
            }
            let lv = touch_levels(&setup, price);
            // The push into the line so far, beside the pushes into the two drives.
            let third = if measured { self.push_now(&market, &setup).await } else { None };
            let marked = client
                .execute(
                    "UPDATE signal_setup SET touched_at = $5, touch_price = $6, levels = $7, deltas = deltas || COALESCE($8, '[]'::jsonb)
                     WHERE market = $1 AND side = $2 AND first_at = $3 AND second_at = $4 AND touched_at IS NULL",
                    &[&market, &setup.side.id(), &setup.drives[0].at, &setup.drives[1].at, &now, &price, &lv.map(levels_json), &third.map(|p| pushes_json(&[p]))],
                )
                .await?;
            if marked > 0 {
                tracing::info!(%market, side = setup.side.id(), price, "a third drive at the line");
                reached.push((market, setup, price, lv));
            }
        }
        let Some(telegram) = self.telegram.as_ref().filter(|_| !reached.is_empty()) else { return Ok(()) };
        for reader in client.query("SELECT owner FROM signal_watch WHERE touch", &[]).await? {
            let owner: String = reader.get(0);
            telegram.send_to_owner(&owner, |offset| touch_text(&reached, now, offset, |market, at| self.touch_link(market, at))).await;
        }
        Ok(())
    }

    /// The push into a setup's line so far: the bars since its second drive,
    /// the one forming with them — as far back as a setup's drives may be.
    async fn push_now(&self, market: &str, setup: &Setup) -> Option<drives::Push> {
        let (bars, taker) = self.bars(market, Frame::H4, i64::MAX, 300).await.ok()?;
        drives::push_since(&bars, taker.as_deref()?, setup.side, setup.drives[1].at)
    }

    /// Last prices for these markets: Binance's perpetuals all in one ask, Bybit's one by one.
    async fn prices<'a>(&self, markets: impl Iterator<Item = &'a str>) -> HashMap<String, f64> {
        let refs: Vec<MarketRef> = markets.filter_map(MarketRef::parse).collect();
        let mut out = HashMap::new();
        if refs.iter().any(|m| m.venue.exchange() == Exchange::Binance) {
            if let Ok(list) = self.json(&format!("{}/fapi/v1/ticker/price", binance::PERP_REST)).await {
                for t in list.as_array().into_iter().flatten() {
                    if let (Some(symbol), Some(price)) = (t["symbol"].as_str(), t["price"].as_str().and_then(|p| p.parse::<f64>().ok())) {
                        out.insert(format!("binance-perp:{symbol}"), price);
                    }
                }
            }
        }
        for m in refs.iter().filter(|m| m.venue.exchange() == Exchange::Bybit) {
            if let Ok(v) = self.json(&format!("{}/v5/market/tickers?category=linear&symbol={}", bybit::REST, m.symbol)).await {
                if let Some(price) = v["result"]["list"][0]["lastPrice"].as_str().and_then(|p| p.parse::<f64>().ok()) {
                    out.insert(format!("bybit-perp:{}", m.symbol), price);
                }
            }
        }
        out
    }

    async fn json(&self, url: &str) -> anyhow::Result<Value> {
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!("{status} from {url}");
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Each line price reached and not yet played out: what the two drives
    /// became, and target or breakout first, against the bars since.
    async fn settle_touches(&self, bar: i64, held: &Held) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let open = client
            .query(
                "SELECT market, side, first_at, second_at, drives, atr, clear, until_at, touched_at, touch_price, pullback, levels FROM signal_setup
                 WHERE touched_at IS NOT NULL AND outcome IS NULL",
                &[],
            )
            .await?;
        for row in open {
            let market: String = row.get(0);
            let side: String = row.get(1);
            let (first_at, second_at, touched_at, entry): (i64, i64, i64, f64) = (row.get(2), row.get(3), row.get(8), row.get(9));
            let Some(mut setup) = setup_of(&side, &row.get(4), row.get(5), row.get(6), row.get(7), row.get(10)) else { continue };
            let fetched;
            let bars = match held_of(held, Frame::H4, &market) {
                Some(bars) => bars,
                None => match self.bars(&market, Frame::H4, bar, BARS).await {
                    Ok((bars, _)) => {
                        fetched = bars;
                        &fetched
                    }
                    Err(_) => continue,
                },
            };
            let touch_bar = touched_at.div_euclid(STEP) * STEP;
            // A touch from before its pullback was kept has it read from the bars.
            if !setup.pullback.is_finite() {
                let pullback = bars.iter().filter(|k| k.open_time > second_at && k.open_time < touch_bar);
                setup.pullback = if setup.side == Side::Bull { pullback.map(|k| k.high).fold(f64::NEG_INFINITY, f64::max) } else { pullback.map(|k| k.low).fold(f64::INFINITY, f64::min) };
            }
            let Some(lv) = levels_of(&row.get::<_, Option<Value>>(11)).or_else(|| touch_levels(&setup, entry)) else { continue };
            let after: Vec<Candle> = bars.iter().filter(|k| k.open_time >= touch_bar).cloned().collect();
            let o = drives::touch_outcome(setup.side, entry, lv, &after);
            let target = lv.tp[0];
            // What the scans made of the two drives: a pattern, and whether it turned.
            let became = client
                .query_opt(
                    "SELECT bool_or(turned_at IS NOT NULL) FROM signal WHERE market = $1 AND side = $2
                     AND (drives->0->>'at')::bigint = $3 AND (drives->1->>'at')::bigint = $4 HAVING count(*) > 0",
                    &[&market, &side, &first_at, &second_at],
                )
                .await?
                .map_or(drives::Became::None, |r| if r.get::<_, Option<bool>>(0).unwrap_or(false) { drives::Became::Turned } else { drives::Became::Pattern });
            let decided = o.result != Hit::Open;
            client
                .execute(
                    "UPDATE signal_setup SET target = $5, became = $6, outcome = $7, outcome_at = $8, ret = $9, best = $10, worst = $11, levels = $12
                     WHERE market = $1 AND side = $2 AND first_at = $3 AND second_at = $4",
                    &[&market, &side, &first_at, &second_at, &target, &became.id(), &decided.then(|| o.result.id()), &decided.then_some(bar), &o.ret, &o.best, &o.worst, &levels_json(lv)],
                )
                .await?;
        }
        Ok(())
    }

    async fn replay_due(&self) -> bool {
        let Ok(client) = self.pool.get().await else { return false };
        match client.query_opt("SELECT at, version FROM signal_backtest_run ORDER BY at DESC LIMIT 1", &[]).await {
            Ok(Some(run)) => run.get::<_, i32>(1) != VERSION || run.get::<_, i64>(0) < now_ms() - REPLAY_EVERY_MS,
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// The rules run over each coin's history as the scans would have run them,
    /// and how each pattern then played out.
    async fn replay(&self, bar: i64) -> anyhow::Result<()> {
        let started = now_ms();
        let markets = self.catalog.perp_leaders(COINS).await;
        if markets.is_empty() {
            bail!("the catalogue lists no perpetuals yet");
        }
        let results: Vec<(String, anyhow::Result<(Vec<Candle>, Option<Vec<f64>>)>)> =
            futures_util::stream::iter(markets).map(|market| async move { (market.clone(), self.bars(&market, Frame::H4, bar, HISTORY_BARS).await) }).buffer_unordered(2).collect().await;
        let from = results.iter().filter_map(|(_, bars)| bars.as_ref().ok()?.0.first().map(|c| c.open_time)).min().unwrap_or(bar);
        // The record's two halves, to see whether a plan holds up in both.
        let middle = from + (bar - from) / 2;
        let mut read = 0;
        let mut rows = Replayed::default();
        let mut touches = TouchesReplayed::default();
        let mut plans = Plans::default();
        for (market, bars) in results {
            let Ok((bars, taker)) = bars else { continue };
            read += 1;
            for side in [Side::Bear, Side::Bull] {
                for t in drives::touches(&bars, side) {
                    if t.outcome.result != Hit::Open {
                        touches.push(&market, side, bars[t.at].open_time, &t);
                    }
                    for plan in Plan::ALL {
                        let lv = drives::plan_levels(plan, side, t.entry, t.entry, t.pullback, t.setup.atr);
                        let o = drives::play(side, t.entry, lv, &bars[t.at..], true);
                        plans.add(plan, "touch", side, None, bars[t.at].open_time >= middle, &o);
                    }
                }
                for (at, f) in drives::history(&bars, taker.as_deref(), side) {
                    let entry = bars[at].close;
                    for plan in Plan::ALL {
                        let lv = drives::plan_levels(plan, side, entry, f.extreme, f.pullback, f.atr);
                        let o = drives::play(side, entry, lv, &bars[at + 1..], false);
                        plans.add(plan, "pattern", side, Some(&f), bars[at].open_time >= middle, &o);
                    }
                    let o = drives::outcome(side, entry, drives::levels(side, entry, f.extreme, f.pullback, f.atr), &bars[at + 1..]);
                    if o.result == Hit::Open {
                        continue;
                    }
                    rows.push("pattern", &market, side, bars[at].open_time + STEP, entry, &f, Some(&o));
                }
                // The limit each pattern's first scan would have set, and the trade if price came back to it.
                for (seen, f) in drives::sightings(&bars, taker.as_deref(), side) {
                    let tp1 = drives::levels(side, f.extreme, f.extreme, f.pullback, f.atr).tp[0];
                    let Some((limit, fill)) = drives::limit_entry(&bars, side, f.drives[2].index, f.extreme, tp1, seen) else { continue };
                    match fill {
                        Fill::Waiting => {}
                        Fill::Missed => rows.push("limit", &market, side, bars[seen].open_time + STEP, limit, &f, None),
                        Fill::At { bar: from, price, within } => {
                            let entry_at = bars[from - 1].open_time + STEP;
                            for plan in Plan::ALL {
                                let lv = drives::plan_levels(plan, side, price, f.extreme, f.pullback, f.atr);
                                plans.add(plan, "limit", side, Some(&f), entry_at >= middle, &drives::play(side, price, lv, &bars[from..], within));
                            }
                            let o = drives::play(side, price, drives::levels(side, price, f.extreme, f.pullback, f.atr), &bars[from..], within);
                            if o.result != Hit::Open {
                                rows.push("limit", &market, side, entry_at, price, &f, Some(&o));
                            }
                        }
                    }
                }
            }
        }
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        let at = now_ms();
        tx.execute(
            "INSERT INTO signal_backtest_run (at, version, markets, from_at, to_at, variants) VALUES ($1, $2, $3, $4, $5, $6)",
            &[&at, &VERSION, &(read as i32), &from, &bar, &plans.json()],
        )
        .await?;
        tx.execute(
            "INSERT INTO signal_backtest (run, kind, market, side, entry_at, entry, target, stop, score, divergence, outcome, bars, ret, best, worst, delta_divergence, delta_against)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::text[], $5::bigint[], $6::float8[], $7::float8[], $8::float8[], $9::int[], $10::bool[], $11::text[], $12::int[], $13::float8[], $14::float8[], $15::float8[], $16::bool[], $17::bool[])
             ON CONFLICT DO NOTHING",
            &[&at, &rows.kind, &rows.market, &rows.side, &rows.entry_at, &rows.entry, &rows.target, &rows.stop, &rows.score, &rows.divergence, &rows.outcome, &rows.bars, &rows.ret, &rows.best, &rows.worst, &rows.delta_divergence, &rows.delta_against],
        )
        .await?;
        tx.execute(
            "INSERT INTO signal_touch_backtest (run, market, side, touched_at, entry, target, became, outcome, bars, ret, best, worst)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::bigint[], $5::float8[], $6::float8[], $7::text[], $8::text[], $9::int[], $10::float8[], $11::float8[], $12::float8[])
             ON CONFLICT DO NOTHING",
            &[&at, &touches.market, &touches.side, &touches.at, &touches.entry, &touches.target, &touches.became, &touches.outcome, &touches.bars, &touches.ret, &touches.best, &touches.worst],
        )
        .await?;
        tx.execute("DELETE FROM signal_backtest_run WHERE at <> $1", &[&at]).await?;
        tx.commit().await?;
        tracing::info!(markets = read, patterns = rows.market.len(), touches = touches.market.len(), secs = (now_ms() - started) / 1000, "signal replay");
        Ok(())
    }

    /// A market's last `count` closed bars of `frame` up to `bar`, oldest first,
    /// with each bar's taker buy volume where the exchange reports it.
    async fn bars(&self, market: &str, frame: Frame, bar: i64, count: u32) -> anyhow::Result<(Vec<Candle>, Option<Vec<f64>>)> {
        let m = MarketRef::parse(market).ok_or_else(|| anyhow!("{market} is not a market"))?;
        let exchange = m.venue.exchange();
        let url = match exchange {
            Exchange::Binance => binance::klines_url(binance::PERP_REST, &m, frame.interval(), None, None, count),
            Exchange::Bybit => bybit::klines_url(bybit::REST, &m, frame.interval(), None, None, count),
            _ => bail!("only Binance and Bybit are screened"),
        };
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!("{status} from {url}");
        }
        let mut klines = if exchange == Exchange::Binance { binance::parse_klines(&body)? } else { bybit::parse_klines(&body)? };
        klines.retain(|k| k.open_time.saturating_add(frame.step()) <= bar);
        candles_of(&klines)
    }

    /// The scan and what it found, a pattern seen before keeping the bar it first turned on.
    async fn keep(&self, bar: i64, read: usize, found: &[(String, Frame, Found, f64)], setups: &[(String, Setup)]) -> anyhow::Result<()> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "INSERT INTO signal_scan (bar, at, markets, found, version) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (bar) DO UPDATE SET at = EXCLUDED.at, markets = EXCLUDED.markets, found = EXCLUDED.found, version = EXCLUDED.version",
            &[&bar, &now_ms(), &(read as i32), &(found.len() as i32), &VERSION],
        )
        .await?;
        // A bar scanned again: what it saw before counts as seen by the scan before, until seen again.
        tx.execute("UPDATE signal SET seen_bar = $1 - $2 WHERE seen_bar = $1", &[&bar, &STEP]).await?;
        tx.execute("UPDATE signal_setup SET seen_bar = $1 - $2 WHERE seen_bar = $1", &[&bar, &STEP]).await?;
        for (market, frame, f, price) in found {
            let drives = Value::Array(f.drives.iter().map(|d| json!({ "at": d.at, "price": d.price })).collect());
            let (turned_at, turn_price) = (f.turn.map(|t| t.at), f.turn.map(|t| t.price));
            // Entered where the first scan to see it turned read up to, at the live plan's prices.
            let (entry_at, entry) = (f.turn.map(|_| bar), f.turn.map(|_| *price));
            let levels = f.turn.map(|_| levels_json(drives::levels(f.side, *price, f.extreme, f.pullback, f.atr)));
            let pushes = f.pushes.as_ref().map(|p| pushes_json(p));
            tx.execute(
                "INSERT INTO signal (kind, market, side, timeframe, third_at, drives, turned_at, turn_price, score, divergence, price, found_bar, seen_bar, entry_at, entry, levels, delta_divergence, delta_against, deltas)
                 VALUES ($1, $2, $3, $18, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17)
                 ON CONFLICT (kind, market, side, timeframe, third_at) DO UPDATE SET
                     drives = EXCLUDED.drives, turned_at = COALESCE(signal.turned_at, EXCLUDED.turned_at),
                     turn_price = COALESCE(signal.turn_price, EXCLUDED.turn_price), score = EXCLUDED.score,
                     divergence = EXCLUDED.divergence, price = EXCLUDED.price, seen_bar = EXCLUDED.seen_bar,
                     entry_at = COALESCE(signal.entry_at, EXCLUDED.entry_at), entry = COALESCE(signal.entry, EXCLUDED.entry),
                     levels = COALESCE(signal.levels, EXCLUDED.levels),
                     delta_divergence = EXCLUDED.delta_divergence, delta_against = EXCLUDED.delta_against, deltas = EXCLUDED.deltas, ended_why = NULL",
                &[&KIND, market, &f.side.id(), &f.drives[2].at, &drives, &turned_at, &turn_price, &(f.score as i32), &f.divergence, price, &bar, &entry_at, &entry, &levels, &f.delta_divergence, &f.delta_against, &pushes, &frame.id()],
            )
            .await?;
        }
        for (market, setup) in setups {
            let drives = Value::Array(setup.drives.iter().map(|d| json!({ "at": d.at, "price": d.price })).collect());
            // Once price has reached the line, the push into it is kept beside the first two.
            let pushes = setup.pushes.as_ref().map(|p| pushes_json(p));
            tx.execute(
                "INSERT INTO signal_setup (market, side, first_at, second_at, drives, atr, clear, until_at, seen_bar, pullback, deltas)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (market, side, first_at, second_at) DO UPDATE SET
                     drives = EXCLUDED.drives, atr = EXCLUDED.atr, clear = EXCLUDED.clear, until_at = EXCLUDED.until_at, seen_bar = EXCLUDED.seen_bar,
                     pullback = EXCLUDED.pullback,
                     deltas = CASE WHEN signal_setup.touched_at IS NULL OR signal_setup.deltas IS NULL THEN EXCLUDED.deltas ELSE signal_setup.deltas END",
                &[market, &setup.side.id(), &setup.drives[0].at, &setup.drives[1].at, &drives, &setup.atr, &setup.clear, &setup.until, &bar, &setup.pullback, &pushes],
            )
            .await?;
        }
        // A setup price never reached goes after two days; one it did stays, for the record.
        tx.execute("DELETE FROM signal_setup WHERE seen_bar < $1 AND touched_at IS NULL", &[&(bar - 2 * 24 * 3_600_000)]).await?;
        tx.execute("DELETE FROM signal_setup WHERE seen_bar < $1", &[&(bar - KEPT_MS)]).await?;
        tx.execute("DELETE FROM signal WHERE seen_bar < $1", &[&(bar - KEPT_MS)]).await?;
        tx.execute("DELETE FROM signal_scan WHERE bar < $1", &[&(bar - KEPT_MS)]).await?;
        tx.commit().await?;
        Ok(())
    }

    /// One message per reader who asked, naming the patterns that turned since the last.
    async fn tell(&self, bar: i64) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let turned = client
            .query(
                // The day's and the week's only where the turn is new: not those a first scan of them finds long turned.
                "UPDATE signal SET told_at = $2 WHERE seen_bar = $1 AND turned_at IS NOT NULL AND told_at IS NULL
                     AND (timeframe = '4h' OR turned_at >= $1::bigint - $3::bigint * CASE timeframe WHEN '1w' THEN 14 ELSE 2 END)
                 RETURNING market, side, drives, score, divergence, price, levels, delta_divergence, id, timeframe",
                &[&bar, &now_ms(), &Frame::D1.step()],
            )
            .await?;
        let Some(telegram) = self.telegram.as_ref().filter(|_| !turned.is_empty()) else { return Ok(()) };
        let mut lines: Vec<Told> = turned
            .iter()
            .map(|r| Told {
                market: r.get(0),
                bull: r.get::<_, String>(1) == "bull",
                drives: r.get::<_, Value>(2).as_array().map(|d| d.iter().filter_map(|p| p["price"].as_f64()).collect()).unwrap_or_default(),
                score: r.get(3),
                divergence: r.get(4),
                price: r.get(5),
                levels: levels_of(&r.get::<_, Option<Value>>(6)),
                delta_divergence: r.get::<_, Option<bool>>(7) == Some(true),
                frame: Frame::of(&r.get::<_, String>(9)),
                link: self.pattern_link(r.get(8), &r.get::<_, String>(0), Frame::of(&r.get::<_, String>(9))),
            })
            // Telegram hears of the best of them: a score of 70 or more, and the day's and the week's whatever theirs.
            .filter(|t| t.score >= TOLD_SCORE || t.frame != Frame::H4)
            .collect();
        if lines.is_empty() {
            return Ok(());
        }
        // The day's and the week's first, the strong kind.
        lines.sort_by_key(|t| (t.frame == Frame::H4, -t.score));
        for reader in client.query("SELECT owner FROM signal_watch WHERE notify", &[]).await? {
            let owner: String = reader.get(0);
            telegram.send_to_owner(&owner, |offset| signal_text(&lines, bar, offset)).await;
        }
        Ok(())
    }
}

impl Screener {
    /// The link that opens a pattern on the site: its chart, at its length,
    /// with it drawn — or, once it has left the list, its chart all the same.
    fn pattern_link(&self, id: i64, market: &str, frame: Frame) -> Option<String> {
        self.site.as_ref().map(|site| format!("{site}/?signal={id}&market={market}&tf={}", frame.id()))
    }

    /// The link that opens a touch of a line on the site.
    fn touch_link(&self, market: &str, at: i64) -> Option<String> {
        self.site.as_ref().map(|site| format!("{site}/?market={market}&tf=4h&touched={at}"))
    }
}

/// Klines as candles, oldest first, with each bar's taker buy volume where
/// the exchange reports it — all of them or none.
pub(crate) fn candles_of(klines: &[OwnedKline]) -> anyhow::Result<(Vec<Candle>, Option<Vec<f64>>)> {
    let taker = klines.iter().map(|k| k.taker_buy_volume.as_deref()?.parse::<f64>().ok()).collect::<Option<Vec<f64>>>();
    let candles = klines
        .iter()
        .map(|k| {
            Ok(Candle {
                open_time: k.open_time,
                open: k.open.parse()?,
                high: k.high.parse()?,
                low: k.low.parse()?,
                close: k.close.parse()?,
                volume: k.volume.parse()?,
                quote_volume: k.quote_volume.as_deref().and_then(|q| q.parse().ok()),
                taker_buy: None,
            })
        })
        .collect::<anyhow::Result<Vec<Candle>>>()?;
    Ok((candles, taker))
}

/// A frame's name as a message writes it, starred where its patterns are the strong kind.
fn frame_label(frame: Frame) -> &'static str {
    match frame {
        Frame::H4 => "4H",
        Frame::D1 => "1D ⭐",
        Frame::W1 => "1W ⭐",
    }
}

/// A setup as the table keeps it; one kept before its pullback was has none.
fn setup_of(side: &str, drives: &Value, atr: f64, clear: f64, until: i64, pullback: Option<f64>) -> Option<Setup> {
    let point = |i: usize| Some(Point { index: 0, at: drives[i]["at"].as_i64()?, price: drives[i]["price"].as_f64()? });
    let side = if side == "bull" { Side::Bull } else { Side::Bear };
    Some(Setup { side, drives: [point(0)?, point(1)?], atr, clear, until, pullback: pullback.unwrap_or(f64::NAN), pushes: None })
}

/// The pushes into the drives as the tables keep them and the panel reads them.
fn pushes_json(pushes: &[drives::Push]) -> Value {
    Value::Array(pushes.iter().map(|p| json!({ "usd": p.usd, "share": p.share })).collect())
}

/// Whether the push into the third drive kept less of the volume the
/// pattern's way than the push into the second — delta divergence, from the
/// pushes as kept.
fn weakened(side: Side, pushes: &Value) -> Option<bool> {
    let share = |i: usize| pushes.get(i)?.get("share")?.as_f64();
    let sign = if side == Side::Bull { -1.0 } else { 1.0 };
    Some(sign * share(2)? < sign * share(1)?)
}

/// A trade's prices as the tables keep them.
fn levels_json(lv: drives::Levels) -> Value {
    json!({ "tp": lv.tp, "sl": lv.sl })
}

fn levels_of(value: &Option<Value>) -> Option<drives::Levels> {
    let v = value.as_ref()?;
    let tp = |i: usize| v["tp"][i].as_f64();
    Some(drives::Levels { tp: [tp(0)?, tp(1)?, tp(2)?], sl: v["sl"].as_f64()? })
}

/// The live plan's prices for price at a setup's line: the touch is the third
/// drive's extreme so far, its leg begun at the pullback after the second.
fn touch_levels(setup: &Setup, price: f64) -> Option<drives::Levels> {
    setup.pullback.is_finite().then(|| drives::levels(setup.side, price, price, setup.pullback, setup.atr))
}

/// The live plan's prices for a pattern entered before they were kept, from
/// its bars: the pullback between the second drive and the third, and the ATR
/// at the third.
fn pattern_levels(side: Side, entry: f64, drives: &Value, bars: &[Candle]) -> Option<drives::Levels> {
    let at = |i: usize| drives[i]["at"].as_i64();
    let (second, third, extreme) = (at(1)?, at(2)?, drives[2]["price"].as_f64()?);
    let between = bars.iter().filter(|k| k.open_time > second && k.open_time < third);
    let pullback = if side == Side::Bull { between.map(|k| k.high).fold(f64::NEG_INFINITY, f64::max) } else { between.map(|k| k.low).fold(f64::INFINITY, f64::min) };
    let i = bars.iter().position(|k| k.open_time == third)?;
    let atr = tradoor_core::indicators::atr(bars, 14)[i]?;
    pullback.is_finite().then(|| drives::levels(side, entry, extreme, pullback, atr))
}

/// What the screener says when price reaches a setup's line, in the reader's own hours.
fn touch_text(reached: &[(String, Setup, f64, Option<drives::Levels>)], at: i64, offset: i32, link: impl Fn(&str, i64) -> Option<String>) -> String {
    let mut text = format!("📐 Three drives · 4h — {} at the line\n", reached.len());
    for (market, setup, price, levels) in reached.iter().take(TOLD_AT_MOST) {
        let name = MarketRef::parse(market).map_or_else(|| market.clone(), |m| format!("{} ({})", m.symbol, m.venue.label()));
        let (dot, way, what) = if setup.side == Side::Bull { ("🟢", "bullish", "a third low") } else { ("🔴", "bearish", "a third high") };
        text += &format!(
            "\n{dot} {name} — {way}: {what} at the line\n   drives {} → {} · line {} · now {}\n{}{}",
            price_text(setup.drives[0].price),
            price_text(setup.drives[1].price),
            price_text(setup.line(at)),
            price_text(*price),
            levels.map_or_else(String::new, |lv| format!("   {}\n", levels_text(lv))),
            link(market, at).map_or_else(String::new, |l| format!("   {l}\n")),
        );
    }
    text + &format!("\nNot confirmed until the four-hour bars close · {}", telegram::stamp(at, offset))
}

/// Replayed trades, a column each, for one insert: entered at the turn
/// (`pattern`) or at the limit (`limit`, `missed` where price never came back).
#[derive(Default)]
struct Replayed {
    kind: Vec<&'static str>,
    market: Vec<String>,
    side: Vec<&'static str>,
    entry_at: Vec<i64>,
    entry: Vec<f64>,
    target: Vec<f64>,
    stop: Vec<f64>,
    score: Vec<i32>,
    divergence: Vec<bool>,
    outcome: Vec<&'static str>,
    bars: Vec<i32>,
    ret: Vec<f64>,
    best: Vec<f64>,
    worst: Vec<f64>,
    delta_divergence: Vec<Option<bool>>,
    delta_against: Vec<Option<bool>>,
}

impl Replayed {
    fn push(&mut self, kind: &'static str, market: &str, side: Side, entry_at: i64, entry: f64, f: &Found, o: Option<&drives::Outcome>) {
        self.kind.push(kind);
        self.market.push(market.to_string());
        self.side.push(side.id());
        self.entry_at.push(entry_at);
        self.entry.push(entry);
        let lv = drives::levels(side, entry, f.extreme, f.pullback, f.atr);
        self.target.push(lv.tp[0]);
        self.stop.push(lv.sl);
        self.score.push(score_of(kind, f));
        self.divergence.push(f.divergence);
        self.delta_divergence.push(f.delta_divergence);
        self.delta_against.push(f.delta_against);
        self.outcome.push(o.map_or("missed", |o| o.result.id()));
        self.bars.push(o.map_or(0, |o| o.bars as i32));
        self.ret.push(o.map_or(0.0, |o| o.ret));
        self.best.push(o.map_or(0.0, |o| o.best));
        self.worst.push(o.map_or(0.0, |o| o.worst));
    }
}

/// The cuts of the record a trade counts in: all of them, its score band, and
/// each sign it carries — RSI divergence, delta divergence, a sold third drive
/// — with the best score band and delta divergence together. A limit is set
/// before the turn adds its fifteen to the score, so its bands are fifteen lower.
fn groups_of(kind: &str, score: i32, divergence: bool, delta_divergence: Option<bool>, delta_against: Option<bool>) -> Vec<&'static str> {
    let ((top, top_id, top_ddiv), (mid, mid_id), low_id) = if kind == "limit" { ((55, "55", "55ddiv"), (40, "40"), "u40") } else { ((70, "70", "70ddiv"), (50, "50"), "0") };
    let mut groups = vec!["all", if score >= top { top_id } else if score >= mid { mid_id } else { low_id }];
    if divergence {
        groups.push("div");
    }
    if delta_divergence == Some(true) {
        groups.push("ddiv");
        if score >= top {
            groups.push(top_ddiv);
        }
    }
    if delta_against == Some(true) {
        groups.push("dagainst");
    }
    groups
}

/// The score a trade is cut by: a limit's is the score before the turn adds
/// its fifteen, as the scan that set it saw it.
fn score_of(kind: &str, f: &Found) -> i32 {
    f.score as i32 - if kind == "limit" && f.turn.is_some() { 15 } else { 0 }
}

/// One plan's record over the replay, in one cut of it.
#[derive(Default)]
struct Tally {
    n: usize,
    /// How many closed in profit.
    win: usize,
    /// How many reached each take-profit, each counting those that went further.
    tp: [usize; 3],
    stop: usize,
    expired: usize,
    ret: f64,
    /// Count and summed result in the first half of the replay and the second.
    halves: [(usize, f64); 2],
}

/// Every plan's tallies, by plan, kind, side and group, in a steady order.
#[derive(Default)]
struct Plans(std::collections::BTreeMap<(usize, &'static str, &'static str, &'static str), Tally>);

impl Plans {
    /// One trade under `plan`; a pattern's score and divergence put it in its groups too.
    fn add(&mut self, plan: Plan, kind: &'static str, side: Side, pattern: Option<&Found>, late: bool, o: &drives::Outcome) {
        if o.result == Hit::Open {
            return;
        }
        let groups = pattern.map_or_else(|| vec!["all"], |f| groups_of(kind, score_of(kind, f), f.divergence, f.delta_divergence, f.delta_against));
        let order = Plan::ALL.iter().position(|p| *p == plan).unwrap_or(0);
        for group in groups {
            let t = self.0.entry((order, kind, side.id(), group)).or_default();
            t.n += 1;
            t.win += usize::from(o.ret > 0.0);
            let takes = match o.result {
                Hit::Tp3 => 3,
                Hit::Tp2 => 2,
                Hit::Tp1 => 1,
                _ => 0,
            };
            for reached in &mut t.tp[..takes] {
                *reached += 1;
            }
            match o.result {
                Hit::Stop => t.stop += 1,
                Hit::Expired => t.expired += 1,
                _ => {}
            }
            t.ret += o.ret;
            let half = &mut t.halves[usize::from(late)];
            half.0 += 1;
            half.1 += o.ret;
        }
    }

    fn json(&self) -> Value {
        let mean = |sum: f64, n: usize| if n == 0 { 0.0 } else { sum / n as f64 };
        Value::Array(
            self.0
                .iter()
                .map(|((order, kind, side, group), t)| {
                    json!({
                        "plan": Plan::ALL[*order].id(),
                        "kind": kind,
                        "side": side,
                        "group": group,
                        "n": t.n,
                        "win": t.win,
                        "tp1": t.tp[0],
                        "tp2": t.tp[1],
                        "tp3": t.tp[2],
                        "stop": t.stop,
                        "expired": t.expired,
                        "ret": mean(t.ret, t.n),
                        "halves": t.halves.map(|(n, sum)| mean(sum, n)),
                    })
                })
                .collect(),
        )
    }
}

/// Replayed touches, a column each, for one insert.
#[derive(Default)]
struct TouchesReplayed {
    market: Vec<String>,
    side: Vec<&'static str>,
    at: Vec<i64>,
    entry: Vec<f64>,
    target: Vec<f64>,
    became: Vec<&'static str>,
    outcome: Vec<&'static str>,
    bars: Vec<i32>,
    ret: Vec<f64>,
    best: Vec<f64>,
    worst: Vec<f64>,
}

impl TouchesReplayed {
    fn push(&mut self, market: &str, side: Side, at: i64, t: &drives::Touch) {
        self.market.push(market.to_string());
        self.side.push(side.id());
        self.at.push(at);
        self.entry.push(t.entry);
        self.target.push(drives::levels(side, t.entry, t.entry, t.pullback, t.setup.atr).tp[0]);
        self.became.push(t.became.id());
        self.outcome.push(t.outcome.result.id());
        self.bars.push(t.outcome.bars as i32);
        self.ret.push(t.outcome.ret);
        self.best.push(t.outcome.best);
        self.worst.push(t.outcome.worst);
    }
}

/// A pattern that turned, as a message names it.
struct Told {
    market: String,
    bull: bool,
    drives: Vec<f64>,
    score: i32,
    divergence: bool,
    price: f64,
    levels: Option<drives::Levels>,
    delta_divergence: bool,
    frame: Frame,
    /// Opens its chart with the pattern drawn.
    link: Option<String>,
}

/// A limit entry set, as a message names it.
struct Limit {
    market: String,
    /// The score before the turn adds its fifteen.
    score: i32,
    divergence: bool,
    delta_divergence: bool,
    price: f64,
    limit: f64,
    until: i64,
    levels: Option<drives::Levels>,
    /// Price already past it at the scan's close: it fills there.
    filled: bool,
    frame: Frame,
    link: Option<String>,
}

/// What the screener says of the limit entries a scan set, in the reader's own hours.
fn limit_text(limits: &[Limit], bar: i64, offset: i32) -> String {
    let mut text = format!("🎯 Three drives — {} limit {}\n", limits.len(), if limits.len() == 1 { "entry" } else { "entries" });
    for l in limits.iter().take(TOLD_AT_MOST) {
        let name = MarketRef::parse(&l.market).map_or_else(|| l.market.clone(), |m| format!("{} ({})", m.symbol, m.venue.label()));
        let divergence = match (l.divergence, l.delta_divergence) {
            (true, true) => " · RSI and delta divergence",
            (true, false) => " · RSI divergence",
            (false, true) => " · delta divergence",
            (false, false) => "",
        };
        let wait = if l.filled { "price is past it: fills at once".to_string() } else { format!("until {}", telegram::stamp(l.until, offset)) };
        let levels = l.levels.map_or_else(String::new, |lv| format!("   {}\n", levels_text(lv)));
        let link = l.link.as_ref().map_or_else(String::new, |link| format!("   {link}\n"));
        text += &format!("\n🔴 {name} · {} — bearish, score {}{divergence}\n   limit sell {} · now {} · {wait}\n{levels}{link}", frame_label(l.frame), l.score, price_text(l.limit), price_text(l.price));
    }
    if limits.len() > TOLD_AT_MOST {
        text += &format!("\n…and {} more in the Signal panel\n", limits.len() - TOLD_AT_MOST);
    }
    text + &format!("\nHalf way back up from the low since the third drive; off if TP1 comes first · {}", telegram::stamp(bar, offset))
}

/// A price to five figures, as a chart of it would read: 13.600, 1,580.00.
fn price_text(price: f64) -> String {
    let digits = (4 - price.abs().log10().floor() as i32).clamp(2, 8);
    format_price(price, Some(digits as usize))
}

/// A trade's prices, as a message names them.
fn levels_text(lv: drives::Levels) -> String {
    format!("TP {} / {} / {} · SL {}", price_text(lv.tp[0]), price_text(lv.tp[1]), price_text(lv.tp[2]), price_text(lv.sl))
}

/// What the screener says when patterns turn, in the reader's own hours.
fn signal_text(turned: &[Told], bar: i64, offset: i32) -> String {
    let mut text = format!("🔔 Three drives — {} turned\n", turned.len());
    for t in turned.iter().take(TOLD_AT_MOST) {
        let name = MarketRef::parse(&t.market).map_or_else(|| t.market.clone(), |m| format!("{} ({})", m.symbol, m.venue.label()));
        let drives = t.drives.iter().map(|p| price_text(*p)).collect::<Vec<_>>().join(" → ");
        let (dot, way) = if t.bull { ("🟢", "bullish") } else { ("🔴", "bearish") };
        let divergence = match (t.divergence, t.delta_divergence) {
            (true, true) => " · RSI and delta divergence",
            (true, false) => " · RSI divergence",
            (false, true) => " · delta divergence",
            (false, false) => "",
        };
        let levels = t.levels.map_or_else(String::new, |lv| format!("   {}\n", levels_text(lv)));
        let link = t.link.as_ref().map_or_else(String::new, |l| format!("   {l}\n"));
        text += &format!("\n{dot} {name} · {} — {way}, score {}{divergence}\n   drives {drives} · now {}\n{levels}{link}", frame_label(t.frame), t.score, price_text(t.price));
    }
    if turned.len() > TOLD_AT_MOST {
        text += &format!("\n…and {} more in the Signal panel\n", turned.len() - TOLD_AT_MOST);
    }
    text + &format!("\n{}", telegram::stamp(bar, offset))
}

// ---------------------------------------------------------------- the API

/// `GET /api/signals`: what the latest scan found, when it ran and when the
/// next will, and whether this reader is told on Telegram.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let client = state.pool.get().await?;
    let watch = client.query_opt("SELECT notify, touch, limits FROM signal_watch WHERE owner = $1", &[&owner]).await?;
    let (notify, notify_touch, notify_limits) = watch.map_or((false, false, false), |r| (r.get::<_, bool>(0), r.get::<_, bool>(1), r.get::<_, bool>(2)));
    let linked = match state.telegram.as_ref() {
        Some(telegram) => !telegram.chats(&owner).await?.is_empty(),
        None => false,
    };
    let now = now_ms();
    let scan = client.query_opt("SELECT bar, at, markets FROM signal_scan ORDER BY bar DESC LIMIT 1", &[]).await?;
    let row = |r: &tokio_postgres::Row| {
        json!({
            "levels": r.get::<_, Option<Value>>(16),
            "deltaDivergence": r.get::<_, Option<bool>>(17),
            "deltaAgainst": r.get::<_, Option<bool>>(18),
            "deltas": r.get::<_, Option<Value>>(19),
            // The limit entry, once the scan that first saw it has been weighed again.
            "limit": r.get::<_, Option<f64>>(20).map(|price| json!({
                "price": price,
                "setAt": r.get::<_, i64>(9),
                "until": r.get::<_, Option<i64>>(21),
                "levels": r.get::<_, Option<Value>>(22),
                "filledAt": r.get::<_, Option<i64>>(23),
                "entry": r.get::<_, Option<f64>>(24),
                "outcome": r.get::<_, Option<String>>(25),
                "ret": r.get::<_, Option<f64>>(26),
            })),
            "endedWhy": r.get::<_, Option<String>>(27),
            "timeframe": r.get::<_, String>(28),
            "id": r.get::<_, i64>(0),
            "market": r.get::<_, String>(1),
            "side": r.get::<_, String>(2),
            "drives": r.get::<_, Value>(3),
            "turnedAt": r.get::<_, Option<i64>>(4),
            "turnPrice": r.get::<_, Option<f64>>(5),
            "score": r.get::<_, i32>(6),
            "divergence": r.get::<_, bool>(7),
            "price": r.get::<_, f64>(8),
            "foundAt": r.get::<_, i64>(9),
            "target": r.get::<_, Option<f64>>(10),
            "stop": r.get::<_, Option<f64>>(11),
            "entry": r.get::<_, Option<f64>>(12),
            "outcome": r.get::<_, Option<String>>(13),
            "ret": r.get::<_, Option<f64>>(14),
            "lastSeen": r.get::<_, i64>(15),
        })
    };
    const COLUMNS: &str = "id, market, side, drives, turned_at, turn_price, score, divergence, price, found_bar, target, stop, entry, outcome, ret, seen_bar, levels, delta_divergence, delta_against, deltas,
        limit_price, limit_until, limit_levels, limit_at, limit_entry, limit_outcome, limit_ret, ended_why, timeframe";
    let (signals, ended, touches) = match &scan {
        Some(scan) => {
            let bar: i64 = scan.get(0);
            // Price at a line whose trade is still open, while the scans keep the
            // line; and those whose trade ended within the last day, for Ended.
            let touches = client
                .query(
                    "SELECT market, side, drives, atr, clear, until_at, touched_at, touch_price, levels, deltas, outcome, ret, outcome_at FROM signal_setup
                     WHERE touched_at IS NOT NULL AND ((seen_bar = $1 AND outcome IS NULL) OR outcome_at >= $1 - $2) ORDER BY touched_at DESC",
                    &[&bar, &(6 * STEP)],
                )
                .await?
                .iter()
                .filter_map(|r| {
                    let setup = setup_of(&r.get::<_, String>(1), &r.get(2), r.get(3), r.get(4), r.get(5), None)?;
                    let at: i64 = r.get(6);
                    let pushes: Option<Value> = r.get(9);
                    Some(json!({
                        "outcome": r.get::<_, Option<String>>(10),
                        "ret": r.get::<_, Option<f64>>(11),
                        "outcomeAt": r.get::<_, Option<i64>>(12),
                        "market": r.get::<_, String>(0),
                        "side": setup.side.id(),
                        "drives": r.get::<_, Value>(2),
                        "touchedAt": at,
                        "touchPrice": r.get::<_, f64>(7),
                        "line": setup.line(at),
                        "lineNow": setup.line(now),
                        "levels": r.get::<_, Option<Value>>(8),
                        "deltaDivergence": pushes.as_ref().and_then(|p| weakened(setup.side, p)),
                        "deltas": pushes,
                    }))
                })
                .collect::<Vec<_>>();
            // The day's and the week's first: the strong kind.
            let now = client
                .query(&format!("SELECT {COLUMNS} FROM signal WHERE seen_bar = $1 ORDER BY timeframe = '4h', turned_at IS NULL, score DESC, market"), &[&bar])
                .await?;
            // Gone from the list within the last day, turned or ended by price: the reader saw them, and should see why they went.
            let gone = client
                .query(
                    &format!(
                        "SELECT {COLUMNS} FROM signal WHERE seen_bar < $1 AND seen_bar >= $1 - $2 AND (turned_at IS NOT NULL OR ended_why IN ('played', 'outdone', 'broken'))
                         ORDER BY seen_bar DESC, score DESC"
                    ),
                    &[&bar, &(6 * STEP)],
                )
                .await?;
            (now.iter().map(row).collect::<Vec<_>>(), gone.iter().map(row).collect::<Vec<_>>(), touches)
        }
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let (ended_touches, touches): (Vec<Value>, Vec<Value>) = touches.into_iter().partition(|t| !t["outcome"].is_null());
    Ok(Json(json!({
        "scannedAt": scan.as_ref().map(|s| s.get::<_, i64>(1)),
        "bar": scan.as_ref().map(|s| s.get::<_, i64>(0)),
        "markets": scan.as_ref().map_or(0, |s| s.get::<_, i32>(2)),
        "nextAt": settled(now) + STEP + SETTLE_MS,
        "timeframe": "4h",
        "notify": notify,
        "notifyTouch": notify_touch,
        "notifyLimits": notify_limits,
        "telegram": state.telegram.is_some(),
        "linked": linked,
        "signals": signals,
        "ended": ended,
        "touches": touches,
        "endedTouches": ended_touches,
    })))
}

/// One pattern's result, as the record counts it.
struct Played {
    side: String,
    score: i32,
    divergence: bool,
    delta_divergence: Option<bool>,
    delta_against: Option<bool>,
    outcome: String,
    ret: f64,
    best: f64,
    worst: f64,
}

/// How many take-profits an outcome took.
fn takes(outcome: &str) -> usize {
    match outcome {
        "tp3" => 3,
        "tp2" => 2,
        "tp1" => 1,
        _ => 0,
    }
}

/// The record of one kind by side, and within a side by score and by
/// divergence: how many, how many closed in profit, how many reached each
/// take-profit after the entry, the stop before the first or neither, and the
/// average result over the whole position, with the furthest price went
/// either way. Limits price never came back to count only as `seen`.
fn summarize(played: &[Played], kind: &str) -> Vec<Value> {
    let groups: [&str; 8] = if kind == "limit" { ["all", "55", "40", "u40", "div", "ddiv", "55ddiv", "dagainst"] } else { ["all", "70", "50", "0", "div", "ddiv", "70ddiv", "dagainst"] };
    let mut out = Vec::new();
    for side in ["bear", "bull"] {
        for group in groups {
            let cut: Vec<&Played> = played.iter().filter(|p| p.side == side && groups_of(kind, p.score, p.divergence, p.delta_divergence, p.delta_against).contains(&group)).collect();
            let rows: Vec<&Played> = cut.iter().copied().filter(|p| p.outcome != "missed").collect();
            let n = rows.len();
            let count = |o: &str| rows.iter().filter(|p| p.outcome == o).count();
            let reached = |tp: usize| rows.iter().filter(|p| takes(&p.outcome) >= tp).count();
            let mean = |f: fn(&Played) -> f64| if n == 0 { 0.0 } else { rows.iter().map(|p| f(p)).sum::<f64>() / n as f64 };
            out.push(json!({
                "side": side,
                "group": group,
                "seen": cut.len(),
                "n": n,
                "win": rows.iter().filter(|p| p.ret > 0.0).count(),
                "tp1": reached(1),
                "tp2": reached(2),
                "tp3": reached(3),
                "stop": count("stop"),
                "expired": count("expired"),
                "ret": mean(|p| p.ret),
                "best": mean(|p| p.best),
                "worst": mean(|p| p.worst),
            }));
        }
    }
    out
}

/// One touch's result, as the record counts it.
struct Reached {
    side: String,
    became: String,
    outcome: String,
    ret: f64,
    best: f64,
    worst: f64,
}

/// The touches by side: how many, how many became three drives and turned,
/// target or breakout first or neither in four days, and the average move.
fn summarize_touches(reached: &[Reached]) -> Vec<Value> {
    ["bear", "bull"]
        .iter()
        .map(|side| {
            let rows: Vec<&Reached> = reached.iter().filter(|r| r.side == *side).collect();
            let n = rows.len();
            let count = |f: &dyn Fn(&Reached) -> bool| rows.iter().filter(|r| f(r)).count();
            let mean = |f: fn(&Reached) -> f64| if n == 0 { 0.0 } else { rows.iter().map(|r| f(r)).sum::<f64>() / n as f64 };
            json!({
                "side": side,
                "n": n,
                "pattern": count(&|r| r.became != "none"),
                "turned": count(&|r| r.became == "turned"),
                "win": count(&|r| r.ret > 0.0),
                "tp1": count(&|r| takes(&r.outcome) >= 1),
                "tp2": count(&|r| takes(&r.outcome) >= 2),
                "tp3": count(&|r| takes(&r.outcome) >= 3),
                "stop": count(&|r| r.outcome == "stop"),
                "expired": count(&|r| r.outcome == "expired"),
                "ret": mean(|r| r.ret),
                "best": mean(|r| r.best),
                "worst": mean(|r| r.worst),
            })
        })
        .collect()
}

/// `GET /api/signals/stats`: the record — the latest replay of the history,
/// the live patterns since the screener began, and the latest of those.
pub async fn stats(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    owner(&headers)?;
    let client = state.pool.get().await?;
    let played = |r: &tokio_postgres::Row| Played {
        side: r.get(0),
        score: r.get(1),
        divergence: r.get(2),
        outcome: r.get(3),
        ret: r.get(4),
        best: r.get(5),
        worst: r.get(6),
        delta_divergence: r.get(7),
        delta_against: r.get(8),
    };
    let backtest = match client.query_opt("SELECT at, version, markets, from_at, to_at, variants FROM signal_backtest_run ORDER BY at DESC LIMIT 1", &[]).await? {
        Some(run) => {
            let at: i64 = run.get(0);
            let rows = client.query("SELECT side, score, divergence, outcome, ret, best, worst, delta_divergence, delta_against, kind FROM signal_backtest WHERE run = $1", &[&at]).await?;
            let of = |kind: &str| rows.iter().filter(|r| r.get::<_, String>(9) == kind).map(played).collect::<Vec<_>>();
            json!({
                "at": at,
                "version": run.get::<_, i32>(1),
                "markets": run.get::<_, i32>(2),
                "from": run.get::<_, i64>(3),
                "to": run.get::<_, i64>(4),
                "variants": run.get::<_, Option<Value>>(5),
                "groups": summarize(&of("pattern"), "pattern"),
                "limits": summarize(&of("limit"), "limit"),
            })
        }
        None => Value::Null,
    };
    let reached = |r: &tokio_postgres::Row| Reached { side: r.get(0), became: r.get(1), outcome: r.get(2), ret: r.get(3), best: r.get(4), worst: r.get(5) };
    let touches_replayed = match backtest["at"].as_i64() {
        Some(run) => summarize_touches(&client.query("SELECT side, became, outcome, ret, best, worst FROM signal_touch_backtest WHERE run = $1", &[&run]).await?.iter().map(reached).collect::<Vec<_>>()),
        None => Vec::new(),
    };
    let touches_live = client.query("SELECT side, became, outcome, ret, best, worst FROM signal_setup WHERE outcome IS NOT NULL", &[]).await?;
    let touches_since = client.query_one("SELECT min(touched_at), count(*) FILTER (WHERE outcome IS NULL) FROM signal_setup WHERE touched_at IS NOT NULL", &[]).await?;
    let touches_recent = client
        .query(
            "SELECT market, side, drives, touched_at, touch_price, target, became, outcome, ret, best, worst FROM signal_setup
             WHERE touched_at IS NOT NULL ORDER BY touched_at DESC LIMIT 40",
            &[],
        )
        .await?
        .iter()
        .map(|r| {
            json!({
                "market": r.get::<_, String>(0),
                "side": r.get::<_, String>(1),
                "drives": r.get::<_, Value>(2),
                "touchedAt": r.get::<_, i64>(3),
                "touchPrice": r.get::<_, f64>(4),
                "target": r.get::<_, Option<f64>>(5),
                "became": r.get::<_, Option<String>>(6),
                "outcome": r.get::<_, Option<String>>(7),
                "ret": r.get::<_, Option<f64>>(8),
                "best": r.get::<_, Option<f64>>(9),
                "worst": r.get::<_, Option<f64>>(10),
            })
        })
        .collect::<Vec<_>>();
    // The four hours' record, and the day's and the week's apart: each weighed over its own bars.
    let decided = client
        .query("SELECT side, score, divergence, outcome, ret, best, worst, delta_divergence, delta_against FROM signal WHERE outcome IS NOT NULL AND timeframe = '4h'", &[])
        .await?;
    let since = client.query_one("SELECT min(entry_at), count(*) FILTER (WHERE outcome IS NULL) FROM signal WHERE entry_at IS NOT NULL AND timeframe = '4h'", &[]).await?;
    let higher_decided = client
        .query("SELECT side, score, divergence, outcome, ret, best, worst, delta_divergence, delta_against FROM signal WHERE outcome IS NOT NULL AND timeframe <> '4h'", &[])
        .await?;
    let higher_since = client.query_one("SELECT min(entry_at), count(*) FILTER (WHERE outcome IS NULL) FROM signal WHERE entry_at IS NOT NULL AND timeframe <> '4h'", &[]).await?;
    // Limits, the score before the turn as the scan that set them saw it.
    let limits_decided = client
        .query(
            "SELECT side, score - CASE WHEN turned_at IS NULL THEN 0 ELSE 15 END, divergence, limit_outcome, COALESCE(limit_ret, 0), COALESCE(limit_best, 0), COALESCE(limit_worst, 0), delta_divergence, delta_against
             FROM signal WHERE limit_outcome IS NOT NULL AND found_bar >= $1 AND timeframe = '4h'",
            &[&LIMIT_SINCE],
        )
        .await?;
    let limits_open = client
        .query_one("SELECT count(*) FROM signal WHERE limit_entry IS NOT NULL AND limit_outcome IS NULL AND found_bar >= $1 AND timeframe = '4h'", &[&LIMIT_SINCE])
        .await?;
    let recent = client
        .query(
            "SELECT id, market, side, drives, score, divergence, entry_at, entry, target, stop, outcome, outcome_at, ret, best, worst, timeframe FROM signal
             WHERE entry_at IS NOT NULL ORDER BY entry_at DESC, score DESC LIMIT 60",
            &[],
        )
        .await?
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, i64>(0),
                "market": r.get::<_, String>(1),
                "side": r.get::<_, String>(2),
                "drives": r.get::<_, Value>(3),
                "score": r.get::<_, i32>(4),
                "divergence": r.get::<_, bool>(5),
                "entryAt": r.get::<_, i64>(6),
                "entry": r.get::<_, Option<f64>>(7),
                "target": r.get::<_, Option<f64>>(8),
                "stop": r.get::<_, Option<f64>>(9),
                "outcome": r.get::<_, Option<String>>(10),
                "outcomeAt": r.get::<_, Option<i64>>(11),
                "ret": r.get::<_, Option<f64>>(12),
                "best": r.get::<_, Option<f64>>(13),
                "worst": r.get::<_, Option<f64>>(14),
                "timeframe": r.get::<_, String>(15),
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "version": VERSION,
        "windowBars": drives::OUTCOME_BARS,
        "backtest": backtest,
        "live": {
            "since": since.get::<_, Option<i64>>(0),
            "open": since.get::<_, i64>(1),
            "groups": summarize(&decided.iter().map(played).collect::<Vec<_>>(), "pattern"),
        },
        "higher": {
            "since": higher_since.get::<_, Option<i64>>(0),
            "open": higher_since.get::<_, i64>(1),
            "groups": summarize(&higher_decided.iter().map(played).collect::<Vec<_>>(), "pattern"),
        },
        "limits": {
            "since": LIMIT_SINCE,
            "open": limits_open.get::<_, i64>(0),
            "groups": summarize(&limits_decided.iter().map(played).collect::<Vec<_>>(), "limit"),
        },
        "recent": recent,
        "touches": {
            "replayed": touches_replayed,
            "live": {
                "since": touches_since.get::<_, Option<i64>>(0),
                "open": touches_since.get::<_, i64>(1),
                "groups": summarize_touches(&touches_live.iter().map(reached).collect::<Vec<_>>()),
            },
            "recent": touches_recent,
        },
    })))
}

/// `PUT /api/signals/notify` `{ "notify": true, "touch": false, "limits": true }`:
/// whether this reader is told on Telegram when a pattern turns, when price
/// reaches a setup's line, and of the limit entries a scan sets. Any may be
/// left out to keep it as it is.
pub async fn set_notify(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let (notify, touch, limits) = (body["notify"].as_bool(), body["touch"].as_bool(), body["limits"].as_bool());
    if notify.is_none() && touch.is_none() && limits.is_none() {
        return Err(ApiError::bad_request("notify, touch or limits must be true or false"));
    }
    let row = state
        .pool
        .get()
        .await?
        .query_one(
            "INSERT INTO signal_watch (owner, notify, touch, limits) VALUES ($1, COALESCE($2, FALSE), COALESCE($3, FALSE), COALESCE($4, FALSE))
             ON CONFLICT (owner) DO UPDATE SET notify = COALESCE($2, signal_watch.notify), touch = COALESCE($3, signal_watch.touch), limits = COALESCE($4, signal_watch.limits)
             RETURNING notify, touch, limits",
            &[&owner, &notify, &touch, &limits],
        )
        .await?;
    Ok(Json(json!({ "notify": row.get::<_, bool>(0), "touch": row.get::<_, bool>(1), "limits": row.get::<_, bool>(2) })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scan_reads_up_to_the_bar_that_has_settled() {
        let close = 1_790_928_000_000; // a four-hour boundary
        assert_eq!(close % STEP, 0);
        assert_eq!(settled(close + 30_000), close - STEP, "half a minute after a close, that bar is still settling");
        assert_eq!(settled(close + SETTLE_MS), close);
        assert_eq!(settled(close + STEP - 1), close);
    }

    #[test]
    fn the_record_counts_each_side_by_score_and_divergence() {
        let p = |side: &str, score: i32, divergence: bool, outcome: &str, ret: f64| Played { side: side.into(), score, divergence, outcome: outcome.into(), ret, best: ret.max(0.0), worst: ret.min(0.0), delta_divergence: None, delta_against: None };
        let played = [p("bear", 80, true, "tp3", 5.25), p("bear", 75, false, "stop", -4.0), p("bear", 40, true, "expired", 1.0), p("bear", 60, false, "tp1", 1.5), p("bull", 55, false, "tp2", 3.0)];
        let groups = summarize(&played, "pattern");
        let find = |side: &str, group: &str| groups.iter().find(|g| g["side"] == side && g["group"] == group).unwrap().clone();
        let all = find("bear", "all");
        assert_eq!((all["n"].as_u64(), all["tp1"].as_u64(), all["tp2"].as_u64(), all["tp3"].as_u64()), (Some(4), Some(2), Some(1), Some(1)), "a take-profit counts every one before it");
        assert_eq!((all["stop"].as_u64(), all["expired"].as_u64()), (Some(1), Some(1)));
        assert!((all["ret"].as_f64().unwrap() - 3.75 / 4.0).abs() < 1e-9);
        assert_eq!(find("bear", "70")["n"].as_u64(), Some(2));
        assert_eq!(find("bear", "div")["tp3"].as_u64(), Some(1));
        assert_eq!(find("bull", "50")["n"].as_u64(), Some(1));
        assert_eq!(find("bull", "70")["n"].as_u64(), Some(0));
    }

    #[test]
    fn the_touches_count_what_they_became_and_how_they_went() {
        let r = |side: &str, became: &str, outcome: &str, ret: f64| Reached { side: side.into(), became: became.into(), outcome: outcome.into(), ret, best: 0.0, worst: 0.0 };
        let groups = summarize_touches(&[r("bear", "turned", "tp2", 3.0), r("bear", "pattern", "stop", -3.5), r("bear", "none", "expired", 0.5), r("bull", "none", "stop", -2.0)]);
        let bear = &groups[0];
        assert_eq!((bear["n"].as_u64(), bear["pattern"].as_u64(), bear["turned"].as_u64()), (Some(3), Some(2), Some(1)));
        assert_eq!((bear["tp1"].as_u64(), bear["tp2"].as_u64(), bear["tp3"].as_u64(), bear["stop"].as_u64()), (Some(1), Some(1), Some(0), Some(1)));
        assert!(bear["ret"].as_f64().unwrap().abs() < 1e-9);
        assert_eq!(groups[1]["n"].as_u64(), Some(1));
    }

    #[test]
    fn the_plans_are_tallied_by_kind_side_and_group_and_by_half() {
        let o = |result: Hit, ret: f64| drives::Outcome { result, bars: 1, ret, best: 0.0, worst: 0.0 };
        let point = Point { index: 0, at: 0, price: 1.0 };
        let found = |score: u32, divergence: bool, delta_divergence: Option<bool>| Found {
            side: Side::Bear,
            drives: [point; 3],
            turn: None,
            divergence,
            score,
            extreme: 1.0,
            pullback: 1.0,
            atr: 1.0,
            delta_divergence,
            delta_against: None,
            pushes: None,
        };
        let mut plans = Plans::default();
        plans.add(Plan::Fib, "pattern", Side::Bear, Some(&found(80, true, Some(true))), false, &o(Hit::Tp2, 3.0));
        plans.add(Plan::Fib, "pattern", Side::Bear, Some(&found(40, false, None)), true, &o(Hit::Stop, -4.0));
        plans.add(Plan::Fib, "pattern", Side::Bear, Some(&found(40, false, Some(false))), true, &o(Hit::Open, 0.0));
        plans.add(Plan::Fixed, "touch", Side::Bull, None, false, &o(Hit::Tp3, 5.0));
        let rows = plans.json();
        let find = |plan: &str, kind: &str, side: &str, group: &str| rows.as_array().unwrap().iter().find(|r| r["plan"] == plan && r["kind"] == kind && r["side"] == side && r["group"] == group).cloned();
        let all = find("fib", "pattern", "bear", "all").unwrap();
        assert_eq!((all["n"].as_u64(), all["tp1"].as_u64(), all["tp2"].as_u64(), all["stop"].as_u64()), (Some(2), Some(1), Some(1), Some(1)), "an open trade is not counted");
        assert_eq!(all["halves"], json!([3.0, -4.0]));
        assert_eq!(find("fib", "pattern", "bear", "70").unwrap()["n"].as_u64(), Some(1));
        assert_eq!(find("fib", "pattern", "bear", "div").unwrap()["n"].as_u64(), Some(1));
        assert_eq!(find("fib", "pattern", "bear", "70ddiv").unwrap()["n"].as_u64(), Some(1), "score 70+ with delta divergence");
        assert!(find("fib", "pattern", "bear", "dagainst").is_none());
        assert_eq!(find("fixed", "touch", "bull", "all").unwrap()["tp3"].as_u64(), Some(1));
        assert!(find("fixed", "touch", "bull", "70").is_none(), "a touch has no score");
        assert_eq!(rows[0]["plan"], "fixed", "in the plans' own order");
    }

    #[test]
    fn a_limit_reads_as_a_message() {
        let limit = |filled: bool| Limit {
            market: "binance-perp:AAVEUSDT".into(),
            score: 58,
            divergence: true,
            delta_divergence: true,
            price: 180.0,
            limit: 184.5,
            until: 30 * STEP,
            levels: Some(drives::Levels { tp: [176.0, 170.0, 160.0], sl: 193.6 }),
            filled,
            frame: Frame::H4,
            link: Some("https://tradoor.example/?signal=7".into()),
        };
        let text = limit_text(&[limit(false)], 24 * STEP, 0);
        assert!(text.starts_with("🎯 Three drives — 1 limit entry\n"), "{text}");
        assert!(text.contains("🔴 AAVEUSDT (Binance Perp) · 4H — bearish, score 58 · RSI and delta divergence\n   limit sell 184.50 · now 180.00 · until "), "{text}");
        assert!(text.contains("   TP 176.00 / 170.00 / 160.00 · SL 193.60\n   https://tradoor.example/?signal=7\n"), "{text}");
        assert!(limit_text(&[limit(true)], 24 * STEP, 0).contains("· price is past it: fills at once"));
    }

    #[test]
    fn a_touch_reads_as_a_message() {
        let setup = Setup {
            side: Side::Bear,
            drives: [Point { index: 0, at: 0, price: 0.0266 }, Point { index: 0, at: 100 * STEP, price: 0.0287 }],
            atr: 0.0005,
            clear: 0.02895,
            until: 300 * STEP,
            pullback: 0.027,
            pushes: None,
        };
        let lv = touch_levels(&setup, 0.02925);
        let text = touch_text(&[("binance-perp:YGGUSDT".into(), setup, 0.02925, lv)], 130 * STEP, 7 * 60, |market, at| Some(format!("https://tradoor.example/?market={market}&tf=4h&touched={at}")));
        assert!(text.contains(&format!("   https://tradoor.example/?market=binance-perp:YGGUSDT&tf=4h&touched={}\n", 130 * STEP)), "{text}");
        assert!(text.starts_with("📐 Three drives · 4h — 1 at the line\n"), "{text}");
        assert!(text.contains("🔴 YGGUSDT (Binance Perp) — bearish: a third high at the line\n   drives 0.026600 → 0.028700 · line 0.029330 · now 0.029250"), "{text}");
        assert!(text.contains("· SL 0.030000\n"), "the stop an ATR and a half over the touch: {text}");
        let tp1 = lv.unwrap().tp[0];
        assert!((tp1 - (0.02925 - 0.382 * 0.00225)).abs() < 1e-12, "the first take-profit 38.2% back down the leg from the pullback");
        assert!(text.contains("Not confirmed until the four-hour bars close"), "{text}");
    }

    #[test]
    fn a_turn_reads_as_a_message() {
        let turned = [
            Told {
                market: "binance-perp:LINKUSDT".into(),
                bull: false,
                drives: vec![13.6, 14.4, 15.8],
                score: 86,
                divergence: true,
                price: 15.1,
                levels: Some(drives::Levels { tp: [14.647, 14.194, 13.741], sl: 16.353 }),
                delta_divergence: false,
                frame: Frame::D1,
                link: Some("https://tradoor.example/?signal=12".into()),
            },
            Told { market: "bybit-perp:ZECUSDT".into(), bull: true, drives: vec![1580.0, 1520.0, 1490.0], score: 70, divergence: false, price: 1510.0, levels: None, delta_divergence: true, frame: Frame::H4, link: None },
        ];
        let text = signal_text(&turned, 1_790_928_000_000, 7 * 60);
        assert!(text.starts_with("🔔 Three drives — 2 turned\n"), "{text}");
        assert!(text.contains("🔴 LINKUSDT (Binance Perp) · 1D ⭐ — bearish, score 86 · RSI divergence\n   drives 13.600 → 14.400 → 15.800 · now 15.100"), "{text}");
        assert!(text.contains("   TP 14.647 / 14.194 / 13.741 · SL 16.353\n   https://tradoor.example/?signal=12\n"), "the link that opens it: {text}");
        assert!(text.contains("🟢 ZECUSDT (Bybit Perp) · 4H — bullish, score 70 · delta divergence\n   drives 1,580.00 → 1,520.00 → 1,490.00"), "{text}");
        assert!(text.contains("   TP 14.647 / 14.194 / 13.741 · SL 16.353\n"), "the trade's own prices: {text}");
        assert!(text.ends_with("UTC+7"), "{text}");
    }
}
