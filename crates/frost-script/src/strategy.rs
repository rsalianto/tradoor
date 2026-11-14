//! Backtesting: the orders a strategy script gives, filled bar by bar as an
//! exchange would fill them, with nothing taken from a bar not yet closed.
//!
//! A script says what it wants at a bar's close: `long(…)`, `short(…)`,
//! `exit(…)`, and where its stop and target stand. What it asks for is filled
//! at the next bar's open (or at this close, where the settings say so), a
//! limit order where the next bar reaches its price. Stops, targets, a trailing
//! stop and liquidation are then watched through the bars that follow.
//!
//! A bar's open, high, low and close do not say in which order its high and
//! low came. Where a bar reached both a position's stop and its target, the
//! lower timeframe's bars inside it say which came first; where none are at
//! hand, the stop is taken as first — the worse of the two — and the bar is
//! counted, so the report can say how many were decided that way.
//!
//! Costs are those of the exchange: a taker's fee on market and stop fills, a
//! maker's on limit fills and targets, slippage against every market and stop
//! fill, funding on a perpetual each time it settles, and the whole margin
//! lost at the liquidation price.
use crate::Bars;

/// Trades kept at most: past this, no new one is opened.
pub const MAX_TRADES: usize = 20_000;
/// Bars whose order is asked of the lower timeframe at most.
pub const MAX_ASKED: usize = 10_000;

const NA: f64 = f64::NAN;

/// How big a position is opened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// A share of the equity, as margin: 1.0 the whole of it.
    Equity(f64),
    /// This much of the quote currency as margin, or the equity where it is less.
    Fixed(f64),
    /// As much as loses this share of the equity at the first stop.
    Risk(f64),
}

/// A stop, target or trailing distance the settings give, from the entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exit {
    None,
    /// A share of the entry price: 0.02 is 2%.
    Pct(f64),
    /// This many times the average true range at the bar the order was given on.
    Atr(f64),
    /// A target this many times the stop's distance.
    R(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The equity to start with, in the quote currency.
    pub capital: f64,
    pub size: Size,
    /// At least 1.
    pub leverage: f64,
    /// Fees as shares of what changes hands: 0.0005 is 0.05%.
    pub taker: f64,
    pub maker: f64,
    /// Lost against each market and stop fill, as a share of the price.
    pub slippage: f64,
    /// Filled at the close of the bar the order was given on, rather than the next open.
    pub on_close: bool,
    pub longs: bool,
    pub shorts: bool,
    pub stop: Exit,
    pub target: Exit,
    pub trail: Exit,
    pub atr_length: usize,
    /// Bars a position is held at most: 0, as long as it lasts.
    pub max_bars: usize,
    /// The maintenance margin rate a liquidation is reckoned at; 0 for none.
    pub mmr: f64,
    /// Positions open from this moment only, and close at `to`: NaN for no bound.
    pub from: f64,
    pub to: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capital: 10_000.0,
            size: Size::Equity(1.0),
            leverage: 1.0,
            taker: 0.0005,
            maker: 0.0002,
            slippage: 0.0,
            on_close: false,
            longs: true,
            shorts: true,
            stop: Exit::None,
            target: Exit::None,
            trail: Exit::None,
            atr_length: 14,
            max_bars: 0,
            mmr: 0.0,
            from: NA,
            to: NA,
        }
    }
}

/// Bars of a lower timeframe, oldest first: where a bar's high and low came in order.
#[derive(Clone, Copy, Default)]
pub struct Inner<'a> {
    pub time: &'a [f64],
    pub open: &'a [f64],
    pub high: &'a [f64],
    pub low: &'a [f64],
    pub close: &'a [f64],
}

/// A perpetual's funding, oldest first: when each settled, and its rate (positive, longs pay).
#[derive(Clone, Copy, Default)]
pub struct Funding<'a> {
    pub time: &'a [f64],
    pub rate: &'a [f64],
}

