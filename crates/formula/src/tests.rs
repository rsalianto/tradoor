//! The built-in indicators' sources, run over the same bars as the indicators
//! themselves and held to the same numbers; and the limits, each tried.
use super::*;
use tradoor_core::indicators::{self, Series};
use tradoor_core::studies::{compute_study, parse_studies};
use tradoor_core::Candle;

const HOUR: i64 = 3_600_000;

/// Three thousand hourly bars of a walk that trends, chops and gaps, with volume that varies.
fn candles() -> Vec<Candle> {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f64 / 10_000.0
    };
    let mut price = 60_000.0;
    // A Monday, so the weeks turn inside the bars.
    let start = 1_798_934_400_000 - 3 * 24 * HOUR;
    (0..3_000)
        .map(|i| {
            let drift = if (i / 400) % 2 == 0 { 8.0 } else { -6.0 };
            let open = price + (next() - 0.5) * 40.0;
            let close = open + drift + (next() - 0.5) * 300.0;
            let high = open.max(close) + next() * 120.0;
            let low = open.min(close) - next() * 120.0;
            price = close;
            Candle { open_time: start + i as i64 * HOUR, open, high, low, close, volume: 50.0 + next() * 400.0, quote_volume: None, taker_buy: None }
        })
        .collect()
}

struct Columns {
    time: Vec<f64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
}

fn columns(c: &[Candle]) -> Columns {
    Columns {
        time: c.iter().map(|c| c.open_time as f64).collect(),
        open: c.iter().map(|c| c.open).collect(),
        high: c.iter().map(|c| c.high).collect(),
        low: c.iter().map(|c| c.low).collect(),
        close: c.iter().map(|c| c.close).collect(),
        volume: c.iter().map(|c| c.volume).collect(),
    }
}

fn source(name: &str) -> &'static str {
    match name {
        "sma" => include_str!("../sources/sma.frost"),
        "ema" => include_str!("../sources/ema.frost"),
        "bb" => include_str!("../sources/bb.frost"),
        "keltner" => include_str!("../sources/keltner.frost"),
        "rsi" => include_str!("../sources/rsi.frost"),
        "stochrsi" => include_str!("../sources/stochrsi.frost"),
        "macd" => include_str!("../sources/macd.frost"),
        "atr" => include_str!("../sources/atr.frost"),
        "vwap" => include_str!("../sources/vwap.frost"),
        "supertrend" => include_str!("../sources/supertrend.frost"),
        "swingmap" => include_str!("../sources/swingmap.frost"),
        "keylevels" => include_str!("../sources/keylevels.frost"),
        "ribbon" => include_str!("../sources/ribbon.frost"),
        "ema200" => include_str!("../sources/ema200.frost"),
        "volume" => include_str!("../sources/volume.frost"),
        "ichimoku" => include_str!("../sources/ichimoku.frost"),
        "avwap" => include_str!("../sources/avwap.frost"),
        "moon" => include_str!("../sources/moon.frost"),
        "sessions" => include_str!("../sources/sessions.frost"),
        _ => panic!("no source {name}"),
    }
}

fn run_source(name: &str, inputs: &[f64], c: &[Candle]) -> Output {
    let k = columns(c);
    let bars = Bars { time: &k.time, open: &k.open, high: &k.high, low: &k.low, close: &k.close, volume: &k.volume };
    run(source(name), inputs, &bars, 2).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn line<'a>(out: &'a Output, title: &str) -> &'a [f64] {
    &out.plots.iter().find(|p| p.title == title).unwrap_or_else(|| panic!("no line {title}")).values
}

/// The formula's line against the indicator's: no value in the same bars, and the
/// same number in the rest — to the last bit where `exact`, else to a billionth.
fn same(name: &str, got: &[f64], want: &Series, exact: bool) {
    assert_eq!(got.len(), want.len(), "{name}: bars");
    let mut compared = 0;
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        match w {
            None => assert!(g.is_nan(), "{name}: bar {i} has {g}, the indicator none"),
            Some(w) => {
                let ok = if exact { g == w } else { (g - w).abs() <= 1e-9 * w.abs().max(1.0) };
                assert!(ok, "{name}: bar {i} is {g}, the indicator {w}");
                compared += 1;
            }
        }
    }
    assert!(compared > 1_000, "{name}: only {compared} bars to compare");
}

