/**
 * What a chart's strategy tester keeps in the workspace: which strategy it
 * tests — rules picked from lists, a script written in it, or one of the
 * library's — and the account it is tested with. Read back with every value
 * checked, as every panel's is (workspace/model). The tester itself, and all
 * it works out, load only when it is opened (components/StrategyTester).
 */

/** A value a rule compares: a bar's price, an indicator of it, or a number. */
export interface Operand {
  kind: string;
  /** Its length, or the number itself. */
  n: number;
  /** A second setting where it has one: Bollinger's width, MACD's slow length. */
  m: number;
  /** MACD's signal length. */
  k: number;
}

export interface Condition {
  a: Operand;
  /** `xa` crosses above, `xb` crosses below, `gt` is above, `lt` is below. */
  op: string;
  b: Operand;
}

export interface RuleSet {
  /** All of them, or any one. */
  all: boolean;
  list: Condition[];
}

export interface Rules {
  long: RuleSet;
  short: RuleSet;
  exitLong: RuleSet;
  exitShort: RuleSet;
}

export interface ExitSetting {
  /** `none`, `pct` of the entry, `atr` multiples, `r` multiples of the stop (a target only). */
  kind: string;
  v: number;
}

export interface BacktestSettings {
  capital: number;
  /** `equity`: a share of it, `fixed`: an amount, `risk`: the share lost at the stop. */
  sizeKind: string;
  size: number;
  leverage: number;
  /** The exchange's own fees, or these, in percent. */
  autoFees: boolean;
  taker: number;
  maker: number;
  /** Percent lost against each market and stop fill. */
  slippage: number;
  /** `open`: the next bar's open; `close`: the close the order was given at. */
  fill: string;
  /** `both`, `long`, `short`. */
  side: string;
  stop: ExitSetting;
  target: ExitSetting;
  trail: ExitSetting;
  atr: number;
  maxBars: number;
  funding: boolean;
  magnify: boolean;
  /** Days tested back from the last bar; 0, every bar the chart holds; -1, the market's whole history. */
  days: number;
  /** Tested from this moment, in place of `days`; 0, not set. */
  since: number;
  /** Percent of the bars the optimizer tunes on; the rest are kept back to test its pick. 0, none. */
  split: number;
}

export interface BacktestState {
  open: boolean;
  /** The tester's height under the chart, and its width beside it, in pixels. */
  height: number;
  width: number;
  /** `bottom`: under the chart; `right`: beside it. */
  dock: string;
  tab: string;
  /** `rules`, `code`, or `script`: one of the library's, by its study kind. */
  source: string;
  rules: Rules;
  code: string;
  script: string;
  inputs: number[];
  settings: BacktestSettings;
  draw: boolean;
  /** Tested forward from this moment: the trades since are reported apart. */
  forwardFrom: number | null;
  /** A notice when the strategy enters or exits on a bar that just closed. */
  alerts: boolean;
}

export const MAX_CODE = 20_000;
export const MAX_RULES = 6;

const op = (kind: string, n = 0, m = 0, k = 0): Operand => ({ kind, n, m, k });

export const DEFAULT_RULES = (): Rules => ({
  long: { all: true, list: [{ a: op('ema', 9), op: 'xa', b: op('ema', 21) }] },
  short: { all: true, list: [{ a: op('ema', 9), op: 'xb', b: op('ema', 21) }] },
  exitLong: { all: true, list: [] },
  exitShort: { all: true, list: [] },
});

export const DEFAULT_SETTINGS = (): BacktestSettings => ({
  capital: 10_000,
  sizeKind: 'equity',
  size: 100,
  leverage: 1,
  autoFees: true,
  taker: 0.05,
  maker: 0.02,
  slippage: 0.02,
  fill: 'open',
  side: 'both',
  stop: { kind: 'none', v: 2 },
  target: { kind: 'none', v: 4 },
  trail: { kind: 'none', v: 3 },
  atr: 14,
  maxBars: 0,
  funding: true,
  magnify: true,
  days: 0,
  since: 0,
  split: 0,
});

export const defaultBacktest = (): BacktestState => ({
  open: true,
  height: 300,
  width: 460,
  dock: 'bottom',
  tab: 'summary',
  source: 'rules',
  rules: DEFAULT_RULES(),
  code: '',
  script: '',
  inputs: [],
  settings: DEFAULT_SETTINGS(),
  draw: true,
  forwardFrom: null,
  alerts: false,
});

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const num = (v: unknown, d: number, lo: number, hi: number): number => {
  const x = Number(v);
  return Number.isFinite(x) && x >= lo && x <= hi ? x : d;
};
const word = (v: unknown, words: readonly string[], d: string): string => (typeof v === 'string' && words.includes(v) ? v : d);

