//! tradoor-wasm — what the browser worker calls.
//!
//! Numbers cross the boundary as typed arrays and plain values, never as JSON:
//! the browser's own `JSON.parse` reads exchange messages, and this side
//! receives only what it needs. The chart itself lives in `tradoor-render`;
//! this crate uploads its frames.

mod gl;
mod js;
mod text;

use js_sys::{Array, Object};
use tradoor_codec::TickCandles;
use tradoor_core::chart::{engine_series, profile_overlay, profile_spec_for};
use tradoor_core::indicators::profile::ProfileBasis;
use tradoor_core::indicators::{rsi, stoch_rsi};
use tradoor_core::live::{Kline, LiveBar, LiveFeed};
use tradoor_core::studies::STUDIES;
use tradoor_core::timeframe::fold;
use tradoor_core::{ChartTimeframe, Interval};
use tradoor_render::drawings::Dash;
use tradoor_render::{Anchor, Chart, DrawingKind, DrawingShape, DrawingStyle, Event, Magnet};
use wasm_bindgen::prelude::*;

use crate::js::{candles, columns, get, obj, opt, set};

#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&JsValue::from_str(&format!("tradoor engine panic: {info}")))));
}

fn timeframe(tf: &str) -> Result<ChartTimeframe, JsValue> {
    ChartTimeframe::parse(tf).ok_or_else(|| JsValue::from_str(&format!("unknown timeframe: {tf}")))
}

