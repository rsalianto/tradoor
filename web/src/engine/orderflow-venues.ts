/**
 * Every trade and the order book on the newer exchanges, for the DOM, the tape
 * and footprints recorded in the browser. On its own because a chart loads
 * `orderflow-streams`, and only a market on one of these needs any of this.
 */
import { binance, book } from './orderflow-streams';
import { num, relay, type Json, type Protocol } from './streams';

/**
 * Gate: the best hundred levels a side, whole, ten times a second. A spot trade
 * is one to a frame and names the side that took it; a perpetual's come in a
 * list, a negative size being a sale, and the last hundred seconds' arrive on
 * subscribing. A ping carries the time, and one without it goes unanswered.
 */
function gate(urls: string[], side: 'spot' | 'futures'): Protocol {
  const now = () => Math.floor(Date.now() / 1000);
  const quotes = (rows: unknown) => (Array.isArray(rows) ? rows.map((r) => [(r as Json).p, (r as Json).s]) : rows);
  return {
    urls,
    topic: (spec) => `${side}.${spec.kind === 'book' ? 'order_book' : 'trades'}|${spec.symbol}`,
    frames: (event, topics) =>
      topics.map((t) => {
        const [channel, symbol] = t.split('|');
        return { time: now(), channel, event, payload: channel!.endsWith('book') ? [symbol, '100', side === 'spot' ? '100ms' : '0'] : [symbol] };
      }),
    read(m, emit) {
      const r = m.result as Json;
      if (m.event === 'update' && m.channel === 'spot.trades') {
        emit(`spot.trades|${r.currency_pair}`, { kind: 'print', time: Math.floor(num(r.create_time_ms)), price: num(r.price), qty: num(r.amount), buy: r.side === 'buy' });
      } else if (m.event === 'update' && m.channel === 'futures.trades' && Array.isArray(m.result)) {
        for (const t of m.result as Json[]) emit(`futures.trades|${t.contract}`, { kind: 'print', time: num(t.create_time_ms), price: num(t.price), qty: Math.abs(num(t.size)), buy: num(t.size) > 0 });
      } else if (m.event === 'update' && m.channel === 'spot.order_book') {
        emit(`spot.order_book|${r.s}`, book(true, r.bids, r.asks));
      } else if (m.event === 'all' && m.channel === 'futures.order_book') {
        emit(`futures.order_book|${r.contract}`, book(true, quotes(r.bids), quotes(r.asks)));
      }
    },
    get heartbeat() {
      return { time: now(), channel: `${side}.ping` };
    },
  };
}

/**
 * HTX: every frame gzipped, and a ping every few seconds that must be answered.
 * The best 150 levels a side, whole. A perpetual's `amount` is contracts, on a
 * trade as in the book, and `direction` is the side that took it.
 */
function htx(urls: string[]): Protocol {
  let tail: Promise<unknown> = Promise.resolve();
  return {
    urls,
    topic: (spec) => `market.${spec.symbol}.${spec.kind === 'book' ? 'depth.step0' : 'trade.detail'}`,
    frames: (op, topics) => topics.map((ch) => ({ [op === 'subscribe' ? 'sub' : 'unsub']: ch, id: ch })),
    reply: (m) => (m.ping ? { pong: m.ping } : null),
    inflate(data) {
      // One after another, so a frame cannot overtake the one before it.
      const text = tail.then(async () => {
        try {
          return await new Response(data.stream().pipeThrough(new DecompressionStream('gzip'))).text();
        } catch {
          return '';
        }
      });
      tail = text;
      return text;
    },
    read(m, emit) {
      const ch = m.ch;
      const t = m.tick as Json | undefined;
      if (typeof ch !== 'string' || !t) return;
      if (ch.endsWith('.trade.detail')) {
        for (const r of t.data as Json[]) emit(ch, { kind: 'print', time: num(r.ts), price: num(r.price), qty: num(r.amount), buy: r.direction === 'buy' });
      } else if (ch.endsWith('.depth.step0')) {
        emit(ch, book(true, t.bids, t.asks));
      }
    },
  };
}

