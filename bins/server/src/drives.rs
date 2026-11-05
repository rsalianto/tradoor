//! Three drives: three pushes to one rising line, each a little past the last,
//! a real pullback between them — and then Stochastic RSI turning down out of
//! overbought, with RSI already cooled. The bullish pattern is the same thing upside down: three lower
//! lows along a falling line, and the turn up out of oversold.
//!
//! Only closed bars are read, and a pivot is only taken once the bars after it
//! that make it one have closed, so what is found does not change with the
//! forming bar. The bullish search runs on the bars mirrored — highs and lows
//! swapped and every price negated — which turns RSI and Stochastic RSI into
//! a hundred less themselves, so one search serves both sides.

use tradoor_core::indicators::{atr, pivot_highs, rsi, stoch_rsi};
use tradoor_core::Candle;

/// Bars either side of a high that make it a drive: five before, and two after
/// so the newest drive is known eight hours on, not a day.
const PIVOT_LEFT: usize = 5;
const PIVOT_RIGHT: usize = 2;
/// Bars at least between two drives: a day of four-hour bars.
const MIN_GAP: usize = 6;
/// Bars at most from the first drive to the third: forty days.
const MAX_SPAN: usize = 240;
/// How deep the pullback between two drives goes, at least, in ATRs —
const PULLBACK_ATR: f64 = 2.0;
/// — and as a part of the push that made the drive before it.
const RETRACE: f64 = 0.382;

/// What sets one length of bar's search apart from another's. The four-hour
/// rules are the ones the record was measured on. A day's or a week's bars
/// are fewer and their moves longer: a drive's push often reaches the line a
/// bar before its top, and a pullback after a long run need not give back
/// quite as much of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    /// How deep a pullback between two drives goes, at least, as a part of the push before it.
    pub retrace: f64,
    /// Whether the bars beside a drive may reach the line with it.
    pub beside: bool,
}

impl Rules {
    pub const FOUR_HOURS: Rules = Rules { retrace: RETRACE, beside: false };
    pub const LONGER: Rules = Rules { retrace: 0.3, beside: true };
}
/// How far each drive clears the last, at least, in ATRs: less is a double top.
const CLEAR_ATR: f64 = 0.5;
/// How far a bar between the drives may poke through their line, in ATRs.
const LINE_ATR: f64 = 0.5;
/// How far price may be from the line through two drives for a setup's third
/// drive, in ATRs; the score measures a pattern's fit against it too.
const ALIGN_ATR: f64 = 1.0;
/// How far a pattern's third drive may sit from that line, in ATRs: further,
/// and it won less than half again as often in the replays.
const FIT_ATR: f64 = 0.6;
/// RSI at the third drive past which the push is too strong to turn on: the
/// replays lost more often than they won past it.
const DRIVE_RSI: f64 = 75.0;
/// RSI at the bar a scan first sees the turn on, past which price has not
/// cooled enough to enter: those turns won some four times in ten, those
/// under it seven.
const ENTRY_RSI: f64 = 65.0;
/// How far a close after the third drive may go through the line before the
/// pattern is broken, in ATRs.
const BREAK_ATR: f64 = 1.0;
/// Bars a third drive waits for the turn: two days.
const WAIT_BARS: usize = 12;
/// Bars a turn stays news: two days.
const KEEP_BARS: usize = 12;
/// Where Stochastic RSI has to have been, in the bars before it turns.
const OVERBOUGHT: f64 = 80.0;
/// Drives looked back over for the first two.
const LOOK_BACK: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Three higher highs, then the turn down.
    Bear,
    /// Three lower lows, then the turn up.
    Bull,
}

impl Side {
    pub fn id(self) -> &'static str {
        match self {
            Side::Bear => "bear",
            Side::Bull => "bull",
        }
    }
}

/// A bar of the pattern: a drive's extreme, or the bar the turn came on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub index: usize,
    pub at: i64,
    pub price: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub side: Side,
    pub drives: [Point; 3],
    /// The bar Stochastic RSI turned on, once it has; its close.
    pub turn: Option<Point>,
    /// RSI weaker at the third drive than at the second.
    pub divergence: bool,
    /// 0 to 100: how well the third drive sits on the line, the divergence,
    /// how evenly the drives are spaced, volume fading, and the turn.
    pub score: u32,
    /// The third drive's high, or its low for the bullish pattern: what the
    /// stop is measured from.
    pub extreme: f64,
    /// Where the third drive's leg began: the extreme of the pullback before it.
    pub pullback: f64,
    /// The ATR at the third drive.
    pub atr: f64,
    /// Whether the push into the third drive bought less of the volume than
    /// the push into the second — price further, buyers weaker — where the
    /// exchange reports who took each trade; `None` where it does not.
    pub delta_divergence: Option<bool>,
    /// Whether more was sold than bought on the third drive's own bar.
    pub delta_against: Option<bool>,
    /// The push into each drive, where the exchange reports who took each trade.
    pub pushes: Option<[Push; 3]>,
}

/// What takers did on the push into a drive, from the bar after the low of
/// the pullback before it through the drive's own: bought less sold, in
/// dollars, and that as a part of all that traded. Buyers are positive on
/// either side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Push {
    pub usd: f64,
    pub share: f64,
}

/// Two drives and the line through them, waiting for a third: what price
/// reaching the line between scans would make a pattern of. Checked against
/// live prices (`touched`) rather than closed bars, so the third drive is
/// known as it happens instead of eight hours on.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub side: Side,
    pub drives: [Point; 2],
    /// The ATR at the newest bar read: how near the line counts as at it.
    pub atr: f64,
    /// Where a third drive starts: past the second by half an ATR.
    pub clear: f64,
    /// The last moment a third drive keeps the three within forty days.
    pub until: i64,
    /// The extreme of the pullback after the second drive: where a third
    /// drive's leg starts.
    pub pullback: f64,
    /// The push into each of the two drives, where the exchange reports who
    /// took each trade.
    pub pushes: Option<[Push; 2]>,
}

impl Setup {
    /// The line's price at a moment.
    pub fn line(&self, at: i64) -> f64 {
        let [a, b] = self.drives;
        a.price + (b.price - a.price) * (at - a.at) as f64 / (b.at - a.at) as f64
    }

    /// Price at the line: past the second drive, within an ATR of the line.
    pub fn touched(&self, price: f64, at: i64) -> bool {
        let past = match self.side {
            Side::Bear => price >= self.clear,
            Side::Bull => price <= self.clear,
        };
        past && at <= self.until && (price - self.line(at)).abs() <= ALIGN_ATR * self.atr
    }
}

/// Taker buy volume per bar, beside the bars, where the exchange reports it —
/// Binance does, Bybit does not — for the delta at the drives.
pub type Taker<'a> = Option<&'a [f64]>;

/// The newest two drives on `side` waiting for a third at their line, if any.
pub fn setup(candles: &[Candle], taker: Taker, side: Side) -> Option<Setup> {
    Prepared::new(candles, taker, side)?.setup()
}

/// The push into a third drive as it happens: from the bar after the low of
/// the pullback since the second drive (the bar opening at `second`) through
/// the newest bar, forming or not.
pub fn push_since(candles: &[Candle], taker: &[f64], side: Side, second: i64) -> Option<Push> {
    let p = Prepared::new(candles, Some(taker), side)?;
    let b = candles.iter().position(|k| k.open_time == second)?;
    let last = candles.len() - 1;
    p.push(p.push_start(b + 1, last), last)
}

/// The pattern on `side` whose third drive is the newest, if there is one now.
#[cfg(test)]
pub fn find(candles: &[Candle], taker: Taker, side: Side) -> Option<Found> {
    find_with(candles, taker, side, Rules::FOUR_HOURS)
}

/// The same, under another length's rules.
pub fn find_with(candles: &[Candle], taker: Taker, side: Side, rules: Rules) -> Option<Found> {
    let mut prepared = Prepared::new(candles, taker, side)?;
    prepared.rules = rules;
    prepared.at(candles.len() - 1)
}

