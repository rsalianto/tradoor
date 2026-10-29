//! Time and price scales.

/// The horizontal axis, in bars. Logical index `i` is bar `i`; positions between
/// and beyond bars are continuous, which is where drawings and projections live.
///
/// A bar is one column wide unless a warp says otherwise: a TPO chart stands its
/// profiles side by side at their own widths, so a day of bars may take only the
/// columns its widest row needs. Spacing and offsets are in columns; positions
/// stay in bars, so whatever is pinned to a time stays on it.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeScale {
    /// Pixels per column.
    pub bar_spacing: f64,
    /// Columns of empty space between the last bar and the right edge (negative
    /// when scrolled back into history).
    pub right_offset: f64,
    /// Plot width in pixels.
    pub width: f64,
    pub bars: usize,
    /// The narrowest a column may get: zooming out stops here.
    pub min_spacing: f64,
    /// `(bar edge, column)` knots, both strictly rising; straight lines between
    /// them, one column a bar beyond them. Empty: no warp. Bar `i` runs from
    /// edge `i` to edge `i + 1`.
    pub warp: Vec<(f64, f64)>,
}

pub const MIN_BAR_SPACING: f64 = 0.5;
/// Zooming in may always reach at least this spacing, however narrow the plot.
const ZOOM_FLOOR: f64 = 80.0;

/// Where `v` falls along the knots' `from` side, read off their `to` side.
fn through(knots: &[(f64, f64)], v: f64, from: impl Fn(&(f64, f64)) -> f64, to: impl Fn(&(f64, f64)) -> f64) -> f64 {
    let (Some(first), Some(last)) = (knots.first(), knots.last()) else { return v };
    if v <= from(first) {
        return to(first) - (from(first) - v);
    }
    if v >= from(last) {
        return to(last) + (v - from(last));
    }
    let i = knots.partition_point(|k| from(k) <= v).max(1);
    let (a, b) = (&knots[i - 1], &knots[i]);
    let span = from(b) - from(a);
    to(a) + if span > 0.0 { (v - from(a)) / span * (to(b) - to(a)) } else { 0.0 }
}

impl TimeScale {
    pub fn new() -> Self {
        Self { bar_spacing: 6.0, right_offset: 2.0, width: 0.0, bars: 0, min_spacing: MIN_BAR_SPACING, warp: Vec::new() }
    }

    /// The widest a column may get: two across the plot.
    pub fn max_spacing(&self) -> f64 {
        (self.width / 2.0).max(ZOOM_FLOOR)
    }

    /// The column at a bar edge.
    pub fn edge_column(&self, edge: f64) -> f64 {
        through(&self.warp, edge, |k| k.0, |k| k.1)
    }

    /// The bar edge at a column.
    pub fn column_edge(&self, column: f64) -> f64 {
        through(&self.warp, column, |k| k.1, |k| k.0)
    }

    /// The column a bar's centre stands at, counted as bars are: 0 for the first.
    fn column(&self, logical: f64) -> f64 {
        self.edge_column(logical + 0.5) - 0.5
    }

    fn right_column(&self) -> f64 {
        self.column(self.bars as f64 - 1.0) + self.right_offset
    }

    fn total_columns(&self) -> f64 {
        self.edge_column(self.bars as f64) - self.edge_column(0.0)
    }

    /// The x of a column's centre, columns counted as `column` counts them.
    fn column_x(&self, column: f64) -> f64 {
        self.width - (self.right_column() - column + 0.5) * self.bar_spacing
    }

    /// The x of a bar's centre.
    pub fn x(&self, logical: f64) -> f64 {
        self.column_x(self.column(logical))
    }

    /// The x of a bar edge: where bar `edge` starts.
    pub fn edge_x(&self, edge: f64) -> f64 {
        self.column_x(self.edge_column(edge) - 0.5)
    }

    pub fn logical(&self, x: f64) -> f64 {
        let column = self.right_column() + 0.5 - (self.width - x) / self.bar_spacing;
        self.column_edge(column + 0.5) - 0.5
    }

    /// Logical range covering the plot, left to right.
    pub fn visible(&self) -> (f64, f64) {
        (self.logical(0.0), self.logical(self.width))
    }

    pub fn nearest_bar(&self, x: f64) -> Option<usize> {
        (self.bars > 0).then(|| self.logical(x).round().clamp(0.0, (self.bars - 1) as f64) as usize)
    }

    /// Keep what is under `x` under `x` while the spacing changes.
    pub fn zoom_at(&mut self, x: f64, factor: f64) {
        let anchor = self.right_column() + 0.5 - (self.width - x) / self.bar_spacing;
        self.bar_spacing = (self.bar_spacing * factor).clamp(self.min_spacing, self.max_spacing());
        let right = anchor + (self.width - x) / self.bar_spacing - 0.5;
        self.right_offset = right - self.column(self.bars as f64 - 1.0);
        self.clamp();
    }