/// Why a position closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Still open at the last bar: its figures are at that bar's close.
    Open = 0,
    Signal = 1,
    Stop = 2,
    Target = 3,
    Trail = 4,
    Time = 5,
    Liquidation = 6,
    Reverse = 7,
    /// The test's range ended.
    End = 8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trade {
    pub long: bool,
    pub entry_bar: usize,
    pub entry_time: f64,
    pub entry_price: f64,
    pub exit_bar: usize,
    pub exit_time: f64,
    pub exit_price: f64,
    pub qty: f64,
    pub margin: f64,
    /// What it made, its fees and funding taken off.
    pub pnl: f64,
    pub fees: f64,
    /// Paid; negative where it was taken.
    pub funding: f64,
    /// What its first stop would have lost: NaN where it had none.
    pub risk: f64,
    pub reason: Reason,
    /// How far it went against and for, as shares of the entry price.
    pub mae: f64,
    pub mfe: f64,
    /// The equity it was opened from.
    pub equity_before: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub trades: Vec<Trade>,
    /// The equity at each bar's close, an open position at that close.
    pub equity: Vec<f64>,
    /// Bars where a stop and a target were both in reach and no lower bars were at hand: taken as stopped.
    pub ambiguous: Vec<usize>,
    /// Bars the lower timeframe's bars decided.
    pub magnified: usize,
    /// Orders not filled: risk sizing with no stop to size by, or no equity left.
    pub skipped: usize,
    /// Bars spent in a position.
    pub held: usize,
    /// What waits for the next bar's open: 1 an entry long, -1 short, 2 an exit, 0 nothing.
    pub next: i8,
    /// The entry's limit price, NaN at the market.
    pub next_limit: f64,
}

/// What a script asked for at one bar's close.
#[derive(Clone, Copy, Debug)]
pub struct Signals {
    pub long: bool,
    /// A limit price; NaN at the market.
    pub long_at: f64,
    pub short: bool,
    pub short_at: f64,
    pub exit: bool,
    /// A stop or target given on this bar: it stands from the next bar until another is given or the
    /// position closes, as Pine's strategy.exit does. Given na, there is none.
    pub stop: Option<f64>,
    pub target: Option<f64>,
}