export const OPERANDS = ['close', 'open', 'high', 'low', 'hl2', 'volume', 'value', 'sma', 'ema', 'wma', 'vwma', 'rsi', 'macd', 'macdsignal', 'macdhist', 'bbupper', 'bbbasis', 'bblower', 'stochk', 'stochd', 'atr', 'highest', 'lowest', 'cci', 'mfi', 'roc', 'volsma'] as const;
export const OPS = ['xa', 'xb', 'gt', 'lt'] as const;

function readOperand(raw: unknown, d: Operand): Operand {
  if (!isObj(raw)) return d;
  return { kind: word(raw.kind, OPERANDS, d.kind), n: num(raw.n, d.n, -1e12, 1e12), m: num(raw.m, 0, -1e6, 1e6), k: num(raw.k, 0, 0, 1e4) };
}

function readSet(raw: unknown, d: RuleSet): RuleSet {
  if (!isObj(raw) || !Array.isArray(raw.list)) return d;
  const list = raw.list.slice(0, MAX_RULES).filter(isObj).map((c) => ({ a: readOperand(c.a, op('close')), op: word(c.op, OPS, 'gt'), b: readOperand(c.b, op('value')) }));
  return { all: raw.all !== false, list };
}

function readExit(raw: unknown, kinds: readonly string[], d: ExitSetting): ExitSetting {
  if (!isObj(raw)) return d;
  return { kind: word(raw.kind, kinds, 'none'), v: num(raw.v, d.v, 0, 10_000) };
}

/** A tester's state as a workspace stored it, every value checked; null for none. */
export function readBacktest(raw: unknown): BacktestState | null {
  if (!isObj(raw)) return null;
  const d = defaultBacktest();
  const ds = d.settings;
  const s = isObj(raw.settings) ? raw.settings : {};
  const r = isObj(raw.rules) ? raw.rules : {};
  return {
    open: raw.open === true,
    height: Math.round(num(raw.height, d.height, 120, 2_000)),
    width: Math.round(num(raw.width, d.width, 300, 3_000)),
    dock: word(raw.dock, ['bottom', 'right'], d.dock),
    tab: word(raw.tab, ['summary', 'trades', 'charts', 'strategy', 'optimize', 'markets'], d.tab),
    source: word(raw.source, ['rules', 'code', 'script'], d.source),
    rules: { long: readSet(r.long, d.rules.long), short: readSet(r.short, d.rules.short), exitLong: readSet(r.exitLong, d.rules.exitLong), exitShort: readSet(r.exitShort, d.rules.exitShort) },
    code: typeof raw.code === 'string' ? raw.code.slice(0, MAX_CODE) : '',
    script: typeof raw.script === 'string' && /^fx-[\w-]{1,40}$/.test(raw.script) ? raw.script : '',
    inputs: Array.isArray(raw.inputs) ? raw.inputs.slice(0, 32).map((v) => num(v, 0, -1e15, 1e15)) : [],
    settings: {
      capital: num(s.capital, ds.capital, 1, 1e12),
      sizeKind: word(s.sizeKind, ['equity', 'fixed', 'risk'], ds.sizeKind),
      size: num(s.size, ds.size, 0, 1e12),
      leverage: num(s.leverage, ds.leverage, 1, 200),
      autoFees: s.autoFees !== false,
      taker: num(s.taker, ds.taker, 0, 5),
      maker: num(s.maker, ds.maker, -1, 5),
      slippage: num(s.slippage, ds.slippage, 0, 10),
      fill: word(s.fill, ['open', 'close'], ds.fill),
      side: word(s.side, ['both', 'long', 'short'], ds.side),
      stop: readExit(s.stop, ['none', 'pct', 'atr'], ds.stop),
      target: readExit(s.target, ['none', 'pct', 'atr', 'r'], ds.target),
      trail: readExit(s.trail, ['none', 'pct', 'atr'], ds.trail),
      atr: Math.round(num(s.atr, ds.atr, 1, 500)),
      maxBars: Math.round(num(s.maxBars, 0, 0, 100_000)),
      funding: s.funding !== false,
      magnify: s.magnify !== false,
      days: Math.round(num(s.days, 0, -1, 36_500)),
      since: Math.round(num(s.since, 0, 0, 8.64e15)),
      split: Math.round(num(s.split, 0, 0, 95)),
    },
    draw: raw.draw !== false,
    forwardFrom: raw.forwardFrom === null || raw.forwardFrom === undefined ? null : num(raw.forwardFrom, 0, 0, 8.64e15) || null,
    alerts: raw.alerts === true,
  };
}
