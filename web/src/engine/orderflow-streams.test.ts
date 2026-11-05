/// <reference lib="deno.ns" />
/**
 * Each exchange's trades and book, as the frames actually arrive (taken from
 * the live sockets), read into prints and book changes.
 */
import { assertEquals } from 'jsr:@std/assert@1';
import { bookFromRest, depthUrl, loadMoreSockets, readFrame, readFrames, socketFor, subscribeFrames, wholeBook } from './orderflow-streams.ts';

await loadMoreSockets();

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

// The newer exchanges' frames as they arrived on 2026-11-05, trimmed.
Deno.test('Gate says a sale two ways, and its books come whole', () => {
  assertEquals(readFrame('gate-spot', { channel: 'spot.trades', event: 'update', result: { create_time_ms: '1793894074848.662000', side: 'sell', currency_pair: 'BTC_USDT', amount: '0.380575', price: '75588.6' } }), [
    ['spot.trades|BTC_USDT', { kind: 'print', time: 1793894074848, price: 75588.6, qty: 0.380575, buy: false }],
  ]);
  assertEquals(readFrame('gate-perp', { channel: 'futures.trades', event: 'update', result: [{ size: 6881, create_time_ms: 1793893971399, price: '75651.6', contract: 'BTC_USDT' }, { size: -20, create_time_ms: 1793893971400, price: '75651.5', contract: 'BTC_USDT' }] }), [
    ['futures.trades|BTC_USDT', { kind: 'print', time: 1793893971399, price: 75651.6, qty: 6881, buy: true }],
    ['futures.trades|BTC_USDT', { kind: 'print', time: 1793893971400, price: 75651.5, qty: 20, buy: false }],
  ]);
  assertEquals(readFrame('gate-spot', { channel: 'spot.order_book', event: 'update', result: { s: 'BTC_USDT', bids: [['75588.6', '0.08261']], asks: [['75590', '1']] } }), [
    ['spot.order_book|BTC_USDT', { kind: 'book', reset: true, bids: [[75588.6, 0.08261]], asks: [[75590, 1]] }],
  ]);
  assertEquals(readFrame('gate-perp', { channel: 'futures.order_book', event: 'all', result: { contract: 'BTC_USDT', asks: [{ p: '75570.1', s: 4097 }], bids: [{ p: '75569', s: 12 }] } }), [
    ['futures.order_book|BTC_USDT', { kind: 'book', reset: true, bids: [[75569, 12]], asks: [[75570.1, 4097]] }],
  ]);
  assertEquals(readFrame('gate-perp', { channel: 'futures.trades', event: 'subscribe', result: { status: 'success' } }), []);
  const { frames } = subscribeFrames('gate-perp', 'book', 'BTC_USDT');
  assertEquals((frames[0] as { payload: string[] }).payload, ['BTC_USDT', '100', '0']);
  assertEquals((subscribeFrames('gate-spot', 'book', 'BTC_USDT').frames[0] as { payload: string[] }).payload, ['BTC_USDT', '100', '100ms']);
  assertEquals((subscribeFrames('gate-spot', 'prints', 'BTC_USDT').frames[0] as { channel: string; payload: string[] }).payload, ['BTC_USDT']);
});

Deno.test('Aster is read as Binance is, its book laid over a REST snapshot', () => {
  assertEquals(readFrame('aster-perp', { e: 'aggTrade', s: 'BTCUSDT', p: '75568.1', q: '0.331', T: 1793894074800, m: false }), [['btcusdt@aggTrade', { kind: 'print', time: 1793894074800, price: 75568.1, qty: 0.331, buy: true }]]);
  const [[, change]] = readFrame('aster-spot', { e: 'depthUpdate', s: 'BTCUSDT', U: 7183181025, u: 7183181082, pu: 7183180968, b: [['75488.62', '0.0135']], a: [] });
  assertEquals(change, { kind: 'book', reset: false, bids: [[75488.62, 0.0135]], asks: [], first: 7183181025, last: 7183181082, prev: 7183180968 });
  assertEquals(bookFromRest('aster-spot') && bookFromRest('aster-perp') && bookFromRest('binance-spot'), true);
  assertEquals(bookFromRest('gate-spot'), false);
  assertEquals(depthUrl('aster-perp', 'BTCUSDT'), 'https://fapi.asterdex.com/fapi/v1/depth?symbol=BTCUSDT&limit=1000');
  assertEquals(depthUrl('aster-spot', 'BTCUSDT'), 'https://sapi.asterdex.com/api/v1/depth?symbol=BTCUSDT&limit=1000');
  assertEquals(depthUrl('binance-perp', 'BTCUSDT'), 'https://fapi.binance.com/fapi/v1/depth?symbol=BTCUSDT&limit=1000');
  assertEquals(depthUrl('binance-spot', 'BTCUSDT'), 'https://api.binance.com/api/v3/depth?symbol=BTCUSDT&limit=1000');
});

