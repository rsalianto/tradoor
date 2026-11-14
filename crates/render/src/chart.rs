//! The chart: data, view, interaction, and the frame that draws them.


use tradoor_core::num::Within;
use tradoor_core::chart::{engine_series_with, EngineSeries};
use tradoor_core::format::{format_compact, format_countdown, format_price, format_turnover, price_precision};
use tradoor_core::live::LiveBar;
use tradoor_core::indicators::profile::value_area;
use tradoor_core::studies::{anchor_period, compute_study_with, idle_study, open_interest_live, parse_studies, ComputedStudy, Group, LineStyle, LineType, Pattern, StudyFill, StudyInputs, StudyKind, StudySpec, Tone};
use tradoor_core::time::DAY;
use tradoor_core::{Candle, ChartTimeframe, Interval};
use tradoor_core::timeframe::Source;

use crate::color::Color;
use crate::drawings::{self, Anchor, Dash, DrawingKind, DrawingShape, DrawingStyle, Placed, MAX_POINTS};
use crate::footprint;
use crate::geometry::{logical_to_time, ribbon_runs, time_to_logical, visible_slice, RibbonRun};
use crate::paint::{Align, Baseline, Frame, Rect};
use crate::scale::{PriceScale, ScaleMode, TimeScale, MIN_BAR_SPACING};
use crate::tpo;
use crate::theme::Theme;
use crate::ticks;

const TIME_AXIS_HEIGHT: f32 = 22.0;
const MIN_AXIS_WIDTH: f32 = 56.0;
const MIN_PANE_HEIGHT: f32 = 40.0;
const SEPARATOR_GRAB: f32 = 4.0;
/// A finger takes the footprint table's top edge from further off.
const TABLE_EDGE_TOUCH: f32 = 12.0;
/// A chart opens on this many bars: all of them at once makes every candle a
/// hairline, which shows the trend and nothing about any setup.
const DEFAULT_VISIBLE_BARS: f64 = 150.0;
/// The most bars a chart of 15 minutes or shorter shows at once, zoomed all
/// the way out — its draw distance. A frame costs what the bars on screen
/// cost, and a minute chart zoomed out to half a pixel a bar drew 4,800 of
/// them, three times as many as at this. Longer charts zoom out to `MIN_BAR_SPACING`.
const MAX_SHOWN_SHORT_BARS: f64 = 1_500.0;
const SHORT_BAR_MS: i64 = 15 * 60_000;
const RIGHT_MARGIN_BARS: f64 = 2.0;
/// The tradoor mark's size, in CSS pixels.
const MARK: f32 = 34.0;

/// Where the mark stands: bottom left of the price pane, where it has room —
/// above `bottom`, the footprint's stats table where it is shown, whose names it covered.
fn mark_at(plot: Rect, bottom: f32) -> Option<(f32, f32)> {
    (plot.w >= 160.0 && bottom - plot.y >= 110.0).then(|| (plot.x + 12.0, bottom - 12.0 - MARK))
}

/// How many bars past the last one a key level still in force runs, unless its settings say otherwise.
const LEVEL_AHEAD_BARS: f64 = 8.0;
const CLICK_SLOP: f32 = 5.0;
/// How near an anchor handle counts as grabbing it.
const HANDLE_HIT: f32 = 9.0;
/// How far a drag must move vertically before it takes an autoscaled pane's
/// price axis over, so a sideways pan with a shaky hand keeps autoscale.
const VERTICAL_PAN_SLOP: f32 = 24.0;
const HIT_TOLERANCE: f32 = 8.0;
/// A weak magnet takes a bar's open, high, low or close within this many pixels.
const MAGNET_PX: f32 = 14.0;
/// How large a flow mark is drawn, by how hard its minute ran.
const FLOW_MIN_RADIUS: f32 = 3.0;
const FLOW_MAX_RADIUS: f32 = 9.0;
/// How near an alert's line the pointer must be to take it. A hairline is hard
/// to hit, so the grab reaches further than the line is drawn.
const ALERT_HIT_PX: f32 = 7.0;
/// And a finger held on one, broader than a pointer and covering what it aims at.
const ALERT_HOLD_PX: f32 = 16.0;

/// TradingView's magnet: anchors placed or dragged land on a bar's open, high,
/// low or close — when near one (weak, elsewhere on a whole tick) or always
/// (strong) — and on the bar's own time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Magnet {
    Off,
    Weak,
    Strong,
}
const BRUSH_SAMPLE: f32 = 3.0;
/// How long a liquidation stays in the feed before it is gone.
const LIQUIDATION_TTL: i64 = 5_000;
/// A new hit slides the stack up over this long, and fades in as it arrives.
const LIQUIDATION_ENTER: i64 = 260;
/// The fade at the end of a hit's five seconds.
const LIQUIDATION_FADE: i64 = 900;
/// Rows of the feed that fit before the chart starts disappearing behind it.
const LIQUIDATION_ROWS: usize = 10;
/// Pings held at once; older ones drop off even before their time is up.
const MAX_LIQUIDATIONS: usize = 400;
/// Ask for older history once fewer than this many bars remain off the left edge.
const HISTORY_MARGIN_BARS: f64 = 80.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The last anchor landed; the page saves it. A `text` drawing still needs its label.
    /// `pane`: the lower pane it was drawn in, `None` for the price pane.
    DrawingCreated { kind: DrawingKind, points: Vec<Anchor>, pane: Option<Group> },
    SelectionChanged(Option<i64>),
    DeleteRequested(i64),
    /// Moved or reshaped by hand; the page saves its new anchors.
    DrawingChanged { id: i64, points: Vec<Anchor> },
    /// Double-clicked; the page opens its label for editing.
    EditRequested(i64),
    /// A tool finished or was cancelled; the palette falls back to select.
    ToolFinished,
    /// The view is near the oldest bar held.
    NeedHistory { before: i64 },
    PaneWeightsChanged(String),
    /// The footprint's table was dragged: how tall a row of it now stands.
    FootprintTableChanged(f32),
    /// Every pane follows the bars on screen again, or one stopped doing so.
    AutoScaleChanged(bool),
    /// A price alert's line was dragged to a new price.
    AlertMoved { id: i64, price: f64 },
}

/// A minute whose taker flow ran hard one way, marked where it traded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowMark {
    pub at: i64,
    pub price: f64,
    /// Net dollars: buyers took more above zero, sellers below.
    pub usd: f64,
}

/// An active price alert, drawn as a level across the price pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertLine {
    pub id: i64,
    pub price: f64,
    /// Its own colour, opacity and all; the theme's alert colour when none.
    pub color: Option<Color>,
    pub dash: Dash,
    pub width: f32,
}

impl AlertLine {
    /// A level as alerts are drawn unless styled: the theme's colour, dashed, a pixel wide.
    pub fn at(id: i64, price: f64) -> Self {
        Self { id, price, color: None, dash: Dash::Dashed, width: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Crosshair,
    Pointer,
    Grabbing,
    Move,
    ColResize,
    RowResize,
    NsResize,
}

impl Cursor {
    pub fn css(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Crosshair => "crosshair",
            Self::Pointer => "pointer",
            Self::Grabbing => "grabbing",
            Self::Move => "move",
            Self::ColResize => "ew-resize",
            Self::RowResize => "row-resize",
            Self::NsResize => "ns-resize",
        }
    }
}

/// The OHLC readout for the bar under the pointer, or the newest bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Legend {
    pub index: Option<usize>,
    pub time: i64,
    pub ohlc: String,
    pub change: String,
    pub up: bool,
    pub volume: String,
    /// `(studyId:lineKey, formatted value)`.
    pub values: Vec<(String, String)>,
    /// The pointer over the price pane: its y and the price there, for the
    /// page's add-alert button on the price axis.
    pub cursor: Option<(f32, f64)>,
    /// Where the price axis starts.
    pub axis_x: f32,
    /// Where the bar under the pointer stands, for a line the page holds on it.
    pub bar_x: Option<f32>,
    /// The alert line under the pointer, and its y.
    pub alert: Option<(i64, f32)>,
    /// The flow mark under the pointer: which one, and where it sits.
    pub flow: Option<(usize, f32, f32)>,
    /// The footprint cell under the pointer, as the screen groups it, for the
    /// page to hang its reading on.
    pub footprint: Option<FootprintCell>,
    /// The TPO profile under the pointer, for the page's menu on it.
    pub tpo: Option<tpo::Hover>,
    /// The tradoor mark's top left and its inks, where the page draws it over a
    /// live chart (`set_page_mark`).
    pub mark: Option<(f32, f32, Color, Color)>,
    /// Each pane below the price pane, by its group, and where its top stands:
    /// for the page to list a pane's studies at its own top.
    pub panes: Vec<(&'static str, f32)>,
}

/// One footprint row under the pointer: its bar, the prices it covers, what
/// was bought and sold there, and where it sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootprintCell {
    pub time: i64,
    pub price: f64,
    pub step: f64,
    pub buy: f64,
    pub sell: f64,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StudyMeta {
    pub id: String,
    pub label: String,
    pub group: &'static str,
    pub hidden: bool,
    /// `(studyId:lineKey, label, colour, width)` as they draw now, the reader's
    /// style applied. A width of 0 is not stroked: it only carries values for
    /// the legend and the axis.
    pub lines: Vec<(String, String, Color, f32)>,
    pub stats: Vec<(String, String)>,
}

/// A line through `points` in its pattern.
fn stroke(f: &mut Frame, points: &[(f32, f32)], width: f32, color: Color, pattern: Pattern) {
    match pattern {
        Pattern::Solid => f.polyline(points, width, color),
        Pattern::Dashed => f.dashed_polyline(points, width, color, 6.0, 4.0),
        Pattern::Dotted => f.dashed_polyline(points, width, color, width.max(1.5), 3.0),
    }
}

/// Volume by price over the bars on screen.
struct VisibleProfile {
    low: f64,
    step: f64,
    up: Vec<f64>,
    down: Vec<f64>,
    total: Vec<f64>,
    poc: usize,
    /// The rows holding the value area, lowest and highest.
    area: (usize, usize),
}

impl VisibleProfile {
    /// The middle of the point of control's row, or an edge of the value area.
    fn level(&self, key: &str) -> Option<f64> {
        match key {
            "poc" => Some(self.low + self.step * (self.poc as f64 + 0.5)),
            "vah" => Some(self.low + self.step * (self.area.1 + 1) as f64),
            "val" => Some(self.low + self.step * self.area.0 as f64),
            _ => None,
        }
    }
}

/// What the reader changed of how a study or layer draws, per chart: a plot's
/// colour, width and whether it shows, and the numbers a layer takes.
#[derive(Clone, Debug, Default, PartialEq)]
struct Styles(Vec<(String, String, PlotStyle)>);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PlotStyle {
    color: Option<Color>,
    width: Option<f32>,
    off: bool,
    pattern: Option<Pattern>,
    number: Option<f64>,
}

impl Styles {
    /// `owner/plot=colour,width,off,pattern` or `owner/plot=number`, by `;`. What does
    /// not read is left at its default: a setting from a newer page must not
    /// break an older engine.
    fn parse(raw: &str) -> Self {
        let mut out = Vec::new();
        for record in raw.split(';') {
            let Some((owner, value)) = record.split_once('=') else { continue };
            let Some((owner, plot)) = owner.split_once('/') else { continue };
            let mut parts = value.split(',');
            let first = parts.next().unwrap_or_default();
            let style = match parts.next() {
                Some(width) => PlotStyle { color: Color::parse(first), width: tradoor_core::format::parse_number(width).map(|w| w as f32).filter(|w| (0.5..=8.0).contains(w)), off: parts.next() == Some("1"),
                    pattern: match parts.next() {
                        Some("0") => Some(Pattern::Solid),
                        Some("1") => Some(Pattern::Dashed),
                        Some("2") => Some(Pattern::Dotted),
                        _ => None,
                    },
                    number: None,
                },
                None => PlotStyle { number: tradoor_core::format::parse_number(first).filter(|n| n.is_finite()), ..PlotStyle::default() },
            };
            out.push((owner.to_string(), plot.to_string(), style));
        }
        Self(out)
    }

    fn of(&self, owner: &str, plot: &str) -> PlotStyle {
        self.0.iter().find(|(o, p, _)| o == owner && p == plot).map_or_else(PlotStyle::default, |e| e.2)
    }

    fn color(&self, owner: &str, plot: &str, default: Color) -> Color {
        self.of(owner, plot).color.unwrap_or(default)
    }

    fn number(&self, owner: &str, key: &str, default: f64) -> f64 {
        self.of(owner, key).number.unwrap_or(default)
    }

