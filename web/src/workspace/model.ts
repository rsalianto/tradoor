/**
 * The workspace: a tree of splits and tab groups, the panels in it, the link
 * groups that make panels follow one market, and theme overrides. Stored whole
 * as one JSON document, locally and on the server.
 *
 * Every operation here mutates a draft; the store clones, applies and
 * reconciles, so the tree functions stay plain and testable.
 */
import { DEFAULT_MARKET, isChartMarket, isMarket } from '../lib/markets';
import { FLAGS, isSection, SECTION_NAME_MAX, sectionName } from '../lib/watchlist';

const isSectionItem = (v: unknown): v is string => typeof v === 'string' && isSection(v) && v.length > 1 && v.length <= SECTION_NAME_MAX + 1;
import { FAVORITE_TIMEFRAMES, isTimeframe, TIMEFRAMES, tfLonger, tfMs } from '../lib/timeframes';

/** What a new panel can be. `tpo` is a preset: a chart drawn as TPO profiles. */
export type PanelType = 'chart' | 'watchlist' | 'tpo' | 'values' | 'liquidations' | 'flow' | 'signals' | 'alerts' | 'dom' | 'tape';

/** How a footprint level reads, named as every footprint chart names them. */
export const FOOTPRINT_MODES: readonly { value: string; label: string }[] = [
  { value: 'delta', label: 'Delta' },
  { value: 'split', label: 'Split' },
  { value: 'total', label: 'Total' },
  { value: 'cluster', label: 'Cluster' },
  { value: 'ladder', label: 'Ladder' },
];

/** What a footprint column carries besides its cells. */
export const FOOTPRINT_EXTRAS: readonly { key: 'footprintValueArea' | 'footprintStats' | 'footprintProfile'; label: string; tip: string }[] = [
  { key: 'footprintValueArea', label: 'Value area', tip: 'Outline the rows holding 70% of a bar’s volume' },
  { key: 'footprintStats', label: 'Stats', tip: 'A table under the bars: each one’s volume, delta, delta %, and the most and least its delta reached, read at each minute’s close' },
  { key: 'footprintProfile', label: 'Profile', tip: 'Every visible bar summed by price, against the right edge' },
];

/**
 * How long a TPO profile runs, as MMT's Session Period names them: a UTC day,
 * Monday-to-Sunday week or calendar month, or a session of each day. The
 * chart's own candles are the brackets, so the timeframe sets how fine a
 * profile is, not how long. A session is hours long: hourly bars at most.
 */
export const TPO_SPLITS: readonly { value: string; label: string; session?: true }[] = [
  { value: '1d', label: 'Daily' },
  { value: '1w', label: 'Weekly' },
  { value: '1M', label: 'Monthly' },
  { value: 'asia', label: 'Asia', session: true },
  { value: 'london', label: 'London', session: true },
  { value: 'ny', label: 'New York', session: true },
  { value: 'intraday', label: 'Intraday', session: true },
  { value: 'custom', label: 'Custom', session: true },
];

/** Whether a chart of `timeframe` bars can stack them into `period` profiles. */
export function tpoFits(period: string, timeframe: string): boolean {
  const split = TPO_SPLITS.find((t) => t.value === period);
  return split ? (split.session ? tfMs(timeframe) <= 3_600_000 : tfLonger(period, timeframe)) : false;
}
export type LinkColor = 'green' | 'blue' | 'red' | 'yellow';

export const LINK_COLORS: readonly LinkColor[] = ['green', 'blue', 'red', 'yellow'];
export const PANEL_TYPES: readonly PanelType[] = ['chart', 'watchlist', 'tpo', 'values', 'dom', 'tape', 'liquidations', 'flow', 'signals', 'alerts'];
export const PANEL_TITLE: Record<PanelType, string> = { chart: 'Chart', watchlist: 'Watchlist', tpo: 'TPO', values: 'Values', liquidations: 'Liquidations', flow: 'Flow', signals: 'Signal', alerts: 'Alerts', dom: 'DOM', tape: 'Tape' };

interface PanelBase {
  id: string;
  /** Linked panels read and write their group's market and timeframe. */
  link: LinkColor | null;
  market: string;
  timeframe: string;
}

