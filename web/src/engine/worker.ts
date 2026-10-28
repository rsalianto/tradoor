/**
 * The engine thread: WASM, history, exchange streams and every chart's
 * canvases. The page sends input and settings; this sends back only what the
 * page shows in the DOM — legend, cursor, events and status.
 */
import init, { ChartEngine, LiveFeedEngine, bar_start, fold_candles, indicator_values, profile_spec, study_catalog, timeframe_info } from '../wasm/pkg/tradoor_wasm.js';
import wasmUrl from '../wasm/pkg/tradoor_wasm_bg.wasm?url';
import { decimalsOf, MAX_PRICE_DECIMALS } from '../lib/format';
import { demoHit } from '../lib/demo';
import { MARK_ORDER, markIndex } from '../lib/mark-order';
import { parseMarket, ratioLegs, type MarketRef } from '../lib/markets';
import { concatCols, emptyCols, formingRows, intervalMs, lowerBound, openSeries, sameCols, sliceCols, type CandleSeries, type Cols, type Series } from './history';
import type { RatioSeries } from './ratio';
import { lacksUtcOf, marketInterval } from './intervals';
import { nativeOf } from '../lib/native';
import type { CallName, Calls, ChartEvent, ChartStatus, DailyBars, FromWorker, IndicatorValues, PointerKind, StudyDefinition, ToWorker } from './protocol';
import { followLiquidations } from '../lib/liquidation-feed';
import { serverTicker } from '../lib/liquidation-venues';
import { coinOf, oiVenues, OpenInterestFeed } from './open-interest';
import { StreamHub, type KlineMsg, type SocketState, type StreamListener, type StreamMsg } from './streams';
import { onWake } from './reach';

/** Closed bars a chart opens with: enough for EMA 200 to settle, with room to scroll. */
const CHART_BARS = 1_500;
/**
 * A chart of 15 minutes or shorter keeps an open world: its engine holds only
 * the bars about the view — the newest few thousand, back to the week's open
 * at least for its levels — widened as the reader pans back and narrowed again
 * once they are back at the newest, so a minute chart with months behind it is
 * worked out and drawn as one with days. Longer charts hold every bar.
 */
const WINDOW_MAX_BAR_MS = 15 * 60_000;
const WINDOW_BARS = 6_000;
/** How far the window widens each time the reader nears its oldest bar. */
const WIDEN_BARS = 4_000;
const WEEK_MS = 7 * 86_400_000;
/** 1970-01-05 was a Monday: weeks open counted from it. */
const MONDAY_MS = 4 * 86_400_000;
/** Cap on the base bars behind them — 1,500 bars of 30m is 45,000 minutes. */
const MAX_BASE_BARS = 60_000;
/**
 * Fewer where the exchange's bars are folded into the chart's: a week or a
 * month of Bitget's stock tokens, read as 4h bars, would otherwise page back
 * through some three hundred answers and a phone's worth of data.
 */
const MAX_FOLDED_BARS = 12_000;
/** Timeframes the exchange does not stream (2D, 3M…) pick up new bars this often. */
const POLL_MS = 5 * 60_000;
/** How long a GPU context the browser took is waited for before the chart says so. */
const GPU_WAIT_MS = 3_000;
/**
 * Charts drawn with WebGL2 at once. Chrome keeps only four contexts a worker,
 * every chart is in this one, and past four it takes the oldest back; a chart
 * asking for its own back then takes the next one's, until a layout of twenty
 * charts had four left drawing. The rest draw on their 2D canvas, which holds
 * up at the size many charts on a screen are.
 */
const GPU_CHARTS = 4;
const GPU_GONE = 'graphics lost — reload to redraw';
/** A closed bar REST has not vouched for yet is asked for again after these,
 *  the last repeating until it answers. */
const ASK_MS = [4_000, 15_000, 30_000, 60_000];
/** How late a price may go out when no frame is drawn to carry it. */
const PRICE_UNDRAWN_MS = 250;

interface TfInfo {
  timeframe: string;
  interval: string;
  group: number;
  base: string;
  /** The kline stream the forming bar is built from: its own length, or for a 2D, 3D, 2W or 3M bar the days it groups. */
  live: string;
  barMs: number;
  baseMs: number;
  intervalMs: number;
}

interface Bar {
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  closed: boolean;
}

interface ProfileSpec {
  session: string;
  from: string;
}

const scope = self as unknown as { postMessage(msg: FromWorker): void; onmessage: ((e: MessageEvent<ToWorker>) => void) | null };
const post = (msg: FromWorker): void => scope.postMessage(msg);
const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
const EMPTY = new Float64Array(0);
/** An alert line's style as the engine reads it: see `set_alerts`. */
const DASHES: Record<string, number> = { solid: 1, dashed: 2, dotted: 3 };
const rgba = (color: string | undefined): number => {
  const hex = /^#([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(color ?? '');
  return hex ? parseInt(hex[1]! + (hex[2] ?? 'ff'), 16) : 0;
};

const hub = new StreamHub();
const theme = new Map<string, string>();
/** The reader's time zone, minutes east of UTC; charts write intraday times in it. */
let utcOffset = 0;
const slots = new Map<string, Slot>();
/** The exchanges' marks, as the page sent them. */
let marks: { index: number; box: number; spec: string }[] = [];

const info = (tf: string): TfInfo => timeframe_info(tf) as unknown as TfInfo;

function barCols(bars: readonly Bar[]): Cols {
  return {
    time: Float64Array.from(bars, (b) => b.time),
    open: Float64Array.from(bars, (b) => b.open),
    high: Float64Array.from(bars, (b) => b.high),
    low: Float64Array.from(bars, (b) => b.low),
    close: Float64Array.from(bars, (b) => b.close),
    volume: Float64Array.from(bars, (b) => b.volume),
  };
}

/** The decimals an instrument trades at, read off its recent prices. */
function precisionOf(c: Cols): number {
  let digits = 2;
  for (let i = Math.max(0, c.time.length - 500); i < c.time.length; i += 1) {
    digits = Math.max(digits, decimalsOf(c.open[i]!), decimalsOf(c.high[i]!), decimalsOf(c.low[i]!), decimalsOf(c.close[i]!));
    if (digits >= MAX_PRICE_DECIMALS) return MAX_PRICE_DECIMALS;
  }
  return digits;
}

