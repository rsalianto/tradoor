//! The tree into a program the machine runs, and every rule checked on the
//! way: names known, each value a number, a text or a list and used as one,
//! calls given what they take, a length a fixed number, the functions that
//! remember past bars called on every bar, plots drawn at the top level, and the
//! limits on how much a script may ask for.
//!
//! A function of the script's own is written out where it is called, each call
//! with its own variables and its own memory of past bars, as a Pine function's
//! is; one that calls itself, or that grows the script past its limit once
//! written out, is refused.
use std::rc::Rc;

use crate::syntax::{At, Default as Given, Expr, Input, Line, Node, Op, Stmt};
use crate::{
    Error, InputKind, InputSpec, PlotKind, PlotStyle, MAX_BACKGROUNDS, MAX_CHOICES, MAX_FILLS, MAX_FUNCTIONS, MAX_INPUTS, MAX_LENGTH, MAX_LISTS, MAX_MARKS, MAX_NODES, MAX_PLOTS, MAX_TITLE, MAX_VARS, TABLE_COLUMNS, TABLE_ROWS,
};

/// A value every bar has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Series {
    Open,
    High,
    Low,
    Close,
    Volume,
    Hl2,
    Hlc3,
    Ohlc4,
    Tr,
    Bar,
    Time,
    Day,
    Week,
    Month,
    Year,
    Hour,
    Minute,
    DayOfWeek,
    DayOfMonth,
    MonthOfYear,
    IsFirst,
    IsLast,
    LastBar,
    Interval,
    Tick,
    Decimals,
}

const SERIES: [(&str, Series); 26] = [
    ("open", Series::Open),
    ("high", Series::High),
    ("low", Series::Low),
    ("close", Series::Close),
    ("volume", Series::Volume),
    ("hl2", Series::Hl2),
    ("hlc3", Series::Hlc3),
    ("ohlc4", Series::Ohlc4),
    ("tr", Series::Tr),
    ("bar", Series::Bar),
    ("time", Series::Time),
    ("day", Series::Day),
    ("week", Series::Week),
    ("month", Series::Month),
    ("year", Series::Year),
    ("hour", Series::Hour),
    ("minute", Series::Minute),
    ("dayofweek", Series::DayOfWeek),
    ("dayofmonth", Series::DayOfMonth),
    ("monthofyear", Series::MonthOfYear),
    ("isfirst", Series::IsFirst),
    ("islast", Series::IsLast),
    ("lastbar", Series::LastBar),
    ("interval", Series::Interval),
    ("tick", Series::Tick),
    ("decimals", Series::Decimals),
];

/// Worked out from its arguments alone, anywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Math {
    Abs,
    Sqrt,
    Log,
    Log10,
    Exp,
    Pow,
    Round,
    Floor,
    Ceil,
    Sign,
    Min,
    Max,
    Avg,
    Clamp,
    Nz,
    IsNa,
    Sin,
    Cos,
    Tan,
    Atan,
    Atan2,
    Rgb,
    Alpha,
    Gradient,
    Date,
}

const MATH: [(&str, Math, usize, usize); 25] = [
    ("abs", Math::Abs, 1, 1),
    ("sqrt", Math::Sqrt, 1, 1),
    ("log", Math::Log, 1, 1),
    ("log10", Math::Log10, 1, 1),
    ("exp", Math::Exp, 1, 1),
    ("pow", Math::Pow, 2, 2),
    ("round", Math::Round, 1, 2),
    ("floor", Math::Floor, 1, 1),
    ("ceil", Math::Ceil, 1, 1),
    ("sign", Math::Sign, 1, 1),
    ("min", Math::Min, 2, 12),
    ("max", Math::Max, 2, 12),
    ("avg", Math::Avg, 2, 12),
    ("clamp", Math::Clamp, 3, 3),
    ("nz", Math::Nz, 1, 2),
    ("isna", Math::IsNa, 1, 1),
    ("sin", Math::Sin, 1, 1),
    ("cos", Math::Cos, 1, 1),
    ("tan", Math::Tan, 1, 1),
    ("atan", Math::Atan, 1, 1),
    ("atan2", Math::Atan2, 2, 2),
    ("rgb", Math::Rgb, 3, 4),
    ("alpha", Math::Alpha, 2, 2),
    ("gradient", Math::Gradient, 5, 5),
    ("date", Math::Date, 3, 5),
];

/// Remembers past bars: called on every bar, and so only outside `if`, `for` and `while`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keep {
    Sma(usize),
    Ema(usize),
    Rma(usize),
    Wma(usize),
    Vwma(usize),
    Hma(usize),
    Stdev(usize),
    Dev(usize),
    Median(usize),
    Highest(usize),
    Lowest(usize),
    HighestBars(usize),
    LowestBars(usize),
    Sum(usize),
    Rsi(usize),
    Atr(usize),
    Change(usize),
    Roc(usize),
    Linreg(usize),
    Cci(usize),
    Mfi(usize),
    Stoch(usize),
    Correlation(usize),
    PercentRank(usize),
    Rising(usize),
    Falling(usize),
    PivotHigh(usize, usize),
    PivotLow(usize, usize),
    CrossOver,
    CrossUnder,
    Cross,
    BarsSince,
    ValueWhen,
    Changed,
    Cum,
}

/// Name, the values it takes, and the lengths after them.
const KEEPS: [(&str, usize, usize); 35] = [
    ("sma", 1, 1),
    ("ema", 1, 1),
    ("rma", 1, 1),
    ("wma", 1, 1),
    ("vwma", 1, 1),
    ("hma", 1, 1),
    ("stdev", 1, 1),
    ("dev", 1, 1),
    ("median", 1, 1),
    ("highest", 1, 1),
    ("lowest", 1, 1),
    ("highestbars", 1, 1),
    ("lowestbars", 1, 1),
    ("sum", 1, 1),
    ("rsi", 1, 1),
    ("atr", 0, 1),
    ("change", 1, 1),
    ("roc", 1, 1),
    ("linreg", 1, 1),
    ("cci", 1, 1),
    ("mfi", 1, 1),
    ("stoch", 3, 1),
    ("correlation", 2, 1),
    ("percentrank", 1, 1),
    ("rising", 1, 1),
    ("falling", 1, 1),
    ("pivothigh", 1, 2),
    ("pivotlow", 1, 2),
    ("crossover", 2, 0),
    ("crossunder", 2, 0),
    ("cross", 2, 0),
    ("barssince", 1, 0),
    ("valuewhen", 2, 0),
    ("changed", 1, 0),
    ("cum", 1, 0),
];

/// The colours a script names, as `0xRRGGBBAA`: the first twelve are the chart's own palette.
pub const COLOURS: [(&str, u32); 19] = [
    ("green", 0x26a6_9aff),
    ("lime", 0xaafd_35ff),
    ("red", 0xff4d_5fff),
    ("orange", 0xff98_00ff),
    ("yellow", 0xffeb_3bff),
    ("blue", 0x2962_ffff),
    ("aqua", 0x00bc_d4ff),
    ("teal", 0x0096_88ff),
    ("purple", 0xb388_ffff),
    ("pink", 0xf062_92ff),
    ("white", 0xe8e8_e8ff),
    ("gray", 0x787b_86ff),
    ("grey", 0x787b_86ff),
    ("black", 0x0000_00ff),
    ("silver", 0xb2b5_beff),
    ("maroon", 0x880e_4fff),
    ("navy", 0x311b_92ff),
    ("olive", 0x8080_00ff),
    ("fuchsia", 0xe040_fbff),
];

/// What a line, a label and a box are drawn in where no colour is named.
const BLUE: u32 = 0x2962_ffff;

/// Words for what a number stands for: a line's style, and the days of the week as Pine numbers them.
const CONSTS: [(&str, f64); 13] = [
    ("pi", std::f64::consts::PI),
    ("solid", 0.0),
    ("dashed", 1.0),
    ("dotted", 2.0),
    ("steps", 3.0),
    ("dots", 4.0),
    ("sunday", 1.0),
    ("monday", 2.0),
    ("tuesday", 3.0),
    ("wednesday", 4.0),
    ("thursday", 5.0),
    ("friday", 6.0),
    ("saturday", 7.0),
];

/// What a script draws along the bars, at its top level.
const DRAWS: [&str; 7] = ["plot", "histogram", "hline", "fill", "mark", "background", "barcolor"];
/// Shapes it keeps and moves, anywhere.
const SHAPES: [&str; 22] = [
    "line", "label", "box", "delete", "set_x1", "set_y1", "set_x2", "set_y2", "set_xy1", "set_xy2", "set_x", "set_y", "set_xy", "set_text", "set_color", "set_textcolor", "set_width", "get_x1", "get_y1", "get_x2", "get_y2",
    "get_text",
];
const OTHERS: [&str; 6] = ["list", "cell", "table", "str", "duration", "barat"];
const KEYWORDS: [&str; 18] = ["input", "state", "if", "else", "for", "to", "while", "break", "continue", "fn", "return", "and", "or", "not", "pane", "na", "true", "false"];

pub fn is_keyword(w: &str) -> bool {
    KEYWORDS.contains(&w)
}

/// Taken: a variable or a function of the script's may not be called any of these.
/// A name a variable, a setting or a parameter cannot take: a word of the language, or a value
/// read by its name alone (a bar's, a colour, a constant), which the variable would hide.
fn reserved(w: &str) -> bool {
    is_keyword(w) || SERIES.iter().any(|(n, _)| *n == w) || COLOURS.iter().any(|(n, _)| *n == w) || CONSTS.iter().any(|(n, _)| *n == w) || w == "above" || w == "below"
}

/// A built-in only ever called, as `rising(close, 3)`. A variable may share its name, as
/// scripts written before it was added do: a call has brackets, and a variable has none.
fn called(w: &str) -> bool {
    MATH.iter().any(|(n, ..)| *n == w) || KEEPS.iter().any(|(n, ..)| *n == w) || DRAWS.contains(&w) || SHAPES.contains(&w) || OTHERS.contains(&w)
}