/// Every pattern the bars held, each as a scan would have reported it the
/// first time it had turned — the bar that scan read up to, and what it saw.
/// Nothing after that bar is looked at to find it.
pub fn history(candles: &[Candle], taker: Taker, side: Side) -> Vec<(usize, Found)> {
    let Some(p) = Prepared::new(candles, taker, side) else { return Vec::new() };
    let mut told = std::collections::HashSet::new();
    // A scan at every bar, as the live ones run, each pattern taken the first time one sees it turned.
    (0..candles.len()).filter_map(|at| p.at(at).filter(|f| f.turn.is_some() && told.insert(f.drives[2].index)).map(|f| (at, f))).collect()
}

/// Why three drives are no longer listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// A close past the third drive.
    Outdone,
    /// A close well through the line.
    Broken,
    /// Price reached the first take-profit before Stochastic RSI turned.
    Played,
    /// No turn within two days of the third drive.
    Waited,
    /// Two days since the turn.
    Lapsed,
    /// Turned with RSI still over 65 (under 35 for the bullish side).
    Hot,
}

impl Ended {
    pub fn id(self) -> &'static str {
        match self {
            Ended::Outdone => "outdone",
            Ended::Broken => "broken",
            Ended::Played => "played",
            Ended::Waited => "waited",
            Ended::Lapsed => "lapsed",
            Ended::Hot => "hot",
        }
    }
}

/// Why the pattern whose third drive is the bar opening at `third_at` is no
/// longer one as of the newest bar; None if it still stands, or is not there.
pub fn ended(candles: &[Candle], side: Side, third_at: i64, rules: Rules) -> Option<Ended> {
    let mut p = Prepared::new(candles, None, side)?;
    p.rules = rules;
    let zi = p.pivots.iter().position(|pivot| candles[pivot.index].open_time == third_at)?;
    p.judge(p.shape(zi).as_ref()?, candles.len() - 1).err()
}

/// Every pattern the bars held, each as the scan that first saw it did —
/// turned or still waiting for the turn — and the bar that scan read up to.
pub fn sightings(candles: &[Candle], taker: Taker, side: Side) -> Vec<(usize, Found)> {
    let Some(p) = Prepared::new(candles, taker, side) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    (0..candles.len()).filter_map(|at| p.at(at).filter(|f| seen.insert(f.drives[2].index)).map(|f| (at, f))).collect()
}

/// The trade a signal is recorded as: take-profits a drop of 3, 6 and 9% from
/// the entry (a rise, for the bullish side), and a stop 3.5% past the third
/// drive's extreme. Half the position is closed at the first, and the stop
/// then moved to the entry; a quarter at each of the others.
pub const TAKE_PROFITS: [f64; 3] = [0.03, 0.06, 0.09];
pub const TAKE_SHARES: [f64; 3] = [0.5, 0.25, 0.25];
pub const STOP_PAST: f64 = 0.035;
/// Bars a trade is given: four days. What is still open then is closed at that close.
pub const OUTCOME_BARS: usize = 24;

/// A trade's prices: the three take-profits and the stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub tp: [f64; 3],
    pub sl: f64,
}

/// The plan the live record keeps, the list shows and Telegram names: the one
/// the replays bore out best (see `Plan`).
pub const LIVE: Plan = Plan::FibAtr;

/// A trade's prices under the live plan.
pub fn levels(side: Side, entry: f64, extreme: f64, pullback: f64, atr: f64) -> Levels {
    plan_levels(LIVE, side, entry, extreme, pullback, atr)
}

/// Ways to set a trade's take-profits and stop, replayed side by side to see
/// which the record bears out; `LIVE` is the one kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// 3, 6 and 9% from the entry; the stop 3.5% past the third drive.
    Fixed,
    /// 38.2, 61.8 and 100% back down the third drive's leg; the stop at its 1.272 extension.
    Fib,
    /// The same take-profits; the stop an ATR and a half past the third drive.
    FibAtr,
    /// 3, 6 and 9% from the entry; the stop 3.5% from the entry.
    FromEntry,
}

impl Plan {
    pub const ALL: [Plan; 4] = [Plan::Fixed, Plan::Fib, Plan::FibAtr, Plan::FromEntry];

    pub fn id(self) -> &'static str {
        match self {
            Plan::Fixed => "fixed",
            Plan::Fib => "fib",
            Plan::FibAtr => "fib-atr",
            Plan::FromEntry => "from-entry",
        }
    }
}

/// Fib retracements of the third drive's leg the take-profits sit at.
const FIB_TAKES: [f64; 3] = [0.382, 0.618, 1.0];
/// The extension of the leg past which the third drive has failed.
const FIB_STOP: f64 = 0.272;
/// ATRs past the third drive the live plan's stop sits: an ATR stopped out
/// trades that went on to their take-profits.
const STOP_ATR: f64 = 1.5;

/// A trade's prices under `plan`: from the entry, the third drive's extreme,
/// where its leg began, and the ATR there.
pub fn plan_levels(plan: Plan, side: Side, entry: f64, extreme: f64, pullback: f64, atr: f64) -> Levels {
    let dir = if side == Side::Bear { -1.0 } else { 1.0 };
    let leg = (extreme - pullback).abs();
    let fixed = TAKE_PROFITS.map(|p| entry * (1.0 + dir * p));
    let fib = FIB_TAKES.map(|r| extreme + dir * r * leg);
    match plan {
        Plan::Fixed => Levels { tp: fixed, sl: extreme * (1.0 - dir * STOP_PAST) },
        Plan::Fib => Levels { tp: fib, sl: extreme - dir * FIB_STOP * leg },
        Plan::FibAtr => Levels { tp: fib, sl: extreme - dir * STOP_ATR * atr },
        Plan::FromEntry => Levels { tp: fixed, sl: entry * (1.0 - dir * STOP_PAST) },
    }
}

/// How a trade went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outcome {
    pub result: Hit,
    /// Bars until it closed, or the bars looked at.
    pub bars: usize,
    /// The result over the whole position, in percent, positive the pattern's
    /// way: each part at the price it was closed at.
    pub ret: f64,
    /// The furthest price went the pattern's way, and against, in percent.
    pub best: f64,
    pub worst: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// All three take-profits.
    Tp3,
    /// Two, the rest out at the entry or at the end of the four days.
    Tp2,
    /// One, likewise.
    Tp1,
    /// The stop, before the first take-profit.
    Stop,
    /// No take-profit and no stop within the four days.
    Expired,
    /// Still running.
    Open,
}

impl Hit {
    pub fn id(self) -> &'static str {
        match self {
            Hit::Tp3 => "tp3",
            Hit::Tp2 => "tp2",
            Hit::Tp1 => "tp1",
            Hit::Stop => "stop",
            Hit::Expired => "expired",
            Hit::Open => "open",
        }
    }
}

/// A pattern that turned, entered at `entry`; `after` is the bars that followed.
pub fn outcome(side: Side, entry: f64, lv: Levels, after: &[Candle]) -> Outcome {
    play(side, entry, lv, after, false)
}

/// Price at a setup's line, entered there: the touch is the third drive's
/// extreme so far, so its levels are measured from it. `after` starts with
/// the bar of the touch, whose low or high may have come before it, so no
/// take-profit is taken in that bar.
pub fn touch_outcome(side: Side, entry: f64, lv: Levels, after: &[Candle]) -> Outcome {
    play(side, entry, lv, after, true)
}

