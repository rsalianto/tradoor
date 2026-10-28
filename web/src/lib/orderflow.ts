/**
 * A market's order flow as the DOM and tape panels read it: every trade with
 * the side that took it, and the order book, kept from the exchange's
 * snapshot and its changes. Owned by the page rather than by a panel, like
 * the liquidation tape: switching tabs keeps what came meanwhile, and once no
 * panel has shown a market for a few minutes its sockets close. Solid-free,
 * so it tests as plain code; panels draw from it on their own frames.
 */
import { OrderFlowHub, bookFromRest, type FlowKind } from '../engine/orderflow-streams';
import { block, isBlocked, reached } from '../engine/reach';
import type { BookMsg, PrintMsg, SocketState } from '../engine/streams';
import { parseMarket, type Venue } from './markets';
import { nativeOf } from './native';

const hub = new OrderFlowHub();

/** Trades held, newest last; the oldest drop off. A busy hour of Binance BTC fits. */
const PRINTS = 200_000;
/** Room for this many to start: a market that hardly trades never takes the rest. */
const PRINTS_FIRST = 4_096;
/** How long a market nobody shows keeps listening. */
const LINGER_MS = 5 * 60_000;
/** Book levels kept a side: where the exchange sends only the best so many, and Binance's, whose far levels no DOM scrolls to. */
const KEEP: Partial<Record<Venue, number>> = { 'binance-spot': 10_000, 'binance-perp': 10_000, 'bybit-spot': 1000, 'bybit-perp': 1000, 'okx-spot': 400, 'okx-perp': 400, 'kraken-spot': 1000, 'bitget-spot': 500, 'bitget-perp': 500 };
/** Pulls and stacks are kept within this share of the price: no DOM scrolls further, and Binance changes levels everywhere. */
const PULL_REACH = 0.03;
/** A trade explains a level's shrinking this long after it, or before. */
const MATCH_MS = 1_500;
/**
 * Pulls and stacks fade, half gone in this long: summed since the book opened
 * they only grow, until every level's is about its size. Fading, they show
 * what the book is doing now.
 */
export const PULL_HALF_LIFE_MS = 30_000;
const fade = (v: number, from: number, to: number): number => v * 2 ** (-(to - from) / PULL_HALF_LIFE_MS);

export type FlowState = SocketState | 'none';

/**
 * Trades in columns, a ring: entry `i` of all ever pushed sits at `i % size`.
 * It starts small and doubles while it has never wrapped, up to `max`.
 */
export class Prints {
  time: Float64Array;
  price: Float64Array;
  qty: Float64Array;
  /** 1 where the aggressor bought. */
  buy: Uint8Array;
  /** Pushed in all; the ones still held are the last `size` of them. */
  count = 0;
  size: number;

  constructor(
    readonly max = PRINTS,
    first = Math.min(max, PRINTS_FIRST),
  ) {
    this.size = first;
    this.time = new Float64Array(first);
    this.price = new Float64Array(first);
    this.qty = new Float64Array(first);
    this.buy = new Uint8Array(first);
  }

  push(time: number, price: number, qty: number, buy: boolean): void {
    // Full and never wrapped: entry `i` is at `i`, and stays there in a bigger ring.
    if (this.count === this.size && this.size < this.max) this.grow(Math.min(this.max, this.size * 2));
    const i = this.count % this.size;
    this.time[i] = time;
    this.price[i] = price;
    this.qty[i] = qty;
    this.buy[i] = buy ? 1 : 0;
    this.count += 1;
  }

  private grow(size: number): void {
    const time = new Float64Array(size);
    const price = new Float64Array(size);
    const qty = new Float64Array(size);
    const buy = new Uint8Array(size);
    time.set(this.time);
    price.set(this.price);
    qty.set(this.qty);
    buy.set(this.buy);
    [this.time, this.price, this.qty, this.buy, this.size] = [time, price, qty, buy, size];
  }

  /** The oldest still held. */
  get first(): number {
    return Math.max(0, this.count - this.size);
  }

  at(i: number): number {
    return i % this.size;
  }

