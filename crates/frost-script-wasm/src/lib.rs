//! Frost Script in the browser, behind the plainest interface there is: the page
//! writes bytes and numbers into this module's memory, calls one function, and
//! reads bytes and numbers back. The module imports nothing — it cannot call
//! out to the page, the network or anything else — so a formula run here can
//! only give back what the formula crate makes: lines of text and numbers.
//!
//! The text is a line a part, fields split by tabs, whole numbers only — a colour
//! is its four bytes as one, `-` none — and the numbers that go with each part
//! follow in the values, in the same order:
//!
//! ```text
//! pane    1
//! plot    line  4278190335  15  0  0  0  RSI    kind, colour, width in tenths, style, offset, coloured a bar each, title
//!                                                 + a value a bar, then a colour a bar where it is coloured so
//! fill    0     2    4278190335                  upper, lower, colour
//! mark    0     4278190335   Buy                 above, colour, text      + 1 or 0 a bar
//! bg                                              a colour a bar
//! bars                                            a colour a bar
//! line    4278190335  10  0  0                   colour, width in tenths, style, extend   + x1 y1 x2 y2
//! label   4278190335  0  0  1  0  HH␟+2%         colour, text colour, place, size, plain, text  + x y
//! box     4278190335  4278190335  10             fill, border, width in tenths   + x1 y1 x2 y2
//! table   0     2                                 down, across
//! cell    0     1     4278190335  0  Avg rise     column, row, colour, background, text
//! error   3     12    ema(…) …                    line, column, message
//! ```
//!
//! A text's line breaks are written as ␟ (0x1F), and its tabs as spaces.
//!
//! `fx_backtest` runs a strategy: its drawing as `fx_run` gives it, and its
//! report as numbers alone, read with `fx_report_ptr` and `fx_report_len` —
//! eight to start with (trades, bars, bars decided as stopped, bars the lower
//! timeframe decided, orders skipped, bars held, the order waiting for the next
//! open — 1 long, -1 short, 2 an exit — and its limit price), then
//! `TRADE` a trade (see `trade`), the equity a bar, and the bars decided as stopped.
use std::cell::RefCell;

use tradoor_frost_script::{backtest, describe, run, Bars, Error, Exit, Funding, Inner, InputKind, Output, PlotKind, PlotStyle, Report, Settings, Size, Trade};

#[derive(Default)]
struct Out {
    text: String,
    values: Vec<f64>,
}

thread_local! {
    static OUT: RefCell<Out> = RefCell::new(Out::default());
    static REPORT: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

/// Numbers a trade is written as.
pub const TRADE: usize = 17;

/// A text on one field of a line: no tab, line break or other control character
/// of a script's can end the field, or the line, or split a list of choices.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// A text that keeps its line breaks, as ␟, and no other control character.
fn multiline(s: &str) -> String {
    s.chars().map(|c| if c == '\n' { '\u{1f}' } else if c.is_control() { ' ' } else { c }).collect()
}

fn colour(c: Option<u32>) -> String {
    c.map_or_else(|| "-".to_string(), |c| c.to_string())
}

fn tenths(w: f32) -> u32 {
    (w * 10.0).round() as u32
}

fn style(s: PlotStyle) -> u8 {
    match s {
        PlotStyle::Line => 0,
        PlotStyle::Dashed => 1,
        PlotStyle::Dotted => 2,
        PlotStyle::Steps => 3,
        PlotStyle::Dots => 4,
    }
}

fn input_kind(k: InputKind) -> &'static str {
    match k {
        InputKind::Number => "number",
        InputKind::Bool => "bool",
        InputKind::Colour => "colour",
        InputKind::Choice => "choice",
        InputKind::Time => "time",
    }
}

fn kind(k: PlotKind) -> &'static str {
    match k {
        PlotKind::Line => "line",
        PlotKind::Histogram => "histogram",
        PlotKind::Level => "level",
    }
}

fn error(e: &Error) -> String {
    format!("error\t{}\t{}\t{}\n", e.line, e.col, clean(&e.message))
}

