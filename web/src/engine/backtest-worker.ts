/**
 * The strategy tester's worker, its own Frost Script module beside the chart's
 * engine, so a long backtest never holds a chart up. Two ways to use it:
 *
 * - `full`: one run, its drawing and its whole report back — the tester's
 *   live backtest (lib/backtest-live) and the markets tab.
 * - `data`, then `run`s: the optimizer's (lib/optimizer), several of these at
 *   once, each run answered with its figures alone — over all the bars, and
 *   over the part tuned on and the part kept back where the bars are split.
 */
import { statsOf, tradesOf, type Stats } from '../lib/backtest';
import { resultOf } from './backtest-report';
import { formulaReady, runBacktest, type FormulaBars, type InnerBars } from './formula';

export interface PoolData {
  bars: FormulaBars;
  inner: InnerBars | null;
  funding: { time: Float64Array; rate: Float64Array } | null;
  decimals: number;
  source: string;
  settings: number[];
  /** The first bar kept back from tuning; 0, none. */
  split: number;
}

export type PoolIn = { t: 'data'; data: PoolData } | { t: 'run'; job: number; inputs: number[] } | { t: 'full'; req: number; data: PoolData; inputs: number[] };
export type PoolOut =
  | { job: number; error: string }
  | { job: number; all: Stats; train: Stats | null; test: Stats | null }
  | { req: number; out: { meta: string; values: Float64Array; report: Float64Array } | { error: string } };

const scope = self as unknown as { onmessage: ((e: MessageEvent<PoolIn>) => void) | null; postMessage(m: PoolOut, transfer?: Transferable[]): void };
let data: PoolData | null = null;

scope.onmessage = (e) => {
  const m = e.data;
  if (m.t === 'data') {
    data = m.data;
    return;
  }
  if (m.t === 'full') {
    void formulaReady().then(
      (x) => {
        const d = m.data;
        const out = runBacktest(x, d.source, m.inputs, d.bars, d.decimals, d.settings, d.inner, d.funding);
        scope.postMessage({ req: m.req, out }, 'error' in out ? [] : [out.values.buffer, out.report.buffer]);
      },
      () => scope.postMessage({ req: m.req, out: { error: 'error\t0\t0\tFrost Script did not load' } }),
    );
    return;
  }
  const d = data;
  if (!d) {
    scope.postMessage({ job: m.job, error: 'no bars yet' });
    return;
  }
  void formulaReady().then((x) => {
    const out = runBacktest(x, d.source, m.inputs, d.bars, d.decimals, d.settings, d.inner, d.funding);
    if ('error' in out) {
      scope.postMessage({ job: m.job, error: out.error.split('\t')[3] ?? 'the strategy did not run' });
      return;
    }
    const { result } = resultOf(out.report, d.bars.time, d.bars.close, '', '');
    const rows = tradesOf(result.trades);
    const split = d.split > 0 && d.split < d.bars.time.length ? d.split : 0;
    scope.postMessage({ job: m.job, all: statsOf(result, rows), train: split ? statsOf(result, rows, 0, split) : null, test: split ? statsOf(result, rows, split) : null });
  });
};