export interface ChartPanelState extends PanelBase {
  type: 'chart';
  /** `rsi:14,ema:50` — the spelling the engine parses. */
  studies: string;
  /** Hidden layer ids: ribbon, ema200, volume, or a study id. */
  hidden: string;
  log: boolean;
  /** `price:3,pct:1`. */
  paneWeights: string;
  /** TPO period — see TPO_SPLITS — drawn in place of candles; empty, candles. */
  tpo: string;
  /** Volume by price in place of the candles, where the server records it. */
  footprint: boolean;
  /** How a footprint level reads: `split`, `delta` or `total`. */
  footprintMode: string;
  /** What a footprint counts in: `coins` or `usd`. */
  footprintUnit: string;
  /** Ticks a footprint row covers; 0 fits the rows to the bar. */
  footprintTicks: number;
  /** The rows holding most of a bar's volume, outlined. */
  footprintValueArea: boolean;
  /** The bar's volume and delta, written under it. */
  footprintStats: boolean;
  /** Every visible bar summed by price, against the right edge. */
  footprintProfile: boolean;
  /** TPO options changed from MMT's defaults; see lib/tpo-settings. */
  tpoSettings: Record<string, string>;
  /** Profiles expanded, merged and split by hand, `e=t,t;m=t;s=t`; see lib/tpo-edits. */
  tpoEdits: string;
  /** The layers under the symbol are folded away. */
  legendShut: boolean;
}

export interface WatchlistPanelState extends PanelBase {
  type: 'watchlist';
  /** Markets and section headers (`#Name`) in order; see lib/watchlist. */
  markets: string[];
  /** Sections folded shut, by name. */
  collapsed: string[];
  /** Colour flags by market. */
  flags: Record<string, string>;
}

export interface ValuesPanelState extends PanelBase {
  type: 'values';
  basis: 'volume' | 'tpo';
}

export interface LiquidationsPanelState extends PanelBase {
  type: 'liquidations';
  /** Hits smaller than this are left out of the tape. */
  minUsd: number;
}

/** What the DOM and tape count trades in. */
export type SizeUnit = 'usd' | 'coins';

/** The order book by price, with what traded at each level; see panels/DomPanel. */
export interface DomPanelState extends PanelBase {
  type: 'dom';
  /** Ticks a row spans; 0 picks a size from the price. */
  ticks: number;
  /** Orders smaller than this, in dollars, are left out of what traded. */
  minUsd: number;
  unit: SizeUnit;
  /** Columns left out: `profile`, `pulls`. */
  hidden: string;
  /** Minutes after which Bought and Sold start again, on the clock; 0, never. */
  resetTrades: number;
  /** Minutes after which the volume profile starts again, on the clock — 1440, each UTC day; 0, never. */
  resetProfile: number;
}

/** Every trade as it prints; see panels/TapePanel. */
export interface TapePanelState extends PanelBase {
  type: 'tape';
  /** Orders smaller than this, in dollars, are left out. */
  minUsd: number;
  unit: SizeUnit;
}

export interface FlowPanelState extends PanelBase {
  type: 'flow';
  /** The coin shown, of those the server follows. */
  coin: string;
  /** The coins' order along the toolbar, as dragged; the rest follow in the server's. */
  order: string[];
}

/** Patterns the server's screener found; see panels/SignalsPanel. */
export interface SignalsPanelState extends PanelBase {
  type: 'signals';
  /** Which way: both, or bearish or bullish only. */
  side: 'all' | 'bear' | 'bull';
  /** The patterns now, or the record of how they went. */
  view: 'list' | 'record';
}

export type AlertSort = 'newest' | 'symbol' | 'state';

export interface AlertsPanelState extends PanelBase {
  type: 'alerts';
  tab: 'alerts' | 'log';
  sort: AlertSort;
}

export type Panel = ChartPanelState | WatchlistPanelState | ValuesPanelState | LiquidationsPanelState | FlowPanelState | SignalsPanelState | AlertsPanelState | DomPanelState | TapePanelState;

export interface SplitNode {
  kind: 'split';
  id: string;
  dir: 'row' | 'column';
  /** Fractions of the split, summing to one. */
  sizes: number[];
  children: LayoutNode[];
}