    /// The ribbon's fast and slow lengths, and the long average's.
    fn lengths(&self) -> (usize, usize, usize) {
        let length = |owner, key, default| self.number(owner, key, default).within(2.0, 1000.0) as usize;
        (length("ribbon", "fast", 12.0), length("ribbon", "slow", 25.0), length("ema200", "length", 200.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneState {
    weight: f64,
    auto: bool,
    margins: (f64, f64),
    manual: Option<(f64, f64)>,
}

impl PaneState {
    fn default_for(group: Group) -> Self {
        match group {
            Group::Price => Self { weight: 3.0, auto: true, margins: (0.2, 0.1), manual: None },
            _ => Self { weight: 1.0, auto: true, margins: (0.08, 0.08), manual: None },
        }
    }
}

/// Each pane's state. A short list rather than a tree map, as a tree weighs
/// several kB of wasm for each type it holds.
#[derive(Clone, Debug, Default)]
struct PaneStates(Vec<(Group, PaneState)>);

impl PaneStates {
    fn get(&self, group: &Group) -> Option<&PaneState> {
        self.0.iter().find(|e| e.0 == *group).map(|e| &e.1)
    }

    fn get_mut(&mut self, group: &Group) -> Option<&mut PaneState> {
        self.0.iter_mut().find(|e| e.0 == *group).map(|e| &mut e.1)
    }

    /// The group's, made with its defaults if it has none.
    fn entry(&mut self, group: Group) -> &mut PaneState {
        let at = self.0.iter().position(|e| e.0 == group).unwrap_or_else(|| {
            self.0.push((group, PaneState::default_for(group)));
            self.0.len() - 1
        });
        &mut self.0[at].1
    }

    fn insert(&mut self, group: Group, state: PaneState) {
        *self.entry(group) = state;
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (&Group, &mut PaneState)> {
        self.0.iter_mut().map(|e| (&e.0, &mut e.1))
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut PaneState> {
        self.0.iter_mut().map(|e| &mut e.1)
    }

    fn values(&self) -> impl Iterator<Item = &PaneState> {
        self.0.iter().map(|e| &e.1)
    }
}

impl std::ops::Index<&Group> for PaneStates {
    type Output = PaneState;
    fn index(&self, group: &Group) -> &PaneState {
        self.get(group).expect("every pane has a state")
    }
}

/// Hidden layer and study ids: a handful.
#[derive(Clone, Debug, Default)]
struct Hidden(Vec<String>);

impl Hidden {
    fn contains(&self, id: &str) -> bool {
        self.0.iter().any(|h| h == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneView {
    group: Group,
    plot: Rect,
    scale: PriceScale,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    /// Pressed on a plot. It is a click until it moves past the slop, then a pan;
    /// `price_pan` once the pane's price follows the pointer too.
    Press { start: (f32, f32), last_x: f32, last_y: f32, pane: usize, panning: bool, price_pan: bool },
    PriceAxis { pane: usize, last_y: f32 },
    TimeAxis { last_x: f32 },
    Separator { upper: usize, last_y: f32 },
    /// The footprint table's top edge, held `grab` below or above where it stands.
    Table { grab: f32 },
    Brush { last: (f32, f32) },
    /// Moving a whole drawing, or one of its anchors.
    Shape { id: i64, last: (f32, f32), moved: bool },
    Anchor { id: i64, index: usize, moved: bool },
    /// Moving a price alert's level.
    Alert { id: i64, moved: bool },
    /// A finger held still: the crosshair follows it and nothing pans, and with
    /// a tool armed, lifting it places the next anchor where it was aimed.
    Held,
    /// Pressed with a tool armed: a drag places its next anchors in one gesture.
    Draw { kind: DrawingKind, start: Anchor, start_px: (f32, f32), moved: bool },
    /// A finger on a chart whose crosshair is pinned: the crosshair moves by the
    /// finger's motion, not to the finger, and a tap is the crosshair's.
    Nudge { start: (f32, f32), last: (f32, f32), moved: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Plot(usize),
    PriceAxis(usize),
    TimeAxis,
    Separator(usize),
    Outside,
}

/// A position closed out by the exchange, drawn as a ping beside the candles.
#[derive(Clone, Copy, Debug)]
struct Liquidation {
    usd: f64,
    /// A long was liquidated — someone was sold out of the market.
    long: bool,
    /// Which exchange's mark the row carries, as the text layer numbers them.
    mark: u8,
    /// When it reached us, for the fade.
    at: i64,
}

/// The most profiles a TPO chart opens on — the current one and the ones before
/// it, to be read against one another; fewer when square blocks would not fit.
const TPO_OPENING_SESSIONS: usize = 14;
/// The price pane's room above TPO profiles — clear of the legend, as a footprint
/// keeps — and below them, as a share of its height.
const TPO_MARGINS: (f64, f64) = (0.1, 0.05);
/// Profiles shown before the price pane is laid out to fit them square.
const TPO_FALLBACK_SESSIONS: usize = 8;
/// The fewest profiles a TPO chart opens on: a week, as MMT holds a daily chart.
const TPO_FEWEST_SESSIONS: usize = 6;
/// The fewest columns of room right of the last profile.
const TPO_RIGHT_COLUMNS: f64 = 6.0;
/// A TPO chart zooms out much further than candles: a week of 15-minute
/// brackets is 672 bars, and a profile is read by its shape, not its bars.
const TPO_MIN_SPACING: f64 = 0.05;

#[derive(Clone, Copy, Debug)]
struct Bar {
    time: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    live: bool,
}

pub struct Chart {
    theme: Theme,
    char_width: f32,
    width: f32,
    height: f32,
    dpr: f32,
    timeframe: ChartTimeframe,
    candles: Vec<Candle>,
    /// The daily bars behind them, when the chart's own bars are longer.
    days: Vec<Candle>,
    /// Hourly and daily bars for VWAP levels, which reach further back than the chart's own.
    level_bars: [Vec<Candle>; 2],
    times: Vec<f64>,
    live: Option<LiveBar>,
    liquidations: Vec<Liquidation>,
    /// Added to a time before it is written on the axis: the reader's time zone.
    utc_offset: i64,
    /// `Some(period)` draws TPO profiles, one a period, in place of candles.
    tpo: Option<tpo::Period>,
    tpo_settings: tpo::Settings,
    /// Profiles expanded, merged and split by hand.
    tpo_edits: tpo::Edits,
    tpo_profiles: Option<tpo::Profiles>,
    /// Volume by price inside each bar, as the page fetched it, oldest first.
    footprint: Vec<footprint::Bar>,
    /// The finest price a footprint row covers, the same for every bar on screen.
    footprint_grid: f64,
    /// Whether the rows are fitted to the zoom, grouping the grid as it needs;
    /// otherwise each covers the grid as the reader set it.
    footprint_fitted: bool,
    /// A bar replay is under way.
    replay: bool,
    /// A bar's time, and where on the plot the next layout puts it back.
    view_pin: Option<(i64, f64)>,
    /// Whether those columns stand in for the candles.
    footprint_shown: bool,
    /// How a level reads, and what else a column carries.
    footprint_options: footprint::Options,
    /// How tall a row of the footprint's table stands.
    footprint_table_row: f32,
    precision: usize,
    precision_override: Option<usize>,
    engine: EngineSeries,
    ribbon: Vec<RibbonRun>,
    study_specs: Vec<StudySpec>,
    styles: Styles,
    /// The visible profile's point of control and value area edges as last drawn,
    /// for the legend: working the rows out twice a frame is a waste.
    profile_levels: std::cell::Cell<Option<[Option<f64>; 3]>>,
    /// Summed open interest from the server, as `(time, [o, h, l, c])` steps.
    open_interest: Vec<(i64, [f64; 4])>,
    studies: Vec<ComputedStudy>,
    /// The reader's formulas, worked out by the page's formula module and drawn as studies are.
    formulas: Vec<ComputedStudy>,
    hidden: Hidden,
    log_scale: bool,
    /// Stoch RSI in a pane of its own rather than in RSI's.
    pct_apart: bool,
    time: TimeScale,
    view_ready: bool,
    /// A span of time asked to fill the plot, framed again as bars arrive — a
    /// market opens on a few bars and fills in behind them — until the reader
    /// moves the view or another is asked for.
    span: Option<(i64, i64)>,
    /// The page draws the mark over a live chart, to move it; a capture paints its own.
    page_mark: bool,
    pane_state: PaneStates,
    panes: Vec<PaneView>,
    axis_width: f32,
    crosshair: Option<(f32, f32)>,
    /// Another chart's crosshair time, in a layout that keeps them in step:
    /// its vertical line and time label, where this chart has no crosshair of its own.
    linked_cross: Option<i64>,
    alerts: Vec<AlertLine>,
    /// The alert a click picked: only it is moved by a drag with a mouse, so a
    /// press that only grazes a line on its way to pan leaves the line be.
    selected_alert: Option<i64>,
    /// Minutes whose flow ran hard, oldest first, as the page sent them.
    flow: Vec<FlowMark>,
    /// Another market's candles are on the way; these are the last one's.
    loading: bool,
    drag: Drag,
    drawings: Vec<DrawingShape>,
    selected: Option<i64>,
    tool: Option<DrawingKind>,
    /// The style a new drawing of the armed tool gets, so its draft looks the part.
    tool_style: DrawingStyle,
    /// Shift held: lines being drawn or reshaped stay level.
    shift: bool,
    /// Where drawing anchors are pulled: nowhere, a bar's price when near it, or always.
    magnet: Magnet,
    /// Ctrl held: a weak magnet for as long as it is, when the magnet is off.
    ctrl: bool,
    /// The press under way is a finger's, not a mouse's.
    touch_input: bool,
    /// A tap showed the crosshair on a touch screen: it stays until the next tap.
    pinned: bool,
    placed: Vec<Anchor>,
    draft: Option<DrawingShape>,
    /// The pane the drawing being placed is in: its first anchor's.
    draw_pane: usize,
    events: Vec<Event>,
    history_asked: Option<i64>,
    message: Option<String>,
    dirty: bool,
    last_second: i64,
    /// When the liquidation feed was last drawn, for a hit that has settled or gone since.
    feed_at: i64,
    feed_dirty: bool,
    cursor: Cursor,
    frame: Frame,
    /// The liquidation feed, on a layer of its own over the chart: a hit sliding
    /// in or fading redraws a few rows, not every candle and study under them.
    feed: Frame,
}

impl Chart {
    pub fn new(timeframe: ChartTimeframe) -> Self {
        Self {
            theme: Theme::default(),
            char_width: 6.6,
            width: 0.0,
            height: 0.0,
            dpr: 1.0,
            timeframe,
            candles: Vec::new(),
            times: Vec::new(),
            live: None,
            liquidations: Vec::new(),
            utc_offset: 0,
            tpo: None,
            tpo_settings: tpo::Settings::default(),
            tpo_edits: tpo::Edits::default(),
            tpo_profiles: None,
            footprint: Vec::new(),
            footprint_grid: 0.0,
            footprint_fitted: true,
            replay: false,
            view_pin: None,
            footprint_shown: false,
            footprint_options: footprint::Options::default(),
            footprint_table_row: footprint::TABLE_ROW,
            precision: 2,
            precision_override: None,
            engine: engine_series_with(&[], Styles::default().lengths()),
            ribbon: Vec::new(),
            study_specs: Vec::new(),
            styles: Styles::default(),
            profile_levels: std::cell::Cell::new(None),
            days: Vec::new(),
            level_bars: [Vec::new(), Vec::new()],
            open_interest: Vec::new(),
            studies: Vec::new(),
            formulas: Vec::new(),
            hidden: Hidden::default(),
            log_scale: false,
            pct_apart: false,
            time: TimeScale::new(),
            view_ready: false,
            span: None,
            page_mark: false,
            pane_state: PaneStates::default(),
            panes: Vec::new(),
            axis_width: MIN_AXIS_WIDTH,
            crosshair: None,
            linked_cross: None,
            alerts: Vec::new(),
            flow: Vec::new(),
            loading: false,
            drag: Drag::None,
            drawings: Vec::new(),
            tool_style: DrawingStyle::default(),
            shift: false,
            magnet: Magnet::Off,
            ctrl: false,
            touch_input: false,
            pinned: false,
            selected: None,
            selected_alert: None,
            tool: None,
            placed: Vec::new(),
            draw_pane: 0,
            draft: None,
            events: Vec::new(),
            history_asked: None,
            message: None,
            dirty: true,
            last_second: 0,
            feed_at: 0,
            feed_dirty: false,
            cursor: Cursor::Crosshair,
            frame: Frame::default(),
            feed: Frame::default(),
        }
    }

    /// Mark the chart for one more paint, when something it does not own changed.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    // ------------------------------------------------------------------ inputs

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme_value(&mut self, key: &str, value: &str) -> bool {
        let ok = self.theme.set(key, value);
        self.dirty |= ok;
        ok
    }

    /// The width of one glyph of the axis font, measured by the backend. Axis
    /// labels are monospace, so this sizes every label box exactly.
    /// Whether the page draws the mark over this chart (`Legend::mark`) rather than the frame.
    pub fn set_page_mark(&mut self, on: bool) {
        if on != self.page_mark {
            self.page_mark = on;
            self.touch();
        }
    }

    pub fn set_char_width(&mut self, width: f32) {
        if width > 1.0 {
            self.char_width = width;
            self.touch();
        }
    }

    pub fn resize(&mut self, width: f32, height: f32, dpr: f32) {
        self.width = width.max(0.0);
        self.height = height.max(0.0);
        self.dpr = dpr.max(0.5);
        // The feed's layer is sized as it is drawn: a new size draws it again too.
        self.feed_dirty = true;
        self.touch();
    }

    /// Waiting on another market's candles: what is drawn is the market just
    /// left, dimmed, rather than an empty chart — the wait is the same, but the
    /// eye keeps its place.
    pub fn set_loading(&mut self, loading: bool) {
        if self.loading != loading {
            self.loading = loading;
            self.touch();
        }
    }

    /// Shown in place of the chart while there are no bars.
    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
        self.touch();
    }

    pub fn timeframe(&self) -> ChartTimeframe {
        self.timeframe
    }

    pub fn set_timeframe(&mut self, timeframe: ChartTimeframe) {
        if timeframe != self.timeframe {
            self.timeframe = timeframe;
            self.view_ready = false;
            self.live = None;
            self.placed.clear();
            self.draft = None;
            // The old bars' footprint would stand between the new candles until
            // the page's fetch for these lands.
            self.footprint.clear();
            self.touch();
        }
    }

    /// Replace the closed bars. Older bars prepended keep the view where it is:
    /// the right offset is measured from the newest bar, which has not moved.
    pub fn set_candles(&mut self, candles: Vec<Candle>) {
        let old_first = self.candles.first().map(|c| c.open_time);
        self.candles = candles;
        self.times = self.candles.iter().map(|c| c.open_time as f64).collect();
        if let (Some(live), Some(last)) = (self.live, self.candles.last()) {
            if live.open_time <= last.open_time {
                self.live = None;
            }
        }
        if self.candles.first().map(|c| c.open_time) != old_first {
            self.history_asked = None;
        }
        self.precision = price_precision(self.candles.iter().rev().take(2000).flat_map(|c| [c.open, c.high, c.low, c.close]));
        self.refresh_engine();
        self.studies = self.compute_studies(&self.study_specs);
        if self.candles.is_empty() {
            self.view_ready = false;
            // Another market on its way: the last one's footprint is not its.
            self.footprint.clear();
        }
        if let Some((from, to)) = self.span {
            self.frame_span(from, to);
        }
        self.touch();
    }

    fn refresh_engine(&mut self) {
        self.engine = engine_series_with(&self.candles, self.styles.lengths());
        self.ribbon = ribbon_runs(&self.engine.ema12, &self.engine.ema25);
    }

    /// How the reader has the studies and layers drawn; see `Styles::parse`.
    pub fn set_styles(&mut self, raw: &str) {
        let styles = Styles::parse(raw);
        if styles == self.styles {
            return;
        }
        let lengths = styles.lengths() != self.styles.lengths();
        self.styles = styles;
        if lengths {
            self.refresh_engine();
        }
        self.feed_dirty = true;
        self.touch();
    }

    /// The daily bars the chart's own were folded from, for the levels that open
    /// inside a longer bar. Empty where the chart's bars are a day or shorter.
    pub fn set_days(&mut self, days: Vec<Candle>) {
        if self.days == days {
            return;
        }
        self.days = days;
        self.refresh_studies(StudyKind::KeyLevels);
        self.touch();
    }

    /// VWAP levels' hourly bars, or with `daily` their daily ones.
    pub fn set_level_bars(&mut self, daily: bool, bars: Vec<Candle>) {
        let held = &mut self.level_bars[usize::from(daily)];
        if *held != bars {
            *held = bars;
            self.refresh_studies(StudyKind::VwapLevels);
            self.touch();
        }
    }

    pub fn candles(&self) -> &[Candle] {
        &self.candles
    }

    /// The forming bar, drawn after the closed ones. Indicators stop at the last
    /// CLOSED bar: the forming one can still change.
    pub fn set_live(&mut self, live: Option<LiveBar>) {
        let live = live.filter(|l| self.candles.last().is_none_or(|c| l.open_time > c.open_time));
        if live != self.live {
            let same_bar = live.zip(self.live).is_some_and(|(a, b)| a.open_time == b.open_time);
            // A trade at a price the bar has already seen moves its volume alone:
            // with no volume drawn, nothing on screen changes and nothing is redrawn.
            let unseen = same_bar
                && live.zip(self.live).is_some_and(|(a, b)| (a.open, a.high, a.low, a.close) == (b.open, b.high, b.low, b.close))
                && self.hidden.contains("volume")
                && self.tpo().is_none()
                && !self.footprint_shown;
            self.live = live;
            if unseen {
                return;
            }
            // Open interest runs into the forming bar, and in dollars moves with
            // its price: a tick within the bar moves that bar's figure alone.
            if same_bar {
                self.refresh_open_interest_live();
            } else {
                self.refresh_open_interest();
            }
            self.touch();
        }
    }

    /// Summed open interest, `(time, [open, high, low, close])` steps oldest first.
    pub fn set_open_interest(&mut self, steps: Vec<(i64, [f64; 4])>) {
        self.open_interest = steps;
        self.refresh_open_interest();
        self.touch();
    }

    fn refresh_open_interest(&mut self) {
        self.refresh_studies(StudyKind::OpenInterest);
    }

    /// Recompute the shown studies of one kind, after what they read changed.
    fn refresh_studies(&mut self, kind: StudyKind) {
        if !self.study_specs.iter().any(|s| s.kind == kind) {
            return;
        }
        let inputs = StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days, hourly: &self.level_bars[0], daily: &self.level_bars[1] };
        for (spec, study) in self.study_specs.iter().zip(self.studies.iter_mut()) {
            if spec.kind == kind && !self.hidden.contains(&study.id) {
                *study = compute_study_with(spec, &self.candles, &inputs);
            }
        }
    }

    /// The shown open interest's forming bar, after a tick within it: the bars
    /// before it stand, and working them all out again on every trade of a busy
    /// market with months of minutes loaded was most of what the chart did.
    fn refresh_open_interest_live(&mut self) {
        let inputs = StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days, hourly: &self.level_bars[0], daily: &self.level_bars[1] };
        for (spec, study) in self.study_specs.iter().zip(self.studies.iter_mut()) {
            if spec.kind == StudyKind::OpenInterest && !self.hidden.contains(&study.id) && !open_interest_live(spec, study, &self.candles, &inputs) {
                *study = compute_study_with(spec, &self.candles, &inputs);
            }
        }
    }

    fn study_inputs(&self) -> StudyInputs<'_> {
        StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days, hourly: &self.level_bars[0], daily: &self.level_bars[1] }
    }

    /// Every study worked out but the hidden ones, which wait until shown.
    fn compute_studies(&self, specs: &[StudySpec]) -> Vec<ComputedStudy> {
        let inputs = self.study_inputs();
        specs.iter().map(|s| if self.hidden.contains(&s.id()) { idle_study(s) } else { compute_study_with(s, &self.candles, &inputs) }).collect()
    }

    /// One liquidation, to ping beside the candles until it fades.
    /// A hit for the feed: what it cost, which side was closed out, and which
    /// exchange it happened on. Where and when it traded do not place it — the
    /// feed stacks hits in the order they arrive.
    pub fn push_liquidation(&mut self, usd: f64, long: bool, mark: u8, at: i64) {
        if !(usd.is_finite() && usd > 0.0) {
            return;
        }
        self.liquidations.retain(|l| at - l.at < LIQUIDATION_TTL);
        if self.liquidations.len() >= MAX_LIQUIDATIONS {
            self.liquidations.remove(0);
        }
        self.liquidations.push(Liquidation { usd, long, mark, at });
        self.feed_dirty = true;
    }

    pub fn clear_liquidations(&mut self) {
        self.liquidations.clear();
        self.feed_dirty = true;
    }

    /// A script's lines, fills, marks, shapes and table, as the formula module gave them
    /// (crates/frost-script-wasm): a part a line, tab-separated, whole numbers only, and the
    /// values each part has after it, in the same order. Values for other bars than
    /// these — the window moved since — are let go.
    pub fn set_formula(&mut self, id: &str, label: &str, meta: &str, values: &[f64]) {
        use tradoor_core::studies::{ArtBox, ArtCell, ArtLabel, ArtLine, FormulaArt, StudyLine, StudyMarker, FORMULA_FILLS, FORMULA_LINES, FORMULA_MARKS};
        let n = self.candles.len();
        let place = self.formulas.iter().position(|s| s.id == id).unwrap_or(self.formulas.len());
        let mut study = ComputedStudy {
            id: id.to_string(),
            kind: StudyKind::Formula,
            label: label.to_string(),
            group: Group::Price,
            params: Vec::new(),
            lines: Vec::new(),
            fills: Vec::new(),
            boxes: Vec::new(),
            segments: Vec::new(),
            markers: Vec::new(),
            stats: Vec::new(),
            candles: Vec::new(),
            art: None,
        };
        let mut art = FormulaArt::default();
        let mut at = 0usize;
        let mut take = |count: usize| -> Option<&[f64]> {
            let part = values.get(at..at + count);
            at += count;
            part
        };
        let int = |s: &str| s.parse::<u32>().unwrap_or(0);
        let small = |s: &str| s.parse::<u8>().unwrap_or(0);
        let colours = |xs: &[f64]| -> Vec<u32> { xs.iter().map(|v| if *v >= 0.0 && *v <= 4_294_967_295.0 { *v as u32 } else { 0 }).collect() };
        let text = |s: &str| s.replace('\u{1f}', "\n");
        for row in meta.lines() {
            let f: Vec<&str> = row.split('\t').collect();
            match f.as_slice() {
                ["pane", own] => study.group = if *own == "1" { Group::Formula((place % 8) as u8) } else { Group::Price },
                ["plot", kind, color, width, style, offset, per_bar, title] => {
                    let Some(series) = take(n) else { return };
                    let series = series.to_vec();
                    let bar_rgba = if *per_bar == "1" {
                        let Some(c) = take(n) else { return };
                        Some(colours(c))
                    } else {
                        None
                    };
                    // Moved by its offset: to the right runs on into the space ahead of the newest bar.
                    let shift = offset.trim_start_matches('-').parse::<usize>().unwrap_or(0).min(500);
                    let ahead = !offset.starts_with('-');
                    let moved = |xs: Vec<Option<f64>>| -> Vec<Option<f64>> {
                        if shift == 0 {
                            xs
                        } else if ahead {
                            std::iter::repeat_n(None, shift).chain(xs).collect()
                        } else {
                            xs.into_iter().skip(shift).chain(std::iter::repeat_n(None, shift)).collect()
                        }
                    };
                    let moved_colours = |xs: Vec<u32>| -> Vec<u32> {
                        if shift == 0 {
                            xs
                        } else if ahead {
                            std::iter::repeat_n(0, shift).chain(xs).collect()
                        } else {
                            xs.into_iter().skip(shift).chain(std::iter::repeat_n(0, shift)).collect()
                        }
                    };
                    let key = FORMULA_LINES[study.lines.len() % FORMULA_LINES.len()];
                    let style = small(style);
                    let line = StudyLine {
                        key,
                        label: (*title).to_string(),
                        kind: if *kind == "histogram" { LineType::Histogram } else { LineType::Line },
                        values: moved(series.iter().map(|v| (!v.is_nan()).then_some(*v)).collect()),
                        style: if style == 3 { LineStyle::Step } else { LineStyle::Solid },
                        pattern: match style {
                            1 => Pattern::Dashed,
                            2 => Pattern::Dotted,
                            _ => Pattern::Solid,
                        },
                        color: None,
                        // Tenths: a whole number, as no float is read in the engine.
                        width: int(width) as f32 / 10.0,
                        badge: *kind == "line" && style != 4,
                        rgba: color.parse::<u32>().ok(),
                        bar_rgba: bar_rgba.map(moved_colours),
                        dots: style == 4,
                    };
                    study.lines.push(line);
                }
                ["fill", upper, lower, color] => {
                    let (Ok(a), Ok(b)) = (upper.parse::<usize>(), lower.parse::<usize>()) else { continue };
                    let (Some(a), Some(b)) = (FORMULA_LINES.get(a), FORMULA_LINES.get(b)) else { continue };
                    let key = FORMULA_FILLS[study.fills.len() % FORMULA_FILLS.len()];
                    study.fills.push(StudyFill { upper: a, lower: b, key, color: "#2962ff1f", crossed_key: key, crossed_color: "#2962ff1f", rgba: int(color) });
                }
                ["mark", above, color, text] => {
                    let Some(hits) = take(n) else { return };
                    let plot = FORMULA_MARKS[study.markers.len().min(FORMULA_MARKS.len() - 1)];
                    let tone = Tone::Rgba(int(color));
                    for (i, hit) in hits.iter().enumerate() {
                        if *hit != 0.0 {
                            study.markers.push(StudyMarker { time: self.candles[i].open_time, above: *above == "1", text: (*text).to_string(), tone, plot, hollow: false });
                        }
                    }
                }
                ["bg"] => {
                    let Some(c) = take(n) else { return };
                    art.backgrounds.push(colours(c));
                }
                ["bars"] => {
                    let Some(c) = take(n) else { return };
                    art.bar_colors = colours(c);
                }
                ["line", color, width, style, extend] => {
                    let Some(&[x1, y1, x2, y2]) = take(4) else { return };
                    art.lines.push(ArtLine { x1, y1, x2, y2, color: int(color), width: int(width) as f32 / 10.0, style: small(style), extend: small(extend) });
                }
                ["label", color, text_color, place, size, plain, words] => {
                    let Some(&[x, y]) = take(2) else { return };
                    art.labels.push(ArtLabel { x, y, text: text(words), color: int(color), text_color: int(text_color), place: small(place), size: small(size), plain: *plain == "1" });
                }
                ["box", fill, border, width] => {
                    let Some(&[x1, y1, x2, y2]) = take(4) else { return };
                    art.boxes.push(ArtBox { x1, y1, x2, y2, fill: int(fill), border: int(border), width: int(width) as f32 / 10.0 });
                }
                ["table", down, across] => art.table = Some((small(down), small(across))),
                ["cell", col, row, color, background, words] => art.cells.push(ArtCell { col: small(col), row: small(row), text: text(words), color: int(color), background: int(background) }),
                ["error", line, _, message] => study.stats.push(("Error", if *line == "0" { (*message).to_string() } else { format!("line {line}: {message}") })),
                _ => {}
            }
        }
        if art != FormulaArt::default() {
            study.art = Some(Box::new(art));
        }
        if place < self.formulas.len() {
            self.formulas[place] = study;
        } else if self.formulas.len() < 8 {
            self.formulas.push(study);
        }
        self.touch();
    }

    /// The formulas still on the chart, by id: the rest are let go.
    pub fn keep_formulas(&mut self, ids: &str) {
        let before = self.formulas.len();
        self.formulas.retain(|s| ids.split(',').any(|id| id == s.id));
        if self.formulas.len() != before {
            self.touch();
        }
    }

    /// `ema:50,rsi:14`.
    pub fn set_studies(&mut self, raw: &str) {
        let specs = parse_studies(raw, 12);
        if specs != self.study_specs {
            self.studies = self.compute_studies(&specs);
            self.study_specs = specs;
            self.touch();
        }
    }

    /// Hidden layer ids: `ribbon`, `ema200`, `volume`, or a study id.
    pub fn set_hidden(&mut self, raw: &str) {
        let was = std::mem::replace(&mut self.hidden, Hidden(raw.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()));
        // A study shown again is worked out now; one hidden lets its figures go.
        let inputs = StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days, hourly: &self.level_bars[0], daily: &self.level_bars[1] };
        for (spec, study) in self.study_specs.iter().zip(self.studies.iter_mut()) {
            match (was.contains(&study.id), self.hidden.contains(&study.id)) {
                (true, false) => *study = compute_study_with(spec, &self.candles, &inputs),
                (false, true) => *study = idle_study(spec),
                _ => {}
            }
        }
        self.touch();
    }

    /// The reader's time zone, as minutes east of UTC. Only intraday times move:
    /// a daily bar is the exchange's UTC day, and naming it by a local date would
    /// put it on the wrong day for half the world.
    pub fn set_utc_offset(&mut self, minutes: i32) {
        let offset = i64::from(minutes.clamp(-14 * 60, 14 * 60)) * 60_000;
        if offset != self.utc_offset {
            self.utc_offset = offset;
            self.touch();
        }
    }

    /// `t` as the axis writes it.
    fn shown_time(&self, t: i64) -> i64 {
        if self.timeframe.source().interval.ms().is_some_and(|ms| ms < DAY) { t + self.utc_offset } else { t }
    }

    /// Draws each `raw` session — `1d`, `1w`, `1M` — as a TPO profile of the
    /// chart's own bars in place of candles; empty draws candles again. The
    /// choice is kept across timeframes, but only takes effect while the session
    /// is longer than the bars: a 1D chart has no brackets to stack into a day.
    pub fn set_tpo(&mut self, raw: &str) {
        let period = tpo::Period::parse(raw);
        if period != self.tpo {
            self.tpo = period;
            self.tpo_profiles = None;
            // Profiles and candles are read at different distances: open each afresh.
            self.view_ready = false;
            self.touch();
        }
    }

    /// The period profiles are drawn by now, if any.
    pub fn tpo(&self) -> Option<tpo::Period> {
        self.tpo.filter(|p| p.fits(self.timeframe))
    }

    /// TPO options as `key=value;key=value`, over MMT's defaults.
    pub fn set_tpo_settings(&mut self, raw: &str) {
        let settings = tpo::Settings::parse(raw);
        if settings != self.tpo_settings {
            self.tpo_settings = settings;
            self.touch();
        }
    }

    /// Profiles expanded, merged and split by hand, as `e=t,t;m=t;s=t`.
    pub fn set_tpo_edits(&mut self, raw: &str) {
        let edits = tpo::Edits::parse(raw);
        if edits != self.tpo_edits {
            self.tpo_edits = edits;
            self.touch();
        }
    }

    /// The active price alerts of this market.
    pub fn set_alerts(&mut self, alerts: Vec<AlertLine>) {
        // A line being dragged keeps the price under the pointer until it is let go.
        if let Drag::Alert { id, .. } = self.drag {
            if let Some(held) = self.alerts.iter().find(|a| a.id == id).copied() {
                self.alerts = alerts.into_iter().map(|a| if a.id == id { held } else { a }).collect();
                self.touch();
                return;
            }
        }
        if self.selected_alert.is_some_and(|id| !alerts.iter().any(|a| a.id == id)) {
            self.selected_alert = None;
        }
        if self.alerts != alerts {
            self.alerts = alerts;
            self.touch();
        }
    }

    /// Pick an alert, or none: a drawing picked is let go, as only one thing is picked at a time.
    fn pick_alert(&mut self, id: Option<i64>) {
        if id.is_some() {
            self.select(None);
        }
        if self.selected_alert != id {
            self.selected_alert = id;
            self.touch();
        }
    }

    /// Volume by price inside each bar: what the page fetched of what the
    /// server recorded, its minutes folded into the bars on screen.
    pub fn set_footprint(&mut self, bars: Vec<footprint::Bar>, grid: f64, fitted: bool) {
        if self.footprint != bars || self.footprint_grid != grid || self.footprint_fitted != fitted {
            self.footprint = bars;
            self.footprint_grid = grid;
            self.footprint_fitted = fitted;
            self.touch();
        }
    }

    /// Whether footprint columns stand in for the candles now.
    fn footprint_drawn(&self) -> bool {
        self.tpo().is_none() && self.footprint_shown && !self.footprint.is_empty()
    }

    /// How tall the footprint's table stands on a plot `plot_h` tall: nothing with it off.
    fn footprint_table(&self, plot_h: f32) -> f32 {
        if self.footprint_options.stats { footprint::table_height(self.footprint_table_row, plot_h) } else { 0.0 }
    }

    /// Where the footprint's table starts, when one is drawn.
    fn table_top(&self) -> Option<f32> {
        let pane = self.panes.first()?;
        (self.footprint_drawn() && self.footprint_options.stats).then(|| pane.plot.bottom() - self.footprint_table(pane.plot.h))
    }

    /// Whether `y` is on the top edge of the footprint's table, which drags.
    fn on_table_edge(&self, y: f32) -> bool {
        let grab = if self.touch_input { TABLE_EDGE_TOUCH } else { SEPARATOR_GRAB };
        self.table_top().is_some_and(|top| (y - top).abs() <= grab)
    }

    /// The price one footprint row covers on this pane as it is zoomed: read at
    /// the middle of the plot, so every bar and the pointer agree on it.
    fn footprint_step(&self, pane: &PaneView) -> f64 {
        let y = |p: f64| pane.scale.y(p) as f32;
        let middle = pane.scale.price(f64::from(pane.plot.y + pane.plot.h / 2.0));
        footprint::row_step(&y, middle, self.footprint_grid, self.footprint_fitted)
    }

    /// Whether those columns are drawn at all: a chart style, as TPO is. Turned
    /// on, the bars widen to where a cell can carry its number, the newest held
    /// where it is: a footprint zoomed out is stripes nobody can read.
    pub fn set_footprint_shown(&mut self, shown: bool) {
        if self.footprint_shown != shown {
            self.footprint_shown = shown;
            if shown && self.view_ready && self.time.width > 0.0 && self.time.bar_spacing < footprint::READABLE_SPACING {
                self.time.zoom_at(self.time.width, footprint::READABLE_SPACING / self.time.bar_spacing);
            }
            self.touch();
        }
    }

    /// How a level reads — `split`, `delta`, `total`, `cluster`, `ladder` —
    /// and what else a column carries, as `mode;va;stats;profile;tableRow`.
    pub fn set_footprint_options(&mut self, raw: &str) {
        let mut parts = raw.split(';');
        let on = |part: Option<&str>| part != Some("0");
        let options = footprint::Options {
            mode: footprint::Mode::parse(parts.next().unwrap_or_default()),
            value_area: on(parts.next()),
            stats: on(parts.next()),
            profile: on(parts.next()),
        };
        let row = parts.next().and_then(tradoor_core::format::parse_number).map_or(footprint::TABLE_ROW, |v| footprint::table_row(v as f32));
        if self.footprint_options != options || self.footprint_table_row != row {
            self.footprint_options = options;
            self.footprint_table_row = row;
            self.touch();
        }
    }

    /// The minutes whose flow ran hard, to mark on the candles. Oldest first.
    pub fn set_flow(&mut self, marks: Vec<FlowMark>) {
        if self.flow != marks {
            self.flow = marks;
            self.touch();
        }
    }

    /// The footprint cell under (x, y): its bar, its level, and where the row
    /// sits — so a reader can put the pointer on a cell and be told what made
    /// it, which is the whole use of the numbers being too small to print.
    pub fn footprint_at(&self, x: f32, y: f32) -> Option<FootprintCell> {
        if !self.footprint_drawn() {
            return None;
        }
        let pane = self.panes.first()?;
        if x < 0.0 || x >= pane.plot.w || y < pane.plot.y || y > pane.plot.bottom() {
            return None;
        }
        if self.table_top().is_some_and(|top| y >= top) {
            return None;
        }
        let step = self.footprint_step(pane);
        let price = pane.scale.price(f64::from(y));
        let newest = self.newest_time();
        for bar in self.footprint.iter().filter(|bar| bar.time <= newest) {
            let Some(logical) = time_to_logical(&self.times, bar.time as f64) else { continue };
            let s = footprint::slot(self.time.x(logical) as f32, self.time.bar_spacing as f32);
            if x < s.left || x > s.left + s.width {
                continue;
            }
            let rows = footprint::grouped(bar, step);
            let &(at, buy, sell) = rows.levels.iter().find(|(p, _, _)| price >= *p && price < *p + step)?;
            return Some(FootprintCell { time: bar.time, price: at, step, buy, sell, x: s.cells + s.cells_width, y: pane.scale.y(at + step / 2.0) as f32 });
        }
        None
    }

    /// Whether the reader asked for the marks: they are a study like any
    /// other, added from the indicator list and hidden with its eye.
    fn flow_shown(&self) -> bool {
        self.studies.iter().any(|s| s.kind == StudyKind::Flow && !self.hidden.contains(&s.id))
    }

    /// The flow mark under (x, y), and where it sits — for the page's label.
    fn flow_style(&self, mark: &FlowMark) -> PlotStyle {
        self.styles.of("flow", if mark.usd > 0.0 { "bought" } else { "sold" })
    }

    pub fn flow_at(&self, x: f32, y: f32) -> Option<(usize, f32, f32)> {
        if !self.flow_shown() {
            return None;
        }
        let pane = self.panes.first()?;
        if x < 0.0 || x >= pane.plot.w || y < pane.plot.y || y > pane.plot.bottom() {
            return None;
        }
        self.flow
            .iter()
            .enumerate()
            .filter(|(_, mark)| !self.flow_style(mark).off)
            .filter_map(|(i, mark)| {
                let (mx, my) = self.flow_xy(mark, pane)?;
                let reach = self.flow_radius(mark) + 5.0;
                ((mx - x).hypot(my - y) <= reach).then_some((i, mx, my))
            })
            .min_by(|a, b| (a.1 - x).hypot(a.2 - y).total_cmp(&(b.1 - x).hypot(b.2 - y)))
    }

    /// Where a mark sits, or nothing when its minute is off the chart.
    fn flow_xy(&self, mark: &FlowMark, pane: &PaneView) -> Option<(f32, f32)> {
        let logical = time_to_logical(&self.times, mark.at as f64)?;
        let x = self.time.x(logical) as f32;
        let y = pane.scale.y(mark.price) as f32;
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }

    /// How big a mark is drawn: the bigger the flow, the bigger the dot, within
    /// reason — a minute ten times the threshold is not ten times the circle.
    fn flow_radius(&self, mark: &FlowMark) -> f32 {
        let biggest = self.flow.iter().map(|m| m.usd.abs()).fold(1.0, f64::max);
        let share = (mark.usd.abs() / biggest).within(0.0, 1.0) as f32;
        FLOW_MIN_RADIUS + (FLOW_MAX_RADIUS - FLOW_MIN_RADIUS) * share.sqrt()
    }

    /// The alert line within a few pixels of (x, y) on the price pane.
    pub fn alert_at(&self, x: f32, y: f32) -> Option<(i64, f32)> {
        self.alert_within(x, y, ALERT_HIT_PX)
    }

    fn alert_within(&self, x: f32, y: f32, reach: f32) -> Option<(i64, f32)> {
        let pane = self.panes.first()?;
        if x < 0.0 || x >= pane.plot.w || y < pane.plot.y || y > pane.plot.bottom() {
            return None;
        }
        self.alerts
            .iter()
            .map(|a| (a.id, pane.scale.y(a.price) as f32))
            .filter(|(_, ay)| (ay - y).abs() <= reach)
            .min_by(|a, b| (a.1 - y).abs().total_cmp(&(b.1 - y).abs()))
    }

    /// The alert whose price badge on the axis is at (x, y): the label is a
    /// handle too, as it is on TradingView.
    fn alert_badge_at(&self, x: f32, y: f32) -> Option<i64> {
        let pane = self.panes.first()?;
        if x < pane.plot.w || y < pane.plot.y || y > pane.plot.bottom() {
            return None;
        }
        let reach = (self.theme.font_px + 6.0) / 2.0;
        self.alerts
            .iter()
            .map(|a| (a.id, pane.scale.y(a.price) as f32))
            .filter(|(_, ay)| (ay - y).abs() <= reach)
            .min_by(|a, b| (a.1 - y).abs().total_cmp(&(b.1 - y).abs()))
            .map(|(id, _)| id)
    }

    /// The alert the pointer is on, by its line or by its badge, and its y.
    pub fn alert_handle_at(&self, x: f32, y: f32) -> Option<(i64, f32)> {
        self.alert_at(x, y).or_else(|| {
            let id = self.alert_badge_at(x, y)?;
            let pane = self.panes.first()?;
            let a = self.alerts.iter().find(|a| a.id == id)?;
            Some((id, pane.scale.y(a.price) as f32))
        })
    }

    /// Soft edges on slanted drawing lines — for a GPU surface; see `Frame::smooth`.
    pub fn set_smooth_lines(&mut self, on: bool) {
        if self.frame.smooth != on {
            self.frame.smooth = on;
            self.touch();
        }
    }

    pub fn set_log_scale(&mut self, log: bool) {
        if log != self.log_scale {
            self.log_scale = log;
            // A manual range is kept in the old mode's space and means nothing in the new one.
            let was = self.auto_scale();
            self.pane_state.insert(Group::Price, PaneState { weight: self.weight(Group::Price), ..PaneState::default_for(Group::Price) });
            self.note_auto(was);
            self.touch();
        }
    }

    pub fn log_scale(&self) -> bool {
        self.log_scale
    }

    /// The exchange's tick decimals, when known; otherwise read off the prices.
    pub fn set_price_decimals(&mut self, decimals: Option<usize>) {
        self.precision_override = decimals.map(|d| d.clamp(0, 10));
        self.touch();
    }

    /// The leftmost bar in view, as an index into the bars held: how far back
    /// the reader is, for the page to know when older bars can be let go.
    pub fn visible_from(&self) -> f64 {
        self.time.visible().0
    }

    pub fn precision(&self) -> usize {
        self.precision_override.unwrap_or(self.precision)
    }

    fn weight(&self, group: Group) -> f64 {
        self.pane_state.get(&group).map_or(PaneState::default_for(group).weight, |s| s.weight)
    }

    /// `price:3,pct:1` — pane heights as relative weights.
    pub fn set_pane_weights(&mut self, raw: &str) {
        for (key, weight) in raw.split(',').filter_map(|p| p.split_once(':')) {
            let group = match key.trim() {
                "price" => Group::Price,
                "pct" => Group::Pct,
                "stoch" => Group::Stoch,
                "macd" => Group::Macd,
                "atr" => Group::Atr,
                "delta" => Group::Delta,
                "oi" => Group::Oi,
                fx => match tradoor_core::studies::FORMULA_PANES.iter().position(|p| *p == fx) {
                    Some(n) => Group::Formula(n as u8),
                    None => continue,
                },
            };
            if let Some(w) = tradoor_core::format::parse_number(weight.trim()) {
                self.pane_state.entry(group).weight = w.within(0.1, 50.0);
            }
        }
        self.touch();
    }

    pub fn pane_weights(&self) -> String {
        self.panes.iter().map(|p| format!("{}:{}", p.group.as_str(), tradoor_core::format::fixed(self.weight(p.group), 3))).collect::<Vec<_>>().join(",")
    }

    pub fn set_drawings(&mut self, shapes: Vec<DrawingShape>) {
        if self.selected.is_some_and(|id| !shapes.iter().any(|s| s.id == Some(id))) {
            self.selected = None;
        }
        self.drawings = shapes;
        self.touch();
    }

    pub fn set_selected(&mut self, id: Option<i64>) {
        self.selected = id;
        // A drawing picked from the page lets a picked alert go. The page also
        // echoes a drawing let go, which an alert just picked must outlive.
        if id.is_some() {
            self.selected_alert = None;
        }
        self.touch();
    }

    pub fn selected(&self) -> Option<i64> {
        self.selected
    }

    pub fn set_tool(&mut self, tool: Option<DrawingKind>) {
        self.set_tool_styled(tool, DrawingStyle::default());
    }

    pub fn set_tool_styled(&mut self, tool: Option<DrawingKind>, style: DrawingStyle) {
        self.tool = tool;
        self.tool_style = style;
        self.placed.clear();
        self.draft = None;
        self.drag = Drag::None;
        self.touch();
    }

    pub fn set_shift(&mut self, shift: bool) {
        self.shift = shift;
    }

    pub fn set_ctrl(&mut self, ctrl: bool) {
        self.ctrl = ctrl;
    }

    pub fn set_touch(&mut self, touch: bool) {
        self.touch_input = touch;
        self.pinned &= touch;
    }

    /// The magnet in force: the chosen one, or a weak one while Ctrl is held over none.
    fn magnet_now(&self) -> Magnet {
        match self.magnet {
            Magnet::Off if self.ctrl => Magnet::Weak,
            m => m,
        }
    }

    pub fn set_magnet(&mut self, magnet: Magnet) {
        self.magnet = magnet;
    }

    pub fn tool(&self) -> Option<DrawingKind> {
        self.tool
    }

    /// Back to the opening view: the last bars, and every axis autoscaling.
    /// A bar replay is under way: the newest bar shown is no bar closing on the clock.
    pub fn set_replay(&mut self, on: bool) {
        if self.replay != on {
            self.replay = on;
            self.touch();
        }
    }

    /// When the bar nearest `x` opened, for a replay to start from.
    pub fn bar_time_at(&self, x: f32) -> Option<i64> {
        self.bar(self.time.nearest_bar(f64::from(x))?).map(|b| b.time)
    }

    /// Holds the bar at `time` where it stands on screen through the changes to
    /// come. Put back at the next layout, by which a TPO chart's warp is settled too.
    pub fn pin_view(&mut self, time: i64) {
        if self.view_pin.is_none() {
            self.view_pin = time_to_logical(&self.times, time as f64).map(|logical| (time, self.time.x(logical)));
            self.touch();
        }
    }

    /// Columns free right of the newest bar; below zero, it is scrolled off the plot.
    pub fn right_room(&self) -> f64 {
        self.time.right_offset
    }

    /// When the newest bar shown opened: past it nothing has a candle to stand on.
    fn newest_time(&self) -> i64 {
        self.total_bars().checked_sub(1).and_then(|i| self.bar(i)).map_or(i64::MAX, |b| b.time)
    }

    /// Back at the newest bars, at the zoom they are being read at.
    pub fn show_newest(&mut self) {
        self.span = None;
        self.time.bars = self.total_bars();
        self.time.right_offset = RIGHT_MARGIN_BARS;
        self.time.clamp();
        self.view_ready = true;
        self.touch();
    }

    pub fn reset_view(&mut self) {
        self.span = None;
        self.view_ready = false;
        let was = self.auto_scale();
        for (group, state) in self.pane_state.iter_mut() {
            *state = PaneState { weight: state.weight, ..PaneState::default_for(*group) };
        }
        self.note_auto(was);
        self.touch();
    }

    /// Zoom to the last `days` of bars (all of them for `None`), counted from the
    /// candles so a gap in the tape cannot stretch the view past the data.
    pub fn show_range(&mut self, days: Option<f64>) {
        self.span = None;
        let n = self.candles.len();
        if n == 0 {
            return;
        }
        let bars = match days {
            None => n,
            Some(d) => {
                let from = self.candles[n - 1].open_time as f64 - d * DAY as f64;
                self.candles.iter().rev().take_while(|c| c.open_time as f64 >= from).count().max(8.min(n))
            }
        };
        self.time.bars = self.total_bars();
        self.time.show_last(bars as f64 + if self.live.is_some() { 1.0 } else { 0.0 }, RIGHT_MARGIN_BARS);
        self.view_ready = true;
        let was = self.auto_scale();
        if let Some(state) = self.pane_state.get_mut(&Group::Price) {
            state.auto = true;
            state.manual = None;
        }
        self.note_auto(was);
        self.touch();
    }

    /// The bars from `from` to `to` (ms) fill the plot, a few bars of room either
    /// side: journal trades drawn where they went (web journal/picture.ts).
    pub fn show_span(&mut self, from: i64, to: i64) {
        self.span = Some((from, to));
        self.frame_span(from, to);
    }

    fn frame_span(&mut self, from: i64, to: i64) {
        let (Some(a), Some(b)) = (time_to_logical(&self.times, from as f64), time_to_logical(&self.times, to as f64)) else { return };
        // Asked for as the bars arrive, before a frame has counted them.
        self.time.bars = self.total_bars();
        self.time.show_between(a, b, 3.0);
        self.view_ready = true;
        let was = self.auto_scale();
        if let Some(state) = self.pane_state.get_mut(&Group::Price) {
            state.auto = true;
            state.manual = None;
        }
        self.note_auto(was);
        self.touch();
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Something changed, or the countdown needs its next second.
    pub fn needs_render(&self, now: i64) -> bool {
        self.dirty || (!self.candles.is_empty() && now.div_euclid(1000) != self.last_second)
    }

    /// A liquidation ping is sliding in or fading, or has just gone: the feed's
    /// layer is drawn again, and the chart under it is not.
    pub fn needs_feed(&self, now: i64) -> bool {
        // A hit moves only as it arrives and as it fades: between, it stands, and the feed rests.
        let moving = |l: &Liquidation| {
            let (was, age) = (self.feed_at - l.at, now - l.at);
            let passed = |at: i64| was < at && age >= at;
            age < LIQUIDATION_ENTER || (LIQUIDATION_TTL - LIQUIDATION_FADE..LIQUIDATION_TTL).contains(&age) || passed(LIQUIDATION_ENTER) || passed(LIQUIDATION_TTL)
        };
        self.feed_dirty || self.liquidations.iter().any(moving)
    }

    // ------------------------------------------------------------------ layout

    /// The pane a study draws in: its own group's, but Stoch RSI's apart from RSI's when the reader parts them.
    fn pane_group(&self, s: &ComputedStudy) -> Group {
        if self.pct_apart && s.kind == StudyKind::StochRsi { Group::Stoch } else { s.group }
    }

    /// RSI and Stoch RSI in one pane, or each in its own.
    pub fn set_pct_apart(&mut self, apart: bool) {
        if self.pct_apart != apart {
            self.pct_apart = apart;
            self.touch();
        }
    }

    fn visible_studies(&self) -> impl Iterator<Item = (usize, &ComputedStudy)> {
        self.studies.iter().chain(&self.formulas).filter(|s| !self.hidden.contains(&s.id)).enumerate()
    }

    fn total_bars(&self) -> usize {
        self.candles.len() + usize::from(self.live.is_some())
    }

    fn bar(&self, i: usize) -> Option<Bar> {
        if let Some(c) = self.candles.get(i) {
            return Some(Bar { time: c.open_time, open: c.open, high: c.high, low: c.low, close: c.close, volume: c.volume, live: false });
        }
        self.live.filter(|_| i == self.candles.len()).map(|l| Bar { time: l.open_time, open: l.open, high: l.high, low: l.low, close: l.close, volume: l.volume, live: true })
    }

    fn plot_width(&self) -> f32 {
        self.panes.first().map_or((self.width - self.axis_width).max(10.0), |p| p.plot.w)
    }

    fn time_axis_top(&self) -> f32 {
        (self.height - TIME_AXIS_HEIGHT).max(20.0)
    }

    fn layout(&mut self) {
        let mut groups = vec![Group::Price];
        for (_, s) in self.visible_studies() {
            let g = self.pane_group(s);
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
        for g in &groups {
            self.pane_state.entry(*g);
        }

        let p = self.precision();
        let (hi, lo) = self.candles.iter().fold((f64::NEG_INFINITY, f64::INFINITY), |(h, l), c| (h.max(c.high), l.min(c.low)));
        let widest = [format_price(hi, Some(p)), format_price(lo, Some(p)), "100.00".to_string()].iter().map(String::len).max().unwrap_or(6);
        self.axis_width = (widest as f32 * self.char_width + 18.0).max(MIN_AXIS_WIDTH).min(self.width * 0.45);
        let plot_w = (self.width - self.axis_width).max(10.0);
        let avail = self.time_axis_top();

        let total: f64 = groups.iter().map(|g| self.weight(*g)).sum();
        let mut panes = Vec::with_capacity(groups.len());
        let mut y = 0.0f32;
        for (i, g) in groups.iter().enumerate() {
            let h = if i + 1 == groups.len() { avail - y } else { ((f64::from(avail) * self.weight(*g) / total) as f32).max(MIN_PANE_HEIGHT) };
            let state = self.pane_state[g];
            let mode = if *g == Group::Price && self.log_scale { ScaleMode::Log } else { ScaleMode::Normal };
            let mut margins = state.margins;
            if *g == Group::Price && self.tpo().is_some() {
                // Profiles are read by their shape: every pixel of height is another row.
                margins = (margins.0.min(TPO_MARGINS.0), margins.1.min(TPO_MARGINS.1));
            }
            if *g == Group::Price && self.footprint_drawn() {
                // Every pixel of height is another row a bar can be read in.
                margins.0 = margins.0.min(0.1);
                // The footprint's table stands along the price pane's foot: the bars keep above it,
                // with a row's room besides, since a bar's lowest row reaches down past its low.
                if self.footprint_options.stats {
                    margins.1 = margins.1.max(f64::from(self.footprint_table(h.max(1.0)) + 24.0) / f64::from(h.max(1.0)));
                }
            }
            panes.push(PaneView { group: *g, plot: Rect { x: 0.0, y, w: plot_w, h: h.max(1.0) }, scale: PriceScale::new(mode, f64::from(y), f64::from(h.max(1.0)), margins) });
            y += h;
        }

        self.time.width = f64::from(plot_w);
        self.time.bars = self.total_bars();
        let warp = self.tpo_profiles.as_ref().filter(|_| self.tpo().is_some()).map(|p| p.warp.clone()).unwrap_or_default();
        self.time.set_warp(warp);
        let short = self.timeframe.source().interval.ms().is_some_and(|ms| ms * i64::from(self.timeframe.source().group) <= SHORT_BAR_MS);
        self.time.min_spacing = if self.tpo().is_some() {
            TPO_MIN_SPACING
        } else if short {
            MIN_BAR_SPACING.max(f64::from(plot_w) / MAX_SHOWN_SHORT_BARS)
        } else {
            MIN_BAR_SPACING
        };
        // Zoomed out past a floor raised since — a wider plot, a shorter timeframe — it holds.
        self.time.bar_spacing = self.time.bar_spacing.max(self.time.min_spacing);
        let opening = !self.view_ready && self.time.bars > 0;
        if opening {
            let (bars, margin) = self.opening_bars();
            self.time.show_last(bars, margin);
            self.view_ready = true;
            // New bars or a reset view: a price range set by hand for the old
            // view would leave the new candles off screen.
            let was = self.auto_scale();
            for state in self.pane_state.values_mut() {
                state.auto = true;
                state.manual = None;
            }
            self.note_auto(was);
        } else {
            self.time.clamp();
            // Bars and warp settled: the bar held goes back where it stood.
            if let Some((time, x)) = self.view_pin.take() {
                if let Some(logical) = time_to_logical(&self.times, time as f64) {
                    self.time.right_offset += (self.time.x(logical) - x) / self.time.bar_spacing;
                    self.time.clamp();
                }
            }
        }

        // A TPO chart opens on square blocks, as MMT's do: the fewest of the newest
        // profiles — a week's worth or more — whose prices fit at a row as tall as
        // a column is wide.
        if let (true, Some(profiles)) = (opening && self.tpo().is_some(), self.tpo_profiles.as_ref()) {
            let tall = panes[0].scale.height * (1.0 - panes[0].scale.margins.0 - panes[0].scale.margins.1);
            let margin = self.opening_bars().1;
            let (mut hi, mut lo, mut columns) = (f64::NEG_INFINITY, f64::INFINITY, margin);
            let mut best: Option<(f64, usize)> = None;
            for (n, p) in profiles.list.iter().rev().enumerate().take(TPO_OPENING_SESSIONS) {
                hi = hi.max(p.session.high);
                lo = lo.min(p.session.low);
                columns += p.columns;
                if n + 1 < TPO_FEWEST_SESSIONS.min(profiles.list.len()) {
                    continue;
                }
                // How much taller than the pane the prices are, at square rows.
                let spacing = self.time.width / columns;
                let over = (hi - lo) / (tall * profiles.row_size / spacing);
                if best.is_none_or(|(b, _)| over < b) {
                    best = Some((over, p.first));
                }
                if over <= 1.0 {
                    break;
                }
            }
            if let Some((_, first)) = best {
                self.time.show_last((self.time.bars - first) as f64, margin);
            }
        }
        self.fit_panes(&mut panes);
        self.panes = panes;
    }

    /// Each pane's price range: its own if set by hand, else what is on screen.
    fn fit_panes(&self, panes: &mut [PaneView]) {
        let (i0, i1) = self.visible_indices(0.0);
        for pane in panes {
            let state = self.pane_state[&pane.group];
            match (state.auto, state.manual) {
                (false, Some(range)) => pane.scale.range = range,
                _ => {
                    let (mut lo, mut hi) = self.pane_extent(pane.group, i0, i1);
                    // TPO blocks stay square: the range is never so tight that a row
                    // would stand taller than a column is wide.
                    if let Some(span) = self.tpo_square_span(pane).filter(|span| hi - lo < *span) {
                        let mid = (lo + hi) / 2.0;
                        (lo, hi) = (mid - span / 2.0, mid + span / 2.0);
                    }
                    pane.scale.fit(lo, hi);
                }
            }
        }
    }

    /// The price span that makes a TPO row as tall as a column is wide, on the price pane.
    fn tpo_square_span(&self, pane: &PaneView) -> Option<f64> {
        let profiles = self.tpo_profiles.as_ref().filter(|_| self.tpo().is_some())?;
        let scale = &pane.scale;
        let tall = scale.height * (1.0 - scale.margins.0 - scale.margins.1);
        (pane.group == Group::Price && scale.mode == ScaleMode::Normal && self.time.bar_spacing > 0.0).then(|| tall * profiles.row_size / self.time.bar_spacing)
    }

    /// Bar indices overlapping the plot, widened by `extra` bars each side.
    fn visible_indices(&self, extra: f64) -> (usize, usize) {
        let n = self.total_bars();
        if n == 0 {
            return (0, 0);
        }
        let (from, to) = self.time.visible();
        let i0 = (from.floor() - extra).within(0.0, (n - 1) as f64) as usize;
        let i1 = (to.ceil() + extra).within(0.0, (n - 1) as f64) as usize;
        (i0, i1)
    }

    fn pane_extent(&self, group: Group, i0: usize, i1: usize) -> (f64, f64) {
        if matches!(group, Group::Pct | Group::Stoch) {
            return (0.0, 100.0);
        }
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut take = |v: f64| {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        };
        // Indicators stop at the last closed bar; a series that runs into the forming one reaches it.
        let series = |values: &[Option<f64>], take: &mut dyn FnMut(f64)| {
            let end = i1.min(values.len().saturating_sub(1));
            for v in values.get(i0..=end).into_iter().flatten().flatten() {
                take(*v);
            }
        };
        if group == Group::Price {
            for i in i0..=i1 {
                if let Some(b) = self.bar(i) {
                    take(b.high);
                    take(b.low);
                }
            }
            // As TPO profiles and footprint columns do not draw the averages, the range
            // does not hold them either: an EMA 200 far off squeezed every row there was.
            let averages = self.tpo().is_none() && !self.footprint_drawn();
            if averages && !self.hidden.contains("ema200") {
                series(&self.engine.ema200, &mut take);
            }
            if averages && !self.hidden.contains("ribbon") {
                series(&self.engine.ema12, &mut take);
                series(&self.engine.ema25, &mut take);
            }
        }
        // VWAP levels are labels at the edge: a year's, far off, would squeeze the bars.
        for (_, study) in self.visible_studies().filter(|(_, s)| self.pane_group(s) == group && s.kind != StudyKind::VwapLevels) {
            for line in study.lines.iter().filter(|l| !self.styles.of(&study.id, l.key).off) {
                series(&line.values, &mut take);
                if line.kind == LineType::Histogram {
                    take(0.0);
                }
            }
            for bar in study.candles.get(i0..=i1.min(study.candles.len().saturating_sub(1))).into_iter().flatten().flatten() {
                take(bar[1]);
                take(bar[2]);
            }
        }
        if lo > hi { (0.0, 1.0) } else { (lo, hi) }
    }

    // ------------------------------------------------------------------ render

    pub fn render(&mut self, now: i64) -> &Frame {
        self.dirty = false;
        self.last_second = now.div_euclid(1000);
        let mut frame = std::mem::take(&mut self.frame);
        frame.begin(self.width, self.height, self.dpr, self.theme.background);
        if self.width >= 40.0 && self.height >= 60.0 {
            // Profiles first: the time scale is warped to their widths.
            self.refresh_tpo();
            self.layout();
            if self.total_bars() == 0 {
                let message = self.message.clone().unwrap_or_else(|| "Loading…".to_string());
                frame.text(self.width / 2.0, self.height / 2.0, message, self.theme.text, Align::Center, Baseline::Middle, self.theme.font_px, None, None);
            } else {
                self.paint(&mut frame, now);
                self.request_history_if_near_start();
                // The market just left, held until the next one's candles land.
                if self.loading {
                    frame.clip(None);
                    frame.rect(0.0, 0.0, self.width, self.height, self.theme.background.with_alpha(170));
                    frame.text(self.width / 2.0, self.height / 2.0, "Loading…".to_string(), self.theme.text, Align::Center, Baseline::Middle, self.theme.font_px, None, None);
                }
            }
        }
        self.frame = frame;
        &self.frame
    }

    /// The liquidation feed's layer, over the chart as last drawn: hits land
    /// above the volume and ride up and out. Empty while there is no chart to
    /// stand on, or the chart is between markets.
    pub fn render_feed(&mut self, now: i64) -> &Frame {
        self.feed_at = now;
        self.feed_dirty = false;
        let mut f = std::mem::take(&mut self.feed);
        f.begin(self.width, self.height, self.dpr, self.theme.background);
        if self.width >= 40.0 && self.height >= 60.0 && self.total_bars() > 0 && !self.loading && !self.panes.is_empty() && !self.hidden.contains("liquidations") {
            self.paint_feed(&mut f, now);
        }
        self.feed = f;
        &self.feed
    }

    fn paint_feed(&self, f: &mut Frame, now: i64) {
        let t = &self.theme;
        let price = self.panes[0];
        let plot_w = price.plot.w;
        f.clip(Some(price.plot));
        let size = t.font_px + 1.0;
        let mark = (size + 4.0).round();
        let row = mark + 4.0;
        // Monospace glyphs scale with the size, from the axis font's width.
        let glyph = self.char_width * size / t.font_px;
        // At the foot of the pane, where the volume is shortest and the
        // candles are not: the feed rises from there rather than sitting
        // among the bodies.
        let foot = price.plot.bottom() - 4.0;
        // The newest hit pushes the whole stack up as it arrives, rather
        // than snapping a row taller between one frame and the next.
        let smallest = self.styles.number("liquidations", "min", 0.0);
        let shift = match self.liquidations.iter().rfind(|l| l.usd >= smallest) {
            Some(l) if now - l.at < LIQUIDATION_ENTER => row * (1.0 - (now - l.at).max(0) as f32 / LIQUIDATION_ENTER as f32),
            _ => 0.0,
        };
        let (long, short) = (self.styles.color("liquidations", "long", t.bear), self.styles.color("liquidations", "short", t.bull));
        for (slot, l) in self.liquidations.iter().rev().filter(|l| l.usd >= smallest).take(LIQUIDATION_ROWS).enumerate() {
            let age = (now - l.at).max(0);
            if age >= LIQUIDATION_TTL {
                break;
            }
            let y = foot - slot as f32 * row + shift;
            if y - row < price.plot.y {
                break;
            }
            // Full strength in the middle of its life: it arrives with the
            // stack and leaves by fading where it stands.
            let fade = (age as f32 / LIQUIDATION_ENTER as f32).min((LIQUIDATION_TTL - age) as f32 / LIQUIDATION_FADE as f32).within(0.0, 1.0);
            // A long liquidated is forced selling, so it takes the bear colour.
            let base = if l.long { long } else { short };
            let alpha = (40.0 + 200.0 * fade) as u8;
            let text = format!("{} ${}", if l.long { "LONG" } else { "SHORT" }, format_turnover(l.usd));
            let width = mark + 4.0 + text.len() as f32 * glyph;
            let left = (plot_w - width - 8.0).max(0.0);
            f.rect(left - 4.0, y - row + 1.0, width + 8.0, row - 1.0, t.background.with_alpha((190.0 * fade) as u8));
            f.icon(left + mark / 2.0, y - row / 2.0, mark, alpha, l.mark, Some(price.plot));
            f.text(left + mark + 4.0, y - row / 2.0, text, base.faded(alpha), Align::Left, Baseline::Middle, size, None, Some(price.plot));
        }
    }

    /// Bars the chart opens on, and the room right of the last: a screenful of
    /// candles, or the last few TPO sessions with space for the current one to grow.
    fn opening_bars(&self) -> (f64, f64) {
        if self.footprint_shown && self.tpo().is_none() && self.time.width > 0.0 {
            return ((self.time.width / footprint::READABLE_SPACING).max(3.0), RIGHT_MARGIN_BARS);
        }
        let n = self.total_bars();
        let (Some(_), Some(profiles)) = (self.tpo(), self.tpo_profiles.as_ref()) else { return (DEFAULT_VISIBLE_BARS, RIGHT_MARGIN_BARS) };
        let from = profiles.list.len().saturating_sub(TPO_FALLBACK_SESSIONS);
        let first = profiles.list.get(from).map_or(0, |p| p.first);
        // Room for the current profile to widen, and for its lines to be read.
        let room = profiles.list.last().map_or(0.0, |p| p.widest as f64 * 0.5).max(TPO_RIGHT_COLUMNS);
        ((n - first).max(1) as f64, room)
    }

    /// Profiles for every split in the loaded bars, rebuilt only when a bar or
    /// an option they are built from changes.
    fn refresh_tpo(&mut self) {
        let Some(period) = self.tpo() else {
            self.tpo_profiles = None;
            return;
        };
        let bars: Vec<Candle> = (0..self.total_bars())
            .filter_map(|i| self.bar(i))
            .map(|b| Candle { open_time: b.time, open: b.open, high: b.high, low: b.low, close: b.close, volume: b.volume, quote_volume: None, taker_buy: None })
            .collect();
        if self.tpo_profiles.as_ref().is_some_and(|p| p.fresh(period, &bars, &self.footprint, &self.tpo_settings, &self.tpo_edits)) {
            return;
        }
        let tick = tpo::infer_tick(&bars, self.precision());
        self.tpo_profiles = tpo::build(&bars, period, tick, &self.footprint, &self.tpo_settings, &self.tpo_edits);
    }

    /// The profile under the pointer, by position, while the pointer is over the price pane.
    fn tpo_hovered(&self) -> Option<(usize, usize)> {
        let (x, y) = self.crosshair?;
        let pane = self.panes.first()?;
        let profiles = self.tpo_profiles.as_ref().filter(|_| self.tpo().is_some())?;
        if !(x >= 0.0 && x < pane.plot.w && y >= pane.plot.y && y <= pane.plot.bottom()) {
            return None;
        }
        let bar = self.time.logical(f64::from(x)).round();
        let bar = (bar >= 0.0).then_some(bar as usize)?;
        profiles.at_bar(bar).map(|k| (k, bar))
    }

    /// The tradoor mark, bottom left of the price pane, drawn on the icon's own
    /// sixteen-unit grid. Part of the frame itself, so every capture of a chart
    /// carries it — the name is not written out; the door is what is known.
    fn paint_watermark(&self, f: &mut Frame, plot: Rect) {
        let Some((x, y)) = mark_at(plot, self.table_top().unwrap_or(plot.bottom())).filter(|_| !self.page_mark) else { return };
        let u = MARK / 16.0;
        let (at, line) = (|dx: f32, dy: f32| (x + dx * u, y + dy * u), (1.5 * u).max(1.0));
        let accent = self.theme.ribbon_fast;
        f.stroke_rect(x + 1.8 * u, y + 3.0 * u, 7.6 * u, 10.0 * u, line, accent.with_alpha(150));
        let door = [(9.4, 3.0), (13.6, 1.2), (13.6, 14.8), (9.4, 13.0), (9.4, 3.0)];
        f.polyline(&door.iter().map(|(dx, dy)| at(*dx, *dy)).collect::<Vec<_>>(), line, accent.with_alpha(175));
        f.rect(x + 3.3 * u, y + 7.6 * u, 1.7 * u, 3.4 * u, accent.with_alpha(205));
        // The middle candle is the falling one, as the icon draws it.
        f.rect(x + 6.2 * u, y + 5.6 * u, 1.7 * u, 5.2 * u, self.theme.bear.with_alpha(205));
    }

    fn request_history_if_near_start(&mut self) {
        let Some(first) = self.candles.first().map(|c| c.open_time) else { return };
        if self.history_asked != Some(first) && self.time.visible().0 < HISTORY_MARGIN_BARS {
            self.history_asked = Some(first);
            self.events.push(Event::NeedHistory { before: first });
        }
    }

    fn line_color(&self, owner: &str, line: &tradoor_core::studies::StudyLine, study_index: usize, line_index: usize) -> Color {
        self.styles
            .of(owner, line.key)
            .color
            .or_else(|| line.rgba.map(Color::rgba))
            .or_else(|| line.color.and_then(Color::parse))
            .unwrap_or(self.theme.study_palette[(study_index + line_index) % self.theme.study_palette.len()])
    }

    /// A script's colour for candle `i`, where one colours them.
    fn bar_rgba(&self, i: usize) -> Option<Color> {
        self.formulas.iter().filter(|s| !self.hidden.contains(&s.id)).filter_map(|s| s.art.as_ref()?.bar_colors.get(i).copied()).filter(|c| *c != 0).last().map(Color::rgba)
    }

    /// What a script shades behind its bars, in its own pane: runs of a colour as one box each.
    fn paint_backgrounds(&self, f: &mut Frame, i0: usize, i1: usize) {
        let half = self.time.bar_spacing as f32 / 2.0;
        for study in self.formulas.iter().filter(|s| !self.hidden.contains(&s.id)) {
            let Some(art) = study.art.as_ref() else { continue };
            let Some(pane) = self.panes.iter().find(|p| p.group == study.group) else { continue };
            for colours in &art.backgrounds {
                let mut i = i0;
                while i <= i1.min(colours.len().saturating_sub(1)) && !colours.is_empty() {
                    let c = colours[i];
                    let mut j = i;
                    while j < i1.min(colours.len() - 1) && colours[j + 1] == c {
                        j += 1;
                    }
                    if c != 0 {
                        let (x0, x1) = (self.time.x(i as f64) as f32 - half, self.time.x(j as f64) as f32 + half);
                        f.rect(x0, pane.plot.y, x1 - x0, pane.plot.h, Color::rgba(c).shade());
                    }
                    i = j + 1;
                }
            }
        }
    }

    /// A script's shapes and table, in its pane: lines, boxes, then labels over them, then the table over all.
    fn paint_art(&self, f: &mut Frame, pane: &PaneView, art: &tradoor_core::studies::FormulaArt) {
        let t = &self.theme;
        let x = |l: f64| self.time.x(l) as f32;
        let y = |p: f64| pane.scale.y(p) as f32;
        let newest = self.candles.len() as f64 - 1.0;
        for b in &art.boxes {
            let (x0, x1, y0, y1) = (x(b.x1).min(x(b.x2)), x(b.x1).max(x(b.x2)), y(b.y1).min(y(b.y2)), y(b.y1).max(y(b.y2)));
            if b.fill != 0 {
                f.rect(x0, y0, x1 - x0, y1 - y0, Color::rgba(b.fill));
            }
            if b.border != 0 {
                f.stroke_rect(x0, y0, x1 - x0, y1 - y0, b.width, Color::rgba(b.border));
            }
        }
        for l in &art.lines {
            if l.color == 0 {
                continue;
            }
            let (mut a, mut b) = ((x(l.x1), y(l.y1)), (x(l.x2), y(l.y2)));
            // Extended to past the pane's edge, along its own direction.
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            if l.extend != 0 && len > 0.0 {
                let far = (pane.plot.w + pane.plot.h) * 2.0 / len;
                if l.extend & 1 != 0 {
                    a = (a.0 - dx * far, a.1 - dy * far);
                }
                if l.extend & 2 != 0 {
                    b = (b.0 + dx * far, b.1 + dy * far);
                }
            }
            let color = Color::rgba(l.color);
            match l.style {
                1 => f.dashed_line(a.0, a.1, b.0, b.1, l.width, color, 6.0, 4.0),
                2 => f.dashed_line(a.0, a.1, b.0, b.1, l.width, color, l.width.max(1.5), 3.0),
                _ => f.line(a.0, a.1, b.0, b.1, l.width, color),
            }
        }
        for l in &art.labels {
            let size = t.font_px * [0.8, 0.9, 1.0, 1.25][usize::from(l.size.min(3))];
            let lines: Vec<&str> = l.text.split('\n').collect();
            let widest = lines.iter().map(|s| s.chars().count()).max().unwrap_or(0) as f32;
            let (pad_x, pad_y, gap, leading) = (5.0, 3.0, 7.0, size * 1.3);
            let (w, h) = (widest * self.char_width * size / t.font_px + pad_x * 2.0, lines.len() as f32 * leading + pad_y * 2.0);
            let (px, py) = (x(l.x), y(l.y));
            let (mut left, top) = match l.place {
                0 => (px - w / 2.0, py - gap - h),
                1 => (px - w / 2.0, py + gap),
                2 => (px - gap - w, py - h / 2.0),
                3 => (px + gap, py - h / 2.0),
                _ => (px - w / 2.0, py - h / 2.0),
            };
            // Kept whole in the pane, moved along: one at a bar in view, and one at the newest bar or
            // past it, which would run off the right edge (the chart keeps little room after its bars).
            let plot = pane.plot;
            let ahead = l.x >= newest;
            let was = left;
            if (ahead || (px >= plot.x && px <= plot.x + plot.w)) && w + 4.0 < plot.w {
                left = left.min(plot.x + plot.w - w - 2.0).max(plot.x + 2.0);
            }
            let moved = left != was;
            let boxed = !l.plain && l.color != 0;
            if boxed {
                let fill = Color::rgba(l.color);
                f.rect(left, top, w, h, fill);
                // A point from the box to its price.
                match l.place {
                    0 => f.triangle((px - 4.0, top + h), (px + 4.0, top + h), (px, py - 2.0), fill),
                    1 => f.triangle((px - 4.0, top), (px + 4.0, top), (px, py + 2.0), fill),
                    2 if !moved => f.triangle((left + w, py - 4.0), (left + w, py + 4.0), (px - 2.0, py), fill),
                    3 if !moved => f.triangle((left, py - 4.0), (left, py + 4.0), (px + 2.0, py), fill),
                    _ => {}
                }
            }
            let ink = if l.text_color != 0 {
                Color::rgba(l.text_color)
            } else if boxed {
                Color::rgba(l.color).ink()
            } else if l.color != 0 {
                Color::rgba(l.color)
            } else {
                t.text
            };
            for (k, line) in lines.iter().enumerate() {
                f.text(left + w / 2.0, top + pad_y + leading * (k as f32 + 0.5), (*line).to_string(), ink, Align::Center, Baseline::Middle, size, None, Some(pane.plot));
            }
        }
        if let (Some((down, across)), false) = (art.table, art.cells.is_empty()) {
            let size = t.font_px - 1.0;
            let cols = art.cells.iter().map(|c| usize::from(c.col) + 1).max().unwrap_or(0);
            let rows = art.cells.iter().map(|c| usize::from(c.row) + 1).max().unwrap_or(0);
            let mut widths = vec![0f32; cols];
            for c in &art.cells {
                let w = c.text.chars().count() as f32 * self.char_width * size / t.font_px + 14.0;
                widths[usize::from(c.col)] = widths[usize::from(c.col)].max(w);
            }
            let (row_h, margin) = (size * 1.6, 8.0);
            let (tw, th) = (widths.iter().sum::<f32>(), rows as f32 * row_h);
            let plot = pane.plot;
            let left = match across {
                0 => plot.x + margin,
                1 => plot.x + (plot.w - tw) / 2.0,
                _ => plot.right() - tw - margin,
            };
            let top = match down {
                0 => plot.y + margin,
                1 => plot.y + (plot.h - th) / 2.0,
                _ => plot.bottom() - th - margin,
            };
            f.rect(left, top, tw, th, t.label_background.with_alpha(230));
            for c in &art.cells {
                let cx = left + widths[..usize::from(c.col)].iter().sum::<f32>();
                let cy = top + f32::from(c.row) * row_h;
                let w = widths[usize::from(c.col)];
                if c.background != 0 {
                    f.rect(cx, cy, w, row_h, Color::rgba(c.background));
                }
                let ink = if c.color != 0 { Color::rgba(c.color) } else { t.text };
                f.text(cx + 7.0, cy + row_h / 2.0, c.text.clone(), ink, Align::Left, Baseline::Middle, size, None, Some(plot));
            }
        }
    }

    /// A line coloured bar by bar: each stretch in the colour of the bar it ends at.
    #[allow(clippy::too_many_arguments)]
    fn paint_coloured(&self, f: &mut Frame, scale: &PriceScale, values: &[Option<f64>], colours: &[u32], i0: usize, i1: usize, width: f32, pattern: Pattern) {
        let end = i1.min(values.len().saturating_sub(1));
        let mut prev: Option<(f32, f32)> = None;
        for i in i0..=end {
            let Some(v) = values[i] else {
                prev = None;
                continue;
            };
            let p = (self.time.x(i as f64) as f32, scale.y(v) as f32);
            let c = colours.get(i).copied().unwrap_or(0);
            if let Some(a) = prev
                && c != 0
            {
                stroke(f, &[a, p], width, Color::rgba(c), pattern);
            }
            prev = Some(p);
        }
    }

    fn paint(&self, f: &mut Frame, now: i64) {
        let t = &self.theme;
        let n = self.total_bars();
        let (i0, i1) = self.visible_indices(1.0);
        let (from, to) = self.time.visible();
        let price = self.panes[0];
        let plot_w = price.plot.w;
        let avail = self.time_axis_top();

        // ---- guides under everything in the lower panes
        let guides = self.styles.of("pct", "guides");
        for pane in self.panes.iter().filter(|p| matches!(p.group, Group::Pct | Group::Stoch) && !guides.off) {
            f.clip(Some(pane.plot));
            for level in [self.styles.number("pct", "lower", 20.0), self.styles.number("pct", "upper", 80.0)] {
                let y = pane.scale.y(level) as f32;
                f.dashed_line(0.0, y, plot_w, y, guides.width.unwrap_or(1.0), guides.color.unwrap_or(t.border), 4.0, 4.0);
            }
        }

        // ---- price pane: the ribbon under the candles, volume at the foot, and
        // the mark over the volume — the bars sit where it does and would bury it
        f.clip(Some(price.plot));
        let tpo = self.tpo().is_some() && self.tpo_profiles.is_some();
        let foot = !tpo && self.footprint_drawn();
        let mut tpo_tags = Vec::new();
        if let (true, Some(profiles)) = (tpo, self.tpo_profiles.as_ref()) {
            let bars: Vec<Candle> = (0..n).filter_map(|i| self.bar(i)).map(|b| Candle { open_time: b.time, open: b.open, high: b.high, low: b.low, close: b.close, volume: b.volume, quote_volume: None, taker_buy: None }).collect();
            let edge_x = |e: f64| self.time.edge_x(e) as f32;
            let y = |p: f64| price.scale.y(p) as f32;
            let hovered = self.tpo_hovered().filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).map(|(k, _)| k);
            let view = tpo::View { plot: price.plot, column: self.time.bar_spacing as f32, char_width: self.char_width, theme: t, edge_x: &edge_x, y: &y, bars: &bars, hovered };
            tpo_tags = tpo::paint(f, &view, profiles, &self.tpo_settings);
        }
        let ribbon_style = |bullish: bool| self.styles.of("ribbon", if bullish { "bull" } else { "bear" });
        self.paint_backgrounds(f, i0, i1);
        if !tpo && !foot && !self.hidden.contains("ribbon") {
            for run in self.ribbon.iter().filter(|run| !ribbon_style(run.bullish).off) {
                let pts = visible_slice(&run.points, from - 1.0, to + 1.0);
                if pts.len() < 2 {
                    continue;
                }
                let upper: Vec<(f32, f32)> = pts.iter().map(|p| (self.time.x(p.logical) as f32, price.scale.y(p.fast) as f32)).collect();
                let lower: Vec<(f32, f32)> = pts.iter().map(|p| (self.time.x(p.logical) as f32, price.scale.y(p.slow) as f32)).collect();
                let color = if run.bullish { t.ribbon_fast } else { t.ribbon_slow };
                f.band(&upper, &lower, ribbon_style(run.bullish).color.unwrap_or(color.with_alpha(t.ribbon_fill_alpha)));
            }
        }
        let dpr = f.dpr;
        let (body_dev, wick) = candle_widths(self.time.bar_spacing, dpr);
        let body = body_dev / dpr;
        if !tpo && !foot && !self.hidden.contains("volume") {
            let max_vol = (i0..=i1).filter_map(|i| self.bar(i)).map(|b| b.volume).fold(0.0, f64::max);
            if max_vol > 0.0 {
                let share = self.styles.number("volume", "height", 18.0).within(5.0, 60.0) as f32 / 100.0;
                let (top, h) = (price.plot.y + price.plot.h * (1.0 - share), price.plot.h * share);
                let (up, down) = (self.styles.color("volume", "up", t.bull.with_alpha(t.volume_alpha)), self.styles.color("volume", "down", t.bear.with_alpha(t.volume_alpha)));
                for i in i0..=i1 {
                    let Some(b) = self.bar(i) else { continue };
                    let y = top + h * (1.0 - (b.volume / max_vol) as f32);
                    let base = if b.close >= b.open { up } else { down };
                    let x = f.snap(self.time.x(i as f64) as f32);
                    f.crisp_rect(x - body / 2.0, y, body, price.plot.bottom() - y, if b.live { base.with_alpha(0x25) } else { base });
                }
            }
        }
        self.paint_watermark(f, price.plot);
        for i in (i0..=i1).filter(|_| !tpo && !foot) {
            let Some(b) = self.bar(i) else { continue };
            let s = &price.scale;
            let x = f.snap(self.time.x(i as f64) as f32);
            let base = self.bar_rgba(i).unwrap_or(if b.close >= b.open { t.bull } else { t.bear });
            let fill = base.with_alpha(if b.live { 0x80 } else { t.candle_alpha });
            let (yh, yl, yo, yc) = (s.y(b.high) as f32, s.y(b.low) as f32, s.y(b.open) as f32, s.y(b.close) as f32);
            let (top, bottom) = (yo.min(yc), yo.max(yc));
            if body_dev < 2.0 {
                f.vline(x, yh, yl, wick, fill);
                continue;
            }
            if top - yh > 0.0 {
                f.vline(x, yh, top, wick, fill);
            }
            if yl - bottom > 0.0 {
                f.vline(x, bottom, yl, wick, fill);
            }
            f.crisp_rect(x - body / 2.0, top, body, (bottom - top).max(1.0 / dpr), fill);
            if b.live {
                f.stroke_rect(x - body / 2.0, top, body, (bottom - top).max(1.0 / dpr), 1.0 / dpr, base);
            }
        }
        if foot {
            // A thin candle at the left of every slot, the cells beside it: a
            // bar the server never recorded still says where it went.
            for i in i0..=i1 {
                let Some(b) = self.bar(i) else { continue };
                let s = footprint::slot(self.time.x(i as f64) as f32, self.time.bar_spacing as f32);
                if s.candle_width <= 0.0 {
                    continue;
                }
                let sc = &price.scale;
                let color = if b.close >= b.open { t.bull } else { t.bear };
                let (yh, yl, yo, yc) = (sc.y(b.high) as f32, sc.y(b.low) as f32, sc.y(b.open) as f32, sc.y(b.close) as f32);
                let x = f.snap(s.candle);
                f.vline(x, yh, yl, 1.0, color);
                f.crisp_rect(x - s.candle_width / 2.0, yo.min(yc), s.candle_width, (yo - yc).abs().max(1.0), color);
            }
            let x = |l: f64| self.time.x(l) as f32;
            let y = |p: f64| price.scale.y(p) as f32;
            let view = footprint::View { plot: price.plot, bar_spacing: self.time.bar_spacing as f32, table: self.footprint_table(price.plot.h), char_width: self.char_width, theme: t, x: &x, y: &y };
            // Each column at the place its own time sits, so a minute the
            // server never recorded leaves a gap rather than shifting the rest.
            let newest = self.newest_time();
            let placed: Vec<(f64, &footprint::Bar)> =
                self.footprint.iter().filter(|bar| bar.time <= newest).filter_map(|bar| time_to_logical(&self.times, bar.time as f64).map(|l| (l, bar))).collect();
            footprint::paint(f, &view, &placed, self.footprint_options, self.footprint_step(&price));
            // Its top edge lights up when it is about to be taken, or is held.
            if let Some(top) = self.table_top().filter(|_| matches!(self.drag, Drag::Table { .. }) || self.crosshair.is_some_and(|(x, y)| x < price.plot.w && self.tool.is_none() && self.on_table_edge(y))) {
                f.hline(price.plot.x, price.plot.right(), top, 1.0, t.text.with_alpha(170));
            }
        }
        if !tpo && !foot && !self.hidden.contains("ema200") {
            let style = self.styles.of("ema200", "line");
            self.paint_series(f, &price.scale, &self.engine.ema200, i0, i1, style.color.unwrap_or(t.ema200), style.width.unwrap_or(1.0), LineStyle::Solid, style.pattern.unwrap_or(Pattern::Solid));
        }

        // ---- studies, each in its pane
        for (index, study) in self.visible_studies() {
            let Some(pane) = self.panes.iter().find(|p| p.group == self.pane_group(study)) else { continue };
            f.clip(Some(pane.plot));
            self.paint_study_candles(f, &pane.scale, &study.candles, i0, i1, body, &study.id);
            // A line can run past the last bar: to what is on screen, not to the bars.
            let far = (to.ceil() + 1.0).max(0.0) as usize;
            for fill in &study.fills {
                self.paint_fill(f, &pane.scale, study, fill, i0, far);
            }
            self.paint_boxes(f, &pane, study);
            if study.kind == StudyKind::VisibleProfile {
                self.paint_visible_profile(f, &pane, study);
            }
            let mut tags = Vec::new();
            for (li, line) in study.lines.iter().enumerate() {
                let own = self.styles.of(&study.id, line.key);
                match (line.kind, line.style) {
                    _ if own.off => {}
                    (LineType::Histogram, _) => {
                        // A formula's histogram in the colour it names, or bar by bar; else above zero and below in the theme's.
                        let own_colour = (study.kind == StudyKind::Formula).then(|| own.color.or_else(|| line.rgba.map(Color::rgba)).or_else(|| line.color.and_then(Color::parse))).flatten();
                        let bars = line.bar_rgba.as_deref().filter(|_| own.color.is_none());
                        self.paint_histogram(f, &pane.scale, &line.values, i0, i1, body, &study.id, own_colour, bars);
                    }
                    (LineType::Line, LineStyle::None) => {}
                    (LineType::Line, _) if line.dots => {
                        let color = self.line_color(&study.id, line, index, li);
                        for (i, v) in line.values.iter().enumerate().take(far.saturating_add(1)).skip(i0) {
                            let Some(v) = v else { continue };
                            let c = line.bar_rgba.as_ref().filter(|_| own.color.is_none()).and_then(|b| b.get(i).copied()).map_or(color, Color::rgba);
                            if c.a > 0 {
                                f.circle(self.time.x(i as f64) as f32, pane.scale.y(*v) as f32, own.width.unwrap_or(line.width).max(1.0) + 0.5, c);
                            }
                        }
                    }
                    (LineType::Line, _) if line.bar_rgba.is_some() && own.color.is_none() => {
                        let colours = line.bar_rgba.as_deref().unwrap_or(&[]);
                        self.paint_coloured(f, &pane.scale, &line.values, colours, i0, far, own.width.unwrap_or(line.width), own.pattern.unwrap_or(line.pattern));
                    }
                    (LineType::Line, LineStyle::Tag) => tags.extend(line.values.iter().rev().flatten().next().map(|v| (pane.scale.y(*v) as f32, line.label.clone(), self.line_color(&study.id, line, index, li)))),
                    (LineType::Line, style) => self.paint_series(f, &pane.scale, &line.values, i0, far, self.line_color(&study.id, line, index, li), own.width.unwrap_or(line.width), style, own.pattern.unwrap_or(line.pattern)),
                }
            }
            self.paint_tags(f, &pane, tags);
            let last = self.times.len().saturating_sub(1) as f64;
            let ahead = self.styles.number(&study.id, "ahead", LEVEL_AHEAD_BARS).within(0.0, 500.0).round();
            for seg in &study.segments {
                let own = self.styles.of(&study.id, seg.key);
                if own.off {
                    continue;
                }
                let (Some(l0), Some(l1)) = (time_to_logical(&self.times, seg.from as f64), time_to_logical(&self.times, seg.to as f64)) else { continue };
                let (x0, mut x1) = (self.time.x(l0) as f32, self.time.x(l1) as f32);
                // A level still in force runs on into the space ahead, as far as its label still fits.
                if l1 >= last && ahead > 0.0 {
                    let label = seg.label.len() as f32 * self.char_width * (t.font_px - 1.0) / t.font_px;
                    x1 = x1.max((self.time.x(last + ahead) as f32).min(plot_w - label - 9.0));
                }
                if x1 < 0.0 || x0 > plot_w {
                    continue;
                }
                let y = pane.scale.y(seg.price) as f32;
                let color = own.color.or_else(|| Color::parse(seg.color)).unwrap_or(t.text);
                f.hline(x0, x1, y, own.width.unwrap_or(1.0), color);
                f.text(x1 + 5.0, y - 3.0, seg.label, color, Align::Left, Baseline::Bottom, t.font_px - 1.0, None, Some(pane.plot));
            }
            for m in &study.markers {
                let own = self.styles.of(&study.id, m.plot);
                let Ok(i) = self.candles.binary_search_by_key(&m.time, |c| c.open_time) else { continue };
                if own.off || i < i0 || i > i1 {
                    continue;
                }
                let c = &self.candles[i];
                let x = self.time.x(i as f64) as f32;
                let color = own.color.unwrap_or(match m.tone {
                    Tone::Bull => t.bull,
                    Tone::Bear => t.bear,
                    Tone::Neutral => t.text,
                    Tone::Ink(ink) => Color::parse(ink).unwrap_or(t.text),
                    Tone::Rgba(v) => Color::rgba(v),
                });
                let radius = if m.text.is_empty() { 5.0 } else { 2.5 };
                let y = if m.above { pane.scale.y(c.high) as f32 - 3.0 - 2.0 * radius } else { pane.scale.y(c.low) as f32 + 3.0 + 2.0 * radius };
                if m.hollow {
                    f.ring(x, y, radius, 1.5, color);
                } else {
                    f.circle(x, y, radius, color);
                }
                if !m.text.is_empty() {
                    let (gap, baseline) = if m.above { (-5.0, Baseline::Bottom) } else { (5.0, Baseline::Top) };
                    f.text(x, y + gap, m.text.clone(), color, Align::Center, baseline, t.font_px - 1.0, None, Some(pane.plot));
                }
            }
            if let Some(art) = study.art.as_ref() {
                self.paint_art(f, &pane, art);
            }
        }

        // ---- flow marks: a minute that ran hard one way, where it traded
        if self.flow_shown() && !self.flow.is_empty() {
            let hovered = self.crosshair.filter(|_| self.tool.is_none()).and_then(|(x, y)| self.flow_at(x, y)).map(|(i, _, _)| i);
            for (i, mark) in self.flow.iter().enumerate() {
                let Some((x, y)) = self.flow_xy(mark, &price) else { continue };
                if x < -FLOW_MAX_RADIUS || x > plot_w + FLOW_MAX_RADIUS {
                    continue;
                }
                let own = self.flow_style(mark);
                if own.off {
                    continue;
                }
                let base = own.color.unwrap_or(if mark.usd > 0.0 { t.bull } else { t.bear });
                let on = hovered == Some(i);
                f.circle(x, y, self.flow_radius(mark), base.faded(if on { 220 } else { 90 }));
                if on {
                    f.ring(x, y, self.flow_radius(mark) + 2.0, 1.0, base);
                }
            }
        }

        // ---- ribbon crosses, last price, drawings
        f.clip(Some(price.plot));
        if !tpo && !self.hidden.contains("ribbon") && !self.styles.of("ribbon", "marks").off {
            let (rising, falling) = (self.styles.color("ribbon", "bull", t.ribbon_fast).with_alpha(255), self.styles.color("ribbon", "bear", t.ribbon_slow).with_alpha(255));
            for cross in self.engine.crosses.iter().filter(|c| c.index >= i0 && c.index <= i1) {
                let c = &self.candles[cross.index];
                let x = self.time.x(cross.index as f64) as f32;
                if cross.bull {
                    let tip = price.scale.y(c.low) as f32 + 6.0;
                    f.triangle((x, tip), (x - 5.0, tip + 8.0), (x + 5.0, tip + 8.0), rising);
                    f.text(x, tip + 10.0, "Bull", rising, Align::Center, Baseline::Top, t.font_px - 1.0, None, Some(price.plot));
                } else {
                    let tip = price.scale.y(c.high) as f32 - 6.0;
                    f.triangle((x, tip), (x - 5.0, tip - 8.0), (x + 5.0, tip - 8.0), falling);
                    f.text(x, tip - 10.0, "Bear", falling, Align::Center, Baseline::Bottom, t.font_px - 1.0, None, Some(price.plot));
                }
            }
        }
        let last = self.bar(n - 1).expect("n > 0");
        let last_color = if last.close >= last.open { t.bull } else { t.bear };
        let last_y = price.scale.y(last.close) as f32;
        f.dashed_line(0.0, last_y, plot_w, last_y, 1.0, last_color, 2.0, 2.0);

        // The drawing under the pointer, for what shows only on hover.
        let hovered = self.crosshair.filter(|_| self.tool.is_none()).and_then(|(x, y)| self.drawing_at(x, y));
        for mut placed in self.drawings.iter().map(|d| (d, false)).chain(self.draft.iter().map(|d| (d, true))).filter_map(|(d, draft)| self.place(d, draft)) {
            placed.hovered = placed.shape.id.is_some() && placed.shape.id == hovered;
            f.clip(Some(placed.plot));
            drawings::paint(f, &placed, placed.plot, t);
        }
        f.clip(Some(price.plot));

        // Price alerts: a level in its own style (dashed unless set otherwise),
        // solid while the pointer holds it, with a ring by the axis as
        // TradingView marks them. The ring and the badge keep the colour whole,
        // however faint the line, so the level can always be found and read.
        let held_alert = match self.drag {
            Drag::Alert { id, .. } => Some(id),
            _ => self.crosshair.filter(|_| self.tool.is_none()).and_then(|(x, y)| self.alert_handle_at(x, y)).map(|(id, _)| id),
        };
        for a in &self.alerts {
            let y = f.snap(price.scale.y(a.price) as f32);
            let color = a.color.unwrap_or(t.alert);
            let picked = self.selected_alert == Some(a.id);
            match a.dash {
                _ if held_alert == Some(a.id) || picked => f.hline(0.0, plot_w, y, a.width, color),
                Dash::Solid => f.hline(0.0, plot_w, y, a.width, color),
                Dash::Dashed => f.dashed_line(0.0, y, plot_w, y, a.width, color, 4.0, 3.0),
                Dash::Dotted => f.dashed_line(0.0, y, plot_w, y, a.width, color, a.width.max(1.5), 3.0),
            }
            // Picked, the ring fills: it can be dragged now.
            f.circle(plot_w - 10.0, y, if picked { 5.5 } else { 4.5 }, color.with_alpha(255));
            if !picked {
                f.circle(plot_w - 10.0, y, 3.0, t.background);
            }
        }

        // ---- crosshair
        let hover = self.crosshair.filter(|(x, y)| *x >= 0.0 && *x < plot_w && *y >= 0.0 && *y < avail && !matches!(self.drag, Drag::Separator { .. } | Drag::Table { .. }));
        // With the magnet on, the crosshair stands where an anchor would land, as
        // TradingView's does: on the bar's open, high, low or close — always when
        // strong, when near one when weak (or Ctrl held), else on a whole tick.
        let snapped = hover.and_then(|(x, y)| self.point_in(0, x, y).filter(|_| self.magnet_now() != Magnet::Off && self.pane_at(y) == Some(0))).map(|a| a.price);
        let hover = hover.map(|(x, y)| (x, snapped.map_or(y, |p| price.scale.y(p) as f32)));
        // Another chart's crosshair, in a layout keeping them in step, stands where this one's is not.
        let hover_slot = match hover {
            Some((x, _)) => self.slot_at(x),
            None => self.linked_slot(),
        };
        f.clip(None);
        if let Some(slot) = hover_slot {
            let x = f.snap(self.time.x(slot as f64) as f32);
            f.dashed_line(x, 0.0, x, avail, 1.0, t.crosshair, 4.0, 4.0);
            if let Some((_, cy)) = hover.filter(|(_, cy)| self.pane_at(*cy).is_some()) {
                f.dashed_line(0.0, cy, plot_w, cy, 1.0, t.crosshair, 4.0, 4.0);
            }
        }

        // ---- chrome
        f.vline(plot_w, 0.0, avail, 1.0, t.border);
        f.hline(0.0, self.width, avail, 1.0, t.border);
        for pane in &self.panes[1..] {
            f.hline(0.0, self.width, pane.plot.y, 1.0, t.border);
        }

        // ---- badges: last values, countdown, crosshair price — placed first so
        // tick labels can step out of their way
        let font = t.font_px;
        let badge_h = font + 6.0;
        struct Badge {
            pane: usize,
            y: f32,
            text: String,
            bg: Color,
            fg: Color,
        }
        let mut badges: Vec<Badge> = Vec::new();
        let p = self.precision();
        badges.push(Badge { pane: 0, y: last_y, text: format_price(last.close, Some(p)), bg: last_color, fg: last_color.contrast_text() });
        if !self.replay && (self.timeframe.source().interval.ms().is_some() || self.timeframe.source().interval == Interval::Mo1) {
            let close_at = self.timeframe.end(self.timeframe.align(now));
            badges.push(Badge { pane: 0, y: last_y + badge_h, text: format_countdown(close_at - now), bg: t.countdown_background, fg: t.countdown_text });
        }
        for tag in &tpo_tags {
            badges.push(Badge { pane: 0, y: price.scale.y(tag.price) as f32, text: format_price(tag.price, Some(p)), bg: tag.color.with_alpha(255), fg: tag.color.with_alpha(255).contrast_text() });
        }
        for a in &self.alerts {
            let bg = a.color.unwrap_or(t.alert).with_alpha(255);
            badges.push(Badge { pane: 0, y: price.scale.y(a.price) as f32, text: format_price(a.price, Some(p)), bg, fg: bg.contrast_text() });
        }
        let last_value = |values: &[Option<f64>]| values.iter().rev().flatten().next().copied();
        if !self.hidden.contains("ema200") {
            if let Some(v) = last_value(&self.engine.ema200) {
                let color = self.styles.color("ema200", "line", t.ema200);
                badges.push(Badge { pane: 0, y: price.scale.y(v) as f32, text: format_price(v, Some(p)), bg: color, fg: color.contrast_text() });
            }
        }
        for (index, study) in self.visible_studies() {
            let Some(pane_index) = self.panes.iter().position(|pn| pn.group == self.pane_group(study)) else { continue };
            let pane = self.panes[pane_index];
            for (li, line) in study.lines.iter().enumerate().filter(|(_, l)| l.kind == LineType::Line && l.badge && !self.styles.of(&study.id, l.key).off) {
                if let Some(v) = last_value(&line.values) {
                    // Candles take the colour of the last one, as the price badge does.
                    let color = match study.candles.iter().rev().flatten().next() {
                        Some(b) if b[3] >= b[0] => self.styles.color(&study.id, "up", t.bull),
                        Some(_) => self.styles.color(&study.id, "down", t.bear),
                        None => self.line_color(&study.id, line, index, li),
                    };
                    badges.push(Badge { pane: pane_index, y: pane.scale.y(v) as f32, text: self.format_value(study.group, v), bg: color, fg: color.contrast_text() });
                }
            }
        }
        tradoor_core::sort::insertion_sort_by(&mut badges, |a, b| (a.pane, a.y) < (b.pane, b.y));
        let mut previous: Option<(usize, f32)> = None;
        for badge in &mut badges {
            if let Some((pane, y)) = previous.filter(|(pane, _)| *pane == badge.pane) {
                let _ = pane;
                badge.y = badge.y.max(y + badge_h);
            }
            previous = Some((badge.pane, badge.y));
        }
        let crosshair_price = hover.and_then(|(_, cy)| self.pane_at(cy).map(|i| (i, cy)));

        // ---- price tick labels, skipping any a badge sits on
        for (pi, pane) in self.panes.iter().enumerate() {
            let axis = Rect { x: plot_w, y: pane.plot.y, w: self.axis_width, h: pane.plot.h };
            let ticks = ticks::price_ticks(&pane.scale, 44.0);
            let decimals = match pane.group {
                Group::Price => p,
                Group::Pct | Group::Stoch => 2,
                _ => ticks::step_decimals(&ticks).max(2),
            };
            let label = |v: f64| if matches!(pane.group, Group::Oi | Group::Delta) { format_compact(v) } else { format_price(v, Some(decimals)) };
            for v in ticks {
                let y = pane.scale.y(v) as f32;
                let near_badge = badges.iter().any(|b| b.pane == pi && (b.y - y).abs() < badge_h);
                let near_cross = crosshair_price.is_some_and(|(i, cy)| i == pi && (cy - y).abs() < badge_h);
                if y < pane.plot.y + 6.0 || y > pane.plot.bottom() - 6.0 || near_badge || near_cross {
                    continue;
                }
                f.text(plot_w + 6.0, y, label(v), t.text, Align::Left, Baseline::Middle, font, None, Some(axis));
            }
            if matches!(pane.group, Group::Pct | Group::Stoch) {
                for level in [20.0, 80.0] {
                    let y = pane.scale.y(level) as f32;
                    f.crisp_rect(plot_w + 1.0, y - badge_h / 2.0, self.axis_width - 1.0, badge_h, t.border);
                    f.text(plot_w + 6.0, y, format_price(level, Some(2)), t.text, Align::Left, Baseline::Middle, font, None, Some(axis));
                }
            }
        }
        for badge in &badges {
            let pane = self.panes[badge.pane];
            if badge.y < pane.plot.y || badge.y > pane.plot.bottom() {
                continue;
            }
            let mut axis = Rect { x: plot_w, y: pane.plot.y, w: self.axis_width, h: pane.plot.h };
            // The crosshair's price is drawn over every badge, and text lies above every
            // box: a badge's text stops at the crosshair's, so none shows through it,
            // and one the crosshair covers the middle of shows none.
            if let Some((_, cy)) = crosshair_price.filter(|(i, _)| *i == badge.pane) {
                let bottom = axis.bottom();
                if (badge.y - cy).abs() < badge_h / 2.0 {
                    axis.h = 0.0;
                } else if badge.y < cy {
                    axis.h = (cy - badge_h / 2.0 - axis.y).max(0.0);
                } else {
                    axis.y = axis.y.max(cy + badge_h / 2.0);
                    axis.h = (bottom - axis.y).max(0.0);
                }
            }
            f.crisp_rect(plot_w + 1.0, badge.y - badge_h / 2.0, self.axis_width - 1.0, badge_h, badge.bg);
            f.text(plot_w + 6.0, badge.y, badge.text.clone(), badge.fg, Align::Left, Baseline::Middle, font, None, Some(axis));
        }
        if let Some((pi, cy)) = crosshair_price {
            let pane = self.panes[pi];
            f.crisp_rect(plot_w + 1.0, cy - badge_h / 2.0, self.axis_width - 1.0, badge_h, t.label_background);
            let value = snapped.filter(|_| pi == 0).unwrap_or_else(|| pane.scale.price(f64::from(cy)));
            f.text(plot_w + 6.0, cy, self.format_value(pane.group, value), t.countdown_text, Align::Left, Baseline::Middle, font, None, None);
        }

        // ---- time labels
        let time_axis = Rect { x: 0.0, y: avail, w: plot_w, h: TIME_AXIS_HEIGHT };
        let cross_label = hover_slot.and_then(|slot| Some((slot, self.slot_time(slot)?))).map(|(slot, time)| {
            let monthly = self.timeframe.source().interval == Interval::Mo1;
            let text = ticks::crosshair_label(self.shown_time(time), self.timeframe.source().interval.ms(), monthly);
            let w = text.len() as f32 * self.char_width + 12.0;
            (self.time.x(slot as f64) as f32, text, w)
        });
        let mut candidates = Vec::new();
        // Under profiles, each one's day where it starts.
        if let (true, Some(profiles), Some(period)) = (tpo, self.tpo_profiles.as_ref(), self.tpo()) {
            let gap = self.char_width * 11.0;
            let mut last_right = f32::NEG_INFINITY;
            for profile in &profiles.list {
                let x = self.time.edge_x(profile.first as f64) as f32;
                let text = tpo::axis_label(period, self.shown_time(profile.start));
                let w = text.len() as f32 * self.char_width;
                if x < -w || x > plot_w || x < last_right + self.char_width * 2.0 {
                    continue;
                }
                if cross_label.as_ref().is_some_and(|(cx, _, cw)| (x + w / 2.0 - cx).abs() < (w + cw) / 2.0) {
                    continue;
                }
                last_right = x + w.max(gap);
                f.text(x, avail + TIME_AXIS_HEIGHT / 2.0, text, t.text, Align::Left, Baseline::Middle, font, None, Some(time_axis));
            }
        }
        for i in (i0..=i1).filter(|_| !tpo) {
            let Some(b) = self.bar(i) else { continue };
            let x = self.time.x(i as f64) as f32;
            if (0.0..plot_w).contains(&x) {
                let prev = i.checked_sub(1).and_then(|j| self.bar(j)).map(|pb| self.shown_time(pb.time));
                let shown = self.shown_time(b.time);
                candidates.push((ticks::boundary_weight(prev, shown), x, shown));
            }
        }
        for (x, label) in ticks::pick_time_labels(&candidates, self.char_width * 6.0 + 22.0) {
            if cross_label.as_ref().is_some_and(|(cx, _, w)| (cx - x).abs() < w / 2.0 + self.char_width * 3.0) {
                continue;
            }
            f.text(x, avail + TIME_AXIS_HEIGHT / 2.0, label, t.text, Align::Center, Baseline::Middle, font, None, Some(time_axis));
        }
        if let Some((x, text, w)) = cross_label {
            let left = (x - w / 2.0).within(0.0, (plot_w - w).max(0.0));
            f.crisp_rect(left, avail + 1.0, w, TIME_AXIS_HEIGHT - 2.0, t.label_background);
            f.text(left + w / 2.0, avail + TIME_AXIS_HEIGHT / 2.0, text, t.countdown_text, Align::Center, Baseline::Middle, font, None, None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_series(&self, f: &mut Frame, scale: &PriceScale, values: &[Option<f64>], i0: usize, i1: usize, color: Color, width: f32, style: LineStyle, pattern: Pattern) {
        if values.is_empty() {
            return;
        }
        let end = i1.min(values.len() - 1);
        let mut pts: Vec<(f32, f32)> = Vec::new();
        for i in i0..=end {
            match values[i] {
                Some(v) => {
                    if self.turned(style, i) {
                        stroke(f, &pts, width, color, pattern);
                        pts.clear();
                    }
                    let (x, y) = (self.time.x(i as f64) as f32, scale.y(v) as f32);
                    if let (LineStyle::Step, Some(&(_, py))) = (style, pts.last()) {
                        pts.push((x, py));
                    }
                    pts.push((x, y));
                }
                None => {
                    stroke(f, &pts, width, color, pattern);
                    pts.clear();
                }
            }
        }
        stroke(f, &pts, width, color, pattern);
    }

    /// Whether an anchored line starts again at bar `i`.
    fn turned(&self, style: LineStyle, i: usize) -> bool {
        let LineStyle::Anchored(anchor) = style else { return false };
        i > 0 && i < self.candles.len() && anchor_period(self.candles[i - 1].open_time, anchor) != anchor_period(self.candles[i].open_time, anchor)
    }

    /// A level the line only names: a short stub and its label beside the newest
    /// bar, kept on the pane when there is no room to its right.
    /// A study's tags, right of the newest bar: a short stub at each price and the
    /// labels in a column beside them, top to bottom. A label is let down a little
    /// past one above it; where it would stand further from its own price than
    /// that, the two are named together, as `qVAL | pwVWAP`.
    fn paint_tags(&self, f: &mut Frame, pane: &PaneView, mut tags: Vec<(f32, String, Color)>) {
        tradoor_core::sort::insertion_sort_by(&mut tags, |a, b| a.0 < b.0);
        let font = self.theme.font_px - 1.0;
        let (stub, line) = (14.0, font + 1.0);
        let mut labels: Vec<(f32, String, Color)> = Vec::new();
        for (y, label, color) in &tags {
            match labels.last_mut() {
                Some(above) if *y < above.0 + line / 2.0 => {
                    above.1 += " | ";
                    above.1 += label;
                }
                _ => labels.push((labels.last().map_or(*y, |above| y.max(above.0 + line)), label.clone(), *color)),
            }
        }
        let wide = |label: &str| label.len() as f32 * self.char_width * font / self.theme.font_px;
        let widest = labels.iter().map(|l| wide(&l.1)).fold(0.0, f32::max);
        // One column past the newest bar, drawn back over the bars where the edge is near.
        let newest = self.time.x(self.total_bars().saturating_sub(1) as f64) as f32;
        let x = (newest + 10.0).min(pane.plot.w - stub - widest - 7.0);
        for (y, _, color) in tags {
            f.dashed_line(x, y, x + stub, y, 1.0, color, 2.0, 2.0);
        }
        let backing = self.theme.background.with_alpha(210);
        for (y, label, color) in labels {
            f.rect(x + stub + 1.0, y - font / 2.0 - 1.0, wide(&label) + 4.0, font + 2.0, backing);
            f.text(x + stub + 3.0, y, label, color, Align::Left, Baseline::Middle, font, None, Some(pane.plot));
        }
    }

    /// The shade between two of a study's lines, breaking where either has no
    /// value and changing colour where they cross.
    #[allow(clippy::too_many_arguments)]
    fn paint_fill(&self, f: &mut Frame, scale: &PriceScale, study: &ComputedStudy, fill: &StudyFill, i0: usize, far: usize) {
        let values = |key: &str| study.lines.iter().find(|l| l.key == key).map(|l| l.values.as_slice());
        let anchored = study.lines.iter().find(|l| l.key == fill.upper).map_or(LineStyle::Solid, |l| l.style);
        let (Some(a), Some(b)) = (values(fill.upper), values(fill.lower)) else { return };
        // Only a forming bar, as on a coin listed this month: no lines yet.
        if a.is_empty() || b.is_empty() {
            return;
        }
        let paint = |key: &str, color: &str| {
            let own = self.styles.of(&study.id, key);
            if own.off { None } else { own.color.or_else(|| (fill.rgba != 0).then(|| Color::rgba(fill.rgba).shade())).or_else(|| Color::parse(color)) }
        };
        let (over, under) = (paint(fill.key, fill.color), paint(fill.crossed_key, fill.crossed_color));
        let (mut top, mut bottom) = (Vec::new(), Vec::new());
        let mut above = true;
        let flush = |f: &mut Frame, top: &mut Vec<(f32, f32)>, bottom: &mut Vec<(f32, f32)>, above: bool| {
            if let Some(color) = if above { over } else { under } {
                f.band(top, bottom, color);
            }
            top.clear();
            bottom.clear();
        };
        for i in i0..=far.min(a.len().min(b.len()) - 1) {
            let (Some(x), Some(y)) = (a[i], b[i]) else {
                flush(f, &mut top, &mut bottom, above);
                continue;
            };
            let px = self.time.x(i as f64) as f32;
            let point = ((px, scale.y(x) as f32), (px, scale.y(y) as f32));
            if self.turned(anchored, i) {
                flush(f, &mut top, &mut bottom, above);
            }
            if (x >= y) != above && !top.is_empty() {
                // The stretch where they cross goes with what follows it.
                let joined = (top.last().copied(), bottom.last().copied());
                flush(f, &mut top, &mut bottom, above);
                top.extend(joined.0);
                bottom.extend(joined.1);
            }
            above = x >= y;
            top.push(point.0);
            bottom.push(point.1);
        }
        flush(f, &mut top, &mut bottom, above);
    }

    /// A study's stretches of time: a tinted box over the bars, with its name.
    fn paint_boxes(&self, f: &mut Frame, pane: &PaneView, study: &ComputedStudy) {
        let half = self.time.bar_spacing as f32 / 2.0;
        for b in &study.boxes {
            let own = self.styles.of(&study.id, b.key);
            let (Some(l0), Some(l1)) = (time_to_logical(&self.times, b.from as f64), time_to_logical(&self.times, b.to as f64)) else { continue };
            let (x0, x1) = (self.time.x(l0) as f32 - half, self.time.x(l1) as f32 + half);
            if own.off || x1 < 0.0 || x0 > pane.plot.w {
                continue;
            }
            let (top, bottom) = (pane.scale.y(b.top) as f32, pane.scale.y(b.bottom) as f32);
            let color = own.color.or_else(|| Color::parse(b.color)).unwrap_or(self.theme.text);
            f.rect(x0, top, x1 - x0, bottom - top, color.faded(0x1c));
            f.stroke_rect(x0, top, x1 - x0, bottom - top, 1.0, color.faded(0x90));
            f.text(x0 + 3.0, top - 2.0, b.label, color, Align::Left, Baseline::Bottom, self.theme.font_px - 1.0, None, Some(pane.plot));
        }
    }

    /// Volume by price over the bars on screen, each bar's spread evenly across
    /// the rows its range covers, and split by whether it closed up or down.
    fn visible_profile(&self, study: &ComputedStudy) -> Option<VisibleProfile> {
        let rows = study.params.first().map_or(24, |r| *r as usize).max(1);
        let (from, to) = self.time.visible();
        let (i0, i1) = (from.ceil().max(0.0) as usize, (to.floor().max(0.0) as usize).min(self.total_bars().checked_sub(1)?));
        let bars = || (i0..=i1).filter_map(|i| self.bar(i));
        let (low, high) = bars().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), b| (l.min(b.low), h.max(b.high)));
        if !(high > low) {
            return None;
        }
        let step = (high - low) / rows as f64;
        let (mut up, mut down) = (vec![0.0; rows], vec![0.0; rows]);
        let row = |price: f64| (((price - low) / step) as usize).min(rows - 1);
        for b in bars() {
            // A bar with its low over its high still lands somewhere.
            let first = row(b.low);
            let last = row(b.high).max(first);
            let share = b.volume / (last - first + 1) as f64;
            let side = if b.close >= b.open { &mut up } else { &mut down };
            for v in &mut side[first..=last] {
                *v += share;
            }
        }
        let total: Vec<f64> = up.iter().zip(&down).map(|(a, b)| a + b).collect();
        let poc = (0..rows).max_by(|a, b| total[*a].total_cmp(&total[*b]))?;
        if total[poc] <= 0.0 {
            return None;
        }
        let area = value_area(&total, poc, study.params.get(1).map_or(0.7, |p| p / 100.0));
        Some(VisibleProfile { low, step, up, down, total, poc, area })
    }

    /// The profile against an edge of the pane, its value area stronger, its
    /// point of control and value area edges across the whole of it.
    fn paint_visible_profile(&self, f: &mut Frame, pane: &PaneView, study: &ComputedStudy) {
        let Some(p) = self.visible_profile(study) else {
            self.profile_levels.set(None);
            return;
        };
        self.profile_levels.set(Some([p.level("poc"), p.level("vah"), p.level("val")]));
        let t = &self.theme;
        let id = study.id.as_str();
        let longest = p.total[p.poc];
        let room = pane.plot.w * study.params.get(2).map_or(30.0, |w| *w as f32) / 100.0;
        let left_side = study.params.get(3).is_some_and(|s| *s == 1.0);
        let (up, down) = (self.styles.of(id, "up"), self.styles.of(id, "down"));
        let (up_ink, down_ink) = (up.color.unwrap_or(t.bull), down.color.unwrap_or(t.bear));
        for r in 0..p.total.len() {
            let (top, bottom) = (pane.scale.y(p.low + p.step * (r + 1) as f64) as f32, pane.scale.y(p.low + p.step * r as f64) as f32);
            let alpha = if r >= p.area.0 && r <= p.area.1 { 0x90 } else { 0x38 };
            let (a, b) = (room * (p.up[r] / longest) as f32, room * (p.down[r] / longest) as f32);
            let height = (bottom - top - 1.0).max(1.0);
            let at = |along: f32, len: f32| if left_side { along } else { pane.plot.w - along - len };
            if !up.off {
                f.rect(at(0.0, a), top, a, height, up_ink.faded(alpha));
            }
            if !down.off {
                f.rect(at(a, b), top, b, height, down_ink.faded(alpha));
            }
        }
        let poc = self.styles.of(id, "poc");
        if !poc.off {
            let y = pane.scale.y(p.level("poc").unwrap_or(0.0)) as f32;
            f.hline(0.0, pane.plot.w, y, poc.width.unwrap_or(1.0), poc.color.unwrap_or(t.tpo_poc));
        }
        let edges = self.styles.of(id, "va");
        if !edges.off {
            for key in ["vah", "val"] {
                let y = pane.scale.y(p.level(key).unwrap_or(0.0)) as f32;
                f.dashed_line(0.0, y, pane.plot.w, y, edges.width.unwrap_or(1.0), edges.color.unwrap_or(t.text), 4.0, 4.0);
            }
        }
    }

    /// A study's own candles — open interest — in its pane, coloured by whether it rose.
    #[allow(clippy::too_many_arguments)]
    fn paint_study_candles(&self, f: &mut Frame, scale: &PriceScale, bars: &[Option<[f64; 4]>], i0: usize, i1: usize, body: f32, owner: &str) {
        let t = &self.theme;
        let wick = (1.0 / f.dpr).max(1.0);
        let (up, down) = (self.styles.color(owner, "up", t.bull), self.styles.color(owner, "down", t.bear));
        // Most studies have no candles: an empty list must not be indexed at all.
        for (i, bar) in bars.iter().enumerate().take(i1.saturating_add(1)).skip(i0) {
            let Some([o, h, l, c]) = *bar else { continue };
            let x = f.snap(self.time.x(i as f64) as f32);
            let color = if c >= o { up } else { down };
            let (yh, yl, yo, yc) = (scale.y(h) as f32, scale.y(l) as f32, scale.y(o) as f32, scale.y(c) as f32);
            let (top, bottom) = (yo.min(yc), yo.max(yc));
            f.vline(x, yh, yl, wick, color);
            if body >= 2.0 {
                f.crisp_rect(x - body / 2.0, top, body, (bottom - top).max(1.0 / f.dpr), color);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn paint_histogram(&self, f: &mut Frame, scale: &PriceScale, values: &[Option<f64>], i0: usize, i1: usize, body: f32, owner: &str, color: Option<Color>, bars: Option<&[u32]>) {
        let zero = scale.y(0.0) as f32;
        let w = (body * 0.7).max(1.0 / f.dpr);
        let (above, below) = match color {
            Some(c) => (c.with_alpha(0x99), c.with_alpha(0x99)),
            None => (self.styles.color(owner, "histogram", self.theme.bull.with_alpha(0x80)), self.styles.color(owner, "histogramDown", self.theme.bear.with_alpha(0x80))),
        };
        for (i, v) in values.iter().enumerate().take(i1.saturating_add(1)).skip(i0) {
            let Some(v) = *v else { continue };
            let y = scale.y(v) as f32;
            let x = f.snap(self.time.x(i as f64) as f32);
            let ink = match bars.and_then(|b| b.get(i).copied()) {
                Some(0) => continue,
                Some(c) => Color::rgba(c),
                None if v >= 0.0 => above,
                None => below,
            };
            f.crisp_rect(x - w / 2.0, y.min(zero), w, (y - zero).abs().max(1.0 / f.dpr), ink);
        }
    }

    /// Where a drawing's point stands on the bars. On a timeframe longer than
    /// the one it was drawn at, on the middle of the bar holding its time, as
    /// TradingView places it: an entry at 08:00 on the 4H stands on that day's
    /// bar on the 1D, not a third of a bar past it. At its own timeframe and
    /// shorter ones, where it was put; and before the first bar and past the
    /// newest, between bar positions, so a box drawn out into the future keeps
    /// its length.
    fn drawing_logical(&self, shape: &DrawingShape, time: f64) -> Option<f64> {
        let logical = time_to_logical(&self.times, time)?;
        let longer = shape.timeframe.is_some_and(|tf| bar_span(tf) < bar_span(self.timeframe));
        let newest = self.bar(self.total_bars().checked_sub(1)?)?.time;
        let held = self.times.first().is_some_and(|first| time >= *first) && time < self.timeframe.end(newest) as f64;
        Some(if longer && held { logical.floor() } else { logical })
    }

    /// The pane a drawing stands in, when it is on the chart: its study's, gone with it.
    fn pane_of(&self, shape: &DrawingShape) -> Option<usize> {
        match shape.style.pane {
            None => Some(0),
            Some(group) => self.panes.iter().position(|p| p.group == group),
        }
    }

    /// The group a drawing placed in pane `index` is kept by: none for the price pane.
    fn group_of(&self, index: usize) -> Option<Group> {
        self.panes.get(index).map(|p| p.group).filter(|_| index > 0)
    }

    fn created(&mut self, kind: DrawingKind, points: Vec<Anchor>) {
        let pane = self.group_of(self.draw_pane);
        self.events.push(Event::DrawingCreated { kind, points, pane });
    }

    /// The drawing being placed, in its pane, as the tool's style has it.
    fn draft_of(&self, kind: DrawingKind, points: Vec<Anchor>) -> DrawingShape {
        DrawingShape { style: DrawingStyle { pane: self.group_of(self.draw_pane), ..self.tool_style.clone() }, ..DrawingShape::new(None, kind, points) }
    }

    fn place<'a>(&self, shape: &'a DrawingShape, draft: bool) -> Option<Placed<'a>> {
        let index = self.pane_of(shape)?;
        let pane = self.panes.get(index)?;
        let mut points = Vec::with_capacity(shape.points.len());
        for a in &shape.points {
            let x = self.time.x(self.drawing_logical(shape, a.time)?) as f32;
            let y = pane.scale.y(a.price) as f32;
            if !(x.is_finite() && y.is_finite()) {
                return None;
            }
            points.push((x, y));
        }
        let bars = match (shape.points.first(), shape.points.get(1)) {
            (Some(a), Some(b)) => (self.drawing_logical(shape, b.time)? - self.drawing_logical(shape, a.time)?).round() as i64,
            _ => 0,
        };
        let last = self.candles.last().map(|c| c.close);
        // A lower pane's values read to two places, as its axis has them.
        let precision = if index == 0 { self.precision() } else { 2 };
        Some(Placed { shape, points, selected: !draft && shape.id.is_some() && shape.id == self.selected, draft, hovered: false, bars, scale: pane.scale, precision, last, plot: pane.plot })
    }

    // ------------------------------------------------------------------ legend

    pub fn legend(&self) -> Legend {
        let n = self.total_bars();
        let Some(pane) = self.panes.first() else { return Legend::default() };
        if n == 0 {
            return Legend::default();
        }
        let hovered = self.crosshair.filter(|(x, y)| *x >= 0.0 && *x < pane.plot.w && *y >= 0.0 && *y < self.time_axis_top()).and_then(|(x, _)| self.time.nearest_bar(f64::from(x)));
        let index = hovered.unwrap_or(n - 1);
        let Some(b) = self.bar(index) else { return Legend::default() };
        let p = self.precision();
        let fp = |v: f64| format_price(v, Some(p));
        let diff = b.close - b.open;
        let pct = if b.open == 0.0 { 0.0 } else { diff / b.open * 100.0 };
        let sign = if diff > 0.0 { "+" } else { "" };
        // Indicators stop at the last closed bar; hovering the forming one reads that.
        let at = index.min(self.candles.len().saturating_sub(1));
        let values = self
            .visible_studies()
            .flat_map(|(_, s)| {
                let levels = if s.kind == StudyKind::VisibleProfile { self.profile_levels.get() } else { None };
                s.lines.iter().map(move |l| {
                    // A series that runs into the forming bar is read there.
                    let i = if l.values.len() > self.candles.len() { index } else { at };
                    let value = levels.map_or_else(|| l.values.get(i).copied().flatten(), |p| ["poc", "vah", "val"].iter().position(|k| *k == l.key).and_then(|at| p[at]));
                    (format!("{}:{}", s.id, l.key), value.map_or("—".to_string(), |v| self.format_value(s.group, v)))
                })
            })
            .collect();
        Legend {
            index: Some(index),
            time: b.time,
            ohlc: format!("O{}  H{}  L{}  C{}", fp(b.open), fp(b.high), fp(b.low), fp(b.close)),
            change: format!("{sign}{} ({sign}{}%)", fp(diff), tradoor_core::format::fixed(pct, 2)),
            up: diff >= 0.0,
            volume: format!("Vol {}", format_turnover(b.volume)),
            values,
            cursor: self
                .crosshair
                .filter(|(x, y)| matches!(self.drag, Drag::None) && *x >= 0.0 && *x < pane.plot.w && *y >= pane.plot.y && *y <= pane.plot.bottom())
                .map(|(_, y)| (y, pane.scale.price(f64::from(y)))),
            axis_x: pane.plot.w,
            bar_x: hovered.map(|i| self.time.x(i as f64) as f32),
            alert: self.crosshair.filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).and_then(|(x, y)| self.alert_handle_at(x, y)),
            flow: self.crosshair.filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).and_then(|(x, y)| self.flow_at(x, y)),
            footprint: self.crosshair.filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).and_then(|(x, y)| self.footprint_at(x, y)),
            tpo: self
                .tpo_hovered()
                .filter(|_| self.tool.is_none())
                .and_then(|(k, bar)| tpo::hover(self.tpo_profiles.as_ref()?, &self.tpo_settings, k, self.bar(bar).map(|b| (bar, b.time)))),
            mark: mark_at(pane.plot, self.table_top().unwrap_or(pane.plot.bottom())).filter(|_| self.page_mark).map(|(x, y)| (x, y, self.theme.ribbon_fast, self.theme.bear)),
            panes: self.panes.iter().skip(1).map(|p| (p.group.as_str(), p.plot.y)).collect(),
        }
    }

    /// A study's figure as its pane labels it: the instrument's decimals for
    /// price, two for the rest, and `5.327M` for open interest.
    fn format_value(&self, group: Group, v: f64) -> String {
        match group {
            Group::Price => format_price(v, Some(self.precision())),
            Group::Oi | Group::Delta => format_compact(v),
            // A formula's own pane holds whatever it works out: decimals by its size, read with no logarithm.
            Group::Formula(_) => {
                let a = v.abs();
                format_price(v, Some(if a >= 100.0 { 2 } else if a >= 1.0 { 3 } else if a >= 0.01 { 4 } else { 6 }))
            }
            _ => format_price(v, Some(2)),
        }
    }

    pub fn studies_meta(&self) -> Vec<StudyMeta> {
        let mut visible = 0usize;
        let mut metas: Vec<StudyMeta> = self
            .studies
            .iter()
            .zip(&self.study_specs)
            .map(|(s, spec)| {
                let hidden = self.hidden.contains(&s.id);
                let index = visible;
                if !hidden {
                    visible += 1;
                }
                // A hidden study holds no lines; its settings still list them, from no bars at all.
                let unread = hidden.then(|| compute_study_with(spec, &[], &StudyInputs::default()));
                let lines = unread.as_ref().map_or(&s.lines, |u| &u.lines);
                StudyMeta {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    group: self.pane_group(s).as_str(),
                    hidden,
                    lines: lines
                        .iter()
                        .enumerate()
                        .map(|(li, l)| (format!("{}:{}", s.id, l.key), l.label.clone(), self.line_color(&s.id, l, index, li), if l.style == LineStyle::None { 0.0 } else { self.styles.of(&s.id, l.key).width.unwrap_or(l.width) }))
                        .collect(),
                    stats: s.stats.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
                }
            })
            .collect();
        // The formulas after the studies, as the chart draws them.
        for s in &self.formulas {
            let hidden = self.hidden.contains(&s.id);
            let index = visible;
            if !hidden {
                visible += 1;
            }
            metas.push(StudyMeta {
                id: s.id.clone(),
                label: s.label.clone(),
                group: s.group.as_str(),
                hidden,
                lines: s.lines.iter().enumerate().map(|(li, l)| (format!("{}:{}", s.id, l.key), l.label.clone(), self.line_color(&s.id, l, index, li), self.styles.of(&s.id, l.key).width.unwrap_or(l.width))).collect(),
                stats: s.stats.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            });
        }
        metas
    }

    // ------------------------------------------------------------------ interaction

    fn pane_at(&self, y: f32) -> Option<usize> {
        self.panes.iter().position(|p| y >= p.plot.y && y < p.plot.bottom())
    }

    fn zone(&self, x: f32, y: f32) -> Zone {
        if self.panes.is_empty() || x < 0.0 || y < 0.0 || x > self.width || y > self.height {
            return Zone::Outside;
        }
        if let Some(i) = (1..self.panes.len()).find(|&i| (y - self.panes[i].plot.y).abs() <= SEPARATOR_GRAB) {
            return Zone::Separator(i - 1);
        }
        let plot_w = self.plot_width();
        if y >= self.time_axis_top() {
            return if x < plot_w { Zone::TimeAxis } else { Zone::Outside };
        }
        match self.pane_at(y) {
            Some(p) if x < plot_w => Zone::Plot(p),
            Some(p) => Zone::PriceAxis(p),
            None => Zone::Outside,
        }
    }

    fn anchor_at(&self, x: f32, y: f32) -> Option<Anchor> {
        self.anchor_in(self.draw_pane, x, y)
    }

    fn anchor_in(&self, pane: usize, x: f32, y: f32) -> Option<Anchor> {
        let pane = self.panes.get(pane)?;
        let time = logical_to_time(&self.times, self.time.logical(f64::from(x)))?;
        let price = pane.scale.price(f64::from(y));
        (time.is_finite() && price.is_finite()).then_some(Anchor { time, price })
    }

    /// An anchor where the pointer is, pulled by the magnet when it is on.
    fn point_at(&self, x: f32, y: f32) -> Option<Anchor> {
        self.point_in(self.draw_pane, x, y)
    }

    fn point_in(&self, pane: usize, x: f32, y: f32) -> Option<Anchor> {
        let raw = self.anchor_in(pane, x, y)?;
        let magnet = self.magnet_now();
        // A lower pane has no candles to hold on to.
        if magnet == Magnet::Off || pane > 0 {
            return Some(raw);
        }
        let pane = self.panes.first()?;
        let scale = 10f64.powi(self.precision() as i32);
        let on_tick = Anchor { price: (raw.price * scale).round() / scale, ..raw };
        let n = self.total_bars();
        let slot = self.time.logical(f64::from(x)).round();
        // Past the last bar there is no candle to hold on to: a whole tick is all.
        if n == 0 || slot < 0.0 || slot > (n - 1) as f64 {
            return Some(on_tick);
        }
        let Some(bar) = self.bar(slot as usize) else { return Some(on_tick) };
        let distance = |p: f64| (pane.scale.y(p) as f32 - y).abs();
        let nearest = [bar.open, bar.high, bar.low, bar.close].into_iter().min_by(|a, b| distance(*a).total_cmp(&distance(*b))).unwrap_or(raw.price);
        let price = if magnet == Magnet::Strong || distance(nearest) <= MAGNET_PX { nearest } else { on_tick.price };
        Some(Anchor { time: bar.time as f64, price })
    }

    /// An anchor handle of the selected drawing under the pointer.
    fn handle_at(&self, x: f32, y: f32) -> Option<(i64, usize)> {
        let id = self.selected?;
        let shape = self.drawings.iter().find(|d| d.id == Some(id)).filter(|d| d.kind != DrawingKind::Brush)?;
        let placed = self.place(shape, false)?;
        let hit = drawings::handles(&placed).iter().position(|p| (p.0 - x).hypot(p.1 - y) <= HANDLE_HIT)?;
        Some((id, hit))
    }

    fn select(&mut self, id: Option<i64>) {
        if id.is_some() {
            self.selected_alert = None;
        }
        if self.selected != id {
            self.selected = id;
            self.events.push(Event::SelectionChanged(id));
            self.touch();
        }
    }

    fn move_anchor(&mut self, id: i64, index: usize, x: f32, y: f32) {
        self.draw_pane = self.drawings.iter().find(|d| d.id == Some(id)).and_then(|d| self.pane_of(d)).unwrap_or(0);
        let Some(mut anchor) = self.point_at(x, y) else { return };
        let shift = self.shift;
        if let Some(shape) = self.drawings.iter_mut().find(|d| d.id == Some(id)) {
            if shape.kind.is_position() && shape.points.len() == 3 {
                // Left-edge handles set the three prices, the right one the width.
                let p = &mut shape.points;
                let mut left = p.iter().map(|a| a.time).fold(f64::INFINITY, f64::min);
                let mut right = p.iter().map(|a| a.time).fold(f64::NEG_INFINITY, f64::max);
                match index {
                    0 => {
                        p[0].price = anchor.price;
                        if anchor.time < right {
                            left = anchor.time;
                        }
                    }
                    1 => p[1].price = anchor.price,
                    2 => p[2].price = anchor.price,
                    _ if anchor.time > left => right = anchor.time,
                    _ => {}
                }
                p[0].time = left;
                p[1].time = right;
                p[2].time = right;
            } else {
                // Shift keeps a line level with its other end.
                if shift && index < 2 && matches!(shape.kind, DrawingKind::Trendline | DrawingKind::Channel | DrawingKind::Measure | DrawingKind::Fib) {
                    if let Some(other) = shape.points.get(1 - index) {
                        anchor.price = other.price;
                    }
                }
                if let Some(point) = shape.points.get_mut(index) {
                    *point = anchor;
                }
            }
        }
        self.touch();
    }

    /// With Shift held, the next anchor of a line sits level with the one before.
    fn constrain(&self, kind: DrawingKind, before: &[Anchor], mut a: Anchor) -> Anchor {
        let lines = matches!(kind, DrawingKind::Trendline | DrawingKind::Path | DrawingKind::Fib | DrawingKind::Measure) || (kind == DrawingKind::Channel && before.len() == 1);
        if self.shift && lines {
            if let Some(prev) = before.last() {
                a.price = prev.price;
            }
        }
        a
    }

    /// Shift a whole drawing by a pointer move: along the bars, and by the same
    /// distance on the price axis — a ratio under a log scale.
    fn move_shape(&mut self, id: i64, from: (f32, f32), to: (f32, f32)) {
        let Some(index) = self.drawings.iter().position(|d| d.id == Some(id)) else { return };
        let Some(pane) = self.pane_of(&self.drawings[index]).and_then(|p| self.panes.get(p)).copied() else { return };
        let bars = f64::from(to.0 - from.0) / self.time.bar_spacing;
        let scale = pane.scale;
        let rise = scale.transform(scale.price(f64::from(to.1))) - scale.transform(scale.price(f64::from(from.1)));
        let moved: Vec<Anchor> = self.drawings[index]
            .points
            .iter()
            .map(|p| Anchor {
                time: time_to_logical(&self.times, p.time).and_then(|l| logical_to_time(&self.times, l + bars)).unwrap_or(p.time),
                price: scale.untransform(scale.transform(p.price) + rise),
            })
            .collect();
        self.drawings[index].points = moved;
        self.touch();
    }

    fn drawing_at(&self, x: f32, y: f32) -> Option<i64> {
        let (mut best, mut best_distance) = (None, HIT_TOLERANCE);
        for d in &self.drawings {
            let Some(placed) = self.place(d, false) else { continue };
            // Only where it is drawn: a line in a lower pane is not caught over the candles.
            if placed.shape.style.locked || !placed.plot.contains(x, y) {
                continue;
            }
            let distance = drawings::distance(&placed, (x, y), placed.plot);
            // `<=`: later drawings are painted on top, so the visible one wins a tie.
            if distance <= best_distance {
                best_distance = distance;
                best = d.id;
            }
        }
        best
    }

    fn cursor_for(&self, x: f32, y: f32) -> Cursor {
        if self.tool.is_some() {
            return Cursor::Crosshair;
        }
        match self.drag {
            Drag::Press { panning: true, .. } => Cursor::Grabbing,
            Drag::Shape { .. } | Drag::Anchor { .. } => Cursor::Move,
            Drag::Alert { .. } => Cursor::NsResize,
            Drag::Held => Cursor::Crosshair,
            Drag::PriceAxis { .. } => Cursor::NsResize,
            Drag::TimeAxis { .. } => Cursor::ColResize,
            Drag::Separator { .. } | Drag::Table { .. } => Cursor::RowResize,
            _ => match self.zone(x, y) {
                Zone::Separator(_) => Cursor::RowResize,
                Zone::Plot(0) if self.on_table_edge(y) => Cursor::RowResize,
                Zone::PriceAxis(0) if self.alert_badge_at(x, y).is_some_and(|id| self.selected_alert != Some(id)) && !self.touch_input => Cursor::Pointer,
                Zone::PriceAxis(_) => Cursor::NsResize,
                Zone::TimeAxis => Cursor::ColResize,
                Zone::Plot(_) if self.handle_at(x, y).is_some() => Cursor::Pointer,
                Zone::Plot(0) if let Some((id, _)) = self.alert_at(x, y) => {
                    if self.selected_alert == Some(id) {
                        Cursor::NsResize
                    } else {
                        Cursor::Pointer
                    }
                }
                Zone::Plot(_) => match self.drawing_at(x, y) {
                    Some(id) if self.selected == Some(id) => Cursor::Move,
                    Some(_) => Cursor::Pointer,
                    None => Cursor::Crosshair,
                },
                Zone::Outside => Cursor::Default,
            },
        }
    }

    fn draft_with(&mut self, kind: DrawingKind, extra: Option<Anchor>) {
        let mut points = self.placed.clone();
        points.extend(extra.map(|a| self.constrain(kind, &self.placed, a)));
        self.draft = Some(self.draft_of(kind, points));
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        // A pinned crosshair goes by the finger's motion, once it is more than a tap's.
        if let Drag::Nudge { start, last, moved } = self.drag {
            if !moved && (x - start.0).hypot(y - start.1) <= CLICK_SLOP {
                return;
            }
            let (cx, cy) = self.crosshair.unwrap_or(start);
            let to = ((cx + x - last.0).max(0.0).min(self.plot_width() - 1.0), (cy + y - last.1).max(0.0).min(self.time_axis_top() - 1.0));
            self.drag = Drag::None;
            self.pointer_move(to.0, to.1);
            self.drag = Drag::Nudge { start, last: (x, y), moved: true };
            return;
        }
        // A finger sliding the chart moves the chart: its crosshair comes with a tap, or under a finger held still.
        if !(self.touch_input && !self.pinned && matches!(self.drag, Drag::Press { .. })) {
            self.crosshair = Some((x, y));
        }
        match self.drag {
            Drag::Press { start, last_x, last_y, pane, panning, price_pan } => {
                if self.tool.is_none() && (panning || (x - start.0).hypot(y - start.1) > CLICK_SLOP) {
                    self.time.pan(f64::from(x - last_x));
                    // Price follows too: at once on a pane already scaled by hand,
                    // and on an autoscaled one when the drag is clearly vertical.
                    let follow = price_pan || !self.pane_auto(pane) || (y - start.1).abs() > VERTICAL_PAN_SLOP;
                    if follow {
                        // The first step covers the whole drag so far, keeping the
                        // grabbed price under the pointer.
                        self.pan_price(pane, if price_pan { y - last_y } else { y - start.1 });
                    }
                    self.drag = Drag::Press { start, last_x: x, last_y: y, pane, panning: true, price_pan: follow };
                }
            }
            Drag::PriceAxis { pane, last_y } => {
                self.zoom_price(pane, (f64::from(y - last_y) * 0.004).exp());
                self.drag = Drag::PriceAxis { pane, last_y: y };
            }
            Drag::TimeAxis { last_x } => {
                // Pulling the dates left stretches the bars, pushing them right
                // squeezes them — the way TradingView's time scale moves.
                let at = self.zoom_anchor(self.plot_width());
                self.time.zoom_at(at, (f64::from(last_x - x) * 0.01).exp());
                self.drag = Drag::TimeAxis { last_x: x };
            }
            Drag::Separator { upper, last_y } => {
                self.resize_panes(upper, y - last_y);
                self.drag = Drag::Separator { upper, last_y: y };
            }
            // The crosshair, set above, is all a held finger moves.
            Drag::Table { grab } => {
                if let Some(pane) = self.panes.first() {
                    self.footprint_table_row = footprint::table_row_for(pane.plot.bottom() - (y - grab), pane.plot.h);
                    self.touch();
                }
            }
            Drag::Held | Drag::Nudge { .. } => {}
            Drag::Alert { id, .. } => {
                if let Some(pane) = self.panes.first() {
                    let step = 10f64.powi(-(self.precision() as i32));
                    let price = (pane.scale.price(f64::from(y)) / step).round() * step;
                    if let Some(a) = self.alerts.iter_mut().find(|a| a.id == id) {
                        a.price = price;
                    }
                }
                self.drag = Drag::Alert { id, moved: true };
            }
            Drag::Anchor { id, index, .. } => {
                self.move_anchor(id, index, x, y);
                self.drag = Drag::Anchor { id, index, moved: true };
            }
            Drag::Shape { id, last, .. } => {
                self.move_shape(id, last, (x, y));
                self.drag = Drag::Shape { id, last: (x, y), moved: true };
            }
            Drag::Draw { kind, start, start_px, moved } => {
                let moved = moved || (x - start_px.0).hypot(y - start_px.1) > CLICK_SLOP;
                if moved {
                    if let Some(end) = self.point_at(x, y) {
                        let mut points = self.placed.clone();
                        points.push(start);
                        let end = self.constrain(kind, &points, end);
                        points.push(end);
                        self.draft = Some(self.draft_of(kind, points));
                    }
                }
                self.drag = Drag::Draw { kind, start, start_px, moved };
            }
            Drag::Brush { last } => {
                if (x - last.0).hypot(y - last.1) >= BRUSH_SAMPLE && self.placed.len() < MAX_POINTS {
                    if let Some(a) = self.anchor_at(x, y) {
                        self.placed.push(a);
                        self.draft_with(DrawingKind::Brush, None);
                        self.drag = Drag::Brush { last: (x, y) };
                    }
                }
            }
            Drag::None => {}
        }
        if let Some(tool) = self.tool.filter(|t| *t != DrawingKind::Brush && !self.placed.is_empty() && !matches!(self.drag, Drag::Draw { .. })) {
            if let Some(a) = self.point_at(x, y) {
                self.draft_with(tool, Some(a));
            }
        }
        self.cursor = self.cursor_for(x, y);
        self.touch();
    }

    pub fn pointer_down(&mut self, x: f32, y: f32, button: u16) {
        if button != 0 {
            return;
        }
        self.span = None;
        if self.pinned && self.tool != Some(DrawingKind::Brush) && matches!(self.zone(x, y), Zone::Plot(_)) {
            self.drag = Drag::Nudge { start: (x, y), last: (x, y), moved: false };
            return;
        }
        let free = self.tool.is_none();
        // A finger passing over an alert on its way to scale or pan is not
        // taking hold of it: on a touch screen the alert waits for a hold.
        let grab_alerts = !self.touch_input;
        self.drag = match self.zone(x, y) {
            Zone::Separator(i) if free => Drag::Separator { upper: i, last_y: y },
            Zone::Plot(0) if free && self.on_table_edge(y) => Drag::Table { grab: y - self.table_top().unwrap_or(y) },
            // An alert's badge on the axis picks the alert, and moves it once
            // picked; the bare axis scales.
            Zone::PriceAxis(0) if free => match self.alert_badge_at(x, y).filter(|_| grab_alerts) {
                Some(id) if self.selected_alert == Some(id) => Drag::Alert { id, moved: false },
                Some(id) => {
                    self.pick_alert(Some(id));
                    Drag::None
                }
                None => Drag::PriceAxis { pane: 0, last_y: y },
            },
            Zone::PriceAxis(p) if free => Drag::PriceAxis { pane: p, last_y: y },
            Zone::TimeAxis if free => Drag::TimeAxis { last_x: x },
            // Drawn in whichever pane it starts in: the candles', or a study's on its values.
            Zone::Plot(p) if self.tool == Some(DrawingKind::Brush) => {
                self.draw_pane = p;
                match self.anchor_at(x, y) {
                    Some(a) => {
                        self.placed = vec![a];
                        self.draft_with(DrawingKind::Brush, None);
                        Drag::Brush { last: (x, y) }
                    }
                    None => Drag::None,
                }
            }
            // A handle of the selected drawing reshapes it and anywhere else on
            // it moves it. Any other drawing is only picked by a click: a drag
            // that starts on it pans, as on bare chart, since a position's box
            // or a fib's levels cover so much of the screen that the chart
            // could hardly be moved past them without moving them. An alert's
            // line likewise: picked by a click, and only then dragged — a line
            // across the chart is easily caught by a press meant for the chart.
            Zone::Plot(p) if free => {
                if let Some((id, index)) = self.handle_at(x, y) {
                    Drag::Anchor { id, index, moved: false }
                } else if let Some((id, _)) = self.alert_at(x, y).filter(|(id, _)| p == 0 && grab_alerts && self.selected_alert == Some(*id)) {
                    Drag::Alert { id, moved: false }
                } else if let Some(id) = self.drawing_at(x, y).filter(|id| self.selected == Some(*id)) {
                    Drag::Shape { id, last: (x, y), moved: false }
                } else {
                    Drag::Press { start: (x, y), last_x: x, last_y: y, pane: p, panning: false, price_pan: false }
                }
            }
            // With a tool armed, a drag places its anchors in one gesture, in the pane it starts in.
            Zone::Plot(p) if !matches!(self.tool, Some(DrawingKind::Text | DrawingKind::Path | DrawingKind::Long | DrawingKind::Short)) => {
                if self.placed.is_empty() {
                    self.draw_pane = p;
                }
                match (self.tool, self.point_at(x, y)) {
                    (Some(kind), Some(start)) => Drag::Draw { kind, start, start_px: (x, y), moved: false },
                    _ => Drag::None,
                }
            }
            Zone::Plot(p) => Drag::Press { start: (x, y), last_x: x, last_y: y, pane: p, panning: false, price_pan: false },
            _ => Drag::None,
        };
        self.cursor = self.cursor_for(x, y);
        self.touch();
    }

    /// A finger held still where it went down — the page times it, as only the
    /// page knows how long a press has lasted. On a phone a press is how the
    /// chart is panned, so the crosshair waits for a hold, as it does in
    /// TradingView's app; held on an alert's line or badge, the finger takes
    /// the alert instead, which a passing one no longer does. With a tool
    /// armed, the finger then aims and lifting it places the anchor.
    pub fn pointer_hold(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::Press { panning: false, pane, .. } if self.tool.is_none() => {
                self.drag = match self.alert_within(x, y, ALERT_HOLD_PX).filter(|_| pane == 0) {
                    Some((id, _)) => Drag::Alert { id, moved: false },
                    None => Drag::Held,
                };
            }
            Drag::PriceAxis { pane: 0, .. } if self.tool.is_none() => match self.alert_badge_at(x, y) {
                Some(id) => self.drag = Drag::Alert { id, moved: false },
                None => return,
            },
            Drag::Draw { moved: false, .. } => self.drag = Drag::Held,
            _ => return,
        }
        self.crosshair = Some((x, y));
        self.cursor = self.cursor_for(x, y);
        self.touch();
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::Press { start, panning: false, .. } if (x - start.0).hypot(y - start.1) <= CLICK_SLOP => {
                if !self.pin(x, y) {
                    self.click(x, y);
                }
            }
            Drag::Held if self.tool.is_some() => self.click(x, y),
            // A tap with the crosshair up: a tool's next anchor where it stands, or the crosshair put away.
            Drag::Nudge { moved: false, .. } => match (self.tool, self.crosshair) {
                (Some(_), Some((cx, cy))) => self.click(cx, cy),
                _ => {
                    self.pinned = false;
                    self.crosshair = None;
                }
            },
            Drag::Brush { .. } => {
                let points = std::mem::take(&mut self.placed);
                self.draft = None;
                // A press with no movement is a dot, not a stroke.
                if points.len() >= DrawingKind::Brush.arity() {
                    self.created(DrawingKind::Brush, points);
                    self.finish_tool();
                }
            }
            Drag::Alert { id, moved: true } => {
                if let Some(a) = self.alerts.iter().find(|a| a.id == id) {
                    self.events.push(Event::AlertMoved { id, price: a.price });
                }
            }
            Drag::Anchor { id, moved: true, .. } | Drag::Shape { id, moved: true, .. } => {
                if let Some(shape) = self.drawings.iter().find(|d| d.id == Some(id)) {
                    self.events.push(Event::DrawingChanged { id, points: shape.points.clone() });
                }
            }
            Drag::Draw { kind, start, start_px, moved } => {
                if !moved || (x - start_px.0).hypot(y - start_px.1) <= CLICK_SLOP {
                    if !self.pin(x, y) {
                        self.click(x, y);
                    }
                } else if let Some(end) = self.point_at(x, y) {
                    let mut points = std::mem::take(&mut self.placed);
                    if kind.arity() == 1 {
                        points.push(end);
                    } else {
                        points.push(start);
                        let end = self.constrain(kind, &points, end);
                        points.push(end);
                    }
                    if points.len() >= kind.arity() {
                        self.created(kind, points);
                        self.finish_tool();
                    } else {
                        self.placed = points;
                        self.draft_with(kind, Some(end));
                    }
                }
            }
            Drag::Separator { .. } => self.events.push(Event::PaneWeightsChanged(self.pane_weights())),
            Drag::Table { .. } => self.events.push(Event::FootprintTableChanged(self.footprint_table_row)),
            _ => {}
        }
        self.cursor = self.cursor_for(x, y);
        self.touch();
    }

    pub fn pointer_leave(&mut self) {
        // A finger lifting leaves too: a pinned crosshair stays for it.
        if !self.pinned {
            self.crosshair = None;
        }
        if !matches!(self.drag, Drag::Brush { .. }) {
            self.drag = Drag::None;
        }
        self.touch();
    }

    // ------------------------------------------------------------ in step with others

    /// The open time of the slot under this chart's own crosshair, for the
    /// other charts of a layout that keeps crosshairs in step.
    pub fn crosshair_time(&self) -> Option<i64> {
        let (x, _) = self.crosshair?;
        self.slot_time(self.slot_at(x)?)
    }

    /// Another chart's crosshair time, or none once it has left that chart.
    pub fn set_linked_crosshair(&mut self, time: Option<i64>) {
        if self.linked_cross != time {
            self.linked_cross = time;
            self.touch();
        }
    }

    /// The slot holding the linked crosshair's time — the bar it falls in at
    /// this chart's timeframe — when it is on screen.
    fn linked_slot(&self) -> Option<i64> {
        let slot = time_to_logical(&self.times, self.linked_cross? as f64)?.floor() as i64;
        let x = self.time.x(slot as f64);
        (x >= 0.0 && x < f64::from(self.plot_width())).then_some(slot)
    }

    /// The time at the plot's right edge, for a layout that keeps the charts'
    /// time in step.
    pub fn edge_time(&self) -> Option<i64> {
        Some(logical_to_time(&self.times, self.time.visible().1)?.round() as i64)
    }

    /// Scrolls so the plot's right edge is at `time`, as another chart of the
    /// layout is, each chart keeping its own zoom.
    pub fn set_edge_time(&mut self, time: i64) {
        let Some(right) = time_to_logical(&self.times, time as f64) else { return };
        // The plot's right edge is logical `bars - 1 + right_offset + 0.5`.
        self.time.right_offset = right - 0.5 - (self.time.bars as f64 - 1.0);
        self.time.clamp();
        self.view_ready = true;
        self.touch();
    }

    /// A finger's tap where nothing takes it: the crosshair shown there and kept,
    /// as on TradingView's app. A finger then moves it by its motion, so it never
    /// covers the point, a tap with a tool places the next anchor where the
    /// crosshair stands, and a tap without one puts it away. A tap on a drawing
    /// or an alert picks it instead, and one with either picked puts it down —
    /// pinning the crosshair then, a picked position could never be let go of
    /// on a phone. The brush draws under the finger.
    fn pin(&mut self, x: f32, y: f32) -> bool {
        let takes = match self.tool {
            Some(tool) => tool != DrawingKind::Brush,
            None => self.selected.is_none() && self.selected_alert.is_none() && self.drawing_at(x, y).is_none() && self.alert_at(x, y).is_none(),
        };
        if !self.touch_input || self.pinned || !takes || !matches!(self.zone(x, y), Zone::Plot(_)) {
            return false;
        }
        self.pinned = true;
        self.crosshair = Some((x, y));
        true
    }

    fn finish_tool(&mut self) {
        self.tool = None;
        self.placed.clear();
        self.draft = None;
        self.events.push(Event::ToolFinished);
    }

    fn click(&mut self, x: f32, y: f32) {
        let zone = self.zone(x, y);
        let on_price = zone == Zone::Plot(0);
        match self.tool {
            None => match self.alert_at(x, y).filter(|_| on_price) {
                Some((id, _)) => self.pick_alert(Some(id)),
                None => {
                    self.pick_alert(None);
                    self.select(if matches!(zone, Zone::Plot(_)) { self.drawing_at(x, y) } else { None });
                }
            },
            Some(DrawingKind::Brush) => {}
            Some(tool) => {
                let Zone::Plot(p) = zone else { return };
                // A position is priced: on the candles only.
                if tool.is_position() && p > 0 {
                    return;
                }
                if self.placed.is_empty() {
                    self.draw_pane = p;
                }
                let Some(anchor) = self.point_at(x, y) else { return };
                if tool.is_position() {
                    self.place_position(tool, x, y);
                    return;
                }
                let anchor = self.constrain(tool, &self.placed, anchor);
                self.placed.push(anchor);
                if tool.open_ended() || self.placed.len() < tool.arity() {
                    // The draft repeats the newest anchor so it has something to
                    // rubber-band from before the pointer moves.
                    self.draft_with(tool, Some(anchor));
                    return;
                }
                let points = std::mem::take(&mut self.placed);
                self.created(tool, points);
                self.finish_tool();
            }
        }
    }

    /// A position lands whole on one click, as on TradingView: the entry where
    /// clicked, stop and target an equal distance either side, some bars wide.
    fn place_position(&mut self, tool: DrawingKind, x: f32, y: f32) {
        let reach = 60.0;
        let (up, down) = (y - reach, y + reach);
        let (stop_y, target_y) = if tool == DrawingKind::Long { (down, up) } else { (up, down) };
        let right = x + ((self.time.bar_spacing * 20.0) as f32).max(120.0);
        let (Some(entry), Some(stop), Some(target), Some(edge)) = (self.point_at(x, y), self.anchor_at(x, stop_y), self.anchor_at(x, target_y), self.anchor_at(right, y)) else { return };
        // On the instrument's ticks, as a trader would type them.
        let scale = 10f64.powi(self.precision() as i32);
        let tick = |p: f64| (p * scale).round() / scale;
        let points = vec![Anchor { price: tick(entry.price), ..entry }, Anchor { time: edge.time, price: tick(stop.price) }, Anchor { time: edge.time, price: tick(target.price) }];
        self.placed.clear();
        self.created(tool, points);
        self.finish_tool();
    }

    fn finish_path(&mut self) -> bool {
        // A double-click reports both clicks first, so the last anchor arrives twice.
        let mut points = std::mem::take(&mut self.placed);
        points.dedup_by(|a, b| a.time == b.time && a.price == b.price);
        if points.len() < DrawingKind::Path.arity() {
            self.placed = points;
            return false;
        }
        self.created(DrawingKind::Path, points);
        self.finish_tool();
        true
    }

    pub fn double_click(&mut self, x: f32, y: f32) {
        if self.tool == Some(DrawingKind::Path) {
            self.finish_path();
        } else {
            match self.zone(x, y) {
                Zone::PriceAxis(p) => {
                    if let Some(pane) = self.panes.get(p) {
                        let group = pane.group;
                        let was = self.auto_scale();
                        self.pane_state.insert(group, PaneState { weight: self.weight(group), ..PaneState::default_for(group) });
                        self.note_auto(was);
                    }
                }
                Zone::TimeAxis => self.view_ready = false,
                // Reopens a note's text; other kinds have nothing to type.
                Zone::Plot(0) => {
                    if let Some(id) = self.drawing_at(x, y) {
                        self.select(Some(id));
                        self.events.push(Event::EditRequested(id));
                    }
                }
                _ => {}
            }
        }
        self.touch();
    }

    /// Returns whether the wheel was used, so the page knows not to scroll.
    pub fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        if self.tool.is_some() {
            return false;
        }
        self.span = None;
        let used = match self.zone(x, y) {
            Zone::PriceAxis(p) => {
                // The same scaling as dragging the axis, without a limit: scrolling
                // up narrows the range, down widens it. Autoscale turns off.
                self.zoom_price(p, (f64::from(dy) * 0.002).exp());
                true
            }
            Zone::Plot(_) | Zone::TimeAxis => {
                if dx.abs() > dy.abs() {
                    self.time.pan(f64::from(-dx));
                } else {
                    let at = self.zoom_anchor(x);
                    self.time.zoom_at(at, (-f64::from(dy) * 0.002).exp());
                }
                true
            }
            _ => false,
        };
        if used {
            self.touch();
        }
        used
    }

    /// Escape, Enter and Delete, when they mean something to the chart. Returns
    /// whether the key was used, so an unused Escape can still navigate.
    pub fn key(&mut self, key: &str) -> bool {
        let used = match key {
            "Escape" if !self.placed.is_empty() => {
                self.placed.clear();
                self.draft = None;
                true
            }
            "Escape" if self.tool.is_some() => {
                self.finish_tool();
                true
            }
            "Escape" if self.selected.is_some() => {
                self.selected = None;
                self.events.push(Event::SelectionChanged(None));
                true
            }
            "Escape" if self.selected_alert.is_some() => {
                self.selected_alert = None;
                self.touch();
                true
            }
            "Enter" if self.tool == Some(DrawingKind::Path) => self.finish_path(),
            "Delete" | "Backspace" => match self.selected {
                Some(id) => {
                    self.events.push(Event::DeleteRequested(id));
                    true
                }
                None => false,
            },
            _ => false,
        };
        if used {
            self.touch();
        }
        used
    }

    /// Whether every pane's price axis follows the bars on screen.
    pub fn auto_scale(&self) -> bool {
        self.pane_state.values().all(|s| s.auto)
    }

    /// Every pane back to following the bars on screen, keeping heights and margins.
    pub fn set_auto_scale(&mut self) {
        let was = self.auto_scale();
        for state in self.pane_state.values_mut() {
            state.auto = true;
            state.manual = None;
        }
        self.note_auto(was);
        self.touch();
    }

    fn note_auto(&mut self, was: bool) {
        let now = self.auto_scale();
        if now != was {
            self.events.push(Event::AutoScaleChanged(now));
        }
    }

    fn pane_auto(&self, pane: usize) -> bool {
        self.panes.get(pane).is_none_or(|v| self.pane_state.get(&v.group).is_none_or(|s| s.auto))
    }

    /// A range set by hand: the pane stops following the bars on screen.
    fn set_manual(&mut self, pane: usize, change: impl FnOnce(&mut PriceScale)) {
        let Some(view) = self.panes.get_mut(pane) else { return };
        change(&mut view.scale);
        let (range, group) = (view.scale.range, view.group);
        let was = self.auto_scale();
        let state = self.pane_state.entry(group);
        state.auto = false;
        state.manual = Some(range);
        self.note_auto(was);
        self.touch();
    }

    /// Where a zoom holds still: the newest bar while it is on screen, so zooming
    /// out brings in history and zooming in keeps the space after it the same
    /// width; scrolled back, the pointer.
    fn zoom_anchor(&self, pointer_x: f32) -> f64 {
        let plot_w = f64::from(self.plot_width());
        let newest = self.time.x(self.time.bars.saturating_sub(1) as f64);
        if self.time.bars > 0 && (0.0..=plot_w).contains(&newest) { newest } else { f64::from(pointer_x).within(0.0, plot_w) }
    }

    /// The bar slot under `x` — past either end of the data too, so the crosshair
    /// follows the pointer into the space after the newest bar. Always one whose
    /// centre is on the plot.
    fn slot_at(&self, x: f32) -> Option<i64> {
        if self.time.bars == 0 {
            return None;
        }
        let mut slot = self.time.logical(f64::from(x)).round() as i64;
        if self.time.x(slot as f64) >= f64::from(self.plot_width()) {
            slot -= 1;
        }
        if self.time.x(slot as f64) < 0.0 {
            slot += 1;
        }
        Some(slot)
    }

    /// A slot's open time: the bar's own, or projected along the calendar past the data.
    fn slot_time(&self, slot: i64) -> Option<i64> {
        let n = self.total_bars() as i64;
        if n == 0 {
            return None;
        }
        if (0..n).contains(&slot) {
            return self.bar(slot as usize).map(|b| b.time);
        }
        if slot >= n {
            let mut t = self.bar((n - 1) as usize)?.time;
            for _ in 0..(slot - n + 1).min(100_000) {
                t = self.timeframe.end(t);
            }
            return Some(t);
        }
        let first = self.bar(0)?.time;
        let step = self.bar(1).map_or(self.timeframe.end(first) - first, |b| b.time - first);
        Some(self.timeframe.align(first + slot * step))
    }

    fn zoom_price(&mut self, pane: usize, factor: f64) {
        let Some(group) = self.panes.get(pane).map(|v| v.group) else { return };
        // Never narrower than a few price steps — past that the axis runs out of
        // float precision and every candle becomes a full-height stripe — and
        // never wider than a thousand times the prices on it.
        let decimals = if group == Group::Price { self.precision() } else { 2 };
        let unit = 10f64.powi(-(decimals as i32));
        self.set_manual(pane, |scale| {
            let (lo, hi) = scale.range;
            let mid = scale.untransform((lo + hi) / 2.0).abs();
            let (min_span, max_span) = match scale.mode {
                ScaleMode::Normal => (unit * 4.0, (mid + (hi - lo)).max(unit) * 1000.0),
                ScaleMode::Log => (unit * 4.0 / mid.max(unit), 20.0),
            };
            scale.zoom(factor, min_span, max_span);
        });
    }

    fn pan_price(&mut self, pane: usize, dy: f32) {
        self.set_manual(pane, |scale| scale.pan(f64::from(dy)));
    }

    fn resize_panes(&mut self, upper: usize, dy: f32) {
        let (Some(a), Some(b)) = (self.panes.get(upper).copied(), self.panes.get(upper + 1).copied()) else { return };
        let (ha, hb) = (a.plot.h + dy, b.plot.h - dy);
        if ha < MIN_PANE_HEIGHT || hb < MIN_PANE_HEIGHT {
            return;
        }
        let total: f64 = self.panes.iter().map(|p| self.weight(p.group)).sum();
        let avail = f64::from(self.time_axis_top());
        for (group, h) in [(a.group, ha), (b.group, hb)] {
            self.pane_state.entry(group).weight = f64::from(h) / avail * total;
        }
        self.panes[upper].plot.h = ha;
        self.panes[upper + 1].plot.y += dy;
        self.panes[upper + 1].plot.h = hb;
        self.touch();
    }
}

/// A candle body in device pixels, odd once it is wide enough so the wick sits on
/// its centre, and the wick in CSS pixels: one pixel at any zoom, as on
/// TradingView — a wick that thickens with the body reads as a second body.
/// About how long a bar of `tf` lasts — a month as thirty days — to tell a longer timeframe from a shorter.
fn bar_span(tf: ChartTimeframe) -> i64 {
    let Source { interval, group } = tf.source();
    interval.ms().unwrap_or(30 * DAY) * i64::from(group)
}

fn candle_widths(bar_spacing: f64, dpr: f32) -> (f32, f32) {
    let mut body = (bar_spacing as f32 * dpr * 0.8).floor().max(1.0);
    if body >= 3.0 && (body as i32) % 2 == 0 {
        body -= 1.0;
    }
    (body, dpr.floor().max(1.0) / dpr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradoor_core::time::HOUR;

    fn candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let p = 100.0 + (i as f64 / 7.0).sin() * 10.0 + i as f64 * 0.05;
                Candle { open_time: 1_700_000_000_000 - 1_700_000_000_000 % HOUR + i as i64 * HOUR, open: p - 0.5, high: p + 1.5, low: p - 2.0, close: p + 0.5, volume: 10.0 + i as f64, quote_volume: None, taker_buy: None }
            })
            .collect()
    }

