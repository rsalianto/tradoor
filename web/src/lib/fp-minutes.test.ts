/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { foldMinutes } from './footprint.ts';
import { cover, coveredMs, gaps, levelFor, MinuteBook, MINUTE, pack, readAggTrades, toMinute, trimSpans, unpack } from './fp-minutes.ts';

Deno.test('a market is recorded a hundred-thousandth of its price apart, no finer than its decimals', () => {
  assertEquals(levelFor(80_000, 1), 0.5, 'BTC: 0.8 rounds down to half a dollar');
  assertEquals(levelFor(2_300, 2), 0.02);
  assertEquals(levelFor(100, 2), 0.01, 'its decimals are the floor');
  assertEquals(levelFor(0.000012, 8), 1e-8);
});

Deno.test('trades land in their minute and level, bought and sold apart, and a resent snapshot is left out', () => {
  const book = new MinuteBook(1, 10 * MINUTE);
  assertEquals(book.add(10 * MINUTE - 5, 100, 1, true), false, 'before the recording began');
  book.add(10 * MINUTE + 1_000, 100.4, 2, true);
  book.add(10 * MINUTE + 2_000, 100.6, 1, false);
  book.add(11 * MINUTE + 1, 100.4, 3, false);
  assertEquals(book.add(10 * MINUTE + 3_000, 100, 1, true), false, 'a minute older than the newest: resent');
  const done = book.take(11 * MINUTE);
  assertEquals(done.map(([at, l]) => [at, [...l]]), [[10 * MINUTE, [[100, [2, 0]], [101, [0, 1]]]]]);
  assertEquals([...book.minutes.keys()], [11 * MINUTE], 'the minute still going stays');
});

Deno.test('levels pack into twelve bytes each and back', () => {
  const levels = new Map<number, [number, number]>([[80_000, [1.5, 0.25]], [80_001, [0, 3]]]);
  const buffer = pack(levels);
  assertEquals(buffer.byteLength, 24);
  assertEquals([...unpack(buffer)], [...levels]);
  assertEquals(toMinute(60_000, levels, 0.5).levels, [[40_000, 1.5, 0.25], [40_000.5, 0, 3]]);
});

Deno.test('recorded spans join, trim and leave the gaps between them', () => {
  let spans = cover([], 0, 10);
  spans = cover(spans, 20, 30);
  spans = cover(spans, 10, 12);
  assertEquals(spans, [[0, 12], [20, 30]], 'touching spans join');
  assertEquals(gaps(spans, 5, 40), [[12, 20], [30, 40]]);
  assertEquals(coveredMs(spans, 5, 40), 17);
  assertEquals(trimSpans(spans, 25), [[25, 30]]);
});

Deno.test("Binance's aggregated trades read with the taker's side", () => {
  const trades = readAggTrades([{ a: 7, p: '100.5', q: '0.2', T: 123, m: true }, { a: 8, p: '100.6', q: '0.1', T: 124, m: false }, { bogus: 1 }]);
  assertEquals(trades, [
    { id: 7, time: 123, price: 100.5, qty: 0.2, buy: false },
    { id: 8, time: 124, price: 100.6, qty: 0.1, buy: true },
  ]);
});

Deno.test('recorded minutes fold into bars as the server’s do', () => {
  const minutes = [
    { at: 0, levels: [[100, 1, 0], [101, 0, 2]] as [number, number, number][] },
    { at: MINUTE, levels: [[100, 3, 1]] as [number, number, number][] },
  ];
  const fp = foldMinutes(minutes, 1, 5 * MINUTE);
  assertEquals(fp.bars, [{ time: 0, levels: [[100, 4, 1], [101, 0, 2]], maxDelta: 1, minDelta: -1 }], 'a delta of -1, then +1 overall');
  assertEquals(foldMinutes(minutes, 1, 5 * MINUTE, undefined, 2).bars[0]!.levels, [[100, 4, 3]], 'a grouping given is kept');
});
