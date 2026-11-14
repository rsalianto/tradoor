/**
 * The strategy tester's working: the settings as the backtest reads them, the
 * rule builder's rules written as Frost Script, example strategies, and every
 * figure of the report worked out from the trades and the equity the backtest
 * gives (crates/frost-script strategy). Loaded with the tester.
 */
import type { BacktestResult, StrategyRun } from '../engine/protocol';
import { TRADE_SIZE } from '../engine/protocol';
import type { BacktestSettings, BacktestState, Condition, Operand, RuleSet, Rules } from './backtest-state';
import { parseMarket, symbolOf } from './markets';

export { defaultBacktest, readBacktest } from './backtest-state';

const DAY = 86_400_000;

// ---------------------------------------------------------------- costs

/** Each exchange's fees at its first tier, in percent: taker, maker. Editable in the tester. */
const FEES: Record<string, [number, number]> = {
  'binance-perp': [0.05, 0.02],
  'binance-spot': [0.1, 0.1],
  'bybit-perp': [0.055, 0.02],
  'bybit-spot': [0.1, 0.1],
  'okx-perp': [0.05, 0.02],
  'okx-spot': [0.1, 0.08],
  'bitget-perp': [0.06, 0.02],
  'bitget-spot': [0.1, 0.1],
  'hyperliquid-perp': [0.045, 0.015],
  'kraken-spot': [0.4, 0.25],
  'kraken-perp': [0.05, 0.02],
  'lighter-perp': [0, 0],
  'gate-spot': [0.2, 0.2],
  'gate-perp': [0.05, 0.02],
  'aster-spot': [0.1, 0.1],
  'aster-perp': [0.035, 0.01],
  'htx-spot': [0.2, 0.2],
  'htx-perp': [0.05, 0.02],
  'bitfinex-spot': [0.2, 0.1],
  'bitfinex-perp': [0.065, 0.02],
  'deribit-perp': [0.05, 0],
  'coinbase-spot': [0.6, 0.4],
  'bitstamp-spot': [0.4, 0.3],
  'bitstamp-perp': [0.05, 0.02],
};

/** The fees a market is tested with where the tester takes the exchange's: taker, maker, in percent. */
export function venueFees(market: string): [number, number] {
  const venue = parseMarket(market)?.venue ?? '';
  return FEES[venue] ?? (venue.endsWith('-perp') ? [0.05, 0.02] : venue === 'index' ? [0, 0] : [0.1, 0.1]);
}

export const isPerp = (market: string): boolean => parseMarket(market)?.venue.endsWith('-perp') === true;

/** The maintenance margin rate a liquidation is reckoned at: the first tier's. */
const mmrOf = (market: string): number => (/:BTCUSDT?$|:BTC$|:BTCUSD$/.test(market) ? 0.004 : 0.005);

/** The 21 numbers `fx_backtest` reads (crates/frost-script-wasm). */
export function settingsOf(s: BacktestSettings, market: string, lastTime: number): number[] {
  const [taker, maker] = s.autoFees ? venueFees(market) : [s.taker, s.maker];
  const size = s.sizeKind === 'fixed' ? [1, s.size] : s.sizeKind === 'risk' ? [2, s.size / 100] : [0, s.size / 100];
  const exit = (e: { kind: string; v: number }) => (e.kind === 'pct' ? [1, e.v / 100] : e.kind === 'atr' ? [2, e.v] : e.kind === 'r' ? [3, e.v] : [0, 0]);
  const from = s.days > 0 && Number.isFinite(lastTime) ? lastTime - s.days * DAY : Number.NaN;
  return [
    s.capital,
    size[0]!,
    size[1]!,
    Math.max(1, s.leverage),
    taker / 100,
    maker / 100,
    s.slippage / 100,
    s.fill === 'close' ? 1 : 0,
    s.side === 'short' ? 0 : 1,
    s.side === 'long' ? 0 : 1,
    ...exit(s.stop),
    ...exit(s.target),
    ...exit(s.trail),
    s.atr,
    s.maxBars,
    s.leverage > 1 || isPerp(market) ? mmrOf(market) : 0,
    from,
    Number.NaN,
  ];
}

// ---------------------------------------------------------------- the rule builder

