/**
 * The bars VWAP levels read, besides the chart's own: hourly ones back to the
 * start of last month, for the day, week and month, and daily ones back to the
 * start of last year, for the quarter and year — so a minute chart, which holds
 * days, still has its year's. Followed while the study is on, each as its bar
 * closes, and cut at a replay's cursor to what had closed by then.
 */
import { lowerBound, sliceCols, type CandleSeries, type Cols } from './history';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
/** Past the turn of an hour, for the exchange to have closed its bar. */
const SETTLE_MS = 20_000;

type Held = { series: CandleSeries; release: () => void };

export class LevelFeed {
  private readonly held: (Held | undefined)[] = [];
  private readonly asked = [0, 0];
  private readonly off: (() => void)[] = [];
  private timer: ReturnType<typeof setTimeout> | undefined;
  private stopped = false;

  constructor(
    /** The market's series of this timeframe, loaded `bars` back and kept until released. */
    private readonly hold: (timeframe: string, bars: number) => Promise<Held>,
    /** Where a replay stands, or null. */
    private readonly at: () => number | null,
    private readonly set: (daily: boolean, bars: Cols) => void,
  ) {
    this.tick();
  }

  /** The bars as they stand, each loaded back far enough for where the chart is. */
  push(): void {
    if (this.stopped) return;
    const at = this.at();
    const d = new Date(at ?? Date.now());
    const since = [Date.UTC(d.getUTCFullYear(), d.getUTCMonth() - 1, 1), Date.UTC(d.getUTCFullYear() - 1, 0, 1)];
    [HOUR, DAY].forEach((ms, i) => {
      const bars = Math.ceil((Date.now() - since[i]!) / ms) + 2;
      if (bars > this.asked[i]!) {
        this.asked[i] = bars;
        this.hold(i ? '1d' : '1h', bars).then(
          (h) => {
            this.off.push(h.release);
            if (this.stopped) return this.stop();
            if (!this.held[i]) this.off.push(h.series.subscribe(() => this.push()));
            this.held[i] = h;
            this.push();
          },
          // Asked again on the hour.
          () => (this.asked[i] = 0),
        );
      }
      const c = this.held[i]?.series.cols;
      if (c) this.set(i === 1, at === null ? c : sliceCols(c, 0, lowerBound(c.time, at - ms + 1)));
    });
  }

  /** On each hour, once its bar has closed: the newest bars, and any not yet loaded. */
  private tick(): void {
    this.timer = setTimeout(() => {
      for (const h of this.held) void h?.series.refreshTail(true).catch(() => undefined);
      this.push();
      this.tick();
    }, HOUR - (Date.now() % HOUR) + SETTLE_MS);
  }

  stop(): void {
    this.stopped = true;
    clearTimeout(this.timer);
    for (const off of this.off.splice(0)) off();
  }
}
