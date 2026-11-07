/// <reference lib="deno.ns" />
/**
 * The newer exchanges' frames and candles, as they arrived on 2026-11-05, read
 * through their protocols and loaders. Sockets and answers are stand-ins: a
 * parser should not wait on a busy market to be checked.
 */
import { assert, assertEquals } from 'jsr:@std/assert@1';
import { PROTOCOLS, rows } from './venue-pack.ts';
import { restInterval } from './intervals.ts';
import type { StreamMsg } from './streams.ts';
import { parseMarket, type MarketRef } from '../lib/markets.ts';

function read(socket: string, frames: unknown[]): [string, StreamMsg][] {
  const proto = PROTOCOLS[socket]!();
  const out: [string, StreamMsg][] = [];
  for (const f of frames) proto.read(f as Record<string, unknown>, (topic, msg) => out.push([topic, msg]));
  return out;
}

const ref = (market: string) => parseMarket(market) as MarketRef;

Deno.test('Gate: a spot bar counts coins, a perpetual its turnover over the close', () => {
  const spot = read('gate-spot', [
    { channel: 'spot.candlesticks', event: 'subscribe', result: { status: 'success' } },
    { channel: 'spot.candlesticks', event: 'update', result: { t: '1793876040', v: '24726.58', c: '80284.4', h: '80284.5', l: '80284.4', o: '80284.5', n: '1m_BTC_USDT', a: '0.307987', w: false } },
  ]);
  assertEquals(spot[0], ['spot.candlesticks|1m|BTC_USDT', { kind: 'kline', openTime: 1793876040000, open: 80284.5, high: 80284.5, low: 80284.4, close: 80284.4, volume: 0.307987, lastTradeId: 0, closed: false }]);
  const perp = read('gate-perp', [
    { channel: 'futures.candlesticks', event: 'update', result: [{ t: 1793876040, c: '80000', h: '80293.9', l: '80293.8', o: '80293.9', a: '160000', n: '1m_BTC_USDT', w: true, v: 25802 }] },
  ]);
  assertEquals(perp[0]![0], 'futures.candlesticks|1m|BTC_USDT');
  assertEquals(perp[0]![1], { kind: 'kline', openTime: 1793876040000, open: 80293.9, high: 80293.9, low: 80293.8, close: 80000, volume: 2, lastTradeId: 0, closed: true });
});

Deno.test('Gate asks by the name each side gives a week, and pings with the time', () => {
  const spot = PROTOCOLS['gate-spot']!();
  assertEquals(spot.topic({ kind: 'kline', symbol: 'BTC_USDT', interval: '1w' }, 'gate-spot'), 'spot.candlesticks|7d|BTC_USDT');
  const perp = PROTOCOLS['gate-perp']!();
  assertEquals(perp.topic({ kind: 'kline', symbol: 'BTC_USDT', interval: '1w' }, 'gate-perp'), 'futures.candlesticks|1w|BTC_USDT');
  const [frame] = perp.frames('subscribe', ['futures.candlesticks|1m|BTC_USDT']) as { channel: string; event: string; payload: string[]; time: number }[];
  assertEquals([frame!.channel, frame!.event, frame!.payload], ['futures.candlesticks', 'subscribe', ['1m', 'BTC_USDT']]);
  assert(frame!.time > 1_000_000_000);
  assertEquals((perp.heartbeat as { channel: string }).channel, 'futures.ping');
  assert((perp.heartbeat as { time: number }).time > 1_000_000_000);
});

Deno.test('Aster reads as Binance does, and a subscribe answer shows the socket is up', () => {
  const proto = PROTOCOLS['aster-spot']!();
  assert(proto.quiet, 'a quiet market pushes nothing for minutes');
  const out = read('aster-perp', [
    { id: 1, result: null },
    { e: 'kline', s: 'BTCUSDT', k: { t: 1793876160000, i: '1m', o: '80260.9', c: '80280.0', h: '80287.2', l: '80255.6', v: '1.989', V: '1.2', x: false } },
  ]);
  assertEquals(out[0]![0], 'btcusdt@kline_1m');
  assertEquals((out[0]![1] as { buy?: number }).buy, 1.2);
});

