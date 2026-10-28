/**
 * Stream protocols of the exchanges beyond Binance and Bybit, loaded with the
 * first market on one of them so the rest of the app does not carry them.
 */
import type { Venue } from '../lib/markets';
import { INTERVALS } from './intervals';
import { chunks, num, relay, type Json, type Protocol, type StreamListener, type TickerMsg } from './streams';

const ticker = (last: number, open: number | null, quoteVolume: number | null): TickerMsg => ({
  kind: 'ticker',
  last,
  changePct: open !== null && open > 0 ? ((last - open) / open) * 100 : null,
  quoteVolume,
});

const intervalName = (venue: Venue, interval: string): string => INTERVALS[venue]?.[interval] ?? interval;

/**
 * OKX puts candles on its `business` socket and tickers on `public`; spot and
 * swaps share both. Rows are `[ts, o, h, l, c, vol, volCcy, volCcyQuote,
 * confirm]`, and a swap's `vol` is contracts, so its `volCcy` is taken.
 */
function okx(urls: string[], channel: 'candle' | 'tickers'): Protocol {
  return {
    urls,
    topic: (spec, venue) => {
      if (channel === 'candle') return spec.kind === 'kline' ? `candle${intervalName(venue, spec.interval)}|${spec.symbol}` : '';
      return spec.kind === 'ticker' ? `tickers|${spec.symbol}` : '';
    },
    frames: (op, topics) => chunks(topics, 20).map((batch) => ({ op, args: batch.map((t) => ({ channel: t.split('|')[0], instId: t.split('|')[1] })) })),
    read(m, emit) {
      const arg = m.arg as Json | undefined;
      if (!arg || !Array.isArray(m.data)) return;
      const ch = String(arg.channel);
      const inst = String(arg.instId);
      if (ch.startsWith('candle')) {
        const swap = inst.endsWith('-SWAP');
        for (const r of m.data as string[][]) {
          emit(`${ch}|${inst}`, { kind: 'kline', openTime: num(r[0]), open: num(r[1]), high: num(r[2]), low: num(r[3]), close: num(r[4]), volume: num(r[swap ? 6 : 5]), lastTradeId: 0, closed: r[8] === '1' });
        }
      } else if (ch === 'tickers') {
        const d = (m.data as Json[])[0];
        if (!d) return;
        const last = num(d.last);
        emit(`tickers|${inst}`, ticker(last, num(d.open24h), inst.endsWith('-SWAP') ? num(d.volCcy24h) * last : num(d.volCcy24h)));
      }
    },
    heartbeat: 'ping',
  };
}

/** Bitget's v2 public socket carries spot and futures, told apart by `instType`. */
function bitget(urls: string[]): Protocol {
  const kind = (venue: Venue) => (venue === 'bitget-perp' ? 'USDT-FUTURES' : 'SPOT');
  return {
    urls,
    topic: (spec, venue) => {
      if (spec.kind === 'kline') return `${kind(venue)}|candle${INTERVALS['bitget-perp']![spec.interval] ?? spec.interval}|${spec.symbol}`;
      return spec.kind === 'ticker' ? `${kind(venue)}|ticker|${spec.symbol}` : '';
    },
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
      if (String(arg.channel).startsWith('candle')) {
        for (const r of m.data as string[][]) emit(topic, { kind: 'kline', openTime: num(r[0]), open: num(r[1]), high: num(r[2]), low: num(r[3]), close: num(r[4]), volume: num(r[5]), lastTradeId: 0, closed: false });
      } else if (arg.channel === 'ticker') {
        const d = (m.data as Json[])[0];
        if (!d) return;
        const pct = num(d.change24h);
        emit(topic, { kind: 'ticker', last: num(d.lastPr), changePct: Number.isFinite(pct) ? pct * 100 : null, quoteVolume: num(d.quoteVolume) });
      }
    },
    heartbeat: 'ping',
  };
}

/** Hyperliquid: a subscription a frame; tickers come from the asset's context. */
function hyperliquid(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec, venue) => (spec.kind === 'kline' ? `candle|${spec.symbol}|${intervalName(venue, spec.interval)}` : spec.kind === 'ticker' ? `ctx|${spec.symbol}` : ''),
    frames: (method, topics) =>
      topics.map((t) => {
        const [type, coin, interval] = t.split('|');
        return { method, subscription: type === 'candle' ? { type: 'candle', coin, interval } : { type: 'activeAssetCtx', coin } };
      }),
    read(m, emit) {
      const d = m.data as Json | undefined;
      if (!d) return;
      if (m.channel === 'candle') {
        emit(`candle|${d.s}|${d.i}`, { kind: 'kline', openTime: num(d.t), open: num(d.o), high: num(d.h), low: num(d.l), close: num(d.c), volume: num(d.v), lastTradeId: 0, closed: false });
      } else if (m.channel === 'activeAssetCtx') {
        const ctx = d.ctx as Json;
        emit(`ctx|${d.coin}`, ticker(num(ctx.markPx), num(ctx.prevDayPx), num(ctx.dayNtlVlm)));
      }
    },
    heartbeat: { method: 'ping' },
  };
}

