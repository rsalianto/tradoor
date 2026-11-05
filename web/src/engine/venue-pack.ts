/**
 * The newer exchanges — Gate, Aster, HTX, Bitfinex, Deribit, Coinbase, Bitstamp: their
 * sockets and their candle loaders, in a file of their own, fetched the first
 * time a market on one of them is opened. The engine's worker carries its lazy
 * imports inline (see vite.config), so anywhere else this would be in every
 * visitor's first download. It is built as one self-contained file that imports
 * nothing back, which also keeps it clear of the Safari trouble noted there:
 * every import below is of types, and it hands itself over on `globalThis`
 * because the build keeps no exports for a file built this way.
 */
import type { MarketRef, Venue } from '../lib/markets';
import type { Range, Row } from './history';
import type { Json, Protocol, StreamMsg } from './streams';

const num = (v: unknown): number => (typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : Number.NaN);

const relay = (socket: string): string[] => (typeof location === 'undefined' ? [] : [`${location.origin.replace(/^http/, 'ws')}/api/relay/ws?venue=${socket}`]);

const iso = (ms: number): string => new Date(ms).toISOString();

/** A forming bar: these exchanges' candle streams carry no comparable trade id, so each is taken whole. */
const bar = (openTime: number, open: unknown, high: unknown, low: unknown, close: unknown, volume: number, closed = false): StreamMsg => ({ kind: 'kline', openTime, open: num(open), high: num(high), low: num(low), close: num(close), volume, lastTradeId: 0, closed });

/**
 * Each exchange's name for a bar length, and only the lengths whose bars start
 * where the chart's do. HTX's days start at 16:00 UTC and Deribit's at 08:00, so
 * those are left out here and in `intervals`, and folded from shorter bars.
 */
const HTX_NAMES: Record<string, string> = { '1m': '1min', '5m': '5min', '15m': '15min', '30m': '30min', '1h': '60min', '4h': '4hour' };
const BITFINEX_NAMES: Record<string, string> = { '1d': '1D', '1w': '1W' };
const DERIBIT_NAMES: Record<string, string> = { '1m': '1', '5m': '5', '15m': '15', '30m': '30', '1h': '60', '2h': '120' };
const COINBASE_NAMES: Record<string, string> = { '1m': '60', '5m': '300', '15m': '900', '1h': '3600', '6h': '21600', '1d': '86400' };
const BITSTAMP_NAMES: Record<string, string> = { '1m': '60', '5m': '300', '15m': '900', '30m': '1800', '1h': '3600', '2h': '7200', '4h': '14400', '6h': '21600', '12h': '43200', '1d': '86400' };

/** Gate's spot week is `7d`, its perpetuals' `1w`: its perpetuals' `7d` starts on a Thursday. */
const gateName = (venue: Venue, interval: string): string => (venue === 'gate-spot' && interval === '1w' ? '7d' : interval);

const GATE_TIDY_FROM = Date.UTC(2023, 0, 1);

const nextAt = new Map<string, number>();
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * The least time between two requests to an exchange that counts them tightly,
 * since a chart's history asks a dozen pages at once: Coinbase takes ten public
 * requests a second, and HTX answers a few together with "request limit".
 */
async function spaced(exchange: string, ms: number): Promise<void> {
  const at = Math.max(Date.now(), nextAt.get(exchange) ?? 0);
  nextAt.set(exchange, at + ms);
  await wait(at - Date.now());
}

type Emit = (topic: string, msg: StreamMsg) => void;

/** An exchange's answer as JSON, through the relay where the exchange answers no browser or this network blocks it. */
const relayed = (url: string): Promise<unknown> => fetch(`/api/relay/rest?url=${encodeURIComponent(url)}`).then((res) => res.json());
const direct = (url: string): Promise<unknown> => fetch(url).then((res) => res.json(), () => relayed(url));

const DAY_MS = 86_400_000;

/**
 * The UTC day, for an exchange whose own days start elsewhere: folded from its
 * hours, the ones before this asked for once and the one under way from the
 * stream. It is the page's day feed that asks, for a watchlist's change and
 * volume since 00:00 UTC; a chart folds its own. Hours are read on the first
 * frame after subscribing, which is what carries somewhere to send the day.
 */
