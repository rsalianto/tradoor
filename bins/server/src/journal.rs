//! The trading journal. Every long or short position drawn on a chart is a
//! trade here — the drawing's entry, stop and target, and the coins its risk
//! settings buy — kept in step as the drawing moves (`sync_drawing`, called by
//! the drawings API), and left in the journal, untied, when the drawing goes.
//! A trade can be added by hand too.
//!
//! Price decides how each went. A watcher reads each market's closed minutes
//! from where it left off and walks every trade still planned or open through
//! them (`Trade::walk`): open once price reaches the entry, from the moment
//! the position was drawn — or from its left edge, for a box drawn wholly on
//! bars already past, a setup looked back on — then a win at the target or a
//! loss at the stop; a minute that reaches both counts as the stop. A status set by hand — closed at a price,
//! break-even, cancelled — takes the trade out of price's hands until it is
//! given back, and moving the drawing weighs it again from the start.
//!
//! The minutes come from the market's own exchange: Binance, Bybit,
//! Hyperliquid, OKX and Bitget. A trade on another venue, or on an index, is
//! kept by hand.
//!
//! Besides its entry, stop and target a position may carry legs, drawn with
//! it: part taken off on the way to the target, part cut on the way to the
//! stop, or more added — each a price and a share of the size the entry buys.
//! Price fills them as it reaches them, the stop's side first, and what the
//! trade made is worked out from its fills against their average entry.
//!
//! A journal is kept for an account: a browser signed in with Telegram, whose
//! journal then follows it to any device it signs in on. One not signed in
//! draws positions as before, and none is written down.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tradoor_core::{Candle, ChartTimeframe, Interval};
use tradoor_store::now_ms;
use tradoor_venues::{binance, bitget, bybit, hyperliquid, okx, Exchange, Kind, MarketRef};

use crate::api::{owner, ApiError, ApiResult, AppState};
use crate::catalog::Catalog;
use crate::dns::Doh;
use crate::drawings::Point;

const MINUTE: i64 = 60_000;
/// How often the watcher reads the markets with trades still in play.
const WATCH_EVERY: Duration = Duration::from_secs(60);
/// Trades walked per round, the longest unread first.
const TRADES_PER_ROUND: i64 = 500;
/// Pages of minutes one market reads per round: a position drawn on old bars
/// catches up over a few rounds rather than holding up the rest.
const PAGES_PER_ROUND: usize = 10;
/// Trades one browser keeps.
const MAX_PER_OWNER: i64 = 50_000;
const MAX_TEXT: usize = 4_000;

/// The figures a position's settings fall back on, as the chart draws them
/// (crates/render, `paint_position`): a 1,000 account risking 1%, or 10 in cash.
const DEFAULT_ACCOUNT: f64 = 1_000.0;
const DEFAULT_RISK_PCT: f64 = 1.0;
const DEFAULT_RISK_CASH: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    fn of(s: &str) -> Option<Self> {
        match s {
            "long" => Some(Self::Long),
            "short" => Some(Self::Short),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }

    fn dir(self) -> f64 {
        match self {
            Self::Long => 1.0,
            Self::Short => -1.0,
        }
    }
}

const STATUSES: [&str; 7] = ["planned", "open", "win", "loss", "closed", "breakeven", "cancelled"];

/// Legs one position may carry.
const MAX_LEGS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegKind {
    /// Part taken off on the way to the target.
    Take,
    /// Part cut on the way to the stop.
    Cut,
    /// More bought (sold, for a short) at another price.
    Add,
}

/// A level drawn with a position: its price, and the share of the entry's
/// size it moves — half is 0.5.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    pub kind: LegKind,
    pub price: f64,
    pub share: f64,
}

/// Coins into or out of a trade: at its entry, a leg, the stop, the target,
/// or by hand.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub kind: String,
    pub price: f64,
    pub qty: f64,
    pub at: i64,
    /// The leg it filled, by its place in the trade's legs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leg: Option<usize>,
}

impl Fill {
    fn new(kind: &str, price: f64, qty: f64, at: i64, leg: Option<usize>) -> Self {
        Self { kind: kind.into(), price, qty, at, leg }
    }

    /// Into the position: the entry or an add; anything else takes coins out.
    fn into_position(&self) -> bool {
        self.kind == "entry" || self.kind == "add"
    }
}

/// A trade's coins in and out, what those in cost and those out fetched.
#[derive(Clone, Copy, Debug, Default)]
struct Book {
    bought: f64,
    cost: f64,
    sold: f64,
    proceeds: f64,
}

impl Book {
    fn of(fills: &[Fill]) -> Self {
        let mut book = Self::default();
        for f in fills {
            if f.into_position() {
                book.put(f.price, f.qty);
            } else {
                book.take(f.price, f.qty);
            }
        }
        book
    }

    fn held(&self) -> f64 {
        (self.bought - self.sold).max(0.0)
    }

    fn put(&mut self, price: f64, qty: f64) {
        self.bought += qty;
        self.cost += price * qty;
    }

    /// Out at `price`, no more than is held: the coins that went.
    fn take(&mut self, price: f64, qty: f64) -> f64 {
        let qty = qty.min(self.held());
        self.sold += qty;
        self.proceeds += price * qty;
        qty
    }

    fn average_entry(&self) -> f64 {
        if self.bought > 0.0 { self.cost / self.bought } else { 0.0 }
    }

    fn average_exit(&self) -> Option<f64> {
        (self.sold > 0.0).then(|| self.proceeds / self.sold)
    }

    /// What the coins out made against the average entry; `dir` is 1 for a long, -1 for a short.
    fn made(&self, dir: f64) -> f64 {
        (self.proceeds - self.average_entry() * self.sold) * dir
    }
}

/// What a trade has done so far, while the plan under it changes and price
/// does not weigh it again.
struct Kept {
    status: String,
    qty: f64,
    exit: Option<f64>,
    opened_at: Option<i64>,
    closed_at: Option<i64>,
    fills: Vec<Fill>,
}

impl Kept {
    /// The fills and exit that go with a plan at `entry` buying `qty`, or None
    /// where they stand. Closed or evened by hand, a moved trade is in at the
    /// new entry and out where the hand said — evened, at the entry; any other
    /// is the same trade at its new size.
    fn follow(&self, moved: bool, entry: f64, qty: f64, start_at: i64, now: i64) -> (Option<Value>, Option<f64>) {
        if moved && matches!(self.status.as_str(), "closed" | "breakeven") {
            let out = if self.status == "breakeven" { entry } else { self.exit.unwrap_or(entry) };
            let fills = [Fill::new("entry", entry, qty, self.opened_at.unwrap_or(start_at), None), Fill::new("hand", out, qty, self.closed_at.unwrap_or(now), None)];
            return (serde_json::to_value(fills).ok(), Some(out));
        }
        if self.qty > 0.0 && qty != self.qty && !self.fills.is_empty() {
            let by = qty / self.qty;
            let fills: Vec<Fill> = self.fills.iter().map(|f| Fill { qty: f.qty * by, ..f.clone() }).collect();
            return (serde_json::to_value(fills).ok(), None);
        }
        (None, None)
    }
}

/// The legs a position's settings list (`legs`: `t,price,percent|c,…|a,…`,
/// the percent of the entry's size) that its prices allow: a take between the
/// entry and the target, a cut between the entry and the stop, an add anywhere
/// between the stop and the target but the entry.
pub fn legs_of(style: &Map<String, Value>, entry: f64, stop: f64, target: f64) -> Vec<Leg> {
    let Some(raw) = style.get("legs").and_then(Value::as_str) else { return Vec::new() };
    let between = |p: f64, a: f64, b: f64| p > a.min(b) && p < a.max(b);
    raw.split('|')
        .filter_map(|part| {
            let mut f = part.split(',').map(str::trim);
            let kind = match f.next()? {
                "t" => LegKind::Take,
                "c" => LegKind::Cut,
                "a" => LegKind::Add,
                _ => return None,
            };
            let price: f64 = f.next()?.parse().ok()?;
            let percent: f64 = f.next()?.parse().ok()?;
            let fits = match kind {
                LegKind::Take => between(price, entry, target),
                LegKind::Cut => between(price, entry, stop),
                LegKind::Add => between(price, stop, target) && price != entry,
            };
            (fits && percent > 0.0 && percent <= 1000.0).then_some(Leg { kind, price, share: percent / 100.0 })
        })
        .take(MAX_LEGS)
        .collect()
}