impl Default for Signals {
    fn default() -> Self {
        Self { long: false, long_at: NA, short: false, short_at: NA, exit: false, stop: None, target: None }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pos {
    long: bool,
    qty: f64,
    entry: f64,
    margin: f64,
    entry_bar: usize,
    entry_time: f64,
    fees: f64,
    funding: f64,
    /// Fixed at the entry from the settings and the order's bar: NaN for none.
    stop: f64,
    target: f64,
    /// The trailing stop's distance, or its share of the best price.
    trail_abs: f64,
    trail_pct: f64,
    /// The best price since the entry, through the last bar: what a trailing stop follows.
    best: f64,
    risk: f64,
    worst_seen: f64,
    best_seen: f64,
    equity_before: f64,
    liq: f64,
    held: usize,
}

/// An order to be filled at the next bar.
#[derive(Clone, Copy, Debug)]
struct Entry {
    long: bool,
    limit: f64,
    /// The script's stop and target at the bar it was given on.
    stop: f64,
    target: f64,
    atr: f64,
}

/// One stretch of a bar's path: the bar itself, or one of the lower timeframe's inside it.
#[derive(Clone, Copy)]
struct Seg {
    open: f64,
    high: f64,
    low: f64,
}

/// What a stretch did to a position.
enum Hit {
    None,
    Exit(f64, Reason, bool),
    Liquidated(f64),
    /// What came first is not known from the stretch alone. `against`: a stop or the
    /// liquidation was in reach, and is taken as first; else only a target was, and is not taken.
    Unknown { against: bool },
}

pub struct Broker<'a> {
    s: Settings,
    inner: Inner<'a>,
    funding: Funding<'a>,
    interval: f64,
    cash: f64,
    pos: Option<Pos>,
    entry: Option<Entry>,
    exit: Option<Reason>,
    script_stop: f64,
    script_target: f64,
    /// Wilder's average true range, through the last closed bar.
    atr: f64,
    tr_sum: f64,
    tr_n: usize,
    funded: usize,
    report: Report,
    side: Vec<f64>,
    entry_price: Vec<f64>,
    held: Vec<f64>,
}

fn fin(v: f64) -> bool {
    v.is_finite()
}

/// The nearer of two levels to the price on the side given, either NaN for none.
fn nearer(a: f64, b: f64, above: bool) -> f64 {
    match (fin(a), fin(b)) {
        (true, true) => {
            if above {
                a.min(b)
            } else {
                a.max(b)
            }
        }
        (true, false) => a,
        (false, true) => b,
        _ => NA,
    }
}

impl<'a> Broker<'a> {
    pub fn new(settings: Settings, inner: Inner<'a>, funding: Funding<'a>, interval: f64, n: usize) -> Self {
        let mut s = settings;
        s.leverage = if fin(s.leverage) && s.leverage >= 1.0 { s.leverage } else { 1.0 };
        s.atr_length = s.atr_length.max(1);
        let capital = if fin(s.capital) && s.capital > 0.0 { s.capital } else { 10_000.0 };
        s.capital = capital;
        Self {
            s,
            inner,
            funding,
            interval,
            cash: capital,
            pos: None,
            entry: None,
            exit: None,
            script_stop: NA,
            script_target: NA,
            atr: NA,
            tr_sum: 0.0,
            tr_n: 0,
            funded: 0,
            report: Report { equity: Vec::with_capacity(n), ..Report::default() },
            side: vec![0.0; n],
            entry_price: vec![NA; n],
            held: vec![NA; n],
        }
    }

    /// `inlong`, `inshort`, `entryprice` and `barsheld` at bar `i`: after its fills, before its close's orders.
    pub fn side_at(&self, i: usize) -> f64 {
        self.side.get(i).copied().unwrap_or(0.0)
    }

    pub fn entry_at(&self, i: usize) -> f64 {
        self.entry_price.get(i).copied().unwrap_or(NA)
    }

    pub fn held_at(&self, i: usize) -> f64 {
        self.held.get(i).copied().unwrap_or(NA)
    }

    fn equity(&self, price: f64) -> f64 {
        match &self.pos {
            Some(p) => self.cash + if p.long { 1.0 } else { -1.0 } * p.qty * (price - p.entry),
            None => self.cash,
        }
    }

    // ------------------------------------------------------------ a bar's open, and the path to its close

    /// Before the script reads bar `i`: what was asked at the last close filled, and the bar's path watched.
    pub fn open_bar(&mut self, i: usize, bars: &Bars) {
        let (t, o, h, l, c) = (bars.time[i], bars.open[i], bars.high[i], bars.low[i], bars.close[i]);
        // Funding settled at or before this open fell on the position held up to it.
        self.settle(t, true, o);
        if let Some(reason) = self.exit.take() {
            let slip = self.s.slippage;
            let price = match &self.pos {
                Some(p) if p.long => o * (1.0 - slip),
                _ => o * (1.0 + slip),
            };
            self.close_at(i, t, price, true, reason);
        }
        let mut filled = false;
        if let Some(e) = self.entry.take() {
            filled = self.fill_entry(i, t, e, Seg { open: o, high: h, low: l });
        }
        if self.pos.is_some() && fin(o) && fin(h) && fin(l) {
            self.walk(i, t, Seg { open: o, high: h, low: l }, filled);
        }
        // Settled inside the bar: on the position still held at its close.
        self.settle(t + self.interval, false, c);
        if let Some(p) = &mut self.pos {
            p.best = if p.long { p.best.max(h) } else { p.best.min(l) };
            self.report.held += 1;
        }
        self.mark(i);
    }

