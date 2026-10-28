/**
 * Exchange WebSocket streams, shared: one socket per venue however many charts
 * and watchlist rows listen, each topic subscribed once and reference counted.
 * The worker uses it for candles and trades, the page for watchlist tickers;
 * nothing here touches the DOM.
 */
import type { Venue } from '../lib/markets';
import { block, isBlocked as known, onWake, reached } from './reach';

export type SocketState = 'connecting' | 'live' | 'stalled' | 'retrying';

/** `symbol` is the exchange's own name for the market (see lib/native). */
export type StreamSpec =
  /// `quiet`: a market that may push nothing for hours — Bitget's stock tokens
  /// outside their trading hours — so the exchange's answer is enough to be live.
  | { kind: 'kline'; symbol: string; interval: string; quiet?: boolean }
  | { kind: 'trades'; symbol: string }
  | { kind: 'ticker'; symbol: string }
  /// Positions the exchange closed out — perpetuals only.
  | { kind: 'liquidations'; symbol: string }
  /// Every trade with the side that took it, and the order book: the DOM and
  /// tape panels' own sockets (see orderflow-streams).
  | { kind: 'prints'; symbol: string }
  | { kind: 'book'; symbol: string };

export interface KlineMsg {
  kind: 'kline';
  openTime: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  /** Last trade folded into the kline, comparable with a trade's id. */
  lastTradeId: number;
  closed: boolean;
}

export interface TradeMsg {
  kind: 'trade';
  time: number;
  price: number;
  qty: number;
  lastTradeId: number;
}

export interface TickerMsg {
  kind: 'ticker';
  last: number;
  changePct: number | null;
  quoteVolume: number | null;
}

export interface LiquidationMsg {
  kind: 'liquidation';
  time: number;
  price: number;
  qty: number;
  /** A long was liquidated: the exchange sold someone out. */
  long: boolean;
}

/** A trade and its aggressor: `buy`, the taker bought. */
export interface PrintMsg {
  kind: 'print';
  time: number;
  price: number;
  qty: number;
  buy: boolean;
}

/**
 * Order book levels as `[price, size]`, a size of 0 taken away. `reset`: the
 * whole book rather than changes to it. `last` and `prev` number the changes
 * where the exchange does, so a missed one shows: `prev` is the `last` the
 * one before carried. `first` is Binance's, for matching changes to a snapshot.
 */
export interface BookMsg {
  kind: 'book';
  reset: boolean;
  bids: [number, number][];
  asks: [number, number][];
  first?: number;
  last?: number;
  prev?: number;
}

export type StreamMsg = KlineMsg | TradeMsg | TickerMsg | LiquidationMsg | PrintMsg | BookMsg;

export interface StreamListener {
  message(msg: StreamMsg): void;
  state(state: SocketState, relayed: boolean): void;
}

export type Json = Record<string, unknown>;

export interface Protocol {
  /** Tried in turn when a socket opens and nothing ever arrives. */
  urls: readonly string[];
  /** The topic for a spec, or '' when this exchange does not stream it. */
  topic(spec: StreamSpec, venue: Venue): string;
  frames(op: 'subscribe' | 'unsubscribe', topics: string[]): unknown[];
  read(msg: Json, emit: (topic: string, msg: StreamMsg) => void): void;
  /** A refused frame: the topic the exchange named, and the batch it came in. */
  refused?(msg: Json): { bad: string; batch: string[] } | null;
  /** Sent every so often; a string goes as it is, anything else as JSON. */
  heartbeat?: unknown;
  /** Sent once each time the socket opens, before the subscriptions. */
  open?: unknown[];
}

export const num = (v: unknown): number => (typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : Number.NaN);

export function chunks<T>(list: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < list.length; i += size) out.push(list.slice(i, i + size));
  return out;
}

/**
 * Binance: a bare `/ws` socket and SUBSCRIBE frames. Streams named in the URL
 * path are silently dropped on some networks — the handshake succeeds and
 * nothing is pushed — while a SUBSCRIBE on the bare path is not.
 */