Deno.test('HTX answers its ping, counts coins, and unpacks its frames in order', async () => {
  const proto = PROTOCOLS['htx-perp']!();
  assertEquals(proto.reply!({ ping: 1793876042000 }), { pong: 1793876042000 });
  assertEquals(proto.reply!({ status: 'ok' }), null);
  assertEquals(proto.frames('subscribe', ['market.BTC-USDT.kline.1min']), [{ sub: 'market.BTC-USDT.kline.1min', id: 'market.BTC-USDT.kline.1min' }]);
  assertEquals(proto.frames('unsubscribe', ['market.BTC-USDT.kline.1min']), [{ unsub: 'market.BTC-USDT.kline.1min', id: 'market.BTC-USDT.kline.1min' }]);
  const perp = read('htx-perp', [
    { id: '1', status: 'ok', subbed: 'market.BTC-USDT.kline.1min' },
    { ch: 'market.BTC-USDT.kline.1min', tick: { id: 1793876040, open: 80283.5, close: 80283.5, high: 80283.5, low: 80283.5, amount: 0.5, vol: 500, trade_turnover: 40000 } },
  ]);
  assertEquals((perp[0]![1] as { volume: number }).volume, 0.5, 'coins, not contracts');
  // A frame as it comes, gzipped, unpacked in order.
  const zip = (text: string) => new Response(new Blob([text]).stream().pipeThrough(new CompressionStream('gzip'))).blob();
  const [a, b] = [await zip('{"n":1}'), await zip('{"n":2}')];
  assertEquals(await Promise.all([proto.inflate!(a), proto.inflate!(b)]), ['{"n":1}', '{"n":2}']);
  assertEquals(await proto.inflate!(new Blob([new Uint8Array(4)])), '', 'a bad frame is skipped, and the next still comes');
  assertEquals(await proto.inflate!(a), '{"n":1}');
});

Deno.test('Bitfinex goes by the number it gives each channel', () => {
  const proto = PROTOCOLS['bitfinex-perp']!();
  const candles = proto.topic({ kind: 'kline', symbol: 'tBTCF0:USTF0', interval: '1d' }, 'bitfinex-perp');
  assertEquals(candles, 'trade:1D:tBTCF0:USTF0');
  assertEquals(proto.frames('subscribe', [candles]), [{ event: 'subscribe', channel: 'candles', key: 'trade:1D:tBTCF0:USTF0' }]);
  assertEquals(proto.frames('unsubscribe', [candles]), [], 'not numbered yet: nothing to end');
  const out: [string, StreamMsg][] = [];
  const feed = (f: unknown) => proto.read(f as Record<string, unknown>, (t, m) => out.push([t, m]));
  feed({ event: 'info', version: 2 });
  feed({ event: 'subscribed', channel: 'candles', chanId: 7, key: 'trade:1D:tBTCF0:USTF0' });
  feed([7, [[1793875980000, 80382, 80372, 80388, 80365, 0.6], [1793875920000, 80382, 80394, 80403, 80370, 0.37]]]);
  feed([7, 'hb']);
  feed([7, [1793875980000, 80382, 80380, 80390, 80365, 0.7]]);
  feed([99, [1, 2, 3, 4, 5, 6]]);
  assertEquals(out.length, 2, 'the newest of a snapshot and an update; no heartbeat, no stranger');
  assertEquals(out[0]![1], { kind: 'kline', openTime: 1793875980000, open: 80382, high: 80388, low: 80365, close: 80372, volume: 0.6, lastTradeId: 0, closed: false });
  assertEquals((out[1]![1] as { close: number }).close, 80380);
  assertEquals(proto.frames('unsubscribe', [candles]), [{ event: 'unsubscribe', chanId: 7 }]);
});