    #[test]
    fn a_trade_that_moves_only_hidden_volume_draws_nothing_and_the_view_says_where_it_starts() {
        let mut c = chart();
        let forming = candles(500)[499].open_time + HOUR;
        let tick = |close: f64, volume: f64| Some(LiveBar { open_time: forming, open: 130.0, high: 131.0, low: 129.0, close, volume, closed: false });
        c.set_live(tick(130.0, 1.0));
        c.render(1_800_000_000_000);
        c.set_live(tick(130.0, 2.0));
        assert!(c.needs_render(1_800_000_000_000), "volume is drawn");
        c.set_hidden("volume");
        c.render(1_800_000_000_000);
        c.set_live(tick(130.0, 3.0));
        assert!(!c.needs_render(1_800_000_000_000), "hidden, a volume-only trade changes nothing on screen");
        assert_eq!(c.live.map(|l| l.volume), Some(3.0), "though it is kept");
        c.set_live(tick(130.5, 4.0));
        assert!(c.needs_render(1_800_000_000_000), "a new price is drawn");
        let from = c.visible_from();
        assert!(from > 0.0 && from < 500.0, "the view starts inside the bars held: {from}");
    }

    #[test]
    fn a_hidden_study_waits_until_shown_and_a_tick_moves_open_interest_as_the_whole_would() {
        let mut c = chart();
        let bars = candles(500);
        let first = bars[0].open_time;
        let steps: Vec<(i64, [f64; 4])> = (0..520).map(|i| (first + i * HOUR, [1000.0 + i as f64, 1010.0 + i as f64, 990.0, 1005.0 + i as f64])).collect();
        c.set_open_interest(steps);
        c.set_hidden("oi:255:1:0,stochrsi:14:14:3:3");
        c.set_studies("oi:255:1:0,stochrsi:14:14:3:3");
        assert!(c.studies.iter().all(|s| s.lines.is_empty()), "hidden studies are not worked out");
        let forming = bars[499].open_time + HOUR;
        let tick = |close: f64| LiveBar { open_time: forming, open: 130.0, high: 131.0, low: 129.0, close, volume: 1.0, closed: false };
        c.set_live(Some(tick(130.0)));
        c.set_hidden("stochrsi:14:14:3:3");
        assert!(!c.studies[0].lines.is_empty() && c.studies[1].lines.is_empty(), "shown again, worked out at once");
        c.set_live(Some(tick(132.5)));
        let ticked = c.studies[0].clone();
        c.refresh_open_interest();
        assert_eq!(ticked.candles, c.studies[0].candles, "the forming bar alone, as the whole would have it");
        assert_eq!(ticked.lines.last().map(|l| l.values.clone()), c.studies[0].lines.last().map(|l| l.values.clone()));
        // The steps from the forming bar's open on, 500 to 519, in dollars at its last price.
        assert_eq!(c.studies[0].candles.last().copied().flatten(), Some([1500.0 * 132.5, 1529.0 * 132.5, 990.0 * 132.5, 1524.0 * 132.5]));
    }