/** Base bars needed for `bars` bars of `tf` from this series. */
const baseBars = (series: CandleSeries, tf: TfInfo, bars: number): number =>
  // A ratio is folded to the chart's bars already, and asks each side for its own.
  series.source === 'ratio'
    ? bars
    : series.source === 'archive'
      ? Math.min(MAX_BASE_BARS, Math.ceil((bars * tf.barMs) / tf.baseMs))
      : Math.min(intervalMs(series.interval) < tf.barMs ? MAX_FOLDED_BARS : MAX_BASE_BARS, Math.ceil((bars * tf.barMs) / intervalMs(series.interval)));

/** A chart's candles: one market's series, or one market priced in another. */
async function openChartSeries(market: string, tf: TfInfo): Promise<CandleSeries> {
  const legs = ratioLegs(market);
  if (!legs) return openSeries(market, tf.base, tf.interval);
  const [a, b] = await Promise.all(legs.map((leg) => openSeries(leg, tf.base, tf.interval)));
  const fold = (c: Cols) => fold_candles(tf.timeframe, c.time, c.open, c.high, c.low, c.close, c.volume, Date.now()) as unknown as Cols;
  // Loaded only here: a chart of one market never pays for it.
  const { RatioSeries } = await import('./ratio');
  return new RatioSeries([a!, b!], tf.timeframe, fold, (leg, bars) => baseBars(leg, tf, bars));
}

/**
 * Shorter klines combined into the chart's forming bar: the parts of it
 * already closed come from the loaded history, the rest as they stream.
 */
function combiner(timeframe: string, history: Cols): (k: KlineMsg) => KlineMsg | null {
  const parts = new Map<number, KlineMsg>();
  let open = Number.NEGATIVE_INFINITY;
  return (k) => {
    // The engine's own calendar, so a 2D, 2W or 3M bar opens where its folded history does.
    const start = bar_start(timeframe, k.openTime);
    if (start < open) return null;
    if (start > open) {
      open = start;
      parts.clear();
      for (let i = lowerBound(history.time, start); i < history.time.length; i += 1) {
        const t = history.time[i]!;
        parts.set(t, { kind: 'kline', openTime: t, open: history.open[i]!, high: history.high[i]!, low: history.low[i]!, close: history.close[i]!, volume: history.volume[i]!, lastTradeId: 0, closed: true });
      }
    }
    parts.set(k.openTime, k);
    const list = [...parts.values()].sort((a, b) => a.openTime - b.openTime);
    return {
      kind: 'kline',
      openTime: open,
      open: list[0]!.open,
      high: Math.max(...list.map((p) => p.high)),
      low: Math.min(...list.map((p) => p.low)),
      close: list[list.length - 1]!.close,
      volume: list.reduce((sum, p) => sum + p.volume, 0),
      lastTradeId: 0,
      closed: false,
    };
  };
}

class Slot {
  readonly engine: ChartEngine;
  private market = '';
  private timeframe = '';
  private bars = CHART_BARS;
  private series: CandleSeries | null = null;
  /** Closed bars at the chart's timeframe, as the engine holds them. */
  private closed: Cols = emptyCols();
  /** Bars the stream closed that stored history and REST do not cover yet. */
  private extra: Bar[] = [];
  /** The newest bar stored history or REST vouches for; `extra` is the rest. */
  private foldedLast = Number.NEGATIVE_INFINITY;
  /** The newest closed bar REST is still to vouch for, and the timer that keeps asking. */
  private owed = Number.NEGATIVE_INFINITY;
  private owedTimer: ReturnType<typeof setTimeout> | undefined;
  private asks = 0;
  private live: Bar | null = null;
  /** A trade moved the forming bar since the last frame: it goes to the engine once a frame. */
  private liveDue = false;
  /** The bars the engine holds, newest last, and what it holds at rest (see WINDOW_BARS). */
  private window = Number.POSITIVE_INFINITY;
  private rest = Number.POSITIVE_INFINITY;
  private feed: LiveFeedEngine | null = null;
  private gen = 0;
  private cleanup: (() => void)[] = [];
  private status: ChartStatus = { loading: false, error: null, bars: 0, source: null, live: 'off', relayed: false, exhausted: false };
  private legendKey = '';
  private cursor = '';
  private studiesKey = '';
  private metaDirty = true;
  private price = Number.NaN;
  private priceDirty = false;
  private priceTimer: ReturnType<typeof setTimeout> | undefined;
  /** Waiting on the first candles of a market just chosen. */
  private fresh = false;
  /** The closed bars of the market chosen are drawn: until they are, the
   *  forming bar waits, for the chart on screen is still the last market's. */
  private candlesIn = false;
  private demo: ReturnType<typeof setInterval> | undefined;
  /** Its tab is not shown: nothing is drawn, everything else carries on. */
  hidden = false;
  /** The ticker's perpetual on each exchange, whose liquidations the feed shows. */
  private liquidationMarkets: string[] = [];
  private liquidationKey = '';
  /** The studies as last set, for whether open interest is shown. */
  private studies = '';
  private readonly openInterest = new OpenInterestFeed((steps) => {
    this.engine.set_open_interest(steps.time, steps.open, steps.high, steps.low, steps.close);
    this.metaDirty = true;
  });
  private liquidationOff: (() => void)[] = [];
  /** The shapes' GPU context: taken by the browser and not back yet, and whether that has been said. */
  /** The market and timeframe last asked for, for `?debug`. */
  asked: string | null = null;
  private gpuLost = false;
  private gpuGone = false;
  private gpuTimer: ReturnType<typeof setTimeout> | undefined;
  /** Gone with the chart: freeing the engine loses its context, and a loss heard then would reach a freed engine. */
  private gpuListening = new AbortController();

