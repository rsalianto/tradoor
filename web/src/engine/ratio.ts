/**
 * One market priced in another — XPL on Bybit in BTC on Binance — as candles.
 *
 * Each side is its own market's series, loaded, archived and refreshed as any
 * chart's would be; the ratio is those two folded to the chart's bars and
 * divided bar by bar, as TradingView draws `A/B`: open over open, close over
 * close, high over high and low over low, the extremes then widened to take in
 * the open and the close. Bars only one side has are left out, and there is no
 * volume: a ratio is not traded.
 *
 * Kept free of the engine and of history's loaders, so the arithmetic can be
 * tested on its own; the worker hands in the series and the fold.
 */
import type { CandleSeries, Cols, Series } from './history';
import type { SocketState } from './streams';

const empty = (): Cols => {
  const e = () => new Float64Array(0);
  return { time: e(), open: e(), high: e(), low: e(), close: e(), volume: e(), buy: e() };
};

export interface Ohlc {
  open: number;
  high: number;
  low: number;
  close: number;
}

/** A side's forming bar, as its stream reports it. */
export interface LiveKline extends Ohlc {
  openTime: number;
  closed: boolean;
}

/** One bar of `a` priced in `b`; null where `b` has no price to divide by. */
export function ratioBar(a: Ohlc, b: Ohlc): Ohlc | null {
  if (!(b.open > 0 && b.high > 0 && b.low > 0 && b.close > 0)) return null;
  const open = a.open / b.open;
  const close = a.close / b.close;
  const high = Math.max(a.high / b.high, open, close);
  const low = Math.min(a.low / b.low, open, close);
  return [open, high, low, close].every(Number.isFinite) ? { open, high, low, close } : null;
}

/** The bars both have, each of `a`'s divided by `b`'s at the same time. */
export function divide(a: Cols, b: Cols): Cols {
  const n = Math.min(a.time.length, b.time.length);
  const out = { time: new Float64Array(n), open: new Float64Array(n), high: new Float64Array(n), low: new Float64Array(n), close: new Float64Array(n), volume: new Float64Array(n) };
  let k = 0;
  for (let i = 0, j = 0; i < a.time.length && j < b.time.length; ) {
    const ta = a.time[i]!;
    const tb = b.time[j]!;
    if (ta < tb) i += 1;
    else if (tb < ta) j += 1;
    else {
      const r = ratioBar({ open: a.open[i]!, high: a.high[i]!, low: a.low[i]!, close: a.close[i]! }, { open: b.open[j]!, high: b.high[j]!, low: b.low[j]!, close: b.close[j]! });
      if (r) {
        out.time[k] = ta;
        out.open[k] = r.open;
        out.high[k] = r.high;
        out.low[k] = r.low;
        out.close[k] = r.close;
        k += 1;
      }
      i += 1;
      j += 1;
    }
  }
  return { time: out.time.subarray(0, k), open: out.open.subarray(0, k), high: out.high.subarray(0, k), low: out.low.subarray(0, k), close: out.close.subarray(0, k), volume: out.volume.subarray(0, k), buy: new Float64Array(k).fill(-1) };
}

/**
 * Decimals for a ratio's prices, which no exchange ticks: enough that a bar's
 * usual range reads to two figures, and the price itself to three. XPL in BTC
 * wants nine, BTC's perpetual in its spot five, BTC in ETH two.
 */
export function ratioDecimals(c: Cols): number {
  const n = c.time.length;
  if (n === 0) return 8;
  const ranges: number[] = [];
  for (let i = Math.max(0, n - 200); i < n; i += 1) {
    const range = c.high[i]! - c.low[i]!;
    if (range > 0) ranges.push(range);
  }
  ranges.sort((a, b) => a - b);
  const range = ranges[ranges.length >> 1];
  const price = Math.abs(c.close[n - 1]!);
  const byRange = range ? Math.ceil(-Math.log10(range)) + 1 : 0;
  const byPrice = price > 0 ? Math.ceil(-Math.log10(price)) + 2 : 0;
  return Math.min(10, Math.max(2, byRange, byPrice));
}

/**
 * A ratio as the chart reads a series. `ensure` and `older` take bars of the
 * chart's own timeframe; each side is asked for what that is in its own bars.
 */
export class RatioSeries implements CandleSeries {
  cols: Cols = empty();
  readonly source = 'ratio' as const;
  /** Each side's latest forming bar, waiting on the other's. */
  private readonly forming: (LiveKline | null)[] = [null, null];
  /** Each side's stream: its state, and whether it comes through the relay. */
  private readonly streams: [SocketState, boolean][] = [
    ['connecting', false],
    ['connecting', false],
  ];
  private readonly listeners = new Set<() => void>();
  private offs: (() => void)[] = [];

  constructor(
    readonly legs: readonly [Series, Series],
    /** The chart's timeframe: the bars the two sides are folded to before dividing. */
    readonly interval: string,
    /** One side's series folded to the chart's bars, the forming one left out. */
    private readonly fold: (cols: Cols) => Cols,
    /** How many of a side's own bars cover so many of the chart's. */
    private readonly legBars: (leg: Series, bars: number) => number,
  ) {}

  /** Older history ends where either side's does. */
  get exhausted(): boolean {
    return this.legs[0].exhausted || this.legs[1].exhausted;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    if (this.offs.length === 0) this.offs = this.legs.map((leg) => leg.subscribe(() => this.changed()));
    return () => {
      this.listeners.delete(listener);
      if (this.listeners.size === 0) for (const off of this.offs.splice(0)) off();
    };
  }

  /**
   * A side's latest kline in; the ratio's forming bar out, once both sides
   * stand on the same bar — one may roll a moment before the other.
   */
  join(side: 0 | 1, k: LiveKline): LiveKline | null {
    this.forming[side] = k;
    const [a, b] = this.forming;
    if (!a || !b || a.openTime !== b.openTime) return null;
    const r = ratioBar(a, b);
    return r && { ...r, openTime: a.openTime, closed: a.closed && b.closed };
  }

  /** A side's stream state in; the ratio's out — the worse of the two, relayed if either is. */
  stream(side: 0 | 1, state: SocketState, relayed: boolean): [SocketState, boolean] {
    this.streams[side] = [state, relayed];
    const worst = (['retrying', 'stalled', 'connecting'] as const).find((s) => this.streams.some(([st]) => st === s)) ?? 'live';
    return [worst, this.streams.some(([, r]) => r)];
  }

  /** The decimals its prices are shown to: see `ratioDecimals`. */
  decimals(): number {
    return ratioDecimals(this.cols);
  }

  async ensure(bars: number): Promise<void> {
    await Promise.all(this.legs.map((leg) => leg.ensure(this.legBars(leg, bars))));
    this.recompute();
  }

  async older(): Promise<boolean> {
    const added = await Promise.all(this.legs.map((leg) => leg.older()));
    this.recompute();
    return added.some(Boolean);
  }

  async refreshTail(force = false): Promise<void> {
    await Promise.all(this.legs.map((leg) => leg.refreshTail(force)));
  }

  private recompute(): void {
    this.cols = divide(this.fold(this.legs[0].cols), this.fold(this.legs[1].cols));
  }

  private changed(): void {
    this.recompute();
    for (const listener of [...this.listeners]) listener();
  }
}