/// What a position loses when price runs to its stop, every add and cut on
/// the way filled: the most it risks.
pub fn worst_loss(side: Side, entry: f64, stop: f64, qty: f64, legs: &[Leg]) -> f64 {
    let mut book = Book::default();
    book.put(entry, qty);
    let mut on_the_way: Vec<&Leg> = legs.iter().filter(|l| (l.price - entry) * (stop - entry) > 0.0).collect();
    on_the_way.sort_by(|a, b| (a.price - entry).abs().total_cmp(&(b.price - entry).abs()));
    for l in on_the_way {
        if l.kind == LegKind::Add {
            book.put(l.price, l.share * qty);
        } else {
            book.take(l.price, l.share * qty);
        }
    }
    book.take(stop, book.held());
    -book.made(side.dir())
}

/// A position as drawn: from when price is weighed against it, its prices,
/// its legs, what it risks at most and what its entry buys.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub side: Side,
    pub start_at: i64,
    pub entry: f64,
    pub stop: f64,
    pub target: f64,
    pub legs: Vec<Leg>,
    pub risk: f64,
    pub qty: f64,
}

/// A style value the page may have written as a number or as a string.
fn style_num(style: &Map<String, Value>, key: &str) -> Option<f64> {
    let v = style.get(key)?;
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()).filter(|v: &f64| v.is_finite() && *v > 0.0)
}

/// What a position risks, as its settings say: cash, or a part of the account.
pub fn risk_of(style: &Map<String, Value>) -> f64 {
    let cash = style.get("riskUnit").and_then(Value::as_str).is_some_and(|u| u != "%");
    if cash {
        style_num(style, "risk").unwrap_or(DEFAULT_RISK_CASH)
    } else {
        style_num(style, "account").unwrap_or(DEFAULT_ACCOUNT) * style_num(style, "risk").unwrap_or(DEFAULT_RISK_PCT) / 100.0
    }
}

/// A long or short drawing's plan: the entry, stop and target its three
/// anchors hold. A box that reaches the bar forming when it was drawn, or
/// past it, is a trade taken then, weighed from that moment — however far
/// back its left edge sits; one wholly on bars already past is a setup looked
/// back on, weighed from its left edge. None for anything else, or prices
/// that make no trade.
pub fn plan_of(kind: &str, points: &[Point], style: &Map<String, Value>, drawn_at: i64, timeframe: &str) -> Option<Plan> {
    let side = Side::of(kind)?;
    let [entry, stop, target] = [points.first()?.price, points.get(1)?.price, points.get(2)?.price];
    let sound = match side {
        Side::Long => stop < entry && entry < target,
        Side::Short => target < entry && entry < stop,
    };
    if !sound || entry <= 0.0 {
        return None;
    }
    let left = points.iter().map(|p| p.time).fold(f64::INFINITY, f64::min) as i64;
    let right = points.iter().map(|p| p.time).fold(f64::NEG_INFINITY, f64::max) as i64;
    let bar = ChartTimeframe::parse(timeframe).and_then(|tf| Some(tf.source().interval.ms()? * i64::from(tf.source().group))).unwrap_or(31 * 24 * 60 * MINUTE);
    let start_at = if right >= drawn_at - bar { drawn_at } else { left };
    let qty = risk_of(style) / (entry - stop).abs();
    let legs = legs_of(style, entry, stop, target);
    let risk = worst_loss(side, entry, stop, qty, &legs);
    Some(Plan { side, start_at: start_at.div_euclid(MINUTE) * MINUTE, entry, stop, target, legs, risk, qty })
}

/// How a trade stands against price.
#[derive(Clone, Debug, PartialEq)]
pub struct Trade {
    pub side: Side,
    pub entry: f64,
    pub stop: f64,
    pub target: f64,
    /// The coins the entry buys; a leg moves a share of them.
    pub qty: f64,
    pub legs: Vec<Leg>,
    pub fills: Vec<Fill>,
    pub status: String,
    pub opened_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub exit: Option<f64>,
    /// The open of the next minute to read.
    pub checked_until: i64,
}

impl Trade {
    /// Walk closed minutes, oldest first, from `checked_until` on. The minute
    /// the entry is reached may reach the stop or the target too: the stop
    /// first, as the cautious reading, then the target.
    pub fn walk(&mut self, bars: &[Candle]) {
        let mut prev_close: Option<f64> = None;
        for k in bars {
            if k.open_time < self.checked_until {
                prev_close = Some(k.close);
                continue;
            }
            if self.status == "planned" {
                // Reached within the minute, or jumped over between two.
                let within = k.low <= self.entry && self.entry <= k.high;
                let gapped = prev_close.is_some_and(|c| (c - self.entry).signum() != (k.open - self.entry).signum());
                if within || gapped {
                    self.status = "open".into();
                    self.opened_at = Some(k.open_time);
                    self.fills = vec![Fill::new("entry", self.entry, self.qty, k.open_time, None)];
                }
            }
            if self.status == "open" {
                // Opened before trades kept their fills: its entry, all of it.
                if self.fills.is_empty() {
                    self.fills.push(Fill::new("entry", self.entry, self.qty, self.opened_at.unwrap_or(k.open_time), None));
                }
                self.step(k);
            }
            self.checked_until = k.open_time + MINUTE;
            prev_close = Some(k.close);
            if self.status != "planned" && self.status != "open" {
                return;
            }
        }
    }

    /// One minute of an open trade: the levels on the stop's side first,
    /// nearest the entry first — adds, cuts, then the stop — as the cautious
    /// reading, then those on the target's. A level not reached keeps those
    /// past it waiting. Closed once nothing is held: a win on the target's
    /// side, a loss on the stop's.
    fn step(&mut self, k: &Candle) {
        let mut book = Book::of(&self.fills);
        let filled: Vec<usize> = self.fills.iter().filter_map(|f| f.leg).collect();
        for (end, end_kind) in [(self.stop, "stop"), (self.target, "target")] {
            let mut levels: Vec<(f64, Option<usize>)> = self
                .legs
                .iter()
                .enumerate()
                .filter(|(i, l)| !filled.contains(i) && (l.price - self.entry) * (end - self.entry) > 0.0)
                .map(|(i, l)| (l.price, Some(i)))
                .collect();
            levels.push((end, None));
            levels.sort_by(|a, b| (a.0 - self.entry).abs().total_cmp(&(b.0 - self.entry).abs()));
            for (price, leg) in levels {
                let reached = if price < self.entry { k.low <= price } else { k.high >= price };
                if !reached {
                    break;
                }
                let (kind, qty) = match leg.map(|i| self.legs[i]) {
                    Some(Leg { kind: LegKind::Add, share, .. }) => {
                        book.put(price, share * self.qty);
                        ("add", share * self.qty)
                    }
                    Some(Leg { kind: LegKind::Take, share, .. }) => ("take", book.take(price, share * self.qty)),
                    Some(Leg { kind: LegKind::Cut, share, .. }) => ("cut", book.take(price, share * self.qty)),
                    None => (end_kind, book.take(price, book.held())),
                };
                self.fills.push(Fill::new(kind, price, qty, k.open_time, leg));
                if book.held() <= self.qty * 1e-9 {
                    self.status = if end_kind == "target" { "win" } else { "loss" }.into();
                    self.exit = book.average_exit();
                    self.closed_at = Some(k.open_time);
                    return;
                }
            }
        }
    }
}