    /// Move the bars `dx` pixels, the way dragging paper moves what is on it:
    /// dragging right brings older bars into view.
    pub fn pan(&mut self, dx: f64) {
        self.right_offset -= dx / self.bar_spacing;
        self.clamp();
    }

    /// At least a few columns always stay on screen, at either end.
    pub fn clamp(&mut self) {
        if self.width <= 0.0 || self.bars == 0 {
            return;
        }
        let visible = self.width / self.bar_spacing;
        let total = self.total_columns();
        // Half the plot when fewer than six columns fit, or a close zoom would push
        // the newest bars off screen to honour the three.
        let keep = 3.0f64.min(total).min(visible / 2.0);
        let max = visible - keep;
        let min = -total + keep;
        self.right_offset = self.right_offset.clamp(min.min(max), max.max(min));
    }

    /// The last `count` bars fill the plot, with `margin` columns of room at the right.
    pub fn show_last(&mut self, count: f64, margin: f64) {
        if self.width <= 0.0 {
            return;
        }
        let n = self.bars as f64;
        let columns = self.edge_column(n) - self.edge_column(n - count.max(1.0));
        self.bar_spacing = (self.width / (columns.max(1.0) + margin)).clamp(self.min_spacing, self.max_spacing());
        self.right_offset = margin;
        self.clamp();
    }

    /// Changes the warp keeping the bar at the right edge where it was.
    pub fn set_warp(&mut self, warp: Vec<(f64, f64)>) {
        if warp == self.warp {
            return;
        }
        let right = self.logical(self.width);
        self.warp = warp;
        if self.width > 0.0 && self.bar_spacing > 0.0 && right.is_finite() {
            self.right_offset = self.column(right) - 0.5 - self.column(self.bars as f64 - 1.0);
            self.clamp();
        }
    }
}

impl Default for TimeScale {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    Normal,
    /// Equal PERCENTAGE moves take equal space.
    Log,
}

/// One pane's vertical axis. The range is kept in transformed space (ln for the
/// log scale), so zooming and margins mean the same thing under either mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceScale {
    pub mode: ScaleMode,
    pub top: f64,
    pub height: f64,
    /// Fractions of the height left empty above and below the data.
    pub margins: (f64, f64),
    /// Transformed (min, max).
    pub range: (f64, f64),
}

impl PriceScale {
    pub fn new(mode: ScaleMode, top: f64, height: f64, margins: (f64, f64)) -> Self {
        Self { mode, top, height, margins, range: (0.0, 1.0) }
    }

    pub fn transform(&self, price: f64) -> f64 {
        match self.mode {
            ScaleMode::Normal => price,
            ScaleMode::Log => price.max(1e-12).ln(),
        }
    }

    pub fn untransform(&self, t: f64) -> f64 {
        match self.mode {
            ScaleMode::Normal => t,
            ScaleMode::Log => t.exp(),
        }
    }

    /// Fit a price range, padding a flat one so it still has height.
    pub fn fit(&mut self, min: f64, max: f64) {
        if !(min.is_finite() && max.is_finite()) || min > max {
            return;
        }
        let (mut lo, mut hi) = (self.transform(min), self.transform(max));
        if hi - lo < 1e-12 {
            let pad = if lo == 0.0 { 1.0 } else { lo.abs() * 0.01 };
            lo -= pad;
            hi += pad;
        }
        self.range = (lo, hi);
    }

    fn inner(&self) -> (f64, f64) {
        let (mt, mb) = self.margins;
        (self.top + mt * self.height, (self.height * (1.0 - mt - mb)).max(1.0))
    }

    pub fn y(&self, price: f64) -> f64 {
        let (top, h) = self.inner();
        let (lo, hi) = self.range;
        top + (hi - self.transform(price)) / (hi - lo) * h
    }

    pub fn price(&self, y: f64) -> f64 {
        let (top, h) = self.inner();
        let (lo, hi) = self.range;
        self.untransform(hi - (y - top) / h * (hi - lo))
    }

    /// Move the range `dy` pixels, the way dragging paper moves what is on it:
    /// dragging down brings higher prices into view.
    pub fn pan(&mut self, dy: f64) {
        let (lo, hi) = self.range;
        let shift = dy / self.inner().1 * (hi - lo);
        if shift.is_finite() {
            self.range = (lo + shift, hi + shift);
        }
    }

