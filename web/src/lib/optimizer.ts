/**
 * The strategy tester's optimizer: a strategy run over a chart's bars once for
 * each pair of settings on a grid, in workers of their own (engine/backtest-worker),
 * as many as the device has cores to spare and four at most.
 */
import type { PoolData, PoolIn, PoolOut } from '../engine/backtest-worker';
import type { Stats } from './backtest';

export interface Cell {
  /** The settings it ran with, in the script's order. */
  inputs: number[];
  all: Stats | null;
  train: Stats | null;
  test: Stats | null;
  error?: string;
}

/** Runs of `data`'s strategy with each set of settings; `progress` as each comes back. Stopped by `signal`. */
export function optimize(data: PoolData, jobs: number[][], progress: (done: number, cell: Cell, index: number) => void, signal: AbortSignal): Promise<Cell[]> {
  const cores = typeof navigator !== 'undefined' && navigator.hardwareConcurrency ? navigator.hardwareConcurrency : 2;
  const count = Math.max(1, Math.min(4, cores - 1, jobs.length));
  const cells: Cell[] = jobs.map((inputs) => ({ inputs, all: null, train: null, test: null }));
  return new Promise((resolve) => {
    const workers: Worker[] = [];
    let next = 0;
    let done = 0;
    const finish = () => {
      for (const w of workers) w.terminate();
      resolve(cells);
    };
    signal.addEventListener('abort', finish, { once: true });
    const feed = (w: Worker) => {
      if (signal.aborted) return;
      if (next >= jobs.length) return;
      const job = next++;
      w.postMessage({ t: 'run', job, inputs: jobs[job]! } satisfies PoolIn);
    };
    for (let i = 0; i < count; i++) {
      const w = new Worker(new URL('../engine/backtest-worker.ts', import.meta.url), { type: 'module', name: 'tradoor-optimizer' });
      workers.push(w);
      w.onmessage = (e: MessageEvent<PoolOut>) => {
        const m = e.data;
        if ('req' in m) return;
        const cell = cells[m.job]!;
        if ('error' in m) cell.error = m.error;
        else {
          cell.all = m.all;
          cell.train = m.train;
          cell.test = m.test;
        }
        done += 1;
        progress(done, cell, m.job);
        if (done >= jobs.length) finish();
        else feed(w);
      };
      w.onerror = () => {
        // A worker that fails ends the run with what came back.
        finish();
      };
      w.postMessage({ t: 'data', data } satisfies PoolIn);
      feed(w);
    }
  });
}
