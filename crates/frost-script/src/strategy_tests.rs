//! Backtests over bars made by hand, each fill worked out on paper first.
use super::*;

const HOUR: f64 = 3_600_000.0;

struct Cols {
    time: Vec<f64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
}

/// Hourly bars from (open, high, low, close).
fn cols(rows: &[(f64, f64, f64, f64)]) -> Cols {
    Cols {
        time: (0..rows.len()).map(|i| i as f64 * HOUR).collect(),
        open: rows.iter().map(|r| r.0).collect(),
        high: rows.iter().map(|r| r.1).collect(),
        low: rows.iter().map(|r| r.2).collect(),
        close: rows.iter().map(|r| r.3).collect(),
        volume: vec![1.0; rows.len()],
    }
}

fn flat(n: usize) -> Vec<(f64, f64, f64, f64)> {
    vec![(100.0, 101.0, 99.0, 100.0); n]
}

/// No fees, no slippage, the whole equity: the plainest account.
fn plain() -> Settings {
    Settings { taker: 0.0, maker: 0.0, ..Settings::default() }
}

fn test(src: &str, rows: &[(f64, f64, f64, f64)], s: Settings) -> Report {
    test_with(src, rows, s, Inner::default(), Funding::default())
}

fn test_with(src: &str, rows: &[(f64, f64, f64, f64)], s: Settings, inner: Inner, funding: Funding) -> Report {
    let c = cols(rows);
    let bars = Bars { time: &c.time, open: &c.open, high: &c.high, low: &c.low, close: &c.close, volume: &c.volume };
    match backtest(src, &[], &bars, 2, s, inner, funding) {
        Ok((_, r)) => r,
        Err(e) => panic!("{e}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * a.abs().max(1.0)
}

#[test]
fn an_order_fills_at_the_next_open_never_at_its_own_bar() {
    let mut rows = flat(6);
    rows[1] = (100.0, 101.0, 99.0, 100.0);
    rows[2] = (104.0, 106.0, 103.0, 105.0);
    rows[4] = (110.0, 111.0, 109.0, 110.0);
    let r = test("long(bar == 1)\nexit(bar == 3)", &rows, plain());
    assert_eq!(r.trades.len(), 1);
    let t = &r.trades[0];
    assert!(t.long);
    assert_eq!((t.entry_bar, t.exit_bar), (2, 4));
    assert_eq!((t.entry_price, t.exit_price), (104.0, 110.0));
    assert_eq!(t.reason, Reason::Signal);
    // The whole 10,000 at 104, sold at 110.
    assert!(close(t.pnl, 10_000.0 / 104.0 * 6.0));
    assert!(close(*r.equity.last().unwrap(), 10_000.0 + t.pnl));
}

#[test]
fn fees_and_slippage_are_paid_on_both_sides() {
    let mut rows = flat(5);
    rows[2] = (100.0, 101.0, 99.0, 100.0);
    rows[4] = (100.0, 101.0, 99.0, 100.0);
    let s = Settings { taker: 0.001, maker: 0.0, slippage: 0.002, ..Settings::default() };
    let r = test("long(bar == 1)\nexit(bar == 3)", &rows, s);
    let t = &r.trades[0];
    assert!(close(t.entry_price, 100.2));
    assert!(close(t.exit_price, 99.8));
    let qty = 10_000.0 / 100.2;
    let fees = qty * 100.2 * 0.001 + qty * 99.8 * 0.001;
    assert!(close(t.fees, fees));
    assert!(close(t.pnl, qty * (99.8 - 100.2) - fees));
}

#[test]
fn a_stop_and_a_target_in_one_bar_take_the_stop_and_say_so() {
    let mut rows = flat(5);
    rows[3] = (100.0, 106.0, 94.0, 100.0);
    let s = Settings { stop: Exit::Pct(0.05), target: Exit::Pct(0.05), ..plain() };
    let r = test("long(bar == 1)", &rows, s);
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Stop);
    assert!(close(t.exit_price, 95.0));
    assert_eq!(r.ambiguous, vec![3]);
    assert_eq!(r.magnified, 0);
}

#[test]
fn the_lower_bars_say_which_came_first() {
    let mut rows = flat(5);
    rows[3] = (100.0, 106.0, 94.0, 100.0);
    let s = Settings { stop: Exit::Pct(0.05), target: Exit::Pct(0.05), ..plain() };
    // Inside bar 3: up to 106 first, then down to 94.
    let t0 = 3.0 * HOUR;
    let time = [t0, t0 + 900_000.0, t0 + 1_800_000.0, t0 + 2_700_000.0];
    let open = [100.0, 103.0, 105.0, 98.0];
    let high = [103.0, 106.0, 105.5, 99.0];
    let low = [99.5, 102.5, 97.0, 94.0];
    let close_ = [103.0, 105.0, 98.0, 100.0];
    let inner = Inner { time: &time, open: &open, high: &high, low: &low, close: &close_ };
    let r = test_with("long(bar == 1)", &rows, s, inner, Funding::default());
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Target);
    assert!(close(t.exit_price, 105.0));
    assert!(r.ambiguous.is_empty());
    assert_eq!(r.magnified, 1);
}

