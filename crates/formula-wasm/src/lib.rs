//! Frost Script in the browser, behind the plainest interface there is: the page
//! writes bytes and numbers into this module's memory, calls one function, and
//! reads bytes and numbers back. The module imports nothing — it cannot call
//! out to the page, the network or anything else — so a formula run here can
//! only give back what the formula crate makes: lines of text and numbers.
//!
//! The text is a line a part, fields split by tabs, numbers as whole numbers:
//!
//! ```text
//! pane    1
//! plot    line    5    15    RSI          (kind, colour, width in tenths, title)
//! fill    0       2    5                  (upper, lower, colour)
//! mark    0       0    Buy                (above, colour, text)
//! error   3       12   ema(…) …           (line, column, message)
//! ```
//!
//! and the numbers are every line's values, then every mark's (1 where it is), a bar each.
use std::cell::RefCell;

use tradoor_formula::{describe, run, Bars, Error, PlotKind};

#[derive(Default)]
struct Out {
    text: String,
    values: Vec<f64>,
}

thread_local! {
    static OUT: RefCell<Out> = RefCell::new(Out::default());
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
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

/// Run a formula over `n` bars, laid out as six columns one after another:
/// time, open, high, low, close, volume. 1 when it ran, 0 when it says why not.
///
/// # Safety
/// The pointers are the page's allocations, of the lengths given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fx_run(src: *const u8, src_len: usize, inputs: *const f64, inputs_len: usize, bars: *const f64, n: usize) -> u32 {
    let (src, inputs, all) = unsafe { (text(src, src_len), numbers(inputs, inputs_len), numbers(bars, n.saturating_mul(6))) };
    let column = |k: usize| all.get(k * n..(k + 1) * n).unwrap_or(&[]);
    let bars = Bars { time: column(0), open: column(1), high: column(2), low: column(3), close: column(4), volume: column(5) };
    let (text, values, ok) = match run(src, inputs, &bars) {
        Ok(out) => {
            let mut t = format!("pane\t{}\n", u8::from(out.own_pane));
            let mut values = Vec::with_capacity((out.plots.len() + out.marks.len()) * n);
            for p in &out.plots {
                t += &format!("plot\t{}\t{}\t{}\t{}\n", kind(p.kind), p.color, (p.width * 10.0).round() as u32, clean(&p.title));
                values.extend_from_slice(&p.values);
            }
            for f in &out.fills {
                t += &format!("fill\t{}\t{}\t{}\n", f.upper, f.lower, f.color);
            }
            for m in &out.marks {
                t += &format!("mark\t{}\t{}\t{}\n", u8::from(m.above), m.color, clean(&m.text));
                values.extend(m.hits.iter().map(|h| if *h { 1.0 } else { 0.0 }));
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
            let opt = |v: Option<f64>| v.map_or(String::new(), |v| v.to_string());
            for i in &d.inputs {
                t += &format!("input\t{}\t{}\t{}\t{}\t{}\t{}\n", i.name, i.default, opt(i.min), opt(i.max), i.step, clean(&i.label));
            }
            for (title, k, color) in &d.plots {
                t += &format!("plot\t{}\t{}\t{}\n", kind(*k), color, clean(title));
            }
            for m in &d.marks {
                t += &format!("mark\t{}\n", clean(m));
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
