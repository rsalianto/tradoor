/** Messages between the page and the engine worker. */
import type { SocketState } from './streams';
import type { AlertStyle } from '../lib/api';

export type LiveStatus = 'off' | SocketState;

export interface Legend {
  index: number | null;
  time: number;
  ohlc: string;
  change: string;
  up: boolean;
  volume: string;
  /** Formatted value per study line, keyed `studyId:lineKey`. */
  values: Record<string, string>;
  /** The pointer over the price pane, and the price there. */
  cursor: { y: number; price: number } | null;
  /** Where the price axis starts, in CSS pixels from the chart's left. */
  axisX: number;
  /** Where the bar under the pointer stands, in the same pixels. */
  barX: number | null;
  /** The alert line under the pointer. */
  alert: { id: number; y: number } | null;
  /** The flow mark under the pointer: which one, and where it sits. */
  flow: { index: number; x: number; y: number } | null;
  /** The footprint row under the pointer, as the screen groups it: its bar, the
   *  prices it covers, what was bought and sold there, and where it sits. */
  footprint: { time: number; price: number; step: number; buy: number; sell: number; x: number; y: number } | null;
  /** The TPO profile under the pointer, for the menu on it. */
  tpo: TpoHover | null;
  /** Where the page draws the tradoor mark over the price pane, and in what inks; null where it has no room. */
  mark: { x: number; y: number; accent: string; bear: string } | null;
}

/** A TPO profile under the pointer: when it runs, its neighbours' starts, how it
 *  was edited, where a split would fall, and its levels by name. */
export interface TpoHover {
  start: number;
  end: number;
  prev: number | null;
  next: number | null;
  expanded: boolean;
  merged: boolean;
  splitAt: number | null;
  prices: [string, number][];
}

export interface StudyLine {
  id: string;
  label: string;
  color: string;
  /** 0: not stroked, it only carries values for the legend and the axis. */
  width: number;
}

export interface StudyMeta {
  id: string;
  label: string;
  group: string;
  hidden: boolean;
  lines: StudyLine[];
  stats: { label: string; value: string }[];
}

export interface StudyParam {
  name: string;
  default: number;
  min: number;
  max: number;
  step: number;
}

/** What the engine computes with; lib/study-schema says the rest. */
export interface StudyDefinition {
  kind: string;
  short: string;
  group: string;
  params: StudyParam[];
}

export interface DrawingPoint {
  time: number;
  price: number;
}

export interface DrawingShape {
  id: number | null;
  kind: string;
  points: DrawingPoint[];
  label: string | null;
  style: Record<string, string | number | boolean>;
  /** Where it was drawn: on a longer timeframe its points stand on the bars holding them. */
  timeframe?: string;
}

export type ChartEvent =
  /** `pane`: the study pane it was drawn in (`pct`, `oi`…), on its values; absent for the candles'. */
  | { type: 'drawingCreated'; kind: string; points: DrawingPoint[]; pane?: string }
  | { type: 'selection'; id: number | null }
  | { type: 'delete'; id: number }
  | { type: 'drawingChanged'; id: number; points: DrawingPoint[] }
  | { type: 'editRequested'; id: number }
  | { type: 'toolFinished' }
  | { type: 'needHistory'; before: number }
  | { type: 'paneWeights'; value: string }
  | { type: 'footprintTable'; value: number }
  | { type: 'autoScale'; value: boolean }
  | { type: 'alertMoved'; id: number; price: number };

export interface ChartStatus {
  loading: boolean;
  error: string | null;
  bars: number;
  /** Stored partitions, or the exchange's REST API for a market nobody archived. */
  source: 'archive' | 'exchange' | 'ratio' | null;
  live: LiveStatus;
  /** Through the tradoor server, because this network blocks the exchange. */
  relayed: boolean;
  /** The oldest bar there is has been loaded. */
  exhausted: boolean;
  /** What the status is of: the market and timeframe last asked for. */
  market?: string;
  timeframe?: string;
}

export interface ProfileLevels {
  session: string;
  from: string;
  basis: string;
  vah: number | null;
  val: number | null;
  poc: number | null;
  covered: number;
  total: number;
}