/// Whether price can decide a trade on this market: its exchange's minutes are read here.
pub fn tracked(market: &str) -> bool {
    MarketRef::parse(market).is_some_and(|m| matches!(m.venue.exchange(), Exchange::Binance | Exchange::Bybit | Exchange::Hyperliquid | Exchange::Okx | Exchange::Bitget))
}

// ---------------------------------------------------------------- the drawings' side

/// Keep the trade of a long or short drawing in step with it: made when the
/// drawing is, its plan moved when the drawing moves — and then weighed
/// against price again from the start. A drawing put back by an undo takes
/// back the trade it had rather than starting another. Anything else, and a
/// ratio chart's positions, are no trade.
pub async fn sync_drawing(client: &tokio_postgres::Client, owner: &str, drawing: &Value) -> Result<(), tokio_postgres::Error> {
    let (Some(id), Some(market), Some(kind)) = (drawing["id"].as_i64(), drawing["market"].as_str(), drawing["kind"].as_str()) else { return Ok(()) };
    if Side::of(kind).is_none() || MarketRef::parse(market).is_none() {
        return Ok(());
    }
    let points: Vec<Point> = drawing["points"].as_array().map_or_else(Vec::new, |a| {
        a.iter().filter_map(|p| Some(Point { time: p["time"].as_f64()?, price: p["price"].as_f64()? })).collect()
    });
    let style = drawing["style"].as_object().cloned().unwrap_or_default();
    let timeframe = drawing["timeframe"].as_str().unwrap_or("");
    let drawn_at = drawing["createdAt"].as_i64().unwrap_or_else(now_ms);
    let Some(plan) = plan_of(kind, &points, &style, drawn_at, timeframe) else { return Ok(()) };
    if !signed_in(client, owner).await? {
        return Ok(());
    }
    let now = now_ms();
    let held = client
        .query_opt(
            "SELECT id, start_at, entry, stop, target, auto, legs, qty, status, exit, opened_at, closed_at, fills FROM journal_trade WHERE drawing_id = $1 AND owner = $2",
            &[&id, &owner],
        )
        .await?;
    let legs = serde_json::to_value(&plan.legs).unwrap_or_default();
    if let Some(row) = held {
        let moved = row.get::<_, i64>(1) != plan.start_at
            || row.get::<_, f64>(2) != plan.entry
            || row.get::<_, f64>(3) != plan.stop
            || row.get::<_, f64>(4) != plan.target
            || row.get::<_, Value>(6) != legs;
        // A moved plan, still price's to decide, is weighed again from its start.
        let again = moved && row.get::<_, bool>(5);
        // Otherwise what was filled keeps up with the plan, so its P&L does too.
        let kept = Kept { status: row.get(8), qty: row.get(7), exit: row.get(9), opened_at: row.get(10), closed_at: row.get(11), fills: serde_json::from_value(row.get(12)).unwrap_or_default() };
        let (fills, exit) = if again { (None, None) } else { kept.follow(moved, plan.entry, plan.qty, plan.start_at, now) };
        client
            .execute(
                "UPDATE journal_trade SET start_at = $2, entry = $3, stop = $4, target = $5, risk = $6, qty = $7, timeframe = $8, updated_at = $9, legs = $11,
                     status = CASE WHEN $10 THEN 'planned' ELSE status END, opened_at = CASE WHEN $10 THEN NULL ELSE opened_at END,
                     closed_at = CASE WHEN $10 THEN NULL ELSE closed_at END, exit = CASE WHEN $10 THEN NULL ELSE COALESCE($13, exit) END,
                     checked_until = CASE WHEN $10 THEN $2 ELSE checked_until END, fills = CASE WHEN $10 THEN '[]'::jsonb ELSE COALESCE($12, fills) END
                 WHERE id = $1",
                &[&row.get::<_, i64>(0), &plan.start_at, &plan.entry, &plan.stop, &plan.target, &plan.risk, &plan.qty, &timeframe, &now, &again, &legs, &fills, &exit],
            )
            .await?;
        return Ok(());
    }
    // Undone deletes come back as new drawings, made just now: the trade they
    // left behind — the same prices on the same market — is theirs again. One
    // added by hand (no timeframe) was never a drawing's, and stays the hand's.
    let back = client
        .execute(
            "UPDATE journal_trade SET drawing_id = $1, updated_at = $8 WHERE id = (
                 SELECT id FROM journal_trade WHERE owner = $2 AND drawing_id IS NULL AND timeframe <> '' AND market = $3 AND side = $4
                     AND entry = $5 AND stop = $6 AND target = $7 ORDER BY id DESC LIMIT 1)",
            &[&id, &owner, &market, &plan.side.id(), &plan.entry, &plan.stop, &plan.target, &now],
        )
        .await?;
    if back > 0 {
        return Ok(());
    }
    client
        .execute(
            "INSERT INTO journal_trade (owner, drawing_id, market, side, timeframe, start_at, entry, stop, target, risk, qty, checked_until, created_at, updated_at, legs)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $6, $12, $12, $13) ON CONFLICT (drawing_id) DO NOTHING",
            &[&owner, &id, &market, &plan.side.id(), &timeframe, &plan.start_at, &plan.entry, &plan.stop, &plan.target, &plan.risk, &plan.qty, &now, &legs],
        )
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- the watcher

pub fn start(pool: deadpool_postgres::Pool, doh: &Arc<Doh>, catalog: Arc<Catalog>) {
    let http = reqwest::Client::builder().dns_resolver(doh.resolver()).timeout(Duration::from_secs(20)).build().unwrap_or_default();
    tokio::spawn(async move {
        loop {
            if let Err(e) = round(&pool, &http, &catalog).await {
                tracing::warn!(error = %e, "weighing the journal's trades");
            }
            tokio::time::sleep(WATCH_EVERY).await;
        }
    });
}

/// One round: each market with a trade in play read from its least-read
/// trade on, and every such trade walked through what was read.
async fn round(pool: &deadpool_postgres::Pool, http: &reqwest::Client, catalog: &Catalog) -> anyhow::Result<()> {
    let client = pool.get().await?;
    let rows = client
        .query(
            "SELECT id, market, side, entry, stop, target, status, opened_at, checked_until, qty, legs, fills FROM journal_trade
             WHERE auto AND status IN ('planned', 'open') AND EXISTS (SELECT 1 FROM account a WHERE a.owner = journal_trade.owner)
             ORDER BY checked_until LIMIT $1",
            &[&TRADES_PER_ROUND],
        )
        .await?;
    let mut by_market: HashMap<String, Vec<(i64, Trade)>> = HashMap::new();
    for r in &rows {
        let market: String = r.get(1);
        if !tracked(&market) {
            continue;
        }
        let Some(side) = Side::of(r.get(2)) else { continue };
        let legs: Vec<Leg> = serde_json::from_value(r.get(10)).unwrap_or_default();
        let fills: Vec<Fill> = serde_json::from_value(r.get(11)).unwrap_or_default();
        let trade = Trade {
            side,
            entry: r.get(3),
            stop: r.get(4),
            target: r.get(5),
            qty: r.get(9),
            legs,
            fills,
            status: r.get(6),
            opened_at: r.get(7),
            closed_at: None,
            exit: None,
            checked_until: r.get(8),
        };
        by_market.entry(market).or_default().push((r.get(0), trade));
    }
    let settled = now_ms().div_euclid(MINUTE) * MINUTE;
    for (market, mut trades) in by_market {
        let from = trades.iter().map(|(_, t)| t.checked_until).min().unwrap_or(settled);
        let bars = match minutes(http, catalog, &market, from, settled).await {
            Ok(bars) => bars,
            Err(e) => {
                tracing::debug!(%market, error = %e, "journal: minutes not read");
                continue;
            }
        };
        for (id, trade) in &mut trades {
            let before = (trade.status.clone(), trade.checked_until);
            trade.walk(&bars);
            if (trade.status.clone(), trade.checked_until) == before {
                continue;
            }
            // Not if the drawing moved meanwhile, and the trade was set to be weighed again.
            client
                .execute(
                    "UPDATE journal_trade SET status = $2, opened_at = $3, closed_at = $4, exit = $5, checked_until = $6, fills = $7
                     WHERE id = $1 AND auto AND checked_until = $8",
                    &[id, &trade.status, &trade.opened_at, &trade.closed_at, &trade.exit, &trade.checked_until, &serde_json::to_value(&trade.fills).unwrap_or_default(), &before.1],
                )
                .await?;
        }
    }
    Ok(())
}