/// A drawing's style: a flat object of strings (numbers and booleans are read too).
fn style(value: &JsValue) -> DrawingStyle {
    let entries = Object::entries(&Object::from(value.clone()));
    let pairs: Vec<(String, String)> = entries
        .iter()
        .filter_map(|e| {
            let e = Array::from(&e);
            let key = e.get(0).as_string()?;
            let v = e.get(1);
            let text = v.as_string().or_else(|| v.as_f64().map(|n| n.to_string())).or_else(|| v.as_bool().map(|b| if b { "1".into() } else { "0".into() }))?;
            Some((key, text))
        })
        .collect();
    DrawingStyle::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

#[wasm_bindgen]
pub struct ChartEngine {
    chart: Chart,
    gl: gl::Surface,
    text: text::TextLayer,
    /// The liquidation feed, on a 2D canvas over the other two.
    feed: text::TextLayer,
    /// The size it is shown at, to go back to after drawing a picture larger.
    size: (f32, f32, f32),
}

#[wasm_bindgen]
impl ChartEngine {
    #[wasm_bindgen(constructor)]
    /// `flat` draws shapes with the 2D canvas even where WebGL2 is there — the
    /// path older iPhones take, forced so it can be looked at anywhere.
    pub fn new(gl_canvas: web_sys::OffscreenCanvas, text_canvas: web_sys::OffscreenCanvas, feed_canvas: web_sys::OffscreenCanvas, tf: &str, flat: bool) -> Result<ChartEngine, JsValue> {
        let mut chart = Chart::new(timeframe(tf)?);
        let text = text::TextLayer::new(text_canvas)?;
        let feed = text::TextLayer::new(feed_canvas)?;
        chart.set_char_width(text.char_width(chart.theme().font_px));
        let gl = if flat { gl::Surface::Flat(gl::Flat::new(gl_canvas)?) } else { gl::Surface::new(gl_canvas)? };
        chart.set_smooth_lines(matches!(gl, gl::Surface::Gpu(_)));
        Ok(Self { chart, gl, text, feed, size: (0.0, 0.0, 1.0) })
    }

    /// The shapes' GPU context is back after the browser took it away: made
    /// ready again, and the chart drawn anew on the next frame.
    pub fn restore_gpu(&mut self) -> Result<(), JsValue> {
        self.gl.restore()?;
        self.chart.touch();
        Ok(())
    }

    /// What draws the shapes: `webgl2`, or `2d` where a worker has no WebGL2.
    pub fn renderer(&self) -> String {
        self.gl.kind().to_string()
    }

    pub fn resize(&mut self, width: f32, height: f32, dpr: f32) {
        self.size = (width, height, dpr);
        self.chart.resize(width, height, dpr);
    }

    /// Draw the chart as it stands into `canvas` at `scale` the pixels, for a
    /// picture to keep. The 2D renderer draws it: a still costs nothing to draw
    /// slowly, and its pixels can be read back where a GPU canvas in a worker
    /// cannot be counted on to hold them.
    pub fn draw_into(&mut self, shapes: web_sys::OffscreenCanvas, labels: web_sys::OffscreenCanvas, scale: f32, now: f64) -> Result<(), JsValue> {
        let (width, height, dpr) = self.size;
        let mut surface = gl::Surface::Flat(gl::Flat::new(shapes)?);
        let mut text = text::TextLayer::new(labels)?;
        let smooth = matches!(self.gl, gl::Surface::Gpu(_));
        self.chart.set_smooth_lines(false);
        self.chart.resize(width, height, dpr * scale);
        self.chart.touch();
        let frame = self.chart.render(now as i64);
        surface.draw(frame);
        text.draw(frame);
        text.draw_layer(self.chart.render_feed(now as i64), false);
        // Back to the size and the renderer it is shown at.
        self.chart.set_smooth_lines(smooth);
        self.chart.resize(width, height, dpr);
        self.chart.touch();
        Ok(())
    }

    pub fn set_timeframe(&mut self, tf: &str) -> Result<(), JsValue> {
        self.chart.set_timeframe(timeframe(tf)?);
        Ok(())
    }

    pub fn set_candles(&mut self, time: &[f64], open: &[f64], high: &[f64], low: &[f64], close: &[f64], volume: &[f64]) {
        self.chart.set_candles(candles(time, open, high, low, close, volume));
    }

    /// The daily bars the chart's own were folded from, for key levels that open
    /// inside a bar longer than a day. Empty clears them.
    pub fn set_days(&mut self, time: &[f64], open: &[f64], high: &[f64], low: &[f64], close: &[f64]) {
        let volume = vec![0.0; time.len()];
        self.chart.set_days(candles(time, open, high, low, close, &volume));
    }

    pub fn set_live(&mut self, open_time: f64, open: f64, high: f64, low: f64, close: f64, volume: f64) {
        self.chart.set_live(Some(LiveBar { open_time: open_time as i64, open, high, low, close, volume, closed: false }));
    }

    pub fn clear_live(&mut self) {
        self.chart.set_live(None);
    }

    /// One liquidation to ping beside the candles: `at` is when it reached us.
    pub fn push_liquidation(&mut self, usd: f64, long: bool, mark: u8, at: f64) {
        self.chart.push_liquidation(usd, long, mark, at as i64);
    }

    /// Exchange `index`'s mark, drawn on the feed rows of hits from there.
    pub fn set_mark(&mut self, index: u8, box_size: f32, spec: &str) {
        self.text.set_mark(index, box_size, spec);
        self.feed.set_mark(index, box_size, spec);
        self.chart.touch();
    }

    pub fn clear_liquidations(&mut self) {
        self.chart.clear_liquidations();
    }

    pub fn set_studies(&mut self, raw: &str) {
        self.chart.set_studies(raw);
    }

    /// Summed open interest from the server: candles of one step, oldest first.
    pub fn set_open_interest(&mut self, time: &[f64], open: &[f64], high: &[f64], low: &[f64], close: &[f64]) {
        let n = time.len().min(open.len()).min(high.len()).min(low.len()).min(close.len());
        self.chart.set_open_interest((0..n).map(|i| (time[i] as i64, [open[i], high[i], low[i], close[i]])).collect());
    }

    pub fn set_hidden(&mut self, raw: &str) {
        self.chart.set_hidden(raw);
    }

    /// TPO options as `key=value;key=value`, named as Exocharts names them.
    pub fn set_tpo_settings(&mut self, raw: &str) {
        self.chart.set_tpo_settings(raw);
    }

    /// The reader's time zone, as minutes east of UTC.
    pub fn set_utc_offset(&mut self, minutes: i32) {
        self.chart.set_utc_offset(minutes);
    }

    /// `1d`, `1w` or `1M` draws that session as TPO profiles; empty, candles.
    pub fn set_tpo(&mut self, raw: &str) {
        self.chart.set_tpo(raw);
    }

    pub fn set_log_scale(&mut self, log: bool) {
        self.chart.set_log_scale(log);
    }

    /// Negative clears the override.
    pub fn set_price_decimals(&mut self, decimals: i32) {
        self.chart.set_price_decimals(usize::try_from(decimals).ok());
    }

    pub fn set_pane_weights(&mut self, raw: &str) {
        self.chart.set_pane_weights(raw);
    }

    pub fn pane_weights(&self) -> String {
        self.chart.pane_weights()
    }

    pub fn set_theme_value(&mut self, key: &str, value: &str) -> bool {
        let ok = self.chart.set_theme_value(key, value);
        if ok && key == "fontPx" {
            let width = self.text.char_width(self.chart.theme().font_px);
            self.chart.set_char_width(width);
        }
        ok
    }

    /// Another market's candles are on the way: hold what is drawn, dimmed.
    pub fn set_loading(&mut self, loading: bool) {
        self.chart.set_loading(loading);
    }

    pub fn set_message(&mut self, message: &str) {
        self.chart.set_message((!message.is_empty()).then(|| message.to_string()));
    }

    /// `[{id, kind, points: [{time, price}], label, style: {key: string}}]`.
    pub fn set_drawings(&mut self, list: &Array) {
        let shapes = list
            .iter()
            .filter_map(|v| {
                let kind = DrawingKind::parse(&get(&v, "kind")?.as_string()?)?;
                let points = Array::from(&get(&v, "points")?)
                    .iter()
                    .filter_map(|p| Some(Anchor { time: get(&p, "time")?.as_f64()?, price: get(&p, "price")?.as_f64()? }))
                    .collect();
                Some(DrawingShape {
                    id: get(&v, "id").and_then(|i| i.as_f64()).map(|i| i as i64),
                    kind,
                    points,
                    label: get(&v, "label").and_then(|l| l.as_string()),
                    style: get(&v, "style").map_or_else(DrawingStyle::default, |s| style(&s)),
                    timeframe: get(&v, "timeframe").and_then(|t| t.as_string()).and_then(|t| ChartTimeframe::parse(&t)),
                })
            })
            .collect();
        self.chart.set_drawings(shapes);
    }

    /// NaN clears the selection.
    pub fn set_selected(&mut self, id: f64) {
        self.chart.set_selected((!id.is_nan()).then_some(id as i64));
    }

    /// An empty string selects the pointer; `style` is what the new drawing will get.
    pub fn set_tool(&mut self, tool: &str, tool_style: &JsValue) {
        let s = if tool_style.is_object() { style(tool_style) } else { DrawingStyle::default() };
        self.chart.set_tool_styled(DrawingKind::parse(tool), s);
    }

    pub fn set_shift(&mut self, shift: bool) {
        self.chart.set_shift(shift);
    }

    pub fn set_ctrl(&mut self, ctrl: bool) {
        self.chart.set_ctrl(ctrl);
    }

    /// The press under way is a finger's: alerts wait for a hold.
    pub fn set_touch(&mut self, touch: bool) {
        self.chart.set_touch(touch);
    }

    /// `off`, `weak` or `strong`.
    pub fn set_magnet(&mut self, mode: &str) {
        self.chart.set_magnet(match mode {
            "weak" => Magnet::Weak,
            "strong" => Magnet::Strong,
            _ => Magnet::Off,
        });
    }

    pub fn reset_view(&mut self) {
        self.chart.reset_view();
    }

    pub fn auto_scale(&self) -> bool {
        self.chart.auto_scale()
    }

    pub fn set_auto_scale(&mut self) {
        self.chart.set_auto_scale();
    }

    /// NaN shows every bar.
    pub fn show_range(&mut self, days: f64) {
        self.chart.show_range((!days.is_nan()).then_some(days));
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.chart.pointer_move(x, y);
    }

    pub fn pointer_down(&mut self, x: f32, y: f32, button: u16) {
        self.chart.pointer_down(x, y, button);
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) {
        self.chart.pointer_up(x, y);
    }

    pub fn pointer_hold(&mut self, x: f32, y: f32) {
        self.chart.pointer_hold(x, y);
    }

    pub fn pointer_leave(&mut self) {
        self.chart.pointer_leave();
    }

    /// The open time under this chart's own crosshair, for the other charts of
    /// a layout keeping them in step; undefined while it has none.
    pub fn crosshair_time(&self) -> Option<f64> {
        self.chart.crosshair_time().map(|t| t as f64)
    }

    /// Another chart's crosshair time, drawn here; undefined takes it away.
    pub fn set_linked_crosshair(&mut self, time: Option<f64>) {
        self.chart.set_linked_crosshair(time.map(|t| t as i64));
    }

    /// The time at the plot's right edge; undefined before there are bars.
    pub fn edge_time(&self) -> Option<f64> {
        self.chart.edge_time().map(|t| t as f64)
    }

    /// Scrolled so the right edge is where another chart of the layout has its own.
    pub fn set_edge_time(&mut self, time: f64) {
        self.chart.set_edge_time(time as i64);
    }

    pub fn double_click(&mut self, x: f32, y: f32) {
        self.chart.double_click(x, y);
    }

    pub fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        self.chart.wheel(x, y, dx, dy)
    }

    pub fn key(&mut self, key: &str) -> bool {
        self.chart.key(key)
    }

    pub fn cursor(&self) -> String {
        self.chart.cursor().css().to_string()
    }

    pub fn visible_from(&self) -> f64 {
        self.chart.visible_from()
    }

    pub fn precision(&self) -> u32 {
        self.chart.precision() as u32
    }

    /// Draw what changed since the last frame: the chart, the liquidation feed
    /// over it, or both. Returns whether the chart was drawn.
    pub fn draw(&mut self, now: f64) -> bool {
        let now = now as i64;
        let chart = self.chart.needs_render(now);
        if chart {
            let frame = self.chart.render(now);
            self.gl.draw(frame);
            self.text.draw(frame);
        }
        // Over a chart drawn anew, whose panes may have moved, or as a hit moves.
        if chart || self.chart.needs_feed(now) {
            self.feed.draw_layer(self.chart.render_feed(now), true);
        }
        chart
    }

    pub fn take_events(&mut self) -> Array {
        self.chart
            .take_events()
            .into_iter()
            .map(|e| {
                let o = obj();
                match e {
                    Event::DrawingCreated { kind, points } => {
                        set(&o, "type", "drawingCreated");
                        set(&o, "kind", kind.as_str());
                        let list: Array = points.iter().map(|p| JsValue::from(js::point(p.time, p.price))).collect();
                        set(&o, "points", list);
                    }
                    Event::SelectionChanged(id) => {
                        set(&o, "type", "selection");
                        set(&o, "id", opt(id.map(|i| i as f64)));
                    }
                    Event::DeleteRequested(id) => {
                        set(&o, "type", "delete");
                        set(&o, "id", id as f64);
                    }
                    Event::DrawingChanged { id, points } => {
                        set(&o, "type", "drawingChanged");
                        set(&o, "id", id as f64);
                        let list: Array = points.iter().map(|p| JsValue::from(js::point(p.time, p.price))).collect();
                        set(&o, "points", list);
                    }
                    Event::EditRequested(id) => {
                        set(&o, "type", "editRequested");
                        set(&o, "id", id as f64);
                    }
                    Event::ToolFinished => set(&o, "type", "toolFinished"),
                    Event::NeedHistory { before } => {
                        set(&o, "type", "needHistory");
                        set(&o, "before", before as f64);
                    }
                    Event::PaneWeightsChanged(value) => {
                        set(&o, "type", "paneWeights");
                        set(&o, "value", value);
                    }
                    Event::AlertMoved { id, price } => {
                        set(&o, "type", "alertMoved");
                        set(&o, "id", id as f64);
                        set(&o, "price", price);
                    }
                    Event::AutoScaleChanged(on) => {
                        set(&o, "type", "autoScale");
                        set(&o, "value", on);
                    }
                }
                JsValue::from(o)
            })
            .collect()
    }

    pub fn legend(&self) -> JsValue {
        let l = self.chart.legend();
        let o = obj();
        set(&o, "index", opt(l.index.map(|i| i as f64)));
        set(&o, "time", l.time as f64);
        set(&o, "ohlc", l.ohlc);
        set(&o, "change", l.change);
        set(&o, "up", l.up);
        set(&o, "volume", l.volume);
        let values = obj();
        for (k, v) in l.values {
            set(&values, &k, v);
        }
        set(&o, "values", values);
        match l.cursor {
            Some((y, price)) => {
                let c = obj();
                set(&c, "y", f64::from(y));
                set(&c, "price", price);
                set(&o, "cursor", c);
            }
            None => set(&o, "cursor", JsValue::NULL),
        }
        set(&o, "axisX", f64::from(l.axis_x));
        match l.alert {
            Some((id, y)) => {
                let a = obj();
                set(&a, "id", id as f64);
                set(&a, "y", f64::from(y));
                set(&o, "alert", a);
            }
            None => set(&o, "alert", JsValue::NULL),
        }
        match l.flow {
            Some((index, x, y)) => {
                let m = obj();
                set(&m, "index", index as f64);
                set(&m, "x", f64::from(x));
                set(&m, "y", f64::from(y));
                set(&o, "flow", m);
            }
            None => set(&o, "flow", JsValue::NULL),
        }
        match l.footprint {
            Some(cell) => {
                let c = obj();
                set(&c, "time", cell.time as f64);
                set(&c, "price", cell.price);
                set(&c, "step", cell.step);
                set(&c, "buy", cell.buy);
                set(&c, "sell", cell.sell);
                set(&c, "x", f64::from(cell.x));
                set(&c, "y", f64::from(cell.y));
                set(&o, "footprint", c);
            }
            None => set(&o, "footprint", JsValue::NULL),
        }
        o.into()
    }

    /// The market's active price alerts, as `[id, price, id, price, …]`.
    /// `[id, price, rgba, dash, width, …]`: see `alert_lines`.
    pub fn set_alerts(&mut self, flat: &[f64]) {
        self.chart.set_alerts(alert_lines(flat));
    }

    /// Minutes whose taker flow ran hard, as `[at, price, usd, …]`, oldest first.
    pub fn set_flow(&mut self, flat: &[f64]) {
        let marks = flat
            .chunks_exact(3)
            .filter(|c| c[1].is_finite() && c[1] > 0.0 && c[2].is_finite())
            .map(|c| tradoor_render::chart::FlowMark { at: c[0] as i64, price: c[1], usd: c[2] })
            .collect();
        self.chart.set_flow(marks);
    }

    /// Volume by price, flat: for each bar, its time, how many levels follow,
    /// the most and least its delta reached, then each level as price, coins
    /// bought, coins sold. One array rather than a tree of objects, because a
    /// busy screen is fifty thousand numbers. `fitted`: the rows join as the
    /// chart zooms out, rather than each covering `grid` as the reader set it.
    pub fn set_footprint(&mut self, flat: &[f64], grid: f64, fitted: bool) {
        let mut bars = Vec::new();
        let mut i = 0usize;
        while i + 3 < flat.len() {
            let time = flat[i] as i64;
            let count = flat[i + 1].max(0.0) as usize;
            let (max_delta, min_delta) = (finite_or_zero(flat[i + 2]), finite_or_zero(flat[i + 3]));
            i += 4;
            let end = (i + count * 3).min(flat.len());
            let levels = flat[i..end]
                .chunks_exact(3)
                .filter(|c| c[0].is_finite() && c[0] > 0.0 && c[1].is_finite() && c[2].is_finite())
                .map(|c| (c[0], c[1], c[2]))
                .collect::<Vec<_>>();
            i = end;
            if !levels.is_empty() {
                bars.push(tradoor_render::footprint::Bar { time, levels, max_delta, min_delta });
            }
        }
        self.chart.set_footprint(bars, grid, fitted);
    }

    pub fn set_footprint_shown(&mut self, shown: bool) {
        self.chart.set_footprint_shown(shown);
    }

    /// `mode;valueArea;stats;profile` — the mode by name, the rest 0 or 1.
    pub fn set_footprint_options(&mut self, raw: &str) {
        self.chart.set_footprint_options(raw);
    }

    pub fn studies(&self) -> Array {
        self.chart
            .studies_meta()
            .into_iter()
            .map(|s| {
                let o = obj();
                set(&o, "id", s.id);
                set(&o, "label", s.label);
                set(&o, "group", s.group);
                set(&o, "hidden", s.hidden);
                let lines: Array = s
                    .lines
                    .into_iter()
                    .map(|(id, label, color)| {
                        let l = obj();
                        set(&l, "id", id);
                        set(&l, "label", label);
                        set(&l, "color", color.css());
                        JsValue::from(l)
                    })
                    .collect();
                set(&o, "lines", lines);
                let stats: Array = s
                    .stats
                    .into_iter()
                    .map(|(label, value)| {
                        let st = obj();
                        set(&st, "label", label);
                        set(&st, "value", value);
                        JsValue::from(st)
                    })
                    .collect();
                set(&o, "stats", stats);
                JsValue::from(o)
            })
            .collect()
    }
}

