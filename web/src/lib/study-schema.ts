/**
 * The indicators as a reader meets them: what each is called, where the picker
 * files it, and how its settings ask for its inputs and styles. Loaded with the
 * picker and the settings dialog; the engine's own list (tradoor-core studies)
 * holds only what it computes with.
 */
import { OI_VENUES } from '../engine/open-interest';
import type { StudyDefinition, StudyLine, StudyParam } from '../engine/protocol';

export const CATEGORIES = ['Trend and averages', 'VWAP', 'Momentum', 'Volatility', 'Levels and structure', 'Flow and positioning', 'Time and cycles', 'Built in'] as const;
export type Category = (typeof CATEGORIES)[number];

interface Described {
  name: string;
  about: string;
  category: Category;
  /** What else a reader might type to find it. */
  keywords: string;
}

const STUDIES: Record<string, Described> = {
  ema: { name: 'Exponential Moving Average', category: 'Trend and averages', keywords: 'ma moving', about: 'Average close, weighted towards recent bars.' },
  sma: { name: 'Simple Moving Average', category: 'Trend and averages', keywords: 'ma moving', about: 'Plain average of the last N closes: every bar in the window counts the same.' },
  ichimoku: { name: 'Ichimoku Cloud', category: 'Trend and averages', keywords: 'kumo tenkan kijun senkou chikou', about: 'Conversion and base lines at the bar, a cloud between two spans carried ahead of it, and price carried behind it.' },
  vwap: { name: 'Volume Weighted Average Price', category: 'VWAP', keywords: 'session bands deviation', about: 'Average price paid since the session opened, each trade weighted by its size, with bands at standard deviations of price around it. Sessions are UTC days.' },
  avwap: { name: 'Anchored VWAP (S/W/M/Y)', category: 'VWAP', keywords: 'multi session weekly monthly yearly', about: 'Volume-weighted average price since the session, week, month and year opened. The session line is drawn on intraday charts only.' },
  rsi: { name: 'Relative Strength Index', category: 'Momentum', keywords: 'oscillator overbought oversold', about: 'Momentum bounded 0–100. Wilder smoothing, so the values match TradingView exactly.' },
  stochrsi: { name: 'Stochastic RSI', category: 'Momentum', keywords: 'oscillator overbought oversold', about: 'Where RSI sits inside its own recent range. Turns earlier than RSI, and whipsaws more.' },
  macd: { name: 'MACD', category: 'Momentum', keywords: 'moving average convergence divergence histogram oscillator', about: 'The gap between two EMAs, and an EMA of that gap. The histogram is the difference between them.' },
  bb: { name: 'Bollinger Bands', category: 'Volatility', keywords: 'deviation squeeze', about: 'An SMA with a band at N standard deviations, shaded between them. Narrow bands mark the quiet before an expansion.' },
  keltner: { name: 'Keltner Channels', category: 'Volatility', keywords: 'kc atr bands channel squeeze', about: 'An EMA with a band at a multiple of ATR, shaded between them. Smoother than Bollinger Bands, as it follows range rather than closes.' },
  atr: { name: 'Average True Range', category: 'Volatility', keywords: 'stop distance', about: 'Average range of a bar, in the instrument’s own price units. A stop distance, not a direction.' },
  keylevels: { name: 'Historical Key Levels', category: 'Levels and structure', keywords: 'weekly monthly yearly open monday high low', about: 'Weekly, monthly and yearly opens plus Monday’s high and low: the prices the week is measured against.' },
  vpvr: { name: 'Volume Profile, Visible Range', category: 'Levels and structure', keywords: 'vp vrvp vpvr poc value area vah val', about: 'Volume by price over the bars on screen, split by whether each bar closed up or down, with its point of control and value area. It redraws as you scroll. Each bar’s volume is spread evenly over its range, so the levels are zones.' },
  swingmap: { name: 'Range Swing Structure Map', category: 'Levels and structure', keywords: 'pivot zigzag hh hl lh ll market structure', about: 'Zigzag of confirmed pivots tagged HH/HL/LH/LL, with each leg’s move and duration, and the averages across the window.' },
  flow: { name: 'Net Taker Flow Marks', category: 'Flow and positioning', keywords: 'delta orderflow buyers sellers', about: 'Minutes whose net taker flow ran hard one way, marked where they traded, the dot larger the harder it ran. What counts as hard is the size set per market in the Flow panel, and the server keeps two days of them.' },
  volumedelta: { name: 'Volume Delta', category: 'Flow and positioning', keywords: 'cvd cumulative taker buyers sellers orderflow', about: 'What market buyers took less what market sellers did, in coins, bar by bar or added up. Exact where the exchange reports its buyers, as Binance does. Any other bar is estimated from where it closed in its range, and the legend says how many are.' },
  oi: { name: 'Aggregated Open Interest', category: 'Flow and positioning', keywords: 'oi positioning derivatives perpetuals', about: 'Open interest summed across Binance, Bybit, OKX, Deribit and Hyperliquid perpetuals, stablecoin- and coin-margined. Recorded by the tradoor server.' },
  sessions: { name: 'Trading Sessions', category: 'Time and cycles', keywords: 'asia london new york ny killzone range', about: 'Each day’s Asia, London and New York sessions boxed from high to low, with London and New York following their summer time. Needs bars of an hour or less.' },
  moon: { name: 'Moon Phases', category: 'Time and cycles', keywords: 'full new lunar', about: 'A disc over the bar a full moon falls in, a ring under the bar a new moon does.' },
};