/** What each value a rule compares is called, and its settings. */
export const OPERAND_INFO: Record<string, { label: string; n?: string; m?: string; k?: string; price?: boolean; d: [number, number, number] }> = {
  close: { label: 'Close', price: true, d: [0, 0, 0] },
  open: { label: 'Open', price: true, d: [0, 0, 0] },
  high: { label: 'High', price: true, d: [0, 0, 0] },
  low: { label: 'Low', price: true, d: [0, 0, 0] },
  hl2: { label: 'HL2', price: true, d: [0, 0, 0] },
  volume: { label: 'Volume', d: [0, 0, 0] },
  value: { label: 'Value', n: 'Value', d: [50, 0, 0] },
  sma: { label: 'SMA', n: 'Length', price: true, d: [50, 0, 0] },
  ema: { label: 'EMA', n: 'Length', price: true, d: [21, 0, 0] },
  wma: { label: 'WMA', n: 'Length', price: true, d: [21, 0, 0] },
  vwma: { label: 'VWMA', n: 'Length', price: true, d: [21, 0, 0] },
  rsi: { label: 'RSI', n: 'Length', d: [14, 0, 0] },
  macd: { label: 'MACD', n: 'Fast', m: 'Slow', k: 'Signal', d: [12, 26, 9] },
  macdsignal: { label: 'MACD signal', n: 'Fast', m: 'Slow', k: 'Signal', d: [12, 26, 9] },
  macdhist: { label: 'MACD histogram', n: 'Fast', m: 'Slow', k: 'Signal', d: [12, 26, 9] },
  bbupper: { label: 'Bollinger upper', n: 'Length', m: 'Width', price: true, d: [20, 2, 0] },
  bbbasis: { label: 'Bollinger basis', n: 'Length', m: 'Width', price: true, d: [20, 2, 0] },
  bblower: { label: 'Bollinger lower', n: 'Length', m: 'Width', price: true, d: [20, 2, 0] },
  stochk: { label: 'Stoch %K', n: 'Length', m: '%K smoothing', d: [14, 3, 0] },
  stochd: { label: 'Stoch %D', n: 'Length', m: '%K smoothing', k: '%D smoothing', d: [14, 3, 3] },
  atr: { label: 'ATR', n: 'Length', d: [14, 0, 0] },
  highest: { label: 'Highest high (before)', n: 'Bars', price: true, d: [20, 0, 0] },
  lowest: { label: 'Lowest low (before)', n: 'Bars', price: true, d: [20, 0, 0] },
  cci: { label: 'CCI', n: 'Length', d: [20, 0, 0] },
  mfi: { label: 'MFI', n: 'Length', d: [14, 0, 0] },
  roc: { label: 'Rate of change %', n: 'Length', d: [10, 0, 0] },
  volsma: { label: 'Volume SMA', n: 'Length', d: [20, 0, 0] },
};

export const OP_LABEL: Record<string, string> = { xa: 'crosses above', xb: 'crosses below', gt: 'is above', lt: 'is below' };

export function operand(kind: string): Operand {
  const d = OPERAND_INFO[kind]?.d ?? [0, 0, 0];
  return { kind, n: d[0], m: d[1], k: d[2] };
}

/** `EMA 21`, `MACD 12 26 9`, `30`. */
export function operandLabel(o: Operand): string {
  const i = OPERAND_INFO[o.kind];
  if (!i) return o.kind;
  if (o.kind === 'value') return String(o.n);
  return [i.label, i.n ? o.n : null, i.m ? o.m : null, i.k ? o.k : null].filter((x) => x !== null).join(' ');
}

const whole = (v: number, lo = 1, hi = 5000) => Math.min(hi, Math.max(lo, Math.round(v)));
const plain = (v: number) => (Number.isFinite(v) ? String(Number(v.toFixed(8))) : '0');

/** Where in the rules a setting of their script came from: a set, a rule, its side, and the field. */
export interface RulePlace {
  set: keyof Rules;
  index: number;
  side: 'a' | 'b';
  field: 'n' | 'm' | 'k';
}

/** The rules as a Frost Script strategy: see `rulesScript`. */
export const rulesSource = (rules: Rules): string => rulesScript(rules).source;

/**
 * The rules as a Frost Script strategy. Each length and level is a setting of the
 * script's, so the optimizer can try others, with where in the rules it came from
 * for its pick to be put back; each value compared is worked out once, and the
 * ones on the price's scale are drawn.
 */
