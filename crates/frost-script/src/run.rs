//! A program run over the bars, one bar at a time, top to bottom. A variable
//! starts each bar with no value unless it is a `state`; `x[n]` reads what it
//! ended an earlier bar on. The functions that remember past bars keep their
//! own memory, one for each place they are called, and do their sums in the
//! order the chart's built-in indicators do, so a script written as one of
//! them gives the same numbers. Lists, shapes and the table are the script's
//! own, and bounded: a list past its size lets its oldest value go, as do the
//! shapes past theirs. Every step is counted against a budget.
use std::collections::VecDeque;
use std::rc::Rc;

use crate::compile::{math, Ex, Fmt, Keep, ListOp, Program, Ret, Run, Series, ShapeOp, Target, Tx};
use crate::syntax::{At, Op};
use crate::{Bars, BoxArt, Cell, Error, Fill, LabelArt, LineArt, Mark, Output, Plot, Table, MAX_AHEAD, MAX_BOXES, MAX_LABELS, MAX_LINES, MAX_LIST, MAX_LOOKBACK, MAX_LOOP, MAX_STEPS, TABLE_COLUMNS, TABLE_ROWS};
use tradoor_core::format::fixed;
use tradoor_core::time::{civil, month_index, week_index, weekday, year_of, DAY};

const NA: f64 = f64::NAN;
/// A text a script makes is cut at this many characters.
const MAX_MADE_TEXT: usize = 1_000;