Deno.test('HTX names the channel, and a perpetual counts contracts on trades as in its book', () => {
  assertEquals(readFrame('htx-spot', { ch: 'market.btcusdt.trade.detail', tick: { data: [{ ts: 1793894074053, amount: 6.75e-4, price: 75595.79, direction: 'buy' }] } }), [
    ['market.btcusdt.trade.detail', { kind: 'print', time: 1793894074053, price: 75595.79, qty: 6.75e-4, buy: true }],
  ]);
  assertEquals(readFrame('htx-perp', { ch: 'market.BTC-USDT.trade.detail', tick: { data: [{ amount: 132, quantity: 0.132, ts: 1793894074585, price: 75578.5, direction: 'sell' }] } })[0]![1], { kind: 'print', time: 1793894074585, price: 75578.5, qty: 132, buy: false });
  assertEquals(readFrame('htx-perp', { ch: 'market.BTC-USDT.depth.step0', tick: { bids: [[75578.4, 401]], asks: [[75580, 2]] } }), [['market.BTC-USDT.depth.step0', { kind: 'book', reset: true, bids: [[75578.4, 401]], asks: [[75580, 2]] }]]);
  assertEquals(readFrame('htx-spot', { id: 'a', status: 'ok', subbed: 'market.btcusdt.trade.detail' }), []);
  assertEquals(subscribeFrames('htx-perp', 'book', 'BTC-USDT').frames, [{ sub: 'market.BTC-USDT.depth.step0', id: 'market.BTC-USDT.depth.step0' }]);
});

Deno.test('Bitfinex: channels by number, each trade once, a level gone when no orders are left', () => {
  const subscribed = [{ event: 'subscribed', channel: 'trades', chanId: 118, symbol: 'tBTCUSD' }, { event: 'subscribed', channel: 'book', chanId: 325, symbol: 'tBTCUSD' }];
  const read = (...frames: unknown[]) => readFrames('bitfinex-spot', [...subscribed, ...frames]);
  assertEquals(read([118, 'te', [2006727741, 1793894076402, 0.03, 75620]], [118, 'tu', [2006727741, 1793894076402, 0.03, 75620]]), [['trades|tBTCUSD', { kind: 'print', time: 1793894076402, price: 75620, qty: 0.03, buy: true }]]);
  assertEquals(read([118, [[3, 3000, -0.5, 100], [2, 2000, 0.25, 99]]]).map(([, m]) => (m as { time: number }).time), [2000, 3000], 'those it opens with, oldest first');
  assertEquals(read([118, 'hb'], [999, 'te', [1, 2, 3, 4]]), [], 'a heartbeat, and a channel nobody named');
  assertEquals(read([325, [[75588, 2, 1.5], [75590, 1, -0.5]]]), [['book|tBTCUSD', { kind: 'book', reset: true, bids: [[75588, 1.5]], asks: [[75590, 0.5]] }]]);
  assertEquals(read([325, [75588, 0, 1]]), [['book|tBTCUSD', { kind: 'book', reset: false, bids: [[75588, 0]], asks: [] }]], 'no orders: the level is gone');
  assertEquals(read([325, [75590, 0, -1]]), [['book|tBTCUSD', { kind: 'book', reset: false, bids: [], asks: [[75590, 0]] }]]);
  assertEquals(subscribeFrames('bitfinex-perp', 'book', 'tBTCF0:USTF0').frames, [{ event: 'subscribe', channel: 'book', symbol: 'tBTCF0:USTF0', prec: 'P0', freq: 'F0', len: '100' }]);
});

Deno.test('Deribit: dollars on the inverse perpetuals become coins, and each book change names the one before', () => {
  const trade = (name: string, amount: number) => ({ method: 'subscription', params: { channel: `trades.${name}.100ms`, data: [{ timestamp: 1793894074575, price: 75000, direction: 'buy', instrument_name: name, amount }] } });
  assertEquals(readFrame('deribit-perp', trade('BTC-PERPETUAL', 1500))[0]![1], { kind: 'print', time: 1793894074575, price: 75000, qty: 0.02, buy: true });
  assertEquals(readFrame('deribit-perp', trade('BTC_USDC-PERPETUAL', 0.3933))[0]![1], { kind: 'print', time: 1793894074575, price: 75000, qty: 0.3933, buy: true });
  const change = { method: 'subscription', params: { channel: 'book.BTC-PERPETUAL.100ms', data: { type: 'change', change_id: 12, prev_change_id: 11, instrument_name: 'BTC-PERPETUAL', bids: [['new', 75000, 1500], ['delete', 74999, 0]], asks: [['change', 75001, 750.01]] } } };
  assertEquals(readFrame('deribit-perp', change), [['book.BTC-PERPETUAL.100ms', { kind: 'book', reset: false, bids: [[75000, 0.02], [74999, 0]], asks: [[75001, 750.01 / 75001]], last: 12, prev: 11 }]]);
  const snapshot = { method: 'subscription', params: { channel: 'book.ETH_USDC-PERPETUAL.100ms', data: { type: 'snapshot', change_id: 5, instrument_name: 'ETH_USDC-PERPETUAL', bids: [['new', 2254.98, 0.0422]], asks: [] } } };
  assertEquals(readFrame('deribit-perp', snapshot)[0]![1], { kind: 'book', reset: true, bids: [[2254.98, 0.0422]], asks: [], last: 5, prev: undefined });
  assertEquals(readFrame('deribit-perp', { jsonrpc: '2.0', id: 0, result: { version: '1' } }), []);
  assertEquals(socketFor('deribit-perp', 'book'), 'deribit-perp');
});