export function rulesScript(rules: Rules): { source: string; places: RulePlace[][] } {
  const lines: string[] = ['// Made with the rule builder. Open it in the code editor to take it further.'];
  const inputs: string[] = [];
  const places: RulePlace[][] = [];
  const values = new Map<string, { name: string; settings: [number, RulePlace['field']][] }>();
  const plots: string[] = [];
  let k = 0;
  let at: Omit<RulePlace, 'field'> = { set: 'long', index: 0, side: 'a' };
  let made: [number, RulePlace['field']][] = [];
  const setting = (name: string, v: number, label: string, fixed: boolean, field: RulePlace['field']) => {
    inputs.push(`input ${name} = ${fixed ? whole(v) : plain(v)}${fixed ? ' min 1 max 5000' : ''} "${label}"`);
    places.push([{ ...at, field }]);
    made.push([places.length - 1, field]);
    return name;
  };
  const value = (o: Operand): string => {
    const key = `${o.kind}:${o.n}:${o.m}:${o.k}`;
    const known = values.get(key);
    if (known) {
      // The same value again: its settings stand for this place too.
      for (const [i, field] of known.settings) places[i]!.push({ ...at, field });
      return known.name;
    }
    const i = OPERAND_INFO[o.kind];
    if (!i) return 'close';
    made = [];
    if (o.kind === 'value') {
      const name = setting(`level${++k}`, o.n, `Level ${k}`, false, 'n');
      values.set(key, { name, settings: made });
      return name;
    }
    if (['close', 'open', 'high', 'low', 'hl2', 'volume'].includes(o.kind)) return o.kind;
    const id = ++k;
    const n = setting(`len${id}`, o.n, `${i.label} ${id} · ${(i.n ?? 'length').toLowerCase()}`, true, 'n');
    const v = `v${id}`;
    const m = i.m ? (o.kind.startsWith('bb') ? setting(`width${id}`, o.m, `${i.label} ${id} · width`, false, 'm') : setting(`slow${id}`, o.m, `${i.label} ${id} · ${i.m.toLowerCase()}`, true, 'm')) : '';
    const sig = i.k ? setting(`signal${id}`, o.k, `${i.label} ${id} · ${i.k.toLowerCase()}`, true, 'k') : '';
    switch (o.kind) {
      case 'sma':
      case 'ema':
      case 'wma':
      case 'vwma':
      case 'rsi':
      case 'cci':
      case 'mfi':
      case 'roc':
        lines.push(`${v} = ${o.kind === 'mfi' ? `mfi(hlc3, ${n})` : `${o.kind}(close, ${n})`}`);
        break;
      case 'macd':
      case 'macdsignal':
      case 'macdhist':
        lines.push(`${v}line = ema(close, ${n}) - ema(close, ${m})`, `${v}signal = ema(${v}line, ${sig})`);
        lines.push(`${v} = ${o.kind === 'macd' ? `${v}line` : o.kind === 'macdsignal' ? `${v}signal` : `${v}line - ${v}signal`}`);
        break;
      case 'bbupper':
      case 'bbbasis':
      case 'bblower':
        lines.push(`${v}basis = sma(close, ${n})`, `${v}dev = stdev(close, ${n})`);
        lines.push(`${v} = ${o.kind === 'bbbasis' ? `${v}basis` : `${v}basis ${o.kind === 'bbupper' ? '+' : '-'} ${m} * ${v}dev`}`);
        break;
      case 'stochk':
      case 'stochd':
        lines.push(`${v}k = sma(stoch(close, high, low, ${n}), ${m})`);
        lines.push(`${v} = ${o.kind === 'stochk' ? `${v}k` : `sma(${v}k, ${sig})`}`);
        break;
      case 'atr':
        lines.push(`${v} = atr(${n})`);
        break;
      case 'highest':
      case 'lowest':
        // The bars before this one: a breakout above them, not above itself.
        lines.push(`${v}all = ${o.kind}(${o.kind === 'highest' ? 'high' : 'low'}, ${n})`, `${v} = ${v}all[1]`);
        break;
      case 'volsma':
        lines.push(`${v} = sma(volume, ${n})`);
        break;
    }
    if (i.price && plots.length < 6) plots.push(`plot(${v}, "${operandLabel(o)}")`);
    values.set(key, { name: v, settings: made });
    return v;
  };
  const condition = (c: Condition, set: keyof Rules, index: number): string => {
    at = { set, index, side: 'a' };
    const a = value(c.a);
    at = { set, index, side: 'b' };
    const b = value(c.b);
    return c.op === 'xa' ? `crossover(${a}, ${b})` : c.op === 'xb' ? `crossunder(${a}, ${b})` : c.op === 'gt' ? `${a} > ${b}` : `${a} < ${b}`;
  };
  const conds = (set: RuleSet, name: string): string | null => {
    if (set.list.length === 0) return null;
    const key = ({ goLong: 'long', goShort: 'short', leaveLong: 'exitLong', leaveShort: 'exitShort' } as const)[name as 'goLong'];
    const parts = set.list.map((c, i) => condition(c, key, i));
    lines.push(`${name} = ${parts.map((p) => (parts.length > 1 ? `(${p})` : p)).join(set.all ? ' and ' : ' or ')}`);
    return name;
  };
  const long = conds(rules.long, 'goLong');
  const short = conds(rules.short, 'goShort');
  const exitLong = conds(rules.exitLong, 'leaveLong');
  const exitShort = conds(rules.exitShort, 'leaveShort');
  lines.push(`long(${long ?? 'false'})`, `short(${short ?? 'false'})`);
  const exits = [exitLong ? `(inlong and ${exitLong})` : null, exitShort ? `(inshort and ${exitShort})` : null].filter(Boolean);
  if (exits.length > 0) lines.push(`exit(${exits.join(' or ')})`);
  return { source: [lines[0]!, ...inputs, ...lines.slice(1), ...plots].join('\n'), places };
}

