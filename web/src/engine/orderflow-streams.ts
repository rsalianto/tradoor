/**
 * Every trade with the side that took it, and the order book, for the DOM and
 * tape panels: sockets of their own, loaded with the first of those panels, so
 * the charts' sockets and the rest of the app carry none of it. Sizes are as
 * the exchange sends them; an OKX swap's are contracts, turned into coins by
 * the reader (lib/orderflow).
 */
import type { Venue } from '../lib/markets';
import { chunks, Connection, num, relay, type BookMsg, type Json, type Protocol, type StreamListener, type StreamMsg } from './streams';

export type FlowKind = 'prints' | 'book';

type Level = [number, number];

/** `[[price, size], …]`, or `[{price, qty}, …]` as Kraken and Lighter send them. */
function levels(rows: unknown): Level[] {
  if (!Array.isArray(rows)) return [];
  return rows.map((r): Level => (Array.isArray(r) ? [num(r[0]), num(r[1])] : [num((r as Json).price), num((r as Json).qty ?? (r as Json).size)]));
}

const book = (reset: boolean, bids: unknown, asks: unknown, seq: Partial<Pick<BookMsg, 'first' | 'last' | 'prev'>> = {}): BookMsg => ({ kind: 'book', reset, bids: levels(bids), asks: levels(asks), ...seq });

/**
 * Binance: aggregated trades, and the book's changes every 100ms — the book
 * itself comes from REST (lib/orderflow lines the two up). A futures change
 * names the one before it (`pu`); a spot change follows on from it (`U`).
 * Nothing at all is pushed for a quiet market, so the subscription list is
 * asked for now and then to show the socket is alive.
 */
function binance(urls: string[]): Protocol {
  let id = 0;
  return {
    urls,
    topic: (spec) => `${spec.symbol.toLowerCase()}@${spec.kind === 'book' ? 'depth@100ms' : 'aggTrade'}`,
    frames: (op, topics) => chunks(topics, 100).map((params) => ({ method: op.toUpperCase(), params, id: ++id })),
    read(m, emit) {
      const s = typeof m.s === 'string' ? m.s.toLowerCase() : '';
      if (m.e === 'aggTrade') {
        // `m`: the buyer was the maker, so the taker sold.
        emit(`${s}@aggTrade`, { kind: 'print', time: num(m.T), price: num(m.p), qty: num(m.q), buy: m.m === false });
      } else if (m.e === 'depthUpdate') {
        const first = num(m.U);
        emit(`${s}@depth@100ms`, book(false, m.b, m.a, { first, last: num(m.u), prev: m.pu === undefined ? first - 1 : num(m.pu) }));
      }
    },
    heartbeat: { method: 'LIST_SUBSCRIPTIONS', id: 0 },
  };
}

/**
 * Bybit: the thousand best levels a side, a snapshot then changes every 200ms.
 * An update id of 1 is the exchange restarted, and is a snapshot too.
 */
function bybit(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => (spec.kind === 'book' ? `orderbook.1000.${spec.symbol}` : `publicTrade.${spec.symbol}`),
    frames: (op, topics) => chunks(topics, 10).map((args) => ({ op, args, req_id: args.join(',') })),
    read(m, emit) {
      const topic = m.topic;
      if (typeof topic !== 'string') return;
      if (topic.startsWith('publicTrade.') && Array.isArray(m.data)) {
        for (const t of m.data as Json[]) emit(topic, { kind: 'print', time: num(t.T), price: num(t.p), qty: num(t.v), buy: t.S === 'Buy' });
      } else if (topic.startsWith('orderbook.') && m.data && typeof m.data === 'object') {
        const d = m.data as Json;
        emit(topic, book(m.type === 'snapshot' || d.u === 1, d.b, d.a));
      }
    },
    heartbeat: { op: 'ping' },
  };
}