  get last(): number {
    return this.count === 0 ? Number.NaN : this.price[(this.count - 1) % this.size]!;
  }
}

/** One side of a book: size by price. */
export type Side = Map<number, number>;

export const bestBid = (bids: Side): number => {
  let best = Number.NEGATIVE_INFINITY;
  for (const p of bids.keys()) if (p > best) best = p;
  return best;
};

export const bestAsk = (asks: Side): number => {
  let best = Number.POSITIVE_INFINITY;
  for (const p of asks.keys()) if (p < best) best = p;
  return best;
};

/**
 * The step between price levels: the least gap among the best levels,
 * rounded clear of float noise. Hyperliquid's moves with the price, so it is
 * worked out from the book rather than looked up.
 */
export function tickOf(bids: Side, asks: Side): number {
  const near = (side: Side, desc: boolean) => [...side.keys()].sort((a, b) => (desc ? b - a : a - b)).slice(0, 200);
  let tick = Number.POSITIVE_INFINITY;
  for (const list of [near(bids, true), near(asks, false)]) {
    for (let i = 1; i < list.length; i++) {
      const gap = Math.abs(list[i]! - list[i - 1]!);
      if (gap > 0 && gap < tick) tick = gap;
    }
  }
  return Number.isFinite(tick) ? Number(tick.toPrecision(6)) : 0;
}

/** The levels past the best `keep` of a side, dropped: the exchange stops telling of them. */
export function trim(side: Side, keep: number, bids: boolean): void {
  if (side.size <= keep) return;
  const sorted = [...side.keys()].sort((a, b) => (bids ? b - a : a - b));
  for (let i = keep; i < sorted.length; i++) side.delete(sorted[i]!);
}

/**
 * The book, and what changed in it that trades do not explain — orders
 * pulled from a level or stacked onto it — summed by price since it opened.
 */
export class Book {
  readonly bids: Side = new Map();
  readonly asks: Side = new Map();
  /** Whole: a snapshot came, and no change since was missed. */
  synced = false;
  /** The last change's number, where the exchange numbers them. */
  seq: number | undefined;
  /** Pulled (below zero) and stacked by price, fading: the sum, and when it was. */
  readonly pulls = new Map<number, [number, number]>();
  private changes = 0;
  /** Trades not yet matched to a level shrinking: by side, price → [time, qty]. */
  private readonly hits = { bids: new Map<number, [number, number]>(), asks: new Map<number, [number, number]>() };
  /** Shrinking not yet matched to a trade, in case the trade comes after. */
  private readonly drops = { bids: new Map<number, [number, number]>(), asks: new Map<number, [number, number]>() };
  private cachedTick = 0;
  /** Between the best bid and ask, as of the last change: where pulls are kept around. */
  private mid = 0;

  constructor(readonly keep = Number.POSITIVE_INFINITY) {}

  reset(msg: Pick<BookMsg, 'bids' | 'asks' | 'last'>): void {
    this.bids.clear();
    this.asks.clear();
    for (const [p, q] of msg.bids) if (q > 0) this.bids.set(p, q);
    for (const [p, q] of msg.asks) if (q > 0) this.asks.set(p, q);
    this.seq = msg.last;
    this.synced = true;
    this.cachedTick = 0;
    this.uncross(true);
  }

  /**
   * A whole book where one was held, as Hyperliquid sends it every time: read
   * as the changes between the two, so pulls and stacks still show. A level
   * gone from beyond the new one's reach is out of sight, not pulled.
   */
  replace(msg: Pick<BookMsg, 'bids' | 'asks'>, now: number): void {
    const gone = (side: Side, levels: [number, number][], bids: boolean): [number, number][] => {
      const prices = levels.map(([p]) => p);
      const edge = bids ? Math.min(...prices) : Math.max(...prices);
      const kept = new Set(prices);
      const out: [number, number][] = [];
      for (const p of side.keys()) {
        if (kept.has(p)) continue;
        if (bids ? p >= edge : p <= edge) out.push([p, 0]);
        else side.delete(p);
      }
      return out;
    };
    this.apply({ bids: [...msg.bids, ...gone(this.bids, msg.bids, true)], asks: [...msg.asks, ...gone(this.asks, msg.asks, false)], last: undefined }, now);
  }

