/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { cents, coinsFor, figures, fits, fullEntry, lossPerCoin, marginFor, parseLegs, runTo, serializeLegs } from './legs.ts';
import { book, closedPnl, niceTicks, openPnl, pnl, type Trade } from './model.ts';

Deno.test('legs read back as they were written, the unreadable dropped', () => {
  const legs = parseLegs('t,110,50|c,95,25|a,97,100|x,1,1|t,112|t,115,0');
  assertEquals(legs, [
    { kind: 'take', price: 110, percent: 50 },
    { kind: 'cut', price: 95, percent: 25 },
    { kind: 'add', price: 97, percent: 100 },
  ]);
  assertEquals(serializeLegs(legs), 't,110,50|c,95,25|a,97,100');
  assertEquals(parseLegs(undefined), []);
});

Deno.test('a leg fits where the prices allow it', () => {
  const at = (kind: 'take' | 'cut' | 'add', price: number) => fits({ kind, price, percent: 50 }, 100, 90, 120);
  assertEquals([at('take', 110), at('take', 95), at('cut', 95), at('cut', 105), at('add', 95), at('add', 110), at('add', 100)], [true, false, true, false, true, true, false]);
});

Deno.test('the target and the stop come to what the legs on the way make of them', () => {
  const legs = parseLegs('t,110,50|a,95,100');
  assertEquals(runTo(100, 120, 2, [], 1), 40);
  assertEquals(runTo(100, 120, 2, legs, 1), 30, 'half taken at 110, half run to 120');
  assertEquals(runTo(100, 90, 2, legs, 1), -30, 'two more coins at 95, all four stopped at 90');
  assertEquals(runTo(100, 90, 1, parseLegs('c,95,50'), -1), 7.5, 'a short gains as it falls');
  assertEquals(fullEntry(100, 2, legs), 97.5);
});

const trade = (over: Partial<Trade>): Trade => ({
  id: 1,
  drawingId: null,
  tracked: true,
  leverage: 1,
  market: 'binance-perp:BTCUSDT',
  side: 'long',
  timeframe: '1h',
  startAt: 0,
  entry: 100,
  stop: 90,
  target: 120,
  risk: 10,
  qty: 1,
  status: 'planned',
  openedAt: null,
  closedAt: null,
  exit: null,
  fees: 0,
  auto: true,
  strategy: '',
  notes: '',
  createdAt: 0,
  updatedAt: 0,
  legs: [],
  fills: [],
  ...over,
});

Deno.test('a trade with fills is weighed by them, against their average entry', () => {
  const fills: Trade['fills'] = [
    { kind: 'entry', price: 100, qty: 1, at: 0 },
    { kind: 'add', price: 96, qty: 1, at: 1 },
    { kind: 'take', price: 110, qty: 1, at: 2, leg: 0 },
  ];
  assertEquals(book(trade({ fills })), { bought: 2, sold: 1, held: 1, avgEntry: 98, realized: 12 });
  assertEquals(openPnl(trade({ status: 'open', fills }), 100), 14, 'twelve taken, two more on the coin held');
  const closed = trade({ status: 'win', exit: 115, fees: 1, fills: [...fills, { kind: 'target', price: 120, qty: 1, at: 3 }] });
  assertEquals(pnl(closed), 33, '12 + 22, a dollar of fees off');
  assertEquals(pnl(trade({ status: 'win', exit: 120 })), 20, 'one that kept no fills, by its exit');
});

Deno.test('a margin holds the same size wherever the stop goes, and the stop sets what that risks', () => {
  const coins = coinsFor(100, 80_000, 10);
  assertEquals(coins, 0.0125);
  assertEquals(marginFor(coins, 80_000, 10), 100);
  assertEquals(figures(80_000, 79_200, 81_600, coins, [], 1), { loss: 10, gain: 20, rr: 2, value: 1000 });
  assertEquals(figures(80_000, 78_400, 81_600, coins, [], 1), { loss: 20, gain: 20, rr: 1, value: 1000 });
  assertEquals(figures(80_000, 80_800, 78_400, coins, [], -1).loss, 10, 'a short, its stop above');
});

Deno.test('with legs drawn the loss is at most what the stop costs with them filled', () => {
  const adds = [{ kind: 'add' as const, price: 95, percent: 100 }];
  assertEquals(figures(100, 90, 120, 1, adds, 1).loss, 15, 'one more coin from 95 to 90');
  assertEquals(lossPerCoin(100, 90, adds, 1), 15);
  assertEquals(figures(100, 90, 120, 1, [{ kind: 'cut', price: 95, percent: 50 }], 1).loss, 7.5);
});

Deno.test('money in a field keeps its cents, or a few figures below a dollar', () => {
  assertEquals([cents(99.999999), cents(12.345), cents(0.012345)], [100, 12.35, 0.01235]);
});

Deno.test('a close by hand reads before it is saved', () => {
  const plain = { side: 'long' as const, entry: 100, qty: 2, fills: [] };
  assertEquals(closedPnl(plain, { entry: 100, qty: 2, exit: 110, fees: 1 }), 19);
  assertEquals(closedPnl(plain, { entry: 100, qty: 4, exit: 90, fees: 0 }), -40, 'at the new size');
  assertEquals(closedPnl(plain, { entry: 100, qty: 2, exit: null, fees: 0 }), 0, 'break-even is out at the entry');
  assertEquals(closedPnl({ ...plain, side: 'short' }, { entry: 100, qty: 2, exit: 90, fees: 0 }), 20);
  const taken = { ...plain, fills: [{ kind: 'entry' as const, price: 100, qty: 2, at: 0 }, { kind: 'take' as const, price: 110, qty: 1, at: 1 }, { kind: 'stop' as const, price: 90, qty: 1, at: 2 }] };
  assertEquals(closedPnl(taken, { entry: 100, qty: 2, exit: 105, fees: 0 }), 15, 'what price took stays, the rest out where the hand says');
  assertEquals(closedPnl(taken, { entry: 100, qty: 4, exit: 105, fees: 0 }), 30, 'all of it at twice the size');
  assertEquals(closedPnl(taken, { entry: 98, qty: 2, exit: 105, fees: 0 }), 14, 'the entry moved: in there and out at once');
});

Deno.test('an axis has round figures that cover its range', () => {
  assertEquals(niceTicks(0, 1234), [0, 500, 1000, 1500]);
  assertEquals(niceTicks(-120, 80), [-150, -100, -50, 0, 50, 100]);
  assertEquals(niceTicks(0, 0)[0], 0, 'nothing moved: zero, whatever the step');
});
