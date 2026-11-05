/**
 * Candle history for one market at one interval, shared by every chart that
 * reads it.
 *
 * Stored partitions come first: immutable files under /data that the browser
 * caches for a year. The exchange's REST API then fills whatever is newer than
 * the last stored period — and everything, for a market nobody archived. Only
 * closed bars are held; the forming one comes from the stream.
 */
import { decode_partition } from '../wasm/pkg/tradoor_wasm.js';
import { parseMarket, type MarketRef, type Venue } from '../lib/markets';
import { nativeOf } from '../lib/native';
import { INTERVALS, lacksUtc, marketInterval } from './intervals';
import { block, isBlocked, reached } from './reach';

export interface Cols {
  time: Float64Array;
  open: Float64Array;
  high: Float64Array;
  low: Float64Array;
  close: Float64Array;
  volume: Float64Array;
  /** What market buyers took, in coins; -1 where the exchange does not say. */
  buy: Float64Array;
}

const KEYS = ['time', 'open', 'high', 'low', 'close', 'volume', 'buy'] as const;

export function emptyCols(): Cols {
  const e = () => new Float64Array(0);
  return { time: e(), open: e(), high: e(), low: e(), close: e(), volume: e(), buy: e() };
}

/** Whether two runs of bars are the same bars, to the last figure. */
export function sameCols(a: Cols, b: Cols): boolean {
  if (a.time.length !== b.time.length) return false;
  for (const key of KEYS) {
    const x = a[key];
    const y = b[key];
    for (let i = 0; i < x.length; i++) if (x[i] !== y[i]) return false;
  }
  return true;
}

export function concatCols(parts: readonly Cols[]): Cols {
  if (parts.length === 1) return parts[0]!;
  const n = parts.reduce((sum, p) => sum + p.time.length, 0);
  const out = emptyCols();
  for (const key of KEYS) {
    const column = new Float64Array(n);
    let offset = 0;
    for (const p of parts) {
      column.set(p[key], offset);
      offset += p[key].length;
    }
    out[key] = column;
  }
  return out;
}

export function sliceCols(c: Cols, from: number, to: number = c.time.length): Cols {
  const out = emptyCols();
  for (const key of KEYS) out[key] = c[key].subarray(from, to);
  return out;
}