export interface TabsNode {
  kind: 'tabs';
  id: string;
  panels: string[];
  active: string;
}

export type LayoutNode = SplitNode | TabsNode;

export interface LinkState {
  market: string;
  timeframe: string;
}

/** What every chart keeps in step, whatever its link — TradingView's sync in layout. */
export interface SyncState {
  /** One ticker on every chart: change one and they all change. */
  symbol: boolean;
  /** One timeframe on every chart. */
  interval: boolean;
  /** The crosshair on one chart drawn on all of them, at its time. */
  crosshair: boolean;
  /** Every chart scrolled to the time the one moved ends at, each at its own zoom. */
  time: boolean;
}

export interface WorkspaceDoc {
  v: 1;
  name: string;
  /** When this copy last changed, so a browser and the server can tell which is newer. */
  savedAt: number;
  root: LayoutNode;
  panels: Record<string, Panel>;
  links: Record<LinkColor, LinkState>;
  /** Chart theme keys, and `ui.*` for the page's own colours. */
  theme: Record<string, string>;
  /** The timeframes a chart's bar shows as buttons, shortest first; the rest are in its menu. */
  timeframes: string[];
  sync: SyncState;
}

export type DropZone = 'center' | 'left' | 'right' | 'top' | 'bottom';

export const DEFAULT_STUDIES = 'rsi:14,stochrsi:14:14:3:3';
export const DEFAULT_TIMEFRAME = '1h';
/** The coins, then the market around them: the index perpetuals, the VIX, and the crypto market's own measures. */
export const DEFAULT_WATCHLIST = [
  'binance-perp:BTCUSDT',
  'binance-perp:ETHUSDT',
  'binance-perp:SOLUSDT',
  'binance-spot:BTCUSDT',
  'bybit-perp:BTCUSDT',
  '#Macro',
  'hyperliquid-perp:SP500USD',
  'hyperliquid-perp:XYZ100USD',
  'index:VIX',
  'index:TOTAL',
  'index:TOTAL2',
  'index:TOTAL3',
  'index:TOTALES',
  'index:OTHERS',
  'index:BTC_D',
  'index:ETH_D',
  'index:USDT_D',
  'index:OTHERS_D',
];

export const uid = (prefix: string): string => `${prefix}-${Math.random().toString(36).slice(2, 9)}`;

export function newPanel(type: PanelType, from?: Partial<Pick<PanelBase, 'link' | 'market' | 'timeframe'>>): Panel {
  const base = { id: uid(type), link: from?.link ?? null, market: from?.market ?? DEFAULT_MARKET, timeframe: from?.timeframe ?? DEFAULT_TIMEFRAME };
  switch (type) {
    case 'chart':
      return { ...base, type, studies: DEFAULT_STUDIES, hidden: '', log: false, paneWeights: '', tpo: '', footprint: false, footprintMode: 'delta', footprintUnit: 'coins', footprintTicks: 0, footprintValueArea: true, footprintStats: true, footprintProfile: true, tpoSettings: {}, tpoEdits: '', legendShut: false };
    case 'watchlist':
      return { ...base, type, markets: [...DEFAULT_WATCHLIST], collapsed: [], flags: {} };
    case 'tpo':
      // Profiles read cleanest bare, as MMT draws them: no studies under them,
      // and daily profiles of 30-minute brackets, the usual daily TPO.
      return { ...base, timeframe: from?.timeframe ?? '30m', id: uid('chart'), type: 'chart', studies: '', hidden: '', log: false, paneWeights: '', tpo: '1d', footprint: false, footprintMode: 'delta', footprintUnit: 'coins', footprintTicks: 0, footprintValueArea: true, footprintStats: true, footprintProfile: true, tpoSettings: {}, tpoEdits: '', legendShut: false };
    case 'values':
      return { ...base, type, basis: 'volume' };
    case 'liquidations':
      return { ...base, type, minUsd: 0 };
    case 'flow':
      return { ...base, type, coin: 'BTC', order: [] };
    case 'signals':
      return { ...base, type, side: 'all', view: 'list' };
    case 'alerts':
      return { ...base, type, tab: 'alerts', sort: 'newest' };
    case 'dom':
      return { ...base, type, ticks: 0, minUsd: 0, unit: 'usd', hidden: '', resetTrades: 0, resetProfile: 0 };
    case 'tape':
      return { ...base, type, minUsd: 0, unit: 'usd' };
  }
}