function folder(load: (symbol: string) => Promise<Row[]>) {
  /** Each followed market's hours by time; null until they are read. */
  const days = new Map<string, Map<number, Row> | null>();
  const send = (symbol: string, emit: Emit) => {
    const start = Math.floor(Date.now() / DAY_MS) * DAY_MS;
    const hours = [...(days.get(symbol)?.values() ?? [])].filter((r) => r[0]! >= start).sort((a, b) => a[0]! - b[0]!);
    if (hours.length === 0) return;
    emit(`day|${symbol}`, bar(start, hours[0]![1], Math.max(...hours.map((r) => r[2]!)), Math.min(...hours.map((r) => r[3]!)), hours[hours.length - 1]![4], hours.reduce((sum, r) => sum + r[5]!, 0)));
  };
  return {
    want: (symbol: string) => void days.set(symbol, null),
    drop: (symbol: string) => void days.delete(symbol),
    heard(emit: Emit) {
      for (const [symbol, hours] of days) {
        if (hours) continue;
        days.set(symbol, new Map());
        load(symbol).then(
          (rows) => {
            const held = days.get(symbol);
            if (!held) return;
            for (const r of rows) if (!held.has(r[0]!)) held.set(r[0]!, r);
            send(symbol, emit);
          },
          // Not read: tried again on a frame a little later.
          () => void setTimeout(() => days.has(symbol) && days.set(symbol, null), 10_000),
        );
      }
    },
    hour(symbol: string, row: Row, emit: Emit) {
      const held = days.get(symbol);
      if (!held) return;
      held.set(row[0]!, row);
      send(symbol, emit);
    },
  };
}

/** Gate. A ping carries the time; one without it is dropped, and so is the socket. */
function gate(urls: string[], side: 'spot' | 'futures'): Protocol {
  const now = () => Math.floor(Date.now() / 1000);
  return {
    urls,
    quiet: true,
    topic: (spec, venue) => (spec.kind === 'kline' ? `${side}.candlesticks|${gateName(venue, spec.interval)}|${spec.symbol}` : ''),
    frames: (event, topics) =>
      topics.map((t) => {
        const [channel, ...payload] = t.split('|');
        return { time: now(), channel, event, payload };
      }),
    read(m, emit) {
      if (m.event !== 'update' || m.channel !== `${side}.candlesticks`) return;
      for (const r of (Array.isArray(m.result) ? m.result : [m.result]) as Json[]) {
        const name = String(r.n);
        const at = name.indexOf('_');
        // `a`: spot counts coins here, a perpetual the quote's turnover.
        const close = num(r.c);
        emit(`${side}.candlesticks|${name.slice(0, at)}|${name.slice(at + 1)}`, bar(num(r.t) * 1000, r.o, r.h, r.l, close, side === 'spot' ? num(r.a) : num(r.a) / close, r.w === true));
      }
    },
    get heartbeat() {
      return { time: now(), channel: `${side}.ping` };
    },
  };
}

/** Aster: Binance's stream, except that nothing is pushed for a quiet market, so the subscriptions are asked for to show the socket is up. */
function aster(urls: string[]): Protocol {
  let id = 0;
  return {
    urls,
    quiet: true,
    topic: (spec) => (spec.kind === 'kline' ? `${spec.symbol.toLowerCase()}@kline_${spec.interval}` : ''),
    frames: (op, topics) => [{ method: op.toUpperCase(), params: topics, id: ++id }],
    read(m, emit) {
      if (m.e !== 'kline' || !m.k) return;
      const k = m.k as Json;
      emit(`${String(m.s).toLowerCase()}@kline_${k.i}`, { ...bar(num(k.t), k.o, k.h, k.l, k.c, num(k.v), k.x === true), buy: num(k.V) } as StreamMsg);
    },
    heartbeat: { method: 'LIST_SUBSCRIPTIONS', id: 0 },
  };
}

async function gunzip(data: Blob): Promise<string> {
  try {
    return await new Response(data.stream().pipeThrough(new DecompressionStream('gzip'))).text();
  } catch {
    return '';
  }
}

const htxRow = (k: Json): Row => [num(k.id) * 1000, num(k.open), num(k.high), num(k.low), num(k.close), num(k.amount)];