fn closes(c: &[Candle]) -> Series {
    c.iter().map(|c| Some(c.close)).collect()
}

#[test]
fn the_averages_are_the_charts_own() {
    let c = candles();
    same("sma", line(&run_source("sma", &[20.0], &c), "SMA"), &indicators::sma(&closes(&c), 20), false);
    same("ema", line(&run_source("ema", &[50.0], &c), "EMA"), &indicators::ema(&closes(&c), 50), true);
    same("ema 200", line(&run_source("ema", &[200.0], &c), "EMA"), &indicators::ema(&closes(&c), 200), true);
}

#[test]
fn bollinger_and_keltner_bands_are_the_charts_own() {
    let c = candles();
    let out = run_source("bb", &[20.0, 2.0], &c);
    let b = indicators::bollinger(&closes(&c), 20, 2.0);
    same("bb upper", line(&out, "Upper"), &b.upper, true);
    same("bb basis", line(&out, "Basis"), &b.basis, true);
    same("bb lower", line(&out, "Lower"), &b.lower, true);
    assert_eq!(out.fills, vec![Fill { upper: 0, lower: 2, color: 0x2962_ffff }]);

    let out = run_source("keltner", &[20.0, 2.0, 10.0], &c);
    let kc = compute_study(&parse_studies("keltner:20:2:10", 12)[0], &c);
    let kline = |key: &str| kc.lines.iter().find(|l| l.key == key).unwrap().values.clone();
    same("kc upper", line(&out, "Upper"), &kline("upper"), true);
    same("kc basis", line(&out, "Basis"), &kline("basis"), true);
    same("kc lower", line(&out, "Lower"), &kline("lower"), true);
}

#[test]
fn rsi_stoch_rsi_macd_and_atr_are_the_charts_own() {
    let c = candles();
    let out = run_source("rsi", &[14.0], &c);
    assert!(out.own_pane);
    same("rsi", line(&out, "RSI"), &indicators::rsi(&closes(&c), 14), true);

    let out = run_source("stochrsi", &[14.0, 14.0, 3.0, 3.0], &c);
    let s = indicators::stoch_rsi(&closes(&c), 14, 14, 3, 3);
    same("%k", line(&out, "%K"), &s.k, true);
    same("%d", line(&out, "%D"), &s.d, true);

    let out = run_source("macd", &[12.0, 26.0, 9.0], &c);
    let m = indicators::macd(&closes(&c), 12, 26, 9);
    same("macd", line(&out, "MACD"), &m.macd, true);
    same("signal", line(&out, "Signal"), &m.signal, true);
    same("histogram", line(&out, "Histogram"), &m.histogram, true);
    assert_eq!(out.plots[0].kind, PlotKind::Histogram);

    same("atr", line(&run_source("atr", &[14.0], &c), "ATR"), &indicators::atr(&c, 14), true);
}

#[test]
fn the_weeks_vwap_is_the_charts_own_from_its_first_whole_week() {
    let c = candles();
    let out = run_source("vwap", &[1.0], &c);
    let v = compute_study(&parse_studies("vwap:1:1:0:0", 12)[0], &c);
    let vline = |key: &str| v.lines.iter().find(|l| l.key == key).unwrap().values.clone();
    // The chart leaves out a week the bars begin inside of; the formula draws it, from its first bar.
    let from = vline("vwap").iter().position(Option::is_some).unwrap();
    for (title, key) in [("VWAP", "vwap"), ("Value Area High", "upper1"), ("Value Area Low", "lower1")] {
        let want: Series = vline(key)[from..].to_vec();
        same(title, &line(&out, title)[from..], &want, true);
    }
}