function binance(urls: string[]): Protocol {
  let id = 0;
  return {
    urls,
    topic(spec) {
      const s = spec.symbol.toLowerCase();
      if (spec.kind === 'kline') return `${s}@kline_${spec.interval}`;
      if (spec.kind === 'trades') return `${s}@aggTrade`;
      return spec.kind === 'liquidations' ? `${s}@forceOrder` : `${s}@miniTicker`;
    },
    // Every frame counts against a five-a-second limit, so topics travel in batches.
    frames: (op, topics) => chunks(topics, 100).map((params) => ({ method: op.toUpperCase(), params, id: ++id })),
    read(m, emit) {
      const s = typeof m.s === 'string' ? m.s.toLowerCase() : '';
      if (m.e === 'kline' && m.k && typeof m.k === 'object') {
        const k = m.k as Json;
        emit(`${s}@kline_${k.i}`, {
          kind: 'kline',
          openTime: num(k.t),
          open: num(k.o),
          high: num(k.h),
          low: num(k.l),
          close: num(k.c),
          volume: num(k.v),
          lastTradeId: typeof k.L === 'number' ? k.L : -1,
          closed: k.x === true,
        });
      } else if (m.e === 'aggTrade') {
        emit(`${s}@aggTrade`, { kind: 'trade', time: num(m.T), price: num(m.p), qty: num(m.q), lastTradeId: typeof m.l === 'number' ? m.l : -1 });
      } else if (m.e === 'forceOrder' && m.o && typeof m.o === 'object') {
        const o = m.o as Json;
        // `p` is the order's limit price, set through the book so the position
        // is certain to close — a percent or more off the market. `ap` is what
        // it actually filled at, which is where the hit belongs on the chart.
        const filled = num(o.ap);
        // A forced SELL closes a long position.
        emit(`${String(o.s ?? '').toLowerCase()}@forceOrder`, {
          kind: 'liquidation',
          time: num(o.T),
          price: filled > 0 ? filled : num(o.p),
          qty: num(o.q),
          long: o.S === 'SELL',
        });
      } else if (m.e === '24hrMiniTicker') {
        const last = num(m.c);
        const open = num(m.o);
        emit(`${s}@miniTicker`, { kind: 'ticker', last, changePct: open > 0 ? ((last - open) / open) * 100 : null, quoteVolume: num(m.q) });
      }
    },
  };
}

const BYBIT_INTERVAL: Record<string, string> = {
  '1m': '1', '5m': '5', '15m': '15', '30m': '30',
  '1h': '60', '2h': '120', '4h': '240', '6h': '360', '12h': '720',
  '1d': 'D', '1w': 'W', '1M': 'M',
};

/**
 * Bybit: a ping every 15 seconds against its 20-second idle timeout, and linear
 * tickers that arrive as a snapshot followed by deltas. Trades are not folded —
 * its trade ids are strings that cannot be compared with a kline, and its kline
 * already pushes often.
 */
function bybit(urls: string[]): Protocol {
  const tickers = new Map<string, { last: number; pct: number | null; turnover: number | null }>();
  return {
    urls,
    topic(spec) {
      if (spec.kind === 'kline') return `kline.${BYBIT_INTERVAL[spec.interval] ?? spec.interval}.${spec.symbol}`;
      if (spec.kind === 'liquidations') return `allLiquidation.${spec.symbol}`;
      return spec.kind === 'ticker' ? `tickers.${spec.symbol}` : `publicTrade.${spec.symbol}`;
    },
    // Spot accepts at most ten topics a frame. The id carries the batch back:
    // Bybit refuses the whole frame over one bad topic and names only that one.
    frames: (op, topics) => chunks(topics, 10).map((args) => ({ op, args, req_id: args.join(',') })),
    refused(m) {
      if (m.op !== 'subscribe' || m.success !== false) return null;
      const named = /topic:([^\s,]+)/.exec(String(m.ret_msg ?? ''));
      return { bad: named?.[1] ?? '', batch: typeof m.req_id === 'string' && m.req_id !== '' ? m.req_id.split(',') : [] };
    },
    read(m, emit) {
      const topic = m.topic;
      if (typeof topic !== 'string') return;
      if (topic.startsWith('kline.') && Array.isArray(m.data)) {
        for (const k of m.data as Json[]) {
          // No trades are folded here, so every kline is taken whole: id 0.
          emit(topic, { kind: 'kline', openTime: num(k.start), open: num(k.open), high: num(k.high), low: num(k.low), close: num(k.close), volume: num(k.volume), lastTradeId: 0, closed: k.confirm === true });
        }
      } else if (topic.startsWith('allLiquidation.') && Array.isArray(m.data)) {
        for (const l of m.data as Json[]) {
          // Bybit reports the POSITION side: Buy means a long was liquidated.
          emit(topic, { kind: 'liquidation', time: num(l.T), price: num(l.p), qty: num(l.v), long: l.S === 'Buy' });
        }
      } else if (topic.startsWith('tickers.') && m.data && typeof m.data === 'object') {
        const d = m.data as Json;
        const prev = m.type === 'delta' ? tickers.get(topic) : undefined;
        const next = {
          last: d.lastPrice !== undefined ? num(d.lastPrice) : (prev?.last ?? Number.NaN),
          pct: d.price24hPcnt !== undefined ? num(d.price24hPcnt) * 100 : (prev?.pct ?? null),
          turnover: d.turnover24h !== undefined ? num(d.turnover24h) : (prev?.turnover ?? null),
        };
        tickers.set(topic, next);
        if (Number.isFinite(next.last)) emit(topic, { kind: 'ticker', last: next.last, changePct: next.pct, quoteVolume: next.turnover });
      }
    },
    heartbeat: { op: 'ping' },
  };
}

