/**
 * The footprint as a browser records it: trades bucketed by the minute they
 * printed in and the price level they fell on, bought and sold apart — what the
 * server keeps for its eight markets, kept here for any other, in this browser
 * only. Plain code, so it tests as such; fp-store keeps the minutes, fp-recorder
 * fills them.
 */
import type { Minute } from './footprint';

export const MINUTE = 60_000;

/** A minute's levels: the level's step (price ÷ level) → `[bought, sold]`, in coins. */
export type Levels = Map<number, [number, number]>;

/** A stretch of time recorded whole, `[from, to)` in ms. */
export type Span = [number, number];

/**
 * The level a market is recorded at: a round step about a hundred-thousandth
 * of its price — a dollar on BTC, as the server keeps it — and never finer than
 * the price's own decimals. Coarser rows are made by adding levels together,
 * which cannot be undone, so the finest worth having is what is kept.
 */
export function levelFor(price: number, decimals: number): number {
  const tick = 10 ** -Math.max(0, Math.min(12, decimals));
  const raw = price * 1e-5;
  if (!(raw > tick)) return tick;
  const power = 10 ** Math.floor(Math.log10(raw));
  const head = raw / power;
  return (head >= 5 ? 5 : head >= 2 ? 2 : 1) * power;
}

/** Trades of one market as they come, by minute. */
export class MinuteBook {
  readonly minutes = new Map<number, Levels>();
  private newest = 0;

  /** `since`: trades before it are not this recording's — a snapshot sent on subscribing. */
  constructor(
    readonly level: number,
    private since: number,
  ) {}

  /** Recording again from `since`, after the socket was down. */
  resume(since: number): void {
    this.since = since;
  }

  /** Adds a trade; false when it is older than this recording or than what came already. */
  add(time: number, price: number, qty: number, buy: boolean): boolean {
    // A second's slack: a batch can come a little out of order, a resent one cannot.
    if (time < this.since || time < this.newest - 1_000 || !(qty > 0) || !(price > 0)) return false;
    this.newest = Math.max(this.newest, time);
    const at = Math.floor(time / MINUTE) * MINUTE;
    let levels = this.minutes.get(at);
    if (!levels) {
      levels = new Map();
      this.minutes.set(at, levels);
    }
    const step = Math.round(price / this.level);
    const held = levels.get(step) ?? [0, 0];
    held[buy ? 0 : 1] += qty;
    levels.set(step, held);
    return true;
  }

  /** The minutes started before `before`, taken out: they are done. */
  take(before: number): [number, Levels][] {
    const done = [...this.minutes].filter(([at]) => at < before).sort((a, b) => a[0] - b[0]);
    for (const [at] of done) this.minutes.delete(at);
    return done;
  }
}

/** Levels as stored: per level its step (int32), bought and sold (float32). */
export function pack(levels: Levels): ArrayBuffer {
  const view = new DataView(new ArrayBuffer(levels.size * 12));
  let i = 0;
  for (const [step, [buy, sell]] of levels) {
    view.setInt32(i, step, true);
    view.setFloat32(i + 4, buy, true);
    view.setFloat32(i + 8, sell, true);
    i += 12;
  }
  return view.buffer;
}

export function unpack(buffer: ArrayBuffer): Levels {
  const view = new DataView(buffer);
  const out: Levels = new Map();
  for (let i = 0; i + 12 <= buffer.byteLength; i += 12) out.set(view.getInt32(i, true), [view.getFloat32(i + 4, true), view.getFloat32(i + 8, true)]);
  return out;
}

/** `b` added into `a`, a fresh map. */
export function merge(a: Levels, b: Levels): Levels {
  const out: Levels = new Map([...a].map(([step, v]) => [step, [v[0], v[1]] as [number, number]]));
  for (const [step, [buy, sell]] of b) {
    const held = out.get(step) ?? [0, 0];
    out.set(step, [held[0] + buy, held[1] + sell]);
  }
  return out;
}

/** A recorded minute as the chart folds it: prices, not steps. */
export const toMinute = (at: number, levels: Levels, level: number): Minute => ({
  at,
  levels: [...levels].sort((a, b) => a[0] - b[0]).map(([step, [buy, sell]]) => [step * level, buy, sell]),
});

/** `from..to` added to the spans: sorted, overlapping and touching ones joined. */
export function cover(spans: readonly Span[], from: number, to: number): Span[] {
  if (!(to > from)) return [...spans];
  const all = [...spans, [from, to] as Span].sort((a, b) => a[0] - b[0]);
  const out: Span[] = [];
  for (const [a, b] of all) {
    const last = out[out.length - 1];
    if (last && a <= last[1]) last[1] = Math.max(last[1], b);
    else out.push([a, b]);
  }
  return out;
}

/** The spans from `from` on: older ones dropped, one straddling it cut. */
export const trimSpans = (spans: readonly Span[], from: number): Span[] => spans.filter(([, b]) => b > from).map(([a, b]): Span => [Math.max(a, from), b]);

/** What of `from..to` no span covers. */
export function gaps(spans: readonly Span[], from: number, to: number): Span[] {
  const out: Span[] = [];
  let at = from;
  for (const [a, b] of [...spans].sort((x, y) => x[0] - y[0])) {
    if (b <= at) continue;
    if (a >= to) break;
    if (a > at) out.push([at, Math.min(a, to)]);
    at = Math.max(at, b);
  }
  if (at < to) out.push([at, to]);
  return out;
}

/** How much of `from..to` the spans cover, in ms. */
export const coveredMs = (spans: readonly Span[], from: number, to: number): number => to - from - gaps(spans, from, to).reduce((sum, [a, b]) => sum + (b - a), 0);

/** One of Binance's aggregated trades: an order's fills at one price. */
export interface AggTrade {
  id: number;
  time: number;
  price: number;
  qty: number;
  /** The buyer took: `m` (buyer was maker) false. */
  buy: boolean;
}

/** Binance's `aggTrades` answer, spot's and futures' alike; what does not read is skipped. */
export function readAggTrades(body: unknown): AggTrade[] {
  if (!Array.isArray(body)) return [];
  return body.flatMap((t: Record<string, unknown>) => {
    const trade = { id: Number(t.a), time: Number(t.T), price: Number(t.p), qty: Number(t.q), buy: t.m === false };
    return Number.isFinite(trade.id) && trade.time > 0 && trade.price > 0 && trade.qty > 0 ? [trade] : [];
  });
}
