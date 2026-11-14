//! Drawing tools: what each kind is, how it is styled, and how it is painted.

use tradoor_core::num::Within;
use tradoor_core::format::fixed;
use tradoor_core::format::format_price;
use tradoor_core::studies::Group;
use tradoor_core::ChartTimeframe;

use crate::color::Color;
use crate::geometry::{distance_to_drawing, distance_to_segment};
use crate::paint::{Align, Baseline, Frame, Rect};
use crate::scale::PriceScale;
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrawingKind {
    Path,
    Brush,
    Trendline,
    Horizontal,
    Ray,
    Rect,
    Fib,
    Text,
    Measure,
    Long,
    Short,
    Channel,
    /// A ring around one point, its label inside: a swing marked, as the
    /// Signal panel marks a pattern's drives.
    Circle,
}

impl DrawingKind {
    pub const ALL: [DrawingKind; 13] =
        [Self::Path, Self::Brush, Self::Trendline, Self::Horizontal, Self::Ray, Self::Rect, Self::Fib, Self::Text, Self::Measure, Self::Long, Self::Short, Self::Channel, Self::Circle];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Brush => "brush",
            Self::Trendline => "trendline",
            Self::Horizontal => "horizontal",
            Self::Ray => "ray",
            Self::Rect => "rect",
            Self::Fib => "fib",
            Self::Text => "text",
            Self::Measure => "measure",
            Self::Long => "long",
            Self::Short => "short",
            Self::Channel => "channel",
            Self::Circle => "circle",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Anchors the kind takes — the minimum for a path or brush.
    pub fn arity(self) -> usize {
        match self {
            Self::Horizontal | Self::Ray | Self::Text | Self::Circle => 1,
            Self::Long | Self::Short | Self::Channel => 3,
            _ => 2,
        }
    }

    pub fn open_ended(self) -> bool {
        matches!(self, Self::Path | Self::Brush)
    }

    pub fn is_position(self) -> bool {
        matches!(self, Self::Long | Self::Short)
    }
}

pub const FIB_LEVELS: [f64; 7] = [0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0];
pub const MAX_POINTS: usize = 2000;
/// A circle's radius on screen, whatever the zoom: it marks a bar, not a range.
const CIRCLE_PX: f32 = 11.0;
/// The narrowest a position box is drawn, so a fresh one is never a sliver.
const MIN_POSITION_WIDTH: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// Epoch ms, on the candles' time axis.
    pub time: f64,
    pub price: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Dash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl Dash {
    pub fn parse(s: &str) -> Self {
        match s {
            "dashed" => Self::Dashed,
            "dotted" => Self::Dotted,
            _ => Self::Solid,
        }
    }
}

/// One line of a fib or a channel: where it sits between the two main lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub value: f64,
    pub on: bool,
    pub color: Option<Color>,
    pub dash: Dash,
}

impl Level {
    /// `value,on,color,dash` lines joined by `|` — `0.5,1,#ff6d00,dashed|…`.
    pub fn parse_list(s: &str) -> Vec<Level> {
        s.split('|')
            .filter_map(|item| {
                let mut parts = item.split(',');
                let value = tradoor_core::format::parse_number(parts.next()?.trim()).filter(|v| v.is_finite())?;
                let on = parts.next().is_none_or(|p| p.trim() != "0");
                let color = parts.next().and_then(Color::parse);
                let dash = parts.next().map_or(Dash::Solid, |d| Dash::parse(d.trim()));
                Some(Level { value, on, color, dash })
            })
            .collect()
    }
}

/// How a drawing looks — every field optional, falling back to the theme and to
/// what suits its kind. Keys and values come from the page as strings.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DrawingStyle {
    pub color: Option<Color>,
    pub width: Option<f32>,
    pub dash: Dash,
    pub fill: Option<Color>,
    pub fill_on: Option<bool>,
    pub extend_left: Option<bool>,
    pub extend_right: Option<bool>,
    pub left_arrow: bool,
    pub right_arrow: bool,
    pub text_color: Option<Color>,
    pub text_size: Option<f32>,
    pub text_bg: Option<Color>,
    pub middle: bool,
    pub price_labels: bool,
    pub stats: bool,
    pub levels: Option<Vec<Level>>,
    pub level_labels: Option<bool>,
    // Positions.
    pub profit_fill: Option<Color>,
    pub stop_fill: Option<Color>,
    pub profit_line: Option<Color>,
    pub stop_line: Option<Color>,
    pub account: Option<f64>,
    pub risk: Option<f64>,
    pub risk_cash: bool,
    pub lot: Option<f64>,
    pub leverage: Option<f64>,
    /// Sized by its margin: moving the stop changes what it risks, not its size.
    pub by_margin: bool,
    pub margin: Option<f64>,
    pub qty_decimals: Option<usize>,
    /// A position's partial takes (`t`), partial cuts (`c`) and adds (`a`):
    /// each a price and a percent of the entry's size.
    pub legs: Vec<(u8, f64, f64)>,
    pub compact: bool,
    /// A position's figures stay up; otherwise they show on hover or selection.
    pub always_stats: bool,
    /// Shown but not picked up: the pointer passes through it.
    pub locked: bool,
    /// The lower pane it is drawn in, by its group (`pct` for RSI and Stoch
    /// RSI, `oi`…), on that pane's values; `None` in the price pane.
    pub pane: Option<Group>,
}