  constructor(
    readonly id: string,
    readonly gl: OffscreenCanvas,
    text: OffscreenCanvas,
    feed: OffscreenCanvas,
    flat = false,
  ) {
    // iOS takes a page's WebGL contexts back when it is put away or short of
    // memory, and the chart then stays blank until the page is loaded again.
    // Asking for it back (preventDefault) is what makes the browser restore
    // it; the engine then makes its GPU state again and draws.
    const { signal } = this.gpuListening;
    gl.addEventListener('webglcontextlost', (e) => {
      e.preventDefault();
      this.gpuLost = true;
      clearTimeout(this.gpuTimer);
      this.gpuTimer = setTimeout(() => {
        if (!this.gpuLost) return;
        this.gpuGone = true;
        this.setStatus({ error: GPU_GONE });
        this.engine.set_message(GPU_GONE);
      }, GPU_WAIT_MS);
    }, { signal });
    gl.addEventListener('webglcontextrestored', () => {
      this.gpuLost = false;
      clearTimeout(this.gpuTimer);
      try {
        this.engine.restore_gpu();
      } catch (e) {
        this.setStatus({ error: `graphics not restored: ${message(e)}` });
        return;
      }
      if (this.gpuGone) {
        this.gpuGone = false;
        this.setStatus({ error: null });
        this.engine.set_message(this.status.loading ? 'Loading…' : '');
      }
      schedule();
    }, { signal });
    this.engine = new ChartEngine(gl, text, feed, '1h', flat);
    for (const [key, value] of theme) this.engine.set_theme_value(key, value);
    this.engine.set_utc_offset(utcOffset);
    // Every exchange's mark, once: a liquidation names the one it came from.
    for (const m of marks) this.engine.set_mark(m.index, m.box, m.spec);
  }

  /**
   * Shown again after its tab was away. The panel mounted afresh and holds
   * nothing, so what is only sent on change — legend, studies, status, price —
   * is sent again.
   */
  show(visible: boolean): void {
    this.hidden = !visible;
    if (!visible) return;
    this.legendKey = '';
    this.studiesKey = '';
    this.cursor = '';
    this.metaDirty = true;
    this.priceDirty = true;
    post({ t: 'status', id: this.id, value: this.status });
  }

  markMeta(): void {
    this.metaDirty = true;
  }

  /** The size it is drawn at, which a capture draws again larger. */
  private shown = { width: 0, height: 0, dpr: 1 };

  resize(width: number, height: number, dpr: number): void {
    this.shown = { width, height, dpr };
    this.engine.resize(width, height, dpr);
  }

  size(): { width: number; height: number; dpr: number } {
    return this.shown;
  }

  setStudies(value: string): void {
    this.studies = value;
    this.engine.set_studies(value);
    this.metaDirty = true;
    this.syncOpenInterest();
  }

  /** Open interest follows the chart's market, bars and the study's venues. */
  private syncOpenInterest(): void {
    const ref = parseMarket(this.market);
    // A window's first bar moves as it widens and narrows: open interest keeps
    // to the first bar of the history behind it instead, and is not fetched again.
    const from = Number.isFinite(this.window) ? this.series?.cols.time : this.closed.time;
    const first = from && from.length > 0 ? from[0]! : Number.NaN;
    let barMs = 0;
    try {
      barMs = info(this.timeframe).barMs;
    } catch {
      barMs = 0;
    }
    this.openInterest.follow(ref ? coinOf(ref.symbol) : '', barMs, first, ref && barMs > 0 ? oiVenues(this.studies) : null);
  }

  destroy(): void {
    this.gpuListening.abort();
    clearInterval(this.demo);
    clearTimeout(this.gpuTimer);
    clearTimeout(this.priceTimer);
    this.watchLiquidations([]);
    this.openInterest.stop();
    this.detach();
    this.engine.free();
  }

  /**
   * The feed follows one ticker on every exchange that lists it. Hits carry
   * their exchange's mark; the list comes from the page, which has the catalogue.
   */
  watchLiquidations(markets: string[]): void {
    // The chart's own market decides the server's ticker: a Lighter BTCUSD chart may list no direct venue at all.
    const symbol = parseMarket(this.market)?.symbol ?? markets.map((m) => parseMarket(m)?.symbol).find(Boolean);
    const key = `${symbol}|${markets.join()}`;
    if (key === this.liquidationKey) return;
    this.liquidationKey = key;
    for (const off of this.liquidationOff.splice(0)) off();
    this.liquidationMarkets = markets;
    this.engine.clear_liquidations();
    if (!symbol) return;
    const ticker = serverTicker(symbol);
    // The server's feed carries every exchange it follows; without it, the chart's own sockets.
    let direct = false;
    this.liquidationOff.push(
      followLiquidations(ticker, {
        hit: (h) => this.engine.push_liquidation(h.usd, h.long, markIndex(h.venue), Date.now()),
        venues: () => {},
        unavailable: () => {
          if (direct || this.liquidationMarkets !== markets) return;
          direct = true;
          this.watchDirectly(markets);
        },
      }),
    );
  }

  private watchDirectly(markets: string[]): void {
    for (const market of markets) {
      const ref = parseMarket(market);
      if (!ref?.venue.endsWith('perp')) continue;
      const mark = markIndex(ref.venue);
      this.liquidationOff.push(
        hub.subscribe(
          ref.venue,
          { kind: 'liquidations', symbol: ref.symbol },
          {
            message: (m: StreamMsg) => {
              if (m.kind === 'liquidation') this.engine.push_liquidation(m.price * m.qty, m.long, mark, Date.now());
            },
            state: () => {},
          },
        ),
      );
    }
  }


  /**
   * Development only: liquidations that never happened, at a given rate, so the
   * pings can be looked at without waiting on the market. Sizes are drawn on a
   * log scale — mostly small hits with the occasional big one, as they arrive
   * in life — and the price wanders a little around the last trade.
   */
  demoLiquidations(perMinute: number): void {
    clearInterval(this.demo);
    this.demo = undefined;
    if (perMinute <= 0) return;
    this.demo = setInterval(
      () => {
        const price = this.price;
        if (!Number.isFinite(price)) return;
        const hit = demoHit(price);
        // Spread across the exchanges the feed follows, as the real ones are.
        const venues = this.liquidationMarkets.length > 0 ? this.liquidationMarkets : MARK_ORDER;
        const from = venues[Math.floor(Math.random() * venues.length)]!;
        this.engine.push_liquidation(hit.price * hit.qty, hit.long, markIndex(from), Date.now());
      },
      Math.max(60_000 / perMinute, 50),
    );
  }

  private detach(): void {
    this.gen += 1;
    for (const fn of this.cleanup.splice(0)) fn();
    clearTimeout(this.owedTimer);
    this.owedTimer = undefined;
    this.owed = Number.NEGATIVE_INFINITY;
    this.feed?.free();
    this.feed = null;
    this.series = null;
  }

