//! TPO profiles drawn in place of candles, laid out and named as MMT has them.
//!
//! The chart's own candles are the brackets, and a session — a UTC day, week or
//! month, Asia, London or New York, or sessions of one's own — is how long a
//! profile runs. Profiles stand side by side, each as wide as its widest row
//! (as its brackets, expanded), and the chart's time scale is warped to match.
//! Everything a profile is read by is worked out once, when a bar changes: rows,
//! value area, volume and who took it at each row, initial balance, single
//! prints, poor highs and lows, and how long each level has stayed untouched
//! ("naked"). Painting only places it.

use tradoor_core::format::format_turnover;
use tradoor_core::indicators::profile::value_area;
use tradoor_core::indicators::tpo::{tpo_session, TpoOptions, TpoSession};
use tradoor_core::time::{civil, utc, weekday, DAY, HOUR, MINUTE};
use tradoor_core::{Candle, ChartTimeframe};

use crate::color::Color;
use crate::footprint;
use crate::paint::{Align, Baseline, Frame, Rect};
use crate::theme::Theme;
use crate::ticks::nice_step;

/// Rows a median day is cut into on auto row size: about MMT's 500 ticks on BTC.
const AUTO_DAY_ROWS: f64 = 40.0;
/// Air between neighbouring blocks, as a share of a block's slot: a sliver, so
/// a row reads as a run of brackets rather than a dotted line.
const BLOCK_GAP: f32 = 0.12;
/// The most air between blocks or rows, in CSS pixels, however far in the zoom.
const MAX_GAP: f32 = 2.0;
/// Columns a bar outside every session takes: next to none.
const OUTSIDE_COLUMNS: f64 = 0.001;
/// How dark a block's first bracket is next to its last, by what the block is.
const DIM_OUTSIDE: f32 = 0.3;
const DIM_VALUE: f32 = 0.6;
const DIM_POC: f32 = 0.72;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// How long a profile runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
    /// Tokyo's day, 00:00–08:00 UTC.
    Asia,
    /// 08:00–16:30 London time.
    London,
    /// 09:30–16:00 New York time.
    NewYork,
    /// Each day as Asia, London and New York, one after the other.
    Intraday,
    /// The sessions in the settings, each day.
    Custom,
}

impl Period {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "1d" => Self::Day,
            "1w" => Self::Week,
            "1M" => Self::Month,
            "asia" => Self::Asia,
            "london" => Self::London,
            "ny" => Self::NewYork,
            "intraday" => Self::Intraday,
            "custom" => Self::Custom,
            _ => return None,
        })
    }

    /// Whether a chart of these bars can stack them: a profile needs brackets
    /// shorter than itself, and a session is hours long.
    pub fn fits(self, timeframe: ChartTimeframe) -> bool {
        match self {
            Self::Day => timeframe < ChartTimeframe::D1,
            Self::Week => timeframe < ChartTimeframe::W1,
            Self::Month => timeframe < ChartTimeframe::Mo1,
            _ => timeframe.source().interval.ms().is_some_and(|ms| ms * i64::from(timeframe.source().group) <= HOUR),
        }
    }

    fn fixed(self) -> Option<ChartTimeframe> {
        match self {
            Self::Day => Some(ChartTimeframe::D1),
            Self::Week => Some(ChartTimeframe::W1),
            Self::Month => Some(ChartTimeframe::Mo1),
            _ => None,
        }
    }
}

/// A session of one's own, in minutes past midnight UTC; an end at or before
/// its start runs into the next day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Session {
    pub start: i64,
    pub end: i64,
    pub color: Color,
}

impl Session {
    /// `0000-0600#e3cd2a,0600-1200#c8202e`; a part that does not read is skipped.
    pub fn parse_list(raw: &str) -> Vec<Self> {
        raw.split(',')
            .filter_map(|part| {
                let (times, color) = part.trim().split_once('#')?;
                let (a, b) = times.split_once('-')?;
                let minutes = |s: &str| -> Option<i64> {
                    let s = s.replace(':', "");
                    let (h, m) = (s.get(..2)?.parse::<i64>().ok()?, s.get(2..4)?.parse::<i64>().ok()?);
                    (s.len() == 4 && h <= 24 && m < 60 && h * 60 + m <= 1440).then_some(h * 60 + m)
                };
                Some(Self { start: minutes(a)?, end: minutes(b)?, color: Color::parse(&format!("#{color}"))? })
            })
            .collect()
    }
}

/// The `n`th Sunday of a month (`0`: the last), at midnight UTC.
fn sunday(year: i64, month0: i64, n: i64) -> i64 {
    if n == 0 {
        let last = utc(year, month0 + 1, 1) - DAY;
        return last - i64::from(weekday(last)) * DAY;
    }
    let first = utc(year, month0, 1);
    first + ((7 - i64::from(weekday(first))) % 7 + 7 * (n - 1)) * DAY
}

/// Whether London keeps summer time on the day starting at `day`.
fn london_summer(day: i64) -> bool {
    let year = civil(day).year;
    day >= sunday(year, 2, 0) && day < sunday(year, 9, 0)
}

/// Whether New York keeps summer time on the day starting at `day`.
fn new_york_summer(day: i64) -> bool {
    let year = civil(day).year;
    day >= sunday(year, 2, 2) && day < sunday(year, 10, 1)
}

/// London's open and close on a day, in minutes past midnight UTC.
fn london(day: i64) -> (i64, i64) {
    let shift = if london_summer(day) { 60 } else { 0 };
    (8 * 60 - shift, 16 * 60 + 30 - shift)
}

/// New York's open and close on a day, in minutes past midnight UTC.
fn new_york(day: i64) -> (i64, i64) {
    let behind = if new_york_summer(day) { 4 * 60 } else { 5 * 60 };
    (9 * 60 + 30 + behind, 16 * 60 + behind)
}

const ASIA: Color = Color { r: 0xe3, g: 0xcd, b: 0x2a, a: 255 };
const LONDON: Color = Color { r: 0xc8, g: 0x20, b: 0x2e, a: 255 };
const NEW_YORK: Color = Color { r: 0x2f, g: 0x5b, b: 0xd8, a: 255 };

/// A stretch of time one profile is built from.
#[derive(Clone, Debug, PartialEq)]
struct Window {
    start: i64,
    end: i64,
    name: &'static str,
    color: Option<Color>,
}

/// Every session window from the one holding `from` to the one holding `to`.
fn windows(period: Period, sessions: &[Session], from: i64, to: i64) -> Vec<Window> {
    let mut out = Vec::new();
    if let Some(tf) = period.fixed() {
        let mut start = tf.align(from);
        while start <= to {
            let end = tf.end(start);
            out.push(Window { start, end, name: "", color: None });
            start = end;
        }
        return out;
    }
    // A session can run past midnight, so the day before counts too.
    let mut day = from.div_euclid(DAY) * DAY - DAY;
    while day <= to {
        let mut push = |a: i64, b: i64, name: &'static str, color: Option<Color>| {
            if b > a {
                out.push(Window { start: day + a * MINUTE, end: day + b * MINUTE, name, color });
            }
        };
        let (lo, lc) = london(day);
        let (no, nc) = new_york(day);
        match period {
            Period::Asia => push(0, 8 * 60, "Asia", Some(ASIA)),
            Period::London => push(lo, lc, "London", Some(LONDON)),
            Period::NewYork => push(no, nc, "New York", Some(NEW_YORK)),
            Period::Intraday => {
                push(0, lo, "Asia", Some(ASIA));
                push(lo, no, "London", Some(LONDON));
                push(no, 24 * 60, "New York", Some(NEW_YORK));
            }
            _ => {
                for s in sessions {
                    let end = if s.end <= s.start { s.end + 24 * 60 } else { s.end };
                    push(s.start, end, "", Some(s.color));
                }
            }
        }
        day += DAY;
    }
    out.sort_by_key(|w| w.start);
    // Overlapping sessions: a bar goes to the one that opened first.
    for i in 1..out.len() {
        out[i].start = out[i].start.max(out[i - 1].end.min(out[i].end));
    }
    out.retain(|w| w.end > w.start);
    out
}