impl DrawingStyle {
    /// Read `(key, value)` pairs; unknown keys and unreadable values are skipped.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut s = Self::default();
        for (key, value) in pairs {
            s.set(key, value);
        }
        s
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let flag = || matches!(value, "1" | "true");
        let color = || Color::parse(value);
        let num = || tradoor_core::format::parse_number(value.trim()).filter(|v| v.is_finite());
        let size = || num().map(|v| v as f32).filter(|v| *v > 0.0);
        match key {
            "color" => self.color = color(),
            "width" => self.width = size().map(|w| w.min(8.0)),
            "dash" => self.dash = Dash::parse(value),
            "fill" => self.fill = color(),
            "fillOn" => self.fill_on = Some(flag()),
            "extendLeft" => self.extend_left = Some(flag()),
            "extendRight" => self.extend_right = Some(flag()),
            "leftArrow" => self.left_arrow = flag(),
            "rightArrow" => self.right_arrow = flag(),
            "textColor" => self.text_color = color(),
            "textSize" => self.text_size = size().map(|s| s.within(6.0, 40.0)),
            "textBg" => self.text_bg = color(),
            "middle" => self.middle = flag(),
            "priceLabels" => self.price_labels = flag(),
            "stats" => self.stats = flag(),
            "levels" => self.levels = Some(Level::parse_list(value)),
            "levelLabels" => self.level_labels = Some(flag()),
            "profitFill" => self.profit_fill = color(),
            "stopFill" => self.stop_fill = color(),
            "profitLine" => self.profit_line = color(),
            "stopLine" => self.stop_line = color(),
            "account" => self.account = num().filter(|v| *v > 0.0),
            "risk" => self.risk = num().filter(|v| *v > 0.0),
            "riskUnit" => self.risk_cash = value != "%",
            "lot" => self.lot = num().filter(|v| *v > 0.0),
            "leverage" => self.leverage = num().filter(|v| *v > 0.0),
            "sizeBy" => self.by_margin = value == "margin",
            "margin" => self.margin = num().filter(|v| *v > 0.0),
            "qtyDecimals" => self.qty_decimals = num().map(|v| v.within(0.0, 10.0) as usize),
            "legs" => {
                self.legs = value
                    .split('|')
                    .filter_map(|part| {
                        let mut f = part.split(',').map(str::trim);
                        let kind = *f.next()?.as_bytes().first()?;
                        let price = tradoor_core::format::parse_number(f.next()?)?;
                        let percent = tradoor_core::format::parse_number(f.next()?)?;
                        (matches!(kind, b't' | b'c' | b'a') && price.is_finite() && percent > 0.0 && percent <= 1000.0).then_some((kind, price, percent))
                    })
                    .take(12)
                    .collect()
            }
            "compact" => self.compact = flag(),
            "alwaysStats" => self.always_stats = flag(),
            "locked" => self.locked = flag(),
            "pane" => self.pane = [Group::Pct, Group::Stoch, Group::Macd, Group::Atr, Group::Oi, Group::Delta].into_iter().find(|g| g.as_str() == value),
            _ => {}
        }
    }

    fn levels_for(&self, kind: DrawingKind) -> Vec<Level> {
        if let Some(levels) = &self.levels {
            return levels.clone();
        }
        // TradingView's defaults: a channel's quarter lines hidden and its middle
        // dashed; a fib's levels each in their own colour.
        let blue = Some(Color::hex(0x2962ff));
        let lvl = |value: f64, on: bool, color: Option<Color>, dash: Dash| Level { value, on, color, dash };
        match kind {
            DrawingKind::Channel => vec![
                lvl(-0.25, false, blue, Dash::Solid),
                lvl(0.0, true, None, Dash::Solid),
                lvl(0.25, false, blue, Dash::Solid),
                lvl(0.5, true, Some(Color::hex(0xff6d00)), Dash::Dashed),
                lvl(0.75, false, blue, Dash::Solid),
                lvl(1.0, true, None, Dash::Solid),
                lvl(1.25, false, blue, Dash::Solid),
            ],
            _ => [
                (0.0, true, 0x787b86),
                (0.236, true, 0xf23645),
                (0.382, true, 0xff9800),
                (0.5, true, 0x4caf50),
                (0.618, true, 0x089981),
                (0.786, true, 0x00bcd4),
                (1.0, true, 0x787b86),
                (1.618, false, 0x2962ff),
                (2.618, false, 0xf23645),
                (3.618, false, 0x9c27b0),
                (4.236, false, 0xe91e63),
            ]
            .into_iter()
            .map(|(v, on, c)| lvl(v, on, Some(Color::hex(c)), Dash::Solid))
            .collect(),
        }
    }

    fn extends(&self, kind: DrawingKind) -> (bool, bool) {
        (self.extend_left.unwrap_or(false), self.extend_right.unwrap_or(kind == DrawingKind::Fib))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingShape {
    /// `None` while it is still being placed.
    pub id: Option<i64>,
    pub kind: DrawingKind,
    pub points: Vec<Anchor>,
    pub label: Option<String>,
    pub style: DrawingStyle,
    /// The timeframe it was drawn at: on a longer one its points stand on the
    /// bars holding their times. `None`: the chart's own.
    pub timeframe: Option<ChartTimeframe>,
}

impl DrawingShape {
    pub fn new(id: Option<i64>, kind: DrawingKind, points: Vec<Anchor>) -> Self {
        Self { id, kind, points, label: None, style: DrawingStyle::default(), timeframe: None }
    }
}

/// A drawing's own numbers use significant digits rather than the instrument's
/// precision: a measure's delta is derived, and seven decimals of it help nobody.
pub fn significant(value: f64) -> String {
    let abs = value.abs();
    if abs >= 1000.0 {
        fixed(value, 0)
    } else if abs >= 1.0 {
        fixed(value, 2)
    } else if abs == 0.0 {
        "0".into()
    } else {
        let decimals = (2 - abs.log10().floor() as i32).max(0) as usize;
        fixed(value, decimals)
    }
}

pub struct Placed<'a> {
    pub shape: &'a DrawingShape,
    pub points: Vec<(f32, f32)>,
    pub selected: bool,
    pub draft: bool,
    /// The pointer is over it.
    pub hovered: bool,
    /// Bars between the first two anchors, for the measure readout.
    pub bars: i64,
    pub scale: PriceScale,
    /// The instrument's price decimals: one tick is `10^-precision`.
    pub precision: usize,
    /// The last close, for a position's open P&L.
    pub last: Option<f64>,
    /// The plot of the pane it stands in.
    pub plot: Rect,
}