  async load(market: string, timeframe: string, bars = CHART_BARS): Promise<void> {
    if (market === this.market && timeframe === this.timeframe && bars === this.bars && !this.status.error) return;
    this.detach();
    const gen = this.gen;
    this.market = market;
    this.timeframe = timeframe;
    this.bars = bars;
    this.closed = emptyCols();
    this.extra = [];
    this.live = null;
    this.price = Number.NaN;
    this.markPrice();
    this.engine.clear_live();
    this.engine.set_days(EMPTY, EMPTY, EMPTY, EMPTY, EMPTY);
    // What is drawn stays, dimmed, rather than blanking the chart for the
    // third of a second the next market's candles take to arrive.
    this.fresh = true;
    this.engine.set_loading(true);
    this.metaDirty = true;
    let tf: TfInfo;
    try {
      tf = info(timeframe);
      this.engine.set_timeframe(timeframe);
    } catch (e) {
      this.fail(message(e));
      return;
    }
    this.liveDue = false;
    this.window = this.rest = this.restWindow(tf);
    this.engine.set_message('Loading…');
    this.candlesIn = false;
    this.setStatus({ loading: true, error: null, bars: 0, source: null, live: 'off', relayed: false, exhausted: false });
    try {
      const series = await openChartSeries(market, tf);
      if (gen !== this.gen) return;
      this.series = series;
      this.cleanup.push(series.subscribe(() => this.refold()));
      // The stream is asked for beside the history, and the forming bar from
      // REST with them, so both are in hand when the closed bars are drawn.
      const early = this.liveEarly(tf);
      if (early) this.startLive(gen, tf);
      this.seed(gen, tf);
      await series.ensure(baseBars(series, tf, bars));
      if (gen !== this.gen) return;
      this.refold();
      if (!early) this.startLive(gen, tf);
    } catch (e) {
      if (gen === this.gen) this.fail(message(e));
    }
  }

  /**
   * Whether the stream can start before the history is in. A ratio's bar is
   * its two markets' bars joined, and a bar the exchange does not stream at
   * the chart's length is built from shorter ones — both read the history.
   */
  private liveEarly(tf: TfInfo): boolean {
    const ref = parseMarket(this.market);
    return !ratioLegs(this.market) && ref !== null && tf.group === 1 && marketInterval(this.market, ref.venue, tf.live) === tf.live;
  }

  /**
   * The forming bar from REST, for the moment before the stream's first word —
   * which for a quiet market can be seconds: without it the chart showed the
   * last closed bar, its close as the price, hours old on a four-hour chart,
   * and then jumped. The stream's bar takes over the moment it comes, for a
   * kline with no trade id is older than any the stream sends.
   */
  private seed(gen: number, tf: TfInfo): void {
    const ref = parseMarket(this.market);
    if (!ref || ratioLegs(this.market)) return;
    const feed = (this.feed ??= new LiveFeedEngine(this.timeframe));
    const start = bar_start(this.timeframe, Date.now());
    void formingRows(this.market, marketInterval(this.market, ref.venue, tf.live), start)
      .then((rows) => {
        if (gen !== this.gen || rows.length === 0 || feed.bar() !== null) return;
        const high = Math.max(...rows.map((r) => r[2]!));
        const low = Math.min(...rows.map((r) => r[3]!));
        const volume = rows.reduce((sum, r) => sum + r[5]!, 0);
        if (feed.apply_kline(start, rows[0]![1]!, high, low, rows[rows.length - 1]![4]!, volume, -1, false)) this.onLive(feed.bar() as Bar | null);
      })
      .catch(() => undefined);
  }

  loadFailed(e: unknown): void {
    this.setStatus({ loading: false, error: `not loaded: ${message(e)}` });
  }

  private fail(error: string): void {
    this.fresh = false;
    this.engine.set_loading(false);
    this.engine.set_candles(EMPTY, EMPTY, EMPTY, EMPTY, EMPTY, EMPTY);
    this.engine.set_message(`Could not load candles — ${error}`);
    this.setStatus({ loading: false, error });
  }

  private lastClosedTime(): number {
    const t = this.closed.time;
    return t.length > 0 ? t[t.length - 1]! : Number.NEGATIVE_INFINITY;
  }

  private refold(): void {
    const series = this.series;
    if (!series) return;
    if (Number.isFinite(this.window)) {
      // As the week goes on, the window at rest keeps back to its open.
      const rest = this.restWindow(info(this.timeframe));
      this.window = this.window <= this.rest ? rest : Math.max(this.window, rest);
      this.rest = rest;
    }
    const c = this.tail(series);
    const folded = fold_candles(this.timeframe, c.time, c.open, c.high, c.low, c.close, c.volume, Date.now()) as unknown as Cols;
    const last = folded.time.length > 0 ? folded.time[folded.time.length - 1]! : Number.NEGATIVE_INFINITY;
    this.foldedLast = last;
    // A ratio has no exchange's tick to keep to: its decimals come from its own
    // moves. Any other market's are read off its prices (-1).
    this.engine.set_price_decimals(series.source === 'ratio' ? (series as RatioSeries).decimals() : -1);
    this.extra = this.extra.filter((b) => b.time > last);
    const closed = this.extra.length > 0 ? concatCols([folded, barCols(this.extra)]) : folded;
    // The exchange vouching for a bar the stream already closed changes no bar:
    // working every study out again over all of them — the slowest thing a long
    // minute chart does, and asked for again until the exchange answers — is skipped.
    const same = !this.fresh && sameCols(closed, this.closed);
    this.closed = closed;
    if (!same) this.pushCandles();
    this.syncOpenInterest();
    this.setStatus({ loading: false, error: null, bars: this.closed.time.length, source: series.source, exhausted: series.exhausted });
  }

  /** The bars the engine holds at rest: see WINDOW_BARS. */
  private restWindow(tf: TfInfo): number {
    if (tf.barMs > WINDOW_MAX_BAR_MS) return Number.POSITIVE_INFINITY;
    const sinceMonday = Math.ceil(((Date.now() - MONDAY_MS) % WEEK_MS) / tf.barMs);
    return Math.max(WINDOW_BARS, this.bars, sinceMonday + 100);
  }

