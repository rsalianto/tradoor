/**
 * Footprints recorded in this browser for every market the server does not
 * record: while a chart shows a market, its trades come off the exchange's
 * socket (the one the DOM and tape share) into minutes, kept in this browser's
 * own database for the days the reader chose and loaded whole when a chart
 * asks. Nothing goes to the server.
 *
 * One tab records a market, whichever holds its lock; the others read what it
 * writes and hear of each write on a broadcast channel. Gaps — the hours no tab
 * was open — are filled from Binance's trade history where it is the market's
 * exchange, a capped number of calls a recording, newest first.
 */
import { createSignal } from 'solid-js';
import { digitsFor } from './alerts';
import { foldMinutes, recordedMarkets, type Footprint, type Minute, type Settings } from './footprint';
import { cover, gaps, levelFor, merge, MINUTE, MinuteBook, pack, readAggTrades, toMinute, trimSpans, unpack, type Levels, type Span } from './fp-minutes';
import { addMinutes, forget, prune, readMarket, readMinutes, writeMarket, type MarketRecord } from './fp-store';
import { isRatio, parseMarket } from './markets';
import { listenPrints } from './orderflow';

const DAY = 86_400_000;
const HOUR = 3_600_000;
/** How often finished minutes are written down, and the minute under way told to charts. */
const FLUSH_MS = 10_000;
/** A market no chart shows any more keeps recording this long, for a chart put back. */
const LINGER_MS = 60_000;
/** How far back a gap is filled from the exchange's history, and how many calls that may take. */
const BACKFILL_MS = 24 * HOUR;
const BACKFILL_CALLS = 120;
/**
 * Time between those calls: a futures call weighs 20 of the 2,400 a minute
 * Binance allows an address, a spot one 4 — paced, the charts' own calls keep room.
 */
const BACKFILL_PACE: Record<'binance-spot' | 'binance-perp', number> = { 'binance-spot': 100, 'binance-perp': 600 };
const SETTINGS_KEY = 'tradoor.footprint';

export interface RecorderSettings {
  /** Record in this browser at all. */
  on: boolean;
  /** Days kept: older minutes are let go. */
  days: number;
}

const DEFAULTS: RecorderSettings = { on: true, days: 7 };
export const KEEP_DAYS = [1, 3, 7, 14, 30] as const;

function loadSettings(): RecorderSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}') as Partial<RecorderSettings>;
    return { on: raw.on !== false, days: KEEP_DAYS.includes(raw.days as (typeof KEEP_DAYS)[number]) ? raw.days! : DEFAULTS.days };
  } catch {
    return { ...DEFAULTS };
  }
}

const [settings, setSettingsSignal] = createSignal<RecorderSettings>(loadSettings());
export const recorderSettings = settings;

export function setRecorderSettings(patch: Partial<RecorderSettings>): void {
  const was = settings();
  const next = { ...was, ...patch };
  setSettingsSignal(next);
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(next));
  } catch {
    // Kept for this page only.
  }
  if (next.on !== was.on) for (const market of wanted.keys()) restart(market);
  tidied = 0;
  void tidy();
}

/** Whether a market's trades can be recorded here: an exchange's, not an index or a ratio. */
export const recordable = (market: string): boolean => {
  const venue = parseMarket(market)?.venue;
  return !!venue && venue !== 'index' && !isRatio(market);
};

// ---------------------------------------------------------------- telling charts

/** `full`: older minutes changed too — a gap filled — not only the newest. */
type Listener = (full: boolean) => void;
const listeners = new Map<string, Set<Listener>>();
/** The minute under way, as its recorder last told it: charts show it before it is written down. */
const live = new Map<string, [number, Levels]>();
const channel = typeof BroadcastChannel === 'function' ? new BroadcastChannel('tradoor-footprint') : null;

function tell(market: string, full = false): void {
  for (const listener of listeners.get(market) ?? []) listener(full);
}

