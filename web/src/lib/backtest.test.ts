/// <reference lib="deno.ns" />
import { assert, assertAlmostEquals, assertEquals } from 'jsr:@std/assert@1';
import { TRADE_SIZE, type BacktestResult } from '../engine/protocol.ts';
import { monteCarlo, monthly, orderNotice, rulesScript, rulesSource, settingsOf, sharedOf, shareOf, statsOf, tradesCsv, tradesOf, withSettings } from './backtest.ts';
import { DEFAULT_RULES, DEFAULT_SETTINGS, defaultBacktest, readBacktest } from './backtest-state.ts';

const HOUR = 3_600_000;

/** A report of `n` hourly bars with the equity given, and trades as [long, entryBar, exitBar, pnl, equityBefore, risk]. */
function report(equity: number[], trades: [number, number, number, number, number, number][], close = equity): BacktestResult & { ok: true } {
  const t = new Float64Array(trades.length * TRADE_SIZE);
  trades.forEach(([long, a, b, pnl, before, risk], i) => {
    t.set([long, a, a * HOUR, 100, b, b * HOUR, 101, 1, 100, pnl, 1, 0, risk, 1, 0.01, 0.02, before], i * TRADE_SIZE);
  });
  return {
    ok: true,
    market: 'binance-perp:BTCUSDT',
    timeframe: '1h',
    time: Float64Array.from(equity, (_, i) => Date.UTC(2026, 0, 1) + i * HOUR),
    close: Float64Array.from(close),
    trades: t,
    equity: Float64Array.from(equity),
    ambiguous: 0,
    magnified: 0,
    inner: '',
    reading: false,
    skipped: 0,
    held: 0,
    next: 0,
    nextLimit: Number.NaN,
    funding: 'off',
    settlements: 0,
  };
}

Deno.test('the rule builder writes each value once, its lengths as settings', () => {
  const src = rulesSource(DEFAULT_RULES());
  assertEquals(src.match(/^input /gm)?.length, 2, 'EMA 9 and EMA 21, each once though both entries read them');
  assert(src.includes('long(goLong)') && src.includes('short(goShort)'));
  assert(!src.includes('exit('), 'no exit rules: none written');
});

Deno.test('the optimizer’s pick goes back to every rule its setting came from', () => {
  const rules = DEFAULT_RULES();
  const { places } = rulesScript(rules);
  const next = withSettings(rules, places, [12, 30]);
  assertEquals([next.long.list[0]!.a.n, next.long.list[0]!.b.n], [12, 30]);
  assertEquals([next.short.list[0]!.a.n, next.short.list[0]!.b.n], [12, 30], 'the same EMAs in the short rule too');
  assertEquals(rules.long.list[0]!.a.n, 9, 'the rules given are not changed');
});

Deno.test('an exit rule reads only the side it closes', () => {
  const rules = DEFAULT_RULES();
  rules.exitLong.list = [{ a: { kind: 'rsi', n: 14, m: 0, k: 0 }, op: 'gt', b: { kind: 'value', n: 70, m: 0, k: 0 } }];
  const src = rulesSource(rules);
  assert(src.includes('exit((inlong and leaveLong))'));
  assert(src.includes('input level'), 'the level is a setting');
});

Deno.test('the settings as the backtest reads them', () => {
  const s = { ...DEFAULT_SETTINGS(), stop: { kind: 'pct', v: 2 }, target: { kind: 'r', v: 3 }, size: 50, days: 10 };
  const v = settingsOf(s, 'binance-perp:BTCUSDT', 1_000 * 86_400_000);
  assertEquals(v.length, 21);
  assertEquals(v.slice(0, 4), [10_000, 0, 0.5, 1]);
  assertAlmostEquals(v[4]!, 0.0005);
  assertAlmostEquals(v[5]!, 0.0002);
  assertEquals(v.slice(10, 14), [1, 0.02, 3, 3]);
  assertEquals(v[18], 0.004, 'a perpetual is liquidated at its first tier');
  assertEquals(v[19], 990 * 86_400_000);
  assert(Number.isNaN(v[20]!));
  assertEquals(settingsOf({ ...s, autoFees: false, taker: 0.1, maker: 0 }, 'okx-spot:BTCUSDT', 0)[4], 0.001);
  assertEquals(settingsOf(DEFAULT_SETTINGS(), 'binance-spot:BTCUSDT', 0)[18], 0, 'spot without leverage: no liquidation');
});

