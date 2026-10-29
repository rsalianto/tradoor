//! A footprint's history: hours of volume by price, kept for good.
//!
//! Minutes are trimmed after a month (footprint.rs). Before they go, each hour
//! of them is added up into a row of its own, and hours are what a chart of
//! hours or days is built from — a day of minutes is fourteen hundred rows and
//! a day of hours twenty-four.
//!
//! Hours from before the server recorded are read from Binance's archive of
//! every trade (data.binance.vision, aggTrades). `TRADOOR_FOOTPRINT_HISTORY`
//! names a market and a first day — `binance-perp:BTCUSDT@2026-01-01` — and the
//! server reads each month since, then each day of this one: one file at a
//! time, downloaded beside the data, checked against its checksum, added up,
//! deleted, and marked read so a restart carries on. An archived hour stands
//! over one added up from recorded minutes: the archive misses no trade, and a
//! recorder that restarted did.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use tradoor_store::now_ms;
use tradoor_venues::{MarketRef, Venue};

use crate::api::{ApiError, ApiResult, AppState};
use crate::dns::Doh;
use crate::footprint::{tick_of, unpack, Bar};
use crate::liquidations::Answer;

const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;
const DAY: i64 = 24 * HOUR;
/// How often finished hours of recorded minutes are added up.
const ROLL_EVERY: Duration = Duration::from_secs(300);
/// How long after an hour ends its last minutes are surely written down.
const SETTLED: i64 = 5 * MINUTE;
/// How often the archive is looked at again for a day it did not have yet.
const ARCHIVE_EVERY: Duration = Duration::from_secs(6 * 3_600);
/// Bars one request may ask for.
const MAX_BARS: i64 = 1_000;
/// Rows a typical bar is sent at most, when the chart fits them to its zoom.
const MAX_FINE_ROWS: i64 = 200;
/// The minutes after the newest hour row a chart is sent as well, so the bar
/// still forming is whole; past this the hours will have caught up.
const RECENT: i64 = 6 * HOUR;

/// How far an hour's delta swung from the hour's open, read at each minute's
/// close, in coins and in dollars: the most it reached, the least, and where
/// it ended.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Swing {
    pub delta: f64,
    pub up: f64,
    pub down: f64,
    pub delta_usd: f64,
    pub up_usd: f64,
    pub down_usd: f64,
}

impl Swing {
    /// A minute closed, having traded `delta` coins net, worth `usd`.
    fn minute(&mut self, delta: f64, usd: f64) {
        self.delta += delta;
        self.delta_usd += usd;
        self.up = self.up.max(self.delta);
        self.down = self.down.min(self.delta);
        self.up_usd = self.up_usd.max(self.delta_usd);
        self.down_usd = self.down_usd.min(self.delta_usd);
    }
}

/// An hour being added up, a trade or a level at a time, its minutes in order.
#[derive(Default)]
struct Hour {
    bar: Bar,
    swing: Swing,
    minute: Option<i64>,
    net: f64,
    net_usd: f64,
}

impl Hour {
    fn add(&mut self, minute: i64, level: i64, price: f64, buy: f64, sell: f64) {
        if self.minute != Some(minute) {
            self.close_minute();
            self.minute = Some(minute);
        }
        let at = self.bar.levels.entry(level).or_insert((0.0, 0.0));
        at.0 += buy;
        at.1 += sell;
        self.net += buy - sell;
        self.net_usd += (buy - sell) * price;
    }

    fn close_minute(&mut self) {
        if self.minute.is_some() {
            self.swing.minute(self.net, self.net_usd);
            self.net = 0.0;
            self.net_usd = 0.0;
        }
    }

    fn finish(mut self) -> (Bar, Swing) {
        self.close_minute();
        (self.bar, self.swing)
    }
}

