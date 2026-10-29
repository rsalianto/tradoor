//! Footprint columns: what was bought and what was sold at every price inside
//! a bar, beside the bar.
//!
//! Read as every footprint chart is read. A bar's slot is a thin candle at its
//! left and a column of cells beside it, one a price level, each carrying its
//! number: the delta, or the two sides apart, or their sum. Every bar's rows
//! sit on ONE price grid, so a level reads straight across the screen; a grid
//! per bar would put neighbouring bars' rows at different prices and there
//! would be nothing to compare.
//!
//! The rows follow the zoom. Left to fit, a row covers the finest round number
//! of the grid's steps at which it stands tall enough to carry its number:
//! zooming in splits rows, zooming out joins them, and a cell is never a
//! hairline nobody can read. Set by the reader, a row covers what they set.
//!
//! A cell is filled against the loudest level of its own bar, so a quiet bar
//! reads as well as the busy one beside it. A cell whose side took far more
//! than the other side took one level away — the diagonal every platform
//! compares, at two and a half times here — is ringed as an imbalance, and the
//! level that traded most as the bar's point of control. Under the bars a
//! table says what each came to: its volume, its delta, that delta as a share
//! of the volume, and the most and least the delta reached on the way.

use crate::paint::{Align, Baseline, Frame, Rect};
use crate::theme::Theme;

/// How a level is shown: the two sides apart, their difference, or their sum.
/// Named as every footprint chart names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Sold on the left, bought on the right.
    Split,
    /// One cell, green where more was bought than sold and red where less.
    #[default]
    Delta,
    /// One cell: everything that traded at the level, whichever way.
    Total,
    /// No numbers: a bar across the cell, as long as the level was busy.
    Cluster,
    /// The delta added up from the foot of the bar, level by level.
    Ladder,
}

impl Mode {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "split" => Self::Split,
            "total" => Self::Total,
            "cluster" => Self::Cluster,
            "ladder" => Self::Ladder,
            _ => Self::Delta,
        }
    }
}

/// What else a column carries, beyond its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub mode: Mode,
    /// The rows that hold most of the bar's volume, outlined.
    pub value_area: bool,
    /// What each bar came to, in a table under the bars.
    pub stats: bool,
    /// Every visible bar summed by price, drawn against the right edge.
    pub profile: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { mode: Mode::default(), value_area: true, stats: true, profile: true }
    }
}

/// A bar's levels — price, coins bought, coins sold, lowest price first — and
/// the most and least its delta reached from the open, read at the close of
/// each of its minutes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bar {
    pub time: i64,
    pub levels: Vec<(f64, f64, f64)>,
    pub max_delta: f64,
    pub min_delta: f64,
}

/// What a cell must have to carry a number, in pixels: the height of its row,
/// and the smallest a number is written at, fitted across it.
const NUMBER_HEIGHT: f32 = 10.0;
const SMALLEST_NUMBER: f32 = 7.0;
/// The bar width a footprint is opened at: room for a candle and a cell of numbers.
pub const READABLE_SPACING: f64 = 90.0;
/// How tall a row is made to stand when the rows are fitted to the zoom: room
/// for a number and the gap between cells.
const READABLE_ROW: f32 = 16.0;
/// A cell is never drawn thinner than this, so a busy bar still reads.
const MIN_ROW: f32 = 1.0;
/// A slot narrower than this is all cells: no room for a candle beside them.
const CANDLE_FROM: f32 = 24.0;
/// How many times the other side a side must take, one level down, to be an
/// imbalance. Exocharts asks two and a half, ATAS one and a half; this is
/// theirs, and the diagonal is everyone's.
const IMBALANCE: f64 = 2.5;
/// A level this quiet against the bar's loudest is noise, whatever its ratio.
const IMBALANCE_FLOOR: f64 = 0.04;
/// Imbalances in a row that make a run worth marking as one.
const STACKED: usize = 3;
/// The share of a bar's volume the value area holds, as every platform has it.
const VALUE_AREA: f64 = 0.7;
/// How wide the profile against the right edge is drawn, as a share of the plot.
const PROFILE_SHARE: f32 = 0.12;
/// The table under the bars, a row a figure. The extremes are read at each
/// minute's close, so they are near the true ones, not the true ones.
pub const TABLE_ROWS: [&str; 5] = ["Volume", "Delta", "Delta %", "Max Δ ≈", "Min Δ ≈"];
/// How tall one row of that table stands.
pub const TABLE_ROW: f32 = 15.0;
/// The column the table's names sit in, at its left.
const TABLE_NAMES: f32 = 64.0;

/// How tall the table under the bars stands, which the price scale keeps clear.
pub fn table_height() -> f32 {
    TABLE_ROWS.len() as f32 * TABLE_ROW
}

/// What the chart lends the painter: where things go, and what they look like.
pub struct View<'a> {
    pub plot: Rect,
    pub bar_spacing: f32,
    pub char_width: f32,
    pub theme: &'a Theme,
    /// Logical bar index to x, and price to y.
    pub x: &'a dyn Fn(f64) -> f32,
    pub y: &'a dyn Fn(f64) -> f32,
}