/** First index whose time is at or after `t`. */
export function lowerBound(times: Float64Array, t: number): number {
  let lo = 0;
  let hi = times.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (times[mid]! < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Older bars followed by newer ones; where they overlap the newer copy wins. */
export function joinCols(older: Cols, newer: Cols): Cols {
  if (newer.time.length === 0) return older;
  if (older.time.length === 0) return newer;
  return concatCols([sliceCols(older, 0, lowerBound(older.time, newer.time[0]!)), newer]);
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

const FIXED: Record<string, number> = {
  '1m': MINUTE, '5m': 5 * MINUTE, '15m': 15 * MINUTE, '30m': 30 * MINUTE,
  '1h': HOUR, '2h': 2 * HOUR, '4h': 4 * HOUR, '6h': 6 * HOUR, '12h': 12 * HOUR,
  '1d': DAY, '1w': 7 * DAY,
};

/** When the bar that opened at `open` stops trading. Months read the calendar. */
export function intervalEnd(interval: string, open: number): number {
  const ms = FIXED[interval];
  if (ms !== undefined) return open + ms;
  const d = new Date(open);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1);
}

export const intervalMs = (interval: string): number => FIXED[interval] ?? 30 * DAY;

// ---------------------------------------------------------------- exchange REST

const BINANCE_SPOT = ['https://api.binance.com', 'https://data-api.binance.vision'];
let binanceSpotHost = 0;

const BYBIT_INTERVAL: Record<string, string> = {
  '1m': '1', '5m': '5', '15m': '15', '30m': '30',
  '1h': '60', '2h': '120', '4h': '240', '6h': '360', '12h': '720',
  '1d': 'D', '1w': 'W', '1M': 'M',
};

/** Bars one request returns at most, per venue. */
const PAGE_LIMIT: Record<Venue, number> = {
  'binance-spot': 1000,
  'binance-perp': 1500,
  'bybit-spot': 1000,
  'bybit-perp': 1000,
  'okx-spot': 100,
  'okx-perp': 100,
  'bitget-spot': 200,
  'bitget-perp': 200,
  'hyperliquid-perp': 5000,
  'kraken-spot': 720,
  'kraken-perp': 2000,
  'lighter-perp': 500,
  index: 2000,
};

const pageLimit = (ref: MarketRef): number => PAGE_LIMIT[ref.venue];


const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * Exchange hosts this browser could not reach go through the tradoor server
 * instead, which can still reach them. Some networks block the exchanges'
 * names — a phone on an Indonesian carrier — and what one call learns, the
 * rest of the page and its next load are told (see reach).
 */
// Kraken Futures answers with an empty Access-Control-Allow-Origin, which no browser accepts.
const RELAYED = new Set(['futures.kraken.com']);

/**
 * A request that has not begun to answer by now is taken as refused: a
 * network that swallows an exchange's requests rather than turn them away
 * would otherwise keep a chart empty for a minute or two.
 */
const ANSWER_MS = 7_000;
/**
 * And a whole answer, body and all, not in by now is given up, to be asked
 * for again: a series fetches one thing at a time, so one request left
 * hanging — begun before a phone slept, say — held up every refresh behind it.
 */
const WHOLE_MS = 30_000;

/**
 * The least time between two requests to a host that counts them tightly:
 * Bitget allows twenty candle requests a second, and a chart's history asked
 * a dozen pages at once, round after round — half of them answered 429, each
 * then waited out a second or more.
 */
const SPACING_MS: Record<string, number> = { 'api.bitget.com': 70 };
const nextAt = new Map<string, number>();

async function fetchExchange(url: string): Promise<Response> {
  const host = new URL(url).host;
  const spacing = SPACING_MS[host];
  if (spacing) {
    const now = Date.now();
    const at = Math.max(now, nextAt.get(host) ?? 0);
    nextAt.set(host, at + spacing);
    if (at > now) await sleep(at - now);
  }
  if (!RELAYED.has(host) && !isBlocked(host)) {
    const abort = new AbortController();
    const started = Date.now();
    const timer = setTimeout(() => abort.abort(), ANSWER_MS);
    try {
      const res = await fetch(url, { signal: abort.signal });
      clearTimeout(timer);
      // Begun: the body has what is left of the whole answer's time.
      setTimeout(() => abort.abort(), WHOLE_MS - (Date.now() - started));
      reached(host);
      return res;
    } catch (e) {
      clearTimeout(timer);
      // A TypeError is the network refusing, an abort the network saying
      // nothing; neither is the exchange answering.
      if (!(e instanceof TypeError) && !(e instanceof DOMException && e.name === 'AbortError')) throw e;
      block(host);
    }
  }
  const abort = new AbortController();
  setTimeout(() => abort.abort(), WHOLE_MS);
  return fetch(`/api/relay/rest?url=${encodeURIComponent(url)}`, { signal: abort.signal });
}

async function getJson(url: string): Promise<unknown> {
  for (let attempt = 0; ; attempt += 1) {
    const res = await fetchExchange(url);
    if (res.ok) return res.json();
    const retryable = res.status === 429 || res.status === 418 || res.status >= 500;
    if (!retryable || attempt >= 3) throw new Error(`${new URL(url).host} answered ${res.status}`);
    await sleep(1000 * 2 ** attempt);
  }
}

/** api.binance.com is unreachable from some networks; its market-data mirror is not. */
async function binanceSpot(query: string): Promise<unknown> {
  for (let i = 0; ; i += 1) {
    const index = (binanceSpotHost + i) % BINANCE_SPOT.length;
    try {
      const body = await getJson(BINANCE_SPOT[index]! + query);
      binanceSpotHost = index;
      return body;
    } catch (e) {
      if (!(e instanceof TypeError) || i >= BINANCE_SPOT.length - 1) throw e;
    }
  }
}

export type Row = number[];
export type Range = { start?: number; end?: number };

interface Page {
  cols: Cols;
  /** Rows the exchange sent in the range, before the forming bar was dropped. */
  raw: number;
}

/** Klines opening inside the range, oldest first, closed bars only. */
async function klines(ref: MarketRef, interval: string, range: Range, limit = pageLimit(ref)): Promise<Page> {
  const rows = await rowsIn(ref, interval, range, limit);
  const now = Date.now();
  const closed = rows.filter((r) => Number.isFinite(r[4]) && intervalEnd(interval, r[0]!) <= now);
  const cols = emptyCols();
  for (const [i, key] of KEYS.entries()) cols[key] = Float64Array.from(closed, (r) => r[i] ?? -1);
  return { cols, raw: rows.length };
}

/**
 * The bars of `interval` from `start` until now, the forming one with them —
 * what a chart draws its forming bar from until its stream's first word. Asked
 * for as few rows as that takes: the exchanges weigh a request by its limit.
 */
export async function formingRows(market: string, interval: string, start: number): Promise<Row[]> {
  const ref = parseMarket(market);
  if (!ref) return [];
  const wanted = Math.ceil((Date.now() - start) / intervalMs(interval)) + 1;
  const rows = await rowsIn(ref, interval, { start }, Math.min(pageLimit(ref), Math.max(2, wanted)));
  return rows.filter((r) => r[0]! >= start && r.slice(1, 5).every(Number.isFinite));
}

/** The exchange's klines opening inside the range, oldest first, the forming one included. */
async function rowsIn(ref: MarketRef, interval: string, range: Range, limit: number): Promise<Row[]> {
  let rows: Row[];
  if (ref.venue.startsWith('binance')) {
    const path = ref.venue === 'binance-perp' ? '/fapi/v1/klines' : '/api/v3/klines';
    let query = `${path}?symbol=${ref.symbol}&interval=${interval}&limit=${limit}`;
    if (range.start !== undefined) query += `&startTime=${range.start}`;
    if (range.end !== undefined) query += `&endTime=${range.end}`;
    const body = ref.venue === 'binance-perp' ? await getJson(`https://fapi.binance.com${query}`) : await binanceSpot(query);
    if (!Array.isArray(body)) throw new Error('Binance sent an unexpected kline response');
    rows = (body as unknown[][]).map((r) => [Number(r[0]), Number(r[1]), Number(r[2]), Number(r[3]), Number(r[4]), Number(r[5]), Number(r[9])]);
  } else if (ref.venue.startsWith('bybit')) {
    const category = ref.venue === 'bybit-perp' ? 'linear' : 'spot';
    let url = `https://api.bybit.com/v5/market/kline?category=${category}&symbol=${ref.symbol}&interval=${BYBIT_INTERVAL[interval] ?? interval}&limit=${limit}`;
    if (range.start !== undefined) url += `&start=${range.start}`;
    if (range.end !== undefined) url += `&end=${range.end}`;
    const body = (await getJson(url)) as { retCode?: number; retMsg?: string; result?: { list?: string[][] } };
    if (body.retCode !== 0) throw new Error(body.retMsg || 'Bybit refused the kline request');
    // Newest first on Bybit.
    rows = (body.result?.list ?? []).map((r) => r.slice(0, 6).map(Number)).reverse();
  } else {
    // The other exchanges' loaders come with the first market on one of them.
    const { venueRows } = await import('./history-venues');
    rows = await venueRows(ref, await nativeOf(`${ref.venue}:${ref.symbol}`), interval, range, limit, getJson);
  }
  return rows;
}

// ---------------------------------------------------------------- stored partitions

interface PartitionInfo {
  period: string;
  /** Its contents' hash: files are cached for a year, so a rebuilt one needs a new address. */
  sha256?: string;
  rows: number;
  firstOpen: number;
  lastOpen: number;
}

interface Manifest {
  market: string;
  updatedAt: number;
  bases: Record<string, PartitionInfo[] | undefined>;
}

/** A manifest changes whenever the recorder writes a period; re-read it now and then. */
const MANIFEST_TTL_MS = 10 * MINUTE;
const manifests = new Map<string, { at: number; value: Promise<Manifest | null> }>();

function manifest(ref: MarketRef): Promise<Manifest | null> {
  const key = `${ref.venue}/${ref.symbol}`;
  const hit = manifests.get(key);
  if (hit && Date.now() - hit.at < MANIFEST_TTL_MS) return hit.value;
  // Anything but a manifest — a 404, a proxy error, an HTML fallback page —
  // means the market is not archived here, not that the chart is broken.
  const value = fetch(`/data/${key}/manifest.json`, { cache: 'no-cache' })
    .then((r) => (r.ok ? (r.json() as Promise<Manifest>) : null))
    .catch(() => null);
  manifests.set(key, { at: Date.now(), value });
  return value;
}

async function fetchPartition(ref: MarketRef, base: string, { period, sha256 }: PartitionInfo): Promise<Cols> {
  const res = await fetch(`/data/${ref.venue}/${ref.symbol}/${base}/${period}.tdc.gz?v=${sha256?.slice(0, 12) ?? ''}`);
  if (!res.ok) throw new Error(`history file ${base}/${period} answered ${res.status}`);
  let bytes = new Uint8Array(await res.arrayBuffer());
  // Still gzip unless a server in between set Content-Encoding and the browser inflated it.
  if (bytes[0] === 0x1f && bytes[1] === 0x8b) {
    const inflated = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip'));
    bytes = new Uint8Array(await new Response(inflated).arrayBuffer());
  }
  return decode_partition(bytes) as unknown as Cols;
}

/** Oldest first, skipping any file that overlaps one before it. */
function disjoint(list: readonly PartitionInfo[]): PartitionInfo[] {
  const out: PartitionInfo[] = [];
  for (const p of [...list].sort((a, b) => a.firstOpen - b.firstOpen)) {
    if (out.length === 0 || p.firstOpen > out[out.length - 1]!.lastOpen) out.push(p);
  }
  return out;
}

// ---------------------------------------------------------------- series

/** `ratio`: one market priced in another, its two sides each archive or exchange. */
export type SeriesSource = 'archive' | 'exchange' | 'ratio';

/** What a chart reads its candles from: one market's series, or one market
 *  priced in another (see ratio.ts). */
export interface CandleSeries {
  readonly cols: Cols;
  readonly exhausted: boolean;
  readonly interval: string;
  readonly source: SeriesSource;
  subscribe(listener: () => void): () => void;
  ensure(bars: number): Promise<void>;
  older(): Promise<boolean>;
  refreshTail(force?: boolean): Promise<void>;
}

/** Base bars fetched per scroll past the oldest one held. */
const OLDER_BARS = 5_000;
/** Enough to draw a chart with at once: a screenful, and EMA 200 warmed. */
const FIRST_BARS = 600;
/** Intervals of a fixed length, whose pages can be worked out in advance. */
const EXACT_INTERVALS = new Set(['1m', '3m', '5m', '15m', '30m', '1h', '2h', '4h', '6h', '8h', '12h', '1d', '3d', '1w']);
const MAX_TAIL_PAGES = 12;
/** Between the chunks of history loaded behind a chart's first screen. */
const BACKFILL_PAUSE_MS = 500;
const TAIL_THROTTLE_MS = 30_000;
/** A series nobody reads is dropped after this long — a deep 1m history is large. */
/** How long a series with no chart on it is kept: long enough that going back
 *  to a market, or to a tab left open, costs nothing. */
const EVICT_MS = 15 * MINUTE;

const registry = new Map<string, Series>();

export class Series {
  cols: Cols = emptyCols();
  exhausted = false;
  private readonly listeners = new Set<() => void>();
  private chain: Promise<unknown> = Promise.resolve();
  private tailAt = 0;
  private evictTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly key: string,
    readonly ref: MarketRef,
    readonly interval: string,
    readonly source: SeriesSource,
    /** Stored files not loaded yet, oldest first. */
    private readonly partitions: PartitionInfo[],
  ) {}

  subscribe(listener: () => void): () => void {
    clearTimeout(this.evictTimer);
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
      if (this.listeners.size > 0) return;
      clearTimeout(this.evictTimer);
      this.evictTimer = setTimeout(() => {
        if (this.listeners.size === 0 && registry.get(this.key) === this) registry.delete(this.key);
      }, EVICT_MS);
    };
  }

  private emit(): void {
    for (const listener of [...this.listeners]) listener();
  }

  /** One load at a time, so two charts on one market cannot fetch a file twice. */
  private run<T>(task: () => Promise<T>): Promise<T> {
    const next = this.chain.then(task);
    this.chain = next.catch(() => undefined);
    return next;
  }

  /**
   * Hold at least `bars` of the newest bars, or all there are — resolving as
   * soon as there is enough to draw with. A chart shows a screenful and warms
   * an EMA on a few hundred bars; the rest is for scrolling back, and arrives
   * behind it. Waiting for all of it means waiting for every page in turn.
   */
  ensure(bars: number): Promise<void> {
    const first = Math.min(bars, FIRST_BARS);
    const ready = this.run(async () => {
      const held = this.cols.time.length > 0;
      await this.fill(first);
      // Held from before — by a chart once shown, a call, a row looked at —
      // its newest bars may have closed since. Drawn from those, a chart
      // jumped to the bar forming now, the ones between missing until the
      // next one closed: an hour on an hour's bars. Nothing closed, nothing
      // is asked; the exchange not answering leaves what is held.
      if (held) {
        const before = this.cols.time.length;
        await this.tail().catch(() => undefined);
        if (this.cols.time.length !== before) this.emit();
      }
    });
    // The rest a chunk at a time, each queued anew behind whatever was asked
    // meanwhile: in one piece, a second chart on these bars waited on all of
    // it for its first screen — sixty of Bitget's 200-bar pages, twenty
    // seconds. A pause between chunks keeps under the exchanges' rates.
    const more = async () => {
      if (this.cols.time.length >= bars || this.exhausted) return;
      const before = this.cols.time.length;
      if ((await this.olderChunk(bars - before)) === 0) this.exhausted = true;
      if (this.cols.time.length !== before) this.emit();
      if (this.cols.time.length < bars && !this.exhausted) setTimeout(() => void this.run(more), BACKFILL_PAUSE_MS);
    };
    // The first chunk waits too, so another chart's first screen asked in the
    // same moment — the Values panel's, a second chart's — goes ahead of it.
    if (bars > first) setTimeout(() => void this.run(more), BACKFILL_PAUSE_MS);
    return ready;
  }

  private async fill(bars: number): Promise<void> {
    if (this.cols.time.length === 0 && this.source === 'archive') {
      // The stored files and the bars since them are fetched together: the
      // manifest already says where the newest file ends.
      const newest = this.partitions.length > 0 ? this.partitions[this.partitions.length - 1]!.lastOpen : 0;
      const [, tail] = await Promise.all([this.loadPartitions(bars), this.tailFrom(intervalEnd(this.interval, newest))]);
      const held = this.cols.time;
      const from = held.length > 0 ? intervalEnd(this.interval, held[held.length - 1]!) : 0;
      this.cols = joinCols(this.cols, sliceCols(tail, lowerBound(tail.time, from)));
    }
    while (this.cols.time.length < bars && !this.exhausted) {
      if ((await this.olderChunk(bars - this.cols.time.length)) === 0) this.exhausted = true;
    }
  }

  /** Load bars older than the oldest held. Resolves false once there are none. */
  older(): Promise<boolean> {
    return this.run(async () => {
      if (this.exhausted) return false;
      const added = await this.olderChunk(OLDER_BARS);
      if (added === 0) this.exhausted = true;
      else this.emit();
      return added > 0;
    });
  }

  /** Append bars that closed since the newest held. */
  /** Bars newer than the last one held. `force` skips the throttle, for a
   *  chart confirming a bar it has just watched close. */
  refreshTail(force = false): Promise<void> {
    if (!force && Date.now() - this.tailAt < TAIL_THROTTLE_MS) return Promise.resolve();
    this.tailAt = Date.now();
    return this.run(async () => {
      const before = this.cols.time.length;
      await this.tail();
      if (this.cols.time.length !== before) this.emit();
    });
  }

  private async olderChunk(want: number): Promise<number> {
    if (this.source === 'archive' && this.partitions.length > 0) return this.loadPartitions(want);
    // Before the first stored file, or a market with none: page back through the exchange.
    return this.pageBack(want);
  }

  private async loadPartitions(want: number): Promise<number> {
    let take = 0;
    let rows = 0;
    while (take < this.partitions.length && (rows < want || take === 0)) {
      rows += this.partitions[this.partitions.length - 1 - take]!.rows;
      take += 1;
    }
    const chosen = this.partitions.slice(this.partitions.length - take);
    const parts = await Promise.all(chosen.map((p) => fetchPartition(this.ref, this.interval, p)));
    this.partitions.splice(this.partitions.length - take, take);
    const loaded = concatCols(parts);
    this.cols = joinCols(loaded, this.cols);
    return loaded.time.length;
  }

  private async pageBack(want: number): Promise<number> {
    const limit = pageLimit(this.ref);
    const end0 = this.cols.time.length > 0 ? this.cols.time[0]! - 1 : Date.now();
    // Bars are evenly spaced, so where each page ends is known before any of
    // them answers: asking in turn would be one round trip per page, and an
    // exchange that gives a hundred at a time would take fifteen of them.
    const parallel = await this.pagesAtOnce(want, limit, end0);
    if (parallel > 0) return parallel;

    const pages: Cols[] = [];
    let added = 0;
    let end = end0;
    while (added < want) {
      // No more than is wanted, and the forming bar: a chart's first page is
      // a screenful, and a full page is several times the bytes to wait on.
      const ask = Math.min(limit, want - added + 1);
      const page = await klines(this.ref, this.interval, { end }, ask);
      if (page.cols.time.length > 0) {
        pages.unshift(page.cols);
        added += page.cols.time.length;
        end = page.cols.time[0]! - 1;
      }
      if (page.raw < ask) {
        this.exhausted = true;
        break;
      }
    }
    if (pages.length > 0) this.cols = joinCols(concatCols(pages), this.cols);
    return added;
  }

  /**
   * The pages a `want` needs, fetched together. Zero when that cannot be
   * counted on — a month's bars are not a fixed length, and one page is no
   * faster in parallel — and the caller walks them in turn instead.
   */
  private async pagesAtOnce(want: number, limit: number, end: number): Promise<number> {
    const step = EXACT_INTERVALS.has(this.interval) ? intervalMs(this.interval) : 0;
    const count = Math.ceil(want / limit);
    if (step <= 0 || count < 2) return 0;
    const windows = Array.from({ length: Math.min(count, MAX_TAIL_PAGES) }, (_, k) => ({ start: end - (k + 1) * limit * step, end: end - k * limit * step }));
    const pages = await Promise.all(windows.map((w) => klines(this.ref, this.interval, w)));
    const cols = pages.map((p) => p.cols).filter((c) => c.time.length > 0).sort((a, b) => a.time[0]! - b.time[0]!);
    if (cols.length === 0) {
      this.exhausted = true;
      return 0;
    }
    const loaded = concatCols(cols);
    const before = this.cols.time.length;
    this.cols = joinCols(loaded, this.cols);
    // The oldest window coming back empty is the start of the market.
    if (pages[pages.length - 1]!.cols.time.length === 0) this.exhausted = true;
    return this.cols.time.length - before;
  }

  private async tail(): Promise<void> {
    const times = this.cols.time;
    if (times.length === 0) return;
    const from = intervalEnd(this.interval, times[times.length - 1]!);
    const tail = await this.tailFrom(from);
    this.cols = joinCols(this.cols, sliceCols(tail, lowerBound(tail.time, from)));
  }

  /** The bars that closed since `from`, without touching what is held. */
  private async tailFrom(from: number): Promise<Cols> {
    this.tailAt = Date.now();
    const now = Date.now();
    if (from <= 0 || intervalEnd(this.interval, from) > now) return emptyCols(); // the next bar has not closed yet
    const limit = pageLimit(this.ref);
    const pages: Cols[] = [];
    if ((now - from) / intervalMs(this.interval) < limit - 1) {
      pages.push((await klines(this.ref, this.interval, { start: from, end: now })).cols);
    } else {
      // A long gap — the recorder has not run for a while. Newest first, so the
      // chart is current even if the pages run out before the gap is closed.
      let end = now;
      for (let i = 0; i < MAX_TAIL_PAGES; i += 1) {
        const page = await klines(this.ref, this.interval, { end });
        if (page.cols.time.length === 0) break;
        pages.unshift(page.cols);
        if (page.cols.time[0]! <= from || page.raw < limit) break;
        end = page.cols.time[0]! - 1;
      }
    }
    return concatCols(pages);
  }
}