/// A `.tdc` payload (already inflated) as `{interval, time, open, high, low, close, volume}`.
#[wasm_bindgen]
pub fn decode_partition(bytes: &[u8]) -> Result<Object, JsValue> {
    let ticks = TickCandles::decode(bytes).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let o = columns(&ticks.to_candles());
    set(&o, "interval", ticks.interval.as_str());
    Ok(o)
}

/// Where the chart bar holding `t` opens: a 2D bar on even days from the
/// epoch, a 2W one on every other Monday, a 3M one on the quarter.
#[wasm_bindgen]
pub fn bar_start(tf: &str, t: f64) -> Result<f64, JsValue> {
    Ok(timeframe(tf)?.align(t as i64) as f64)
}

/// Fold candles into a chart timeframe, dropping a bucket that has not closed.
#[wasm_bindgen]
pub fn fold_candles(tf: &str, time: &[f64], open: &[f64], high: &[f64], low: &[f64], close: &[f64], volume: &[f64], now: f64) -> Result<Object, JsValue> {
    Ok(columns(&fold(&candles(time, open, high, low, close, volume), timeframe(tf)?, now as i64)))
}

#[wasm_bindgen]
pub fn timeframes() -> Array {
    ChartTimeframe::ALL
        .iter()
        .map(|tf| {
            let o = obj();
            set(&o, "value", tf.as_str());
            set(&o, "label", tf.label());
            set(&o, "group", match tf.source().interval {
                Interval::M1 | Interval::M5 | Interval::M15 | Interval::M30 => 0,
                Interval::H1 | Interval::H2 | Interval::H4 | Interval::H6 | Interval::H12 => 1,
                Interval::D1 => 2,
                Interval::W1 => 3,
                Interval::Mo1 => 4,
            });
            JsValue::from(o)
        })
        .collect()
}