pub fn start(pool: deadpool_postgres::Pool, doh: &Arc<Doh>, data_dir: PathBuf, history: Vec<String>) -> anyhow::Result<()> {
    tokio::spawn(roll(pool.clone()));
    let asked: Vec<(String, i64)> = history.iter().map(|h| parse_history(h)).collect::<anyhow::Result<_>>()?;
    if asked.is_empty() {
        return Ok(());
    }
    let http = reqwest::Client::builder()
        .dns_resolver(doh.resolver())
        .user_agent(concat!("tradoor/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(1_800))
        .build()?;
    tracing::info!(?asked, "reading footprint history from the archive");
    tokio::spawn(async move {
        loop {
            for (market, from) in &asked {
                if let Err(e) = archive_market(&pool, &http, &data_dir, market, *from).await {
                    tracing::warn!(market, error = %e, "reading footprint history");
                }
            }
            sleep(ARCHIVE_EVERY).await;
        }
    });
    Ok(())
}

/// `binance-perp:BTCUSDT@2026-01-01`: a recorded market on Binance, and its first day.
fn parse_history(raw: &str) -> anyhow::Result<(String, i64)> {
    let (market, day) = raw.trim().split_once('@').ok_or_else(|| anyhow!("TRADOOR_FOOTPRINT_HISTORY: {raw:?} is not market@YYYY-MM-DD"))?;
    let parsed = MarketRef::parse(market).ok_or_else(|| anyhow!("TRADOOR_FOOTPRINT_HISTORY: {market:?} is not a market"))?;
    if !matches!(parsed.venue, Venue::BinancePerp | Venue::BinanceSpot) || tick_of(market).is_none() {
        bail!("TRADOOR_FOOTPRINT_HISTORY: {market} is not a Binance market whose footprint is recorded");
    }
    Ok((market.to_string(), day_ms(day).ok_or_else(|| anyhow!("TRADOOR_FOOTPRINT_HISTORY: {day:?} is not YYYY-MM-DD"))?))
}

// ---------------------------------------------------------------- recorded minutes into hours

/// Every finished hour of recorded minutes, added up into its own row, the
/// oldest first: at the start the month the minutes cover, then each hour as
/// it ends.
async fn roll(pool: deadpool_postgres::Pool) {
    let mut since = now_ms() - 32 * DAY;
    loop {
        match roll_round(&pool, since).await {
            Ok(Some(next)) => since = next,
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "adding up footprint hours"),
        }
        sleep(ROLL_EVERY).await;
    }
}

/// One round: the hours since `since` that have minutes but no row. Returns
/// where the next round starts: two hours back from the last finished one, in
/// case a late minute landed in it.
async fn roll_round(pool: &deadpool_postgres::Pool, since: i64) -> anyhow::Result<Option<i64>> {
    let until = (now_ms() - SETTLED).div_euclid(HOUR) * HOUR;
    let client = pool.get().await?;
    let missing = client
        .query(
            "SELECT DISTINCT m.market, m.minute - m.minute % $3 AS hour FROM footprint_minute m
             WHERE m.minute >= $1 AND m.minute < $2
               AND NOT EXISTS (SELECT 1 FROM footprint_hour h WHERE h.market = m.market AND h.hour = m.minute - m.minute % $3)
             ORDER BY hour",
            &[&since, &until, &HOUR],
        )
        .await?;
    for row in &missing {
        let (market, hour): (String, i64) = (row.get(0), row.get(1));
        let Some(tick) = tick_of(&market) else { continue };
        let minutes = client
            .query("SELECT minute, base, levels, trades FROM footprint_minute WHERE market = $1 AND minute >= $2 AND minute < $3 ORDER BY minute", &[&market, &hour, &(hour + HOUR)])
            .await?;
        let mut sum = Hour::default();
        for m in &minutes {
            let minute: i64 = m.get(0);
            for (level, buy, sell) in unpack(m.get(1), m.get(2)) {
                sum.add(minute, level, level as f64 * tick, buy, sell);
            }
            sum.bar.trades += m.get::<_, i64>(3);
        }
        let (bar, swing) = sum.finish();
        write_hour(&client, &market, hour, tick, &bar, &swing, false).await?;
    }
    Ok(Some((until - 2 * HOUR).max(since)))
}

/// An hour's row. One read from the archive replaces whatever was there; one
/// added up from minutes never replaces an archived one.
async fn write_hour(client: &tokio_postgres::Client, market: &str, hour: i64, tick: f64, bar: &Bar, swing: &Swing, archived: bool) -> anyhow::Result<()> {
    let (base, bytes) = bar.pack();
    let sql = if archived {
        "INSERT INTO footprint_hour (market, hour, tick, base, levels, trades, swing_up, swing_down, swing_up_usd, swing_down_usd, archived)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, true)
         ON CONFLICT (market, hour) DO UPDATE SET tick = EXCLUDED.tick, base = EXCLUDED.base, levels = EXCLUDED.levels, trades = EXCLUDED.trades,
             swing_up = EXCLUDED.swing_up, swing_down = EXCLUDED.swing_down, swing_up_usd = EXCLUDED.swing_up_usd, swing_down_usd = EXCLUDED.swing_down_usd, archived = true"
    } else {
        "INSERT INTO footprint_hour (market, hour, tick, base, levels, trades, swing_up, swing_down, swing_up_usd, swing_down_usd, archived)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, false)
         ON CONFLICT (market, hour) DO NOTHING"
    };
    client.execute(sql, &[&market, &hour, &tick, &base, &bytes, &bar.trades, &swing.up, &swing.down, &swing.up_usd, &swing.down_usd]).await?;
    Ok(())
}

// ---------------------------------------------------------------- the archive

/// A month of the archive, or a day of the month still running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Period {
    Month(i32, u32),
    Day(i32, u32, u32),
}