/// Space for `len` bytes the page writes into, freed with `fx_free`: on an
/// eight-byte boundary, as the numbers written there are 64-bit.
#[unsafe(no_mangle)]
pub extern "C" fn fx_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<f64>::with_capacity(len.div_ceil(8).max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p.cast()
}

/// # Safety
/// `ptr` and `len` are what `fx_alloc` gave and was asked for.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fx_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(unsafe { Vec::<f64>::from_raw_parts(ptr.cast(), 0, len.div_ceil(8).max(1)) });
    }
}

/// # Safety
/// Each pointer is the page's own allocation of at least the length given.
unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    if ptr.is_null() || len == 0 {
        return "";
    }
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).unwrap_or("")
}

/// # Safety
/// As `text`, for `len` numbers.
unsafe fn numbers<'a>(ptr: *const f64, len: usize) -> &'a [f64] {
    if ptr.is_null() || len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(ptr, len) } }
}

/// What a script draws, as the text and numbers the chart engine reads.
fn drawing(out: &Output, n: usize) -> (String, Vec<f64>) {
    let mut t = format!("pane\t{}\n", u8::from(out.own_pane));
    let mut values = Vec::with_capacity((out.plots.len() + out.marks.len()) * n);
    for p in &out.plots {
        t += &format!("plot\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", kind(p.kind), colour(p.color), tenths(p.width), style(p.style), p.offset, u8::from(p.colors.is_some()), clean(&p.title));
        values.extend_from_slice(&p.values);
        if let Some(c) = &p.colors {
            values.extend(c.iter().map(|c| f64::from(*c)));
        }
    }
    for f in &out.fills {
        t += &format!("fill\t{}\t{}\t{}\n", f.upper, f.lower, f.color);
    }
    for m in &out.marks {
        t += &format!("mark\t{}\t{}\t{}\n", u8::from(m.above), m.color, clean(&m.text));
        values.extend(m.hits.iter().map(|h| if *h { 1.0 } else { 0.0 }));
    }
    for b in &out.backgrounds {
        t += "bg\n";
        values.extend(b.iter().map(|c| f64::from(*c)));
    }
    if let Some(b) = &out.bar_colors {
        t += "bars\n";
        values.extend(b.iter().map(|c| f64::from(*c)));
    }
    for l in &out.lines {
        t += &format!("line\t{}\t{}\t{}\t{}\n", l.color, tenths(l.width), l.style, l.extend);
        values.extend([l.x1, l.y1, l.x2, l.y2]);
    }
    for l in &out.labels {
        t += &format!("label\t{}\t{}\t{}\t{}\t{}\t{}\n", l.color, l.text_color, l.place, l.size, u8::from(l.plain), multiline(&l.text));
        values.extend([l.x, l.y]);
    }
    for b in &out.boxes {
        t += &format!("box\t{}\t{}\t{}\n", b.fill, b.border, tenths(b.width));
        values.extend([b.x1, b.y1, b.x2, b.y2]);
    }
    if let Some(table) = &out.table {
        t += &format!("table\t{}\t{}\n", table.down, table.across);
        for c in &table.cells {
            t += &format!("cell\t{}\t{}\t{}\t{}\t{}\n", c.col, c.row, c.color, c.background, multiline(&c.text));
        }
    }
    (t, values)
}

/// Run a script over `n` bars, laid out as six columns one after another:
/// time, open, high, low, close, volume; `decimals` is how many a price of the
/// market is written with. 1 when it ran, 0 when it says why not.
///
/// # Safety
/// The pointers are the page's allocations, of the lengths given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fx_run(src: *const u8, src_len: usize, inputs: *const f64, inputs_len: usize, bars: *const f64, n: usize, decimals: u32) -> u32 {
    let (src, inputs, all) = unsafe { (text(src, src_len), numbers(inputs, inputs_len), numbers(bars, n.saturating_mul(6))) };
    let column = |k: usize| all.get(k * n..(k + 1) * n).unwrap_or(&[]);
    let bars = Bars { time: column(0), open: column(1), high: column(2), low: column(3), close: column(4), volume: column(5) };
    let (text, values, ok) = match run(src, inputs, &bars, decimals.min(12) as u8) {
        Ok(out) => {
            let (t, values) = drawing(&out, n);
            (t, values, 1)
        }
        Err(e) => (error(&e), Vec::new(), 0),
    };
    OUT.with(|o| *o.borrow_mut() = Out { text, values });
    ok
}

