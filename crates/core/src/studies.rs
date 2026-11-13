//! Indicators a reader adds to the chart.
//!
//! Studies sharing a `group` share a pane: a pane has one price scale, and mixing
//! scales is how RSI ends up a flat line beside a MACD measured in dollars.

use crate::format::{fixed, plain};
use crate::indicators::{self, Series};
use crate::time::{london, new_york, weekday, week_index, month_index, year_of, DAY, HOUR, MINUTE};
use crate::Candle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudyKind {
    Ema,
    Sma,
    Bb,
    Rsi,
    StochRsi,
    Macd,
    Atr,
    Keltner,
    Ichimoku,
    Sessions,
    Moon,
    VolumeDelta,
    VisibleProfile,
    KeyLevels,
    Avwap,
    Vwap,
    VwapLevels,
    SwingMap,
    OpenInterest,
    Flow,
    /// One of the reader's own formulas (crates/formula): its lines come worked out, from the page.
    Formula,
}

impl StudyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ema => "ema",
            Self::Sma => "sma",
            Self::Bb => "bb",
            Self::Rsi => "rsi",
            Self::StochRsi => "stochrsi",
            Self::Macd => "macd",
            Self::Atr => "atr",
            Self::Keltner => "keltner",
            Self::Ichimoku => "ichimoku",
            Self::Sessions => "sessions",
            Self::Moon => "moon",
            Self::VolumeDelta => "volumedelta",
            Self::VisibleProfile => "vpvr",
            Self::KeyLevels => "keylevels",
            Self::Avwap => "avwap",
            Self::Vwap => "vwap",
            Self::VwapLevels => "vwaplevels",
            Self::SwingMap => "swingmap",
            Self::OpenInterest => "oi",
            Self::Flow => "flow",
            Self::Formula => "formula",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Price,
    Pct,
    Macd,
    Atr,
    /// Open interest: its own pane, in coins or dollars.
    Oi,
    /// Bought less sold, in coins.
    Delta,
    /// A formula's own pane: one each, by its place among the chart's formulas.
    Formula(u8),
}

/// The panes formulas draw in, by name, and their lines', fills' and marks' keys.
pub const FORMULA_PANES: [&str; 8] = ["fx0", "fx1", "fx2", "fx3", "fx4", "fx5", "fx6", "fx7"];
pub const FORMULA_LINES: [&str; 16] = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13", "p14", "p15"];
pub const FORMULA_FILLS: [&str; 8] = ["f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7"];
pub const FORMULA_MARKS: [&str; 8] = ["m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7"];

impl Group {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Price => "price",
            Self::Pct => "pct",
            Self::Macd => "macd",
            Self::Atr => "atr",
            Self::Oi => "oi",
            Self::Delta => "delta",
            Self::Formula(n) => FORMULA_PANES[usize::from(n) % FORMULA_PANES.len()],
        }
    }
}

pub struct StudyParam {
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    /// What a value moves by: whole for a length, finer for a multiplier.
    pub step: f64,
}

/// What the engine needs of a study: the page's own catalogue
/// (web/src/lib/study-schema) names and describes it, in the picker's chunk
/// rather than in the wasm every chart waits for.
pub struct StudyDefinition {
    pub kind: StudyKind,
    pub short: &'static str,
    pub group: Group,
    pub params: &'static [StudyParam],
}

const fn p(name: &'static str, default: u32, min: u32, max: u32) -> StudyParam {
    StudyParam { name, default: default as f64, min: min as f64, max: max as f64, step: 1.0 }
}

/// A multiplier, in tenths.
const fn x(name: &'static str, default: f64, min: f64) -> StudyParam {
    StudyParam { name, default, min, max: 10.0, step: 0.1 }
}

/// What `vwap`'s first parameter starts it from.
const VWAP_ANCHORS: [&str; 5] = ["Session", "Week", "Month", "Year", "Quarter"];
/// The last period's levels, as the chart labels them: `pMVAH` is last month's.
const VWAP_PREVIOUS: [[&str; 3]; 5] = [["pDVAH", "pDVWAP", "pDVAL"], ["pWVAH", "pWVWAP", "pWVAL"], ["pMVAH", "pMVWAP", "pMVAL"], ["pYVAH", "pYVWAP", "pYVAL"], ["pQVAH", "pQVWAP", "pQVAL"]];

/// The colours a formula names (crates/formula): its line, and the shade of a fill in it.
pub const FORMULA_COLORS: [(&str, &str, &str); 12] = [
    ("green", "#26a69a", "#26a69a26"),
    ("lime", "#aafd35", "#aafd3526"),
    ("red", "#ff4d5f", "#ff4d5f26"),
    ("orange", "#ff9800", "#ff980026"),
    ("yellow", "#ffeb3b", "#ffeb3b26"),
    ("blue", "#2962ff", "#2962ff1f"),
    ("aqua", "#00bcd4", "#00bcd426"),
    ("teal", "#009688", "#00968826"),
    ("purple", "#b388ff", "#b388ff26"),
    ("pink", "#f06292", "#f0629226"),
    ("white", "#e8e8e8", "#e8e8e81f"),
    ("gray", "#787b86", "#787b8626"),
];

pub static STUDIES: [StudyDefinition; 20] = [
    StudyDefinition { kind: StudyKind::Ema, short: "EMA", group: Group::Price, params: &[p("length", 50, 2, 1000)] },
    StudyDefinition { kind: StudyKind::Sma, short: "SMA", group: Group::Price, params: &[p("length", 50, 2, 1000)] },
    StudyDefinition { kind: StudyKind::Bb, short: "BB", group: Group::Price, params: &[p("length", 20, 2, 1000), x("mult", 2.0, 0.1)] },
    StudyDefinition { kind: StudyKind::Keltner, short: "KC", group: Group::Price, params: &[p("length", 20, 2, 1000), x("mult", 2.0, 0.1), p("atr", 10, 1, 1000)] },
    StudyDefinition { kind: StudyKind::Ichimoku, short: "Ichimoku", group: Group::Price, params: &[p("conversion", 9, 1, 1000), p("base", 26, 1, 1000), p("span", 52, 1, 1000), p("shift", 26, 1, 500)] },
    StudyDefinition { kind: StudyKind::Rsi, short: "RSI", group: Group::Pct, params: &[p("length", 14, 2, 1000)] },
    StudyDefinition { kind: StudyKind::StochRsi, short: "Stoch RSI", group: Group::Pct, params: &[p("rsi", 14, 2, 1000), p("stoch", 14, 2, 1000), p("k", 3, 1, 100), p("d", 3, 1, 100)] },
    StudyDefinition { kind: StudyKind::Macd, short: "MACD", group: Group::Macd, params: &[p("fast", 12, 1, 1000), p("slow", 26, 2, 1000), p("signal", 9, 1, 1000)] },
    StudyDefinition { kind: StudyKind::KeyLevels, short: "Key Levels", group: Group::Price, params: &[] },
    // A band of 0 is not drawn.
    StudyDefinition { kind: StudyKind::Vwap, short: "VWAP", group: Group::Price, params: &[p("anchor", 0, 0, 4), x("band1", 1.0, 0.0), x("band2", 0.0, 0.0), x("band3", 0.0, 0.0)] },
    // `periods` a bit each for the day, week, month, quarter and year; `previous` 1 labels the last of each too.
    StudyDefinition { kind: StudyKind::VwapLevels, short: "VWAP Levels", group: Group::Price, params: &[p("periods", 30, 1, 31), p("previous", 1, 0, 1), x("band", 1.0, 0.1)] },
    StudyDefinition { kind: StudyKind::Avwap, short: "Multi-VWAP", group: Group::Price, params: &[] },
    StudyDefinition { kind: StudyKind::SwingMap, short: "Swings", group: Group::Price, params: &[p("left", 5, 1, 100), p("right", 5, 1, 100)] },
    StudyDefinition { kind: StudyKind::Flow, short: "Flow", group: Group::Price, params: &[] },
    StudyDefinition { kind: StudyKind::Sessions, short: "Sessions", group: Group::Price, params: &[] },
    StudyDefinition { kind: StudyKind::Moon, short: "Moon", group: Group::Price, params: &[] },
    // `view` 0 each bar's, 1 cumulative.
    StudyDefinition { kind: StudyKind::VolumeDelta, short: "Vol Delta", group: Group::Delta, params: &[p("view", 0, 0, 1)] },
    // `side` 0 against the right edge, 1 the left.
    StudyDefinition { kind: StudyKind::VisibleProfile, short: "VPVR", group: Group::Price, params: &[p("rows", 24, 5, 200), p("va", 70, 1, 100), p("width", 30, 5, 90), p("side", 0, 0, 1)] },
    StudyDefinition { kind: StudyKind::Atr, short: "ATR", group: Group::Atr, params: &[p("length", 14, 2, 1000)] },
    // `venues` is a bit per venue in the server's order; `measure` 0 coins, 1 dollars; `view` 0 candles, 1 line.
    StudyDefinition { kind: StudyKind::OpenInterest, short: "Aggregated OI", group: Group::Oi, params: &[p("venues", OI_ALL_VENUES, 1, OI_ALL_VENUES), p("measure", 0, 0, 1), p("view", 0, 0, 1)] },
];

/// One bit per open interest venue, in the server's order.
pub const OI_VENUES: [&str; 8] = ["binance-usdt", "bybit-usdt", "okx-usdt", "deribit-usd", "hyperliquid-usd", "binance-usd", "bybit-usd", "okx-usd"];
pub const OI_ALL_VENUES: u32 = (1 << OI_VENUES.len()) - 1;

pub fn definition(kind: &str) -> Option<&'static StudyDefinition> {
    STUDIES.iter().find(|d| d.kind.as_str() == kind)
}

#[derive(Clone, Debug, PartialEq)]
pub struct StudySpec {
    pub kind: StudyKind,
    pub params: Vec<f64>,
}