fn truth(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// A colour a script holds, as the four bytes it is; none, where it has no value.
fn colour(v: f64) -> u32 {
    if (0.0..=4_294_967_295.0).contains(&v) { v as u32 } else { 0 }
}

/// The simple average, as the chart's: a running sum, worked out afresh every `len` bars so it does not drift.
#[derive(Debug)]
struct Sma {
    len: usize,
    window: VecDeque<f64>,
    sum: f64,
    run: usize,
    since: usize,
}

impl Sma {
    fn new(len: usize) -> Self {
        Self { len, window: VecDeque::new(), sum: 0.0, run: 0, since: 0 }
    }

    fn step(&mut self, v: f64) -> f64 {
        if v.is_nan() {
            (self.sum, self.run, self.since) = (0.0, 0, 0);
            self.window.clear();
            return NA;
        }
        self.sum += v;
        self.run += 1;
        self.window.push_back(v);
        if self.run > self.len {
            if let Some(old) = self.window.pop_front() {
                self.sum -= old;
            }
            self.run = self.len;
        }
        if self.run < self.len {
            return NA;
        }
        self.since += 1;
        if self.since >= self.len {
            self.sum = self.window.iter().copied().sum();
            self.since = 0;
        }
        self.sum / self.len as f64
    }
}

/// Exponential smoothing seeded with the simple average of the first `len` values: EMA and Wilder's RMA.
#[derive(Debug)]
struct Smooth {
    len: usize,
    alpha: f64,
    prev: Option<f64>,
    sum: f64,
    run: usize,
}

impl Smooth {
    fn new(len: usize, alpha: f64) -> Self {
        Self { len, alpha, prev: None, sum: 0.0, run: 0 }
    }

    fn step(&mut self, v: f64) -> f64 {
        if v.is_nan() {
            (self.prev, self.sum, self.run) = (None, 0.0, 0);
            return NA;
        }
        match self.prev {
            None => {
                self.sum += v;
                self.run += 1;
                if self.run == self.len {
                    let seed = self.sum / self.len as f64;
                    self.prev = Some(seed);
                    return seed;
                }
                NA
            }
            Some(p) => {
                let next = self.alpha * v + (1.0 - self.alpha) * p;
                self.prev = Some(next);
                next
            }
        }
    }
}

/// The last `len` values in a row with a value.
#[derive(Debug)]
struct Window {
    len: usize,
    values: VecDeque<f64>,
}

impl Window {
    fn new(len: usize) -> Self {
        Self { len, values: VecDeque::new() }
    }

    /// Whether it is full after this value; a value missing starts it again.
    fn push(&mut self, v: f64) -> bool {
        if v.is_nan() {
            self.values.clear();
            return false;
        }
        self.values.push_back(v);
        if self.values.len() > self.len {
            self.values.pop_front();
        }
        self.values.len() == self.len
    }

    /// The weighted average, the newest weighing `len`: WMA.
    fn weighted(&self) -> f64 {
        let mut acc = 0.0;
        for (k, v) in self.values.iter().enumerate() {
            acc += v * (k + 1) as f64;
        }
        acc / (self.len * (self.len + 1) / 2) as f64
    }
}

/// The last `len` values, a missing one kept as missing: what `x[n]`-like functions read.
#[derive(Debug)]
struct Past {
    len: usize,
    values: VecDeque<f64>,
}

impl Past {
    fn push(&mut self, v: f64) {
        self.values.push_back(v);
        if self.values.len() > self.len {
            self.values.pop_front();
        }
    }

    fn full(&self) -> bool {
        self.values.len() == self.len
    }
}

#[derive(Debug)]
struct Wma(Window);

impl Wma {
    fn step(&mut self, v: f64) -> f64 {
        if self.0.push(v) { self.0.weighted() } else { NA }
    }
}

#[derive(Debug)]
enum State {
    Sma(Sma),
    Smooth(Smooth),
    Wma(Wma),
    Vwma(Sma, Sma),
    Hma { half: Wma, full: Wma, out: Wma },
    Stdev(Sma),
    Dev(Sma),
    Median(Window),
    Highest(Window),
    Lowest(Window),
    HighestBars(Window),
    LowestBars(Window),
    Sum(Window),
    Rsi { prev: f64, gain: Smooth, loss: Smooth },
    Atr(Smooth),
    Change(Past),
    Roc(Past),
    Linreg(Window),
    Cci(Sma),
    Mfi { prev: f64, up: Window, down: Window },
    Stoch { high: Window, low: Window },
    Correlation(Window, Window),
    PercentRank(Past),
    Rising(Past),
    Falling(Past),
    Pivot { left: usize, past: Past, high: bool },
    Cross { a: f64, b: f64, over: bool, under: bool },
    BarsSince(Option<f64>),
    ValueWhen(f64),
    Changed(Option<f64>),
    Cum(f64),
}

/// What a remembering function reads besides what it is given.
struct Ctx {
    tr: f64,
    volume: f64,
}

impl State {
    fn new(k: Keep) -> Self {
        let ema = |len: usize| Smooth::new(len, 2.0 / (len as f64 + 1.0));
        let rma = |len: usize| Smooth::new(len, 1.0 / len as f64);
        let past = |len: usize| Past { len, values: VecDeque::new() };
        match k {
            Keep::Sma(n) => State::Sma(Sma::new(n)),
            Keep::Ema(n) => State::Smooth(ema(n)),
            Keep::Rma(n) => State::Smooth(rma(n)),
            Keep::Wma(n) => State::Wma(Wma(Window::new(n))),
            Keep::Vwma(n) => State::Vwma(Sma::new(n), Sma::new(n)),
            Keep::Hma(n) => State::Hma { half: Wma(Window::new((n / 2).max(1))), full: Wma(Window::new(n)), out: Wma(Window::new(((n as f64).sqrt().floor() as usize).max(1))) },
            Keep::Stdev(n) => State::Stdev(Sma::new(n)),
            Keep::Dev(n) => State::Dev(Sma::new(n)),
            Keep::Median(n) => State::Median(Window::new(n)),
            Keep::Highest(n) => State::Highest(Window::new(n)),
            Keep::Lowest(n) => State::Lowest(Window::new(n)),
            Keep::HighestBars(n) => State::HighestBars(Window::new(n)),
            Keep::LowestBars(n) => State::LowestBars(Window::new(n)),
            Keep::Sum(n) => State::Sum(Window::new(n)),
            Keep::Rsi(n) => State::Rsi { prev: NA, gain: rma(n), loss: rma(n) },
            Keep::Atr(n) => State::Atr(rma(n)),
            Keep::Change(n) => State::Change(past(n + 1)),
            Keep::Roc(n) => State::Roc(past(n + 1)),
            Keep::Linreg(n) => State::Linreg(Window::new(n)),
            Keep::Cci(n) => State::Cci(Sma::new(n)),
            Keep::Mfi(n) => State::Mfi { prev: NA, up: Window::new(n), down: Window::new(n) },
            Keep::Stoch(n) => State::Stoch { high: Window::new(n), low: Window::new(n) },
            Keep::Correlation(n) => State::Correlation(Window::new(n), Window::new(n)),
            Keep::PercentRank(n) => State::PercentRank(past(n + 1)),
            Keep::Rising(n) => State::Rising(past(n + 1)),
            Keep::Falling(n) => State::Falling(past(n + 1)),
            Keep::PivotHigh(l, r) => State::Pivot { left: l, past: past(l + r + 1), high: true },
            Keep::PivotLow(l, r) => State::Pivot { left: l, past: past(l + r + 1), high: false },
            Keep::CrossOver => State::Cross { a: NA, b: NA, over: true, under: false },
            Keep::CrossUnder => State::Cross { a: NA, b: NA, over: false, under: true },
            Keep::Cross => State::Cross { a: NA, b: NA, over: true, under: true },
            Keep::BarsSince => State::BarsSince(None),
            Keep::ValueWhen => State::ValueWhen(NA),
            Keep::Changed => State::Changed(None),
            Keep::Cum => State::Cum(0.0),
        }
    }

    /// This bar's value from its arguments, and how many steps the window took.
    fn step(&mut self, v: [f64; 3], ctx: &Ctx) -> (f64, u64) {
        let [a, b, c] = v;
        match self {
            State::Sma(s) => (s.step(a), 1),
            State::Smooth(s) => (s.step(a), 1),
            State::Wma(w) => (w.step(a), w.0.len as u64),
            State::Vwma(xv, vol) => {
                let (top, bottom) = (xv.step(a * ctx.volume), vol.step(ctx.volume));
                (if bottom == 0.0 { NA } else { top / bottom }, 1)
            }
            State::Hma { half, full, out } => {
                let raw = 2.0 * half.step(a) - full.step(a);
                let n = (half.0.len + full.0.len + out.0.len) as u64;
                (out.step(raw), n)
            }
            State::Stdev(s) => deviation(s, a, false),
            State::Dev(s) => deviation(s, a, true),
            State::Median(w) => {
                if !w.push(a) {
                    return (NA, 1);
                }
                let mut xs: Vec<f64> = w.values.iter().copied().collect();
                tradoor_core::sort::heap_sort_by(&mut xs, |x, y| x < y);
                let m = xs.len() / 2;
                let v = if xs.len() % 2 == 1 { xs[m] } else { (xs[m - 1] + xs[m]) / 2.0 };
                (v, (w.len * 8) as u64)
            }
            State::Highest(w) => extreme(w, a, true),
            State::Lowest(w) => extreme(w, a, false),
            State::HighestBars(w) => bars_to(w, a, true),
            State::LowestBars(w) => bars_to(w, a, false),
            State::Sum(w) => {
                if !w.push(a) {
                    return (NA, 1);
                }
                (w.values.iter().copied().sum(), w.len as u64)
            }
            State::Rsi { prev, gain, loss } => {
                let d = a - *prev;
                *prev = a;
                let (up, down) = if d.is_nan() { (NA, NA) } else { (d.max(0.0), (-d).max(0.0)) };
                let (g, l) = (gain.step(up), loss.step(down));
                let v = if g.is_nan() || l.is_nan() {
                    NA
                } else if l == 0.0 {
                    100.0
                } else if g == 0.0 {
                    0.0
                } else {
                    100.0 - 100.0 / (1.0 + g / l)
                };
                (v, 1)
            }
            State::Atr(s) => (s.step(ctx.tr), 1),
            State::Change(p) => moved(p, a, false),
            State::Roc(p) => moved(p, a, true),
            State::Linreg(w) => {
                if !w.push(a) {
                    return (NA, 1);
                }
                // Least squares over the window, read at its newest bar.
                let n = w.len as f64;
                let (mut sx, mut sy, mut sxy, mut sxx) = (0.0, 0.0, 0.0, 0.0);
                for (k, y) in w.values.iter().enumerate() {
                    let x = k as f64;
                    sx += x;
                    sy += y;
                    sxy += x * y;
                    sxx += x * x;
                }
                let denom = n * sxx - sx * sx;
                let slope = if denom == 0.0 { 0.0 } else { (n * sxy - sx * sy) / denom };
                let intercept = (sy - slope * sx) / n;
                (intercept + slope * (n - 1.0), w.len as u64)
            }
            State::Cci(s) => {
                let mean = s.step(a);
                if mean.is_nan() {
                    return (NA, 1);
                }
                let dev: f64 = s.window.iter().map(|v| (v - mean).abs()).sum::<f64>() / s.len as f64;
                (if dev == 0.0 { 0.0 } else { (a - mean) / (0.015 * dev) }, s.len as u64)
            }
            State::Mfi { prev, up, down } => {
                let d = a - *prev;
                *prev = a;
                if d.is_nan() || ctx.volume.is_nan() {
                    up.values.clear();
                    down.values.clear();
                    return (NA, 1);
                }
                let flow = a * ctx.volume;
                let (u, dn) = (if d > 0.0 { flow } else { 0.0 }, if d < 0.0 { flow } else { 0.0 });
                if !(up.push(u) & down.push(dn)) {
                    return (NA, 1);
                }
                let (su, sd): (f64, f64) = (up.values.iter().sum(), down.values.iter().sum());
                (if sd == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + su / sd) }, up.len as u64)
            }
            State::Stoch { high, low } => {
                let full = high.push(b) & low.push(c);
                if !full || a.is_nan() {
                    return (NA, 1);
                }
                let (hh, ll) = (high.values.iter().copied().fold(f64::MIN, f64::max), low.values.iter().copied().fold(f64::MAX, f64::min));
                (if hh == ll { NA } else { 100.0 * (a - ll) / (hh - ll) }, high.len as u64)
            }
            State::Correlation(wa, wb) => {
                let full = wa.push(a) & wb.push(b);
                if !full {
                    return (NA, 1);
                }
                let n = wa.len as f64;
                let (ma, mb) = (wa.values.iter().sum::<f64>() / n, wb.values.iter().sum::<f64>() / n);
                let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
                for (x, y) in wa.values.iter().zip(&wb.values) {
                    cov += (x - ma) * (y - mb);
                    va += (x - ma) * (x - ma);
                    vb += (y - mb) * (y - mb);
                }
                (if va == 0.0 || vb == 0.0 { NA } else { cov / (va * vb).sqrt() }, wa.len as u64)
            }
            State::PercentRank(p) => {
                p.push(a);
                if !p.full() || a.is_nan() {
                    return (NA, 1);
                }
                let before = p.len - 1;
                let at_most = p.values.iter().take(before).filter(|v| **v <= a).count();
                (100.0 * at_most as f64 / before as f64, p.len as u64)
            }
            State::Rising(p) => trend(p, a, true),
            State::Falling(p) => trend(p, a, false),
            State::Pivot { left, past, high } => {
                past.push(a);
                if !past.full() {
                    return (NA, 1);
                }
                // Strict on both sides, as the chart's own: a tie is not a pivot.
                let centre = past.values[*left];
                if centre.is_nan() {
                    return (NA, 1);
                }
                let pivot = past.values.iter().enumerate().filter(|(k, _)| k != left).all(|(_, v)| !v.is_nan() && if *high { *v < centre } else { *v > centre });
                (if pivot { centre } else { NA }, past.len as u64)
            }
            State::Cross { a: pa, b: pb, over, under } => {
                let up = *over && a > b && *pa <= *pb;
                let down = *under && a < b && *pa >= *pb;
                (*pa, *pb) = (a, b);
                (flag(up || down), 1)
            }
            State::BarsSince(count) => {
                *count = if truth(a) { Some(0.0) } else { count.map(|c| c + 1.0) };
                (count.unwrap_or(NA), 1)
            }
            State::ValueWhen(last) => {
                if truth(a) {
                    *last = b;
                }
                (*last, 1)
            }
            State::Changed(prev) => {
                let changed = prev.is_some_and(|p| p != a && !(p.is_nan() && a.is_nan()));
                *prev = Some(a);
                (flag(changed), 1)
            }
            State::Cum(total) => {
                if !a.is_nan() {
                    *total += a;
                }
                (*total, 1)
            }
        }
    }
}