/// The settings, as the page writes them: see `fx_backtest`.
fn settings(v: &[f64]) -> Settings {
    let at = |i: usize| v.get(i).copied().unwrap_or(f64::NAN);
    let or = |i: usize, d: f64| if at(i).is_finite() { at(i) } else { d };
    let yes = |i: usize, d: bool| if at(i).is_finite() { at(i) != 0.0 } else { d };
    let exit = |kind: usize, value: usize, target: bool| match (or(kind, 0.0) as u8, or(value, 0.0)) {
        (1, x) if x > 0.0 => Exit::Pct(x),
        (2, x) if x > 0.0 => Exit::Atr(x),
        (3, x) if x > 0.0 && target => Exit::R(x),
        _ => Exit::None,
    };
    let d = Settings::default();
    Settings {
        capital: or(0, d.capital),
        size: match or(1, 0.0) as u8 {
            1 => Size::Fixed(or(2, 1_000.0)),
            2 => Size::Risk(or(2, 0.01)),
            _ => Size::Equity(or(2, 1.0)),
        },
        leverage: or(3, 1.0),
        taker: or(4, d.taker).max(0.0),
        maker: or(5, d.maker),
        slippage: or(6, 0.0).max(0.0),
        on_close: yes(7, false),
        longs: yes(8, true),
        shorts: yes(9, true),
        stop: exit(10, 11, false),
        target: exit(12, 13, true),
        trail: exit(14, 15, false),
        atr_length: (or(16, 14.0).max(1.0).min(500.0)) as usize,
        max_bars: (or(17, 0.0).max(0.0).min(1e9)) as usize,
        mmr: or(18, 0.0).max(0.0).min(0.5),
        from: at(19),
        to: at(20),
    }
}

/// A trade's numbers, `TRADE` of them, in this order.
fn trade(t: &Trade, out: &mut Vec<f64>) {
    out.extend([
        if t.long { 1.0 } else { 0.0 },
        t.entry_bar as f64,
        t.entry_time,
        t.entry_price,
        t.exit_bar as f64,
        t.exit_time,
        t.exit_price,
        t.qty,
        t.margin,
        t.pnl,
        t.fees,
        t.funding,
        t.risk,
        f64::from(t.reason as u8),
        t.mae,
        t.mfe,
        t.equity_before,
    ]);
}

fn report(r: &Report) -> Vec<f64> {
    let mut v = Vec::with_capacity(8 + r.trades.len() * TRADE + r.equity.len() + r.ambiguous.len());
    v.extend([r.trades.len() as f64, r.equity.len() as f64, r.ambiguous.len() as f64, r.magnified as f64, r.skipped as f64, r.held as f64, f64::from(r.next), r.next_limit]);
    for t in &r.trades {
        trade(t, &mut v);
    }
    v.extend_from_slice(&r.equity);
    v.extend(r.ambiguous.iter().map(|i| *i as f64));
    v
}

