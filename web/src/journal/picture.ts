/**
 * Journal trades as a chart draws them: each a long or short position from
 * when it was entered to when it closed, numbered in the order they were
 * taken, with how it went written over it. Shown on a chart for a while, and
 * saved there if wanted, as pictures of the trades rather than new ones
 * (bins/server journal, `pictures_a_trade`).
 */
import type { DrawingPoint } from '../engine/protocol';
import type { Style } from '../lib/drawing-style';
import { isClosed, money, outcome, pnl, rMultiple, rText, type Trade } from './model';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

export interface PicturePart {
  kind: string;
  points: DrawingPoint[];
  label: string | null;
  style: Style;
}

export interface Picture {
  parts: PicturePart[];
  /** The first entry drawn, and the last close. */
  from: number;
  to: number;
  won: number;
  lost: number;
  even: number;
  open: number;
  /** What those closed made, in R and in money. */
  r: number;
  pnl: number;
}

/** Whether it ever went in: a plan never filled, or called off, has nothing to draw. */
export const entered = (t: Trade): boolean => isClosed(t) || (t.status === 'open' && t.openedAt !== null);

/**
 * When it went in and out. A close at or before its entry, which the record
 * cannot mean, is an hour after it; one still open runs to now.
 */
export function spanOf(t: Trade, now: number): { from: number; to: number } {
  const from = t.openedAt ?? t.startAt;
  const close = isClosed(t) ? t.closedAt : now;
  return { from, to: close !== null && close > from ? close : from + HOUR };
}

const between = (p: number, a: number, b: number): boolean => p > Math.min(a, b) && p < Math.max(a, b);

/**
 * Where it came out short of its stop or target, as the position's legs draw it:
 * a dashed line across the box at each price taken, cut, or added at.
 */
export function exitLegs(t: Trade): string {
  const bought = t.fills.filter((f) => f.kind === 'entry').reduce((sum, f) => sum + f.qty, 0) || t.qty;
  const part = (qty: number) => Math.min(1000, Math.max(0.01, Number(((qty / bought) * 100).toFixed(2))));
  const legs = t.fills.flatMap((f): string[] => {
    const kind = f.kind === 'take' ? 't' : f.kind === 'cut' ? 'c' : f.kind === 'add' ? 'a' : f.kind === 'hand' ? (between(f.price, t.entry, t.target) ? 't' : 'c') : null;
    return kind && bought > 0 ? [`${kind},${f.price},${part(f.qty)}`] : [];
  });
  // Closed by hand with nothing recorded of how: the whole of it at its exit.
  const out = t.fills.some((f) => f.kind === 'take' || f.kind === 'cut' || f.kind === 'hand' || f.kind === 'stop' || f.kind === 'target');
  if (!out && t.exit !== null && isClosed(t) && between(t.exit, t.stop, t.target) && t.exit !== t.entry) legs.push(`${between(t.exit, t.entry, t.target) ? 't' : 'c'},${t.exit},100`);
  return legs.slice(0, 12).join('|');
}

const WORD = { win: 'Won', loss: 'Lost', even: 'Break-even', open: 'Open', planned: 'Planned', cancelled: 'Cancelled' } as const;

/** `#2 Short · Lost −1.00R −$15.00`: which it was, and how it went. */
export function labelOf(t: Trade, n: number): string {
  const o = outcome(t);
  const side = t.side === 'long' ? 'Long' : 'Short';
  return o === 'open' ? `#${n} ${side} · Open` : `#${n} ${side} · ${WORD[o]} ${rText(rMultiple(t))} ${money(pnl(t))}`;
}

/** The trades entered among those given, drawn oldest first; null when none was. */
export function tradePicture(trades: readonly Trade[], now: number): Picture | null {
  const list = trades.filter(entered).sort((a, b) => spanOf(a, now).from - spanOf(b, now).from || a.id - b.id);
  if (list.length === 0) return null;
  const tally = { won: 0, lost: 0, even: 0, open: 0, r: 0, pnl: 0 };
  const parts = list.map((t, i): PicturePart => {
    const { from, to } = spanOf(t, now);
    const o = outcome(t);
    if (o === 'win') tally.won++;
    else if (o === 'loss') tally.lost++;
    else if (o === 'even') tally.even++;
    else tally.open++;
    tally.r += rMultiple(t) ?? 0;
    tally.pnl += pnl(t) ?? 0;
    const leverage = t.leverage > 0 ? t.leverage : 1;
    const legs = exitLegs(t);
    return {
      kind: t.side,
      points: [
        { time: from, price: t.entry },
        { time: from, price: t.stop },
        { time: to, price: t.target },
      ],
      label: labelOf(t, i + 1),
      // Sized as it was traded, so its figures read what it held.
      style: { sizeBy: 'margin', margin: (t.qty * t.entry) / leverage, leverage, ofTrade: t.id, ...(legs ? { legs } : {}) },
    };
  });
  const spans = list.map((t) => spanOf(t, now));
  return { parts, from: Math.min(...spans.map((s) => s.from)), to: Math.max(...spans.map((s) => s.to)), ...tally };
}

/** `4 trades · 1 won · 3 lost · −2.40R · −$19.86`. */
export function summaryOf(p: Picture): string {
  const count = p.parts.length;
  const bits = [`${count} trade${count === 1 ? '' : 's'}`, `${p.won} won`, `${p.lost} lost`];
  if (p.even > 0) bits.push(`${p.even} even`);
  if (p.open > 0) bits.push(`${p.open} open`);
  if (count > p.open) bits.push(rText(p.r), money(p.pnl));
  return bits.join(' · ');
}

/** Bar lengths a chart is opened at for trades, shortest first. */
const FRAMES: readonly [string, number][] = [
  ['1m', MINUTE],
  ['5m', 5 * MINUTE],
  ['15m', 15 * MINUTE],
  ['1h', HOUR],
  ['4h', 4 * HOUR],
  ['1d', 24 * HOUR],
];

/**
 * The shortest bar that shows the trades together, some three hundred bars
 * across at the most, and still reaches back to the first of them in the
 * bars a chart first loads.
 */
export function frameFor(from: number, to: number, now: number): string {
  for (const [tf, ms] of FRAMES) if ((to - from) / ms <= 300 && (now - from) / ms <= 1400) return tf;
  return '1d';
}

/** The market most of them were on: the chart they are drawn on. */
export function marketOf(trades: readonly Trade[]): string | null {
  const counts = new Map<string, number>();
  for (const t of trades.filter(entered)) counts.set(t.market, (counts.get(t.market) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
}