/** OKX: 400 levels a side, each change numbering the one before it. */
function okx(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'books' : 'trades'}|${spec.symbol}`,
    frames: (op, topics) => chunks(topics, 20).map((batch) => ({ op, args: batch.map((t) => ({ channel: t.split('|')[0], instId: t.split('|')[1] })) })),
    read(m, emit) {
      const arg = m.arg as Json | undefined;
      if (!arg || !Array.isArray(m.data)) return;
      const topic = `${arg.channel}|${arg.instId}`;
      if (arg.channel === 'trades') {
        for (const t of m.data as Json[]) emit(topic, { kind: 'print', time: num(t.ts), price: num(t.px), qty: num(t.sz), buy: t.side === 'buy' });
      } else if (arg.channel === 'books') {
        const d = (m.data as Json[])[0];
        if (!d) return;
        const reset = m.action === 'snapshot';
        emit(topic, book(reset, d.bids, d.asks, { last: num(d.seqId), prev: reset ? undefined : num(d.prevSeqId) }));
      }
    },
    heartbeat: 'ping',
  };
}

/** Bitget: every level, a snapshot then changes; spot and futures on one socket. */
function bitget(urls: string[]): Protocol {
  const kind = (venue: Venue) => (venue === 'bitget-perp' ? 'USDT-FUTURES' : 'SPOT');
  return {
    urls,
    topic: (spec, venue) => `${kind(venue)}|${spec.kind === 'book' ? 'books' : 'trade'}|${spec.symbol}`,
    frames: (op, topics) =>
      chunks(topics, 20).map((batch) => ({
        op,
        args: batch.map((t) => {
          const [instType, channel, instId] = t.split('|');
          return { instType, channel, instId };
        }),
      })),
    read(m, emit) {
      const arg = m.arg as Json | undefined;
      if (!arg || !Array.isArray(m.data)) return;
      const topic = `${arg.instType}|${arg.channel}|${arg.instId}`;
      if (arg.channel === 'trade') {
        // Newest first, the snapshot on subscribing as much as the rest.
        for (const t of (m.data as Json[]).slice().reverse()) emit(topic, { kind: 'print', time: num(t.ts), price: num(t.price), qty: num(t.size), buy: t.side === 'buy' });
      } else if (arg.channel === 'books') {
        const d = (m.data as Json[])[0];
        if (d) emit(topic, book(m.action === 'snapshot', d.bids, d.asks));
      }
    },
    heartbeat: 'ping',
  };
}

/** Hyperliquid: the twenty best levels a side, whole every time. */
function hyperliquid(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'l2Book' : 'trades'}|${spec.symbol}`,
    frames: (method, topics) =>
      topics.map((t) => {
        const [type, coin] = t.split('|');
        return { method, subscription: { type, coin } };
      }),
    read(m, emit) {
      if (m.channel === 'trades' && Array.isArray(m.data)) {
        // `B`: the aggressor bought; `A`: it sold into the bid.
        for (const t of m.data as Json[]) emit(`trades|${t.coin}`, { kind: 'print', time: num(t.time), price: num(t.px), qty: num(t.sz), buy: t.side === 'B' });
      } else if (m.channel === 'l2Book' && m.data && typeof m.data === 'object') {
        const d = m.data as Json;
        const [bids, asks] = Array.isArray(d.levels) ? (d.levels as unknown[]) : [];
        const side = (rows: unknown) => (Array.isArray(rows) ? rows.map((r) => [(r as Json).px, (r as Json).sz]) : []);
        emit(`l2Book|${d.coin}`, book(true, side(bids), side(asks)));
      }
    },
    heartbeat: { method: 'ping' },
  };
}

/** Kraken spot, v2: a thousand levels a side; trades with an ISO time. */
function krakenSpot(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'book' : 'trade'}|${spec.symbol}`,
    frames: (method, topics) =>
      topics.map((t) => {
        const [channel, symbol] = t.split('|');
        return { method, params: channel === 'book' ? { channel, symbol: [symbol], depth: 1000 } : { channel, symbol: [symbol] } };
      }),
    read(m, emit) {
      if (!Array.isArray(m.data)) return;
      for (const d of m.data as Json[]) {
        if (m.channel === 'trade') emit(`trade|${d.symbol}`, { kind: 'print', time: Date.parse(String(d.timestamp)), price: num(d.price), qty: num(d.qty), buy: d.side === 'buy' });
        else if (m.channel === 'book') emit(`book|${d.symbol}`, book(m.type === 'snapshot', d.bids, d.asks));
      }
    },
    heartbeat: { method: 'ping' },
  };
}

/** Kraken Futures: the whole book, then each level as it changes, one a frame. */
function krakenFutures(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'book' : 'trade'}|${spec.symbol}`,
    frames: (event, topics) =>
      topics.map((t) => {
        const [feed, product] = t.split('|');
        return { event, feed, product_ids: [product] };
      }),
    read(m, emit) {
      const feed = m.feed;
      const print = (t: Json) => emit(`trade|${m.product_id}`, { kind: 'print', time: num(t.time), price: num(t.price), qty: num(t.qty), buy: t.side === 'buy' });
      if (feed === 'trade') print(m);
      else if (feed === 'trade_snapshot' && Array.isArray(m.trades)) for (const t of (m.trades as Json[]).slice().reverse()) print(t);
      else if (feed === 'book_snapshot') emit(`book|${m.product_id}`, book(true, m.bids, m.asks));
      else if (feed === 'book') {
        const level = [[m.price, m.qty]];
        emit(`book|${m.product_id}`, m.side === 'buy' ? book(false, level, []) : book(false, [], level));
      }
    },
    open: [{ event: 'subscribe', feed: 'heartbeat' }],
  };
}

