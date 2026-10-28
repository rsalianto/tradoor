import { assert, assertEquals } from 'jsr:@std/assert';
import { defaultWorkspace, newPanel, type LayoutNode, type WorkspaceDoc } from './model.ts';
import { applyTemplate, cellRects, surplus, TEMPLATES } from './templates.ts';

const template = (id: string) => TEMPLATES.find((t) => t.id === id)!;
const placed = (node: LayoutNode): string[] => (node.kind === 'tabs' ? node.panels : node.children.flatMap(placed));
const charts = (doc: WorkspaceDoc) => placed(doc.root).map((id) => doc.panels[id]!).filter((p) => p.type === 'chart');
const marketOf = (doc: WorkspaceDoc, id: string) => {
  const p = doc.panels[id]!;
  return p.link ? doc.links[p.link].market : p.market;
};

Deno.test('every template has a cell for each of its charts, and they fill the square', () => {
  for (const t of TEMPLATES) {
    const rects = cellRects(t.shape);
    assertEquals(rects.length, t.count, t.id);
    const area = rects.reduce((sum, [, , w, h]) => sum + w * h, 0);
    assert(Math.abs(area - 1) < 1e-9, `${t.id} covers ${area}`);
  }
});

Deno.test('a template lays out the charts and keeps every side panel at the right', () => {
  const doc = defaultWorkspace();
  const sides = Object.values(doc.panels).filter((p) => p.type !== 'chart').map((p) => p.id);
  const closed = applyTemplate(doc, template('4g'));
  assertEquals(closed, 0);
  const shown = charts(doc);
  assertEquals(shown.length, 4, 'the chart and the TPO chart, and two new ones');
  assertEquals(new Set(shown.map((p) => marketOf(doc, p.id))).size, 3, 'the two linked to BTC, and two tickers more');
  for (const id of sides) assert(placed(doc.root).includes(id), `${doc.panels[id]?.type} is still in the layout`);
  assert(doc.root.kind === 'split' && doc.root.dir === 'row', 'charts at the left, a column at the right');
  const right = doc.root.kind === 'split' ? placed(doc.root.children.at(-1)!) : [];
  assert(right.every((id) => doc.panels[id]!.type !== 'chart'), 'no chart in the right column');
  assert(right.includes(sides.find((id) => doc.panels[id]!.type === 'values')!), 'Values, from under the chart, joins the column');
});

Deno.test('new charts take the next tickers of the watchlist, unlinked', () => {
  const doc = defaultWorkspace();
  applyTemplate(doc, template('9'));
  const shown = charts(doc);
  assertEquals(shown.length, 9);
  const fresh = shown.slice(2);
  assert(fresh.every((p) => p.link === null), 'each its own');
  assertEquals(fresh[0]!.market, 'binance-perp:ETHUSDT', 'the first one the watchlist has that is not on screen');
  assertEquals(new Set(fresh.map((p) => p.market)).size, fresh.length, 'no two alike');
});

Deno.test('among several, a new chart is bare and every legend folded', () => {
  const doc = defaultWorkspace();
  const first = charts(doc)[0]!;
  assertEquals(first.studies !== '' && first.legendShut, false, 'the default chart starts with its indicators, legend open');
  applyTemplate(doc, template('4g'));
  const shown = charts(doc);
  assert(shown.every((p) => p.legendShut), 'every legend folded');
  assertEquals(shown[0]!.studies, first.studies, 'a chart that had indicators keeps them');
  for (const fresh of shown.slice(2)) {
    assertEquals(fresh.studies, '', 'no RSI pane under a new one');
    assertEquals(fresh.hidden, 'ribbon,ema200,liquidations', 'candles and volume alone');
  }
});

Deno.test('one chart alone keeps its legend as it was', () => {
  const doc = defaultWorkspace();
  applyTemplate(doc, template('1'));
  assertEquals(charts(doc)[0]!.legendShut, false);
});

Deno.test('twenty charts are twenty tickers, past the end of the watchlist', () => {
  const doc = defaultWorkspace();
  applyTemplate(doc, template('20c'));
  const fresh = charts(doc).slice(2).map((p) => p.market);
  assertEquals(fresh.length, 18);
  assertEquals(new Set(fresh).size, 18, 'none of the new ones alike');
  assert(!fresh.includes('binance-perp:BTCUSDT'), 'nor the one the first two show');
});

Deno.test('charts past the last cell are closed, and counted first', () => {
  const doc = defaultWorkspace();
  applyTemplate(doc, template('20c'));
  assertEquals(charts(doc).length, 20);
  assertEquals(surplus(doc, template('3l')), 17);
  assertEquals(applyTemplate(doc, template('3l')), 17);
  assertEquals(charts(doc).length, 3);
  assertEquals(Object.values(doc.panels).filter((p) => p.type === 'chart').length, 3, 'the closed ones are gone from the panels too');
});

Deno.test('a layout of charts alone stays charts alone', () => {
  const doc = defaultWorkspace();
  const only = newPanel('chart');
  doc.panels = { [only.id]: only };
  doc.root = { kind: 'tabs', id: 'tabs-only', panels: [only.id], active: only.id };
  applyTemplate(doc, template('2c'));
  const root = doc.root as LayoutNode;
  assert(root.kind === 'split' && root.dir === 'row' && root.children.length === 2, 'two charts, no side column');
});