impl Period {
    fn name(self) -> String {
        match self {
            Self::Month(y, m) => format!("{y}-{m:02}"),
            Self::Day(y, m, d) => format!("{y}-{m:02}-{d:02}"),
        }
    }
}

/// The archive's periods from `from` until yesterday: whole months while they
/// are over, then the days of this one. A month the archive has not packed
/// yet is read by its days instead (see `archive_market`).
fn periods(from: i64, now: i64) -> Vec<Period> {
    let (today, _) = ymd(now);
    let (start, _) = ymd(from);
    let (mut y, mut m) = (start.0, start.1);
    let mut out = Vec::new();
    while (y, m) < (today.0, today.1) {
        out.push(Period::Month(y, m));
        (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    }
    // This month from its first day, or from the day asked for when that is in it.
    let first = if (start.0, start.1) == (today.0, today.1) { start.2 } else { 1 };
    for d in first..today.2 {
        out.push(Period::Day(today.0, today.1, d));
    }
    out
}

/// Every day of a month up to yesterday, for one the archive has not packed yet.
fn days_of(y: i32, m: u32, now: i64) -> Vec<Period> {
    let today = now - now.rem_euclid(DAY);
    (1..=days_in(y, m)).filter(|d| day_ms(&format!("{y}-{m:02}-{d:02}")).is_some_and(|t| t < today)).map(|d| Period::Day(y, m, d)).collect()
}

fn archive_url(market: &str, period: Period) -> Option<String> {
    let m = MarketRef::parse(market)?;
    let segment = match m.venue {
        Venue::BinancePerp => "futures/um",
        Venue::BinanceSpot => "spot",
        _ => return None,
    };
    let (frequency, name) = match period {
        Period::Month(..) => ("monthly", period.name()),
        Period::Day(..) => ("daily", period.name()),
    };
    Some(format!("{}/data/{segment}/{frequency}/aggTrades/{s}/{s}-aggTrades-{name}.zip", tradoor_venues::binance::ARCHIVE, s = m.symbol))
}

/// Every period of a market not read yet, one at a time, the oldest first.
async fn archive_market(pool: &deadpool_postgres::Pool, http: &reqwest::Client, dir: &Path, market: &str, from: i64) -> anyhow::Result<()> {
    let tick = tick_of(market).ok_or_else(|| anyhow!("{market} is not recorded"))?;
    let done: std::collections::HashSet<String> = pool
        .get()
        .await?
        .query("SELECT period FROM footprint_archive WHERE market = $1", &[&market])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let mut queue: std::collections::VecDeque<Period> = periods(from, now_ms()).into_iter().filter(|p| !done.contains(&p.name())).collect();
    let mark = |period: Period, hours: i32| async move {
        pool.get()
            .await?
            .execute(
                "INSERT INTO footprint_archive (market, period, hours, read_at) VALUES ($1, $2, $3, $4) ON CONFLICT (market, period) DO UPDATE SET hours = EXCLUDED.hours, read_at = EXCLUDED.read_at",
                &[&market, &period.name(), &hours, &now_ms()],
            )
            .await?;
        anyhow::Ok(())
    };
    while let Some(period) = queue.pop_front() {
        // A month already read day by day, while it ran, is not read again whole.
        if let Period::Month(y, m) = period {
            let days = days_of(y, m, now_ms());
            if !days.is_empty() && days.len() == days_in(y, m) as usize && days.iter().all(|d| done.contains(&d.name())) {
                mark(period, 0).await?;
                continue;
            }
        }
        let url = archive_url(market, period).ok_or_else(|| anyhow!("{market} has no archive"))?;
        let started = std::time::Instant::now();
        let Some(file) = download(http, &url, dir).await? else {
            // A month not packed yet is there by its days; a day not out yet, next time.
            if let Period::Month(y, m) = period {
                for day in days_of(y, m, now_ms()).into_iter().rev() {
                    if !done.contains(&day.name()) {
                        queue.push_front(day);
                    }
                }
            }
            continue;
        };
        let read = tokio::task::spawn_blocking({
            let file = file.clone();
            move || read_archive(&file, tick)
        })
        .await
        .map_err(|e| anyhow!("reading {url}: {e}"));
        let _ = std::fs::remove_file(&file);
        let hours = read??;
        let client = pool.get().await?;
        for (hour, (bar, swing)) in &hours {
            write_hour(&client, market, *hour, tick, bar, swing, true).await?;
        }
        mark(period, hours.len() as i32).await?;
        tracing::info!(market, period = %period.name(), hours = hours.len(), secs = started.elapsed().as_secs(), "footprint history read");
    }
    Ok(())
}

/// The file at `url`, written beside the data and checked against the
/// checksum the archive publishes; None where the archive has no such file.
async fn download(http: &reqwest::Client, url: &str, dir: &Path) -> anyhow::Result<Option<PathBuf>> {
    let mut res = http.get(url).send().await.with_context(|| format!("asking for {url}"))?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !res.status().is_success() {
        bail!("{url}: {}", res.status());
    }
    let expected = http.get(format!("{url}.CHECKSUM")).send().await?.error_for_status()?.text().await?;
    let expected = expected.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    std::fs::create_dir_all(dir)?;
    let path = dir.join(url.rsplit('/').next().unwrap_or("archive.zip"));
    let mut out = std::fs::File::create(&path)?;
    let mut hash = Sha256::new();
    while let Some(chunk) = res.chunk().await? {
        hash.update(&chunk);
        out.write_all(&chunk)?;
    }
    out.flush()?;
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != expected {
        let _ = std::fs::remove_file(&path);
        bail!("{url}: checksum {got} is not the {expected} published");
    }
    Ok(Some(path))
}

/// An archive file's trades added up into hours. Its CSV is the aggregate
/// trades in order: id, price, quantity, first and last trade id, time, and
/// whether the buyer was the maker — the taker sold, then. Futures files open
/// with a header; spot files from 2025 on stamp times in microseconds.
fn read_archive(file: &Path, tick: f64) -> anyhow::Result<BTreeMap<i64, (Bar, Swing)>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(file)?)?;
    let entry = zip.by_index(0)?;
    let mut out = BTreeMap::new();
    let mut current: Option<(i64, Hour)> = None;
    for (n, line) in BufReader::with_capacity(1 << 20, entry).lines().enumerate() {
        let line = line?;
        let Some(trade) = parse_trade(&line) else {
            if n == 0 {
                continue;
            }
            bail!("{}: line {} is not a trade: {line:?}", file.display(), n + 1);
        };
        let hour = trade.at - trade.at.rem_euclid(HOUR);
        if current.as_ref().map(|(h, _)| *h) != Some(hour) {
            if let Some((h, sum)) = current.take() {
                out.insert(h, sum.finish());
            }
            current = Some((hour, Hour::default()));
        }
        let (_, sum) = current.as_mut().expect("set above");
        let (buy, sell) = if trade.buy { (trade.qty, 0.0) } else { (0.0, trade.qty) };
        sum.add(trade.at - trade.at.rem_euclid(MINUTE), (trade.price / tick).floor() as i64, trade.price, buy, sell);
        sum.bar.trades += trade.prints;
    }
    if let Some((h, sum)) = current.take() {
        out.insert(h, sum.finish());
    }
    Ok(out)
}