/** The rules with the optimizer's pick put back where each setting came from. */
export function withSettings(rules: Rules, places: RulePlace[][], inputs: readonly number[]): Rules {
  const next: Rules = JSON.parse(JSON.stringify(rules));
  inputs.forEach((v, i) => {
    for (const p of places[i] ?? []) {
      const c = next[p.set].list[p.index];
      if (c) c[p.side][p.field] = v;
    }
  });
  return next;
}

// ---------------------------------------------------------------- examples

export const EXAMPLES: { id: string; name: string; about: string; source: string }[] = [
  {
    id: 'ema-cross',
    name: 'EMA crossover',
    about: 'Long when the fast average crosses above the slow one, short when it crosses below: always in the market.',
    source: `input fast = 9 min 2 max 200 "Fast"
input slow = 21 min 3 max 500 "Slow"
a = ema(close, fast)
b = ema(close, slow)
long(crossover(a, b))
short(crossunder(a, b))
plot(a, "Fast", blue)
plot(b, "Slow", orange)`,
  },
  {
    id: 'rsi-reversion',
    name: 'RSI reversion',
    about: 'Buys an oversold RSI turning up, sells once it is overbought, with a stop two ATRs under the entry.',
    source: `input length = 14 min 2 max 100 "RSI length"
input oversold = 30 min 1 max 50 "Oversold"
input overbought = 70 min 50 max 99 "Overbought"
input stops = 2 min 0.5 max 10 step 0.5 "Stop, ATRs"
r = rsi(close, length)
range = atr(14)
long(crossover(r, oversold))
exit(r > overbought)
stop(entryprice - stops * range)`,
  },
  {
    id: 'donchian',
    name: 'Donchian breakout',
    about: 'The turtles’ rule: in on a break of the last 20 bars’ high or low, out on a break of the last 10 bars’ against it.',
    source: `input entry = 20 min 2 max 300 "Entry bars"
input exitBars = 10 min 2 max 300 "Exit bars"
up = highest(high, entry)
down = lowest(low, entry)
outUp = highest(high, exitBars)
outDown = lowest(low, exitBars)
long(close > up[1])
short(close < down[1])
exit((inlong and close < outDown[1]) or (inshort and close > outUp[1]))
plot(up[1], "Upper", green)
plot(down[1], "Lower", red)`,
  },
  {
    id: 'bollinger',
    name: 'Bollinger reversion',
    about: 'Buys a close back inside the lower band, sells a close back inside the upper one; out at the basis.',
    source: `input length = 20 min 5 max 200 "Length"
input width = 2 min 0.5 max 4 step 0.1 "Width"
basis = sma(close, length)
dev = stdev(close, length)
upper = basis + width * dev
lower = basis - width * dev
long(crossover(close, lower))
short(crossunder(close, upper))
exit((inlong and close > basis) or (inshort and close < basis))
plot(upper, "Upper", gray)
plot(basis, "Basis", orange)
plot(lower, "Lower", gray)`,
  },
  {
    id: 'macd-trend',
    name: 'MACD with a trend filter',
    about: 'MACD crossing its signal, taken only on the side of the 200 EMA.',
    source: `input fast = 12 min 2 max 100 "Fast"
input slow = 26 min 3 max 200 "Slow"
input signal = 9 min 2 max 50 "Signal"
input trend = 200 min 20 max 500 "Trend EMA"
line = ema(close, fast) - ema(close, slow)
sig = ema(line, signal)
filter = ema(close, trend)
long(crossover(line, sig) and close > filter)
short(crossunder(line, sig) and close < filter)
exit((inlong and crossunder(line, sig)) or (inshort and crossover(line, sig)))
plot(filter, "Trend", purple)`,
  },
  {
    id: 'pullback',
    name: 'Trend pullback, limit entry',
    about: 'In an uptrend, a limit order at the 20 EMA; a 2R target from a stop under the last 10 bars’ low.',
    source: `input fast = 20 min 5 max 200 "Pullback EMA"
input trend = 100 min 20 max 500 "Trend EMA"
input rr = 2 min 0.5 max 10 step 0.5 "Target, R"
e = ema(close, fast)
t = ema(close, trend)
lows = lowest(low, 10)
state risk = na
long(close > t and close > e, e)
if inlong and barsheld == 0 {
  risk = entryprice - lows[1]
}
stop(inlong ? entryprice - risk : na)
target(inlong ? entryprice + rr * risk : na)
plot(e, "Pullback", blue)
plot(t, "Trend", orange)`,
  },
];

// ---------------------------------------------------------------- the report

export const REASONS = ['Open', 'Signal', 'Stop', 'Target', 'Trailing stop', 'Time', 'Liquidation', 'Reversed', 'Range end'] as const;

