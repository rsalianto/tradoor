/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { dropOwner, numberOf, plotStyle, readStyles, renameOwner, replaceStudy, serializeStyles, withLayer, withNumber, withoutLayer, withPlot } from './study-style.ts';

Deno.test('a removed layer stays undrawn and comes back drawn, its settings gone', () => {
  const chart = { hidden: 'ema200,rsi:14', removed: '', studyStyles: { 'ribbon/fast': '9', 'ema200/length': '100' } };
  const off = withoutLayer(chart, 'ribbon');
  assertEquals(off, { hidden: 'ema200,rsi:14,ribbon', removed: 'ribbon', studyStyles: { 'ema200/length': '100' } });
  assertEquals(withLayer(off, 'ribbon'), { hidden: 'ema200,rsi:14', removed: '' });
  // A layer already hidden is listed once, and comes back shown.
  const hiddenFirst = withoutLayer(off, 'ema200');
  assertEquals(hiddenFirst.hidden, 'rsi:14,ribbon,ema200');
  assertEquals(hiddenFirst.removed, 'ribbon,ema200');
  assertEquals(withLayer(hiddenFirst, 'ema200'), { hidden: 'rsi:14,ribbon', removed: 'ribbon' });
});

Deno.test('a plot left at its defaults stores nothing', () => {
  let s = withPlot({}, 'ema:50', 'ema', { color: '#ff0000' });
  assertEquals(s, { 'ema:50/ema': '#ff0000,,' });
  s = withPlot(s, 'ema:50', 'ema', { width: '2', off: true });
  assertEquals(plotStyle(s, 'ema:50', 'ema'), { color: '#ff0000', width: '2', off: true, pattern: '' });
  s = withPlot(s, 'ema:50', 'ema', { pattern: '2' });
  assertEquals(s, { 'ema:50/ema': '#ff0000,2,1,2' });
  assertEquals(plotStyle(s, 'ema:50', 'ema').pattern, '2');
  s = withPlot(s, 'ema:50', 'ema', { color: '', width: '', off: false, pattern: '' });
  assertEquals(s, {});
  assertEquals(plotStyle(undefined, 'ema:50', 'ema'), { color: '', width: '', off: false, pattern: '' });
});

Deno.test('a number back at its default is forgotten', () => {
  let s = withNumber({}, 'ribbon', 'fast', 9, 12);
  assertEquals(numberOf(s, 'ribbon', 'fast', 12), 9);
  s = withNumber(s, 'ribbon', 'fast', 12, 12);
  assertEquals(s, {});
  assertEquals(numberOf(s, 'ribbon', 'fast', 12), 12);
  assertEquals(numberOf({ 'ribbon/fast': 'x' }, 'ribbon', 'fast', 12), 12);
});

Deno.test('styles follow a study to its new parameters and go with it', () => {
  const s = { 'ema:50/ema': '#ff0000,,', 'ema:500/ema': '#00ff00,,', 'pct/upper': '70' };
  assertEquals(renameOwner(s, 'ema:50', 'ema:21'), { 'ema:21/ema': '#ff0000,,', 'ema:500/ema': '#00ff00,,', 'pct/upper': '70' });
  assertEquals(dropOwner(s, 'ema:50'), { 'ema:500/ema': '#00ff00,,', 'pct/upper': '70' });
  assertEquals(replaceStudy({ studies: 'rsi:14,ema:50', hidden: 'ribbon,ema:50', studyStyles: s }, 'ema:50', 'ema:21'), { studies: 'rsi:14,ema:21', hidden: 'ribbon,ema:21', studyStyles: renameOwner(s, 'ema:50', 'ema:21') });
});

Deno.test('the engine reads key=value by semicolon', () => {
  assertEquals(serializeStyles({ 'ema:50/ema': '#ff0000,2,1', 'ribbon/fast': '9' }), 'ema:50/ema=#ff0000,2,1;ribbon/fast=9');
  assertEquals(serializeStyles(undefined), '');
});

Deno.test('a stored chart keeps only what can be a style', () => {
  assertEquals(readStyles(null), {});
  assertEquals(readStyles(['x']), {});
  assertEquals(readStyles({ 'bb:20:2.5/upper': '#fff,1.5,', 'ribbon/fast': '9', 'no slash': '1', 'a/b': 3, 'c/d': 'x;y=z' }), { 'bb:20:2.5/upper': '#fff,1.5,', 'ribbon/fast': '9' });
});