#[derive(Debug, PartialEq)]
struct Trade {
    at: i64,
    price: f64,
    qty: f64,
    buy: bool,
    prints: i64,
}

fn parse_trade(line: &str) -> Option<Trade> {
    let f: Vec<&str> = line.trim().split(',').collect();
    if f.len() < 7 {
        return None;
    }
    let (price, qty): (f64, f64) = (f[1].parse().ok()?, f[2].parse().ok()?);
    let (first, last): (i64, i64) = (f[3].parse().ok()?, f[4].parse().ok()?);
    let raw: i64 = f[5].parse().ok()?;
    let maker = match f[6].to_ascii_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    (price > 0.0 && qty > 0.0).then_some(Trade { at: if raw >= 100_000_000_000_000 { raw / 1_000 } else { raw }, price, qty, buy: !maker, prints: (last - first + 1).max(1) })
}

// ---------------------------------------------------------------- the chart's side

#[derive(Deserialize)]
pub struct Ask {
    market: String,
    from: i64,
    to: i64,
    /// The bar, in milliseconds: an hour, or a whole number of hours in a day.
    bar: i64,
    /// `usd` for dollars at each level's price; coins otherwise.
    unit: Option<String>,
    /// Ticks a row covers, or 0 to fit the rows to the chart.
    ticks: Option<i64>,
}