Deno.test('Deribit reads its channels by name, the volume in coins', () => {
  const out = read('deribit-perp', [
    { jsonrpc: '2.0', id: 1, result: ['chart.trades.BTC-PERPETUAL.1'] },
    { jsonrpc: '2.0', method: 'subscription', params: { channel: 'chart.trades.BTC-PERPETUAL.1', data: { close: 80253, high: 80258.5, low: 80229.5, open: 80235, tick: 1793876160000, cost: 1754520, volume: 21.87 } } },
  ]);
  assertEquals(out[0], ['chart.trades.BTC-PERPETUAL.1', { kind: 'kline', openTime: 1793876160000, open: 80235, high: 80258.5, low: 80229.5, close: 80253, volume: 21.87, lastTradeId: 0, closed: false }]);
  const proto = PROTOCOLS['deribit-perp']!();
  assertEquals(proto.topic({ kind: 'kline', symbol: 'BTC-PERPETUAL', interval: '2h' }, 'deribit-perp'), 'chart.trades.BTC-PERPETUAL.120');
});

Deno.test('Coinbase folds matches into every chart on the product, and the last chart out ends them', () => {
  const proto = PROTOCOLS['coinbase-spot']!();
  const [one, five] = [proto.topic({ kind: 'kline', symbol: 'BTC-USD', interval: '1m' }, 'coinbase-spot'), proto.topic({ kind: 'kline', symbol: 'BTC-USD', interval: '5m' }, 'coinbase-spot')];
  assertEquals([one, five], ['candles|BTC-USD|60', 'candles|BTC-USD|300']);
  assertEquals(proto.frames('subscribe', [one, five]).length, 2);
  const out: [string, StreamMsg][] = [];
  proto.read({ type: 'match', trade_id: 1115497489, product_id: 'BTC-USD', size: '0.5', price: '80216.75', time: '2026-11-05T10:54:04.000000Z' }, (t, m) => out.push([t, m]));
  assertEquals(out.map(([t]) => t), [one, five]);
  assertEquals(out[0]![1], { kind: 'trade', time: Date.parse('2026-11-05T10:54:04Z'), price: 80216.75, qty: 0.5, lastTradeId: 1115497489 });
  assertEquals(proto.frames('unsubscribe', [one]), [], 'the other chart still reads them');
  assertEquals(proto.frames('unsubscribe', [five]), [{ type: 'unsubscribe', product_ids: ['BTC-USD'], channels: ['matches'] }]);
});

