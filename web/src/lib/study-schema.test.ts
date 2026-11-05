/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import type { StudyDefinition } from '../engine/protocol.ts';
import { anotherId, CATEGORIES, defaultId, entries, instanceLabel, LAYER_DEFS, searchEntries, studyPlots, toHex } from './study-schema.ts';

const param = (name: string, value: number, min = 1, max = 1000, step = 1) => ({ name, default: value, min, max, step });
const defs: StudyDefinition[] = [
  { kind: 'ema', short: 'EMA', group: 'price', params: [param('length', 50)] },
  { kind: 'vwap', short: 'VWAP', group: 'price', params: [param('anchor', 0, 0, 3), param('band1', 1, 0, 10, 0.1), param('band2', 0, 0, 10, 0.1), param('band3', 0, 0, 10, 0.1)] },
  { kind: 'avwap', short: 'Multi-VWAP', group: 'price', params: [] },
  { kind: 'rsi', short: 'RSI', group: 'pct', params: [param('length', 14)] },
  { kind: 'macd', short: 'MACD', group: 'macd', params: [param('fast', 12), param('slow', 26), param('signal', 9)] },
];
const list = entries(defs);
const found = (q: string) => searchEntries(list, q).map((e) => e.id);

Deno.test('the list files every study and the layers built in', () => {
  assertEquals(list.map((e) => e.id), ['ema', 'vwap', 'avwap', 'rsi', 'macd', ...LAYER_DEFS.map((l) => l.id)]);
  assertEquals(list.every((e) => CATEGORIES.includes(e.category)), true);
  assertEquals(list.find((e) => e.id === 'rsi')?.where, 'own pane');
});

Deno.test('a search puts the closest match first', () => {
  assertEquals(found('rsi')[0], 'rsi');
  assertEquals(found('vwap').slice(0, 2), ['vwap', 'avwap'], 'the exact name before the one that holds it');
  assertEquals(found('VOLUME')[0], 'volume', 'the layer named that, before what only mentions it');
  assertEquals(found('volume').includes('vwap'), true, 'volume weighted');
  assertEquals(found('moving average').includes('ema'), true, 'by words in any order');
  assertEquals(found('ma').includes('ema'), true, 'by what a reader types');
  assertEquals(found('nothing like this'), []);
  assertEquals(found('').length, list.length);
});

Deno.test('a study’s first id is its defaults, and another takes a value not yet there', () => {
  assertEquals(defaultId(defs[0]!), 'ema:50');
  assertEquals(defaultId(defs[1]!), 'vwap:0:1:0:0');
  assertEquals(anotherId(defs[0]!, ['ema:50']), 'ema:200');
  assertEquals(anotherId(defs[0]!, ['ema:50', 'ema:200']), 'ema:20');
  assertEquals(anotherId(defs[1]!, ['vwap:0:1:0:0']), 'vwap:1:1:0:0', 'the next anchor');
  assertEquals(anotherId(defs[3]!, ['rsi:14']), 'rsi:28');
  assertEquals(anotherId(defs[2]!, ['avwap']), null, 'nothing to tell a second one apart by');
});

Deno.test('what a new indicator paints besides lines has a row of its own', () => {
  const keys = (id: string) => studyPlots(id, []).map((p) => p.key);
  assertEquals(keys('sessions'), ['asia', 'london', 'ny']);
  assertEquals(keys('moon'), ['full', 'new']);
  assertEquals(keys('vpvr:24:70:30:0'), ['up', 'down', 'poc', 'va']);
  assertEquals(studyPlots('bb:20:2', [{ id: 'bb:20:2:upper', label: 'Upper', color: '#000', width: 1 }]).map((p) => p.key), ['upper', 'fill']);
  assertEquals(studyPlots('ichimoku:9:26:52:26', [{ id: 'ichimoku:9:26:52:26:leadA', label: 'Lead A', color: '#000', width: 1 }]).map((p) => [p.key, p.label]), [['leadA', 'Leading span A'], ['cloudUp', 'Cloud, A above B'], ['cloudDown', 'Cloud, A below B']]);
  assertEquals(studyPlots('volumedelta:0', [{ id: 'volumedelta:0:histogram', label: 'Delta', color: '#000', width: 1 }]).map((p) => p.key), ['histogram', 'histogramDown']);
  assertEquals(studyPlots('volumedelta:1', [{ id: 'volumedelta:1:cvd', label: 'CVD', color: '#000', width: 1 }]).map((p) => p.key), ['cvd'], 'a line has no second colour');
  // Levels the legend reads are not drawn as lines of their own: a width of 0.
  assertEquals(studyPlots('vpvr:24:70:30:0', [{ id: 'vpvr:24:70:30:0:poc', label: 'POC', color: '#000', width: 0 }]).map((p) => p.key), ['up', 'down', 'poc', 'va']);
});