/// The standard deviation of the whole set (÷ length), or the mean absolute deviation.
fn deviation(s: &mut Sma, a: f64, absolute: bool) -> (f64, u64) {
    let mean = s.step(a);
    if mean.is_nan() {
        return (NA, 1);
    }
    let acc: f64 = if absolute { s.window.iter().map(|v| (v - mean).abs()).sum() } else { s.window.iter().map(|v| (v - mean) * (v - mean)).sum() };
    let n = s.len as f64;
    (if absolute { acc / n } else { (acc / n).sqrt() }, s.len as u64)
}

/// Bars back to the extreme, as Pine gives it: 0 or less, the newest of a tie.
fn bars_to(w: &mut Window, a: f64, high: bool) -> (f64, u64) {
    if !w.push(a) {
        return (NA, 1);
    }
    let mut best = (f64::NAN, 0usize);
    for (k, v) in w.values.iter().enumerate() {
        if best.0.is_nan() || if high { *v >= best.0 } else { *v <= best.0 } {
            best = (*v, k);
        }
    }
    (-((w.values.len() - 1 - best.1) as f64), w.len as u64)
}

/// `x - x[n]`, or that as a percentage of `x[n]`.
fn moved(p: &mut Past, a: f64, percent: bool) -> (f64, u64) {
    p.push(a);
    if !p.full() {
        return (NA, 1);
    }
    let old = p.values.front().copied().unwrap_or(NA);
    (if !percent { a - old } else if old == 0.0 { NA } else { 100.0 * (a - old) / old }, 1)
}