/**
 * A chart among several: candles and volume alone, its legend folded. A screen
 * of many charts has no room for a column of indicators down each, nor a
 * pane of RSI under it; any of them is a click away in the legend.
 */
export function bareChart(from?: Partial<Pick<PanelBase, 'link' | 'market' | 'timeframe'>>): ChartPanelState {
  return { ...(newPanel('chart', from) as ChartPanelState), studies: '', hidden: 'ribbon,ema200,liquidations', legendShut: true };
}

/** A group of tabs, the first of them showing. */
export const tabsFor = (first: Panel, ...rest: Panel[]): TabsNode => ({ kind: 'tabs', id: uid('tabs'), panels: [first, ...rest].map((p) => p.id), active: first.id });

/** What a first visit opens: the chart over TPO and Values; the watchlist with flow, signals and alerts behind it; liquidations under them. */
export function defaultWorkspace(): WorkspaceDoc {
  const group = { link: 'green' as const, market: DEFAULT_MARKET, timeframe: DEFAULT_TIMEFRAME };
  const chart = newPanel('chart', group);
  const tpo = newPanel('tpo', group);
  const values = newPanel('values', group);
  const watchlist = newPanel('watchlist', group);
  const flow = newPanel('flow', group);
  const signals = newPanel('signals', group);
  const alerts = newPanel('alerts', group);
  const liquidations = newPanel('liquidations', group);
  const links = {} as Record<LinkColor, LinkState>;
  for (const c of LINK_COLORS) links[c] = { market: DEFAULT_MARKET, timeframe: DEFAULT_TIMEFRAME };
  return {
    v: 1,
    name: 'Default',
    savedAt: 0,
    root: {
      kind: 'split',
      id: uid('split'),
      dir: 'row',
      sizes: [0.8, 0.2],
      children: [
        {
          kind: 'split',
          id: uid('split'),
          dir: 'column',
          sizes: [0.7, 0.3],
          children: [tabsFor(chart), { kind: 'split', id: uid('split'), dir: 'row', sizes: [0.62, 0.38], children: [tabsFor(tpo), tabsFor(values)] }],
        },
        { kind: 'split', id: uid('split'), dir: 'column', sizes: [0.6, 0.4], children: [tabsFor(watchlist, flow, signals, alerts), tabsFor(liquidations)] },
      ],
    },
    panels: Object.fromEntries([chart, tpo, values, watchlist, flow, signals, alerts, liquidations].map((p) => [p.id, p])),
    links,
    theme: {},
    timeframes: [...FAVORITE_TIMEFRAMES],
    sync: { symbol: false, interval: false, crosshair: false, time: false },
  };
}

// ---------------------------------------------------------------- reading stored documents

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, fallback: string): string => (typeof v === 'string' ? v : fallback);
const isLink = (v: unknown): v is LinkColor => LINK_COLORS.includes(v as LinkColor);

