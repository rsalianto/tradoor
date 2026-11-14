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
//! Safe by what it is, not by what it forbids. A script is read and run here,
//! by this crate, and nowhere becomes code a browser or server runs; the
//! language has no word for the page, the network, storage, an account or the
//! clock, so there is nothing for a script to reach. It reads candles and gives
//! back numbers, colours and short texts: lines, marks, shapes and a table to
//! draw. Every limit is checked — the text's length, how deep it nests, how
//! large it grows when its functions are written out, how many things it draws,
//! how much it keeps, how far it looks back, how often a loop goes round and how
//! much work it does in all — so a script always ends, and one that asks too
//! much ends with an error saying which line.
mod compile;
mod run;
mod syntax;

pub use syntax::At;

/// The longest script, in bytes.
pub const MAX_SOURCE: usize = 40_000;
pub const MAX_TOKENS: usize = 12_000;
/// How deep brackets and blocks nest.
pub const MAX_DEPTH: usize = 40;
/// How deep one expression's tree grows: brackets, calls, and each `+` or `and` of a chain.
pub const MAX_TREE: usize = 100;
/// How deep the whole script goes once its functions are written out where they
/// are called: what bounds the stack its checking and running take.
pub const MAX_LEVEL: usize = 200;
/// What one call is given.
pub const MAX_ARGS: usize = 12;
/// A text's characters: a label's, a table cell's, one in quotes.
pub const MAX_TEXT: usize = 200;
/// A title's: a line's, a setting's label, a mark's name.
pub const MAX_TITLE: usize = 40;
pub const MAX_INPUTS: usize = 32;
/// What one choice setting offers.
pub const MAX_CHOICES: usize = 16;
/// Lines, histograms and levels together.
pub const MAX_PLOTS: usize = 32;
pub const MAX_FILLS: usize = 16;
pub const MAX_MARKS: usize = 16;
pub const MAX_BACKGROUNDS: usize = 4;
/// Variables, counting each use of a function's own.
pub const MAX_VARS: usize = 1_000;
pub const MAX_FUNCTIONS: usize = 64;
/// How large a script may grow once every function is written out where it is called.
pub const MAX_NODES: usize = 60_000;
/// The longest length an average or window takes.
pub const MAX_LENGTH: usize = 5_000;
/// How far `x[n]` looks back.
pub const MAX_LOOKBACK: usize = 5_000;
/// Past values kept at once, every window and every `[n]` together: what bounds
/// a script's memory, 8 MB of them.
pub const MAX_KEPT: usize = 1_000_000;
/// How often one loop goes round.
pub const MAX_LOOP: usize = 10_000;
/// Steps in one run over every bar: past this, the script stops with an error.
pub const MAX_STEPS: u64 = 40_000_000;
pub const MAX_BARS: usize = 100_000;
pub const MAX_LISTS: usize = 32;
/// What one list holds: pushed when full, its oldest value is let go.
pub const MAX_LIST: usize = 5_000;
/// Shapes kept at once: past these, the oldest is let go as a new one is drawn.
pub const MAX_LINES: usize = 500;
pub const MAX_LABELS: usize = 500;
pub const MAX_BOXES: usize = 200;
pub const TABLE_COLUMNS: usize = 4;
pub const TABLE_ROWS: usize = 16;
/// How far past the newest bar a shape may reach, in bars.
pub const MAX_AHEAD: f64 = 500.0;

/// What went wrong, and where: line and column from 1, or 0 for the script as a whole.
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

/// What a setting is, and so how the page asks for it. Its value is always a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Number,
    /// 1 or 0.
    Bool,
    /// A colour, as `0xRRGGBBAA`.
    Colour,
    /// Which of its `options`, from 0.
    Choice,
    /// A moment, in milliseconds since 1970, UTC.
    Time,
}

/// One setting a script offers: `input length = 14 min 2 max 500`.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSpec {
    pub name: String,
    pub label: String,
    pub kind: InputKind,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: f64,
    /// A choice's texts.
    pub options: Vec<String>,
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

/// How a line is drawn through its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlotStyle {
    Line,
    /// Each value held flat until the next.
    Steps,
    /// A dot a bar, with nothing between.
    Dots,
    Dashed,
    Dotted,
}

