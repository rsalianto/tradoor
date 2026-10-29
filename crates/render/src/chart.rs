//! The chart: data, view, interaction, and the frame that draws them.

use std::collections::{BTreeMap, BTreeSet};

use tradoor_core::chart::{engine_series, EngineSeries};
use tradoor_core::format::{format_compact, format_countdown, format_price, format_turnover, price_precision};
use tradoor_core::live::LiveBar;
use tradoor_core::studies::{compute_study_with, idle_study, open_interest_live, parse_studies, ComputedStudy, Group, LineStyle, LineType, StudyInputs, StudyKind, StudySpec, Tone};
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
    DrawingCreated { kind: DrawingKind, points: Vec<Anchor> },
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
    /// The alert line under the pointer, and its y.
    pub alert: Option<(i64, f32)>,
    /// The flow mark under the pointer: which one, and where it sits.
    pub flow: Option<(usize, f32, f32)>,
    /// The footprint cell under the pointer, as the screen groups it, for the
    /// page to hang its reading on.
    pub footprint: Option<FootprintCell>,
    /// The TPO profile under the pointer, for the page's menu on it.
    pub tpo: Option<tpo::Hover>,
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
    /// `(studyId:lineKey, label, colour)`.
    pub lines: Vec<(String, String, Color)>,
    pub stats: Vec<(String, String)>,
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
    /// Whether those columns stand in for the candles.
    footprint_shown: bool,
    /// How a level reads, and what else a column carries.
    footprint_options: footprint::Options,
    precision: usize,
    precision_override: Option<usize>,
    engine: EngineSeries,
    ribbon: Vec<RibbonRun>,
    study_specs: Vec<StudySpec>,
    /// Summed open interest from the server, as `(time, [o, h, l, c])` steps.
    open_interest: Vec<(i64, [f64; 4])>,
    studies: Vec<ComputedStudy>,
    hidden: BTreeSet<String>,
    log_scale: bool,
    time: TimeScale,
    view_ready: bool,
    pane_state: BTreeMap<Group, PaneState>,
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
    placed: Vec<Anchor>,
    draft: Option<DrawingShape>,
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
            footprint_shown: false,
            footprint_options: footprint::Options::default(),
            precision: 2,
            precision_override: None,
            engine: engine_series(&[]),
            ribbon: Vec::new(),
            study_specs: Vec::new(),
            days: Vec::new(),
            open_interest: Vec::new(),
            studies: Vec::new(),
            hidden: BTreeSet::new(),
            log_scale: false,
            time: TimeScale::new(),
            view_ready: false,
            pane_state: BTreeMap::new(),
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
            selected: None,
            selected_alert: None,
            tool: None,
            placed: Vec::new(),
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
        self.engine = engine_series(&self.candles);
        self.ribbon = ribbon_runs(&self.engine.ema12, &self.engine.ema25);
        self.studies = self.compute_studies(&self.study_specs);
        if self.candles.is_empty() {
            self.view_ready = false;
        }
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
        let inputs = StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days };
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
        let inputs = StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days };
        for (spec, study) in self.study_specs.iter().zip(self.studies.iter_mut()) {
            if spec.kind == StudyKind::OpenInterest && !self.hidden.contains(&study.id) && !open_interest_live(spec, study, &self.candles, &inputs) {
                *study = compute_study_with(spec, &self.candles, &inputs);
            }
        }
    }

    fn study_inputs(&self) -> StudyInputs<'_> {
        StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days }
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
        let was = std::mem::replace(&mut self.hidden, raw.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect());
        // A study shown again is worked out now; one hidden lets its figures go.
        let inputs = StudyInputs { open_interest: &self.open_interest, live: self.live.map(|l| (l.open_time, l.close)), days: &self.days };
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
    /// and what else a column carries, as `mode;va;stats;profile`.
    pub fn set_footprint_options(&mut self, raw: &str) {
        let mut parts = raw.split(';');
        let on = |part: Option<&str>| part != Some("0");
        let options = footprint::Options {
            mode: footprint::Mode::parse(parts.next().unwrap_or_default()),
            value_area: on(parts.next()),
            stats: on(parts.next()),
            profile: on(parts.next()),
        };
        if self.footprint_options != options {
            self.footprint_options = options;
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
        if self.footprint_options.stats && y >= pane.plot.bottom() - footprint::table_height() {
            return None;
        }
        let step = self.footprint_step(pane);
        let price = pane.scale.price(f64::from(y));
        for bar in &self.footprint {
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
        let share = (mark.usd.abs() / biggest).clamp(0.0, 1.0) as f32;
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
                "macd" => Group::Macd,
                "atr" => Group::Atr,
                "oi" => Group::Oi,
                _ => continue,
            };
            if let Ok(w) = weight.trim().parse::<f64>() {
                self.pane_state.entry(group).or_insert_with(|| PaneState::default_for(group)).weight = w.clamp(0.1, 50.0);
            }
        }
        self.touch();
    }

    pub fn pane_weights(&self) -> String {
        self.panes.iter().map(|p| format!("{}:{:.3}", p.group.as_str(), self.weight(p.group))).collect::<Vec<_>>().join(",")
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
    pub fn reset_view(&mut self) {
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

    fn visible_studies(&self) -> impl Iterator<Item = (usize, &ComputedStudy)> {
        self.studies.iter().filter(|s| !self.hidden.contains(&s.id)).enumerate()
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
            if !groups.contains(&s.group) {
                groups.push(s.group);
            }
        }
        for g in &groups {
            self.pane_state.entry(*g).or_insert_with(|| PaneState::default_for(*g));
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
                    margins.1 = margins.1.max(f64::from(footprint::table_height() + 24.0) / f64::from(h.max(1.0)));
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
        let i0 = (from.floor() - extra).clamp(0.0, (n - 1) as f64) as usize;
        let i1 = (to.ceil() + extra).clamp(0.0, (n - 1) as f64) as usize;
        (i0, i1)
    }

    fn pane_extent(&self, group: Group, i0: usize, i1: usize) -> (f64, f64) {
        if group == Group::Pct {
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
        for (_, study) in self.visible_studies().filter(|(_, s)| s.group == group) {
            for line in &study.lines {
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
        let shift = match self.liquidations.last() {
            Some(l) if now - l.at < LIQUIDATION_ENTER => row * (1.0 - (now - l.at).max(0) as f32 / LIQUIDATION_ENTER as f32),
            _ => 0.0,
        };
        for (slot, l) in self.liquidations.iter().rev().take(LIQUIDATION_ROWS).enumerate() {
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
            let fade = (age as f32 / LIQUIDATION_ENTER as f32).min((LIQUIDATION_TTL - age) as f32 / LIQUIDATION_FADE as f32).clamp(0.0, 1.0);
            // A long liquidated is forced selling, so it takes the bear colour.
            let base = if l.long { t.bear } else { t.bull };
            let alpha = (40.0 + 200.0 * fade) as u8;
            let text = format!("{} ${}", if l.long { "LONG" } else { "SHORT" }, format_turnover(l.usd));
            let width = mark + 4.0 + text.len() as f32 * glyph;
            let left = (plot_w - width - 8.0).max(0.0);
            f.rect(left - 4.0, y - row + 1.0, width + 8.0, row - 1.0, t.background.with_alpha((190.0 * fade) as u8));
            f.icon(left + mark / 2.0, y - row / 2.0, mark, alpha, l.mark, Some(price.plot));
            f.text(left + mark + 4.0, y - row / 2.0, text, base.with_alpha(alpha), Align::Left, Baseline::Middle, size, None, Some(price.plot));
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
            .map(|b| Candle { open_time: b.time, open: b.open, high: b.high, low: b.low, close: b.close, volume: b.volume, quote_volume: None })
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
        const MARK: f32 = 34.0;
        if plot.w < 160.0 || plot.h < 110.0 {
            return;
        }
        let u = MARK / 16.0;
        let (x, y) = (plot.x + 12.0, plot.bottom() - 12.0 - MARK);
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

    fn line_color(&self, line: &tradoor_core::studies::StudyLine, study_index: usize, line_index: usize) -> Color {
        line.color.and_then(Color::parse).unwrap_or(self.theme.study_palette[(study_index + line_index) % self.theme.study_palette.len()])
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
        for pane in self.panes.iter().filter(|p| p.group == Group::Pct) {
            f.clip(Some(pane.plot));
            for level in [20.0, 80.0] {
                let y = pane.scale.y(level) as f32;
                f.dashed_line(0.0, y, plot_w, y, 1.0, t.border, 4.0, 4.0);
            }
        }

        // ---- price pane: the ribbon under the candles, volume at the foot, and
        // the mark over the volume — the bars sit where it does and would bury it
        f.clip(Some(price.plot));
        let tpo = self.tpo().is_some() && self.tpo_profiles.is_some();
        let foot = !tpo && self.footprint_drawn();
        let mut tpo_tags = Vec::new();
        if let (true, Some(profiles)) = (tpo, self.tpo_profiles.as_ref()) {
            let bars: Vec<Candle> = (0..n).filter_map(|i| self.bar(i)).map(|b| Candle { open_time: b.time, open: b.open, high: b.high, low: b.low, close: b.close, volume: b.volume, quote_volume: None }).collect();
            let edge_x = |e: f64| self.time.edge_x(e) as f32;
            let y = |p: f64| price.scale.y(p) as f32;
            let hovered = self.tpo_hovered().filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).map(|(k, _)| k);
            let view = tpo::View { plot: price.plot, column: self.time.bar_spacing as f32, char_width: self.char_width, theme: t, edge_x: &edge_x, y: &y, bars: &bars, hovered };
            tpo_tags = tpo::paint(f, &view, profiles, &self.tpo_settings);
        }
        if !tpo && !foot && !self.hidden.contains("ribbon") {
            for run in &self.ribbon {
                let pts = visible_slice(&run.points, from - 1.0, to + 1.0);
                if pts.len() < 2 {
                    continue;
                }
                let upper: Vec<(f32, f32)> = pts.iter().map(|p| (self.time.x(p.logical) as f32, price.scale.y(p.fast) as f32)).collect();
                let lower: Vec<(f32, f32)> = pts.iter().map(|p| (self.time.x(p.logical) as f32, price.scale.y(p.slow) as f32)).collect();
                let color = if run.bullish { t.ribbon_fast } else { t.ribbon_slow };
                f.band(&upper, &lower, color.with_alpha(t.ribbon_fill_alpha));
            }
        }
        let dpr = f.dpr;
        let (body_dev, wick) = candle_widths(self.time.bar_spacing, dpr);
        let body = body_dev / dpr;
        if !tpo && !foot && !self.hidden.contains("volume") {
            let max_vol = (i0..=i1).filter_map(|i| self.bar(i)).map(|b| b.volume).fold(0.0, f64::max);
            if max_vol > 0.0 {
                let (top, h) = (price.plot.y + price.plot.h * 0.82, price.plot.h * 0.18);
                for i in i0..=i1 {
                    let Some(b) = self.bar(i) else { continue };
                    let y = top + h * (1.0 - (b.volume / max_vol) as f32);
                    let base = if b.close >= b.open { t.bull } else { t.bear };
                    let x = f.snap(self.time.x(i as f64) as f32);
                    f.crisp_rect(x - body / 2.0, y, body, price.plot.bottom() - y, base.with_alpha(if b.live { 0x25 } else { t.volume_alpha }));
                }
            }
        }
        self.paint_watermark(f, price.plot);
        for i in (i0..=i1).filter(|_| !tpo && !foot) {
            let Some(b) = self.bar(i) else { continue };
            let s = &price.scale;
            let x = f.snap(self.time.x(i as f64) as f32);
            let base = if b.close >= b.open { t.bull } else { t.bear };
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
            let view = footprint::View { plot: price.plot, bar_spacing: self.time.bar_spacing as f32, char_width: self.char_width, theme: t, x: &x, y: &y };
            // Each column at the place its own time sits, so a minute the
            // server never recorded leaves a gap rather than shifting the rest.
            let placed: Vec<(f64, &footprint::Bar)> =
                self.footprint.iter().filter_map(|bar| time_to_logical(&self.times, bar.time as f64).map(|l| (l, bar))).collect();
            footprint::paint(f, &view, &placed, self.footprint_options, self.footprint_step(&price));
        }
        if !tpo && !foot && !self.hidden.contains("ema200") {
            self.paint_series(f, &price.scale, &self.engine.ema200, i0, i1, t.ema200, 1.0, LineStyle::Solid);
        }

        // ---- studies, each in its pane
        for (index, study) in self.visible_studies() {
            let Some(pane) = self.panes.iter().find(|p| p.group == study.group) else { continue };
            f.clip(Some(pane.plot));
            self.paint_study_candles(f, &pane.scale, &study.candles, i0, i1, body);
            for (li, line) in study.lines.iter().enumerate() {
                match (line.kind, line.style) {
                    (LineType::Histogram, _) => self.paint_histogram(f, &pane.scale, &line.values, i0, i1, body),
                    (LineType::Line, LineStyle::None) => {}
                    (LineType::Line, style) => self.paint_series(f, &pane.scale, &line.values, i0, i1, self.line_color(line, index, li), line.width, style),
                }
            }
            for seg in &study.segments {
                let (Some(l0), Some(l1)) = (time_to_logical(&self.times, seg.from as f64), time_to_logical(&self.times, seg.to as f64)) else { continue };
                let (x0, x1) = (self.time.x(l0) as f32, self.time.x(l1) as f32);
                if x1 < 0.0 || x0 > plot_w {
                    continue;
                }
                let y = pane.scale.y(seg.price) as f32;
                let color = Color::parse(seg.color).unwrap_or(t.text);
                f.hline(x0, x1, y, 1.0, color);
                f.text(x1 + 5.0, y - 3.0, seg.label, color, Align::Left, Baseline::Bottom, t.font_px - 1.0, None, Some(pane.plot));
            }
            for m in &study.markers {
                let Ok(i) = self.candles.binary_search_by_key(&m.time, |c| c.open_time) else { continue };
                if i < i0 || i > i1 {
                    continue;
                }
                let c = &self.candles[i];
                let x = self.time.x(i as f64) as f32;
                let color = match m.tone {
                    Tone::Bull => t.bull,
                    Tone::Bear => t.bear,
                    Tone::Neutral => t.text,
                };
                if m.above {
                    let y = pane.scale.y(c.high) as f32 - 8.0;
                    f.circle(x, y, 2.5, color);
                    f.text(x, y - 5.0, m.text.clone(), color, Align::Center, Baseline::Bottom, t.font_px - 1.0, None, Some(pane.plot));
                } else {
                    let y = pane.scale.y(c.low) as f32 + 8.0;
                    f.circle(x, y, 2.5, color);
                    f.text(x, y + 5.0, m.text.clone(), color, Align::Center, Baseline::Top, t.font_px - 1.0, None, Some(pane.plot));
                }
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
                let base = if mark.usd > 0.0 { t.bull } else { t.bear };
                let on = hovered == Some(i);
                f.circle(x, y, self.flow_radius(mark), base.with_alpha(if on { 220 } else { 90 }));
                if on {
                    f.ring(x, y, self.flow_radius(mark) + 2.0, 1.0, base);
                }
            }
        }

        // ---- ribbon crosses, last price, drawings
        f.clip(Some(price.plot));
        if !tpo && !self.hidden.contains("ribbon") {
            for cross in self.engine.crosses.iter().filter(|c| c.index >= i0 && c.index <= i1) {
                let c = &self.candles[cross.index];
                let x = self.time.x(cross.index as f64) as f32;
                if cross.bull {
                    let tip = price.scale.y(c.low) as f32 + 6.0;
                    f.triangle((x, tip), (x - 5.0, tip + 8.0), (x + 5.0, tip + 8.0), t.ribbon_fast);
                    f.text(x, tip + 10.0, "Bull", t.ribbon_fast, Align::Center, Baseline::Top, t.font_px - 1.0, None, Some(price.plot));
                } else {
                    let tip = price.scale.y(c.high) as f32 - 6.0;
                    f.triangle((x, tip), (x - 5.0, tip - 8.0), (x + 5.0, tip - 8.0), t.ribbon_slow);
                    f.text(x, tip - 10.0, "Bear", t.ribbon_slow, Align::Center, Baseline::Bottom, t.font_px - 1.0, None, Some(price.plot));
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
            drawings::paint(f, &placed, price.plot, t);
        }

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
        let hover = self.crosshair.filter(|(x, y)| *x >= 0.0 && *x < plot_w && *y >= 0.0 && *y < avail && !matches!(self.drag, Drag::Separator { .. }));
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
        if self.timeframe.source().interval.ms().is_some() || self.timeframe.source().interval == Interval::Mo1 {
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
                badges.push(Badge { pane: 0, y: price.scale.y(v) as f32, text: format_price(v, Some(p)), bg: t.ema200, fg: t.ema200.contrast_text() });
            }
        }
        for (index, study) in self.visible_studies() {
            let Some(pane_index) = self.panes.iter().position(|pn| pn.group == study.group) else { continue };
            let pane = self.panes[pane_index];
            for (li, line) in study.lines.iter().enumerate().filter(|(_, l)| l.kind == LineType::Line && l.badge) {
                if let Some(v) = last_value(&line.values) {
                    // Candles take the colour of the last one, as the price badge does.
                    let color = match study.candles.iter().rev().flatten().next() {
                        Some(b) => if b[3] >= b[0] { t.bull } else { t.bear },
                        None => self.line_color(line, index, li),
                    };
                    badges.push(Badge { pane: pane_index, y: pane.scale.y(v) as f32, text: self.format_value(study.group, v), bg: color, fg: color.contrast_text() });
                }
            }
        }
        badges.sort_by(|a, b| a.pane.cmp(&b.pane).then(a.y.total_cmp(&b.y)));
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
                Group::Pct => 2,
                _ => ticks::step_decimals(&ticks).max(2),
            };
            let label = |v: f64| if pane.group == Group::Oi { format_compact(v) } else { format_price(v, Some(decimals)) };
            for v in ticks {
                let y = pane.scale.y(v) as f32;
                let near_badge = badges.iter().any(|b| b.pane == pi && (b.y - y).abs() < badge_h);
                let near_cross = crosshair_price.is_some_and(|(i, cy)| i == pi && (cy - y).abs() < badge_h);
                if y < pane.plot.y + 6.0 || y > pane.plot.bottom() - 6.0 || near_badge || near_cross {
                    continue;
                }
                f.text(plot_w + 6.0, y, label(v), t.text, Align::Left, Baseline::Middle, font, None, Some(axis));
            }
            if pane.group == Group::Pct {
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
            let axis = Rect { x: plot_w, y: pane.plot.y, w: self.axis_width, h: pane.plot.h };
            f.crisp_rect(plot_w + 1.0, badge.y - badge_h / 2.0, self.axis_width - 1.0, badge_h, badge.bg);
            f.text(plot_w + 6.0, badge.y, badge.text.clone(), badge.fg, Align::Left, Baseline::Middle, font, None, Some(axis));
        }
        if let Some((pi, cy)) = crosshair_price {
            let pane = self.panes[pi];
            f.crisp_rect(plot_w + 1.0, cy - badge_h / 2.0, self.axis_width - 1.0, badge_h, t.label_background);
            f.text(plot_w + 6.0, cy, self.format_value(pane.group, pane.scale.price(f64::from(cy))), t.countdown_text, Align::Left, Baseline::Middle, font, None, None);
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
            let left = (x - w / 2.0).clamp(0.0, (plot_w - w).max(0.0));
            f.crisp_rect(left, avail + 1.0, w, TIME_AXIS_HEIGHT - 2.0, t.label_background);
            f.text(left + w / 2.0, avail + TIME_AXIS_HEIGHT / 2.0, text, t.countdown_text, Align::Center, Baseline::Middle, font, None, None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_series(&self, f: &mut Frame, scale: &PriceScale, values: &[Option<f64>], i0: usize, i1: usize, color: Color, width: f32, style: LineStyle) {
        if values.is_empty() {
            return;
        }
        let end = i1.min(values.len() - 1);
        let mut pts: Vec<(f32, f32)> = Vec::new();
        for i in i0..=end {
            match values[i] {
                Some(v) => {
                    let (x, y) = (self.time.x(i as f64) as f32, scale.y(v) as f32);
                    if let (LineStyle::Step, Some(&(_, py))) = (style, pts.last()) {
                        pts.push((x, py));
                    }
                    pts.push((x, y));
                }
                None => {
                    f.polyline(&pts, width, color);
                    pts.clear();
                }
            }
        }
        f.polyline(&pts, width, color);
    }

    /// A study's own candles — open interest — in its pane, coloured by whether it rose.
    fn paint_study_candles(&self, f: &mut Frame, scale: &PriceScale, bars: &[Option<[f64; 4]>], i0: usize, i1: usize, body: f32) {
        let t = &self.theme;
        let wick = (1.0 / f.dpr).max(1.0);
        // Most studies have no candles: an empty list must not be indexed at all.
        for (i, bar) in bars.iter().enumerate().take(i1.saturating_add(1)).skip(i0) {
            let Some([o, h, l, c]) = *bar else { continue };
            let x = f.snap(self.time.x(i as f64) as f32);
            let color = if c >= o { t.bull } else { t.bear };
            let (yh, yl, yo, yc) = (scale.y(h) as f32, scale.y(l) as f32, scale.y(o) as f32, scale.y(c) as f32);
            let (top, bottom) = (yo.min(yc), yo.max(yc));
            f.vline(x, yh, yl, wick, color);
            if body >= 2.0 {
                f.crisp_rect(x - body / 2.0, top, body, (bottom - top).max(1.0 / f.dpr), color);
            }
        }
    }

    fn paint_histogram(&self, f: &mut Frame, scale: &PriceScale, values: &[Option<f64>], i0: usize, i1: usize, body: f32) {
        let zero = scale.y(0.0) as f32;
        let w = (body * 0.7).max(1.0 / f.dpr);
        for i in i0..=i1.min(values.len().saturating_sub(1)) {
            let Some(v) = values[i] else { continue };
            let y = scale.y(v) as f32;
            let x = f.snap(self.time.x(i as f64) as f32);
            let color = if v >= 0.0 { self.theme.bull } else { self.theme.bear };
            f.crisp_rect(x - w / 2.0, y.min(zero), w, (y - zero).abs().max(1.0 / f.dpr), color.with_alpha(0x80));
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

    fn place<'a>(&self, shape: &'a DrawingShape, draft: bool) -> Option<Placed<'a>> {
        let pane = self.panes.first()?;
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
        Some(Placed { shape, points, selected: !draft && shape.id.is_some() && shape.id == self.selected, draft, hovered: false, bars, scale: pane.scale, precision: self.precision(), last })
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
                s.lines.iter().map(move |l| {
                    // A series that runs into the forming bar is read there.
                    let i = if l.values.len() > self.candles.len() { index } else { at };
                    (format!("{}:{}", s.id, l.key), l.values.get(i).copied().flatten().map_or("—".to_string(), |v| self.format_value(s.group, v)))
                })
            })
            .collect();
        Legend {
            index: Some(index),
            time: b.time,
            ohlc: format!("O{}  H{}  L{}  C{}", fp(b.open), fp(b.high), fp(b.low), fp(b.close)),
            change: format!("{sign}{} ({sign}{pct:.2}%)", fp(diff)),
            up: diff >= 0.0,
            volume: format!("Vol {}", format_turnover(b.volume)),
            values,
            cursor: self
                .crosshair
                .filter(|(x, y)| matches!(self.drag, Drag::None) && *x >= 0.0 && *x < pane.plot.w && *y >= pane.plot.y && *y <= pane.plot.bottom())
                .map(|(_, y)| (y, pane.scale.price(f64::from(y)))),
            axis_x: pane.plot.w,
            alert: self.crosshair.filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).and_then(|(x, y)| self.alert_handle_at(x, y)),
            flow: self.crosshair.filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).and_then(|(x, y)| self.flow_at(x, y)),
            footprint: self.crosshair.filter(|_| self.tool.is_none() && matches!(self.drag, Drag::None)).and_then(|(x, y)| self.footprint_at(x, y)),
            tpo: self
                .tpo_hovered()
                .filter(|_| self.tool.is_none())
                .and_then(|(k, bar)| tpo::hover(self.tpo_profiles.as_ref()?, &self.tpo_settings, k, self.bar(bar).map(|b| (bar, b.time)))),
        }
    }

    /// A study's figure as its pane labels it: the instrument's decimals for
    /// price, two for the rest, and `5.327M` for open interest.
    fn format_value(&self, group: Group, v: f64) -> String {
        match group {
            Group::Price => format_price(v, Some(self.precision())),
            Group::Oi => format_compact(v),
            _ => format_price(v, Some(2)),
        }
    }

    pub fn studies_meta(&self) -> Vec<StudyMeta> {
        let mut visible = 0usize;
        self.studies
            .iter()
            .map(|s| {
                let hidden = self.hidden.contains(&s.id);
                let index = visible;
                if !hidden {
                    visible += 1;
                }
                StudyMeta {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    group: s.group.as_str(),
                    hidden,
                    lines: s.lines.iter().enumerate().map(|(li, l)| (format!("{}:{}", s.id, l.key), l.label.clone(), self.line_color(l, index, li))).collect(),
                    stats: s.stats.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
                }
            })
            .collect()
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
        let pane = self.panes.first()?;
        let time = logical_to_time(&self.times, self.time.logical(f64::from(x)))?;
        let price = pane.scale.price(f64::from(y));
        (time.is_finite() && price.is_finite()).then_some(Anchor { time, price })
    }

    /// An anchor where the pointer is, pulled by the magnet when it is on.
    fn point_at(&self, x: f32, y: f32) -> Option<Anchor> {
        let raw = self.anchor_at(x, y)?;
        let magnet = self.magnet_now();
        if magnet == Magnet::Off {
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
        let Some(pane) = self.panes.first().copied() else { return };
        let Some(index) = self.drawings.iter().position(|d| d.id == Some(id)) else { return };
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
        let plot = self.panes.first()?.plot;
        for d in &self.drawings {
            let Some(placed) = self.place(d, false) else { continue };
            if placed.shape.style.locked {
                continue;
            }
            let distance = drawings::distance(&placed, (x, y), plot);
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
            Drag::Separator { .. } => Cursor::RowResize,
            _ => match self.zone(x, y) {
                Zone::Separator(_) => Cursor::RowResize,
                Zone::PriceAxis(0) if self.alert_badge_at(x, y).is_some_and(|id| self.selected_alert != Some(id)) && !self.touch_input => Cursor::Pointer,
                Zone::PriceAxis(_) => Cursor::NsResize,
                Zone::TimeAxis => Cursor::ColResize,
                Zone::Plot(0) if self.handle_at(x, y).is_some() => Cursor::Pointer,
                Zone::Plot(0) if let Some((id, _)) = self.alert_at(x, y) => {
                    if self.selected_alert == Some(id) {
                        Cursor::NsResize
                    } else {
                        Cursor::Pointer
                    }
                }
                Zone::Plot(0) => match self.drawing_at(x, y) {
                    Some(id) if self.selected == Some(id) => Cursor::Move,
                    Some(_) => Cursor::Pointer,
                    None => Cursor::Crosshair,
                },
                Zone::Plot(_) => Cursor::Crosshair,
                Zone::Outside => Cursor::Default,
            },
        }
    }

    fn draft_with(&mut self, kind: DrawingKind, extra: Option<Anchor>) {
        let mut points = self.placed.clone();
        points.extend(extra.map(|a| self.constrain(kind, &self.placed, a)));
        self.draft = Some(DrawingShape { style: self.tool_style.clone(), ..DrawingShape::new(None, kind, points) });
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.crosshair = Some((x, y));
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
            Drag::Held => {}
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
                        self.draft = Some(DrawingShape { style: self.tool_style.clone(), ..DrawingShape::new(None, kind, points) });
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
        let free = self.tool.is_none();
        // A finger passing over an alert on its way to scale or pan is not
        // taking hold of it: on a touch screen the alert waits for a hold.
        let grab_alerts = !self.touch_input;
        self.drag = match self.zone(x, y) {
            Zone::Separator(i) if free => Drag::Separator { upper: i, last_y: y },
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
            Zone::Plot(0) if self.tool == Some(DrawingKind::Brush) => match self.anchor_at(x, y) {
                Some(a) => {
                    self.placed = vec![a];
                    self.draft_with(DrawingKind::Brush, None);
                    Drag::Brush { last: (x, y) }
                }
                None => Drag::None,
            },
            // A handle of the selected drawing reshapes it and anywhere else on
            // it moves it. Any other drawing is only picked by a click: a drag
            // that starts on it pans, as on bare chart, since a position's box
            // or a fib's levels cover so much of the screen that the chart
            // could hardly be moved past them without moving them. An alert's
            // line likewise: picked by a click, and only then dragged — a line
            // across the chart is easily caught by a press meant for the chart.
            Zone::Plot(0) if free => {
                if let Some((id, index)) = self.handle_at(x, y) {
                    Drag::Anchor { id, index, moved: false }
                } else if let Some((id, _)) = self.alert_at(x, y).filter(|(id, _)| grab_alerts && self.selected_alert == Some(*id)) {
                    Drag::Alert { id, moved: false }
                } else if let Some(id) = self.drawing_at(x, y).filter(|id| self.selected == Some(*id)) {
                    Drag::Shape { id, last: (x, y), moved: false }
                } else {
                    Drag::Press { start: (x, y), last_x: x, last_y: y, pane: 0, panning: false, price_pan: false }
                }
            }
            // With a tool armed, a drag places its anchors in one gesture.
            Zone::Plot(0) if !matches!(self.tool, Some(DrawingKind::Text | DrawingKind::Path | DrawingKind::Long | DrawingKind::Short)) => match (self.tool, self.point_at(x, y)) {
                (Some(kind), Some(start)) => Drag::Draw { kind, start, start_px: (x, y), moved: false },
                _ => Drag::None,
            },
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
            Drag::Press { start, panning: false, .. } if (x - start.0).hypot(y - start.1) <= CLICK_SLOP => self.click(x, y),
            Drag::Held if self.tool.is_some() => self.click(x, y),
            Drag::Brush { .. } => {
                let points = std::mem::take(&mut self.placed);
                self.draft = None;
                // A press with no movement is a dot, not a stroke.
                if points.len() >= DrawingKind::Brush.arity() {
                    self.events.push(Event::DrawingCreated { kind: DrawingKind::Brush, points });
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
                    self.click(x, y);
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
                        self.events.push(Event::DrawingCreated { kind, points });
                        self.finish_tool();
                    } else {
                        self.placed = points;
                        self.draft_with(kind, Some(end));
                    }
                }
            }
            Drag::Separator { .. } => self.events.push(Event::PaneWeightsChanged(self.pane_weights())),
            _ => {}
        }
        self.cursor = self.cursor_for(x, y);
        self.touch();
    }

    pub fn pointer_leave(&mut self) {
        self.crosshair = None;
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

    fn finish_tool(&mut self) {
        self.tool = None;
        self.placed.clear();
        self.draft = None;
        self.events.push(Event::ToolFinished);
    }

    fn click(&mut self, x: f32, y: f32) {
        let on_price = self.zone(x, y) == Zone::Plot(0);
        match self.tool {
            None => match self.alert_at(x, y).filter(|_| on_price) {
                Some((id, _)) => self.pick_alert(Some(id)),
                None => {
                    self.pick_alert(None);
                    self.select(if on_price { self.drawing_at(x, y) } else { None });
                }
            },
            Some(DrawingKind::Brush) => {}
            Some(tool) => {
                let Some(anchor) = self.point_at(x, y).filter(|_| on_price) else { return };
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
                self.events.push(Event::DrawingCreated { kind: tool, points });
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
        self.events.push(Event::DrawingCreated { kind: tool, points });
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
        self.events.push(Event::DrawingCreated { kind: DrawingKind::Path, points });
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
        let state = self.pane_state.entry(group).or_insert_with(|| PaneState::default_for(group));
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
        if self.time.bars > 0 && (0.0..=plot_w).contains(&newest) { newest } else { f64::from(pointer_x).clamp(0.0, plot_w) }
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
            self.pane_state.entry(group).or_insert_with(|| PaneState::default_for(group)).weight = f64::from(h) / avail * total;
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
                Candle { open_time: 1_700_000_000_000 - 1_700_000_000_000 % HOUR + i as i64 * HOUR, open: p - 0.5, high: p + 1.5, low: p - 2.0, close: p + 0.5, volume: 10.0 + i as f64, quote_volume: None }
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
        let Event::DrawingCreated { kind, points } = &events[0] else { panic!("{events:?}") };
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
        let Some(Event::DrawingCreated { kind, points }) = events.first().cloned() else { panic!("{events:?}") };
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
        let Some(Event::DrawingCreated { kind, points }) = events.first().cloned() else { panic!("{events:?}") };
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
            Event::DrawingCreated { kind: DrawingKind::Trendline, points } => Some(points),
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
        assert!(matches!(&c.take_events()[0], Event::DrawingCreated { kind: DrawingKind::Path, points } if points.len() == 3));

        c.set_tool(Some(DrawingKind::Brush));
        c.pointer_down(100.0, 100.0, 0);
        for i in 1..20 {
            c.pointer_move(100.0 + i as f32 * 5.0, 100.0 + i as f32);
        }
        c.pointer_up(200.0, 120.0);
        assert!(matches!(&c.take_events()[0], Event::DrawingCreated { kind: DrawingKind::Brush, points } if points.len() > 10));
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
}