#[test]
fn supertrend_trails_price_and_marks_each_turn() {
    let c = candles();
    let out = run_source("supertrend", &[10.0, 3.0], &c);
    let (long, short) = (line(&out, "Long stop"), line(&out, "Short stop"));
    let buys = out.marks[0].hits.iter().filter(|h| **h).count();
    let sells = out.marks[1].hits.iter().filter(|h| **h).count();
    assert!(buys > 3 && sells > 3, "a walk that trends both ways turns: {buys} buys, {sells} sells");
    for i in 20..c.len() {
        assert!(long[i].is_nan() || short[i].is_nan(), "one side at a time, bar {i}");
        if !long[i].is_nan() {
            assert!(long[i] <= c[i].close, "a long stop sits below the close, bar {i}");
        }
    }
    assert_eq!((out.marks[0].text.as_str(), out.marks[0].above, out.marks[1].above), ("Buy", false, true));
}

#[test]
fn the_swing_map_finds_the_charts_own_swings_and_tags_them_alike() {
    let c = candles();
    let out = run_source("swingmap", &[], &c);
    let study = compute_study(&parse_studies("swingmap:5:5", 12)[0], &c);
    let index = |t: i64| c.iter().position(|k| k.open_time == t).unwrap();
    let want: Vec<(usize, f64, String)> = study
        .markers
        .iter()
        .map(|m| {
            let i = index(m.time);
            (i, if m.above { c[i].high } else { c[i].low }, m.text.split(' ').next().unwrap().to_string())
        })
        .collect();
    let got: Vec<(usize, f64, String)> = out.labels.iter().filter(|l| !l.text.contains("(live)")).map(|l| (l.x as usize, l.y, l.text.lines().next().unwrap().to_string())).collect();
    assert!(want.len() > 50, "{} swings to compare", want.len());
    assert_eq!(got, want);
    // Every leg a line, the two ends of the range, and the swing still forming.
    let legs = out.lines.iter().filter(|l| l.x1 != l.x2).count();
    assert!(legs >= want.len() - 1, "{legs} legs for {} points", want.len());
    let table = out.table.as_ref().unwrap();
    assert_eq!((table.down, table.across), (0, 2));
    let stat = |row: u8| table.cells.iter().find(|cell| cell.col == 1 && cell.row == row).unwrap().text.clone();
    let rise = study.stats.iter().find(|s| s.0 == "Avg rise").unwrap().1.clone();
    assert_eq!(stat(1).trim_end_matches('0').trim_end_matches('.'), rise.trim_end_matches('%').trim_end_matches('0').trim_end_matches('.').to_string() + "%");
    assert_eq!(stat(2), study.stats.iter().find(|s| s.0 == "Avg rise time").unwrap().1);
}

#[test]
fn key_levels_draw_each_open_where_the_chart_does() {
    let c = candles();
    let out = run_source("keylevels", &[], &c);
    let study = compute_study(&parse_studies("keylevels", 12)[0], &c);
    let index = |t: i64| c.iter().position(|k| k.open_time == t).unwrap() as f64;
    let mut want: Vec<(f64, f64, f64)> = study.segments.iter().filter(|s| ["wo", "mo", "yo"].contains(&s.key)).map(|s| (index(s.from), index(s.to), s.price)).collect();
    let monday: Vec<f64> = study.segments.iter().filter(|s| s.key.starts_with("mon")).map(|s| s.price).collect();
    let mut got: Vec<(f64, f64, f64)> = out.lines.iter().filter(|l| l.y1 == l.y2 && !monday.contains(&l.y1)).map(|l| (l.x1, l.x2, l.y1)).collect();
    let order = |a: &(f64, f64, f64), b: &(f64, f64, f64)| a.partial_cmp(b).unwrap();
    want.sort_by(order);
    got.sort_by(order);
    assert!(want.len() > 10, "{} opens to compare", want.len());
    assert_eq!(got, want);
    // This week's Monday, high and low, run a week of bars ahead.
    let mut drawn: Vec<f64> = out.lines.iter().filter(|l| monday.contains(&l.y1)).map(|l| l.y1).collect();
    drawn.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut monday = monday.clone();
    monday.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(drawn, monday);
    assert!(out.labels.iter().any(|l| l.text.starts_with("WO ") && l.plain));
}