/// A chart's history as last sent, by what it asked: every chart of a market at
/// one bar asks the same of a minute, so one reading serves them all — and when
/// it was last asked for, so one nobody wants any more goes.
type Key = (String, i64, bool, i64, i64);
type Sent = Arc<tokio::sync::Mutex<Option<Answer>>>;
static SENT: OnceLock<std::sync::Mutex<HashMap<Key, (std::time::Instant, Sent)>>> = OnceLock::new();
/// How long a history as sent stands: the bar still forming moves, slowly.
const FRESH: Duration = Duration::from_secs(60);

/// `GET /api/footprint/history?market=&from=&to=&bar=`: the hours in that
/// span added up into bars, their levels grouped onto one grid — with each
/// bar's most and least delta from its open — and the minutes since the
/// newest hour, so the bar still forming is whole. A span reaching now, as an
/// open chart asks, is read once a minute for everyone who asks it.
pub async fn history(State(state): State<AppState>, headers: HeaderMap, Query(ask): Query<Ask>) -> ApiResult<Response> {
    let Some(tick) = tick_of(&ask.market) else {
        return Err(ApiError::bad_request("this server does not record that market's footprint"));
    };
    if ask.bar < HOUR || ask.bar % HOUR != 0 || DAY % ask.bar != 0 {
        return Err(ApiError::bad_request("a bar is an hour, or a whole number of hours that divides a day"));
    }
    let (from, to) = (ask.from.div_euclid(ask.bar) * ask.bar, ask.to);
    if to < from || (to - from) / ask.bar > MAX_BARS {
        return Err(ApiError::bad_request(format!("ask for at most {MAX_BARS} bars at a time")));
    }
    let usd = ask.unit.as_deref() == Some("usd");
    let ticks = ask.ticks.filter(|t| *t > 0).unwrap_or(0).min(1_000);
    let now = now_ms();
    if to < now - MINUTE {
        return Ok(Json(read(&state, &ask.market, tick, ask.bar, usd, ticks, from, to).await?).into_response());
    }
    let slot = {
        let mut sent = SENT.get_or_init(Default::default).lock().map_err(|_| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "the history cache is poisoned".into()))?;
        sent.retain(|_, (used, _)| used.elapsed() < 10 * FRESH);
        let entry = sent.entry((ask.market.clone(), ask.bar, usd, ticks, from)).or_insert_with(|| (std::time::Instant::now(), Sent::default()));
        entry.0 = std::time::Instant::now();
        entry.1.clone()
    };
    let mut held = slot.lock().await;
    if let Some(answer) = held.as_ref().filter(|a| a.at.elapsed() < FRESH) {
        return Ok(answer.clone().send(&headers));
    }
    let body = read(&state, &ask.market, tick, ask.bar, usd, ticks, from, now).await?;
    let answer = Answer::new(serde_json::to_vec(&body).map_err(|e| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?);
    *held = Some(answer.clone());
    Ok(answer.send(&headers))
}

