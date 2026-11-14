/**
 * A chart's strategy, backtested as the chart's closed bars change — so a bar
 * closing live tests it forward. The chart's worker hands the bars over
 * (`watchBars`); they are run in the tester's own worker (engine/backtest-worker),
 * never the chart's; the trades go back to the chart to be drawn, and the
 * report to the tester.
 *
 * Two things a bar's open, high, low and close cannot say are read beside it:
 * where a bar reached both a position's stop and its target, the lower
 * timeframe's bars inside it, a round at a time, so the next run can say which
 * came first; and a perpetual's funding, from its exchange (lib/funding).
 */
import { resultOf } from '../engine/backtest-report';
import type { PoolData, PoolIn, PoolOut } from '../engine/backtest-worker';
import { call, send } from '../engine/client';
import type { InnerBars } from '../engine/formula';
import { TRADE_SIZE, type BacktestResult, type ChartBars, type StrategyDrawing, type StrategyRun } from '../engine/protocol';
import { fundingKnown, fundingRates, type FundingRates } from './funding';
import { parseMarket } from './markets';

const LOWER: [string, number][] = [
  ['1m', 60_000],
  ['3m', 180_000],
  ['5m', 300_000],
  ['15m', 900_000],
  ['30m', 1_800_000],
  ['1h', 3_600_000],
  ['2h', 7_200_000],
  ['4h', 14_400_000],
];
/** The lower timeframe a bar is looked inside: the shortest with 300 of its bars or fewer in one. */
export function lowerOf(barMs: number): [string, number] | null {
  for (const [tf, ms] of LOWER) if (ms < barMs && barMs / ms <= 300) return [tf, ms];
  return null;
}

/** Bars looked inside a round, and in all for one market and timeframe. */
const ROUND = 20;
const MOST_LOOKED = 400;
/** Trades drawn on the chart: the newest. */
const DRAWN = 1_000;
/** Bars a run takes at most (crates/frost-script MAX_BARS): the newest, past these. */
export const MOST_BARS = 100_000;

const GREEN = 0x26a69aff;
const RED = 0xff4d5fff;
const BLUE = 0x2962ffff;
const ORANGE = 0xff9800ff;
const GREY = 0x787b86ff;

// ---------------------------------------------------------------- the worker, shared by every chart's tester

let worker: Worker | null = null;
let nextReq = 1;
const waiting = new Map<number, (out: { meta: string; values: Float64Array; report: Float64Array } | { error: string }) => void>();

function pool(): Worker {
  if (worker) return worker;
  worker = new Worker(new URL('../engine/backtest-worker.ts', import.meta.url), { type: 'module', name: 'tradoor-backtest' });
  worker.onmessage = (e: MessageEvent<PoolOut>) => {
    const m = e.data;
    if (!('req' in m)) return;
    waiting.get(m.req)?.(m.out);
    waiting.delete(m.req);
  };
  worker.onerror = () => {
    // Started again for the next run.
    for (const done of waiting.values()) done({ error: 'error\t0\t0\tthe backtest stopped; it runs again with the next bar' });
    waiting.clear();
    worker?.terminate();
    worker = null;
  };
  return worker;
}

/** One run in the tester's worker: its drawing and its report's numbers. */
export function runOnce(data: PoolData, inputs: number[]): Promise<{ meta: string; values: Float64Array; report: Float64Array } | { error: string }> {
  return new Promise((resolve) => {
    const req = nextReq++;
    waiting.set(req, resolve);
    pool().postMessage({ t: 'full', req, data, inputs } satisfies PoolIn);
  });
}

function failure(error: string): BacktestResult {
  const [, line, col, message] = error.trim().split('\t');
  return { ok: false, line: Number(line) || 0, col: Number(col) || 0, message: message ?? 'the strategy did not run' };
}

/** A perpetual's funding over a stretch: what the report says of it, and the rates where they were read. */
async function fundingOf(market: string, from: number, to: number): Promise<{ state: string; rates: FundingRates | null }> {
  if (!parseMarket(market)?.venue.endsWith('-perp')) return { state: 'spot', rates: null };
  if (!fundingKnown(market)) return { state: 'none', rates: null };
  try {
    const rates = await fundingRates(market, from, to);
    return { state: rates ? 'paid' : 'none', rates };
  } catch {
    return { state: 'failed', rates: null };
  }
}