/// Where a bar's parts sit across its slot: the candle at the left, as narrow
/// as still reads, and the cells beside it. Too narrow for both, the cells
/// take it all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub left: f32,
    pub width: f32,
    /// The candle's middle, and its body's width: 0 where there is none.
    pub candle: f32,
    pub candle_width: f32,
    pub cells: f32,
    pub cells_width: f32,
}

pub fn slot(center: f32, bar_spacing: f32) -> Slot {
    let width = (bar_spacing * 0.86).max(2.0);
    let left = center - width / 2.0;
    if width < CANDLE_FROM {
        return Slot { left, width, candle: center, candle_width: 0.0, cells: left, cells_width: width };
    }
    let candle_width = (width * 0.1).clamp(3.0, 9.0).round();
    let gap = 2.0;
    Slot { left, width, candle: left + candle_width / 2.0, candle_width, cells: left + candle_width + gap, cells_width: width - candle_width - gap }
}

/// The price one row covers. Fitted, it is the finest round number of `grid`'s
/// steps — 1, 2, 5, 10, 20, 25, 50… of them — at which a row stands
/// `READABLE_ROW` tall where `at` sits; not fitted, it is `grid` as sent.
pub fn row_step(y: &dyn Fn(f64) -> f32, at: f64, grid: f64, fitted: bool) -> f64 {
    if !fitted || grid <= 0.0 {
        return grid;
    }
    let tall = |step: f64| (y(at) - y(at + step)).abs();
    let mut power = 1.0;
    while power < 1e7 {
        // Twenty-five once there are tens of steps to a row: still whole steps, and
        // the gap between twenty and fifty is too wide to jump.
        for head in [1.0, 2.0, 2.5, 5.0].into_iter().filter(|h| *h != 2.5 || power >= 10.0) {
            let step = grid * head * power;
            if tall(step) >= READABLE_ROW {
                return step;
            }
        }
        power *= 10.0;
    }
    grid * power
}

/// A bar's levels added up onto rows of `step`, lowest first.
pub fn grouped(bar: &Bar, step: f64) -> Bar {
    let mut levels: Vec<(f64, f64, f64)> = Vec::with_capacity(bar.levels.len());
    for (price, buy, sell) in &bar.levels {
        // A hair over, so a level that is a whole number of steps lands on its own row.
        let at = (price / step + 1e-9).floor() * step;
        match levels.last_mut() {
            Some(row) if (row.0 - at).abs() < step * 1e-6 => {
                row.1 += buy;
                row.2 += sell;
            }
            _ => levels.push((at, *buy, *sell)),
        }
    }
    Bar { levels, ..bar.clone() }
}

/// How a figure is written inside a cell: three figures at most, because the
/// room is short — and keeping its sign, since a delta that lost its minus
/// says the opposite.
fn short(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    if a < 0.001 {
        return "0".into();
    }
    let (n, unit) = match a {
        a if a >= 999_500.0 => (a / 1_000_000.0, "M"),
        a if a >= 999.5 => (a / 1_000.0, "K"),
        a => (a, ""),
    };
    let decimals = match n {
        n if n >= 99.95 => 0,
        n if n >= 9.995 => 1,
        n if n >= 0.01 => 2,
        _ => 3,
    };
    format!("{sign}{n:.decimals$}{unit}")
}

/// A delta, with the sign either way.
fn signed(v: f64) -> String {
    if v > 0.0 { format!("+{}", short(v)) } else { short(v) }
}