/** What a market's recorder is doing, for its charts to say so. */
export interface RecordStatus {
  /**
   * Filling the gaps of the last day from Binance: how much is done of how
   * much (ms), the oldest time reached, and the share of the calls it may make
   * used — a busy market runs out of calls before the day is in.
   */
  filling: { done: number; of: number; back: number; calls: number } | null;
  /** Recording live since then; 0 while the socket is not up. */
  live: number;
}
const [statuses, setStatuses] = createSignal(new Map<string, RecordStatus>());
/** The markets this tab records, whose status it answers for. */
const here = new Set<string>();
export const recordStatus = (market: string): RecordStatus | undefined => statuses().get(market);

function setStatus(market: string, status: RecordStatus | null, broadcast = true): void {
  const next = new Map(statuses());
  if (status) next.set(market, status);
  else next.delete(market);
  setStatuses(next);
  if (broadcast) channel?.postMessage({ market, status });
}

type Message = { market: string; live?: [number, ArrayBuffer] | null; full?: boolean; status?: RecordStatus | null; ask?: true };

channel?.addEventListener('message', (e: MessageEvent<Message>) => {
  const { market, live: minute, full } = e.data;
  // A tab just opened on a market asks its recorder where things stand.
  if (e.data.ask) {
    if (here.has(market)) channel.postMessage({ market, status: statuses().get(market) ?? null });
    return;
  }
  if ('status' in e.data) return setStatus(market, e.data.status ?? null, false);
  if (minute) live.set(market, [minute[0], unpack(minute[1])]);
  else if (minute === null) live.delete(market);
  tell(market, full === true);
});

/** Called whenever minutes of `market` are written, here or in another tab. */
export function onRecorded(market: string, listener: Listener): () => void {
  let set = listeners.get(market);
  if (!set) listeners.set(market, (set = new Set()));
  set.add(listener);
  return () => set.delete(listener);
}

export const liveMinute = (market: string): [number, Levels] | undefined => live.get(market);

function announce(market: string, minute: [number, Levels] | null): void {
  if (minute) live.set(market, minute);
  else live.delete(market);
  channel?.postMessage({ market, live: minute ? [minute[0], pack(minute[1])] : null });
  tell(market);
}

// ---------------------------------------------------------------- recording

interface Want {
  users: number;
  linger?: ReturnType<typeof setTimeout>;
  stop?: AbortController;
}
const wanted = new Map<string, Want>();

/** `market` recorded while a chart shows it; let go with what this returns. */
export function want(market: string): () => void {
  if (!recordable(market)) return () => {};
  let entry = wanted.get(market);
  if (!entry) wanted.set(market, (entry = { users: 0 }));
  clearTimeout(entry.linger);
  entry.users += 1;
  if (!entry.stop) start(market, entry);
  channel?.postMessage({ market, ask: true });
  let released = false;
  return () => {
    if (released) return;
    released = true;
    entry.users -= 1;
    if (entry.users > 0) return;
    entry.linger = setTimeout(() => {
      entry.stop?.abort();
      wanted.delete(market);
    }, LINGER_MS);
  };
}

function restart(market: string): void {
  const entry = wanted.get(market);
  if (!entry) return;
  entry.stop?.abort();
  entry.stop = undefined;
  if (entry.users > 0) start(market, entry);
}

function start(market: string, entry: Want): void {
  if (!settings().on) return;
  const stop = new AbortController();
  entry.stop = stop;
  void (async () => {
    // The server's markets are the server's: a month of them, for everyone.
    if ((await recordedMarkets()).has(market) || stop.signal.aborted) return;
    void tidy();
    const run = () => record(market, stop.signal);
    if (typeof navigator !== 'undefined' && navigator.locks) {
      // One tab a market: the rest wait their turn, reading what it writes meanwhile.
      await navigator.locks.request(`tradoor-footprint:${market}`, { signal: stop.signal }, run).catch(() => {});
    } else await run();
  })();
}