  /** Changes on top of what is held. */
  apply(msg: Pick<BookMsg, 'bids' | 'asks' | 'last'>, now: number): void {
    this.side(this.bids, msg.bids, 'bids', now);
    this.side(this.asks, msg.asks, 'asks', now);
    if (msg.last !== undefined) this.seq = msg.last;
    if (Number.isFinite(this.keep)) {
      if (this.bids.size > this.keep * 1.25) trim(this.bids, this.keep, true);
      if (this.asks.size > this.keep * 1.25) trim(this.asks, this.keep, false);
    }
    this.uncross(msg.bids.length >= msg.asks.length);
  }

  private side(side: Side, levels: [number, number][], name: 'bids' | 'asks', now: number): void {
    const hits = this.hits[name];
    for (const [p, q] of levels) {
      if (!Number.isFinite(p) || !Number.isFinite(q)) continue;
      const before = side.get(p) ?? 0;
      if (q > 0) side.set(p, q);
      else side.delete(p);
      let change = q - before;
      if (change < 0) {
        // Traded away, not pulled: as much of it as trades there account for.
        const hit = hits.get(p);
        if (hit && now - hit[0] <= MATCH_MS) {
          const used = Math.min(hit[1], -change);
          change += used;
          if (hit[1] - used > 0) hit[1] -= used;
          else hits.delete(p);
        }
        if (change < 0) this.drops[name].set(p, [now, -change]);
      }
      if (change !== 0 && !(this.mid > 0 && Math.abs(p - this.mid) > this.mid * PULL_REACH)) this.pull(p, change, now);
    }
  }

  private pull(price: number, change: number, now: number): void {
    const held = this.pulls.get(price);
    this.pulls.set(price, [(held ? fade(held[0], held[1], now) : 0) + change, now]);
    // Now and then, the ones long faded go.
    if (++this.changes % 5_000 === 0) for (const [p, [, at]] of this.pulls) if (now - at > PULL_HALF_LIFE_MS * 12) this.pulls.delete(p);
  }

  /** Pulled (below zero) or stacked at a price, faded to `now`. */
  pulled(price: number, now: number): number {
    const held = this.pulls.get(price);
    return held ? fade(held[0], held[1], now) : 0;
  }

  /** A trade: it takes from the side it hit, which a level shrinking then is not a pull. */
  traded(price: number, qty: number, buy: boolean, now: number): void {
    const name = buy ? 'asks' : 'bids';
    // The book said so first: what was counted as pulled was this trade.
    const drop = this.drops[name].get(price);
    if (drop && now - drop[0] <= MATCH_MS) {
      const used = Math.min(drop[1], qty);
      this.pull(price, used, now);
      if (drop[1] - used > 0) drop[1] -= used;
      else this.drops[name].delete(price);
      qty -= used;
    }
    if (qty <= 0) return;
    const hits = this.hits[name];
    const held = hits.get(price);
    hits.set(price, held && now - held[0] <= MATCH_MS ? [now, held[1] + qty] : [now, qty]);
    if (hits.size > 256) for (const [p, [t]] of hits) if (now - t > MATCH_MS) hits.delete(p);
    if (this.drops[name].size > 256) for (const [p, [t]] of this.drops[name]) if (now - t > MATCH_MS) this.drops[name].delete(p);
  }

  /**
   * A bid at or above the best ask cannot stand: one side is stale — a level
   * the exchange stopped telling of. The side just changed is taken as right.
   */
  private uncross(bidsRight: boolean): void {
    for (let guard = 0; guard < 10_000; guard++) {
      const bid = bestBid(this.bids);
      const ask = bestAsk(this.asks);
      if (!(bid >= ask)) {
        if (Number.isFinite(bid) && Number.isFinite(ask)) this.mid = (bid + ask) / 2;
        return;
      }
      if (bidsRight) this.asks.delete(ask);
      else this.bids.delete(bid);
    }
  }