/// The bars in view, each at its logical index, their levels put onto rows of
/// `step` — the same for every bar on screen.
pub fn paint(f: &mut Frame, view: &View<'_>, bars: &[(f64, &Bar)], options: Options, step: f64) {
    let t = view.theme;
    let mode = options.mode;
    let weight = |buy: f64, sell: f64| match mode {
        Mode::Split => buy.max(sell),
        Mode::Delta | Mode::Ladder => (buy - sell).abs(),
        Mode::Total | Mode::Cluster => buy + sell,
    };
    // Only the bars in view, each put onto the screen's rows.
    let rows: Vec<(f32, Bar)> = bars
        .iter()
        .filter_map(|(logical, bar)| {
            let x = (view.x)(*logical);
            let s = slot(x, view.bar_spacing);
            if bar.levels.is_empty() || s.left + s.width < view.plot.x || s.left > view.plot.right() {
                return None;
            }
            let step = if step > 0.0 { step } else { level_step(bar) };
            Some((x, grouped(bar, step)))
        })
        .collect();
    if !rows.iter().flat_map(|(_, bar)| bar.levels.iter()).any(|(_, buy, sell)| weight(*buy, *sell) > 0.0) {
        return;
    }

    for (x, bar) in &rows {
        // A cell is filled against the loudest level of ITS OWN bar. Against
        // the loudest on screen, a busy minute an hour ago left every level of
        // every quiet bar the same near-invisible wash.
        let loudest = bar.levels.iter().map(|(_, buy, sell)| weight(*buy, *sell)).fold(0.0_f64, f64::max);
        if loudest <= 0.0 {
            continue;
        }
        let s = slot(*x, view.bar_spacing);
        let step = if step > 0.0 { step } else { level_step(bar) };
        let row = row_height(view, bar.levels[bar.levels.len() / 2].0, step);
        let half = s.cells_width / 2.0;
        let pad = 3.0;
        // A number as big as its row allows and small enough to fit across its cell, or none.
        let fit = |text: &str, room: f32| fitted(view, text, room, row);
        let top_of = |price: f64| (view.y)(price + step);
        let full_at = |price: f64| row_height(view, price, step);
        // A pixel between rows once there is room for one, so each cell reads as its own.
        let drawn_at = |price: f64| {
            let full = full_at(price);
            (full - if full >= 6.0 { 1.0 } else { 0.5 }).max(MIN_ROW)
        };
        let on_screen = |price: f64| top_of(price) + full_at(price) >= view.plot.y && top_of(price) <= view.plot.bottom();

        // The delta added up from the foot of the bar, for the ladder.
        let mut running = 0.0;
        for (price, buy, sell) in &bar.levels {
            running += buy - sell;
            if !on_screen(*price) {
                continue;
            }
            let (top, drawn) = (top_of(*price), drawn_at(*price));
            let alpha = |v: f64| (55.0 + 200.0 * (v / loudest).clamp(0.0, 1.0).sqrt() as f32) as u8;
            // A number over a filled cell has to fight the fill: over a strong
            // one the background colour reads, over a faint one the text
            // colour does. Grey over both reads over neither.
            let ink = |fill: u8| if fill >= 150 { t.background } else { t.text };
            let side = if buy >= sell { t.bull } else { t.bear };
            let middle = top + drawn / 2.0;
            match mode {
                Mode::Split => {
                    f.rect(s.cells, top, half - 0.5, drawn, t.bear.with_alpha(alpha(*sell)));
                    f.rect(s.cells + half + 0.5, top, half - 0.5, drawn, t.bull.with_alpha(alpha(*buy)));
                    let (sold, bought) = (short(*sell), short(*buy));
                    if let Some(size) = fit(&sold, half - pad) {
                        f.text(s.cells + half - pad, middle, sold, ink(alpha(*sell)), Align::Right, Baseline::Middle, size, None, Some(view.plot));
                    }
                    if let Some(size) = fit(&bought, half - pad) {
                        f.text(s.cells + half + pad, middle, bought, ink(alpha(*buy)), Align::Left, Baseline::Middle, size, None, Some(view.plot));
                    }
                }
                Mode::Cluster => {
                    // No numbers: the length of the bar is the reading, which
                    // is what a chart zoomed out has room to say.
                    let share = ((buy + sell) / loudest).clamp(0.0, 1.0) as f32;
                    f.rect(s.cells, top, s.cells_width * share, drawn, side.with_alpha(alpha(buy + sell)));
                }
                Mode::Delta | Mode::Total | Mode::Ladder => {
                    let value = match mode {
                        Mode::Total => buy + sell,
                        Mode::Ladder => running,
                        _ => buy - sell,
                    };
                    let color = if mode == Mode::Total { side } else if value >= 0.0 { t.bull } else { t.bear };
                    f.rect(s.cells, top, s.cells_width, drawn, color.with_alpha(alpha(value.abs())));
                    let text = if mode == Mode::Total { short(value) } else { signed(value) };
                    if let Some(size) = fit(&text, s.cells_width) {
                        f.text(s.cells + s.cells_width / 2.0, middle, text, ink(alpha(value.abs())), Align::Center, Baseline::Middle, size, None, Some(view.plot));
                    }
                }
            }
        }

        // A side that took far more than the other side took one level away.
        // The diagonal, because a buyer lifts the offer where a seller hits the
        // bid a level beneath it.
        let mut bought_runs = vec![false; bar.levels.len()];
        let mut sold_runs = vec![false; bar.levels.len()];
        if row >= 2.0 {
            let at = |price: f64| bar.levels.iter().find(|(p, _, _)| (p - price).abs() < step / 2.0);
            for (i, (price, buy, sell)) in bar.levels.iter().enumerate() {
                let (top, drawn) = (top_of(*price), drawn_at(*price));
                let below_sell = at(*price - step).map_or(0.0, |(_, _, s)| *s);
                let above_buy = at(*price + step).map_or(0.0, |(_, b, _)| *b);
                let (bought_into, sold_into) = imbalanced(*buy, *sell, below_sell, above_buy, loudest);
                bought_runs[i] = bought_into;
                sold_runs[i] = sold_into;
                if !on_screen(*price) {
                    continue;
                }
                let (bought_x, sold_x, w) = if mode == Mode::Split { (s.cells + half, s.cells, half) } else { (s.cells, s.cells, s.cells_width) };
                if bought_into {
                    f.stroke_rect(bought_x, top, w, drawn, 1.0, t.bull);
                }
                if sold_into {
                    f.stroke_rect(sold_x, top, w, drawn, 1.0, t.bear);
                }
            }
        }

        // Three imbalances in a row or more: a band of one-sided aggression,
        // which is what an order-flow reader is actually looking for. Marked
        // beside the cells so the run reads as one thing.
        for (side, runs) in [(t.bull, stacked(&bought_runs)), (t.bear, stacked(&sold_runs))] {
            for (from, to) in runs {
                let top = top_of(bar.levels[to].0);
                let bottom = top_of(bar.levels[from].0) + drawn_at(bar.levels[from].0);
                let at_x = if side == t.bull { s.cells + s.cells_width + 1.0 } else { s.cells - 2.0 };
                f.rect(at_x, top, 2.0, bottom - top, side);
            }
        }

        // The rows holding most of the bar's volume, and the one that held most.
        if options.value_area {
            if let Some((low, high)) = value_area(&bar.levels) {
                let top = top_of(bar.levels[high].0);
                let bottom = top_of(bar.levels[low].0) + drawn_at(bar.levels[low].0);
                f.stroke_rect(s.cells, top, s.cells_width, bottom - top, 1.0, t.text.with_alpha(70));
            }
        }
        if let Some(index) = bar.levels.iter().enumerate().max_by(|a, b| (a.1 .1 + a.1 .2).total_cmp(&(b.1 .1 + b.1 .2))).map(|(i, _)| i) {
            let price = bar.levels[index].0;
            f.stroke_rect(s.cells, top_of(price), s.cells_width, drawn_at(price), 1.0, t.text.with_alpha(150));
        }
    }

    // Every visible bar summed by price, against the right edge: where the
    // screen as a whole traded, which no single column can say.
    if options.profile && step > 0.0 {
        let mut summed: Vec<(f64, f64, f64)> = Vec::new();
        for (_, bar) in &rows {
            for (price, buy, sell) in &bar.levels {
                match summed.iter_mut().find(|(p, _, _)| (p - price).abs() < step / 2.0) {
                    Some(row) => {
                        row.1 += buy;
                        row.2 += sell;
                    }
                    None => summed.push((*price, *buy, *sell)),
                }
            }
        }
        let loudest = summed.iter().map(|(_, buy, sell)| buy + sell).fold(0.0_f64, f64::max);
        let room = view.plot.w * PROFILE_SHARE;
        for (price, buy, sell) in &summed {
            let (top, drawn) = ((view.y)(price + step), (row_height(view, *price, step) - 0.5).max(MIN_ROW));
            if top + drawn < view.plot.y || top > view.plot.bottom() || loudest <= 0.0 {
                continue;
            }
            let w = (room * ((buy + sell) / loudest) as f32).max(1.0);
            let color = if buy >= sell { t.bull } else { t.bear };
            f.rect(view.plot.right() - w, top, w, drawn, color.with_alpha(90));
        }
    }

    if options.stats {
        table(f, view, &rows);
    }
}

