//! The tree into a program the machine runs, and every rule checked on the
//! way: names known, calls given what they take, a length a fixed number, the
//! functions that remember past bars called on every bar, drawing done at the
//! top level, and the limits on how much a formula may ask for.
use crate::syntax::{At, Expr, Input, Line, Node, Op, Stmt};
use crate::{Error, InputSpec, PlotKind, MAX_FILLS, MAX_INPUTS, MAX_LENGTH, MAX_MARKS, MAX_PLOTS, MAX_VARS};
use tradoor_core::studies::FORMULA_COLORS;

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
}

const SERIES: [(&str, Series); 15] = [
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
];

/// Worked out from its arguments alone, anywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Math {
    Abs,
    Sqrt,
    Log,
    Exp,
    Pow,
    Round,
    Floor,
    Ceil,
    Min,
    Max,
    Clamp,
    Nz,
    IsNa,
}

const MATH: [(&str, Math, usize, usize); 13] = [
    ("abs", Math::Abs, 1, 1),
    ("sqrt", Math::Sqrt, 1, 1),
    ("log", Math::Log, 1, 1),
    ("exp", Math::Exp, 1, 1),
    ("pow", Math::Pow, 2, 2),
    ("round", Math::Round, 1, 2),
    ("floor", Math::Floor, 1, 1),
    ("ceil", Math::Ceil, 1, 1),
    ("min", Math::Min, 2, 8),
    ("max", Math::Max, 2, 8),
    ("clamp", Math::Clamp, 3, 3),
    ("nz", Math::Nz, 1, 2),
    ("isna", Math::IsNa, 1, 1),
];

/// Remembers past bars: called on every bar, and so only outside `if` and `for`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keep {
    Sma(usize),
    Ema(usize),
    Rma(usize),
    Stdev(usize),
    Highest(usize),
    Lowest(usize),
    Sum(usize),
    Rsi(usize),
    Atr(usize),
    Change(usize),
    CrossOver,
    CrossUnder,
    BarsSince,
    ValueWhen,
    Changed,
}

/// Name, the series arguments it takes, and whether a length follows them.
const KEEPS: [(&str, usize, bool); 15] = [
    ("sma", 1, true),
    ("ema", 1, true),
    ("rma", 1, true),
    ("stdev", 1, true),
    ("highest", 1, true),
    ("lowest", 1, true),
    ("sum", 1, true),
    ("rsi", 1, true),
    ("atr", 0, true),
    ("change", 1, false),
    ("crossover", 2, false),
    ("crossunder", 2, false),
    ("barssince", 1, false),
    ("valuewhen", 2, false),
    ("changed", 1, false),
];

const DRAWS: [&str; 5] = ["plot", "histogram", "hline", "fill", "mark"];
const KEYWORDS: [&str; 13] = ["input", "state", "if", "else", "for", "to", "and", "or", "not", "pane", "na", "true", "false"];

pub fn is_keyword(w: &str) -> bool {
    KEYWORDS.contains(&w)
}

/// Taken: a variable may not be called any of these.
fn reserved(w: &str) -> bool {
    is_keyword(w)
        || SERIES.iter().any(|(n, _)| *n == w)
        || MATH.iter().any(|(n, ..)| *n == w)
        || KEEPS.iter().any(|(n, ..)| *n == w)
        || DRAWS.contains(&w)
        || FORMULA_COLORS.iter().any(|(n, ..)| *n == w)
        || w == "above"
        || w == "below"
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Slot(usize),
    Bar(Series),
}

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
}