/// Backtest a strategy over `n` bars laid out as `fx_run`'s, with `settings` (21 numbers:
/// capital; size kind — 0 a share of equity, 1 a fixed margin, 2 a share risked — and its
/// value; leverage; taker and maker fees; slippage; 1 to fill at the close; longs; shorts;
/// stop kind — 0 none, 1 a share of the price, 2 ATRs — and value; target kind, 3 for R
/// too, and value; trailing kind and value; ATR length; most bars held; maintenance margin
/// rate; the first and last moments, NaN for none), `inner_n` lower-timeframe bars as five
/// columns (time, open, high, low, close), and `funding_n` settlements as two (time, rate).
/// 1 when it ran, 0 when it says why not.
///
/// # Safety
/// The pointers are the page's allocations, of the lengths given.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn fx_backtest(src: *const u8, src_len: usize, inputs: *const f64, inputs_len: usize, bars: *const f64, n: usize, decimals: u32, set: *const f64, set_len: usize, inner: *const f64, inner_n: usize, funding: *const f64, funding_n: usize) -> u32 {
    let (src, inputs, all, set) = unsafe { (text(src, src_len), numbers(inputs, inputs_len), numbers(bars, n.saturating_mul(6)), numbers(set, set_len)) };
    let (low, rates) = unsafe { (numbers(inner, inner_n.saturating_mul(5)), numbers(funding, funding_n.saturating_mul(2))) };
    let column = |k: usize| all.get(k * n..(k + 1) * n).unwrap_or(&[]);
    let bars = Bars { time: column(0), open: column(1), high: column(2), low: column(3), close: column(4), volume: column(5) };
    let lower = |k: usize| low.get(k * inner_n..(k + 1) * inner_n).unwrap_or(&[]);
    let inner = Inner { time: lower(0), open: lower(1), high: lower(2), low: lower(3), close: lower(4) };
    let paid = |k: usize| rates.get(k * funding_n..(k + 1) * funding_n).unwrap_or(&[]);
    let funding = Funding { time: paid(0), rate: paid(1) };
    let (text, values, numbers, ok) = match backtest(src, inputs, &bars, decimals.min(12) as u8, settings(set), inner, funding) {
        Ok((out, r)) => {
            let (t, values) = drawing(&out, n);
            (t, values, report(&r), 1)
        }
        Err(e) => (error(&e), Vec::new(), Vec::new(), 0),
    };
    OUT.with(|o| *o.borrow_mut() = Out { text, values });
    REPORT.with(|r| *r.borrow_mut() = numbers);
    ok
}

#[unsafe(no_mangle)]
pub extern "C" fn fx_report_ptr() -> *const f64 {
    REPORT.with(|r| r.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn fx_report_len() -> usize {
    REPORT.with(|r| r.borrow().len())
}

/// What a formula offers and draws, without running it. 1 when it checks, 0 when it says why not.
///
/// # Safety
/// As `fx_run`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fx_describe(src: *const u8, src_len: usize) -> u32 {
    let src = unsafe { text(src, src_len) };
    let (text, ok) = match describe(src) {
        Ok(d) => {
            let mut t = format!("pane\t{}\n", u8::from(d.own_pane));
            // Written by tradoor's own printer: the standard one weighs some 20 kB of wasm.
            let num = tradoor_core::format::plain;
            let opt = |v: Option<f64>| v.map_or(String::new(), num);
            for i in &d.inputs {
                let options: Vec<String> = i.options.iter().map(|o| clean(o)).collect();
                t += &format!("input\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", i.name, num(i.default), opt(i.min), opt(i.max), num(i.step), clean(&i.label), input_kind(i.kind), options.join("\u{1f}"));
            }
            for (title, k, color) in &d.plots {
                t += &format!("plot\t{}\t{}\t{}\n", kind(*k), colour(*color), clean(title));
            }
            for m in &d.marks {
                t += &format!("mark\t{}\n", clean(m));
            }
            if d.draws {
                t += "draws\t1\n";
            }
            if d.strategy {
                t += "strategy\t1\n";
            }
            (t, 1)
        }
        Err(e) => (error(&e), 0),
    };
    OUT.with(|o| *o.borrow_mut() = Out { text, values: Vec::new() });
    ok
}

#[unsafe(no_mangle)]
pub extern "C" fn fx_text_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().text.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn fx_text_len() -> usize {
    OUT.with(|o| o.borrow().text.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn fx_values_ptr() -> *const f64 {
    OUT.with(|o| o.borrow().values.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn fx_values_len() -> usize {
    OUT.with(|o| o.borrow().values.len())
}
