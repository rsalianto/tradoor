//! Frost Script: tradoor's own small language for indicators.
//!
//! ```text
//! // Relative Strength Index
//! pane own
//! input length = 14 min 2 max 1000
//! move = change(close)
//! gain = rma(max(move, 0), length)
//! loss = rma(max(-move, 0), length)
//! plot(100 - 100 / (1 + gain / loss), "RSI", purple)
//! ```
//!
//! Safe by what it is, not by what it forbids. A formula is read and run here,
//! by this crate, and nowhere becomes code a browser or server runs; the
//! language has no word for the page, the network, storage, an account or the
//! clock, so there is nothing for a formula to reach. It reads candles and gives
//! back numbers. Every limit is checked — the text's length, how deep it nests,
//! how many lines it draws, how far it looks back, how often a loop goes round
//! and how much work it does in all — so a formula always ends, and one that
//! asks too much ends with an error saying which line.
mod compile;
mod run;
mod syntax;

pub use syntax::At;

/// The longest formula, in bytes.
pub const MAX_SOURCE: usize = 20_000;
pub const MAX_TOKENS: usize = 5_000;
/// How deep brackets and blocks nest.
pub const MAX_DEPTH: usize = 40;
pub const MAX_INPUTS: usize = 16;
/// Lines, histograms and levels together.
pub const MAX_PLOTS: usize = 16;
pub const MAX_FILLS: usize = 8;
pub const MAX_MARKS: usize = 8;
pub const MAX_VARS: usize = 128;
/// The longest length an average or window takes.
pub const MAX_LENGTH: usize = 5_000;
/// How far `x[n]` looks back.
pub const MAX_LOOKBACK: usize = 5_000;
/// How often one loop goes round.
pub const MAX_LOOP: usize = 1_000;
/// Steps in one run over every bar: past this, the formula stops with an error.
pub const MAX_STEPS: u64 = 40_000_000;
pub const MAX_BARS: usize = 100_000;

/// What went wrong, and where: line and column from 1, or 0 for the formula as a whole.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl Error {
    pub(crate) fn at(at: At, message: impl Into<String>) -> Self {
        Self { line: at.line, col: at.col, message: message.into() }
    }

    pub(crate) fn whole(message: impl Into<String>) -> Self {
        Self { line: 0, col: 0, message: message.into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 { write!(f, "{}", self.message) } else { write!(f, "line {}: {}", self.line, self.message) }
    }
}

/// One setting a formula offers: `input length = 14 min 2 max 500`.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSpec {
    pub name: String,
    pub label: String,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: f64,
    /// What it was run with.
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlotKind {
    Line,
    Histogram,
    /// A horizontal level: `hline`.
    Level,
}

/// The candles a formula reads, oldest first, every column as long as the others.
pub struct Bars<'a> {
    pub time: &'a [f64],
    pub open: &'a [f64],
    pub high: &'a [f64],
    pub low: &'a [f64],
    pub close: &'a [f64],
    pub volume: &'a [f64],
}

impl Bars<'_> {
    pub fn len(&self) -> usize {
        self.close.len()
    }

    pub fn is_empty(&self) -> bool {
        self.close.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    pub title: String,
    pub kind: PlotKind,
    /// A colour of `tradoor_core::studies::FORMULA_COLORS`.
    pub color: u8,
    pub width: f32,
    /// One a bar; NaN where it has none.
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub upper: usize,
    pub lower: usize,
    pub color: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub text: String,
    pub above: bool,
    pub color: u8,
    /// One a bar: whether the mark is there.
    pub hits: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub own_pane: bool,
    pub plots: Vec<Plot>,
    pub fills: Vec<Fill>,
    pub marks: Vec<Mark>,
}

/// What a formula offers and draws, read without running it: for its settings and the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub own_pane: bool,
    pub inputs: Vec<InputSpec>,
    pub plots: Vec<(String, PlotKind, u8)>,
    pub marks: Vec<String>,
}

fn program(src: &str, inputs: &[f64]) -> Result<compile::Program, Error> {
    if src.len() > MAX_SOURCE {
        return Err(Error::whole(format!("a script is {MAX_SOURCE} characters at most")));
    }
    compile::compile(&syntax::parse(src)?, inputs)
}

/// Checked, and what it offers: an error points to its line and column.
pub fn describe(src: &str) -> Result<Description, Error> {
    let p = program(src, &[])?;
    Ok(Description {
        own_pane: p.own_pane,
        inputs: p.inputs,
        plots: p.plots.iter().map(|s| (s.title.clone(), s.kind, s.color)).collect(),
        marks: p.marks.iter().map(|m| m.text.clone()).collect(),
    })
}

/// Run over `bars` with these settings, in the order the formula writes them; missing ones take their default.
pub fn run(src: &str, inputs: &[f64], bars: &Bars) -> Result<Output, Error> {
    let n = bars.len();
    if [bars.time.len(), bars.open.len(), bars.high.len(), bars.low.len(), bars.volume.len()].iter().any(|l| *l != n) {
        return Err(Error::whole("the bars' columns differ in length"));
    }
    if n > MAX_BARS {
        return Err(Error::whole(format!("{MAX_BARS} bars at most")));
    }
    run::run(&program(src, inputs)?, bars)
}

#[cfg(test)]
mod tests;
