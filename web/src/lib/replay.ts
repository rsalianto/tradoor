/**
 * A bar replay's footprint. The cursor only ever shows what had happened by
 * then: bars whole before it, and of the bar it stands inside the minutes it
 * has passed. They are asked for ahead of it, so a step draws at once.
 */
import { foldMinutes, type Footprint, type Levels, type Minute, type Settings } from './footprint';

/** Steps a second a replay can play at. */
export const REPLAY_SPEEDS = [0.5, 1, 2, 3, 5, 10];

/** The steps shorter than a chart's own bar, where its base bars are short enough. */
export const REPLAY_STEPS = [
  { label: '1m', ms: 60_000 },
  { label: '5m', ms: 300_000 },
  { label: '15m', ms: 900_000 },
  { label: '1h', ms: 3_600_000 },
  { label: '4h', ms: 14_400_000 },
  { label: '1D', ms: 86_400_000 },
];

const MINUTE = 60_000;

export interface ReplaySource {
  /** How far behind and ahead of the cursor to ask at a time. */
  back: number;
  ahead: number;
  bars(from: number, to: number): Promise<Footprint | null>;
  /** The level minutes are kept at; 0 where a bar under way cannot be asked for. */
  tick: number;
  minutes(from: number, to: number): Promise<Minute[]>;
}

export interface ReplayFeed {
  moveTo(at: number): void;
  stop(): void;
}

/** `show`: the bars the cursor has reached, and whether any were found at all. */
export function replayFootprint(source: ReplaySource, barMs: number, settings: Settings, show: (bars: Levels[], grid: number, fitted: boolean, any: boolean) => void): ReplayFeed {
  const held = new Map<number, Levels>();
  let [grid, fitted, landed] = [0, true, false];
  /** The span asked for so far, and whether it is being asked for now. */
  let [from, to, asking] = [Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, false];
  /** The minutes of the bar under way, and which bar was last asked for. */
  let part: { start: number; minutes: Minute[] } | null = null;
  let partAsked = Number.NaN;
  let at = Number.NaN;
  let stopped = false;

  const draw = () => {
    if (!landed || stopped) return;
    const bars = [...held.values()].filter((b) => b.time + barMs <= at).sort((a, b) => a.time - b.time);
    const start = Math.floor(at / barMs) * barMs;
    if (part?.start === start && source.tick > 0 && grid > 0) {
      const passed = part.minutes.filter((m) => m.at + MINUTE <= at);
      const bar = passed.length > 0 ? foldMinutes(passed, source.tick, barMs, settings, Math.max(1, Math.round(grid / source.tick))).bars[0] : undefined;
      if (bar) bars.push(bar);
    }
    show(bars, grid, fitted, held.size > 0);
  };

  const ask = () => {
    if (asking || stopped || (at >= from && at + source.ahead / 4 <= to)) return;
    // Still inside what is held: only the stretch after it. Picked elsewhere: all of it anew.
    const inside = at >= from && at <= to;
    const [start, end] = [inside ? to : at - source.back, at + source.ahead];
    asking = true;
    const landing = (fp: Footprint | null) => {
      asking = false;
      if (stopped) return;
      if (!inside) held.clear();
      for (const bar of fp?.bars ?? []) held.set(bar.time, bar);
      if (fp) [grid, fitted] = [fp.grid, fp.fitted];
      // Asked for even when the answer failed: a server down is not asked again at every step.
      if (!inside) from = start;
      to = end;
      landed = true;
      draw();
      ask();
    };
    source.bars(start, Math.min(end, Date.now())).then(landing, () => landing(null));
  };

  const askPart = () => {
    const start = Math.floor(at / barMs) * barMs;
    if (source.tick <= 0 || start === at || start === partAsked) return;
    partAsked = start;
    source.minutes(start, start + barMs - 1).then(
      (minutes) => {
        if (start !== partAsked) return;
        part = { start, minutes };
        draw();
      },
      () => undefined,
    );
  };

  return {
    moveTo(next) {
      at = next;
      ask();
      askPart();
      draw();
    },
    stop() {
      stopped = true;
    },
  };
}