  /**
   * The base bars the window's bars fold from, so a bar closing works over the
   * window's bars and not the months behind them. All of them for a longer chart.
   */
  private tail(series: CandleSeries): Cols {
    const c = series.cols;
    const n = c.time.length;
    if (!Number.isFinite(this.window)) return c;
    // Counted, not timed: across a gap in the bars a stretch of time holds
    // fewer, and widening by time could add none.
    const barMs = info(this.timeframe).barMs;
    let from = n - (this.window + 1) * Math.max(1, Math.round(barMs / intervalMs(series.interval)));
    if (from <= 0) return c;
    // From the first of a bar's base bars, so the oldest bar held is whole.
    while (from < n && Math.floor(c.time[from]! / barMs) === Math.floor(c.time[from - 1]! / barMs)) from += 1;
    return sliceCols(c, from);
  }

  /** Back about the newest bars, the window lets the older ones go again. */
  private narrow(): void {
    if (this.window <= this.rest) return;
    const held = Math.min(this.window, this.closed.time.length);
    if (this.engine.visible_from() < held - this.rest + WIDEN_BARS) return;
    this.window = this.rest = this.restWindow(info(this.timeframe));
    this.refold();
  }

  private pushCandles(): void {
    const all = this.closed;
    const k = all.time.length > this.window ? sliceCols(all, all.time.length - this.window) : all;
    this.engine.set_candles(k.time, k.open, k.high, k.low, k.close, k.volume);
    if (this.fresh && k.time.length > 0) {
      // A market of its own opens at its own last bars, not where the one
      // before was scrolled to.
      this.fresh = false;
      this.engine.set_loading(false);
      this.engine.reset_view();
    }
    // A bar longer than a day opens before the week, month or year it sits in,
    // so key levels read the daily bars it was folded from where there are any.
    const days = this.series?.interval === '1d' && info(this.timeframe).barMs > intervalMs('1d') ? this.series.cols : null;
    this.engine.set_days(days?.time ?? EMPTY, days?.open ?? EMPTY, days?.high ?? EMPTY, days?.low ?? EMPTY, days?.close ?? EMPTY);
    this.engine.set_message(k.time.length > 0 ? '' : 'No closed candles for this market and timeframe yet.');
    this.candlesIn = true;
    this.pushLive();
    this.metaDirty = true;
    if (this.live) this.setPrice(this.live.close);
    else if (k.time.length > 0) this.setPrice(k.close[k.time.length - 1]!);
  }

  private startLive(gen: number, tf: TfInfo): void {
    // Back from a lock: the bars that closed while the phone slept are asked
    // for at once, before the stream has even found its feet again.
    this.cleanup.push(onWake(() => void this.series?.refreshTail(true).catch(() => undefined)));
    const legs = ratioLegs(this.market);
    if (legs && this.series?.source === 'ratio') {
      this.startRatioLive(gen, tf, legs, this.series as RatioSeries);
      return;
    }
    const ref = parseMarket(this.market);
    if (!ref) {
      this.setStatus({ live: 'off' });
      const timer = setInterval(() => void this.series?.refreshTail(), POLL_MS);
      this.cleanup.push(() => clearInterval(timer));
      return;
    }
    const feed = (this.feed ??= new LiveFeedEngine(this.timeframe));
    // A bar of several days streams them, and an exchange without this length
    // a shorter one: either is combined here into the chart's bar.
    const streamed = marketInterval(this.market, ref.venue, tf.live);
    const combine = streamed !== tf.live || tf.group > 1 ? combiner(this.timeframe, this.series?.cols ?? emptyCols()) : null;
    const listener: StreamListener = {
      message: (m: StreamMsg) => {
        if (gen !== this.gen) return;
        let changed: boolean;
        if (m.kind === 'kline') {
          const k = combine ? combine(m) : m;
          changed = k !== null && feed.apply_kline(k.openTime, k.open, k.high, k.low, k.close, k.volume, k.lastTradeId, k.closed);
        } else {
          changed = m.kind === 'trade' && feed.apply_trade(m.time, m.price, m.qty, m.lastTradeId);
        }
        if (changed) this.onLive(feed.bar() as Bar | null);
      },
      state: (state: SocketState, relayed: boolean) => {
        if (gen !== this.gen) return;
        const was = this.status.live;
        this.setStatus({ live: state, relayed });
        // Back from an outage: bars that closed meanwhile come from REST.
        if (state === 'live' && (was === 'retrying' || was === 'stalled')) void this.series?.refreshTail().catch(() => undefined);
      },
    };
    let off = false;
    this.cleanup.push(() => {
      off = true;
    });
    void nativeOf(this.market).then((symbol) => {
      if (off || gen !== this.gen) return;
      this.cleanup.push(hub.subscribe(ref.venue, { kind: 'kline', symbol, interval: streamed, quiet: lacksUtcOf(this.market) }, listener));
      // Binance's kline publishes about once a second; trades fill in between.
      if (ref.venue.startsWith('binance')) this.cleanup.push(hub.subscribe(ref.venue, { kind: 'trades', symbol }, listener));
    });
  }

  /**
   * A ratio's forming bar: each market's own stream — combined into the
   * chart's bar where its exchange streams a shorter one — and the two divided
   * whenever both stand on the same bar. It is live when both are.
   */
  private startRatioLive(gen: number, tf: TfInfo, legs: [string, string], series: RatioSeries): void {
    const feed = new LiveFeedEngine(this.timeframe);
    this.feed = feed;
    let off = false;
    this.cleanup.push(() => {
      off = true;
    });
    legs.forEach((leg, i) => {
      const ref = parseMarket(leg) as MarketRef;
      const streamed = marketInterval(leg, ref.venue, tf.live);
      const combine = streamed !== tf.live || tf.group > 1 ? combiner(this.timeframe, series.legs[i]!.cols) : null;
      const listener: StreamListener = {
        message: (m: StreamMsg) => {
          if (gen !== this.gen || m.kind !== 'kline') return;
          const k = combine ? combine(m) : m;
          const r = k && series.join(i as 0 | 1, k);
          if (r && feed.apply_kline(r.openTime, r.open, r.high, r.low, r.close, 0, 0, r.closed)) this.onLive(feed.bar() as Bar | null);
        },
        state: (state: SocketState, viaRelay: boolean) => {
          if (gen !== this.gen) return;
          const was = this.status.live;
          const [worst, relayed] = series.stream(i as 0 | 1, state, viaRelay);
          this.setStatus({ live: worst, relayed });
          if (worst === 'live' && (was === 'retrying' || was === 'stalled')) void this.series?.refreshTail().catch(() => undefined);
        },
      };
      void nativeOf(leg).then((symbol) => {
        if (off || gen !== this.gen) return;
        this.cleanup.push(hub.subscribe(ref.venue, { kind: 'kline', symbol, interval: streamed, quiet: lacksUtcOf(leg) }, listener));
      });
    });
  }