const PLOT_STYLES: [(&str, f64); 7] = [("line", 0.0), ("solid", 0.0), ("dashed", 1.0), ("dotted", 2.0), ("steps", 3.0), ("dots", 4.0), ("circles", 4.0)];
const LINE_STYLES: [(&str, f64); 3] = [("solid", 0.0), ("dashed", 1.0), ("dotted", 2.0)];
const EXTENDS: [(&str, f64); 4] = [("none", 0.0), ("left", 1.0), ("right", 2.0), ("both", 3.0)];
const PLACES: [(&str, f64); 5] = [("above", 0.0), ("below", 1.0), ("left", 2.0), ("right", 3.0), ("center", 4.0)];
const SIZES: [(&str, f64); 5] = [("tiny", 0.0), ("small", 1.0), ("normal", 2.0), ("large", 3.0), ("huge", 3.0)];
const YES_NO: [(&str, f64); 2] = [("true", 1.0), ("false", 0.0)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Slot(usize),
    Bar(Series),
}

/// What a list does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListOp {
    Push,
    Pop,
    Shift,
    Unshift,
    Get,
    Set,
    Insert,
    Remove,
    Clear,
    Size,
    First,
    Last,
    Sum,
    Avg,
    Min,
    Max,
    IndexOf,
    Includes,
    Sort,
    Reverse,
}

const LIST_OPS: [(&str, ListOp, usize); 20] = [
    ("push", ListOp::Push, 1),
    ("pop", ListOp::Pop, 0),
    ("shift", ListOp::Shift, 0),
    ("unshift", ListOp::Unshift, 1),
    ("get", ListOp::Get, 1),
    ("set", ListOp::Set, 2),
    ("insert", ListOp::Insert, 2),
    ("remove", ListOp::Remove, 1),
    ("clear", ListOp::Clear, 0),
    ("size", ListOp::Size, 0),
    ("first", ListOp::First, 0),
    ("last", ListOp::Last, 0),
    ("sum", ListOp::Sum, 0),
    ("avg", ListOp::Avg, 0),
    ("min", ListOp::Min, 0),
    ("max", ListOp::Max, 0),
    ("indexof", ListOp::IndexOf, 1),
    ("includes", ListOp::Includes, 1),
    ("sort", ListOp::Sort, 0),
    ("reverse", ListOp::Reverse, 0),
];

/// What a shape does: made, moved, read or let go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeOp {
    Line,
    Label,
    Box,
    Delete,
    SetX1,
    SetY1,
    SetX2,
    SetY2,
    SetXy1,
    SetXy2,
    SetText,
    SetColor,
    SetTextColor,
    SetWidth,
    GetX1,
    GetY1,
    GetX2,
    GetY2,
}

/// How a number is written as a text.
#[derive(Clone, Debug, PartialEq)]
pub enum Fmt {
    Plain,
    Fixed(Box<Ex>),
    Duration,
}

/// A number.
#[derive(Clone, Debug, PartialEq)]
pub enum Ex {
    Num(f64),
    Slot(usize),
    Bar(Series),
    Back(Target, Box<Ex>, At),
    Neg(Box<Ex>),
    Not(Box<Ex>),
    Bin(Op, Box<Ex>, Box<Ex>),
    Pick(Box<Ex>, Box<Ex>, Box<Ex>),
    Math(Math, Vec<Ex>),
    Keep(usize, Vec<Ex>),
    /// Two texts the same (or not).
    TextEq(bool, Box<Tx>, Box<Tx>),
    List(ListOp, usize, Vec<Ex>, At),
    Shape(ShapeOp, Vec<Ex>, Option<Box<Tx>>, At),
    /// The place of the bar that holds a moment.
    BarAt(Box<Ex>),
    /// A function of the script's, written out here: its lines, then the slot it gives back in.
    Call(Box<(Vec<Run>, usize)>),
}