#[allow(clippy::too_many_arguments)]
async fn read(state: &AppState, market: &str, tick: f64, bar_ms: i64, usd: bool, ticks: i64, from: i64, to: i64) -> ApiResult<Value> {
    let client = state.pool.get().await?;
    let hours = client
        .query(
            "SELECT hour, base, levels, swing_up, swing_down, swing_up_usd, swing_down_usd FROM footprint_hour WHERE market = $1 AND hour >= $2 AND hour <= $3 ORDER BY hour",
            &[&market, &from, &to],
        )
        .await?;
    let newest = hours.last().map_or(from - HOUR, |r| r.get::<_, i64>(0));
    let minutes = client
        .query(
            "SELECT minute, base, levels FROM footprint_minute WHERE market = $1 AND minute >= $2 AND minute <= $3 ORDER BY minute",
            &[&market, &(newest + HOUR).max(from).max(to - RECENT), &to],
        )
        .await?;

    // Each bar: its levels in ticks, and its delta as it went — hours carry
    // their own swing, minutes are read one close at a time.
    let mut bars: BTreeMap<i64, Folded> = BTreeMap::new();
    let worth = |level: i64| if usd { level as f64 * tick } else { 1.0 };
    for r in &hours {
        let at: i64 = r.get(0);
        let bar = bars.entry(at.div_euclid(bar_ms) * bar_ms).or_default();
        let mut net = 0.0;
        for (level, buy, sell) in unpack(r.get(1), r.get(2)) {
            let held = bar.levels.entry(level).or_insert((0.0, 0.0));
            held.0 += buy * worth(level);
            held.1 += sell * worth(level);
            net += (buy - sell) * worth(level);
        }
        let (up, down): (f64, f64) = if usd { (r.get(5), r.get(6)) } else { (r.get(3), r.get(4)) };
        bar.swing(up, down, net);
    }
    for r in &minutes {
        let at: i64 = r.get(0);
        let bar = bars.entry(at.div_euclid(bar_ms) * bar_ms).or_default();
        let mut net = 0.0;
        for (level, buy, sell) in unpack(r.get(1), r.get(2)) {
            let held = bar.levels.entry(level).or_insert((0.0, 0.0));
            held.0 += buy * worth(level);
            held.1 += sell * worth(level);
            net += (buy - sell) * worth(level);
        }
        bar.swing(net.max(0.0), net.min(0.0), net);
    }

    // One grid for every bar, the reader's or fitted: no more than a couple of
    // hundred rows in a typical bar, for the chart to join further as it zooms out.
    let fitted = ticks == 0;
    let by = if fitted {
        let mut spans: Vec<i64> = bars.values().filter_map(|b| Some(b.levels.keys().next_back()? - b.levels.keys().next()? + 1)).collect();
        spans.sort_unstable();
        let typical = spans.get(spans.len() * 4 / 5).copied().unwrap_or(0);
        let mut by = 1;
        while typical / by > MAX_FINE_ROWS {
            by = coarser(by);
        }
        by
    } else {
        ticks
    };
    let out: Vec<Value> = bars
        .iter()
        .map(|(at, bar)| {
            let mut rows: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
            for (level, (buy, sell)) in &bar.levels {
                let row = rows.entry(level.div_euclid(by) * by).or_insert((0.0, 0.0));
                row.0 += buy;
                row.1 += sell;
            }
            json!({
                "time": at,
                "levels": rows.iter().map(|(level, (buy, sell))| json!([*level as f64 * tick, buy, sell])).collect::<Vec<_>>(),
                "maxDelta": bar.max,
                "minDelta": bar.min,
            })
        })
        .collect();
    Ok(json!({ "market": market, "tick": tick, "grid": by as f64 * tick, "fitted": fitted, "bars": out }))
}

