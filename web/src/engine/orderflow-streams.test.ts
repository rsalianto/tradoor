/// <reference lib="deno.ns" />
/**
 * Each exchange's trades and book, as the frames actually arrive (taken from
 * the live sockets), read into prints and book changes.
 */
import { assertEquals } from 'jsr:@std/assert@1';
import { readFrame, socketFor, subscribeFrames } from './orderflow-streams.ts';

Deno.test('binance trades carry the taker side, and futures book changes name the one before', () => {
  assertEquals(readFrame('binance-perp', { e: 'aggTrade', s: 'BTCUSDT', p: '80420.90', q: '0.022', T: 1793195384000, m: true }), [
    ['btcusdt@aggTrade', { kind: 'print', time: 1793195384000, price: 80420.9, qty: 0.022, buy: false }],
  ]);
  const [[topic, msg]] = readFrame('binance-perp-book', { e: 'depthUpdate', s: 'BTCUSDT', U: 100, u: 120, pu: 99, b: [['80426.50', '4.9']], a: [['80427.00', '0.000']] });
  assertEquals(topic, 'btcusdt@depth@100ms');
  assertEquals(msg, { kind: 'book', reset: false, bids: [[80426.5, 4.9]], asks: [[80427, 0]], first: 100, last: 120, prev: 99 });
  // Spot names no previous change: it follows on from U - 1.
  const [[, spot]] = readFrame('binance-spot', { e: 'depthUpdate', s: 'BTCUSDT', U: 101996398221, u: 101996398251, b: [], a: [] });
  assertEquals(spot, { kind: 'book', reset: false, bids: [], asks: [], first: 101996398221, last: 101996398251, prev: 101996398220 });
});

Deno.test('the futures book is on its own socket', () => {
  assertEquals(socketFor('binance-perp', 'book'), 'binance-perp-book');
  assertEquals(socketFor('binance-perp', 'prints'), 'binance-perp');
  assertEquals(socketFor('okx-perp', 'book'), 'okx-public');
  assertEquals(subscribeFrames('binance-perp', 'book', 'BTCUSDT').frames, [{ method: 'SUBSCRIBE', params: ['btcusdt@depth@100ms'], id: 1 }]);
});

Deno.test('bybit: snapshot, then deltas; trades by S', () => {
  const [[topic, snap]] = readFrame('bybit-perp', { topic: 'orderbook.1000.BTCUSDT', type: 'snapshot', data: { s: 'BTCUSDT', b: [['80456.10', '4.902']], a: [['80456.20', '1']], u: 5 } });
  assertEquals(topic, 'orderbook.1000.BTCUSDT');
  assertEquals(snap, { kind: 'book', reset: true, bids: [[80456.1, 4.902]], asks: [[80456.2, 1]] });
  const [[, restart]] = readFrame('bybit-perp', { topic: 'orderbook.1000.BTCUSDT', type: 'delta', data: { s: 'BTCUSDT', b: [], a: [], u: 1 } });
  assertEquals((restart as { reset: boolean }).reset, true, 'u = 1 is the service restarting: a snapshot');
  assertEquals(readFrame('bybit-spot', { topic: 'publicTrade.BTCUSDT', data: [{ s: 'BTCUSDT', p: '84251.5', v: '0.2', S: 'Buy', T: 9 }] }), [
    ['publicTrade.BTCUSDT', { kind: 'print', time: 9, price: 84251.5, qty: 0.2, buy: true }],
  ]);
});

Deno.test('okx: books numbered by seqId, trades by side', () => {
  const frame = { arg: { channel: 'books', instId: 'BTC-USDT-SWAP' }, action: 'update', data: [{ asks: [['80441.8', '802.89', '0', '2']], bids: [], seqId: 349118283410, prevSeqId: 349118283293 }] };
  assertEquals(readFrame('okx-public', frame), [['books|BTC-USDT-SWAP', { kind: 'book', reset: false, bids: [], asks: [[80441.8, 802.89]], last: 349118283410, prev: 349118283293 }]]);
  const [[, trade]] = readFrame('okx-public', { arg: { channel: 'trades', instId: 'BTC-USDT-SWAP' }, data: [{ px: '80475.5', sz: '100', side: 'buy', ts: '1793195470090' }] });
  assertEquals(trade, { kind: 'print', time: 1793195470090, price: 80475.5, qty: 100, buy: true });
  assertEquals(subscribeFrames('okx-perp', 'book', 'BTC-USDT-SWAP').frames, [{ op: 'subscribe', args: [{ channel: 'books', instId: 'BTC-USDT-SWAP' }] }]);
});