/// A text.
#[derive(Clone, Debug, PartialEq)]
pub enum Tx {
    Lit(Rc<str>),
    Slot(usize),
    Join(Vec<Tx>),
    Num(Box<Ex>, Fmt),
    Pick(Box<Ex>, Box<Tx>, Box<Tx>),
    /// A label's text.
    Of(Box<Ex>),
    Call(Box<(Vec<Run>, usize)>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ret {
    Num(usize, Ex),
    Text(usize, Tx),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Set(usize, Ex),
    SetText(usize, Tx),
    /// The first time it is reached: a `state` variable's starting value.
    Start(usize, Ex),
    StartText(usize, Tx),
    ListClear(usize),
    If(Vec<(Ex, Vec<Run>)>, Vec<Run>),
    For { slot: usize, from: Ex, to: Ex, by: Option<Ex>, body: Vec<Run>, at: At },
    While { cond: Ex, body: Vec<Run>, at: At },
    Break,
    Continue,
    Return(Vec<Ret>),
    /// A function's lines, where a `return` ends them.
    Block(Vec<Run>),
    /// A plot's value, and its colour where it changes from bar to bar.
    Plot(usize, Ex, Option<Ex>),
    Mark(usize, Ex),
    Background(usize, Ex),
    BarColor(Ex),
    Cell(Ex, Ex, Tx, Ex, Ex),
    Eval(Ex),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlotSpec {
    pub title: String,
    pub kind: PlotKind,
    pub style: PlotStyle,
    pub color: Option<u32>,
    pub width: f32,
    pub offset: i32,
    pub per_bar: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkSpec {
    pub text: String,
    pub above: bool,
    pub color: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub own_pane: bool,
    pub inputs: Vec<InputSpec>,
    pub nums: usize,
    pub texts: usize,
    /// Which variables keep their value from bar to bar (`state`).
    pub kept: Vec<bool>,
    pub kept_texts: Vec<bool>,
    /// Which are read back with `[n]`, and so remembered.
    pub remembered: Vec<bool>,
    pub sites: Vec<Keep>,
    pub lists: usize,
    pub body: Vec<Run>,
    pub plots: Vec<PlotSpec>,
    pub marks: Vec<MarkSpec>,
    pub fills: Vec<(usize, usize, u32)>,
    pub backgrounds: usize,
    pub bar_colors: bool,
    pub table: (u8, u8),
    /// Whether it draws shapes, shades the background, colours candles, or fills a table.
    pub draws: bool,
}

/// What a name stands for where it is read.
#[derive(Clone, Debug, PartialEq)]
enum Bind {
    Num(usize),
    Text(usize),
    List(usize),
    Const(f64),
    ConstText(Rc<str>),
    /// Set somewhere, and not yet seen set: read before, it is a number.
    Later,
}

/// A value, whichever it is.
enum Any {
    N(Ex),
    T(Tx),
    L(usize),
}

/// A function of the script's: its parameters and its lines.
struct Function<'a> {
    name: String,
    params: &'a [String],
    body: &'a [Line],
}

/// One call of a function being written out: its own names, and the slots it gives back in.
struct Frame {
    names: Vec<(String, Bind)>,
    returns: Option<Vec<Ret>>,
}

struct Compiler<'a> {
    program: Program,
    globals: Vec<(String, Bind)>,
    frames: Vec<Frame>,
    functions: Vec<Function<'a>>,
    /// The functions being written out now, the newest last: what stops one calling itself.
    calling: Vec<String>,
    /// Loops around the line being compiled, in this function: where `break` may go.
    loops: usize,
    nodes: usize,
}

/// `given` are the inputs' values in the order they are written; missing ones take their default.
pub fn compile(lines: &[Line], given: &[f64]) -> Result<Program, Error> {
    let mut c = Compiler { program: Program { table: (0, 2), ..Program::default() }, globals: Vec::new(), frames: Vec::new(), functions: Vec::new(), calling: Vec::new(), loops: 0, nodes: 0 };
    for line in lines {
        match &line.stmt {
            Stmt::Input(input) => c.input(input, line.at, given)?,
            Stmt::Pane(own) => c.program.own_pane = *own,
            Stmt::Table(down, across) => c.program.table = (*down, *across),
            Stmt::Fn { name, params, body } => {
                if reserved(name) || called(name) || c.functions.iter().any(|f| f.name == *name) || c.globals.iter().any(|(n, _)| n == name) {
                    return Err(Error::at(line.at, format!("{name} is taken: give the function another name")));
                }
                if c.functions.len() >= MAX_FUNCTIONS {
                    return Err(Error::at(line.at, format!("{MAX_FUNCTIONS} functions at most")));
                }
                for (i, p) in params.iter().enumerate() {
                    if reserved(p) || params[..i].contains(p) {
                        return Err(Error::at(line.at, format!("{p} cannot be a parameter's name")));
                    }
                }
                // Its shape checked here, called or not: what it works out is checked where it is called.
                assigned(body, &mut Vec::new(), false, line.at)?;
                c.functions.push(Function { name: name.clone(), params, body });
            }
            _ => {}
        }
    }
    let mut later = Vec::new();
    assigned(lines, &mut later, true, line_of(lines))?;
    for name in later {
        if c.functions.iter().any(|f| f.name == name) {
            return Err(Error::at(At { line: 0, col: 0 }, format!("{name} is a function: a variable needs another name")));
        }
        if !c.globals.iter().any(|(n, _)| *n == name) {
            c.globals.push((name, Bind::Later));
        }
    }
    c.program.body = c.lines(lines, true)?;
    c.program.kept.resize(c.program.nums, false);
    c.program.remembered.resize(c.program.nums, false);
    c.program.kept_texts.resize(c.program.texts, false);
    Ok(c.program)
}

fn line_of(lines: &[Line]) -> At {
    lines.first().map_or(At { line: 1, col: 1 }, |l| l.at)
}

/// Every name a block sets, its loops' and `a, b = …`'s too, into `out`; settings and functions only at the top.
fn assigned(lines: &[Line], out: &mut Vec<String>, top: bool, _at: At) -> Result<(), Error> {
    fn add(n: &str, out: &mut Vec<String>) {
        if !out.iter().any(|o| o == n) {
            out.push(n.to_string());
        }
    }
    for line in lines {
        match &line.stmt {
            Stmt::Set { name, .. } => add(name, out),
            Stmt::SetMany { names, .. } => names.iter().for_each(|n| add(n, out)),
            Stmt::For { name, body, .. } => {
                add(name, out);
                assigned(body, out, false, line.at)?;
            }
            Stmt::While { body, .. } => assigned(body, out, false, line.at)?,
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    assigned(body, out, false, line.at)?;
                }
                assigned(otherwise, out, false, line.at)?;
            }
            Stmt::Input(_) | Stmt::Pane(_) | Stmt::Table(..) | Stmt::Fn { .. } if !top => {
                return Err(Error::at(line.at, "settings, functions, the pane and the table go at the top level, outside if, for and while"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A value the same on every bar: a number, a setting, or what pure maths makes of them.
pub fn constant(e: &Ex) -> Option<f64> {
    match e {
        Ex::Num(v) => Some(*v),
        Ex::Neg(a) => Some(-constant(a)?),
        Ex::Not(a) => Some(f64::from(u8::from(!truth(constant(a)?)))),
        Ex::Bin(op, a, b) => Some(crate::run::binary(*op, constant(a)?, constant(b)?)),
        Ex::Pick(c, a, b) => {
            if truth(constant(c)?) {
                constant(a)
            } else {
                constant(b)
            }
        }
        Ex::Math(f, args) => {
            let mut vals = [f64::NAN; 12];
            for (slot, a) in vals.iter_mut().zip(args) {
                *slot = constant(a)?;
            }
            Some(math(*f, &vals[..args.len().min(12)]))
        }
        _ => None,
    }
}

fn truth(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// A colour's four channels, 0–255, as the number a script holds: `0xRRGGBBAA`.
fn rgba(r: f64, g: f64, b: f64, a: f64) -> f64 {
    if [r, g, b, a].iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    let ch = |v: f64| v.round().clamp(0.0, 255.0) as u32;
    f64::from(ch(r) << 24 | ch(g) << 16 | ch(b) << 8 | ch(a))
}

fn channels(c: f64) -> Option<[f64; 4]> {
    if !(0.0..=4_294_967_295.0).contains(&c) {
        return None;
    }
    let v = c as u32;
    Some([(v >> 24) as f64, ((v >> 16) & 255) as f64, ((v >> 8) & 255) as f64, (v & 255) as f64])
}

pub fn math(f: Math, v: &[f64]) -> f64 {
    let nan = f64::NAN;
    let a = v.first().copied().unwrap_or(nan);
    let b = v.get(1).copied();
    match f {
        Math::Abs => a.abs(),
        Math::Sqrt if a < 0.0 => nan,
        Math::Sqrt => a.sqrt(),
        Math::Log | Math::Log10 if a <= 0.0 => nan,
        Math::Log => a.ln(),
        Math::Log10 => a.ln() * std::f64::consts::LOG10_E,
        Math::Exp => a.exp(),
        Math::Pow => a.powf(b.unwrap_or(nan)),
        Math::Round => match b {
            Some(d) if d.is_finite() && (0.0..=10.0).contains(&d) => {
                let scale = 10f64.powi(d as i32);
                (a * scale).round() / scale
            }
            _ => a.round(),
        },
        Math::Floor => a.floor(),
        Math::Ceil => a.ceil(),
        Math::Sign if a.is_nan() => nan,
        Math::Sign => flag(a > 0.0) - flag(a < 0.0),
        // No value in, no value out: max(na, 0) is not 0.
        Math::Min | Math::Max | Math::Avg => {
            if v.iter().any(|x| x.is_nan()) {
                return nan;
            }
            match f {
                Math::Avg => v.iter().sum::<f64>() / v.len() as f64,
                _ => v.iter().copied().reduce(|x, y| if f == Math::Max { x.max(y) } else { x.min(y) }).unwrap_or(nan),
            }
        }
        Math::Clamp => {
            let (lo, hi) = (b.unwrap_or(nan), v.get(2).copied().unwrap_or(nan));
            if a.is_nan() || lo.is_nan() || hi.is_nan() { nan } else { a.max(lo).min(hi) }
        }
        Math::Nz => {
            if a.is_nan() { b.unwrap_or(0.0) } else { a }
        }
        Math::IsNa => flag(a.is_nan()),
        Math::Sin => sin_cos(a).0,
        Math::Cos => sin_cos(a).1,
        Math::Tan => {
            let (s, c) = sin_cos(a);
            if c == 0.0 { nan } else { s / c }
        }
        Math::Atan => atan(a),
        Math::Atan2 => atan2(a, b.unwrap_or(nan)),
        // An opacity from 0 to 1, as CSS writes it.
        Math::Rgb => rgba(a, b.unwrap_or(nan), v.get(2).copied().unwrap_or(nan), v.get(3).map_or(255.0, |o| o.clamp(0.0, 1.0) * 255.0)),
        Math::Alpha => match (channels(a), b) {
            (Some([r, g, bl, _]), Some(o)) => rgba(r, g, bl, o.clamp(0.0, 1.0) * 255.0),
            _ => nan,
        },
        Math::Gradient => {
            let (lo, hi) = (b.unwrap_or(nan), v.get(2).copied().unwrap_or(nan));
            let (Some(c0), Some(c1)) = (v.get(3).copied().and_then(channels), v.get(4).copied().and_then(channels)) else { return nan };
            if a.is_nan() || lo.is_nan() || hi.is_nan() {
                return nan;
            }
            let t = if hi == lo { 1.0 } else { ((a - lo) / (hi - lo)).clamp(0.0, 1.0) };
            let mix = |i: usize| c0[i] + (c1[i] - c0[i]) * t;
            rgba(mix(0), mix(1), mix(2), mix(3))
        }
        Math::Date => {
            let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
            if v.iter().any(|x| !x.is_finite()) || !(1.0..=12.0).contains(&b.unwrap_or(0.0)) || !(1.0..=31.0).contains(&at(2)) {
                return nan;
            }
            let days = tradoor_core::time::days_from_civil(a as i64, at(1) as u32, at(2) as u32);
            (days * tradoor_core::time::DAY) as f64 + at(3) * 3_600_000.0 + at(4) * 60_000.0
        }
    }
}

/// The sine and cosine of `x` radians: brought to within an eighth of a turn, then short series —
/// a hundred-billionth off at most, where the library's weighs kilobytes of wasm for its reduction.
fn sin_cos(x: f64) -> (f64, f64) {
    use std::f64::consts::FRAC_PI_2;
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    let q = (x / FRAC_PI_2).round();
    let r = x - q * FRAC_PI_2;
    let r2 = r * r;
    let s = r * (1.0 - r2 / 6.0 * (1.0 - r2 / 20.0 * (1.0 - r2 / 42.0 * (1.0 - r2 / 72.0 * (1.0 - r2 / 110.0 * (1.0 - r2 / 156.0))))));
    let c = 1.0 - r2 / 2.0 * (1.0 - r2 / 12.0 * (1.0 - r2 / 30.0 * (1.0 - r2 / 56.0 * (1.0 - r2 / 90.0 * (1.0 - r2 / 132.0)))));
    match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// The angle whose tangent is `x`, in radians: brought below tan(π/12), then its series.
fn atan(x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_6};
    if x.is_nan() {
        return f64::NAN;
    }
    let (sign, a) = if x < 0.0 { (-1.0, -x) } else { (1.0, x) };
    let (outer, a) = if a > 1.0 { (true, 1.0 / a) } else { (false, a) };
    let root3 = 1.732_050_807_568_877_2;
    let (base, a) = if a > 0.267_949_192_431_122_7 { (FRAC_PI_6, (a * root3 - 1.0) / (a + root3)) } else { (0.0, a) };
    let a2 = a * a;
    let mut term = a;
    let mut sum = 0.0;
    for k in 0..12 {
        sum += term / f64::from(2 * k + 1) * if k % 2 == 0 { 1.0 } else { -1.0 };
        term *= a2;
    }
    let angle = base + sum;
    sign * if outer { FRAC_PI_2 - angle } else { angle }
}

fn atan2(y: f64, x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    if x > 0.0 {
        atan(y / x)
    } else if x < 0.0 {
        atan(y / x) + if y >= 0.0 { PI } else { -PI }
    } else if y > 0.0 {
        FRAC_PI_2
    } else if y < 0.0 {
        -FRAC_PI_2
    } else {
        0.0
    }
}

/// Whether a setting's default names a colour, and so it is a colour.
fn names_colour(node: &Node) -> bool {
    match &node.expr {
        Expr::Colour(_) => true,
        Expr::Name(w) => COLOURS.iter().any(|(n, _)| n == w),
        Expr::Call(name, ..) => name == "rgb" || name == "alpha",
        _ => false,
    }
}

fn shape(name: &str, spec: &[&str]) -> String {
    format!("{name}({})", spec.join(", "))
}

/// The plain text of a node, where only a text written out will do: a title.
fn title_of(node: &Node, c: &Compiler) -> Option<String> {
    match &node.expr {
        Expr::Text(t) => Some(t.clone()),
        Expr::Name(w) => match c.lookup(w) {
            Some(Bind::ConstText(t)) => Some(t.to_string()),
            _ => None,
        },
        _ => None,
    }
}

impl<'a> Compiler<'a> {
    fn grow(&mut self, at: At) -> Result<(), Error> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(Error::at(at, "this script grows too large once its functions are written out where they are called: call them less, or make them smaller"));
        }
        Ok(())
    }

    fn input(&mut self, input: &Input, at: At, given: &[f64]) -> Result<(), Error> {
        if reserved(&input.name) || self.globals.iter().any(|(n, _)| *n == input.name) {
            return Err(Error::at(at, format!("{} is taken: give the setting another name", input.name)));
        }
        if self.program.inputs.len() >= MAX_INPUTS {
            return Err(Error::at(at, format!("{MAX_INPUTS} settings at most")));
        }
        if input.label.as_ref().is_some_and(|l| l.chars().count() > MAX_TITLE) {
            return Err(Error::at(at, format!("a setting's label is {MAX_TITLE} characters at most")));
        }
        let given = given.get(self.program.inputs.len()).copied().filter(|v| v.is_finite());
        let label = input.label.clone().unwrap_or_else(|| input.name.clone());
        let mut spec = InputSpec { name: input.name.clone(), label, kind: InputKind::Number, default: 0.0, min: None, max: None, step: 1.0, options: Vec::new(), value: 0.0 };
        let bind = match &input.default {
            Given::Choice(first, options) => {
                if options.len() < 2 || options.len() > MAX_CHOICES {
                    return Err(Error::at(at, format!("a choice is one of 2 to {MAX_CHOICES}")));
                }
                if options.iter().any(|o| o.chars().count() > MAX_TITLE) {
                    return Err(Error::at(at, format!("a choice is {MAX_TITLE} characters at most")));
                }
                let Some(start) = options.iter().position(|o| o == first) else {
                    return Err(Error::at(at, format!("\"{first}\" is not one of the choices it is from")));
                };
                let pick = given.map_or(start, |v| (v.round().max(0.0) as usize).min(options.len() - 1));
                spec.kind = InputKind::Choice;
                spec.default = start as f64;
                spec.value = pick as f64;
                spec.options = options.clone();
                Bind::ConstText(Rc::from(options[pick].as_str()))
            }
            Given::Value(node) => {
                let kind = match &node.expr {
                    Expr::Name(w) if w == "true" || w == "false" => InputKind::Bool,
                    Expr::Call(n, ..) if n == "date" => InputKind::Time,
                    _ if names_colour(node) => InputKind::Colour,
                    _ => InputKind::Number,
                };
                let e = self.num(node, true)?;
                let Some(default) = constant(&e).filter(|v| v.is_finite()) else {
                    return Err(Error::at(at, "a setting starts at a number, true or false, a colour, a date, or a choice"));
                };
                spec.kind = kind;
                spec.default = default;
                let value = match kind {
                    InputKind::Number => {
                        let (min, max) = (input.min.unwrap_or(f64::MIN), input.max.unwrap_or(f64::MAX));
                        if min > max || !(min..=max).contains(&default) {
                            return Err(Error::at(at, "the default is not between min and max"));
                        }
                        let whole = default.fract() == 0.0 && input.step.is_none_or(|s| s.fract() == 0.0);
                        spec.min = input.min;
                        spec.max = input.max;
                        spec.step = input.step.unwrap_or(if whole { 1.0 } else { 0.1 });
                        let v = given.unwrap_or(default).max(min).min(max);
                        if whole { v.round() } else { v }
                    }
                    InputKind::Bool => flag(truth(given.unwrap_or(default))),
                    InputKind::Colour => given.filter(|v| channels(*v).is_some()).unwrap_or(default).round(),
                    _ => given.unwrap_or(default),
                };
                spec.value = value;
                Bind::Const(value)
            }
        };
        self.program.inputs.push(spec);
        self.globals.push((input.name.clone(), bind));
        Ok(())
    }

    // ---------------------------------------------------------------- names

    fn lookup(&self, name: &str) -> Option<Bind> {
        if let Some(frame) = self.frames.last()
            && let Some((_, b)) = frame.names.iter().find(|(n, _)| n == name)
        {
            return Some(b.clone());
        }
        self.globals.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone())
    }

    /// A name bound where it is set: in the function being written out, or the script's.
    fn put(&mut self, name: &str, bind: Bind) {
        let names = match self.frames.last_mut() {
            Some(f) => &mut f.names,
            None => &mut self.globals,
        };
        if let Some(slot) = names.iter_mut().find(|(n, _)| n == name) {
            slot.1 = bind;
        } else {
            names.push((name.to_string(), bind));
        }
    }

    /// A name of the script's read before the line that sets it: a number, from now on.
    fn read_later(&mut self, w: &str, at: At) -> Result<usize, Error> {
        let s = self.new_num(at)?;
        if let Some(slot) = self.globals.iter_mut().find(|(n, _)| n == w) {
            slot.1 = Bind::Num(s);
        }
        Ok(s)
    }

    fn new_num(&mut self, at: At) -> Result<usize, Error> {
        if self.program.nums + self.program.texts >= MAX_VARS {
            return Err(Error::at(at, format!("{MAX_VARS} variables at most, counting each call's own")));
        }
        self.program.nums += 1;
        Ok(self.program.nums - 1)
    }

    fn new_text(&mut self, at: At) -> Result<usize, Error> {
        if self.program.nums + self.program.texts >= MAX_VARS {
            return Err(Error::at(at, format!("{MAX_VARS} variables at most, counting each call's own")));
        }
        self.program.texts += 1;
        Ok(self.program.texts - 1)
    }

    /// Where a name set here keeps its number: given one when first set, and checked to stay a number.
    fn num_slot(&mut self, name: &str, at: At) -> Result<usize, Error> {
        match self.writable(name, at)? {
            Bind::Num(s) => Ok(s),
            Bind::Later => {
                let s = self.new_num(at)?;
                self.put(name, Bind::Num(s));
                Ok(s)
            }
            Bind::Text(_) => Err(Error::at(at, format!("{name} holds a text: it cannot be set to a number"))),
            Bind::List(_) => Err(Error::at(at, format!("{name} is a list: it cannot be set to a number"))),
            _ => Err(Error::at(at, format!("{name} is a setting: it cannot be set here"))),
        }
    }

    fn text_slot(&mut self, name: &str, at: At) -> Result<usize, Error> {
        match self.writable(name, at)? {
            Bind::Text(s) => Ok(s),
            Bind::Later => {
                let s = self.new_text(at)?;
                self.put(name, Bind::Text(s));
                Ok(s)
            }
            Bind::Num(_) => Err(Error::at(at, format!("{name} holds a number: it cannot be set to a text"))),
            Bind::List(_) => Err(Error::at(at, format!("{name} is a list: it cannot be set to a text"))),
            _ => Err(Error::at(at, format!("{name} is a setting: it cannot be set here"))),
        }
    }

    /// What a name set here is bound to now: inside a function, its own name (a new one where it
    /// has none yet, as a Pine function's are); else the script's. Later where it is new.
    fn writable(&self, name: &str, at: At) -> Result<Bind, Error> {
        if reserved(name) {
            return Err(Error::at(at, format!("{name} is built in: give the variable another name")));
        }
        if self.functions.iter().any(|f| f.name == name) {
            return Err(Error::at(at, format!("{name} is a function: a variable needs another name")));
        }
        let names = match self.frames.last() {
            Some(f) => &f.names,
            None => &self.globals,
        };
        Ok(names.iter().find(|(n, _)| n == name).map_or(Bind::Later, |(_, b)| b.clone()))
    }

    // ---------------------------------------------------------------- statements

    fn lines(&mut self, lines: &[Line], top: bool) -> Result<Vec<Run>, Error> {
        let mut out = Vec::new();
        for line in lines {
            self.grow(line.at)?;
            match &line.stmt {
                Stmt::Pane(_) | Stmt::Input(_) | Stmt::Table(..) | Stmt::Fn { .. } => {}
                Stmt::Set { name, value, keep } => out.extend(self.set(name, value, *keep, top, line.at)?),
                Stmt::SetMany { names, value } => {
                    let Expr::Call(f, args, named) = &value.expr else {
                        return Err(Error::at(line.at, "a, b = f(…): several names take what a function of the script's gives back"));
                    };
                    let Some(index) = self.functions.iter().position(|x| x.name == *f) else {
                        return Err(Error::at(line.at, format!("{f}(…) is not a function of this script's, giving back several values")));
                    };
                    let (body, rets) = self.inline(index, args, named, top, value.at)?;
                    if rets.len() != names.len() {
                        return Err(Error::at(line.at, format!("{f}(…) gives back {} values, and {} names take them", rets.len(), names.len())));
                    }
                    out.push(Run::Block(body));
                    for (name, ret) in names.iter().zip(rets) {
                        match ret {
                            Ret::Num(s, _) => {
                                let slot = self.num_slot(name, line.at)?;
                                out.push(Run::Set(slot, Ex::Slot(s)));
                            }
                            Ret::Text(s, _) => {
                                let slot = self.text_slot(name, line.at)?;
                                out.push(Run::SetText(slot, Tx::Slot(s)));
                            }
                        }
                    }
                }
                Stmt::If { arms, otherwise } => {
                    let mut compiled = Vec::new();
                    for (cond, body) in arms {
                        let cond = self.num(cond, top)?;
                        compiled.push((cond, self.lines(body, false)?));
                    }
                    let otherwise = self.lines(otherwise, false)?;
                    out.push(Run::If(compiled, otherwise));
                }
                Stmt::For { name, from, to, by, body } => {
                    let slot = self.num_slot(name, line.at)?;
                    let (from, to) = (self.num(from, false)?, self.num(to, false)?);
                    let by = match by {
                        Some(b) => Some(self.num(b, false)?),
                        None => None,
                    };
                    self.loops += 1;
                    let body = self.lines(body, false);
                    self.loops -= 1;
                    out.push(Run::For { slot, from, to, by, body: body?, at: line.at });
                }
                Stmt::While { cond, body } => {
                    let cond = self.num(cond, false)?;
                    self.loops += 1;
                    let body = self.lines(body, false);
                    self.loops -= 1;
                    out.push(Run::While { cond, body: body?, at: line.at });
                }
                Stmt::Break | Stmt::Continue if self.loops == 0 => return Err(Error::at(line.at, "break and continue go inside for or while")),
                Stmt::Break => out.push(Run::Break),
                Stmt::Continue => out.push(Run::Continue),
                Stmt::Return(values) => {
                    if self.frames.is_empty() {
                        return Err(Error::at(line.at, "return goes inside a function"));
                    }
                    let mut rets = Vec::new();
                    for v in values {
                        rets.push(match self.any(v, top)? {
                            Any::N(e) => Ret::Num(usize::MAX, e),
                            Any::T(t) => Ret::Text(usize::MAX, t),
                            Any::L(_) => return Err(Error::at(v.at, "a function gives back numbers and texts, not a list: it can fill one it is given")),
                        });
                    }
                    let frame_returns = self.frames.last().and_then(|f| f.returns.clone());
                    let slots = match frame_returns {
                        Some(known) => {
                            if known.len() != rets.len() || known.iter().zip(&rets).any(|(a, b)| matches!(a, Ret::Num(..)) != matches!(b, Ret::Num(..))) {
                                return Err(Error::at(line.at, "every return of a function gives back as many values, of the same kinds"));
                            }
                            known
                        }
                        None => {
                            let mut fresh = Vec::new();
                            for r in &rets {
                                fresh.push(match r {
                                    Ret::Num(..) => Ret::Num(self.new_num(line.at)?, Ex::Num(f64::NAN)),
                                    Ret::Text(..) => Ret::Text(self.new_text(line.at)?, Tx::Lit(Rc::from(""))),
                                });
                            }
                            if let Some(f) = self.frames.last_mut() {
                                f.returns = Some(fresh.clone());
                            }
                            fresh
                        }
                    };
                    let filled = slots
                        .iter()
                        .zip(rets)
                        .map(|(slot, r)| match (slot, r) {
                            (Ret::Num(s, _), Ret::Num(_, e)) => Ret::Num(*s, e),
                            (Ret::Text(s, _), Ret::Text(_, t)) => Ret::Text(*s, t),
                            (_, r) => r,
                        })
                        .collect();
                    out.push(Run::Return(filled));
                }
                Stmt::Do(node) => out.extend(self.statement(node, top)?),
            }
        }
        Ok(out)
    }

    fn set(&mut self, name: &str, value: &Node, keep: bool, top: bool, at: At) -> Result<Vec<Run>, Error> {
        // A list: made here, or emptied again here.
        if let Expr::Call(f, args, _) = &value.expr
            && f == "list"
        {
            if !args.is_empty() {
                return Err(Error::at(value.at, "list() starts empty: push what it holds"));
            }
            return match self.writable(name, at)? {
                Bind::List(id) => Ok(if keep { Vec::new() } else { vec![Run::ListClear(id)] }),
                Bind::Later => {
                    if self.program.lists >= MAX_LISTS {
                        return Err(Error::at(at, format!("{MAX_LISTS} lists at most, counting each call's own")));
                    }
                    self.program.lists += 1;
                    let id = self.program.lists - 1;
                    self.put(name, Bind::List(id));
                    Ok(if keep { Vec::new() } else { vec![Run::ListClear(id)] })
                }
                _ => Err(Error::at(at, format!("{name} holds a value: a list needs a name of its own"))),
            };
        }
        let value = self.any(value, top)?;
        if keep && match &value {
            Any::N(e) => has_keep(e),
            _ => false,
        } {
            return Err(Error::at(at, "a state starts at a plain value, as state total = 0"));
        }
        Ok(vec![match value {
            Any::N(e) => {
                let slot = self.num_slot(name, at)?;
                if keep {
                    self.program.kept.resize(self.program.nums, false);
                    self.program.kept[slot] = true;
                    Run::Start(slot, e)
                } else {
                    Run::Set(slot, e)
                }
            }
            Any::T(t) => {
                let slot = self.text_slot(name, at)?;
                if keep {
                    self.program.kept_texts.resize(self.program.texts, false);
                    self.program.kept_texts[slot] = true;
                    Run::StartText(slot, t)
                } else {
                    Run::SetText(slot, t)
                }
            }
            Any::L(_) => return Err(Error::at(at, "a list keeps its name: name = list() makes one")),
        }])
    }

    /// A line that is a call alone: drawing, a list or a shape changed, a cell set, a function of the script's.
    fn statement(&mut self, node: &Node, top: bool) -> Result<Vec<Run>, Error> {
        let Expr::Call(name, args, named) = &node.expr else {
            return Err(Error::at(node.at, "this works out a value and keeps it nowhere: name it, as x = …"));
        };
        if DRAWS.contains(&name.as_str()) {
            if !top || !self.frames.is_empty() {
                return Err(Error::at(node.at, format!("{name}(…) draws on every bar: put it at the script's top level, outside if, for, while and functions")));
            }
            return Ok(self.draw(node, name, args, named)?.into_iter().collect());
        }
        // Where the table stands, from a text: table("top right"), or a choice setting's.
        if name == "table" {
            let words = match args.as_slice() {
                [pos] if named.is_empty() => title_of(pos, self),
                _ => None,
            };
            let Some(words) = words else { return Err(Error::at(node.at, "table(\"top right\"): top, middle or bottom, then left, center or right")) };
            let (mut down, mut across) = (0u8, 2u8);
            for w in words.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty()) {
                match w {
                    "top" => down = 0,
                    "middle" => down = 1,
                    "bottom" => down = 2,
                    "left" => across = 0,
                    "center" | "centre" => across = 1,
                    "right" => across = 2,
                    _ => return Err(Error::at(node.at, format!("{w} is not a place for the table: top, middle or bottom, then left, center or right"))),
                }
            }
            self.program.table = (down, across);
            return Ok(Vec::new());
        }
        if name == "cell" {
            let spec = ["col", "row", "text", "color", "background"];
            let a = self.bind(node.at, name, &spec, args, named)?;
            let (Some(col), Some(row)) = (a[0], a[1]) else { return Err(Error::at(node.at, shape(name, &spec))) };
            let (col, row) = (self.num(col, false)?, self.num(row, false)?);
            let text = match a[2] {
                Some(t) => self.text(t, false)?,
                None => Tx::Lit(Rc::from("")),
            };
            let color = self.opt_num(a[3], f64::NAN)?;
            let background = self.opt_num(a[4], f64::NAN)?;
            self.program.draws = true;
            return Ok(vec![Run::Cell(col, row, text, color, background)]);
        }
        if let Some(index) = self.functions.iter().position(|f| f.name == *name) {
            let (body, _) = self.inline(index, args, named, top, node.at)?;
            return Ok(vec![Run::Block(body)]);
        }
        // What changes something: a list, a shape. What only works out a value is kept somewhere.
        if name.starts_with('.') || (SHAPES.contains(&name.as_str()) && !name.starts_with("get_")) {
            return Ok(vec![Run::Eval(self.num(node, top)?)]);
        }
        Err(Error::at(node.at, format!("{name}(…) works out a value and keeps it nowhere: name it, as x = {name}(…)")))
    }

    /// Positional, then named, into the order `spec` writes them.
    fn bind<'n>(&self, at: At, name: &str, spec: &[&str], args: &'n [Node], named: &'n [(String, Node)]) -> Result<Vec<Option<&'n Node>>, Error> {
        if args.len() > spec.len() {
            return Err(Error::at(at, shape(name, spec)));
        }
        let mut out: Vec<Option<&Node>> = (0..spec.len()).map(|i| args.get(i)).collect();
        for (key, value) in named {
            let key = match key.as_str() {
                "colour" => "color",
                "textcolour" => "textcolor",
                k => k,
            };
            let Some(i) = spec.iter().position(|s| *s == key) else {
                return Err(Error::at(value.at, format!("{name}(…) has no {key}: {}", shape(name, spec))));
            };
            if out[i].is_some() {
                return Err(Error::at(value.at, format!("{key} is given twice")));
            }
            out[i] = Some(value);
        }
        Ok(out)
    }

    fn opt_num(&mut self, node: Option<&Node>, otherwise: f64) -> Result<Ex, Error> {
        match node {
            Some(n) => self.num(n, false),
            None => Ok(Ex::Num(otherwise)),
        }
    }

    /// A word of `words` where one is written, else a number: `place=below`, `style=dashed`, `place = up ? above : below`.
    fn word(&mut self, node: Option<&Node>, words: &[(&str, f64)], otherwise: f64) -> Result<Ex, Error> {
        let Some(node) = node else { return Ok(Ex::Num(otherwise)) };
        match &node.expr {
            Expr::Name(w) => {
                if let Some((_, v)) = words.iter().find(|(n, _)| n == w) {
                    return Ok(Ex::Num(*v));
                }
                self.num(node, false)
            }
            Expr::Pick(c, a, b) => {
                let c = self.num(c, false)?;
                Ok(Ex::Pick(Box::new(c), Box::new(self.word(Some(a), words, otherwise)?), Box::new(self.word(Some(b), words, otherwise)?)))
            }
            _ => self.num(node, false),
        }
    }

    /// A number the same on every bar, within a range.
    fn fixed_num(&mut self, node: Option<&Node>, otherwise: f64, range: std::ops::RangeInclusive<f64>, what: &str) -> Result<f64, Error> {
        let Some(node) = node else { return Ok(otherwise) };
        let e = self.num(node, true)?;
        let (lo, hi) = (tradoor_core::format::plain(*range.start()), tradoor_core::format::plain(*range.end()));
        constant(&e).filter(|v| range.contains(v)).ok_or_else(|| Error::at(node.at, format!("{what} is a number from {lo} to {hi}, the same on every bar")))
    }

    /// A colour the same on every bar, or none.
    fn fixed_colour(&mut self, node: Option<&Node>) -> Result<Option<Ex>, Error> {
        let Some(node) = node else { return Ok(None) };
        Ok(Some(self.num(node, true)?))
    }

    fn colour_of(e: &Ex) -> Option<Option<u32>> {
        constant(e).map(|v| channels(v).map(|_| v as u32))
    }

    /// `plot(…)` and the others: what a script shows along the bars. A fill is drawn from its lines and runs nothing.
    fn draw(&mut self, node: &Node, name: &str, args: &[Node], named: &[(String, Node)]) -> Result<Option<Run>, Error> {
        match name {
            "plot" | "histogram" | "hline" => {
                let spec: &[&str] = match name {
                    "plot" => &["value", "title", "color", "width", "style", "offset"],
                    "histogram" => &["value", "title", "color", "offset"],
                    _ => &["value", "title", "color", "width", "style"],
                };
                let a = self.bind(node.at, name, spec, args, named)?;
                let Some(value_node) = a[0] else { return Err(Error::at(node.at, shape(name, spec))) };
                if self.program.plots.len() >= MAX_PLOTS {
                    return Err(Error::at(node.at, format!("{MAX_PLOTS} lines at most")));
                }
                let title = match a[1] {
                    None => format!("Line {}", self.program.plots.len() + 1),
                    Some(t) => title_of(t, self).ok_or_else(|| Error::at(t.at, "a title goes in quotes, as \"RSI\""))?,
                };
                if title.chars().count() > MAX_TITLE {
                    return Err(Error::at(node.at, format!("a title is {MAX_TITLE} characters at most")));
                }
                if self.program.plots.iter().any(|p| p.title == title) {
                    return Err(Error::at(node.at, format!("two lines are called \"{title}\": give each its own title")));
                }
                let (width_at, style_at, offset_at) = match name {
                    "plot" => (Some(3), Some(4), Some(5)),
                    "histogram" => (None, None, Some(3)),
                    _ => (Some(3), Some(4), None),
                };
                let width = self.fixed_num(width_at.and_then(|i| a[i]), 1.5, 0.5..=10.0, "a width")? as f32;
                let style_default = if name == "hline" { 1.0 } else { 0.0 };
                let style = match style_at.and_then(|i| a[i]) {
                    None => style_default,
                    Some(n) => {
                        let e = self.word(Some(n), &PLOT_STYLES, style_default)?;
                        constant(&e).ok_or_else(|| Error::at(n.at, "a style is line, steps, dots, dashed or dotted, the same on every bar"))?
                    }
                };
                let offset = self.fixed_num(offset_at.and_then(|i| a[i]), 0.0, -500.0..=500.0, "an offset")?.round() as i32;
                let kind = match name {
                    "plot" => PlotKind::Line,
                    "histogram" => PlotKind::Histogram,
                    _ => PlotKind::Level,
                };
                let value = self.num(value_node, true)?;
                let colour = self.fixed_colour(a[2])?;
                // Not named: the next of the chart's. Named na: none at all, a line drawn for its values alone.
                let (color, per_bar) = match &colour {
                    None => (None, None),
                    Some(e) => match Self::colour_of(e) {
                        Some(c) => (Some(c.unwrap_or(0)), None),
                        None => (None, colour.clone()),
                    },
                };
                // A line named no colour takes the next of the chart's.
                let color = match (color, &per_bar, kind) {
                    (None, None, PlotKind::Line) => Some(COLOURS[[5, 3, 8, 6, 2, 0, 4, 9][self.program.plots.len() % 8]].1),
                    (None, None, PlotKind::Level) => Some(COLOURS[11].1),
                    (c, ..) => c,
                };
                let style = match style as u8 {
                    1 => PlotStyle::Dashed,
                    2 => PlotStyle::Dotted,
                    3 => PlotStyle::Steps,
                    4 => PlotStyle::Dots,
                    _ => PlotStyle::Line,
                };
                self.program.plots.push(PlotSpec { title, kind, style, color, width, offset, per_bar: per_bar.is_some() });
                Ok(Some(Run::Plot(self.program.plots.len() - 1, value, per_bar)))
            }
            "fill" => {
                let spec = ["upper", "lower", "color"];
                let a = self.bind(node.at, name, &spec, args, named)?;
                let titles = (a[0].and_then(|n| title_of(n, self)), a[1].and_then(|n| title_of(n, self)));
                let (Some(top), Some(bottom)) = titles else {
                    return Err(Error::at(node.at, "fill(\"Upper\", \"Lower\", color): the titles of two lines above it"));
                };
                let find = |t: &str| self.program.plots.iter().position(|p| p.title == t);
                let (Some(ia), Some(ib)) = (find(&top), find(&bottom)) else {
                    return Err(Error::at(node.at, "fill names two lines drawn above it, by their titles"));
                };
                if self.program.fills.len() >= MAX_FILLS {
                    return Err(Error::at(node.at, format!("{MAX_FILLS} fills at most")));
                }
                let colour = match self.fixed_colour(a[2])? {
                    None => self.program.plots[ia].color.unwrap_or(COLOURS[5].1),
                    Some(e) => Self::colour_of(&e).ok_or_else(|| Error::at(node.at, "a fill's colour is the same on every bar"))?.unwrap_or(0),
                };
                self.program.fills.push((ia, ib, colour));
                Ok(None)
            }
            "mark" => {
                let spec = ["condition", "text", "place", "color"];
                let a = self.bind(node.at, name, &spec, args, named)?;
                let Some(cond) = a[0] else { return Err(Error::at(node.at, "mark(condition, \"Text\", above or below, color)")) };
                if self.program.marks.len() >= MAX_MARKS {
                    return Err(Error::at(node.at, format!("{MAX_MARKS} marks at most")));
                }
                let text = match a[1] {
                    None => String::new(),
                    Some(t) => title_of(t, self).ok_or_else(|| Error::at(t.at, "a mark's text goes in quotes; for a text that changes, draw a label"))?,
                };
                if text.chars().count() > 12 {
                    return Err(Error::at(node.at, "a mark's text is 12 characters at most"));
                }
                let above = match a[2] {
                    None => false,
                    Some(n) => match &n.expr {
                        Expr::Name(w) if w == "above" => true,
                        Expr::Name(w) if w == "below" => false,
                        _ => return Err(Error::at(n.at, "above or below")),
                    },
                };
                let colour = match self.fixed_colour(a[3])? {
                    None => COLOURS[if above { 2 } else { 0 }].1,
                    Some(e) => Self::colour_of(&e).ok_or_else(|| Error::at(node.at, "a mark's colour is the same on every bar"))?.unwrap_or(0),
                };
                let cond = self.num(cond, true)?;
                self.program.marks.push(MarkSpec { text, above, color: colour });
                Ok(Some(Run::Mark(self.program.marks.len() - 1, cond)))
            }
            "background" => {
                let a = self.bind(node.at, name, &["color"], args, named)?;
                let Some(c) = a[0] else { return Err(Error::at(node.at, "background(color)")) };
                if self.program.backgrounds >= crate::MAX_BACKGROUNDS {
                    return Err(Error::at(node.at, format!("{MAX_BACKGROUNDS} backgrounds at most")));
                }
                let c = self.num(c, true)?;
                self.program.backgrounds += 1;
                self.program.draws = true;
                Ok(Some(Run::Background(self.program.backgrounds - 1, c)))
            }
            _ => {
                let a = self.bind(node.at, name, &["color"], args, named)?;
                let Some(c) = a[0] else { return Err(Error::at(node.at, "barcolor(color)")) };
                if self.program.bar_colors {
                    return Err(Error::at(node.at, "barcolor(…) once a script"));
                }
                let c = self.num(c, true)?;
                self.program.bar_colors = true;
                self.program.draws = true;
                Ok(Some(Run::BarColor(c)))
            }
        }
    }

    // ---------------------------------------------------------------- functions of the script's

    /// The function written out here: its arguments bound to its parameters, its lines compiled
    /// afresh with names of their own, and the slots it gives back in.
    fn inline(&mut self, index: usize, args: &[Node], named: &[(String, Node)], top: bool, at: At) -> Result<(Vec<Run>, Vec<Ret>), Error> {
        let (name, params, body) = {
            let f = &self.functions[index];
            (f.name.clone(), f.params, f.body)
        };
        if self.calling.contains(&name) {
            return Err(Error::at(at, format!("{name}(…) calls itself: a function may call others, but not itself")));
        }
        let spec: Vec<&str> = params.iter().map(String::as_str).collect();
        let given = self.bind(at, &name, &spec, args, named)?;
        let mut frame = Frame { names: Vec::new(), returns: None };
        let mut start = Vec::new();
        for (param, node) in params.iter().zip(given) {
            let Some(node) = node else { return Err(Error::at(at, format!("{} is not given", shape(&name, &spec)))) };
            let bind = match self.any(node, top)? {
                Any::N(e) => match constant(&e) {
                    Some(v) => Bind::Const(v),
                    None => {
                        let s = self.new_num(at)?;
                        start.push(Run::Set(s, e));
                        Bind::Num(s)
                    }
                },
                Any::T(Tx::Lit(t)) => Bind::ConstText(t),
                Any::T(t) => {
                    let s = self.new_text(at)?;
                    start.push(Run::SetText(s, t));
                    Bind::Text(s)
                }
                Any::L(id) => Bind::List(id),
            };
            frame.names.push((param.clone(), bind));
        }
        // Its own names come as they are set; until then a name reads the script's.
        let mut nested = Vec::new();
        assigned(body, &mut nested, false, at)?;
        self.frames.push(frame);
        self.calling.push(name);
        let loops = std::mem::replace(&mut self.loops, 0);
        let compiled = self.lines(body, top);
        self.loops = loops;
        self.calling.pop();
        let frame = self.frames.pop();
        start.extend(compiled?);
        Ok((start, frame.and_then(|f| f.returns).unwrap_or_default()))
    }

    // ---------------------------------------------------------------- values

    fn num(&mut self, node: &Node, top: bool) -> Result<Ex, Error> {
        match self.any(node, top)? {
            Any::N(e) => Ok(e),
            Any::T(_) => Err(Error::at(node.at, "this is a text, where a number goes")),
            Any::L(_) => Err(Error::at(node.at, "this is a list, where a number goes: list.get(i) reads one of its values")),
        }
    }

    /// A text, where a number is written as one.
    fn text(&mut self, node: &Node, top: bool) -> Result<Tx, Error> {
        match self.any(node, top)? {
            Any::T(t) => Ok(t),
            Any::N(e) => Ok(Tx::Num(Box::new(e), Fmt::Plain)),
            Any::L(_) => Err(Error::at(node.at, "this is a list, where a text goes")),
        }
    }

    fn any(&mut self, node: &Node, top: bool) -> Result<Any, Error> {
        self.grow(node.at)?;
        Ok(match &node.expr {
            Expr::Num(v) => Any::N(Ex::Num(*v)),
            Expr::Text(t) => Any::T(Tx::Lit(Rc::from(t.as_str()))),
            Expr::Colour(v) => Any::N(Ex::Num(f64::from(*v))),
            Expr::Name(w) => self.name(w, node.at)?,
            Expr::Back(w, back) => {
                let target = match self.lookup(w) {
                    Some(Bind::Num(s)) => Target::Slot(s),
                    Some(Bind::Later) => Target::Slot(self.read_later(w, node.at)?),
                    Some(Bind::Const(_)) => return Err(Error::at(node.at, format!("{w} is a setting, the same on every bar: it has no [n]"))),
                    Some(Bind::Text(_) | Bind::ConstText(_)) => return Err(Error::at(node.at, format!("{w} is a text: only numbers are kept bar to bar for [n]"))),
                    Some(Bind::List(_)) => return Err(Error::at(node.at, format!("{w} is a list: {w}.get(i) reads one of its values"))),
                    None => match SERIES.iter().find(|(n, _)| n == w) {
                        Some((_, s)) => Target::Bar(*s),
                        None => return Err(Error::at(node.at, format!("{w} is not known here"))),
                    },
                };
                if let Target::Slot(s) = target {
                    self.program.remembered.resize(self.program.nums, false);
                    self.program.remembered[s] = true;
                }
                Any::N(Ex::Back(target, Box::new(self.num(back, top)?), node.at))
            }
            Expr::Neg(inner) => Any::N(Ex::Neg(Box::new(self.num(inner, top)?))),
            Expr::Not(inner) => Any::N(Ex::Not(Box::new(self.num(inner, top)?))),
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.any(a, top)?, self.any(b, top)?);
                match (op, a, b) {
                    (Op::Add, Any::T(x), Any::T(y)) => Any::T(join(x, y)),
                    (Op::Add, Any::T(x), Any::N(y)) => Any::T(join(x, Tx::Num(Box::new(y), Fmt::Plain))),
                    (Op::Add, Any::N(x), Any::T(y)) => Any::T(join(Tx::Num(Box::new(x), Fmt::Plain), y)),
                    (Op::Eq | Op::Ne, Any::T(x), Any::T(y)) => Any::N(Ex::TextEq(*op == Op::Eq, Box::new(x), Box::new(y))),
                    (_, Any::N(x), Any::N(y)) => Any::N(Ex::Bin(*op, Box::new(x), Box::new(y))),
                    (_, Any::L(_), _) | (_, _, Any::L(_)) => return Err(Error::at(node.at, "a list is not worked out with: read its values with get(i), sum() or avg()")),
                    _ => return Err(Error::at(node.at, "texts join with +, and compare with == and !=: str(x) writes a number as one")),
                }
            }
            Expr::Pick(c, a, b) => {
                let c = self.num(c, top)?;
                let is_na = |n: &Node| matches!(&n.expr, Expr::Name(w) if w == "na");
                let (x, y) = (self.any(a, top)?, self.any(b, top)?);
                match (x, y) {
                    (Any::N(x), Any::N(y)) => Any::N(Ex::Pick(Box::new(c), Box::new(x), Box::new(y))),
                    (Any::T(x), Any::T(y)) => Any::T(Tx::Pick(Box::new(c), Box::new(x), Box::new(y))),
                    (Any::T(x), Any::N(_)) if is_na(b) => Any::T(Tx::Pick(Box::new(c), Box::new(x), Box::new(Tx::Lit(Rc::from(""))))),
                    (Any::N(_), Any::T(y)) if is_na(a) => Any::T(Tx::Pick(Box::new(c), Box::new(Tx::Lit(Rc::from(""))), Box::new(y))),
                    _ => return Err(Error::at(node.at, "both sides of c ? a : b are numbers, or both are texts")),
                }
            }
            Expr::Call(name, args, named) => self.call(node, name, args, named, top)?,
        })
    }

    fn name(&mut self, w: &str, at: At) -> Result<Any, Error> {
        match w {
            "na" => return Ok(Any::N(Ex::Num(f64::NAN))),
            "true" => return Ok(Any::N(Ex::Num(1.0))),
            "false" => return Ok(Any::N(Ex::Num(0.0))),
            _ => {}
        }
        if let Some(bind) = self.lookup(w) {
            return Ok(match bind {
                Bind::Num(s) => Any::N(Ex::Slot(s)),
                Bind::Text(s) => Any::T(Tx::Slot(s)),
                Bind::List(id) => Any::L(id),
                Bind::Const(v) => Any::N(Ex::Num(v)),
                Bind::ConstText(t) => Any::T(Tx::Lit(t)),
                Bind::Later => Any::N(Ex::Slot(self.read_later(w, at)?)),
            });
        }
        if let Some((_, s)) = SERIES.iter().find(|(n, _)| *n == w) {
            return Ok(Any::N(Ex::Bar(*s)));
        }
        if let Some((_, c)) = COLOURS.iter().find(|(n, _)| *n == w) {
            return Ok(Any::N(Ex::Num(f64::from(*c))));
        }
        if let Some((_, v)) = CONSTS.iter().find(|(n, _)| *n == w) {
            return Ok(Any::N(Ex::Num(*v)));
        }
        if MATH.iter().any(|(n, ..)| *n == w) || KEEPS.iter().any(|(n, ..)| *n == w) || self.functions.iter().any(|f| f.name == w) || SHAPES.contains(&w) {
            return Err(Error::at(at, format!("{w} is a function: {w}(…)")));
        }
        Err(Error::at(at, format!("{w} is not known here")))
    }

    fn call(&mut self, node: &Node, name: &str, args: &[Node], named: &[(String, Node)], top: bool) -> Result<Any, Error> {
        if let Some(index) = self.functions.iter().position(|f| f.name == name) {
            let (body, rets) = self.inline(index, args, named, top, node.at)?;
            return match rets.as_slice() {
                [Ret::Num(s, _)] => Ok(Any::N(Ex::Call(Box::new((body, *s))))),
                [Ret::Text(s, _)] => Ok(Any::T(Tx::Call(Box::new((body, *s))))),
                [] => Err(Error::at(node.at, format!("{name}(…) gives back nothing: call it on a line of its own"))),
                _ => Err(Error::at(node.at, format!("{name}(…) gives back {} values: a, b = {name}(…) takes them", rets.len()))),
            };
        }
        if !named.is_empty() && !SHAPES.contains(&name) {
            return Err(Error::at(named[0].1.at, format!("{name}(…) is given its values in order, not by name")));
        }
        if let Some(method) = name.strip_prefix('.') {
            return self.list_op(node, method, args);
        }
        match name {
            "list" => return Err(Error::at(node.at, "a list is made on a line of its own: name = list()")),
            "str" => {
                return match args {
                    [x] => Ok(Any::T(match self.any(x, top)? {
                        Any::T(t) => t,
                        Any::N(e) => Tx::Num(Box::new(e), Fmt::Plain),
                        Any::L(_) => return Err(Error::at(x.at, "str(…) writes a number as a text")),
                    })),
                    [x, d] => {
                        let (x, d) = (self.num(x, top)?, self.num(d, top)?);
                        Ok(Any::T(Tx::Num(Box::new(x), Fmt::Fixed(Box::new(d)))))
                    }
                    _ => Err(Error::at(node.at, "str(value) or str(value, decimals)")),
                };
            }
            "duration" => {
                let [x] = args else { return Err(Error::at(node.at, "duration(milliseconds)")) };
                let x = self.num(x, top)?;
                return Ok(Any::T(Tx::Num(Box::new(x), Fmt::Duration)));
            }
            "barat" => {
                let [t] = args else { return Err(Error::at(node.at, "barat(time)")) };
                return Ok(Any::N(Ex::BarAt(Box::new(self.num(t, top)?))));
            }
            _ => {}
        }
        if SHAPES.contains(&name) {
            return self.shape(node, name, args, named);
        }
        if let Some((_, f, least, most)) = MATH.iter().find(|(n, ..)| *n == name) {
            if args.len() < *least || args.len() > *most {
                let takes = if least == most { format!("{least}") } else { format!("{least} to {most}") };
                return Err(Error::at(node.at, format!("{name}(…) takes {takes} values")));
            }
            let args = args.iter().map(|a| self.num(a, top)).collect::<Result<Vec<_>, _>>()?;
            return Ok(Any::N(Ex::Math(*f, args)));
        }
        if let Some((_, series, lengths)) = KEEPS.iter().find(|(n, ..)| *n == name) {
            if !top {
                return Err(Error::at(node.at, format!("{name}(…) remembers past bars and so runs on every bar: call it outside if, for and while (work it out above, use it inside)")));
            }
            return self.keep(node, name, args, *series, *lengths, top);
        }
        if DRAWS.contains(&name) || name == "cell" {
            return Err(Error::at(node.at, format!("{name}(…) draws, and gives no value: a line of its own")));
        }
        Err(Error::at(node.at, format!("{name}(…) is not a function here")))
    }

    fn keep(&mut self, node: &Node, name: &str, args: &[Node], series: usize, lengths: usize, top: bool) -> Result<Any, Error> {
        // Pine's short forms: highest(length) reads the highs, pivothigh(left, right) too; change(x) is change(x, 1).
        let implicit = match (name, args.len()) {
            ("highest" | "highestbars", 1) | ("pivothigh", 2) => Some(Series::High),
            ("lowest" | "lowestbars", 1) | ("pivotlow", 2) => Some(Series::Low),
            _ => None,
        };
        let (values, lens): (Vec<Ex>, Vec<&Node>) = if let Some(s) = implicit {
            (vec![Ex::Bar(s)], args.iter().collect())
        } else if name == "change" && args.len() == 1 {
            (vec![self.num(&args[0], top)?], Vec::new())
        } else if args.len() == series + lengths {
            let values = args[..series].iter().map(|a| self.num(a, top)).collect::<Result<Vec<_>, _>>()?;
            (values, args[series..].iter().collect())
        } else {
            let names: Vec<&str> = match series {
                0 => vec![],
                1 => vec!["value"],
                2 => vec!["a", "b"],
                _ => vec!["value", "high", "low"],
            };
            let tail: &[&str] = match (name, lengths) {
                ("pivothigh" | "pivotlow", _) => &["left", "right"],
                (_, 1) => &["length"],
                _ => &[],
            };
            return Err(Error::at(node.at, shape(name, &[names.as_slice(), tail].concat())));
        };
        let mut fixed = Vec::new();
        for a in lens {
            let e = self.num(a, true)?;
            let v = constant(&e).ok_or_else(|| Error::at(a.at, "a length is a number or a setting, the same on every bar"))?;
            if !(1.0..=crate::MAX_LENGTH as f64).contains(&v.round()) {
                return Err(Error::at(a.at, format!("a length is from 1 to {MAX_LENGTH}")));
            }
            fixed.push(v.round() as usize);
        }
        let n = fixed.first().copied().unwrap_or(1);
        let keep = match name {
            "sma" => Keep::Sma(n),
            "ema" => Keep::Ema(n),
            "rma" => Keep::Rma(n),
            "wma" => Keep::Wma(n),
            "vwma" => Keep::Vwma(n),
            "hma" => Keep::Hma(n),
            "stdev" => Keep::Stdev(n),
            "dev" => Keep::Dev(n),
            "median" => Keep::Median(n),
            "highest" => Keep::Highest(n),
            "lowest" => Keep::Lowest(n),
            "highestbars" => Keep::HighestBars(n),
            "lowestbars" => Keep::LowestBars(n),
            "sum" => Keep::Sum(n),
            "rsi" => Keep::Rsi(n),
            "atr" => Keep::Atr(n),
            "change" => Keep::Change(n),
            "roc" => Keep::Roc(n),
            "linreg" => Keep::Linreg(n),
            "cci" => Keep::Cci(n),
            "mfi" => Keep::Mfi(n),
            "stoch" => Keep::Stoch(n),
            "correlation" => Keep::Correlation(n),
            "percentrank" => Keep::PercentRank(n),
            "rising" => Keep::Rising(n),
            "falling" => Keep::Falling(n),
            "pivothigh" => Keep::PivotHigh(n, fixed.get(1).copied().unwrap_or(n)),
            "pivotlow" => Keep::PivotLow(n, fixed.get(1).copied().unwrap_or(n)),
            "crossover" => Keep::CrossOver,
            "crossunder" => Keep::CrossUnder,
            "cross" => Keep::Cross,
            "barssince" => Keep::BarsSince,
            "valuewhen" => Keep::ValueWhen,
            "changed" => Keep::Changed,
            _ => Keep::Cum,
        };
        self.program.sites.push(keep);
        Ok(Any::N(Ex::Keep(self.program.sites.len() - 1, values)))
    }

    fn list_op(&mut self, node: &Node, method: &str, args: &[Node]) -> Result<Any, Error> {
        let list = match &args[0].expr {
            Expr::Name(w) => match self.lookup(w) {
                Some(Bind::List(id)) => id,
                _ => return Err(Error::at(node.at, format!("{w} is not a list: {w} = list() makes one"))),
            },
            _ => return Err(Error::at(node.at, "a list's functions are called on its name")),
        };
        let Some((_, op, takes)) = LIST_OPS.iter().find(|(n, ..)| *n == method) else {
            let names: Vec<&str> = LIST_OPS.iter().map(|(n, ..)| *n).collect();
            return Err(Error::at(node.at, format!("a list has no {method}(…): it has {}", names.join(", "))));
        };
        if args.len() - 1 != *takes {
            return Err(Error::at(node.at, format!(".{method}(…) takes {takes} values")));
        }
        let values = args[1..].iter().map(|a| self.num(a, false)).collect::<Result<Vec<_>, _>>()?;
        Ok(Any::N(Ex::List(*op, list, values, node.at)))
    }

    fn shape(&mut self, node: &Node, name: &str, args: &[Node], named: &[(String, Node)]) -> Result<Any, Error> {
        self.program.draws = true;
        let at = node.at;
        let op = match name {
            "line" => {
                let spec = ["x1", "y1", "x2", "y2", "color", "width", "style", "extend"];
                let a = self.bind(at, name, &spec, args, named)?;
                if a[..4].iter().any(Option::is_none) {
                    return Err(Error::at(at, shape(name, &spec)));
                }
                let mut v = Vec::new();
                for n in &a[..4] {
                    v.push(self.num(n.unwrap_or(node), false)?);
                }
                v.push(self.opt_num(a[4], f64::from(BLUE))?);
                v.push(self.opt_num(a[5], 1.0)?);
                v.push(self.word(a[6], &LINE_STYLES, 0.0)?);
                v.push(self.word(a[7], &EXTENDS, 0.0)?);
                return Ok(Any::N(Ex::Shape(ShapeOp::Line, v, None, at)));
            }
            "label" => {
                let spec = ["x", "y", "text", "color", "textcolor", "place", "size", "box"];
                let a = self.bind(at, name, &spec, args, named)?;
                let (Some(x), Some(y)) = (a[0], a[1]) else { return Err(Error::at(at, shape(name, &spec))) };
                let mut v = vec![self.num(x, false)?, self.num(y, false)?];
                let text = match a[2] {
                    Some(t) => self.text(t, false)?,
                    None => Tx::Lit(Rc::from("")),
                };
                v.push(self.opt_num(a[3], f64::from(BLUE))?);
                v.push(self.opt_num(a[4], f64::NAN)?);
                v.push(self.word(a[5], &PLACES, 0.0)?);
                v.push(self.word(a[6], &SIZES, 1.0)?);
                v.push(self.word(a[7], &YES_NO, 1.0)?);
                return Ok(Any::N(Ex::Shape(ShapeOp::Label, v, Some(Box::new(text)), at)));
            }
            "box" => {
                let spec = ["x1", "y1", "x2", "y2", "color", "border", "width"];
                let a = self.bind(at, name, &spec, args, named)?;
                if a[..4].iter().any(Option::is_none) {
                    return Err(Error::at(at, shape(name, &spec)));
                }
                let mut v = Vec::new();
                for n in &a[..4] {
                    v.push(self.num(n.unwrap_or(node), false)?);
                }
                v.push(self.opt_num(a[4], f64::from(BLUE & 0xffff_ff00 | 0x33))?);
                v.push(self.opt_num(a[5], f64::from(BLUE))?);
                v.push(self.opt_num(a[6], 1.0)?);
                return Ok(Any::N(Ex::Shape(ShapeOp::Box, v, None, at)));
            }
            "get_text" => {
                let [id] = args else { return Err(Error::at(at, "get_text(label)")) };
                return Ok(Any::T(Tx::Of(Box::new(self.num(id, false)?))));
            }
            "delete" => (ShapeOp::Delete, 0),
            "set_x1" => (ShapeOp::SetX1, 1),
            "set_y1" => (ShapeOp::SetY1, 1),
            "set_x2" => (ShapeOp::SetX2, 1),
            "set_y2" => (ShapeOp::SetY2, 1),
            "set_xy1" | "set_xy" => (ShapeOp::SetXy1, 2),
            "set_xy2" => (ShapeOp::SetXy2, 2),
            "set_x" => (ShapeOp::SetX1, 1),
            "set_y" => (ShapeOp::SetY1, 1),
            "set_text" => (ShapeOp::SetText, 1),
            "set_color" => (ShapeOp::SetColor, 1),
            "set_textcolor" => (ShapeOp::SetTextColor, 1),
            "set_width" => (ShapeOp::SetWidth, 1),
            "get_x1" => (ShapeOp::GetX1, 0),
            "get_y1" => (ShapeOp::GetY1, 0),
            "get_x2" => (ShapeOp::GetX2, 0),
            _ => (ShapeOp::GetY2, 0),
        };
        let (op, takes) = op;
        if !named.is_empty() || args.len() != takes + 1 {
            let mut spec = vec!["shape"];
            spec.extend(match op {
                ShapeOp::SetXy1 | ShapeOp::SetXy2 => vec!["x", "y"],
                ShapeOp::SetText => vec!["text"],
                ShapeOp::SetColor | ShapeOp::SetTextColor => vec!["color"],
                ShapeOp::SetWidth => vec!["width"],
                ShapeOp::SetX1 | ShapeOp::SetX2 => vec!["x"],
                ShapeOp::SetY1 | ShapeOp::SetY2 => vec!["y"],
                _ => vec![],
            });
            return Err(Error::at(at, shape(name, &spec)));
        }
        let id = self.num(&args[0], false)?;
        if op == ShapeOp::SetText {
            let text = self.text(&args[1], false)?;
            return Ok(Any::N(Ex::Shape(op, vec![id], Some(Box::new(text)), at)));
        }
        let mut v = vec![id];
        for a in &args[1..] {
            v.push(self.num(a, false)?);
        }
        Ok(Any::N(Ex::Shape(op, v, None, at)))
    }
}

fn join(a: Tx, b: Tx) -> Tx {
    match a {
        Tx::Join(mut parts) => {
            parts.push(b);
            Tx::Join(parts)
        }
        a => Tx::Join(vec![a, b]),
    }
}

fn has_keep(e: &Ex) -> bool {
    match e {
        Ex::Keep(..) | Ex::Call(..) => true,
        Ex::Back(_, a, _) | Ex::Neg(a) | Ex::Not(a) | Ex::BarAt(a) => has_keep(a),
        Ex::Bin(_, a, b) => has_keep(a) || has_keep(b),
        Ex::Pick(c, a, b) => has_keep(c) || has_keep(a) || has_keep(b),
        Ex::Math(_, args) | Ex::List(_, _, args, _) | Ex::Shape(_, args, _, _) => args.iter().any(has_keep),
        _ => false,
    }
}

const _: () = assert!(TABLE_COLUMNS <= 255 && TABLE_ROWS <= 255);
