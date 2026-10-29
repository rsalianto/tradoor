/// <reference lib="deno.ns" />
/** Moving panels: onto a group, beside it, and among a strip's tabs. */
import { assertEquals } from 'jsr:@std/assert@1';
import { defaultWorkspace, movePanel, tabsOf, type LayoutNode, type WorkspaceDoc } from './model.ts';

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