#[test]
fn a_gap_past_the_stop_fills_at_the_open() {
    let mut rows = flat(5);
    rows[3] = (90.0, 91.0, 88.0, 89.0);
    let s = Settings { stop: Exit::Pct(0.05), ..plain() };
    let r = test("long(bar == 1)", &rows, s);
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Stop);
    assert_eq!(t.exit_price, 90.0);
    assert!(r.ambiguous.is_empty());
}

#[test]
fn a_target_gapped_past_fills_at_the_better_open() {
    let mut rows = flat(5);
    rows[3] = (112.0, 113.0, 111.0, 112.0);
    let s = Settings { target: Exit::Pct(0.05), ..plain() };
    let r = test("long(bar == 1)", &rows, s);
    assert_eq!(r.trades[0].reason, Reason::Target);
    assert_eq!(r.trades[0].exit_price, 112.0);
}

#[test]
fn a_limit_fills_only_where_the_price_reaches_it() {
    let mut rows = flat(6);
    // Bar 2 never comes down to 97: no fill, and the order lasts one bar.
    rows[2] = (100.0, 101.0, 98.0, 100.0);
    let r = test("long(bar == 1, 97)", &rows, plain());
    assert!(r.trades.is_empty());
    rows[2] = (100.0, 101.0, 96.0, 100.0);
    let r = test("long(bar == 1, 97)", &rows, plain());
    assert_eq!(r.trades[0].entry_price, 97.0);
    assert_eq!(r.trades[0].reason, Reason::Open);
    // Opened below the limit: filled at the open, the better price.
    rows[2] = (95.0, 101.0, 94.0, 100.0);
    let r = test("long(bar == 1, 97)", &rows, plain());
    assert_eq!(r.trades[0].entry_price, 95.0);
}

#[test]
fn a_limit_filled_inside_a_bar_takes_no_target_it_may_have_passed_before() {
    let mut rows = flat(6);
    rows[2] = (100.0, 104.0, 96.0, 100.0);
    let s = Settings { target: Exit::Pct(0.04), ..plain() };
    let r = test("long(bar == 1, 98)", &rows, s);
    let t = &r.trades[0];
    assert_eq!(t.entry_price, 98.0);
    // 101.92 was reached in the bar, but maybe before the fill: not taken, and asked of the lower bars.
    assert_eq!(t.reason, Reason::Open);
    assert_eq!(r.ambiguous, vec![2]);
}

#[test]
fn filled_at_the_close_when_asked() {
    let mut rows = flat(5);
    rows[1] = (100.0, 101.0, 99.0, 102.0);
    rows[3] = (100.0, 101.0, 99.0, 108.0);
    let r = test("long(bar == 1)\nexit(bar == 3)", &rows, Settings { on_close: true, ..plain() });
    let t = &r.trades[0];
    assert_eq!((t.entry_bar, t.exit_bar), (1, 3));
    assert_eq!((t.entry_price, t.exit_price), (102.0, 108.0));
}