/**
 * HTX: every frame gzipped, and a ping every few seconds that must be answered.
 * Frames are unpacked one after another, so bars cannot overtake each other.
 * A kline's `amount` is coins on both sides.
 */
function htx(urls: string[], hours: (symbol: string) => string): Protocol {
  let tail: Promise<unknown> = Promise.resolve();
  const days = folder(async (symbol) => ((await direct(hours(symbol))) as { data?: Json[] }).data?.map(htxRow) ?? []);
  const HOURS = /^market\.(.+)\.kline\.60min$/;
  return {
    urls,
    quiet: true,
    topic: (spec) => (spec.kind !== 'kline' ? '' : spec.interval === '1d' ? `day|${spec.symbol}` : `market.${spec.symbol}.kline.${HTX_NAMES[spec.interval] ?? spec.interval}`),
    frames: (op, topics) =>
      topics.map((t) => {
        const symbol = t.startsWith('day|') ? t.slice(4) : null;
        if (symbol) op === 'subscribe' ? days.want(symbol) : days.drop(symbol);
        const ch = symbol ? `market.${symbol}.kline.60min` : t;
        return { [op === 'subscribe' ? 'sub' : 'unsub']: ch, id: ch };
      }),
    reply: (m) => (m.ping ? { pong: m.ping } : null),
    inflate(data) {
      const text = tail.then(() => gunzip(data));
      tail = text;
      return text;
    },
    read(m, emit) {
      days.heard(emit);
      const ch = m.ch;
      const t = m.tick as Json | undefined;
      if (typeof ch !== 'string' || !t) return;
      const row = htxRow(t);
      const hour = HOURS.exec(ch);
      if (hour) days.hour(hour[1]!, row, emit);
      emit(ch, bar(row[0]!, row[1], row[2], row[3], row[4], row[5]!));
    },
  };
}

/**
 * Bitfinex numbers a subscription when it answers, and sends only that number
 * after: `[chanId, row]`, `[chanId, [rows, newest first]]` on subscribing, and
 * `[chanId, "hb"]` to show the channel is alive. A candle is `[time, open,
 * close, high, low, volume]`.
 */
function bitfinex(urls: string[]): Protocol {
  const topics = new Map<number, string>();
  const channels = new Map<string, number>();
  return {
    urls,
    quiet: true,
    topic: (spec) => (spec.kind === 'kline' ? `trade:${BITFINEX_NAMES[spec.interval] ?? spec.interval}:${spec.symbol}` : ''),
    frames: (op, list) =>
      list.flatMap((key): unknown[] => {
        if (op === 'subscribe') return [{ event: 'subscribe', channel: 'candles', key }];
        const chanId = channels.get(key);
        return chanId === undefined ? [] : [{ event: 'unsubscribe', chanId }];
      }),
    read(m, emit) {
      if (!Array.isArray(m)) {
        if (m.event === 'subscribed') {
          topics.set(m.chanId as number, String(m.key));
          channels.set(String(m.key), m.chanId as number);
        }
        return;
      }
      const [chan, body] = m as unknown as [number, unknown];
      const topic = topics.get(chan);
      if (!topic || !Array.isArray(body)) return;
      const k = (Array.isArray(body[0]) ? body[0] : body) as number[];
      if (k.length >= 6) emit(topic, bar(k[0]!, k[1], k[3], k[4], k[2], k[5]!));
    },
  };
}

const deribitChart = (instrument: string, from: number, to: number, resolution: string): string => `https://www.deribit.com/api/v2/public/get_tradingview_chart_data?instrument_name=${instrument}&start_timestamp=${from}&end_timestamp=${to}&resolution=${resolution}`;

interface DeribitChart {
  result?: { ticks?: number[]; open: number[]; high: number[]; low: number[]; close: number[]; volume: number[] };
  error?: { message?: string };
}

const deribitRows = ({ result: r }: DeribitChart): Row[] => (r?.ticks ?? []).map((t, i) => [t, r!.open[i]!, r!.high[i]!, r!.low[i]!, r!.close[i]!, r!.volume[i]!]);

