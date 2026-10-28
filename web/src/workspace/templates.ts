/**
 * Layouts of many charts at once, picked from a list as TradingView offers
 * them: two side by side, a big one over three, four by four. A template lays
 * out the charts; the panels that are not charts keep their groups, in a column
 * at the right.
 *
 * The charts already open fill the cells in the order they stand, keeping all
 * they have; a cell left over gets a new chart on the next ticker of the
 * watchlist, so twenty charts are twenty tickers. Among several, a new chart is
 * bare and every legend folded (see `bareChart`). Charts past the last cell
 * are closed — the caller asks first.
 */
import { isSection } from '../lib/watchlist';
import { bareChart, DEFAULT_TIMEFRAME, DEFAULT_WATCHLIST, newPanel, normalize, uid, type ChartPanelState, type LayoutNode, type Panel, type TabsNode, type WorkspaceDoc } from './model';

/** A template's shape: a cell, or a split of shapes. */
export type Shape = 'cell' | { dir: 'row' | 'column'; sizes?: number[]; of: Shape[] };

export interface Template {
  id: string;
  /** Charts it holds. */
  count: number;
  /** What it is, for the picker's tip. */
  label: string;
  shape: Shape;
}

/** After the watchlist, the coins most traded: a grid of twenty is twenty tickers whatever the list holds. */
const POPULAR = ['XRP', 'DOGE', 'ADA', 'LINK', 'AVAX', 'DOT', 'LTC', 'BNB', 'TRX', 'SUI', 'APT', 'NEAR', 'ARB', 'OP', 'INJ', 'TIA', 'SEI'].map((c) => `binance-perp:${c}USDT`);

const cells = (n: number): Shape[] => Array.from({ length: n }, () => 'cell' as const);
const row = (of: Shape[], sizes?: number[]): Shape => ({ dir: 'row', of, sizes });
const column = (of: Shape[], sizes?: number[]): Shape => ({ dir: 'column', of, sizes });
/** `cols` across and `rows` down. */
const grid = (cols: number, rows: number): Shape => (rows === 1 ? row(cells(cols)) : column(Array.from({ length: rows }, () => (cols === 1 ? 'cell' : row(cells(cols))))));
/** One big chart, and `n` more down its right or along its foot. */
const bigLeft = (n: number): Shape => row(['cell', column(cells(n))], [0.62, 0.38]);
const bigTop = (n: number): Shape => column(['cell', row(cells(n))], [0.62, 0.38]);

export const TEMPLATES: readonly Template[] = [
  { id: '1', count: 1, label: 'One chart', shape: 'cell' },
  { id: '2c', count: 2, label: 'Two side by side', shape: grid(2, 1) },
  { id: '2r', count: 2, label: 'Two, one over the other', shape: grid(1, 2) },
  { id: '3c', count: 3, label: 'Three side by side', shape: grid(3, 1) },
  { id: '3r', count: 3, label: 'Three stacked', shape: grid(1, 3) },
  { id: '3l', count: 3, label: 'One big, two at its right', shape: bigLeft(2) },
  { id: '3t', count: 3, label: 'One big, two under it', shape: bigTop(2) },
  { id: '4g', count: 4, label: 'Two by two', shape: grid(2, 2) },
  { id: '4c', count: 4, label: 'Four side by side', shape: grid(4, 1) },
  { id: '4r', count: 4, label: 'Four stacked', shape: grid(1, 4) },
  { id: '4l', count: 4, label: 'One big, three at its right', shape: bigLeft(3) },
  { id: '4t', count: 4, label: 'One big, three under it', shape: bigTop(3) },
  { id: '6c', count: 6, label: 'Three across, two down', shape: grid(3, 2) },
  { id: '6r', count: 6, label: 'Two across, three down', shape: grid(2, 3) },
  { id: '8c', count: 8, label: 'Four across, two down', shape: grid(4, 2) },
  { id: '8r', count: 8, label: 'Two across, four down', shape: grid(2, 4) },
  { id: '9', count: 9, label: 'Three by three', shape: grid(3, 3) },
  { id: '12c', count: 12, label: 'Four across, three down', shape: grid(4, 3) },
  { id: '12r', count: 12, label: 'Three across, four down', shape: grid(3, 4) },
  { id: '16', count: 16, label: 'Four by four', shape: grid(4, 4) },
  { id: '20c', count: 20, label: 'Five across, four down', shape: grid(5, 4) },
  { id: '20r', count: 20, label: 'Four across, five down', shape: grid(4, 5) },
];

/** The cells of a shape as rectangles in a unit square, in the order charts fill them — for the picker's icons. */
export function cellRects(shape: Shape, x = 0, y = 0, w = 1, h = 1): [number, number, number, number][] {
  if (shape === 'cell') return [[x, y, w, h]];
  const sizes = shape.sizes ?? shape.of.map(() => 1 / shape.of.length);
  const out: [number, number, number, number][] = [];
  let at = 0;
  shape.of.forEach((child, i) => {
    const s = sizes[i]!;
    out.push(...(shape.dir === 'row' ? cellRects(child, x + at * w, y, s * w, h) : cellRects(child, x, y + at * h, w, s * h)));
    at += s;
  });
  return out;
}