/** One run over another market's bars, its funding read first: the markets tab's, where nothing is drawn. */
export async function backtestMarket(run: StrategyRun, market: string, timeframe: string, bars: number): Promise<BacktestResult> {
  const b = await call('marketBars', { market, timeframe, bars });
  if (b.time.length === 0) return failure('error\t0\t0\tno bars');
  const funding = run.funding ? await fundingOf(market, b.time[0]!, b.time[b.time.length - 1]! + b.barMs) : { state: 'off', rates: null };
  const out = await runOnce({ bars: b, inner: null, funding: funding.rates, decimals: b.decimals, source: run.source, settings: run.settings, split: 0 }, run.inputs);
  if ('error' in out) return failure(out.error);
  const { result } = resultOf(out.report, b.time, b.close, market, timeframe);
  result.funding = funding.state;
  result.settlements = funding.rates?.time.length ?? 0;
  return result;
}

// ---------------------------------------------------------------- one chart's

export class LiveBacktest {
  private run: StrategyRun | null = null;
  private bars: ChartBars | null = null;
  /** Lower bars by the open of the bar they are inside; bars they could not be read for. */
  private looked = new Map<number, InnerBars>();
  private unread = new Set<number>();
  private reading = false;
  private funding: FundingRates | null = null;
  private fundingKey = '';
  private fundingState = 'off';
  private key = '';
  /** A run asked for while one is out: only the newest is run once it is back. */
  private busy = false;
  private again = false;
  private closed = false;
  /** What the last run read beside the bars, for the optimizer to run over again. */
  private lastInner: InnerBars | null = null;
  private lastFunding: FundingRates | null = null;

  constructor(
    private readonly sid: string,
    private readonly show: (result: BacktestResult | null) => void,
  ) {
    send({ t: 'watchBars', id: sid, on: true });
  }

  set(run: StrategyRun | null): void {
    this.run = run;
    this.go();
  }

  /** The chart's closed bars, as the worker hands them over: the newest MOST_BARS of them. */
  setBars(all: ChartBars): void {
    const cut = all.time.length - MOST_BARS;
    const bars = cut > 0 ? { ...all, time: all.time.slice(cut), open: all.open.slice(cut), high: all.high.slice(cut), low: all.low.slice(cut), close: all.close.slice(cut), volume: all.volume.slice(cut) } : all;
    const key = `${bars.market}|${bars.timeframe}`;
    if (key !== this.key) {
      this.key = key;
      this.looked = new Map();
      this.unread = new Set();
      this.reading = false;
      this.funding = null;
      this.fundingKey = '';
    }
    this.bars = bars;
    this.go();
  }

  /** The bars and what the last run read beside them: the optimizer's to run over again. */
  data(): Omit<PoolData, 'source' | 'settings' | 'split'> | null {
    const b = this.bars;
    return b ? { bars: b, inner: this.lastInner, funding: this.lastFunding, decimals: b.decimals } : null;
  }

  dispose(): void {
    this.closed = true;
    send({ t: 'watchBars', id: this.sid, on: false });
    send({ t: 'strategyDrawing', id: this.sid, own: null, trades: null });
  }

  private go(): void {
    if (this.closed) return;
    if (this.busy) {
      this.again = true;
      return;
    }
    const run = this.run;
    const bars = this.bars;
    if (!run) {
      send({ t: 'strategyDrawing', id: this.sid, own: null, trades: null });
      this.show(null);
      return;
    }
    if (!bars || bars.time.length === 0) return;
    this.wantFunding(run, bars);
    const lower = lowerOf(bars.barMs);
    const inner = run.magnify && lower ? this.innerOf(bars) : null;
    const funding = run.funding && this.fundingState === 'paid' ? this.funding : null;
    this.lastInner = inner;
    this.lastFunding = funding;
    this.busy = true;
    const key = this.key;
    void runOnce({ bars, inner, funding, decimals: bars.decimals, source: run.source, settings: run.settings, split: 0 }, run.inputs).then((out) => {
      this.busy = false;
      if (this.closed) return;
      if (this.again) {
        this.again = false;
        this.go();
        return;
      }
      if (key !== this.key) return;
      if ('error' in out) {
        send({ t: 'strategyDrawing', id: this.sid, own: null, trades: null });
        this.show(failure(out.error));
        return;
      }
      const { result, trades, ambiguous } = resultOf(out.report, bars.time, bars.close, bars.market, bars.timeframe);
      // Bars to look inside, a round at a time: each round runs it again.
      if (run.magnify && lower && !this.reading && this.looked.size < MOST_LOOKED) {
        const want = [...ambiguous].map((i) => bars.time[i]!).filter((t) => Number.isFinite(t) && !this.looked.has(t) && !this.unread.has(t));
        if (want.length > 0) void this.look(want.slice(0, ROUND), lower, bars);
      }
      result.inner = run.magnify && lower ? lower[0] : '';
      result.reading = this.reading;
      result.funding = run.funding ? this.fundingState : 'off';
      result.settlements = funding?.time.length ?? 0;
      const own: StrategyDrawing = { label: `Strategy · ${run.name}`, meta: out.meta, values: out.values };
      const drawn = run.draw && trades.length > 0 ? { label: 'Trades', ...tradesDrawn(trades, bars.time.length) } : null;
      send({ t: 'strategyDrawing', id: this.sid, own, trades: drawn });
      this.show(result);
    });
  }