/// The trade bar by bar. A bar that reaches the stop and a take-profit both
/// counts as the stop: which came first is not known.
pub fn play(side: Side, entry: f64, lv: Levels, after: &[Candle], from_touch: bool) -> Outcome {
    let bear = side == Side::Bear;
    let pct = |p: f64| if bear { (entry - p) / entry * 100.0 } else { (p - entry) / entry * 100.0 };
    let mut out = Outcome { result: Hit::Open, bars: 0, ret: 0.0, best: 0.0, worst: 0.0 };
    let (mut taken, mut banked, mut stop) = (0usize, 0.0, lv.sl);
    // A take-profit price is already past at the entry is closed there, for nothing, and the stop moves up.
    while taken < 3 && ((bear && entry <= lv.tp[taken]) || (!bear && entry >= lv.tp[taken])) {
        taken += 1;
        stop = entry;
    }
    let passed = taken;
    // The furthest take-profit price reached after the entry — one already
    // past at the entry was not reached — else the stop, or neither when the
    // stop had been moved to the entry.
    let closed = |taken: usize| match taken {
        t if t > passed => [Hit::Tp1, Hit::Tp2, Hit::Tp3][t - 1],
        _ if passed > 0 => Hit::Expired,
        _ => Hit::Stop,
    };
    for (i, k) in after.iter().take(OUTCOME_BARS).enumerate() {
        let (with, against) = if bear { (k.low, k.high) } else { (k.high, k.low) };
        out.bars = i + 1;
        out.worst = out.worst.min(pct(against));
        if (bear && against >= stop) || (!bear && against <= stop) {
            out.ret = banked + TAKE_SHARES[taken..].iter().sum::<f64>() * pct(stop);
            out.result = closed(taken);
            return out;
        }
        if from_touch && i == 0 {
            continue;
        }
        out.best = out.best.max(pct(with));
        while taken < 3 && ((bear && with <= lv.tp[taken]) || (!bear && with >= lv.tp[taken])) {
            banked += TAKE_SHARES[taken] * pct(lv.tp[taken]);
            taken += 1;
            // Half off, and the rest cannot lose.
            stop = entry;
        }
        if taken == 3 {
            (out.result, out.ret) = (closed(taken), banked);
            return out;
        }
    }
    if out.bars >= OUTCOME_BARS {
        out.ret = banked + TAKE_SHARES[taken..].iter().sum::<f64>() * pct(after[OUTCOME_BARS - 1].close);
        out.result = if taken > passed { closed(taken) } else { Hit::Expired };
    }
    out
}

/// How far back toward the third drive a limit entry waits for price: half
/// the way from the furthest it has gone since.
pub const BOUNCE: f64 = 0.5;
/// Bars a limit entry waits to be filled: two days.
pub const LIMIT_BARS: usize = 12;

/// What became of a limit entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    /// Filled at `price`. The trade's bars start at `bar`; filled within that
    /// bar, it takes no take-profit there (`within`), not knowing the order.
    At { bar: usize, price: f64, within: bool },
    /// Not reached in its two days, or the first take-profit came first.
    Missed,
    /// Neither yet.
    Waiting,
}

/// A pattern's limit entry, set by the scan that first saw it, reading up to
/// bar `seen`: half way back from the furthest price since the third drive
/// (bar `third`, its extreme `extreme`) toward it — and what became of it. It
/// fills at that close if price is already past it; else on the first of the
/// next two days' bars to reach it, at its open if it opened past it. It is
/// missed if price reaches the first take-profit (`tp1`) first. None if the
/// scan read no bar after the third drive.
pub fn limit_entry(candles: &[Candle], side: Side, third: usize, extreme: f64, tp1: f64, seen: usize) -> Option<(f64, Fill)> {
    let bear = side == Side::Bear;
    let since = candles.get(third + 1..=seen).filter(|s| !s.is_empty())?;
    let far = if bear { since.iter().map(|k| k.low).fold(f64::INFINITY, f64::min) } else { since.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max) };
    let limit = far + BOUNCE * (extreme - far);
    // Back to a price, the pattern's wrong way; and on to the first take-profit, its right way.
    let back = |p: f64| if bear { p >= limit } else { p <= limit };
    let on = |p: f64| if bear { p <= tp1 } else { p >= tp1 };
    if back(candles[seen].close) {
        return Some((limit, Fill::At { bar: seen + 1, price: candles[seen].close, within: false }));
    }
    for (bar, k) in candles.iter().enumerate().skip(seen + 1).take(LIMIT_BARS) {
        let (against, with) = if bear { (k.high, k.low) } else { (k.low, k.high) };
        if on(k.open) {
            return Some((limit, Fill::Missed));
        }
        if back(k.open) {
            return Some((limit, Fill::At { bar, price: k.open, within: true }));
        }
        if back(against) {
            return Some((limit, Fill::At { bar, price: limit, within: true }));
        }
        if on(with) {
            return Some((limit, Fill::Missed));
        }
    }
    Some((limit, if candles.len() > seen + LIMIT_BARS { Fill::Missed } else { Fill::Waiting }))
}

/// Highs and lows swapped and every price negated: lows become highs.
fn mirror(c: &Candle) -> Candle {
    Candle { open: -c.open, high: -c.low, low: -c.high, close: -c.close, ..*c }
}

/// The geometry of three drives ending at one pivot: which earlier pivots are
/// the first two, how well the third sits on their line, and its pullback.
#[derive(Clone, Copy)]
struct Shape {
    a: usize,
    b: usize,
    z: usize,
    fit: f64,
    /// The extreme of the pullback before the third drive.
    pullback: f64,
    /// The ATR at the third drive.
    atr: f64,
}

/// Two drives that could start a pattern, and from which bar to which their
/// line is a setup: once the pullback after the second is made, until a bar
/// pokes through the line.
#[derive(Clone, Copy)]
struct Pair {
    a: usize,
    b: usize,
    /// The first bar the pullback after the second drive is deep enough by.
    ready: usize,
    /// The first bar through the line, or past the bars there are.
    broken: usize,
}

/// What became of price at a setup's line within four days.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Became {
    /// A pattern, whose Stochastic RSI turned.
    Turned,
    /// A pattern, that never turned.
    Pattern,
    None,
}

impl Became {
    pub fn id(self) -> &'static str {
        match self {
            Became::Turned => "turned",
            Became::Pattern => "pattern",
            Became::None => "none",
        }
    }
}

/// Price reaching a setup's line in the history, and how it went.
pub struct Touch {
    pub setup: Setup,
    /// The bar it got there on.
    pub at: usize,
    pub entry: f64,
    /// Where the third drive's leg began: the extreme of the pullback after the second.
    pub pullback: f64,
    pub became: Became,
    pub outcome: Outcome,
}

/// Every time price reached a setup's line in the bars, as the live check would
/// have caught it: the setup the scan before had kept, and the bar after it
/// getting into the line's reach — at the line's near edge, or at the open if
/// it opened past that.
pub fn touches(candles: &[Candle], side: Side) -> Vec<Touch> {
    let Some(p) = Prepared::new(candles, None, side) else { return Vec::new() };
    let sign = if side == Side::Bull { -1.0 } else { 1.0 };
    let mut told = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in 1..candles.len() {
        // A scan with a pattern on this side keeps no setup.
        if p.at(t - 1).is_some() {
            continue;
        }
        let Some((setup, pair)) = p.setup_at(t - 1) else { continue };
        let k = &p.c[t];
        let (a, b) = (p.pivots[pair.a], p.pivots[pair.b]);
        let line = a.value + (b.value - a.value) * (t as f64 - a.index as f64) / (b.index - a.index) as f64;
        let near = (sign * setup.clear).max(line - ALIGN_ATR * setup.atr);
        let far = line + ALIGN_ATR * setup.atr;
        if near > far || k.high < near || k.open > far || k.open_time > setup.until || told.contains(&(pair.a, pair.b)) {
            continue;
        }
        told.insert((pair.a, pair.b));
        let entry = sign * k.open.max(near);
        let pullback = setup.pullback;
        // What the scans of the next four days made of it: these two drives with a third, turned or not.
        let seen: Vec<Found> = (t..candles.len().min(t + OUTCOME_BARS)).filter_map(|at| p.at(at)).filter(|f| f.drives[0].index == a.index && f.drives[1].index == b.index).collect();
        let became = if seen.iter().any(|f| f.turn.is_some()) {
            Became::Turned
        } else if seen.is_empty() {
            Became::None
        } else {
            Became::Pattern
        };
        let outcome = touch_outcome(side, entry, levels(side, entry, entry, pullback, setup.atr), &candles[t..]);
        out.push(Touch { setup, at: t, entry, pullback, became, outcome });
    }
    out
}

