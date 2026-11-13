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
use std::cell::RefCell;

use tradoor_formula::{describe, run, Bars, Error, InputKind, PlotKind, PlotStyle};

#[derive(Default)]
struct Out {
    text: String,
    values: Vec<f64>,
}

thread_local! {
    static OUT: RefCell<Out> = RefCell::new(Out::default());
}

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
            (t, values, 1)
        }
        Err(e) => (error(&e), Vec::new(), 0),
    };
    OUT.with(|o| *o.borrow_mut() = Out { text, values });
    ok
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