/** The tab groups of a tree, left to right and top to bottom. */
function groups(node: LayoutNode): TabsNode[] {
  return node.kind === 'tabs' ? [node] : node.children.flatMap(groups);
}

const isChart = (p: Panel | undefined): p is ChartPanelState => p?.type === 'chart';
const hasChart = (doc: WorkspaceDoc, node: LayoutNode): boolean => groups(node).some((g) => g.panels.some((id) => isChart(doc.panels[id])));

/** Charts the layout would close to take this template. */
export function surplus(doc: WorkspaceDoc, template: Template): number {
  const charts = groups(doc.root).flatMap((g) => g.panels).filter((id) => isChart(doc.panels[id]));
  return Math.max(0, charts.length - template.count);
}

/** Lays the workspace out as `template`; returns how many charts were closed. */
export function applyTemplate(doc: WorkspaceDoc, template: Template): number {
  const all = groups(doc.root);
  const charts = all.flatMap((g) => g.panels).filter((id) => isChart(doc.panels[id]));
  const marketOf = (p: Panel): string => (p.link ? doc.links[p.link].market : p.market);

  // The column at the right: what the layout had there already, when its last
  // column holds no chart, and every other group of side panels under it.
  const side = doc.root.kind === 'split' && doc.root.dir === 'row' && !hasChart(doc, doc.root.children.at(-1)!) ? doc.root.children.at(-1)! : null;
  const sideWidth = side && doc.root.kind === 'split' ? doc.root.sizes.at(-1)! : 0.2;
  const inSide = new Set(side ? groups(side).map((g) => g.id) : []);
  const strays: TabsNode[] = [];
  for (const g of all) {
    if (inSide.has(g.id)) continue;
    const panels = g.panels.filter((id) => !isChart(doc.panels[id]));
    if (panels.length > 0) strays.push({ kind: 'tabs', id: uid('tabs'), panels, active: panels.includes(g.active) ? g.active : panels[0]! });
  }

  // Charts for every cell: those open, then new ones down the watchlist.
  const kept = charts.slice(0, template.count);
  const closed = charts.slice(template.count);
  for (const id of closed) delete doc.panels[id];
  const first = kept[0] ? (doc.panels[kept[0]] as ChartPanelState) : null;
  const timeframe = first ? (first.link ? doc.links[first.link].timeframe : first.timeframe) : DEFAULT_TIMEFRAME;
  const shown = new Set(kept.map((id) => marketOf(doc.panels[id]!)));
  const listed = Object.values(doc.panels).flatMap((p) => (p.type === 'watchlist' ? p.markets : []));
  const next = [...new Set([...listed, ...DEFAULT_WATCHLIST, ...POPULAR])].filter((m) => !isSection(m) && !shown.has(m));
  while (kept.length < template.count) {
    const market = next.shift() ?? marketOf(doc.panels[kept[0] ?? ''] ?? newPanel('chart'));
    const chart = template.count > 1 ? bareChart({ link: null, market, timeframe }) : newPanel('chart', { link: null, market, timeframe });
    doc.panels[chart.id] = chart;
    kept.push(chart.id);
  }
  // Among several, every chart's legend is folded: its indicators stay, out of the way.
  if (template.count > 1) for (const id of kept) (doc.panels[id] as ChartPanelState).legendShut = true;

  let at = 0;
  const build = (shape: Shape): LayoutNode => {
    if (shape === 'cell') {
      const id = kept[at++]!;
      return { kind: 'tabs', id: uid('tabs'), panels: [id], active: id };
    }
    return { kind: 'split', id: uid('split'), dir: shape.dir, sizes: shape.sizes ?? shape.of.map(() => 1 / shape.of.length), children: shape.of.map(build) };
  };
  const chartTree = build(template.shape);

  const sideParts: LayoutNode[] = [...(side ? [side] : []), ...strays];
  let column: LayoutNode | null = null;
  if (sideParts.length === 1) column = sideParts[0]!;
  else if (sideParts.length > 1) {
    // What stood at the right keeps most of it; each group brought over gets a share under it.
    const share = side ? Math.min(0.3, 0.6 / strays.length) : 1 / sideParts.length;
    const sizes = side ? [1 - share * strays.length, ...strays.map(() => share)] : sideParts.map(() => share);
    column = { kind: 'split', id: uid('split'), dir: 'column', sizes, children: sideParts };
  }
  const root: LayoutNode = column ? { kind: 'split', id: uid('split'), dir: 'row', sizes: [1 - sideWidth, sideWidth], children: [chartTree, column] } : chartTree;
  doc.root = normalize(root) ?? chartTree;
  return closed.length;
}
