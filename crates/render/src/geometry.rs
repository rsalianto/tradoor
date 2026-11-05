//! Drawing anchors and the EMA ribbon, in data space.
//!
//! Drawings are stored in TIME and placed through the logical (bar index) axis
//! by interpolating over the candle times, which is what keeps a line drawn
//! across a weekend touching the same two wicks — a fixed bar duration cannot.

use tradoor_core::num::Within;

pub type Pt = (f32, f32);

/// Where `time` sits on the logical axis. Past either end the nearest gap is
/// extended, which is what lets a projection run beyond the last bar.
pub fn time_to_logical(times: &[f64], time: f64) -> Option<f64> {
    let n = times.len();
    match n {
        0 => return None,
        1 => return Some(0.0),
        _ => {}
    }
    let (first, last) = (times[0], times[n - 1]);
    if time <= first {
        let gap = times[1] - first;
        return Some(if gap > 0.0 { (time - first) / gap } else { 0.0 });
    }
    if time >= last {
        let gap = last - times[n - 2];
        return Some(if gap > 0.0 { (n - 1) as f64 + (time - last) / gap } else { (n - 1) as f64 });
    }
    let (mut lo, mut hi) = (0usize, n - 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if times[mid] <= time {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let span = times[hi] - times[lo];
    Some(if span > 0.0 { lo as f64 + (time - times[lo]) / span } else { lo as f64 })
}

pub fn logical_to_time(times: &[f64], logical: f64) -> Option<f64> {
    let n = times.len();
    match n {
        0 => return None,
        1 => return Some(times[0]),
        _ => {}
    }
    let (first, last) = (times[0], times[n - 1]);
    if logical <= 0.0 {
        return Some(first + logical * (times[1] - first));
    }
    if logical >= (n - 1) as f64 {
        return Some(last + (logical - (n - 1) as f64) * (last - times[n - 2]));
    }
    let i = logical.floor() as usize;
    Some(times[i] + (logical - i as f64) * (times[i + 1] - times[i]))
}

/// Distance to a SEGMENT: a trendline is not selectable a thousand pixels past
/// where it stops being drawn.
pub fn distance_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len_sq = dx * dx + dy * dy;
    if len_sq == 0.0 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len_sq).within(0.0, 1.0);
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Distance to a rectangle's OUTLINE: a zone's interior is the chart showing
/// through, and clicks inside it must still reach the candles.
pub fn distance_to_rect(p: Pt, a: Pt, b: Pt) -> f32 {
    let (l, r, t, bo) = (a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1));
    distance_to_segment(p, (l, t), (r, t)).min(distance_to_segment(p, (r, t), (r, bo))).min(distance_to_segment(p, (r, bo), (l, bo))).min(distance_to_segment(p, (l, bo), (l, t)))
}

/// A few pixels of slack at a ray's or fib's starting edge.
const HANDLE_SLACK: f32 = 4.0;

