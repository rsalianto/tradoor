/// <reference lib="deno.ns" />
/**
 * A series held from before — by a chart once shown, a call, a row looked
 * at — and asked for again. The exchange is faked; nothing here touches a
 * network.
 *
 * Run with `deno task test`; tsc skips this file (see tsconfig exclude).
 */
import { assertEquals } from 'jsr:@std/assert@1';
import { concatCols, emptyCols, sameCols, Series } from './history.ts';
import { parseMarket } from '../lib/markets.ts';

const MINUTE = 60_000;

/** Binance's perpetual klines of a minute up to `until`, the forming one included, as its REST answers. */
function fakeBinance(until: () => number): () => void {
  const real = globalThis.fetch;
  globalThis.fetch = ((input: string | URL | Request) => {
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
    const limit = Number(url.searchParams.get('limit'));
    const end = Math.min(Number(url.searchParams.get('endTime') ?? until()), until());
    const start = Number(url.searchParams.get('startTime') ?? end - (limit - 1) * MINUTE);
    const rows: unknown[][] = [];
    for (let t = Math.ceil(start / MINUTE) * MINUTE; t <= end && rows.length < limit; t += MINUTE) rows.push([t, '1', '2', '0.5', '1.5', '10']);
    // The newest `limit` of them, as Binance answers an end time.
    return Promise.resolve(new Response(JSON.stringify(url.searchParams.has('startTime') ? rows : rows.slice(-limit)), { status: 200 }));
  }) as typeof fetch;
  return () => void (globalThis.fetch = real);
}

Deno.test('a series asked for again takes in the bars that closed since it was loaded', async () => {
  const now = Date.now();
  // Loaded when the exchange had nothing past five minutes ago.
  let until = now - 5 * MINUTE;
  const restore = fakeBinance(() => until);
  try {
    const series = new Series('binance-perp:BTCUSDT|1m', parseMarket('binance-perp:BTCUSDT')!, '1m', 'exchange', []);
    await series.ensure(20);
    const held = series.cols.time[series.cols.time.length - 1]!;
    assertEquals(held < now - 5 * MINUTE, true, 'the first load ends five minutes back');
    // Now, asked for again by the next chart to show it.
    until = now;
    await series.ensure(20);
    const newest = series.cols.time[series.cols.time.length - 1]!;
    // The last closed minute: the one before the minute forming now.
    assertEquals(newest, Math.floor(now / MINUTE) * MINUTE - MINUTE, 'the bars closed since are in');
  } finally {
    restore();
  }
});

Deno.test('bars brought again unchanged are known as the same, and one changed figure is not', () => {
  const cols = (close: number[]) => ({ time: Float64Array.from(close.map((_, i) => i * 60_000)), open: Float64Array.from(close), high: Float64Array.from(close), low: Float64Array.from(close), close: Float64Array.from(close), volume: Float64Array.from(close.map(() => 1)), buy: Float64Array.from(close.map(() => -1)) });
  assertEquals(sameCols(cols([1, 2, 3]), cols([1, 2, 3])), true);
  assertEquals(sameCols(cols([1, 2, 3]), cols([1, 2, 4])), false, 'the last close');
  assertEquals(sameCols(cols([1, 2, 3]), cols([1, 2])), false, 'a bar more');
  assertEquals(sameCols(cols([9, 2, 3]), cols([1, 2, 3])), false, 'an older bar put right');
  assertEquals(sameCols(emptyCols(), concatCols([emptyCols()])), true);
});