  private onLive(bar: Bar | null): void {
    if (!bar) return;
    const prev = this.live;
    this.live = bar;
    if (prev && bar.time > prev.time) {
      // The previous bar is final. Keep it until stored history or REST covers it.
      if (prev.time > this.lastClosedTime()) {
        const done = { ...prev, closed: true };
        this.extra.push(done);
        if (this.candlesIn) {
          this.closed = concatCols([this.closed, barCols([done])]);
          this.pushCandles();
        }
      }
      if (this.candlesIn) this.confirm(prev.time);
    }
    // Before the closed bars are in, the chart drawn is the last market's: the
    // bar and its price wait, to go out with them.
    if (!this.candlesIn) return;
    this.liveDue = true;
    this.setPrice(bar.close);
  }

  /**
   * A bar the stream has just finished is only what the stream saw of it. A
   * socket that went quiet before the close leaves it short — its body stops
   * where the feed did, while the price went on — and nothing else would put it
   * right until the NEXT bar closed: an hour, on an hourly chart. REST has the
   * bar the exchange actually closed, so it is asked until it answers with it:
   * at once, then again and again, further apart, each time past the throttle
   * that keeps ordinary polling polite.
   *
   * Once was not enough. The bar rolls when the exchange's first trade of the
   * next one arrives, stamped by the exchange's clock; a computer whose clock
   * runs a second behind still thinks the old bar is open, asks for nothing,
   * and the throttle then turned away every other ask for half a minute.
   *
   * Three times was not enough either. A phone back from a lock rolls from the
   * bar it fell asleep in to the one forming now, and the bars between come
   * only from REST; its network may take longer than a quarter of a minute to
   * answer, and the chart then jumped from the one bar to the other, the ones
   * between missing, until the next bar closed.
   */
  private confirm(time: number): void {
    if (time > this.owed) {
      this.owed = time;
      this.asks = 0;
    }
    this.ask();
  }

  private ask(): void {
    clearTimeout(this.owedTimer);
    this.owedTimer = undefined;
    const series = this.series;
    if (!series || this.foldedLast >= this.owed) return;
    void series.refreshTail(true).catch(() => undefined);
    this.owedTimer = setTimeout(() => this.ask(), ASK_MS[Math.min(this.asks, ASK_MS.length - 1)]);
    this.asks += 1;
  }

  private pushLive(): void {
    const b = this.live;
    if (b && b.time > this.lastClosedTime()) this.engine.set_live(b.time, b.open, b.high, b.low, b.close, b.volume);
  }

  private setPrice(price: number): void {
    if (price === this.price) return;
    this.price = price;
    this.markPrice();
  }

  /**
   * The price goes out with the next frame — or, when no frame comes because
   * the page is in the background or this tab is not shown, a moment later:
   * the browser tab's title follows it there.
   */
  private markPrice(): void {
    this.priceDirty = true;
    this.priceTimer ??= setTimeout(() => {
      this.priceTimer = undefined;
      this.flushPrice();
    }, PRICE_UNDRAWN_MS);
  }

  private flushPrice(): void {
    if (!this.priceDirty) return;
    this.priceDirty = false;
    post({ t: 'price', id: this.id, market: this.market, price: this.price, precision: this.engine.precision() });
  }

  private setStatus(patch: Partial<ChartStatus>): void {
    // A market loaded while the graphics are gone is still not drawn: the word stays.
    const next = { ...this.status, ...patch, ...(this.gpuGone && !patch.error ? { error: GPU_GONE } : {}), market: this.market, timeframe: this.timeframe };
    const keys = Object.keys(next) as (keyof ChartStatus)[];
    if (keys.every((k) => next[k] === this.status[k])) return;
    this.status = next;
    post({ t: 'status', id: this.id, value: next });
  }

  pointer(kind: PointerKind, x: number, y: number, button: number, shift = false, ctrl = false, touch = false): void {
    const e = this.engine;
    e.set_shift(shift);
    e.set_ctrl(ctrl);
    e.set_touch(touch);
    if (kind === 'move') e.pointer_move(x, y);
    else if (kind === 'down') e.pointer_down(x, y, button);
    else if (kind === 'up') e.pointer_up(x, y);
    else if (kind === 'leave') e.pointer_leave();
    else if (kind === 'hold') e.pointer_hold(x, y);
    else e.double_click(x, y);
    this.afterInput();
  }

  afterInput(): void {
    const cursor = this.engine.cursor();
    if (cursor !== this.cursor) {
      this.cursor = cursor;
      post({ t: 'cursor', id: this.id, value: cursor });
    }
    this.flushEvents();
  }

  private flushEvents(): void {
    for (const event of this.engine.take_events() as ChartEvent[]) {
      if (event.type === 'needHistory') void this.loadOlder();
      else post({ t: 'event', id: this.id, event });
    }
  }

  private async loadOlder(): Promise<void> {
    // Bars already held come into the window before any are fetched.
    const held = this.series?.cols.time;
    if (held && held.length > 0 && this.closed.time.length > 0 && held[0]! < this.closed.time[0]!) {
      this.window += WIDEN_BARS;
      this.refold();
      return;
    }
    const series = this.series;
    const gen = this.gen;
    if (!series || series.exhausted) return;
    if (Number.isFinite(this.window)) this.window += WIDEN_BARS;
    this.setStatus({ loading: true });
    try {
      await series.older();
      if (gen === this.gen) this.setStatus({ loading: false, exhausted: series.exhausted });
    } catch (e) {
      if (gen === this.gen) this.setStatus({ loading: false, error: `older history: ${message(e)}` });
    }
  }

  /** The forming bar as trades last moved it, into the engine: once a frame, and before a picture. */
  flushLive(): void {
    if (!this.liveDue) return;
    this.liveDue = false;
    this.pushLive();
  }