/// Where a timeframe's bars come from: `{interval, group, base, live, barMs, baseMs}`.
#[wasm_bindgen]
pub fn timeframe_info(tf: &str) -> Result<Object, JsValue> {
    let tf = timeframe(tf)?;
    let source = tf.source();
    let approx = |i: Interval| i.ms().unwrap_or(30 * tradoor_core::time::DAY) as f64;
    let o = obj();
    set(&o, "timeframe", tf.as_str());
    set(&o, "interval", source.interval.as_str());
    set(&o, "group", source.group);
    set(&o, "base", tf.history_base().as_str());
    set(&o, "live", tf.live_interval().as_str());
    set(&o, "barMs", approx(source.interval) * f64::from(source.group));
    set(&o, "baseMs", approx(tf.history_base()));
    set(&o, "intervalMs", approx(source.interval));
    Ok(o)
}

#[wasm_bindgen]
pub fn study_catalog() -> Array {
    STUDIES
        .iter()
        .map(|d| {
            let o = obj();
            set(&o, "kind", d.kind.as_str());
            set(&o, "name", d.name);
            set(&o, "short", d.short);
            set(&o, "group", d.group.as_str());
            set(&o, "about", d.about);
            let params: Array = d
                .params
                .iter()
                .map(|p| {
                    let po = obj();
                    set(&po, "name", p.name);
                    set(&po, "default", p.default);
                    set(&po, "min", p.min);
                    set(&po, "max", p.max);
                    JsValue::from(po)
                })
                .collect();
            set(&o, "params", params);
            JsValue::from(o)
        })
        .collect()
}