/**
 * The same stream through the tradoor server, tried last: on networks that
 * block the exchanges' names the server can still reach them. A socket that
 * fails before a word moves to the next address, so the relay is only used
 * where the exchange itself is out of reach.
 */
export const relay = (socket: string): string[] =>
  typeof location === 'undefined' ? [] : [`${location.origin.replace(/^http/, 'ws')}/api/relay/ws?venue=${socket}`];

/** Sockets by name: one per venue, except where an exchange shares or splits them. */
const PROTOCOLS: Record<string, () => Protocol> = {
  'binance-spot': () => binance(['wss://stream.binance.com:9443/ws', 'wss://data-stream.binance.vision/ws', ...relay('binance-spot')]),
  'binance-perp': () => binance(['wss://fstream.binance.com/market/ws', 'wss://fstream.binance.com/ws', ...relay('binance-perp')]),
  'bybit-spot': () => bybit(['wss://stream.bybit.com/v5/public/spot', ...relay('bybit-spot')]),
  'bybit-perp': () => bybit(['wss://stream.bybit.com/v5/public/linear', ...relay('bybit-perp')]),
};

/** The other exchanges' protocols load with the first market that needs one. */
let more: Promise<typeof import('./streams-more')> | null = null;
const loadMore = () => (more ??= import('./streams-more'));

/** The other exchanges' protocols, fetched now rather than with the first
 *  market on one: a watchlist's indices and index perpetuals need them. */
export const loadOtherStreams = (): void => void loadMore();

/** The socket a venue's spec travels on. */
export function socketOf(venue: Venue, spec: StreamSpec): string {
  if (venue.startsWith('okx')) return spec.kind === 'kline' ? 'okx-business' : 'okx-public';
  if (venue.startsWith('bitget')) return 'bitget';
  if (venue === 'hyperliquid-perp') return 'hyperliquid';
  if (venue === 'lighter-perp') return 'lighter';
  return venue;
}

/**
 * Addresses that answered nothing lately are skipped (see reach): a host that
 * never connected, or a URL that connected and said nothing.
 *
 * A network that blocks an exchange blocks it for every socket and every
 * market: without this, each new chart works through the same dead addresses
 * and their timeouts again before it reaches the relay, and a phone on a
 * carrier that blocks them never seems to finish reconnecting.
 */
export const isBlocked = (url: string, now = Date.now()): boolean => known(url, now) || known(hostOf(url), now);

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** The address to try: the next one that is not known dead, or the next of all. */
export function nextUrl(urls: readonly string[], index: number, dead: (url: string) => boolean = isBlocked): string {
  const alive = urls.filter((url) => !dead(url));
  const list = alive.length > 0 ? alive : urls;
  return list[index % list.length]!;
}

/** Whether a stream arrives through the tradoor server rather than the exchange. */
export const isRelayed = (url: string): boolean => url.includes('/api/relay/ws');

/**
 * A socket that has said nothing this long is dead whatever it claims: iOS
 * hands a page back after a lock with sockets that are open and carry nothing,
 * and the price simply stops. Every exchange either pushes a candle every
 * couple of seconds or answers the heartbeat every fifteen, so half a minute of
 * silence is not a quiet market — it is a socket to throw away.
 */