export interface TradeRow {
  n: number;
  long: boolean;
  entryBar: number;
  entryTime: number;
  entryPrice: number;
  exitBar: number;
  exitTime: number;
  exitPrice: number;
  qty: number;
  margin: number;
  pnl: number;
  fees: number;
  funding: number;
  risk: number;
  reason: number;
  mae: number;
  mfe: number;
  equityBefore: number;
  /** What it made as a share of the equity it was opened from. */
  ret: number;
  /** In multiples of its first stop's loss; NaN with no stop. */
  r: number;
  open: boolean;
}

export function tradesOf(f: Float64Array): TradeRow[] {
  const out: TradeRow[] = [];
  for (let i = 0; i + TRADE_SIZE <= f.length; i += TRADE_SIZE) {
    const t = f.subarray(i, i + TRADE_SIZE);
    const pnl = t[9]!;
    const risk = t[12]!;
    const before = t[16]!;
    out.push({
      n: out.length + 1,
      long: t[0] === 1,
      entryBar: t[1]!,
      entryTime: t[2]!,
      entryPrice: t[3]!,
      exitBar: t[4]!,
      exitTime: t[5]!,
      exitPrice: t[6]!,
      qty: t[7]!,
      margin: t[8]!,
      pnl,
      fees: t[10]!,
      funding: t[11]!,
      risk,
      reason: t[13]!,
      mae: t[14]!,
      mfe: t[15]!,
      equityBefore: before,
      ret: before > 0 ? pnl / before : 0,
      r: risk > 0 ? pnl / risk : Number.NaN,
      open: t[13] === 0,
    });
  }
  return out;
}

export interface Stats {
  trades: number;
  closed: number;
  wins: number;
  losses: number;
  winRate: number;
  start: number;
  end: number;
  startEquity: number;
  endEquity: number;
  net: number;
  netPct: number;
  cagr: number;
  grossProfit: number;
  grossLoss: number;
  profitFactor: number;
  avgWin: number;
  avgLoss: number;
  payoff: number;
  expectancy: number;
  expectancyPct: number;
  avgR: number;
  rTrades: number;
  maxDd: number;
  maxDdPct: number;
  /** The longest the equity stayed under a high, in time. */
  ddLongest: number;
  sharpe: number;
  sortino: number;
  /** How the equity fared against its worst drop: the net return over the deepest drawdown. */
  recovery: number;
  exposure: number;
  avgBars: number;
  winStreak: number;
  lossStreak: number;
  fees: number;
  funding: number;
  buyHoldPct: number;
  largestWin: number;
  largestLoss: number;
  liquidations: number;
  longs: number;
  shorts: number;
  longWinRate: number;
  shortWinRate: number;
}

const mean = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : Number.NaN);
const std = (xs: number[]) => {
  if (xs.length < 2) return Number.NaN;
  const m = mean(xs);
  return Math.sqrt(xs.reduce((a, b) => a + (b - m) ** 2, 0) / (xs.length - 1));
};