/// The size a number is written at in a cell `room` wide in a row `row` tall:
/// as big as the row allows, small enough to fit across — or none, where it
/// would be too small to read.
fn fitted(view: &View<'_>, text: &str, room: f32, row: f32) -> Option<f32> {
    // The font's character width for its size, as the chart measured it.
    let per_px = if view.theme.font_px > 0.0 { view.char_width / view.theme.font_px } else { 0.6 };
    let size = (row * 0.62).min(11.0).min((room - 4.0) / (text.chars().count().max(1) as f32 * per_px));
    (row >= NUMBER_HEIGHT && size >= SMALLEST_NUMBER).then_some(size)
}

/// What each bar came to, in a table along the foot of the plot: a row a
/// figure, a cell a bar, under the bar it belongs to. Each row is shaded
/// against its own loudest on screen, so a big bar stands out of its row.
fn table(f: &mut Frame, view: &View<'_>, rows: &[(f32, Bar)]) {
    let t = view.theme;
    let top = view.plot.bottom() - table_height();
    f.rect(view.plot.x, top, view.plot.w, table_height(), t.background);
    f.hline(view.plot.x, view.plot.right(), top, 1.0, t.border);
    // A bar's figures: volume, delta, delta %, the most and the least the delta reached.
    // None for a bar under the names: their column holds it, and numbers are written over every fill.
    let names_end = view.plot.x + TABLE_NAMES;
    let figures: Vec<(f32, [f64; 5])> = rows
        .iter()
        .filter(|(x, _)| slot(*x, view.bar_spacing).left >= names_end)
        .map(|(x, bar)| {
            let (bought, sold) = bar.levels.iter().fold((0.0, 0.0), |(b, s), (_, buy, sell)| (b + buy, s + sell));
            let volume = bought + sold;
            let delta = bought - sold;
            let share = if volume > 0.0 { delta / volume * 100.0 } else { 0.0 };
            (*x, [volume, delta, share, bar.max_delta.max(0.0), bar.min_delta.min(0.0)])
        })
        .collect();
    let size = 9.5;
    for (r, _) in TABLE_ROWS.iter().enumerate() {
        let loudest = figures.iter().map(|(_, v)| v[r].abs()).fold(0.0_f64, f64::max);
        let y = top + r as f32 * TABLE_ROW;
        for (x, values) in &figures {
            let s = slot(*x, view.bar_spacing);
            let v = values[r];
            let strength = if loudest > 0.0 { (v.abs() / loudest).clamp(0.0, 1.0).sqrt() } else { 0.0 };
            let color = if r == 0 { t.text } else if v >= 0.0 { t.bull } else { t.bear };
            let alpha = (if r == 0 { 12.0 + 60.0 * strength } else { 25.0 + 160.0 * strength }) as u8;
            f.rect(s.left, y + 0.5, s.width, TABLE_ROW - 1.0, color.with_alpha(alpha));
            let text = match r {
                0 => short(v),
                2 => format!("{}{:.0}%", if v > 0.0 { "+" } else { "" }, v),
                _ => signed(v),
            };
            if let Some(size) = fitted(view, &text, s.width, TABLE_ROW).map(|s| s.min(size)) {
                let ink = if alpha >= 150 { t.background } else { t.text };
                f.text(s.left + s.width / 2.0, y + TABLE_ROW / 2.0, text, ink, Align::Center, Baseline::Middle, size, None, Some(view.plot));
            }
        }
    }
    // The names, in a column of their own at the left.
    for (r, name) in TABLE_ROWS.iter().enumerate() {
        f.text(view.plot.x + 6.0, top + r as f32 * TABLE_ROW + TABLE_ROW / 2.0, (*name).to_string(), t.text.with_alpha(150), Align::Left, Baseline::Middle, size, None, Some(view.plot));
    }
}