impl StudySpec {
    fn def(&self) -> &'static StudyDefinition {
        STUDIES.iter().find(|d| d.kind == self.kind).expect("every kind has a definition")
    }

    /// Its `i`th parameter, or the definition's default.
    fn number(&self, i: usize) -> f64 {
        self.params.get(i).copied().unwrap_or(self.def().params.get(i).map_or(0.0, |p| p.default))
    }

    fn param(&self, i: usize) -> usize {
        self.number(i) as usize
    }

    /// `ema:50` — the id AND the wire format: what localStorage holds and what a
    /// legend row is keyed by, so a study cannot be added twice under two names.
    pub fn id(&self) -> String {
        std::iter::once(self.kind.as_str().to_string()).chain(self.params.iter().map(|v| plain(*v))).collect::<Vec<_>>().join(":")
    }

    /// `EMA 50`, `Stoch RSI 14 14 3 3`.
    pub fn label(&self) -> String {
        let short = self.def().short;
        if self.kind == StudyKind::OpenInterest {
            let venues = self.params.first().map_or(OI_ALL_VENUES, |v| *v as u32);
            let measure = if self.param(1) == 1 { "USD" } else { "Coins" };
            let view = if self.param(2) == 1 { "Line" } else { "Candles" };
            let count = if venues == OI_ALL_VENUES { String::new() } else { format!(" · {} venues", venues.count_ones()) };
            return format!("{short} {measure} {view}{count}");
        }
        if self.kind == StudyKind::VolumeDelta {
            return if self.param(0) == 1 { "CVD".to_string() } else { short.to_string() };
        }
        if self.kind == StudyKind::VisibleProfile {
            return format!("{short} {}", self.param(0));
        }
        if self.kind == StudyKind::Vwap {
            let mut label = format!("{short} {}", VWAP_ANCHORS[self.param(0).min(4)]);
            for band in (1..4).map(|i| self.number(i)).filter(|b| *b > 0.0) {
                label += " ±";
                label += &plain(band);
            }
            return label;
        }
        if self.params.is_empty() || self.kind == StudyKind::VwapLevels {
            short.to_string()
        } else {
            format!("{short} {}", self.params.iter().map(|v| plain(*v)).collect::<Vec<_>>().join(" "))
        }
    }

    pub fn group(&self) -> Group {
        self.def().group
    }
}