#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Set(usize, Ex),
    /// Only on the first bar: a `state` variable's starting value.
    Start(usize, Ex),
    If(Vec<(Ex, Vec<Run>)>, Vec<Run>),
    For(usize, Ex, Ex, Vec<Run>, At),
    Plot(usize, Ex),
    Mark(usize, Ex),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlotSpec {
    pub title: String,
    pub kind: PlotKind,
    pub color: u8,
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkSpec {
    pub text: String,
    pub above: bool,
    pub color: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub own_pane: bool,
    pub inputs: Vec<InputSpec>,
    pub slots: usize,
    /// Which variables keep their value from bar to bar (`state`).
    pub kept: Vec<bool>,
    /// Which are read back with `[n]`, and so remembered.
    pub remembered: Vec<bool>,
    pub sites: Vec<Keep>,
    pub body: Vec<Run>,
    pub plots: Vec<PlotSpec>,
    pub marks: Vec<MarkSpec>,
    pub fills: Vec<(usize, usize, u8)>,
}

struct Compiler {
    names: Vec<String>,
    inputs: Vec<(String, f64)>,
    program: Program,
}

/// `given` are the inputs' values in the order they are written; missing ones take their default.
pub fn compile(lines: &[Line], given: &[f64]) -> Result<Program, Error> {
    let mut c = Compiler { names: Vec::new(), inputs: Vec::new(), program: Program::default() };
    for line in lines {
        match &line.stmt {
            Stmt::Input(input) => c.input(input, line.at, given)?,
            Stmt::Pane(own) => c.program.own_pane = *own,
            _ => {}
        }
    }
    collect(lines, &mut c, true)?;
    c.program.slots = c.names.len();
    c.program.kept = vec![false; c.names.len()];
    c.program.remembered = vec![false; c.names.len()];
    mark_kept(lines, &mut c);
    c.program.body = c.lines(lines, true)?;
    Ok(c.program)
}

/// Every variable a formula sets, given its place before any line is compiled:
/// a value may be read back before the line that sets it, as `x[1]` in its own definition.
fn collect(lines: &[Line], c: &mut Compiler, top: bool) -> Result<(), Error> {
    for line in lines {
        match &line.stmt {
            Stmt::Set { name, .. } => c.declare(name, line.at)?,
            Stmt::For { name, body, .. } => {
                c.declare(name, line.at)?;
                collect(body, c, false)?;
            }
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    collect(body, c, false)?;
                }
                collect(otherwise, c, false)?;
            }
            Stmt::Input(_) | Stmt::Pane(_) if !top => return Err(Error::at(line.at, "settings and the pane go at the top level, outside if and for")),
            _ => {}
        }
    }
    Ok(())
}

fn mark_kept(lines: &[Line], c: &mut Compiler) {
    for line in lines {
        if let Stmt::Set { name, keep: true, .. } = &line.stmt
            && let Some(slot) = c.names.iter().position(|n| n == name)
        {
            c.program.kept[slot] = true;
        }
    }
}

impl Compiler {
    fn input(&mut self, input: &Input, at: At, given: &[f64]) -> Result<(), Error> {
        if reserved(&input.name) || self.inputs.iter().any(|(n, _)| *n == input.name) {
            return Err(Error::at(at, format!("{} is taken: give the setting another name", input.name)));
        }
        if self.inputs.len() >= MAX_INPUTS {
            return Err(Error::at(at, format!("{MAX_INPUTS} settings at most")));
        }
        let (min, max) = (input.min.unwrap_or(f64::MIN), input.max.unwrap_or(f64::MAX));
        if min > max || !(min..=max).contains(&input.default) {
            return Err(Error::at(at, "the default is not between min and max"));
        }
        let whole = input.default.fract() == 0.0 && input.step.is_none_or(|s| s.fract() == 0.0);
        let mut value = given.get(self.inputs.len()).copied().filter(|v| v.is_finite()).unwrap_or(input.default);
        value = value.max(min).min(max);
        if whole {
            value = value.round();
        }
        self.inputs.push((input.name.clone(), value));
        self.program.inputs.push(InputSpec {
            name: input.name.clone(),
            label: input.label.clone().unwrap_or_else(|| input.name.clone()),
            default: input.default,
            min: input.min,
            max: input.max,
            step: input.step.unwrap_or(if whole { 1.0 } else { 0.1 }),
            value,
        });
        Ok(())
    }

    fn declare(&mut self, name: &str, at: At) -> Result<(), Error> {
        if reserved(name) {
            return Err(Error::at(at, format!("{name} is built in: give the variable another name")));
        }
        if self.inputs.iter().any(|(n, _)| n == name) {
            return Err(Error::at(at, format!("{name} is a setting: it cannot be set here")));
        }
        if !self.names.iter().any(|n| n == name) {
            if self.names.len() >= MAX_VARS {
                return Err(Error::at(at, format!("{MAX_VARS} variables at most")));
            }
            self.names.push(name.to_string());
        }
        Ok(())
    }

