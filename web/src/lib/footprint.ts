/**
 * Volume by price, as the server recorded it, folded into the bars on screen.
 *
 * The server keeps one minute at a time at the finest level it can — a dollar
 * on BTC, a tenth on ETH. A chart showing five-minute bars wants five of those
 * minutes added together, and a chart zoomed out wants the levels added too:
 * forty rows in a bar three pixels wide is a smudge, not a reading. Both are
 * sums, which is why the fine grain is what is stored — and why the rows are
 * sent fine and the chart joins them as it zooms out.
 */
import { api } from './api';

/** A bar's levels — price, coins bought, coins sold, lowest price first — and
 *  the most and least its delta reached from the open, at each minute's close. */
export interface Levels {
  time: number;
  levels: [number, number, number][];
  maxDelta: number;
  minDelta: number;
}

/** The bars in a range, and the price one row of them covers. */
export interface Footprint {
  bars: Levels[];
  /** The same for every bar: a row must read straight across the screen. */
  grid: number;
  /** Whether the chart may join rows as it zooms out: the reader set no row size. */
  fitted: boolean;
}

/** Rows a typical bar is sent at most when the chart fits them to its zoom:
 *  fine enough to split rows zoomed in, few enough to send every few seconds. */
const MAX_FINE_ROWS = 200;

/** What the reader chose: how many ticks a row covers, and in what unit. */
export interface Settings {
  /** Ticks a row covers, or 0 to fit the rows to the bar. */
  ticks: number;
  /** `coins` as the exchange counts them, or `usd` at the level's own price. */
  unit: 'coins' | 'usd';
}

/** The markets this server records, asked for once. */
let recorded: Promise<Map<string, number>> | null = null;
export function recordedMarkets(): Promise<Map<string, number>> {
  recorded ??= api
    .footprintMarkets()
    .then((r) => new Map(r.markets.map((m) => [m.market, m.tick])))
    .catch(() => new Map<string, number>());
  return recorded;
}

/** Minutes one request may carry; a longer range is asked for in pages. */
const PAGE_MINUTES = 1_440;
/** Pages one fetch will ask for: a week of minutes, which is plenty of screen. */
const MAX_PAGES = 7;

/**
 * The minutes between `from` and `to`, folded into bars of `barMs` and grouped
 * onto one price grid — the same for every bar, so a level reads straight
 * across the screen rather than each bar keeping a grid of its own.
 *
 * The server hands out a day of minutes at a time, so a longer range is asked
 * for in pages: a chart of fifteen-minute bars covers a day in a screenful.
 */
export async function fetchFootprint(market: string, barMs: number, from: number, to: number, settings: Settings = { ticks: 0, unit: 'coins' }): Promise<Footprint> {
  const pages: Promise<{ tick: number; bars: { at: number; levels: [number, number, number][] }[] }>[] = [];
  const page = PAGE_MINUTES * 60_000;
  for (let start = Math.max(from, to - MAX_PAGES * page); start < to; start += page) {
    pages.push(api.footprint(market, start, Math.min(start + page - 1, to)));
  }
  const answers = await Promise.all(pages);
  const tick = answers.find((a) => a.tick > 0)?.tick ?? 1;
  const minutes = answers.flatMap((a) => a.bars).sort((a, b) => a.at - b.at);
  const byBar = new Map<number, Map<number, [number, number]>>();
  // The delta as the bar went, minute by minute, and the most and least it reached.
  const swings = new Map<number, { delta: number; max: number; min: number }>();
  for (const minute of minutes) {
    const at = Math.floor(minute.at / barMs) * barMs;
    let levels = byBar.get(at);
    if (!levels) {
      levels = new Map();
      byBar.set(at, levels);
    }
    let delta = 0;
    for (const [price, buy, sell] of minute.levels) {
      const step = Math.round(price / tick);
      const held = levels.get(step);
      if (held) {
        held[0] += buy;
        held[1] += sell;
      } else {
        levels.set(step, [buy, sell]);
      }
      delta += (buy - sell) * (settings.unit === 'usd' ? price : 1);
    }
    const swing = swings.get(at) ?? { delta: 0, max: 0, min: 0 };
    swing.delta += delta;
    swing.max = Math.max(swing.max, swing.delta);
    swing.min = Math.min(swing.min, swing.delta);
    swings.set(at, swing);
  }
  // One grouping for the lot, taken from a bar in the middle of the pack
  // rather than the wildest one: a single violent minute in the day would
  // otherwise set a grid so coarse that every quiet bar collapses into one row
  // and says nothing at all.
  const spans = [...byBar.values()]
    .map((levels) => {
      const steps = [...levels.keys()];
      return steps.length > 0 ? Math.max(...steps) - Math.min(...steps) + 1 : 0;
    })
    .sort((a, b) => a - b);
  const typical = spans.length > 0 ? spans[Math.floor(spans.length * 0.8)]! : 0;
  // A row size the reader set is theirs; left to fit, the chart joins these rows as it zooms out.
  const fitted = !(settings.ticks > 0);
  let by = fitted ? 1 : Math.round(settings.ticks);
  while (fitted && typical / by > MAX_FINE_ROWS) by = coarser(by);
  return {
    grid: by * tick,
    fitted,
    bars: [...byBar.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([time, levels]) => ({ time, levels: group(levels, tick, by, settings), maxDelta: swings.get(time)?.max ?? 0, minDelta: swings.get(time)?.min ?? 0 })),
  };
}

/** Steps to prices, adding neighbouring levels together onto the shared grid. */
function group(levels: Map<number, [number, number]>, tick: number, by: number, settings: Settings): [number, number, number][] {
  const rounded = Math.round(by);
  const out = new Map<number, [number, number]>();
  for (const [step, [buy, sell]] of levels) {
    const key = Math.floor(step / rounded) * rounded;
    const held = out.get(key);
    if (held) {
      held[0] += buy;
      held[1] += sell;
    } else {
      out.set(key, [buy, sell]);
    }
  }
  return [...out.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([step, [buy, sell]]) => {
      const price = step * tick;
      // In dollars, a coin at this level is worth what it traded at here.
      const worth = settings.unit === 'usd' ? price : 1;
      return [price, buy * worth, sell * worth] as [number, number, number];
    });
}

/** The next grouping up: 1, 2, 5, 10, 20, 50, and so on. */
function coarser(by: number): number {
  const power = 10 ** Math.floor(Math.log10(by));
  const head = by / power;
  return (head < 2 ? 2 : head < 5 ? 5 : 10) * power;
}

/** The flat array the engine reads: per bar, its time, its row count, the most
 *  and least its delta reached, its rows. */
export function flatten(bars: Levels[]): Float64Array {
  const flat: number[] = [];
  for (const bar of bars) {
    flat.push(bar.time, bar.levels.length, bar.maxDelta, bar.minDelta);
    for (const [price, buy, sell] of bar.levels) flat.push(price, buy, sell);
  }
  return Float64Array.from(flat);
}