/// The bars, as they are or mirrored, with what every search reads of them.
/// Each series is causal — its value at a bar depends on no later bar — so the
/// search can be run as of any bar without looking past it.
struct Prepared {
    side: Side,
    c: Vec<Candle>,
    atr: Vec<Option<f64>>,
    rsi: Vec<Option<f64>>,
    k: Vec<Option<f64>>,
    d: Vec<Option<f64>>,
    pivots: Vec<tradoor_core::indicators::Pivot>,
    /// Each pivot's shape, worked out the first time it is asked for.
    shapes: Vec<std::cell::OnceCell<Option<Shape>>>,
    rules: Rules,
    /// Each pivot's pairs as the second drive, its first drives newest first.
    pairs: Vec<std::cell::OnceCell<Vec<Pair>>>,
    /// What takers bought less what they sold, per bar, the pattern's way: for
    /// the bullish side, mirrored like the prices, sold less bought.
    delta: Option<Vec<f64>>,
}

impl Prepared {
    fn new(candles: &[Candle], taker: Taker, side: Side) -> Option<Self> {
        if candles.len() < 60 {
            return None;
        }
        let sign = if side == Side::Bull { -1.0 } else { 1.0 };
        let delta = taker.filter(|t| t.len() == candles.len()).map(|t| t.iter().zip(candles).map(|(bought, k)| sign * (2.0 * bought - k.volume)).collect());
        let c: Vec<Candle> = match side {
            Side::Bear => candles.to_vec(),
            Side::Bull => candles.iter().map(mirror).collect(),
        };
        let high: Vec<Option<f64>> = c.iter().map(|k| Some(k.high)).collect();
        let close: Vec<Option<f64>> = c.iter().map(|k| Some(k.close)).collect();
        let stoch = stoch_rsi(&close, 14, 14, 3, 3);
        let pivots = pivot_highs(&high, PIVOT_LEFT, PIVOT_RIGHT);
        let shapes = (0..pivots.len()).map(|_| std::cell::OnceCell::new()).collect();
        let pairs = (0..pivots.len()).map(|_| std::cell::OnceCell::new()).collect();
        Some(Self { side, atr: atr(&c, 14), rsi: rsi(&close, 14), k: stoch.k, d: stoch.d, pivots, shapes, pairs, delta, c, rules: Rules::FOUR_HOURS })
    }

    fn atr_at(&self, i: usize) -> Option<f64> {
        self.atr[i].filter(|a| *a > 0.0)
    }

    /// The bar a push into the drive at `to` starts on: the one after the low
    /// of the bars from `from` up to it, or the drive's own if there are none.
    fn push_start(&self, from: usize, to: usize) -> usize {
        (from..to).min_by(|&i, &j| self.c[i].low.total_cmp(&self.c[j].low)).map_or(to, |low| low + 1)
    }

    /// The push over bars `from` to `to`, both in; none without taker volume.
    fn push(&self, from: usize, to: usize) -> Option<Push> {
        let delta = self.delta.as_ref()?;
        let sign = if self.side == Side::Bull { -1.0 } else { 1.0 };
        let (mut net, mut usd, mut volume) = (0.0, 0.0, 0.0);
        for (k, d) in self.c[from..=to].iter().zip(&delta[from..=to]) {
            // Each bar's coins at its average price: what traded in dollars over what traded.
            let price = k.quote_volume.filter(|q| *q > 0.0 && k.volume > 0.0).map_or((k.high + k.low + k.close).abs() / 3.0, |q| q / k.volume);
            (net, usd, volume) = (net + d, usd + d * price, volume + k.volume);
        }
        (volume > 0.0).then(|| Push { usd: sign * usd, share: sign * net / volume })
    }

    /// The pattern as a scan reading up to `last` would see it: the oldest one
    /// still standing among the drives recent enough to end one. A high since —
    /// a bounce in the pullback, a wick past the third drive, a quiet weekend's
    /// wobble — does not take its place while it stands.
    fn at(&self, last: usize) -> Option<Found> {
        let horizon = last.saturating_sub(WAIT_BARS + KEEP_BARS + PIVOT_RIGHT);
        (0..self.pivots.len())
            .filter(|&zi| self.pivots[zi].index >= horizon && self.pivots[zi].confirmed_at <= last)
            .find_map(|zi| self.judge(self.shapes[zi].get_or_init(|| self.shape(zi)).as_ref()?, last).ok())
    }

    /// The setup a scan reading up to `last` keeps: two drives as `shape`
    /// would take the first two, the pullback after the second made and the
    /// line not yet reached — the newest second drive first, then its newest first.
    fn setup_at(&self, last: usize) -> Option<(Setup, Pair)> {
        let c = &self.c;
        let atr_now = self.atr_at(last)?;
        let known = self.pivots.partition_point(|p| p.confirmed_at <= last);
        for bi in (0..known).rev().take(LOOK_BACK) {
            let b = self.pivots[bi];
            // A third drive in the bar now forming must be a day past the second.
            if last + 1 < b.index + MIN_GAP {
                continue;
            }
            if last + 1 - b.index > MAX_SPAN {
                break;
            }
            let pairs = self.pairs[bi].get_or_init(|| self.pairs_for(bi));
            let Some(pair) = pairs.iter().find(|p| last + 1 - self.pivots[p.a].index <= MAX_SPAN && p.ready <= last && p.broken > last) else { continue };
            let a = self.pivots[pair.a];
            let sign = if self.side == Side::Bull { -1.0 } else { 1.0 };
            let point = |i: usize, price: f64| Point { index: i, at: c[i].open_time, price: sign * price };
            let step = c[b.index].open_time - c[b.index - 1].open_time;
            let atr_b = self.atr_at(b.index)?;
            let pushes = self.delta.as_ref().and_then(|_| {
                let first = self.push(self.push_start(a.index.saturating_sub(b.index - a.index), a.index), a.index)?;
                Some([first, self.push(self.push_start(a.index + 1, b.index), b.index)?])
            });
            return Some((
                Setup {
                    side: self.side,
                    drives: [point(a.index, a.value), point(b.index, b.value)],
                    atr: atr_now,
                    clear: sign * (b.value + CLEAR_ATR * atr_b),
                    until: c[a.index].open_time + MAX_SPAN as i64 * step,
                    pullback: sign * c[b.index + 1..=last].iter().map(|k| k.low).fold(f64::INFINITY, f64::min),
                    pushes,
                },
                *pair,
            ));
        }
        None
    }

    fn setup(&self) -> Option<Setup> {
        self.setup_at(self.c.len() - 1).map(|(s, _)| s)
    }

    /// The first drives pivot `bi` can be the second of, newest first, each with
    /// the bars its line is a setup between.
    fn pairs_for(&self, bi: usize) -> Vec<Pair> {
        let c = &self.c;
        let b = self.pivots[bi];
        let Some(atr_b) = self.atr_at(b.index) else { return Vec::new() };
        let mut out = Vec::new();
        for ai in (0..bi).rev().take(LOOK_BACK) {
            let a = self.pivots[ai];
            if a.index + MIN_GAP > b.index || a.value >= b.value || b.value - a.value < CLEAR_ATR * atr_b {
                continue;
            }
            let before = &c[a.index.saturating_sub(b.index - a.index)..a.index];
            if before.iter().any(|k| k.high >= a.value) {
                continue;
            }
            let low_before = before.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let low_ab = c[a.index + 1..b.index].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            if a.value - low_ab < (PULLBACK_ATR * atr_b).max(self.rules.retrace * (a.value - low_before)) {
                continue;
            }
            let end = (a.index + MAX_SPAN).min(c.len());
            let line = |i: usize| a.value + (b.value - a.value) * (i as f64 - a.index as f64) / (b.index - a.index) as f64;
            // The line holds up to a bar poking through it: a third drive come, or a break.
            let broken = (a.index + 1..end).find(|&i| i != b.index && c[i].high > line(i) + LINE_ATR * self.atr_at(i).unwrap_or(atr_b)).unwrap_or(c.len());
            if broken < b.index {
                continue;
            }
            // And the pullback after the second drive is made by then.
            let mut low = f64::INFINITY;
            let ready = (b.index + 1..end)
                .find(|&t| {
                    low = low.min(c[t].low);
                    b.value - low >= (PULLBACK_ATR * self.atr_at(t).unwrap_or(atr_b)).max(self.rules.retrace * (b.value - low_ab))
                })
                .unwrap_or(c.len());
            out.push(Pair { a: ai, b: bi, ready, broken });
        }
        out
    }

