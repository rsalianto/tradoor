/**
 * Layouts to start from, in the Layouts menu: Classic, the first visit's;
 * Multi-timeframe, one ticker at four timeframes; Order flow, the footprint
 * and its profile with the flow and liquidations under them; and DOM & Tape,
 * a one-minute chart beside the order book and every trade. Picking one lays
 * out the panels; the workspace keeps its theme, favourite timeframes and
 * watchlists.
 */
import { DEFAULT_MARKET } from '../lib/markets';
import { bareChart, defaultWorkspace, LINK_COLORS, newPanel, tabsFor, uid, type ChartPanelState, type LayoutNode, type LinkColor, type LinkState, type Panel, type WatchlistPanelState, type WorkspaceDoc } from './model';

export interface LayoutPreset {
  id: string;
  name: string;
  /** What it is, for the menu. */
  note: string;
  build(): Pick<WorkspaceDoc, 'root' | 'panels' | 'links' | 'sync'>;
}

const split = (dir: 'row' | 'column', sizes: number[], children: LayoutNode[]): LayoutNode => ({ kind: 'split', id: uid('split'), dir, sizes, children });
const links = (green: LinkState): Record<LinkColor, LinkState> => Object.fromEntries(LINK_COLORS.map((c) => [c, c === 'green' ? green : { market: DEFAULT_MARKET, timeframe: '1h' }])) as Record<LinkColor, LinkState>;
const byId = (...panels: Panel[]): Record<string, Panel> => Object.fromEntries(panels.map((p) => [p.id, p]));

export const LAYOUTS: readonly LayoutPreset[] = [
  {
    id: 'classic',
    name: 'Classic',
    note: 'A chart over its TPO profile and values; the watchlist, flow, signals and liquidations beside',
    build: () => {
      const d = defaultWorkspace();
      return { root: d.root, panels: d.panels, links: d.links, sync: d.sync };
    },
  },
  {
    id: 'multi-timeframe',
    name: 'Multi-timeframe',
    note: 'One ticker at 4h, 1h, 15m and 5m, kept on the same ticker; crosshairs in step',
    build: () => {
      // The first chart holds the green link, so the side panels follow the ticker the four share.
      const green = { market: DEFAULT_MARKET, timeframe: '4h' };
      const charts = ['4h', '1h', '15m', '5m'].map((timeframe, i) => ({ ...bareChart({ link: i === 0 ? 'green' : null, market: DEFAULT_MARKET, timeframe }), hidden: 'liquidations' }));
      const at = { link: 'green' as const, ...green };
      const watchlist = newPanel('watchlist', at);
      const flow = newPanel('flow', at);
      const signals = newPanel('signals', at);
      const alerts = newPanel('alerts', at);
      const liquidations = newPanel('liquidations', at);
      const [a, b, c, d] = charts as [ChartPanelState, ChartPanelState, ChartPanelState, ChartPanelState];
      return {
        root: split('row', [0.8, 0.2], [
          split('column', [0.5, 0.5], [split('row', [0.5, 0.5], [tabsFor(a), tabsFor(b)]), split('row', [0.5, 0.5], [tabsFor(c), tabsFor(d)])]),
          split('column', [0.6, 0.4], [tabsFor(watchlist, flow, signals, alerts), tabsFor(liquidations)]),
        ]),
        panels: byId(...charts, watchlist, flow, signals, alerts, liquidations),
        links: links(green),
        sync: { symbol: true, interval: false, crosshair: true, time: false },
      };
    },
  },
  {
    id: 'order-flow',
    name: 'Order flow',
    note: 'A five-minute footprint beside its TPO profile, with net flow and liquidations under them',
    build: () => {
      const green = { market: DEFAULT_MARKET, timeframe: '5m' };
      const at = { link: 'green' as const, ...green };
      const footprint: ChartPanelState = { ...bareChart(at), footprint: true, hidden: 'ribbon,ema200,liquidations' };
      const tpo = { ...(newPanel('tpo', { link: null, market: DEFAULT_MARKET, timeframe: '1h' }) as ChartPanelState), legendShut: true };
      const flow = newPanel('flow', at);
      const liquidations = newPanel('liquidations', at);
      const watchlist = newPanel('watchlist', at);
      const values = newPanel('values', at);
      const signals = newPanel('signals', at);
      const alerts = newPanel('alerts', at);
      return {
        root: split('row', [0.8, 0.2], [
          split('column', [0.7, 0.3], [split('row', [0.64, 0.36], [tabsFor(footprint), tabsFor(tpo)]), split('row', [0.5, 0.5], [tabsFor(flow), tabsFor(liquidations)])]),
          tabsFor(watchlist, values, signals, alerts),
        ]),
        panels: byId(footprint, tpo, flow, liquidations, watchlist, values, signals, alerts),
        links: links(green),
        sync: { symbol: true, interval: false, crosshair: true, time: true },
      };
    },
  },
  {
    id: 'dom-tape',
    name: 'DOM & Tape',
    note: 'A one-minute chart beside the order book and every trade as it prints, with liquidations and flow under the chart',
    build: () => {
      // All on the green link: a new ticker in the chart is a new book and tape.
      const green = { market: DEFAULT_MARKET, timeframe: '1m' };
      const at = { link: 'green' as const, ...green };
      const chart: ChartPanelState = { ...bareChart(at), hidden: 'ribbon,ema200', legendShut: false };
      const dom = newPanel('dom', at);
      const tape = newPanel('tape', at);
      const liquidations = newPanel('liquidations', at);
      const flow = newPanel('flow', at);
      const watchlist = newPanel('watchlist', at);
      const values = newPanel('values', at);
      const signals = newPanel('signals', at);
      const alerts = newPanel('alerts', at);
      return {
        root: split('row', [0.47, 0.31, 0.22], [
          split('column', [0.7, 0.3], [tabsFor(chart), split('row', [0.5, 0.5], [tabsFor(liquidations), tabsFor(flow)])]),
          tabsFor(dom),
          split('column', [0.62, 0.38], [tabsFor(tape), tabsFor(watchlist, values, signals, alerts)]),
        ]),
        panels: byId(chart, dom, tape, liquidations, flow, watchlist, values, signals, alerts),
        links: links(green),
        sync: { symbol: true, interval: false, crosshair: false, time: false },
      };
    },
  },
];