    /// Stretch or squeeze the range around its centre, keeping its span (in
    /// transformed units) within `[min_span, max_span]`.
    pub fn zoom(&mut self, factor: f64, min_span: f64, max_span: f64) {
        let (lo, hi) = self.range;
        let span = ((hi - lo) * factor).clamp(min_span, max_span.max(min_span));
        let mid = (lo + hi) / 2.0;
        if span.is_finite() && span > 0.0 {
            self.range = (mid - span / 2.0, mid + span / 2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale() -> TimeScale {
        TimeScale { bar_spacing: 10.0, right_offset: 2.0, width: 1000.0, bars: 500, min_spacing: MIN_BAR_SPACING, warp: Vec::new() }
    }

    #[test]
    fn x_and_logical_are_inverse() {
        let t = scale();
        for l in [-3.5, 0.0, 250.25, 499.0, 510.0] {
            assert!((t.logical(t.x(l)) - l).abs() < 1e-9);
        }
        assert!((t.x(499.0) - (1000.0 - 25.0)).abs() < 1e-9, "the last bar sits 2.5 bars from the edge");
        assert_eq!(t.nearest_bar(t.x(123.2)), Some(123));
    }

    #[test]
    fn zoom_keeps_the_bar_under_the_pointer() {
        let mut t = scale();
        let before = t.logical(400.0);
        t.zoom_at(400.0, 1.7);
        assert!((t.logical(400.0) - before).abs() < 1e-9);
        t.zoom_at(400.0, 1e9);
        assert_eq!(t.bar_spacing, t.max_spacing());
    }

    #[test]
    fn a_close_zoom_keeps_the_newest_bar_on_screen() {
        let mut t = scale();
        let x = t.x(499.0);
        t.zoom_at(x, 1e9);
        t.clamp();
        assert_eq!(t.bar_spacing, 500.0, "two bars across a 1000px plot");
        assert!((t.x(499.0) - x).abs() < 1e-9, "clamping did not push it away");
    }

    #[test]
    fn panning_is_clamped_so_bars_stay_visible() {
        let mut t = scale();
        t.pan(1e9);
        assert!(t.visible().0 <= (t.bars - 3) as f64, "bars remain on the left");
        t.pan(-1e12);
        assert!(t.visible().1 >= 2.0, "bars remain on the right");
    }

    /// Ten bars squeezed into two columns, the rest a column each.
    fn warped() -> TimeScale {
        TimeScale { warp: vec![(100.0, 100.0), (110.0, 102.0)], ..scale() }
    }

    #[test]
    fn a_warp_squeezes_its_bars_and_keeps_x_and_logical_inverse() {
        let t = warped();
        for l in [-3.5, 0.0, 99.7, 104.2, 109.9, 250.25, 499.0, 510.0] {
            assert!((t.logical(t.x(l)) - l).abs() < 1e-9, "{l}");
        }
        assert!((t.edge_x(110.0) - t.edge_x(100.0) - 20.0).abs() < 1e-9, "ten bars in two columns of 10px");
        assert!((t.edge_x(120.0) - t.edge_x(110.0) - 100.0).abs() < 1e-9, "past the warp, a column a bar");
        assert!((t.x(499.0) - (1000.0 - 25.0)).abs() < 1e-9, "the right edge is held in columns as before");
        assert!((t.edge_x(300.0) - (t.x(300.0) - 5.0)).abs() < 1e-9, "an edge is half a column left of its bar's centre");
        assert_eq!(t.nearest_bar(t.edge_x(105.0) + 0.1), Some(105), "just right of an edge is the bar it starts");
    }

    #[test]
    fn showing_the_last_bars_counts_their_columns() {
        let mut t = TimeScale { warp: vec![(400.0, 400.0), (500.0, 410.0)], ..scale() };
        t.show_last(100.0, 10.0);
        assert!((t.bar_spacing - 50.0).abs() < 1e-9, "a hundred bars in ten columns, plus ten of room");
    }

    #[test]
    fn changing_the_warp_holds_the_right_edge() {
        let mut t = scale();
        t.pan(300.0);
        let right = t.logical(t.width);
        t.set_warp(vec![(100.0, 100.0), (110.0, 102.0)]);
        assert!((t.logical(t.width) - right).abs() < 1e-9);
    }

    #[test]
    fn price_scale_maps_both_modes_with_margins() {
        let mut p = PriceScale::new(ScaleMode::Normal, 0.0, 100.0, (0.2, 0.1));
        p.fit(100.0, 200.0);
        assert!((p.y(200.0) - 20.0).abs() < 1e-9 && (p.y(100.0) - 90.0).abs() < 1e-9);
        assert!((p.price(p.y(150.0)) - 150.0).abs() < 1e-9);

        let mut l = PriceScale::new(ScaleMode::Log, 0.0, 100.0, (0.0, 0.0));
        l.fit(10.0, 1000.0);
        assert!((l.y(100.0) - 50.0).abs() < 1e-9, "100 is halfway between 10 and 1000 on a log axis");
        assert!((l.price(50.0) - 100.0).abs() < 1e-9);

        let mut flat = PriceScale::new(ScaleMode::Normal, 0.0, 100.0, (0.0, 0.0));
        flat.fit(5.0, 5.0);
        assert!(flat.range.1 > flat.range.0);
    }
}