#[test]
fn the_ribbon_ema_200_and_volume_are_the_charts_own() {
    let c = candles();
    let engine = tradoor_core::chart::engine_series(&c);
    let out = run_source("ribbon", &[], &c);
    let joined = |a: &str, b: &str| -> Vec<f64> { line(&out, a).iter().zip(line(&out, b)).map(|(x, y)| if x.is_nan() { *y } else { *x }).collect() };
    same("ema 12", &joined("Fast, rising", "Fast, falling"), &engine.ema12, true);
    same("ema 25", &joined("Slow, rising", "Slow, falling"), &engine.ema25, true);
    let marked = |m: usize| out.marks[m].hits.iter().enumerate().filter(|(_, h)| **h).map(|(i, _)| i).collect::<Vec<_>>();
    assert_eq!(marked(0), engine.crosses.iter().filter(|x| x.bull).map(|x| x.index).collect::<Vec<_>>());
    assert_eq!(marked(1), engine.crosses.iter().filter(|x| !x.bull).map(|x| x.index).collect::<Vec<_>>());
    same("ema 200", line(&run_source("ema200", &[], &c), "EMA 200"), &engine.ema200, true);
    let volume: Series = c.iter().map(|k| Some(k.volume)).collect();
    let out = run_source("volume", &[], &c);
    same("volume", line(&out, "Volume"), &volume, true);
    assert!(out.plots[0].colors.as_ref().unwrap().iter().zip(&c).all(|(col, k)| (*col == 0x26a6_9a99) == (k.close >= k.open)), "green where it closed up");
}

#[test]
fn ichimoku_and_anchored_vwap_are_the_charts_own() {
    let c = candles();
    let out = run_source("ichimoku", &[], &c);
    let study = compute_study(&parse_studies("ichimoku", 12)[0], &c);
    let built = |key: &str| study.lines.iter().find(|l| l.key == key).unwrap().values.clone();
    // Moved as the chart moves them: the leading spans ahead by 25 bars, the lagging line back by 25.
    let moved = |title: &str| -> Vec<f64> {
        let p = out.plots.iter().find(|p| p.title == title).unwrap();
        let k = p.offset.unsigned_abs() as usize;
        if p.offset >= 0 { std::iter::repeat_n(f64::NAN, k).chain(p.values.iter().copied()).collect() } else { p.values.iter().copied().skip(k).chain(std::iter::repeat_n(f64::NAN, k)).collect() }
    };
    for (title, key) in [("Conversion", "conversion"), ("Base", "base"), ("Lagging", "lagging"), ("Lead A", "leadA"), ("Lead B", "leadB")] {
        same(title, &moved(title), &built(key), true);
    }
    let out = run_source("avwap", &[], &c);
    let study = compute_study(&parse_studies("avwap", 12)[0], &c);
    for (title, key) in [("Session", "session"), ("Weekly", "weekly"), ("Monthly", "monthly"), ("Yearly", "yearly")] {
        same(title, line(&out, title), &study.lines.iter().find(|l| l.key == key).unwrap().values, true);
    }
}

#[test]
fn moon_phases_and_sessions_fall_where_the_charts_do() {
    let c = candles();
    let out = run_source("moon", &[], &c);
    let study = compute_study(&parse_studies("moon", 12)[0], &c);
    let index = |t: i64| c.iter().position(|k| k.open_time == t).unwrap();
    for (m, full) in [(0, true), (1, false)] {
        let got: Vec<usize> = out.marks[m].hits.iter().enumerate().filter(|(_, h)| **h).map(|(i, _)| i).collect();
        let want: Vec<usize> = study.markers.iter().filter(|x| x.above == full).map(|x| index(x.time)).collect();
        assert!(want.len() >= 3, "{} {} moons", want.len(), if full { "full" } else { "new" });
        assert_eq!(got, want, "{} moons", if full { "full" } else { "new" });
    }
    let out = run_source("sessions", &[], &c);
    let study = compute_study(&parse_studies("sessions", 12)[0], &c);
    let mut want: Vec<(usize, usize, f64, f64)> = study.boxes.iter().map(|b| (index(b.from), index(b.to), b.top, b.bottom)).collect();
    let mut got: Vec<(usize, usize, f64, f64)> = out.boxes.iter().map(|b| (b.x1 as usize, b.x2 as usize, b.y1, b.y2)).collect();
    let order = |a: &(usize, usize, f64, f64), b: &(usize, usize, f64, f64)| a.partial_cmp(b).unwrap();
    want.sort_by(order);
    got.sort_by(order);
    assert!(want.len() > 100, "{} sessions", want.len());
    assert_eq!(got.len(), want.len().min(crate::MAX_BOXES));
    assert_eq!(got, want[want.len() - got.len()..].to_vec(), "the newest of them, the oldest let go past the limit");
}