/** Deribit: JSON-RPC with channels by name, and any answered call as a heartbeat. */
function deribit(urls: string[]): Protocol {
  const days = folder(async (symbol) => {
    const now = Date.now();
    return deribitRows((await relayed(deribitChart(symbol, Math.floor(now / DAY_MS) * DAY_MS, now, '60'))) as DeribitChart);
  });
  const HOURS = /^chart\.trades\.(.+)\.60$/;
  return {
    urls,
    quiet: true,
    topic: (spec) => (spec.kind !== 'kline' ? '' : spec.interval === '1d' ? `day|${spec.symbol}` : `chart.trades.${spec.symbol}.${DERIBIT_NAMES[spec.interval] ?? spec.interval}`),
    frames: (op, topics) => [
      {
        jsonrpc: '2.0',
        id: 1,
        method: `public/${op}`,
        params: {
          channels: topics.map((t) => {
            const symbol = t.startsWith('day|') ? t.slice(4) : null;
            if (symbol) op === 'subscribe' ? days.want(symbol) : days.drop(symbol);
            return symbol ? `chart.trades.${symbol}.60` : t;
          }),
        },
      },
    ],
    read(m, emit) {
      days.heard(emit);
      if (m.method !== 'subscription') return;
      const { channel, data } = m.params as { channel: string; data: Json };
      if (!channel.startsWith('chart.')) return;
      const row: Row = [num(data.tick), num(data.open), num(data.high), num(data.low), num(data.close), num(data.volume)];
      const hour = HOURS.exec(channel);
      if (hour) days.hour(hour[1]!, row, emit);
      emit(channel, bar(row[0]!, row[1], row[2], row[3], row[4], row[5]!));
    },
    heartbeat: { jsonrpc: '2.0', id: 0, method: 'public/test' },
  };
}

/** How often the last bars are asked for between an exchange's trades. */
const BARS_MS = 5_000;

/**
 * For an exchange that streams no candles, only every trade. The chart folds
 * those into its bar, as it does Binance's trades, but a bar starts only from a
 * candle, so a quiet market would stop for good. Its last two come from REST
 * every few seconds, and being older than any trade they correct the bar without
 * rewinding it (see LiveFeed). The socket's frames keep the time, so nothing is
 * asked for once the socket is gone.
 */
function barAsker(charts: Set<string>, load: (topic: string) => Promise<Row[]>) {
  let asked = 0;
  let asking = false;
  const askAll = async (emit: Emit) => {
    for (const topic of [...charts]) {
      const list = await load(topic).catch((): Row[] => []);
      if (charts.has(topic)) for (const r of list) emit(topic, bar(r[0]!, r[1], r[2], r[3], r[4], r[5]!));
    }
  };
  return (emit: Emit) => {
    if (asking || Date.now() - asked < BARS_MS) return;
    asked = Date.now();
    asking = true;
    void askAll(emit).finally(() => (asking = false));
  };
}

/** `[time, low, high, open, close, volume]`, newest first. */
const coinbaseRows = (list: number[][]): Row[] => list.map((k) => [k[0]! * 1000, k[3]!, k[2]!, k[1]!, k[4]!, k[5]!]).reverse();

/**
 * Coinbase: every match, and the last bars asked for (see barAsker). One
 * product's heartbeat, a frame a second, keeps a quiet socket from looking dead
 * and is what the asking keeps time by.
 */
function coinbase(urls: string[]): Protocol {
  const charts = new Set<string>();
  const productOf = (topic: string) => topic.split('|')[1];
  const ask = barAsker(charts, async (topic) => {
    const [, product, granularity] = topic.split('|');
    await spaced('coinbase', 110);
    const now = Date.now();
    const list = (await direct(`https://api.exchange.coinbase.com/products/${product}/candles?granularity=${granularity}&start=${iso(now - 2_000 * Number(granularity))}&end=${iso(now)}`)) as number[][];
    return Array.isArray(list) ? coinbaseRows(list.slice(0, 2)) : [];
  });
  return {
    urls,
    quiet: true,
    topic: (spec) => (spec.kind === 'kline' ? `candles|${spec.symbol}|${COINBASE_NAMES[spec.interval] ?? spec.interval}` : ''),
    frames: (type, topics) =>
      topics.flatMap((t) => {
        const product = productOf(t);
        if (type === 'subscribe') charts.add(t);
        else charts.delete(t);
        // Two charts on a product share its matches: the last one out ends them.
        if (type === 'unsubscribe' && [...charts].some((c) => productOf(c) === product)) return [];
        return [{ type, product_ids: [product], channels: ['matches'] }];
      }),
    read(m, emit) {
      if (m.type === 'match') {
        for (const topic of charts) if (productOf(topic) === m.product_id) emit(topic, { kind: 'trade', time: Date.parse(String(m.time)), price: num(m.price), qty: num(m.size), lastTradeId: num(m.trade_id) });
      } else if (m.type === 'heartbeat') ask(emit);
    },
    open: [{ type: 'subscribe', product_ids: ['BTC-USD'], channels: ['heartbeat'] }],
  };
}