impl Placed<'_> {
    fn y(&self, price: f64) -> f32 {
        self.scale.y(price) as f32
    }

    fn price(&self, i: usize) -> f64 {
        self.shape.points.get(i).map_or(f64::NAN, |p| p.price)
    }

    /// A channel's offset from its first line to its second, in price, and the
    /// price of level `f` at each end of the first line.
    /// The channel line `f` of the way from the main line to the third point's.
    /// Offset on screen, not in price, so the lines stay parallel as drawn on a
    /// log scale too — as TradingView keeps them.
    fn channel_line(&self, f: f64) -> Option<((f32, f32), (f32, f32))> {
        let (a, b, c) = (self.points.first()?, self.points.get(1)?, self.points.get(2)?);
        let t = if (b.0 - a.0).abs() > f32::EPSILON { (c.0 - a.0) / (b.0 - a.0) } else { 0.0 };
        let offset = (c.1 - (a.1 + (b.1 - a.1) * t)) * f as f32;
        Some(((a.0, a.1 + offset), (b.0, b.1 + offset)))
    }

    /// A position box: left and right edges, and the entry, stop and target rows.
    fn position_box(&self) -> Option<(f32, f32, f32, f32, f32)> {
        let (a, b, c) = (self.points.first()?, self.points.get(1)?, self.points.get(2)?);
        let left = a.0.min(b.0).min(c.0);
        let right = a.0.max(b.0).max(c.0).max(left + MIN_POSITION_WIDTH);
        Some((left, right, a.1, b.1, c.1))
    }
}

/// Where the grab handles sit. For most kinds these are the anchors; a position
/// has handles on its left edge for the three prices and one on the right for
/// its width, and a channel's third handle rides the middle of its second line.
pub fn handles(d: &Placed<'_>) -> Vec<(f32, f32)> {
    match d.shape.kind {
        DrawingKind::Brush => Vec::new(),
        DrawingKind::Long | DrawingKind::Short => match d.position_box() {
            Some((left, right, entry, stop, target)) => vec![(left, entry), (left, stop), (left, target), (right, entry)],
            None => d.points.clone(),
        },
        DrawingKind::Channel => match d.channel_line(1.0) {
            Some((p, q)) => vec![d.points[0], d.points[1], ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0)],
            None => d.points.clone(),
        },
        _ => d.points.clone(),
    }
}

/// A segment carried out to the plot's edges on the sides asked for.
fn extend(a: (f32, f32), b: (f32, f32), plot: Rect, left: bool, right: bool) -> ((f32, f32), (f32, f32)) {
    let dx = b.0 - a.0;
    if dx.abs() < f32::EPSILON {
        return (a, b);
    }
    let slope = (b.1 - a.1) / dx;
    let at = |x: f32| (x, a.1 + (x - a.0) * slope);
    let (l, r) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    (if left { at(plot.x) } else { l }, if right { at(plot.right()) } else { r })
}

fn stroke(frame: &mut Frame, a: (f32, f32), b: (f32, f32), width: f32, color: Color, dash: Dash) {
    match dash {
        Dash::Dashed => frame.smooth_dashed_line(a.0, a.1, b.0, b.1, width, color, 6.0, 4.0),
        Dash::Dotted => frame.smooth_dashed_line(a.0, a.1, b.0, b.1, width, color, width.max(1.5), 3.0),
        Dash::Solid if a.1 == b.1 => frame.hline(a.0, b.0, a.1, width, color),
        Dash::Solid => frame.smooth_line(a.0, a.1, b.0, b.1, width, color),
    }
}