/** One number an owner takes, kept in its styles rather than in its id. */
export interface NumberInput {
  /** Whose it is, where not the study's or layer's own: RSI's guides are the pane's. */
  owner?: string;
  key: string;
  label: string;
  default: number;
  min: number;
  max: number;
  step?: number;
  hint?: string;
}

/** One row of the Style tab. */
export interface StylePlot {
  owner?: string;
  key: string;
  label: string;
  /** The colour it draws in unless changed: a theme entry's and an opacity, or its own. */
  theme?: string;
  alpha?: string;
  color?: string;
  width?: number;
  /** Shows or hides, and has nothing else to set. */
  onlyToggle?: boolean;
  noToggle?: boolean;
  noWidth?: boolean;
  /** Under a row with a box to tick, and lined up with it. */
  indent?: boolean;
}

export interface LayerDef extends Described {
  id: string;
  short: string;
  /** Drawn with the candles, so not on a chart of TPO profiles. */
  candles: boolean;
  inputs: readonly NumberInput[];
  plots: readonly StylePlot[];
}

export const LAYER_DEFS: readonly LayerDef[] = [
  {
    id: 'ribbon',
    name: 'EMA ribbon',
    short: 'EMA 12/25',
    category: 'Built in',
    keywords: 'cloud moving average ma cross bull bear trend',
    about: 'The band between a fast and a slow EMA, coloured by which is on top, with a mark where they cross.',
    candles: true,
    inputs: [
      { key: 'fast', label: 'Fast length', default: 12, min: 2, max: 1000 },
      { key: 'slow', label: 'Slow length', default: 25, min: 2, max: 1000 },
    ],
    plots: [
      { key: 'bull', label: 'Fast above slow', theme: 'ribbonFast', alpha: '33', noWidth: true },
      { key: 'bear', label: 'Fast below slow', theme: 'ribbonSlow', alpha: '33', noWidth: true },
      { key: 'marks', label: 'Cross marks', onlyToggle: true },
    ],
  },
  {
    id: 'ema200',
    name: 'Long EMA',
    short: 'EMA 200',
    category: 'Built in',
    keywords: 'moving average ma trend',
    about: 'One long average, with its value on the price axis.',
    candles: true,
    inputs: [{ key: 'length', label: 'Length', default: 200, min: 2, max: 1000 }],
    plots: [{ key: 'line', label: 'Line', theme: 'ema200', width: 1, noToggle: true }],
  },
  {
    id: 'volume',
    name: 'Volume',
    short: 'Vol',
    category: 'Built in',
    keywords: 'bars',
    about: 'Each bar’s volume along the foot of the price pane, against the busiest bar on screen.',
    candles: true,
    inputs: [{ key: 'height', label: 'Height, % of pane', default: 18, min: 5, max: 60 }],
    plots: [
      { key: 'up', label: 'Up bars', theme: 'bull', alpha: '40', noToggle: true, noWidth: true },
      { key: 'down', label: 'Down bars', theme: 'bear', alpha: '40', noToggle: true, noWidth: true },
    ],
  },
  {
    id: 'liquidations',
    name: 'Liquidations',
    short: 'Liqs',
    category: 'Built in',
    keywords: 'feed long short forced',
    about: 'Positions force-closed in this market, as they happen, stacked in the corner of the price pane.',
    candles: false,
    inputs: [{ key: 'min', label: 'Smallest shown, USD', default: 0, min: 0, max: 1_000_000_000, step: 1000 }],
    plots: [
      { key: 'long', label: 'Longs closed out', theme: 'bear', noToggle: true, noWidth: true },
      { key: 'short', label: 'Shorts closed out', theme: 'bull', noToggle: true, noWidth: true },
    ],
  },
];