pub fn distance_to_drawing(kind: &str, points: &[Pt], cursor: Pt, fib_levels: &[f64]) -> f32 {
    let Some(&a) = points.first() else { return f32::INFINITY };
    match kind {
        "horizontal" => return (cursor.1 - a.1).abs(),
        "ray" => return if cursor.0 >= a.0 - HANDLE_SLACK { (cursor.1 - a.1).abs() } else { f32::INFINITY },
        "text" => return (cursor.0 - a.0).hypot(cursor.1 - a.1),
        "path" | "brush" => return points.windows(2).map(|w| distance_to_segment(cursor, w[0], w[1])).fold(f32::INFINITY, f32::min),
        _ => {}
    }
    let Some(&b) = points.get(1) else { return f32::INFINITY };
    match kind {
        "rect" | "measure" => distance_to_rect(cursor, a, b),
        "fib" => {
            let left = a.0.min(b.0);
            if cursor.0 < left - HANDLE_SLACK {
                return f32::INFINITY;
            }
            fib_levels.iter().map(|level| (cursor.1 - (a.1 + (b.1 - a.1) * *level as f32)).abs()).fold(f32::INFINITY, f32::min)
        }
        "long" | "short" => match points.get(2) {
            Some(&c) => distance_to_rect(cursor, a, b).min(distance_to_rect(cursor, a, c)),
            None => distance_to_rect(cursor, a, b),
        },
        _ => distance_to_segment(cursor, a, b),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonPoint {
    pub logical: f64,
    pub fast: f64,
    pub slow: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RibbonRun {
    pub bullish: bool,
    pub points: Vec<RibbonPoint>,
}

/// Cut the EMA 12/25 band into single-colour runs, split at the EXACT crossing
/// rather than at a bar — rounding paints a bar's width of the wrong colour
/// across every cross, and flatters the turn. Warm-up ends a run.
pub fn ribbon_runs(fast: &[Option<f64>], slow: &[Option<f64>]) -> Vec<RibbonRun> {
    let mut runs = Vec::new();
    let mut current: Option<RibbonRun> = None;
    let mut prev: Option<RibbonPoint> = None;
    let close = |runs: &mut Vec<RibbonRun>, current: &mut Option<RibbonRun>| {
        if let Some(run) = current.take().filter(|r| r.points.len() >= 2) {
            runs.push(run);
        }
    };
    for i in 0..fast.len().min(slow.len()) {
        let (Some(f), Some(s)) = (fast[i], slow[i]) else {
            close(&mut runs, &mut current);
            prev = None;
            continue;
        };
        let point = RibbonPoint { logical: i as f64, fast: f, slow: s };
        // Ties are bearish, as in the Pine original: `buy = fast > slow`.
        let bullish = f > s;
        match (&mut current, prev) {
            (Some(run), Some(p)) if run.bullish != bullish => {
                let crossing = cross_point(p, point);
                if let Some(c) = crossing {
                    run.points.push(c);
                }
                close(&mut runs, &mut current);
                current = Some(RibbonRun { bullish, points: crossing.into_iter().chain([point]).collect() });
            }
            (Some(run), _) => run.points.push(point),
            (None, _) => current = Some(RibbonRun { bullish, points: vec![point] }),
        }
        prev = Some(point);
    }
    close(&mut runs, &mut current);
    runs
}

fn cross_point(a: RibbonPoint, b: RibbonPoint) -> Option<RibbonPoint> {
    let (gap_a, gap_b) = (a.fast - a.slow, b.fast - b.slow);
    let delta = gap_a - gap_b;
    if delta == 0.0 {
        return None;
    }
    let t = gap_a / delta;
    t.is_finite().then(|| {
        let value = a.fast + t * (b.fast - a.fast);
        RibbonPoint { logical: a.logical + t * (b.logical - a.logical), fast: value, slow: value }
    })
}

/// The stretch of a run worth converting to pixels, with one point of overhang
/// either side so the band enters and leaves the pane at the right slope.
pub fn visible_slice(points: &[RibbonPoint], from: f64, to: f64) -> &[RibbonPoint] {
    let Some(mut first) = points.iter().position(|p| p.logical >= from) else {
        return match points.last() {
            Some(last) if last.logical >= from => &points[points.len() - 1..],
            _ => &[],
        };
    };
    let mut last = points.len() - 1;
    while last > first && points[last].logical > to {
        last -= 1;
    }
    first = first.saturating_sub(1);
    if last < points.len() - 1 {
        last += 1;
    }
    &points[first..=last]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86_400_000.0;
    const WEEK: [f64; 7] = [0.0, DAY, 2.0 * DAY, 3.0 * DAY, 4.0 * DAY, 7.0 * DAY, 8.0 * DAY];

    #[test]
    fn time_and_logical_round_trip_across_gaps() {
        for (i, t) in WEEK.iter().enumerate() {
            assert_eq!(time_to_logical(&WEEK, *t), Some(i as f64));
        }
        assert_eq!(time_to_logical(&WEEK, 5.0 * DAY), Some(4.0 + 1.0 / 3.0), "a weekend is one bar wide");
        assert_eq!(time_to_logical(&WEEK, 10.0 * DAY), Some(8.0));
        assert_eq!(time_to_logical(&WEEK, -DAY), Some(-1.0));
        assert_eq!((time_to_logical(&[], 1.0), time_to_logical(&[5.0], 9.0), time_to_logical(&[5.0, 5.0, 5.0], 5.0)), (None, Some(0.0), Some(0.0)));
        for time in [DAY / 3.0, 2.5 * DAY, 5.0 * DAY, 6.5 * DAY, 9.0 * DAY, -2.0 * DAY] {
            let back = logical_to_time(&WEEK, time_to_logical(&WEEK, time).unwrap()).unwrap();
            assert!((back - time).abs() < 1e-6, "{time} → {back}");
        }
    }

    #[test]
    fn distances() {
        assert_eq!(distance_to_segment((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)), 3.0);
        assert_eq!(distance_to_segment((20.0, 0.0), (0.0, 0.0), (10.0, 0.0)), 10.0);
        assert_eq!(distance_to_rect((50.0, 25.0), (0.0, 0.0), (100.0, 50.0)), 25.0);
        assert_eq!(distance_to_rect((50.0, -4.0), (100.0, 50.0), (0.0, 0.0)), 4.0);
        assert_eq!(distance_to_drawing("horizontal", &[(10.0, 100.0)], (9999.0, 103.0), &[]), 3.0);
        assert_eq!(distance_to_drawing("trendline", &[], (0.0, 0.0), &[]), f32::INFINITY);
        assert_eq!(distance_to_drawing("rect", &[(0.0, 0.0), (100.0, 0.0)], (50.0, 10.0), &[]), 10.0);
    }

    #[test]
    fn ribbon_splits_at_the_exact_crossing() {
        let s = |v: &[f64]| v.iter().copied().map(Some).collect::<Vec<_>>();
        let runs = ribbon_runs(&s(&[0.0, 2.0]), &s(&[1.0, 1.0]));
        assert_eq!(runs.iter().map(|r| r.bullish).collect::<Vec<_>>(), [false, true]);
        assert_eq!(runs[0].points.last(), runs[1].points.first());
        assert_eq!(runs[0].points.last().unwrap().logical, 0.5);
        assert_eq!(ribbon_runs(&s(&[4.0, 0.0]), &s(&[1.0, 1.0]))[0].points.last().unwrap().logical, 0.75);
        let holes = ribbon_runs(&[Some(2.0), Some(3.0), None, Some(4.0), Some(5.0)], &[Some(1.0), Some(1.0), None, Some(1.0), Some(1.0)]);
        assert_eq!(holes.iter().map(|r| r.points.iter().map(|p| p.logical).collect::<Vec<_>>()).collect::<Vec<_>>(), [vec![0.0, 1.0], vec![3.0, 4.0]]);
        assert!(ribbon_runs(&[Some(2.0), None, Some(3.0)], &[Some(1.0), None, Some(1.0)]).is_empty());
        assert!(!ribbon_runs(&s(&[1.0, 1.0]), &s(&[1.0, 1.0]))[0].bullish);
    }

    #[test]
    fn visible_slice_keeps_one_point_of_overhang() {
        let pts: Vec<RibbonPoint> = (0..20).map(|i| RibbonPoint { logical: i as f64, fast: 2.0, slow: 1.0 }).collect();
        assert_eq!(visible_slice(&pts, 5.0, 10.0).iter().map(|p| p.logical as i32).collect::<Vec<_>>(), [4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(visible_slice(&pts[..6], -3.0, 99.0).len(), 6);
        assert!(visible_slice(&pts[..5], 100.0, 200.0).is_empty());
        assert_eq!(visible_slice(&pts, -50.0, -10.0).len(), 2);
        assert_eq!(visible_slice(&pts[..5], 4.0, 20.0).len(), 2);
    }
}