/// What was done to profiles by hand: which are drawn expanded, which were
/// joined to the one before, and where any was cut in two. Profiles are known
/// by their start; `key=t,t;key=t` as the page keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edits {
    pub expanded: Vec<i64>,
    pub merged: Vec<i64>,
    pub splits: Vec<i64>,
}

impl Edits {
    pub fn parse(raw: &str) -> Self {
        let mut e = Self::default();
        for pair in raw.split(';') {
            let Some((key, list)) = pair.split_once('=') else { continue };
            let times = list.split(',').filter_map(|t| t.trim().parse::<i64>().ok());
            match key.trim() {
                "e" => e.expanded.extend(times),
                "m" => e.merged.extend(times),
                "s" => e.splits.extend(times),
                _ => {}
            }
        }
        e
    }
}

/// One profile's windows, after merges and splits.
#[derive(Clone, Debug)]
struct Piece {
    ranges: Vec<(i64, i64)>,
    name: &'static str,
    color: Option<Color>,
    merged: bool,
}

fn pieces(windows: Vec<Window>, edits: &Edits) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for w in windows {
        let mut cuts: Vec<i64> = edits.splits.iter().copied().filter(|t| *t > w.start && *t < w.end).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut start = w.start;
        for end in cuts.into_iter().chain([w.end]) {
            match out.last_mut() {
                Some(prev) if edits.merged.contains(&start) => {
                    prev.ranges.push((start, end));
                    prev.merged = true;
                }
                _ => out.push(Piece { ranges: vec![(start, end)], name: w.name, color: w.color, merged: false }),
            }
            start = end;
        }
    }
    out
}