Deno.test('the figures of a report', () => {
  const r = report([10_000, 10_500, 10_200, 9_800, 10_400, 10_900], [
    [1, 0, 1, 500, 10_000, 250],
    [0, 1, 3, -700, 10_500, 350],
    [1, 3, 5, 1_100, 9_800, Number.NaN],
  ]);
  const s = statsOf(r, tradesOf(r.trades));
  assertEquals([s.trades, s.wins, s.losses], [3, 2, 1]);
  assertAlmostEquals(s.net, 900);
  assertAlmostEquals(s.netPct, 0.09);
  assertAlmostEquals(s.profitFactor, 1600 / 700);
  assertAlmostEquals(s.maxDdPct, 700 / 10_500);
  assertAlmostEquals(s.avgR, (2 - 2) / 2, 1e-9, 'R where a stop was known: +2R and −2R');
  assertEquals(s.rTrades, 2);
  assertEquals([s.winStreak, s.lossStreak], [1, 1]);
});

Deno.test('a part of the bars is reported alone', () => {
  const r = report([10_000, 10_500, 10_200, 9_800, 10_400, 10_900], [
    [1, 0, 1, 500, 10_000, 250],
    [1, 3, 5, 1_100, 9_800, 0],
  ]);
  const rows = tradesOf(r.trades);
  const kept = statsOf(r, rows, 3);
  assertEquals(kept.trades, 1);
  assertAlmostEquals(kept.startEquity, 10_200, 1e-9, 'from the close before the part');
  assertAlmostEquals(kept.net, 700);
});

Deno.test('months, and the trades drawn again at random', () => {
  const r = report(Array.from({ length: 24 * 40 }, (_, i) => 10_000 + i), []);
  const m = monthly(r);
  assertEquals(m.map(([y, mo]: [number, number, number]) => `${y}-${mo}`), ['2026-0', '2026-1']);
  const trades = tradesOf(report([1, 1, 1, 1, 1, 1, 1], Array.from({ length: 6 }, (_, i): [number, number, number, number, number, number] => [1, i, i + 1, i % 2 ? -100 : 150, 10_000, 100])).trades);
  const a = monteCarlo(trades);
  const b = monteCarlo(trades);
  assert(a && b);
  assertEquals(a, b, 'the same draws each time it is shown');
  assert(a.ret[0] <= a.ret[1] && a.ret[1] <= a.ret[2]);
  assertEquals(monteCarlo(trades.slice(0, 4)), null, 'too few to say anything');
});

Deno.test('a stored tester is read with every value checked', () => {
  const t = readBacktest({ open: true, source: 'evil', settings: { leverage: 1e9, sizeKind: 'all', capital: -5, stop: { kind: 'pct', v: 3 } }, code: 'x'.repeat(30_000), script: '<b>', rules: { long: { all: false, list: Array(20).fill({ a: { kind: 'nope' }, op: 'xa', b: {} }) } } });
  assert(t);
  assertEquals(t.source, 'rules');
  assertEquals(t.settings.leverage, 1);
  assertEquals(t.settings.sizeKind, 'equity');
  assertEquals(t.settings.capital, 10_000);
  assertEquals(t.settings.stop, { kind: 'pct', v: 3 });
  assertEquals(t.code.length, 20_000);
  assertEquals(t.script, '');
  assertEquals(t.rules.long.list.length, 6);
  assertEquals(t.rules.long.list[0]!.a.kind, 'close');
  assertEquals(readBacktest('nope'), null);
});

Deno.test('a shared link carries the strategy and its account', () => {
  const t = { ...defaultBacktest(), source: 'code', code: 'long(close > open) // é ✓' };
  const back = readBacktest(sharedOf(shareOf(t)));
  assertEquals(back?.code, t.code);
  assertEquals(back?.settings, t.settings);
  assertEquals(sharedOf('%%%'), null);
});

Deno.test('a notice for an order only once a new bar has closed', () => {
  const r = report([1, 1, 1], []);
  const first = orderNotice({ ...r, next: 1 }, '');
  assertEquals(first.what, '', 'the first report of a market: nothing has just closed');
  const same = orderNotice({ ...r, next: 1 }, first.seen);
  assertEquals(same.what, '', 'the same bar again');
  const later = report([1, 1, 1, 1], []);
  assertEquals(orderNotice({ ...later, next: 1 }, first.seen).what, 'Long at the next open');
  assertEquals(orderNotice({ ...later, next: 2 }, first.seen).what, 'Exit at the next open');
  assertEquals(orderNotice({ ...later, next: 0 }, first.seen).what, '');
});

Deno.test('the trades as a spreadsheet', () => {
  const r = report([10_000, 10_500, 10_200], [[1, 0, 1, 500, 10_000, 250], [0, 1, 2, -300, 10_500, Number.NaN]]);
  const lines = tradesCsv(tradesOf(r.trades), 'binance-perp:BTCUSDT').split('\n');
  assertEquals(lines.length, 3);
  assertEquals(lines[0]!.split(',').length, 18);
  const first = lines[1]!.split(',');
  assertEquals([first[0], first[1], first[2], first[9], first[11], first[14]], ['1', 'binance-perp:BTCUSDT', 'Long', '500.00', '2.00', 'Signal']);
  assertEquals(lines[2]!.split(',')[11], '', 'no R without a stop');
});