Deno.test('Bitstamp: trades fold into every chart on the pair, and a quiet market still gets its bars', async () => {
  const real = globalThis.fetch;
  const asked: string[] = [];
  globalThis.fetch = ((url: string) => {
    asked.push(String(url));
    return Promise.resolve(new Response(JSON.stringify({ data: { market: 'BTC/USD-PERP', ohlc: [{ timestamp: '1793890380', open: '78582', high: '78695', low: '78582', close: '78662', volume: '0.17036' }, { timestamp: '1793890440', open: '78658', high: '78667', low: '78548', close: '78548', volume: '0.30091' }] } })));
  }) as typeof fetch;
  try {
    const proto = PROTOCOLS['bitstamp-perp']!();
    const [one, day] = [proto.topic({ kind: 'kline', symbol: 'btcusd-perp', interval: '1m' }, 'bitstamp-perp'), proto.topic({ kind: 'kline', symbol: 'btcusd-perp', interval: '1d' }, 'bitstamp-perp')];
    assertEquals([one, day], ['ohlc|btcusd-perp|60', 'ohlc|btcusd-perp|86400']);
    assertEquals(proto.topic({ kind: 'prints', symbol: 'btcusd-perp' }, 'bitstamp-perp'), '');
    assertEquals(proto.open, [{ event: 'bts:subscribe', data: { channel: 'live_trades_btcusd' } }], 'what keeps time, whichever charts are open');
    assertEquals(proto.frames('subscribe', [one])[0], { event: 'bts:subscribe', data: { channel: 'live_trades_btcusd-perp' } });
    proto.frames('subscribe', [day]);
    const out: [string, StreamMsg][] = [];
    const emit = (t: string, m: StreamMsg) => out.push([t, m]);
    proto.read({ event: 'bts:subscription_succeeded', channel: 'live_trades_btcusd-perp', data: {} }, emit);
    await new Promise((r) => setTimeout(r, 300));
    assertEquals(asked, ['https://www.bitstamp.net/api/v2/ohlc/btcusd-perp/?step=60&limit=2', 'https://www.bitstamp.net/api/v2/ohlc/btcusd-perp/?step=86400&limit=2']);
    assertEquals(out.length, 4, 'the last two bars of each chart');
    assertEquals(out[1], [one, { kind: 'kline', openTime: 1793890440000, open: 78658, high: 78667, low: 78548, close: 78548, volume: 0.30091, lastTradeId: 0, closed: false }]);
    out.length = 0;
    // A perpetual's id runs past what a number holds, and its `timestamp` is in milliseconds.
    proto.read({ event: 'trade', channel: 'live_trades_btcusd-perp', data: { id_str: '2020537793373028352', type: 0, amount: 0.01624, price: 78423, microtimestamp: '1793890549716000', timestamp: '1793890549716' } }, emit);
    assertEquals(out.map(([t]) => t), [one, day]);
    assertEquals(out[0]![1], { kind: 'trade', time: 1793890549716, price: 78423, qty: 0.01624, lastTradeId: 1 });
    out.length = 0;
    // Spot's `timestamp` is in seconds, and its trades here only keep the time.
    proto.read({ event: 'trade', channel: 'live_trades_btcusd', data: { id: 667879171, type: 0, amount: 0.06, price: 78414, microtimestamp: '1793890549668000', timestamp: '1793890549' } }, emit);
    assertEquals(out, []);
    assertEquals(proto.frames('unsubscribe', [one]), [], 'the other chart still reads them');
    assertEquals(proto.frames('unsubscribe', [day]), [{ event: 'bts:unsubscribe', data: { channel: 'live_trades_btcusd-perp' } }]);
    const spot = PROTOCOLS['bitstamp-spot']!();
    const clock = spot.topic({ kind: 'kline', symbol: 'btcusd', interval: '1m' }, 'bitstamp-spot');
    spot.frames('subscribe', [clock]);
    assertEquals(spot.frames('unsubscribe', [clock]), [], 'the market keeping time stays on');
  } finally {
    globalThis.fetch = real;
  }
});