#[test]
fn leverage_loses_the_whole_margin_at_the_liquidation_price() {
    let mut rows = flat(5);
    rows[3] = (100.0, 100.5, 85.0, 86.0);
    let s = Settings { leverage: 10.0, mmr: 0.005, ..plain() };
    let r = test("long(bar == 1)", &rows, s);
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Liquidation);
    // (100·qty − margin) / (qty · 0.995), the margin a tenth of what it held.
    assert!(close(t.exit_price, 90.0 / 0.995));
    assert!(close(t.pnl, -10_000.0));
    assert!(close(*r.equity.last().unwrap(), 0.0));
}

#[test]
fn a_stop_above_the_liquidation_closes_it_first() {
    let mut rows = flat(5);
    rows[3] = (100.0, 100.5, 85.0, 86.0);
    let s = Settings { leverage: 10.0, mmr: 0.005, stop: Exit::Pct(0.05), ..plain() };
    let r = test("long(bar == 1)", &rows, s);
    assert_eq!(r.trades[0].reason, Reason::Stop);
    assert!(close(r.trades[0].exit_price, 95.0));
}

#[test]
fn funding_is_paid_by_a_long_held_as_it_settles() {
    let rows = flat(6);
    // Settled at bar 3's open, and inside bar 4.
    let time = [3.0 * HOUR, 4.5 * HOUR];
    let rate = [0.001, 0.001];
    let r = test_with("long(bar == 1)", &rows, plain(), Inner::default(), Funding { time: &time, rate: &rate });
    let t = &r.trades[0];
    let qty = 10_000.0 / 100.0;
    assert!(close(t.funding, 2.0 * qty * 100.0 * 0.001));
    assert!(close(t.pnl, -t.funding));
}

#[test]
fn funding_before_the_entry_is_not_paid() {
    let rows = flat(6);
    // At bar 2's open, as the position is entered: settled before it held anything.
    let time = [2.0 * HOUR];
    let rate = [0.01];
    let r = test_with("long(bar == 1)", &rows, plain(), Inner::default(), Funding { time: &time, rate: &rate });
    assert_eq!(r.trades[0].funding, 0.0);
}

#[test]
fn risk_sizing_loses_its_share_at_the_stop() {
    let mut rows = flat(5);
    rows[3] = (100.0, 100.5, 97.0, 98.0);
    let s = Settings { size: Size::Risk(0.01), stop: Exit::Pct(0.02), ..plain() };
    let r = test("long(bar == 1)", &rows, s);
    let t = &r.trades[0];
    assert!(close(t.qty, 100.0 / 2.0));
    assert!(close(t.risk, 100.0));
    assert!(close(t.pnl, -100.0));
}

#[test]
fn risk_sizing_without_a_stop_takes_no_trade() {
    let rows = flat(5);
    let r = test("long(bar == 1)", &rows, Settings { size: Size::Risk(0.01), ..plain() });
    assert!(r.trades.is_empty());
    assert_eq!(r.skipped, 1);
}

#[test]
fn the_other_side_reverses_it() {
    let mut rows = flat(7);
    rows[3] = (102.0, 103.0, 101.0, 102.0);
    let r = test("long(bar == 1)\nshort(bar == 2)", &rows, plain());
    assert_eq!(r.trades.len(), 2);
    assert_eq!(r.trades[0].reason, Reason::Reverse);
    assert_eq!(r.trades[0].exit_bar, 3);
    assert!(!r.trades[1].long);
    assert_eq!(r.trades[1].entry_bar, 3);
    assert_eq!(r.trades[1].entry_price, 102.0);
}

#[test]
fn a_script_reads_its_position() {
    let rows = flat(10);
    let r = test("long(bar == 1)\nexit(barsheld >= 3)\nmark(inlong and entryprice == 100, \"In\")", &rows, plain());
    let t = &r.trades[0];
    assert_eq!((t.entry_bar, t.exit_bar), (2, 6));
    assert_eq!(t.reason, Reason::Signal);
}