/// The forming bar for one market and interval.
#[wasm_bindgen]
pub struct LiveFeedEngine {
    feed: LiveFeed,
}

#[wasm_bindgen]
impl LiveFeedEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(tf: &str) -> Result<LiveFeedEngine, JsValue> {
        Ok(Self { feed: LiveFeed::new(timeframe(tf)?) })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_kline(&mut self, open_time: f64, open: f64, high: f64, low: f64, close: f64, volume: f64, last_trade_id: f64, closed: bool) -> bool {
        self.feed.apply_kline(Kline { open_time: open_time as i64, open, high, low, close, volume, last_trade_id: last_trade_id as i64, closed })
    }

    pub fn apply_trade(&mut self, time: f64, price: f64, qty: f64, last_trade_id: f64) -> bool {
        self.feed.apply_trade(time as i64, price, qty, last_trade_id as i64)
    }

    /// `{time, open, high, low, close, volume, closed}` or null.
    pub fn bar(&self) -> JsValue {
        match self.feed.current() {
            None => JsValue::NULL,
            Some(b) => {
                let o = obj();
                set(&o, "time", b.open_time as f64);
                set(&o, "open", b.open);
                set(&o, "high", b.high);
                set(&o, "low", b.low);
                set(&o, "close", b.close);
                set(&o, "volume", b.volume);
                set(&o, "closed", b.closed);
                o.into()
            }
        }
    }
}