export interface IndicatorValues {
  time: number;
  close: number;
  ema12: number | null;
  ema25: number | null;
  ema200: number | null;
  rsi14: number | null;
  stochK: number | null;
  stochD: number | null;
  profile: ProfileLevels | null;
  precision: number;
}

export type ProfileBasis = 'volume' | 'tpo';

export interface DailyBars {
  time: number[];
  open: number[];
  high: number[];
  low: number[];
  close: number[];
  precision: number;
}

/** Where a bar replay stands: bars opening from `at` on are hidden, and none are when it is null. */
export interface ReplayState {
  at: number | null;
  /** Nothing closed is left to show. */
  ended: boolean;
  /** The length of the bars the chart is folded from: the shortest step there is. */
  base: number;
}

/** One of the reader's formulas on a chart: its study id (`fx-abc:14`), legend label, text and settings. */
export interface FormulaRun {
  id: string;
  label: string;
  source: string;
  inputs: number[];
}

export interface FormulaInput {
  name: string;
  label: string;
  default: number;
  min?: number;
  max?: number;
  step: number;
}

/** A formula checked (crates/formula): what it offers and draws, or its first mistake, by line and column. */
export type FormulaDescription =
  | { ok: true; ownPane: boolean; inputs: FormulaInput[]; plots: { kind: string; color: number; title: string }[]; marks: string[] }
  | { ok: false; line: number; col: number; message: string };

/** Request/response calls, answered by `reply`. */
export interface Calls {
  /** A formula checked by the formula module, which loads with the first. */
  formulaDescribe: { args: { source: string }; result: FormulaDescription };
  studyCatalog: { args: Record<string, never>; result: StudyDefinition[] };
  key: { args: { id: string; key: string }; result: boolean };
  /** `profile: false` skips the session profile; `bars` caps the history read (1,500 by default). */
  values: { args: { market: string; timeframe: string; basis: ProfileBasis; profile?: boolean; bars?: number }; result: IndicatorValues | null };
  /** The last `days` closed UTC days. */
  daily: { args: { market: string; days: number }; result: DailyBars };
  /** A picture of one chart as it stands, `wide` pixels across at most. */
  /** The chart drawn again `wide` across, and how many of its pixels make one of the page's. */
  capture: { args: { id: string; wide: number }; result: { picture: ImageBitmap; px: number } };
  /** Fetch a market's history before a chart asks for it. */
  prefetch: { args: { market: string; timeframe: string }; result: null };
  /** `?debug`: whether the worker hears the page — how many messages it has had, and the market a chart was last asked for. */
  ping: { args: { id: string }; result: { worker: string; up: number; got: number; slots: number; asked: string | null } };
}

export type CallName = keyof Calls;

/** Where drawing anchors are pulled: nowhere, a bar's price when near, or always. */
export type MagnetMode = 'off' | 'weak' | 'strong';

/** `hold`: a finger kept still where it went down, timed by the page. */
export type PointerKind = 'move' | 'down' | 'up' | 'leave' | 'dblclick' | 'hold';