#[test]
fn a_description_lists_the_settings_and_lines() {
    let d = describe(source("stochrsi")).unwrap();
    assert!(d.own_pane);
    assert_eq!(d.inputs.iter().map(|i| (i.name.as_str(), i.default)).collect::<Vec<_>>(), [("rsiLength", 14.0), ("stochLength", 14.0), ("kSmoothing", 3.0), ("dSmoothing", 3.0)]);
    assert_eq!(d.inputs[0].label, "RSI length");
    assert_eq!(d.plots.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["%K", "%D", "Upper", "Lower"]);
    let bb = describe(source("bb")).unwrap();
    assert_eq!((bb.inputs[1].step, bb.inputs[1].min, bb.inputs[1].max), (0.1, Some(0.1), Some(10.0)));
}

fn check(src: &str) -> Result<Output, Error> {
    let c = candles();
    let k = columns(&c[..300]);
    run(src, &[], &Bars { time: &k.time, open: &k.open, high: &k.high, low: &k.low, close: &k.close, volume: &k.volume }, 2)
}

fn fails(src: &str, line: u32, says: &str) {
    match check(src) {
        Ok(_) => panic!("ran: {src}"),
        Err(e) => {
            assert_eq!(e.line, line, "{src}: {e}");
            assert!(e.message.contains(says), "{src}: {e}");
        }
    }
}

#[test]
fn mistakes_say_where_and_what() {
    fails("plot(foo, \"x\")", 1, "foo is not known");
    fails("x = 1\nplot(x, \"A\")\nplot(x, \"A\")", 3, "two lines are called");
    fails("x = ema(close)", 1, "ema(value, length)");
    fails("x = ema(close, close)", 1, "a length is a number or a setting");
    fails("x = ema(close, 0)", 1, "a length is from 1");
    fails("if close > open {\n  x = ema(close, 5)\n}", 2, "outside if, for and while");
    fails("if close > open {\n  plot(close, \"C\")\n}", 2, "outside if, for, while and functions");
    fails("close = 1", 1, "is built in");
    fails("input length = 5\nlength = 3", 2, "is a setting");
    fails("plot(close, \"C\", magenta)", 1, "magenta is not known");
    fails("plot(close, \"C\", #12345)", 1, "a colour is written");
    fails("x = (1 + 2", 1, ") was expected");
    fails("x = 1 y = 2", 1, "one statement a line");
    fails("input n = 50 min 60", 1, "between min and max");
    fails("state s = sma(close, 3)", 1, "plain value");
    fails("x = close[-1]\nplot(x, \"X\")", 1, "0 or more");
    fails("ema(close, 5)", 1, "name it");
    fails("x = \"text\" * 2", 1, "texts join with +");
    fails("x = 1\nx = \"text\"", 2, "holds a number");
    fails("fn f(a) {\n  return f(a)\n}\nx = f(1)", 2, "calls itself");
    fails("fn f() {\n  input n = 3\n}", 2, "go at the top level");
    fails("break", 1, "inside for or while");
    fails("xs = list()\ny = xs.get(0)\nplot(y, \"Y\")", 2, "not a place in this list");
}