  /** The lower bars inside each of these bars, read by the chart's worker. */
  private async look(times: number[], lower: [string, number], bars: ChartBars): Promise<void> {
    const key = this.key;
    this.reading = true;
    try {
      const got = await call('lowerBars', { market: bars.market, interval: lower[0], barMs: bars.barMs, times });
      if (key !== this.key) return;
      const expected = bars.barMs / lower[1];
      times.forEach((t, i) => {
        const all = got[i];
        // Only the rows inside the bar: an exchange may send one past it.
        const end = all ? all.time.findIndex((x) => x >= t + bars.barMs) : -1;
        const c = all && end >= 0 ? { time: all.time.slice(0, end), open: all.open.slice(0, end), high: all.high.slice(0, end), low: all.low.slice(0, end), close: all.close.slice(0, end) } : all;
        // Whole, or near it: a bar read in part could say the wrong one came first.
        if (c && c.time.length >= Math.floor(expected * 0.9) && c.time[0] === t) this.looked.set(t, c);
        else this.unread.add(t);
      });
    } catch {
      for (const t of times) this.unread.add(t);
    } finally {
      if (key === this.key) {
        this.reading = false;
        this.go();
      }
    }
  }

  /** The lower bars looked at so far inside these bars, oldest first, as one set. */
  private innerOf(bars: ChartBars): InnerBars | null {
    if (this.looked.size === 0) return null;
    const first = bars.time[0]!;
    const last = bars.time[bars.time.length - 1]!;
    const parts = [...this.looked.entries()].filter(([t]) => t >= first && t <= last).sort((a, b) => a[0] - b[0]).map(([, c]) => c);
    if (parts.length === 0) return null;
    const total = parts.reduce((s, p) => s + p.time.length, 0);
    const out: InnerBars = { time: new Float64Array(total), open: new Float64Array(total), high: new Float64Array(total), low: new Float64Array(total), close: new Float64Array(total) };
    let at = 0;
    for (const p of parts) {
      for (const k of ['time', 'open', 'high', 'low', 'close'] as const) out[k].set(p[k], at);
      at += p.time.length;
    }
    return out;
  }

  /** A perpetual's funding over the bars, read once for the stretch they cover, and again a day on. */
  private wantFunding(run: StrategyRun, bars: ChartBars): void {
    if (!run.funding) return;
    const from = bars.time[0]!;
    const to = bars.time[bars.time.length - 1]! + bars.barMs;
    const key = `${this.key}|${from}|${Math.floor(to / 86_400_000)}`;
    if (key === this.fundingKey) return;
    this.fundingKey = key;
    this.fundingState = 'reading';
    const was = this.key;
    void fundingOf(bars.market, from, to).then((f) => {
      if (was !== this.key) return;
      this.funding = f.rates;
      this.fundingState = f.state;
      this.go();
    });
  }
}

/**
 * The trades as the chart draws them, on the price: an arrow where each was
 * entered and closed, as marks, and a dashed line from its entry to its exit
 * in the colour of how it went.
 */
function tradesDrawn(trades: Float64Array, n: number): { meta: string; values: Float64Array } {
  const count = trades.length / TRADE_SIZE;
  const from = Math.max(0, count - DRAWN);
  const marks = [new Float64Array(n), new Float64Array(n), new Float64Array(n), new Float64Array(n)];
  const lines: number[] = [];
  let rows = '';
  for (let k = from; k < count; k++) {
    const t = trades.subarray(k * TRADE_SIZE, (k + 1) * TRADE_SIZE);
    const long = t[0] === 1;
    const entryBar = t[1]!;
    const exitBar = t[4]!;
    const open = t[13] === 0;
    if (entryBar >= 0 && entryBar < n) marks[long ? 0 : 1]![entryBar] = 1;
    if (!open && exitBar >= 0 && exitBar < n) marks[long ? 2 : 3]![exitBar] = 1;
    rows += `line\t${open ? GREY : t[9]! > 0 ? GREEN : RED}\t12\t1\t0\n`;
    lines.push(entryBar, t[3]!, exitBar, t[6]!);
  }
  const meta = `pane\t0\nmark\t0\t${BLUE}\tLong\nmark\t1\t${ORANGE}\tShort\nmark\t1\t${GREY}\tClose\nmark\t0\t${GREY}\tClose\n${rows}`;
  const all = new Float64Array(4 * n + lines.length);
  marks.forEach((m, i) => all.set(m, i * n));
  all.set(lines, 4 * n);
  return { meta, values: all };
}