/// Above (or below) every one of the `len` values before it.
fn trend(p: &mut Past, a: f64, rising: bool) -> (f64, u64) {
    p.push(a);
    if !p.full() || a.is_nan() {
        return (0.0, 1);
    }
    let mut before = p.values.iter().take(p.len - 1);
    let hit = if rising { before.all(|v| a > *v) } else { before.all(|v| a < *v) };
    (flag(hit), p.len as u64)
}

/// The highest or lowest of a window, scanned from its oldest as the chart's own is.
fn extreme(w: &mut Window, a: f64, max: bool) -> (f64, u64) {
    if !w.push(a) {
        return (NA, 1);
    }
    let mut acc = a;
    for v in &w.values {
        if if max { *v > acc } else { *v < acc } {
            acc = *v;
        }
    }
    (acc, w.len as u64)
}

/// The shapes a script keeps: each a serial number, the oldest let go past its kind's limit.
#[derive(Default)]
struct Shapes {
    lines: VecDeque<(u64, LineArt)>,
    labels: VecDeque<(u64, LabelArt)>,
    boxes: VecDeque<(u64, BoxArt)>,
    next: u64,
}

/// A shape's id, as a script holds it: its serial and its kind (1 line, 2 label, 3 box).
fn id(serial: u64, kind: u64) -> f64 {
    (serial * 4 + kind) as f64
}

fn parts(id: f64) -> Option<(u64, u64)> {
    (id.is_finite() && id >= 0.0).then_some((id as u64 / 4, id as u64 % 4))
}

fn find<T>(list: &mut VecDeque<(u64, T)>, serial: u64) -> Option<&mut T> {
    let i = list.binary_search_by_key(&serial, |(s, _)| *s).ok()?;
    list.get_mut(i).map(|(_, t)| t)
}

/// What a run of lines did: carried on, or left a loop or a function.
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

struct Machine<'a> {
    bars: &'a Bars<'a>,
    n: usize,
    i: usize,
    decimals: u8,
    interval: f64,
    nums: Vec<f64>,
    texts: Vec<Rc<str>>,
    started: Vec<bool>,
    started_texts: Vec<bool>,
    /// Each remembered variable's values at the end of past bars, the newest last.
    past: Vec<VecDeque<f64>>,
    states: Vec<State>,
    steps: u64,
    plots: Vec<Vec<f64>>,
    plot_colors: Vec<Option<Vec<u32>>>,
    marks: Vec<Vec<bool>>,
    backgrounds: Vec<Vec<u32>>,
    bar_colors: Option<Vec<u32>>,
    /// Rings: taken from or added at either end at no cost.
    lists: Vec<VecDeque<f64>>,
    shapes: Shapes,
    cells: Vec<Cell>,
    empty: Rc<str>,
}

pub fn run(p: &Program, bars: &Bars, decimals: u8) -> Result<Output, Error> {
    let n = bars.len();
    // A bar's length: the commonest gap among the first ones.
    let mut gaps: Vec<f64> = bars.time.windows(2).take(21).map(|w| w[1] - w[0]).filter(|g| *g > 0.0).collect();
    tradoor_core::sort::insertion_sort_by(&mut gaps, |a, b| a < b);
    let interval = gaps.get(gaps.len() / 2).copied().unwrap_or(NA);
    let empty: Rc<str> = Rc::from("");
    let mut m = Machine {
        bars,
        n,
        i: 0,
        decimals,
        interval,
        nums: vec![NA; p.nums],
        texts: vec![empty.clone(); p.texts],
        started: vec![false; p.nums],
        started_texts: vec![false; p.texts],
        past: vec![VecDeque::new(); p.nums],
        states: p.sites.iter().map(|k| State::new(*k)).collect(),
        steps: 0,
        plots: vec![vec![NA; n]; p.plots.len()],
        plot_colors: p.plots.iter().map(|s| s.per_bar.then(|| vec![0; n])).collect(),
        marks: vec![vec![false; n]; p.marks.len()],
        backgrounds: vec![vec![0; n]; p.backgrounds],
        bar_colors: p.bar_colors.then(|| vec![0; n]),
        lists: vec![VecDeque::new(); p.lists],
        shapes: Shapes::default(),
        cells: Vec::new(),
        empty,
    };
    for i in 0..n {
        m.i = i;
        for (slot, kept) in p.kept.iter().enumerate() {
            if !kept {
                m.nums[slot] = NA;
            }
        }
        for (slot, kept) in p.kept_texts.iter().enumerate() {
            if !kept {
                m.texts[slot] = m.empty.clone();
            }
        }
        m.lines(&p.body)?;
        m.count(p.kept_back, None)?;
        for (slot, &deep) in p.remembered.iter().enumerate() {
            if deep > 0 {
                let past = &mut m.past[slot];
                past.push_back(m.nums[slot]);
                if past.len() > deep {
                    past.pop_front();
                }
            }
        }
    }
    let plots = p
        .plots
        .iter()
        .zip(m.plots)
        .zip(m.plot_colors)
        .map(|((s, values), colors)| Plot { title: s.title.clone(), kind: s.kind, style: s.style, color: s.color, width: s.width, offset: s.offset, values, colors })
        .collect();
    let marks = p.marks.iter().zip(m.marks).map(|(s, hits)| Mark { text: s.text.clone(), above: s.above, color: s.color, hits }).collect();
    let fills = p.fills.iter().map(|(upper, lower, color)| Fill { upper: *upper, lower: *lower, color: *color }).collect();
    let reach = |x: f64| x.clamp(-MAX_AHEAD, n as f64 - 1.0 + MAX_AHEAD);
    let ok = |v: &[f64]| v.iter().all(|x| x.is_finite());
    let lines = m.shapes.lines.into_iter().map(|(_, l)| l).filter(|l| ok(&[l.x1, l.y1, l.x2, l.y2])).map(|l| LineArt { x1: reach(l.x1), x2: reach(l.x2), ..l }).collect();
    let labels = m.shapes.labels.into_iter().map(|(_, l)| l).filter(|l| ok(&[l.x, l.y])).map(|l| LabelArt { x: reach(l.x), ..l }).collect();
    let boxes = m.shapes.boxes.into_iter().map(|(_, b)| b).filter(|b| ok(&[b.x1, b.y1, b.x2, b.y2])).map(|b| BoxArt { x1: reach(b.x1), x2: reach(b.x2), ..b }).collect();
    let table = (!m.cells.is_empty()).then_some(Table { down: p.table.0, across: p.table.1, cells: m.cells });
    Ok(Output { own_pane: p.own_pane, plots, fills, marks, backgrounds: m.backgrounds, bar_colors: m.bar_colors, lines, labels, boxes, table })
}