async function record(market: string, signal: AbortSignal): Promise<void> {
  let meta: MarketRecord = (await readMarket(market)) ?? { market, level: 0, spans: [] };
  let book: MinuteBook | null = null;
  let liveSince = 0;
  let filling = false;
  const venue = parseMarket(market)?.venue;
  here.add(market);
  let status: RecordStatus = { filling: null, live: 0 };
  const report = (patch: Partial<RecordStatus>) => {
    status = { ...status, ...patch };
    if (!signal.aborted) setStatus(market, status);
  };
  report({});
  const off = listenPrints(market, {
    print(time, price, qty, buy) {
      if (!book) {
        if (!(meta.level > 0)) meta = { ...meta, level: levelFor(price, digitsFor(market, price)) };
        book = new MinuteBook(meta.level, liveSince || Date.now());
      }
      book.add(time, price, qty, buy);
    },
    state(state) {
      const now = Date.now();
      if (state === 'live' && !liveSince) {
        liveSince = now;
        book?.resume(now);
        report({ live: now });
        if (!filling && (venue === 'binance-spot' || venue === 'binance-perp')) {
          filling = true;
          void backfill(market, venue, now, () => meta, (m) => (meta = m), (f) => report({ filling: f }), signal);
        }
      } else if (state !== 'live' && liveSince) {
        meta = { ...meta, spans: cover(meta.spans, liveSince, now) };
        liveSince = 0;
        report({ live: 0 });
      }
    },
  });
  const flush = async (all: boolean) => {
    const now = Date.now();
    const finished = book ? book.take(all ? Number.POSITIVE_INFINITY : Math.floor(now / MINUTE) * MINUTE) : [];
    if (liveSince) meta = { ...meta, spans: cover(meta.spans, liveSince, now) };
    meta = { ...meta, spans: trimSpans(meta.spans, now - settings().days * DAY) };
    try {
      await addMinutes(market, finished);
      if (meta.level > 0) await writeMarket(meta);
    } catch {
      // A full disk or a closed database: this stretch is not kept.
    }
    const current = book ? [...book.minutes][0] : undefined;
    announce(market, all || !current ? null : [current[0], new Map(current[1])]);
  };
  const timer = setInterval(() => void flush(false), FLUSH_MS);
  await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  clearInterval(timer);
  off();
  if (liveSince) meta = { ...meta, spans: cover(meta.spans, liveSince, Date.now()) };
  liveSince = 0;
  await flush(true);
  here.delete(market);
  setStatus(market, null);
}

/** Binance's trades for the gaps of the last day, newest first, as far as the calls allow; `progress` hears how far. */
async function backfill(
  market: string,
  venue: 'binance-spot' | 'binance-perp',
  until: number,
  get: () => MarketRecord,
  set: (m: MarketRecord) => void,
  progress: (filling: RecordStatus['filling']) => void,
  signal: AbortSignal,
): Promise<void> {
  const holes = gaps(get().spans, Math.max(until - BACKFILL_MS, until - settings().days * DAY), until).reverse();
  const of = holes.reduce((sum, [a, b]) => sum + (b - a), 0);
  if (of < MINUTE) return;
  let done = 0;
  progress({ done, of, back: until, calls: 0 });
  try {
    await fill(market, venue, holes, get, set, signal, (ms, back, calls) => progress({ done: (done += ms), of, back, calls }));
  } finally {
    progress(null);
  }
}