export interface Entry extends Described {
  id: string;
  short: string;
  /** Where it draws, as the picker says it. */
  where: string;
  def?: StudyDefinition;
  layer?: LayerDef;
}

/** The picker's list: what the engine computes, then what every chart has built in. */
export function entries(defs: readonly StudyDefinition[]): Entry[] {
  return [
    ...defs.map((def): Entry => {
      const own = STUDIES[def.kind] ?? { name: def.short, about: '', category: 'Built in' as const, keywords: '' };
      return { ...own, id: def.kind, short: def.short, where: def.group === 'price' ? 'on price' : 'own pane', def };
    }),
    ...LAYER_DEFS.map((layer): Entry => ({ ...layer, where: 'on price', layer })),
  ];
}

/** The search: everything typed must appear, the closest matches first. */
export function searchEntries<T extends Pick<Entry, 'id' | 'name' | 'short' | 'keywords' | 'category'>>(list: readonly T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...list];
  const words = q.split(/\s+/);
  const rank = (e: T): number => {
    const [short, name] = [e.short.toLowerCase(), e.name.toLowerCase()];
    const text = `${name} ${short} ${e.id} ${e.keywords} ${e.category.toLowerCase()}`;
    if (!words.every((w) => text.includes(w))) return -1;
    if (short === q || e.id === q) return 0;
    if (short.startsWith(q) || name.startsWith(q)) return 1;
    return text.split(/[\s/()-]+/).some((w) => w.startsWith(q)) ? 2 : 3;
  };
  return list
    .map((e) => ({ e, r: rank(e) }))
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r)
    .map((x) => x.e);
}

export const defaultId = (def: StudyDefinition): string => [def.kind, ...def.params.map((p) => p.default)].join(':');

const ANOTHER_AVERAGE = [50, 200, 20, 100, 9, 21, 34, 55, 89, 144, 233];

/** One more of a study already on the chart: a study's identity is its parameters, so its first takes a value not yet there. Null where there is none to pick. */
export function anotherId(def: StudyDefinition, active: readonly string[]): string | null {
  const first = def.params[0];
  if (!first || def.kind === 'oi' || def.kind === 'vpvr') return null;
  const rest = def.params.slice(1).map((p) => p.default);
  const values = def.kind === 'vwap' ? [1, 2, 3] : def.kind === 'volumedelta' ? [0, 1] : def.kind === 'ema' || def.kind === 'sma' ? ANOTHER_AVERAGE : [2, 3, 4, 5, 6, 7, 8].map((k) => first.default * k);
  for (const v of values) {
    const id = [def.kind, v, ...rest].join(':');
    if (v <= first.max && !active.includes(id)) return id;
  }
  return null;
}

/** What the picker writes for one on the chart: its parameters, or nothing where they would not say which. */
export function instanceLabel(id: string): string {
  const [kind, ...params] = id.split(':');
  return params.length === 0 || ['vwap', 'oi', 'vpvr', 'volumedelta'].includes(kind!) ? 'On chart' : params.join(' ');
}

// ---------------------------------------------------------------- inputs

export interface ParamUi {
  label: string;
  hint?: string;
  options?: readonly string[];
  /** A band: on with this multiple, off at 0. */
  band?: number;
  /** One checkbox per bit, with these names. */
  bits?: readonly string[];
}