Deno.test('bitget: spot and futures on one socket; trades come newest first and leave oldest first', () => {
  const book = readFrame('bitget', { action: 'snapshot', arg: { instType: 'USDT-FUTURES', channel: 'books', instId: 'BTCUSDT' }, data: [{ asks: [['80520.4', '2.8507']], bids: [['80511.8', '9.2751']] }] });
  assertEquals(book, [['USDT-FUTURES|books|BTCUSDT', { kind: 'book', reset: true, bids: [[80511.8, 9.2751]], asks: [[80520.4, 2.8507]] }]]);
  const trades = readFrame('bitget', { action: 'snapshot', arg: { instType: 'SPOT', channel: 'trade', instId: 'BTCUSDT' }, data: [{ ts: '2', price: '1', size: '1', side: 'buy' }, { ts: '1', price: '1', size: '1', side: 'sell' }] });
  assertEquals(trades.map(([, m]) => (m as { time: number }).time), [1, 2]);
});

Deno.test('hyperliquid: the whole book every time; B is a buyer', () => {
  const [[topic, msg]] = readFrame('hyperliquid', { channel: 'l2Book', data: { coin: 'BTC', levels: [[{ px: '80519.0', sz: '29.79607', n: 104 }], [{ px: '80520.0', sz: '1.5', n: 3 }]] } });
  assertEquals(topic, 'l2Book|BTC');
  assertEquals(msg, { kind: 'book', reset: true, bids: [[80519, 29.79607]], asks: [[80520, 1.5]] });
  const [[, trade]] = readFrame('hyperliquid', { channel: 'trades', data: [{ coin: 'BTC', side: 'A', px: '84119.0', sz: '0.1', time: 1790000000001 }] });
  assertEquals(trade, { kind: 'print', time: 1790000000001, price: 84119, qty: 0.1, buy: false });
});

Deno.test('kraken spot and futures', () => {
  const [[, spot]] = readFrame('kraken-spot', { channel: 'book', type: 'update', data: [{ symbol: 'BTC/USD', bids: [], asks: [{ price: 80435.0, qty: 0 }] }] });
  assertEquals(spot, { kind: 'book', reset: false, bids: [], asks: [[80435, 0]] });
  const [[, trade]] = readFrame('kraken-spot', { channel: 'trade', data: [{ symbol: 'BTC/USD', side: 'sell', price: 84090.1, qty: 0.02, timestamp: '2026-09-25T08:10:49.100Z' }] });
  assertEquals(trade, { kind: 'print', time: Date.parse('2026-09-25T08:10:49.100Z'), price: 84090.1, qty: 0.02, buy: false });
  const [[topic, level]] = readFrame('kraken-perp', { feed: 'book', product_id: 'PF_XBTUSD', side: 'sell', seq: 1, price: 80416.0, qty: 0.2052 });
  assertEquals(topic, 'book|PF_XBTUSD');
  assertEquals(level, { kind: 'book', reset: false, bids: [], asks: [[80416, 0.2052]] });
  assertEquals(subscribeFrames('kraken-spot', 'book', 'BTC/USD').frames, [{ method: 'subscribe', params: { channel: 'book', symbol: ['BTC/USD'], depth: 1000 } }]);
});

Deno.test('lighter: a maker on the ask means the taker bought', () => {
  const [[topic, trade]] = readFrame('lighter', { channel: 'trade:1', type: 'update/trade', trades: [{ price: '80461.0', size: '0.00003', is_maker_ask: true, timestamp: 1793195568303 }] });
  assertEquals(topic, 'trade/1');
  assertEquals(trade, { kind: 'print', time: 1793195568303, price: 80461, qty: 0.00003, buy: true });
  const [[, snap]] = readFrame('lighter', { channel: 'order_book:1', type: 'subscribed/order_book', order_book: { asks: [{ price: '80453.7', size: '0.01065' }], bids: [] } });
  assertEquals((snap as { reset: boolean }).reset, true);
  const [[, update]] = readFrame('lighter', { channel: 'order_book:1', type: 'update/order_book', order_book: { asks: [{ price: '80460.5', size: '0.00000' }], bids: [] } });
  assertEquals(update, { kind: 'book', reset: false, bids: [], asks: [[80460.5, 0]] });
});