  tick(): number {
    if (this.cachedTick === 0) this.cachedTick = tickOf(this.bids, this.asks);
    return this.cachedTick;
  }

  clear(): void {
    this.bids.clear();
    this.asks.clear();
    this.synced = false;
    this.seq = undefined;
    this.cachedTick = 0;
  }
}

/** Changes held for a REST snapshot: Binance sends ten a second. */
const HELD_MAX = 300;

/**
 * What keeps a book whole: snapshots and changes in the order the exchange
 * numbers them. Binance's book comes from REST and its changes on the stream:
 * changes are held until the snapshot is in, those it already has dropped, and
 * the first one kept must start at or before the snapshot's number. A change
 * that does not follow on from the last one means one was missed: `gap`, and
 * the book waits for a new snapshot.
 */
export class BookSync {
  /** Changes held while a REST snapshot is on its way. */
  private held: BookMsg[] | null = null;
  /** The REST snapshot's number, until the first change after it. */
  private floor: number | undefined;

  constructor(
    readonly book: Book,
    /** Every message is the whole book (Hyperliquid). */
    private readonly whole = false,
  ) {}

  /** A REST snapshot is coming: hold the changes until it does. */
  await(): void {
    this.book.synced = false;
    this.held ??= [];
  }

  get waiting(): boolean {
    return this.held !== null;
  }

  message(m: BookMsg, now: number): 'ok' | 'gap' | 'skip' {
    if (this.held) {
      // Half a minute of changes; a longer wait drops the oldest — the
      // snapshot then will not line up, and is asked for again.
      if (this.held.push(m) > HELD_MAX) this.held.shift();
      return 'skip';
    }
    if (m.reset) {
      if (this.whole && this.book.synced) this.book.replace(m, now);
      else this.book.reset(m);
      return 'ok';
    }
    if (!this.book.synced) return 'skip';
    if (this.floor !== undefined) {
      if ((m.last ?? 0) < this.floor) return 'skip';
      if ((m.first ?? 0) > this.floor + 1) return this.lost();
      this.floor = undefined;
    } else if (m.prev !== undefined && this.book.seq !== undefined && m.prev !== this.book.seq) return this.lost();
    this.book.apply(m, now);
    return 'ok';
  }

  /** The REST snapshot, and the changes held since laid over it. */
  snapshot(id: number, bids: [number, number][], asks: [number, number][], now: number): 'ok' | 'gap' {
    const held = this.held ?? [];
    this.held = null;
    this.book.reset({ bids, asks, last: undefined });
    this.floor = id;
    for (const m of held) if (this.message(m, now) === 'gap') return 'gap';
    return 'ok';
  }

  private lost(): 'gap' {
    this.book.synced = false;
    this.floor = undefined;
    return 'gap';
  }
}

/** An answer from an exchange, through the tradoor server where this network blocks it. */
async function rest(url: string): Promise<unknown> {
  const host = new URL(url).host;
  if (!isBlocked(host)) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(7_000) });
      reached(host);
      if (!res.ok) throw new Error(`${host} answered ${res.status}`);
      return await res.json();
    } catch (e) {
      if (!(e instanceof TypeError) && !(e instanceof DOMException)) throw e;
      block(host);
    }
  }
  const res = await fetch(`/api/relay/rest?url=${encodeURIComponent(url)}`, { signal: AbortSignal.timeout(20_000) });
  if (!res.ok) throw new Error(`relay answered ${res.status}`);
  return res.json();
}

/** The coins in one OKX swap contract: its sizes are contracts. */
const contracts = new Map<string, Promise<number>>();
function contractSize(native: string): Promise<number> {
  let size = contracts.get(native);
  if (!size) {
    size = rest(`https://www.okx.com/api/v5/public/instruments?instType=SWAP&instId=${encodeURIComponent(native)}`)
      .then((body) => Number((body as { data?: { ctVal?: string }[] }).data?.[0]?.ctVal) || 1)
      .catch(() => {
        contracts.delete(native);
        return 1;
      });
    contracts.set(native, size);
  }
  return size;
}