    fn mark(&mut self, i: usize) {
        if let Some(cell) = self.side.get_mut(i) {
            *cell = match &self.pos {
                Some(p) if p.long => 1.0,
                Some(_) => -1.0,
                None => 0.0,
            };
        }
        if let (Some(e), Some(h)) = (self.entry_price.get_mut(i), self.held.get_mut(i)) {
            match &self.pos {
                Some(p) => {
                    *e = p.entry;
                    *h = p.held as f64;
                }
                None => {
                    *e = NA;
                    *h = NA;
                }
            }
        }
    }

    /// Funding settled up to `until` (at or before an open, or before the next bar's), paid by the position held.
    fn settle(&mut self, until: f64, inclusive: bool, price: f64) {
        let f = self.funding;
        while self.funded < f.time.len().min(f.rate.len()) {
            let at = f.time[self.funded];
            if !(at < until || (inclusive && at == until)) {
                break;
            }
            self.funded += 1;
            let rate = f.rate[self.funded - 1];
            if let Some(p) = &mut self.pos
                && fin(rate)
                && fin(price)
                && at >= p.entry_time
            {
                let paid = if p.long { 1.0 } else { -1.0 } * p.qty * price * rate;
                p.funding += paid;
                self.cash -= paid;
            }
        }
    }

    /// The order's fill, where it is filled: at the open, or where a limit's price is reached.
    /// True where a limit filled past the open, inside the bar.
    fn fill_entry(&mut self, i: usize, t: f64, e: Entry, bar: Seg) -> bool {
        if !fin(bar.open) {
            return false;
        }
        if !fin(e.limit) {
            let price = bar.open * (1.0 + if e.long { self.s.slippage } else { -self.s.slippage });
            self.open_position(i, t, e, price, false);
            return false;
        }
        // A limit at or better than the open fills there; past it, where its price is reached.
        let better = if e.long { bar.open <= e.limit } else { bar.open >= e.limit };
        if better {
            self.open_position(i, t, e, bar.open, true);
            return false;
        }
        let reached = if e.long { bar.low <= e.limit } else { bar.high >= e.limit };
        if reached {
            self.open_position(i, t, e, e.limit, true);
        }
        reached && self.pos.is_some()
    }

    fn open_position(&mut self, i: usize, t: f64, e: Entry, price: f64, maker: bool) {
        if self.report.trades.len() >= MAX_TRADES || !fin(price) || price <= 0.0 {
            return;
        }
        let equity = self.cash;
        if equity <= 0.0 {
            self.report.skipped += 1;
            return;
        }
        let s = self.s;
        let dir = if e.long { 1.0 } else { -1.0 };
        // The first stop: the settings', or the script's, whichever is nearer the price.
        let from_settings = match s.stop {
            Exit::Pct(p) if p > 0.0 => price * (1.0 - dir * p),
            Exit::Atr(k) if k > 0.0 && fin(e.atr) => price - dir * k * e.atr,
            _ => NA,
        };
        let scripts = if fin(e.stop) && (e.stop - price) * dir < 0.0 { e.stop } else { NA };
        let stop = nearer(from_settings, scripts, !e.long);
        let distance = if fin(stop) { (price - stop).abs() } else { NA };
        let most = equity * s.leverage / price;
        let qty = match s.size {
            Size::Equity(f) => equity * f.max(0.0).min(1.0) * s.leverage / price,
            Size::Fixed(m) => m.max(0.0).min(equity) * s.leverage / price,
            Size::Risk(r) => {
                if !(distance > 0.0) {
                    self.report.skipped += 1;
                    return;
                }
                (equity * r.max(0.0) / distance).min(most)
            }
        };
        if !(qty > 0.0) || !fin(qty) {
            self.report.skipped += 1;
            return;
        }
        let notional = qty * price;
        let fee = notional * if maker { s.maker } else { s.taker };
        self.cash -= fee;
        let target_settings = match s.target {
            Exit::Pct(p) if p > 0.0 => price * (1.0 + dir * p),
            Exit::Atr(k) if k > 0.0 && fin(e.atr) => price + dir * k * e.atr,
            Exit::R(k) if k > 0.0 && distance > 0.0 => price + dir * k * distance,
            _ => NA,
        };
        let target_script = if fin(e.target) && (e.target - price) * dir > 0.0 { e.target } else { NA };
        let target = nearer(target_settings, target_script, e.long);
        let (trail_abs, trail_pct) = match s.trail {
            Exit::Pct(p) if p > 0.0 => (NA, p),
            Exit::Atr(k) if k > 0.0 && fin(e.atr) => (k * e.atr, NA),
            _ => (NA, NA),
        };
        let margin = notional / s.leverage;
        // Where what is left of the margin is the maintenance margin: as Training reckons it.
        let liq = if s.mmr > 0.0 {
            let at = if e.long { (price * qty - margin) / (qty * (1.0 - s.mmr)) } else { (price * qty + margin) / (qty * (1.0 + s.mmr)) };
            if at > 0.0 { at } else { NA }
        } else {
            NA
        };
        self.pos = Some(Pos {
            long: e.long,
            qty,
            entry: price,
            margin,
            entry_bar: i,
            entry_time: t,
            fees: fee,
            funding: 0.0,
            stop,
            target,
            trail_abs,
            trail_pct,
            best: price,
            risk: if distance > 0.0 { qty * distance } else { NA },
            worst_seen: price,
            best_seen: price,
            equity_before: equity,
            liq,
            held: 0,
        });
    }