function readPanel(id: string, raw: unknown): Panel | null {
  if (!isObj(raw) || !PANEL_TYPES.includes(raw.type as PanelType)) return null;
  const p = newPanel(raw.type as PanelType, {
    link: isLink(raw.link) ? raw.link : null,
    market: isChartMarket(raw.market) ? raw.market : DEFAULT_MARKET,
    timeframe: isTimeframe(raw.timeframe) ? raw.timeframe : DEFAULT_TIMEFRAME,
  });
  p.id = id;
  if (p.type === 'chart') {
    p.studies = str(raw.studies, p.studies);
    p.hidden = str(raw.hidden, '');
    p.log = raw.log === true;
    p.legendShut = raw.legendShut === true;
    p.paneWeights = str(raw.paneWeights, '');
    // A TPO panel from before TPO was a chart style keeps its weekly split, as
    // does a chart saved while TPO was an on/off switch.
    if (raw.type === 'chart') p.tpo = TPO_SPLITS.some((t) => t.value === raw.tpo) ? (raw.tpo as string) : raw.tpo === true ? '1w' : '';
    if (raw.type === 'chart') {
      p.footprint = raw.footprint === true;
      if (FOOTPRINT_MODES.some((m) => m.value === raw.footprintMode)) p.footprintMode = raw.footprintMode as string;
      if (raw.footprintUnit === 'usd') p.footprintUnit = 'usd';
      const ticks = Number(raw.footprintTicks);
      if (Number.isFinite(ticks) && ticks >= 0 && ticks <= 1_000) p.footprintTicks = Math.round(ticks);
      for (const extra of FOOTPRINT_EXTRAS) {
        const on = raw[extra.key];
        if (typeof on === 'boolean') p[extra.key] = on;
      }
    }
    else p.tpo = '1w';
    if (isObj(raw.tpoSettings)) {
      for (const [k, v] of Object.entries(raw.tpoSettings).slice(0, 80)) {
        // The custom sessions are a list; every other value is short.
        if (k.length <= 40 && typeof v === 'string' && v.length <= (k === 'sessions' ? 200 : 40)) p.tpoSettings[k] = v;
      }
    }
    if (typeof raw.tpoEdits === 'string' && raw.tpoEdits.length <= 4_000 && /^[0-9es=m;,-]*$/.test(raw.tpoEdits)) p.tpoEdits = raw.tpoEdits;
  } else if (p.type === 'watchlist') {
    if (Array.isArray(raw.markets)) p.markets = [...new Set(raw.markets.filter((m: unknown) => isMarket(m) || isSectionItem(m)))].slice(0, 200);
    const names = new Set(p.markets.filter(isSection).map(sectionName));
    if (Array.isArray(raw.collapsed)) p.collapsed = raw.collapsed.filter((n: unknown): n is string => typeof n === 'string' && names.has(n));
    if (isObj(raw.flags)) {
      for (const [m, c] of Object.entries(raw.flags)) if (p.markets.includes(m) && typeof c === 'string' && FLAGS.includes(c)) p.flags[m] = c;
    }
  } else if (p.type === 'alerts') {
    if (raw.tab === 'log') p.tab = 'log';
    if (raw.sort === 'symbol' || raw.sort === 'state') p.sort = raw.sort;
  } else if (p.type === 'liquidations' || p.type === 'dom' || p.type === 'tape') {
    const n = Number(raw.minUsd);
    if (Number.isFinite(n) && n >= 0) p.minUsd = n;
    if (p.type !== 'liquidations' && raw.unit === 'coins') p.unit = 'coins';
    if (p.type === 'dom') {
      const ticks = Number(raw.ticks);
      if (Number.isInteger(ticks) && ticks >= 0 && ticks <= 100_000) p.ticks = ticks;
      p.hidden = str(raw.hidden, '').slice(0, 40);
      for (const key of ['resetTrades', 'resetProfile'] as const) {
        const m = Number(raw[key]);
        if (Number.isInteger(m) && m > 0 && m <= 1440) p[key] = m;
      }
    }
  } else if (p.type === 'flow') {
    if (typeof raw.coin === 'string' && /^[A-Z0-9]{1,20}$/.test(raw.coin)) p.coin = raw.coin;
    if (Array.isArray(raw.order)) p.order = [...new Set(raw.order.filter((c: unknown): c is string => typeof c === 'string' && /^[A-Z0-9]{1,20}$/.test(c)))].slice(0, 20);
  } else if (p.type === 'signals') {
    if (raw.side === 'bear' || raw.side === 'bull') p.side = raw.side;
    if (raw.view === 'record') p.view = 'record';
  } else if (p.type === 'values' && raw.basis === 'tpo') {
    p.basis = 'tpo';
  }
  return p;
}

function readNode(raw: unknown, panels: Record<string, Panel>, placed: Set<string>): LayoutNode | null {
  if (!isObj(raw) || typeof raw.id !== 'string') return null;
  if (raw.kind === 'tabs') {
    const ids = Array.isArray(raw.panels) ? raw.panels.filter((p): p is string => typeof p === 'string' && p in panels && !placed.has(p)) : [];
    for (const id of ids) placed.add(id);
    if (ids.length === 0) return null;
    return { kind: 'tabs', id: raw.id, panels: ids, active: typeof raw.active === 'string' && ids.includes(raw.active) ? raw.active : ids[0]! };
  }
  if (raw.kind !== 'split' || !Array.isArray(raw.children)) return null;
  const rawSizes = Array.isArray(raw.sizes) ? raw.sizes : [];
  const children: LayoutNode[] = [];
  const sizes: number[] = [];
  raw.children.forEach((child, i) => {
    const node = readNode(child, panels, placed);
    if (!node) return;
    const size = Number(rawSizes[i]);
    children.push(node);
    sizes.push(Number.isFinite(size) && size > 0 ? size : 1);
  });
  return normalize({ kind: 'split', id: raw.id, dir: raw.dir === 'column' ? 'column' : 'row', sizes, children });
}