export class Flow {
  readonly prints = new Prints();
  readonly book: Book;
  readonly venue: Venue | null;
  state: Record<FlowKind, FlowState> = { prints: 'connecting', book: 'connecting' };
  relayed = false;
  /** Bumped by every change, so a panel draws only when something did. */
  version = 0;
  /** When the book last changed. */
  bookAt = 0;
  /** Each DOM's count of what traded here, by panel: gone with the flow. */
  readonly tallies = new Map<string, Tally>();
  private offs: (() => void)[] = [];
  private native = '';
  private size = 1;
  private closed = false;
  private readonly sync: BookSync;
  private loading = false;
  private batch: PrintMsg[] = [];

  constructor(readonly market: string) {
    this.venue = parseMarket(market)?.venue ?? null;
    this.book = new Book(this.venue ? KEEP[this.venue] : undefined);
    this.sync = new BookSync(this.book, this.venue === 'hyperliquid-perp');
    if (!this.venue || this.venue === 'index') {
      this.state = { prints: 'none', book: 'none' };
      return;
    }
    void this.open(this.venue);
  }

  private async open(venue: Venue): Promise<void> {
    this.native = await nativeOf(this.market);
    if (venue === 'okx-perp') this.size = await contractSize(this.native);
    if (this.closed || this.native === '') return;
    this.offs.push(this.listen(venue, 'prints'));
    this.offs.push(this.listen(venue, 'book'));
    if (bookFromRest(venue)) {
      this.sync.await();
      // The stream first, so no change between the two is missed.
      setTimeout(() => void this.snapshot(), 500);
    }
  }

  private listen(venue: Venue, kind: FlowKind): () => void {
    return hub.subscribe(venue, kind, this.native, {
      message: (msg) => {
        if (msg.kind === 'print') this.print(msg);
        else if (msg.kind === 'book') this.onBook(msg);
      },
      state: (state, relayed) => {
        const was = this.state[kind];
        this.state[kind] = state;
        this.relayed = relayed;
        this.version += 1;
        if (kind !== 'book' || state === was) return;
        // Down: the book stops being whole, and comes back from a new snapshot.
        if (state !== 'live') this.book.synced = false;
        else if (bookFromRest(venue) && !this.book.synced) {
          this.sync.await();
          void this.snapshot();
        }
      },
    });
  }

  /** Trades a message brought, put in time order: a snapshot on subscribing can come newest first. */
  private print(msg: PrintMsg): void {
    if (!(msg.qty > 0) || !(msg.price > 0)) return;
    if (this.batch.push(msg) > 1) return;
    queueMicrotask(() => {
      const batch = this.batch.sort((a, b) => a.time - b.time);
      this.batch = [];
      const now = Date.now();
      const newest = this.prints.count > 0 ? this.prints.time[(this.prints.count - 1) % this.prints.size]! : 0;
      for (const p of batch) {
        // Older than what is held: the snapshot of a socket that reconnected.
        if (p.time < newest - 5_000) continue;
        const qty = p.qty * this.size;
        this.prints.push(p.time || now, p.price, qty, p.buy);
        this.book.traded(p.price, qty, p.buy, now);
      }
      this.version += 1;
    });
  }

  private onBook(msg: BookMsg): void {
    const scale = (levels: [number, number][]) => levels.map(([p, q]): [number, number] => [p, q * this.size]);
    const m = this.size === 1 ? msg : { ...msg, bids: scale(msg.bids), asks: scale(msg.asks) };
    const now = Date.now();
    const done = this.sync.message(m, now);
    if (done === 'gap') this.resync();
    if (done !== 'ok') return;
    this.bookAt = now;
    this.version += 1;
  }

  /** A change was missed: the book is whole again only from a new snapshot. */
  private resync(): void {
    this.version += 1;
    if (this.venue && bookFromRest(this.venue)) {
      this.sync.await();
      void this.snapshot();
      return;
    }
    // Asked for again: the exchange answers with the whole book.
    const off = this.offs.pop();
    off?.();
    setTimeout(() => {
      if (!this.closed && this.venue) this.offs.push(this.listen(this.venue, 'book'));
    }, 300);
  }