#[test]
fn a_script_stop_from_the_entry_price() {
    let mut rows = flat(6);
    rows[4] = (100.0, 100.5, 96.0, 97.0);
    let r = test("long(bar == 1)\nstop(entryprice - 3)", &rows, plain());
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Stop);
    assert_eq!(t.exit_price, 97.0);
    assert_eq!(t.exit_bar, 4);
}

#[test]
fn a_trailing_stop_follows_the_best_price() {
    let rows = [
        (100.0, 101.0, 99.0, 100.0),
        (100.0, 101.0, 99.0, 100.0),
        (100.0, 105.0, 99.5, 104.0),
        (104.0, 110.0, 103.0, 109.0),
        (109.0, 109.5, 98.0, 99.0),
        (99.0, 100.0, 98.0, 99.0),
    ];
    let r = test("long(bar == 1)", &rows, Settings { trail: Exit::Pct(0.05), ..plain() });
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Trail);
    assert!(close(t.exit_price, 110.0 * 0.95));
}

#[test]
fn an_atr_stop_and_an_r_target() {
    let mut rows = flat(20);
    // Every bar's range is 2: the ATR is 2, a 1.5 ATR stop 3 away, a 2R target 6 away.
    rows[17] = (100.0, 106.5, 99.0, 106.0);
    let s = Settings { stop: Exit::Atr(1.5), target: Exit::R(2.0), atr_length: 14, ..plain() };
    let r = test("long(bar == 15)", &rows, s);
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Target);
    assert!(close(t.exit_price, 106.0));
    assert!(close(t.risk, 100.0 * 3.0));
}

#[test]
fn held_no_longer_than_asked() {
    let rows = flat(10);
    let r = test("long(bar == 1)", &rows, Settings { max_bars: 2, ..plain() });
    assert_eq!(r.trades[0].reason, Reason::Time);
    assert_eq!(r.trades[0].exit_bar, 4);
}

#[test]
fn opened_and_closed_within_its_range() {
    let rows = flat(10);
    let s = Settings { from: 3.0 * HOUR, to: 6.0 * HOUR, ..plain() };
    let r = test("long(true)", &rows, s);
    assert_eq!(r.trades.len(), 1);
    assert_eq!(r.trades[0].entry_bar, 4);
    assert_eq!(r.trades[0].reason, Reason::End);
    assert_eq!(r.trades[0].exit_bar, 7);
}

#[test]
fn only_one_side_where_asked() {
    let rows = flat(8);
    let r = test("long(bar == 1)\nshort(bar == 3)", &rows, Settings { shorts: false, ..plain() });
    assert_eq!(r.trades.len(), 1);
    assert!(r.trades[0].long);
}

#[test]
fn the_furthest_against_and_for() {
    let rows = [
        (100.0, 101.0, 99.0, 100.0),
        (100.0, 101.0, 99.0, 100.0),
        (100.0, 104.0, 97.0, 100.0),
        (100.0, 108.0, 99.0, 100.0),
        (100.0, 101.0, 99.0, 100.0),
    ];
    let r = test("long(bar == 1)\nexit(bar == 3)", &rows, plain());
    let t = &r.trades[0];
    assert!(close(t.mae, 0.03));
    assert!(close(t.mfe, 0.08));
}

#[test]
fn a_strategy_needs_an_entry() {
    let c = cols(&flat(5));
    let bars = Bars { time: &c.time, open: &c.open, high: &c.high, low: &c.low, close: &c.close, volume: &c.volume };
    let none = backtest("exit(true)", &[], &bars, 2, plain(), Inner::default(), Funding::default());
    assert!(none.is_err_and(|e| e.message.contains("long(condition)")));
}