/** A stored document made safe to render, or null when it cannot be. */
export function validate(raw: unknown): WorkspaceDoc | null {
  if (!isObj(raw) || raw.v !== 1 || !isObj(raw.panels)) return null;
  const panels: Record<string, Panel> = {};
  for (const [id, p] of Object.entries(raw.panels)) {
    const panel = readPanel(id, p);
    if (panel) panels[id] = panel;
  }
  const placed = new Set<string>();
  const root = readNode(raw.root, panels, placed);
  if (!root) return null;
  for (const id of Object.keys(panels)) if (!placed.has(id)) delete panels[id];
  const links = {} as Record<LinkColor, LinkState>;
  for (const c of LINK_COLORS) {
    const l = isObj(raw.links) ? raw.links[c] : null;
    links[c] = {
      market: isObj(l) && isChartMarket(l.market) ? l.market : DEFAULT_MARKET,
      timeframe: isObj(l) && isTimeframe(l.timeframe) ? l.timeframe : DEFAULT_TIMEFRAME,
    };
  }
  const theme: Record<string, string> = {};
  if (isObj(raw.theme)) for (const [k, v] of Object.entries(raw.theme)) if (typeof v === 'string' && v.length <= 40) theme[k] = v;
  const savedAt = Number(raw.savedAt);
  // Kept in the list's own order; a layout from before favourites has the usual ones.
  const timeframes = Array.isArray(raw.timeframes) ? TIMEFRAMES.map((t) => t.value).filter((tf) => (raw.timeframes as unknown[]).includes(tf)) : [...FAVORITE_TIMEFRAMES];
  const on = (key: keyof SyncState): boolean => isObj(raw.sync) && raw.sync[key] === true;
  const sync = { symbol: on('symbol'), interval: on('interval'), crosshair: on('crosshair'), time: on('time') };
  return { v: 1, name: str(raw.name, '').slice(0, 120) || 'Default', savedAt: Number.isFinite(savedAt) ? savedAt : 0, root, panels, links, theme, timeframes, sync };
}

// ---------------------------------------------------------------- the tree

/** Drop empty tab groups, collapse one-child splits, flatten same-direction ones. */
export function normalize(node: LayoutNode): LayoutNode | null {
  if (node.kind === 'tabs') {
    if (node.panels.length === 0) return null;
    if (!node.panels.includes(node.active)) node.active = node.panels[0]!;
    return node;
  }
  const children: LayoutNode[] = [];
  const sizes: number[] = [];
  node.children.forEach((child, i) => {
    const n = normalize(child);
    if (!n) return;
    const size = node.sizes[i] ?? 1;
    if (n.kind === 'split' && n.dir === node.dir) {
      const total = n.sizes.reduce((a, b) => a + b, 0) || 1;
      n.children.forEach((c, j) => {
        children.push(c);
        sizes.push((size * (n.sizes[j] ?? 1)) / total);
      });
    } else {
      children.push(n);
      sizes.push(size);
    }
  });
  if (children.length === 0) return null;
  if (children.length === 1) return children[0]!;
  const total = sizes.reduce((a, b) => a + b, 0) || 1;
  node.children = children;
  node.sizes = sizes.map((s) => s / total);
  return node;
}

export function findNode(node: LayoutNode, id: string): LayoutNode | null {
  if (node.id === id) return node;
  if (node.kind === 'split') {
    for (const child of node.children) {
      const hit = findNode(child, id);
      if (hit) return hit;
    }
  }
  return null;
}

function findParent(node: LayoutNode, id: string): SplitNode | null {
  if (node.kind !== 'split') return null;
  for (const child of node.children) {
    if (child.id === id) return node;
    const hit = findParent(child, id);
    if (hit) return hit;
  }
  return null;
}