  private async snapshot(): Promise<void> {
    if (this.loading || this.closed) return;
    this.loading = true;
    const url = this.venue === 'binance-perp' ? `https://fapi.binance.com/fapi/v1/depth?symbol=${this.native}&limit=1000` : `https://api.binance.com/api/v3/depth?symbol=${this.native}&limit=1000`;
    let again = false;
    try {
      const body = (await rest(url)) as { lastUpdateId: number; bids: [string, string][]; asks: [string, string][] };
      const levels = (rows: [string, string][]) => rows.map(([p, q]): [number, number] => [Number(p), Number(q)]);
      if (this.closed || !this.sync.waiting) return;
      again = this.sync.snapshot(body.lastUpdateId, levels(body.bids), levels(body.asks), Date.now()) === 'gap';
      this.bookAt = Date.now();
      this.version += 1;
    } catch {
      again = true;
    } finally {
      this.loading = false;
    }
    if (again) {
      this.sync.await();
      setTimeout(() => void this.snapshot(), 2_000);
    }
  }

  /** The step between levels; 0 until the book has come. */
  tick(): number {
    return this.book.tick();
  }

  close(): void {
    this.closed = true;
    for (const off of this.offs) off();
    this.offs = [];
  }
}

interface Entry {
  flow: Flow;
  users: number;
  linger: ReturnType<typeof setTimeout> | undefined;
}

const flows = new Map<string, Entry>();

/** The market's flow, listening from now on; let go with `releaseFlow`. */
export function holdFlow(market: string): Flow {
  let entry = flows.get(market);
  if (!entry) {
    entry = { flow: new Flow(market), users: 0, linger: undefined };
    flows.set(market, entry);
  }
  clearTimeout(entry.linger);
  entry.users += 1;
  return entry.flow;
}

export function releaseFlow(flow: Flow): void {
  const entry = flows.get(flow.market);
  if (!entry || entry.flow !== flow) return;
  entry.users -= 1;
  if (entry.users > 0) return;
  entry.linger = setTimeout(() => {
    flows.delete(flow.market);
    flow.close();
  }, LINGER_MS);
}

// ---------------------------------------------------------------- reading trades

/**
 * One aggressor's order: the trades that share its time and side. A market
 * order sweeping five levels prints five times; read as one, its size is what
 * a size filter should judge.
 */
export interface Group {
  time: number;
  buy: boolean;
  qty: number;
  /** In the quote: coins times price, summed. */
  usd: number;
  /** The first and last price it traded at. */
  from: number;
  to: number;
  trades: number;
}

/** Newest first, up to `max` groups of at least `minUsd`, from the trades held. */
export function recentGroups(prints: Prints, minUsd: number, max: number, scanLimit = 50_000): Group[] {
  const out: Group[] = [];
  const stop = Math.max(prints.first, prints.count - scanLimit);
  let open: Group | null = null;
  for (let n = prints.count - 1; n >= stop && out.length < max; n--) {
    const i = prints.at(n);
    const time = prints.time[i]!;
    const buy = prints.buy[i] === 1;
    const price = prints.price[i]!;
    const qty = prints.qty[i]!;
    if (open && open.time === time && open.buy === buy) {
      open.qty += qty;
      open.usd += qty * price;
      open.from = price;
      open.trades += 1;
      continue;
    }
    if (open && open.usd >= minUsd) out.push(open);
    open = { time, buy, qty, usd: qty * price, from: price, to: price, trades: 1 };
  }
  if (open && open.usd >= minUsd && out.length < max) out.push(open);
  return out;
}

/** Everywhere, or the levels above or below a price. */
export type Where = 'all' | 'above' | 'below';
const reaches = (where: Where, price: number) => (p: number): boolean => where === 'all' || (where === 'above' ? p > price : p < price);

