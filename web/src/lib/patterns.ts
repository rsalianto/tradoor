/**
 * A pattern from the Signal panel as the chart draws it, and opening one on
 * the chart the panel drives — at the pattern's own length, with it drawn —
 * from the panel, a link in a Telegram message, or a notice of a new one.
 */
import { batch } from 'solid-js';
import { setShownPattern, setShownTrades } from '../app/ui';
import type { Signal, SignalLimit, SignalPoint, SignalTouch } from './api';
import { activate, tabsOf } from '../workspace/model';
import { doc, setPanelMarket, setPanelTimeframe, targetChart, update } from '../workspace/store';

const FOUR_HOURS = 4 * 3_600_000;

/** Any pattern a row can open: the list's, or the record's. */
export interface Openable {
  market: string;
  side: 'bear' | 'bull';
  drives: SignalPoint[];
  score: number;
  divergence: boolean;
  stage?: 'line';
  levels?: { tp: number[]; sl: number } | null;
  enteredAt?: number;
  limit?: SignalLimit | null;
  /** The length of bar it was found on; four hours where not said. */
  timeframe?: string;
}

/** A touch as a pattern to draw: its two drives, and the third where price reached the line, on that bar. */
export const touchPattern = (t: Pick<SignalTouch, 'market' | 'side' | 'drives' | 'touchedAt' | 'touchPrice'> & { levels?: { tp: number[]; sl: number } | null }): Openable => ({
  market: t.market,
  side: t.side,
  drives: [...t.drives, { at: Math.floor(t.touchedAt / FOUR_HOURS) * FOUR_HOURS, price: t.touchPrice }],
  score: 0,
  divergence: false,
  stage: 'line',
  levels: t.levels,
  enteredAt: Math.floor(t.touchedAt / FOUR_HOURS) * FOUR_HOURS,
});

/** A pattern of the list, entered where it turned. */
export const signalPattern = (s: Signal): Openable => ({ ...s, enteredAt: s.turnedAt ?? undefined });

/**
 * The pattern on a chart — the one given, or the one the panels drive — at its
 * own length, drawn; and with `front`, from outside the panels, that chart
 * brought from behind another tab.
 */
export function openPattern(s: Openable, target: { id: string } | null = targetChart(), front = false): void {
  if (!target) return;
  const tabs = front ? tabsOf(doc.root, target.id) : null;
  batch(() => {
    if (tabs && tabs.active !== target.id) update((d) => activate(d, tabs.id, target.id));
    const limit = s.limit ? { price: s.limit.price, from: s.limit.setAt, until: s.limit.until ?? s.limit.setAt } : undefined;
    setShownTrades(null);
    setShownPattern({ market: s.market, side: s.side, drives: s.drives, score: s.score, divergence: s.divergence, stage: s.stage, levels: (s.levels ?? s.limit?.levels) ?? undefined, enteredAt: s.enteredAt ?? s.limit?.setAt, limit, timeframe: s.timeframe });
    setPanelMarket(target.id, s.market);
    setPanelTimeframe(target.id, s.timeframe ?? '4h');
  });
}
