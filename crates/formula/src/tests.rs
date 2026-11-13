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
        _ => panic!("no source {name}"),
    }
}

fn run_source(name: &str, inputs: &[f64], c: &[Candle]) -> Output {
    let k = columns(c);
    let bars = Bars { time: &k.time, open: &k.open, high: &k.high, low: &k.low, close: &k.close, volume: &k.volume };
    run(source(name), inputs, &bars).unwrap_or_else(|e| panic!("{name}: {e}"))
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
    assert_eq!(out.fills, vec![Fill { upper: 0, lower: 2, color: 5 }]);

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
    run(src, &[], &Bars { time: &k.time, open: &k.open, high: &k.high, low: &k.low, close: &k.close, volume: &k.volume })
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
    fails("if close > open {\n  x = ema(close, 5)\n}", 2, "outside if and for");
    fails("if close > open {\n  plot(close, \"C\")\n}", 2, "outside if and for");
    fails("close = 1", 1, "is built in");
    fails("input length = 5\nlength = 3", 2, "is a setting");
    fails("plot(close, \"C\", magenta)", 1, "a colour is one of");
    fails("x = (1 + 2", 1, ") was expected");
    fails("x = 1 y = 2", 1, "one statement a line");
    fails("input n = 50 min 60", 1, "between min and max");
    fails("state s = sma(close, 3)", 1, "plain value");
    fails("x = close[-1]\nplot(x, \"X\")", 1, "0 or more");
    fails("ema(close, 5)", 1, "name it");
    fails("x = \"text\"", 1, "only in a title");
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
    assert!(check(&"x = 1\n".repeat(5_000)).unwrap_err().message.contains("characters at most"));
    // A loop past its rounds, and loops that together ask too much.
    fails("for i = 1 to 5000 {\n  x = i\n}", 1, "1000 times at most");
    let slow = "total = 0\nfor i = 1 to 1000 {\n  for j = 1 to 1000 {\n    total = total + j\n  }\n}\nplot(total, \"T\")";
    assert!(check(slow).unwrap_err().message.contains("takes too long"));
    fails("x = close[6000]\nplot(x, \"X\")", 1, "5000 bars at most");
    let many = (0..17).map(|i| format!("plot(close, \"P{i}\")")).collect::<Vec<_>>().join("\n");
    assert!(check(&many).unwrap_err().message.contains("16 lines at most"));
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