    #[test]
    fn a_plot_takes_the_colour_and_width_the_reader_set_and_can_be_turned_off() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("ema:20,rsi:14");
        let red = Color::hex(0xff0000);
        let painted_now = |c: &mut Chart, color: Color| {
            let frame = c.render(1_800_000_000_000).clone();
            frame.colors.chunks(4).filter(|p| *p == [color.r, color.g, color.b, color.a]).count()
        };
        assert_eq!(painted_now(&mut c, red), 0);
        let default = c.studies_meta()[0].lines[0].2;
        assert!(painted_now(&mut c, default) > 0, "the palette's colour, until it is changed");

        c.set_styles("ema:20/ema=#ff0000,,");
        let thin = painted_now(&mut c, red);
        assert!(thin > 0, "the reader's colour");
        assert_eq!(painted_now(&mut c, default), 0, "and not the palette's");
        let meta = c.studies_meta();
        assert_eq!(meta[0].lines[0].2, red, "the legend reads the colour it draws in");

        c.set_styles("ema:20/ema=#ff0000,3,");
        assert!(painted_now(&mut c, red) > thin, "a wider line covers more");
        assert_eq!(c.studies_meta()[0].lines[0].3, 3.0);

        // Off: not drawn, no badge and out of the range.
        c.set_styles("ema:20/ema=#ff0000,3,1");
        assert_eq!(painted_now(&mut c, red), 0);
        assert_eq!(c.studies_meta()[0].lines[0].0, "ema:20:ema", "its settings still list it");