/** Kraken spot, v2: bars start at `interval_begin`. */
function krakenSpot(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec, venue) => (spec.kind === 'kline' ? `ohlc|${spec.symbol}|${intervalName(venue, spec.interval)}` : spec.kind === 'ticker' ? `ticker|${spec.symbol}` : ''),
    frames: (method, topics) =>
      topics.map((t) => {
        const [channel, symbol, interval] = t.split('|');
        return { method, params: channel === 'ohlc' ? { channel, symbol: [symbol], interval: Number(interval) } : { channel, symbol: [symbol] } };
      }),
    read(m, emit) {
      if (!Array.isArray(m.data)) return;
      for (const d of m.data as Json[]) {
        if (m.channel === 'ohlc') {
          emit(`ohlc|${d.symbol}|${d.interval}`, { kind: 'kline', openTime: Date.parse(String(d.interval_begin)), open: num(d.open), high: num(d.high), low: num(d.low), close: num(d.close), volume: num(d.volume), lastTradeId: 0, closed: false });
        } else if (m.channel === 'ticker') {
          emit(`ticker|${d.symbol}`, { kind: 'ticker', last: num(d.last), changePct: num(d.change_pct), quoteVolume: num(d.volume) * num(d.vwap) });
        }
      }
    },
    heartbeat: { method: 'ping' },
  };
}

/** Kraken Futures: candle feeds by length, and a ticker; a heartbeat feed keeps it open. */
function krakenFutures(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec, venue) => (spec.kind === 'kline' ? `candles_trade_${intervalName(venue, spec.interval)}|${spec.symbol}` : spec.kind === 'ticker' ? `ticker|${spec.symbol}` : ''),
    frames: (event, topics) =>
      topics.map((t) => {
        const [feed, product] = t.split('|');
        return { event, feed, product_ids: [product] };
      }),
    read(m, emit) {
      const feed = typeof m.feed === 'string' ? m.feed : '';
      if (feed.startsWith('candles_trade_') && m.candle && typeof m.candle === 'object') {
        const c = m.candle as Json;
        emit(`${feed.replace(/_snapshot$/, '')}|${m.product_id}`, { kind: 'kline', openTime: num(c.time), open: num(c.open), high: num(c.high), low: num(c.low), close: num(c.close), volume: num(c.volume), lastTradeId: 0, closed: false });
      } else if (feed === 'ticker') {
        const change = m.change ?? m.change24h;
        emit(`ticker|${m.product_id}`, { kind: 'ticker', last: num(m.last), changePct: change === undefined ? null : num(change), quoteVolume: m.volumeQuote === undefined ? null : num(m.volumeQuote) });
      }
    },
    open: [{ event: 'subscribe', feed: 'heartbeat' }],
  };
}

/** Lighter streams market stats; its bars are polled (see `Poller`). */
function lighter(urls: string[]): Protocol {
  return {
    urls,
    topic: (spec) => (spec.kind === 'ticker' ? `market_stats/${spec.symbol}` : ''),
    frames: (type, topics) => topics.map((channel) => ({ type, channel })),
    read(m, emit) {
      const channel = typeof m.channel === 'string' ? m.channel : '';
      const s = m.market_stats as Json | undefined;
      if (!channel.startsWith('market_stats:') || !s) return;
      emit(`market_stats/${channel.slice('market_stats:'.length)}`, { kind: 'ticker', last: num(s.last_trade_price), changePct: num(s.daily_price_change), quoteVolume: num(s.daily_quote_token_volume) });
    },
  };
}

export const PROTOCOLS: Record<string, () => Protocol> = {
  'okx-business': () => okx(['wss://ws.okx.com/ws/v5/business', ...relay('okx-business')], 'candle'),
  'okx-public': () => okx(['wss://ws.okx.com/ws/v5/public', ...relay('okx-public')], 'tickers'),
  bitget: () => bitget(['wss://ws.bitget.com/v2/ws/public', ...relay('bitget')]),
  hyperliquid: () => hyperliquid(['wss://api.hyperliquid.xyz/ws', ...relay('hyperliquid')]),
  'kraken-spot': () => krakenSpot(['wss://ws.kraken.com/v2', ...relay('kraken-spot')]),
  'kraken-perp': () => krakenFutures(['wss://futures.kraken.com/ws/v1', ...relay('kraken-perp')]),
  lighter: () => lighter(['wss://mainnet.zklighter.elliot.ai/stream', ...relay('lighter')]),
};

