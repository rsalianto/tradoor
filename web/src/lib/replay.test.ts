/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { replayFootprint, type ReplaySource } from './replay.ts';
import type { Levels, Minute } from './footprint.ts';

const HOUR = 3_600_000;
const bar = (time: number): Levels => ({ time, levels: [[100, 1, 2]], maxDelta: 0, minDelta: 0 });
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function source(over: Partial<ReplaySource> = {}): ReplaySource & { asked: [number, number][] } {
  const asked: [number, number][] = [];
  return {
    asked,
    back: 10 * HOUR,
    ahead: 4 * HOUR,
    tick: 0,
    bars: (from, to) => {
      asked.push([from, to]);
      const bars: Levels[] = [];
      for (let t = from - (from % HOUR); t < to; t += HOUR) bars.push(bar(t));
      return Promise.resolve({ bars, grid: 1, fitted: true });
    },
    minutes: () => Promise.resolve([]),
    ...over,
  };
}

Deno.test('only bars whole before the cursor are shown', async () => {
  let shown: number[] = [];
  const feed = replayFootprint(source(), HOUR, { ticks: 0, unit: 'coins' }, (bars) => (shown = bars.map((b) => b.time)));
  feed.moveTo(20 * HOUR);
  await tick();
  assertEquals(shown.at(-1), 19 * HOUR, 'the bar ending at the cursor is in');
  feed.moveTo(21 * HOUR + 30 * 60_000);
  assertEquals(shown.at(-1), 20 * HOUR, 'the bar it stands inside is not, whole');
});

Deno.test('asks ahead once, and again only near the end of it', async () => {
  const s = source();
  const feed = replayFootprint(s, HOUR, { ticks: 0, unit: 'coins' }, () => {});
  feed.moveTo(20 * HOUR);
  await tick();
  assertEquals(s.asked.length, 1);
  for (const at of [21, 22, 23]) feed.moveTo(at * HOUR);
  await tick();
  assertEquals(s.asked.length, 1, 'still well inside what it has');
  feed.moveTo(23.5 * HOUR);
  await tick();
  assertEquals(s.asked.length, 2);
  assertEquals(s.asked[1]![0], 24 * HOUR, 'only the stretch after it');
});

Deno.test('picked elsewhere, all of it is asked for anew', async () => {
  const s = source();
  let shown: number[] = [];
  const feed = replayFootprint(s, HOUR, { ticks: 0, unit: 'coins' }, (bars) => (shown = bars.map((b) => b.time)));
  feed.moveTo(100 * HOUR);
  await tick();
  feed.moveTo(50 * HOUR);
  await tick();
  assertEquals(s.asked[1]![0], 40 * HOUR);
  assertEquals(shown.at(-1), 49 * HOUR);
  assertEquals(shown[0], 40 * HOUR, 'nothing from before the jump is kept');
});

Deno.test('of the bar under way, the minutes the cursor has passed', async () => {
  const minutes: Minute[] = [0, 1, 2].map((i) => ({ at: 5 * HOUR + i * 60_000, levels: [[100, 1, 0]] }));
  let last: Levels | undefined;
  const feed = replayFootprint(source({ tick: 1, minutes: () => Promise.resolve(minutes) }), HOUR, { ticks: 0, unit: 'coins' }, (bars) => (last = bars.at(-1)));
  feed.moveTo(5 * HOUR + 2 * 60_000);
  await tick();
  assertEquals(last?.time, 5 * HOUR);
  assertEquals(last?.levels, [[100, 2, 0]], 'two minutes of three');
  feed.moveTo(5 * HOUR + 3 * 60_000);
  assertEquals(last?.levels, [[100, 3, 0]]);
});

Deno.test('a failed answer is not asked for again at every step', async () => {
  let asks = 0;
  const feed = replayFootprint(source({ bars: () => (asks++, Promise.reject(new Error('down'))) }), HOUR, { ticks: 0, unit: 'coins' }, () => {});
  feed.moveTo(20 * HOUR);
  await tick();
  feed.moveTo(21 * HOUR);
  await tick();
  assertEquals(asks, 1);
});

Deno.test('stopped, nothing more is shown', async () => {
  let shows = 0;
  const feed = replayFootprint(source(), HOUR, { ticks: 0, unit: 'coins' }, () => shows++);
  feed.moveTo(20 * HOUR);
  feed.stop();
  await tick();
  assertEquals(shows, 0);
});