/** Lighter: markets by number; asked with a slash, answered with a colon. */
function lighter(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => `${spec.kind === 'book' ? 'order_book' : 'trade'}/${spec.symbol}`,
    frames: (type, topics) => topics.map((channel) => ({ type, channel })),
    read(m, emit) {
      const channel = typeof m.channel === 'string' ? m.channel.replace(':', '/') : '';
      const type = typeof m.type === 'string' ? m.type : '';
      if (channel.startsWith('trade/') && Array.isArray(m.trades)) {
        // The maker was the ask: the taker bought.
        for (const t of (m.trades as Json[]).slice().reverse()) emit(channel, { kind: 'print', time: num(t.timestamp), price: num(t.price), qty: num(t.size), buy: t.is_maker_ask === true });
      } else if (channel.startsWith('order_book/') && m.order_book && typeof m.order_book === 'object') {
        const d = m.order_book as Json;
        emit(channel, book(type.startsWith('subscribed'), d.bids, d.asks));
      }
    },
  };
}

/** Sockets by name; the relay's names where it has the same socket. */
const SOCKETS: Record<string, () => Protocol> = {
  'binance-spot': () => binance(['wss://stream.binance.com:9443/ws', 'wss://data-stream.binance.vision/ws', ...relay('binance-spot')]),
  // Futures split their streams: trades on `market`, the book on `public`.
  'binance-perp': () => binance(['wss://fstream.binance.com/market/ws', 'wss://fstream.binance.com/ws', ...relay('binance-perp')]),
  'binance-perp-book': () => binance(['wss://fstream.binance.com/public/ws', 'wss://fstream.binance.com/ws', ...relay('binance-perp-book')]),
  'bybit-spot': () => bybit(['wss://stream.bybit.com/v5/public/spot', ...relay('bybit-spot')]),
  'bybit-perp': () => bybit(['wss://stream.bybit.com/v5/public/linear', ...relay('bybit-perp')]),
  'okx-public': () => okx(['wss://ws.okx.com/ws/v5/public', ...relay('okx-public')]),
  bitget: () => bitget(['wss://ws.bitget.com/v2/ws/public', ...relay('bitget')]),
  hyperliquid: () => hyperliquid(['wss://api.hyperliquid.xyz/ws', ...relay('hyperliquid')]),
  'kraken-spot': () => krakenSpot(['wss://ws.kraken.com/v2', ...relay('kraken-spot')]),
  'kraken-perp': () => krakenFutures(['wss://futures.kraken.com/ws/v1', ...relay('kraken-perp')]),
  lighter: () => lighter(['wss://mainnet.zklighter.elliot.ai/stream', ...relay('lighter')]),
};

export function socketFor(venue: Venue, kind: FlowKind): string {
  if (venue === 'binance-perp' && kind === 'book') return 'binance-perp-book';
  if (venue.startsWith('okx')) return 'okx-public';
  if (venue.startsWith('bitget')) return 'bitget';
  if (venue === 'hyperliquid-perp') return 'hyperliquid';
  if (venue === 'lighter-perp') return 'lighter';
  return venue;
}

/** Whether a venue's book arrives as changes only, to be laid over a REST snapshot. */
export const bookFromRest = (venue: Venue): boolean => venue === 'binance-spot' || venue === 'binance-perp';

export class OrderFlowHub {
  private readonly connections = new Map<string, Connection>();

  /** `symbol` is the exchange's own name (lib/native). Nothing for an index. */
  subscribe(venue: Venue, kind: FlowKind, symbol: string, listener: StreamListener): () => void {
    const socket = socketFor(venue, kind);
    const make = SOCKETS[socket];
    if (!make) {
      listener.state('stalled', false);
      return () => {};
    }
    let conn = this.connections.get(socket);
    if (!conn) {
      conn = new Connection(make());
      this.connections.set(socket, conn);
    }
    // Quiet: a market nobody trades pushes nothing for minutes, and any frame
    // at all — the answer to a subscription, a pong — shows the socket is up.
    return conn.add(conn.topic({ kind, symbol }, venue), listener, true);
  }
}

/** For tests: one frame read by a socket's protocol. */
export function readFrame(socket: string, frame: Json): [string, StreamMsg][] {
  const out: [string, StreamMsg][] = [];
  SOCKETS[socket]!().read(frame, (topic, msg) => out.push([topic, msg]));
  return out;
}

/** For tests: the topic and subscribe frames a spec makes. */
export function subscribeFrames(venue: Venue, kind: FlowKind, symbol: string): { topic: string; frames: unknown[] } {
  const proto = SOCKETS[socketFor(venue, kind)]!();
  const topic = proto.topic({ kind, symbol }, venue);
  return { topic, frames: proto.frames('subscribe', [topic]) };
}
