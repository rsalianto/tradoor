//! A program run over the bars, one bar at a time, top to bottom. A variable
//! starts each bar with no value unless it is a `state`; `x[n]` reads what it
//! ended an earlier bar on. The functions that remember past bars keep their
//! own memory, one for each place they are called, and do their sums in the
//! order the chart's built-in indicators do, so a formula written as one of
//! them gives the same numbers. Every step is counted against a budget.
use std::collections::VecDeque;

use crate::compile::{Ex, Keep, Math, Program, Run, Series, Target};
use crate::syntax::{At, Op};
use crate::{Bars, Error, Fill, Mark, Output, Plot, MAX_LOOKBACK, MAX_LOOP, MAX_STEPS};
use tradoor_core::time::{month_index, week_index, year_of, DAY};

const NA: f64 = f64::NAN;

fn truth(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
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

/// The last `len` values in a row with a value: what highest, lowest and sum read.
#[derive(Debug)]
struct Window {
    len: usize,
    values: VecDeque<f64>,
}

impl Window {
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
}

#[derive(Debug)]
enum State {
    Sma(Sma),
    Smooth(Smooth),
    Stdev(Sma),
    Highest(Window),
    Lowest(Window),
    Sum(Window),
    Rsi { prev: f64, gain: Smooth, loss: Smooth },
    Atr(Smooth),
    Change(VecDeque<f64>, usize),
    Cross { a: f64, b: f64, over: bool },
    BarsSince(Option<f64>),
    ValueWhen(f64),
    Changed(Option<f64>),
}

impl State {
    fn new(k: Keep) -> Self {
        let ema = |len: usize| Smooth::new(len, 2.0 / (len as f64 + 1.0));
        let rma = |len: usize| Smooth::new(len, 1.0 / len as f64);
        let window = |len| Window { len, values: VecDeque::new() };
        match k {
            Keep::Sma(n) => State::Sma(Sma::new(n)),
            Keep::Ema(n) => State::Smooth(ema(n)),
            Keep::Rma(n) => State::Smooth(rma(n)),
            Keep::Stdev(n) => State::Stdev(Sma::new(n)),
            Keep::Highest(n) => State::Highest(window(n)),
            Keep::Lowest(n) => State::Lowest(window(n)),
            Keep::Sum(n) => State::Sum(window(n)),
            Keep::Rsi(n) => State::Rsi { prev: NA, gain: rma(n), loss: rma(n) },
            Keep::Atr(n) => State::Atr(rma(n)),
            Keep::Change(n) => State::Change(VecDeque::new(), n),
            Keep::CrossOver => State::Cross { a: NA, b: NA, over: true },
            Keep::CrossUnder => State::Cross { a: NA, b: NA, over: false },
            Keep::BarsSince => State::BarsSince(None),
            Keep::ValueWhen => State::ValueWhen(NA),
            Keep::Changed => State::Changed(None),
        }
    }

    /// This bar's value from its arguments, and how many steps the window took.
    fn step(&mut self, a: f64, b: f64, tr: f64) -> (f64, u64) {
        match self {
            State::Sma(s) => (s.step(a), 1),
            State::Smooth(s) => (s.step(a), 1),
            State::Stdev(s) => {
                let mean = s.step(a);
                if mean.is_nan() {
                    return (NA, 1);
                }
                let acc: f64 = s.window.iter().map(|v| (v - mean) * (v - mean)).sum();
                ((acc / s.len as f64).sqrt(), s.len as u64)
            }
            State::Highest(w) => extreme(w, a, true),
            State::Lowest(w) => extreme(w, a, false),
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
            State::Atr(s) => (s.step(tr), 1),
            State::Change(past, n) => {
                let v = if past.len() == *n { past.front().map_or(NA, |old| a - old) } else { NA };
                past.push_back(a);
                if past.len() > *n {
                    past.pop_front();
                }
                (v, 1)
            }
            State::Cross { a: pa, b: pb, over } => {
                let hit = if *over { a > b && *pa <= *pb } else { a < b && *pa >= *pb };
                (*pa, *pb) = (a, b);
                (flag(hit), 1)
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
        }
    }
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

struct Machine<'a> {
    bars: &'a Bars<'a>,
    i: usize,
    vals: Vec<f64>,
    /// Each remembered variable's values at the end of past bars, the newest last.
    past: Vec<VecDeque<f64>>,
    states: Vec<State>,
    steps: u64,
    plots: Vec<Vec<f64>>,
    marks: Vec<Vec<bool>>,
}

pub fn run(p: &Program, bars: &Bars) -> Result<Output, Error> {
    let n = bars.len();
    let mut m = Machine {
        bars,
        i: 0,
        vals: vec![NA; p.slots],
        past: vec![VecDeque::new(); p.slots],
        states: p.sites.iter().map(|k| State::new(*k)).collect(),
        steps: 0,
        plots: vec![vec![NA; n]; p.plots.len()],
        marks: vec![vec![false; n]; p.marks.len()],
    };
    for i in 0..n {
        m.i = i;
        for (slot, kept) in p.kept.iter().enumerate() {
            if !kept {
                m.vals[slot] = NA;
            }
        }
        m.lines(&p.body)?;
        for (slot, remembered) in p.remembered.iter().enumerate() {
            if *remembered {
                let past = &mut m.past[slot];
                past.push_back(m.vals[slot]);
                if past.len() > MAX_LOOKBACK {
                    past.pop_front();
                }
            }
        }
    }
    let plots = p.plots.iter().zip(m.plots).map(|(s, values)| Plot { title: s.title.clone(), kind: s.kind, color: s.color, width: s.width, values }).collect();
    let marks = p.marks.iter().zip(m.marks).map(|(s, hits)| Mark { text: s.text.clone(), above: s.above, color: s.color, hits }).collect();
    let fills = p.fills.iter().map(|(upper, lower, color)| Fill { upper: *upper, lower: *lower, color: *color }).collect();
    Ok(Output { own_pane: p.own_pane, plots, fills, marks })
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

    fn lines(&mut self, lines: &[Run]) -> Result<(), Error> {
        for line in lines {
            self.count(1, None)?;
            match line {
                Run::Set(slot, e) => {
                    let v = self.ex(e)?;
                    self.vals[*slot] = v;
                }
                Run::Start(slot, e) => {
                    if self.i == 0 {
                        let v = self.ex(e)?;
                        self.vals[*slot] = v;
                    }
                }
                Run::If(arms, otherwise) => {
                    let mut done = false;
                    for (cond, body) in arms {
                        if truth(self.ex(cond)?) {
                            self.lines(body)?;
                            done = true;
                            break;
                        }
                    }
                    if !done {
                        self.lines(otherwise)?;
                    }
                }
                Run::For(slot, from, to, body, at) => {
                    let (from, to) = (self.ex(from)?.round(), self.ex(to)?.round());
                    if from.is_nan() || to.is_nan() || to < from {
                        continue;
                    }
                    if to - from + 1.0 > MAX_LOOP as f64 {
                        return Err(Error::at(*at, format!("a loop goes round {MAX_LOOP} times at most")));
                    }
                    let mut k = from;
                    while k <= to {
                        self.vals[*slot] = k;
                        self.lines(body)?;
                        k += 1.0;
                    }
                }
                Run::Plot(index, e) => {
                    let v = self.ex(e)?;
                    if let Some(cell) = self.plots.get_mut(*index).and_then(|p| p.get_mut(self.i)) {
                        *cell = v;
                    }
                }
                Run::Mark(index, e) => {
                    let hit = truth(self.ex(e)?);
                    if let Some(cell) = self.marks.get_mut(*index).and_then(|m| m.get_mut(self.i)) {
                        *cell = hit;
                    }
                }
            }
        }
        Ok(())
    }

    fn series(&self, s: Series, i: usize) -> f64 {
        let b = self.bars;
        let at = |xs: &[f64]| xs.get(i).copied().unwrap_or(NA);
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
            Series::Day => (at(b.time) as i64).div_euclid(DAY) as f64,
            Series::Week => week_index(at(b.time) as i64) as f64,
            Series::Month => month_index(at(b.time) as i64) as f64,
            Series::Year => year_of(at(b.time) as i64) as f64,
        }
    }

    fn ex(&mut self, e: &Ex) -> Result<f64, Error> {
        self.count(1, None)?;
        Ok(match e {
            Ex::Num(v) => *v,
            Ex::Slot(s) => self.vals.get(*s).copied().unwrap_or(NA),
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
                    Target::Slot(s) if k == 0 => self.vals.get(*s).copied().unwrap_or(NA),
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
                let mut vals = [NA; 8];
                for (slot, a) in vals.iter_mut().zip(args) {
                    *slot = self.ex(a)?;
                }
                math(*f, &vals[..args.len().min(8)])
            }
            Ex::Keep(site, args) => {
                let a = match args.first() {
                    Some(x) => self.ex(x)?,
                    None => NA,
                };
                let b = match args.get(1) {
                    Some(x) => self.ex(x)?,
                    None => NA,
                };
                let tr = self.series(Series::Tr, self.i);
                let Some(state) = self.states.get_mut(*site) else { return Ok(NA) };
                let (v, steps) = state.step(a, b, tr);
                self.count(steps, None)?;
                v
            }
        })
    }
}