const bitstampBars = (pair: string, step: string, query: string): string => `https://www.bitstamp.net/api/v2/ohlc/${pair}/?step=${step}&${query}`;

function bitstampRows(body: unknown): Row[] {
  const list = (body as { data?: { ohlc?: Json[] } }).data?.ohlc;
  if (!list) throw new Error('Bitstamp refused the candle request');
  return list.map((k) => [num(k.timestamp) * 1000, num(k.open), num(k.high), num(k.low), num(k.close), num(k.volume)]);
}

/** The market whose trades keep time on a Bitstamp socket, whichever charts are open. */
const BITSTAMP_CLOCK = 'btcusd';
const LIVE_TRADES = 'live_trades_';

/**
 * Bitstamp: every trade, and the last bars asked for (see barAsker). Asking for
 * a channel twice sends its trades once. A trade's id is too long for a number
 * on perpetuals, and all the chart wants of one is that it is newer than a bar
 * from REST, which carries 0. Spot's `timestamp` is in seconds and a perpetual's
 * in milliseconds, so the time is read from `microtimestamp`, the same on both.
 */
function bitstamp(urls: string[]): Protocol {
  const charts = new Set<string>();
  const pairOf = (topic: string) => topic.split('|')[1]!;
  const ask = barAsker(charts, async (topic) => {
    const [, pair, step] = topic.split('|');
    await spaced('bitstamp', 100);
    return bitstampRows(await direct(bitstampBars(pair!, step!, 'limit=2')));
  });
  const frame = (op: string, pair: string) => ({ event: `bts:${op}`, data: { channel: LIVE_TRADES + pair } });
  return {
    urls,
    quiet: true,
    topic: (spec) => (spec.kind === 'kline' ? `ohlc|${spec.symbol}|${BITSTAMP_NAMES[spec.interval] ?? spec.interval}` : ''),
    frames: (op, topics) =>
      topics.flatMap((t) => {
        const pair = pairOf(t);
        if (op === 'subscribe') charts.add(t);
        else charts.delete(t);
        if (op === 'unsubscribe' && (pair === BITSTAMP_CLOCK || [...charts].some((c) => pairOf(c) === pair))) return [];
        return [frame(op, pair)];
      }),
    read(m, emit) {
      if (m.event === 'trade') {
        const d = m.data as Json;
        const pair = String(m.channel).slice(LIVE_TRADES.length);
        for (const topic of charts) if (pairOf(topic) === pair) emit(topic, { kind: 'trade', time: Math.floor(num(d.microtimestamp) / 1000), price: num(d.price), qty: num(d.amount), lastTradeId: 1 });
      }
      ask(emit);
    },
    open: [frame('subscribe', BITSTAMP_CLOCK)],
    heartbeat: { event: 'bts:heartbeat' },
  };
}