async function fill(
  market: string,
  venue: 'binance-spot' | 'binance-perp',
  holes: Span[],
  get: () => MarketRecord,
  set: (m: MarketRecord) => void,
  signal: AbortSignal,
  filled: (ms: number, back: number, calls: number) => void,
): Promise<void> {
  const symbol = parseMarket(market)?.symbol;
  if (!symbol) return;
  const hosts = venue === 'binance-perp' ? ['https://fapi.binance.com/fapi/v1/aggTrades'] : ['https://api.binance.com/api/v3/aggTrades', 'https://data-api.binance.vision/api/v3/aggTrades'];
  let calls = BACKFILL_CALLS;
  const ask = async (query: string): Promise<ReturnType<typeof readAggTrades> | null> => {
    for (const host of hosts) {
      if (calls <= 0 || signal.aborted) return null;
      calls -= 1;
      await new Promise((resolve) => setTimeout(resolve, BACKFILL_PACE[venue]));
      try {
        const res = await fetch(`${host}?symbol=${symbol}&${query}&limit=1000`, { signal: AbortSignal.any([signal, AbortSignal.timeout(7_000)]) });
        if (res.ok) return readAggTrades(await res.json());
        // Throttled or refused: no more this time.
        if (res.status === 418 || res.status === 429) calls = 0;
      } catch {
        // Unreachable from this network: the next host, or none.
      }
    }
    return null;
  };
  for (const [from, to] of holes) {
    // An hour a window, the newest first: a window is as far as one start and end may span.
    for (let end = to; end > from && calls > 0 && !signal.aborted; end -= HOUR) {
      const start = Math.max(from, end - HOUR);
      const trades = [];
      let page = await ask(`startTime=${start}&endTime=${end - 1}`);
      while (page) {
        trades.push(...page.filter((t) => t.time < end));
        if (page.length < 1000 || page[page.length - 1]!.time >= end) break;
        page = await ask(`fromId=${page[page.length - 1]!.id + 1}`);
      }
      if (trades.length === 0 && page === null) return;
      // As far as the trades reach: a window too busy for the calls left is kept only so far.
      const reached = page === null ? (trades[trades.length - 1]?.time ?? start) + 1 : end;
      const meta = get();
      const level = meta.level > 0 ? meta.level : levelFor(trades[0]?.price ?? 1, digitsFor(market, trades[0]?.price ?? 1));
      const book = new MinuteBook(level, start);
      for (const t of trades) book.add(t.time, t.price, t.qty, t.buy);
      await addMinutes(market, book.take(Number.POSITIVE_INFINITY));
      set({ ...get(), level, spans: cover(get().spans, start, reached) });
      await writeMarket(get());
      channel?.postMessage({ market, full: true });
      tell(market, true);
      filled(reached - start, start, 1 - calls / BACKFILL_CALLS);
      if (reached < end) return;
    }
  }
}

/** Minutes older than the days kept let go: when recording starts, and now and then after. */
let tidied = 0;
async function tidy(): Promise<void> {
  const now = Date.now();
  if (now - tidied < 30 * MINUTE) return;
  tidied = now;
  await prune(now - settings().days * DAY).catch(() => {});
}

/** One market's recordings gone, or all of them; what is recording now starts afresh. */
export async function forgetRecorded(market?: string): Promise<void> {
  for (const [m] of wanted) if (!market || m === market) wanted.get(m)!.stop?.abort();
  // A recording stopping writes its last minutes down first.
  await new Promise((resolve) => setTimeout(resolve, 300));
  await forget(market);
  live.clear();
  tidied = 0;
  for (const m of wanted.keys()) restart(m);
  for (const m of listeners.keys()) tell(m, true);
}

/**
 * A market's footprint as this browser recorded it, folded into bars of
 * `barMs` from `from` on, with the minute under way; null when nothing was.
 * `by` keeps a grouping already chosen, for bars folded again as they grow.
 */
export async function recordedFootprint(market: string, barMs: number, from: number, settings: Settings, by?: number): Promise<(Footprint & { by: number }) | null> {
  const meta = await readMarket(market).catch(() => undefined);
  if (!meta || !(meta.level > 0)) return null;
  const minutes: [number, Levels][] = await readMinutes(market, from, Date.now()).catch(() => []);
  const current = live.get(market);
  // The minute under way is not written down yet; a stretch of it an earlier recording kept, is.
  if (current && current[0] >= from) {
    const i = minutes.findIndex(([at]) => at === current[0]);
    if (i >= 0) minutes[i] = [current[0], merge(minutes[i]![1], current[1])];
    else minutes.push(current);
  }
  const folded: Minute[] = minutes.map(([at, levels]) => toMinute(at, levels, meta.level));
  return foldMinutes(folded, meta.level, barMs, settings, by);
}

/** How much of the days kept this browser recorded whole, for a market; null when nothing. */
export async function recordedShare(market: string): Promise<{ ms: number; of: number } | null> {
  const meta = await readMarket(market).catch(() => undefined);
  if (!meta || meta.spans.length === 0) return null;
  const now = Date.now();
  const of = settings().days * DAY;
  return { ms: meta.spans.reduce((sum, [a, b]) => sum + Math.max(0, Math.min(b, now) - Math.max(a, now - of)), 0), of };
}
