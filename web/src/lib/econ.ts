/**
 * The US economic calendar as a panel reads it: whether a release came in above
 * or below what was expected, what is next, and the list in the order it is
 * read, with today's "now" standing between what is out and what is to come.
 */
import type { EconEvent } from './api';

const SCALE: Record<string, number> = { '': 1, K: 1e3, M: 1e6, B: 1e9, T: 1e12 };

/** A cell's figure as a number: `1,718K`, `0.4%`, `-1.2B`; null for anything else. */
export function figure(cell: string | null): number | null {
  const m = /^\s*([-+]?)\$?([\d,]*\.?\d+)\s*([KMBT]?)/i.exec(cell ?? '');
  if (!m) return null;
  const n = Number(m[2]!.replace(/,/g, '')) * SCALE[m[3]!.toUpperCase()]!;
  return m[1] === '-' ? -n : n;
}

/** 1 for a release above its forecast, -1 below, 0 on it or where one is missing. */
export function surprise(actual: string | null, forecast: string | null): -1 | 0 | 1 {
  const [a, f] = [figure(actual), figure(forecast)];
  return a === null || f === null || a === f ? 0 : a > f ? 1 : -1;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** `1d 04:12:33`, `04:12:33` or `12:33`. */
export function countdown(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const [d, h, m, s] = [Math.floor(total / 86_400), Math.floor(total / 3600) % 24, Math.floor(total / 60) % 60, total % 60];
  return `${d > 0 ? `${d}d ` : ''}${d > 0 || h > 0 ? `${pad(h)}:` : ''}${pad(m)}:${pad(s)}`;
}

/** How long after its time a release counts as due rather than simply without a figure. */
const DUE_MS = 20 * 60_000;

/** Its time has come, its figure has not, and there is a figure to come: a speech has none. */
export const isDue = (e: EconEvent, now: number): boolean => !e.allDay && e.at <= now && now - e.at < DUE_MS && e.actual === null && (e.forecast !== null || e.previous !== null);

/** The releases next to come, all at the one time, the ones everyone waits for first. */
export function nextOf(events: readonly EconEvent[], now: number): EconEvent[] {
  const first = events.find((e) => !e.allDay && e.at > now);
  if (!first) return [];
  return events.filter((e) => !e.allDay && e.at === first.at).sort((a, b) => Number(b.core) - Number(a.core));
}

export type EconRow = { kind: 'day'; key: string } | { kind: 'now' } | { kind: 'event'; event: EconEvent; past: boolean };

/**
 * The list in reading order: each day under its heading, and the line for now
 * just before the first release to come, or before that release's heading when
 * it opens a day. `dayOf` is the date a release is filed under, in the reader's zone.
 */
export function arrange(events: readonly EconEvent[], now: number, dayOf: (e: EconEvent) => string): EconRow[] {
  const dated = events.map((event) => ({ event, key: dayOf(event) })).sort((a, b) => a.key.localeCompare(b.key) || a.event.at - b.event.at || a.event.title.localeCompare(b.event.title));
  const rows: EconRow[] = [];
  let day = '';
  let placed = false;
  for (const { event, key } of dated) {
    const past = event.at <= now;
    if (!past && !placed) {
      placed = true;
      rows.push({ kind: 'now' });
    }
    if (key !== day) {
      day = key;
      rows.push({ kind: 'day', key });
    }
    rows.push({ kind: 'event', event, past });
  }
  if (!placed) rows.push({ kind: 'now' });
  return rows;
}

/** `2026-11-11` moved by whole days. */
export const shiftDay = (key: string, days: number): string => new Date(Date.parse(`${key}T00:00:00Z`) + days * 86_400_000).toISOString().slice(0, 10);

/** `Wed 11 Nov`, read from the date itself so no zone can move it. */
export const dayName = (key: string): string => new Date(`${key}T00:00:00Z`).toLocaleDateString('en-GB', { timeZone: 'UTC', weekday: 'short', day: 'numeric', month: 'short' }).replace(',', '');
