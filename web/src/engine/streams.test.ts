/// <reference lib="deno.ns" />
/**
 * The exchange payloads, as they actually arrive, read through the real hub.
 * The socket is faked so the test needs neither network nor a busy market: a
 * quiet hour on Binance is no reason for a parser to go unchecked.
 *
 * Run with `deno task test`; tsc skips this file (see tsconfig exclude).
 */
import { assert, assertEquals } from 'jsr:@std/assert@1';
import { FakeTime } from 'jsr:@std/testing@1/time';
import { isBlocked, isRelayed, nextUrl, StreamHub, type StreamMsg, type StreamSpec } from './streams.ts';
import { reached, wake } from './reach.ts';
import type { Venue } from '../lib/markets.ts';

class FakeSocket {
  static opened: FakeSocket[] = [];
  readyState = 1;
  readonly sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(readonly url: string) {
    FakeSocket.opened.push(this);
    queueMicrotask(() => this.onopen?.());
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.onclose?.();
  }
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Subscribes through a real hub on a fake socket, and reports what it saw. */
async function listen(venue: Venue, spec: StreamSpec) {
  FakeSocket.opened = [];
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
  const seen: StreamMsg[] = [];
  const stop = new StreamHub().subscribe(venue, spec, { message: (m) => seen.push(m), state: () => {} });
  await tick();
  const socket = FakeSocket.opened.at(-1);
  assert(socket, 'the hub opened no socket');
  return {
    url: socket.url,
    frames: socket.sent.join(' '),
    seen,
    deliver: (payload: unknown) => socket.onmessage?.({ data: JSON.stringify(payload) }),
    stop,
  };
}

/** The hub keeps heartbeat and linger timers of its own; they outlive the test. */
const test = (name: string, fn: () => Promise<void>) => Deno.test({ name, fn, sanitizeOps: false, sanitizeResources: false });

// Captured from fstream.binance.com, 2026-09-16.
const FORCE_ORDER = {
  e: 'forceOrder',
  E: 1789531696996,
  o: { s: 'ONUSDT', S: 'BUY', o: 'LIMIT', f: 'IOC', q: '706', p: '0.1781000', ap: '0.1753635', X: 'FILLED', l: '193', z: '706', T: 1789531695983, ps: 'ONUSDT' },
};

test('a Binance forced BUY is a short being liquidated', async () => {
  const s = await listen('binance-perp', { kind: 'liquidations', symbol: 'ONUSDT' });
  assert(s.url.startsWith('wss://fstream.binance.com/'), s.url);
  assert(s.frames.includes('onusdt@forceOrder'), s.frames);

  // 0.1753635 is where it filled; 0.1781 is only the limit it was pushed through.
  s.deliver(FORCE_ORDER);
  assertEquals(s.seen, [{ kind: 'liquidation', time: 1789531695983, price: 0.1753635, qty: 706, long: false }]);

  // An order that has not filled yet has no average price to report.
  s.deliver({ ...FORCE_ORDER, o: { ...FORCE_ORDER.o, ap: '0', X: 'NEW' } });
  assertEquals((s.seen[1] as { price: number }).price, 0.1781, 'the limit price stands in until it fills');

  s.deliver({ ...FORCE_ORDER, o: { ...FORCE_ORDER.o, S: 'SELL' } });
  assertEquals((s.seen[2] as { long: boolean }).long, true, 'a forced SELL closes a long');
  s.stop();
});

test('a Bybit allLiquidation batch reports the position side', async () => {
  const s = await listen('bybit-perp', { kind: 'liquidations', symbol: 'BTCUSDT' });
  assert(s.frames.includes('allLiquidation.BTCUSDT'), s.frames);

  s.deliver({
    topic: 'allLiquidation.BTCUSDT',
    type: 'snapshot',
    ts: 1789531696000,
    data: [
      { T: 1789531695000, s: 'BTCUSDT', S: 'Buy', v: '0.4', p: '75900.5' },
      { T: 1789531695500, s: 'BTCUSDT', S: 'Sell', v: '0.1', p: '75880' },
    ],
  });
  assertEquals(s.seen, [
    { kind: 'liquidation', time: 1789531695000, price: 75900.5, qty: 0.4, long: true },
    { kind: 'liquidation', time: 1789531695500, price: 75880, qty: 0.1, long: false },
  ]);
  s.stop();
});

test('a liquidation topic is not confused with the candle stream', async () => {
  const s = await listen('binance-perp', { kind: 'kline', symbol: 'BTCUSDT', interval: '15m' });
  s.deliver(FORCE_ORDER);
  assertEquals(s.seen, [], 'nobody subscribed to ONUSDT liquidations');

  s.deliver({ e: 'kline', s: 'BTCUSDT', k: { t: 1789531680000, i: '15m', o: '75800', h: '75950', l: '75790', c: '75900', v: '12.5', V: '7.5', L: 42, x: false } });
  assertEquals(s.seen, [{ kind: 'kline', openTime: 1789531680000, open: 75800, high: 75950, low: 75790, close: 75900, volume: 12.5, buy: 7.5, lastTradeId: 42, closed: false }]);
  s.stop();
});

test('one unknown Bybit symbol does not take the rest of the frame with it', async () => {
  FakeSocket.opened = [];
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
  const seen: StreamMsg[] = [];
  const hub = new StreamHub();
  const listener = { message: (m: StreamMsg) => seen.push(m), state: () => {} };
  hub.subscribe('bybit-perp', { kind: 'liquidations', symbol: 'NOPEUSDT' }, listener);
  hub.subscribe('bybit-perp', { kind: 'liquidations', symbol: 'BTCUSDT' }, listener);
  await tick();
  const socket = FakeSocket.opened.at(-1);
  assert(socket, 'the hub opened no socket');
  assertEquals(socket.sent.length, 1, 'both topics travel in one frame');

  // Bybit's own words, verbatim: the frame is refused, one topic is named.
  socket.onmessage?.({
    data: JSON.stringify({
      success: false,
      ret_msg: 'error:handler not found,topic:allLiquidation.NOPEUSDT',
      conn_id: 'da7to24d40ndveq8rdsg-66uqh',
      req_id: 'allLiquidation.NOPEUSDT,allLiquidation.BTCUSDT',
      op: 'subscribe',
    }),
  });
  assertEquals(JSON.parse(socket.sent[1] ?? '{}').args, ['allLiquidation.BTCUSDT'], 'the good topic is re-sent alone');

  socket.onmessage?.({ data: JSON.stringify({ topic: 'allLiquidation.BTCUSDT', data: [{ T: 1789531695000, s: 'BTCUSDT', S: 'Buy', v: '0.4', p: '75900.5' }] }) });
  assertEquals(seen.length, 1, 'BTCUSDT still streams');

  // The refused topic is never sent again, alone or in a batch.
  socket.onmessage?.({ data: JSON.stringify({ success: false, ret_msg: 'error:handler not found,topic:allLiquidation.BTCUSDT', req_id: 'allLiquidation.BTCUSDT', op: 'subscribe' }) });
  assertEquals(socket.sent.length, 2, 'a frame of one is not retried');
});

test('a socket of liquidations only is live on the exchange\'s answer, not on a hit', async () => {
  const states: string[] = [];
  FakeSocket.opened = [];
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
  const stop = new StreamHub().subscribe('binance-perp', { kind: 'liquidations', symbol: 'BTCUSDT' }, { message: () => {}, state: (st) => states.push(st) });
  await tick();
  const socket = FakeSocket.opened.at(-1);
  assert(socket, 'the hub opened no socket');

  // BTCUSDT may go minutes without a liquidation; the subscribe answer is enough.
  socket.onmessage?.({ data: JSON.stringify({ result: null, id: 1 }) });
  assertEquals(states.at(-1), 'live', 'answered, so connected');
  stop();
});

test('a quiet market\'s candles are live on the exchange\'s answer', async () => {
  const states: string[] = [];
  FakeSocket.opened = [];
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
  // A stock token outside its hours pushes no candle at all.
  const stop = new StreamHub().subscribe('binance-perp', { kind: 'kline', symbol: 'BTCUSDT', interval: '4h', quiet: true }, { message: () => {}, state: (st) => states.push(st) });
  await tick();
  const socket = FakeSocket.opened.at(-1);
  assert(socket, 'the hub opened no socket');
  socket.onmessage?.({ data: JSON.stringify({ result: null, id: 1 }) });
  assertEquals(states.at(-1), 'live', 'answered, so connected, though nothing trades');
  stop();
});

test('a candle socket still needs data before it counts as live', async () => {
  const states: string[] = [];
  FakeSocket.opened = [];
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
  const stop = new StreamHub().subscribe('binance-perp', { kind: 'kline', symbol: 'BTCUSDT', interval: '1m' }, { message: () => {}, state: (st) => states.push(st) });
  await tick();
  const socket = FakeSocket.opened.at(-1);
  assert(socket, 'the hub opened no socket');

  // Some networks answer the handshake and the subscribe, then push nothing.
  socket.onmessage?.({ data: JSON.stringify({ result: null, id: 1 }) });
  assert(!states.includes('live'), 'an answer alone does not make candles live');
  socket.onmessage?.({ data: JSON.stringify({ e: 'kline', s: 'BTCUSDT', k: { t: 1, i: '1m', o: '1', h: '1', l: '1', c: '1', v: '1', L: 1, x: false } }) });
  assertEquals(states.at(-1), 'live', 'the first candle does');
  stop();
});

Deno.test('a blocked address is stepped over, so the relay is reached at once', () => {
  const urls = ['wss://exchange.example/ws', 'wss://exchange2.example/ws', 'wss://tradoor.example/api/relay/ws?venue=binance-perp'];
  const dead = (url: string) => url.includes('exchange');
  assertEquals(nextUrl(urls, 0, () => false), urls[0], 'nothing known dead: the exchange first, as always');
  assertEquals(nextUrl(urls, 0, dead), urls[2], 'both exchange addresses dead on this network: straight to the relay');
  assertEquals(nextUrl(urls, 1, dead), urls[2], 'and it stays there while they are');
  assertEquals(nextUrl(urls, 1, () => true), urls[1], 'everything dead: work through them all again rather than stop');
  assert(isRelayed(urls[2]!) && !isRelayed(urls[0]!), 'and the page can say which way it came');
});

test('a socket that has gone quiet is thrown away, whatever it claims', async () => {
  const time = new FakeTime();
  try {
    FakeSocket.opened = [];
    (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
    const states: string[] = [];
    const stop = new StreamHub().subscribe(
      'binance-perp',
      { kind: 'kline', symbol: 'BTCUSDT', interval: '1m' },
      { message: () => {}, state: (state) => states.push(state) },
    );
    await time.tickAsync(1);
    const first = FakeSocket.opened.at(-1)!;
    first.onmessage?.({ data: JSON.stringify({ e: 'kline', s: 'BTCUSDT', k: { t: 0, o: '1', h: '1', l: '1', c: '1', v: '1', L: 1, x: false } }) });
    assertEquals(states.at(-1), 'live', 'it carried a candle');

    // Half a minute of silence: on iOS that is a socket handed back dead after a lock.
    await time.tickAsync(36_000);
    assertEquals(first.readyState, 3, 'the quiet socket was closed');
    assertEquals(states.at(-1), 'retrying', 'and the chart says so rather than showing a frozen price as live');
    await time.tickAsync(2_000);
    assert(FakeSocket.opened.length > 1, 'and another was opened in its place');
    stop();
  } finally {
    time.restore();
  }
});

/** A socket the network swallows: never open, never refused. */
class HangingSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static opened: HangingSocket[] = [];
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(readonly url: string) {
    HangingSocket.opened.push(this);
  }

  send(): void {}

  close(): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.onclose?.();
  }
}