/** First bar index at or after `t`. */
function barAt(time: Float64Array, t: number): number {
  let lo = 0;
  let hi = time.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (time[mid]! < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/**
 * The report's figures over the bars from `from` to `to` (bar indexes, the
 * end not included) and the trades entered in them: the whole run, or the part
 * the optimizer tuned on, the part kept back, or the forward test.
 */
export function statsOf(r: BacktestResult & { ok: true }, rows: TradeRow[], fromBar = 0, toBar = r.equity.length): Stats {
  const eq = r.equity;
  const time = r.time;
  const a = Math.max(0, Math.min(fromBar, eq.length - 1));
  const b = Math.max(a + 1, Math.min(toBar, eq.length));
  const list = rows.filter((t) => t.entryBar >= a && t.entryBar < b);
  const closed = list.filter((t) => !t.open);
  const wins = closed.filter((t) => t.pnl > 0);
  const losses = closed.filter((t) => t.pnl <= 0);
  const startEquity = a === 0 ? (list[0]?.equityBefore ?? eq[0] ?? 0) : eq[a - 1]!;
  const first = a === 0 ? Math.max(startEquity, 0) : startEquity;
  const endEquity = eq[b - 1] ?? first;
  const net = endEquity - first;
  const start = time[a] ?? 0;
  const end = time[b - 1] ?? start;
  const years = Math.max((end - start) / (365 * DAY), 1 / 365);
  const grossProfit = wins.reduce((s, t) => s + t.pnl, 0);
  const grossLoss = -losses.reduce((s, t) => s + t.pnl, 0);
  // Drawdown, and its longest stretch under a high.
  let peak = first;
  let peakAt = start;
  let maxDd = 0;
  let maxDdPct = 0;
  let ddLongest = 0;
  for (let i = a; i < b; i++) {
    const v = eq[i]!;
    if (v >= peak) {
      ddLongest = Math.max(ddLongest, time[i]! - peakAt);
      peak = v;
      peakAt = time[i]!;
    } else {
      maxDd = Math.max(maxDd, peak - v);
      if (peak > 0) maxDdPct = Math.max(maxDdPct, (peak - v) / peak);
    }
  }
  ddLongest = Math.max(ddLongest, end - peakAt);
  // Daily returns of the equity, for Sharpe and Sortino: 365 days a year, as crypto trades.
  const daily: number[] = [];
  let lastDay = Math.floor(start / DAY);
  let dayOpen = first;
  for (let i = a; i < b; i++) {
    const d = Math.floor(time[i]! / DAY);
    if (d !== lastDay) {
      const close = eq[i - 1] ?? dayOpen;
      if (dayOpen > 0) daily.push(close / dayOpen - 1);
      dayOpen = close;
      lastDay = d;
    }
  }
  if (dayOpen > 0 && b > a) daily.push(endEquity / dayOpen - 1);
  const sd = std(daily);
  const down = Math.sqrt(daily.reduce((s, x) => s + Math.min(0, x) ** 2, 0) / Math.max(1, daily.length - 1));
  const m = mean(daily);
  const held = list.reduce((s, t) => s + (t.exitBar - t.entryBar + 1), 0);
  let winStreak = 0;
  let lossStreak = 0;
  let run = 0;
  for (const t of closed) {
    run = t.pnl > 0 ? (run > 0 ? run + 1 : 1) : run < 0 ? run - 1 : -1;
    winStreak = Math.max(winStreak, run);
    lossStreak = Math.max(lossStreak, -run);
  }
  const withR = closed.filter((t) => Number.isFinite(t.r));
  const longs = closed.filter((t) => t.long);
  const shorts = closed.filter((t) => !t.long);
  const c0 = r.close[a];
  const c1 = r.close[b - 1];
  return {
    trades: list.length,
    closed: closed.length,
    wins: wins.length,
    losses: losses.length,
    winRate: closed.length ? wins.length / closed.length : Number.NaN,
    start,
    end,
    startEquity: first,
    endEquity,
    net,
    netPct: first > 0 ? net / first : Number.NaN,
    cagr: first > 0 && endEquity > 0 && years >= 30 / 365 ? (endEquity / first) ** (1 / years) - 1 : Number.NaN,
    grossProfit,
    grossLoss,
    profitFactor: grossLoss > 0 ? grossProfit / grossLoss : grossProfit > 0 ? Number.POSITIVE_INFINITY : Number.NaN,
    avgWin: wins.length ? grossProfit / wins.length : Number.NaN,
    avgLoss: losses.length ? -grossLoss / losses.length : Number.NaN,
    payoff: wins.length && losses.length && grossLoss > 0 ? grossProfit / wins.length / (grossLoss / losses.length) : Number.NaN,
    expectancy: closed.length ? (grossProfit - grossLoss) / closed.length : Number.NaN,
    expectancyPct: closed.length ? mean(closed.map((t) => t.ret)) : Number.NaN,
    avgR: withR.length ? mean(withR.map((t) => t.r)) : Number.NaN,
    rTrades: withR.length,
    maxDd,
    maxDdPct,
    ddLongest,
    sharpe: sd > 0 ? (m / sd) * Math.sqrt(365) : Number.NaN,
    sortino: down > 0 ? (m / down) * Math.sqrt(365) : Number.NaN,
    recovery: maxDd > 0 ? net / maxDd : Number.NaN,
    exposure: b > a ? Math.min(1, held / (b - a)) : 0,
    avgBars: list.length ? held / list.length : Number.NaN,
    winStreak,
    lossStreak,
    fees: list.reduce((s, t) => s + t.fees, 0),
    funding: list.reduce((s, t) => s + t.funding, 0),
    buyHoldPct: c0 && c1 ? c1 / c0 - 1 : Number.NaN,
    largestWin: wins.length ? Math.max(...wins.map((t) => t.pnl)) : Number.NaN,
    largestLoss: losses.length ? Math.min(...losses.map((t) => t.pnl)) : Number.NaN,
    liquidations: closed.filter((t) => t.reason === 6).length,
    longs: longs.length,
    shorts: shorts.length,
    longWinRate: longs.length ? longs.filter((t) => t.pnl > 0).length / longs.length : Number.NaN,
    shortWinRate: shorts.length ? shorts.filter((t) => t.pnl > 0).length / shorts.length : Number.NaN,
  };
}

/** The bar a moment falls at: for the split and the forward test. */
export const barOf = (r: BacktestResult & { ok: true }, t: number): number => barAt(r.time, t);

/** The equity's return each calendar month, UTC: `[year, month 0–11, return]`. */
export function monthly(r: BacktestResult & { ok: true }): [number, number, number][] {
  const out: [number, number, number][] = [];
  const eq = r.equity;
  if (eq.length === 0) return out;
  let key = -1;
  let open = eq[0]!;
  let prevClose = open;
  for (let i = 0; i < eq.length; i++) {
    const d = new Date(r.time[i]!);
    const k = d.getUTCFullYear() * 12 + d.getUTCMonth();
    if (key === -1) key = k;
    if (k !== key) {
      out.push([Math.floor(key / 12), key % 12, open > 0 ? prevClose / open - 1 : Number.NaN]);
      open = prevClose;
      key = k;
    }
    prevClose = eq[i]!;
  }
  out.push([Math.floor(key / 12), key % 12, open > 0 ? prevClose / open - 1 : Number.NaN]);
  return out;
}

/** A small seeded generator: the same shuffles each time the report is drawn. */
function seeded(seed: number): () => number {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return (s >>> 0) / 4_294_967_296;
  };
}

export interface MonteCarlo {
  runs: number;
  /** Final returns and deepest drawdowns: the 5th, 50th and 95th percentiles. */
  ret: [number, number, number];
  dd: [number, number, number];
  /** Runs that lost money. */
  losing: number;
}

/**
 * The trades drawn again at random, a thousand times, as many as there were and
 * any of them more than once: how the return and the drawdown could have gone
 * with the same edge and other luck.
 */
export function monteCarlo(rows: TradeRow[], runs = 1000): MonteCarlo | null {
  const rets = rows.filter((t) => !t.open).map((t) => t.ret);
  if (rets.length < 5) return null;
  const rand = seeded(rets.length * 7919 + 17);
  const finals: number[] = [];
  const dds: number[] = [];
  const order = rets.slice();
  for (let k = 0; k < runs; k++) {
    for (let i = 0; i < order.length; i++) order[i] = rets[Math.floor(rand() * rets.length)]!;
    let eq = 1;
    let peak = 1;
    let dd = 0;
    for (const x of order) {
      eq *= 1 + x;
      if (eq > peak) peak = eq;
      else dd = Math.max(dd, (peak - eq) / peak);
    }
    finals.push(eq - 1);
    dds.push(dd);
  }
  finals.sort((x, y) => x - y);
  dds.sort((x, y) => x - y);
  const at = (xs: number[], p: number) => xs[Math.min(xs.length - 1, Math.floor(p * xs.length))]!;
  return { runs, ret: [at(finals, 0.05), at(finals, 0.5), at(finals, 0.95)], dd: [at(dds, 0.05), at(dds, 0.5), at(dds, 0.95)], losing: finals.filter((x) => x < 0).length / runs };
}

/** How the trades' results spread: in R where most had a stop, else in percent of the equity. */
export function distribution(rows: TradeRow[]): { unit: 'R' | '%'; bins: { from: number; to: number; count: number }[] } {
  const closed = rows.filter((t) => !t.open);
  const inR = closed.filter((t) => Number.isFinite(t.r)).length >= closed.length * 0.8 && closed.length > 0;
  const xs = closed.map((t) => (inR ? t.r : t.ret * 100)).filter(Number.isFinite);
  if (xs.length === 0) return { unit: inR ? 'R' : '%', bins: [] };
  // The 2nd to the 98th percentile across, the rest in the end bins: one far winner leaves the others their room.
  const sorted = xs.slice().sort((a, b) => a - b);
  const at = (p: number) => sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))]!;
  const lo = Math.min(0, at(0.02));
  const hi = Math.max(0, at(0.98));
  const want = Math.min(24, Math.max(6, Math.ceil(Math.sqrt(xs.length))));
  const width = hi > lo ? (hi - lo) / want : 1;
  // Edges on zero: each bin wholly a loss or wholly a win.
  const first = Math.floor(lo / width);
  const count = Math.max(1, Math.ceil(hi / width) - first);
  const bins = Array.from({ length: count }, (_, i) => ({ from: (first + i) * width, to: (first + i + 1) * width, count: 0 }));
  for (const x of xs) bins[Math.max(0, Math.min(count - 1, Math.floor(x / width) - first))]!.count += 1;
  return { unit: inR ? 'R' : '%', bins };
}

