/// <reference lib="deno.ns" />
import { assert, assertEquals } from 'jsr:@std/assert@1';
import type { Trade } from './model.ts';
import { exitLegs, frameFor, marketOf, spanOf, summaryOf, tradePicture } from './picture.ts';

const HOUR = 3_600_000;
const NOW = Date.UTC(2026, 10, 7, 12);

/** A short on XPLUSDT, closed, unless told otherwise. */
const trade = (over: Partial<Trade> = {}): Trade => ({
  id: 1,
  drawingId: null,
  tracked: true,
  market: 'bybit-perp:XPLUSDT',
  side: 'short',
  timeframe: '15m',
  startAt: NOW - 50 * HOUR,
  entry: 0.08,
  stop: 0.082,
  target: 0.076,
  risk: 15,
  qty: 7500,
  leverage: 10,
  status: 'loss',
  openedAt: NOW - 48 * HOUR,
  closedAt: NOW - 46 * HOUR,
  exit: 0.082,
  fees: 0,
  auto: true,
  strategy: '',
  notes: '',
  createdAt: NOW - 50 * HOUR,
  updatedAt: NOW - 46 * HOUR,
  legs: [],
  fills: [],
  ...over,
});

Deno.test('a trade runs from its entry to its close, an hour where the close cannot be, to now while open', () => {
  assertEquals(spanOf(trade(), NOW), { from: NOW - 48 * HOUR, to: NOW - 46 * HOUR });
  assertEquals(spanOf(trade({ closedAt: NOW - 49 * HOUR }), NOW), { from: NOW - 48 * HOUR, to: NOW - 47 * HOUR }, 'closed before it opened');
  assertEquals(spanOf(trade({ closedAt: NOW - 48 * HOUR }), NOW), { from: NOW - 48 * HOUR, to: NOW - 47 * HOUR }, 'closed as it opened');
  assertEquals(spanOf(trade({ status: 'open', closedAt: null, exit: null }), NOW).to, NOW);
  assertEquals(spanOf(trade({ openedAt: null }), NOW).from, NOW - 50 * HOUR, 'one added by hand with no fill time: its start');
});

Deno.test('the trades entered are drawn oldest first, numbered, each a position marked as a picture of it', () => {
  const won = trade({ id: 7, status: 'win', exit: 0.076, openedAt: NOW - 30 * HOUR, closedAt: NOW - 28 * HOUR });
  const lost = trade({ id: 3 });
  const planned = trade({ id: 9, status: 'planned', openedAt: null, closedAt: null, exit: null });
  const cancelled = trade({ id: 10, status: 'cancelled', closedAt: null, exit: null });
  const p = tradePicture([won, planned, lost, cancelled], NOW)!;
  assertEquals(p.parts.length, 2, 'never filled, nothing to draw');
  const [first, second] = p.parts as [typeof p.parts[0], typeof p.parts[0]];
  assertEquals(first.kind, 'short');
  assertEquals(first.points, [
    { time: NOW - 48 * HOUR, price: 0.08 },
    { time: NOW - 48 * HOUR, price: 0.082 },
    { time: NOW - 46 * HOUR, price: 0.076 },
  ]);
  assertEquals(first.style.ofTrade, 3);
  assertEquals(second.style.ofTrade, 7);
  assertEquals(first.label, '#1 Short · Lost −1.00R −$15.00');
  assertEquals(second.label, '#2 Short · Won +2.00R +$30.00');
  assertEquals(first.style.margin, 60, 'sized as traded: 7,500 coins at 0.08, ten times over');
  assertEquals([p.from, p.to, p.won, p.lost, Number(p.r.toFixed(9)), Number(p.pnl.toFixed(9))], [NOW - 48 * HOUR, NOW - 28 * HOUR, 1, 1, 1, 15]);
  assertEquals(summaryOf(p), '2 trades · 1 won · 1 lost · +1.00R · +$15.00');
  assertEquals(tradePicture([planned, cancelled], NOW), null);
});

Deno.test('one still open says so, and counts no money', () => {
  const p = tradePicture([trade({ status: 'open', closedAt: null, exit: null })], NOW)!;
  assertEquals(p.parts[0]!.label, '#1 Short · Open');
  assertEquals(summaryOf(p), '1 trade · 0 won · 0 lost · 1 open');
});

Deno.test('an exit short of the stop or target is drawn across the box', () => {
  assertEquals(exitLegs(trade()), '', 'stopped: the edge of the box says it');
  assertEquals(exitLegs(trade({ status: 'closed', exit: 0.0789 })), 't,0.0789,100', 'closed by hand in profit, with nothing kept of how');
  assertEquals(exitLegs(trade({ status: 'closed', exit: 0.081 })), 'c,0.081,100', 'and at a loss');
  const fills = [
    { kind: 'entry' as const, price: 0.08, qty: 7500, at: NOW - 48 * HOUR },
    { kind: 'take' as const, price: 0.078, qty: 3750, at: NOW - 47 * HOUR },
    { kind: 'hand' as const, price: 0.0805, qty: 3750, at: NOW - 46 * HOUR },
  ];
  assertEquals(exitLegs(trade({ status: 'closed', exit: 0.0805, fills })), 't,0.078,50|c,0.0805,50');
});

Deno.test('the bar length shows them together and still reaches back to the first', () => {
  assertEquals(frameFor(NOW - 4 * HOUR, NOW - 2 * HOUR, NOW), '1m');
  assertEquals(frameFor(NOW - 48 * HOUR, NOW - 30 * HOUR, NOW), '5m');
  assertEquals(frameFor(NOW - 11 * 24 * HOUR, NOW - 7 * 24 * HOUR, NOW), '1h');
  assertEquals(frameFor(NOW - 120 * 24 * HOUR, NOW - 100 * 24 * HOUR, NOW), '4h');
  assertEquals(frameFor(NOW - 900 * 24 * HOUR, NOW - 800 * 24 * HOUR, NOW), '1d');
});

Deno.test('they are drawn on the market most of them were on', () => {
  assertEquals(marketOf([trade(), trade({ market: 'binance-perp:XPLUSDT' }), trade()]), 'bybit-perp:XPLUSDT');
  assert(marketOf([]) === null);
});