/// The session a timeframe's profile describes and the interval it is built from.
#[wasm_bindgen]
pub fn profile_spec(tf: &str) -> Result<JsValue, JsValue> {
    Ok(match profile_spec_for(timeframe(tf)?) {
        None => JsValue::NULL,
        Some(spec) => {
            let o = obj();
            set(&o, "session", spec.session.as_str());
            set(&o, "from", spec.from.as_str());
            o.into()
        }
    })
}

/// Indicator values on the last closed bar, and the prior session's profile
/// levels built from `src_*` candles at the profile's source interval.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn indicator_values(
    tf: &str,
    time: &[f64],
    open: &[f64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
    volume: &[f64],
    src_time: &[f64],
    src_open: &[f64],
    src_high: &[f64],
    src_low: &[f64],
    src_close: &[f64],
    src_volume: &[f64],
    basis: &str,
) -> Result<JsValue, JsValue> {
    let tf = timeframe(tf)?;
    let bars = candles(time, open, high, low, close, volume);
    let Some(last) = bars.last() else { return Ok(JsValue::NULL) };
    let engine = engine_series(&bars);
    let closes: Vec<Option<f64>> = bars.iter().map(|c| Some(c.close)).collect();
    let (r, s) = (rsi(&closes, 14), stoch_rsi(&closes, 14, 14, 3, 3));
    let at = |v: &[Option<f64>]| opt(v.last().copied().flatten());
    let o = obj();
    set(&o, "time", last.open_time as f64);
    set(&o, "close", last.close);
    set(&o, "ema12", at(&engine.ema12));
    set(&o, "ema25", at(&engine.ema25));
    set(&o, "ema200", at(&engine.ema200));
    set(&o, "rsi14", at(&r));
    set(&o, "stochK", at(&s.k));
    set(&o, "stochD", at(&s.d));
    let profile = match profile_spec_for(tf) {
        None => JsValue::NULL,
        Some(spec) => {
            let basis = if basis == "tpo" { ProfileBasis::Tpo } else { ProfileBasis::Volume };
            let source = candles(src_time, src_open, src_high, src_low, src_close, src_volume);
            let overlay = profile_overlay(&bars, &source, spec, basis);
            let p = obj();
            set(&p, "session", spec.session.as_str());
            set(&p, "from", spec.from.as_str());
            set(&p, "basis", basis.as_str());
            set(&p, "vah", at(&overlay.vah));
            set(&p, "val", at(&overlay.val));
            set(&p, "poc", at(&overlay.poc));
            set(&p, "covered", overlay.covered as u32);
            set(&p, "total", bars.len() as u32);
            p.into()
        }
    };
    set(&o, "profile", profile);
    Ok(o.into())
}

