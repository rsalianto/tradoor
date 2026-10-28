/// <reference lib="deno.ns" />
/** The order book kept from snapshots and changes, and trades read for the DOM and tape. */
import { assert, assertAlmostEquals, assertEquals } from 'jsr:@std/assert@1';
import type { BookMsg } from '../engine/streams.ts';
import { bestAsk, bestBid, Book, BookSync, Prints, recentGroups, Tally, tickOf } from './orderflow.ts';

const change = (bids: [number, number][], asks: [number, number][], seq: Partial<BookMsg> = {}): BookMsg => ({ kind: 'book', reset: false, bids, asks, ...seq });

Deno.test('binance: changes held for the snapshot, the stale ones dropped, the rest laid over it', () => {
  const book = new Book();
  const sync = new BookSync(book);
  sync.await();
  assertEquals(sync.message(change([[100, 1]], [], { first: 1, last: 5, prev: 0 }), 0), 'skip');
  assertEquals(sync.message(change([[100, 2]], [], { first: 6, last: 9, prev: 5 }), 0), 'skip');
  assertEquals(sync.message(change([[99, 3]], [], { first: 10, last: 12, prev: 9 }), 0), 'skip');
  // The snapshot has everything to 8: the change to 5 is old, the one 6–9 straddles it.
  assertEquals(sync.snapshot(8, [[100, 7], [98, 1]], [[101, 1]], 0), 'ok');
  assertEquals([...book.bids].sort(), [[100, 2], [98, 1], [99, 3]].sort());
  assertEquals(book.seq, 12);
  assertEquals(sync.message(change([], [[101, 0]], { first: 13, last: 13, prev: 12 }), 0), 'ok');
  assertEquals(book.asks.size, 0);
  // One missed: the book waits for a new snapshot.
  assertEquals(sync.message(change([[97, 1]], [], { first: 20, last: 21, prev: 19 }), 0), 'gap');
  assertEquals(book.synced, false);
});

Deno.test('binance: a snapshot older than every change held does not line up', () => {
  const sync = new BookSync(new Book());
  sync.await();
  sync.message(change([], [], { first: 50, last: 60, prev: 49 }), 0);
  assertEquals(sync.snapshot(10, [], [], 0), 'gap');
});

Deno.test('a snapshot after a gap makes the book whole again', () => {
  const book = new Book();
  const sync = new BookSync(book);
  sync.message({ kind: 'book', reset: true, bids: [[10, 1]], asks: [[11, 1]], last: 1 }, 0);
  assertEquals(sync.message(change([], [], { last: 3, prev: 2 }), 0), 'gap');
  assertEquals(sync.message(change([[9, 1]], []), 0), 'skip', 'nothing laid on a book with a hole in it');
  sync.message({ kind: 'book', reset: true, bids: [[10, 2]], asks: [[11, 1]], last: 7 }, 0);
  assertEquals(book.synced, true);
  assertEquals(book.bids.get(10), 2);
});

Deno.test('a level that shrinks with a trade there is traded, not pulled', () => {
  const book = new Book();
  book.reset({ bids: [[100, 5]], asks: [[101, 5]], last: undefined });
  book.traded(101, 2, true, 1_000);
  book.apply({ bids: [], asks: [[101, 3]], last: undefined }, 1_100);
  assertEquals(book.pulls.get(101), undefined, 'the trade took it');
  book.apply({ bids: [[100, 1]], asks: [], last: undefined }, 1_200);
  assertEquals(book.pulled(100, 1_200), -4, 'nobody traded there: pulled');
  book.apply({ bids: [[100, 3]], asks: [], last: undefined }, 1_300);
  assertAlmostEquals(book.pulled(100, 1_300), -2, 0.01, 'two stacked back on');
  // The book first, the trade a moment later: the shrinking is given back.
  book.apply({ bids: [[100, 1]], asks: [], last: undefined }, 1_400);
  book.traded(100, 2, false, 1_450);
  assertAlmostEquals(book.pulled(100, 1_450), -2, 0.05, 'what fades in a quarter second is a hair');
  assertAlmostEquals(book.pulled(100, 31_450), book.pulled(100, 1_450) / 2, 1e-9, 'half gone half a minute on');
});

Deno.test('a whole book sent each time reads as the changes between', () => {
  const book = new Book();
  const sync = new BookSync(book, true);
  sync.message({ kind: 'book', reset: true, bids: [[100, 5], [99, 5]], asks: [[101, 5], [102, 5]] }, 0);
  sync.message({ kind: 'book', reset: true, bids: [[100, 5], [98, 1]], asks: [[101, 8], [102, 5]] }, 10);
  assertEquals(book.pulled(99, 10), -5, 'gone from inside the new book: pulled');
  assertEquals(book.pulled(101, 10), 3);
  assertEquals(book.bids.has(99), false);
});

Deno.test('crossed levels are stale and go', () => {
  const book = new Book();
  book.reset({ bids: [[100, 1]], asks: [[101, 1], [102, 1]], last: undefined });
  book.apply({ bids: [[101.5, 2]], asks: [], last: undefined }, 0);
  assertEquals([bestBid(book.bids), bestAsk(book.asks)], [101.5, 102]);
});