/** Lays the workspace out as `preset`, keeping its theme, favourite timeframes and watchlists. */
export function applyLayout(doc: WorkspaceDoc, preset: LayoutPreset): void {
  const lists = watchlists(doc);
  Object.assign(doc, preset.build(), { name: preset.name });
  keepWatchlists(doc, lists);
}

type List = Pick<WatchlistPanelState, 'markets' | 'collapsed' | 'flags'>;

/** The watchlist panels, in the order the layout shows them. */
function placedWatchlists(doc: WorkspaceDoc): WatchlistPanelState[] {
  const ids = (node: LayoutNode): string[] => (node.kind === 'tabs' ? node.panels : node.children.flatMap(ids));
  return ids(doc.root).flatMap((id) => {
    const p = doc.panels[id];
    return p?.type === 'watchlist' ? [p] : [];
  });
}

/** The watchlists' markets, sections, folds and flags, copied. */
export const watchlists = (doc: WorkspaceDoc): List[] => placedWatchlists(doc).map((p) => ({ markets: [...p.markets], collapsed: [...p.collapsed], flags: { ...p.flags } }));

/**
 * A reader's lists are theirs, not the layout's: whatever layout comes in —
 * a recommended one, a saved one, a file — its watchlists take the lists the
 * one before had, first to first. One it has more of keeps its own.
 */
export function keepWatchlists(doc: WorkspaceDoc, lists: readonly List[]): void {
  placedWatchlists(doc).forEach((p, i) => {
    const list = lists[i];
    if (list) Object.assign(p, { markets: [...list.markets], collapsed: [...list.collapsed], flags: { ...list.flags } });
  });
}