/// The options, as MMT's TPO settings name them. The page sends only what it
/// changed, as `key=value;key=value`.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    // General
    /// Row size as ticks × this; 0 picks one from the loaded days.
    pub row_ticks: u32,
    /// Blocks outside the value area.
    pub profile_color: Option<Color>,
    pub block_mode: BlockMode,
    pub heat_low: Color,
    pub heat_high: Color,
    /// The first bracket's block at the open, the last one's at the close.
    pub highlight: bool,
    pub start_color: Option<Color>,
    pub end_color: Option<Color>,
    pub hide_text: bool,
    /// Columns between neighbouring profiles.
    pub spacing: u32,
    // Session
    pub sessions: Vec<Session>,
    pub session_header: bool,
    // Point of control
    pub poc: bool,
    pub poc_width: f32,
    pub poc_color: Option<Color>,
    pub poc_ray_color: Option<Color>,
    pub naked_poc: bool,
    // Value area
    pub va_pct: f64,
    pub va_color: Option<Color>,
    pub va_rays: bool,
    pub va_ray_color: Option<Color>,
    pub naked_va: bool,
    pub fill_va: bool,
    pub fill_va_color: Option<Color>,
    // Single prints
    pub singles: bool,
    pub single_color: Option<Color>,
    pub single_ray_color: Option<Color>,
    pub naked_singles: bool,
    // Poor high, poor low
    pub poor: bool,
    pub poor_color: Option<Color>,
    pub naked_poor: bool,
    // Initial balance
    pub ib: bool,
    pub ib_color: Option<Color>,
    pub ib_candles: usize,
    // Volume profile
    pub vp: bool,
    pub vp_buy: Option<Color>,
    pub vp_sell: Option<Color>,
    pub vp_color: Option<Color>,
    pub vp_mode: VpMode,
    pub vp_right: bool,
    /// The longest row, as a share of the profile's widest.
    pub vp_width: f64,
    pub vp_va_rays: bool,
    pub vp_va_color: Option<Color>,
    pub vp_naked_va: bool,
    pub vp_va_pct: f64,
    pub vp_labels: bool,
    pub vp_label_color: Option<Color>,
    pub vp_poc: bool,
    pub vp_poc_color: Option<Color>,
    pub vp_naked_poc: bool,
    pub vp_hide_text: bool,
    /// Rows under this share of the busiest row, in percent, are not drawn.
    pub vp_bin_filter: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockMode {
    /// A block brighter the later its bracket printed.
    Brightness,
    Solid,
    /// Rows shaded by how many brackets traded there.
    HeatMap,
    /// Each bracket green or red by its candle.
    Candle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpMode {
    /// Bought and sold, side by side.
    Standard,
    Total,
    /// Bought less sold.
    Delta,
}

pub const DEFAULT_SESSIONS: &str = "0000-0600#e3cd2a,0600-1200#c8202e,1200-2000#2f5bd8,2000-0000#2aa12a";

impl Default for Settings {
    fn default() -> Self {
        Self {
            row_ticks: 0,
            profile_color: None,
            block_mode: BlockMode::Brightness,
            heat_low: Color::hex(0xd32f2f),
            heat_high: Color::hex(0xffd54f),
            highlight: true,
            start_color: None,
            end_color: None,
            hide_text: false,
            spacing: 5,
            sessions: Session::parse_list(DEFAULT_SESSIONS),
            session_header: false,
            poc: true,
            poc_width: 2.0,
            poc_color: None,
            poc_ray_color: None,
            naked_poc: false,
            va_pct: 70.0,
            va_color: None,
            va_rays: true,
            va_ray_color: None,
            naked_va: false,
            fill_va: false,
            fill_va_color: None,
            singles: true,
            single_color: None,
            single_ray_color: None,
            naked_singles: false,
            poor: true,
            poor_color: None,
            naked_poor: false,
            ib: false,
            ib_color: None,
            ib_candles: 2,
            vp: false,
            vp_buy: None,
            vp_sell: None,
            vp_color: None,
            vp_mode: VpMode::Standard,
            vp_right: true,
            vp_width: 1.0,
            vp_va_rays: false,
            vp_va_color: None,
            vp_naked_va: false,
            vp_va_pct: 70.0,
            vp_labels: true,
            vp_label_color: None,
            vp_poc: false,
            vp_poc_color: None,
            vp_naked_poc: false,
            vp_hide_text: false,
            vp_bin_filter: 0.0,
        }
    }
}

impl Settings {
    /// `key=value;key=value`, over the defaults. An unknown key or a value that
    /// does not parse is skipped, never an error: a setting from a newer page
    /// must not break an older engine.
    pub fn parse(raw: &str) -> Self {
        let mut s = Self::default();
        for pair in raw.split(';') {
            let Some((key, value)) = pair.split_once('=') else { continue };
            s.set(key.trim(), value.trim());
        }
        s
    }

    fn set(&mut self, key: &str, v: &str) {
        let flag = || match v {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        };
        let color = || Color::parse(v);
        let count = |lo: usize, hi: usize| v.parse::<usize>().ok().filter(|n| (lo..=hi).contains(n));
        let number = |lo: f64, hi: f64| v.parse::<f64>().ok().filter(|n| (lo..=hi).contains(n));
        macro_rules! apply {
            ($slot:expr, $value:expr) => {
                if let Some(x) = $value {
                    $slot = x;
                }
            };
        }
        match key {
            "rowTicks" => apply!(self.row_ticks, v.parse::<u32>().ok().filter(|n| *n <= 1_000_000)),
            "profileColor" => apply!(self.profile_color, color().map(Some)),
            "blockMode" => apply!(
                self.block_mode,
                match v {
                    "brightness" => Some(BlockMode::Brightness),
                    "solid" => Some(BlockMode::Solid),
                    "heatmap" => Some(BlockMode::HeatMap),
                    "candle" => Some(BlockMode::Candle),
                    _ => None,
                }
            ),
            "heatLow" => apply!(self.heat_low, color()),
            "heatHigh" => apply!(self.heat_high, color()),
            "highlight" => apply!(self.highlight, flag()),
            "startColor" => apply!(self.start_color, color().map(Some)),
            "endColor" => apply!(self.end_color, color().map(Some)),
            "hideText" => apply!(self.hide_text, flag()),
            "spacing" => apply!(self.spacing, v.parse::<u32>().ok().filter(|n| *n <= 50)),
            "sessions" => self.sessions = Session::parse_list(v),
            "sessionHeader" => apply!(self.session_header, flag()),
            "poc" => apply!(self.poc, flag()),
            "pocWidth" => apply!(self.poc_width, v.parse::<f32>().ok().filter(|n| (1.0..=6.0).contains(n))),
            "pocColor" => apply!(self.poc_color, color().map(Some)),
            "pocRayColor" => apply!(self.poc_ray_color, color().map(Some)),
            "nakedPoc" => apply!(self.naked_poc, flag()),
            "vaPct" => apply!(self.va_pct, number(1.0, 100.0)),
            "vaColor" => apply!(self.va_color, color().map(Some)),
            "vaRays" => apply!(self.va_rays, flag()),
            "vaRayColor" => apply!(self.va_ray_color, color().map(Some)),
            "nakedVa" => apply!(self.naked_va, flag()),
            "fillVa" => apply!(self.fill_va, flag()),
            "fillVaColor" => apply!(self.fill_va_color, color().map(Some)),
            "singlePrints" => apply!(self.singles, flag()),
            "singlePrintColor" => apply!(self.single_color, color().map(Some)),
            "singlePrintRayColor" => apply!(self.single_ray_color, color().map(Some)),
            "nakedSinglePrints" => apply!(self.naked_singles, flag()),
            "poor" => apply!(self.poor, flag()),
            "poorColor" => apply!(self.poor_color, color().map(Some)),
            "nakedPoor" => apply!(self.naked_poor, flag()),
            "ib" => apply!(self.ib, flag()),
            "ibColor" => apply!(self.ib_color, color().map(Some)),
            "ibCandles" => apply!(self.ib_candles, count(1, 100)),
            "vp" => apply!(self.vp, flag()),
            "vpBuy" => apply!(self.vp_buy, color().map(Some)),
            "vpSell" => apply!(self.vp_sell, color().map(Some)),
            "vpColor" => apply!(self.vp_color, color().map(Some)),
            "vpMode" => apply!(
                self.vp_mode,
                match v {
                    "standard" => Some(VpMode::Standard),
                    "total" => Some(VpMode::Total),
                    "delta" => Some(VpMode::Delta),
                    _ => None,
                }
            ),
            "vpOrigin" => apply!(
                self.vp_right,
                match v {
                    "right" => Some(true),
                    "left" => Some(false),
                    _ => None,
                }
            ),
            "vpWidth" => apply!(self.vp_width, number(0.1, 1.0)),
            "vpVaRays" => apply!(self.vp_va_rays, flag()),
            "vpVaColor" => apply!(self.vp_va_color, color().map(Some)),
            "vpNakedVa" => apply!(self.vp_naked_va, flag()),
            "vpVaPct" => apply!(self.vp_va_pct, number(1.0, 100.0)),
            "vpLabels" => apply!(self.vp_labels, flag()),
            "vpLabelColor" => apply!(self.vp_label_color, color().map(Some)),
            "vpPoc" => apply!(self.vp_poc, flag()),
            "vpPocColor" => apply!(self.vp_poc_color, color().map(Some)),
            "vpNakedPoc" => apply!(self.vp_naked_poc, flag()),
            "vpHideText" => apply!(self.vp_hide_text, flag()),
            "vpBinFilter" => apply!(self.vp_bin_filter, number(0.0, 100.0)),
            _ => {}
        }
    }

    /// What the profiles themselves depend on; colours and lines do not rebuild them.
    fn shape(&self) -> Shape {
        Shape {
            row_ticks: self.row_ticks,
            va_pct: self.va_pct.to_bits(),
            ib_candles: self.ib_candles,
            spacing: self.spacing,
            vp: (self.vp, self.vp_right, self.vp_width.to_bits(), self.vp_va_pct.to_bits()),
            sessions: self.sessions.iter().map(|s| (s.start, s.end, s.color.r, s.color.g, s.color.b)).collect(),
        }
    }
}

/// The colours a profile is drawn in: each one set by hand, or worked out from
/// the theme's three TPO colours and its candles'.
struct Colors {
    outside: Color,
    start: Color,
    end: Color,
    poc: Color,
    poc_ray: Color,
    va: Color,
    va_ray: Color,
    fill_va: Color,
    single: Color,
    single_ray: Color,
    poor: Color,
    ib: Color,
    vp_buy: Color,
    vp_sell: Color,
    vp: Color,
    vp_va: Color,
    vp_label: Color,
    vp_poc: Color,
}

impl Settings {
    fn colors(&self, t: &Theme) -> Colors {
        let or = |set: Option<Color>, theme: Color| set.unwrap_or(theme);
        Colors {
            outside: or(self.profile_color, t.tpo_outside),
            start: or(self.start_color, t.bull),
            end: or(self.end_color, t.bear),
            poc: or(self.poc_color, t.tpo_poc),
            poc_ray: or(self.poc_ray_color, t.tpo_poc),
            va: or(self.va_color, t.tpo),
            va_ray: or(self.va_ray_color, t.tpo),
            fill_va: or(self.fill_va_color, t.tpo.with_alpha(0x22)),
            // Single prints' own blocks sunk most of the way into the background.
            single: or(self.single_color, mix(t.background, t.tpo, 0.22)),
            single_ray: or(self.single_ray_color, t.tpo.with_alpha(0x5c)),
            poor: or(self.poor_color, t.tpo_poc),
            ib: or(self.ib_color, t.tpo_poc),
            vp_buy: or(self.vp_buy, t.bull),
            vp_sell: or(self.vp_sell, t.bear),
            // The value area's colour, see-through, so the volume reads apart from the blocks.
            vp: or(self.vp_color, t.tpo.with_alpha(0x8c)),
            vp_va: or(self.vp_va_color, t.tpo_outside),
            vp_label: or(self.vp_label_color, t.tpo_poc),
            vp_poc: or(self.vp_poc_color, t.tpo_outside),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Shape {
    row_ticks: u32,
    va_pct: u64,
    ib_candles: usize,
    spacing: u32,
    vp: (bool, bool, u64, u64),
    sessions: Vec<(i64, i64, u8, u8, u8)>,
}

/// A band of rows, bottom and top row index inclusive.
pub type Band = (i64, i64);

/// One profile and everything read off it.
pub struct Profile {
    pub session: TpoSession,
    /// The span of time it covers, first window's start to last one's end.
    pub start: i64,
    pub end: i64,
    /// What its session is called and coloured, when it is one of a day's.
    pub name: &'static str,
    pub color: Option<Color>,
    /// Joined from more than one window by hand.
    pub merged: bool,
    pub expanded: bool,
    /// Bar index of its first and last bracket.
    pub first: usize,
    pub last: usize,
    /// Each bracket's bar, counted from the first.
    pub offsets: Vec<usize>,
    /// Columns it stands in, the space after it included.
    pub columns: f64,
    /// Columns its blocks take.
    pub block_columns: usize,
    /// Its widest row's brackets.
    pub widest: usize,
    /// Volume at each of `session.rows`, spread over the rows each candle spans.
    pub volume: Vec<f64>,
    /// Bought and sold at each row, when every bracket's trades are known.
    pub flow: Option<Vec<(f64, f64)>>,
    /// The volume profile's own point of control and value area.
    pub vp_poc: f64,
    pub vp_vah: f64,
    pub vp_val: f64,
    pub ib: Option<(f64, f64)>,
    pub singles: Vec<Band>,
    /// More than one bracket at the very high, or low: an unfinished auction.
    pub poor_high: bool,
    pub poor_low: bool,
    pub open: f64,
    pub close: f64,
    /// The bar that first traded back to a level, or None while it is still naked.
    pub naked: Naked,
}

impl Profile {
    /// Its lowest and highest row.
    fn bounds(&self) -> (i64, i64) {
        (self.session.rows.first().map_or(0, |r| r.index), self.session.rows.last().map_or(0, |r| r.index))
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Naked {
    pub poc: Option<usize>,
    pub vah: Option<usize>,
    pub val: Option<usize>,
    pub singles: Vec<Option<usize>>,
    pub high: Option<usize>,
    pub low: Option<usize>,
    pub vp_poc: Option<usize>,
    pub vp_vah: Option<usize>,
    pub vp_val: Option<usize>,
}

pub struct Profiles {
    pub row_size: f64,
    pub grid_low: f64,
    pub list: Vec<Profile>,
    /// The time scale's warp: `(bar edge, column)` knots.
    pub warp: Vec<(f64, f64)>,
    key: Key,
}

#[derive(Clone, Debug, PartialEq)]
struct Key {
    period: Period,
    bars: usize,
    first: i64,
    last: Option<(i64, u64, u64)>,
    shape: Shape,
    edits: Edits,
    flow: (usize, i64, i64, u64),
}

fn key(period: Period, bars: &[Candle], flow: &[footprint::Bar], settings: &Settings, edits: &Edits) -> Key {
    Key {
        period,
        bars: bars.len(),
        first: bars.first().map_or(0, |b| b.open_time),
        last: bars.last().map(|b| (b.open_time, b.high.to_bits(), b.low.to_bits())),
        shape: settings.shape(),
        edits: edits.clone(),
        flow: (flow.len(), flow.first().map_or(0, |b| b.time), flow.last().map_or(0, |b| b.time), flow.last().map_or(0, |b| b.levels.iter().map(|l| l.1 + l.2).sum::<f64>().to_bits())),
    }
}

impl Profiles {
    /// Whether these were built from exactly these bars and options.
    pub fn fresh(&self, period: Period, bars: &[Candle], flow: &[footprint::Bar], settings: &Settings, edits: &Edits) -> bool {
        self.key == key(period, bars, flow, settings, edits)
    }

    /// The profile holding bar `index`, by position in the list.
    pub fn at_bar(&self, index: usize) -> Option<usize> {
        let k = self.list.partition_point(|p| p.last < index);
        self.list.get(k).filter(|p| p.first <= index).map(|_| k)
    }
}

/// The smallest step the prices move by: the coarsest of 1, 2, 5 and their
/// tens that every recent price sits on, down to the displayed decimals.
pub fn infer_tick(bars: &[Candle], decimals: usize) -> f64 {
    let base = 10f64.powi(-(decimals.min(12) as i32));
    let recent = &bars[bars.len().saturating_sub(400)..];
    let on = |step: f64| recent.iter().all(|b| [b.open, b.high, b.low, b.close].iter().all(|p| ((p / step).round() * step - p).abs() < base * 0.01));
    [100.0, 50.0, 25.0, 20.0, 10.0, 5.0, 2.0].into_iter().map(|m| base * m).find(|s| !recent.is_empty() && on(*s)).unwrap_or(base)
}

/// Runs of rows where exactly one bracket traded, away from either end.
fn single_runs(session: &TpoSession) -> Vec<Band> {
    let mut out = Vec::new();
    let mut run: Option<Band> = None;
    for row in &session.rows {
        let single = row.periods.len() == 1;
        run = match (run, single) {
            (Some((lo, hi)), true) if row.index == hi + 1 => Some((lo, row.index)),
            (Some(done), _) => {
                out.push(done);
                single.then_some((row.index, row.index))
            }
            (None, true) => Some((row.index, row.index)),
            (None, false) => None,
        };
    }
    out.extend(run);
    let (bottom, top) = (session.rows.first().map_or(0, |r| r.index), session.rows.last().map_or(0, |r| r.index));
    // At an end a run of single prints is a tail: excess, a good high or low.
    out.retain(|(lo, hi)| *lo != bottom && *hi != top);
    out
}

/// The first bar after `after` whose range reaches `low..=high`.
fn touched(bars: &[Candle], after: usize, low: f64, high: f64) -> Option<usize> {
    bars.iter().enumerate().skip(after + 1).find(|(_, b)| b.low <= high && b.high >= low).map(|(i, _)| i)
}

/// The row size for `row_ticks`, or, on auto, about forty rows to a median day.
fn row_size(bars: &[Candle], tick: f64, row_ticks: u32) -> f64 {
    if row_ticks > 0 && tick > 0.0 {
        return tick * f64::from(row_ticks);
    }
    let mut ranges: Vec<f64> = Vec::new();
    let mut day = (i64::MIN, f64::NEG_INFINITY, f64::INFINITY);
    for b in bars {
        let d = b.open_time.div_euclid(DAY);
        if d != day.0 {
            if day.1 > day.2 {
                ranges.push(day.1 - day.2);
            }
            day = (d, b.high, b.low);
        } else {
            day = (d, day.1.max(b.high), day.2.min(b.low));
        }
    }
    if day.1 > day.2 {
        ranges.push(day.1 - day.2);
    }
    if ranges.is_empty() {
        return tick.max(1e-9);
    }
    ranges.sort_by(f64::total_cmp);
    let raw = ranges[ranges.len() / 2] / AUTO_DAY_ROWS;
    if !(tick > 0.0) {
        return nice_step(raw);
    }
    // The nearest round number of ticks, not the next one up.
    let ticks = (raw / tick).max(1.0);
    let pow = 10f64.powf(ticks.log10().floor());
    let off = |a: f64| (a / ticks).max(ticks / a);
    let best = [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().map(|m| m * pow).min_by(|a, b| off(*a).total_cmp(&off(*b))).unwrap_or(1.0);
    tick * best.round().max(1.0)
}

/// Every profile of `bars` — the chart's own candles, sorted — cut by `period`.
/// `tick` is the smallest price step, for row sizes in ticks; `flow` is what
/// the footprint knows was bought and sold, by bar.
pub fn build(bars: &[Candle], period: Period, tick: f64, flow: &[footprint::Bar], settings: &Settings, edits: &Edits) -> Option<Profiles> {
    let (first_bar, last_bar) = (bars.first()?, bars.last()?);
    let high = bars.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
    let low = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    if !(high > low) {
        return None;
    }
    let row_size = row_size(bars, tick, settings.row_ticks);
    // Too fine a row for the range would be millions of rows: fall back to auto.
    let row_size = if (high - low) / row_size > 20_000.0 { self::row_size(bars, tick, 0) } else { row_size };
    let grid_low = (low / row_size).floor() * row_size;
    let opts = TpoOptions { min_periods: 1, value_area_pct: settings.va_pct / 100.0, initial_balance_periods: settings.ib_candles, ..TpoOptions::new(row_size, grid_low) };
    let row_of = |price: f64| ((price - grid_low) / row_size).floor() as i64;

    // Bars to the piece that holds them; a bar outside every session belongs to none.
    let pieces = pieces(windows(period, &settings.sessions, first_bar.open_time, last_bar.open_time), edits);
    let mut ranges: Vec<(i64, i64, usize)> = pieces.iter().enumerate().flat_map(|(k, p)| p.ranges.iter().map(move |r| (r.0, r.1, k))).collect();
    ranges.sort_by_key(|r| r.0);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); pieces.len()];
    let mut at = 0;
    for (i, b) in bars.iter().enumerate() {
        while at < ranges.len() && ranges[at].1 <= b.open_time {
            at += 1;
        }
        if let Some(r) = ranges.get(at).filter(|r| r.0 <= b.open_time) {
            members[r.2].push(i);
        }
    }
    let flow_at = |time: i64| flow.binary_search_by_key(&time, |f| f.time).ok().map(|i| &flow[i]);

    let mut list = Vec::new();
    for (piece, indices) in pieces.into_iter().zip(members) {
        let (Some(&first), Some(&last)) = (indices.first(), indices.last()) else { continue };
        let brackets: Vec<Candle> = indices.iter().map(|i| bars[*i]).collect();
        let (start, end) = (piece.ranges.first().map_or(0, |r| r.0), piece.ranges.last().map_or(0, |r| r.1));
        let Some(session) = tpo_session(&brackets, start, end, &opts) else { continue };

        let dense = |index: i64| session.rows.binary_search_by_key(&index, |r| r.index).ok();
        let mut volume = vec![0.0; session.rows.len()];
        for b in &brackets {
            let (lo, hi) = (row_of(b.low), row_of(b.high));
            let share = b.volume / (hi - lo + 1) as f64;
            for r in lo..=hi {
                if let Some(i) = dense(r) {
                    volume[i] += share;
                }
            }
        }
        // Who took the volume, only when the footprint has every bracket: half a
        // profile's flow would read as the whole of it.
        let flow = brackets.iter().all(|b| flow_at(b.open_time).is_some()).then(|| {
            let mut rows = vec![(0.0, 0.0); session.rows.len()];
            for b in &brackets {
                for &(price, buy, sell) in flow_at(b.open_time).map(|f| f.levels.as_slice()).unwrap_or_default() {
                    if let Some(i) = dense(row_of(price)) {
                        rows[i].0 += buy;
                        rows[i].1 += sell;
                    }
                }
            }
            rows
        });

        // The volume profile's own levels, over every row from bottom to top.
        let (bottom, top) = (session.rows.first().map_or(0, |r| r.index), session.rows.last().map_or(0, |r| r.index));
        let mut contiguous = vec![0.0; (top - bottom + 1) as usize];
        for (i, row) in session.rows.iter().enumerate() {
            contiguous[(row.index - bottom) as usize] = flow.as_ref().map_or(volume[i], |f| f[i].0 + f[i].1);
        }
        let busiest = contiguous.iter().enumerate().fold(0, |best, (i, v)| if *v > contiguous[best] { i } else { best });
        let (va_lo, va_hi) = value_area(&contiguous, busiest, settings.vp_va_pct / 100.0);
        let price_of = |offset: f64| grid_low + (bottom as f64 + offset) * row_size;
        let (vp_poc, vp_vah, vp_val) = (price_of(busiest as f64 + 0.5), price_of(va_hi as f64 + 1.0), price_of(va_lo as f64));

        let singles = single_runs(&session);
        let count = |index: i64| dense(index).map_or(0, |i| session.rows[i].periods.len());
        let (poor_high, poor_low) = (count(top) >= 2, count(bottom) >= 2);
        let band = |(lo, hi): Band| (grid_low + lo as f64 * row_size, grid_low + (hi + 1) as f64 * row_size);
        let ib = session.initial_balance;
        let naked = Naked {
            poc: touched(bars, last, session.poc, session.poc),
            vah: touched(bars, last, session.vah, session.vah),
            val: touched(bars, last, session.val, session.val),
            singles: singles.iter().map(|s| band(*s)).map(|(lo, hi)| touched(bars, last, lo, hi)).collect(),
            high: touched(bars, last, session.high, session.high),
            low: touched(bars, last, session.low, session.low),
            vp_poc: touched(bars, last, vp_poc, vp_poc),
            vp_vah: touched(bars, last, vp_vah, vp_vah),
            vp_val: touched(bars, last, vp_val, vp_val),
        };
        let widest = session.rows.iter().map(|r| r.periods.len()).max().unwrap_or(1);
        let expanded = edits.expanded.contains(&start);
        let block_columns = if expanded { last - first + 1 } else { widest };
        let vp_columns = if settings.vp && settings.vp_right { (widest as f64 * settings.vp_width).ceil() + 1.0 } else { 0.0 };
        list.push(Profile {
            open: brackets.first().map_or(session.poc, |b| b.open),
            close: brackets.last().map_or(session.poc, |b| b.close),
            start,
            end,
            name: piece.name,
            color: piece.color,
            merged: piece.merged,
            expanded,
            first,
            last,
            offsets: indices.iter().map(|i| i - first).collect(),
            columns: block_columns as f64 + vp_columns + f64::from(settings.spacing),
            block_columns,
            widest,
            volume,
            flow,
            vp_poc,
            vp_vah,
            vp_val,
            ib,
            singles,
            poor_high,
            poor_low,
            naked,
            session,
        });
    }

    // Profiles side by side: each its columns, a bar between them next to none.
    let mut warp: Vec<(f64, f64)> = Vec::new();
    let mut column = list.first().map_or(0.0, |p| p.first as f64);
    let mut edge = column;
    for p in &list {
        let (from, to) = (p.first as f64, (p.last + 1) as f64);
        if warp.is_empty() || from > edge {
            if !warp.is_empty() {
                column += (from - edge) * OUTSIDE_COLUMNS;
            }
            warp.push((from, column));
        }
        column += p.columns;
        warp.push((to, column));
        edge = to;
    }
    Some(Profiles { row_size, grid_low, list, warp, key: key(period, bars, flow, settings, edits) })
}

/// Where profiles are drawn and with what.
pub struct View<'a> {
    pub plot: Rect,
    /// Pixels per column.
    pub column: f32,
    pub char_width: f32,
    pub theme: &'a Theme,
    /// x where a bar edge stands: where bar `edge` starts.
    pub edge_x: &'a dyn Fn(f64) -> f32,
    /// y of a price.
    pub y: &'a dyn Fn(f64) -> f32,
    pub bars: &'a [Candle],
    /// The profile under the pointer, by position.
    pub hovered: Option<usize>,
}

/// A level to mark on the price axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tag {
    pub price: f64,
    pub color: Color,
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color { r: ch(a.r, b.r), g: ch(a.g, b.g), b: ch(a.b, b.b), a: ch(a.a, b.a) }
}

/// What a block stands for, which sets its colour.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Outside,
    Value,
    Poc,
    Poor,
    Single,
}

/// Draws every profile with any part on screen; returns the levels the price
/// axis should carry.
pub fn paint(f: &mut Frame, v: &View, p: &Profiles, s: &Settings) -> Vec<Tag> {
    let t = v.theme;
    let c = s.colors(t);
    let mut tags = Vec::new();
    let plot_w = v.plot.w;
    let dpr = f.dpr;
    let hair = 1.0 / dpr;
    let col = v.column;
    let gap_x = (col * BLOCK_GAP).clamp(hair, MAX_GAP);
    let block = f.snap((col - gap_x).max(hair)).max(hair);
    let merged = col * dpr < 1.5;
    let label_size = t.font_px - 2.0;
    let glyph = v.char_width * label_size / t.font_px;
    let text = !s.hide_text;
    // x where a level stops: the bar that traded back into it, or the chart's edge.
    let naked_end = |touch: Option<usize>| touch.map_or(plot_w, |i| (v.edge_x)(i as f64));
    let reach_naked = s.naked_poc || s.naked_va || s.naked_singles || s.naked_poor || (s.vp && (s.vp_naked_poc || s.vp_naked_va));

    for (k, profile) in p.list.iter().enumerate() {
        let session = &profile.session;
        let x0 = (v.edge_x)(profile.first as f64);
        let slot_end = x0 + profile.columns as f32 * col;
        let blocks_right = x0 + profile.block_columns as f32 * col;
        if x0 > plot_w || (slot_end < 0.0 && !reach_naked) {
            continue;
        }
        let y_top = |index: i64| (v.y)(p.grid_low + (index + 1) as f64 * p.row_size);
        let y_bottom = |index: i64| (v.y)(p.grid_low + index as f64 * p.row_size);
        let row_of = |price: f64| ((price - p.grid_low) / p.row_size).floor() as i64;
        let last_period = session.periods.len().saturating_sub(1).max(1) as f32;
        let max_count = profile.widest.max(1) as f32;
        let (bottom, top) = profile.bounds();
        let poc_row = row_of(session.poc);
        let (vah, val) = (session.vah, session.val);
        let x_of = |j: usize, period: usize| x0 + (if profile.expanded { profile.offsets[period] } else { j }) as f32 * col;
        // Where a row's blocks end, for a line that carries on from them.
        let row_end = |index: i64| -> f32 {
            session.rows.binary_search_by_key(&index, |r| r.index).ok().map_or(x0, |i| {
                let row = &session.rows[i];
                row.periods.last().map_or(x0, |last| x_of(row.periods.len() - 1, *last) + col)
            })
        };
        let line_end = |naked: bool, touch: Option<usize>| if naked { naked_end(touch) } else { slot_end };
        let outside = profile.color.unwrap_or(c.outside);

        // ---- under the blocks: the value area filled, single prints carried on
        if s.fill_va {
            let right = line_end(s.naked_va, profile.naked.vah.max(profile.naked.val));
            f.rect(x0, (v.y)(vah), (right - x0).max(0.0), (v.y)(val) - (v.y)(vah), c.fill_va);
        }
        if s.singles {
            for (i, (lo, hi)) in profile.singles.iter().enumerate() {
                let from = (*lo..=*hi).map(row_end).fold(x0, f32::max);
                let right = line_end(s.naked_singles, profile.naked.singles.get(i).copied().flatten());
                f.rect(from, y_top(*hi), (right - from).max(0.0), y_bottom(*lo) - y_top(*hi), c.single_ray);
            }
        }
        if s.vp && !s.vp_right {
            paint_volume(f, v, p, s, profile, x0, slot_end, true);
        }

        // ---- the blocks
        let single_rows: Vec<i64> = if s.singles { profile.singles.iter().flat_map(|(lo, hi)| *lo..=*hi).collect() } else { Vec::new() };
        let open_row = row_of(profile.open);
        let close_row = row_of(profile.close);
        let last_index = session.periods.len().saturating_sub(1);
        for row in &session.rows {
            let top_y = y_top(row.index);
            let bottom_y = y_bottom(row.index);
            if bottom_y < v.plot.y || top_y > v.plot.bottom() {
                continue;
            }
            let height = bottom_y - top_y;
            let gap = if height >= 3.0 { f.snap((height * BLOCK_GAP).clamp(hair, MAX_GAP)).max(hair) } else { 0.0 };
            let h = (height - gap).max(hair);
            let mid = p.grid_low + (row.index as f64 + 0.5) * p.row_size;
            let role = if s.poc && row.index == poc_row {
                Role::Poc
            } else if s.poor && ((profile.poor_high && row.index == top) || (profile.poor_low && row.index == bottom)) {
                Role::Poor
            } else if single_rows.contains(&row.index) {
                Role::Single
            } else if mid >= val && mid <= vah {
                Role::Value
            } else {
                Role::Outside
            };
            let count = row.periods.len() as f32;
            let color_of = |period: usize| -> Color {
                if s.highlight && period == 0 && row.index == open_row {
                    return c.start;
                }
                if s.highlight && period == last_index && row.index == close_row && last_index > 0 {
                    return c.end;
                }
                let (base, dim) = match role {
                    Role::Poc => (c.poc, DIM_POC),
                    Role::Poor => return c.poor,
                    Role::Single => return c.single,
                    Role::Value => (c.va, DIM_VALUE),
                    Role::Outside => (outside, if profile.color.is_some() { DIM_VALUE } else { DIM_OUTSIDE }),
                };
                match s.block_mode {
                    BlockMode::Brightness => mix(t.background, base, dim + (1.0 - dim) * period as f32 / last_period),
                    BlockMode::Solid => base,
                    BlockMode::HeatMap if role != Role::Poc => mix(s.heat_low, s.heat_high, count / max_count),
                    BlockMode::HeatMap => base,
                    BlockMode::Candle if role != Role::Poc => match v.bars.get(profile.first + profile.offsets[period]) {
                        Some(b) if b.close < b.open => t.bear,
                        _ => t.bull,
                    },
                    BlockMode::Candle => base,
                }
            };
            if merged && !profile.expanded {
                // Columns narrower than a device pixel: a few pixels at a time,
                // each shaded by the brackets it stands for.
                let per = ((3.0 / dpr / col).ceil() as usize).max(1);
                for (c, chunk) in row.periods.chunks(per).enumerate() {
                    let x = x0 + (c * per) as f32 * col;
                    if x > plot_w {
                        break;
                    }
                    let mean = (chunk.iter().sum::<usize>() as f32 / chunk.len() as f32).round() as usize;
                    f.rect(x, top_y, (chunk.len() as f32 * col).max(hair), h, color_of(mean));
                }
            } else {
                for (j, period) in row.periods.iter().enumerate() {
                    let x = f.snap(x_of(j, *period) + (col - block) / 2.0);
                    if x > plot_w {
                        break;
                    }
                    if x + block >= 0.0 {
                        f.rect(x, top_y, block, h, color_of(*period));
                    }
                }
            }
        }

        // The volume beside the profile, under the lines that cross it.
        if s.vp && s.vp_right {
            paint_volume(f, v, p, s, profile, x0, slot_end, false);
        }

        // ---- lines from the profile on to the next one, or until traded back to
        // Named just under the line, at its far end, as MMT has them.
        let label = |f: &mut Frame, right: f32, y: f32, name: &str, color: Color| {
            if text && right - x0 >= name.len() as f32 * glyph + 6.0 {
                f.text(right - 2.0, y + 2.0, name, color, Align::Right, Baseline::Top, label_size, None, Some(v.plot));
            }
        };
        if s.va_rays || s.naked_va {
            for (name, level, touch) in [("VAH", vah, profile.naked.vah), ("VAL", val, profile.naked.val)] {
                let y = (v.y)(level);
                if !(v.plot.y..=v.plot.bottom()).contains(&y) {
                    continue;
                }
                let right = line_end(s.naked_va, touch).min(plot_w);
                f.dashed_line(x0.max(0.0), y, right, y, hair, c.va_ray.with_alpha(0x90), 4.0, 3.0);
                label(f, right, y, name, c.va_ray.with_alpha(0xd0));
            }
        }
        if s.poc {
            let y = (v.y)(session.poc);
            let right = line_end(s.naked_poc, profile.naked.poc).min(plot_w);
            let from = row_end(poc_row);
            if right > from {
                f.hline(from, right, y, s.poc_width / 2.0, c.poc_ray);
                label(f, right, y, "POC", c.poc_ray);
            }
            if s.naked_poc && profile.naked.poc.is_none() {
                tags.push(Tag { price: session.poc, color: c.poc_ray });
            }
        }
        if s.poor {
            for (poor, index, name, touch) in [(profile.poor_high, top, "PH", profile.naked.high), (profile.poor_low, bottom, "PL", profile.naked.low)] {
                if !poor {
                    continue;
                }
                let y = if index == top { y_top(index) } else { y_bottom(index) };
                let from = row_end(index);
                let right = line_end(s.naked_poor, touch).min(plot_w);
                if right > from {
                    f.hline(from, right, y, 1.0, c.poor);
                    label(f, right, y, name, c.poor);
                }
            }
        }

        // ---- the first brackets' range, by the profile
        if s.ib {
            if let Some((h, l)) = profile.ib {
                f.rect(f.snap(x0 - 4.0), (v.y)(h), 2.0, ((v.y)(l) - (v.y)(h)).max(2.0), c.ib);
            }
        }
        if v.hovered == Some(k) {
            let (y0, y1) = ((v.y)(session.high), (v.y)(session.low));
            f.stroke_rect(f.snap(x0) - 1.0, y0 - 1.0, blocks_right - x0 + 2.0, y1 - y0 + 2.0, 1.0, t.text.with_alpha(0xc0));
        }
        if s.session_header && text {
            let c = civil(profile.start);
            let name = if profile.name.is_empty() { format!("{} {:02}/{:02}", WEEKDAYS[weekday(profile.start) as usize], c.day, c.month0 + 1) } else { profile.name.to_string() };
            f.text(x0, y_top(top) - 3.0, name, profile.color.unwrap_or(t.text), Align::Left, Baseline::Bottom, label_size, None, Some(v.plot));
        }
        if s.vp && s.vp_naked_poc && s.vp_poc && profile.naked.vp_poc.is_none() {
            tags.push(Tag { price: profile.vp_poc, color: c.vp_poc });
        }
    }
    tags
}

/// One profile's volume by row, against the far side of its slot (or under
/// its blocks, from the left), with its own levels.
#[allow(clippy::too_many_arguments)]
fn paint_volume(f: &mut Frame, v: &View, p: &Profiles, s: &Settings, profile: &Profile, x0: f32, slot_end: f32, under: bool) {
    let t = v.theme;
    let c = s.colors(t);
    let hair = 1.0 / f.dpr;
    let col = v.column;
    let length = (profile.widest as f64 * s.vp_width) as f32 * col;
    let anchor = if under { x0 } else { slot_end - hair };
    let mode = if profile.flow.is_none() { VpMode::Total } else { s.vp_mode };
    let rows: Vec<(i64, f64, f64)> = profile
        .session
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| match &profile.flow {
            Some(flow) => (r.index, flow[i].0, flow[i].1),
            None => (r.index, profile.volume[i], 0.0),
        })
        .collect();
    let busiest = rows.iter().map(|r| r.1 + r.2).fold(0.0, f64::max);
    let widest_delta = rows.iter().map(|r| (r.1 - r.2).abs()).fold(0.0, f64::max);
    if busiest <= 0.0 {
        return;
    }
    let alpha = if under { 0x99 } else { 0xff };
    let label_size = (t.font_px - 3.0).max(7.0);
    for (index, buy, sell) in rows {
        let total = buy + sell;
        if total < busiest * s.vp_bin_filter / 100.0 {
            continue;
        }
        let top = (v.y)(p.grid_low + (index + 1) as f64 * p.row_size);
        let bottom = (v.y)(p.grid_low + index as f64 * p.row_size);
        if bottom < v.plot.y || top > v.plot.bottom() {
            continue;
        }
        let height = bottom - top;
        let gap = if height >= 3.0 { (height * BLOCK_GAP).clamp(hair, MAX_GAP) } else { 0.0 };
        let h = (height - gap).max(hair);
        // From the anchor outwards: right to left against the slot's end, or left to right.
        let mut bar = |from: f32, w: f32, color: Color| {
            let x = if under { anchor + from } else { anchor - from - w };
            f.rect(x, top, w.max(hair), h, color.with_alpha(alpha));
        };
        let w = length * (total / busiest) as f32;
        match mode {
            VpMode::Standard => {
                // Bought nearest the profile, sold beyond it.
                let wb = w * (buy / total) as f32;
                if under {
                    bar(0.0, wb, c.vp_buy);
                    bar(wb, w - wb, c.vp_sell);
                } else {
                    bar(w - wb, wb, c.vp_buy);
                    bar(0.0, w - wb, c.vp_sell);
                }
            }
            VpMode::Total => bar(0.0, w, c.vp),
            VpMode::Delta => {
                let d = buy - sell;
                let wd = if widest_delta > 0.0 { length * (d.abs() / widest_delta) as f32 } else { 0.0 };
                bar(0.0, wd, if d >= 0.0 { c.vp_buy } else { c.vp_sell });
            }
        }
        if s.vp_labels && !s.vp_hide_text && height >= label_size + 1.0 {
            let value = if mode == VpMode::Delta { buy - sell } else { total };
            let (x, align) = if under { (anchor + 2.0, Align::Left) } else { (anchor - 2.0, Align::Right) };
            f.text(x, (top + bottom) / 2.0, format_turnover(value), c.vp_label, align, Baseline::Middle, label_size, None, Some(v.plot));
        }
    }
    let start = if under { x0 } else { anchor - length };
    let line_end = |naked: bool, touch: Option<usize>| if naked { touch.map_or(v.plot.w, |i| (v.edge_x)(i as f64)) } else { slot_end };
    let text = !s.vp_hide_text;
    if s.vp_poc {
        let y = (v.y)(profile.vp_poc);
        let right = line_end(s.vp_naked_poc, profile.naked.vp_poc).min(v.plot.w);
        f.hline(start, right, y, 1.0, c.vp_poc);
        if text {
            f.text(right - 2.0, y - 2.0, "VPOC", c.vp_poc, Align::Right, Baseline::Bottom, t.font_px - 2.0, None, Some(v.plot));
        }
    }
    if s.vp_va_rays || s.vp_naked_va {
        for (name, level, touch) in [("VAH", profile.vp_vah, profile.naked.vp_vah), ("VAL", profile.vp_val, profile.naked.vp_val)] {
            let y = (v.y)(level);
            let right = line_end(s.vp_naked_va, touch).min(v.plot.w);
            f.dashed_line(start.max(0.0), y, right, y, hair, c.vp_va.with_alpha(0xa0), 2.0, 2.0);
            if text {
                f.text(right - 2.0, y - 2.0, name, c.vp_va, Align::Right, Baseline::Bottom, t.font_px - 2.0, None, Some(v.plot));
            }
        }
    }
}

/// A label for the time axis under each profile: its day, or its month.
pub fn axis_label(period: Period, shown: i64) -> String {
    let c = civil(shown);
    match period {
        Period::Month => format!("{} {}", MONTHS[c.month0 as usize], c.year),
        _ => format!("{} {:02}/{:02}", WEEKDAYS[weekday(shown) as usize], c.day, c.month0 + 1),
    }
}

/// What the page shows about the profile under the pointer, for its menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub start: i64,
    pub end: i64,
    /// The start of the profile before it and after it, for merging.
    pub prev: Option<i64>,
    pub next: Option<i64>,
    pub expanded: bool,
    pub merged: bool,
    /// The bracket under the pointer, when it is expanded and not the first: where a split falls.
    pub split_at: Option<i64>,
    /// Its levels, named.
    pub prices: Vec<(&'static str, f64)>,
}

/// `bar`: the bar under the pointer, and its time.
pub fn hover(p: &Profiles, s: &Settings, k: usize, bar: Option<(usize, i64)>) -> Option<Hover> {
    let profile = p.list.get(k)?;
    let session = &profile.session;
    let mut prices = vec![("POC", session.poc), ("VAH", session.vah), ("VAL", session.val), ("High", session.high), ("Low", session.low)];
    if let (true, Some((h, l))) = (s.ib, profile.ib) {
        prices.extend([("IB high", h), ("IB low", l)]);
    }
    if s.vp {
        prices.extend([("Volume POC", profile.vp_poc), ("Volume VAH", profile.vp_vah), ("Volume VAL", profile.vp_val)]);
    }
    let split_at = bar.filter(|(b, _)| profile.expanded && *b > profile.first && *b <= profile.last).map(|(_, time)| time);
    Some(Hover {
        start: profile.start,
        end: profile.end,
        prev: k.checked_sub(1).and_then(|j| p.list.get(j)).map(|q| q.start),
        next: p.list.get(k + 1).map(|q| q.start),
        expanded: profile.expanded,
        merged: profile.merged,
        split_at,
        prices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(i: i64, low: f64, high: f64, volume: f64) -> Candle {
        Candle { open_time: i * HOUR, open: low, high, low, close: high, volume, quote_volume: None }
    }

    #[test]
    fn settings_are_read_over_the_defaults_and_nonsense_is_skipped() {
        let s = Settings::parse("blockMode=heatmap;vaPct=68;ibCandles=3;vpOrigin=left;poorColor=#112233;bogus=9;vaPct=999;sessions=0800-1630#ff0000");
        assert_eq!(s.block_mode, BlockMode::HeatMap);
        assert_eq!(s.va_pct, 68.0, "an out-of-range value leaves the last good one");
        assert_eq!(s.ib_candles, 3);
        assert!(!s.vp_right);
        assert_eq!(s.poor_color, Some(Color::hex(0x112233)));
        assert_eq!(Settings::default().va_color, None, "the theme's, until set by hand");
        assert_eq!(s.sessions, vec![Session { start: 480, end: 990, color: Color::hex(0xff0000) }]);
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Settings::default().sessions.len(), 4, "MMT's four sessions to start from");
    }

    /// A day of hourly brackets: a thin run up, a fat middle, a thin run down.
    fn day() -> Vec<Candle> {
        let mut bars = vec![bar(0, 100.0, 100.5, 10.0)];
        for i in 1..20 {
            bars.push(bar(i, 104.0, 106.0, 50.0));
        }
        bars.push(bar(20, 108.0, 112.0, 10.0));
        bars
    }

    fn build_day(bars: &[Candle], s: &Settings, edits: &Edits) -> Profiles {
        build(bars, Period::Day, 0.1, &[], s, edits).expect("a profile")
    }

    #[test]
    fn profiles_find_the_value_area_single_prints_and_poor_extremes() {
        let mut bars = day();
        // Two brackets at the very top: a poor high.
        bars.push(bar(21, 111.0, 112.0, 10.0));
        let s = Settings { row_ticks: 10, ..Settings::default() };
        let p = build_day(&bars, &s, &Edits::default());
        assert_eq!(p.row_size, 1.0, "ten ticks of 0.1");
        let profile = &p.list[0];
        let session = &profile.session;
        assert!(session.val >= 104.0 && session.vah <= 107.0, "value area around the fat middle: {}–{}", session.val, session.vah);
        assert!(profile.poor_high, "two brackets share the high");
        assert!(!profile.poor_low, "the low is one bracket's");
        let prices = |(lo, hi): Band| (p.grid_low + lo as f64 * p.row_size, p.grid_low + hi as f64 * p.row_size);
        assert_eq!(profile.singles.iter().copied().map(prices).collect::<Vec<_>>(), vec![(108.0, 110.0)], "the run up the middle, not the tail at the bottom");
        assert_eq!(profile.ib, Some((106.0, 100.0)), "the first two brackets' range");
    }

    #[test]
    fn profiles_stand_side_by_side_at_their_own_widths() {
        let mut bars = day();
        for i in 24..30 {
            bars.push(bar(i, 110.0, 115.0, 5.0));
        }
        let s = Settings { row_ticks: 10, spacing: 3, ..Settings::default() };
        let p = build_day(&bars, &s, &Edits::default());
        assert_eq!(p.list.len(), 2);
        let (a, b) = (&p.list[0], &p.list[1]);
        assert_eq!(a.columns, (a.widest + 3) as f64, "its widest row and the space after it");
        assert_eq!(p.warp, vec![(0.0, 0.0), (21.0, a.columns), (27.0, a.columns + b.columns)]);
        let expanded = build_day(&bars, &s, &Edits { expanded: vec![0], ..Edits::default() });
        assert_eq!(expanded.list[0].columns, 21.0 + 3.0, "expanded, a column a bracket");
    }

    #[test]
    fn a_level_stays_naked_until_a_later_bar_trades_back_to_it() {
        let mut bars = day();
        // The next day stays above the value area, then comes back down to it.
        for i in 24..30 {
            bars.push(bar(i, 110.0, 115.0, 5.0));
        }
        bars.push(bar(30, 104.5, 111.0, 5.0));
        let p = build_day(&bars, &Settings { row_ticks: 10, ..Settings::default() }, &Edits::default());
        assert_eq!(p.list[0].naked.poc, Some(bars.len() - 1), "the POC is naked until the last bar reaches it");
        assert_eq!(p.list[1].naked.poc, None, "today's POC has nothing after it yet");
    }

    #[test]
    fn profiles_merge_and_split_by_hand() {
        let mut bars = day();
        for i in 24..30 {
            bars.push(bar(i, 110.0, 115.0, 5.0));
        }
        let s = Settings { row_ticks: 10, ..Settings::default() };
        let merged = build_day(&bars, &s, &Edits { merged: vec![DAY], ..Edits::default() });
        assert_eq!(merged.list.len(), 1, "the second day joined to the first");
        assert!(merged.list[0].merged && merged.list[0].session.periods.len() == bars.len());
        let split = build_day(&bars, &s, &Edits { splits: vec![10 * HOUR], ..Edits::default() });
        assert_eq!(split.list.len(), 3);
        assert_eq!((split.list[0].start, split.list[1].start, split.list[1].session.periods.len()), (0, 10 * HOUR, 11));
    }

    #[test]
    fn sessions_follow_london_and_new_york_summer_time() {
        // 2026-01-15, winter in both; 2026-07-15, summer in both; 2026-03-16,
        // between New York's change (8 March) and London's (29 March).
        let winter = utc(2026, 0, 15);
        let summer = utc(2026, 6, 15);
        let gap = utc(2026, 2, 16);
        assert_eq!(london(winter), (480, 990));
        assert_eq!(london(summer), (420, 930));
        assert_eq!(new_york(winter), (870, 1260));
        assert_eq!(new_york(summer), (810, 1200));
        assert_eq!((london(gap), new_york(gap).0), ((480, 990), 810));
        let day = windows(Period::Intraday, &[], winter, winter);
        let names: Vec<_> = day.iter().filter(|w| w.start >= winter).map(|w| (w.name, (w.start - winter) / MINUTE, (w.end - winter) / MINUTE)).collect();
        assert_eq!(names, vec![("Asia", 0, 480), ("London", 480, 870), ("New York", 870, 1440)]);
    }

    #[test]
    fn custom_sessions_run_past_midnight_and_bars_outside_any_belong_to_none() {
        let s = Settings { row_ticks: 10, sessions: Session::parse_list("2200-0200#ffffff"), ..Settings::default() };
        let bars: Vec<Candle> = (0..48).map(|i| bar(i, 100.0 + (i % 5) as f64, 102.0 + (i % 5) as f64, 1.0)).collect();
        let p = build(&bars, Period::Custom, 0.1, &[], &s, &Edits::default()).unwrap();
        let spans: Vec<_> = p.list.iter().map(|q| (q.first, q.last)).collect();
        assert_eq!(spans, vec![(0, 1), (22, 25), (46, 47)], "22:00 to 02:00, the day before's into this one");
    }

    #[test]
    fn the_tick_is_read_off_the_prices() {
        let bars: Vec<Candle> = (0..50).map(|i| Candle { open_time: i, open: 100.0 + i as f64 * 0.1, high: 101.5, low: 99.3, close: 100.2, volume: 1.0, quote_volume: None }).collect();
        assert!((infer_tick(&bars, 2) - 0.1).abs() < 1e-12, "two decimals shown, a tenth traded");
        let halves: Vec<Candle> = bars.iter().map(|b| Candle { open: 100.5, high: 101.5, low: 99.0, close: 100.0, ..*b }).collect();
        assert!((infer_tick(&halves, 2) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn auto_rows_cut_a_median_day_into_about_forty() {
        // Days of 2,500 on a 0.1 tick: 50 a row, 500 ticks — MMT's BTC setting.
        let bars: Vec<Candle> = (0..72).map(|i| bar(i, 60_000.0, 62_500.0, 1.0)).collect();
        assert!((row_size(&bars, 0.1, 0) - 50.0).abs() < 1e-9);
        assert!((row_size(&bars, 0.1, 250) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn volume_profiles_split_buying_from_selling_when_the_footprint_has_every_bracket() {
        let bars: Vec<Candle> = (0..4).map(|i| bar(i, 100.0, 101.9, 10.0)).collect();
        let flow: Vec<footprint::Bar> = bars.iter().map(|b| footprint::Bar { time: b.open_time, levels: vec![(100.0, 3.0, 1.0), (101.0, 1.0, 5.0)], ..footprint::Bar::default() }).collect();
        let s = Settings { row_ticks: 10, vp: true, ..Settings::default() };
        let p = build(&bars, Period::Day, 0.1, &flow, &s, &Edits::default()).unwrap();
        assert_eq!(p.list[0].flow, Some(vec![(12.0, 4.0), (4.0, 20.0)]));
        assert!((p.list[0].vp_poc - 101.5).abs() < 1e-9, "most traded at 101");
        let partial = build(&bars, Period::Day, 0.1, &flow[1..], &s, &Edits::default()).unwrap();
        assert_eq!(partial.list[0].flow, None, "one bracket missing: no flow");
    }
}