/// The candles a script reads, oldest first, every column as long as the others.
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

/// Colours are `0xRRGGBBAA`; 0 is none.
#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    pub title: String,
    pub kind: PlotKind,
    pub style: PlotStyle,
    /// None: the chart's own, as a histogram's above zero and below.
    pub color: Option<u32>,
    pub width: f32,
    /// Bars it is moved by: to the right where above 0, into the space ahead of the newest.
    pub offset: i32,
    /// One a bar; NaN where it has none.
    pub values: Vec<f64>,
    /// One a bar where the script colours the line bar by bar.
    pub colors: Option<Vec<u32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub upper: usize,
    pub lower: usize,
    pub color: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub text: String,
    pub above: bool,
    pub color: u32,
    /// One a bar: whether the mark is there.
    pub hits: Vec<bool>,
}

/// A straight line between two points, each a bar's place (0 the first bar; past the newest is ahead of it) and a price.
#[derive(Clone, Debug, PartialEq)]
pub struct LineArt {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub color: u32,
    pub width: f32,
    /// 0 solid, 1 dashed, 2 dotted.
    pub style: u8,
    /// 0 its own length, 1 on to the left, 2 to the right, 3 both ways.
    pub extend: u8,
}

/// A text at a point.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelArt {
    pub x: f64,
    pub y: f64,
    pub text: String,
    /// Its box; a plain label's text. 0: the chart's own.
    pub color: u32,
    /// 0: whichever of dark and light reads on its box.
    pub text_color: u32,
    /// 0 above the point, 1 below, 2 left of it, 3 right, 4 on it.
    pub place: u8,
    /// 0 tiny, 1 small, 2 normal, 3 large.
    pub size: u8,
    /// Text alone, no box.
    pub plain: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxArt {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub fill: u32,
    pub border: u32,
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub col: u8,
    pub row: u8,
    pub text: String,
    pub color: u32,
    pub background: u32,
}

/// A small table in a corner of the pane.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// 0 top, 1 middle, 2 bottom.
    pub down: u8,
    /// 0 left, 1 centre, 2 right.
    pub across: u8,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub own_pane: bool,
    pub plots: Vec<Plot>,
    pub fills: Vec<Fill>,
    pub marks: Vec<Mark>,
    /// Each a colour a bar behind the chart; 0 where there is none.
    pub backgrounds: Vec<Vec<u32>>,
    /// The candles' colours a bar each, where the script sets them; 0 leaves a candle as it is.
    pub bar_colors: Option<Vec<u32>>,
    pub lines: Vec<LineArt>,
    pub labels: Vec<LabelArt>,
    pub boxes: Vec<BoxArt>,
    pub table: Option<Table>,
}

/// What a script offers and draws, read without running it: for its settings and the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub own_pane: bool,
    pub inputs: Vec<InputSpec>,
    pub plots: Vec<(String, PlotKind, Option<u32>)>,
    pub marks: Vec<String>,
    /// Whether it draws lines, labels or boxes, shades the background, colours candles, fills a table.
    pub draws: bool,
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
        plots: p.plots.iter().map(|s| (s.title.clone(), s.kind, s.color)).collect(),
        marks: p.marks.iter().map(|m| m.text.clone()).collect(),
        draws: p.draws,
        inputs: p.inputs,
    })
}

/// Run over `bars` with these settings, in the order the script writes them; missing ones take their default.
/// `decimals` is how many a price of this market is written with, for `price(x)` and `tick`.
pub fn run(src: &str, inputs: &[f64], bars: &Bars, decimals: u8) -> Result<Output, Error> {
    let n = bars.len();
    if [bars.time.len(), bars.open.len(), bars.high.len(), bars.low.len(), bars.volume.len()].iter().any(|l| *l != n) {
        return Err(Error::whole("the bars' columns differ in length"));
    }
    if n > MAX_BARS {
        return Err(Error::whole(format!("{MAX_BARS} bars at most")));
    }
    run::run(&program(src, inputs)?, bars, decimals.min(12))
}

#[cfg(test)]
mod tests;