    fn shape(&self, zi: usize) -> Option<Shape> {
        let c = &self.c;
        let third = self.pivots[zi];
        let z = third.index;
        let atr_z = self.atr_at(z)?;
        // A third drive pushed on too hard is a trend, not the end of one.
        if self.rsi[z].is_some_and(|r| r > DRIVE_RSI) {
            return None;
        }
        let mut best: Option<Shape> = None;
        let earlier = &self.pivots[zi.saturating_sub(LOOK_BACK)..zi];
        for (bi, b) in earlier.iter().enumerate() {
            if b.index + MIN_GAP > z || b.value >= third.value {
                continue;
            }
            let Some(atr_b) = self.atr_at(b.index) else { continue };
            for a in &earlier[..bi] {
                if a.index + MIN_GAP > b.index || z - a.index > MAX_SPAN || a.value >= b.value {
                    continue;
                }
                // Each push clears the last: three drives, not a double top.
                if b.value - a.value < CLEAR_ATR * atr_b || third.value - b.value < CLEAR_ATR * atr_z {
                    continue;
                }
                // The first drive is a fresh high, over everything as far back as the drives are apart.
                let before = &c[a.index.saturating_sub(b.index - a.index)..a.index];
                if before.iter().any(|k| k.high >= a.value) {
                    continue;
                }
                let line = |i: usize| a.value + (b.value - a.value) * (i as f64 - a.index as f64) / (b.index - a.index) as f64;
                // The third drive reaches the line, give or take.
                let off = (third.value - line(z)) / atr_z;
                if off.abs() > FIT_ATR {
                    continue;
                }
                // The line holds between the drives: they are pushes to it, not a
                // range under it. Under the longer lengths' rules a bar beside a
                // drive is part of that drive's push and may reach the line with it.
                let near = |i: usize| i == a.index + 1 || i + 1 == b.index || i == b.index + 1 || i + 1 == z;
                let beside = |i: usize| i == b.index || (self.rules.beside && near(i));
                if (a.index + 1..z).filter(|i| !beside(*i)).any(|i| c[i].high > line(i) + LINE_ATR * self.atr_at(i).unwrap_or(atr_z)) {
                    continue;
                }
                // And between them price pulled back for real.
                let low_between = |from: usize, to: usize| c[from + 1..to].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
                let low_before = before.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
                let (low_ab, low_bz) = (low_between(a.index, b.index), low_between(b.index, z));
                if a.value - low_ab < (PULLBACK_ATR * atr_b).max(self.rules.retrace * (a.value - low_before))
                    || b.value - low_bz < (PULLBACK_ATR * atr_z).max(self.rules.retrace * (b.value - low_ab))
                {
                    continue;
                }
                let fit = 1.0 - off.abs() / ALIGN_ATR;
                if best.as_ref().is_none_or(|s| fit > s.fit) {
                    best = Some(Shape { a: a.index, b: b.index, z, fit, pullback: low_bz, atr: atr_z });
                }
            }
        }
        best
    }

    /// A shape as of `last`: still standing, turned or still waiting, scored.
    fn judge(&self, shape: &Shape, last: usize) -> Result<Found, Ended> {
        let c = &self.c;
        let Shape { a, b, z, fit, pullback, atr: atr_z } = *shape;
        let top = c[z].high;
        // Outdone: a close past the third drive. A wick past it is a retest.
        if c[z + 1..=last].iter().any(|k| k.close > top) {
            return Err(Ended::Outdone);
        }
        let line = |i: usize| c[a].high + (c[b].high - c[a].high) * (i as f64 - a as f64) / (b - a) as f64;
        // Broken: a close well through the line since the third drive.
        if (z + 1..=last).any(|i| c[i].close > line(i) + BREAK_ATR * self.atr_at(i).unwrap_or(atr_z)) {
            return Err(Ended::Broken);
        }
        let turn = turn_down(&self.k[..=last], &self.d[..=last], z);
        // Played out before the turn: price down to the first take-profit,
        // 38.2% back down the third drive's leg, by the bar a scan first sees
        // the turn on — the move made with nothing to enter it on.
        let first_take = top - FIB_TAKES[0] * (top - pullback);
        let entered = turn.map_or(last, |t| t.max(z + PIVOT_RIGHT)).min(last);
        if c[z + 1..=entered].iter().any(|k| k.low <= first_take) {
            return Err(Ended::Played);
        }
        match turn {
            Some(t) if last - t > KEEP_BARS => return Err(Ended::Lapsed),
            None if last - z > WAIT_BARS => return Err(Ended::Waited),
            _ => {}
        }
        // Turned with RSI still high at the bar it would be entered on.
        if turn.is_some() && self.rsi[entered].is_some_and(|r| r > ENTRY_RSI) {
            return Err(Ended::Hot);
        }

        let (ra, rb, rz) = (self.rsi[a].unwrap_or(50.0), self.rsi[b].unwrap_or(50.0), self.rsi[z].unwrap_or(50.0));
        let divergence = rz < rb;
        let (gap1, gap2) = ((b - a) as f64, (z - b) as f64);
        let volume = |i: usize| c[i.saturating_sub(2)..=(i + 2).min(last)].iter().map(|k| k.volume).sum::<f64>();
        let mut score = 30.0 * fit + 15.0 * gap1.min(gap2) / gap1.max(gap2);
        if divergence {
            score += 15.0;
            if rb < ra {
                score += 10.0;
            }
        }
        if volume(z) < volume(b) {
            score += 15.0;
        }
        if turn.is_some() {
            score += 15.0;
        }

        // The delta at the drives: how much of the volume buyers took on the
        // push into each, from the low of the pullback before it — the first
        // from the low of as many bars before it as the first two are apart.
        let sign = if self.side == Side::Bull { -1.0 } else { 1.0 };
        let (second, third) = match &self.delta {
            Some(_) => (self.push(self.push_start(a + 1, b), b), self.push(self.push_start(b + 1, z), z)),
            None => (None, None),
        };
        // Buyers weaker into the third drive, or sellers for the bullish side.
        let delta_divergence = second.zip(third).map(|(second, third)| sign * third.share < sign * second.share);
        let delta_against = self.delta.as_ref().map(|delta| delta[z] < 0.0);
        let first = second.and_then(|_| self.push(self.push_start(a.saturating_sub(b - a), a), a));
        let pushes = first.zip(second).zip(third).map(|((first, second), third)| [first, second, third]);

        // Prices back the right way up for the bullish side.
        let point = |i: usize, price: f64| Point { index: i, at: c[i].open_time, price: sign * price };
        Ok(Found {
            side: self.side,
            drives: [point(a, c[a].high), point(b, c[b].high), point(z, top)],
            turn: turn.map(|t| point(t, c[t].close)),
            divergence,
            score: score.round().clamp(0.0, 100.0) as u32,
            extreme: sign * top,
            pullback: sign * pullback,
            atr: atr_z,
            delta_divergence,
            delta_against,
            pushes,
        })
    }
}