/**
 * Bitfinex numbers a subscription when it answers, and sends only that number
 * after. Every trade comes twice, `te` and then `tu`, and the last few, newest
 * first, on subscribing; a negative amount is a sale. A book row is `[price,
 * orders, amount]`: no orders is the level gone, and a negative amount an ask.
 */
function bitfinex(urls: string[]): Protocol {
  const topics = new Map<number, string>();
  const channels = new Map<string, number>();
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'book' : 'trades'}|${spec.symbol}`,
    frames: (op, list) =>
      list.flatMap((t): unknown[] => {
        const [channel, symbol] = t.split('|');
        if (op === 'subscribe') return [channel === 'book' ? { event: 'subscribe', channel, symbol, prec: 'P0', freq: 'F0', len: '100' } : { event: 'subscribe', channel, symbol }];
        const chanId = channels.get(t);
        return chanId === undefined ? [] : [{ event: 'unsubscribe', chanId }];
      }),
    read(m, emit) {
      if (!Array.isArray(m)) {
        if (m.event === 'subscribed') {
          const t = `${m.channel}|${m.symbol}`;
          topics.set(m.chanId as number, t);
          channels.set(t, m.chanId as number);
        }
        return;
      }
      const [chan, first, second] = m as unknown as [number, unknown, unknown];
      const topic = topics.get(chan);
      if (!topic) return;
      if (topic.startsWith('trades')) {
        const rows = (first === 'te' ? [second] : Array.isArray(first) && Array.isArray(first[0]) ? first.slice().reverse() : []) as number[][];
        for (const r of rows) emit(topic, { kind: 'print', time: r[1]!, price: r[3]!, qty: Math.abs(r[2]!), buy: r[2]! > 0 });
      } else if (Array.isArray(first)) {
        const whole = Array.isArray(first[0]);
        const rows = (whole ? first : [first]) as number[][];
        const side = (bids: boolean) => rows.filter((r) => r[2]! > 0 === bids).map((r) => [r[0], r[1] === 0 ? 0 : Math.abs(r[2]!)]);
        emit(topic, book(whole, side(true), side(false)));
      }
    },
  };
}

/**
 * Deribit: the whole book and then its changes, each naming the one before. On
 * `BTC-PERPETUAL` and `ETH-PERPETUAL` sizes are dollars; on the USDC ones, named
 * with an underscore, they are coins. They leave here as coins.
 */
function deribit(urls: string[]): Protocol {
  const coins = (name: unknown, price: number, amount: number) => (String(name).includes('_') ? amount : amount / price);
  return {
    urls,
    topic: (spec) => (spec.kind === 'book' ? `book.${spec.symbol}.100ms` : `trades.${spec.symbol}.100ms`),
    frames: (op, channels) => [{ jsonrpc: '2.0', id: 1, method: `public/${op}`, params: { channels } }],
    read(m, emit) {
      if (m.method !== 'subscription') return;
      const { channel, data } = m.params as { channel: string; data: Json & Json[] };
      if (channel.startsWith('trades.')) {
        for (const t of data) {
          const price = num(t.price);
          emit(channel, { kind: 'print', time: num(t.timestamp), price, qty: coins(t.instrument_name, price, num(t.amount)), buy: t.direction === 'buy' });
        }
      } else if (channel.startsWith('book.')) {
        const side = (rows: unknown) => (rows as [string, number, number][]).map(([, price, amount]) => [price, coins(data.instrument_name, price, amount)]);
        emit(channel, book(data.type === 'snapshot', side(data.bids), side(data.asks), { last: num(data.change_id), prev: data.prev_change_id === undefined ? undefined : num(data.prev_change_id) }));
      }
    },
    heartbeat: { jsonrpc: '2.0', id: 0, method: 'public/test' },
  };
}

/**
 * Coinbase: `side` on a match is the maker's, so the taker went the other way.
 * `level2` wants a key, and `level2_batch` does not: the whole book once, which
 * can be a megabyte, then its changes. One product's heartbeat keeps a quiet
 * socket from looking dead.
 */
function coinbase(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'level2_batch' : 'matches'}|${spec.symbol}`,
    frames: (type, topics) =>
      topics.map((t) => {
        const [channel, product] = t.split('|');
        return { type, product_ids: [product], channels: [channel] };
      }),
    read(m, emit) {
      if (m.type === 'match') {
        emit(`matches|${m.product_id}`, { kind: 'print', time: Date.parse(String(m.time)), price: num(m.price), qty: num(m.size), buy: m.side === 'sell' });
      } else if (m.type === 'snapshot') {
        emit(`level2_batch|${m.product_id}`, book(true, m.bids, m.asks));
      } else if (m.type === 'l2update') {
        const side = (name: string) => (m.changes as string[][]).filter((c) => c[0] === name).map((c) => [c[1], c[2]]);
        emit(`level2_batch|${m.product_id}`, book(false, side('buy'), side('sell')));
      }
    },
    open: [{ type: 'subscribe', product_ids: ['BTC-USD'], channels: ['heartbeat'] }],
  };
}

