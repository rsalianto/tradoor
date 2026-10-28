import { assert, assertEquals } from 'jsr:@std/assert';
import { defaultWorkspace, validate, type LayoutNode, type WorkspaceDoc } from './model.ts';
import { applyLayout, LAYOUTS } from './presets.ts';

const placed = (node: LayoutNode): string[] => (node.kind === 'tabs' ? node.panels : node.children.flatMap(placed));
const preset = (id: string) => LAYOUTS.find((l) => l.id === id)!;
const charts = (doc: WorkspaceDoc) => placed(doc.root).map((id) => doc.panels[id]!).filter((p) => p.type === 'chart');

Deno.test('every recommended layout is a workspace that reads back whole', () => {
  for (const l of LAYOUTS) {
    const doc = defaultWorkspace();
    applyLayout(doc, l);
    const back = validate(JSON.parse(JSON.stringify(doc)));
    assert(back, `${l.id} reads back`);
    assertEquals(placed(back.root).sort(), Object.keys(doc.panels).sort(), `${l.id}: every panel placed, none lost`);
    assertEquals(back.sync, doc.sync, `${l.id}: its sync switches kept`);
    assertEquals(back.name, l.name);
  }
});

Deno.test('a layout keeps the theme and the favourite timeframes', () => {
  const doc = defaultWorkspace();
  doc.theme = { bull: '#123456' };
  doc.timeframes = ['5m', '1h'];
  applyLayout(doc, preset('order-flow'));
  assertEquals(doc.theme, { bull: '#123456' });
  assertEquals(doc.timeframes, ['5m', '1h']);
});

Deno.test('multi-timeframe is one ticker at four timeframes, kept on the same ticker', () => {
  const doc = defaultWorkspace();
  applyLayout(doc, preset('multi-timeframe'));
  const shown = charts(doc);
  const tf = (p: (typeof shown)[number]) => (p.link ? doc.links[p.link].timeframe : p.timeframe);
  assertEquals(shown.map(tf), ['4h', '1h', '15m', '5m']);
  assertEquals(doc.sync.symbol && !doc.sync.interval, true, 'the ticker in step, each its timeframe');
});

Deno.test('order flow leads with a footprint and its profile', () => {
  const doc = defaultWorkspace();
  applyLayout(doc, preset('order-flow'));
  const [footprint, tpo] = charts(doc);
  assert(footprint?.footprint, 'a footprint first');
  assertEquals(tpo?.tpo, '1d', 'then TPO by day');
  assert(Object.values(doc.panels).some((p) => p.type === 'flow') && Object.values(doc.panels).some((p) => p.type === 'liquidations'));
});

Deno.test('the watchlists stay the reader\'s whatever layout is picked', () => {
  const doc = defaultWorkspace();
  const list = Object.values(doc.panels).find((p) => p.type === 'watchlist')!;
  if (list.type !== 'watchlist') throw new Error('no watchlist');
  list.markets = ['#Mine', 'bybit-perp:XRPUSDT', 'okx-perp:DOGEUSDT'];
  list.collapsed = ['Mine'];
  list.flags = { 'bybit-perp:XRPUSDT': 'red' };
  for (const l of LAYOUTS) {
    applyLayout(doc, l);
    const shown = placed(doc.root).map((id) => doc.panels[id]!).filter((p) => p.type === 'watchlist');
    assert(shown.length > 0, `${l.id} has a watchlist`);
    const first = shown[0]!;
    if (first.type !== 'watchlist') throw new Error('not a watchlist');
    assertEquals([first.markets, first.collapsed, first.flags], [['#Mine', 'bybit-perp:XRPUSDT', 'okx-perp:DOGEUSDT'], ['Mine'], { 'bybit-perp:XRPUSDT': 'red' }], `${l.id} keeps the list`);
  }
});

Deno.test('dom & tape puts the book and every trade beside a one-minute chart, all on one ticker', () => {
  const doc = defaultWorkspace();
  applyLayout(doc, preset('dom-tape'));
  const shown = placed(doc.root).map((id) => doc.panels[id]!);
  const of = (type: string) => shown.filter((p) => p.type === type);
  assertEquals([of('chart').length, of('dom').length, of('tape').length], [1, 1, 1]);
  assert([...of('chart'), ...of('dom'), ...of('tape')].every((p) => p.link === 'green'), 'one link: a new ticker moves all three');
  assertEquals(doc.links.green.timeframe, '1m');
});