export const lighterResolution = (interval: string): string => INTERVALS['lighter-perp']?.[interval] ?? interval;

export const lighterPoller = (): Poller => new Poller(lighterRead, 3_000);
/** The server reads its sources every minute or so; a quarter of that is soon enough. */
export const indexPoller = (): Poller => new Poller(indexRead, 15_000);

/** A bar as a poll reads it: `[open time, open, high, low, close, volume]`. */
type Bar = [number, number, number, number, number, number];
/** Each asked-for key's last bars, the forming one last, by `id|resolution`. */
type Read = (keys: string[]) => Promise<Map<string, Bar[]>>;

/** Lighter: a market at a time, as its API asks; one that fails is left out. */
const lighterRead: Read = async (keys) => {
  const now = Date.now();
  const got = await Promise.allSettled(
    keys.map(async (key): Promise<[string, Bar[]]> => {
      const [id, resolution] = key.split('|');
      const res = await fetch(`https://mainnet.zklighter.elliot.ai/api/v1/candles?market_id=${id}&resolution=${resolution}&start_timestamp=${now - 3 * 86_400_000}&end_timestamp=${now}&count_back=2`);
      const body = (await res.json()) as { c?: { t: number; o: number; h: number; l: number; c: number; v: number }[] };
      return [key, (body.c ?? []).slice(-2).map((k) => [k.t, k.o, k.h, k.l, k.c, k.v])];
    }),
  );
  return new Map(got.flatMap((r) => (r.status === 'fulfilled' ? [r.value] : [])));
};

/** The indices this server works out: every one of a length in one request. */
const indexRead: Read = async (keys) => {
  const byLength = new Map<string, string[]>();
  for (const key of keys) {
    const [symbol, interval] = key.split('|') as [string, string];
    byLength.set(interval, [...(byLength.get(interval) ?? []), symbol]);
  }
  const out = new Map<string, Bar[]>();
  await Promise.all(
    [...byLength].map(async ([interval, symbols]) => {
      const res = await fetch(`/api/index/latest?interval=${interval}&symbols=${symbols.join(',')}`);
      if (!res.ok) return;
      for (const [symbol, bars] of Object.entries((await res.json()) as Record<string, Bar[]>)) out.set(`${symbol}|${interval}`, bars);
    }),
  );
  return out;
};

/**
 * Bars for a venue with no candle stream — Lighter, and the indices this
 * server works out: the forming bar asked for every few seconds, every market
 * and length on one timer, however many listen.
 */
export class Poller {
  private readonly topics = new Map<string, Set<StreamListener>>();
  private timer: ReturnType<typeof setInterval> | undefined;
  /** Keys added since the last poll: asked for at once, those added together in one go. */
  private fresh: string[] = [];

  constructor(
    private readonly read: Read,
    private readonly every: number,
  ) {}

  add(id: string, resolution: string, listener: StreamListener): () => void {
    const key = `${id}|${resolution}`;
    let listeners = this.topics.get(key);
    if (!listeners) {
      listeners = new Set();
      this.topics.set(key, listeners);
      if (this.fresh.push(key) === 1) setTimeout(() => void this.poll(this.fresh.splice(0)), 0);
    }
    listeners.add(listener);
    listener.state('connecting', false);
    this.timer ??= setInterval(() => void this.poll([...this.topics.keys()]), this.every);
    return () => {
      const held = this.topics.get(key);
      if (!held || !held.delete(listener) || held.size > 0) return;
      this.topics.delete(key);
      if (this.topics.size > 0) return;
      clearInterval(this.timer);
      this.timer = undefined;
    };
  }

  private async poll(keys: string[]): Promise<void> {
    const got = await this.read(keys).catch(() => new Map<string, Bar[]>());
    for (const key of keys) {
      const listeners = this.topics.get(key);
      const rows = got.get(key);
      if (!listeners) continue;
      for (const l of listeners) l.state(rows ? 'live' : 'retrying', false);
      for (const k of rows ?? []) for (const l of listeners) l.message({ kind: 'kline', openTime: k[0], open: k[1], high: k[2], low: k[3], close: k[4], volume: k[5], lastTradeId: 0, closed: false });
    }
  }
}