#[test]
fn orders_inside_if_as_pine_gives_them() {
    let mut rows = flat(6);
    rows[2] = (104.0, 106.0, 103.0, 105.0);
    rows[4] = (110.0, 111.0, 109.0, 110.0);
    let r = test("if bar == 1 {\n  long(true)\n}\nif bar == 3 {\n  exit(true)\n}", &rows, plain());
    assert_eq!(r.trades.len(), 1);
    assert_eq!((r.trades[0].entry_bar, r.trades[0].exit_bar), (2, 4));
}

#[test]
fn an_entry_given_twice_stands_where_either_is_true() {
    let rows = flat(8);
    let r = test("long(bar == 1)\nlong(bar == 4)\nexit(bar == 2)", &rows, plain());
    assert_eq!(r.trades.len(), 2);
    assert_eq!(r.trades[1].entry_bar, 5);
}

#[test]
fn a_stop_given_once_stands_until_the_position_closes() {
    let mut rows = flat(8);
    rows[5] = (100.0, 100.5, 96.0, 97.0);
    // Given on the entry bar alone, as Pine's strategy.exit is: it guards the bars after it.
    let r = test("long(bar == 1)\nif inlong and barsheld == 0 {\n  stop(entryprice - 3)\n}", &rows, plain());
    let t = &r.trades[0];
    assert_eq!(t.reason, Reason::Stop);
    assert_eq!((t.exit_bar, t.exit_price), (5, 97.0));
}

#[test]
fn a_closed_position_takes_its_stop_with_it() {
    let mut rows = flat(10);
    rows[7] = (100.0, 100.5, 95.0, 96.0);
    // Stopped trade one's stop is gone: the second, entered at bar 5, has none and stays open.
    let r = test("long(bar == 1 or bar == 4)\nexit(bar == 2)\nif bar == 1 {\n  stop(99)\n}", &rows, plain());
    assert_eq!(r.trades.len(), 2);
    assert_eq!(r.trades[1].reason, Reason::Open);
}

#[test]
fn an_indicator_run_reads_no_position() {
    let c = cols(&flat(5));
    let bars = Bars { time: &c.time, open: &c.open, high: &c.high, low: &c.low, close: &c.close, volume: &c.volume };
    let out = run("plot(inlong + inshort, \"A\")\nlong(close > 0)", &[], &bars, 2).unwrap();
    assert!(out.plots[0].values.iter().all(|v| *v == 0.0));
    let d = describe("long(close > 0)").unwrap();
    assert!(d.strategy);
    assert!(!describe("plot(close)").unwrap().strategy);
}

#[test]
fn both_sides_at_once_ask_nothing() {
    let rows = flat(6);
    let r = test("long(bar == 1)\nshort(bar == 1)", &rows, plain());
    assert!(r.trades.is_empty());
}

#[test]
fn the_order_waiting_for_the_next_open_is_said() {
    let rows = flat(5);
    let r = test("long(bar == 4, 99)", &rows, plain());
    assert_eq!(r.next, 1);
    assert_eq!(r.next_limit, 99.0);
    let r = test("long(bar == 1)\nexit(bar == 4)", &rows, plain());
    assert_eq!(r.next, 2);
    let r = test("long(bar == 1)", &rows, plain());
    assert_eq!(r.next, 0);
}

#[test]
fn a_label_s_place_and_size_kept_in_variables_and_settings() {
    let c = cols(&flat(5));
    let bars = Bars { time: &c.time, open: &c.open, high: &c.high, low: &c.low, close: &c.close, volume: &c.volume };
    // As converted scripts write them: the place worked out into a name, the size a choice setting.
    let src = "input size = \"large\" from \"tiny\", \"small\", \"normal\", \"large\"\nwhere = close > open ? above : below\nif islast {\n  l = label(bar, close, \"x\", place = where, size = size)\n}";
    let out = run(src, &[], &bars, 2).unwrap();
    assert_eq!(out.labels.len(), 1);
    assert_eq!((out.labels[0].place, out.labels[0].size), (1, 3));
    // A variable of the same name as a word is the variable's.
    let own = run("left = 7\nplot(left, \"L\")", &[], &bars, 2).unwrap();
    assert_eq!(own.plots[0].values[0], 7.0);
}