/// The first bar from `from` on where K crosses under D, having been
/// overbought in the three bars before.
fn turn_down(k: &[Option<f64>], d: &[Option<f64>], from: usize) -> Option<usize> {
    (from.max(3)..k.len()).find(|&t| {
        let crossed = matches!((k[t - 1], d[t - 1], k[t], d[t]), (Some(k0), Some(d0), Some(k1), Some(d1)) if k0 >= d0 && k1 < d1);
        crossed && k[t - 3..t].iter().flatten().any(|v| *v >= OVERBOUGHT)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const H4: i64 = 4 * 3_600_000;

    /// Bars along straight legs between `(index, price)` corners, with a small
    /// wobble so RSI moves; the highs half a point over the close, the lows under.
    fn legs(corners: &[(usize, f64)]) -> Vec<Candle> {
        let mut closes = Vec::new();
        for pair in corners.windows(2) {
            let ((i0, p0), (i1, p1)) = (pair[0], pair[1]);
            for i in i0..i1 {
                closes.push(p0 + (p1 - p0) * (i - i0) as f64 / (i1 - i0) as f64);
            }
        }
        closes.push(corners.last().unwrap().1);
        let peaks: Vec<usize> = corners.iter().map(|(i, _)| *i).collect();
        closes
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                // Corners and the bars beside them stay clean, so each peak is one bar.
                let close = if peaks.iter().any(|&q| i.abs_diff(q) <= 2) { p } else { p + if i % 2 == 0 { 0.9 } else { -0.9 } };
                Candle { open_time: i as i64 * H4, open: close, high: close + 0.5, low: close - 0.5, close, volume: 100.0, quote_volume: None, taker_buy: None }
            })
            .collect()
    }

    /// Three pushes to a line rising a sixth of a point a bar — 110, 115, 120 —
    /// with ten-point pullbacks, then the fall that follows.
    fn three_up(after: &[(usize, f64)]) -> Vec<Candle> {
        let mut corners = vec![(0, 95.0), (60, 110.0), (75, 100.0), (90, 115.0), (105, 105.0), (120, 120.0)];
        corners.extend_from_slice(after);
        legs(&corners)
    }

    #[test]
    fn three_pushes_to_one_line_are_found() {
        let bars = three_up(&[(126, 114.0)]);
        let found = find(&bars, None, Side::Bear).expect("three drives");
        assert_eq!(found.drives.map(|d| d.index), [60, 90, 120]);
        assert_eq!(found.drives.map(|d| d.price), [110.5, 115.5, 120.5]);
        assert_eq!(found.drives[2].at, 120 * H4);
        assert!(found.score >= 30, "on the line, evenly spaced: {}", found.score);
        if let Some(turn) = found.turn {
            assert!(turn.index >= 120, "the turn comes after the third drive");
        }
    }

    #[test]
    fn the_history_reports_a_pattern_as_a_scan_would_have_then() {
        let bars = three_up(&[(126, 114.0), (150, 104.0)]);
        let reported = history(&bars, None, Side::Bear);
        assert_eq!(reported.len(), 1);
        let (at, found) = &reported[0];
        assert_eq!(*at, 122, "the first scan after the third drive is known, the turn already in");
        assert_eq!(find(&bars[..=*at], None, Side::Bear).as_ref(), Some(found), "the same as that scan found, seeing nothing later");
        assert_eq!(found.extreme, 120.5, "the stop is measured from the third drive's high");
    }

    #[test]
    fn a_trade_takes_half_at_the_first_profit_and_then_cannot_lose() {
        let bar = |high: f64, low: f64, close: f64| Candle { open_time: 0, open: close, high, low, close, volume: 1.0, quote_volume: None, taker_buy: None };
        // Short at 100 under a third drive at 104: profits at 97, 94 and 91, the stop at 107.64.
        let lv = plan_levels(Plan::Fixed, Side::Bear, 100.0, 104.0, 104.0, 0.0);
        assert!((lv.tp[0] - 97.0).abs() < 1e-9 && (lv.tp[2] - 91.0).abs() < 1e-9 && (lv.sl - 107.64).abs() < 1e-9);
        let outcome = |after: &[Candle]| outcome(Side::Bear, 100.0, lv, after);
        let all = outcome(&[bar(101.0, 96.0, 97.0), bar(97.0, 90.0, 91.0)]);
        assert_eq!((all.result, all.bars), (Hit::Tp3, 2));
        assert!((all.ret - (0.5 * 3.0 + 0.25 * 6.0 + 0.25 * 9.0)).abs() < 1e-9, "half at 3%, a quarter each at 6 and 9: {}", all.ret);
        let back = outcome(&[bar(101.0, 96.5, 97.0), bar(100.5, 98.0, 100.0)]);
        assert_eq!(back.result, Hit::Tp1, "the first profit, then back to the entry");
        assert!((back.ret - 1.5).abs() < 1e-9, "half at 3%, half at nothing: {}", back.ret);
        let stopped = outcome(&[bar(108.0, 99.0, 106.0)]);
        assert_eq!(stopped.result, Hit::Stop);
        assert!((stopped.ret + 7.64).abs() < 1e-9, "the whole of it at the stop: {}", stopped.ret);
        assert_eq!(outcome(&[bar(108.0, 96.0, 100.0)]).result, Hit::Stop, "both in one bar: the stop, not knowing which came first");
        let flat = vec![bar(101.0, 99.0, 99.0); OUTCOME_BARS];
        let expired = outcome(&flat);
        assert_eq!(expired.result, Hit::Expired);
        assert!((expired.ret - 1.0).abs() < 1e-9, "closed after four days at that close");
        assert_eq!(outcome(&flat[..3]).result, Hit::Open, "too soon to say");
        let up_levels = plan_levels(Plan::Fixed, Side::Bull, 100.0, 96.0, 96.0, 0.0);
        let up = super::outcome(Side::Bull, 100.0, up_levels, &[bar(103.5, 99.0, 103.0), bar(106.5, 102.0, 106.0)]);
        assert_eq!(up.result, Hit::Open, "the bullish way up: two taken, still running");
        assert!(up_levels.sl < 96.0);
        // The live plan: take-profits back down the third drive's leg, the stop an ATR and a half past its high.
        let live = levels(Side::Bear, 108.0, 112.0, 100.0, 2.0);
        assert_eq!(live, plan_levels(Plan::FibAtr, Side::Bear, 108.0, 112.0, 100.0, 2.0));
    }

    #[test]
    fn the_bullish_one_is_the_same_upside_down() {
        let bars: Vec<Candle> = three_up(&[(126, 114.0)])
            .iter()
            .map(|c| Candle { open: 300.0 - c.open, high: 300.0 - c.low, low: 300.0 - c.high, close: 300.0 - c.close, ..*c })
            .collect();
        let found = find(&bars, None, Side::Bull).expect("three drives down");
        assert_eq!(found.side, Side::Bull);
        assert_eq!(found.drives.map(|d| d.price), [189.5, 184.5, 179.5], "the lows, in prices as they are");
        assert!(find(&bars, None, Side::Bear).is_none(), "and nothing bearish in it");
    }

    #[test]
    fn a_third_drive_off_the_line_is_not_one() {
        // The third push overshoots the line by five points: a breakout, not a drive.
        let corners = [(0, 95.0), (60, 110.0), (75, 100.0), (90, 115.0), (105, 105.0), (120, 125.0), (126, 118.0)];
        assert!(find(&legs(&corners), None, Side::Bear).is_none());
    }

    #[test]
    fn a_third_drive_well_short_of_the_line_is_not_one() {
        // Short of the line by three quarters of an ATR: under a cooled RSI, still not a drive on it.
        let corners = [(0, 95.0), (60, 110.0), (75, 100.0), (90, 115.0), (105, 105.0), (120, 118.0), (126, 113.0)];
        let bars = legs(&corners);
        assert!(Prepared::new(&bars, None, Side::Bear).unwrap().rsi[120].is_some_and(|r| r <= DRIVE_RSI));
        assert!(find(&bars, None, Side::Bear).is_none());
    }

    #[test]
    fn a_third_drive_pushed_too_hard_is_not_one() {
        // The same line, the pullback held a while and the third push made in eight bars instead of fifteen.
        let bars = legs(&[(0, 95.0), (60, 110.0), (75, 100.0), (90, 115.0), (105, 105.0), (112, 105.0), (120, 120.0), (126, 114.0)]);
        assert!(Prepared::new(&bars, None, Side::Bear).unwrap().rsi[120].is_some_and(|r| r > DRIVE_RSI), "RSI over 75 at the third drive");
        assert!(find(&bars, None, Side::Bear).is_none());
        assert_eq!(ended(&bars, Side::Bear, 120 * H4, Rules::FOUR_HOURS), None, "never a pattern, so none that ended");
    }

    #[test]
    fn a_turn_with_rsi_still_high_ends_it() {
        // A slow drift down from the third drive: Stochastic RSI turns, RSI stays over 65.
        let bars = three_up(&[(130, 117.0)]);
        assert!(find(&bars, None, Side::Bear).is_none());
        assert_eq!(ended(&bars, Side::Bear, 120 * H4, Rules::FOUR_HOURS), Some(Ended::Hot));
        // The fall that cools it is entered.
        assert!(find(&three_up(&[(126, 114.0)]), None, Side::Bear).and_then(|f| f.turn).is_some());
    }

    #[test]
    fn a_new_high_or_a_close_through_the_line_ends_it() {
        // Back up past the third drive: a fourth push, and the three are history.
        assert!(find(&three_up(&[(124, 116.0), (128, 121.5)]), None, Side::Bear).is_none());
        // Without the pullbacks there is a trend, not three drives.
        let shallow = [(0, 95.0), (60, 110.0), (75, 109.5), (90, 115.0), (105, 114.5), (120, 120.0), (126, 114.0)];
        assert!(find(&legs(&shallow), None, Side::Bear).is_none());
    }

    #[test]
    fn a_smaller_high_after_the_third_drive_does_not_hide_it() {
        // A bounce in the pullback makes a high of its own, under the third drive.
        let bars = three_up(&[(126, 114.0), (129, 116.8), (131, 115.0)]);
        assert!(Prepared::new(&bars, None, Side::Bear).unwrap().pivots.iter().any(|p| p.index == 129), "the bounce is a high of its own");
        let found = find(&bars, None, Side::Bear).expect("still three drives");
        assert_eq!(found.drives[2].index, 120);
    }

    #[test]
    fn a_wick_past_the_third_drive_leaves_it_standing() {
        let mut bars = three_up(&[(126, 114.0)]);
        bars[123].high = 122.0;
        let found = find(&bars, None, Side::Bear).expect("a wick is a retest");
        assert_eq!(found.drives[2].index, 120);
        // A close past it, well off the line: not a drive on it either.
        (bars[123].high, bars[123].close) = (125.0, 124.0);
        assert!(find(&bars, None, Side::Bear).is_none(), "a close past it ends it");
    }

    #[test]
    fn two_drives_wait_for_the_third_at_their_line() {
        // Drives at 110.5 and 115.5, the pullback to 105 made, price on its way back up.
        let bars = three_up(&[])[..115].to_vec();
        let setup = setup(&bars, None, Side::Bear).expect("two drives and a line");
        assert_eq!(setup.drives.map(|d| d.index), [60, 90]);
        let at = 120 * H4;
        assert!((setup.line(at) - 120.5).abs() < 1e-9, "the line reaches 120.5 where the third drive came");
        assert!(setup.touched(120.2, at), "at the line, past the second drive");
        assert!(!setup.touched(118.0, at), "short of it");
        assert!(!setup.touched(124.0, at), "well through it is a breakout, not a drive");
        assert!(!setup.touched(115.7, 91 * H4), "near the line, but not past the second drive by half an ATR");
        // Before the pullback after the second drive there is nothing to wait for.
        assert!(super::setup(&three_up(&[])[..95], None, Side::Bear).is_none());
        // Once the third drive is there and pulled back from, it is the second of the next two
        // (the screener keeps a pattern found ahead of a setup on the same side).
        assert_eq!(super::setup(&three_up(&[(126, 114.0)]), None, Side::Bear).map(|s| s.drives.map(|d| d.index)), Some([90, 120]));
    }

    #[test]
    fn the_history_catches_price_at_the_line_and_what_became_of_it() {
        let bars = three_up(&[(126, 114.0), (135, 103.0), (150, 100.0)]);
        let touches = touches(&bars, Side::Bear);
        let touch = touches.iter().find(|t| t.setup.drives.map(|d| d.index) == [60, 90]).expect("price reached the line of the first two drives");
        assert!((118..=120).contains(&touch.at), "on its way to the third drive, at {}", touch.at);
        let line = touch.setup.line(bars[touch.at].open_time);
        assert!((touch.entry - line).abs() <= touch.setup.atr + 1e-9 && touch.entry >= touch.setup.clear, "within an ATR of the line, past the second drive");
        assert_eq!(touch.became, Became::Turned, "and it became three drives that turned");
        assert_eq!(touch.outcome.result, Hit::Tp3, "under the live plan the stop is an ATR and a half over the touch: room for the third drive, then all the way down");
        let tight = Levels { sl: touch.entry + touch.setup.atr, ..levels(Side::Bear, touch.entry, touch.entry, touch.pullback, touch.setup.atr) };
        assert_eq!(play(Side::Bear, touch.entry, tight, &bars[touch.at..], true).result, Hit::Stop, "an ATR over it, the third drive went past the stop");
        let fixed = plan_levels(Plan::Fixed, Side::Bear, touch.entry, touch.entry, touch.pullback, touch.setup.atr);
        assert_eq!(play(Side::Bear, touch.entry, fixed, &bars[touch.at..], true).result, Hit::Tp3, "with 3.5% of room it rode all the way down");
    }

    #[test]
    fn each_plan_sets_its_own_levels() {
        // Short at 108 under a third drive at 112, its leg up from 100, the ATR 2.
        let at = |plan| plan_levels(plan, Side::Bear, 108.0, 112.0, 100.0, 2.0);
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(close(at(Plan::Fixed).tp, [104.76, 101.52, 98.28]) && (at(Plan::Fixed).sl - 115.92).abs() < 1e-9);
        assert!(close(at(Plan::Fib).tp, [107.416, 104.584, 100.0]), "38.2, 61.8 and 100% back down the leg: {:?}", at(Plan::Fib).tp);
        assert!((at(Plan::Fib).sl - 115.264).abs() < 1e-9, "the 1.272 extension of the leg");
        assert!((at(Plan::FibAtr).sl - 115.0).abs() < 1e-9, "an ATR and a half past the third drive");
        assert!((at(Plan::FromEntry).sl - 111.78).abs() < 1e-9, "3.5% from the entry");
        let up = plan_levels(Plan::Fib, Side::Bull, 92.0, 88.0, 100.0, 2.0);
        assert!(close(up.tp, [92.584, 95.416, 100.0]) && (up.sl - 84.736).abs() < 1e-9, "the bullish way up: {up:?}");
    }

    #[test]
    fn a_take_profit_already_past_at_the_entry_is_closed_there() {
        let bar = |high: f64, low: f64, close: f64| Candle { open_time: 0, open: close, high, low, close, volume: 1.0, quote_volume: None, taker_buy: None };
        // Entered at 107 under Fib take-profits at 107.4, 104.6 and 100: the first is already past.
        let lv = plan_levels(Plan::Fib, Side::Bear, 107.0, 112.0, 100.0, 2.0);
        let back = play(Side::Bear, 107.0, lv, &[bar(107.5, 106.0, 107.2)], false);
        assert_eq!(back.result, Hit::Expired, "half closed at the entry, the rest stopped there too: no take-profit reached, and no loss");
        assert!(back.ret.abs() < 1e-9);
        let mut after = vec![bar(106.9, 104.0, 104.5)];
        after.resize(OUTCOME_BARS, bar(105.0, 104.2, 104.5));
        assert_eq!(play(Side::Bear, 107.0, lv, &after, false).result, Hit::Tp2, "the second reached after the entry, and the third not in four days");
    }

    #[test]
    fn a_touch_is_its_own_third_drive_for_the_stop() {
        let bar = |high: f64, low: f64, close: f64| Candle { open_time: 0, open: close, high, low, close, volume: 1.0, quote_volume: None, taker_buy: None };
        // Price at the line at 120: the stop 3.5% above it, at 124.2.
        let lv = plan_levels(Plan::Fixed, Side::Bear, 120.0, 120.0, 120.0, 0.0);
        let touch_outcome = |after: &[Candle]| touch_outcome(Side::Bear, 120.0, lv, after);
        let broke = touch_outcome(&[bar(121.0, 115.0, 118.0), bar(124.5, 119.0, 124.0)]);
        assert_eq!((broke.result, broke.bars), (Hit::Stop, 2));
        // The touch bar's low may have come before the touch: no profit is taken in it.
        let early = touch_outcome(&[bar(121.0, 116.0, 118.0), bar(118.0, 116.2, 117.0)]);
        assert_eq!((early.result, early.bars), (Hit::Open, 2), "116 in the touch bar does not count; 116.2 is short of 116.4");
        assert_eq!(touch_outcome(&[bar(121.0, 116.0, 118.0), bar(118.0, 116.3, 117.0)]).result, Hit::Open);
        assert_eq!(touch_outcome(&[bar(121.0, 116.0, 118.0), bar(118.0, 116.0, 117.0)]).bars, 2);
    }

    #[test]
    fn the_delta_says_whether_buyers_weakened_into_the_third_drive() {
        let bars = three_up(&[(126, 114.0)]);
        assert_eq!(find(&bars, None, Side::Bear).unwrap().delta_divergence, None, "no taker volume, no delta");
        // Buyers take 80% of the volume on the push into the second drive, 60% into the third.
        let taker: Vec<f64> = bars.iter().enumerate().map(|(i, k)| k.volume * if (76..=90).contains(&i) { 0.8 } else { 0.6 }).collect();
        let found = find(&bars, Some(&taker), Side::Bear).unwrap();
        assert_eq!(found.delta_divergence, Some(true), "price further, buyers weaker");
        assert_eq!(found.delta_against, Some(false), "and on the third drive's bar they still bought more");
        let [first, second, third] = found.pushes.expect("the push into each drive");
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(first.share, 0.2) && near(second.share, 0.6) && near(third.share, 0.2), "bought less sold over all traded: {first:?} {second:?} {third:?}");
        assert!(second.usd > third.usd && third.usd > 0.0, "and in dollars");
        // As it happens: the push since the pullback after the second drive, two bars short of the third.
        let now = push_since(&bars[..=118], &taker[..=118], Side::Bear, 90 * H4).expect("a push under way");
        assert!(near(now.share, 0.2) && now.usd < third.usd, "{now:?}");
        let stronger: Vec<f64> = bars.iter().enumerate().map(|(i, k)| k.volume * if i == 120 { 0.3 } else if (106..=119).contains(&i) { 0.9 } else { 0.6 }).collect();
        let found = find(&bars, Some(&stronger), Side::Bear).unwrap();
        assert_eq!(found.delta_divergence, Some(false), "buyers stronger into the third");
        assert_eq!(found.delta_against, Some(true), "though its own bar was sold");
        // The bullish side reads sellers the same way: sold less bought.
        let mirrored: Vec<Candle> = bars.iter().map(|c| Candle { open: 300.0 - c.open, high: 300.0 - c.low, low: 300.0 - c.high, close: 300.0 - c.close, ..*c }).collect();
        let sold: Vec<f64> = taker.iter().zip(&bars).map(|(b, k)| k.volume - b).collect();
        let down = find(&mirrored, Some(&sold), Side::Bull).unwrap();
        assert_eq!(down.delta_divergence, Some(true));
        assert!(near(down.pushes.unwrap()[1].share, -0.6), "sellers took 80% into the second low: buyers' share as the exchange has it");
        // The two drives a setup waits on are measured the same way.
        let setup = setup(&bars[..115], Some(&taker[..115]), Side::Bear).expect("two drives and a line");
        assert_eq!(setup.pushes.map(|p| p.map(|p| p.share)), Some([first.share, second.share]));
    }

    #[test]
    fn a_fall_to_the_first_take_profit_before_the_turn_ends_it() {
        // Past the first take-profit, 114.4, by the bar the turn is first seen on: nothing left to enter.
        let bars = three_up(&[(121, 113.0), (130, 110.0)]);
        assert!(find(&bars, None, Side::Bear).is_none());
        assert_eq!(ended(&bars, Side::Bear, 120 * H4, Rules::FOUR_HOURS), Some(Ended::Played));
        // Once it has turned, the fall is the trade at work.
        assert_eq!(ended(&three_up(&[(126, 114.0)]), Side::Bear, 120 * H4, Rules::FOUR_HOURS), None);
        assert_eq!(ended(&three_up(&[(124, 116.0), (128, 121.5)]), Side::Bear, 120 * H4, Rules::FOUR_HOURS), Some(Ended::Outdone), "a close past the third drive");
    }

    #[test]
    fn a_limit_waits_for_half_the_drop_back() {
        let first_seen = |bars: &[Candle]| sightings(bars, None, Side::Bear).into_iter().find(|(_, f)| f.drives[2].index == 120).expect("three drives");
        let bars = three_up(&[(126, 116.0), (132, 119.5), (140, 108.0)]);
        let (seen, f) = first_seen(&bars);
        let tp1 = levels(Side::Bear, f.extreme, f.extreme, f.pullback, f.atr).tp[0];
        let (limit, fill) = limit_entry(&bars, Side::Bear, 120, f.extreme, tp1, seen).unwrap();
        let low = bars[121..=seen].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
        assert!((limit - (low + 0.5 * (f.extreme - low))).abs() < 1e-9, "half way back up from the low since the third drive");
        let Fill::At { bar, price, within } = fill else { panic!("filled: {fill:?}") };
        assert!(within && price >= limit, "within the bar, at the limit or better: {price} for {limit}");
        assert!(bars[bar].high >= limit && bars[seen + 1..bar].iter().all(|k| k.high < limit), "on the first bar back up to it");
        // Down to the first take-profit before coming back: missed.
        let bars = three_up(&[(126, 112.0), (132, 119.5)]);
        let (seen, f) = first_seen(&bars);
        assert_eq!(limit_entry(&bars, Side::Bear, 120, f.extreme, tp1, seen).map(|(_, fill)| fill), Some(Fill::Missed));
        // Not yet back, with bars of its two days still to come.
        assert_eq!(limit_entry(&bars[..seen + 3], Side::Bear, 120, f.extreme, 100.0, seen).map(|(_, fill)| fill), Some(Fill::Waiting));
    }

    #[test]
    fn the_first_drive_is_a_fresh_high() {
        // A higher high a week before the first drive: a range, not a run of drives.
        let corners = [(0, 95.0), (35, 112.0), (45, 100.0), (60, 110.0), (75, 100.0), (90, 115.0), (105, 105.0), (120, 120.0), (126, 114.0)];
        assert!(find(&legs(&corners), None, Side::Bear).is_none());
    }

    #[test]
    fn a_pattern_waits_two_days_for_its_turn() {
        // Three weeks after the third drive it is no longer news.
        assert!(find(&three_up(&[(126, 114.0), (246, 112.0)]), None, Side::Bear).is_none());
    }

    #[test]
    fn the_turn_is_a_cross_down_from_overbought() {
        let k = [Some(70.0), Some(85.0), Some(90.0), Some(88.0), Some(75.0), Some(60.0)];
        let d = [Some(60.0), Some(75.0), Some(84.0), Some(87.0), Some(82.0), Some(70.0)];
        assert_eq!(turn_down(&k, &d, 0), Some(4), "K under D on the fifth bar, having been over 80");
        let low = k.map(|v| v.map(|v| v - 40.0));
        let low_d = d.map(|v| v.map(|v| v - 40.0));
        assert_eq!(turn_down(&low, &low_d, 0), None, "the same cross down at 40 is not a turn from overbought");
        assert_eq!(turn_down(&k, &d, 5), None, "nor one before the third drive");
    }
}