/// The rows holding `VALUE_AREA` of a bar's volume, taken from the busiest
/// level outwards, a neighbour at a time — the way every platform builds it.
/// Returns the lowest and highest row of that band.
fn value_area(levels: &[(f64, f64, f64)]) -> Option<(usize, usize)> {
    let total: f64 = levels.iter().map(|(_, buy, sell)| buy + sell).sum();
    if total <= 0.0 {
        return None;
    }
    let at = |i: usize| levels[i].1 + levels[i].2;
    let poc = (0..levels.len()).max_by(|a, b| at(*a).total_cmp(&at(*b)))?;
    let (mut low, mut high, mut held) = (poc, poc, at(poc));
    while held < total * VALUE_AREA && (low > 0 || high + 1 < levels.len()) {
        let below = if low > 0 { at(low - 1) } else { f64::MIN };
        let above = if high + 1 < levels.len() { at(high + 1) } else { f64::MIN };
        if above >= below {
            high += 1;
            held += above;
        } else {
            low -= 1;
            held += below;
        }
    }
    Some((low, high))
}

/// Runs of `STACKED` or more imbalances on the same side, as index ranges.
fn stacked(flags: &[bool]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (i, on) in flags.iter().chain(std::iter::once(&false)).enumerate() {
        match (*on, start) {
            (true, None) => start = Some(i),
            (false, Some(from)) => {
                if i - from >= STACKED {
                    runs.push((from, i - 1));
                }
                start = None;
            }
            _ => {}
        }
    }
    runs
}

/// Whether a side of a level took far more than the other side took one level
/// away — the diagonal, because a buyer lifts the offer where a seller hits the
/// bid beneath it. A level too quiet against the bar's loudest says nothing,
/// whatever its ratio: one coin against a tenth of one is not aggression.
fn imbalanced(buy: f64, sell: f64, below_sell: f64, above_buy: f64, loudest: f64) -> (bool, bool) {
    let loud = |v: f64| v >= loudest * IMBALANCE_FLOOR;
    let over = |side: f64, diagonal: f64| side >= diagonal * IMBALANCE && side > 0.0;
    (loud(buy) && over(buy, below_sell), loud(sell) && over(sell, above_buy))
}

/// How tall one level stands on screen, measured at the price it sits at.
fn row_height(view: &View<'_>, price: f64, step: f64) -> f32 {
    ((view.y)(price) - (view.y)(price + step)).abs().max(MIN_ROW)
}