const probes = new Map<string, Promise<void>>();

/**
 * Whether a Bitget market takes its UTC day, week and month: asked once, of
 * one bar, whatever the chart's length. Its stock tokens answer 400, code
 * 48001 — and their live candles at those lengths say nothing — so theirs are
 * read and streamed shorter, and their quiet hours are not taken for a dead
 * socket. A probe that could not be made is forgotten, to be made again.
 */
function probeUtc(market: string, ref: MarketRef): Promise<void> {
  const name = INTERVALS[ref.venue]?.['1d'];
  if (!ref.venue.startsWith('bitget') || !name) return Promise.resolve();
  let probe = probes.get(market);
  if (!probe) {
    probe = (async () => {
      const native = await nativeOf(market);
      const path = ref.venue === 'bitget-perp' ? 'mix/market/candles?productType=usdt-futures&' : 'spot/market/candles?';
      const res = await fetchExchange(`https://api.bitget.com/api/v2/${path}symbol=${native}&granularity=${name}&limit=1`);
      const body = (await res.json().catch(() => null)) as { code?: string } | null;
      if (body?.code === '48001') lacksUtc(market);
    })().catch(() => void probes.delete(market));
    probes.set(market, probe);
  }
  return probe;
}

/**
 * The series behind a chart: stored partitions at `base` when the market has
 * them, the exchange at `interval` when it does not.
 */
export async function openSeries(market: string, base: string, interval: string): Promise<Series> {
  const ref = parseMarket(market);
  if (!ref) throw new Error(`not a market: ${market}`);
  const stored = (await manifest(ref))?.bases[base] ?? [];
  const source: SeriesSource = stored.length > 0 ? 'archive' : 'exchange';
  if (source === 'exchange') await probeUtc(market, ref);
  // A length the exchange lacks is fetched shorter and folded by the chart.
  const at = source === 'archive' ? base : marketInterval(market, ref.venue, interval);
  const key = `${market}|${at}`;
  let series = registry.get(key);
  if (!series) {
    series = new Series(key, ref, at, source, disjoint(stored));
    registry.set(key, series);
  }
  return series;
}