/** What a DOM shows of trades since it was cleared: by price, bought and sold, and the totals. */
export class Tally {
  readonly bought = new Map<number, number>();
  readonly sold = new Map<number, number>();
  volume = 0;
  delta = 0;
  high = Number.NEGATIVE_INFINITY;
  low = Number.POSITIVE_INFINITY;
  /** The trades read so far, as a count of all pushed. */
  private cursor = 0;
  /** The group being read, not yet known to pass the filter. */
  private open: { time: number; buy: boolean; usd: number; counted: boolean; idx: number[] } | null = null;
  /** Pulls and stacks at the last clear, fading as the book's do: what is shown is what came since. */
  readonly pullBase = new Map<number, [number, number]>();
  /** Levels cleared above or below a price: their trades until then are not counted again. */
  private clears: { at: number; above: boolean; price: number }[] = [];

  constructor(
    public minUsd: number,
    /** Trades before this are not counted. */
    public since: number,
  ) {}

  /** Reads what came since the last call; from the start again after a clear or a new filter. */
  read(prints: Prints): void {
    if (this.cursor < prints.first) this.cursor = prints.first;
    for (; this.cursor < prints.count; this.cursor++) {
      const i = prints.at(this.cursor);
      const time = prints.time[i]!;
      if (time < this.since || this.cleared(time, prints.price[i]!)) continue;
      const buy = prints.buy[i] === 1;
      const usd = prints.qty[i]! * prints.price[i]!;
      if (!this.open || this.open.time !== time || this.open.buy !== buy) this.open = { time, buy, usd: 0, counted: false, idx: [] };
      const g = this.open;
      g.usd += usd;
      if (g.counted) this.count(prints, i);
      else {
        g.idx.push(i);
        if (g.usd >= this.minUsd) {
          g.counted = true;
          for (const j of g.idx) this.count(prints, j);
          g.idx = [];
        }
      }
    }
  }

  private cleared(time: number, price: number): boolean {
    for (const c of this.clears) if (time <= c.at && (c.above ? price > c.price : price < c.price)) return true;
    return false;
  }

  private count(prints: Prints, i: number): void {
    const price = prints.price[i]!;
    const qty = prints.qty[i]!;
    const into = prints.buy[i] === 1 ? this.bought : this.sold;
    into.set(price, (into.get(price) ?? 0) + qty);
    this.volume += qty;
    this.delta += prints.buy[i] === 1 ? qty : -qty;
    if (price > this.high) this.high = price;
    if (price < this.low) this.low = price;
  }

  /** What traded — everywhere, or above or below `price` — forgotten from `now` on. */
  clear(where: Where = 'all', price = 0, now = Date.now()): void {
    const hit = reaches(where, price);
    for (const map of [this.bought, this.sold]) for (const p of map.keys()) if (hit(p)) map.delete(p);
    if (where !== 'all') this.clears.push({ at: now, above: where === 'above', price });
    else {
      this.since = now;
      this.clears = [];
      this.volume = 0;
      this.delta = 0;
      this.high = Number.NEGATIVE_INFINITY;
      this.low = Number.POSITIVE_INFINITY;
    }
  }

  /** The book's pulls and stacks so far, there: shown from `now` on only what comes after. */
  clearPulls(book: Book, where: Where = 'all', price = 0, now = Date.now()): void {
    const hit = reaches(where, price);
    for (const p of this.pullBase.keys()) if (!book.pulls.has(p)) this.pullBase.delete(p);
    for (const p of book.pulls.keys()) if (hit(p)) this.pullBase.set(p, [book.pulled(p, now), now]);
  }

  /** Read again from `since`, as with a new filter. */
  restart(prints: Prints, since = this.since): void {
    this.since = since;
    this.bought.clear();
    this.sold.clear();
    this.volume = 0;
    this.delta = 0;
    this.high = Number.NEGATIVE_INFINITY;
    this.low = Number.POSITIVE_INFINITY;
    this.cursor = prints.first;
    this.open = null;
    this.read(prints);
  }

  /** Pulled or stacked at a price since the last clear, faded to `now`. */
  pull(book: Book, price: number, now: number): number {
    const base = this.pullBase.get(price);
    return book.pulled(price, now) - (base ? fade(base[0], base[1], now) : 0);
  }
}
