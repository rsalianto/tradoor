/// <reference lib="deno.ns" />
/** Moving panels: onto a group, beside it, and among a strip's tabs. */
import { assertEquals } from 'jsr:@std/assert@1';
import { defaultWorkspace, movePanel, newPanel, panelTimeframe, tabsOf, validate, type ChartPanelState, type LayoutNode, type WorkspaceDoc } from './model.ts';

const types = (doc: WorkspaceDoc, n: LayoutNode): string => (n.kind === 'tabs' ? `[${n.panels.map((p) => doc.panels[p]!.type).join(',')}]` : `${n.dir}(${n.children.map((c) => types(doc, c)).join(' ')})`);
const idOf = (doc: WorkspaceDoc, type: string) => Object.values(doc.panels).find((p) => p.type === type)!.id;

Deno.test('a tab dropped among a strip\'s tabs takes that place, in its own group or another', () => {
  const doc = defaultWorkspace();
  const group = tabsOf(doc.root, idOf(doc, 'watchlist'))!;
  movePanel(doc, idOf(doc, 'alerts'), group.id, 'center', 0);
  assertEquals(group.panels.map((p) => doc.panels[p]!.type), ['alerts', 'watchlist', 'flow', 'signals']);
  movePanel(doc, idOf(doc, 'alerts'), group.id, 'center', 4);
  assertEquals(group.panels.map((p) => doc.panels[p]!.type), ['watchlist', 'flow', 'signals', 'alerts'], 'to the end');
  movePanel(doc, idOf(doc, 'values'), group.id, 'center', 1);
  assertEquals(group.panels.map((p) => doc.panels[p]!.type), ['watchlist', 'values', 'flow', 'signals', 'alerts'], 'from another group, between two');
  assertEquals(group.active, idOf(doc, 'values'));
});

Deno.test('two tabs of one group set side by side, either way round', () => {
  const doc = defaultWorkspace();
  const liq = () => tabsOf(doc.root, idOf(doc, 'liquidations'))!.id;
  movePanel(doc, idOf(doc, 'values'), liq(), 'center');
  movePanel(doc, idOf(doc, 'values'), liq(), 'left');
  assertEquals(types(doc, doc.root).endsWith('row([values] [liquidations])))'), true, types(doc, doc.root));
  movePanel(doc, idOf(doc, 'values'), liq(), 'right');
  assertEquals(types(doc, doc.root).endsWith('row([liquidations] [values])))'), true, types(doc, doc.root));
});

Deno.test('the height dragged on a footprint table comes back with the layout, if it is one the chart can draw', () => {
  const row = (saved: unknown) => {
    const doc = defaultWorkspace();
    const chart = Object.values(doc.panels).find((p) => p.type === 'chart')!;
    const back = validate(JSON.parse(JSON.stringify({ ...doc, panels: { ...doc.panels, [chart.id]: { ...chart, footprintStatsRow: saved } } })))!;
    return (back.panels[chart.id] as ChartPanelState).footprintStatsRow;
  };
  assertEquals(row(34), 34);
  assertEquals(row(undefined), undefined, 'a chart saved before there was one has none');
  assertEquals(row(500), undefined);
  assertEquals(row('tall'), undefined);
});

Deno.test('linked charts share a market and keep their own timeframes; a linked side panel goes by the group', () => {
  const d = defaultWorkspace();
  const chart = newPanel('chart', { link: 'green', market: 'binance-perp:BTCUSDT', timeframe: '5m' });
  const values = newPanel('values', { link: 'green', market: 'binance-perp:BTCUSDT', timeframe: '5m' });
  d.links.green.timeframe = '1h';
  assertEquals(panelTimeframe(d.links, chart), '5m');
  assertEquals(panelTimeframe(d.links, values), '1h');
  assertEquals(panelTimeframe(d.links, { ...chart, link: null }), '5m');
});

Deno.test('a layout stored before charts kept their own timeframes stays as it showed, then each is its own', () => {
  const d = defaultWorkspace();
  const linked = Object.values(d.panels).find((p) => p.type === 'chart' && p.link)!;
  linked.timeframe = '5m';
  d.links.green.timeframe = '30m';
  const { ownTimeframes: _, ...before } = d;
  assertEquals(validate(before)?.panels[linked.id]?.timeframe, '30m', 'it was showing its group\'s');
  assertEquals(validate(d)?.panels[linked.id]?.timeframe, '5m', 'once its own, the group does not touch it');
  assertEquals(validate(validate(before))?.panels[linked.id]?.timeframe, '30m');
});

Deno.test('a chart keeps how its studies draw, and nothing that cannot be a style', () => {
  const doc = defaultWorkspace();
  const chart = Object.values(doc.panels).find((p) => p.type === 'chart')! as ChartPanelState;
  chart.studyStyles = { 'bb:20:2.5/upper': '#ff0000,2,', 'ribbon/fast': '9' };
  (chart.studyStyles as Record<string, unknown>)['bad key'] = '1';
  const back = validate(JSON.parse(JSON.stringify(doc)))!;
  assertEquals((back.panels[chart.id] as ChartPanelState).studyStyles, { 'bb:20:2.5/upper': '#ff0000,2,', 'ribbon/fast': '9' });
  // From before a chart had any.
  delete (chart as Partial<ChartPanelState>).studyStyles;
  assertEquals((validate(JSON.parse(JSON.stringify(doc)))!.panels[chart.id] as ChartPanelState).studyStyles, {});
});