  frame(now: number): void {
    this.flushLive();
    this.narrow();
    if (this.engine.draw(now)) {
      const legend = this.engine.legend();
      const key = JSON.stringify(legend);
      if (key !== this.legendKey) {
        this.legendKey = key;
        post({ t: 'legend', id: this.id, value: legend });
      }
      this.flushEvents();
    }
    if (this.metaDirty) {
      this.metaDirty = false;
      const studies = this.engine.studies();
      const key = JSON.stringify(studies);
      if (key !== this.studiesKey) {
        this.studiesKey = key;
        post({ t: 'studies', id: this.id, value: studies });
      }
    }
    this.flushPrice();
  }
}

// ---------------------------------------------------------------- analysis calls

/** A series kept alive and loaded for the length of one call. */
async function hold(market: string, timeframe: string, bars: number): Promise<{ series: CandleSeries; tf: TfInfo; release: () => void }> {
  const tf = info(timeframe);
  const series = await openChartSeries(market, tf);
  const release = series.subscribe(() => {});
  try {
    await series.ensure(baseBars(series, tf, bars));
  } catch (e) {
    release();
    throw e;
  }
  return { series, tf, release };
}

const lastBars = (c: Cols, n: number): Cols => sliceCols(c, Math.max(0, c.time.length - n));

/** Source bars in one profile session. */
const SESSION_BARS: Record<string, number> = { '1d': 24, '1w': 7, '1M': 31 };

async function valuesCall({ market, timeframe, basis, profile = true, bars: wanted = CHART_BARS }: Calls['values']['args']): Promise<IndicatorValues | null> {
  const chart = await hold(market, timeframe, Math.max(50, Math.min(CHART_BARS, wanted)));
  const spec = profile ? (profile_spec(timeframe) as ProfileSpec | null) : null;
  let src: Awaited<ReturnType<typeof hold>> | null = null;
  try {
    const c = chart.series.cols;
    const bars = fold_candles(timeframe, c.time, c.open, c.high, c.low, c.close, c.volume, Date.now()) as unknown as Cols;
    let s = emptyCols();
    if (spec) {
      // Enough source bars to cover the prior session of the last bar, and the view.
      const want = Math.min(20_000, Math.ceil((Math.min(bars.time.length, 300) * chart.tf.barMs) / info(spec.from).barMs) + 2 * (SESSION_BARS[spec.session] ?? 31));
      src = await hold(market, spec.from, want);
      s = lastBars(src.series.cols, want);
    }
    const k = lastBars(bars, 300);
    const out = indicator_values(timeframe, bars.time, bars.open, bars.high, bars.low, bars.close, bars.volume, s.time, s.open, s.high, s.low, s.close, s.volume, basis) as Omit<IndicatorValues, 'precision'> | null;
    return out && { ...out, precision: precisionOf(k) };
  } finally {
    chart.release();
    src?.release();
  }
}

async function dailyCall({ market, days }: Calls['daily']['args']): Promise<DailyBars> {
  const held = await hold(market, '1d', Math.max(1, Math.min(CHART_BARS, days)));
  try {
    const c = held.series.cols;
    const bars = lastBars(fold_candles('1d', c.time, c.open, c.high, c.low, c.close, c.volume, Date.now()) as unknown as Cols, days);
    return { time: [...bars.time], open: [...bars.open], high: [...bars.high], low: [...bars.low], close: [...bars.close], precision: precisionOf(bars) };
  } finally {
    held.release();
  }
}

type Handlers = { [K in CallName]: (args: Calls[K]['args']) => Calls[K]['result'] | Promise<Calls[K]['result']> };

const calls: Handlers = {
  studyCatalog: () => study_catalog() as StudyDefinition[],
  key: ({ id, key }) => {
    const slot = slots.get(id);
    if (!slot) return false;
    const used = slot.engine.key(key);
    slot.afterInput();
    return used;
  },
  values: valuesCall,
  daily: dailyCall,
  capture: captureCall,
  prefetch: prefetchCall,
  ping: ({ id }) => ({ worker: WORKER, up: Math.round((Date.now() - STARTED) / 1000), got, slots: slots.size, asked: slots.get(id)?.asked ?? null }),
};

/**
 * A market the reader is about to ask for: the pointer resting on a watchlist
 * row is enough. The series lands in the registry, where the chart finds it
 * already loaded — on a market with no archive that is the third of a second
 * the exchange takes. Failures are silent: nobody asked for anything yet.
 */
async function prefetchCall({ market, timeframe }: Calls['prefetch']['args']): Promise<null> {
  try {
    const tf = info(timeframe);
    const series = await openChartSeries(market, tf);
    // Held while it loads, so it is not evicted before the chart arrives.
    const release = series.subscribe(() => {});
    try {
      await series.ensure(baseBars(series, tf, CHART_BARS));
    } finally {
      release();
    }
  } catch {
    // The chart will say what went wrong when it is really asked for.
  }
  return null;
}

/** The most a picture is blown up: past this it is pixels, not detail. */
const MAX_CAPTURE_SCALE = 6;

/**
 * The chart as it stands, drawn again into canvases of its own at `wide`
 * pixels across. Nothing on screen changes: the engine goes back to the size it
 * was at the moment it is done.
 */
async function captureCall({ id, wide }: Calls['capture']['args']): Promise<Blob> {
  const slot = slots.get(id);
  if (!slot) throw new Error('that chart is not open');
  const { width, height, dpr } = slot.size();
  if (!(width > 0 && height > 0)) throw new Error('that chart has no size yet');
  const scale = Math.min(MAX_CAPTURE_SCALE, Math.max(1, wide / (width * dpr)));
  const [w, h] = [Math.round(width * dpr * scale), Math.round(height * dpr * scale)];
  const shapes = new OffscreenCanvas(w, h);
  const labels = new OffscreenCanvas(w, h);
  slot.flushLive();
  slot.engine.draw_into(shapes, labels, scale, Date.now());
  const out = new OffscreenCanvas(w, h);
  const ctx = out.getContext('2d');
  if (!ctx) throw new Error('this browser will not draw the picture');
  ctx.drawImage(shapes, 0, 0);
  ctx.drawImage(labels, 0, 0);
  return await out.convertToBlob({ type: 'image/png' });
}

async function answer(req: number, name: CallName, args: unknown): Promise<void> {
  try {
    const value = await (calls[name] as (a: unknown) => unknown)(args);
    post({ t: 'reply', req, value });
  } catch (e) {
    post({ t: 'reply', req, error: message(e) });
  }
}

// ---------------------------------------------------------------- loop and messages