Deno.test('a vwap lists its value area, shading and last period’s levels', () => {
  const line = (key: string) => ({ id: `vwap:0:1:0:0:${key}`, label: key, color: '#000', width: 1 });
  const plots = studyPlots('vwap:0:1:0:0', ['vwap', 'upper1', 'lower1', 'prevVah', 'prevVwap', 'prevVal'].map(line));
  assertEquals(plots.map((p) => [p.key, p.label]), [['vwap', 'vwap'], ['upper1', 'Value Area High'], ['lower1', 'Value Area Low'], ['fill1', 'Value area shading'], ['prevVah', 'Previous VAH'], ['prevVwap', 'Previous VWAP'], ['prevVal', 'Previous VAL']]);
  assertEquals(plots[0]!.pattern, '2', 'dotted unless changed');
  assertEquals(studyPlots('vwap:0:0:0:0', ['vwap', 'prevVwap'].map(line)).map((p) => p.key), ['vwap', 'prevVwap'], 'no band, no shading');
});

Deno.test('the picker says which one is on the chart', () => {
  assertEquals(instanceLabel('ema:50'), '50');
  assertEquals(instanceLabel('bb:20:2.5'), '20 2.5');
  assertEquals(instanceLabel('keylevels'), 'On chart');
  assertEquals(instanceLabel('vwap:0:1:0:0'), 'On chart');
  assertEquals(instanceLabel('vpvr:24:70:30:0'), 'On chart');
  assertEquals(instanceLabel('keltner:20:2:10'), '20 2 10');
});

Deno.test('the engine’s colours read back as the colour field’s', () => {
  assertEquals(toHex('rgba(127,178,255,1.000)'), '#7fb2ff');
  assertEquals(toHex('rgba(255,0,0,0.502)'), '#ff000080');
  assertEquals(toHex('#123456'), '#123456');
});

Deno.test('the style tab lists what a study draws, and what only carries values it does not', () => {
  const line = (key: string, width: number) => ({ id: `oi:255:0:0:${key}`, label: key, color: 'rgba(0,0,0,1.000)', width });
  const keys = (id: string, lines: ReturnType<typeof line>[]) => studyPlots(id, lines).map((p) => p.key);
  assertEquals(keys('oi:255:0:0', [line('open', 0), line('close', 0)]), ['up', 'down'], 'candles: their colours');
  assertEquals(studyPlots('oi:255:0:1', [{ id: 'oi:255:0:1:close', label: 'OI', color: '#f0b90b', width: 1.5 }]).map((p) => p.key), ['close'], 'a line: its stroke');
  assertEquals(studyPlots('keylevels', [{ id: 'keylevels:wo', label: 'WO', color: '#fffcbc', width: 0 }]).map((p) => [p.key, p.label, p.width]), [['wo', 'Weekly open', 1]], 'levels draw as segments');
  assertEquals(studyPlots('macd:12:26:9', [{ id: 'macd:12:26:9:histogram', label: 'Histogram', color: '#000', width: 1 }, { id: 'macd:12:26:9:macd', label: 'MACD', color: '#000', width: 1 }]).map((p) => p.key), ['histogram', 'histogramDown', 'macd']);
  assertEquals(studyPlots('rsi:14', [{ id: 'rsi:14:rsi', label: 'RSI 14', color: '#000', width: 1 }]).at(-1)?.owner, 'pct', 'the guides are the pane’s');
});