export function tabsOf(node: LayoutNode, panelId: string): TabsNode | null {
  if (node.kind === 'tabs') return node.panels.includes(panelId) ? node : null;
  for (const child of node.children) {
    const hit = tabsOf(child, panelId);
    if (hit) return hit;
  }
  return null;
}

export function firstTabs(node: LayoutNode): TabsNode {
  return node.kind === 'tabs' ? node : firstTabs(node.children[0]!);
}

function insert(doc: WorkspaceDoc, panelId: string, targetId: string | null, zone: DropZone, at?: number): void {
  const found = targetId ? findNode(doc.root, targetId) : null;
  const target = found?.kind === 'tabs' ? found : firstTabs(doc.root);
  if (zone === 'center') {
    target.panels.splice(at === undefined ? target.panels.length : Math.max(0, Math.min(at, target.panels.length)), 0, panelId);
    target.active = panelId;
    return;
  }
  const fresh: TabsNode = { kind: 'tabs', id: uid('tabs'), panels: [panelId], active: panelId };
  const dir = zone === 'left' || zone === 'right' ? 'row' : 'column';
  const before = zone === 'left' || zone === 'top';
  const parent = findParent(doc.root, target.id);
  if (parent && parent.dir === dir) {
    const i = parent.children.indexOf(target);
    const half = (parent.sizes[i] ?? 1) / 2;
    parent.sizes[i] = half;
    parent.children.splice(before ? i : i + 1, 0, fresh);
    parent.sizes.splice(before ? i : i + 1, 0, half);
    return;
  }
  const split: SplitNode = { kind: 'split', id: uid('split'), dir, sizes: [0.5, 0.5], children: before ? [fresh, target] : [target, fresh] };
  if (parent) parent.children[parent.children.indexOf(target)] = split;
  else doc.root = split;
}

export function addPanel(doc: WorkspaceDoc, panel: Panel, targetId: string | null, zone: DropZone): void {
  doc.panels[panel.id] = panel;
  insert(doc, panel.id, targetId, zone);
}

export function removePanel(doc: WorkspaceDoc, panelId: string): void {
  const tabs = tabsOf(doc.root, panelId);
  if (!tabs) return;
  tabs.panels = tabs.panels.filter((p) => p !== panelId);
  delete doc.panels[panelId];
  const root = normalize(doc.root);
  if (root) {
    doc.root = root;
    return;
  }
  // The last panel went: an empty workspace has nothing to click, so start a chart.
  const chart = newPanel('chart');
  doc.panels[chart.id] = chart;
  doc.root = tabsFor(chart);
}

/** `at`: onto a tab strip, the place among its tabs — which reorders a group's own. */
export function movePanel(doc: WorkspaceDoc, panelId: string, targetId: string, zone: DropZone, at?: number): void {
  const source = tabsOf(doc.root, panelId);
  const target = findNode(doc.root, targetId);
  if (!source || target?.kind !== 'tabs') return;
  if (source === target && zone === 'center' && at !== undefined) {
    const from = source.panels.indexOf(panelId);
    source.panels.splice(from, 1);
    source.panels.splice(Math.min(at > from ? at - 1 : at, source.panels.length), 0, panelId);
    source.active = panelId;
    return;
  }
  if (source === target && (zone === 'center' || source.panels.length === 1)) {
    source.active = panelId;
    return;
  }
  source.panels = source.panels.filter((p) => p !== panelId);
  const root = normalize(doc.root);
  if (!root) return;
  doc.root = root;
  insert(doc, panelId, targetId, zone, at);
}

export function activate(doc: WorkspaceDoc, tabsId: string, panelId: string): void {
  const node = findNode(doc.root, tabsId);
  if (node?.kind === 'tabs' && node.panels.includes(panelId)) node.active = panelId;
}

export function setSizes(doc: WorkspaceDoc, splitId: string, sizes: number[]): void {
  const node = findNode(doc.root, splitId);
  if (node?.kind !== 'split' || sizes.length !== node.children.length) return;
  const total = sizes.reduce((a, b) => a + b, 0) || 1;
  node.sizes = sizes.map((s) => s / total);
}