/// A market's closed minutes from `from` up to `until`, oldest first, a few
/// pages' worth at most: the rest are read next round.
async fn minutes(http: &reqwest::Client, catalog: &Catalog, market: &str, from: i64, until: i64) -> anyhow::Result<Vec<Candle>> {
    let m = MarketRef::parse(market).ok_or_else(|| anyhow!("{market} is not a market"))?;
    let native = catalog.native_names(m.venue, std::slice::from_ref(&m.symbol)).await.remove(&m.symbol).unwrap_or_else(|| m.symbol.clone());
    let mut out: Vec<Candle> = Vec::new();
    let mut at = from;
    for _ in 0..PAGES_PER_ROUND {
        if at >= until {
            break;
        }
        let page = minute_page(http, &m, &native, at, until).await?;
        let page: Vec<Candle> = page.into_iter().filter(|k| k.open_time >= at && k.open_time + MINUTE <= until).collect();
        let Some(last) = page.last().map(|k| k.open_time) else { break };
        out.extend(page);
        at = last + MINUTE;
    }
    Ok(out)
}

fn candle(t: i64, o: f64, h: f64, l: f64, c: f64) -> Candle {
    Candle { open_time: t, open: o, high: h, low: l, close: c, volume: 0.0, quote_volume: None }
}

/// A number the exchanges send as a string or as a number.
fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

async fn text(http: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let resp = http.get(url).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        bail!("{status} from {url}");
    }
    Ok(body)
}

/// One page of minutes from `from` on, as the market's exchange gives them.
async fn minute_page(http: &reqwest::Client, m: &MarketRef, native: &str, from: i64, until: i64) -> anyhow::Result<Vec<Candle>> {
    let rows_of = |body: &str, path: &[&str]| -> anyhow::Result<Vec<Value>> {
        let mut v: Value = serde_json::from_str(body)?;
        for key in path {
            v = v.get(*key).cloned().unwrap_or(Value::Null);
        }
        Ok(v.as_array().cloned().unwrap_or_default())
    };
    let arrays = |rows: Vec<Value>| -> Vec<Candle> {
        let mut out: Vec<Candle> = rows
            .iter()
            .filter_map(|r| Some(candle(num(r.get(0)?)? as i64, num(r.get(1)?)?, num(r.get(2)?)?, num(r.get(3)?)?, num(r.get(4)?)?)))
            .collect();
        out.sort_by_key(|k| k.open_time);
        out
    };
    let klines = |list: Vec<tradoor_venues::OwnedKline>| -> Vec<Candle> {
        list.iter()
            .filter_map(|k| Some(candle(k.open_time, k.open.parse().ok()?, k.high.parse().ok()?, k.low.parse().ok()?, k.close.parse().ok()?)))
            .collect()
    };
    Ok(match m.venue.exchange() {
        Exchange::Binance => {
            let base = if m.venue.kind() == Kind::Perp { binance::PERP_REST } else { binance::SPOT_REST };
            klines(binance::parse_klines(&text(http, &binance::klines_url(base, m, Interval::M1, Some(from), None, 1000)).await?)?)
        }
        Exchange::Bybit => {
            // Bybit answers the newest of a range: a range of one page.
            let end = (from + 1000 * MINUTE - 1).min(until);
            klines(bybit::parse_klines(&text(http, &bybit::klines_url(bybit::REST, m, Interval::M1, Some(from), Some(end), 1000)).await?)?)
        }
        Exchange::Hyperliquid => {
            let body = json!({ "type": "candleSnapshot", "req": { "coin": native, "interval": "1m", "startTime": from, "endTime": (from + 5000 * MINUTE).min(until) } });
            let resp = http.post(hyperliquid::INFO).header("content-type", "application/json").body(body.to_string()).send().await?;
            let status = resp.status();
            let text = resp.text().await?;
            if !status.is_success() {
                bail!("{status} from Hyperliquid");
            }
            let mut out: Vec<Candle> = rows_of(&text, &[])?
                .iter()
                .filter_map(|k| Some(candle(k["t"].as_i64()?, num(&k["o"])?, num(&k["h"])?, num(&k["l"])?, num(&k["c"])?)))
                .collect();
            out.sort_by_key(|k| k.open_time);
            out
        }
        Exchange::Okx => {
            // Newer than `before`, older than `after`, a hundred at most.
            let url = format!("{}/api/v5/market/history-candles?instId={native}&bar=1m&before={}&after={}&limit=100", okx::REST, from - 1, from + 100 * MINUTE);
            arrays(rows_of(&text(http, &url).await?, &["data"])?)
        }
        Exchange::Bitget => {
            let end = (from + 200 * MINUTE).min(until);
            let url = if m.venue.kind() == Kind::Perp {
                format!("{}/api/v2/mix/market/history-candles?symbol={native}&productType=usdt-futures&granularity=1m&endTime={end}&limit=200", bitget::REST)
            } else {
                format!("{}/api/v2/spot/market/history-candles?symbol={native}&granularity=1min&endTime={end}&limit=200", bitget::REST)
            };
            arrays(rows_of(&text(http, &url).await?, &["data"])?)
        }
        _ => bail!("{} has no minutes read here", m.venue.id()),
    })
}

// ---------------------------------------------------------------- the API

const COLUMNS: &str =
    "id, drawing_id, market, side, timeframe, start_at, entry, stop, target, risk, qty, status, opened_at, closed_at, exit, fees, auto, strategy, notes, created_at, updated_at, legs, fills";

fn row_json(r: &tokio_postgres::Row) -> Value {
    let market: String = r.get(2);
    json!({
        "id": r.get::<_, i64>(0),
        "drawingId": r.get::<_, Option<i64>>(1),
        "tracked": tracked(&market),
        "market": market,
        "side": r.get::<_, String>(3),
        "timeframe": r.get::<_, String>(4),
        "startAt": r.get::<_, i64>(5),
        "entry": r.get::<_, f64>(6),
        "stop": r.get::<_, f64>(7),
        "target": r.get::<_, f64>(8),
        "risk": r.get::<_, f64>(9),
        "qty": r.get::<_, f64>(10),
        "status": r.get::<_, String>(11),
        "openedAt": r.get::<_, Option<i64>>(12),
        "closedAt": r.get::<_, Option<i64>>(13),
        "exit": r.get::<_, Option<f64>>(14),
        "fees": r.get::<_, f64>(15),
        "auto": r.get::<_, bool>(16),
        "strategy": r.get::<_, String>(17),
        "notes": r.get::<_, String>(18),
        "createdAt": r.get::<_, i64>(19),
        "updatedAt": r.get::<_, i64>(20),
        "legs": r.get::<_, Value>(21),
        "fills": r.get::<_, Value>(22),
    })
}

/// What a browser not signed in is told when it would write to the journal.
const SIGN_IN_FIRST: &str = "sign in with Telegram to keep a journal";

/// Whether this browser id is an account's: one signed in with Telegram.
async fn signed_in(client: &tokio_postgres::Client, owner: &str) -> Result<bool, tokio_postgres::Error> {
    Ok(client.query_one("SELECT EXISTS (SELECT 1 FROM account WHERE owner = $1)", &[&owner]).await?.get(0))
}

async fn account_only(client: &tokio_postgres::Client, owner: &str) -> ApiResult<()> {
    if signed_in(client, owner).await? { Ok(()) } else { Err(ApiError::forbidden(SIGN_IN_FIRST)) }
}

