/**
 * The reader's formulas, run in this worker by the formula module
 * (crates/formula-wasm): loaded only when a chart first has one, given bytes
 * and numbers, giving back text and numbers. It imports nothing, so a formula
 * has no way out of it; one that faults it is answered with an error, and the
 * module is loaded afresh for the next.
 */
import formulaUrl from '../wasm/formula/formula.wasm?url';
import type { FormulaDescription, FormulaInput } from './protocol';

interface Exports {
  memory: WebAssembly.Memory;
  fx_alloc(len: number): number;
  fx_free(ptr: number, len: number): void;
  fx_run(src: number, srcLen: number, inputs: number, inputsLen: number, bars: number, n: number): number;
  fx_describe(src: number, srcLen: number): number;
  fx_text_ptr(): number;
  fx_text_len(): number;
  fx_values_ptr(): number;
  fx_values_len(): number;
}

/** The columns a formula reads, oldest first. */
export interface FormulaBars {
  time: Float64Array;
  open: Float64Array;
  high: Float64Array;
  low: Float64Array;
  close: Float64Array;
  volume: Float64Array;
}

let module: Exports | null = null;
let loading: Promise<Exports> | null = null;

/** The module, once it is in: null until then, while it loads. */
export function formulaModule(): Exports | null {
  if (!module && !loading) {
    loading = WebAssembly.instantiateStreaming(fetch(formulaUrl), {})
      .catch(async () => WebAssembly.instantiate(await (await fetch(formulaUrl)).arrayBuffer(), {}))
      .then((r) => (module = r.instance.exports as unknown as Exports))
      .finally(() => (loading = null));
  }
  return module;
}

/** Resolves once the module is in. */
export const formulaReady = (): Promise<Exports> => (module ? Promise.resolve(module) : (formulaModule(), loading ?? Promise.reject(new Error('Frost Script did not load'))));

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** The module's work in one go: what it is given freed after, and a fault answered as an error. */
function using<T>(x: Exports, src: string, work: (src: number, len: number) => T): T | { line: number; col: number; message: string } {
  const bytes = encoder.encode(src);
  const ptr = x.fx_alloc(bytes.length);
  try {
    new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
    return work(ptr, bytes.length);
  } catch {
    // A trap leaves the module's memory as it was mid-way: a fresh one for the next.
    module = null;
    return { line: 0, col: 0, message: 'the script stopped Frost Script; it starts again with the next' };
  } finally {
    if (module === x) x.fx_free(ptr, bytes.length);
  }
}

const text = (x: Exports): string => decoder.decode(new Uint8Array(x.memory.buffer, x.fx_text_ptr(), x.fx_text_len()));

function parseError(t: string): { line: number; col: number; message: string } {
  const [, line, col, message] = t.trim().split('\t');
  return { line: Number(line) || 0, col: Number(col) || 0, message: message ?? 'the script did not run' };
}

/** What it gives for the chart engine: its text and values as they are, or why it did not run. */
export function runFormula(x: Exports, src: string, inputs: readonly number[], bars: FormulaBars): { meta: string; values: Float64Array } | { error: string } {
  const n = bars.time.length;
  const result = using(x, src, (sp, sl) => {
    const ip = x.fx_alloc(inputs.length * 8);
    const bp = x.fx_alloc(n * 6 * 8);
    try {
      new Float64Array(x.memory.buffer, ip, inputs.length).set(inputs);
      const cols = new Float64Array(x.memory.buffer, bp, n * 6);
      [bars.time, bars.open, bars.high, bars.low, bars.close, bars.volume].forEach((c, k) => cols.set(c.subarray(0, n), k * n));
      const ok = x.fx_run(sp, sl, ip, inputs.length, bp, n) === 1;
      const meta = text(x);
      // Copied out: the module's memory is written again by the next.
      return ok ? { meta, values: new Float64Array(x.memory.buffer, x.fx_values_ptr(), x.fx_values_len()).slice() } : { error: meta };
    } finally {
      x.fx_free(ip, inputs.length * 8);
      x.fx_free(bp, n * 6 * 8);
    }
  });
  if ('message' in result) return { error: `error\t${result.line}\t${result.col}\t${result.message}` };
  return result;
}

/** A formula checked, and what it offers: its settings, lines and marks, or the first mistake in it. */
export function describeFormula(x: Exports, src: string): FormulaDescription {
  const result = using(x, src, (sp, sl) => ({ ok: x.fx_describe(sp, sl) === 1, t: text(x) }));
  if ('message' in result) return { ok: false, ...result };
  if (!result.ok) return { ok: false, ...parseError(result.t) };
  const d: FormulaDescription & { ok: true } = { ok: true, ownPane: false, inputs: [], plots: [], marks: [] };
  for (const row of result.t.split('\n')) {
    const f = row.split('\t');
    if (f[0] === 'pane') d.ownPane = f[1] === '1';
    else if (f[0] === 'input') {
      const num = (s?: string) => (s === undefined || s === '' ? undefined : Number(s));
      const input: FormulaInput = { name: f[1]!, default: Number(f[2]), min: num(f[3]), max: num(f[4]), step: Number(f[5]) || 1, label: f[6] ?? f[1]! };
      d.inputs.push(input);
    } else if (f[0] === 'plot') d.plots.push({ kind: f[1]!, color: Number(f[2]), title: f[3] ?? '' });
    else if (f[0] === 'mark') d.marks.push(f[1] ?? '');
  }
  return d;
}