        // A hidden study lists its plots for its settings, its figures unread.
        c.set_hidden("ribbon,ema200,volume,liquidations,rsi:14");
        let meta = c.studies_meta();
        assert!(c.studies[1].lines.is_empty(), "nothing worked out");
        assert_eq!(meta[1].lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["rsi:14:rsi"]);
    }

    #[test]
    fn a_vwap_starts_each_session_with_no_line_across_the_turn() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("vwap:0:0:0:0");
        c.set_styles("vwap:0:0:0:0/vwap=#ff0000,1,,0");
        let frame = c.render(1_800_000_000_000).clone();
        let red = [255u8, 0, 0, 255];
        let xs: Vec<f32> = frame.positions.chunks(2).zip(frame.colors.chunks(4)).filter(|(_, p)| *p == red).map(|(xy, _)| xy[0]).collect();
        assert!(!xs.is_empty());
        let spacing = c.time.bar_spacing as f32;
        let mut sorted = xs.clone();
        sorted.sort_by(f32::total_cmp);
        assert!(sorted.windows(2).any(|w| w[1] - w[0] > spacing * 0.9), "a bar's width with nothing in it, where the day turned");
    }

    /// How many vertices of a colour a frame holds.
    fn inked(c: &mut Chart, color: Color) -> usize {
        let frame = c.render(1_800_000_000_000).clone();
        frame.colors.chunks(4).filter(|p| *p == [color.r, color.g, color.b, color.a]).count()
    }

    #[test]
    fn only_a_forming_bar_draws_without_a_panic() {
        let mut c = Chart::new(ChartTimeframe::Mo1);
        c.resize(1000.0, 600.0, 1.0);
        c.set_studies("bb,keltner,ichimoku,macd,volumedelta,vpvr,sessions,moon,vwap");
        c.set_live(Some(LiveBar { open_time: 1_800_000_000_000, open: 10.0, high: 12.0, low: 9.0, close: 11.0, volume: 5.0, closed: false }));
        c.render(1_800_000_000_000);
        // And a bar whose low stands over its high.
        c.set_live(Some(LiveBar { open_time: 1_800_000_000_000, open: 10.0, high: 9.0, low: 12.0, close: 11.0, volume: 5.0, closed: false }));
        c.render(1_800_000_000_000);
    }

    #[test]
    fn a_colours_opacity_carries_through_where_a_study_sets_its_own() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("sessions");
        c.set_styles("sessions/london=#00aaff80,,");
        assert!(inked(&mut c, Color::hex(0x00aaff).with_alpha(0x0e)) > 0, "half of the box's own tint");
    }

    #[test]
    fn a_channel_is_shaded_between_its_lines_until_it_is_turned_off() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("bb");
        c.set_styles("bb:20:2/fill=#123456ff,,");
        let ink = Color::hex(0x123456);
        assert!(inked(&mut c, ink) > 0, "the reader's shade");
        c.set_styles("bb:20:2/fill=#123456ff,,1");
        assert_eq!(inked(&mut c, ink), 0, "off");
    }

    #[test]
    fn the_cloud_is_drawn_past_the_last_bar_in_two_colours() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("ichimoku");
        let (up, down) = (Color::hex(0x00ff01), Color::hex(0xff0002));
        c.set_styles("ichimoku:9:26:52:26/cloudUp=#00ff01ff,,;ichimoku:9:26:52:26/cloudDown=#ff0002ff,,;ichimoku:9:26:52:26/leadA=#00ff03ff,,");
        // The prices swing, so span A runs above span B and below it.
        assert!(inked(&mut c, up) > 0 && inked(&mut c, down) > 0);
        // Scrolled so what lies ahead of the bars is on screen.
        let last = c.total_bars() as f64 - 1.0;
        let lead = Color::hex(0x00ff03);
        let frame = c.render(1_800_000_000_000).clone();
        let last_x = c.time.x(last) as f32;
        let beyond = frame.positions.chunks(2).zip(frame.colors.chunks(4)).filter(|(_, p)| *p == [lead.r, lead.g, lead.b, lead.a]).filter(|(xy, _)| xy[0] > last_x).count();
        assert!(beyond > 0, "the leading span runs past the last bar");
    }

    #[test]
    fn sessions_are_boxed_and_moons_marked_in_the_readers_colours() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("sessions");
        let ink = Color::hex(0x00aaff);
        c.set_styles("sessions/london=#00aaff,,");
        assert!(inked(&mut c, ink.with_alpha(0x1c)) > 0, "London tinted");
        c.set_styles("sessions/london=#00aaff,,1");
        assert_eq!(inked(&mut c, ink.with_alpha(0x1c)), 0, "and gone when turned off");

        // A hundred days: phases on screen whatever the view.
        let t0 = c.candles()[0].open_time;
        let days: Vec<Candle> = candles(100).into_iter().enumerate().map(|(i, bar)| Candle { open_time: t0 + i as i64 * 24 * HOUR, ..bar }).collect();
        c.set_candles(days);
        c.set_studies("moon");
        c.set_styles("moon/full=#00aaff,,;moon/new=#00aaff,,");
        assert!(inked(&mut c, ink) > 0);
        c.set_styles("moon/full=#00aaff,,1;moon/new=#00aaff,,1");
        assert_eq!(inked(&mut c, ink), 0);
    }

    #[test]
    fn the_visible_range_profile_sums_the_bars_on_screen() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("vpvr:10:70:30:0");
        let study = c.studies[0].clone();
        assert_eq!(study.label, "VPVR 10");
        let profile = c.visible_profile(&study).expect("bars on screen");
        assert_eq!(profile.total.len(), 10);
        let (from, to) = c.time.visible();
        let shown: f64 = (from.ceil() as usize..=(to.floor() as usize).min(c.total_bars() - 1)).filter_map(|i| c.bar(i)).map(|b| b.volume).sum();
        assert!((profile.total.iter().sum::<f64>() - shown).abs() < 1e-6, "every bar on screen and none other");
        assert!(profile.total[profile.poc] >= *profile.total.iter().max_by(|a, b| a.total_cmp(b)).unwrap());
        assert!(profile.area.0 <= profile.poc && profile.poc <= profile.area.1);
        let (val, poc, vah) = (profile.level("val").unwrap(), profile.level("poc").unwrap(), profile.level("vah").unwrap());
        assert!(val <= poc && poc <= vah);

        // Against the edge, in the reader's colours; the legend reads its levels.
        c.set_styles("vpvr:10:70:30:0/up=#00aaff,,;vpvr:10:70:30:0/down=#00aaff,,");
        let ink = Color::hex(0x00aaff);
        let frame = c.render(1_800_000_000_000).clone();
        let w = c.panes[0].plot.w;
        let xs: Vec<f32> = frame.positions.chunks(2).zip(frame.colors.chunks(4)).filter(|(_, p)| *p == [ink.r, ink.g, ink.b, 0x90] || *p == [ink.r, ink.g, ink.b, 0x38]).map(|(xy, _)| xy[0]).collect();
        assert!(!xs.is_empty() && xs.iter().all(|x| *x >= w * 0.69 && *x <= w + 0.01), "no wider than its share of the pane");
        c.pointer_move(500.0, 200.0);
        let legend = c.legend();
        assert!(legend.values.iter().any(|(id, v)| id == "vpvr:10:70:30:0:poc" && v != "—"));
    }

    #[test]
    fn volume_delta_has_a_pane_of_its_own_and_reads_the_exchanges_buyers() {
        let mut c = Chart::new(ChartTimeframe::H1);
        c.resize(1000.0, 600.0, 1.0);
        let mut bars = candles(300);
        for (i, bar) in bars.iter_mut().enumerate() {
            bar.taker_buy = Some(if i % 2 == 0 { bar.volume } else { 0.0 });
        }
        c.set_candles(bars);
        c.set_studies("volumedelta");
        c.render(1_800_000_000_000);
        assert_eq!(c.panes.iter().map(|p| p.group).collect::<Vec<_>>(), [Group::Price, Group::Delta]);
        let (above, below) = (c.theme.bull.with_alpha(0x80), c.theme.bear.with_alpha(0x80));
        assert!(inked(&mut c, above) > 0 && inked(&mut c, below) > 0);
        assert!(c.studies[0].stats.is_empty(), "nothing estimated");
    }

    #[test]
    fn a_span_of_time_fills_the_plot_and_a_position_carries_its_label_over_it() {
        let mut c = chart();
        // Asked for the moment new bars arrive, before any frame has counted them.
        c.set_candles(candles(800));
        let (from, to) = (c.times[100] as i64, c.times[140] as i64);
        c.show_span(from, to);
        c.render(1_800_000_000_000);
        let plot_w = c.panes[0].plot.w as f64;
        let (x0, x1) = (c.time.x(100.0), c.time.x(140.0));
        assert!(x0 > 0.0 && x1 < plot_w, "both ends on screen: {x0}, {x1} of {plot_w}");
        assert!((x0 + x1 - plot_w).abs() < c.time.bar_spacing * 2.0, "centred");
        assert!(x1 - x0 > plot_w * 0.8, "filling the plot");
        // More bars come in behind the first few a market opens on: still framed.
        c.set_candles(candles(1200));
        c.render(1_800_000_000_000);
        assert!((c.time.x(100.0) - x0).abs() < 1.0 && (c.time.x(140.0) - x1).abs() < 1.0, "framed again as the bars came");
        // Until the reader takes the view: then it is theirs.
        c.wheel(400.0, 200.0, 0.0, -120.0);
        c.set_candles(candles(1300));
        c.render(1_800_000_000_000);
        assert!((c.time.x(100.0) - x0).abs() > 5.0, "not framed again once the reader zoomed: {} against {x0}", c.time.x(100.0));
        c.set_candles(candles(800));
        c.render(1_800_000_000_000);
        c.show_span(from, to);
        c.render(1_800_000_000_000);

        let price = c.candles[120].close;
        let mut long = DrawingShape::new(
            Some(4),
            DrawingKind::Long,
            vec![Anchor { time: c.times[110], price }, Anchor { time: c.times[110], price: price * 0.99 }, Anchor { time: c.times[125], price: price * 1.02 }],
        );
        long.label = Some("#1 Long · Won +2.00R".into());
        c.set_drawings(vec![long]);
        let frame = c.render(1_800_000_000_000).clone();
        let tag = frame.texts.iter().find(|t| t.text == "#1 Long · Won +2.00R").expect("the label is drawn");
        let top = c.panes[0].scale.y(price * 1.02) as f32;
        assert!(tag.y < top && tag.y > top - 12.0, "just over the box, at {} for a top at {top}", tag.y);
        assert!((tag.x - c.time.x(110.0) as f32).abs() < c.time.bar_spacing as f32, "from its left edge");
    }

    #[test]
    fn a_key_level_in_force_runs_on_past_the_last_bar_while_its_label_fits() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("keylevels");
        let id = c.studies[0].id.clone();
        c.set_styles(&format!("{id}/wo=#ff0000,1,,0"));
        // The plot's own red: the price axis carries the level's tag in it too.
        let reach = |c: &mut Chart| {
            let frame = c.render(1_800_000_000_000).clone();
            let plot_w = c.panes[0].plot.w;
            frame.positions.chunks(2).zip(frame.colors.chunks(4)).filter(|(xy, p)| *p == [255u8, 0, 0, 255] && xy[0] < plot_w).map(|(xy, _)| xy[0]).fold(f32::MIN, f32::max)
        };
        let last = (c.times.len() - 1) as f64;

        // With room ahead, eight bars on.
        c.time.right_offset = 20.0;
        let far = reach(&mut c);
        let ahead = c.time.x(last + LEVEL_AHEAD_BARS) as f32;
        assert!((far - ahead).abs() < 2.0, "ends at {far}, eight bars on is {ahead}");

        // A length of its own, and none at all: then it stops at the last bar.
        c.time.right_offset = 40.0;
        c.set_styles(&format!("{id}/wo=#ff0000,1,,0;{id}/ahead=20"));
        let twenty = c.time.x(last + 20.0) as f32;
        assert!((reach(&mut c) - twenty).abs() < 2.0, "twenty bars on is {twenty}");
        c.set_styles(&format!("{id}/wo=#ff0000,1,,0;{id}/ahead=0"));
        let at_last = c.time.x(last) as f32;
        assert!((reach(&mut c) - at_last).abs() < 2.0, "stops at the last bar, {at_last}");
        c.set_styles(&format!("{id}/wo=#ff0000,1,,0"));

        // With the last bar near the edge, only as far as leaves its label on screen.
        c.time.right_offset = 2.0;
        let near = reach(&mut c);
        let plot_w = c.panes[0].plot.w;
        assert!(near > c.time.x(last) as f32 && near < plot_w - 2.0 * c.char_width, "ends at {near} of {plot_w}");
    }

    #[test]
    fn a_vwap_is_dotted_shaded_by_session_and_names_the_last_ones_levels() {
        let mut c = chart();
        c.set_hidden("ribbon,ema200,volume,liquidations");
        c.set_studies("vwap");
        let id = "vwap:0:1:0:0";
        let red = [255u8, 0, 0, 255];
        let segments = |c: &mut Chart, style: String| {
            c.set_styles(&style);
            let frame = c.render(1_800_000_000_000).clone();
            frame.colors.chunks(4).filter(|p| *p == red).count()
        };
        let dotted = segments(&mut c, format!("{id}/vwap=#ff0000,1,"));
        let solid = segments(&mut c, format!("{id}/vwap=#ff0000,1,,0"));
        assert!(dotted > solid * 2, "dots by default, {dotted} vertices against {solid}, and a solid line when asked");
        assert!(segments(&mut c, format!("{id}/vwap=#ff0000,1,,1")) < dotted, "dashes are longer than dots");

        // The shade stops at the turn of each session, with no slant across it.
        let blue = [0u8, 0, 255, 255];
        c.set_styles(&format!("{id}/fill1=#0000ff,,"));
        let frame = c.render(1_800_000_000_000).clone();
        let xs: Vec<f32> = frame.positions.chunks(2).zip(frame.colors.chunks(4)).filter(|(_, p)| *p == blue).map(|(xy, _)| xy[0]).collect();
        let spacing = c.time.bar_spacing as f32;
        let mut sorted = xs.clone();
        sorted.sort_by(f32::total_cmp);
        assert!(!sorted.is_empty() && sorted.windows(2).any(|w| w[1] - w[0] > spacing * 0.9), "a gap where the day turned");

        // Each previous level labelled beside the newest bar.
        let frame = c.render(1_800_000_000_000).clone();
        let labels: Vec<&str> = frame.texts.iter().map(|t| t.text.as_str()).filter(|t| t.starts_with("pD")).collect();
        assert_eq!(labels, ["pDVAH", "pDVWAP", "pDVAL"]);
        let w = c.panes[0].plot.w;
        assert!(frame.texts.iter().filter(|t| t.text.starts_with("pD")).all(|t| t.x > 0.0 && t.x < w), "on the pane");
        c.set_styles(&format!("{id}/prevVwap=,,1"));
        let frame = c.render(1_800_000_000_000).clone();
        assert_eq!(frame.texts.iter().filter(|t| t.text.starts_with("pD")).count(), 2, "and each can be turned off");
    }

    #[test]
    fn a_layer_takes_its_lengths_and_colours_from_the_reader() {
        let mut c = chart();
        let standard = c.engine.ema200.clone();
        c.set_styles("ema200/length=50;ribbon/fast=5;ribbon/slow=9");
        assert_ne!(c.engine.ema200, standard, "worked out again at once");
        let shorter = c.engine.ema200.clone();
        assert_ne!(c.engine.ema12, engine_series_with(c.candles(), Styles::default().lengths()).ema12, "the ribbon too");
        c.set_candles(candles(500));
        assert_eq!(c.engine.ema200, shorter, "and kept when the bars change");

        let green = Color::hex(0x00ff00);
        let painted_now = |c: &mut Chart, color: Color| {
            let frame = c.render(1_800_000_000_000).clone();
            frame.colors.chunks(4).filter(|p| *p == [color.r, color.g, color.b, color.a]).count()
        };
        c.set_styles("ema200/line=#00ff00,2,");
        assert!(painted_now(&mut c, green) > 0);

        // Volume in its own colour and height.
        let blue = Color::hex(0x0000ff).with_alpha(0x80);
        let top_of_blue = |c: &mut Chart| {
            let frame = c.render(1_800_000_000_000).clone();
            frame.positions.chunks(2).zip(frame.colors.chunks(4)).filter(|(_, p)| *p == [blue.r, blue.g, blue.b, blue.a]).map(|(xy, _)| xy[1]).fold(f32::MAX, f32::min)
        };
        c.set_styles("volume/up=#0000ff80,,;volume/down=#0000ff80,,;volume/height=40");
        let tall = top_of_blue(&mut c);
        c.set_styles("volume/up=#0000ff80,,;volume/down=#0000ff80,,;volume/height=10");
        assert!(tall < f32::MAX && top_of_blue(&mut c) > tall, "shorter at a tenth of the pane than at two fifths");

        // A number that does not read, or is out of bounds, falls back or is held in.
        assert_eq!(Styles::parse("ema200/length=x").lengths().2, 200);
        assert_eq!(Styles::parse("ema200/length=1").lengths().2, 2);
        assert_eq!(Styles::parse("nonsense;=;a=b").0.len(), 0);
    }

    #[test]
    fn the_liquidation_feed_keeps_to_the_size_and_colours_the_reader_set() {
        let mut c = chart();
        let now = 1_800_000_000_000;
        c.push_liquidation(2_000_000.0, true, 0, now);
        c.push_liquidation(500.0, false, 0, now);
        let ink = Color::hex(0x123456);
        c.set_styles("liquidations/long=#123456,,;liquidations/short=#123456,,");
        c.pointer_leave();
        let seen = |c: &mut Chart, at: i64| {
            let frame = c.render_feed(at);
            frame.texts.iter().filter(|t| t.color.r == ink.r && t.color.g == ink.g && t.color.b == ink.b).count()
        };
        assert_eq!(seen(&mut c, now + 2_000), 2);
        c.set_styles("liquidations/long=#123456,,;liquidations/short=#123456,,;liquidations/min=1000");
        assert_eq!(seen(&mut c, now + 2_000), 1, "the small one is left out");
    }

    fn chart() -> Chart {
        let mut c = Chart::new(ChartTimeframe::H1);
        c.resize(1000.0, 600.0, 1.0);
        c.set_candles(candles(500));
        c.render(1_800_000_000_000);
        c
    }

    fn click(c: &mut Chart, x: f32, y: f32) {
        c.pointer_down(x, y, 0);
        c.pointer_up(x, y);
    }

    /// Vertices painted in exactly `color`.
    fn painted(frame: &Frame, color: Color) -> usize {
        frame.colors.chunks(4).filter(|c| *c == [color.r, color.g, color.b, color.a]).count()
    }

    #[test]
    fn tpo_style_draws_each_session_as_a_profile_in_place_of_candles() {
        let mut c = chart();
        let t = Theme::default();
        let candle = t.bull.with_alpha(t.candle_alpha);
        assert!(painted(c.render(1_800_000_000_000), candle) > 0, "candles first");

        c.set_tpo("1d");
        assert_eq!(c.tpo(), Some(tpo::Period::Day));
        let frame = c.render(1_800_000_000_000);
        assert_eq!(painted(frame, candle), 0, "no candle bodies under the profiles");
        assert!(painted(frame, t.tpo_poc) > 0, "the point of control's line carries on to the next profile, in the theme's colour");
        let labels: Vec<&str> = frame.texts.iter().map(|t| t.text.as_str()).collect();
        assert!(["POC", "VAH", "VAL"].iter().all(|name| labels.contains(name)), "levels are named as MMT names them: {labels:?}");
        assert!(labels.iter().any(|l| l.len() == 9 && l.contains('/')), "each profile's day under it, like Fri 13/02");

        let cache = c.tpo_profiles.as_ref().expect("profiles were built");
        // 500 hourly bars are 21 days: the first and last partial, all of them drawn.
        assert!(cache.list.len() >= 20, "a profile per day, {} of them", cache.list.len());
        for profile in &cache.list {
            let brackets: usize = profile.session.rows.iter().map(|r| r.periods.len()).max().unwrap_or(0);
            assert!(brackets <= 24, "a day holds at most 24 hourly brackets in a row, not {brackets}");
        }
        // Side by side: each profile starts where the one before it ends its columns.
        for pair in cache.list.windows(2) {
            let (a, b) = (c.time.edge_x(pair[0].first as f64), c.time.edge_x(pair[1].first as f64));
            assert!((b - a - pair[0].columns * c.time.bar_spacing).abs() < 1e-6, "{a} then {b}");
        }

        c.set_tpo("");
        assert!(painted(c.render(1_800_000_000_000), candle) > 0, "and candles again");
    }

    #[test]
    fn a_tpo_session_must_be_longer_than_the_bars_it_stacks() {
        let mut c = chart();
        c.set_tpo("1h");
        assert_eq!(c.tpo(), None, "an hourly session of hourly bars has nothing to stack");
        c.set_tpo("nonsense");
        assert_eq!(c.tpo(), None);
        c.set_tpo("1w");
        assert_eq!(c.tpo(), Some(tpo::Period::Week));

        // Kept across a timeframe it cannot be drawn on, and back when it can.
        c.set_timeframe(ChartTimeframe::W1);
        assert_eq!(c.tpo(), None, "a weekly chart has no brackets to stack into a week");
        c.set_timeframe(ChartTimeframe::H1);
        assert_eq!(c.tpo(), Some(tpo::Period::Week));

        // A session is hours long: hourly brackets at the longest.
        c.set_tpo("london");
        assert_eq!(c.tpo(), Some(tpo::Period::London));
        c.set_timeframe(ChartTimeframe::H4);
        assert_eq!(c.tpo(), None);
    }

    #[test]
    fn a_tpo_chart_opens_on_square_blocks_and_keeps_them_while_auto_scaled() {
        let mut c = chart();
        c.set_tpo("1d");
        c.render(1_800_000_000_000);
        let row = c.tpo_profiles.as_ref().unwrap().row_size;
        let scale = c.panes[0].scale;
        let tall = (scale.y(1_000.0) - scale.y(1_000.0 + row)).abs();
        let wide = c.time.bar_spacing;
        assert!(tall <= wide * 1.001, "a row no taller than a column is wide: {tall} × {wide}");
        // Zoomed in, the range opens up with it, so rows grow as columns do.
        c.wheel(c.time.width as f32 / 2.0, 100.0, 0.0, -120.0);
        c.render(1_800_000_000_000);
        let scale = c.panes[0].scale;
        assert!((scale.y(1_000.0) - scale.y(1_000.0 + row)).abs() <= c.time.bar_spacing * 1.001);
    }

    #[test]
    fn hovering_a_profile_names_it_for_the_page_and_boxes_it() {
        let mut c = chart();
        c.set_tpo("1d");
        c.render(1_800_000_000_000);
        let profiles = c.tpo_profiles.as_ref().unwrap();
        let k = profiles.list.len() - 2;
        let (first, start) = (profiles.list[k].first, profiles.list[k].start);
        let x = c.time.edge_x(first as f64) as f32 + 2.0;
        let y = c.panes[0].scale.y(profiles.list[k].session.poc) as f32;
        c.pointer_move(x, y);
        let hover = c.legend().tpo.expect("a profile under the pointer");
        assert_eq!((hover.start, hover.expanded, hover.merged), (start, false, false));
        assert!(hover.prev.is_some() && hover.next.is_some());
        assert_eq!(hover.prices[0].0, "POC");

        c.set_tpo_edits(&format!("e={start}"));
        c.render(1_800_000_000_000);
        let profile = &c.tpo_profiles.as_ref().unwrap().list[k];
        assert!(profile.expanded && profile.columns >= profile.session.periods.len() as f64, "expanded, a column a bracket");
    }

    #[test]
    fn the_current_tpo_session_grows_with_the_live_bar() {
        let mut c = chart();
        c.set_tpo("1d");
        c.render(1_800_000_000_000);
        let before = c.tpo_profiles.as_ref().unwrap().list.last().unwrap().session.periods.len();
        let last = *c.candles().last().unwrap();
        c.set_live(Some(LiveBar { open_time: last.open_time + HOUR, open: last.close, high: last.close + 5.0, low: last.close - 5.0, close: last.close, volume: 1.0, closed: false }));
        c.render(1_800_000_000_000);
        let session = &c.tpo_profiles.as_ref().unwrap().list.last().unwrap().session;
        // The live hour opens a bracket in the current day, or starts the next day.
        assert!(session.periods.len() == before + 1 || session.periods.len() == 1, "the forming bracket is part of the profile");
        assert_eq!(*session.periods.last().unwrap(), last.open_time + HOUR);
    }

    #[test]
    fn a_new_timeframe_drops_the_old_bars_footprint() {
        let mut c = chart();
        let bar = footprint::Bar { time: c.candles()[10].open_time, levels: vec![(100.0, 1.0, 2.0)], ..footprint::Bar::default() };
        c.set_footprint(vec![bar], 1.0, true);
        c.set_timeframe(ChartTimeframe::H4);
        assert!(c.footprint.is_empty(), "an hour's columns would stand between four-hour candles");
    }

    #[test]
    fn wicks_stay_one_pixel_at_any_zoom() {
        for dpr in [1.0, 1.5, 2.0, 3.0] {
            let (_, thin) = candle_widths(2.0, dpr);
            let (body, wide) = candle_widths(2000.0, dpr);
            assert_eq!(thin, wide, "dpr {dpr}");
            assert!((wide * dpr - dpr.floor().max(1.0)).abs() < 1e-6, "one CSS pixel on dpr {dpr}");
            assert!(body > 40.0 && body as i32 % 2 == 1);
        }
    }

    #[test]
    fn empty_chart_says_so() {
        let mut c = Chart::new(ChartTimeframe::D1);
        c.resize(800.0, 400.0, 2.0);
        c.set_message(Some("no data".into()));
        let frame = c.render(0);
        assert_eq!(frame.texts.len(), 1);
        assert_eq!(frame.texts[0].text, "no data");
    }

    #[test]
    fn a_chart_draws_candles_axes_and_badges() {
        let mut c = chart();
        let frame = c.render(1_800_000_000_000);
        assert!(frame.vertex_count() > 150 * 6, "candles, wicks and volume");
        assert!(frame.texts.iter().any(|t| t.text.contains(':')), "time labels");
        assert!(frame.positions.iter().all(|v| v.is_finite()));
        assert_eq!(frame.colors.len(), frame.positions.len() * 2);
        let legend = c.legend();
        assert_eq!(legend.index, Some(499));
        assert!(legend.ohlc.starts_with('O'));
    }

    #[test]
    fn wheel_zoom_holds_the_newest_bar_or_else_the_bar_under_the_pointer() {
        let mut c = chart();
        let newest = (c.time.bars - 1) as f64;
        let x = c.time.x(newest);
        assert!(c.wheel(400.0, 200.0, 0.0, 240.0));
        assert!((c.time.x(newest) - x).abs() < 1e-6, "zooming out keeps the newest bar where it was");

        c.time.pan(100.0 * c.time.bar_spacing);
        assert!(c.time.right_offset < 0.0);
        let before = c.time.logical(400.0);
        assert!(c.wheel(400.0, 200.0, 0.0, -120.0));
        assert!((c.time.logical(400.0) - before).abs() < 1e-6);
    }

    #[test]
    fn zooming_in_goes_on_until_two_bars_fill_the_plot() {
        let mut c = chart();
        let newest = (c.time.bars - 1) as f64;
        let x = c.time.x(newest);
        for _ in 0..40 {
            c.wheel(300.0, 200.0, 0.0, -240.0);
            c.render(1_800_000_000_000);
        }
        assert!((c.time.bar_spacing - c.time.max_spacing()).abs() < 1e-6);
        assert!(c.time.bar_spacing > 400.0, "well past the old 80px stop");
        assert!((c.time.x(newest) - x).abs() < 1e-3, "the newest bar holds still");
    }

    #[test]
    fn dragging_the_dates_left_stretches_the_bars_and_right_squeezes_them() {
        let mut c = chart();
        let y = c.time_axis_top() + TIME_AXIS_HEIGHT / 2.0;
        let x = c.plot_width() / 2.0;
        let before = c.time.bar_spacing;

        c.pointer_down(x, y, 0);
        c.pointer_move(x - 60.0, y);
        c.pointer_up(x - 60.0, y);
        let stretched = c.time.bar_spacing;
        assert!(stretched > before, "left: bars wider, {before} to {stretched}");

        c.pointer_down(x, y, 0);
        c.pointer_move(x + 120.0, y);
        c.pointer_up(x + 120.0, y);
        assert!(c.time.bar_spacing < stretched, "right: bars narrower");
    }

    #[test]
    fn wheel_on_the_price_axis_zooms_price_without_a_limit() {
        let mut c = chart();
        let (lo, hi) = c.panes[0].scale.range;
        let axis_x = c.plot_width() + 10.0;
        for _ in 0..40 {
            assert!(c.wheel(axis_x, 200.0, 0.0, -240.0));
            c.render(1_800_000_000_000);
        }
        let (a, b) = c.panes[0].scale.range;
        assert!((b - a) < (hi - lo) * 0.01, "scrolled in to under a hundredth of the range");
        assert!(!c.auto_scale());
        for _ in 0..80 {
            c.wheel(axis_x, 200.0, 0.0, 240.0);
            c.render(1_800_000_000_000);
        }
        let (a, b) = c.panes[0].scale.range;
        assert!((b - a) > (hi - lo) * 10.0, "and out past ten times it");
    }

    #[test]
    fn price_zoom_stops_before_the_axis_loses_precision() {
        let mut c = chart();
        let axis_x = c.plot_width() + 10.0;
        for _ in 0..2000 {
            c.wheel(axis_x, 200.0, 0.0, -2400.0);
        }
        let texts = c.render(1_800_000_000_000).texts.clone();
        let (price, plot_w) = (c.panes[0], c.plot_width());
        let (lo, hi) = price.scale.range;
        let unit = 10f64.powi(-(c.precision() as i32));
        assert!(hi - lo >= unit * 4.0 * (1.0 - 1e-9), "span {} below four price steps", hi - lo);
        let labels = texts.iter().filter(|t| t.x > plot_w && t.y > price.plot.y && t.y < price.plot.bottom()).count();
        assert!(labels >= 3, "price labels still drawn, found {labels}");
        assert!(c.frame.positions.iter().all(|v| v.is_finite()));

        for _ in 0..2000 {
            c.wheel(axis_x, 200.0, 0.0, 2400.0);
        }
        c.render(1_800_000_000_000);
        let (lo, hi) = c.panes[0].scale.range;
        assert!(lo.is_finite() && hi.is_finite() && hi > lo);
        assert!(c.frame.positions.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn intraday_times_are_written_in_the_readers_time_zone() {
        let mut c = chart();
        let newest = c.time.bars - 1;
        let bar = c.candles()[newest].open_time;
        let x = c.time.x(newest as f64) as f32;

        c.set_utc_offset(7 * 60);
        c.pointer_move(x, 200.0);
        let shifted = ticks::crosshair_label(bar + 7 * HOUR, Some(HOUR), false);
        assert!(c.render(1_800_000_000_000).texts.iter().any(|t| t.text == shifted), "the crosshair reads {shifted} in UTC+7");

        // A day is the exchange's UTC day wherever it is read.
        c.set_timeframe(ChartTimeframe::D1);
        assert_eq!(c.shown_time(bar), bar);
    }

    #[test]
    fn the_crosshair_follows_the_pointer_past_the_newest_bar() {
        let mut c = chart();
        let newest = c.time.bars - 1;
        let px = (c.time.x(newest as f64) + 2.0 * c.time.bar_spacing) as f32;
        c.pointer_move(px, 200.0);
        assert_eq!(c.slot_at(px), Some(newest as i64 + 2));
        let projected = c.candles().last().unwrap().open_time + 2 * HOUR;
        assert_eq!(c.slot_time(newest as i64 + 2), Some(projected));
        let label = ticks::crosshair_label(projected, Some(HOUR), false);
        assert!(c.render(1_800_000_000_000).texts.iter().any(|t| t.text == label), "the time label shows the projected slot");
        assert_eq!(c.legend().index, Some(newest), "the legend keeps the newest bar");
    }

    #[test]
    fn a_formula_draws_as_a_study_in_a_pane_of_its_own() {
        let mut c = chart();
        let n = c.candles().len();
        let (blue, red) = (0x2962_ffff_u32, 0xff4d_5fff_u32);
        let mut values: Vec<f64> = (0..n).map(|i| i as f64).collect();
        values.extend(vec![f64::NAN; n]);
        let mut hits = vec![0.0; n];
        hits[n - 5] = 1.0;
        values.extend(hits);
        // A line, a label and a box, each its points; a background a colour a bar.
        values.extend([1.0, 100.0, 5.0, 110.0, 3.0, 105.0, 2.0, 90.0, 6.0, 95.0]);
        values.extend((0..n).map(|i| if i % 2 == 0 { f64::from(red) } else { 0.0 }));
        let meta = format!(
            "pane\t1\nplot\tline\t{blue}\t15\t0\t0\t0\tA\nplot\thistogram\t-\t15\t0\t0\t0\tB\nfill\t0\t1\t{blue}\nmark\t1\t{red}\tSell\nline\t{blue}\t20\t1\t2\nlabel\t{red}\t0\t0\t1\t0\tHH\x1f+2%\nbox\t{blue}\t{red}\t10\nbg\ntable\t2\t0\ncell\t1\t0\t{red}\t0\tAvg rise\n"
        );
        c.set_formula("fx-abc:14", "Mine 14", &meta, &values);
        let study = c.formulas.iter().find(|s| s.id == "fx-abc:14").expect("kept");
        assert_eq!((study.group, study.lines.len(), study.fills.len(), study.markers.len()), (Group::Formula(0), 2, 1, 1));
        assert_eq!(study.lines[0].rgba, Some(blue));
        assert_eq!(study.lines[1].rgba, None, "a histogram with no colour named takes the theme's, by sign");
        assert_eq!(study.lines[0].values[3], Some(3.0));
        assert!(study.markers[0].above && study.markers[0].text == "Sell" && study.markers[0].tone == Tone::Rgba(red));
        let art = study.art.as_ref().expect("its shapes");
        assert_eq!((art.lines.len(), art.labels.len(), art.boxes.len(), art.backgrounds.len(), art.cells.len()), (1, 1, 1, 1, 1));
        assert_eq!((art.lines[0].width, art.lines[0].style, art.lines[0].extend), (2.0, 1, 2));
        assert_eq!(art.labels[0].text, "HH\n+2%", "a label keeps its line breaks");
        assert_eq!((art.table, art.cells[0].text.as_str()), (Some((2, 0)), "Avg rise"));
        assert!(c.studies_meta().iter().any(|m| m.id == "fx-abc:14" && m.group == "fx0" && m.lines.len() == 2));
        let frame = c.render(1_800_000_000_000);
        assert!(frame.texts.iter().any(|t| t.text == "HH") && frame.texts.iter().any(|t| t.text == "+2%"), "the label's two lines");
        assert!(frame.texts.iter().any(|t| t.text == "Avg rise"), "the table");
        assert!(c.panes.iter().any(|p| p.group == Group::Formula(0)), "drawn below the chart");
        // Values for other bars than the chart's are let go; an error is said in the legend.
        c.set_formula("fx-abc:14", "Mine 14", &meta, &values[..10]);
        assert_eq!(c.formulas[0].lines.len(), 2, "the last good lines stand");
        c.set_formula("fx-bad", "Bad", "error\t3\t5\tema(value, length)\n", &[]);
        assert_eq!(c.formulas[1].stats, vec![("Error", "line 3: ema(value, length)".to_string())]);
        c.keep_formulas("fx-bad");
        assert!(c.formulas.iter().all(|s| s.id != "fx-abc:14"));
    }

    #[test]
    fn a_script_colours_its_candles_and_moves_a_line_by_its_offset() {
        let mut c = chart();
        let n = c.candles().len();
        let mut values: Vec<f64> = (0..n).map(|i| i as f64).collect();
        values.extend((0..n).map(|_| f64::from(0xffeb_3bff_u32)));
        let meta = "plot\tline\t4278190335\t15\t0\t26\t0\tLead\nbars\n";
        c.set_formula("fx-lead", "Lead", meta, &values);
        let study = &c.formulas[0];
        assert_eq!(study.lines[0].values.len(), n + 26, "on into the space ahead of the newest bar");
        assert_eq!((study.lines[0].values[25], study.lines[0].values[26]), (None, Some(0.0)));
        assert_eq!(c.bar_rgba(3), Some(Color::rgba(0xffeb_3bff)));
    }

    #[test]
    fn the_crosshair_price_stands_over_the_badges_it_covers() {
        let mut c = chart();
        let now = 1_800_000_000_000;
        let frame = c.render(now);
        let countdown = frame.texts.iter().find(|t| t.text.contains(':') && t.x > 900.0).expect("a countdown badge on the axis").clone();
        let badge_h = c.theme.font_px + 6.0;
        let clip = |c: &mut Chart, y: f32| {
            c.pointer_move(500.0, y);
            c.render(now).texts.iter().find(|t| t.text == countdown.text).and_then(|t| t.clip).expect("drawn, clipped to the axis")
        };
        assert_eq!(clip(&mut c, countdown.y - 4.0).h, 0.0, "its middle covered, the badge shows no text");
        let cut = clip(&mut c, countdown.y - 12.0);
        assert!((cut.y - (countdown.y - 12.0 + badge_h / 2.0)).abs() < 0.01, "an edge covered, its text stops where the crosshair's label ends");
        // Off the badges, they are whole.
        c.pointer_move(500.0, 100.0);
        let frame = c.render(now);
        let text = frame.texts.iter().find(|t| t.text == countdown.text).unwrap();
        assert!(text.clip.unwrap().contains(text.x, text.y + 6.0) && text.clip.unwrap().contains(text.x, text.y - 6.0));
    }

    #[test]
    fn another_charts_crosshair_stands_on_the_bar_holding_its_time() {
        let mut c = chart();
        let bar = c.candles()[480].open_time;
        let label = ticks::crosshair_label(bar, Some(HOUR), false);
        let shows = |c: &mut Chart, text: &str| c.render(1_800_000_000_000).texts.iter().any(|t| t.text == text);
        // Half an hour in: a minute chart's crosshair lands in this hour.
        c.set_linked_crosshair(Some(bar + HOUR / 2));
        assert!(shows(&mut c, &label), "its time label, at the bar holding it");
        c.pointer_move(100.0, 200.0);
        let own = ticks::crosshair_label(c.crosshair_time().unwrap(), Some(HOUR), false);
        assert!(shows(&mut c, &own) && !shows(&mut c, &label), "the chart's own crosshair takes its place");
        c.pointer_leave();
        assert!(shows(&mut c, &label), "and gives it back on leaving");
        c.set_linked_crosshair(None);
        assert!(!shows(&mut c, &label), "gone once the other chart's is");
    }

    #[test]
    fn another_charts_edge_time_scrolls_this_one_to_it_at_its_own_zoom() {
        let mut c = chart();
        let spacing = c.time.bar_spacing;
        let to = c.candles()[400].open_time;
        c.set_edge_time(to);
        c.render(1_800_000_000_000);
        let edge = c.edge_time().unwrap();
        assert!((edge - to).abs() <= HOUR, "the right edge at {edge}, asked {to}");
        assert_eq!(c.time.bar_spacing, spacing, "the zoom is its own");
        // Further back than the bars go, it stops a few bars in, as a drag does.
        c.set_edge_time(c.candles()[0].open_time - 1_000 * HOUR);
        assert!(c.time.visible().1 > 0.0, "some bars still on screen");
    }

    #[test]
    fn dragging_up_or_down_moves_price_and_turns_autoscale_off() {
        let mut c = chart();
        let grabbed = c.panes[0].scale.price(200.0);
        c.pointer_down(500.0, 200.0, 0);
        c.pointer_move(505.0, 204.0);
        c.pointer_move(510.0, 210.0);
        assert!(c.auto_scale(), "a sideways pan keeps autoscale");
        c.pointer_move(510.0, 260.0);
        c.pointer_up(510.0, 260.0);
        assert!(!c.auto_scale());
        assert!((c.panes[0].scale.price(260.0) - grabbed).abs() < 1e-6, "the grabbed price stays under the pointer");
        assert!(c.take_events().contains(&Event::AutoScaleChanged(false)));
        let range = c.panes[0].scale.range;
        c.render(1_800_000_000_000);
        assert_eq!(c.panes[0].scale.range, range, "a range set by hand survives a frame");

        c.set_auto_scale();
        assert!(c.auto_scale());
        assert_eq!(c.take_events(), vec![Event::AutoScaleChanged(true)]);

        c.pointer_down(500.0, 200.0, 0);
        c.pointer_move(500.0, 300.0);
        c.pointer_up(500.0, 300.0);
        assert!(!c.auto_scale());
        c.set_candles(Vec::new());
        c.render(1_800_000_000_000);
        c.set_candles(candles(300));
        c.render(1_800_000_000_000);
        assert!(c.auto_scale(), "new bars start autoscaled");
    }

    #[test]
    fn dragging_the_plot_pans() {
        let mut c = chart();
        let offset = c.time.right_offset;
        c.pointer_down(500.0, 200.0, 0);
        c.pointer_move(520.0, 200.0);
        c.pointer_move(600.0, 200.0);
        c.pointer_up(600.0, 200.0);
        assert!(c.time.right_offset < offset, "dragging right reveals older bars");
        assert!(c.take_events().iter().all(|e| !matches!(e, Event::SelectionChanged(_))), "a pan is not a click");
    }

    #[test]
    fn two_clicks_place_a_trendline_at_the_prices_clicked() {
        let mut c = chart();
        c.set_tool(Some(DrawingKind::Trendline));
        click(&mut c, 300.0, 200.0);
        c.pointer_move(500.0, 150.0);
        click(&mut c, 500.0, 150.0);
        let events = c.take_events();
        let Event::DrawingCreated { kind, points, .. } = &events[0] else { panic!("{events:?}") };
        assert_eq!((*kind, points.len()), (DrawingKind::Trendline, 2));
        assert!((c.panes[0].scale.y(points[1].price) - 150.0).abs() < 1e-6);
        assert_eq!(events[1], Event::ToolFinished);
        assert_eq!(c.tool(), None);
    }

    #[test]
    fn clicking_a_level_selects_it_and_delete_asks_to_remove_it() {
        let mut c = chart();
        let price = c.panes[0].scale.price(180.0);
        c.set_drawings(vec![DrawingShape::new(Some(7), DrawingKind::Horizontal, vec![Anchor { time: c.times[10], price }])]);
        c.render(1_800_000_000_000);
        click(&mut c, 50.0, 183.0);
        assert_eq!(c.take_events(), vec![Event::SelectionChanged(Some(7))]);
        assert!(c.key("Delete"));
        assert_eq!(c.take_events(), vec![Event::DeleteRequested(7)]);
        assert!(c.key("Escape"));
        assert!(!c.key("Escape"), "nothing left to cancel");
    }

    #[test]
    fn a_circle_is_picked_by_its_ring_and_a_locked_drawing_not_at_all() {
        let mut c = chart();
        let price = c.panes[0].scale.price(180.0);
        let mut circle = DrawingShape::new(Some(3), DrawingKind::Circle, vec![Anchor { time: c.times[400], price }]);
        circle.label = Some("1".into());
        c.set_drawings(vec![circle.clone()]);
        c.render(1_800_000_000_000);
        let (x, y) = c.place(&c.drawings[0], false).unwrap().points[0];
        click(&mut c, x + 11.0, y);
        assert_eq!(c.take_events(), vec![Event::SelectionChanged(Some(3))], "on its ring");
        assert!(c.key("Escape"));
        c.take_events();
        circle.style.locked = true;
        c.set_drawings(vec![circle]);
        c.render(1_800_000_000_000);
        click(&mut c, x + 11.0, y);
        assert!(!c.take_events().iter().any(|e| matches!(e, Event::SelectionChanged(Some(_)))), "a locked one is only shown");
    }

    #[test]
    fn the_liquidation_feed_stacks_the_newest_hit_at_the_foot() {
        let mut c = chart();
        let now = 1_800_000_000_000;
        let quiet = c.render(now).vertex_count();
        assert_eq!(c.render_feed(now).vertex_count(), 0, "no hit, nothing on the feed's layer");

        c.push_liquidation(2_000_000.0, true, 0, now);
        assert!(!c.needs_render(now + 100), "a hit leaves the chart under it as it was drawn");
        assert!(c.needs_feed(now + 100), "and slides in on a layer of its own");
        let frame = c.render_feed(now + 100);
        let (texts, icons) = (frame.texts.clone(), frame.icons.len());
        assert!(frame.vertex_count() > 0, "the hit is drawn");
        assert!(texts.iter().any(|t| t.text == "LONG $2.0M"), "says which side was closed out, and how big it was");
        assert_eq!(icons, 1, "with the venue's mark beside the figure");
        assert_eq!(c.render(now + 100).vertex_count(), quiet, "and none of it is drawn with the chart");
        assert!(c.needs_feed(now + 400), "it moves to where it settles");
        c.render_feed(now + 400);
        assert!(!c.needs_feed(now + 900), "where it stands, and the feed rests");
        c.render_feed(now + 4_050);
        assert!(c.needs_feed(now + 4_150), "until it fades");

        // Out of the candles' way: at the foot of the pane, inside it.
        let pane = c.panes[0].plot;
        let row = texts.iter().find(|t| t.text == "LONG $2.0M").unwrap();
        assert!(row.y < pane.bottom(), "the feed stays inside the pane");
        assert!(row.y > pane.y + pane.h * 0.9, "and sits at its foot");

        // A second hit, on another exchange, takes the foot and pushes the first up a row.
        c.push_liquidation(4_000.0, false, 1, now + 1_000);
        let frame = c.render_feed(now + 1_400);
        let marks: Vec<u8> = frame.icons.iter().map(|i| i.mark).collect();
        assert_eq!(marks, [1, 0], "each row carries its own exchange's mark, newest first");
        let ys: Vec<(String, f32)> = frame.texts.iter().filter(|t| t.text.contains('$')).map(|t| (t.text.clone(), t.y)).collect();
        assert_eq!(ys.len(), 2, "both hits are in the feed");
        let newest = ys.iter().find(|(text, _)| text == "SHORT $4K").unwrap().1;
        let older = ys.iter().find(|(text, _)| text == "LONG $2.0M").unwrap().1;
        assert!(newest > older, "the newest hit is at the foot and the older one has risen");

        assert!(c.needs_feed(now + 1_000 + LIQUIDATION_TTL), "gone, they are cleared once more");
        assert_eq!(c.render_feed(now + 1_000 + LIQUIDATION_TTL).vertex_count(), 0, "five seconds and they are gone");
        assert!(!c.needs_feed(now + 1_000 + LIQUIDATION_TTL), "and the feed rests again");

        c.push_liquidation(900.0, false, 0, now);
        c.set_hidden("liquidations");
        assert_eq!(c.render_feed(now + 100).vertex_count(), 0, "the layer can be switched off");
    }

    #[test]
    fn the_feed_stops_before_it_covers_the_chart() {
        let mut c = chart();
        let now = 1_800_000_000_000;

        for i in 0..20 {
            c.push_liquidation(1_000.0 * f64::from(i + 1), true, 0, now);
        }
        c.render(now);
        let frame = c.render_feed(now + 300);
        let rows = frame.texts.iter().filter(|t| t.text.contains('$')).count();
        assert!(rows <= LIQUIDATION_ROWS, "at most {LIQUIDATION_ROWS} rows, not {rows}");
        assert_eq!(rows, frame.icons.len(), "every row carries its mark");
        // The newest hits are the ones kept.
        assert!(frame.texts.iter().any(|t| t.text == "LONG $20K"), "the newest hit is in the feed");
        assert!(!frame.texts.iter().any(|t| t.text == "LONG $1K"), "the oldest has been pushed out");
    }

    #[test]
    fn a_point_drawn_at_a_shorter_timeframe_stands_on_the_bar_holding_it() {
        // A day chart and an hour chart over the same days.
        let base = 1_700_006_400_000 - 1_700_006_400_000 % DAY;
        let days: Vec<Candle> = candles(60).into_iter().enumerate().map(|(i, c)| Candle { open_time: base + i as i64 * DAY, ..c }).collect();
        let mut d = Chart::new(ChartTimeframe::D1);
        d.resize(1_000.0, 600.0, 1.0);
        d.set_candles(days);
        d.render(1_800_000_000_000);
        // A long entered at 08:00 on day 50, out at 20:00 on day 51, drawn on the 4H.
        let (from, to) = ((base + 50 * DAY + 8 * HOUR) as f64, (base + 51 * DAY + 20 * HOUR) as f64);
        let mut long = DrawingShape::new(Some(1), DrawingKind::Long, vec![Anchor { time: from, price: 100.0 }, Anchor { time: from, price: 95.0 }, Anchor { time: to, price: 110.0 }]);
        long.timeframe = Some(ChartTimeframe::H4);
        let xs = |c: &Chart, s: &DrawingShape| c.place(s, false).unwrap().points.iter().map(|p| p.0).collect::<Vec<f32>>();
        let on_day = xs(&d, &long);
        assert!((on_day[0] - d.time.x(50.0) as f32).abs() < 0.01, "on day 50's bar, not a third of a bar past it");
        assert!((on_day[2] - d.time.x(51.0) as f32).abs() < 0.01, "and out on day 51's");

        // At its own timeframe, and at shorter ones, where it was put.
        long.timeframe = Some(ChartTimeframe::D1);
        assert!((xs(&d, &long)[0] - d.time.x(50.0 + 8.0 / 24.0) as f32).abs() < 0.01, "drawn on the day chart, between bars as it was put");
        long.timeframe = None;
        assert!((xs(&d, &long)[0] - d.time.x(50.0 + 8.0 / 24.0) as f32).abs() < 0.01, "a shape with no timeframe is the chart's own");

        // Past the newest bar it runs on, so a box into the future keeps its length.
        long.timeframe = Some(ChartTimeframe::H4);
        long.points[2].time = (base + 63 * DAY + 12 * HOUR) as f64;
        assert!((xs(&d, &long)[2] - d.time.x(63.5) as f32).abs() < 0.01);
    }

    #[test]
    fn a_channel_reaching_far_off_screen_costs_what_shows_of_it() {
        let mut c = Chart::new(ChartTimeframe::H1);
        c.resize(1_000.0, 600.0, 1.0);
        let bars = candles(6_000);
        let (first, last) = (bars[0].open_time as f64, bars[5_999].open_time as f64);
        c.set_candles(bars);
        let now = 1_800_000_000_000;
        let bare = c.render(now).vertex_count();
        // Six thousand bars long at the opening zoom: some 40,000 pixels, its middle line dashed.
        let price = c.panes[0].scale.price(300.0);
        c.set_drawings(vec![DrawingShape::new(Some(1), DrawingKind::Channel, vec![Anchor { time: first, price }, Anchor { time: last, price }, Anchor { time: first, price: price * 0.98 }])]);
        let drawn = c.render(now).vertex_count() - bare;
        assert!(drawn > 0, "the channel is drawn");
        assert!(drawn < 3_000, "and only what shows of it: {drawn} vertices");
    }

    #[test]
    fn a_minute_chart_zooms_out_no_further_than_its_draw_distance() {
        let shown = |tf: ChartTimeframe| {
            let mut c = Chart::new(tf);
            c.resize(2_400.0, 600.0, 1.0);
            c.set_candles(candles(6_000));
            c.render(1_800_000_000_000);
            for _ in 0..60 {
                c.wheel(1_000.0, 200.0, 0.0, 400.0);
            }
            c.render(1_800_000_000_000);
            c.time.width / c.time.bar_spacing
        };
        assert!((shown(ChartTimeframe::M1) - MAX_SHOWN_SHORT_BARS).abs() < 1.0, "a minute chart stops at {MAX_SHOWN_SHORT_BARS} bars");
        assert!((shown(ChartTimeframe::M15) - MAX_SHOWN_SHORT_BARS).abs() < 1.0, "and a 15-minute one");
        assert!(shown(ChartTimeframe::H1) > 4_000.0, "an hour chart goes on to half a pixel a bar");

        // Zoomed out before the floor came up, it is brought in to it.
        let mut c = Chart::new(ChartTimeframe::M1);
        c.resize(2_400.0, 600.0, 1.0);
        c.set_candles(candles(6_000));
        c.render(1_800_000_000_000);
        c.time.bar_spacing = MIN_BAR_SPACING;
        c.render(1_800_000_000_000);
        assert!((c.time.width / c.time.bar_spacing - MAX_SHOWN_SHORT_BARS).abs() < 1.0);
    }

    #[test]
    fn a_drawing_moves_by_dragging_it_and_reshapes_by_its_handles() {
        let mut c = chart();
        let (a, b) = (Anchor { time: c.times[400], price: c.panes[0].scale.price(200.0) }, Anchor { time: c.times[450], price: c.panes[0].scale.price(150.0) });
        c.set_drawings(vec![DrawingShape::new(Some(3), DrawingKind::Trendline, vec![a, b])]);
        c.render(1_800_000_000_000);

        let (mx, my) = ((c.time.x(400.0) + c.time.x(450.0)) as f32 / 2.0, 175.0);
        // Not chosen yet, it is passed over: the drag pans the chart.
        let offset = c.time.right_offset;
        c.pointer_down(mx, my, 0);
        c.pointer_move(mx + 60.0, my + 40.0);
        c.pointer_up(mx + 60.0, my + 40.0);
        assert!(!c.take_events().iter().any(|e| matches!(e, Event::DrawingChanged { .. })), "an unchosen drawing stays where it was");
        assert_ne!(c.time.right_offset, offset, "and the chart moved instead");
        c.render(1_800_000_000_000);

        // A click chooses it; then a drag moves it. The pan moved the line on
        // screen, so its middle is found again.
        let ends = c.place(&c.drawings[0], false).unwrap().points;
        let (mx, my) = ((ends[0].0 + ends[1].0) / 2.0, (ends[0].1 + ends[1].1) / 2.0);
        click(&mut c, mx, my);
        assert!(c.take_events().contains(&Event::SelectionChanged(Some(3))));
        c.pointer_down(mx, my, 0);
        c.pointer_move(mx, my + 40.0);
        c.pointer_up(mx, my + 40.0);
        let events = c.take_events();
        let Some(Event::DrawingChanged { id, points }) = events.into_iter().find(|e| matches!(e, Event::DrawingChanged { .. })) else { panic!("no change event") };
        assert_eq!(id, 3);
        assert!(points[0].price < a.price && points[1].price < b.price, "dragging down lowers both ends");
        assert!((points[0].time - a.time).abs() < 1.0, "a straight-down drag keeps the times");

        c.render(1_800_000_000_000);
        let handle = c.place(&c.drawings[0], false).unwrap().points[1];
        c.pointer_down(handle.0, handle.1, 0);
        c.pointer_move(handle.0, handle.1 - 30.0);
        c.pointer_up(handle.0, handle.1 - 30.0);
        let Some(Event::DrawingChanged { points: reshaped, .. }) = c.take_events().into_iter().find(|e| matches!(e, Event::DrawingChanged { .. })) else { panic!("no change event") };
        assert_eq!(reshaped[0], points[0], "the other end stays put");
        assert!(reshaped[1].price > points[1].price, "the grabbed end rose");
    }

    #[test]
    fn a_tool_can_be_dragged_to_place_a_drawing_in_one_gesture() {
        let mut c = chart();
        c.set_tool(Some(DrawingKind::Rect));
        c.pointer_down(300.0, 200.0, 0);
        c.pointer_move(360.0, 230.0);
        assert!(c.draft.is_some(), "the rectangle follows the pointer while dragging");
        c.pointer_move(420.0, 260.0);
        c.pointer_up(420.0, 260.0);
        let events = c.take_events();
        let Some(Event::DrawingCreated { kind, points, .. }) = events.first().cloned() else { panic!("{events:?}") };
        assert_eq!((kind, points.len()), (DrawingKind::Rect, 2));
        assert_eq!(c.tool(), None, "and the tool is done");
    }

    #[test]
    fn studies_without_candles_draw_when_the_first_bar_is_in_view() {
        // A study pane with the whole history on screen: its first bar is index 0.
        let mut c = chart();
        c.set_studies("rsi:14,oi");
        c.set_open_interest(vec![(c.candles[0].open_time, [1.0, 2.0, 0.5, 1.5])]);
        c.show_range(None);
        let _ = c.render(0);
        c.set_candles(c.candles[..3].to_vec());
        let _ = c.render(0);
    }

    #[test]
    fn the_magnet_pulls_anchors_onto_a_bars_prices() {
        let mut c = chart();
        let _ = c.render(0);
        let i = c.candles.len() - 10;
        let bar = c.candles[i];
        let x = c.time.x(i as f64) as f32 + 1.0;
        let y_close = c.panes[0].scale.y(bar.close) as f32;
        c.set_magnet(Magnet::Weak);
        let near = c.point_at(x, y_close + 3.0).unwrap();
        assert_eq!((near.time, near.price), (bar.open_time as f64, bar.close), "a weak magnet takes the close three pixels off");
        let far_y = c.panes[0].scale.y(bar.high) as f32 - 80.0;
        let far = c.point_at(x, far_y).unwrap();
        assert!(far.price > bar.high, "far from the bar a weak magnet leaves the price");
        let ticks = 10f64.powi(c.precision() as i32);
        assert_eq!(far.price, (far.price * ticks).round() / ticks, "on a whole tick");
        c.set_magnet(Magnet::Strong);
        assert_eq!(c.point_at(x, far_y).unwrap().price, bar.high, "a strong magnet always takes the nearest");
        c.set_magnet(Magnet::Off);
        assert_eq!(c.point_at(x, far_y), c.anchor_at(x, far_y));
        // Ctrl held over no magnet: a weak one while it is.
        c.set_ctrl(true);
        assert_eq!(c.point_at(x, y_close + 3.0).unwrap().price, bar.close);
        c.set_ctrl(false);
        assert_ne!(c.point_at(x, y_close + 3.0).unwrap().price, bar.close);

        // The crosshair stands where the anchor would: its price tag reads the high, not the pointer's price.
        let tag = |c: &mut Chart, y: f32| {
            c.pointer_move(x, y);
            let frame = c.render(0).clone();
            frame.texts.iter().any(|t| t.text == c.format_value(Group::Price, bar.high))
        };
        let near_high = c.panes[0].scale.y(bar.high) as f32 + 3.0;
        c.set_magnet(Magnet::Strong);
        assert!(tag(&mut c, far_y), "strong: on the high from far off");
        c.set_magnet(Magnet::Weak);
        assert!(!tag(&mut c, far_y), "weak: free far from the bar");
        assert!(tag(&mut c, near_high), "weak: on the high three pixels off");
        c.set_magnet(Magnet::Off);
        assert!(!tag(&mut c, near_high), "off: where the pointer is");
    }

    #[test]
    fn shift_keeps_a_line_level_with_its_first_point() {
        let mut c = chart();
        c.set_tool(Some(DrawingKind::Trendline));
        c.set_shift(true);
        c.pointer_down(300.0, 200.0, 0);
        c.pointer_up(300.0, 200.0);
        c.pointer_down(420.0, 280.0, 0);
        c.pointer_up(420.0, 280.0);
        let events = c.take_events();
        let Some(Event::DrawingCreated { points, .. }) = events.first() else { panic!("{events:?}") };
        assert_eq!(points[0].price, points[1].price, "the second end sits at the first one's price");
        assert!(points[1].time > points[0].time);
    }

    #[test]
    fn a_position_lands_whole_on_one_click_and_its_handles_set_prices() {
        let mut c = chart();
        c.set_tool(Some(DrawingKind::Long));
        c.pointer_down(300.0, 200.0, 0);
        c.pointer_up(300.0, 200.0);
        let events = c.take_events();
        let Some(Event::DrawingCreated { kind, points, .. }) = events.first().cloned() else { panic!("{events:?}") };
        assert_eq!(kind, DrawingKind::Long);
        let (entry, stop, target) = (points[0], points[1], points[2]);
        assert!(stop.price < entry.price && target.price > entry.price, "a long's stop is below and its target above");
        assert_eq!(stop.time, target.time, "the right edge is one time");
        assert!(stop.time > entry.time);

        c.set_drawings(vec![DrawingShape::new(Some(9), DrawingKind::Long, points)]);
        c.set_selected(Some(9));
        let placed = c.place(&c.drawings[0], false).unwrap();
        let handles = drawings::handles(&placed);
        assert_eq!(handles.len(), 4);
        // The stop handle sits on the left edge and moves only the stop.
        let (hx, hy) = handles[1];
        c.pointer_down(hx, hy, 0);
        c.pointer_move(hx + 50.0, hy + 20.0);
        c.pointer_up(hx + 50.0, hy + 20.0);
        let shape = &c.drawings[0];
        assert!(shape.points[1].price < stop.price);
        assert_eq!((shape.points[0], shape.points[2].price), (entry, target.price));
        assert_eq!(shape.points[1].time, stop.time, "a price handle leaves the width alone");
    }

    #[test]
    fn a_positions_figures_show_on_hover_and_selection_only() {
        let mut c = chart();
        c.set_tool(Some(DrawingKind::Long));
        c.pointer_down(300.0, 200.0, 0);
        c.pointer_up(300.0, 200.0);
        let Some(Event::DrawingCreated { points, .. }) = c.take_events().into_iter().next() else { panic!() };
        c.set_drawings(vec![DrawingShape::new(Some(5), DrawingKind::Long, points)]);
        let figures = |c: &mut Chart| c.render(0).texts.iter().any(|t| t.text.starts_with("Target"));
        c.pointer_move(50.0, 400.0);
        assert!(!figures(&mut c), "hidden away from it");
        c.pointer_move(310.0, 190.0);
        assert!(figures(&mut c), "shown under the pointer");
        c.pointer_move(50.0, 400.0);
        c.set_selected(Some(5));
        assert!(figures(&mut c), "shown while selected");
        c.set_selected(None);
        c.drawings[0].style = DrawingStyle::from_pairs([("alwaysStats", "1")]);
        assert!(figures(&mut c), "or always, when asked");
    }

    #[test]
    fn a_channel_is_picked_between_its_lines_and_offsets_by_its_third_point() {
        let mut c = chart();
        let a = c.anchor_at(200.0, 200.0).unwrap();
        let b = c.anchor_at(400.0, 150.0).unwrap();
        let third = c.anchor_at(300.0, 275.0).unwrap();
        c.set_drawings(vec![DrawingShape::new(Some(4), DrawingKind::Channel, vec![a, b, third])]);
        let placed = c.place(&c.drawings[0], false).unwrap();
        let handles = drawings::handles(&placed);
        // The third handle rides the middle of the parallel line: 100px below the first.
        assert!((handles[2].0 - 300.0).abs() < 1.0 && (handles[2].1 - 275.0).abs() < 1.0, "{handles:?}");
        assert_eq!(c.drawing_at(300.0, 230.0), Some(4), "inside the band");
        assert_eq!(c.drawing_at(300.0, 330.0), None, "outside it");
    }

    #[test]
    fn a_channel_stays_parallel_on_a_log_scale() {
        let mut c = chart();
        c.set_log_scale(true);
        c.render(1_800_000_000_000);
        // Across most of the pane's prices, where log and linear part ways most.
        let a = c.anchor_at(200.0, 520.0).unwrap();
        let b = c.anchor_at(400.0, 60.0).unwrap();
        let third = c.anchor_at(300.0, 390.0).unwrap();
        c.set_drawings(vec![DrawingShape::new(Some(4), DrawingKind::Channel, vec![a, b, third])]);
        let placed = c.place(&c.drawings[0], false).unwrap();
        let handles = drawings::handles(&placed);
        // 100px below the main line at both ends, so its middle is at the third point.
        assert!((handles[2].0 - 300.0).abs() < 0.5 && (handles[2].1 - 390.0).abs() < 0.5, "{handles:?}");
    }

    #[test]
    fn an_alert_line_is_found_under_the_pointer_and_dragged_to_a_new_price() {
        let mut c = chart();
        let level = c.anchor_at(300.0, 200.0).unwrap().price;
        c.set_alerts(vec![AlertLine::at(9, level)]);
        c.render(1_800_000_000_000);
        assert_eq!(c.alert_at(300.0, 202.0).map(|(id, _)| id), Some(9));
        assert_eq!(c.alert_at(300.0, 220.0), None);
        c.pointer_move(300.0, 200.0);
        assert_eq!(c.legend().alert.map(|(id, _)| id), Some(9), "the page is told which one is hovered");

        // Not picked yet, a drag that starts on it pans instead of moving it.
        c.pointer_down(300.0, 200.0, 0);
        c.pointer_move(300.0, 260.0);
        c.pointer_up(300.0, 260.0);
        assert!(!c.take_events().iter().any(|e| matches!(e, Event::AlertMoved { .. })), "a press that grazes the line leaves it be");
        let level_y = c.panes[0].scale.y(level) as f32;
        click(&mut c, 300.0, level_y);
        assert_eq!(c.selected_alert, Some(9), "a click picks it");

        c.pointer_down(300.0, level_y, 0);
        c.pointer_move(300.0, level_y + 60.0);
        // A refresh from the page mid-drag does not snap it back.
        c.set_alerts(vec![AlertLine::at(9, level)]);
        c.pointer_up(300.0, level_y + 60.0);
        let moved = c.take_events().into_iter().find_map(|e| match e {
            Event::AlertMoved { id: 9, price } => Some(price),
            _ => None,
        });
        let expected = c.anchor_at(300.0, level_y + 60.0).unwrap().price;
        assert!(moved.is_some_and(|p| (p - expected).abs() < 0.01 * expected.abs()), "{moved:?} vs {expected}");
        assert!(moved.unwrap() < level, "dragged down is lower");
        click(&mut c, 300.0, 40.0);
        assert_eq!(c.selected_alert, None, "a click elsewhere lets it go");
        let level_y = c.panes[0].scale.y(c.alerts[0].price) as f32;
        click(&mut c, 300.0, level_y);
        c.set_selected(None);
        assert_eq!(c.selected_alert, Some(9), "the page echoing no drawing picked does not let it go");
        assert!(c.key("Escape"));
        assert_eq!(c.selected_alert, None, "Escape does");
    }

    #[test]
    fn an_alert_is_dragged_by_its_badge_on_the_price_axis_too() {
        let mut c = chart();
        let level = c.anchor_at(300.0, 200.0).unwrap().price;
        c.set_alerts(vec![AlertLine::at(9, level)]);
        c.render(1_800_000_000_000);
        let axis_x = c.plot_width() + 10.0;
        assert_eq!(c.alert_handle_at(axis_x, 203.0).map(|(id, _)| id), Some(9), "the badge is a handle");
        assert_eq!(c.alert_handle_at(axis_x, 260.0), None, "the bare axis is not");

        // The first press on the badge picks the alert, the next moves it.
        c.pointer_down(axis_x, 200.0, 0);
        c.pointer_move(axis_x, 150.0);
        c.pointer_up(axis_x, 150.0);
        assert!(!c.take_events().iter().any(|e| matches!(e, Event::AlertMoved { .. })));
        assert_eq!(c.selected_alert, Some(9));
        c.pointer_down(axis_x, 200.0, 0);
        c.pointer_move(axis_x, 150.0);
        c.pointer_up(axis_x, 150.0);
        let moved = c.take_events().into_iter().find_map(|e| match e {
            Event::AlertMoved { id: 9, price } => Some(price),
            _ => None,
        });
        assert!(moved.is_some_and(|p| p > level), "dragged up is higher, not a price scale zoom: {moved:?}");
        assert!(c.auto_scale(), "the scale was not touched");
    }

    #[test]
    fn an_alert_line_is_drawn_in_its_own_style() {
        let mut c = chart();
        let level = c.anchor_at(300.0, 200.0).unwrap().price;
        let t = c.theme.clone();
        let count = |c: &mut Chart, color: Color| c.render(1_800_000_000_000).colors.chunks(4).filter(|p| *p == [color.r, color.g, color.b, color.a]).count();

        c.set_alerts(vec![AlertLine::at(9, level)]);
        assert!(count(&mut c, t.alert) > 0, "unstyled, in the theme's colour as before");

        let faint = Color::hex(0xf23645).with_alpha(0x60);
        c.set_alerts(vec![AlertLine { color: Some(faint), dash: Dash::Solid, width: 2.0, ..AlertLine::at(9, level) }]);
        assert!(count(&mut c, faint) > 0, "the line in its colour, at its opacity");
        assert!(count(&mut c, faint.with_alpha(255)) > 0, "its ring and badge in the colour whole, to be found and read");
        assert_eq!(count(&mut c, t.alert), 0, "and nothing left in the theme's");
    }

    #[test]
    fn a_finger_scaling_or_panning_past_an_alert_leaves_it_where_it_is() {
        let mut c = chart();
        let level = c.anchor_at(300.0, 200.0).unwrap().price;
        c.set_alerts(vec![AlertLine::at(9, level)]);
        c.render(1_800_000_000_000);
        c.set_touch(true);
        let axis_x = c.plot_width() + 10.0;

        // On its badge, the drag a phone scales prices with.
        c.pointer_down(axis_x, 200.0, 0);
        c.pointer_move(axis_x, 150.0);
        c.pointer_up(axis_x, 150.0);
        // And across its line, the drag that pans.
        c.pointer_down(300.0, 201.0, 0);
        c.pointer_move(300.0, 260.0);
        c.pointer_up(300.0, 260.0);
        assert!(!c.take_events().iter().any(|e| matches!(e, Event::AlertMoved { .. })), "neither moved the alert");
        assert!(!c.auto_scale(), "the axis drag scaled prices instead");

        // Held, the line is taken — from a little further than a pointer reaches.
        c.pointer_down(300.0, c.panes[0].scale.y(level) as f32 + 12.0, 0);
        c.pointer_hold(300.0, c.panes[0].scale.y(level) as f32 + 12.0);
        c.pointer_move(300.0, 100.0);
        c.pointer_up(300.0, 100.0);
        let moved = c.take_events().into_iter().find_map(|e| match e {
            Event::AlertMoved { id: 9, price } => Some(price),
            _ => None,
        });
        assert!(moved.is_some_and(|p| p > level), "{moved:?}");
    }

    #[test]
    fn drawings_go_in_a_study_pane_on_its_values() {
        let mut c = chart();
        c.set_studies("rsi:14");
        c.render(1_800_000_000_000);
        let (price, rsi) = (c.panes[0], c.panes[1]);
        assert_eq!(rsi.group, Group::Pct);
        let (top, bottom) = (rsi.plot.y + rsi.plot.h * 0.2, rsi.plot.y + rsi.plot.h * 0.8);

        // A line dragged across the RSI pane is the RSI pane's, on its own values.
        c.set_tool(Some(DrawingKind::Trendline));
        c.pointer_down(300.0, bottom, 0);
        c.pointer_move(360.0, (top + bottom) / 2.0);
        c.pointer_move(420.0, top);
        c.pointer_up(420.0, top);
        let Some(Event::DrawingCreated { kind: DrawingKind::Trendline, points, pane }) = c.take_events().into_iter().find(|e| matches!(e, Event::DrawingCreated { .. })) else { panic!("drawn") };
        assert_eq!(pane, Some(Group::Pct));
        let value = |y: f32| rsi.scale.price(f64::from(y));
        assert!((points[0].price - value(bottom)).abs() < 1e-6 && (points[1].price - value(top)).abs() < 1e-6, "its values, not prices");

        // Kept, it stands in that pane, is caught there and nowhere over the candles.
        let mut shape = DrawingShape::new(Some(7), DrawingKind::Trendline, points.clone());
        shape.style.set("pane", "pct");
        c.set_drawings(vec![shape]);
        c.render(1_800_000_000_000);
        let placed = c.place(&c.drawings[0], false).expect("placed");
        assert_eq!(placed.plot, rsi.plot);
        assert!((placed.points[0].1 - bottom).abs() < 0.5 && (placed.points[1].1 - top).abs() < 0.5);
        let middle = ((placed.points[0].0 + placed.points[1].0) / 2.0, (placed.points[0].1 + placed.points[1].1) / 2.0);
        assert_eq!(c.drawing_at(middle.0, middle.1), Some(7), "caught on its line");
        let over_candles = price.plot.y + price.plot.h / 2.0;
        assert_eq!(c.drawing_at(middle.0, over_candles), None);
        // A click there picks it, and its handle drags it within the pane.
        c.set_tool(None);
        c.pointer_down(middle.0, middle.1, 0);
        c.pointer_up(middle.0, middle.1);
        assert_eq!(c.selected, Some(7));

        // A brush likewise; a position, which is priced, is not placed there.
        c.set_tool(Some(DrawingKind::Brush));
        c.pointer_down(200.0, bottom, 0);
        for i in 1..20 {
            c.pointer_move(200.0 + i as f32 * 5.0, bottom - i as f32);
        }
        c.pointer_up(300.0, bottom - 20.0);
        assert!(c.take_events().iter().any(|e| matches!(e, Event::DrawingCreated { kind: DrawingKind::Brush, pane: Some(Group::Pct), .. })));
        c.set_tool(Some(DrawingKind::Long));
        c.pointer_down(300.0, bottom, 0);
        c.pointer_up(300.0, bottom);
        assert!(!c.take_events().iter().any(|e| matches!(e, Event::DrawingCreated { .. })));

        // The study gone, so is its drawing: it comes back with it.
        c.set_studies("");
        c.render(1_800_000_000_000);
        assert!(c.place(&c.drawings[0], false).is_none());
    }

    #[test]
    fn a_finger_sliding_the_chart_shows_no_crosshair_and_a_mouse_dragging_does() {
        for touch in [true, false] {
            let mut c = chart();
            c.render(1_800_000_000_000);
            c.set_touch(touch);
            let before = c.time.right_offset;
            c.pointer_down(400.0, 300.0, 0);
            for i in 1..=10 {
                c.pointer_move(400.0 + i as f32 * 12.0, 300.0);
            }
            assert_ne!(c.time.right_offset, before, "it pans");
            assert_eq!(c.crosshair.is_some(), !touch, "touch {touch}");
            c.pointer_up(520.0, 300.0);
        }
    }

    #[test]
    fn a_tap_pins_the_crosshair_a_finger_moves_it_and_a_tool_places_there() {
        let mut c = chart();
        c.render(1_800_000_000_000);
        c.set_touch(true);
        let tap = |c: &mut Chart, x: f32, y: f32| {
            c.pointer_down(x, y, 0);
            c.pointer_up(x, y);
            c.pointer_leave();
        };
        tap(&mut c, 300.0, 200.0);
        assert_eq!(c.crosshair, Some((300.0, 200.0)), "shown where tapped, and kept when the finger lifts");

        // A finger anywhere moves it by its motion; the chart does not pan.
        let before = c.time.right_offset;
        c.pointer_down(500.0, 300.0, 0);
        c.pointer_move(520.0, 290.0);
        c.pointer_move(540.0, 280.0);
        c.pointer_up(540.0, 280.0);
        c.pointer_leave();
        assert_eq!(c.crosshair, Some((340.0, 180.0)));
        assert_eq!(c.time.right_offset, before);

        tap(&mut c, 600.0, 300.0);
        assert_eq!(c.crosshair, None, "the next tap puts it away");

        // With a tool: the first tap shows it, a tap places an anchor where it stands, not under the finger.
        c.set_tool(Some(DrawingKind::Trendline));
        tap(&mut c, 300.0, 200.0);
        assert!(c.placed.is_empty() && c.crosshair == Some((300.0, 200.0)));
        tap(&mut c, 700.0, 350.0);
        let first = c.anchor_at(300.0, 200.0).unwrap();
        assert_eq!(c.placed, vec![first], "placed at the crosshair");
        c.pointer_down(700.0, 350.0, 0);
        c.pointer_move(760.0, 330.0);
        c.pointer_up(760.0, 330.0);
        tap(&mut c, 100.0, 100.0);
        let events = c.take_events();
        let Some(Event::DrawingCreated { points, .. }) = events.iter().find(|e| matches!(e, Event::DrawingCreated { .. })) else { panic!("{events:?}") };
        assert_eq!(points[1], c.anchor_at(360.0, 180.0).unwrap());

        // A drawing picked by a tap is put down by the next tap beside it, not pinned under the crosshair.
        let (a, b) = (c.anchor_at(300.0, 150.0).unwrap(), c.anchor_at(500.0, 250.0).unwrap());
        c.set_drawings(vec![DrawingShape::new(Some(9), DrawingKind::Trendline, vec![a, b])]);
        c.render(1_800_000_000_000);
        if c.crosshair.is_some() {
            tap(&mut c, 650.0, 380.0);
        }
        assert_eq!(c.crosshair, None, "the line drawn last left the crosshair up: a tap put it away");
        tap(&mut c, 400.0, 200.0);
        assert_eq!((c.selected, c.crosshair), (Some(9), None), "picked, no crosshair");
        tap(&mut c, 650.0, 380.0);
        assert_eq!((c.selected, c.crosshair), (None, None), "put down by a tap beside it");
        tap(&mut c, 650.0, 380.0);
        assert_eq!(c.crosshair, Some((650.0, 380.0)), "and the next tap shows the crosshair again");
        tap(&mut c, 650.0, 380.0);

        // A mouse again: no pinned crosshair.
        c.set_touch(false);
        c.pointer_leave();
        assert_eq!(c.crosshair, None);
    }

    #[test]
    fn a_held_finger_is_a_crosshair_and_with_a_tool_it_aims() {
        let mut c = chart();
        c.set_touch(true);
        let before = c.time.right_offset;
        c.pointer_down(400.0, 300.0, 0);
        c.pointer_hold(400.0, 300.0);
        c.pointer_move(250.0, 180.0);
        assert_eq!(c.time.right_offset, before, "a held finger does not pan");
        assert_eq!(c.crosshair, Some((250.0, 180.0)), "the crosshair goes with it");
        c.pointer_up(250.0, 180.0);
        assert_eq!(c.crosshair, Some((250.0, 180.0)), "and stays where it was let go, to be read");
        assert!(c.drawings.is_empty() && c.take_events().is_empty(), "with no tool, nothing is placed");

        c.set_tool(Some(DrawingKind::Trendline));
        let aim = |c: &mut Chart, from: (f32, f32), to: (f32, f32)| {
            c.pointer_down(from.0, from.1, 0);
            c.pointer_hold(from.0, from.1);
            c.pointer_move(to.0, to.1);
            c.pointer_up(to.0, to.1);
        };
        aim(&mut c, (300.0, 300.0), (320.0, 250.0));
        aim(&mut c, (500.0, 300.0), (540.0, 200.0));
        let points = c.take_events().into_iter().find_map(|e| match e {
            Event::DrawingCreated { kind: DrawingKind::Trendline, points, .. } => Some(points),
            _ => None,
        });
        let points = points.expect("two holds placed a trend line");
        let first = c.anchor_at(320.0, 250.0).unwrap();
        assert!((points[0].price - first.price).abs() < 1e-6 * first.price.abs(), "anchored where the finger was lifted, not where it went down");
    }

    #[test]
    fn the_legend_reads_the_price_under_the_pointer() {
        let mut c = chart();
        c.pointer_move(300.0, 150.0);
        let l = c.legend();
        let (y, price) = l.cursor.unwrap();
        assert_eq!(y, 150.0);
        assert!((price - c.anchor_at(300.0, 150.0).unwrap().price).abs() < 1e-6);
        assert!(l.axis_x > 900.0);
    }

    #[test]
    fn a_path_finishes_on_enter_and_the_brush_on_release() {
        let mut c = chart();
        c.set_tool(Some(DrawingKind::Path));
        for x in [200.0, 300.0, 400.0] {
            click(&mut c, x, 200.0);
        }
        assert!(c.key("Enter"));
        assert!(matches!(&c.take_events()[0], Event::DrawingCreated { kind: DrawingKind::Path, points, .. } if points.len() == 3));

        c.set_tool(Some(DrawingKind::Brush));
        c.pointer_down(100.0, 100.0, 0);
        for i in 1..20 {
            c.pointer_move(100.0 + i as f32 * 5.0, 100.0 + i as f32);
        }
        c.pointer_up(200.0, 120.0);
        assert!(matches!(&c.take_events()[0], Event::DrawingCreated { kind: DrawingKind::Brush, points, .. } if points.len() > 10));
    }

    #[test]
    fn the_footprint_cell_under_the_pointer_is_the_one_the_price_falls_in() {
        let mut c = chart();
        // A recent bar: turned on, the footprint widens the bars and keeps the newest in view.
        let i = c.candles().len() - 3;
        let bar = c.candles()[i];
        // Levels on the grid, as the server records them: whole multiples of a row.
        let base = (bar.low / 0.5).floor() * 0.5;
        let levels = vec![(base, 1.0, 2.0), (base + 0.5, 3.0, 0.5), (base + 1.0, 0.25, 4.0)];
        // Rows as the reader set them, half a dollar each, so the test knows where they are.
        c.set_footprint(vec![footprint::Bar { time: bar.open_time, levels, ..Default::default() }], 0.5, false);
        c.set_footprint_options("delta;1;0;1");
        c.render(1_800_000_000_000);
        assert_eq!(c.footprint_at(10.0, 10.0), None, "nothing is drawn until the style is chosen");

        c.set_footprint_shown(true);
        c.render(1_800_000_000_000);
        assert!(c.time.bar_spacing >= footprint::READABLE_SPACING - 0.01, "the bars widened to where a cell can be read: {}", c.time.bar_spacing);
        let pane = c.panes[0];
        let x = c.time.x(i as f64) as f32;
        // The middle of the lowest row, and the middle of the one above it.
        let low = pane.scale.y(base + 0.25) as f32;
        let up = pane.scale.y(base + 0.75) as f32;
        assert_eq!(c.footprint_at(x, low).map(|cell| (cell.buy, cell.sell)), Some((1.0, 2.0)));
        assert_eq!(c.footprint_at(x, up).map(|cell| (cell.buy, cell.sell)), Some((3.0, 0.5)));
        assert_eq!(c.footprint_at(x - 400.0, low), None, "and a bar away from it is not it");
    }

    #[test]
    fn fitted_footprint_rows_join_as_the_chart_zooms_out() {
        let mut c = chart();
        let bar = c.candles()[400];
        let levels: Vec<(f64, f64, f64)> = (0..40).map(|i| (bar.low + f64::from(i) * 0.01, 1.0, 1.0)).collect();
        c.set_footprint(vec![footprint::Bar { time: bar.open_time, levels, ..Default::default() }], 0.01, true);
        c.set_footprint_shown(true);
        c.render(1_800_000_000_000);
        let pane = c.panes[0];
        let step = c.footprint_step(&pane);
        let middle = pane.scale.price(f64::from(pane.plot.y + pane.plot.h / 2.0));
        let tall = (pane.scale.y(middle) - pane.scale.y(middle + step)).abs();
        assert!(tall >= 16.0, "a row stands tall enough for its number: {tall}px for {step}");
        let finer = step / 2.0;
        assert!(step <= 0.01 || (pane.scale.y(middle) - pane.scale.y(middle + finer)).abs() < 16.0 * 1.01, "and is the finest round grouping that does");
    }

    #[test]
    fn a_minute_that_ran_hard_is_marked_where_it_traded_and_found_under_the_pointer() {
        let mut c = chart();
        let bar = c.candles()[400];
        let (green, red) = (FlowMark { at: bar.open_time, price: bar.close, usd: 5_000_000.0 }, FlowMark { at: c.candles()[402].open_time, price: c.candles()[402].close, usd: -1_000_000.0 });
        c.set_flow(vec![green, red]);
        let t = c.theme.clone();
        let painted_now = |c: &mut Chart| {
            let frame = c.render(1_800_000_000_000).clone();
            move |color: Color| frame.colors.chunks(4).filter(|p| *p == [color.r, color.g, color.b, color.a]).count()
        };
        assert_eq!(painted_now(&mut c)(t.bull.with_alpha(90)), 0, "nothing is marked until the study is added");
        assert_eq!(c.flow_at(0.0, 0.0), None, "and nothing is found under the pointer either");

        c.set_studies("flow");
        let painted = painted_now(&mut c);
        assert!(painted(t.bull.with_alpha(90)) > 0, "the buyers' minute is green");
        assert!(painted(t.bear.with_alpha(90)) > 0, "the sellers' minute is red");

        // The bigger flow draws the bigger dot, and the pointer finds it.
        let pane = c.panes[0];
        let (x, y) = c.flow_xy(&green, &pane).unwrap();
        assert!(c.flow_radius(&green) > c.flow_radius(&red));
        assert_eq!(c.flow_at(x + 1.0, y + 1.0).map(|(i, _, _)| i), Some(0));
        assert_eq!(c.flow_at(x, y + 40.0), None, "nowhere near it");
        c.pointer_move(x, y);
        assert_eq!(c.legend().flow.map(|(i, _, _)| i), Some(0), "the page is told which one is hovered");
        assert!(painted(t.bull.with_alpha(220)) == 0, "and it brightens only once drawn again");
        assert!(c.render(1_800_000_000_000).colors.chunks(4).any(|p| *p == [t.bull.r, t.bull.g, t.bull.b, 220]));
    }

    #[test]
    fn the_market_just_left_is_held_dimmed_while_the_next_one_loads() {
        let mut c = chart();
        let bright = c.render(1_800_000_000_000).clone();
        c.set_loading(true);
        let dim = c.render(1_800_000_000_000).clone();
        assert!(dim.positions.len() > bright.positions.len(), "the candles are still drawn, with a veil over them");
        let scrim = c.theme.background.with_alpha(170);
        assert!(dim.colors.chunks(4).any(|p| *p == [scrim.r, scrim.g, scrim.b, scrim.a]), "the veil is the background, half there");
        assert!(dim.texts.iter().any(|t| t.text == "Loading…"));

        c.set_loading(false);
        assert_eq!(c.render(1_800_000_000_000).positions.len(), bright.positions.len(), "and lifts when the candles land");
    }

    #[test]
    fn the_watermark_sits_bottom_left_of_the_price_pane() {
        let mut c = chart();
        c.set_studies("rsi:14");
        let frame = c.render(1_800_000_000_000).clone();
        let price = c.panes[0].plot;
        assert!(!frame.texts.iter().any(|t| t.text == "tradoor"), "the mark stands alone; the name is not written out");

        // The mark is drawn in the accent, and nowhere else is down there.
        let accent = c.theme.ribbon_fast;
        let corner: Vec<(f32, f32)> = frame
            .positions
            .chunks(2)
            .zip(frame.colors.chunks(4))
            .filter(|(_, c)| *c == [accent.r, accent.g, accent.b, 205])
            .map(|(p, _)| (p[0], p[1]))
            .collect();
        assert!(!corner.is_empty(), "the mark is painted");
        assert!(corner.iter().all(|(x, y)| *x < price.x + 60.0 && *y > price.bottom() - 60.0 && *y < price.bottom()), "bottom left of the price pane");
        assert_eq!(c.legend().mark, None, "the frame draws it: the page need not");

        // Drawn by the page instead, over a live chart: the frame leaves it out and says where it goes.
        c.set_page_mark(true);
        let frame = c.render(1_800_000_000_000).clone();
        assert!(!frame.colors.chunks(4).any(|c| *c == [accent.r, accent.g, accent.b, 205]), "not painted twice");
        let (x, y, ink, _) = c.legend().mark.expect("the page is told where");
        assert_eq!((x, y, ink), (price.x + 12.0, price.bottom() - 46.0, accent));

        // Over a footprint's stats table it stood on the rows' names: it stands above the table.
        c.set_footprint_shown(true);
        c.set_footprint_options("delta;1;1;1");
        let last = *c.candles().last().unwrap();
        c.set_footprint(vec![footprint::Bar { time: last.open_time, levels: vec![(last.close.floor(), 1.0, 2.0)], ..Default::default() }], 1.0, false);
        c.render(1_800_000_000_000);
        let top = c.table_top().expect("the table is shown");
        let (_, y, _, _) = c.legend().mark.expect("still there");
        assert_eq!(y, top - 46.0);
        c.set_footprint_options("delta;1;0;1");
        c.render(1_800_000_000_000);
        assert_eq!(c.legend().mark.map(|m| m.1), Some(c.panes[0].plot.bottom() - 46.0), "back down with the table off");
    }

    #[test]
    fn stoch_rsi_parts_from_rsi_into_a_pane_of_its_own_and_back() {
        let mut c = chart();
        c.set_studies("rsi:14,stochrsi:14:14:3:3");
        c.render(1_800_000_000_000);
        assert_eq!(c.panes.iter().map(|p| p.group).collect::<Vec<_>>(), [Group::Price, Group::Pct]);
        c.set_pct_apart(true);
        c.render(1_800_000_000_000);
        assert_eq!(c.panes.iter().map(|p| p.group).collect::<Vec<_>>(), [Group::Price, Group::Pct, Group::Stoch]);
        let meta = c.studies_meta();
        assert_eq!(meta.iter().map(|m| m.group).collect::<Vec<_>>(), ["pct", "stoch"], "the page lists each in its own pane");
        let panes = c.legend().panes;
        assert_eq!(panes.iter().map(|p| p.0).collect::<Vec<_>>(), ["pct", "stoch"]);
        assert!(panes[0].1 < panes[1].1, "each pane's top, in order down the chart");
        assert_eq!(c.pane_extent(Group::Stoch, 0, 10), (0.0, 100.0), "on the same scale as RSI's");
        c.set_pct_apart(false);
        c.render(1_800_000_000_000);
        assert_eq!(c.panes.len(), 2);
    }

    #[test]
    fn studies_open_their_own_panes() {
        let mut c = chart();
        c.set_studies("rsi:14,stochrsi,macd,ema:50");
        c.render(1_800_000_000_000);
        assert_eq!(c.panes.iter().map(|p| p.group).collect::<Vec<_>>(), [Group::Price, Group::Pct, Group::Macd]);
        let meta = c.studies_meta();
        assert_eq!(meta.len(), 4);
        assert_eq!(meta[2].lines.len(), 3);
        c.set_hidden("macd:12:26:9");
        c.render(1_800_000_000_000);
        assert_eq!(c.panes.len(), 2);
        assert!(c.legend().values.iter().any(|(id, _)| id == "rsi:14:rsi"));
    }

    #[test]
    fn scrolling_back_asks_for_history_once() {
        let mut c = chart();
        assert!(c.take_events().is_empty());
        c.time.pan(1e9);
        c.render(1_800_000_000_000);
        c.render(1_800_000_000_000);
        let asks = c.take_events().into_iter().filter(|e| matches!(e, Event::NeedHistory { .. })).count();
        assert_eq!(asks, 1);
    }

    #[test]
    fn log_scale_and_live_bar() {
        let mut c = chart();
        c.set_log_scale(true);
        let last = *c.candles().last().unwrap();
        c.set_live(Some(LiveBar { open_time: last.open_time + HOUR, open: 120.0, high: 125.0, low: 119.0, close: 124.0, volume: 3.0, closed: false }));
        let frame = c.render(1_800_000_000_000);
        assert!(frame.vertex_count() > 0);
        assert!(c.panes[0].scale.y(125.0) < c.panes[0].scale.y(119.0));
        assert_eq!(c.legend().index, Some(500));
        c.set_live(Some(LiveBar { open_time: last.open_time, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 1.0, closed: false }));
        assert!(c.live.is_none(), "a bar that is already stored is not forming");
    }

    #[test]
    fn a_pinned_bar_stands_where_it_stood_as_the_bars_after_it_go() {
        let mut c = chart();
        c.render(1_800_000_000_000);
        let i = c.candles().len() - 25;
        let x = c.time.x(i as f64);
        assert!((0.0..c.time.width).contains(&x), "on the plot to begin with");
        c.pin_view(c.candles()[i].open_time);
        c.set_candles(c.candles()[..=i].to_vec());
        c.render(1_800_000_000_000);
        assert!((c.time.x(i as f64) - x).abs() < 0.01, "held at {x}");
        assert!(c.right_room() > 20.0, "the bars gone leave their room");
    }

    #[test]
    fn back_at_the_newest_bars_the_zoom_stays() {
        let mut c = chart();
        c.render(1_800_000_000_000);
        c.time.pan(600.0);
        let spacing = c.time.bar_spacing;
        c.show_newest();
        c.render(1_800_000_000_000);
        assert_eq!(c.right_room(), RIGHT_MARGIN_BARS);
        assert_eq!(c.time.bar_spacing, spacing);
    }

    #[test]
    fn a_bar_is_found_by_where_it_stands() {
        let mut c = chart();
        c.render(1_800_000_000_000);
        let i = c.candles().len() - 10;
        assert_eq!(c.bar_time_at(c.time.x(i as f64) as f32), Some(c.candles()[i].open_time));
        c.pointer_move(c.time.x(i as f64) as f32, 100.0);
        assert_eq!(c.legend().bar_x.map(f64::from).map(f64::round), Some(c.time.x(i as f64).round()));
        c.pointer_leave();
        assert_eq!(c.legend().bar_x, None, "no line with no pointer");
    }

    #[test]
    fn a_footprint_bar_past_the_newest_candle_is_not_read() {
        let mut c = chart();
        c.set_footprint_shown(true);
        let n = c.candles().len();
        let last = c.candles()[n - 1];
        let level = last.close.floor();
        let levels = vec![(level, 1.0, 2.0)];
        let bar = |time: i64| footprint::Bar { time, levels: levels.clone(), ..Default::default() };
        c.set_footprint(vec![bar(last.open_time), bar(last.open_time + HOUR)], 1.0, false);
        c.render(1_800_000_000_000);
        let y = c.panes[0].scale.y(level + 0.5) as f32;
        assert!(c.footprint_at(c.time.x((n - 1) as f64) as f32, y).is_some(), "the newest bar's own");
        assert_eq!(c.footprint_at(c.time.x(n as f64) as f32, y), None, "no candle under this one yet");
    }

    #[test]
    fn separators_resize_panes() {
        let mut c = chart();
        c.set_studies("rsi:14");
        c.render(1_800_000_000_000);
        let y = c.panes[1].plot.y;
        let before = c.panes[1].plot.h;
        c.pointer_down(300.0, y, 0);
        c.pointer_move(300.0, y - 40.0);
        c.pointer_up(300.0, y - 40.0);
        c.render(1_800_000_000_000);
        assert!((c.panes[1].plot.h - (before + 40.0)).abs() < 2.0);
        assert!(c.take_events().iter().any(|e| matches!(e, Event::PaneWeightsChanged(_))));
    }

    #[test]
    fn the_footprint_table_is_as_tall_as_its_top_edge_is_dragged() {
        let mut c = chart();
        let i = c.candles().len() - 2;
        let bar = c.candles()[i];
        c.set_footprint(vec![footprint::Bar { time: bar.open_time, levels: vec![(bar.low, 1.0, 2.0)], ..Default::default() }], 0.5, false);
        c.set_footprint_options("delta;1;1;1");
        c.set_footprint_shown(true);
        c.render(1_800_000_000_000);
        let pane = c.panes[0];
        let top = c.table_top().expect("a table");
        assert_eq!(pane.plot.bottom() - top, footprint::table_height(footprint::TABLE_ROW, pane.plot.h));

        c.pointer_down(300.0, top, 0);
        c.pointer_move(300.0, top - 30.0);
        c.pointer_up(300.0, top - 30.0);
        c.render(1_800_000_000_000);
        let taller = c.table_top().expect("a table");
        assert!((top - taller - 30.0).abs() < 0.01, "the edge follows the pointer: {top} to {taller}");
        let row = c.take_events().iter().find_map(|e| if let Event::FootprintTableChanged(row) = e { Some(*row) } else { None }).expect("the page is told");
        assert_eq!(row, c.footprint_table_row);

        // What the page saved comes back as it left, and a chart saved before this has the default.
        c.set_footprint_options(&format!("delta;1;1;1;{row}"));
        assert_eq!(c.footprint_table_row, row);
        c.set_footprint_options("delta;1;1;1");
        assert_eq!(c.footprint_table_row, footprint::TABLE_ROW);

        // It stops at half the plot, however far it is pulled.
        c.render(1_800_000_000_000);
        let top = c.table_top().expect("a table");
        c.pointer_down(300.0, top, 0);
        c.pointer_move(300.0, pane.plot.y);
        c.pointer_up(300.0, pane.plot.y);
        c.render(1_800_000_000_000);
        assert!(pane.plot.bottom() - c.table_top().expect("a table") <= pane.plot.h * 0.5 + 0.01);

        // A press well clear of the edge is a press on the chart.
        let top = c.table_top().expect("a table");
        c.pointer_down(300.0, top - 60.0, 0);
        assert!(!matches!(c.drag, Drag::Table { .. }));
        c.pointer_up(300.0, top - 60.0);

        // With a drawing tool armed the edge is chart too: the tool's next point goes there.
        c.set_tool(Some(DrawingKind::Horizontal));
        c.pointer_down(300.0, top, 0);
        assert!(!matches!(c.drag, Drag::Table { .. }));
    }
}