Deno.test('Coinbase names the maker, so a taker goes the other way; its book is the batched one', () => {
  const match = { type: 'match', side: 'sell', size: '0.0008', price: '75479', product_id: 'BTC-USD', time: '2026-11-05T15:54:35.753Z' };
  assertEquals(readFrame('coinbase-spot', match), [['matches|BTC-USD', { kind: 'print', time: Date.parse('2026-11-05T15:54:35.753Z'), price: 75479, qty: 0.0008, buy: true }]]);
  assertEquals((readFrame('coinbase-spot', { ...match, side: 'buy' })[0]![1] as { buy: boolean }).buy, false);
  assertEquals(readFrame('coinbase-spot', { ...match, type: 'last_match' }), [], 'the one it opens with is old');
  assertEquals(readFrame('coinbase-spot', { type: 'snapshot', product_id: 'BTC-USD', bids: [['75480', '1']], asks: [['75481.48', '0.0194']] }), [['level2_batch|BTC-USD', { kind: 'book', reset: true, bids: [[75480, 1]], asks: [[75481.48, 0.0194]] }]]);
  assertEquals(readFrame('coinbase-spot', { type: 'l2update', product_id: 'BTC-USD', changes: [['buy', '75452.46', '0.16'], ['sell', '75500', '0.00000000']] }), [['level2_batch|BTC-USD', { kind: 'book', reset: false, bids: [[75452.46, 0.16]], asks: [[75500, 0]] }]]);
  assertEquals(subscribeFrames('coinbase-spot', 'book', 'BTC-USD').frames, [{ type: 'subscribe', product_ids: ['BTC-USD'], channels: ['level2_batch'] }]);
});

Deno.test('Bitstamp: the same time on both sides, though its timestamps differ, and whole books', () => {
  assertEquals(readFrame('bitstamp-spot', { event: 'trade', channel: 'live_trades_btcusd', data: { timestamp: '1793894075', amount: 0.007044, price: 75482.9, type: 1, microtimestamp: '1793894075583000' } }), [
    ['live_trades_btcusd', { kind: 'print', time: 1793894075583, price: 75482.9, qty: 0.007044, buy: false }],
  ]);
  assertEquals(readFrame('bitstamp-perp', { event: 'trade', channel: 'live_trades_btcusd-perp', data: { timestamp: '1793894078103', amount: 0.00026, price: 75537, type: 0, microtimestamp: '1793894078103000' } })[0]![1], { kind: 'print', time: 1793894078103, price: 75537, qty: 0.00026, buy: true });
  assertEquals(readFrame('bitstamp-perp', { event: 'data', channel: 'order_book_btcusd-perp', data: { bids: [['75504', '0.33113']], asks: [['75505', '1']] } }), [['order_book_btcusd-perp', { kind: 'book', reset: true, bids: [[75504, 0.33113]], asks: [[75505, 1]] }]]);
  assertEquals(readFrame('bitstamp-spot', { event: 'bts:subscription_succeeded', channel: 'order_book_btcusd', data: {} }), []);
  assertEquals(subscribeFrames('bitstamp-perp', 'prints', 'btcusd-perp').frames, [{ event: 'bts:subscribe', data: { channel: 'live_trades_btcusd-perp' } }]);
});

Deno.test('whole books are the ones that say everything every time', () => {
  for (const venue of ['hyperliquid-perp', 'gate-spot', 'gate-perp', 'htx-spot', 'htx-perp', 'bitstamp-spot', 'bitstamp-perp'] as const) assertEquals(wholeBook(venue), true, venue);
  for (const venue of ['binance-perp', 'aster-perp', 'bitfinex-spot', 'deribit-perp', 'coinbase-spot', 'okx-perp'] as const) assertEquals(wholeBook(venue), false, venue);
});