    fn slot(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    fn lines(&mut self, lines: &[Line], top: bool) -> Result<Vec<Run>, Error> {
        let mut out = Vec::new();
        for line in lines {
            match &line.stmt {
                Stmt::Pane(_) | Stmt::Input(_) => {}
                Stmt::Set { name, value, keep } => {
                    let slot = self.slot(name).unwrap_or(0);
                    if *keep {
                        if !top {
                            return Err(Error::at(line.at, "state goes at the top level: it starts once, on the first bar"));
                        }
                        let start = self.expr(value, true)?;
                        if has_keep(&start) {
                            return Err(Error::at(line.at, "a state starts at a plain value, as state total = 0"));
                        }
                        out.push(Run::Start(slot, start));
                    } else {
                        out.push(Run::Set(slot, self.expr(value, top)?));
                    }
                }
                Stmt::If { arms, otherwise } => {
                    let mut compiled = Vec::new();
                    for (cond, body) in arms {
                        let cond = self.expr(cond, top)?;
                        compiled.push((cond, self.lines(body, false)?));
                    }
                    let otherwise = self.lines(otherwise, false)?;
                    out.push(Run::If(compiled, otherwise));
                }
                Stmt::For { name, from, to, body } => {
                    let slot = self.slot(name).unwrap_or(0);
                    let (from, to) = (self.expr(from, false)?, self.expr(to, false)?);
                    out.push(Run::For(slot, from, to, self.lines(body, false)?, line.at));
                }
                Stmt::Do(node) => out.extend(self.draw(node, top)?),
            }
        }
        Ok(out)
    }

    /// `plot(…)` and the others: what a formula shows. A fill is drawn from its lines and runs nothing.
    fn draw(&mut self, node: &Node, top: bool) -> Result<Option<Run>, Error> {
        let Expr::Call(name, args) = &node.expr else {
            return Err(Error::at(node.at, "this works out a value and keeps it nowhere: name it, as x = …"));
        };
        if !DRAWS.contains(&name.as_str()) {
            return Err(Error::at(node.at, format!("{name}(…) gives a value: name it, as x = {name}(…)")));
        }
        if !top {
            return Err(Error::at(node.at, format!("{name}(…) draws on every bar: put it outside if and for")));
        }
        let text = |i: usize| -> Result<Option<String>, Error> {
            match args.get(i).map(|a| &a.expr) {
                None => Ok(None),
                Some(Expr::Text(t)) => Ok(Some(t.clone())),
                Some(_) => Err(Error::at(args[i].at, "a title goes in quotes, as \"RSI\"")),
            }
        };
        let colour = |i: usize, fallback: u8| -> Result<u8, Error> { args.get(i).map_or(Ok(fallback), color_of) };
        let next_color = (self.program.plots.len() % 8) as u8;
        match name.as_str() {
            "plot" | "histogram" | "hline" => {
                if args.is_empty() || args.len() > if name == "plot" { 4 } else { 3 } {
                    return Err(Error::at(node.at, format!("{name}(value, \"Title\", colour{})", if name == "plot" { ", width" } else { "" })));
                }
                if self.program.plots.len() >= MAX_PLOTS {
                    return Err(Error::at(node.at, format!("{MAX_PLOTS} lines at most")));
                }
                let title = text(1)?.unwrap_or_else(|| format!("Line {}", self.program.plots.len() + 1));
                if self.program.plots.iter().any(|p| p.title == title) {
                    return Err(Error::at(node.at, format!("two lines are called \"{title}\": give each its own title")));
                }
                let width = match args.get(3) {
                    Some(w) => constant(&self.expr(w, false)?).filter(|w| (0.5..=6.0).contains(w)).ok_or_else(|| Error::at(w.at, "a width is a number from 0.5 to 6"))? as f32,
                    None => 1.5,
                };
                let kind = match name.as_str() {
                    "plot" => PlotKind::Line,
                    "histogram" => PlotKind::Histogram,
                    _ => PlotKind::Level,
                };
                let value = self.expr(&args[0], true)?;
                let fallback = match kind {
                    PlotKind::Level => gray(),
                    PlotKind::Histogram => THEME,
                    PlotKind::Line => DEFAULTS[next_color as usize],
                };
                let color = colour(2, fallback)?;
                self.program.plots.push(PlotSpec { title, kind, color, width });
                Ok(Some(Run::Plot(self.program.plots.len() - 1, value)))
            }
            "fill" => {
                let (Some(a), Some(b)) = (text(0)?, text(1)?) else {
                    return Err(Error::at(node.at, "fill(\"Upper\", \"Lower\", colour): the titles of two lines above it"));
                };
                let find = |t: &str| self.program.plots.iter().position(|p| p.title == t);
                let (Some(ia), Some(ib)) = (find(&a), find(&b)) else {
                    return Err(Error::at(node.at, "fill names two lines drawn above it, by their titles"));
                };
                if self.program.fills.len() >= MAX_FILLS {
                    return Err(Error::at(node.at, format!("{MAX_FILLS} fills at most")));
                }
                let color = colour(2, self.program.plots[ia].color)?;
                self.program.fills.push((ia, ib, color));
                Ok(None)
            }
            _ => {
                if args.is_empty() || args.len() > 4 {
                    return Err(Error::at(node.at, "mark(condition, \"Text\", above or below, colour)"));
                }
                if self.program.marks.len() >= MAX_MARKS {
                    return Err(Error::at(node.at, format!("{MAX_MARKS} marks at most")));
                }
                let mark_text = text(1)?.unwrap_or_default();
                if mark_text.chars().count() > 12 {
                    return Err(Error::at(args[1].at, "a mark's text is 12 characters at most"));
                }
                let above = match args.get(2).map(|a| &a.expr) {
                    None => false,
                    Some(Expr::Name(w)) if w == "above" => true,
                    Some(Expr::Name(w)) if w == "below" => false,
                    Some(_) => return Err(Error::at(args[2].at, "above or below")),
                };
                let cond = self.expr(&args[0], true)?;
                let color = colour(3, if above { red() } else { green() })?;
                self.program.marks.push(MarkSpec { text: mark_text, above, color });
                Ok(Some(Run::Mark(self.program.marks.len() - 1, cond)))
            }
        }
    }

    fn expr(&mut self, node: &Node, top: bool) -> Result<Ex, Error> {
        Ok(match &node.expr {
            Expr::Num(v) => Ex::Num(*v),
            Expr::Text(_) => return Err(Error::at(node.at, "text goes only in a title")),
            Expr::Name(w) => match w.as_str() {
                "na" => Ex::Num(f64::NAN),
                "true" => Ex::Num(1.0),
                "false" => Ex::Num(0.0),
                _ => {
                    if let Some((_, v)) = self.inputs.iter().find(|(n, _)| n == w) {
                        Ex::Num(*v)
                    } else if let Some(slot) = self.slot(w) {
                        Ex::Slot(slot)
                    } else if let Some((_, s)) = SERIES.iter().find(|(n, _)| n == w) {
                        Ex::Bar(*s)
                    } else if FORMULA_COLORS.iter().any(|(n, ..)| n == w) {
                        return Err(Error::at(node.at, format!("{w} is a colour: it goes in plot, fill or mark")));
                    } else if MATH.iter().any(|(n, ..)| n == w) || KEEPS.iter().any(|(n, ..)| n == w) {
                        return Err(Error::at(node.at, format!("{w} is a function: {w}(…)")));
                    } else {
                        return Err(Error::at(node.at, format!("{w} is not known here")));
                    }
                }
            },
            Expr::Back(w, back) => {
                let target = if let Some(slot) = self.slot(w) {
                    self.program.remembered[slot] = true;
                    Target::Slot(slot)
                } else if let Some((_, s)) = SERIES.iter().find(|(n, _)| n == w) {
                    Target::Bar(*s)
                } else if self.inputs.iter().any(|(n, _)| n == w) {
                    return Err(Error::at(node.at, format!("{w} is a setting, the same on every bar: it has no [n]")));
                } else {
                    return Err(Error::at(node.at, format!("{w} is not known here")));
                };
                Ex::Back(target, Box::new(self.expr(back, top)?), node.at)
            }
            Expr::Neg(inner) => Ex::Neg(Box::new(self.expr(inner, top)?)),
            Expr::Not(inner) => Ex::Not(Box::new(self.expr(inner, top)?)),
            Expr::Binary(op, a, b) => Ex::Bin(*op, Box::new(self.expr(a, top)?), Box::new(self.expr(b, top)?)),
            Expr::Pick(c, a, b) => Ex::Pick(Box::new(self.expr(c, top)?), Box::new(self.expr(a, top)?), Box::new(self.expr(b, top)?)),
            Expr::Call(name, args) => self.call(node, name, args, top)?,
        })
    }

    fn call(&mut self, node: &Node, name: &str, args: &[Node], top: bool) -> Result<Ex, Error> {
        if let Some((_, f, least, most)) = MATH.iter().find(|(n, ..)| *n == name) {
            if args.len() < *least || args.len() > *most {
                let takes = if least == most { format!("{least}") } else { format!("{least} to {most}") };
                return Err(Error::at(node.at, format!("{name}(…) takes {takes} values")));
            }
            let args = args.iter().map(|a| self.expr(a, top)).collect::<Result<Vec<_>, _>>()?;
            return Ok(Ex::Math(*f, args));
        }
        if let Some((_, series, length)) = KEEPS.iter().find(|(n, ..)| *n == name) {
            if !top {
                return Err(Error::at(node.at, format!("{name}(…) remembers past bars and so runs on every bar: call it outside if and for")));
            }
            // `change` alone takes its length or not: change(close) is change(close, 1).
            let optional_length = name == "change";
            let wanted = series + usize::from(*length);
            let fits = if optional_length { (1..=2).contains(&args.len()) } else { args.len() == wanted };
            if !fits {
                let shape = match (*series, *length) {
                    (0, _) => format!("{name}(length)"),
                    (1, true) => format!("{name}(value, length)"),
                    (1, false) => format!("{name}(value)"),
                    _ => format!("{name}(a, b)"),
                };
                return Err(Error::at(node.at, if optional_length { "change(value) or change(value, bars)".to_string() } else { shape }));
            }
            let length_at = if optional_length { 1 } else { *series };
            let length = match args.get(length_at) {
                Some(a) if *length || optional_length => {
                    let v = constant(&self.expr(a, true)?).ok_or_else(|| Error::at(a.at, "a length is a number or a setting, the same on every bar"))?;
                    if !(1.0..=MAX_LENGTH as f64).contains(&v.round()) {
                        return Err(Error::at(a.at, format!("a length is from 1 to {MAX_LENGTH}")));
                    }
                    v.round() as usize
                }
                _ => 1,
            };
            let keep = match name {
                "sma" => Keep::Sma(length),
                "ema" => Keep::Ema(length),
                "rma" => Keep::Rma(length),
                "stdev" => Keep::Stdev(length),
                "highest" => Keep::Highest(length),
                "lowest" => Keep::Lowest(length),
                "sum" => Keep::Sum(length),
                "rsi" => Keep::Rsi(length),
                "atr" => Keep::Atr(length),
                "change" => Keep::Change(length),
                "crossover" => Keep::CrossOver,
                "crossunder" => Keep::CrossUnder,
                "barssince" => Keep::BarsSince,
                "valuewhen" => Keep::ValueWhen,
                _ => Keep::Changed,
            };
            let values = args.iter().take(*series).map(|a| self.expr(a, true)).collect::<Result<Vec<_>, _>>()?;
            self.program.sites.push(keep);
            return Ok(Ex::Keep(self.program.sites.len() - 1, values));
        }
        if DRAWS.contains(&name) {
            return Err(Error::at(node.at, format!("{name}(…) draws, and gives no value: a line of its own")));
        }
        Err(Error::at(node.at, format!("{name}(…) is not a function here")))
    }
}

/// A value the same on every bar: a number, a setting, or arithmetic on them.
pub fn constant(e: &Ex) -> Option<f64> {
    match e {
        Ex::Num(v) => Some(*v),
        Ex::Neg(a) => Some(-constant(a)?),
        Ex::Bin(op, a, b) => {
            let (a, b) = (constant(a)?, constant(b)?);
            Some(match op {
                Op::Add => a + b,
                Op::Sub => a - b,
                Op::Mul => a * b,
                Op::Div => a / b,
                _ => return None,
            })
        }
        _ => None,
    }
}

fn has_keep(e: &Ex) -> bool {
    match e {
        Ex::Keep(..) => true,
        Ex::Back(_, a, _) | Ex::Neg(a) | Ex::Not(a) => has_keep(a),
        Ex::Bin(_, a, b) => has_keep(a) || has_keep(b),
        Ex::Pick(c, a, b) => has_keep(c) || has_keep(a) || has_keep(b),
        Ex::Math(_, args) => args.iter().any(has_keep),
        _ => false,
    }
}

fn index_of(name: &str) -> u8 {
    FORMULA_COLORS.iter().position(|(n, ..)| *n == name).unwrap_or(0) as u8
}

fn gray() -> u8 {
    index_of("gray")
}

fn red() -> u8 {
    index_of("red")
}

fn green() -> u8 {
    index_of("green")
}

/// No colour of the palette: the chart's own, as a histogram's above zero and below.
pub const THEME: u8 = u8::MAX;

/// The colours lines take in turn where a formula names none.
const DEFAULTS: [u8; 8] = [5, 3, 8, 6, 2, 0, 4, 9];

fn color_of(node: &Node) -> Result<u8, Error> {
    match &node.expr {
        Expr::Name(w) => FORMULA_COLORS.iter().position(|(n, ..)| n == w).map(|i| i as u8).ok_or_else(|| Error::at(node.at, colours_error())),
        _ => Err(Error::at(node.at, colours_error())),
    }
}

fn colours_error() -> String {
    let names: Vec<&str> = FORMULA_COLORS.iter().map(|(n, ..)| *n).collect();
    format!("a colour is one of {}", names.join(", "))
}