/// Alert levels off the flat array the page sends: `[id, price, rgba, dash,
/// width, …]` — `rgba` as `0xrrggbbaa`, 0 for the theme's colour; `dash` 1
/// solid, 2 dashed, 3 dotted, 0 for dashed; `width` 0 for a pixel.
/// A figure off the wire, or nothing where it is not a number.
fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn alert_lines(flat: &[f64]) -> Vec<tradoor_render::chart::AlertLine> {
    flat.chunks_exact(5)
        .filter(|c| c[1].is_finite() && c[1] > 0.0)
        .map(|c| {
            let mut line = tradoor_render::chart::AlertLine::at(c[0] as i64, c[1]);
            if c[2] > 0.0 && c[2] <= f64::from(u32::MAX) {
                let v = c[2] as u32;
                line.color = Some(tradoor_render::color::Color { r: (v >> 24) as u8, g: (v >> 16) as u8, b: (v >> 8) as u8, a: v as u8 });
            }
            line.dash = match c[3] as u8 {
                1 => Dash::Solid,
                3 => Dash::Dotted,
                _ => Dash::Dashed,
            };
            if c[4].is_finite() && c[4] > 0.0 {
                line.width = (c[4] as f32).min(4.0);
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradoor_render::chart::AlertLine;
    use tradoor_render::color::Color;

    #[test]
    fn alert_lines_read_their_style_off_the_flat_array() {
        let lines = alert_lines(&[7.0, 84_345.0, 0.0, 0.0, 0.0, 8.0, 84_000.0, f64::from(0xf236_4580_u32), 1.0, 2.0]);
        assert_eq!(lines[0], AlertLine::at(7, 84_345.0), "nothing set: the default line");
        assert_eq!(lines[1].color, Some(Color { r: 0xf2, g: 0x36, b: 0x45, a: 0x80 }));
        assert_eq!((lines[1].dash, lines[1].width), (Dash::Solid, 2.0));
        assert!(alert_lines(&[7.0, 84_345.0]).is_empty(), "the old two-number form is not misread");
    }
}