/** The trades as a spreadsheet's rows. */
export function tradesCsv(rows: TradeRow[], market: string): string {
  const iso = (t: number) => new Date(t).toISOString().replace('.000Z', 'Z');
  const head = ['#', 'Market', 'Side', 'Entry time', 'Entry price', 'Exit time', 'Exit price', 'Quantity', 'Margin', 'P&L', 'Return %', 'R', 'Fees', 'Funding', 'Exit', 'MAE %', 'MFE %', 'Bars'];
  const lines = rows.map((t) =>
    [t.n, market, t.long ? 'Long' : 'Short', iso(t.entryTime), t.entryPrice, t.open ? '' : iso(t.exitTime), t.exitPrice, t.qty, t.margin.toFixed(2), t.pnl.toFixed(2), (t.ret * 100).toFixed(3), Number.isFinite(t.r) ? t.r.toFixed(2) : '', t.fees.toFixed(2), t.funding.toFixed(2), REASONS[t.reason] ?? '', (t.mae * 100).toFixed(2), (t.mfe * 100).toFixed(2), t.exitBar - t.entryBar + 1].join(','),
  );
  return [head.join(','), ...lines].join('\n');
}

/** A grid of values to try for one setting: from, to, by, at most 24. */
export function steps(from: number, to: number, by: number): number[] {
  if (!(by > 0) || !Number.isFinite(from) || !Number.isFinite(to)) return [from];
  const out: number[] = [];
  for (let v = Math.min(from, to); v <= Math.max(from, to) + by * 1e-9 && out.length < 24; v += by) out.push(Number(v.toFixed(8)));
  return out;
}