Deno.test('only the best levels are kept where the exchange tells of no more', () => {
  const book = new Book(2);
  book.reset({ bids: [[100, 1], [99, 1]], asks: [[101, 1], [102, 1]], last: undefined });
  book.apply({ bids: [[100.5, 1], [99.5, 1], [98, 1]], asks: [], last: undefined }, 0);
  assertEquals([...book.bids.keys()].sort((a, b) => b - a), [100.5, 100]);
});

Deno.test('the tick is the least gap among the best levels', () => {
  const side = (...p: number[]) => new Map(p.map((x) => [x, 1]));
  assertEquals(tickOf(side(80426.5, 80426.4, 80426.1), side(80426.6, 80427)), 0.1);
  assertEquals(tickOf(side(80519, 80518, 80516), side(80520)), 1);
  assertEquals(tickOf(new Map(), new Map()), 0);
});

Deno.test('trades sharing a time and side are one order; the filter judges the order', () => {
  const prints = new Prints(16);
  prints.push(1, 100, 0.5, true);
  prints.push(2, 100, 1, false);
  prints.push(2, 99.9, 1, false);
  prints.push(3, 100, 0.1, true);
  const groups = recentGroups(prints, 150, 10);
  assertEquals(groups.length, 1);
  assertEquals(groups[0], { time: 2, buy: false, qty: 2, usd: 199.9, from: 100, to: 99.9, trades: 2 });
  assertEquals(recentGroups(prints, 0, 10).map((g) => g.time), [3, 2, 1]);
});

Deno.test('the ring keeps the newest', () => {
  const prints = new Prints(3);
  for (let t = 1; t <= 5; t++) prints.push(t, 10, 1, true);
  assertEquals(prints.first, 2);
  assertEquals(recentGroups(prints, 0, 10).map((g) => g.time), [5, 4, 3]);
});

Deno.test('a tally counts by price, filtered by order, and forgets what is cleared', () => {
  const prints = new Prints(64);
  const book = new Book();
  const tally = new Tally(150, 0);
  prints.push(1, 100, 1, true);
  tally.read(prints);
  assertEquals(tally.bought.size, 0, 'a hundred dollars: under the filter');
  prints.push(2, 100, 1, false);
  tally.read(prints);
  assertEquals(tally.sold.size, 0, 'not yet');
  prints.push(2, 99, 1, false);
  tally.read(prints);
  assertEquals([...tally.sold], [[100, 1], [99, 1]], 'the order passed: both its trades count');
  prints.push(2, 98, 0.1, false);
  tally.read(prints);
  assertEquals(tally.sold.get(98), 0.1, 'and the rest of it as it comes');
  assertEquals([tally.volume, tally.delta, tally.high, tally.low], [2.1, -2.1, 100, 98]);
  tally.clear(book, 'above', 99, 5);
  assertEquals([...tally.sold.keys()].sort(), [98, 99]);
  tally.restart(prints);
  assert(!tally.sold.has(100), 'cleared levels stay cleared when read again');
  tally.clear(book, 'all', 0, 10);
  assertEquals([tally.sold.size, tally.volume], [0, 0]);
});

Deno.test('pulls show from the last clear', () => {
  const book = new Book();
  book.reset({ bids: [[100, 5]], asks: [], last: undefined });
  book.apply({ bids: [[100, 2]], asks: [], last: undefined }, 0);
  const tally = new Tally(0, 0);
  assertEquals(tally.pull(book, 100, 0), -3);
  tally.clear(book, 'all', 0, 0);
  assertEquals(tally.pull(book, 100, 0), 0);
  book.apply({ bids: [[100, 6]], asks: [], last: undefined }, 0);
  assertEquals(tally.pull(book, 100, 0), 4);
  assertAlmostEquals(tally.pull(book, 100, 30_000), 2, 1e-9, 'what came since fades as the rest does');
});

Deno.test('the ring starts small and grows only as trades come, keeping their order', () => {
  const prints = new Prints(8, 2);
  for (let t = 1; t <= 5; t++) prints.push(t, 10, 1, true);
  assertEquals(prints.size, 8);
  assertEquals(recentGroups(prints, 0, 10).map((g) => g.time), [5, 4, 3, 2, 1]);
  for (let t = 6; t <= 11; t++) prints.push(t, 10, 1, true);
  assertEquals([prints.size, prints.first], [8, 3], 'at its most it wraps, as before');
  assertEquals(recentGroups(prints, 0, 3).map((g) => g.time), [11, 10, 9]);
});

Deno.test('pulls far from the price are not kept', () => {
  const book = new Book();
  book.reset({ bids: [[100, 1], [50, 1]], asks: [[101, 1]], last: undefined });
  book.apply({ bids: [[50, 9], [99, 2]], asks: [], last: undefined }, 0);
  assertEquals([book.pulls.has(50), book.pulled(99, 0)], [false, 2]);
});