/// Parse `ema:50,rsi:14`. Unknown kinds and out-of-range parameters are DROPPED
/// rather than failing the list: it comes from storage an older version may have
/// written, and a chart that refuses to load is worse than one missing a study.
pub fn parse_studies(raw: &str, max: usize) -> Vec<StudySpec> {
    let mut out: Vec<StudySpec> = Vec::new();
    for token in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let mut parts = token.split(':');
        let Some(def) = parts.next().and_then(definition) else { continue };
        let rest: Vec<&str> = parts.collect();
        let params: Option<Vec<f64>> = def
            .params
            .iter()
            .enumerate()
            .map(|(i, param)| {
                let value = match rest.get(i) {
                    None => param.default,
                    Some(s) if s.trim().is_empty() => 0.0,
                    Some(s) => crate::format::parse_number(s.trim())?,
                };
                // Hundredths at the finest, so a typed 0.3 does not come back 0.30000000000000004.
                (value.is_finite() && value >= param.min && value <= param.max).then(|| if param.step >= 1.0 { value.round() } else { (value * 100.0).round() / 100.0 })
            })
            .collect();
        let Some(params) = params else { continue };
        let spec = StudySpec { kind: def.kind, params };
        if out.iter().any(|s| s == &spec) {
            continue;
        }
        out.push(spec);
        if out.len() >= max {
            break;
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineType {
    Line,
    Histogram,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Solid,
    /// Holds each value flat until the next — what an opening price does.
    Step,
    /// Starts again each time this anchor turns over, with no line across the turn.
    Anchored(usize),
    /// Its last value alone: a short stub and its label, right of the newest bar.
    Tag,
    /// Not stroked: the series only carries values for the legend and the axis.
    None,
}

/// How a line is stroked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Debug)]
pub struct StudyLine {
    pub key: &'static str,
    pub label: String,
    pub kind: LineType,
    pub values: Series,
    pub style: LineStyle,
    pub pattern: Pattern,
    /// Fixed colour for lines told apart BY colour; otherwise the chart's palette.
    pub color: Option<&'static str>,
    pub width: f32,
    /// Whether its last value gets a badge on the axis.
    pub badge: bool,
}

impl StudyLine {
    fn line(key: &'static str, label: impl Into<String>, values: Series) -> Self {
        Self { key, label: label.into(), kind: LineType::Line, values, style: LineStyle::Solid, pattern: Pattern::Solid, color: None, width: 1.0, badge: true }
    }

    fn inked(key: &'static str, label: &'static str, values: Series, color: &'static str) -> Self {
        Self { color: Some(color), ..Self::line(key, label, values) }
    }
}

/// What a study reads besides the closed candles.
#[derive(Clone, Copy, Debug, Default)]
pub struct StudyInputs<'a> {
    /// Summed open interest as `(time, [open, high, low, close])` candles of
    /// some step no longer than a bar, oldest first.
    pub open_interest: &'a [(i64, [f64; 4])],
    /// The forming bar's open time and last price: open interest runs to it.
    pub live: Option<(i64, f64)>,
    /// The daily bars the chart's own were folded from, oldest first. A bar
    /// longer than a day straddles the turn of a week, month or year, so its
    /// open is not the period's open; these say where the period really opened.
    /// Empty when the chart's bars are a day or shorter, where they already do.
    pub days: &'a [Candle],
    /// Hourly and daily bars back to the start of last month and last year, for VWAP levels.
    pub hourly: &'a [Candle],
    pub daily: &'a [Candle],
}

/// A level that holds for a bounded stretch of time — last week's open is a fact
/// about last week, not a line carried to the right edge.
#[derive(Clone, Debug, PartialEq)]
pub struct StudySegment {
    pub from: i64,
    pub to: i64,
    pub price: f64,
    /// The line whose style it takes.
    pub key: &'static str,
    pub label: &'static str,
    pub color: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Bull,
    Bear,
    Neutral,
    /// A colour of its own.
    Ink(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StudyMarker {
    pub time: i64,
    pub above: bool,
    /// None draws a larger mark with nothing beside it.
    pub text: String,
    pub tone: Tone,
    /// The plot whose style it takes.
    pub plot: &'static str,
    /// A ring, not a disc.
    pub hollow: bool,
}

/// The shade between two lines, in another colour where they have crossed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StudyFill {
    pub upper: &'static str,
    pub lower: &'static str,
    pub key: &'static str,
    pub color: &'static str,
    pub crossed_key: &'static str,
    pub crossed_color: &'static str,
}

impl StudyFill {
    /// One colour, whichever line is on top: a channel's background.
    const fn between(upper: &'static str, lower: &'static str, key: &'static str, color: &'static str) -> Self {
        Self { upper, lower, key, color, crossed_key: key, crossed_color: color }
    }
}

/// A stretch of time and the range price traded in it.
#[derive(Clone, Debug, PartialEq)]
pub struct StudyBox {
    pub from: i64,
    pub to: i64,
    pub top: f64,
    pub bottom: f64,
    pub key: &'static str,
    pub label: &'static str,
    pub color: &'static str,
}

#[derive(Clone, Debug)]
pub struct ComputedStudy {
    pub id: String,
    pub kind: StudyKind,
    pub label: String,
    pub group: Group,
    pub params: Vec<f64>,
    pub lines: Vec<StudyLine>,
    pub fills: Vec<StudyFill>,
    pub boxes: Vec<StudyBox>,
    pub segments: Vec<StudySegment>,
    pub markers: Vec<StudyMarker>,
    pub stats: Vec<(&'static str, String)>,
    /// Candles drawn in the study's pane, one per bar — the forming one included.
    pub candles: Vec<Option<[f64; 4]>>,
}

pub fn compute_study(spec: &StudySpec, candles: &[Candle]) -> ComputedStudy {
    compute_study_with(spec, candles, &StudyInputs::default())
}

/// A study named and listed, with nothing worked out: what a hidden one holds
/// until it is shown, its bars unread.
pub fn idle_study(spec: &StudySpec) -> ComputedStudy {
    let def = spec.def();
    ComputedStudy { id: spec.id(), kind: def.kind, label: spec.label(), group: def.group, params: spec.params.clone(), lines: Vec::new(), fills: Vec::new(), boxes: Vec::new(), segments: Vec::new(), markers: Vec::new(), stats: Vec::new(), candles: Vec::new() }
}

pub fn compute_study_with(spec: &StudySpec, candles: &[Candle], inputs: &StudyInputs<'_>) -> ComputedStudy {
    let def = spec.def();
    let close: Series = candles.iter().map(|c| Some(c.close)).collect();
    let label = spec.label();
    let param = |i: usize| spec.param(i);
    let mut study = idle_study(spec);

    match def.kind {
        StudyKind::Ema => study.lines.push(StudyLine::line("ema", label, indicators::ema(&close, param(0)))),
        StudyKind::Sma => study.lines.push(StudyLine::line("sma", label, indicators::sma(&close, param(0)))),
        StudyKind::Bb => {
            let b = indicators::bollinger(&close, param(0), param(1) as f64);
            study.lines = vec![StudyLine::line("upper", "Upper", b.upper), StudyLine::line("basis", "Basis", b.basis), StudyLine::line("lower", "Lower", b.lower)];
            study.fills = vec![StudyFill::between("upper", "lower", "fill", BAND_FILL)];
        }
        StudyKind::Keltner => {
            let (basis, atr, k) = (indicators::ema(&close, param(0)), indicators::atr(candles, param(2)), spec.number(1));
            study.lines = vec![StudyLine::inked("upper", "Upper", mixed(&basis, &atr, 1.0, k), "#2962ff"), StudyLine::inked("basis", "Basis", basis.clone(), "#2962ff"), StudyLine::inked("lower", "Lower", mixed(&basis, &atr, 1.0, -k), "#2962ff")];
            study.fills = vec![StudyFill::between("upper", "lower", "fill", BAND_FILL)];
        }
        StudyKind::Ichimoku => (study.lines, study.fills) = ichimoku(candles, &close, [param(0), param(1), param(2)], param(3)),
        StudyKind::Sessions => (study.boxes, study.stats) = sessions(candles),
        StudyKind::Moon => study.markers = moon_marks(candles),
        StudyKind::VolumeDelta => (study.lines, study.stats) = volume_delta(candles, param(0) == 1),
        // The bars on screen are the chart's to know: it works the rows out as it
        // draws them, and these only carry the levels for the legend.
        StudyKind::VisibleProfile => {
            let level = |key, label, color| StudyLine { style: LineStyle::None, color: Some(color), badge: false, ..StudyLine::line(key, label, Vec::new()) };
            study.lines = vec![level("poc", "POC", "#e6f5d3"), level("vah", "VAH", "#7c847d"), level("val", "VAL", "#7c847d")];
        }
        StudyKind::Rsi => study.lines.push(StudyLine::line("rsi", label, indicators::rsi(&close, param(0)))),
        StudyKind::StochRsi => {
            let s = indicators::stoch_rsi(&close, param(0), param(1), param(2), param(3));
            study.lines = vec![StudyLine::line("k", "%K", s.k), StudyLine::line("d", "%D", s.d)];
        }
        StudyKind::Macd => {
            let m = indicators::macd(&close, param(0), param(1), param(2));
            // Histogram first so the lines are drawn over the bars.
            study.lines = vec![
                StudyLine { kind: LineType::Histogram, ..StudyLine::line("histogram", "Histogram", m.histogram) },
                StudyLine::line("macd", "MACD", m.macd),
                StudyLine::line("signal", "Signal", m.signal),
            ];
        }
        // Its marks are minutes the server measured, not anything the candles
        // hold: the chart draws them from what the page sent, and the study
        // being here is what says to draw them at all.
        StudyKind::Flow | StudyKind::Formula => {}
        StudyKind::Atr => study.lines.push(StudyLine::line("atr", label, indicators::atr(candles, param(0)))),
        StudyKind::KeyLevels => (study.lines, study.segments) = key_levels(candles, inputs.days),
        StudyKind::Avwap => study.lines = anchored_vwap(candles),
        StudyKind::VwapLevels => (study.lines, study.stats) = vwap_levels(inputs, param(0), param(1) == 1, spec.number(2)),
        StudyKind::Vwap => (study.lines, study.fills, study.stats) = vwap(candles, param(0).min(4), [spec.number(1), spec.number(2), spec.number(3)]),
        StudyKind::SwingMap => (study.lines, study.markers, study.stats) = swing_map(candles, param(0), param(1)),
        StudyKind::OpenInterest => {
            let usd = param(1) == 1;
            let (mut bars, closes) = open_interest_bars(candles, inputs);
            if usd {
                open_interest_usd(&mut bars, &closes);
            }
            let line = param(2) == 1;
            let pick = |k: usize| -> Series { bars.iter().map(|b| b.map(|b| b[k])).collect() };
            let hidden = |key: &'static str, label: &'static str, values: Series| StudyLine { style: LineStyle::None, badge: false, ..StudyLine::line(key, label, values) };
            let close = StudyLine { style: if line { LineStyle::Solid } else { LineStyle::None }, color: Some(OI_COLOR), width: 1.5, ..StudyLine::line("close", if usd { "OI $" } else { "OI" }, pick(3)) };
            study.lines = if line { vec![close] } else { vec![hidden("open", "O", pick(0)), hidden("high", "H", pick(1)), hidden("low", "L", pick(2)), close] };
            if !line {
                study.candles = bars;
            }
        }
    }
    study
}

const OI_COLOR: &str = "#f0b90b";
const BAND_FILL: &str = "#2196f31a";

/// How long a bar is: the shortest gap among the first few, so a weekend or a
/// listing gap at the start of the window is not taken for one.
fn bar_length(candles: &[Candle]) -> Option<i64> {
    candles.windows(2).take(32).map(|w| w[1].open_time - w[0].open_time).min()
}

/// `a` and `b` in these proportions, bar by bar.
fn mixed(a: &Series, b: &Series, of_a: f64, of_b: f64) -> Series {
    a.iter().zip(b).map(|(x, y)| Some(of_a * (*x)? + of_b * (*y)?)).collect()
}

/// The cloud as TradingView draws it: conversion and base lines at the bar, the
/// two leading spans ahead of it and price behind it by `shift` less one, so the
/// lines run past the last bar.
fn ichimoku(candles: &[Candle], close: &Series, lengths: [usize; 3], shift: usize) -> (Vec<StudyLine>, Vec<StudyFill>) {
    let n = candles.len();
    let highs: Series = candles.iter().map(|c| Some(c.high)).collect();
    let lows: Series = candles.iter().map(|c| Some(c.low)).collect();
    let mid = |len| mixed(&indicators::highest(&highs, len), &indicators::lowest(&lows, len), 0.5, 0.5);
    let (conversion, base, span_b) = (mid(lengths[0]), mid(lengths[1]), mid(lengths[2]));
    let span_a = mixed(&conversion, &base, 0.5, 0.5);
    let moved = shift - 1;
    let ahead = |v: &Series| -> Series {
        let mut out = vec![None; if n == 0 { 0 } else { moved }];
        out.extend_from_slice(v);
        out
    };
    let mut behind = vec![None; n];
    for i in moved..n {
        behind[i - moved] = close[i];
    }
    let unbadged = |line: StudyLine| StudyLine { badge: false, ..line };
    (
        vec![
            StudyLine::inked("conversion", "Conversion", conversion.clone(), "#2962ff"),
            StudyLine::inked("base", "Base", base.clone(), "#b71c1c"),
            unbadged(StudyLine::inked("lagging", "Lagging", behind, "#43a047")),
            unbadged(StudyLine::inked("leadA", "Lead A", ahead(&span_a), "#a5d6a7")),
            unbadged(StudyLine::inked("leadB", "Lead B", ahead(&span_b), "#ef9a9a")),
        ],
        vec![StudyFill { upper: "leadA", lower: "leadB", key: "cloudUp", color: "#43a04733", crossed_key: "cloudDown", crossed_color: "#f4433633" }],
    )
}

/// Asia, London and New York, each day's box from its first bar to its last,
/// from the high to the low traded in it. Hours as the TPO's sessions keep them,
/// London's and New York's following their summer time. A bar longer than an
/// hour cannot say which session it traded in.
fn sessions(candles: &[Candle]) -> (Vec<StudyBox>, Vec<(&'static str, String)>) {
    if bar_length(candles).unwrap_or(0) > HOUR {
        return (Vec::new(), vec![("Needs", "bars of an hour or less".to_string())]);
    }
    const NAMES: [(&str, &str, &str); 3] = [("asia", "Asia", "#e3cd2a"), ("london", "London", "#c8202e"), ("ny", "New York", "#2f5bd8")];
    let mut boxes: Vec<StudyBox> = Vec::new();
    let mut open: [Option<usize>; 3] = [None; 3];
    let mut hours: [(i64, i64); 3] = [(0, 0); 3];
    let mut today = None;
    for c in candles {
        let day = c.open_time.div_euclid(DAY) * DAY;
        if today != Some(day) {
            today = Some(day);
            hours = [(0, 8 * 60), london(day), new_york(day)];
            open = [None; 3];
        }
        let minute = (c.open_time - day) / MINUTE;
        for (s, &(start, end)) in hours.iter().enumerate() {
            if minute < start || minute >= end {
                continue;
            }
            match open[s] {
                Some(at) => {
                    let b = &mut boxes[at];
                    b.to = c.open_time;
                    b.top = b.top.max(c.high);
                    b.bottom = b.bottom.min(c.low);
                }
                None => {
                    let (key, label, color) = NAMES[s];
                    open[s] = Some(boxes.len());
                    boxes.push(StudyBox { from: c.open_time, to: c.open_time, top: c.high, bottom: c.low, key, label, color });
                }
            }
        }
    }
    (boxes, Vec::new())
}

/// A disc over the bar a full moon falls in, a ring under the bar a new one does.
fn moon_marks(candles: &[Candle]) -> Vec<StudyMarker> {
    let (Some(first), Some(last)) = (candles.first(), candles.last()) else { return Vec::new() };
    let bar = bar_length(candles).unwrap_or(DAY);
    indicators::moon::moon_phases(first.open_time, last.open_time + bar - 1)
        .into_iter()
        .map(|(at, full)| {
            let i = candles.partition_point(|c| c.open_time <= at) - 1;
            let (plot, ink) = if full { ("full", "#f5efc8") } else { ("new", "#9aa0a6") };
            StudyMarker { time: candles[i].open_time, above: full, text: String::new(), tone: Tone::Ink(ink), plot, hollow: !full }
        })
        .collect()
}

/// What market buyers took less what market sellers did, bar by bar or added up.
/// Exact where the exchange reports its buyers, as Binance does. Elsewhere it is
/// estimated from where the bar closed inside its range, and says so.
fn volume_delta(candles: &[Candle], cumulative: bool) -> (Vec<StudyLine>, Vec<(&'static str, String)>) {
    let mut estimated = 0;
    let mut sum = 0.0;
    let values: Series = candles
        .iter()
        .map(|c| {
            let range = c.high - c.low;
            let delta = match c.taker_buy {
                Some(buy) => 2.0 * buy - c.volume,
                None if range > 0.0 => {
                    estimated += 1;
                    c.volume * (2.0 * (c.close - c.low) / range - 1.0)
                }
                None => 0.0,
            };
            sum += delta;
            (c.volume > 0.0).then_some(if cumulative { sum } else { delta })
        })
        .collect();
    let silent = !candles.is_empty() && values.iter().all(Option::is_none);
    let stats = if silent {
        vec![("Volume", "none in this market".to_string())]
    } else if estimated > 0 {
        vec![("Estimated", format!("{estimated} of {} bars", candles.len()))]
    } else {
        Vec::new()
    };
    let line = if cumulative { StudyLine::line("cvd", "CVD", values) } else { StudyLine { kind: LineType::Histogram, ..StudyLine::line("histogram", "Delta", values) } };
    (vec![line], stats)
}

/// Fold open interest steps into the chart's bars, the forming one last: each
/// bar is the candle of the steps that start inside it. A bar no step starts in
/// holds the last close flat; bars before the first step have none. In dollars
/// each figure is multiplied by the bar's close.
fn open_interest_bars(candles: &[Candle], inputs: &StudyInputs<'_>) -> (Vec<Option<[f64; 4]>>, Vec<f64>) {
    let steps = inputs.open_interest;
    let mut bars: Vec<(i64, f64)> = candles.iter().map(|c| (c.open_time, c.close)).collect();
    if let Some(live) = inputs.live.filter(|l| bars.last().is_none_or(|b| l.0 > b.0)) {
        bars.push(live);
    }
    let mut out = Vec::with_capacity(bars.len());
    let mut j = 0usize;
    let mut last: Option<f64> = None;
    for (i, &(open, _)) in bars.iter().enumerate() {
        let end = bars.get(i + 1).map_or(i64::MAX, |b| b.0);
        // Steps before this bar carry their close in, without drawing.
        while j < steps.len() && steps[j].0 < open {
            last = Some(steps[j].1[3]);
            j += 1;
        }
        let mut bar: Option<[f64; 4]> = None;
        while j < steps.len() && steps[j].0 < end {
            let s = steps[j].1;
            bar = Some(match bar {
                None => s,
                Some(b) => [b[0], b[1].max(s[1]), b[2].min(s[2]), s[3]],
            });
            j += 1;
        }
        let bar = bar.or_else(|| last.map(|c| [c, c, c, c]));
        if let Some(b) = bar {
            last = Some(b[3]);
        }
        out.push(bar);
    }
    (out, bars.iter().map(|b| b.1).collect())
}

/// The forming bar's open interest worked out again on its own, the bars before
/// it as they were: within one bar a tick moves only that bar's figure — in
/// dollars, by its price. False where the study is not of these bars and this
/// forming one, and is to be worked out whole.
pub fn open_interest_live(spec: &StudySpec, study: &mut ComputedStudy, candles: &[Candle], inputs: &StudyInputs<'_>) -> bool {
    let Some((open, close)) = inputs.live.filter(|l| candles.last().is_none_or(|c| l.0 > c.open_time)) else { return false };
    let n = candles.len() + 1;
    if spec.kind != StudyKind::OpenInterest || study.lines.is_empty() || study.lines.iter().any(|l| l.values.len() != n) || !(study.candles.is_empty() || study.candles.len() == n) {
        return false;
    }
    // The steps that start inside it, else the last close before it, held flat.
    let steps = inputs.open_interest;
    let first = steps.partition_point(|s| s.0 < open);
    let bar = steps[first..].iter().map(|s| s.1).reduce(|b, s| [b[0], b[1].max(s[1]), b[2].min(s[2]), s[3]]).or_else(|| first.checked_sub(1).map(|i| [steps[i].1[3]; 4]));
    let bar = bar.map(|b| if spec.param(1) == 1 { b.map(|v| v * close) } else { b });
    for line in &mut study.lines {
        let k = ["open", "high", "low", "close"].iter().position(|key| *key == line.key).unwrap_or(3);
        line.values[n - 1] = bar.map(|b| b[k]);
    }
    if let Some(last) = study.candles.last_mut() {
        *last = bar;
    }
    true
}

/// The dollar value of the open interest, bar by bar.
fn open_interest_usd(bars: &mut [Option<[f64; 4]>], closes: &[f64]) {
    for (bar, close) in bars.iter_mut().zip(closes) {
        if let Some(b) = bar {
            for v in b.iter_mut() {
                *v *= close;
            }
        }
    }
}

const WEEKLY: &str = "#fffcbc";
const MONTHLY: &str = "#08d48c";
const YEARLY: &str = "#ff9800";
const MONDAY: &str = "#cfcfcf";

/// `2d 4h`, `6h`, `3d` — to the hour.
pub fn format_duration(ms: f64) -> String {
    let total_hours = (ms / HOUR as f64).floor() as i64;
    let (days, hours) = (total_hours.div_euclid(24), total_hours.rem_euclid(24));
    match (days > 0, hours > 0) {
        (true, true) => format!("{days}d {hours}h"),
        (true, false) => format!("{days}d"),
        _ => format!("{hours}h"),
    }
}

/// Weekly / monthly / yearly opens and Monday's range ("Boyen_Clean Historical
/// Key Levels"). The FIRST period in view is partial — its real open is before
/// the data — so a run only starts once a boundary turns over inside the data.
///
/// Levels are read from `days` when the page has them, because a bar longer
/// than a day opens before the week, month or year it belongs to: the week of
/// 29 December opens in the old year, and its open is not the year's open.
fn key_levels(candles: &[Candle], days: &[Candle]) -> (Vec<StudyLine>, Vec<StudySegment>) {
    #[derive(Clone, Copy)]
    struct Run {
        price: f64,
        from: i64,
        to: i64,
    }
    #[derive(Clone, Copy)]
    struct Monday {
        high: f64,
        low: f64,
        week: i64,
        from: i64,
        to: i64,
    }

    let source = if days.is_empty() { candles } else { days };
    let (mut weeks, mut months, mut years) = (Vec::<Run>::new(), Vec::<Run>::new(), Vec::<Run>::new());
    let mut mondays = Vec::<Monday>::new();
    let (mut week, mut month, mut year): (Option<i64>, Option<i64>, Option<i64>) = (None, None, None);

    for c in source {
        let (w, m, y) = (week_index(c.open_time), month_index(c.open_time), year_of(c.open_time));
        let start = Run { price: c.open, from: c.open_time, to: c.open_time };
        for (runs, turned) in [(&mut weeks, week.is_some_and(|p| p != w)), (&mut months, month.is_some_and(|p| p != m)), (&mut years, year.is_some_and(|p| p != y))] {
            if turned {
                runs.push(start);
            }
            if let Some(run) = runs.last_mut() {
                run.to = c.open_time;
            }
        }
        (week, month, year) = (Some(w), Some(m), Some(y));

        if weekday(c.open_time) == 1 && mondays.last().is_none_or(|mon| mon.week != w) {
            mondays.push(Monday { high: c.high, low: c.low, week: w, from: c.open_time, to: c.open_time });
        }
        if let Some(mon) = mondays.last_mut().filter(|mon| mon.week == w) {
            if weekday(c.open_time) == 1 {
                mon.high = mon.high.max(c.high);
                mon.low = mon.low.min(c.low);
            }
            mon.to = c.open_time;
        }
    }

    let mut segments = Vec::new();
    for (runs, key, label, color) in [(&weeks, "wo", "WO", WEEKLY), (&months, "mo", "MO", MONTHLY), (&years, "yo", "YO", YEARLY)] {
        segments.extend(runs.iter().map(|r| StudySegment { from: r.from, to: r.to, price: r.price, key, label, color }));
    }
    // Only the current week's Monday is still a reference.
    let last_week = source.last().map(|c| week_index(c.open_time));
    if let Some(mon) = mondays.last().filter(|mon| last_week == Some(mon.week)) {
        segments.push(StudySegment { from: mon.from, to: mon.to, price: mon.high, key: "monH", label: "Mon H", color: MONDAY });
        segments.push(StudySegment { from: mon.from, to: mon.to, price: mon.low, key: "monL", label: "Mon L", color: MONDAY });
    }

    // What each of the chart's own bars reads: the level in force as it closed,
    // so a bar that straddles the turn already carries the new one.
    let n = candles.len();
    let (mut wo, mut mo, mut yo, mut mon_high, mut mon_low) = (vec![None; n], vec![None; n], vec![None; n], vec![None; n], vec![None; n]);
    let (mut wi, mut mi, mut yi, mut di) = (0, 0, 0, 0);
    let walk = |runs: &[Run], at: &mut usize, t: i64| -> Option<f64> {
        while *at + 1 < runs.len() && runs[*at + 1].from <= t {
            *at += 1;
        }
        runs.get(*at).filter(|r| r.from <= t).map(|r| r.price)
    };
    for (i, c) in candles.iter().enumerate() {
        let end = candles.get(i + 1).map_or(i64::MAX, |next| next.open_time - 1);
        wo[i] = walk(&weeks, &mut wi, end);
        mo[i] = walk(&months, &mut mi, end);
        yo[i] = walk(&years, &mut yi, end);
        while di + 1 < mondays.len() && mondays[di + 1].from <= end {
            di += 1;
        }
        // A bar reads Monday's range only while it is in Monday's own week.
        if let Some(mon) = mondays.get(di).filter(|mon| mon.from <= end && mon.week == week_index(c.open_time)) {
            (mon_high[i], mon_low[i]) = (Some(mon.high), Some(mon.low));
        }
    }

    let hidden = |key: &'static str, label: &'static str, values: Series, color: &'static str| StudyLine { style: LineStyle::None, color: Some(color), ..StudyLine::line(key, label, values) };
    (
        vec![hidden("wo", "WO", wo, WEEKLY), hidden("mo", "MO", mo, MONTHLY), hidden("yo", "YO", yo, YEARLY), hidden("monH", "Mon H", mon_high, MONDAY), hidden("monL", "Mon L", mon_low, MONDAY)],
        segments,
    )
}

/// Which session, week, month, year or quarter a time falls in.
pub fn anchor_period(t: i64, anchor: usize) -> i64 {
    match anchor {
        0 => t.div_euclid(DAY),
        1 => week_index(t),
        2 => month_index(t),
        3 => year_of(t),
        _ => month_index(t).div_euclid(3),
    }
}

/// VWAP of hlc3 since `anchor` last turned over, and the volume-weighted
/// deviation of price around it. A period with no volume has neither: never the
/// last price.
fn anchored(candles: &[Candle], anchor: usize) -> (Series, Series) {
    let mut turn = None;
    let (mut volume, mut sum, mut squares) = (0.0f64, 0.0f64, 0.0f64);
    candles
        .iter()
        .map(|c| {
            let now = anchor_period(c.open_time, anchor);
            if turn != Some(now) {
                turn = Some(now);
                (volume, sum, squares) = (0.0, 0.0, 0.0);
            }
            let typical = (c.high + c.low + c.close) / 3.0;
            volume += c.volume;
            sum += typical * c.volume;
            squares += typical * typical * c.volume;
            let vwap = sum / volume;
            if volume > 0.0 { (Some(vwap), Some((squares / volume - vwap * vwap).max(0.0).sqrt())) } else { (None, None) }
        })
        .unzip()
}

/// VWAP since the session, week, month and year opened ("Boyen_Multiple Anchor
/// VWAP"). The session line is empty unless bars are intraday — measured from
/// the data, so it stays right for folded timeframes no label covers.
fn anchored_vwap(candles: &[Candle]) -> Vec<StudyLine> {
    let intraday = bar_length(candles).is_some_and(|bar| bar < DAY);
    let colored = |key, label, anchor, color, width| StudyLine { color: Some(color), width, ..StudyLine::line(key, label, anchored(candles, anchor).0) };
    let mut session = colored("session", "Session", 0, "#2962ff", 1.0);
    if !intraday {
        session.values.fill(None);
    }
    vec![session, colored("weekly", "Weekly", 1, "#ffa726", 2.0), colored("monthly", "Monthly", 2, "#e040fb", 2.0), colored("yearly", "Yearly", 3, "#e8e8e8", 2.0)]
}

/// The standard VWAP, as TradingView's: dotted, with bands at standard deviations
/// of price around it, shaded, the first of them read as the value area (about
/// two thirds of the volume). Each period's last VWAP, VAH and VAL are carried
/// into the next and labelled at the right. Sessions are UTC days, as the
/// exchanges' daily bars are.
///
/// It draws nothing where it could not be right, and says why: an anchor no longer
/// than a bar starts again on every bar, a market with no volume has no VWAP, and
/// a period whose start the chart does not hold is an average of its later bars.
fn vwap(candles: &[Candle], anchor: usize, bands: [f64; 3]) -> (Vec<StudyLine>, Vec<StudyFill>, Vec<(&'static str, String)>) {
    let bar = bar_length(candles).unwrap_or(0);
    let fits = bar < [DAY, 7 * DAY, 28 * DAY, 365 * DAY, 89 * DAY][anchor];
    let (mut mid, deviation) = anchored(candles, anchor);
    if !fits {
        mid.fill(None);
    } else if let Some(first) = candles.first().filter(|c| anchor_period(c.open_time - bar, anchor) == anchor_period(c.open_time, anchor)) {
        let partial = anchor_period(first.open_time, anchor);
        let held = candles.iter().take_while(|c| anchor_period(c.open_time, anchor) == partial).count();
        mid[..held].fill(None);
    }
    let stats = if !fits {
        vec![("Anchor", "needs shorter bars".to_string())]
    } else if !candles.is_empty() && mid.iter().all(Option::is_none) {
        if candles.iter().any(|c| c.volume > 0.0) {
            vec![("Needs", "bars back to its start: scroll left".to_string())]
        } else {
            vec![("Volume", "none in this market".to_string())]
        }
    } else {
        Vec::new()
    };

    let mut lines = vec![StudyLine { style: LineStyle::Anchored(anchor), pattern: Pattern::Dotted, color: Some("#2962ff"), width: 1.5, ..StudyLine::line("vwap", "VWAP", mid.clone()) }];
    let mut fills = Vec::new();
    let edges = [("upper1", "lower1", "fill1", "#6b7fb8", "#2962ff12"), ("upper2", "lower2", "fill2", "#5a6da3", "#2962ff0c"), ("upper3", "lower3", "fill3", "#4a5c8f", "#2962ff08")];
    for (multiple, (upper, lower, fill, color, shade)) in bands.into_iter().zip(edges) {
        if multiple > 0.0 {
            for (key, k) in [(upper, multiple), (lower, -multiple)] {
                let values = mid.iter().zip(&deviation).map(|(m, d)| Some((*m)? + k * (*d)?)).collect();
                lines.push(StudyLine { style: LineStyle::Anchored(anchor), color: Some(color), badge: false, ..StudyLine::line(key, key, values) });
            }
            fills.push(StudyFill::between(upper, lower, fill, shade));
        }
    }

    // What each bar's period opened under: the last figures of the one before it.
    let mut before: Option<(f64, f64)> = None;
    let mut held = Vec::with_capacity(candles.len());
    for i in 0..candles.len() {
        if i > 0 && anchor_period(candles[i - 1].open_time, anchor) != anchor_period(candles[i].open_time, anchor) {
            before = mid[i - 1].zip(deviation[i - 1]);
        }
        held.push(before);
    }
    let [high, middle, low] = VWAP_PREVIOUS[anchor];
    let tag = |key, label, k: f64, color| StudyLine { style: LineStyle::Tag, color: Some(color), badge: false, ..StudyLine::line(key, label, held.iter().map(|h| h.map(|(m, d)| m + k * d)).collect()) };
    if bands[0] > 0.0 {
        lines.push(tag("prevVah", high, bands[0], "#6b7fb8"));
    }
    lines.push(tag("prevVwap", middle, 0.0, "#5b8cff"));
    if bands[0] > 0.0 {
        lines.push(tag("prevVal", low, -bands[0], "#6b7fb8"));
    }
    (lines, fills, stats)
}

/// Each period's labels, high to low, then the previous period's: `pmVAH` is last month's.
const LEVELS: [(usize, &str, [[&str; 3]; 2]); 5] = [
    (0, "#9aa0a6", [["dVAH", "dVWAP", "dVAL"], ["pdVAH", "pdVWAP", "pdVAL"]]),
    (1, "#d8d29a", [["wVAH", "wVWAP", "wVAL"], ["pwVAH", "pwVWAP", "pwVAL"]]),
    (2, "#5fc79d", [["mVAH", "mVWAP", "mVAL"], ["pmVAH", "pmVWAP", "pmVAL"]]),
    (4, "#b98be0", [["qVAH", "qVWAP", "qVAL"], ["pqVAH", "pqVWAP", "pqVAL"]]),
    (3, "#e0a050", [["yVAH", "yVWAP", "yVAL"], ["pyVAH", "pyVWAP", "pyVAL"]]),
];

/// The VWAP and value area of the day, week, month, quarter and year, and of
/// the one before each, as labels at the right edge and nothing else. Worked
/// out from hourly bars for the day, week and month and from daily bars for the
/// quarter and year, not from the chart's own, so a minute chart has its year's
/// too. A period whose start the bars do not reach has no level.
fn vwap_levels(inputs: &StudyInputs<'_>, periods: usize, previous: bool, band: f64) -> (Vec<StudyLine>, Vec<(&'static str, String)>) {
    let mut lines = Vec::new();
    for (bit, (anchor, color, names)) in LEVELS.into_iter().enumerate() {
        let bars = if anchor >= 3 { inputs.daily } else { inputs.hourly };
        let (Some(first), Some(last)) = (bars.first(), bars.last()) else { continue };
        if periods & (1 << bit) == 0 {
            continue;
        }
        let (mid, deviation) = anchored(bars, anchor);
        let period = |i: usize| anchor_period(bars[i].open_time, anchor);
        // The bars' first period began before them unless a bar ahead of the first would fall in another.
        let partial = anchor_period(first.open_time - bar_length(bars).unwrap_or(0), anchor) == period(0);
        let now = anchor_period(last.open_time, anchor);
        let before = (0..bars.len()).rev().find(|&i| period(i) != now);
        for (at, names) in [(Some(bars.len() - 1), names[0]), (before.filter(|_| previous), names[1])] {
            let Some(i) = at.filter(|&i| !(partial && period(i) == period(0))) else { continue };
            let (Some(m), Some(d)) = (mid[i], deviation[i]) else { continue };
            for (name, k) in names.into_iter().zip([band, 0.0, -band]) {
                lines.push(StudyLine { style: LineStyle::Tag, color: Some(color), badge: false, ..StudyLine::line(name, name, vec![Some(m + k * d)]) });
            }
        }
    }
    let silent = !inputs.hourly.is_empty() && inputs.hourly.iter().chain(inputs.daily).all(|c| c.volume <= 0.0);
    (lines, if silent { vec![("Volume", "none in this market".to_string())] } else { Vec::new() })
}

/// Zigzag of CONFIRMED pivots tagged HH/HL/LH/LL ("Boyen_Range Swing Structure
/// Map"). The original's live, unconfirmed preview is not ported: a tentative
/// swing is a repaint, so the last leg ends `right` bars back.
fn swing_map(candles: &[Candle], left: usize, right: usize) -> (Vec<StudyLine>, Vec<StudyMarker>, Vec<(&'static str, String)>) {
    #[derive(Clone, Copy)]
    struct Point {
        index: usize,
        price: f64,
        high: bool,
        time: i64,
    }
    let n = candles.len();
    let highs: Series = candles.iter().map(|c| Some(c.high)).collect();
    let lows: Series = candles.iter().map(|c| Some(c.low)).collect();
    let mut pivots: Vec<Point> = indicators::pivot_highs(&highs, left, right)
        .into_iter()
        .map(|p| Point { index: p.index, price: p.value, high: true, time: candles[p.index].open_time })
        .chain(indicators::pivot_lows(&lows, left, right).into_iter().map(|p| Point { index: p.index, price: p.value, high: false, time: candles[p.index].open_time }))
        .collect();
    // A bar can be both: its high first.
    crate::sort::heap_sort_by(&mut pivots, |a, b| (a.index, !a.high) < (b.index, !b.high));

    // Two of the same type in a row are one swing seen twice: keep the extreme.
    let mut points: Vec<Point> = Vec::new();
    for p in pivots {
        match points.last_mut() {
            Some(last) if last.high == p.high => {
                if if p.high { p.price > last.price } else { p.price < last.price } {
                    *last = p;
                }
            }
            _ => points.push(p),
        }
    }

    let mut zigzag = vec![None; n];
    let mut markers = Vec::new();
    let (mut up_pct, mut up_dur, mut dn_pct, mut dn_dur) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, p) in points.iter().enumerate() {
        zigzag[p.index] = Some(p.price);
        let mut text = points[..i]
            .iter()
            .rev()
            .find(|prev| prev.high == p.high)
            .map_or(if p.high { "H" } else { "L" }, |prev| match (p.high, p.price > prev.price) {
                (true, true) => "HH",
                (true, false) => "LH",
                (false, true) => "HL",
                (false, false) => "LL",
            })
            .to_string();
        if i > 0 {
            let prev = points[i - 1];
            let pct = (p.price - prev.price) / prev.price * 100.0;
            let dur = (p.time - prev.time) as f64;
            text += &format!(" {}{}% {}", if pct >= 0.0 { "+" } else { "" }, fixed(pct, 2), format_duration(dur));
            if p.high {
                up_pct.push(pct);
                up_dur.push(dur);
            } else {
                dn_pct.push(pct);
                dn_dur.push(dur);
            }
            let span = p.index - prev.index;
            for k in 1..span {
                zigzag[prev.index + k] = Some(prev.price + (p.price - prev.price) * k as f64 / span as f64);
            }
        }
        markers.push(StudyMarker { time: p.time, above: p.high, text, tone: if p.high { Tone::Bear } else { Tone::Bull }, plot: "labels", hollow: false });
    }

    let avg = |xs: &[f64]| (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64);
    let na = || "n/a".to_string();
    let stats = vec![
        ("Swings", points.len().to_string()),
        ("Avg rise", avg(&up_pct).map_or_else(na, |v| format!("+{}%", fixed(v, 2)))),
        ("Avg rise time", avg(&up_dur).map_or_else(na, format_duration)),
        ("Avg drop", avg(&dn_pct).map_or_else(na, |v| fixed(v, 2) + "%")),
        ("Avg drop time", avg(&dn_dur).map_or_else(na, format_duration)),
    ];
    let line = StudyLine { color: Some("#8ab4f8"), width: 2.0, ..StudyLine::line("zigzag", "Swings", zigzag) };
    (vec![line], markers, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_iso;

    fn ids(raw: &str) -> Vec<String> {
        parse_studies(raw, 12).iter().map(StudySpec::id).collect()
    }

    fn series(start: &str, count: usize, step: i64, shape: impl Fn(usize) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>)) -> Vec<Candle> {
        let t0 = parse_iso(start).unwrap();
        (0..count)
            .map(|i| {
                let base = 100.0 + i as f64;
                let (o, h, l, c, v) = shape(i);
                Candle { open_time: t0 + i as i64 * step, open: o.unwrap_or(base), high: h.unwrap_or(base + 1.0), low: l.unwrap_or(base - 1.0), close: c.unwrap_or(base), volume: v.unwrap_or(1.0), quote_volume: None, taker_buy: None }
            })
            .collect()
    }

    #[test]
    fn open_interest_folds_steps_into_bars_and_the_forming_one() {
        let hour = 3_600_000;
        let candles = series("2026-09-01", 3, hour, |i| (None, None, None, Some(100.0 * (i + 1) as f64), None));
        let t0 = candles[0].open_time;
        let m = 15 * 60_000;
        // Four 15-minute steps in the first hour, none in the second, one in the third, one in the forming fourth.
        let steps = vec![
            (t0 - m, [9.0, 9.0, 9.0, 9.0]),
            (t0, [10.0, 12.0, 10.0, 11.0]),
            (t0 + m, [11.0, 15.0, 11.0, 14.0]),
            (t0 + 2 * m, [14.0, 14.0, 8.0, 9.0]),
            (t0 + 3 * m, [9.0, 10.0, 9.0, 10.0]),
            (t0 + 2 * hour, [20.0, 21.0, 19.0, 20.0]),
            (t0 + 3 * hour, [30.0, 30.0, 30.0, 30.0]),
        ];
        let inputs = StudyInputs { open_interest: &steps, live: Some((t0 + 3 * hour, 400.0)), ..StudyInputs::default() };
        let study = compute_study_with(&parse_studies("oi", 12)[0], &candles, &inputs);
        assert_eq!(study.candles, vec![Some([10.0, 15.0, 8.0, 10.0]), Some([10.0; 4]), Some([20.0, 21.0, 19.0, 20.0]), Some([30.0; 4])]);
        assert_eq!(line(&study, "close"), &vec![Some(10.0), Some(10.0), Some(20.0), Some(30.0)]);
        assert!(study.label.starts_with("Aggregated OI Coins Candles"));

        // In dollars each bar is multiplied by its close; as a line there are no candles.
        let usd = compute_study_with(&parse_studies("oi:3:1:1", 12)[0], &candles, &inputs);
        assert!(usd.candles.is_empty());
        assert_eq!(line(&usd, "close"), &vec![Some(1000.0), Some(2000.0), Some(6000.0), Some(12_000.0)]);
        assert!(usd.label.contains("USD Line · 2 venues"));
    }

    #[test]
    fn a_tick_in_the_forming_bar_moves_its_open_interest_as_the_whole_would() {
        let hour = 3_600_000;
        let candles = series("2026-09-01", 3, hour, |i| (None, None, None, Some(100.0 * (i + 1) as f64), None));
        let t0 = candles[0].open_time;
        let forming = t0 + 3 * hour;
        let steps = vec![(t0, [10.0, 12.0, 10.0, 11.0]), (t0 + 2 * hour, [20.0, 21.0, 19.0, 20.0]), (forming, [30.0, 33.0, 29.0, 31.0]), (forming + 900_000, [31.0, 35.0, 31.0, 34.0])];
        let quiet = vec![(t0, [10.0, 12.0, 10.0, 11.0]), (t0 + 2 * hour, [20.0, 21.0, 19.0, 20.0])];
        for raw in ["oi", "oi:255:1:0", "oi:255:1:1", "oi:255:0:1"] {
            let spec = &parse_studies(raw, 12)[0];
            for steps in [&steps, &quiet] {
                let before = StudyInputs { open_interest: steps, live: Some((forming, 400.0)), ..StudyInputs::default() };
                let after = StudyInputs { open_interest: steps, live: Some((forming, 415.5)), ..StudyInputs::default() };
                let mut study = compute_study_with(spec, &candles, &before);
                assert!(open_interest_live(spec, &mut study, &candles, &after), "{raw}: the forming bar alone");
                let whole = compute_study_with(spec, &candles, &after);
                assert_eq!(study.candles, whole.candles, "{raw}");
                for (a, b) in study.lines.iter().zip(&whole.lines) {
                    assert_eq!(a.values, b.values, "{raw} {}", a.key);
                }
            }
        }
        // A forming bar not yet in the study: worked out whole.
        let spec = &parse_studies("oi", 12)[0];
        let mut closed = compute_study_with(spec, &candles, &StudyInputs { open_interest: &steps, ..StudyInputs::default() });
        assert!(!open_interest_live(spec, &mut closed, &candles, &StudyInputs { open_interest: &steps, live: Some((forming, 400.0)), ..StudyInputs::default() }));
        assert!(idle_study(spec).lines.is_empty(), "a hidden study holds nothing");
    }

    fn line<'a>(study: &'a ComputedStudy, key: &str) -> &'a Series {
        &study.lines.iter().find(|l| l.key == key).unwrap_or_else(|| panic!("missing line {key}")).values
    }

    #[test]
    fn parsing_is_forgiving_and_exact() {
        assert_eq!(ids("ema:50,notathing:3,rsi:14"), ["ema:50", "rsi:14"]);
        assert_eq!(ids("macd"), ["macd:12:26:9"]);
        assert_eq!(ids("ema:1,ema:50"), ["ema:50"]);
        assert_eq!(ids("rsi:14,rsi:14").len(), 1);
        assert_eq!(parse_studies("stochrsi", 12)[0].label(), "Stoch RSI 14 14 3 3");
        assert_eq!(parse_studies("keylevels", 12)[0].label(), "Key Levels");
    }

    #[test]
    fn a_study_is_index_aligned_and_macd_draws_histogram_first() {
        let candles = series("2026-08-01", 40, DAY, |_| (None, None, None, None, None));
        let out = compute_study(&parse_studies("ema:10", 12)[0], &candles);
        assert_eq!((out.id.as_str(), out.group), ("ema:10", Group::Price));
        assert_eq!(out.lines[0].values.len(), 40);
        assert_eq!(out.lines[0].values[0], None);
        assert!(out.lines[0].values[39].is_some());

        let candles = series("2026-08-01", 61, DAY, |i| (None, None, None, Some(100.0 + (i as f64).sin() * 5.0), None));
        let out = compute_study(&parse_studies("macd", 12)[0], &candles);
        assert_eq!(out.lines.iter().map(|l| l.key).collect::<Vec<_>>(), ["histogram", "macd", "signal"]);
        assert_eq!(out.lines[0].kind, LineType::Histogram);
    }

    #[test]
    fn key_levels_skip_the_partial_first_period() {
        let candles = series("2026-08-03", 21, DAY, |_| (None, None, None, None, None));
        let study = compute_study(&parse_studies("keylevels", 12)[0], &candles);
        let wo = line(&study, "wo");
        assert!(wo[..7].iter().all(Option::is_none));
        assert_eq!(wo[7], Some(candles[7].open));
        assert!(wo[7..14].iter().all(|v| *v == Some(candles[7].open)));
        assert_ne!(wo[14], wo[13]);

        let weekly: Vec<_> = study.segments.iter().filter(|s| s.label == "WO").collect();
        assert!(weekly.len() >= 2);
        for w in weekly.windows(2) {
            assert!(w[1].from > w[0].to - 1);
        }
        assert!(weekly.iter().all(|s| s.to > s.from && s.to - s.from <= 7 * DAY));
        assert!(study.lines.iter().all(|l| l.style == LineStyle::None));
        let mondays: Vec<_> = study.segments.iter().filter(|s| s.label.starts_with("Mon")).collect();
        assert_eq!(mondays.len(), 2);
        assert_eq!(mondays[0].to, candles[20].open_time);

        let spiky = series("2026-08-03", 14, DAY, |i| if i == 1 { (None, Some(9999.0), Some(1.0), None, None) } else { (None, None, None, None, None) });
        let s = compute_study(&parse_studies("keylevels", 12)[0], &spiky);
        let mon_h = line(&s, "monH");
        assert_eq!((mon_h[0], mon_h[1], mon_h[7]), (Some(spiky[0].high), Some(spiky[0].high), Some(spiky[7].high)));
    }

    #[test]
    fn weekly_bars_take_their_levels_from_the_daily_bars_under_them() {
        // Weeks open on Monday, so the week holding 1 January opens in the old
        // year: on a weekly chart the year's open is inside the first bar.
        let days = series("2025-12-29", 14, DAY, |i| (Some(1000.0 + i as f64), None, None, None, None));
        let weeks = series("2025-12-29", 2, 7 * DAY, |i| (Some(days[i * 7].open), None, None, None, None));
        let spec = &parse_studies("keylevels", 12)[0];

        let plain = compute_study_with(spec, &weeks, &StudyInputs::default());
        assert_eq!(line(&plain, "yo")[1], Some(weeks[1].open), "without dailies the year opens a week late, at 5 January");

        let study = compute_study_with(spec, &weeks, &StudyInputs { days: &days, ..StudyInputs::default() });
        let new_year = days[3].open; // 1 January
        assert_eq!(line(&study, "yo")[..], [Some(new_year), Some(new_year)], "both bars trade under the new year's open");
        let yo: Vec<_> = study.segments.iter().filter(|s| s.label == "YO").collect();
        assert_eq!(yo.len(), 1);
        assert_eq!((yo[0].from, yo[0].price), (days[3].open_time, new_year), "the level starts where the year did");

        // Monday's range is one day of the week, not the whole of it.
        let mon: Vec<_> = study.segments.iter().filter(|s| s.label == "Mon H").collect();
        assert_eq!(mon.len(), 1);
        assert_eq!(mon[0].price, days[7].high, "the current week's Monday");
        assert_eq!(line(&study, "monH")[0], Some(days[0].high), "the bar before reads its own week's Monday");
    }

    #[test]
    fn anchored_vwap_weights_and_resets() {
        let flat = series("2026-08-03", 10, DAY, |_| (None, Some(100.0), Some(100.0), Some(100.0), None));
        assert_eq!(line(&compute_study(&parse_studies("avwap", 12)[0], &flat), "weekly")[5], Some(100.0));

        let t0 = parse_iso("2026-08-03").unwrap();
        let heavy = vec![
            Candle { open_time: t0, open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1.0, quote_volume: None, taker_buy: None },
            Candle { open_time: t0 + DAY, open: 200.0, high: 200.0, low: 200.0, close: 200.0, volume: 99.0, quote_volume: None, taker_buy: None },
        ];
        assert!(line(&compute_study(&parse_studies("avwap", 12)[0], &heavy), "weekly")[1].unwrap() > 198.0);

        let daily = compute_study(&parse_studies("avwap", 12)[0], &series("2026-08-03", 10, DAY, |_| (None, None, None, None, None)));
        assert!(line(&daily, "session").iter().all(Option::is_none));
        let hourly = compute_study(&parse_studies("avwap", 12)[0], &series("2026-08-03", 10, HOUR, |_| (None, None, None, None, None)));
        assert!(line(&hourly, "session").iter().any(Option::is_some));

        let two_weeks = series("2026-08-03", 14, DAY, |i| {
            let p = if i < 7 { 100.0 } else { 200.0 };
            (None, Some(p), Some(p), Some(p), None)
        });
        let study = compute_study(&parse_studies("avwap", 12)[0], &two_weeks);
        assert_eq!(line(&study, "weekly")[7], Some(200.0));
        assert!(line(&study, "monthly")[7].unwrap() < 200.0);
    }

    /// `(high, low, close, volume)` bars, an hour apart from a UTC midnight.
    fn hourly(start: &str, bars: &[(f64, f64, f64, f64)]) -> Vec<Candle> {
        let t0 = parse_iso(start).unwrap();
        bars.iter().enumerate().map(|(i, &(high, low, close, volume))| Candle { open_time: t0 + i as i64 * HOUR, open: close, high, low, close, volume, quote_volume: None, taker_buy: None }).collect()
    }

    #[test]
    fn vwap_weights_by_volume_with_bands_at_its_weighted_deviation() {
        let candles = hourly("2026-08-03", &[(100.0, 100.0, 100.0, 1.0), (200.0, 200.0, 200.0, 3.0)]);
        let study = compute_study(&parse_studies("vwap:0:1:2:0", 12)[0], &candles);
        assert_eq!(study.id, "vwap:0:1:2:0");
        assert_eq!(study.label, "VWAP Session ±1 ±2");
        assert_eq!(study.lines.iter().map(|l| l.key).collect::<Vec<_>>(), ["vwap", "upper1", "lower1", "upper2", "lower2", "prevVah", "prevVwap", "prevVal"], "a band of 0 is left out");
        assert!(study.lines.iter().filter(|l| l.key != "vwap").all(|l| !l.badge), "only the VWAP itself gets a badge");
        assert_eq!(study.lines[0].pattern, Pattern::Dotted);
        assert_eq!(line(&study, "vwap"), &vec![Some(100.0), Some(175.0)]);
        // One lot 75 under the mean and three lots 25 over it: (75² + 3 × 25²) / 4.
        let deviation = 1875.0f64.sqrt();
        assert_eq!(line(&study, "upper1"), &vec![Some(100.0), Some(175.0 + deviation)]);
        assert_eq!(line(&study, "lower2"), &vec![Some(100.0), Some(175.0 - 2.0 * deviation)]);
        // A shade for each band in use.
        assert_eq!(study.fills.iter().map(|f| (f.upper, f.lower, f.key)).collect::<Vec<_>>(), [("upper1", "lower1", "fill1"), ("upper2", "lower2", "fill2")]);
    }

    fn at_midnight(day: &str, prices: &[f64]) -> Vec<Candle> {
        let t0 = parse_iso(day).unwrap();
        prices.iter().enumerate().map(|(i, &p)| Candle { open_time: t0 + i as i64 * 12 * HOUR, open: p, high: p, low: p, close: p, volume: 1.0, quote_volume: None, taker_buy: None }).collect()
    }

    #[test]
    fn vwap_starts_again_when_its_anchor_turns_over() {
        // Two 12-hour bars a day, a Monday first.
        let candles = at_midnight("2026-08-03", &[100.0, 200.0, 300.0, 500.0]);
        let session = compute_study(&parse_studies("vwap", 12)[0], &candles);
        assert_eq!(session.id, "vwap:0:1:0:0", "the one band on by default");
        assert_eq!(line(&session, "vwap"), &vec![Some(100.0), Some(150.0), Some(300.0), Some(400.0)]);
        assert!(matches!(session.lines[0].style, LineStyle::Anchored(0)));
        let week = compute_study(&parse_studies("vwap:1:1:0:0", 12)[0], &candles);
        assert_eq!(week.label, "VWAP Week ±1");
        assert_eq!(line(&week, "vwap"), &vec![Some(100.0), Some(150.0), Some(200.0), Some(275.0)], "the same week throughout");
    }

    #[test]
    fn vwap_levels_label_each_period_and_the_one_before_from_their_own_bars() {
        // Hourly bars from the first of September to the middle of October: September at 100, October at 200.
        let flat = |i: usize, september: usize| {
            let price = if i < september { 100.0 } else { 200.0 };
            (Some(price), Some(price), Some(price), Some(price), None)
        };
        let hourly = series("2026-09-01", 44 * 24, HOUR, |i| flat(i, 30 * 24));
        // Daily bars from the start of 2025 to the same day: 2025 at 100, 2026 at 200.
        let daily = series("2025-01-01", 365 + 286, DAY, |i| flat(i, 365));
        let inputs = StudyInputs { hourly: &hourly, daily: &daily, ..StudyInputs::default() };
        let spec = &parse_studies("vwaplevels", 12)[0];
        assert_eq!((spec.id(), spec.label()), ("vwaplevels:30:1:1".to_string(), "VWAP Levels".to_string()));
        let study = compute_study_with(spec, &[], &inputs);
        let level = |key: &str| study.lines.iter().find(|l| l.key == key).map(|l| l.values[0].unwrap());
        assert_eq!((level("mVWAP"), level("pmVWAP"), level("wVWAP"), level("pwVWAP")), (Some(200.0), Some(100.0), Some(200.0), Some(200.0)));
        assert_eq!((level("yVWAP"), level("pyVWAP"), level("qVWAP"), level("pqVWAP")), (Some(200.0), Some(100.0), Some(200.0), Some(200.0)));
        assert_eq!(level("dVWAP"), None, "the day is off unless asked for");
        assert!(study.lines.iter().all(|l| l.style == LineStyle::Tag && l.label == l.key));

        // Bars from the middle of September hold no whole September: no last month, this one still.
        let late = &hourly[14 * 24..];
        let study = compute_study_with(&parse_studies("vwaplevels:4:1:1", 12)[0], &[], &StudyInputs { hourly: late, ..StudyInputs::default() });
        assert_eq!(study.lines.iter().map(|l| l.key).collect::<Vec<_>>(), ["mVAH", "mVWAP", "mVAL"]);

        // The value area is a band of standard deviations: bars of equal volume taking turns at 100 and 200.
        let swing = series("2026-10-12", 48, HOUR, |i| {
            let price = if i % 2 == 0 { 100.0 } else { 200.0 };
            (Some(price), Some(price), Some(price), Some(price), None)
        });
        let study = compute_study_with(&parse_studies("vwaplevels:1:0:2", 12)[0], &[], &StudyInputs { hourly: &swing, ..StudyInputs::default() });
        let level = |key: &str| study.lines.iter().find(|l| l.key == key).and_then(|l| l.values[0]);
        assert_eq!((level("dVAH"), level("dVWAP"), level("dVAL"), level("pdVWAP")), (Some(250.0), Some(150.0), Some(50.0), None));

        let silent = series("2026-10-12", 48, HOUR, |_| (None, None, None, None, Some(0.0)));
        let study = compute_study_with(spec, &[], &StudyInputs { hourly: &silent, ..StudyInputs::default() });
        assert!(study.lines.is_empty() && study.stats == [("Volume", "none in this market".to_string())]);
    }

    #[test]
    fn each_period_opens_under_the_last_figures_of_the_one_before() {
        let candles = at_midnight("2026-08-03", &[100.0, 200.0, 300.0, 500.0, 700.0]);
        let study = compute_study(&parse_studies("vwap", 12)[0], &candles);
        let prev = line(&study, "prevVwap");
        assert_eq!(prev[..2], [None, None], "the first period has none before it");
        assert_eq!(prev[2..4], [Some(150.0), Some(150.0)], "Monday's last VWAP, through Tuesday");
        assert_eq!(prev[4], Some(400.0), "Tuesday's, on Wednesday");
        // Monday's two bars of 100 and 200: 50 either side.
        assert_eq!(line(&study, "prevVah")[2], Some(200.0));
        assert_eq!(line(&study, "prevVal")[2], Some(100.0));
        let tags: Vec<_> = study.lines.iter().filter(|l| l.style == LineStyle::Tag).map(|l| l.label.as_str()).collect();
        assert_eq!(tags, ["pDVAH", "pDVWAP", "pDVAL"]);
        let month = compute_study(&parse_studies("vwap:2:1:0:0", 12)[0], &[]);
        assert_eq!(month.lines.iter().filter(|l| l.style == LineStyle::Tag).map(|l| l.label.as_str()).collect::<Vec<_>>(), ["pMVAH", "pMVWAP", "pMVAL"]);
        // No value area, no value area levels.
        let bare = compute_study(&parse_studies("vwap:0:0:0:0", 12)[0], &candles);
        assert_eq!(bare.lines.iter().map(|l| l.key).collect::<Vec<_>>(), ["vwap", "prevVwap"]);
        assert!(bare.fills.is_empty());
    }

    #[test]
    fn a_quarter_turns_with_the_calendar() {
        let t0 = parse_iso("2026-09-30").unwrap();
        let candles: Vec<Candle> = (0..4).map(|i| Candle { open_time: t0 + i * DAY, open: 100.0, high: 100.0, low: 100.0, close: 100.0 + i as f64, volume: 1.0, quote_volume: None, taker_buy: None }).collect();
        let study = compute_study(&parse_studies("vwap:4:1:0:0", 12)[0], &candles);
        assert_eq!(study.label, "VWAP Quarter ±1");
        assert_eq!(line(&study, "vwap")[0], None, "September's quarter began before these bars");
        assert!(line(&study, "vwap")[1].is_some(), "October begins one");
        assert_eq!(anchor_period(parse_iso("2026-10-01").unwrap(), 4), anchor_period(parse_iso("2026-12-31").unwrap(), 4));
        assert_ne!(anchor_period(parse_iso("2026-09-30").unwrap(), 4), anchor_period(parse_iso("2026-10-01").unwrap(), 4));
    }

    #[test]
    fn a_period_whose_start_the_chart_does_not_hold_is_left_out() {
        // From Tuesday: this week's average would be of its later bars alone.
        let candles = at_midnight("2026-08-04", &[100.0, 200.0, 300.0, 500.0]);
        let week = compute_study(&parse_studies("vwap:1:1:0:0", 12)[0], &candles);
        assert!(line(&week, "vwap").iter().all(Option::is_none));
        assert_eq!(week.stats[0].0, "Needs", "and says to scroll back");
        // From noon: that day is left out, the next is whole.
        let t0 = parse_iso("2026-08-03").unwrap() + 12 * HOUR;
        let noon: Vec<Candle> = (0..3).map(|i| Candle { open_time: t0 + i * 12 * HOUR, open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1.0, quote_volume: None, taker_buy: None }).collect();
        let day = compute_study(&parse_studies("vwap", 12)[0], &noon);
        assert_eq!(line(&day, "vwap"), &vec![None, Some(100.0), Some(100.0)]);
        assert_eq!(line(&day, "prevVwap"), &vec![None, None, None], "nothing to carry from a day left out");
    }

    #[test]
    fn a_gap_at_the_start_is_not_taken_for_the_bars_length() {
        let mut bars = series("2026-08-03", 30, HOUR, |_| (None, None, None, None, None));
        bars[0].open_time -= 3 * DAY;
        let vwap = compute_study(&parse_studies("vwap", 12)[0], &bars);
        assert!(line(&vwap, "vwap").iter().any(Option::is_some) && vwap.stats.is_empty());
        assert!(!compute_study(&parse_studies("sessions", 12)[0], &bars).boxes.is_empty());
    }

    #[test]
    fn vwap_says_why_it_draws_nothing() {
        let days = series("2026-08-03", 5, DAY, |_| (None, None, None, None, None));
        let study = compute_study(&parse_studies("vwap", 12)[0], &days);
        assert!(line(&study, "vwap").iter().all(Option::is_none), "a session is no longer than a daily bar");
        assert_eq!(study.stats[0].0, "Anchor");
        let weekly = compute_study(&parse_studies("vwap:1:1:0:0", 12)[0], &days);
        assert!(line(&weekly, "vwap").iter().all(Option::is_some) && weekly.stats.is_empty(), "a week folds daily bars");

        let silent = series("2026-08-03", 5, HOUR, |_| (None, None, None, None, Some(0.0)));
        let study = compute_study(&parse_studies("vwap", 12)[0], &silent);
        assert!(line(&study, "vwap").iter().all(Option::is_none), "never the last price");
        assert_eq!(study.stats[0].0, "Volume");
        assert!(compute_study(&parse_studies("vwap", 12)[0], &[]).stats.is_empty(), "no bars yet is not a complaint");
    }

    #[test]
    fn keltner_is_an_ema_with_bands_at_multiples_of_atr() {
        let candles = series("2026-08-01", 60, DAY, |i| (None, Some(101.0 + i as f64), Some(99.0 + i as f64), Some(100.0 + i as f64), None));
        let study = compute_study(&parse_studies("keltner:20:2.5:10", 12)[0], &candles);
        assert_eq!(study.id, "keltner:20:2.5:10");
        assert_eq!(study.label, "KC 20 2.5 10");
        let close: Series = candles.iter().map(|c| Some(c.close)).collect();
        let (basis, atr) = (indicators::ema(&close, 20), indicators::atr(&candles, 10));
        assert_eq!(line(&study, "basis")[59], basis[59]);
        assert_eq!(line(&study, "upper")[59], Some(basis[59].unwrap() + 2.5 * atr[59].unwrap()));
        assert_eq!(line(&study, "lower")[59], Some(basis[59].unwrap() - 2.5 * atr[59].unwrap()));
        assert_eq!((study.fills[0].upper, study.fills[0].lower, study.fills[0].key), ("upper", "lower", "fill"));
        assert_eq!(parse_studies("keltner", 12)[0].id(), "keltner:20:2:10");
    }

    #[test]
    fn the_ichimoku_cloud_runs_ahead_of_the_bars_and_price_behind_them() {
        let candles = series("2026-08-01", 120, DAY, |i| (None, Some(110.0 + i as f64), Some(90.0 + i as f64), Some(100.0 + i as f64), None));
        let study = compute_study(&parse_studies("ichimoku", 12)[0], &candles);
        assert_eq!(study.id, "ichimoku:9:26:52:26");
        let (conversion, base, a, b, lag) = (line(&study, "conversion"), line(&study, "base"), line(&study, "leadA"), line(&study, "leadB"), line(&study, "lagging"));
        assert_eq!(conversion.len(), 120);
        // The mid of the last nine highs and lows.
        assert_eq!(conversion[119], Some(((110.0 + 119.0) + (90.0 + 111.0)) / 2.0));
        assert_eq!(base[119], Some(((110.0 + 119.0) + (90.0 + 94.0)) / 2.0));
        // Displacement 26 moves it 25 bars, as TradingView's offset does.
        assert_eq!(a.len(), 145, "past the last bar");
        assert_eq!(a[144], Some((conversion[119].unwrap() + base[119].unwrap()) / 2.0));
        assert_eq!(b[144], Some(((110.0 + 119.0) + (90.0 + 68.0)) / 2.0));
        assert_eq!(lag[94], Some(candles[119].close), "today's close, 25 bars back");
        assert_eq!(lag.len(), 120);
        assert_eq!(lag[95], None);
        assert!(study.lines.iter().filter(|l| l.badge).all(|l| l.key == "conversion" || l.key == "base"), "no axis badge for a value in the future");
        assert_eq!(study.fills[0].crossed_key, "cloudDown");
        let few = compute_study(&parse_studies("ichimoku", 12)[0], &[]);
        assert!(few.lines.iter().all(|l| l.values.is_empty()), "no bars, no lines");
    }

    #[test]
    fn sessions_box_each_day_from_first_bar_to_last_and_high_to_low() {
        let spiky = |i: usize| if i == 9 { (None, Some(999.0), Some(1.0), None, None) } else { (None, None, None, None, None) };
        // Midnight UTC on a summer weekday, 48 hours.
        let candles = series("2026-07-15", 48, HOUR, spiky);
        let study = compute_study(&parse_studies("sessions", 12)[0], &candles);
        assert_eq!(study.boxes.len(), 6, "three a day");
        let t0 = candles[0].open_time;
        let asia = &study.boxes[0];
        assert_eq!((asia.key, asia.from, asia.to), ("asia", t0, t0 + 7 * HOUR), "midnight to 08:00");
        let london = study.boxes.iter().find(|b| b.key == "london").unwrap();
        assert_eq!((london.from, london.to), (t0 + 7 * HOUR, t0 + 15 * HOUR), "07:00 to 15:30 in summer, in hourly bars");
        assert_eq!((london.top, london.bottom), (999.0, 1.0), "holds the spike at 09:00");
        let ny = study.boxes.iter().find(|b| b.key == "ny").unwrap();
        assert_eq!(ny.from, t0 + 14 * HOUR, "13:30 falls in the 14:00 bar's hour");
        let daily = compute_study(&parse_studies("sessions", 12)[0], &series("2026-07-15", 10, DAY, |_| (None, None, None, None, None)));
        assert!(daily.boxes.is_empty());
        assert_eq!(daily.stats[0].0, "Needs", "says why it draws nothing");
    }

    #[test]
    fn moon_phases_mark_the_bar_they_fall_in() {
        let candles = series("2024-04-01", 30, DAY, |_| (None, None, None, None, None));
        let study = compute_study(&parse_studies("moon", 12)[0], &candles);
        let new = study.markers.iter().find(|m| m.plot == "new").unwrap();
        assert_eq!(new.time, parse_iso("2024-04-08").unwrap());
        assert!(!new.above && new.hollow);
        let full = study.markers.iter().find(|m| m.plot == "full").unwrap();
        assert_eq!(full.time, parse_iso("2024-04-23").unwrap());
        assert!(full.above && !full.hollow);
        assert!(compute_study(&parse_studies("moon", 12)[0], &[]).markers.is_empty());
    }

    #[test]
    fn volume_delta_reads_the_exchanges_buyers_and_says_when_it_cannot() {
        let t0 = parse_iso("2026-08-03").unwrap();
        let bar = |i: i64, close: f64, volume: f64, taker_buy: Option<f64>| Candle { open_time: t0 + i * HOUR, open: 100.0, high: 110.0, low: 90.0, close, volume, quote_volume: None, taker_buy };
        let exact = [bar(0, 100.0, 10.0, Some(7.0)), bar(1, 100.0, 10.0, Some(2.0))];
        let study = compute_study(&parse_studies("volumedelta", 12)[0], &exact);
        assert_eq!(study.label, "Vol Delta");
        assert_eq!(study.group, Group::Delta);
        assert_eq!(line(&study, "histogram"), &vec![Some(4.0), Some(-6.0)]);
        assert_eq!(study.lines[0].kind, LineType::Histogram);
        assert!(study.stats.is_empty(), "the real thing");
        let cumulative = compute_study(&parse_studies("volumedelta:1", 12)[0], &exact);
        assert_eq!(cumulative.label, "CVD");
        assert_eq!(line(&cumulative, "cvd"), &vec![Some(4.0), Some(-2.0)]);

        // No buyers reported: from where it closed, top of the range all bought.
        let guess = [bar(0, 110.0, 10.0, None), bar(1, 100.0, 10.0, None), bar(2, 90.0, 10.0, None)];
        let study = compute_study(&parse_studies("volumedelta", 12)[0], &guess);
        assert_eq!(line(&study, "histogram"), &vec![Some(10.0), Some(0.0), Some(-10.0)]);
        assert_eq!(study.stats, vec![("Estimated", "3 of 3 bars".to_string())]);

        let silent = [bar(0, 100.0, 0.0, None)];
        let study = compute_study(&parse_studies("volumedelta", 12)[0], &silent);
        assert_eq!(line(&study, "histogram"), &vec![None]);
        assert_eq!(study.stats[0].0, "Volume");
    }

    #[test]
    fn a_multiplier_keeps_its_tenths_and_a_length_stays_whole() {
        assert_eq!(ids("bb:20:2.5"), ["bb:20:2.5"]);
        assert_eq!(ids("bb:20:0.3"), ["bb:20:0.3"], "not 0.30000000000000004");
        assert_eq!(ids("ema:50.4"), ["ema:50"]);
        assert_eq!(ids("bb"), ["bb:20:2"], "older ids read as they did");
        assert_eq!(parse_studies("bb:20:2.5", 12)[0].label(), "BB 20 2.5");
        assert_eq!(ids("bb:20:11,vwap:9"), Vec::<String>::new(), "out of range is dropped");
        assert_eq!(ids("oi:255:1:0"), ["oi:255:1:0"]);
    }

    #[test]
    fn swing_map_tags_confirmed_turns() {
        let mut prices = Vec::new();
        for leg in 0..4 {
            for _ in 0..10 {
                let last = prices.last().copied().unwrap_or(100.0);
                prices.push(if leg % 2 == 0 { last + 2.0 } else { last - 2.0 });
            }
        }
        let t0 = parse_iso("2026-08-03").unwrap();
        let candles: Vec<Candle> = prices
            .iter()
            .enumerate()
            .map(|(i, &p)| Candle { open_time: t0 + i as i64 * DAY, open: p, high: p + 0.5, low: p - 0.5, close: p, volume: 1.0, quote_volume: None, taker_buy: None })
            .collect();
        let study = compute_study(&parse_studies("swingmap:3:3", 12)[0], &candles);
        assert!(study.markers.len() >= 2);
        assert!(study.markers.iter().all(|m| ["HH", "HL", "LH", "LL", "H", "L"].contains(&m.text.split(' ').next().unwrap())));
        assert!(study.markers.iter().any(|m| m.above) && study.markers.iter().any(|m| !m.above));

        let zig = line(&study, "zigzag");
        let first = zig.iter().position(Option::is_some).unwrap();
        let last = zig.iter().rposition(Option::is_some).unwrap();
        assert!(zig[first..=last].iter().all(Option::is_some));
        assert_eq!(study.stats.iter().map(|s| s.0).collect::<Vec<_>>(), ["Swings", "Avg rise", "Avg rise time", "Avg drop", "Avg drop time"]);
        let last_marker = study.markers.last().unwrap();
        let idx = candles.iter().position(|c| c.open_time == last_marker.time).unwrap();
        assert!(idx <= candles.len() - 1 - 3);
    }

    #[test]
    fn durations_read_to_the_hour() {
        assert_eq!(format_duration((2 * DAY + 4 * HOUR) as f64), "2d 4h");
        assert_eq!(format_duration((3 * DAY) as f64), "3d");
        assert_eq!(format_duration((6 * HOUR + 59 * 60_000) as f64), "6h");
    }
}