/// The gap between levels, which is the market's own step wherever two levels
/// are neighbours — a bar that traded at four prices in a row says so itself.
pub(crate) fn level_step(bar: &Bar) -> f64 {
    let mut step = f64::MAX;
    for pair in bar.levels.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        if gap > 0.0 && gap < step {
            step = gap;
        }
    }
    if step == f64::MAX {
        1.0
    } else {
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> Bar {
        Bar { time: 0, levels: vec![(100.0, 1.0, 4.0), (101.0, 2.0, 1.0), (102.0, 8.0, 0.5)], ..Default::default() }
    }

    #[test]
    fn a_number_is_short_enough_for_the_cell_it_sits_in() {
        // Three figures, whatever the size: 1,240 is 1.24K, not a bare 1K.
        assert_eq!(short(1_240.0), "1.24K");
        assert_eq!(short(12_400.0), "12.4K");
        assert_eq!(short(124_000.0), "124K");
        assert_eq!(short(1_240_000.0), "1.24M");
        assert_eq!(short(999.6), "1.00K", "rounding up into the next unit writes it in that unit");
        assert_eq!(short(142.0), "142");
        assert_eq!(short(14.2), "14.2");
        assert_eq!(short(4.25), "4.25");
        assert_eq!(short(0.37), "0.37");
        assert_eq!(short(0.004), "0.004", "a few thousandths of a coin still says something");
        assert_eq!(short(0.0004), "0");
        assert_eq!(short(-3.0), "-3.00", "a delta that lost its minus would say the opposite");
        assert_eq!(signed(3.0), "+3.00");
    }

    #[test]
    fn the_step_between_levels_is_the_market_s_own() {
        assert!((level_step(&bar()) - 1.0).abs() < f64::EPSILON);
        assert!((level_step(&Bar { time: 0, levels: vec![(100.0, 1.0, 1.0), (100.5, 1.0, 1.0)], ..Default::default() }) - 0.5).abs() < f64::EPSILON);
        assert!((level_step(&Bar { time: 0, levels: vec![(100.0, 1.0, 1.0)], ..Default::default() }) - 1.0).abs() < f64::EPSILON, "one level alone has no gap to measure");
    }

    /// A cell is as tall as its own level is on screen — never as tall as the
    /// chart. Measuring the step from price zero said otherwise on a scale
    /// where zero is nowhere near, and painted a bar over the whole plot.
    #[test]
    fn a_cell_is_as_tall_as_its_level_even_where_zero_is_nowhere_near() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let mut f = Frame::default();
        f.begin(400.0, 300.0, 1.0, theme.background);
        let x = |l: f64| 200.0 + l as f32;
        // A logarithmic scale over prices around 84,000, as a chart of BTC has.
        let y = |p: f64| 300.0 - ((p.ln() - 84_000f64.ln()) * 100_000.0) as f32;
        let bar = Bar { time: 0, levels: vec![(84_000.0, 1.0, 2.0), (84_001.0, 3.0, 1.0), (84_002.0, 2.0, 2.0)], ..Default::default() };
        let view = View { plot, bar_spacing: 30.0, char_width: 6.0, theme: &theme, x: &x, y: &y };
        paint(&mut f, &view, &[(0.0, &bar)], Options { mode: Mode::Split, value_area: false, stats: false, profile: false }, 1.0);
        // A dollar at 84,000 is about a pixel on this scale, so every vertex
        // sits within a few pixels of the levels — not spread over the plot.
        let ys: Vec<f32> = f.positions.chunks_exact(2).map(|p| p[1]).collect();
        let (top, bottom) = (ys.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MIN, f32::max));
        assert!(bottom - top < 20.0, "the column stands where its levels are, not over the whole chart: {top} to {bottom}");
    }

    #[test]
    fn a_minute_that_traded_at_one_price_is_still_drawn() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let mut f = Frame::default();
        f.begin(400.0, 300.0, 1.0, theme.background);
        let x = |l: f64| 200.0 + l as f32;
        let y = |p: f64| 300.0 - (p as f32 - 99.0) * 100.0;
        let view = View { plot, bar_spacing: 30.0, char_width: 6.0, theme: &theme, x: &x, y: &y };
        // One row, because the grid the screen shares is coarser than the
        // minute's own range. It is still a minute that traded.
        let bar = Bar { time: 0, levels: vec![(100.0, 2.0, 1.0)], ..Default::default() };
        paint(&mut f, &view, &[(0.0, &bar)], Options { mode: Mode::Split, value_area: false, stats: false, profile: false }, 1.0);
        assert!(f.vertex_count() > 0, "a hole where there was trade is worse than a coarse row");
    }

    #[test]
    fn a_number_takes_the_colour_that_reads_over_its_own_cell() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let mut f = Frame::default();
        f.begin(400.0, 300.0, 1.0, theme.background);
        let x = |l: f64| 200.0 + l as f32;
        let y = |p: f64| 300.0 - (p as f32 - 99.0) * 100.0;
        let view = View { plot, bar_spacing: 80.0, char_width: 6.0, theme: &theme, x: &x, y: &y };
        // One level traded a hundred coins, the next a fiftieth of that.
        let bar = Bar { time: 0, levels: vec![(100.0, 100.0, 0.02), (101.0, 0.02, 0.02)], ..Default::default() };
        paint(&mut f, &view, &[(0.0, &bar)], Options { mode: Mode::Split, value_area: false, stats: false, profile: false }, 1.0);
        let colour_of = |text: &str| f.texts.iter().find(|t| t.text == text).map(|t| t.color);
        assert_eq!(colour_of("100"), Some(theme.background), "over a filled cell, the background colour reads");
        assert_eq!(colour_of("0.02"), Some(theme.text), "over a faint one, the text colour does");
    }

    #[test]
    fn the_value_area_grows_from_the_busiest_level_outwards() {
        // A hundred coins: 3, 25, 50, 15, 7. The busiest row holds fifty, and
        // the seventy is reached by taking the busier neighbour, beneath it.
        let levels = vec![(100.0, 2.0, 1.0), (101.0, 13.0, 12.0), (102.0, 25.0, 25.0), (103.0, 8.0, 7.0), (104.0, 4.0, 3.0)];
        assert_eq!(value_area(&levels), Some((1, 2)), "the point of control and the busier side of it");
        assert_eq!(value_area(&[(100.0, 1.0, 1.0)]), Some((0, 0)), "one level is its own value area");
        assert_eq!(value_area(&[(100.0, 0.0, 0.0)]), None, "a bar that traded nothing has none");
    }

    #[test]
    fn three_imbalances_in_a_row_are_a_run_and_two_are_not() {
        assert_eq!(stacked(&[false, true, true, true, false]), vec![(1, 3)]);
        assert_eq!(stacked(&[true, true, false, true, true]), vec![], "two is not a stack");
        assert_eq!(stacked(&[true, true, true, true]), vec![(0, 3)], "and a run to the end is still a run");
        assert_eq!(stacked(&[]), vec![]);
    }

    #[test]
    fn a_side_that_took_far_more_than_the_diagonal_is_an_imbalance() {
        // Ten bought against one sold a level beneath: the buyers were paying up.
        assert_eq!(imbalanced(10.0, 1.0, 1.0, 1.0, 10.0), (true, false));
        // The same on the other side of the book.
        assert_eq!(imbalanced(1.0, 10.0, 1.0, 1.0, 10.0), (false, true));
        // Evenly matched, whichever way it is read.
        assert_eq!(imbalanced(1.0, 1.0, 1.0, 1.0, 10.0), (false, false));
        // A tenth of a coin against a hundredth is ten times as much and still
        // nothing at all, on a bar that traded ten.
        assert_eq!(imbalanced(0.1, 0.0, 0.01, 0.0, 10.0), (false, false));
        // Nothing on the diagonal at all, and a side worth noticing: an
        // imbalance, since the other side was not there to take it.
        assert_eq!(imbalanced(5.0, 0.0, 0.0, 0.0, 10.0), (true, false));
    }

    #[test]
    fn a_quiet_bar_is_as_legible_as_the_busy_one_beside_it() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let mut f = Frame::default();
        f.begin(400.0, 300.0, 1.0, theme.background);
        let x = |l: f64| 100.0 + l as f32 * 100.0;
        let y = |p: f64| 300.0 - (p as f32 - 99.0) * 100.0;
        let view = View { plot, bar_spacing: 60.0, char_width: 6.0, theme: &theme, x: &x, y: &y };
        // One bar trades in hundreds, the next in tenths.
        let busy = Bar { time: 0, levels: vec![(100.0, 400.0, 100.0), (101.0, 50.0, 200.0)], ..Default::default() };
        let quiet = Bar { time: 60_000, levels: vec![(100.0, 0.4, 0.1), (101.0, 0.05, 0.2)], ..Default::default() };
        paint(&mut f, &view, &[(0.0, &busy), (1.0, &quiet)], Options { mode: Mode::Split, value_area: false, stats: false, profile: false }, 1.0);
        // The strongest cell of each bar carries the same weight on screen.
        let alphas: Vec<u8> = f.colors.chunks_exact(4).map(|c| c[3]).collect();
        let strongest = *alphas.iter().max().unwrap();
        let faintest = *alphas.iter().min().unwrap();
        assert!(strongest > 240, "the loudest level of a bar is filled: {strongest}");
        assert!(faintest >= 55, "and the quietest is still visible, not a wash: {faintest}");
        assert_eq!(alphas.iter().filter(|a| **a > 240).count() % 6, 0, "each bar has its own loudest, filled the same");
    }

    #[test]
    fn each_mode_writes_what_it_is_about() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let wrote = |mode: Mode| {
            let mut f = Frame::default();
            f.begin(400.0, 300.0, 1.0, theme.background);
            let x = |l: f64| 200.0 + l as f32;
            let y = |p: f64| 300.0 - (p as f32 - 99.0) * 100.0;
            let view = View { plot, bar_spacing: 80.0, char_width: 6.0, theme: &theme, x: &x, y: &y };
            paint(&mut f, &view, &[(0.0, &bar())], Options { mode, value_area: false, stats: false, profile: false }, 1.0);
            f.texts.iter().map(|t| t.text.clone()).collect::<Vec<_>>()
        };
        // The lowest level was 1 bought against 4 sold.
        assert!(wrote(Mode::Split).contains(&"4.00".to_string()) && wrote(Mode::Split).contains(&"1.00".to_string()), "both sides of it");
        assert!(wrote(Mode::Delta).contains(&"-3.00".to_string()), "the difference, and which way it went");
        assert!(wrote(Mode::Total).contains(&"5.00".to_string()), "everything that traded there");
    }

    #[test]
    fn a_wide_bar_carries_its_numbers_and_a_narrow_one_only_its_cells() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let paint_at = |spacing: f32| {
            let mut f = Frame::default();
            f.begin(400.0, 300.0, 1.0, theme.background);
            let x = |l: f64| 200.0 + l as f32;
            // A hundred pixels to the price, so the rows are tall.
            let y = |p: f64| 300.0 - (p as f32 - 99.0) * 100.0;
            let view = View { plot, bar_spacing: spacing, char_width: 6.0, theme: &theme, x: &x, y: &y };
            paint(&mut f, &view, &[(0.0, &bar())], Options { mode: Mode::Split, value_area: false, stats: false, profile: false }, 1.0);
            f
        };
        let wide = paint_at(80.0);
        assert!(wide.texts.iter().any(|t| t.text == "8.00"), "the loudest level writes what it was");
        assert!(wide.texts.len() >= 6, "both sides of every level: {:?}", wide.texts.len());

        let narrow = paint_at(6.0);
        assert!(narrow.texts.is_empty(), "no room for numbers, so none are drawn");
        assert!(narrow.vertex_count() > 0, "the cells are still there");
    }

    #[test]
    fn rows_join_onto_round_steps_and_keep_every_coin() {
        let bar = Bar { time: 0, levels: vec![(100.0, 1.0, 2.0), (101.0, 3.0, 0.5), (102.0, 0.25, 4.0), (103.0, 1.0, 1.0)], max_delta: 2.0, min_delta: -3.0 };
        let joined = grouped(&bar, 2.0);
        assert_eq!(joined.levels, vec![(100.0, 4.0, 2.5), (102.0, 1.25, 5.0)]);
        assert_eq!((joined.max_delta, joined.min_delta), (2.0, -3.0), "the bar's own figures go with it");
    }

    #[test]
    fn fitted_rows_are_the_finest_round_grouping_that_reads() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // Ten pixels to the dollar: one dollar is ten, two are twenty.
        let y = |p: f64| 1000.0 - p as f32 * 10.0;
        assert!(near(row_step(&y, 100.0, 1.0, true), 2.0));
        assert!(near(row_step(&y, 100.0, 0.1, true), 2.0), "from a finer grid, the same round step");
        // Half a pixel to the dollar: fifty a row.
        let y = |p: f64| 1000.0 - p as f32 * 0.5;
        assert!(near(row_step(&y, 100.0, 1.0, true), 50.0));
        // Two thirds of a pixel: twenty is thirteen pixels, twenty-five is the row.
        let y = |p: f64| 1000.0 - p as f32 * 0.65;
        assert!(near(row_step(&y, 100.0, 1.0, true), 25.0));
        // Never two and a half steps: a row is whole levels.
        let y = |p: f64| 1000.0 - p as f32 * 7.0;
        assert!(near(row_step(&y, 100.0, 1.0, true), 5.0));
        assert!(near(row_step(&y, 100.0, 1.0, false), 1.0), "set by the reader, the grid stands");
    }

    #[test]
    fn a_wide_slot_has_a_thin_candle_at_its_left_and_the_cells_beside_it() {
        let wide = slot(100.0, 80.0);
        assert!(wide.candle_width >= 3.0 && wide.candle < wide.cells, "the candle first: {wide:?}");
        assert!(wide.cells + wide.cells_width <= wide.left + wide.width + 0.01, "the cells within the slot: {wide:?}");
        let narrow = slot(100.0, 10.0);
        assert_eq!(narrow.candle_width, 0.0, "too narrow for both, the cells take it all");
        assert_eq!((narrow.cells, narrow.cells_width), (narrow.left, narrow.width));
    }

    #[test]
    fn the_table_under_the_bars_says_what_each_came_to() {
        let theme = Theme::default();
        let plot = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let mut f = Frame::default();
        f.begin(400.0, 300.0, 1.0, theme.background);
        let x = |l: f64| 200.0 + l as f32;
        let y = |p: f64| 200.0 - (p as f32 - 99.0) * 40.0;
        let view = View { plot, bar_spacing: 80.0, char_width: 6.0, theme: &theme, x: &x, y: &y };
        // Eleven bought and five and a half sold; on the way the delta reached six and fell to minus one.
        let bar = Bar { max_delta: 6.0, min_delta: -1.0, ..bar() };
        paint(&mut f, &view, &[(0.0, &bar)], Options { mode: Mode::Delta, value_area: false, stats: true, profile: false }, 1.0);
        let texts: Vec<&str> = f.texts.iter().map(|t| t.text.as_str()).collect();
        for name in TABLE_ROWS {
            assert!(texts.contains(&name), "the row {name} is named: {texts:?}");
        }
        for figure in ["16.5", "+5.50", "+33%", "+6.00", "-1.00"] {
            assert!(texts.contains(&figure), "{figure} is written: {texts:?}");
        }
    }
}