test('an address that never opens is given up in seconds, and its host with it', async () => {
  const time = new FakeTime();
  const host = 'stream.bybit.com';
  try {
    HangingSocket.opened = [];
    (globalThis as unknown as { WebSocket: unknown }).WebSocket = HangingSocket;
    const stop = new StreamHub().subscribe('bybit-perp', { kind: 'kline', symbol: 'BTCUSDT', interval: '1m' }, { message: () => {}, state: () => {} });
    await time.tickAsync(1);
    const first = HangingSocket.opened[0]!;

    await time.tickAsync(4_000);
    assertEquals(first.readyState, 0, 'a slow line is given its chance');
    await time.tickAsync(1_500);
    assertEquals(first.readyState, 3, 'but not the minute a phone would wait');
    assert(isBlocked(`wss://${host}/v5/public/spot`), 'and every socket on that host goes past it, not only this one');
    stop();
  } finally {
    // What this test taught the list is not for the next one, or the next run.
    reached(host);
    reached(`wss://${host}/v5/public/linear`);
    time.restore();
  }
});

/** The fake socket, with the state constants the hub reads off a real one. */
class WakeSocket extends FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
}

test('back from a lock, a silent socket is replaced in seconds and a live one kept', async () => {
  const time = new FakeTime();
  try {
    FakeSocket.opened = [];
    (globalThis as unknown as { WebSocket: unknown }).WebSocket = WakeSocket;
    const stop = new StreamHub().subscribe('binance-perp', { kind: 'kline', symbol: 'ETHUSDT', interval: '1m' }, { message: () => {}, state: () => {} });
    await time.tickAsync(1);
    const socket = FakeSocket.opened.at(-1)!;
    const candle = { data: JSON.stringify({ e: 'kline', s: 'ETHUSDT', k: { t: 0, i: '1m', o: '1', h: '1', l: '1', c: '1', v: '1', L: 1, x: false } }) };
    socket.onmessage?.(candle);

    wake();
    await time.tickAsync(2_000);
    socket.onmessage?.(candle);
    await time.tickAsync(5_000);
    assertEquals(socket.readyState, 1, 'it spoke after the wake: kept');

    wake();
    await time.tickAsync(6_500);
    assertEquals(socket.readyState, 3, 'silent after the wake: replaced, not left to the half-minute watch');
    stop();
  } finally {
    time.restore();
  }
});