export type ToWorker =
  /** `feed`: the liquidation feed's layer, over the other two. `flat`: shapes on the 2D canvas even where WebGL2 is available (development only). */
  | { t: 'create'; id: string; gl: OffscreenCanvas; text: OffscreenCanvas; feed: OffscreenCanvas; width: number; height: number; dpr: number; flat?: boolean }
  | { t: 'destroy'; id: string }
  /** Off screen (a tab not shown): not drawn, its bars and streams kept. */
  | { t: 'visible'; id: string; value: boolean }
  | { t: 'resize'; id: string; width: number; height: number; dpr: number }
  /** `bars`: how many bars of `timeframe` to open with, when not the usual screenful. */
  | { t: 'market'; id: string; market: string; timeframe: string; bars?: number }
  | { t: 'studies'; id: string; value: string }
  /** The reader's formulas on this chart, worked out on its closed bars. */
  | { t: 'formulas'; id: string; value: FormulaRun[] }
  | { t: 'hidden'; id: string; value: string }
  | { t: 'studyStyles'; id: string; value: string }
  | { t: 'log'; id: string; value: boolean }
  /** A TPO period — `1d`, `london`, `custom`… — drawn as profiles of the loaded bars. Empty: candles. */
  | { t: 'tpo'; id: string; value: string }
  /** TPO options, `key=value;key=value`. */
  | { t: 'tpoSettings'; id: string; value: string }
  /** Profiles expanded, merged and split by hand, `e=t,t;m=t;s=t`. */
  | { t: 'tpoEdits'; id: string; value: string }
  | { t: 'paneWeights'; id: string; value: string }
  /** `style`: what the new drawing will look like, so its draft matches. */
  | { t: 'tool'; id: string; value: string; style: Record<string, string | number | boolean> }
  | { t: 'magnet'; id: string; value: MagnetMode }
  | { t: 'drawings'; id: string; list: DrawingShape[] }
  /** The market's active price alerts. */
  | { t: 'alerts'; id: string; list: { id: number; price: number; style?: AlertStyle }[] }
  /** Minutes whose taker flow ran hard, marked on the candles. */
  | { t: 'flow'; id: string; marks: { at: number; price: number; usd: number }[] }
  /** Volume by price: per bar, its time, how many levels follow, then each
   *  level as price, coins bought, coins sold. */
  | { t: 'footprint'; id: string; flat: Float64Array; grid: number; fitted: boolean }
  | { t: 'footprintShown'; id: string; value: boolean }
  | { t: 'footprintOptions'; id: string; value: string }
  /** A bar replay from the bar at `x` on the chart: it is the last one shown. */
  | { t: 'replayPick'; id: string; x: number }
  /** A replay forward by `ms`, or with 0 to where the next bar opens. Always answered. */
  | { t: 'replayStep'; id: string; ms: number }
  /** A replay at a moment already known, or null for the present: at the newest bars, or left where it is scrolled. */
  | { t: 'replayAt'; id: string; at: number | null; keepView?: boolean }
  | { t: 'selected'; id: string; value: number | null }
  | { t: 'resetView'; id: string }
  | { t: 'autoScale'; id: string }
  | { t: 'range'; id: string; days: number | null }
  /** The bars from `from` to `to` (ms) fill the plot. */
  | { t: 'span'; id: string; from: number; to: number }
  /** `shift`: held, so lines being drawn stay level. `ctrl` (or Cmd): a weak magnet while held. */
  | { t: 'pointer'; id: string; kind: PointerKind; x: number; y: number; button: number; shift?: boolean; ctrl?: boolean; touch?: boolean }
  | { t: 'wheel'; id: string; x: number; y: number; dx: number; dy: number }
  | { t: 'theme'; key: string; value: string }
  /** What the layout keeps in step across its charts: the crosshair's time, and the time at the right edge. */
  | { t: 'sync'; crosshair: boolean; time: boolean }
  /** Every exchange's mark, as `markSpec` writes it, numbered by MARK_ORDER. */
  | { t: 'marks'; list: { index: number; box: number; spec: string }[] }
  /** The reader's time zone, minutes east of UTC, for every chart. */
  | { t: 'timeZone'; offsetMinutes: number }
  /** Development only: fake liquidations, so the ping design can be judged. */
  | { t: 'demoLiquidations'; id: string; perMinute: number }
  /** Development only: the chart's GPU context lost as iOS loses it, and back after `ms` (never when negative). */
  | { t: 'loseGpu'; id: string; ms: number }
  /** The ticker's perpetual on each exchange that lists it, for the liquidation feed. */
  | { t: 'liquidationMarkets'; id: string; markets: string[] }
  | { t: 'call'; req: number; name: CallName; args: unknown };

export type FromWorker =
  | { t: 'ready' }
  | { t: 'fatal'; message: string }
  | { t: 'cursor'; id: string; value: string }
  | { t: 'legend'; id: string; value: Legend }
  | { t: 'studies'; id: string; value: StudyMeta[] }
  | { t: 'event'; id: string; event: ChartEvent }
  | { t: 'status'; id: string; value: ChartStatus }
  | { t: 'price'; id: string; market: string; price: number; precision: number }
  | { t: 'replay'; id: string; value: ReplayState }
  | { t: 'reply'; req: number; value?: unknown; error?: string };