const SILENT_MS = 30_000;
const WATCH_MS = 5_000;

/**
 * A socket not yet open by now never will be: a network that swallows an
 * address leaves it connecting until the phone gives up, a minute or more.
 */
const OPEN_MS = 5_000;
/**
 * Except the relay, which is where a blocked exchange leads and so has nothing
 * after it to hurry on to; its handshake crosses Cloudflare and the server,
 * and on a slow line takes a few seconds on its own.
 */
const RELAY_OPEN_MS = 20_000;
/** Back from a lock, a socket that says nothing this long is replaced. */
const WAKE_MS = 6_000;
const STALL_MS = 12_000;
const RETRY_MS = [1_000, 3_000, 5_000, 10_000];
/** Onto another address after one failed: nothing to back off from yet. */
const MOVE_MS = 250;
const HEARTBEAT_MS = 15_000;
/** An unused socket stays open this long, so switching timeframe does not reconnect. */
const LINGER_MS = 10_000;

type Timer = ReturnType<typeof setTimeout>;

export class Connection {
  private ws: WebSocket | null = null;
  private current: SocketState = 'connecting';
  private readonly topics = new Map<string, Set<StreamListener>>();
  /**
   * Topics that push only when something happens — liquidations. A socket that
   * carries nothing else is rightly silent for minutes, so for it any frame at
   * all, a subscribe answer or a pong, proves the exchange is there.
   */
  private readonly quiet = new Set<string>();
  /** Topics the exchange has refused; re-sending them would refuse the frame again. */
  private readonly dead = new Set<string>();
  private readonly pending = { subscribe: new Set<string>(), unsubscribe: new Set<string>() };
  private flushQueued = false;
  private urlIndex = 0;
  /** The address this attempt is on, for what it says about the network. */
  private url = '';
  private attempt = 0;
  private heard = false;
  private retryTimer: Timer | undefined;
  private stallTimer: Timer | undefined;
  private watchTimer: ReturnType<typeof setInterval> | undefined;
  /** When this socket last carried anything at all. */
  private lastHeard = 0;
  private lingerTimer: Timer | undefined;
  private beatTimer: ReturnType<typeof setInterval> | undefined;
  private openTimer: Timer | undefined;
  private wakeTimer: Timer | undefined;

  constructor(private readonly proto: Protocol) {
    onWake(() => this.wake());
  }

  topic(spec: StreamSpec, venue: Venue): string {
    return this.proto.topic(spec, venue);
  }

  add(topic: string, listener: StreamListener, quiet = false): () => void {
    clearTimeout(this.lingerTimer);
    let set = this.topics.get(topic);
    if (!set) {
      set = new Set();
      this.topics.set(topic, set);
      if (quiet) this.quiet.add(topic);
      this.queue('subscribe', topic);
    }
    set.add(listener);
    listener.state(this.current, isRelayed(this.url));
    if (!this.ws && this.retryTimer === undefined) this.connect();
    return () => this.remove(topic, listener);
  }

  private remove(topic: string, listener: StreamListener): void {
    const set = this.topics.get(topic);
    if (!set || !set.delete(listener) || set.size > 0) return;
    this.topics.delete(topic);
    this.quiet.delete(topic);
    this.queue('unsubscribe', topic);
    if (this.topics.size === 0) this.lingerTimer = setTimeout(() => this.shutdown(), LINGER_MS);
  }

  /** Coalesces the changes made in one task into as few frames as possible. */
  private queue(op: 'subscribe' | 'unsubscribe', topic: string): void {
    const opposite = op === 'subscribe' ? this.pending.unsubscribe : this.pending.subscribe;
    if (opposite.delete(topic)) return;
    this.pending[op].add(topic);
    if (this.flushQueued) return;
    this.flushQueued = true;
    queueMicrotask(() => {
      this.flushQueued = false;
      const ws = this.ws;
      // Not open yet: onopen subscribes everything wanted at that moment.
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      for (const kind of ['unsubscribe', 'subscribe'] as const) {
        const list = [...this.pending[kind]];
        this.pending[kind].clear();
        if (list.length > 0) this.sendFrames(ws, kind, list);
      }
    });
  }