    /// The stop in force for the bar: the entry's, the script's from the last close, and the trailing one.
    fn stop_now(&self, p: &Pos) -> f64 {
        let trail = if fin(p.trail_abs) {
            if p.long { p.best - p.trail_abs } else { p.best + p.trail_abs }
        } else if fin(p.trail_pct) {
            p.best * if p.long { 1.0 - p.trail_pct } else { 1.0 + p.trail_pct }
        } else {
            NA
        };
        let script = self.script_stop;
        // The highest of a long's stops, the lowest of a short's: each would close it.
        let both = nearer(p.stop, script, !p.long);
        nearer(both, trail, !p.long)
    }

    /// Which of the stops is the one in force, for the reason a close is given.
    fn stop_reason(&self, p: &Pos, level: f64) -> Reason {
        let fixed = nearer(p.stop, self.script_stop, !p.long);
        if fin(fixed) && level == fixed { Reason::Stop } else { Reason::Trail }
    }

    fn target_now(&self, p: &Pos) -> f64 {
        nearer(p.target, self.script_target, p.long)
    }

    /// Bar `i`'s path, through its lower-timeframe bars where they are at hand.
    fn walk(&mut self, i: usize, t: f64, bar: Seg, filled: bool) {
        let Some(p) = self.pos else { return };
        let against = match self.check(&p, bar, filled) {
            Hit::Unknown { against } => against,
            hit => {
                self.apply(i, t, bar, hit);
                return;
            }
        };
        let (from, to) = self.inner_of(t);
        if from == to {
            if self.report.ambiguous.len() < MAX_ASKED {
                self.report.ambiguous.push(i);
            }
            self.worse_first(i, t, bar, against);
            return;
        }
        self.report.magnified += 1;
        // A limit filled inside the bar: its lower bars from the first that reached its price.
        let mut waiting = filled;
        for k in from..to {
            let seg = Seg { open: self.inner.open[k], high: self.inner.high[k], low: self.inner.low[k] };
            let Some(now) = self.pos else { return };
            let here = waiting && if now.long { seg.low <= now.entry } else { seg.high >= now.entry };
            if waiting && !here {
                continue;
            }
            waiting = false;
            match self.check(&now, seg, here) {
                Hit::None => self.seen(seg, None),
                Hit::Unknown { against } => {
                    // Both inside one lower bar too.
                    self.worse_first(i, t, seg, against);
                    return;
                }
                hit => {
                    self.apply(i, t, seg, hit);
                    return;
                }
            }
        }
    }

