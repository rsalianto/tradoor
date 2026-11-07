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
import type { Filling, Stopped } from './fp-fill';
import { cover, levelFor, merge, MINUTE, MinuteBook, pack, toMinute, trimSpans, unpack, type Levels } from './fp-minutes';
import { addMinutes, forget, prune, readMarket, readMinutes, writeMarket, type MarketRecord } from './fp-store';
import { isRatio, parseMarket } from './markets';
import { listenPrints } from './orderflow';

export type { Stopped };

const DAY = 86_400_000;
/** How often finished minutes are written down. */
const FLUSH_MS = 10_000;
/** How often charts hear of trades not written down yet: often enough to read as live. */
const LIVE_MS = 1_000;
/** A market no chart shows any more keeps recording this long, for a chart put back. */
const LINGER_MS = 60_000;
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
/** The minutes not written down yet — the one under way, and any just finished — as their recorder last told them. */
const live = new Map<string, [number, Levels][]>();
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
  filling: Filling | null;
  /** Why the last fill stopped short, and when it tries again: Binance would not give its history to this network. */
  stopped: Stopped | null;
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

type Message = { market: string; live?: [number, ArrayBuffer][] | null; full?: boolean; status?: RecordStatus | null; ask?: true };

channel?.addEventListener('message', (e: MessageEvent<Message>) => {
  const { market, live: minute, full } = e.data;
  // A tab just opened on a market asks its recorder where things stand.
  if (e.data.ask) {
    if (here.has(market)) channel.postMessage({ market, status: statuses().get(market) ?? null });
    return;
  }
  if ('status' in e.data) return setStatus(market, e.data.status ?? null, false);
  if (minute) live.set(market, minute.map(([at, levels]): [number, Levels] => [at, unpack(levels)]));
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

function announce(market: string, minutes: [number, Levels][]): void {
  if (minutes.length > 0) live.set(market, minutes);
  else live.delete(market);
  channel?.postMessage({ market, live: minutes.length > 0 ? minutes.map(([at, levels]) => [at, pack(levels)]) : null });
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
  let status: RecordStatus = { filling: null, stopped: null, live: 0 };
  const report = (patch: Partial<RecordStatus>) => {
    status = { ...status, ...patch };
    if (!signal.aborted) setStatus(market, status);
  };
  report({});
  let again: ReturnType<typeof setTimeout> | undefined;
  /** The gaps up to when this recording went live filled; a fill Binance stopped is tried again later. */
  const fillUpTo = (until: number, venue: 'binance-spot' | 'binance-perp') => {
    clearTimeout(again);
    filling = true;
    report({ stopped: null });
    const wrote = () => {
      channel?.postMessage({ market, full: true });
      tell(market, true);
    };
    void import('./fp-fill')
      .then(
        ({ backfill }) => backfill(market, venue, until, until - settings().days * DAY, () => meta, (m) => (meta = m), (f) => report({ filling: f }), wrote, signal),
        // Not loaded: the network, as for a call that did not answer.
        (): Stopped => ({ why: 'unreachable', retry: Date.now() + 120_000 }),
      )
      .then((stopped) => {
        if (!stopped || signal.aborted) return;
        filling = false;
        report({ stopped });
        again = setTimeout(() => liveSince && fillUpTo(liveSince, venue), stopped.retry - Date.now());
      });
  };
  // Trades since charts last heard, and a write under way: the minutes it takes are neither here nor there until it is done.
  let changed = false;
  let writing = false;
  const unwritten = (): [number, Levels][] => (book ? [...book.minutes].sort((a, b) => a[0] - b[0]).map(([at, levels]): [number, Levels] => [at, new Map(levels)]) : []);
  const off = listenPrints(market, {
    print(time, price, qty, buy) {
      if (!book) {
        if (!(meta.level > 0)) meta = { ...meta, level: levelFor(price, digitsFor(market, price)) };
        book = new MinuteBook(meta.level, liveSince || Date.now());
      }
      if (book.add(time, price, qty, buy)) changed = true;
    },
    state(state) {
      const now = Date.now();
      if (state === 'live' && !liveSince) {
        liveSince = now;
        book?.resume(now);
        report({ live: now });
        // A throttled fill waits for its time, a reconnect or not.
        if (!filling && !(status.stopped && now < status.stopped.retry) && (venue === 'binance-spot' || venue === 'binance-perp')) fillUpTo(now, venue);
      } else if (state !== 'live' && liveSince) {
        meta = { ...meta, spans: cover(meta.spans, liveSince, now) };
        liveSince = 0;
        report({ live: 0 });
      }
    },
  });
  const flush = async (all: boolean) => {
    writing = true;
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
    writing = false;
    changed = false;
    announce(market, all ? [] : unwritten());
  };
  const timer = setInterval(() => void flush(false), FLUSH_MS);
  const ticker = setInterval(() => {
    if (!changed || writing) return;
    changed = false;
    announce(market, unwritten());
  }, LIVE_MS);
  await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  clearInterval(timer);
  clearInterval(ticker);
  clearTimeout(again);
  off();
  if (liveSince) meta = { ...meta, spans: cover(meta.spans, liveSince, Date.now()) };
  liveSince = 0;
  await flush(true);
  here.delete(market);
  setStatus(market, null);
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
  // The minutes not written down yet; a stretch of one an earlier recording kept, is.
  for (const [at, levels] of live.get(market) ?? []) {
    if (at < from) continue;
    const i = minutes.findIndex(([m]) => m === at);
    if (i >= 0) minutes[i] = [at, merge(minutes[i]![1], levels)];
    else minutes.push([at, levels]);
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