export const PROTOCOLS: Record<string, () => Protocol> = {
  'gate-spot': () => gate(['wss://api.gateio.ws/ws/v4/', ...relay('gate-spot')], 'spot'),
  'gate-perp': () => gate(['wss://fx-ws.gateio.ws/v4/ws/usdt', ...relay('gate-perp')], 'futures'),
  'aster-spot': () => aster(['wss://sstream.asterdex.com/ws', ...relay('aster-spot')]),
  'aster-perp': () => aster(['wss://fstream.asterdex.com/ws', ...relay('aster-perp')]),
  'htx-spot': () => htx(['wss://api.huobi.pro/ws', ...relay('htx-spot')], (s) => `https://api.huobi.pro/market/history/kline?symbol=${s}&period=60min&size=25`),
  'htx-perp': () => htx(['wss://api.hbdm.com/linear-swap-ws', ...relay('htx-perp')], (s) => `https://api.hbdm.com/linear-swap-ex/market/history/kline?contract_code=${s}&period=60min&size=25`),
  'bitfinex-spot': () => bitfinex(['wss://api-pub.bitfinex.com/ws/2', ...relay('bitfinex-spot')]),
  'bitfinex-perp': () => bitfinex(['wss://api-pub.bitfinex.com/ws/2', ...relay('bitfinex-perp')]),
  'deribit-perp': () => deribit(['wss://www.deribit.com/ws/api/v2', ...relay('deribit-perp')]),
  'coinbase-spot': () => coinbase(['wss://ws-feed.exchange.coinbase.com', ...relay('coinbase-spot')]),
  'bitstamp-spot': () => bitstamp(['wss://ws.bitstamp.net', ...relay('bitstamp-spot')]),
  'bitstamp-perp': () => bitstamp(['wss://ws.bitstamp.net', ...relay('bitstamp-perp')]),
};

const inRange = (list: Row[], range: Range): Row[] => list.filter((r) => (range.start === undefined || r[0]! >= range.start) && (range.end === undefined || r[0]! <= range.end));

/**
 * Bars from an exchange that is asked by time, a page's span at most (`read`).
 * A thin market has fewer bars than its span has minutes and a short page reads
 * as the start of its history, so asked from the end alone, this looks further
 * back before coming up short.
 */
async function byTime(range: Range, limit: number, ms: number, read: (from: number, to: number) => Promise<Row[]>): Promise<Row[]> {
  const end = range.end ?? Date.now();
  if (range.start !== undefined) return read(Math.max(range.start, end - limit * ms, 0), end);
  const span = Math.max(limit, 200) * ms;
  let found: Row[] = [];
  let to = end;
  for (let i = 0; i < 6 && found.length < limit && to > 0; i++) {
    found = [...(await read(Math.max(to - span, 0), to)), ...found];
    to -= span + 1;
  }
  return found.slice(-limit);
}

type Ask = (url: string) => Promise<unknown>;

/** HTX's bars. Its "request limit" comes with a 200, so nothing but this waits it out. */
async function htxBars(url: string, getJson: Ask): Promise<Row[]> {
  for (let attempt = 0; ; attempt += 1) {
    await spaced('htx', 120);
    const body = (await getJson(url)) as { data?: Json[]; 'err-msg'?: string };
    if (body.data) return body.data.map(htxRow);
    if (body['err-msg'] !== 'request limit' || attempt >= 3) throw new Error(body['err-msg'] || 'HTX refused the candle request');
    await wait(1000);
  }
}

