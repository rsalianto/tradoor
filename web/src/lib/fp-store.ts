/**
 * Footprints recorded in this browser, kept in its own database (IndexedDB) and
 * nowhere else: a minute a record, keyed by market and minute, and a record a
 * market with the level it is recorded at and the spans recorded whole. Where
 * the browser keeps no database — a private window may not — they are held in
 * memory instead, for as long as the page is open.
 */
import { merge, pack, trimSpans, unpack, type Levels, type Span } from './fp-minutes';

export interface MarketRecord {
  market: string;
  /** The price step a level is: fixed when the market is first recorded. */
  level: number;
  /** What was recorded whole, oldest first. */
  spans: Span[];
}

interface MinuteRecord {
  market: string;
  at: number;
  levels: ArrayBuffer;
}

const DB = 'tradoor-footprint';

let opened: Promise<IDBDatabase | null> | null = null;
const memory = { markets: new Map<string, MarketRecord>(), minutes: new Map<string, MinuteRecord>() };

function open(): Promise<IDBDatabase | null> {
  opened ??= new Promise((resolve) => {
    if (typeof indexedDB === 'undefined') return resolve(null);
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      db.createObjectStore('markets', { keyPath: 'market' });
      db.createObjectStore('minutes', { keyPath: ['market', 'at'] }).createIndex('at', 'at');
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => resolve(null);
    req.onblocked = () => resolve(null);
  });
  return opened;
}

const done = <T>(req: IDBRequest<T>): Promise<T> => new Promise((resolve, reject) => ((req.onsuccess = () => resolve(req.result)), (req.onerror = () => reject(req.error))));
const finished = (tx: IDBTransaction): Promise<void> => new Promise((resolve, reject) => ((tx.oncomplete = () => resolve()), (tx.onerror = () => reject(tx.error)), (tx.onabort = () => reject(tx.error))));
const key = (market: string, at: number) => `${market}|${at}`;

export async function readMarket(market: string): Promise<MarketRecord | undefined> {
  const db = await open();
  if (!db) return memory.markets.get(market);
  return (await done(db.transaction('markets').objectStore('markets').get(market))) as MarketRecord | undefined;
}

export async function writeMarket(record: MarketRecord): Promise<void> {
  const db = await open();
  if (!db) return void memory.markets.set(record.market, record);
  await done(db.transaction('markets', 'readwrite').objectStore('markets').put(record));
}

/** A market's minutes from `from` to `to`, oldest first. */
export async function readMinutes(market: string, from: number, to: number): Promise<[number, Levels][]> {
  const db = await open();
  const rows = db
    ? ((await done(db.transaction('minutes').objectStore('minutes').getAll(IDBKeyRange.bound([market, from], [market, to])))) as MinuteRecord[])
    : [...memory.minutes.values()].filter((r) => r.market === market && r.at >= from && r.at <= to).sort((a, b) => a.at - b.at);
  return rows.map((r) => [r.at, unpack(r.levels)]);
}

/** Minutes added to what is kept for them: a minute two recordings shared holds both. */
export async function addMinutes(market: string, minutes: readonly [number, Levels][]): Promise<void> {
  if (minutes.length === 0) return;
  const db = await open();
  if (!db) {
    for (const [at, levels] of minutes) {
      const held = memory.minutes.get(key(market, at));
      memory.minutes.set(key(market, at), { market, at, levels: pack(held ? merge(unpack(held.levels), levels) : levels) });
    }
    return;
  }
  const tx = db.transaction('minutes', 'readwrite');
  const store = tx.objectStore('minutes');
  for (const [at, levels] of minutes) {
    const get = store.get([market, at]);
    get.onsuccess = () => {
      const held = get.result as MinuteRecord | undefined;
      store.put({ market, at, levels: pack(held ? merge(unpack(held.levels), levels) : levels) });
    };
  }
  await finished(tx);
}

/** Everything recorded before `before` let go, and the spans trimmed to match. */
export async function prune(before: number): Promise<void> {
  const db = await open();
  if (!db) {
    for (const [k, r] of memory.minutes) if (r.at < before) memory.minutes.delete(k);
    for (const m of memory.markets.values()) m.spans = trimSpans(m.spans, before);
    return;
  }
  const tx = db.transaction(['minutes', 'markets'], 'readwrite');
  const cursor = tx.objectStore('minutes').index('at').openCursor(IDBKeyRange.upperBound(before, true));
  cursor.onsuccess = () => {
    const c = cursor.result;
    if (!c) return;
    c.delete();
    c.continue();
  };
  const markets = tx.objectStore('markets');
  const all = markets.getAll();
  all.onsuccess = () => {
    for (const m of all.result as MarketRecord[]) {
      const spans = trimSpans(m.spans, before);
      if (spans.length !== m.spans.length || spans[0]?.[0] !== m.spans[0]?.[0]) markets.put({ ...m, spans });
    }
  };
  await finished(tx);
}

/** What each market keeps: its minutes and their bytes, the busiest first. */
export async function usage(): Promise<{ market: string; minutes: number; bytes: number }[]> {
  const db = await open();
  const by = new Map<string, { minutes: number; bytes: number }>();
  const count = (r: MinuteRecord) => {
    const held = by.get(r.market) ?? { minutes: 0, bytes: 0 };
    held.minutes += 1;
    held.bytes += r.levels.byteLength + 32;
    by.set(r.market, held);
  };
  if (!db) memory.minutes.forEach(count);
  else {
    const tx = db.transaction('minutes');
    const cursor = tx.objectStore('minutes').openCursor();
    cursor.onsuccess = () => {
      const c = cursor.result;
      if (!c) return;
      count(c.value as MinuteRecord);
      c.continue();
    };
    await finished(tx);
  }
  return [...by].map(([market, u]) => ({ market, ...u })).sort((a, b) => b.bytes - a.bytes);
}

/** One market's recordings, or every market's, gone. */
export async function forget(market?: string): Promise<void> {
  const db = await open();
  if (!db) {
    for (const [k, r] of memory.minutes) if (!market || r.market === market) memory.minutes.delete(k);
    if (market) memory.markets.delete(market);
    else memory.markets.clear();
    return;
  }
  const tx = db.transaction(['minutes', 'markets'], 'readwrite');
  if (market) {
    tx.objectStore('minutes').delete(IDBKeyRange.bound([market, -Infinity], [market, Infinity]));
    tx.objectStore('markets').delete(market);
  } else {
    tx.objectStore('minutes').clear();
    tx.objectStore('markets').clear();
  }
  await finished(tx);
}
