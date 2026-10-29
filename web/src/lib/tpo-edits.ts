/**
 * What was done to TPO profiles by hand, as a chart keeps it: `e=t,t;m=t;s=t`
 * — the profiles drawn expanded, those joined to the one before, and where any
 * was cut in two. A profile is known by its start, as the engine knows it.
 */
import type { TpoHover } from '../engine/protocol';

interface Edits {
  e: number[];
  m: number[];
  s: number[];
}

/** The most of each kind a chart keeps; the oldest go first. */
const MAX = 400;

export function parseEdits(raw: string): Edits {
  const out: Edits = { e: [], m: [], s: [] };
  for (const pair of raw.split(';')) {
    const [key, list] = pair.split('=');
    if ((key === 'e' || key === 'm' || key === 's') && list) out[key] = list.split(',').map(Number).filter(Number.isFinite);
  }
  return out;
}

export function serializeEdits(e: Edits): string {
  return (['e', 'm', 's'] as const)
    .filter((k) => e[k].length > 0)
    .map((k) => `${k}=${[...new Set(e[k])].slice(-MAX).join(',')}`)
    .join(';');
}

const edit = (raw: string, change: (e: Edits) => void): string => {
  const e = parseEdits(raw);
  change(e);
  return serializeEdits(e);
};

/** Expanded to a column a bracket, at its time, or folded back into its shape. */
export const toggleExpanded = (raw: string, h: TpoHover): string =>
  edit(raw, (e) => {
    e.e = h.expanded ? e.e.filter((t) => t !== h.start) : [...e.e, h.start];
  });

/** Joined to the profile before it. */
export const mergeLeft = (raw: string, h: TpoHover): string => (h.prev === null ? raw : edit(raw, (e) => void e.m.push(h.start)));

/** The profile after it joined to it. */
export const mergeRight = (raw: string, h: TpoHover): string => (h.next === null ? raw : edit(raw, (e) => void e.m.push(h.next!)));

/** A joined profile back into the ones it was made of; an expanded one cut at the bracket under the pointer. */
export const canSplit = (h: TpoHover): boolean => h.merged || h.splitAt !== null;

export const splitProfile = (raw: string, h: TpoHover): string =>
  edit(raw, (e) => {
    if (h.merged) {
      e.m = e.m.filter((t) => t <= h.start || t >= h.end);
      e.s = e.s.filter((t) => t <= h.start || t >= h.end);
    } else if (h.splitAt !== null) {
      e.s.push(h.splitAt);
    }
  });