/** `[time, open, high, low, close, volume]` rows in the range, oldest first, in coins (`ms`: the bar's length). */
export async function rows(ref: MarketRef, native: string, interval: string, range: Range, limit: number, getJson: Ask, ms: number): Promise<Row[]> {
  const end = range.end ?? Date.now();
  // A long bar's pages run back past 1970 soon enough, and some of these refuse that.
  if (end <= 0) return [];
  const sec = (t: number) => Math.floor(t / 1000);
  switch (ref.venue) {
    case 'gate-spot':
    case 'gate-perp':
      return byTime(range, limit, ms, async (from, to) => {
        // It keeps the last ten thousand bars of a length, and refuses to look further.
        const oldest = Date.now() - 9_990 * ms;
        if (to < oldest) return [];
        const query = `interval=${gateName(ref.venue, interval)}&from=${sec(Math.max(from, oldest, 0))}&to=${sec(to)}`;
        if (ref.venue === 'gate-spot') {
          // `[time, quote volume, close, high, low, open, coin volume, closed]`
          const body = (await getJson(`https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair=${native}&${query}`)) as string[][];
          // Its weekly bars before 2023 run to ten decimals, and a chart reads its decimals off its bars.
          const price = (t: number, v: unknown) => (interval === '1w' && t < GATE_TIDY_FROM ? Number(num(v).toPrecision(5)) : num(v));
          return inRange(body.map((r) => { const t = num(r[0]) * 1000; return [t, price(t, r[5]), price(t, r[3]), price(t, r[4]), price(t, r[2]), num(r[6])]; }), { start: from, end: to });
        }
        // `v` is contracts; `sum`, the turnover, over the close is the coins near enough.
        const body = (await getJson(`https://api.gateio.ws/api/v4/futures/usdt/candlesticks?contract=${native}&${query}`)) as Json[];
        return inRange(body.map((k) => [num(k.t) * 1000, num(k.o), num(k.h), num(k.l), num(k.c), num(k.sum) / num(k.c)]), { start: from, end: to });
      });
    case 'aster-spot':
    case 'aster-perp': {
      let url = ref.venue === 'aster-perp' ? 'https://fapi.asterdex.com/fapi/v1/klines' : 'https://sapi.asterdex.com/api/v1/klines';
      url += `?symbol=${native}&interval=${interval}&limit=${limit}`;
      if (range.start !== undefined) url += `&startTime=${range.start}`;
      if (range.end !== undefined) url += `&endTime=${range.end}`;
      // Binance's rows, the takers' volume with them.
      return ((await getJson(url)) as unknown[][]).map((r) => [num(r[0]), num(r[1]), num(r[2]), num(r[3]), num(r[4]), num(r[5]), num(r[9])]);
    }
    case 'htx-spot': {
      // Only the latest two thousand are served, newest first: paging back past them
      // finds nothing. All of them unless this is only the forming bar being asked for.
      const size = range.end === undefined ? Math.min(2000, limit) : 2000;
      const bars = await htxBars(`https://api.huobi.pro/market/history/kline?symbol=${native}&period=${HTX_NAMES[interval] ?? interval}&size=${size}`, getJson);
      return inRange(bars.reverse(), range).slice(-limit);
    }
    case 'htx-perp':
      return byTime(range, limit, ms, async (from, to) => inRange(await htxBars(`https://api.hbdm.com/linear-swap-ex/market/history/kline?contract_code=${native}&period=${HTX_NAMES[interval] ?? interval}&from=${sec(from)}&to=${sec(to)}`, getJson), { start: from, end: to }));
    case 'bitfinex-spot':
    case 'bitfinex-perp': {
      // Newest first, whatever the range. Through the relay, which is why few pages matter: see limits.
      let url = `https://api-pub.bitfinex.com/v2/candles/trade:${BITFINEX_NAMES[interval] ?? interval}:${native}/hist?limit=${limit}&end=${end}`;
      if (range.start !== undefined) url += `&start=${range.start}`;
      const body = (await getJson(url)) as number[][];
      return body.map((k) => [k[0]!, k[1]!, k[3]!, k[4]!, k[2]!, k[5]!]).reverse();
    }
    case 'deribit-perp': {
      const from = Math.max(range.start ?? 0, end - limit * ms);
      const body = (await getJson(deribitChart(native, from, end, DERIBIT_NAMES[interval] ?? interval))) as DeribitChart;
      if (!body.result) throw new Error(body.error?.message || 'Deribit refused the candle request');
      return inRange(deribitRows(body), range);
    }
    case 'coinbase-spot':
      return byTime(range, limit, ms, async (from, to) => {
        await spaced('coinbase', 110);
        // A minute nobody traded in has no row.
        const url = `https://api.exchange.coinbase.com/products/${native}/candles?granularity=${COINBASE_NAMES[interval] ?? interval}&start=${iso(from)}&end=${iso(to)}`;
        return inRange(coinbaseRows((await getJson(url)) as number[][]), { start: from, end: to });
      });
    case 'bitstamp-spot':
    case 'bitstamp-perp':
      // The last `limit` bars up to `end`, oldest first, whatever a start says. A
      // minute nobody traded in, or a closed market's, has a bar of its own.
      return inRange(bitstampRows(await getJson(bitstampBars(native, BITSTAMP_NAMES[interval] ?? interval, `limit=${limit}&end=${sec(end)}`))), range);
    default:
      throw new Error(`no candle loader for ${ref.venue}`);
  }
}

export interface VenuePack {
  PROTOCOLS: typeof PROTOCOLS;
  rows: typeof rows;
}

(globalThis as { tradoorVenues?: VenuePack }).tradoorVenues = { PROTOCOLS, rows };