Deno.test('HTX and Deribit: the UTC day is folded from hours, as their own days start elsewhere', async () => {
  const hour = 3_600_000;
  const today = Math.floor(Date.now() / 86_400_000) * 86_400_000;
  const real = globalThis.fetch;
  const asked: string[] = [];
  const answer = (body: unknown) => Promise.resolve(new Response(JSON.stringify(body)));
  // Two hours of today and one of yesterday; the stream then brings the hour under way.
  globalThis.fetch = ((url: string) => {
    asked.push(String(url));
    return String(url).includes('deribit')
      ? answer({ result: { ticks: [today, today + hour], open: [100, 110], high: [120, 115], low: [90, 105], close: [110, 112], volume: [1, 2] } })
      : answer({ data: [{ id: (today - hour) / 1000, open: 1, high: 999, low: 0.1, close: 1, amount: 50 }, { id: today / 1000, open: 100, high: 120, low: 90, close: 110, amount: 1 }, { id: (today + hour) / 1000, open: 110, high: 115, low: 105, close: 112, amount: 2 }] });
  }) as typeof fetch;
  try {
    const settle = () => new Promise((r) => setTimeout(r, 10));
    for (const [socket, symbol, venue, hourFrame, tick, frames] of [
      ['htx-perp', 'BTC-USDT', 'htx-perp', (k: Record<string, number>) => ({ ch: 'market.BTC-USDT.kline.60min', tick: { id: k.t! / 1000, open: k.o, high: k.h, low: k.l, close: k.c, amount: k.v } }), { status: 'ok' }, [{ sub: 'market.BTC-USDT.kline.60min', id: 'market.BTC-USDT.kline.60min' }]],
      ['deribit-perp', 'BTC-PERPETUAL', 'deribit-perp', (k: Record<string, number>) => ({ method: 'subscription', params: { channel: 'chart.trades.BTC-PERPETUAL.60', data: { tick: k.t, open: k.o, high: k.h, low: k.l, close: k.c, volume: k.v } } }), { id: 1, result: [] }, [{ jsonrpc: '2.0', id: 1, method: 'public/subscribe', params: { channels: ['chart.trades.BTC-PERPETUAL.60'] } }]],
    ] as const) {
      const proto = PROTOCOLS[socket]!();
      const topic = proto.topic({ kind: 'kline', symbol, interval: '1d' }, venue);
      assertEquals(topic, `day|${symbol}`);
      assertEquals(proto.frames('subscribe', [topic]), [...frames] as unknown[], 'hours are what the socket is asked for');
      const days: StreamMsg[] = [];
      const emit = (t: string, m: StreamMsg) => t === topic && days.push(m);
      proto.read(tick as Record<string, unknown>, emit);
      await settle();
      assertEquals(days[0], { kind: 'kline', openTime: today, open: 100, high: 120, low: 90, close: 112, volume: 3, lastTradeId: 0, closed: false }, 'today from the hours read, yesterday left out');
      proto.read(hourFrame({ t: today + 2 * hour, o: 112, h: 130, l: 111, c: 125, v: 4 }) as Record<string, unknown>, emit);
      assertEquals(days[1], { kind: 'kline', openTime: today, open: 100, high: 130, low: 90, close: 125, volume: 7, lastTradeId: 0, closed: false }, 'and the hour under way on top');
      proto.read(hourFrame({ t: today + 2 * hour, o: 112, h: 131, l: 111, c: 126, v: 5 }) as Record<string, unknown>, emit);
      assertEquals((days[2] as { volume: number }).volume, 8, 'the same hour again replaces itself');
      const reads = asked.length;
      proto.read(tick as Record<string, unknown>, emit);
      await settle();
      assertEquals(asked.length, reads, 'read once');
      proto.frames('unsubscribe', [topic]);
      const after = days.length;
      proto.read(hourFrame({ t: today + 2 * hour, o: 112, h: 131, l: 111, c: 126, v: 5 }) as Record<string, unknown>, emit);
      assertEquals(days.length, after, 'and nothing once nobody follows it');
    }
    assert(asked.some((u) => u.startsWith('/api/relay/rest?url=') && u.includes('deribit')), 'Deribit\'s bars answer no browser');
    assert(asked.some((u) => u.startsWith('https://api.hbdm.com/')), 'HTX\'s do');
  } finally {
    globalThis.fetch = real;
  }
});

/** A stand-in for the page's fetch: says which address it was asked, and answers. */
const answering = (body: unknown) => {
  const asked: string[] = [];
  return { asked, getJson: (url: string) => (asked.push(url), Promise.resolve(body)) };
};
const MINUTE = 60_000;