/// `GET /api/journal`: this browser's trades, newest first, tagged with what
/// they read as: an open journal asks again every half minute, and while
/// nothing has changed the tag it sends back is answered 304, with no body.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let owner = owner(&headers)?;
    let client = state.pool.get().await?;
    let rows = client.query(&format!("SELECT {COLUMNS} FROM journal_trade WHERE owner = $1 ORDER BY start_at DESC, id DESC"), &[&owner]).await?;
    let body = json!({ "trades": rows.iter().map(row_json).collect::<Vec<_>>() }).to_string();
    let tag = tag_of(&body);
    // A proxy that recompresses the answer weakens the tag: `W/"…"` still matches.
    let held = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(',').any(|t| t.trim().trim_start_matches("W/") == tag));
    if held {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, tag)]).into_response());
    }
    Ok(([(header::CONTENT_TYPE, "application/json".to_owned()), (header::CACHE_CONTROL, "no-store".to_owned()), (header::ETAG, tag)], body).into_response())
}

fn tag_of(body: &str) -> String {
    let mut h = DefaultHasher::new();
    h.write(body.as_bytes());
    format!("\"{:016x}\"", h.finish())
}

fn finite(b: &Map<String, Value>, key: &str) -> ApiResult<Option<f64>> {
    match b.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).map(Some).ok_or_else(|| ApiError::bad_request(format!("{key} must be a finite number"))),
    }
}

fn short_text(b: &Map<String, Value>, key: &str) -> ApiResult<Option<String>> {
    match b.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(String::new())),
        Some(v) => {
            let s = v.as_str().ok_or_else(|| ApiError::bad_request(format!("{key} must be a string")))?;
            if s.len() > MAX_TEXT {
                return Err(ApiError::bad_request(format!("{key} may be at most {MAX_TEXT} bytes")));
            }
            Ok(Some(s.trim().to_string()))
        }
    }
}