const OI_LABELS: Record<(typeof OI_VENUES)[number], string> = {
  'binance-usdt': 'Binance USDT Perp',
  'bybit-usdt': 'Bybit USDT Perp',
  'okx-usdt': 'OKX USDT Swap',
  'deribit-usd': 'Deribit USD Perp',
  'hyperliquid-usd': 'Hyperliquid USD Perp',
  'binance-usd': 'Binance USD Perp',
  'bybit-usd': 'Bybit USD Perp',
  'okx-usd': 'OKX USD Perp',
};

const PARAMS: Record<string, Record<string, ParamUi>> = {
  bb: { mult: { label: 'Multiplier', hint: 'Standard deviations from the average to each band' } },
  keltner: { mult: { label: 'Multiplier', hint: 'ATRs from the average to each band' }, atr: { label: 'ATR length' } },
  ichimoku: { conversion: { label: 'Conversion length' }, base: { label: 'Base length' }, span: { label: 'Leading span B length' }, shift: { label: 'Displacement', hint: 'How far the spans run ahead and price runs behind, counted as TradingView counts it: 26 moves them 25 bars' } },
  vpvr: { rows: { label: 'Rows' }, va: { label: 'Value area, %' }, width: { label: 'Width, % of pane' }, side: { label: 'Side', options: ['Right', 'Left'] } },
  volumedelta: { view: { label: 'View', options: ['Each bar', 'Cumulative'] } },
  stochrsi: { rsi: { label: 'RSI length' }, stoch: { label: 'Stochastic length' }, k: { label: '%K smoothing' }, d: { label: '%D smoothing' } },
  macd: { fast: { label: 'Fast length' }, slow: { label: 'Slow length' }, signal: { label: 'Signal length' } },
  swingmap: { left: { label: 'Left bars', hint: 'Bars a pivot must stand above on its left' }, right: { label: 'Right bars', hint: 'Bars to its right before it is confirmed' } },
  vwap: {
    anchor: { label: 'Anchor', options: ['Session', 'Week', 'Month', 'Year'], hint: 'Where the average starts again. A session is a UTC day, and needs bars shorter than a day' },
    band1: { label: 'Bands 1', band: 1, hint: 'Standard deviations of price around the VWAP' },
    band2: { label: 'Bands 2', band: 2 },
    band3: { label: 'Bands 3', band: 3 },
  },
  oi: {
    venues: { label: 'Venues', bits: OI_VENUES.map((v) => OI_LABELS[v]) },
    measure: { label: 'Measure', options: ['Coins', 'USD'] },
    view: { label: 'View', options: ['Candles', 'Line'] },
  },
};

const titled = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);
export const paramUi = (kind: string, param: StudyParam): ParamUi => PARAMS[kind]?.[param.name] ?? { label: titled(param.name) };

/** RSI and Stochastic RSI share a pane, and it one pair of guides. */
const PANE_GUIDES: readonly NumberInput[] = [
  { owner: 'pct', key: 'upper', label: 'Upper guide', default: 80, min: 0, max: 100, hint: 'Shared by every study in this pane' },
  { owner: 'pct', key: 'lower', label: 'Lower guide', default: 20, min: 0, max: 100, hint: 'Shared by every study in this pane' },
];
const GUIDES_PLOT: StylePlot = { owner: 'pct', key: 'guides', label: 'Guides', theme: 'border', width: 1 };

export const studyNumbers = (kind: string): readonly NumberInput[] => (kind === 'rsi' || kind === 'stochrsi' ? PANE_GUIDES : []);
/** The owners besides a study's own that its settings change. */
export const otherOwners = (kind: string): readonly string[] => (kind === 'rsi' || kind === 'stochrsi' ? ['pct'] : []);

/** Written under the inputs, where the description does not say it. */
export const NOTES: Record<string, string> = {
  oi: 'Recorded by the tradoor server every 15 seconds. Binance and Bybit are backfilled for the last month and OKX for five days. Before a venue’s own record begins, it is estimated at the share it held when it began.',
};

// ---------------------------------------------------------------- styles

const LABELS: Record<string, Record<string, string>> = {
  keylevels: { wo: 'Weekly open', mo: 'Monthly open', yo: 'Yearly open', monH: 'Monday high', monL: 'Monday low' },
  macd: { histogram: 'Histogram, above zero' },
  vwap: { upper1: 'Upper band 1', lower1: 'Lower band 1', upper2: 'Upper band 2', lower2: 'Lower band 2', upper3: 'Upper band 3', lower3: 'Lower band 3' },
  swingmap: { zigzag: 'Zigzag' },
  volumedelta: { histogram: 'Delta, above zero', cvd: 'Cumulative delta' },
  ichimoku: { conversion: 'Conversion line', base: 'Base line', lagging: 'Lagging span', leadA: 'Leading span A', leadB: 'Leading span B' },
};

