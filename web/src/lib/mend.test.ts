/// <reference lib="deno.ns" />
import { assert, assertEquals, assertStringIncludes } from 'jsr:@std/assert@1';
import { lostLines, restoreLines } from './frost-repair.ts';
import { mendScript } from './mend.ts';
import { pineLeftovers } from './pine-leftovers.ts';

/** As a chat copied it: the line breaks gone, the indentation left as runs of spaces, names fused. */
const COLLAPSED = [
  'input startTime = date(2024, 1, 1) "Start"input showTag = true "Show tag"input len = 5 min 1 max 100 "Length"',
  'rangeStart = startTimerangeEnd = time',
  'state ptBar = list()state ptPrice = list()state liveLine = nastate liveLabel = na',
  'fn tagOf(typ) {  tag = typ == 1 ? "H" : "L"  if len > 0 {    tag = "X"  }  return tag}',
  'pivotTime = time[len]pivotInRange = pivotTime >= rangeStart and pivotTime <= rangeEndif showTag {  ptBar.push(bar)}',
  'background(time <= rangeEnd ? blue : na)',
].join('\n');

Deno.test('a copy that lost its line breaks reads as one, and gets them back', () => {
  assert(lostLines(COLLAPSED.repeat(3)));
  const fixed = restoreLines(COLLAPSED);
  const lines = fixed.split('\n').map((l) => l.trim());
  for (const want of ['input showTag = true "Show tag"', 'input len = 5 min 1 max 100 "Length"', 'rangeStart = startTime', 'rangeEnd = time', 'state liveLine = na', 'state liveLabel = na', 'tag = typ == 1 ? "H" : "L"', 'if len > 0 {', 'pivotTime = time[len]', 'pivotInRange = pivotTime >= rangeStart and pivotTime <= rangeEnd', 'if showTag {', 'return tag', '}']) {
    assert(lines.includes(want), `${want} on a line of its own in:\n${fixed}`);
  }
});

Deno.test('a script with its lines is not taken for one that lost them', () => {
  assert(!lostLines('input len = 14\nr = rsi(close, len)\nplot(r, "RSI")'));
  assert(!lostLines('a  = 1   // aligned\nb  = 2'));
});

Deno.test('Pine’s leftovers written as Frost Script', () => {
  const m = pineLeftovers(['//@version=6', 'indicator("T", overlay=false)', 'length = input.int(14, "Length", minval=1, maxval=200)', 'var float total = 0.0', 'total := total + ta.sma(close, length)', 'if barstate.islast and not na(total)', '    label.new(bar_index, high, str.tostring(total, "#.##"))', 'bgcolor(close > open ? color.new(color.green, 80) : na)', 'plotshape(ta.crossover(close, ta.ema(close, 9)), title="Up", location=location.belowbar, color=color.lime, style=shape.triangleup)'].join('\n'));
  const t = m.text;
  assertStringIncludes(t, 'pane own');
  assertStringIncludes(t, 'input length = 14 min 1 max 200 "Length"');
  assertStringIncludes(t, 'state total = 0.0');
  assertStringIncludes(t, 'total = total + sma(close, length)');
  assertStringIncludes(t, 'if islast and not isna(total) {');
  assertStringIncludes(t, 'str(round(total, 2))');
  assertStringIncludes(t, 'background(close > open ? alpha(green, 1 - (80) / 100) : na)');
  assertStringIncludes(t, 'mark(crossover(close, ema(close, 9)), "Up", below, lime)');
  assert(m.left.some((l) => l.startsWith('label.new')), 'what it cannot carry over is said');
  assert(!t.includes('indicator('));
});

Deno.test('a strategy’s orders, functions and switch', () => {
  const m = pineLeftovers(['fast(x) => ta.ema(x, 9)', 'pick(n) =>', '    switch n', '        1 => "one"', '        => "many"', 'if ta.crossover(close, fast(close))', '    strategy.entry("L", strategy.long)', 'if strategy.position_size > 0 and close < open', '    strategy.close("L")', 'strategy.exit("x", "L", stop=low[1], limit=high * 1.02)'].join('\n'));
  const t = m.text;
  assertStringIncludes(t, 'fn fast(x) {\n  return ema(x, 9)\n}');
  assertStringIncludes(t, 'return n == 1 ? "one" : "many"');
  assertStringIncludes(t, 'long(true)');
  assertStringIncludes(t, 'if (inlong ? 1 : inshort ? -1 : 0) > 0 and close < open {');
  assertStringIncludes(t, 'exit(true)');
  assertStringIncludes(t, 'stop(low[1])\ntarget(high * 1.02)');
});

Deno.test('text in quotes is never touched, and Frost Script is left as it is', () => {
  const quoted = pineLeftovers('plot(close, "ta.sma := color.red")');
  assertEquals(quoted.text, 'plot(close, "ta.sma := color.red")');
  const frost = 'input length = 20 min 2 max 500 "Length"\naverage = ema(close, length)\nplot(average, "Average", blue)\nif islast {\n  x = 1\n}';
  assertEquals(pineLeftovers(frost).text, frost);
  assertEquals(mendScript(frost).changes, []);
});