// ---------------------------------------------------------------- the chart's side

/** What a chart's worker backtests for its tester: null where there is nothing to test. */
export function runOf(t: BacktestState, market: string, script: { source: string; name: string } | undefined): StrategyRun | null {
  const source = t.source === 'rules' ? rulesSource(t.rules) : t.source === 'code' ? t.code : (script?.source ?? '');
  if (!source.trim()) return null;
  const name = t.source === 'rules' ? 'Rules' : t.source === 'code' ? 'Code' : (script?.name ?? 'Script');
  return { name, source, inputs: t.source === 'rules' ? [] : [...t.inputs], settings: settingsOf(t.settings, market, Date.now()), draw: t.draw, funding: t.settings.funding, magnify: t.settings.magnify };
}

/**
 * What a report says to notice, as a bar has just closed: the order waiting for
 * the next open, or a fill at this close. `seen` is the newest bar the last
 * report was of; nothing is said for the first report of a market, or the same bar again.
 */
export function orderNotice(v: BacktestResult, seen: string): { seen: string; title: string; what: string } {
  if (!v.ok || v.time.length === 0) return { seen, title: '', what: '' };
  const n = v.time.length;
  const key = `${v.market}|${v.timeframe}`;
  const bar = `${key}|${v.time[n - 1]}`;
  const title = `${symbolOf(v.market)} ${v.timeframe} · strategy`;
  if (!seen.startsWith(`${key}|`) || seen === bar) return { seen: bar, title, what: '' };
  const last = v.trades.length >= TRADE_SIZE ? v.trades.subarray(v.trades.length - TRADE_SIZE) : null;
  let what = '';
  if (v.next === 1 || v.next === -1) what = `${v.next === 1 ? 'Long' : 'Short'} at the next open${Number.isFinite(v.nextLimit) ? `, limit ${v.nextLimit}` : ''}`;
  else if (v.next === 2) what = 'Exit at the next open';
  else if (last && last[1] === n - 1) what = `${last[0] === 1 ? 'Long' : 'Short'} entered at the close, ${last[3]}`;
  else if (last && last[4] === n - 1 && last[13] !== 0) what = `Closed at ${last[6]}`;
  return { seen: bar, title, what };
}

// ---------------------------------------------------------------- written out

/** `$12,345`, `−$12.40`. */
export function money(v: number): string {
  if (!Number.isFinite(v)) return '—';
  const a = Math.abs(v);
  const digits = a >= 1000 ? 0 : 2;
  return `${v < 0 ? '−' : ''}$${a.toLocaleString('en-US', { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
}

/** A share as a percentage: `+12.34%`, `−3.10%`; unsigned where asked. */
export function pct(f: number, signed = true): string {
  if (!Number.isFinite(f)) return '—';
  const v = Math.abs(f * 100);
  return `${f < 0 ? '−' : signed && f > 0 ? '+' : ''}${v >= 1000 ? Math.round(v).toLocaleString('en-US') : v.toFixed(2)}%`;
}

export function ratio(v: number, digits = 2): string {
  if (v === Number.POSITIVE_INFINITY) return '∞';
  return Number.isFinite(v) ? `${v < 0 ? '−' : ''}${Math.abs(v).toFixed(digits)}` : '—';
}

/** `3d 4h`, `45m`. */
export function span(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '—';
  const m = Math.round(ms / 60_000);
  const d = Math.floor(m / 1440);
  const h = Math.floor((m % 1440) / 60);
  if (d >= 30) return `${d}d`;
  if (d > 0) return `${d}d ${h}h`;
  return h > 0 ? `${h}h ${m % 60}m` : `${m}m`;
}

/** The tester's strategy and account as a link's part: the reader's browser works its report out afresh. */
export function shareOf(s: { source: string; rules: Rules; code: string; script: string; inputs: number[]; settings: BacktestSettings }): string {
  const json = JSON.stringify({ source: s.source, rules: s.rules, code: s.code, script: s.script, inputs: s.inputs, settings: s.settings });
  const bytes = new TextEncoder().encode(json);
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** A link's part back, unchecked: workspace readBacktest checks every value. */
export function sharedOf(text: string): unknown {
  try {
    const bin = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
    return JSON.parse(new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0))));
  } catch {
    return null;
  }
}
