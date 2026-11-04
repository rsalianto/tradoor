/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { SeenBars, type Levels } from './footprint.ts';

const MINUTE = 60_000;
const bar = (time: number, bought = 1): Levels => ({ time, levels: [[100, bought, 0]], maxDelta: 0, minDelta: 0 });
const times = (bars: Levels[]) => bars.map((b) => b.time);
const bought = (bars: Levels[], time: number) => bars.find((b) => b.time === time)?.levels[0]?.[1];

Deno.test('the bar under way lies over what the server has of it', () => {
  const seen = new SeenBars(MINUTE);
  const out = seen.over([bar(0), bar(MINUTE, 5)], bar(MINUTE, 7), MINUTE);
  assertEquals(times(out), [0, MINUTE]);
  assertEquals(bought(out, MINUTE), 7);
});

Deno.test('a bar that closes stays until the server has it', () => {
  const seen = new SeenBars(MINUTE);
  seen.over([bar(0)], bar(MINUTE, 7), MINUTE);
  // The next minute, the server still without the one that closed.
  const out = seen.over([bar(0)], bar(2 * MINUTE), 2 * MINUTE);
  assertEquals(times(out), [0, MINUTE, 2 * MINUTE]);
  assertEquals(bought(out, MINUTE), 7);
  // Before any trade of the new minute, too.
  assertEquals(times(new SeenBars(MINUTE).over([bar(0)], undefined, MINUTE)), [0]);
  const quiet = new SeenBars(MINUTE);
  quiet.over([], bar(MINUTE), MINUTE);
  assertEquals(times(quiet.over([], undefined, 2 * MINUTE)), [MINUTE]);
});

Deno.test("the server's bar takes over once it has written all of it", () => {
  const seen = new SeenBars(MINUTE);
  seen.over([], bar(MINUTE, 7), MINUTE);
  seen.over([], undefined, 2 * MINUTE);
  const closedAt = 2 * MINUTE;
  // Asked straight after the close, its copy may be short of the last minute.
  seen.answered([bar(MINUTE, 6)], closedAt + 1_000);
  assertEquals(bought(seen.over([bar(MINUTE, 6)], undefined, 2 * MINUTE), MINUTE), 7);
  seen.answered([bar(MINUTE, 8)], closedAt + 11_000);
  assertEquals(bought(seen.over([bar(MINUTE, 8)], undefined, 2 * MINUTE), MINUTE), 8);
});

Deno.test('a bar the server never gets stays, and an answer without it changes nothing', () => {
  const seen = new SeenBars(MINUTE);
  seen.over([], bar(MINUTE, 7), MINUTE);
  seen.over([], bar(2 * MINUTE), 2 * MINUTE);
  seen.answered([bar(0)], 10 * MINUTE);
  assertEquals(bought(seen.over([bar(0)], undefined, 3 * MINUTE), MINUTE), 7);
});
