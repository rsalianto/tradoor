/// <reference lib="deno.ns" />
import { assert, assertEquals } from 'jsr:@std/assert@1';
import { smoothPath, type Point } from './curve.ts';

/** Each command of a path, with its numbers. */
const commands = (d: string) => [...d.matchAll(/([MLC])([^MLC]*)/g)].map((m) => ({ c: m[1]!, n: m[2]!.split(/[ ,]/).filter(Boolean).map(Number) }));

Deno.test('nothing, one point, and two points joined straight', () => {
  assertEquals(smoothPath([]), '');
  assertEquals(smoothPath([{ x: 1, y: 2 }]), 'M1,2');
  const [, c] = commands(smoothPath([{ x: 0, y: 0 }, { x: 3, y: 6 }]));
  assertEquals(c!.n, [1, 2, 2, 4, 3, 6], 'its handles on the line itself');
});

Deno.test('the curve goes through every point and never past one', () => {
  const points: Point[] = [{ x: 0, y: 50 }, { x: 10, y: 48 }, { x: 12, y: 20 }, { x: 40, y: 22 }, { x: 41, y: 70 }, { x: 70, y: 69 }, { x: 100, y: 90 }];
  const cs = commands(smoothPath(points));
  assertEquals(cs[0], { c: 'M', n: [0, 50] });
  cs.slice(1).forEach((cmd, i) => {
    const [a, b] = [points[i]!, points[i + 1]!];
    assertEquals(cmd.n.slice(-2), [b.x, b.y], 'ends on its point');
    const [lo, hi] = [Math.min(a.y, b.y), Math.max(a.y, b.y)];
    for (const y of [cmd.n[1]!, cmd.n[3]!]) assert(y >= lo - 1e-9 && y <= hi + 1e-9, `a handle at ${y} outside ${lo}–${hi}`);
  });
});

Deno.test('a run that stays level stays level, and a jump at one moment is drawn straight', () => {
  const flat = commands(smoothPath([{ x: 0, y: 5 }, { x: 10, y: 5 }, { x: 20, y: 5 }]));
  for (const cmd of flat.slice(1)) assertEquals([cmd.n[1], cmd.n[3], cmd.n[5]], [5, 5, 5]);
  const d = smoothPath([{ x: 0, y: 0 }, { x: 10, y: 10 }, { x: 10, y: 30 }, { x: 20, y: 40 }]);
  assert(!d.includes('NaN') && !d.includes('Infinity'), d);
  assertEquals(commands(d)[2], { c: 'L', n: [10, 30] });
});