    fn apply(&mut self, i: usize, t: f64, seg: Seg, hit: Hit) {
        match hit {
            Hit::None | Hit::Unknown { .. } => self.seen(seg, None),
            Hit::Exit(price, reason, maker) => {
                self.seen(seg, Some(price));
                self.close_at(i, t, price, !maker, reason);
            }
            Hit::Liquidated(price) => {
                self.seen(seg, Some(price));
                self.liquidate(i, t, price);
            }
        }
    }

    /// Where the order inside a stretch is not known: the stop (or the liquidation before
    /// it) first when one was in reach; else the target untaken.
    fn worse_first(&mut self, i: usize, t: f64, seg: Seg, against: bool) {
        let Some(p) = self.pos else { return };
        if !against {
            self.seen(seg, None);
            return;
        }
        let stop = self.stop_now(&p);
        let liq_first = fin(p.liq) && (!fin(stop) || if p.long { p.liq > stop } else { p.liq < stop });
        if liq_first {
            self.seen(seg, Some(p.liq));
            self.liquidate(i, t, p.liq);
        } else {
            let s = self.s.slippage;
            let price = if p.long { seg.open.min(stop) * (1.0 - s) } else { seg.open.max(stop) * (1.0 + s) };
            let reason = self.stop_reason(&p, stop);
            self.seen(seg, Some(price));
            self.close_at(i, t, price, true, reason);
        }
    }

    /// The lower bars inside the bar opening at `t`, as a range of their index.
    fn inner_of(&self, t: f64) -> (usize, usize) {
        let times = self.inner.time;
        let n = times.len().min(self.inner.open.len()).min(self.inner.high.len()).min(self.inner.low.len()).min(self.inner.close.len());
        let end_t = t + self.interval;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if times[mid] < t { lo = mid + 1 } else { hi = mid }
        }
        let start = lo;
        let mut end = start;
        while end < n && times[end] < end_t {
            end += 1;
        }
        if end > start { (start, end) } else { (0, 0) }
    }

    /// A stretch's highs and lows on the position's furthest against and for; to `exit` where it closed there.
    fn seen(&mut self, seg: Seg, exit: Option<f64>) {
        if let Some(p) = &mut self.pos {
            let (lo, hi) = match exit {
                Some(x) => (seg.open.min(x), seg.open.max(x)),
                None => (seg.low, seg.high),
            };
            if fin(lo) && fin(hi) {
                if p.long {
                    p.worst_seen = p.worst_seen.min(lo);
                    p.best_seen = p.best_seen.max(hi);
                } else {
                    p.worst_seen = p.worst_seen.max(hi);
                    p.best_seen = p.best_seen.min(lo);
                }
            }
        }
    }