/**
 * Bitstamp: the best hundred levels a side, whole, ten times a second. `type` is
 * the side that took a trade, 0 a buy. Spot's `timestamp` is in seconds and a
 * perpetual's in milliseconds, so the time is read from `microtimestamp`.
 */
function bitstamp(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'order_book_' : 'live_trades_'}${spec.symbol}`,
    frames: (op, topics) => topics.map((channel) => ({ event: `bts:${op}`, data: { channel } })),
    read(m, emit) {
      const channel = String(m.channel);
      const d = m.data as Json;
      if (m.event === 'trade') {
        emit(channel, { kind: 'print', time: Math.floor(num(d.microtimestamp) / 1000), price: num(d.price), qty: num(d.amount), buy: d.type === 0 });
      } else if (m.event === 'data' && channel.startsWith('order_book_')) {
        emit(channel, book(true, d.bids, d.asks));
      }
    },
    heartbeat: { event: 'bts:heartbeat' },
  };
}

export const SOCKETS: Record<string, () => Protocol> = {
  'gate-spot': () => gate(['wss://api.gateio.ws/ws/v4/', ...relay('gate-spot')], 'spot'),
  'gate-perp': () => gate(['wss://fx-ws.gateio.ws/v4/ws/usdt', ...relay('gate-perp')], 'futures'),
  // Aster is Binance's stream, and answers the same asking.
  'aster-spot': () => binance(['wss://sstream.asterdex.com/ws', ...relay('aster-spot')]),
  'aster-perp': () => binance(['wss://fstream.asterdex.com/ws', ...relay('aster-perp')]),
  'htx-spot': () => htx(['wss://api.huobi.pro/ws', ...relay('htx-spot')]),
  'htx-perp': () => htx(['wss://api.hbdm.com/linear-swap-ws', ...relay('htx-perp')]),
  'bitfinex-spot': () => bitfinex(['wss://api-pub.bitfinex.com/ws/2', ...relay('bitfinex-spot')]),
  'bitfinex-perp': () => bitfinex(['wss://api-pub.bitfinex.com/ws/2', ...relay('bitfinex-perp')]),
  'deribit-perp': () => deribit(['wss://www.deribit.com/ws/api/v2', ...relay('deribit-perp')]),
  'coinbase-spot': () => coinbase(['wss://ws-feed.exchange.coinbase.com', ...relay('coinbase-spot')]),
  'bitstamp-spot': () => bitstamp(['wss://ws.bitstamp.net', ...relay('bitstamp-spot')]),
  'bitstamp-perp': () => bitstamp(['wss://ws.bitstamp.net', ...relay('bitstamp-perp')]),
};