Deno.test('candles: each exchange\'s order of columns comes out the same', async () => {
  const end = 1793876160000;
  const range = { end };
  // Gate's reach is counted back from now, so its bar is a recent one.
  const recent = Math.floor(Date.now() / MINUTE) * MINUTE - 5 * MINUTE;
  const gateSpot = answering([[String(recent / 1000), '393863.33', '80305.7', '80310', '80244.6', '80250', '4.906', 'true']]);
  assertEquals(await rows(ref('gate-spot:BTCUSDT'), 'BTC_USDT', '1m', { start: 0, end: Date.now() }, 999, gateSpot.getJson, MINUTE), [[recent, 80250, 80310, 80244.6, 80305.7, 4.906]]);
  assert(gateSpot.asked[0]!.includes('/spot/candlesticks?currency_pair=BTC_USDT&interval=1m&from='));
  const gatePerp = answering([{ o: '80250', h: '80400', l: '80200', c: '80000', sum: '160000', t: recent / 1000, v: 565419 }]);
  assertEquals(await rows(ref('gate-perp:BTCUSDT'), 'BTC_USDT', '1m', {}, 1999, gatePerp.getJson, MINUTE), [[recent, 80250, 80400, 80200, 80000, 2]]);
  const aster = answering([[1793875800000, '80304.8', '80321.3', '80213.8', '80231.4', '22.06', 1793875859999, '1770617.84', 97, '6.543', '524']]);
  assertEquals(await rows(ref('aster-perp:BTCUSDT'), 'BTCUSDT', '1m', range, 999, aster.getJson, MINUTE), [[1793875800000, 80304.8, 80321.3, 80213.8, 80231.4, 22.06, 6.543]]);
  assert(aster.asked[0]!.startsWith('https://fapi.asterdex.com/fapi/v1/klines?symbol=BTCUSDT&interval=1m&limit=999&endTime='));
  const htxPerp = answering({ data: [{ id: 1793875800, open: 1, high: 3, low: 0.5, close: 2, amount: 4.082, vol: 4082 }] });
  assertEquals(await rows(ref('htx-perp:BTCUSDT'), 'BTC-USDT', '4h', range, 1999, htxPerp.getJson, 4 * 60 * MINUTE), [[1793875800000, 1, 3, 0.5, 2, 4.082]]);
  assert(htxPerp.asked[0]!.includes('contract_code=BTC-USDT&period=4hour&from='));
  const bitfinex = answering([[1793875980000, 80331, 80330, 80338, 80326, 0.13], [1793875920000, 80343, 80326, 80373, 80326, 0.42]]);
  assertEquals(await rows(ref('bitfinex-perp:BTCUSDT'), 'tBTCF0:USTF0', '1d', range, 100, bitfinex.getJson, 1440 * MINUTE), [[1793875920000, 80343, 80373, 80326, 80326, 0.42], [1793875980000, 80331, 80338, 80326, 80330, 0.13]]);
  assert(bitfinex.asked[0]!.includes('/candles/trade:1D:tBTCF0:USTF0/hist?limit=100&end='));
  const deribit = answering({ result: { status: 'ok', ticks: [1793875800000], open: [1], high: [3], low: [0.5], close: [2], volume: [19.4] } });
  assertEquals(await rows(ref('deribit-perp:BTCUSD'), 'BTC-PERPETUAL', '2h', range, 5000, deribit.getJson, 120 * MINUTE), [[1793875800000, 1, 3, 0.5, 2, 19.4]]);
  assert(deribit.asked[0]!.includes('instrument_name=BTC-PERPETUAL&start_timestamp=') && deribit.asked[0]!.endsWith('resolution=120'));
  const coinbase = answering([[1793875980, 80226.03, 80241.46, 80229.08, 80231.54, 2.14], [1793875920, 80226.72, 80270, 80238.9, 80229.04, 1.83]]);
  const got = await rows(ref('coinbase-spot:BTCUSD'), 'BTC-USD', '1m', range, 299, coinbase.getJson, MINUTE);
  assertEquals(got[0], [1793875920000, 80238.9, 80270, 80226.72, 80229.04, 1.83], 'oldest first');
  assert(coinbase.asked[0]!.includes('granularity=60&start='));
  const bitstamp = answering({ data: { pair: 'BTC/USD', ohlc: [{ timestamp: '1793876100', open: '1', high: '3', low: '0.5', close: '2', volume: '4.5' }, { timestamp: '1793876160', open: '2', high: '2', low: '2', close: '2', volume: '0.00000000' }] } });
  assertEquals(await rows(ref('bitstamp-spot:BTCUSD'), 'btcusd', '1m', { start: 1793876160000, end }, 999, bitstamp.getJson, MINUTE), [[1793876160000, 2, 2, 2, 2, 0]], 'a bar before the start is left out, an empty minute kept');
  assertEquals(bitstamp.asked[0], 'https://www.bitstamp.net/api/v2/ohlc/btcusd/?step=60&limit=999&end=1793876160');
  let refused = '';
  await rows(ref('bitstamp-spot:BTCUSD'), 'nope', '1m', range, 999, answering({ code: 'validation-error' }).getJson, MINUTE).catch((e: Error) => (refused = e.message));
  assertEquals(refused, 'Bitstamp refused the candle request');
  assertEquals(await rows(ref('bitstamp-perp:BTCUSD'), 'btcusd-perp', '1d', { end: -1 }, 999, bitstamp.getJson, 1440 * MINUTE), [], 'nothing before 1970');
});