const raf: (cb: () => void) => void =
  typeof requestAnimationFrame === 'function' ? (cb) => void requestAnimationFrame(cb) : (cb) => void setTimeout(cb, 16);
let scheduled = false;

/** Frames run while a chart is seen: with every one hidden the loop stops, and showing one starts it. */
function schedule(): void {
  if (scheduled || ![...slots.values()].some((s) => !s.hidden)) return;
  scheduled = true;
  raf(tick);
}

function tick(): void {
  scheduled = false;
  const now = Date.now();
  for (const slot of slots.values()) {
    if (slot.hidden) continue;
    try {
      slot.frame(now);
    } catch (e) {
      console.error('chart frame failed', e);
    }
  }
  schedule();
}

function handle(msg: ToWorker): void {
  if (msg.t === 'call') {
    void answer(msg.req, msg.name, msg.args);
    return;
  }
  if (msg.t === 'timeZone') {
    utcOffset = msg.offsetMinutes;
    for (const slot of slots.values()) slot.engine.set_utc_offset(utcOffset);
    return;
  }
  if (msg.t === 'marks') {
    marks = msg.list;
    for (const slot of slots.values()) for (const m of marks) slot.engine.set_mark(m.index, m.box, m.spec);
    return;
  }
  if (msg.t === 'theme') {
    theme.set(msg.key, msg.value);
    for (const slot of slots.values()) {
      slot.engine.set_theme_value(msg.key, msg.value);
      slot.markMeta();
    }
    return;
  }
  if (msg.t === 'create') {
    try {
      // Past the graphics budget a chart draws its shapes on its 2D canvas.
      const gpu = [...slots.values()].filter((s) => s.engine.renderer() === 'webgl2').length;
      const slot = new Slot(msg.id, msg.gl, msg.text, msg.feed, msg.flat === true || gpu >= GPU_CHARTS);
      slot.resize(msg.width, msg.height, msg.dpr);
      slots.set(msg.id, slot);
      schedule();
    } catch (e) {
      post({ t: 'status', id: msg.id, value: { loading: false, error: message(e), bars: 0, source: null, live: 'off', relayed: false, exhausted: false } });
    }
    return;
  }
  const slot = slots.get(msg.id);
  if (!slot) return;
  switch (msg.t) {
    case 'destroy':
      slot.destroy();
      slots.delete(msg.id);
      break;
    case 'resize':
      slot.resize(msg.width, msg.height, msg.dpr);
      break;
    case 'visible':
      slot.show(msg.value);
      schedule();
      break;
    case 'loseGpu':
      // Development only: the GPU context taken as iOS takes it, and given back after `ms` (never when negative).
      if (import.meta.env.DEV) {
        const ext = (slot.gl.getContext('webgl2') as WebGL2RenderingContext | null)?.getExtension('WEBGL_lose_context');
        ext?.loseContext();
        if (ext && msg.ms >= 0) setTimeout(() => ext.restoreContext(), msg.ms);
      }
      break;
    case 'demoLiquidations':
      slot.demoLiquidations(msg.perMinute);
      break;
    case 'liquidationMarkets':
      slot.watchLiquidations(msg.markets);
      break;
    case 'market':
      slot.asked = `${msg.market} ${msg.timeframe}`;
      // A load that throws says so on the chart: unheard, the chart kept the
      // last market under the new one's name — and a market already noted as
      // the one held, without an error, is not loaded again when asked.
      slot.load(msg.market, msg.timeframe, msg.bars).catch((e: unknown) => slot.loadFailed(e));
      break;
    case 'studies':
      slot.setStudies(msg.value);
      break;
    case 'hidden':
      slot.engine.set_hidden(msg.value);
      slot.markMeta();
      break;
    case 'log':
      slot.engine.set_log_scale(msg.value);
      break;
    case 'tpo':
      slot.engine.set_tpo(msg.value);
      slot.markMeta();
      break;
    case 'tpoSettings':
      slot.engine.set_tpo_settings(msg.value);
      break;
    case 'paneWeights':
      slot.engine.set_pane_weights(msg.value);
      break;
    case 'tool':
      slot.engine.set_tool(msg.value, msg.style);
      break;
    case 'magnet':
      slot.engine.set_magnet(msg.value);
      break;
    case 'drawings':
      slot.engine.set_drawings(msg.list);
      break;
    case 'alerts':
      slot.engine.set_alerts(Float64Array.from(msg.list.flatMap((a) => [a.id, a.price, rgba(a.style?.color), DASHES[a.style?.dash ?? ''] ?? 0, a.style?.width ?? 0])));
      break;
    case 'footprint':
      slot.engine.set_footprint(msg.flat, msg.grid);
      break;
    case 'footprintShown':
      slot.engine.set_footprint_shown(msg.value);
      break;
    case 'footprintOptions':
      slot.engine.set_footprint_options(msg.value);
      break;
    case 'flow':
      slot.engine.set_flow(Float64Array.from(msg.marks.flatMap((m) => [m.at, m.price, m.usd])));
      break;
    case 'selected':
      slot.engine.set_selected(msg.value ?? Number.NaN);
      break;
    case 'resetView':
      slot.engine.reset_view();
      break;
    case 'autoScale':
      slot.engine.set_auto_scale();
      slot.afterInput();
      break;
    case 'range':
      slot.engine.show_range(msg.days ?? Number.NaN);
      break;
    case 'pointer':
      slot.pointer(msg.kind, msg.x, msg.y, msg.button, msg.shift, msg.ctrl, msg.touch);
      break;
    case 'wheel':
      slot.engine.wheel(msg.x, msg.y, msg.dx, msg.dy);
      slot.afterInput();
      break;
  }
}

let ready = false;
const queued: ToWorker[] = [];

/** Messages from the page so far, and which worker this is and since when, for `?debug`. */
let got = 0;
const WORKER = Math.random().toString(36).slice(2, 6);
const STARTED = Date.now();

scope.onmessage = (e) => {
  got += 1;
  if (ready) handle(e.data);
  else queued.push(e.data);
};

init({ module_or_path: wasmUrl })
  .then(() => {
    ready = true;
    post({ t: 'ready' });
    for (const msg of queued.splice(0)) handle(msg);
  })
  .catch((e: unknown) => post({ t: 'fatal', message: `the chart engine failed to load: ${message(e)}` }));