/// A bar being added up from hours and minutes.
#[derive(Default)]
struct Folded {
    levels: BTreeMap<i64, (f64, f64)>,
    delta: f64,
    max: f64,
    min: f64,
}

impl Folded {
    /// A stretch of the bar — an hour or a minute — whose delta swung `up` and
    /// `down` from where it started and ended `net` from it.
    fn swing(&mut self, up: f64, down: f64, net: f64) {
        self.max = self.max.max(self.delta + up);
        self.min = self.min.min(self.delta + down);
        self.delta += net;
    }
}

/// The next grouping up: 1, 2, 5, 10, 20, 50, and so on.
fn coarser(by: i64) -> i64 {
    let power = 10_i64.pow((by as f64).log10().floor() as u32);
    let head = by / power;
    (if head < 2 { 2 } else if head < 5 { 5 } else { 10 }) * power
}

// ---------------------------------------------------------------- days

/// `2026-01-01` as the UTC midnight it starts at.
fn day_ms(day: &str) -> Option<i64> {
    let mut parts = day.trim().split('-');
    let (y, m, d): (i32, u32, u32) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    if parts.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > days_in(y, m) {
        return None;
    }
    // Days from 1970-01-01, by the civil calendar.
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as i32;
    Some((i64::from(era) * 146_097 + i64::from(doe) - 719_468) * DAY)
}

/// The UTC date a moment falls on, and how far into the day it is.
fn ymd(ms: i64) -> ((i32, u32, u32), i64) {
    let days = ms.div_euclid(DAY);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as i32;
    ((y, m, d), ms.rem_euclid(DAY))
}