Deno.test('candles: HTX spot serves only its latest, and Gate refuses to look far back', async () => {
  const latest = [{ id: 300, open: 3, high: 3, low: 3, close: 3, amount: 3 }, { id: 240, open: 2, high: 2, low: 2, close: 2, amount: 2 }, { id: 180, open: 1, high: 1, low: 1, close: 1, amount: 1 }];
  const htx = answering({ data: latest });
  assertEquals((await rows(ref('htx-spot:BTCUSDT'), 'btcusdt', '1m', { end: 400_000 }, 2, htx.getJson, MINUTE)).map((r) => r[0]), [240_000, 300_000], 'the newest of those asked for');
  assertEquals((await rows(ref('htx-spot:BTCUSDT'), 'btcusdt', '1m', { end: 200_000 }, 2, htx.getJson, MINUTE)).map((r) => r[0]), [180_000], 'older comes from what was served');
  assertEquals(await rows(ref('htx-spot:BTCUSDT'), 'btcusdt', '1m', { end: 1_000 }, 2, htx.getJson, MINUTE), [], 'and past it, nothing: the history ends there');
  const gate = answering([]);
  assertEquals(await rows(ref('gate-perp:BTCUSDT'), 'BTC_USDT', '1m', { end: Date.now() - 20_000 * MINUTE }, 999, gate.getJson, MINUTE), []);
  assertEquals(gate.asked, [], 'ten thousand bars back it answers 400 anyway, so it is not asked');
});

Deno.test('candles: Gate\'s old weekly bars lose the noise a chart would read its decimals from', async () => {
  const old = 1562544000; // July 2019
  const recent = 1793577600;
  const week = answering([[String(old), '1', '11384.1234567890', '12000.12345678', '11000.5', '11200.1234', '9'], [String(recent), '1', '80345.8', '81190.7', '75731.4', '78342.5', '9']]);
  const got = await rows(ref('gate-spot:BTCUSDT'), 'BTC_USDT', '1w', { start: 0, end: Date.now() }, 999, week.getJson, 7 * 1440 * MINUTE);
  assertEquals(got[0], [old * 1000, 11200, 12000, 11001, 11384, 9]);
  assertEquals(got[1], [recent * 1000, 78342.5, 81190.7, 75731.4, 80345.8, 9], 'a recent bar keeps its tick');
  const day = answering([[String(old), '1', '11384.123', '12000', '11000', '11200', '9']]);
  assertEquals((await rows(ref('gate-spot:BTCUSDT'), 'BTC_USDT', '1d', { start: 0, end: old * 1000 + 1 }, 999, day.getJson, 1440 * MINUTE))[0]![4], 11384.123, 'only weeks reach back that far');
});