/// `POST /api/journal` `{market, side, entry, stop, target, risk?, startAt?,
/// status?, exit?, closedAt?, strategy?, notes?, fees?}`: a trade added by
/// hand. One already closed is the hand's to keep; one planned or open is
/// price's to follow, where its market's minutes are read.
pub async fn create(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> ApiResult<(StatusCode, Json<Value>)> {
    let owner = owner(&headers)?;
    let b = body.as_object().ok_or_else(|| ApiError::bad_request("body must be a JSON object"))?;
    let market = b.get("market").and_then(Value::as_str).and_then(MarketRef::parse).ok_or_else(|| ApiError::bad_request("market must be venue:SYMBOL"))?.to_string();
    let side = b.get("side").and_then(Value::as_str).and_then(Side::of).ok_or_else(|| ApiError::bad_request("side must be long or short"))?;
    let need = |key: &str| finite(b, key)?.filter(|v| *v > 0.0).ok_or_else(|| ApiError::bad_request(format!("{key} must be a positive number")));
    let (entry, stop, target) = (need("entry")?, need("stop")?, need("target")?);
    let sound = match side {
        Side::Long => stop < entry && entry < target,
        Side::Short => target < entry && entry < stop,
    };
    if !sound {
        return Err(ApiError::bad_request("a long's stop is under its entry and its target over; a short's the other way"));
    }
    let now = now_ms();
    let risk = finite(b, "risk")?.filter(|v| *v > 0.0).unwrap_or(DEFAULT_ACCOUNT * DEFAULT_RISK_PCT / 100.0);
    let start_at = finite(b, "startAt")?.map_or(now, |v| v as i64).div_euclid(MINUTE) * MINUTE;
    let status = b.get("status").and_then(Value::as_str).unwrap_or("planned");
    if !STATUSES.contains(&status) {
        return Err(ApiError::bad_request(format!("status is one of {}", STATUSES.join(", "))));
    }
    let exit = finite(b, "exit")?;
    let done = !matches!(status, "planned" | "open");
    if matches!(status, "win" | "loss" | "closed") && exit.is_none() {
        return Err(ApiError::bad_request("a closed trade needs its exit price"));
    }
    let closed_at = finite(b, "closedAt")?.map(|v| v as i64).or(done.then_some(now));
    let client = state.pool.get().await?;
    account_only(&client, &owner).await?;
    let held: i64 = client.query_one("SELECT count(*) FROM journal_trade WHERE owner = $1", &[&owner]).await?.get(0);
    if held >= MAX_PER_OWNER {
        return Err(ApiError::bad_request(format!("a browser may keep at most {MAX_PER_OWNER} trades")));
    }
    let row = client
        .query_one(
            &format!(
                "INSERT INTO journal_trade (owner, market, side, start_at, entry, stop, target, risk, qty, status, opened_at, closed_at, exit, fees, auto, checked_until, strategy, notes, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $4, $16, $17, $18, $18) RETURNING {COLUMNS}"
            ),
            &[
                &owner,
                &market,
                &side.id(),
                &start_at,
                &entry,
                &stop,
                &target,
                &risk,
                &(risk / (entry - stop).abs()),
                &status,
                &(status != "planned").then_some(start_at),
                &closed_at.filter(|_| done),
                &exit.filter(|_| done),
                &finite(b, "fees")?.unwrap_or(0.0),
                &!done,
                &short_text(b, "strategy")?.unwrap_or_default(),
                &short_text(b, "notes")?.unwrap_or_default(),
                &now,
            ],
        )
        .await?;
    Ok((StatusCode::CREATED, Json(json!({ "trade": row_json(&row) }))))
}

/// `PATCH /api/journal/{id}`: its strategy, notes, fees or risk; its entry,
/// stop or target; or its status by hand — `closed` at `exit` (and
/// `closedAt`), `breakeven`, `cancelled` — which takes it out of price's
/// hands; or `auto: true`, which gives it back, weighed again from its start.
pub async fn update(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>, Json(body): Json<Value>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let b = body.as_object().ok_or_else(|| ApiError::bad_request("body must be a JSON object"))?;
    let client = state.pool.get().await?;
    account_only(&client, &owner).await?;
    let drawing_id: Option<i64> = client
        .query_opt("SELECT drawing_id FROM journal_trade WHERE id = $1 AND owner = $2", &[&id, &owner])
        .await?
        .ok_or_else(|| ApiError::not_found(format!("trade {id} not found")))?
        .get(0);
    let now = now_ms();
    if let Some(strategy) = short_text(b, "strategy")? {
        client.execute("UPDATE journal_trade SET strategy = $2 WHERE id = $1", &[&id, &strategy]).await?;
    }
    if let Some(notes) = short_text(b, "notes")? {
        client.execute("UPDATE journal_trade SET notes = $2 WHERE id = $1", &[&id, &notes]).await?;
    }
    if let Some(fees) = finite(b, "fees")? {
        client.execute("UPDATE journal_trade SET fees = $2 WHERE id = $1", &[&id, &fees]).await?;
    }
    let prices = [finite(b, "entry")?, finite(b, "stop")?, finite(b, "target")?];
    if prices.iter().any(Option::is_some) {
        replan(&client, &owner, id, drawing_id, prices, now).await?;
    }
    if let Some(risk) = finite(b, "risk")? {
        resize(&client, &owner, id, drawing_id, risk, now).await?;
    }
    // Read after the plan and the size, so a close by hand weighs the trade as it now is.
    let row = client.query_one("SELECT entry, start_at, qty, opened_at, fills FROM journal_trade WHERE id = $1", &[&id]).await?;
    let (entry, start_at, qty, opened_at): (f64, i64, f64, Option<i64>) = (row.get(0), row.get(1), row.get(2), row.get(3));
    if b.get("auto").and_then(Value::as_bool) == Some(true) {
        client
            .execute(
                "UPDATE journal_trade SET auto = true, status = 'planned', opened_at = NULL, closed_at = NULL, exit = NULL, checked_until = start_at, fills = '[]'::jsonb
                 WHERE id = $1",
                &[&id],
            )
            .await?;
    }
    if let Some(status) = b.get("status").and_then(Value::as_str) {
        let given = finite(b, "closedAt")?.map(|v| v as i64);
        if given.is_some_and(|at| at < opened_at.unwrap_or(start_at)) {
            return Err(ApiError::bad_request("it cannot close before it opened"));
        }
        let closed_at = given.unwrap_or(now);
        match status {
            "closed" | "breakeven" => {
                // What price took off on the way stays; the close is the hand's.
                let mut fills: Vec<Fill> = serde_json::from_value(row.get(4)).unwrap_or_default();
                fills.retain(|f| !matches!(f.kind.as_str(), "hand" | "stop" | "target"));
                if fills.is_empty() {
                    fills.push(Fill::new("entry", entry, qty, opened_at.unwrap_or(start_at), None));
                }
                let mut book = Book::of(&fills);
                let price = if status == "closed" {
                    finite(b, "exit")?.filter(|v| *v > 0.0).ok_or_else(|| ApiError::bad_request("closing by hand needs the exit price"))?
                } else {
                    book.average_entry()
                };
                let left = book.take(price, book.held());
                if left > 0.0 {
                    fills.push(Fill::new("hand", price, left, closed_at, None));
                }
                client
                    .execute(
                        "UPDATE journal_trade SET auto = false, status = $2, exit = $3, closed_at = $4, opened_at = COALESCE(opened_at, $5), fills = $6 WHERE id = $1",
                        &[&id, &status, &book.average_exit().unwrap_or(price), &closed_at, &start_at, &serde_json::to_value(&fills).unwrap_or_default()],
                    )
                    .await?;
            }
            "cancelled" => {
                client.execute("UPDATE journal_trade SET auto = false, status = 'cancelled', exit = NULL, closed_at = $2 WHERE id = $1", &[&id, &closed_at]).await?;
            }
            other => return Err(ApiError::bad_request(format!("a status set by hand is closed, breakeven or cancelled, not '{other}'"))),
        }
    }
    let row = client
        .query_one(&format!("UPDATE journal_trade SET updated_at = $2 WHERE id = $1 RETURNING {COLUMNS}"), &[&id, &now])
        .await?;
    Ok(Json(json!({ "trade": row_json(&row) })))
}

/// New prices for a trade, typed in the journal. One that follows a drawing
/// moves the drawing, and follows it as it would a drag on the chart; one
/// added by hand, or whose drawing is gone, takes them itself — weighed again
/// from its start while price decides it, else still closed where it was.
async fn replan(client: &tokio_postgres::Client, owner: &str, id: i64, drawing_id: Option<i64>, prices: [Option<f64>; 3], now: i64) -> ApiResult<()> {
    let row = client
        .query_one("SELECT side, entry, stop, target, risk, auto, status, qty, exit, opened_at, closed_at, fills, start_at FROM journal_trade WHERE id = $1", &[&id])
        .await?;
    let side = Side::of(row.get(0)).ok_or_else(|| ApiError::bad_request("the trade has no side"))?;
    let [entry, stop, target] = [prices[0].unwrap_or(row.get(1)), prices[1].unwrap_or(row.get(2)), prices[2].unwrap_or(row.get(3))];
    if [entry, stop, target] == [row.get::<_, f64>(1), row.get(2), row.get(3)] {
        return Ok(());
    }
    let sound = match side {
        Side::Long => stop < entry && entry < target,
        Side::Short => target < entry && entry < stop,
    };
    if !sound || entry <= 0.0 {
        return Err(ApiError::bad_request("a long's stop goes below its entry and its target above; a short's the other way round"));
    }
    if let Some(drawing_id) = drawing_id {
        let held = client
            .query_opt(&format!("SELECT {} FROM drawing WHERE id = $1 AND owner = $2", crate::drawings::COLUMNS), &[&drawing_id, &owner])
            .await?;
        if let Some(d) = held {
            let mut drawing = crate::drawings::row_json(&d);
            if let Some(points) = drawing["points"].as_array_mut() {
                for (point, price) in points.iter_mut().zip([entry, stop, target]) {
                    point["price"] = json!(price);
                }
            }
            client.execute("UPDATE drawing SET points = $2 WHERE id = $1", &[&drawing_id, &drawing["points"]]).await?;
            sync_drawing(client, owner, &drawing).await?;
            return Ok(());
        }
    }
    let qty = row.get::<_, f64>(4) / (entry - stop).abs();
    if row.get::<_, bool>(5) {
        client
            .execute(
                "UPDATE journal_trade SET entry = $2, stop = $3, target = $4, qty = $5, status = 'planned', opened_at = NULL, closed_at = NULL, exit = NULL,
                     checked_until = start_at, fills = '[]'::jsonb WHERE id = $1",
                &[&id, &entry, &stop, &target, &qty],
            )
            .await?;
    } else {
        let kept = Kept { status: row.get(6), qty: row.get(7), exit: row.get(8), opened_at: row.get(9), closed_at: row.get(10), fills: serde_json::from_value(row.get(11)).unwrap_or_default() };
        let (fills, exit) = kept.follow(true, entry, qty, row.get(12), now);
        client
            .execute(
                "UPDATE journal_trade SET entry = $2, stop = $3, target = $4, qty = $5, fills = COALESCE($6, fills), exit = COALESCE($7, exit) WHERE id = $1",
                &[&id, &entry, &stop, &target, &qty, &fills, &exit],
            )
            .await?;
    }
    Ok(())
}

/// A new risk for a trade, typed in the journal: the size scales with it, and
/// what was filled with the size. A drawing it follows is set to risk the same,
/// in the unit it keeps, so the chart shows it and moving the drawing keeps it.
async fn resize(client: &tokio_postgres::Client, owner: &str, id: i64, drawing_id: Option<i64>, risk: f64, now: i64) -> ApiResult<()> {
    if risk <= 0.0 {
        return Err(ApiError::bad_request("risk must be positive"));
    }
    let row = client
        .query_one("SELECT risk, qty, entry, stop, status, exit, opened_at, closed_at, fills, start_at FROM journal_trade WHERE id = $1", &[&id])
        .await?;
    let (old, qty, entry, stop): (f64, f64, f64, f64) = (row.get(0), row.get(1), row.get(2), row.get(3));
    let by = if old > 0.0 { risk / old } else { 1.0 };
    let sized = if old > 0.0 { qty * by } else { risk / (entry - stop).abs() };
    let kept = Kept { status: row.get(4), qty, exit: row.get(5), opened_at: row.get(6), closed_at: row.get(7), fills: serde_json::from_value(row.get(8)).unwrap_or_default() };
    let (fills, _) = kept.follow(false, entry, sized, row.get(9), now);
    client
        .execute("UPDATE journal_trade SET risk = $2, qty = $3, fills = COALESCE($4, fills) WHERE id = $1", &[&id, &risk, &sized, &fills])
        .await?;
    let Some(drawing_id) = drawing_id else { return Ok(()) };
    let Some(d) = client.query_opt("SELECT style FROM drawing WHERE id = $1 AND owner = $2", &[&drawing_id, &owner]).await? else { return Ok(()) };
    let mut style = d.get::<_, Value>(0).as_object().cloned().unwrap_or_default();
    // The drawing's own risk scales as the trade's did: with adds drawn, the trade's is the larger.
    let base = risk_of(&style) * by;
    let cash = style.get("riskUnit").and_then(Value::as_str).is_some_and(|u| u != "%");
    let value = if cash { base } else { base / style_num(&style, "account").unwrap_or(DEFAULT_ACCOUNT) * 100.0 };
    style.insert("risk".into(), json!((value * 1e8).round() / 1e8));
    client.execute("UPDATE drawing SET style = $2 WHERE id = $1", &[&drawing_id, &Value::Object(style)]).await?;
    Ok(())
}

/// `DELETE /api/journal/{id}`: gone from the journal; its drawing, if it has one, stays on the chart.
pub async fn delete_one(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let client = state.pool.get().await?;
    account_only(&client, &owner).await?;
    match client.execute("DELETE FROM journal_trade WHERE id = $1 AND owner = $2", &[&id, &owner]).await? {
        0 => Err(ApiError::not_found(format!("trade {id} not found"))),
        _ => Ok(Json(json!({ "id": id }))),
    }
}

/// An ISO week: `2026-W42`.
fn valid_week(week: &str) -> bool {
    let Some((year, w)) = week.split_once("-W") else { return false };
    year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()) && w.len() == 2 && w.parse::<u8>().is_ok_and(|n| (1..=53).contains(&n))
}