#[test]
fn every_limit_holds() {
    // Brackets nested past the limit: an error, never a parser out of stack.
    let deep = format!("x = {}1{}", "(".repeat(1_000), ")".repeat(1_000));
    assert!(check(&deep).unwrap_err().message.contains("nested more than"));
    let deep_minus = format!("x = {}1", "-".repeat(1_000));
    assert!(check(&deep_minus).is_err());
    let deep_blocks = format!("{}x = 1{}", "if close > 0 {\n".repeat(200), "\n}".repeat(200));
    assert!(check(&deep_blocks).unwrap_err().message.contains("nested more than"));
    // Too long, too many words.
    assert!(check(&"x = 1\n".repeat(7_000)).unwrap_err().message.contains("characters at most"));
    // A loop past its rounds, and loops that together ask too much.
    fails("for i = 1 to 50000 {\n  x = i\n}", 1, "10000 times at most");
    fails("i = 0\nwhile true {\n  i = i + 1\n}", 2, "10000 times at most");
    let slow = "total = 0\nfor i = 1 to 1000 {\n  for j = 1 to 1000 {\n    total = total + j\n  }\n}\nplot(total, \"T\")";
    assert!(check(slow).unwrap_err().message.contains("takes too long"));
    fails("x = close[6000]\nplot(x, \"X\")", 1, "5000 bars at most");
    let many = (0..33).map(|i| format!("plot(close, \"P{i}\")")).collect::<Vec<_>>().join("\n");
    assert!(check(&many).unwrap_err().message.contains("32 lines at most"));
    // A list past its size lets its oldest go; shapes past theirs too.
    let out = check("state xs = list()\nxs.push(bar)\nplot(xs.size(), \"N\")\nplot(xs.first(), \"F\")").unwrap();
    assert_eq!(line(&out, "N")[299], 300.0);
    let out = check("l = label(bar, close, \"x\")").unwrap();
    assert_eq!(out.labels.len(), 300);
    let lines = check("l = line(bar, close, bar + 1, close)").unwrap();
    assert_eq!(lines.lines.len(), 300);
    // Functions written out at every call: a script that grows too large is refused, not run.
    let grow = format!("fn a(x) {{\n  return x + 1\n}}\n{}", (0..12).map(|i| format!("fn f{i}(x) {{\n  return {}\n}}", if i == 0 { "a(x) + a(x) + a(x) + a(x)".to_string() } else { format!("f{}(x) + f{}(x) + f{}(x) + f{}(x)", i - 1, i - 1, i - 1, i - 1) })).collect::<Vec<_>>().join("\n"));
    let refused = check(&format!("{grow}\nplot(f11(close), \"G\")")).unwrap_err().message;
    assert!(refused.contains("grows too large") || refused.contains("variables at most"), "{refused}");
}

#[test]
fn no_value_stays_no_value_and_division_by_zero_gives_none() {
    let out = check("a = close[1]\nb = max(a, 0)\nc = close / 0\nd = nz(a, -1)\nplot(b, \"B\")\nplot(c, \"C\")\nplot(d, \"D\")").unwrap();
    assert!(line(&out, "B")[0].is_nan(), "max(na, 0) is not 0");
    assert!(line(&out, "C").iter().all(|v| v.is_nan()));
    assert_eq!(line(&out, "D")[0], -1.0);
    assert!(line(&out, "B")[1] > 0.0);
}

#[test]
fn no_script_runs_the_checker_out_of_stack_memory_or_time() {
    // On a thread with room for a debug build's frames, many times a release build's.
    std::thread::Builder::new().stack_size(256 << 20).spawn(hostile_scripts_are_stopped).unwrap().join().unwrap();
}

