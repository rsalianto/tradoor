/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import type { TpoHover } from '../engine/protocol.ts';
import { canSplit, mergeLeft, mergeRight, parseEdits, splitProfile, toggleExpanded } from './tpo-edits.ts';

const day = 86_400_000;
const hover = (over: Partial<TpoHover> = {}): TpoHover => ({ start: day, end: 2 * day, prev: 0, next: 2 * day, expanded: false, merged: false, splitAt: null, prices: [], ...over });

Deno.test('a profile expands and folds back by its start', () => {
  const once = toggleExpanded('', hover());
  assertEquals(once, `e=${day}`);
  assertEquals(toggleExpanded(once, hover({ expanded: true })), '');
});

Deno.test('merging joins a profile to its neighbour, and only where there is one', () => {
  assertEquals(mergeLeft('', hover()), `m=${day}`);
  assertEquals(mergeRight('', hover()), `m=${2 * day}`);
  assertEquals(mergeLeft('', hover({ prev: null })), '', 'the first profile has nothing before it');
  assertEquals(mergeRight('e=5', hover({ next: null })), 'e=5');
});

Deno.test('splitting undoes a merge, or cuts an expanded profile at the pointer', () => {
  const merged = hover({ start: 0, end: 3 * day, merged: true });
  assertEquals(canSplit(merged), true);
  assertEquals(splitProfile(`m=${day},${2 * day},${5 * day}`, merged), `m=${5 * day}`, 'its own joins undone, others kept');
  assertEquals(canSplit(hover()), false, 'a folded profile has no bracket to cut at');
  assertEquals(splitProfile('', hover({ expanded: true, splitAt: day + 3_600_000 })), `s=${day + 3_600_000}`);
  assertEquals(parseEdits('e=1,2;m=x;s=3;zz=4'), { e: [1, 2], m: [], s: [3] });
});