/// `GET /api/journal/reviews`: this browser's weekly reviews, by week.
pub async fn reviews(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let client = state.pool.get().await?;
    let rows = client.query("SELECT week, answers, saved_at FROM journal_review WHERE owner = $1 ORDER BY week DESC", &[&owner]).await?;
    let list: Vec<Value> = rows.iter().map(|r| json!({ "week": r.get::<_, String>(0), "answers": r.get::<_, Value>(1), "savedAt": r.get::<_, i64>(2) })).collect();
    Ok(Json(json!({ "reviews": list })))
}

/// `PUT /api/journal/reviews/{week}` `{answers: {q1: "…", …}}`: a week's review, as it now stands.
pub async fn save_review(State(state): State<AppState>, headers: HeaderMap, Path(week): Path<String>, Json(body): Json<Value>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    if !valid_week(&week) {
        return Err(ApiError::bad_request("week is an ISO week, like 2026-W42"));
    }
    let answers = body.get("answers").and_then(Value::as_object).ok_or_else(|| ApiError::bad_request("answers must be an object"))?;
    if answers.len() > 20 || answers.iter().any(|(k, v)| k.len() > 8 || !v.is_string() || v.as_str().is_some_and(|s| s.len() > MAX_TEXT)) {
        return Err(ApiError::bad_request(format!("answers are a few short keys, each a string of at most {MAX_TEXT} bytes")));
    }
    let now = now_ms();
    let client = state.pool.get().await?;
    account_only(&client, &owner).await?;
    client
        .execute(
            "INSERT INTO journal_review (owner, week, answers, saved_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (owner, week) DO UPDATE SET answers = EXCLUDED.answers, saved_at = EXCLUDED.saved_at",
            &[&owner, &week, &Value::Object(answers.clone()), &now],
        )
        .await?;
    Ok(Json(json!({ "week": week, "savedAt": now })))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 60 * MINUTE;

    fn pts(entry: f64, stop: f64, target: f64, time: f64) -> Vec<Point> {
        vec![Point { time, price: entry }, Point { time, price: stop }, Point { time: time + 10.0 * H as f64, price: target }]
    }

    fn bar(t: i64, h: f64, l: f64) -> Candle {
        candle(t, (h + l) / 2.0, h, l, (h + l) / 2.0)
    }

    #[test]
    fn a_position_risks_what_its_settings_say() {
        let style = |s: Value| s.as_object().unwrap().clone();
        assert_eq!(risk_of(&Map::new()), 10.0, "1% of a 1,000 account, as the chart draws it");
        assert_eq!(risk_of(&style(json!({ "account": "5000", "risk": 2 }))), 100.0);
        assert_eq!(risk_of(&style(json!({ "riskUnit": "USD", "risk": "25" }))), 25.0, "risk in cash");
        let plan = plan_of("long", &pts(100.0, 95.0, 110.0, 0.0), &Map::new(), 30 * H, "1h").unwrap();
        assert_eq!((plan.qty, plan.start_at), (2.0, 0), "ten risked over five a coin; a box wholly on bars past, weighed from its edge");
        assert!(plan_of("long", &pts(100.0, 105.0, 110.0, 0.0), &Map::new(), 0, "1h").is_none(), "a long's stop is under its entry");
        assert!(plan_of("trendline", &pts(100.0, 95.0, 110.0, 0.0), &Map::new(), 0, "1h").is_none());
    }

    #[test]
    fn a_position_reaching_the_present_is_weighed_from_when_it_was_drawn() {
        // Drawn half an hour into the 4h bar its edge sits on: that bar's earlier swings are not its own.
        let drawn = 4 * H + 30 * MINUTE;
        let plan = plan_of("short", &pts(100.0, 105.0, 90.0, (4 * H) as f64), &Map::new(), drawn, "4h").unwrap();
        assert_eq!(plan.start_at, drawn);
        // Its left edge twenty minutes back on a 1m chart, its box running on into the minutes to come: a trade taken now.
        let drawn = 100 * H;
        let plan = plan_of("long", &pts(100.0, 95.0, 110.0, (drawn - 20 * MINUTE) as f64), &Map::new(), drawn, "1m").unwrap();
        assert_eq!(plan.start_at, drawn);
    }

    fn trade(side: Side, entry: f64, stop: f64, target: f64) -> Trade {
        Trade { side, entry, stop, target, qty: 1.0, legs: Vec::new(), fills: Vec::new(), status: "planned".into(), opened_at: None, closed_at: None, exit: None, checked_until: 0 }
    }

    fn with_legs(mut t: Trade, qty: f64, legs: &str) -> Trade {
        let style = json!({ "legs": legs });
        t.qty = qty;
        t.legs = legs_of(style.as_object().unwrap(), t.entry, t.stop, t.target);
        t
    }

    fn kinds(t: &Trade) -> Vec<(&str, f64, f64)> {
        t.fills.iter().map(|f| (f.kind.as_str(), f.price, f.qty)).collect()
    }

    #[test]
    fn legs_are_read_where_the_prices_allow_them() {
        let style = json!({ "legs": "t,110,50|c,95,25|a,97,100|t,95,50|c,105,50|a,100,50|a,130,50|x,1,1|t,115,0|t,112" });
        let legs = legs_of(style.as_object().unwrap(), 100.0, 90.0, 120.0);
        assert_eq!(
            legs,
            vec![Leg { kind: LegKind::Take, price: 110.0, share: 0.5 }, Leg { kind: LegKind::Cut, price: 95.0, share: 0.25 }, Leg { kind: LegKind::Add, price: 97.0, share: 1.0 }],
            "a take past the entry towards the target, a cut towards the stop, an add between them but not at the entry"
        );
    }

    #[test]
    fn a_position_risks_most_with_its_adds_filled_and_least_with_its_cuts() {
        let legs = |raw: &str| legs_of(json!({ "legs": raw }).as_object().unwrap(), 100.0, 90.0, 120.0);
        assert_eq!(worst_loss(Side::Long, 100.0, 90.0, 1.0, &[]), 10.0);
        assert_eq!(worst_loss(Side::Long, 100.0, 90.0, 1.0, &legs("a,95,100")), 15.0, "one more coin from 95 to 90");
        assert_eq!(worst_loss(Side::Long, 100.0, 90.0, 1.0, &legs("c,95,50")), 7.5, "half cut at 95, half to 90");
        assert_eq!(worst_loss(Side::Long, 100.0, 90.0, 1.0, &legs("t,110,50")), 10.0, "a take never comes on the way down");
        let plan = plan_of("long", &pts(100.0, 90.0, 120.0, 0.0), json!({ "riskUnit": "cash", "risk": 10, "legs": "a,95,100" }).as_object().unwrap(), 30 * H, "1h").unwrap();
        assert_eq!((plan.qty, plan.risk, plan.legs.len()), (1.0, 15.0, 1), "the entry's size from the risk set; the risk counted with the add");
    }

    #[test]
    fn price_takes_part_off_on_the_way_and_the_target_takes_the_rest() {
        let mut t = with_legs(trade(Side::Long, 100.0, 90.0, 120.0), 2.0, "t,110,50");
        t.walk(&[bar(0, 101.0, 99.0), bar(MINUTE, 111.0, 104.0), bar(2 * MINUTE, 121.0, 108.0)]);
        assert_eq!(kinds(&t), vec![("entry", 100.0, 2.0), ("take", 110.0, 1.0), ("target", 120.0, 1.0)]);
        assert_eq!((t.status.as_str(), t.exit, t.closed_at), ("win", Some(115.0), Some(2 * MINUTE)), "the exit is the average of the two");
        // Half taken, then stopped: a loss by how it ended, nothing made in all.
        let mut t = with_legs(trade(Side::Long, 100.0, 90.0, 120.0), 2.0, "t,110,50");
        t.walk(&[bar(0, 101.0, 99.0), bar(MINUTE, 111.0, 104.0), bar(2 * MINUTE, 105.0, 89.0)]);
        assert_eq!(kinds(&t), vec![("entry", 100.0, 2.0), ("take", 110.0, 1.0), ("stop", 90.0, 1.0)]);
        assert_eq!((t.status.as_str(), Book::of(&t.fills).made(1.0)), ("loss", 0.0));
    }

    #[test]
    fn a_cut_and_an_add_fill_on_the_way_to_the_stop_nearest_first() {
        // A short: adds at 104, cuts half of the entry's size at 107, stopped at 110.
        let mut t = with_legs(trade(Side::Short, 100.0, 110.0, 80.0), 1.0, "c,107,50|a,104,100");
        t.walk(&[bar(0, 101.0, 99.0), bar(MINUTE, 105.0, 101.0), bar(2 * MINUTE, 111.0, 103.0)]);
        assert_eq!(kinds(&t), vec![("entry", 100.0, 1.0), ("add", 104.0, 1.0), ("cut", 107.0, 0.5), ("stop", 110.0, 1.5)]);
        assert_eq!(t.status, "loss");
        assert_eq!(Book::of(&t.fills).made(-1.0), -14.5, "two coins from an average of 102: 0.5 cut at 107, 1.5 stopped at 110");
        // The add filled, then the target: both coins from the average of 98.
        let mut t = with_legs(trade(Side::Long, 100.0, 90.0, 120.0), 1.0, "a,96,100");
        t.walk(&[bar(0, 101.0, 99.0), bar(MINUTE, 99.0, 95.0), bar(2 * MINUTE, 121.0, 100.0)]);
        assert_eq!((t.status.as_str(), Book::of(&t.fills).made(1.0)), ("win", 44.0));
    }

    #[test]
    fn a_minute_reaching_every_level_reads_the_stops_side_first() {
        let mut t = with_legs(trade(Side::Long, 100.0, 90.0, 120.0), 1.0, "t,110,50|a,95,100");
        t.walk(&[bar(0, 101.0, 99.0), bar(MINUTE, 125.0, 85.0)]);
        assert_eq!(kinds(&t), vec![("entry", 100.0, 1.0), ("add", 95.0, 1.0), ("stop", 90.0, 2.0)], "the take never comes");
    }

    #[test]
    fn a_trade_opened_before_fills_were_kept_is_held_whole() {
        let mut t = trade(Side::Long, 100.0, 90.0, 120.0);
        (t.status, t.opened_at, t.checked_until) = ("open".into(), Some(0), MINUTE);
        t.walk(&[bar(MINUTE, 121.0, 101.0)]);
        assert_eq!(kinds(&t), vec![("entry", 100.0, 1.0), ("target", 120.0, 1.0)]);
    }

    #[test]
    fn price_opens_a_trade_at_its_entry_and_closes_it_at_target_or_stop() {
        let mut t = trade(Side::Long, 100.0, 95.0, 110.0);
        t.walk(&[bar(0, 104.0, 101.0), bar(MINUTE, 102.0, 99.5), bar(2 * MINUTE, 108.0, 101.0), bar(3 * MINUTE, 111.0, 104.0), bar(4 * MINUTE, 120.0, 90.0)]);
        assert_eq!((t.status.as_str(), t.opened_at, t.closed_at, t.exit), ("win", Some(MINUTE), Some(3 * MINUTE), Some(110.0)));
        assert_eq!(t.checked_until, 4 * MINUTE, "nothing read past the close");
        let mut short = trade(Side::Short, 100.0, 105.0, 90.0);
        short.walk(&[bar(0, 100.5, 99.0), bar(MINUTE, 106.0, 98.0)]);
        assert_eq!((short.status.as_str(), short.exit), ("loss", Some(105.0)), "the short's stop is over it");
    }

    #[test]
    fn a_minute_reaching_both_counts_as_the_stop() {
        let mut t = trade(Side::Long, 100.0, 95.0, 110.0);
        t.walk(&[bar(0, 101.0, 99.0), bar(MINUTE, 112.0, 94.0)]);
        assert_eq!(t.status, "loss");
    }

    #[test]
    fn a_planned_trade_waits_for_its_entry_and_reads_on_from_where_it_stopped() {
        let mut t = trade(Side::Long, 90.0, 85.0, 100.0);
        t.walk(&[bar(0, 99.0, 95.0), bar(MINUTE, 101.0, 96.0)]);
        assert_eq!((t.status.as_str(), t.checked_until), ("planned", 2 * MINUTE), "the target reached before the entry is no trade");
        t.walk(&[bar(0, 99.0, 95.0), bar(MINUTE, 101.0, 96.0), bar(2 * MINUTE, 96.0, 89.0)]);
        assert_eq!((t.status.as_str(), t.opened_at), ("open", Some(2 * MINUTE)), "read again from the third minute only");
        // Price jumping over the entry between two minutes reaches it too.
        let mut gap = trade(Side::Short, 100.0, 110.0, 80.0);
        gap.walk(&[bar(0, 98.0, 97.0), bar(MINUTE, 103.0, 101.0)]);
        assert_eq!(gap.status, "open");
    }

    #[test]
    fn weeks_are_iso_weeks() {
        assert!(valid_week("2026-W42"));
        assert!(!valid_week("2026-W54") && !valid_week("2026-42") && !valid_week("26-W01"));
    }

    #[test]
    fn only_markets_whose_minutes_are_read_are_followed() {
        assert!(tracked("binance-perp:BTCUSDT") && tracked("hyperliquid-perp:SP500USD") && tracked("okx-spot:BTCUSDT"));
        assert!(!tracked("index:TOTAL") && !tracked("kraken-spot:BTCUSD") && !tracked("lighter-perp:ETHUSD"));
    }

    fn kept(status: &str, qty: f64, exit: Option<f64>, fills: Vec<Fill>) -> Kept {
        Kept { status: status.into(), qty, exit, opened_at: Some(10), closed_at: Some(20), fills }
    }

    #[test]
    fn a_trade_closed_by_hand_follows_its_moved_plan_to_the_same_exit() {
        let k = kept("closed", 2.0, Some(110.0), vec![Fill::new("entry", 100.0, 2.0, 10, None), Fill::new("hand", 110.0, 2.0, 20, None)]);
        let (fills, exit) = k.follow(true, 104.0, 4.0, 0, 99);
        let fills: Vec<Fill> = serde_json::from_value(fills.unwrap()).unwrap();
        assert_eq!(fills, vec![Fill::new("entry", 104.0, 4.0, 10, None), Fill::new("hand", 110.0, 4.0, 20, None)], "in at the new entry, the new size, out where the hand said");
        assert_eq!(exit, Some(110.0));
        let even = kept("breakeven", 2.0, Some(100.0), vec![]);
        assert_eq!(even.follow(true, 104.0, 4.0, 0, 99).1, Some(104.0), "evened is out at the entry, wherever it is now");
    }

    #[test]
    fn a_new_size_scales_what_was_filled_and_nothing_else_moves() {
        let k = kept("win", 2.0, Some(120.0), vec![Fill::new("entry", 100.0, 2.0, 10, None), Fill::new("take", 110.0, 1.0, 15, Some(0)), Fill::new("target", 120.0, 1.0, 20, None)]);
        let fills: Vec<Fill> = serde_json::from_value(k.follow(false, 100.0, 5.0, 0, 99).0.unwrap()).unwrap();
        assert_eq!(fills.iter().map(|f| f.qty).collect::<Vec<_>>(), vec![5.0, 2.5, 2.5], "the same trade at two and a half times the size");
        assert_eq!(Book::of(&fills).made(1.0), 75.0);
        assert_eq!(k.follow(false, 100.0, 2.0, 0, 99), (None, None), "the same size: nothing to write");
        assert_eq!(kept("planned", 2.0, None, vec![]).follow(true, 104.0, 4.0, 0, 99), (None, None), "nothing filled yet");
    }
}