  private sendFrames(ws: WebSocket, op: 'subscribe' | 'unsubscribe', topics: string[]): void {
    const wanted = op === 'subscribe' ? topics.filter((t) => !this.dead.has(t)) : topics;
    if (wanted.length === 0) return;
    for (const frame of this.proto.frames(op, wanted)) ws.send(JSON.stringify(frame));
  }

  /**
   * One unknown symbol must not take nine good topics down with it: drop the
   * one the exchange named and re-send the rest a topic at a time. A frame of
   * one is already as narrow as it gets, so it is never retried.
   */
  private retryWithout(ws: WebSocket, bad: string, batch: string[]): void {
    if (bad !== '') this.dead.add(bad);
    if (batch.length <= 1) return;
    for (const topic of batch) if (topic !== bad && this.topics.has(topic)) this.sendFrames(ws, 'subscribe', [topic]);
  }

  private hear(): void {
    this.heard = true;
    this.attempt = 0;
    // It answers after all, whatever it did before.
    reached(this.url);
    reached(hostOf(this.url));
    this.setState('live', true);
  }

  /**
   * The page is back in view or on another network. A retry waiting out its
   * back-off goes now; an open socket is asked to prove itself, and one that
   * stays silent is replaced rather than left to the half-minute watch.
   */
  private wake(): void {
    if (this.topics.size === 0) return;
    const ws = this.ws;
    if (!ws) {
      if (this.retryTimer !== undefined) {
        clearTimeout(this.retryTimer);
        this.connect();
      }
      return;
    }
    if (ws.readyState !== WebSocket.OPEN) return;
    const since = Date.now();
    this.beat(ws);
    clearTimeout(this.wakeTimer);
    this.wakeTimer = setTimeout(() => {
      if (this.ws !== ws || this.lastHeard >= since) return;
      this.setState('stalled');
      ws.close();
    }, WAKE_MS);
  }

  private beat(ws: WebSocket): void {
    const beat = this.proto.heartbeat;
    if (beat === undefined || ws.readyState !== WebSocket.OPEN) return;
    ws.send(typeof beat === 'string' ? beat : JSON.stringify(beat));
  }

  private setState(state: SocketState, always = false): void {
    if (state === this.current && !always) return;
    this.current = state;
    const relayed = isRelayed(this.url);
    for (const set of this.topics.values()) for (const listener of set) listener.state(state, relayed);
  }