const LINE_STYLE: Record<string, Record<string, Partial<StylePlot>>> = {
  macd: { histogram: { theme: 'bull', alpha: '80', noWidth: true } },
  volumedelta: { histogram: { theme: 'bull', alpha: '80', noWidth: true } },
};

/** `rgba(r,g,b,a)`, as the engine writes a colour, in the spelling the colour field takes. */
export function toHex(css: string): string {
  const m = /^rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)$/.exec(css);
  if (!m) return css;
  const hex = (n: number) => n.toString(16).padStart(2, '0');
  const alpha = Math.round(Number(m[4] ?? 1) * 255);
  return `#${hex(Number(m[1]))}${hex(Number(m[2]))}${hex(Number(m[3]))}${alpha === 255 ? '' : hex(alpha)}`;
}

/** A study's rows on the Style tab: the lines it draws, and what else it paints. */
export function studyPlots(id: string, lines: readonly StudyLine[]): StylePlot[] {
  const kind = id.split(':')[0]!;
  const out: StylePlot[] = lines
    // A width of 0 only carries values for the legend; key levels draw theirs as segments.
    .filter((l) => l.width > 0 || kind === 'keylevels')
    .map((l) => {
      const key = l.id.slice(id.length + 1);
      return { key, label: LABELS[kind]?.[key] ?? l.label, color: toHex(l.color), width: l.width || 1, ...LINE_STYLE[kind]?.[key] };
    });
  if (kind === 'macd') out.splice(1, 0, { key: 'histogramDown', label: 'Histogram, below zero', theme: 'bear', alpha: '80', noToggle: true, noWidth: true, indent: true });
  if (kind === 'volumedelta' && out.some((p) => p.key === 'histogram')) out.push({ key: 'histogramDown', label: 'Delta, below zero', theme: 'bear', alpha: '80', noToggle: true, noWidth: true, indent: true });
  if (kind === 'bb' || kind === 'keltner') out.push({ key: 'fill', label: 'Background', color: '#2196f31a', noWidth: true });
  if (kind === 'ichimoku') out.push({ key: 'cloudUp', label: 'Cloud, A above B', color: '#43a04733', noWidth: true }, { key: 'cloudDown', label: 'Cloud, A below B', color: '#f4433633', noWidth: true });
  if (kind === 'sessions') out.push({ key: 'asia', label: 'Asia', color: '#e3cd2a', noWidth: true }, { key: 'london', label: 'London', color: '#c8202e', noWidth: true }, { key: 'ny', label: 'New York', color: '#2f5bd8', noWidth: true });
  if (kind === 'moon') out.push({ key: 'full', label: 'Full moon', color: '#f5efc8', noWidth: true }, { key: 'new', label: 'New moon', color: '#9aa0a6', noWidth: true });
  if (kind === 'vpvr') out.push({ key: 'up', label: 'Bars that closed up', theme: 'bull', noWidth: true }, { key: 'down', label: 'Bars that closed down', theme: 'bear', noWidth: true }, { key: 'poc', label: 'Point of control', theme: 'tpoPoc', width: 1 }, { key: 'va', label: 'Value area edges', theme: 'text', width: 1 });
  if (kind === 'swingmap') out.push({ key: 'labels', label: 'Swing labels', onlyToggle: true }, { key: 'stats', label: 'Averages in the legend', onlyToggle: true });
  if (kind === 'flow') out.push({ key: 'bought', label: 'Bought', theme: 'bull', noWidth: true }, { key: 'sold', label: 'Sold', theme: 'bear', noWidth: true });
  if (kind === 'oi' && id.split(':')[3] !== '1') out.push({ key: 'up', label: 'Rising', theme: 'bull', noToggle: true, noWidth: true }, { key: 'down', label: 'Falling', theme: 'bear', noToggle: true, noWidth: true });
  if (kind === 'rsi' || kind === 'stochrsi') out.push(GUIDES_PLOT);
  return out;
}