    /// What one stretch does to a position. `filled` says its limit filled in this stretch, past
    /// its open: the stop is then certain where its low reaches it, the target not, as its high may have come first.
    fn check(&self, p: &Pos, seg: Seg, filled: bool) -> Hit {
        let stop = self.stop_now(p);
        let target = self.target_now(p);
        let (o, h, l) = (seg.open, seg.high, seg.low);
        let s = self.s.slippage;
        if !filled {
            // At the open: a gap past a level is filled there.
            if p.long {
                if fin(stop) && o <= stop {
                    return Hit::Exit(o * (1.0 - s), self.stop_reason(p, stop), false);
                }
                if fin(p.liq) && o <= p.liq {
                    return Hit::Liquidated(o);
                }
                if fin(target) && o >= target {
                    return Hit::Exit(o, Reason::Target, true);
                }
            } else {
                if fin(stop) && o >= stop {
                    return Hit::Exit(o * (1.0 + s), self.stop_reason(p, stop), false);
                }
                if fin(p.liq) && o >= p.liq {
                    return Hit::Liquidated(o);
                }
                if fin(target) && o <= target {
                    return Hit::Exit(o, Reason::Target, true);
                }
            }
        }
        let (stopped, liquidated, reached) = if p.long {
            (fin(stop) && l <= stop, fin(p.liq) && l <= p.liq, fin(target) && h >= target)
        } else {
            (fin(stop) && h >= stop, fin(p.liq) && h >= p.liq, fin(target) && l <= target)
        };
        // Of the two against it, the one the price reaches first.
        let liq_first = liquidated && (!fin(stop) || if p.long { p.liq > stop } else { p.liq < stop });
        let against = stopped || liquidated;
        if against && reached {
            return Hit::Unknown { against: true };
        }
        if filled && reached {
            // A target past a limit's fill: its high may have come before the fill.
            return Hit::Unknown { against: false };
        }
        if liq_first {
            return Hit::Liquidated(p.liq);
        }
        if stopped {
            let price = stop * if p.long { 1.0 - s } else { 1.0 + s };
            return Hit::Exit(price, self.stop_reason(p, stop), false);
        }
        if reached && !filled {
            return Hit::Exit(target, Reason::Target, true);
        }
        Hit::None
    }

    /// The position closed at `price`: a taker's fee where `taker`, a maker's else.
    fn close_at(&mut self, i: usize, t: f64, price: f64, taker: bool, reason: Reason) {
        let Some(p) = self.pos.take() else { return };
        if !fin(price) {
            self.pos = Some(p);
            return;
        }
        let dir = if p.long { 1.0 } else { -1.0 };
        let gross = dir * p.qty * (price - p.entry);
        let fee = p.qty * price * if taker { self.s.taker } else { self.s.maker };
        self.cash += gross - fee;
        let pnl = self.cash - p.equity_before;
        self.record(p, i, t, price, p.fees + fee, pnl, reason);
    }

