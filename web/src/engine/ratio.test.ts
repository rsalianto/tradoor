/// <reference lib="deno.ns" />
/**
 * One market priced in another: the arithmetic, and the series that keeps it
 * as its two sides load. The sides are faked; nothing here touches a network.
 *
 * Run with `deno task test`; tsc skips this file (see tsconfig exclude).
 */
import { assert, assertEquals } from 'jsr:@std/assert@1';
import type { Cols, Series } from './history.ts';
import { divide, RatioSeries, ratioBar, ratioDecimals } from './ratio.ts';

const cols = (rows: [number, number, number, number, number][]): Cols => ({
  time: Float64Array.from(rows.map((r) => r[0])),
  open: Float64Array.from(rows.map((r) => r[1])),
  high: Float64Array.from(rows.map((r) => r[2])),
  low: Float64Array.from(rows.map((r) => r[3])),
  close: Float64Array.from(rows.map((r) => r[4])),
  volume: new Float64Array(rows.length),
  buy: new Float64Array(rows.length).fill(-1),
});

Deno.test('a ratio bar divides like for like, and takes in its open and close', () => {
  const r = ratioBar({ open: 2, high: 4, low: 1, close: 3 }, { open: 1, high: 2, low: 1, close: 1 });
  // High over high is 2, below the close of 3: the bar is widened to hold it.
  assertEquals(r, { open: 2, high: 3, low: 1, close: 3 });
  assertEquals(ratioBar({ open: 1, high: 1, low: 1, close: 1 }, { open: 0, high: 1, low: 1, close: 1 }), null, 'nothing to divide by');
});

Deno.test('a ratio keeps only the bars both markets have', () => {
  const a = cols([[1, 10, 12, 9, 11], [2, 11, 13, 10, 12], [3, 12, 12, 12, 12]]);
  const b = cols([[2, 2, 2, 2, 2], [3, 0, 1, 1, 1], [4, 5, 5, 5, 5]]);
  const r = divide(a, b);
  // Time 1 is only a's, 4 only b's, and at 3 b has no open to divide by.
  assertEquals([...r.time], [2]);
  assertEquals([r.open[0], r.close[0]], [5.5, 6]);
  assertEquals([...r.volume], [0], 'a ratio is not traded');
});

/** A side as the ratio sees one: bars, history to ask for, and a way to say it changed. */
function fakeLeg(bars: Cols) {
  const listeners = new Set<() => void>();
  const asked: number[] = [];
  const leg = {
    cols: bars,
    exhausted: false,
    interval: '1h',
    source: 'exchange' as const,
    subscribe(l: () => void) {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    ensure: (n: number) => {
      asked.push(n);
      return Promise.resolve();
    },
    older: () => Promise.resolve(false),
    refreshTail: () => Promise.resolve(),
  };
  return { leg, asked, listeners, emit: () => listeners.forEach((l) => l()) };
}

Deno.test('a ratio series asks each side for its own bars and follows both', async () => {
  const a = fakeLeg(cols([[1, 10, 10, 10, 10]]));
  const b = fakeLeg(cols([[1, 5, 5, 5, 5]]));
  const series = new RatioSeries([a.leg as unknown as Series, b.leg as unknown as Series], '1h', (c) => c, (_, bars) => bars * 4);
  await series.ensure(100);
  assertEquals([a.asked, b.asked], [[400], [400]], 'each side, in its own bars');
  assertEquals(series.cols.close[0], 2);

  let heard = 0;
  const off = series.subscribe(() => (heard += 1));
  assertEquals([a.listeners.size, b.listeners.size], [1, 1], 'one subscription to each side, however many listen');
  a.leg.cols = cols([[1, 10, 10, 10, 10], [2, 20, 20, 20, 20]]);
  b.leg.cols = cols([[1, 5, 5, 5, 5], [2, 4, 4, 4, 4]]);
  a.emit();
  assertEquals([heard, [...series.cols.close]], [1, [2, 5]], 'a side that moves moves the ratio');

  b.leg.exhausted = true;
  assert(series.exhausted, 'older history ends where either side runs out');
  off();
  assertEquals([a.listeners.size, b.listeners.size], [0, 0], 'and the sides are let go with the last listener');
});

Deno.test('the forming ratio waits for both sides to stand on one bar, and is as live as the worse', () => {
  const series = new RatioSeries([fakeLeg(cols([])).leg as unknown as Series, fakeLeg(cols([])).leg as unknown as Series], '1h', (c) => c, (_, n) => n);
  const k = (openTime: number, price: number, closed = false) => ({ openTime, open: price, high: price, low: price, close: price, closed });
  assertEquals(series.join(0, k(100, 10)), null, 'one side alone says nothing');
  assertEquals(series.join(1, k(100, 4)), { open: 2.5, high: 2.5, low: 2.5, close: 2.5, openTime: 100, closed: false });
  assertEquals(series.join(0, k(200, 12)), null, 'one side has rolled to the next bar, the other not yet');
  assertEquals(series.join(1, k(200, 3, true))?.close, 4);

  assertEquals(series.stream(0, 'live', false), ['connecting', false], 'live only once both are');
  assertEquals(series.stream(1, 'live', true), ['live', true], 'and relayed if either is');
  assertEquals(series.stream(0, 'retrying', false), ['retrying', true]);
});

Deno.test('a ratio is shown to the decimals its own moves need', () => {
  const bars = (price: number, range: number) => cols(Array.from({ length: 5 }, (_, i) => [i, price, price + range / 2, price - range / 2, price] as [number, number, number, number, number]));
  assertEquals(ratioDecimals(bars(0.00000114, 0.000000015)), 9, 'XPL in BTC: a tiny price moving in its ninth decimal');
  assertEquals(ratioDecimals(bars(1.0003, 0.0002)), 5, "BTC's perpetual in its spot: all the interest past the point");
  assertEquals(ratioDecimals(bars(31.07, 0.2)), 2, 'BTC in ETH: as any price');
  assertEquals(ratioDecimals(bars(0.03218, 0.0002)), 5);
  assertEquals(ratioDecimals(cols([])), 8, 'nothing loaded yet');
});