  private connect(): void {
    this.retryTimer = undefined;
    this.url = nextUrl(this.proto.urls, this.urlIndex);
    const ws = new WebSocket(this.url);
    this.ws = ws;
    this.heard = false;
    this.lastHeard = Date.now();
    const relayed = isRelayed(this.url);
    this.openTimer = setTimeout(() => {
      if (ws.readyState !== WebSocket.CONNECTING) return;
      // Neither open nor refused: the network is swallowing this host, so
      // every path on it is stepped over, not only this one. Not this site's
      // own, though: the page came from it.
      if (!relayed) block(hostOf(this.url));
      ws.close();
    }, relayed ? RELAY_OPEN_MS : OPEN_MS);

    ws.onopen = () => {
      clearTimeout(this.openTimer);
      this.pending.subscribe.clear();
      this.pending.unsubscribe.clear();
      for (const frame of this.proto.open ?? []) ws.send(JSON.stringify(frame));
      this.sendFrames(ws, 'subscribe', [...this.topics.keys()]);
      this.stallTimer = setTimeout(() => {
        if (this.heard) return;
        // Connected and subscribed, yet silent. Say so rather than show a
        // frozen price as live, and try the next endpoint if there is one.
        this.setState('stalled');
        if (this.proto.urls.length > 1) ws.close();
      }, STALL_MS);
      // Open and carrying nothing is not a state a socket reports, so it is
      // watched for: gone quiet, it is closed, and closing is what reconnects.
      this.watchTimer = setInterval(() => {
        if (Date.now() - this.lastHeard < SILENT_MS) return;
        this.setState('stalled');
        ws.close();
      }, WATCH_MS);
      if (this.proto.heartbeat !== undefined) this.beatTimer = setInterval(() => this.beat(ws), HEARTBEAT_MS);
    };

    ws.onmessage = (event) => {
      this.lastHeard = Date.now();
      let msg: unknown;
      try {
        msg = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (!msg || typeof msg !== 'object') return;
      const refused = this.proto.refused?.(msg as Json);
      if (refused) {
        this.retryWithout(ws, refused.bad, refused.batch);
        return;
      }
      if (!this.heard && this.quiet.size === this.topics.size) this.hear();
      this.proto.read(msg as Json, (topic, m) => {
        if (!this.heard) this.hear();
        const set = this.topics.get(topic);
        if (set) for (const listener of set) listener.message(m);
      });
    };

    ws.onclose = () => {
      this.clearTimers();
      if (this.ws === ws) this.ws = null;
      if (this.topics.size === 0) return;
      let moving = false;
      if (!this.heard) {
        // It opened nothing, or opened and said nothing: the next socket on
        // this network starts past it rather than waiting it out again.
        block(this.url);
        this.urlIndex += 1;
        // Another address still untried is gone to at once; with every one
        // failed (the phone offline, say), the back-off paces the rounds.
        moving = this.proto.urls.some((url) => !isBlocked(url));
      }
      this.attempt += 1;
      this.setState('retrying');
      this.retryTimer = setTimeout(() => this.connect(), moving ? MOVE_MS : RETRY_MS[Math.min(this.attempt, RETRY_MS.length) - 1]);
    };
    ws.onerror = () => ws.close();
  }

  private clearTimers(): void {
    clearTimeout(this.stallTimer);
    clearTimeout(this.openTimer);
    clearTimeout(this.wakeTimer);
    clearInterval(this.beatTimer);
    clearInterval(this.watchTimer);
    this.stallTimer = undefined;
    this.openTimer = undefined;
    this.wakeTimer = undefined;
    this.beatTimer = undefined;
    this.watchTimer = undefined;
  }

  private shutdown(): void {
    if (this.topics.size > 0) return;
    clearTimeout(this.retryTimer);
    this.retryTimer = undefined;
    this.clearTimers();
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onopen = null;
      ws.onclose = null;
      ws.onerror = null;
      ws.onmessage = null;
      ws.close();
    }
    this.pending.subscribe.clear();
    this.pending.unsubscribe.clear();
    this.attempt = 0;
    this.current = 'connecting';
  }
}

export class StreamHub {
  private readonly connections = new Map<string, Connection>();
  private extra: Awaited<ReturnType<typeof loadMore>> | null = null;
  private poller: InstanceType<Awaited<ReturnType<typeof loadMore>>['Poller']> | null = null;
  private indexPoller: InstanceType<Awaited<ReturnType<typeof loadMore>>['Poller']> | null = null;

  subscribe(venue: Venue, spec: StreamSpec, listener: StreamListener): () => void {
    const socket = socketOf(venue, spec);
    if (!PROTOCOLS[socket] && !this.extra) {
      // Loaded once, then this subscription goes ahead — unless dropped meanwhile.
      let off: (() => void) | null = null;
      let dropped = false;
      listener.state('connecting', false);
      void loadMore().then((m) => {
        this.extra ??= m;
        if (!dropped) off = this.subscribe(venue, spec, listener);
      });
      return () => {
        dropped = true;
        off?.();
      };
    }
    if (venue === 'lighter-perp' && spec.kind === 'kline' && this.extra) {
      this.poller ??= this.extra.lighterPoller();
      return this.poller.add(spec.symbol, this.extra.lighterResolution(spec.interval), listener);
    }
    // An index is the server's own: its bars are asked for, and it has nothing else to stream.
    if (venue === 'index' && this.extra) {
      if (spec.kind !== 'kline') {
        listener.state('stalled', false);
        return () => {};
      }
      this.indexPoller ??= this.extra.indexPoller();
      return this.indexPoller.add(spec.symbol, spec.interval, listener);
    }
    let conn = this.connections.get(socket);
    if (!conn) {
      const make = PROTOCOLS[socket] ?? this.extra?.PROTOCOLS[socket];
      if (!make) return () => {};
      conn = new Connection(make());
      this.connections.set(socket, conn);
    }
    const topic = conn.topic(spec, venue);
    // This exchange does not stream it: say so rather than wait forever.
    if (topic === '') {
      listener.state('stalled', false);
      return () => {};
    }
    return conn.add(topic, listener, spec.kind === 'liquidations' || (spec.kind === 'kline' && spec.quiet === true));
  }
}