    /// All of the margin gone at the liquidation price.
    fn liquidate(&mut self, i: usize, t: f64, price: f64) {
        let Some(p) = self.pos.take() else { return };
        self.cash -= p.margin;
        let pnl = self.cash - p.equity_before;
        self.record(p, i, t, price, p.fees, pnl, Reason::Liquidation);
    }

    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, p: Pos, i: usize, t: f64, price: f64, fees: f64, pnl: f64, reason: Reason) {
        // A closed position's stop and target go with it: the next is guarded by its own.
        self.script_stop = NA;
        self.script_target = NA;
        let (worst, best) = if p.long {
            ((p.entry - p.worst_seen.min(price)).max(0.0), (p.best_seen.max(price) - p.entry).max(0.0))
        } else {
            ((p.worst_seen.max(price) - p.entry).max(0.0), (p.entry - p.best_seen.min(price)).max(0.0))
        };
        // Filled at a close: the trade's moment is the close's, the end of its bar.
        let exit_time = if self.s.on_close && matches!(reason, Reason::Signal | Reason::Reverse | Reason::Time | Reason::End) { t + self.interval } else { t };
        self.report.trades.push(Trade {
            long: p.long,
            entry_bar: p.entry_bar,
            entry_time: p.entry_time,
            entry_price: p.entry,
            exit_bar: i,
            exit_time,
            exit_price: price,
            qty: p.qty,
            margin: p.margin,
            pnl,
            fees,
            funding: p.funding,
            risk: p.risk,
            reason,
            mae: worst / p.entry,
            mfe: best / p.entry,
            equity_before: p.equity_before,
        });
    }

    // ------------------------------------------------------------ a bar's close

    /// After the script read bar `i`: the orders it gave, for the next open or this close.
    pub fn close_bar(&mut self, i: usize, bars: &Bars, sig: Signals) {
        let (t, h, l, c) = (bars.time[i], bars.high[i], bars.low[i], bars.close[i]);
        // The average true range through this bar, as Wilder's: what an order given now sizes its stop by.
        let tr = match i.checked_sub(1).and_then(|j| bars.close.get(j)) {
            Some(&prev) if fin(prev) => (h - l).max((h - prev).abs()).max((l - prev).abs()),
            _ => h - l,
        };
        if fin(tr) {
            let n = self.s.atr_length as f64;
            if self.tr_n < self.s.atr_length {
                self.tr_sum += tr;
                self.tr_n += 1;
                if self.tr_n == self.s.atr_length {
                    self.atr = self.tr_sum / n;
                }
            } else {
                self.atr = (self.atr * (n - 1.0) + tr) / n;
            }
        }
        if let Some(v) = sig.stop {
            self.script_stop = v;
        }
        if let Some(v) = sig.target {
            self.script_target = v;
        }
        if let Some(p) = &mut self.pos {
            p.held += 1;
        }
        let s = self.s;
        let before = fin(s.from) && t < s.from;
        let after = fin(s.to) && t >= s.to;
        let open_ok = !before && !after && self.report.trades.len() < MAX_TRADES;
        let want_long = sig.long && s.longs && open_ok;
        let want_short = sig.short && s.shorts && open_ok;
        // Both at once ask nothing of a flat account.
        let (want_long, want_short) = if want_long && want_short { (false, false) } else { (want_long, want_short) };
        let mut exit = None;
        let mut entry = None;
        let order = |long: bool, at: f64| Entry { long, limit: at, stop: self.script_stop, target: self.script_target, atr: self.atr };
        match self.pos {
            Some(p) => {
                let held_out = s.max_bars > 0 && p.held >= s.max_bars;
                if (p.long && want_short) || (!p.long && want_long) {
                    exit = Some(Reason::Reverse);
                    entry = Some(order(!p.long, if p.long { sig.short_at } else { sig.long_at }));
                } else if after {
                    exit = Some(Reason::End);
                } else if sig.exit {
                    exit = Some(Reason::Signal);
                } else if held_out {
                    exit = Some(Reason::Time);
                }
            }
            None => {
                if want_long {
                    entry = Some(order(true, sig.long_at));
                } else if want_short {
                    entry = Some(order(false, sig.short_at));
                }
            }
        }
        if s.on_close {
            if let Some(reason) = exit {
                let slip = s.slippage;
                let price = match &self.pos {
                    Some(p) if p.long => c * (1.0 - slip),
                    Some(_) => c * (1.0 + slip),
                    None => c,
                };
                self.close_at(i, t, price, true, reason);
            }
            match entry {
                // At the market: at this close. A limit rests until the next bar.
                Some(e) if !fin(e.limit) => {
                    let price = c * (1.0 + if e.long { s.slippage } else { -s.slippage });
                    self.open_position(i, t + self.interval, e, price, false);
                    if let Some(p) = &mut self.pos {
                        p.best = c;
                    }
                }
                Some(e) => self.entry = Some(e),
                None => {}
            }
            self.mark(i);
        } else {
            self.exit = exit;
            self.entry = entry;
        }
        let eq = self.equity(c);
        self.report.equity.push(eq);
    }

    /// The report, a position still open written as a trade at the last close.
    pub fn finish(mut self, bars: &Bars) -> Report {
        match (&self.entry, &self.exit) {
            (Some(e), _) => {
                self.report.next = if e.long { 1 } else { -1 };
                self.report.next_limit = e.limit;
            }
            (None, Some(_)) => self.report.next = 2,
            _ => self.report.next_limit = NA,
        }
        if let (Some(p), Some(last)) = (self.pos.take(), bars.len().checked_sub(1)) {
            let c = bars.close[last];
            let pnl = self.cash + if p.long { 1.0 } else { -1.0 } * p.qty * (c - p.entry) - p.equity_before;
            self.record(p, last, bars.time[last], c, p.fees, pnl, Reason::Open);
        }
        self.report
    }
}