/// Arithmetic gives no value where a side has none, or on a division by zero;
/// a comparison with no value is not true.
fn binary(op: Op, a: f64, b: f64) -> f64 {
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

fn math(f: Math, v: &[f64]) -> f64 {
    let a = v.first().copied().unwrap_or(NA);
    let b = v.get(1).copied();
    match f {
        Math::Abs => a.abs(),
        Math::Sqrt if a < 0.0 => NA,
        Math::Sqrt => a.sqrt(),
        Math::Log if a <= 0.0 => NA,
        Math::Log => a.ln(),
        Math::Exp => a.exp(),
        Math::Pow => a.powf(b.unwrap_or(NA)),
        Math::Round => match b {
            Some(d) if d.is_finite() && (0.0..=10.0).contains(&d) => {
                let scale = 10f64.powi(d as i32);
                (a * scale).round() / scale
            }
            _ => a.round(),
        },
        Math::Floor => a.floor(),
        Math::Ceil => a.ceil(),
        // No value in, no value out: max(na, 0) is not 0.
        Math::Min | Math::Max => {
            if v.iter().any(|x| x.is_nan()) {
                return NA;
            }
            v.iter().copied().reduce(|x, y| if f == Math::Max { x.max(y) } else { x.min(y) }).unwrap_or(NA)
        }
        Math::Clamp => {
            let (lo, hi) = (b.unwrap_or(NA), v.get(2).copied().unwrap_or(NA));
            if a.is_nan() || lo.is_nan() || hi.is_nan() { NA } else { a.max(lo).min(hi) }
        }
        Math::Nz => {
            if a.is_nan() { b.unwrap_or(0.0) } else { a }
        }
        Math::IsNa => flag(a.is_nan()),
    }
}