/// `v` as a text: a whole number bare, else up to eight decimals with no zeros left over.
fn plain(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return fixed(v, 0);
    }
    let t = fixed(v, 8);
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Machine<'_> {
    fn count(&mut self, steps: u64, at: Option<At>) -> Result<(), Error> {
        self.steps += steps;
        if self.steps > MAX_STEPS {
            let message = "this script takes too long to work out: fewer loops, or shorter ones";
            return Err(at.map_or_else(|| Error::whole(message), |at| Error::at(at, message)));
        }
        Ok(())
    }

    fn lines(&mut self, lines: &[Run]) -> Result<Flow, Error> {
        for line in lines {
            self.count(1, None)?;
            match line {
                Run::Set(slot, e) => {
                    let v = self.ex(e)?;
                    self.nums[*slot] = v;
                }
                Run::SetText(slot, t) => {
                    let v = self.tx(t)?;
                    self.texts[*slot] = v;
                }
                Run::Start(slot, e) => {
                    if !self.started[*slot] {
                        self.started[*slot] = true;
                        let v = self.ex(e)?;
                        self.nums[*slot] = v;
                    }
                }
                Run::StartText(slot, t) => {
                    if !self.started_texts[*slot] {
                        self.started_texts[*slot] = true;
                        let v = self.tx(t)?;
                        self.texts[*slot] = v;
                    }
                }
                Run::ListClear(id) => self.lists[*id].clear(),
                Run::If(arms, otherwise) => {
                    let mut taken = None;
                    for (cond, body) in arms {
                        if truth(self.ex(cond)?) {
                            taken = Some(body);
                            break;
                        }
                    }
                    let flow = self.lines(taken.unwrap_or(otherwise))?;
                    if flow != Flow::Next {
                        return Ok(flow);
                    }
                }
                Run::For { slot, from, to, by, body, at } => {
                    let (from, to) = (self.ex(from)?.round(), self.ex(to)?.round());
                    let by = match by {
                        Some(b) => self.ex(b)?,
                        None => 1.0,
                    };
                    if by == 0.0 || by.is_nan() {
                        return Err(Error::at(*at, "a loop's by is a number other than 0"));
                    }
                    if from.is_nan() || to.is_nan() || (by > 0.0 && to < from) || (by < 0.0 && to > from) {
                        continue;
                    }
                    if ((to - from) / by).floor() + 1.0 > MAX_LOOP as f64 {
                        return Err(Error::at(*at, format!("a loop goes round {MAX_LOOP} times at most")));
                    }
                    let mut k = from;
                    while if by > 0.0 { k <= to } else { k >= to } {
                        self.nums[*slot] = k;
                        match self.lines(body)? {
                            Flow::Break => break,
                            Flow::Return => return Ok(Flow::Return),
                            _ => {}
                        }
                        k += by;
                    }
                }
                Run::While { cond, body, at } => {
                    let mut rounds = 0usize;
                    while truth(self.ex(cond)?) {
                        rounds += 1;
                        if rounds > MAX_LOOP {
                            return Err(Error::at(*at, format!("a loop goes round {MAX_LOOP} times at most")));
                        }
                        match self.lines(body)? {
                            Flow::Break => break,
                            Flow::Return => return Ok(Flow::Return),
                            _ => {}
                        }
                    }
                }
                Run::Break => return Ok(Flow::Break),
                Run::Continue => return Ok(Flow::Continue),
                Run::Return(rets) => {
                    for r in rets {
                        match r {
                            Ret::Num(slot, e) => {
                                let v = self.ex(e)?;
                                self.nums[*slot] = v;
                            }
                            Ret::Text(slot, t) => {
                                let v = self.tx(t)?;
                                self.texts[*slot] = v;
                            }
                        }
                    }
                    return Ok(Flow::Return);
                }
                Run::Block(body) => {
                    self.lines(body)?;
                }
                Run::Plot(index, e, c) => {
                    let v = self.ex(e)?;
                    if let Some(cell) = self.plots.get_mut(*index).and_then(|p| p.get_mut(self.i)) {
                        *cell = v;
                    }
                    if let Some(c) = c {
                        let c = colour(self.ex(c)?);
                        if let Some(cell) = self.plot_colors.get_mut(*index).and_then(|p| p.as_mut()).and_then(|p| p.get_mut(self.i)) {
                            *cell = c;
                        }
                    }
                }
                Run::Mark(index, e) => {
                    let hit = truth(self.ex(e)?);
                    if let Some(cell) = self.marks.get_mut(*index).and_then(|m| m.get_mut(self.i)) {
                        *cell = hit;
                    }
                }
                Run::Background(index, e) => {
                    let c = colour(self.ex(e)?);
                    if let Some(cell) = self.backgrounds.get_mut(*index).and_then(|b| b.get_mut(self.i)) {
                        *cell = c;
                    }
                }
                Run::BarColor(e) => {
                    let c = colour(self.ex(e)?);
                    let i = self.i;
                    if let Some(cell) = self.bar_colors.as_mut().and_then(|b| b.get_mut(i)) {
                        *cell = c;
                    }
                }
                Run::Cell(col, row, text, color, background) => {
                    let (col, row) = (self.ex(col)?.round(), self.ex(row)?.round());
                    let text = self.tx(text)?;
                    let (color, background) = (self.ex(color)?, self.ex(background)?);
                    if !(0.0..TABLE_COLUMNS as f64).contains(&col) || !(0.0..TABLE_ROWS as f64).contains(&row) {
                        continue;
                    }
                    let (col, row) = (col as u8, row as u8);
                    let cell = Cell { col, row, text: text.to_string(), color: if color.is_nan() { 0 } else { colour(color) }, background: colour(background) };
                    match self.cells.iter_mut().find(|c| c.col == col && c.row == row) {
                        Some(c) => *c = cell,
                        None => self.cells.push(cell),
                    }
                }
                Run::Eval(e) => {
                    self.ex(e)?;
                }
            }
        }
        Ok(Flow::Next)
    }

    fn series(&self, s: Series, i: usize) -> f64 {
        let b = self.bars;
        let at = |xs: &[f64]| xs.get(i).copied().unwrap_or(NA);
        let t = || at(b.time) as i64;
        match s {
            Series::Open => at(b.open),
            Series::High => at(b.high),
            Series::Low => at(b.low),
            Series::Close => at(b.close),
            Series::Volume => at(b.volume),
            Series::Hl2 => (at(b.high) + at(b.low)) / 2.0,
            Series::Hlc3 => (at(b.high) + at(b.low) + at(b.close)) / 3.0,
            Series::Ohlc4 => (at(b.open) + at(b.high) + at(b.low) + at(b.close)) / 4.0,
            // The first bar has no close before it: its range alone.
            Series::Tr => {
                let (h, l) = (at(b.high), at(b.low));
                match i.checked_sub(1).and_then(|j| b.close.get(j)) {
                    None => h - l,
                    Some(&prev) => (h - l).max((h - prev).abs()).max((l - prev).abs()),
                }
            }
            Series::Bar => i as f64,
            Series::Time => at(b.time),
            Series::Day => t().div_euclid(DAY) as f64,
            Series::Week => week_index(t()) as f64,
            Series::Month => month_index(t()) as f64,
            Series::Year => year_of(t()) as f64,
            Series::Hour => f64::from(civil(t()).hour),
            Series::Minute => f64::from(civil(t()).minute),
            Series::DayOfWeek => f64::from(weekday(t()) + 1),
            Series::DayOfMonth => f64::from(civil(t()).day),
            Series::MonthOfYear => f64::from(civil(t()).month0 + 1),
            Series::IsFirst => flag(i == 0),
            Series::IsLast => flag(i + 1 == self.n),
            Series::LastBar => self.n as f64 - 1.0,
            Series::Interval => self.interval,
            Series::Tick => 10f64.powi(-i32::from(self.decimals)),
            Series::Decimals => f64::from(self.decimals),
        }
    }

    fn ex(&mut self, e: &Ex) -> Result<f64, Error> {
        self.count(1, None)?;
        Ok(match e {
            Ex::Num(v) => *v,
            Ex::Slot(s) => self.nums.get(*s).copied().unwrap_or(NA),
            Ex::Bar(s) => self.series(*s, self.i),
            Ex::Back(target, back, at) => {
                let k = self.ex(back)?.round();
                if k.is_nan() {
                    return Ok(NA);
                }
                if k < 0.0 {
                    return Err(Error::at(*at, "[n] looks back: n is 0 or more"));
                }
                if k > MAX_LOOKBACK as f64 {
                    return Err(Error::at(*at, format!("[n] looks back {MAX_LOOKBACK} bars at most")));
                }
                let k = k as usize;
                match target {
                    _ if k > self.i => NA,
                    Target::Bar(s) => self.series(*s, self.i - k),
                    Target::Slot(s) if k == 0 => self.nums.get(*s).copied().unwrap_or(NA),
                    Target::Slot(s) => self.past.get(*s).and_then(|p| p.len().checked_sub(k).and_then(|j| p.get(j))).copied().unwrap_or(NA),
                }
            }
            Ex::Neg(a) => -self.ex(a)?,
            Ex::Not(a) => flag(!truth(self.ex(a)?)),
            Ex::Bin(op, a, b) => {
                // Both sides always: a function that remembers past bars must hear of every one.
                let (a, b) = (self.ex(a)?, self.ex(b)?);
                binary(*op, a, b)
            }
            Ex::Pick(c, a, b) => {
                let (c, a, b) = (self.ex(c)?, self.ex(a)?, self.ex(b)?);
                if truth(c) { a } else { b }
            }
            Ex::Math(f, args) => {
                self.count(3, None)?;
                let mut vals = [NA; 12];
                for (slot, a) in vals.iter_mut().zip(args) {
                    *slot = self.ex(a)?;
                }
                math(*f, &vals[..args.len().min(12)])
            }
            Ex::Keep(site, args) => {
                let mut v = [NA; 3];
                for (slot, a) in v.iter_mut().zip(args) {
                    *slot = self.ex(a)?;
                }
                let ctx = Ctx { tr: self.series(Series::Tr, self.i), volume: self.series(Series::Volume, self.i) };
                let Some(state) = self.states.get_mut(*site) else { return Ok(NA) };
                let (v, steps) = state.step(v, &ctx);
                self.count(steps, None)?;
                v
            }
            Ex::TextEq(same, a, b) => {
                let (a, b) = (self.tx(a)?, self.tx(b)?);
                flag((a == b) == *same)
            }
            Ex::List(op, id, args, at) => self.list(*op, *id, args, *at)?,
            Ex::Shape(op, args, text, at) => self.shape(*op, args, text.as_deref(), *at)?,
            Ex::BarAt(t) => {
                let t = self.ex(t)?;
                let times = self.bars.time;
                if t.is_nan() || times.first().is_none_or(|first| t < *first) {
                    return Ok(NA);
                }
                (times.partition_point(|x| *x <= t) - 1) as f64
            }
            Ex::Call(call) => {
                let (body, slot) = &**call;
                self.nums[*slot] = NA;
                self.lines(body)?;
                self.nums[*slot]
            }
        })
    }

    fn tx(&mut self, t: &Tx) -> Result<Rc<str>, Error> {
        self.count(1, None)?;
        Ok(match t {
            Tx::Lit(s) => s.clone(),
            Tx::Slot(s) => self.texts.get(*s).cloned().unwrap_or_else(|| self.empty.clone()),
            Tx::Join(parts) => {
                let mut out = String::new();
                for p in parts {
                    let piece = self.tx(p)?;
                    out.push_str(&piece);
                }
                // Charged by what was copied: a long text made again and again is real work.
                self.count(out.len() as u64 / 2, None)?;
                cut(out)
            }
            Tx::Num(e, fmt) => {
                // Writing a number out is some tens of steps' work.
                self.count(24, None)?;
                let v = self.ex(e)?;
                Rc::from(match fmt {
                    Fmt::Plain => plain(v),
                    Fmt::Fixed(d) => {
                        let d = self.ex(d)?;
                        if v.is_nan() { "NaN".to_string() } else { fixed(v, d.round().clamp(0.0, 10.0) as usize) }
                    }
                    Fmt::Duration if v.is_nan() => "NaN".to_string(),
                    Fmt::Duration => tradoor_core::studies::format_duration(v),
                })
            }
            Tx::Pick(c, a, b) => {
                let (c, a, b) = (self.ex(c)?, self.tx(a)?, self.tx(b)?);
                if truth(c) { a } else { b }
            }
            Tx::Of(e) => {
                let id = self.ex(e)?;
                match parts(id) {
                    Some((serial, 2)) => find(&mut self.shapes.labels, serial).map_or_else(|| self.empty.clone(), |l| Rc::from(l.text.as_str())),
                    _ => self.empty.clone(),
                }
            }
            Tx::Call(call) => {
                let (body, slot) = &**call;
                self.texts[*slot] = self.empty.clone();
                self.lines(body)?;
                self.texts[*slot].clone()
            }
        })
    }

    fn list(&mut self, op: ListOp, id: usize, args: &[Ex], at: At) -> Result<f64, Error> {
        let mut v = [NA; 2];
        for (slot, a) in v.iter_mut().zip(args) {
            *slot = self.ex(a)?;
        }
        let xs = &mut self.lists[id];
        let len = xs.len();
        // From the start, or from the end where it is below 0: get(-1) is the last.
        let index = |i: f64, room: usize| -> Result<usize, Error> {
            let i = i.round();
            let k = if i < 0.0 { room as f64 + i } else { i };
            if k.is_nan() || k < 0.0 || k >= room as f64 {
                return Err(Error::at(at, format!("{} is not a place in this list of {len}", plain(i))));
            }
            Ok(k as usize)
        };
        // Charged by how much of the list each goes over or moves: either end is free.
        let steps = match op {
            ListOp::Sum | ListOp::Avg | ListOp::IndexOf | ListOp::Includes | ListOp::Reverse | ListOp::Insert | ListOp::Remove => len as u64 / 8,
            ListOp::Min | ListOp::Max => len as u64 / 4,
            ListOp::Sort => len as u64 * 10,
            _ => 0,
        };
        let value = match op {
            ListOp::Push => {
                if xs.len() >= MAX_LIST {
                    xs.pop_front();
                }
                xs.push_back(v[0]);
                NA
            }
            ListOp::Unshift => {
                if xs.len() >= MAX_LIST {
                    xs.pop_back();
                }
                xs.push_front(v[0]);
                NA
            }
            ListOp::Pop => xs.pop_back().unwrap_or(NA),
            ListOp::Shift => xs.pop_front().unwrap_or(NA),
            ListOp::Get => xs[index(v[0], len)?],
            ListOp::Set => {
                let k = index(v[0], len)?;
                xs[k] = v[1];
                NA
            }
            ListOp::Insert => {
                let k = index(v[0], len + 1)?;
                if xs.len() >= MAX_LIST {
                    return Err(Error::at(at, format!("a list holds {MAX_LIST} values at most")));
                }
                xs.insert(k, v[1]);
                NA
            }
            ListOp::Remove => {
                let k = index(v[0], len)?;
                xs.remove(k).unwrap_or(NA)
            }
            ListOp::Clear => {
                xs.clear();
                NA
            }
            ListOp::Size => len as f64,
            ListOp::First => xs.front().copied().unwrap_or(NA),
            ListOp::Last => xs.back().copied().unwrap_or(NA),
            ListOp::Sum | ListOp::Avg => {
                let (sum, count) = xs.iter().filter(|x| !x.is_nan()).fold((0.0, 0usize), |(s, c), x| (s + x, c + 1));
                match (op, count) {
                    (ListOp::Avg, 0) => NA,
                    (ListOp::Avg, c) => sum / c as f64,
                    _ => sum,
                }
            }
            ListOp::Min | ListOp::Max => xs.iter().copied().filter(|x| !x.is_nan()).reduce(|a, b| if op == ListOp::Max { a.max(b) } else { a.min(b) }).unwrap_or(NA),
            ListOp::IndexOf => xs.iter().position(|x| *x == v[0]).map_or(-1.0, |p| p as f64),
            ListOp::Includes => flag(xs.contains(&v[0])),
            ListOp::Sort => {
                // Missing values last.
                tradoor_core::sort::heap_sort_by(xs.make_contiguous(), |a, b| if a.is_nan() { false } else if b.is_nan() { true } else { a < b });
                NA
            }
            ListOp::Reverse => {
                xs.make_contiguous().reverse();
                NA
            }
        };
        self.count(steps, Some(at))?;
        Ok(value)
    }

    fn shape(&mut self, op: ShapeOp, args: &[Ex], text: Option<&Tx>, _at: At) -> Result<f64, Error> {
        // A shape made, found or let go is a few steps' work.
        self.count(4, None)?;
        let mut v = [NA; 8];
        for (slot, a) in v.iter_mut().zip(args) {
            *slot = self.ex(a)?;
        }
        let text = match text {
            Some(t) => Some(self.tx(t)?),
            None => None,
        };
        let s = &mut self.shapes;
        let width = |w: f64| if w.is_finite() { w.clamp(0.5, 10.0) as f32 } else { 1.0 };
        let small = |w: f64, top: f64| if w.is_finite() { w.round().clamp(0.0, top) as u8 } else { 0 };
        match op {
            ShapeOp::Line => {
                let serial = s.next;
                s.next += 1;
                s.lines.push_back((serial, LineArt { x1: v[0], y1: v[1], x2: v[2], y2: v[3], color: colour(v[4]), width: width(v[5]), style: small(v[6], 2.0), extend: small(v[7], 3.0) }));
                if s.lines.len() > MAX_LINES {
                    s.lines.pop_front();
                }
                return Ok(id(serial, 1));
            }
            ShapeOp::Label => {
                let serial = s.next;
                s.next += 1;
                let text = text.map_or_else(String::new, |t| t.to_string());
                s.labels.push_back((serial, LabelArt { x: v[0], y: v[1], text, color: colour(v[2]), text_color: colour(v[3]), place: small(v[4], 4.0), size: small(v[5], 3.0), plain: !truth(v[6]) }));
                if s.labels.len() > MAX_LABELS {
                    s.labels.pop_front();
                }
                return Ok(id(serial, 2));
            }
            ShapeOp::Box => {
                let serial = s.next;
                s.next += 1;
                s.boxes.push_back((serial, BoxArt { x1: v[0], y1: v[1], x2: v[2], y2: v[3], fill: colour(v[4]), border: colour(v[5]), width: width(v[6]) }));
                if s.boxes.len() > MAX_BOXES {
                    s.boxes.pop_front();
                }
                return Ok(id(serial, 3));
            }
            _ => {}
        }
        // A shape already let go, or never one, is left alone: as Pine leaves it.
        let Some((serial, kind)) = parts(v[0]) else { return Ok(NA) };
        let (a, b) = (v[1], v[2]);
        Ok(match kind {
            1 => {
                if op == ShapeOp::Delete {
                    if let Ok(i) = s.lines.binary_search_by_key(&serial, |(k, _)| *k) {
                        s.lines.remove(i);
                    }
                    return Ok(NA);
                }
                let Some(l) = find(&mut s.lines, serial) else { return Ok(NA) };
                match op {
                    ShapeOp::SetX1 => l.x1 = a,
                    ShapeOp::SetY1 => l.y1 = a,
                    ShapeOp::SetX2 => l.x2 = a,
                    ShapeOp::SetY2 => l.y2 = a,
                    ShapeOp::SetXy1 => (l.x1, l.y1) = (a, b),
                    ShapeOp::SetXy2 => (l.x2, l.y2) = (a, b),
                    ShapeOp::SetColor => l.color = colour(a),
                    ShapeOp::SetWidth => l.width = width(a),
                    ShapeOp::GetX1 => return Ok(l.x1),
                    ShapeOp::GetY1 => return Ok(l.y1),
                    ShapeOp::GetX2 => return Ok(l.x2),
                    ShapeOp::GetY2 => return Ok(l.y2),
                    _ => {}
                }
                NA
            }
            2 => {
                if op == ShapeOp::Delete {
                    if let Ok(i) = s.labels.binary_search_by_key(&serial, |(k, _)| *k) {
                        s.labels.remove(i);
                    }
                    return Ok(NA);
                }
                let Some(l) = find(&mut s.labels, serial) else { return Ok(NA) };
                match op {
                    ShapeOp::SetX1 => l.x = a,
                    ShapeOp::SetY1 => l.y = a,
                    ShapeOp::SetXy1 | ShapeOp::SetXy2 => (l.x, l.y) = (a, b),
                    ShapeOp::SetText => l.text = text.map_or_else(String::new, |t| t.to_string()),
                    ShapeOp::SetColor => l.color = colour(a),
                    ShapeOp::SetTextColor => l.text_color = colour(a),
                    ShapeOp::GetX1 | ShapeOp::GetX2 => return Ok(l.x),
                    ShapeOp::GetY1 | ShapeOp::GetY2 => return Ok(l.y),
                    _ => {}
                }
                NA
            }
            3 => {
                if op == ShapeOp::Delete {
                    if let Ok(i) = s.boxes.binary_search_by_key(&serial, |(k, _)| *k) {
                        s.boxes.remove(i);
                    }
                    return Ok(NA);
                }
                let Some(bx) = find(&mut s.boxes, serial) else { return Ok(NA) };
                match op {
                    ShapeOp::SetX1 => bx.x1 = a,
                    ShapeOp::SetY1 => bx.y1 = a,
                    ShapeOp::SetX2 => bx.x2 = a,
                    ShapeOp::SetY2 => bx.y2 = a,
                    ShapeOp::SetXy1 => (bx.x1, bx.y1) = (a, b),
                    ShapeOp::SetXy2 => (bx.x2, bx.y2) = (a, b),
                    ShapeOp::SetColor => bx.fill = colour(a),
                    ShapeOp::SetTextColor => bx.border = colour(a),
                    ShapeOp::SetWidth => bx.width = width(a),
                    ShapeOp::GetX1 => return Ok(bx.x1),
                    ShapeOp::GetY1 => return Ok(bx.y1),
                    ShapeOp::GetX2 => return Ok(bx.x2),
                    ShapeOp::GetY2 => return Ok(bx.y2),
                    _ => {}
                }
                NA
            }
            _ => NA,
        })
    }
}

fn cut(mut out: String) -> Rc<str> {
    if out.chars().count() > MAX_MADE_TEXT {
        out = out.chars().take(MAX_MADE_TEXT).collect();
    }
    Rc::from(out)
}

/// Arithmetic gives no value where a side has none, or on a division by zero;
/// a comparison with no value is not true.
pub(crate) fn binary(op: Op, a: f64, b: f64) -> f64 {
    match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * b,
        Op::Div if b == 0.0 => NA,
        Op::Div => a / b,
        Op::Rem if b == 0.0 => NA,
        Op::Rem => a % b,
        Op::And => flag(truth(a) && truth(b)),
        Op::Or => flag(truth(a) || truth(b)),
        _ if a.is_nan() || b.is_nan() => NA,
        Op::Eq => flag(a == b),
        Op::Ne => flag(a != b),
        Op::Lt => flag(a < b),
        Op::Le => flag(a <= b),
        Op::Gt => flag(a > b),
        Op::Ge => flag(a >= b),
    }
}