Deno.test('candles: a thin market is not taken for the start of its history', async () => {
  // A trade every so often: each page has a few bars, far fewer than asked for.
  const end = 1793876160000;
  const asked: string[] = [];
  const getJson = (url: string) => {
    asked.push(url);
    const start = Date.parse(decodeURIComponent(/start=([^&]+)/.exec(url)![1]!));
    return Promise.resolve([[Math.floor(start / 60_000) * 60 + 600, 1, 3, 2, 2, 5], [Math.floor(start / 60_000) * 60 + 1200, 1, 3, 2, 2, 5]].reverse());
  };
  const got = await rows(ref('coinbase-spot:XLMEUR'), 'XLM-EUR', '1m', { end }, 5, getJson, MINUTE);
  assertEquals(got.length, 5, 'further back until there are enough');
  assertEquals(asked.length, 3);
  assert(got.every((r, i) => i === 0 || r[0]! > got[i - 1]![0]!), 'oldest first');
  const quiet = answering([]);
  assertEquals(await rows(ref('coinbase-spot:XLMEUR'), 'XLM-EUR', '1m', { end }, 5, quiet.getJson, MINUTE), []);
  assertEquals(quiet.asked.length, 6, 'but not for ever: past that, it is where the history ends');
  const weeks = answering([]);
  await rows(ref('gate-perp:BTCUSDT'), 'BTC_USDT', '1w', {}, 999, weeks.getJson, 7 * 1440 * MINUTE);
  assert(weeks.asked.length > 0 && weeks.asked.every((u) => !u.includes('from=-') && !u.includes('to=-')), 'and never before 1970');
  const pages = answering([]);
  assertEquals(await rows(ref('gate-spot:BTCUSDT'), 'BTC_USDT', '1w', { start: -2e12, end: -1e12 }, 999, pages.getJson, 7 * 1440 * MINUTE), []);
  assertEquals(pages.asked, [], 'a page wholly before it is not asked for');
});

Deno.test('candles: HTX is waited out when it says too many, and believed otherwise', async () => {
  let calls = 0;
  const busy = (_: string) => Promise.resolve(++calls === 1 ? { status: 'error', 'err-code': 'invalid-parameter', 'err-msg': 'request limit' } : { status: 'ok', data: [{ id: 300, open: 1, high: 1, low: 1, close: 1, amount: 1 }] });
  assertEquals(await rows(ref('htx-spot:BTCUSDT'), 'btcusdt', '1m', { start: 0, end: 400_000 }, 2, busy, MINUTE), [[300_000, 1, 1, 1, 1, 1]]);
  assertEquals(calls, 2);
  const refused = (_: string) => Promise.resolve({ status: 'error', 'err-msg': 'invalid symbol' });
  let message = '';
  await rows(ref('htx-spot:BTCUSDT'), 'nope', '1m', { end: 400_000 }, 2, refused, MINUTE).catch((e: Error) => (message = e.message));
  assertEquals(message, 'invalid symbol');
});

Deno.test('lengths an exchange lacks, or starts off the chart\'s boundaries, are folded from shorter ones', () => {
  // HTX's days start at 16:00 UTC, Deribit's at 08:00: neither is taken.
  assertEquals(['1h', '2h', '6h', '12h', '1d', '1w', '1M'].map((i) => restInterval('htx-perp', i)), ['1h', '1h', '1h', '4h', '4h', '4h', '4h']);
  assertEquals(['1h', '2h', '4h', '6h', '12h', '1d', '1w', '1M'].map((i) => restInterval('deribit-perp', i)), ['1h', '2h', '2h', '2h', '2h', '2h', '2h', '2h']);
  assertEquals(['15m', '30m', '2h', '4h', '6h', '12h', '1d', '1w', '1M'].map((i) => restInterval('coinbase-spot', i)), ['15m', '15m', '1h', '1h', '6h', '6h', '1d', '1d', '1d']);
  assertEquals(['2h', '1d', '1w', '1M'].map((i) => restInterval('bitfinex-spot', i)), ['1h', '1d', '1w', '1M']);
  assertEquals(['30m', '2h', '6h', '12h', '1d', '1w', '1M'].map((i) => restInterval('bitstamp-perp', i)), ['30m', '2h', '6h', '12h', '1d', '1d', '1d']);
  assertEquals(['1d', '1w', '1M'].map((i) => restInterval('gate-spot', i)), ['1d', '1w', '1d']);
  assertEquals(['2h', '1M'].map((i) => restInterval('aster-perp', i)), ['2h', '1M'], 'Binance\'s lengths, all of them');
  // Nothing moves for the exchanges already here.
  assertEquals(restInterval('kraken-spot', '12h'), '4h');
  assertEquals(restInterval('hyperliquid-perp', '1w'), '1d');
  assertEquals(restInterval('binance-perp', '1M'), '1M');
});