fn arrow(frame: &mut Frame, from: (f32, f32), tip: (f32, f32), width: f32, color: Color) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = dx.hypot(dy);
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let size = 7.0 + width * 1.5;
    let back = (tip.0 - ux * size, tip.1 - uy * size);
    frame.smooth_line(tip.0, tip.1, back.0 - uy * size * 0.5, back.1 + ux * size * 0.5, width, color);
    frame.smooth_line(tip.0, tip.1, back.0 + uy * size * 0.5, back.1 - ux * size * 0.5, width, color);
}

fn pct(from: f64, to: f64) -> f64 {
    if from == 0.0 {
        0.0
    } else {
        (to - from) / from * 100.0
    }
}

fn qty_text(qty: f64, decimals: Option<usize>) -> String {
    match decimals {
        Some(d) => fixed(qty, d),
        None => significant(qty),
    }
}

pub fn paint(frame: &mut Frame, d: &Placed<'_>, plot: Rect, theme: &Theme) {
    let Some(&a) = d.points.first() else { return };
    let style = &d.shape.style;
    let kind = d.shape.kind;
    let base = style.color.unwrap_or(theme.drawing);
    let color = if d.selected && style.color.is_none() { theme.drawing_selected } else { base };
    let width = style.width.unwrap_or(1.5) + if d.selected { 0.5 } else { 0.0 };
    let dash = if d.draft && style.dash == Dash::Solid { Dash::Dashed } else { style.dash };
    let font = style.text_size.unwrap_or(theme.font_px - 1.0);
    let text_color = style.text_color.unwrap_or(color);
    let fill = if style.fill_on.unwrap_or(true) { Some(style.fill.unwrap_or(base.with_alpha(26))) } else { None };
    let prices: Vec<f64> = d.shape.points.iter().map(|p| p.price).collect();
    let digits = Some(d.precision);
    let label = |frame: &mut Frame, x: f32, y: f32, text: String, align: Align, baseline: Baseline| {
        frame.text(x, y, text, text_color, align, baseline, font, style.text_bg, Some(plot));
    };
    let (b, c) = (d.points.get(1).copied(), d.points.get(2).copied());
    let (ext_left, ext_right) = style.extends(kind);

    match (kind, b, c) {
        (DrawingKind::Path | DrawingKind::Brush, _, _) => {
            let w = if kind == DrawingKind::Brush { width + 0.5 } else { width };
            if d.draft || dash != Dash::Solid {
                for pair in d.points.windows(2) {
                    stroke(frame, pair[0], pair[1], w, color, dash);
                }
            } else {
                frame.polyline(&d.points, w, color);
            }
            if let (Some(pair), true) = (d.points.windows(2).last(), style.right_arrow) {
                arrow(frame, pair[0], pair[1], w, color);
            }
        }
        (DrawingKind::Horizontal, _, _) => {
            stroke(frame, (plot.x, a.1), (plot.right(), a.1), width, color, dash);
            if let Some(text) = &d.shape.label {
                label(frame, plot.x + 6.0, a.1 - 3.0, text.clone(), Align::Left, Baseline::Bottom);
            }
            if style.price_labels {
                frame.text(plot.right() - 4.0, a.1 - 2.0, format_price(prices[0], digits), text_color, Align::Right, Baseline::Bottom, font, None, Some(plot));
            }
        }
        (DrawingKind::Ray, _, _) => {
            stroke(frame, a, (plot.right(), a.1), width, color, dash);
            if let Some(text) = &d.shape.label {
                label(frame, a.0 + 6.0, a.1 - 3.0, text.clone(), Align::Left, Baseline::Bottom);
            }
            if style.price_labels {
                frame.text(a.0 + 6.0, a.1 + 2.0, format_price(prices[0], digits), text_color, Align::Left, Baseline::Top, font, None, Some(plot));
            }
        }
        (DrawingKind::Circle, _, _) => {
            if let Some(fill) = fill {
                frame.circle(a.0, a.1, CIRCLE_PX, fill);
            }
            frame.ring(a.0, a.1, CIRCLE_PX, width, color);
            if let Some(text) = &d.shape.label {
                frame.text(a.0, a.1, text.clone(), text_color, Align::Center, Baseline::Middle, font, None, Some(plot));
            }
        }
        (DrawingKind::Text, _, _) => {
            if d.selected || d.draft {
                frame.vline(a.0, a.1 - 4.0, a.1 + 4.0, width, color);
            }
            let text = d.shape.label.clone().unwrap_or_default();
            let size = style.text_size.unwrap_or(theme.font_px + 1.0);
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let y = a.1 - 2.0 - (lines.len() - 1 - i) as f32 * (size + 4.0);
                frame.text(a.0 + 6.0, y, *line, style.text_color.unwrap_or(color), Align::Left, Baseline::Bottom, size, style.text_bg, Some(plot));
            }
        }
        (DrawingKind::Rect, Some(b), _) => {
            if let Some(fill) = fill {
                frame.rect(a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs(), fill);
            }
            let (l, r, t, btm) = (a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1));
            let (l2, r2) = (if ext_left { plot.x } else { l }, if ext_right { plot.right() } else { r });
            stroke(frame, (l2, t), (r2, t), width, color, dash);
            stroke(frame, (l2, btm), (r2, btm), width, color, dash);
            stroke(frame, (l, t), (l, btm), width, color, dash);
            stroke(frame, (r, t), (r, btm), width, color, dash);
            if style.middle {
                stroke(frame, (l2, (t + btm) / 2.0), (r2, (t + btm) / 2.0), width.min(1.0), color, Dash::Dashed);
            }
            if let Some(text) = &d.shape.label {
                label(frame, l + 4.0, t + 3.0, text.clone(), Align::Left, Baseline::Top);
            }
        }
        (DrawingKind::Fib, Some(b), _) => {
            let (left, right) = (a.0.min(b.0), a.0.max(b.0));
            let (x0, x1) = (if ext_left { plot.x } else { left }, if ext_right { plot.right() } else { right });
            let levels = style.levels_for(kind);
            let mut on: Vec<&Level> = levels.iter().filter(|l| l.on).collect();
            tradoor_core::sort::insertion_sort_by(&mut on, |p, q| p.value < q.value);
            if let Some(fill) = fill.filter(|_| style.fill_on == Some(true)) {
                for pair in on.windows(2) {
                    let y0 = a.1 + (b.1 - a.1) * pair[0].value as f32;
                    let y1 = a.1 + (b.1 - a.1) * pair[1].value as f32;
                    frame.rect(x0, y0.min(y1), x1 - x0, (y1 - y0).abs(), pair[0].color.unwrap_or(fill).with_alpha(fill.a));
                }
            }
            if !d.draft {
                stroke(frame, a, b, 1.0, color.with_alpha(110), Dash::Dashed);
            }
            for level in on {
                let y = a.1 + (b.1 - a.1) * level.value as f32;
                let price = prices[0] + (prices[1] - prices[0]) * level.value;
                let c = level.color.unwrap_or(if level.value == 0.0 || level.value == 1.0 { color } else { color.with_alpha(180) });
                stroke(frame, (x0, y), (x1, y), width, c, if d.draft { Dash::Dashed } else { level.dash });
                if style.level_labels.unwrap_or(true) {
                    frame.text(x0 + 4.0, y - 2.0, format!("{}  ({})", trim_level(level.value), format_price(price, digits)), style.text_color.unwrap_or(c), Align::Left, Baseline::Bottom, font, None, Some(plot));
                }
            }
        }
        (DrawingKind::Measure, Some(b), _) => {
            let up = prices[1] >= prices[0];
            let tone = style.color.unwrap_or(if up { theme.reward } else { theme.risk });
            frame.rect(a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs(), style.fill.unwrap_or(tone.with_alpha(40)));
            let mid_y = (a.1 + b.1) / 2.0;
            let mid_x = (a.0 + b.0) / 2.0;
            stroke(frame, (a.0, mid_y), (b.0, mid_y), 1.0, tone, Dash::Solid);
            stroke(frame, (mid_x, a.1), (mid_x, b.1), 1.0, tone, Dash::Solid);
            arrow(frame, (a.0, mid_y), (b.0, mid_y), 1.0, tone);
            arrow(frame, (mid_x, a.1), (mid_x, b.1), 1.0, tone);
            let (from, to) = (prices[0], prices[1]);
            let sign = if up { "+" } else { "" };
            let bars = d.bars.abs();
            let ticks = ((to - from) / 10f64.powi(-(d.precision as i32))).round();
            let text = format!("{sign}{} ({sign}{}%) {sign}{}, {bars} bar{}", format_price(to - from, digits), fixed(pct(from, to), 2), fixed(ticks, 0), if bars == 1 { "" } else { "s" });
            let y = if up { a.1.min(b.1) - 6.0 } else { a.1.max(b.1) + 6.0 };
            frame.text(mid_x, y, text, tone.contrast_text(), Align::Center, if up { Baseline::Bottom } else { Baseline::Top }, font, Some(tone), Some(plot));
        }
        (DrawingKind::Long | DrawingKind::Short, Some(_), Some(_)) => paint_position(frame, d, plot, theme, font),
        (DrawingKind::Channel, Some(b), None) => stroke(frame, a, b, width, color, dash),
        (DrawingKind::Channel, Some(_), Some(_)) => {
            let levels = style.levels_for(kind);
            let line = |f: f64| d.channel_line(f).map(|(p, q)| extend(p, q, plot, ext_left, ext_right));
            if let (Some(fill), Some((p0, q0)), Some((p1, q1))) = (fill, line(0.0), line(1.0)) {
                // Two triangles: the band between the main lines.
                frame.triangle(p0, q0, q1, fill);
                frame.triangle(p0, q1, p1, fill);
            }
            for level in levels.iter().filter(|l| l.on || l.value == 0.0 || l.value == 1.0) {
                let Some((p, q)) = line(level.value) else { continue };
                let main = level.value == 0.0 || level.value == 1.0;
                let c = level.color.unwrap_or(if main { color } else { theme.drawing_selected.with_alpha(200) });
                stroke(frame, p, q, if main { width } else { width.min(1.5) }, c, if d.draft { Dash::Dashed } else { level.dash });
            }
            if let (Some(text), Some((p, q))) = (&d.shape.label, line(1.0)) {
                label(frame, (p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0 - 4.0, text.clone(), Align::Center, Baseline::Bottom);
            }
        }
        (_, Some(b), _) => {
            // Trendline.
            let (p, q) = extend(a, b, plot, ext_left, ext_right);
            stroke(frame, p, q, width, color, dash);
            if style.left_arrow {
                arrow(frame, b, a, width, color);
            }
            if style.right_arrow {
                arrow(frame, a, b, width, color);
            }
            if style.middle {
                let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                frame.circle(m.0, m.1, 3.0 + width / 2.0, color);
            }
            if style.price_labels {
                for (i, pt) in [a, b].iter().enumerate() {
                    let above = pt.1 <= [a, b][1 - i].1;
                    frame.text(pt.0, pt.1 + if above { -6.0 } else { 6.0 }, format_price(prices[i], digits), color.contrast_text(), Align::Center, if above { Baseline::Bottom } else { Baseline::Top }, font, Some(color), Some(plot));
                }
            }
            if let Some(text) = &d.shape.label {
                label(frame, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0 - 4.0, text.clone(), Align::Center, Baseline::Bottom);
            }
            if style.stats && !d.draft {
                let (from, to) = (prices[0], prices[1]);
                let sign = if to >= from { "+" } else { "" };
                let bars = d.bars.abs();
                let text = format!("{sign}{} ({sign}{}%), {bars} bar{}", format_price(to - from, digits), fixed(pct(from, to), 2), if bars == 1 { "" } else { "s" });
                let right = if b.0 >= a.0 { b } else { a };
                frame.text(right.0 + 8.0, right.1, text, text_color, Align::Left, Baseline::Middle, font, Some(theme.background.with_alpha(210)), Some(plot));
            }
        }
        _ => {}
    }

    if d.selected {
        for p in handles(d) {
            frame.circle(p.0, p.1, 5.0, theme.drawing_selected);
            frame.circle(p.0, p.1, 3.5, theme.handle);
        }
    }
}