fn hostile_scripts_are_stopped() {
    // A chain as long as the text allows is refused as it is read, before its tree is deep.
    let chain = format!("t = {}\nplot(close, \"C\")", vec!["\"ab\""; 5000].join(" + "));
    fails(&chain, 1, "more than 100 deep");
    // Functions that nest each other's results: refused at the limit.
    let nest = |n: usize| {
        let mut src = String::from("fn g0(x) {\n  return x + 1\n}\n");
        for i in 1..n {
            src += &format!("fn g{i}(x) {{\n  return {}g{}(x){}\n}}\n", "(".repeat(30), i - 1, " + 1)".repeat(30));
        }
        src + &format!("plot(g{}(close), \"G\")", n - 1)
    };
    assert!(check(&nest(64)).unwrap_err().message.contains("nests more than 200 deep"));
    assert_eq!(check(&nest(6)).map(|o| o.plots.len()), Ok(1));
    // As many long windows as a script can write: refused for what they would keep.
    let body = (0..40).map(|i| format!("median(x + {i}, 5000)")).collect::<Vec<_>>().join(" + ");
    let calls = (0..40).map(|j| format!("w(close + {j})")).collect::<Vec<_>>().join(" + ");
    fails(&format!("fn w(x) {{\n  return {body}\n}}\na = {calls}\nplot(a, \"A\")"), 2, "keeps more past values");
    // A list pushed while full lets its oldest go at no cost; a text grown costs what it copies,
    // so a loop of them stops at the step limit.
    assert_eq!(check("state xs = list()\nfor i = 0 to 9999 {\n  xs.push(i)\n}\nplot(xs.size(), \"N\")").map(|o| o.plots[0].values[5]), Ok(5000.0));
    assert!(check("state t = \"\"\nfor i = 0 to 9999 {\n  t = t + \"abcdefghij\"\n}\nplot(close, \"C\")").unwrap_err().message.contains("too long"));
    // x[1] keeps one bar, not five thousand: many of them fit.
    let ones = (0..400).map(|i| format!("v{i} = close + {i}\nw{i} = v{i}[1]")).collect::<Vec<_>>().join("\n") + "\nplot(w1, \"W\")";
    assert!(check(&ones).is_ok());
}

#[test]
fn a_variable_may_share_a_functions_name_but_not_a_bar_values() {
    // As scripts written before rising(…), label(…) and the rest were added do.
    let out = check("rising = close > close[1] and close[1] > close[2]\nsum = 0\nlabel = 2\nfor i = 0 to 2 {\n  sum = sum + close[i]\n}\nplot(rising, \"R\")\nplot(sum, \"S\")\nplot(rising(close, 2), \"F\")\nplot(label, \"L\")").unwrap();
    let c = candles();
    assert_eq!(line(&out, "R")[2], f64::from(u8::from(c[2].close > c[1].close && c[1].close > c[0].close)));
    assert!((line(&out, "S")[5] - (c[5].close + c[4].close + c[3].close)).abs() < 1e-9);
    assert_eq!(line(&out, "F")[2], line(&out, "R")[2]);
    assert_eq!(line(&out, "L")[0], 2.0);
    assert!(check("input change = 3\nfn f(label) {\n  return label * change\n}\ntable = f(2)\nplot(table, \"T\")").is_ok());
    fails("fn rising(x) {\n  return x\n}", 1, "is taken");
    fails("hour = 1", 1, "is built in");
    fails("input blue = 1", 1, "is taken");
}

#[test]
fn state_carries_from_bar_to_bar_and_a_loop_reads_back() {
    let out = check("state count = 0\ncount = count + 1\ntotal = 0\nfor i = 0 to 2 {\n  total = total + close[i]\n}\nplot(count, \"N\")\nplot(total, \"T\")").unwrap();
    assert_eq!(&line(&out, "N")[..3], &[1.0, 2.0, 3.0]);
    let c = candles();
    assert!(line(&out, "T")[1].is_nan(), "two bars back is before the first");
    assert!((line(&out, "T")[5] - (c[5].close + c[4].close + c[3].close)).abs() < 1e-9);
}

#[test]
fn a_function_that_remembers_hears_every_bar_even_on_the_side_not_taken() {
    // ema() sits in the branch not taken on odd bars, and still counts them.
    let out = check("e = bar % 2 == 0 ? ema(close, 10) : 0\nplot(e, \"E\")").unwrap();
    let want = indicators::ema(&closes(&candles()[..300]), 10);
    assert_eq!(line(&out, "E")[20], want[20].unwrap());
}