fn days_in(y: i32, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_read_and_written_as_the_calendar_has_them() {
        assert_eq!(day_ms("1970-01-01"), Some(0));
        assert_eq!(day_ms("2026-01-01"), Some(1_767_225_600_000));
        assert_eq!(ymd(1_767_225_600_000 + 5 * HOUR), ((2026, 1, 1), 5 * HOUR));
        assert_eq!(ymd(day_ms("2024-02-29").unwrap()).0, (2024, 2, 29));
        assert_eq!(day_ms("2026-02-29"), None, "not a leap year");
        assert_eq!(day_ms("2026-13-01"), None);
    }

    #[test]
    fn the_archive_is_read_by_months_while_they_are_over_then_by_days() {
        let now = day_ms("2026-03-04").unwrap() + 10 * HOUR;
        let names: Vec<String> = periods(day_ms("2026-01-15").unwrap(), now).iter().map(|p| p.name()).collect();
        assert_eq!(names, ["2026-01", "2026-02", "2026-03-01", "2026-03-02", "2026-03-03"], "today is not out yet");
        let recent: Vec<String> = periods(day_ms("2026-03-02").unwrap(), now).iter().map(|p| p.name()).collect();
        assert_eq!(recent, ["2026-03-02", "2026-03-03"], "a first day in this month starts there");
        let late: Vec<String> = days_of(2026, 2, now).iter().map(|p| p.name()).collect();
        assert_eq!((late.len(), late[0].as_str(), late[27].as_str()), (28, "2026-02-01", "2026-02-28"), "a month not packed yet, by its days");
        assert_eq!(
            archive_url("binance-perp:BTCUSDT", Period::Month(2026, 9)).as_deref(),
            Some("https://data.binance.vision/data/futures/um/monthly/aggTrades/BTCUSDT/BTCUSDT-aggTrades-2026-09.zip")
        );
        assert_eq!(
            archive_url("binance-spot:ETHUSDT", Period::Day(2026, 10, 5)).as_deref(),
            Some("https://data.binance.vision/data/spot/daily/aggTrades/ETHUSDT/ETHUSDT-aggTrades-2026-10-05.zip")
        );
    }

    #[test]
    fn a_trade_line_is_read_whichever_file_it_comes_from() {
        // Futures: the buyer was the maker, so the taker sold.
        assert_eq!(parse_trade("2950000000,87150.10,0.012,6000000000,6000000002,1767225600123,true"), Some(Trade { at: 1_767_225_600_123, price: 87_150.1, qty: 0.012, buy: false, prints: 3 }));
        // Spot from 2025: microseconds, a capitalised flag, and one more column.
        assert_eq!(parse_trade("1,87150.1,0.5,7,7,1767225600123456,False,True").map(|t| (t.at, t.buy)), Some((1_767_225_600_123, true)));
        assert_eq!(parse_trade("agg_trade_id,price,quantity,first_trade_id,last_trade_id,transact_time,is_buyer_maker"), None, "the header");
    }

    #[test]
    fn an_hour_keeps_how_far_its_delta_swung_at_each_minute_close() {
        let mut hour = Hour::default();
        // Minute one: three bought. Minute two: five sold. Minute three: one bought.
        hour.add(0, 100, 100.0, 3.0, 0.0);
        hour.add(MINUTE, 100, 100.0, 0.0, 5.0);
        hour.add(2 * MINUTE, 101, 101.0, 1.0, 0.0);
        let (bar, swing) = hour.finish();
        assert_eq!(bar.levels.get(&100), Some(&(3.0, 5.0)));
        assert_eq!((swing.delta, swing.up, swing.down), (-1.0, 3.0, -2.0));
        assert_eq!((swing.up_usd, swing.down_usd), (300.0, -200.0));
    }

    #[test]
    fn a_bar_of_hours_knows_its_extremes_from_theirs() {
        let mut bar = Folded::default();
        // An hour that rose to +5 and ended +2; one that fell 6 below where it started and ended -4.
        bar.swing(5.0, 0.0, 2.0);
        bar.swing(0.0, -6.0, -4.0);
        assert_eq!((bar.max, bar.min, bar.delta), (5.0, -4.0, -2.0));
    }

    #[test]
    fn rows_group_by_round_numbers_of_ticks() {
        assert_eq!([1, 2, 5, 10, 20].map(coarser), [2, 5, 10, 20, 50]);
    }
}