/// `0.618` rather than `0.6180000`, `1` rather than `1.0`.
fn trim_level(v: f64) -> String {
    let s = fixed(v, 3);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The coins a position's entry buys (bins/server journal, `coins_of`): what its
/// margin buys at its leverage, or what its risk buys over its stop.
fn position_coins(style: &DrawingStyle, entry: f64, stop: f64) -> f64 {
    let risk_per = (entry - stop).abs();
    if style.by_margin {
        if entry > 0.0 { style.margin.unwrap_or(100.0) * style.leverage.unwrap_or(1.0) / entry } else { 0.0 }
    } else if risk_per > 0.0 {
        let risk = if style.risk_cash { style.risk.unwrap_or(10.0) } else { style.account.unwrap_or(1000.0) * style.risk.unwrap_or(1.0) / 100.0 };
        risk / risk_per
    } else {
        0.0
    }
}

/// TradingView's long and short position: a target box and a stop box either
/// side of the entry, with the quantity it holds and what that makes.
fn paint_position(frame: &mut Frame, d: &Placed<'_>, plot: Rect, theme: &Theme, font: f32) {
    let Some((left, right, entry_y, stop_y, target_y)) = d.position_box() else { return };
    let style = &d.shape.style;
    let long = d.shape.kind == DrawingKind::Long;
    let (entry, stop, target) = (d.price(0), d.price(1), d.price(2));
    let profit_line = style.profit_line.unwrap_or(theme.reward);
    let stop_line = style.stop_line.unwrap_or(theme.risk);
    let profit_fill = style.profit_fill.unwrap_or(profit_line.with_alpha(56));
    let stop_fill = style.stop_fill.unwrap_or(stop_line.with_alpha(56));
    let w = right - left;

    frame.rect(left, target_y.min(entry_y), w, (target_y - entry_y).abs(), profit_fill);
    frame.rect(left, stop_y.min(entry_y), w, (stop_y - entry_y).abs(), stop_fill);
    let entry_line = style.color.unwrap_or(Color::hex(0x9598a1));
    frame.hline(left, right, entry_y, 1.0, entry_line);
    if d.selected {
        frame.hline(left, right, target_y, 1.0, profit_line);
        frame.hline(left, right, stop_y, 1.0, stop_line);
    }
    // Its name over the box, or under it where the top is off the plot: a journal
    // trade drawn where it went says which and how it did (web journal/picture.ts).
    if let Some(text) = &d.shape.label {
        let (top, bottom) = (target_y.min(stop_y), target_y.max(stop_y));
        let (y, baseline) = if top - font - 6.0 < plot.y { (bottom + 4.0, Baseline::Top) } else { (top - 4.0, Baseline::Bottom) };
        frame.text(left, y, text.clone(), theme.text, Align::Left, baseline, font, None, Some(plot));
    }
    // Its legs, those its prices allow (bins/server journal, `legs_of`): a
    // dashed line each across the box, named once the figures are up.
    let between = |p: f64, a: f64, b: f64| p > a.min(b) && p < a.max(b);
    let legs: Vec<(u8, f64, f64)> = style
        .legs
        .iter()
        .copied()
        .filter(|&(k, p, _)| match k {
            b't' => between(p, entry, target),
            b'c' => between(p, entry, stop),
            _ => between(p, stop, target) && p != entry,
        })
        .collect();
    let figures = style.always_stats || d.selected || d.hovered || d.draft;
    for &(k, p, percent) in &legs {
        let (color, name) = match k {
            b't' => (profit_line, "TP"),
            b'c' => (stop_line, "SL"),
            _ => (entry_line, "Add"),
        };
        let y = d.y(p);
        frame.dashed_line(left, y, right, y, 1.0, color, 4.0, 3.0);
        if figures {
            frame.text(right - 4.0, y - 2.0, format!("{name} {}%", trim_level(percent)), color, Align::Right, Baseline::Bottom, font, None, Some(plot));
        }
    }
    // As on TradingView: the figures come up under the pointer, or once selected.
    if !figures {
        return;
    }

    let tick = 10f64.powi(-(d.precision as i32));
    let risk_per = (entry - stop).abs();
    let reward_per = (target - entry).abs();
    let lot = style.lot.unwrap_or(1.0);
    let qty = position_coins(style, entry, stop) / lot;
    let rr = if risk_per > 0.0 { reward_per / risk_per } else { 0.0 };
    let digits = Some(d.precision);
    let cx = left + w / 2.0;
    let text = style.text_color;
    let tag = |frame: &mut Frame, y: f32, s: String, bg: Color, baseline: Baseline| {
        frame.text(cx, y, s, text.unwrap_or(bg.contrast_text()), Align::Center, baseline, font, Some(bg), Some(plot));
    };
    let line = |name: &str, price: f64, amount: f64| {
        let move_pct = pct(entry, price).abs();
        let ticks = ((price - entry).abs() / tick).round();
        if style.compact {
            format!("{} ({}%) {}", format_price(price, digits), fixed(move_pct, 2), significant(amount))
        } else {
            format!("{name}: {} ({}%) {}, Amount: {}", format_price(price, digits), fixed(move_pct, 2), fixed(ticks, 0), significant(amount))
        }
    };
    let target_above = target_y <= entry_y;
    // What running to the target makes and to the stop loses, the legs on the way filled.
    let dir = if long { 1.0 } else { -1.0 };
    let made = |end: f64| run_to(entry, end, qty * lot, &legs, dir).abs();
    tag(frame, target_y + if target_above { -5.0 } else { 5.0 }, line("Target", target, made(target)), profit_line, if target_above { Baseline::Bottom } else { Baseline::Top });
    tag(frame, stop_y + if target_above { 5.0 } else { -5.0 }, line("Stop", stop, made(stop)), stop_line, if target_above { Baseline::Top } else { Baseline::Bottom });

    let qty = qty_text(qty, style.qty_decimals);
    let (first, second) = match d.last.filter(|_| !d.draft) {
        Some(last) if !style.compact => {
            let pnl = (last - entry) * if long { 1.0 } else { -1.0 } * lot;
            let q = tradoor_core::format::parse_number(&qty).unwrap_or(0.0);
            (format!("Open P&L: {}, Qty: {qty}", significant(pnl * q)), format!("Risk/Reward Ratio: {}", fixed(rr, 2)))
        }
        _ if style.compact => (qty.clone(), fixed(rr, 2)),
        _ => (format!("Qty: {qty}"), format!("Risk/Reward Ratio: {}", fixed(rr, 2))),
    };
    let grey = Color::hex(0x9598a1);
    let (up, down) = if target_above { (Baseline::Top, Baseline::Top) } else { (Baseline::Bottom, Baseline::Bottom) };
    let row = font + 5.0;
    if target_above {
        tag(frame, entry_y + 4.0, first, grey, up);
        tag(frame, entry_y + 4.0 + row, second, grey, up);
    } else {
        tag(frame, entry_y - 4.0 - row, first, grey, down);
        tag(frame, entry_y - 4.0, second, grey, down);
    }
}

/// What a position of `qty` coins makes running from its entry to `end`, the
/// legs on the way filled, against their average entry: all of it closed by
/// then, so the order they fill in does not count — but for takes and cuts
/// asking more than was bought, scaled down to it. (Unsorted: a sort here is
/// kilobytes of engine.)
fn run_to(entry: f64, end: f64, qty: f64, legs: &[(u8, f64, f64)], dir: f64) -> f64 {
    let (mut bought, mut cost, mut out, mut proceeds) = (qty, entry * qty, 0.0, 0.0);
    for &(kind, price, percent) in legs.iter().filter(|l| (l.1 - entry) * (end - entry) > 0.0) {
        let q = percent / 100.0 * qty;
        if kind == b'a' {
            bought += q;
            cost += price * q;
        } else {
            out += q;
            proceeds += price * q;
        }
    }
    if out > bought {
        proceeds *= bought / out;
        out = bought;
    }
    proceeds += end * (bought - out);
    (proceeds - cost) * dir
}

/// How far the pointer is from a drawing, in pixels — zero inside a filled
/// shape that is picked by its body.
pub fn distance(d: &Placed<'_>, cursor: (f32, f32), plot: Rect) -> f32 {
    let style = &d.shape.style;
    let kind = d.shape.kind;
    let (ext_left, ext_right) = style.extends(kind);
    match kind {
        DrawingKind::Long | DrawingKind::Short => match d.position_box() {
            Some((left, right, _, stop, target)) => {
                let (top, bottom) = (stop.min(target), stop.max(target));
                if cursor.0 >= left && cursor.0 <= right && cursor.1 >= top && cursor.1 <= bottom {
                    0.0
                } else {
                    let dx = (left - cursor.0).max(cursor.0 - right).max(0.0);
                    let dy = (top - cursor.1).max(cursor.1 - bottom).max(0.0);
                    dx.hypot(dy)
                }
            }
            None => f32::INFINITY,
        },
        DrawingKind::Channel if d.points.len() >= 3 => {
            let levels = style.levels_for(kind);
            let line = |f: f64| d.channel_line(f).map(|(p, q)| extend(p, q, plot, ext_left, ext_right));
            let mut best = levels
                .iter()
                .filter(|l| l.on || l.value == 0.0 || l.value == 1.0)
                .filter_map(|l| line(l.value))
                .map(|(p, q)| distance_to_segment(cursor, p, q))
                .fold(f32::INFINITY, f32::min);
            if let (true, Some((p0, q0)), Some((p1, _))) = (style.fill_on.unwrap_or(true), line(0.0), line(1.0)) {
                let span = q0.0 - p0.0;
                if span.abs() > f32::EPSILON && (cursor.0 - p0.0) / span >= 0.0 && (cursor.0 - p0.0) / span <= 1.0 {
                    let t = (cursor.0 - p0.0) / span;
                    let y0 = p0.1 + (q0.1 - p0.1) * t;
                    let y1 = y0 + (p1.1 - p0.1);
                    if cursor.1 >= y0.min(y1) && cursor.1 <= y0.max(y1) {
                        best = 0.0;
                    }
                }
            }
            best
        }
        DrawingKind::Fib if d.points.len() >= 2 => {
            let (a, b) = (d.points[0], d.points[1]);
            let (left, right) = (a.0.min(b.0), a.0.max(b.0));
            let (x0, x1) = (if ext_left { plot.x } else { left }, if ext_right { plot.right() } else { right });
            if cursor.0 < x0 - 4.0 || cursor.0 > x1 + 4.0 {
                return f32::INFINITY;
            }
            style.levels_for(kind).iter().filter(|l| l.on).map(|l| (cursor.1 - (a.1 + (b.1 - a.1) * l.value as f32)).abs()).fold(f32::INFINITY, f32::min)
        }
        DrawingKind::Trendline if d.points.len() >= 2 => {
            let (p, q) = extend(d.points[0], d.points[1], plot, ext_left, ext_right);
            distance_to_segment(cursor, p, q)
        }
        DrawingKind::Circle => ((cursor.0 - d.points[0].0).hypot(cursor.1 - d.points[0].1) - CIRCLE_PX).max(0.0),
        _ => distance_to_drawing(kind.as_str(), &d.points, cursor, &FIB_LEVELS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_positions_legs_change_what_its_target_and_stop_come_to() {
        let s = DrawingStyle::from_pairs([("legs", "t,110,50|a,95,100|x,1,1|c,96")]);
        assert_eq!(s.legs, vec![(b't', 110.0, 50.0), (b'a', 95.0, 100.0)]);
        assert_eq!(run_to(100.0, 120.0, 2.0, &[], 1.0), 40.0);
        assert_eq!(run_to(100.0, 120.0, 2.0, &s.legs, 1.0), 30.0, "half taken at 110, half run to 120; the add is not on the way");
        assert_eq!(run_to(100.0, 90.0, 2.0, &s.legs, 1.0), -30.0, "two more coins at 95, all four stopped at 90");
        assert_eq!(run_to(100.0, 90.0, 1.0, &[(b'c', 95.0, 50.0)], -1.0), 7.5, "a short gains as it falls");
    }

    #[test]
    fn kinds_and_significant_digits() {
        for k in DrawingKind::ALL {
            assert_eq!(DrawingKind::parse(k.as_str()), Some(k));
            assert!(k.arity() >= 1);
        }
        assert_eq!(significant(77_123.4), "77123");
        assert_eq!(significant(12.345), "12.35");
        assert_eq!(significant(0.000012345), "0.0000123");
        assert_eq!(significant(-0.5), "-0.500");
    }

    #[test]
    fn styles_read_from_strings() {
        let margin = DrawingStyle::from_pairs([("sizeBy", "margin"), ("margin", "200"), ("leverage", "5")]);
        assert_eq!(position_coins(&margin, 100.0, 90.0), position_coins(&margin, 100.0, 50.0), "the stop moves what it risks, not its size");
        assert_eq!(position_coins(&margin, 100.0, 90.0), 10.0);
        assert_eq!(position_coins(&DrawingStyle::from_pairs([("riskUnit", "cash"), ("risk", "20")]), 100.0, 90.0), 2.0, "its risk over its stop");
        let s = DrawingStyle::from_pairs([("color", "#ff0000"), ("width", "3"), ("dash", "dotted"), ("extendRight", "1"), ("levels", "0,1|0.5,0,#00ff00,dashed|bad"), ("riskUnit", "cash"), ("nope", "x")]);
        assert_eq!(s.color, Some(Color::hex(0xff0000)));
        assert_eq!((s.width, s.dash, s.extend_right, s.risk_cash), (Some(3.0), Dash::Dotted, Some(true), true));
        let levels = s.levels.unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], Level { value: 0.5, on: false, color: Some(Color::hex(0x00ff00)), dash: Dash::Dashed });
        assert_eq!(trim_level(0.618), "0.618");
        assert_eq!(trim_level(1.0), "1");
    }
}
