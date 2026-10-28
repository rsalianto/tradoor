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
  /** The alert line under the pointer. */
  alert: { id: number; y: number } | null;
  /** The flow mark under the pointer: which one, and where it sits. */
  flow: { index: number; x: number; y: number } | null;
  /** The footprint cell under the pointer: which bar, which level, and where. */
  footprint: { bar: number; level: number; x: number; y: number } | null;
}

export interface StudyLine {
  id: string;
  label: string;
  color: string;
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
}

export interface StudyDefinition {
  kind: string;
  name: string;
  short: string;
  group: string;
  about: string;
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
  | { type: 'drawingCreated'; kind: string; points: DrawingPoint[] }
  | { type: 'selection'; id: number | null }
  | { type: 'delete'; id: number }
  | { type: 'drawingChanged'; id: number; points: DrawingPoint[] }
  | { type: 'editRequested'; id: number }
  | { type: 'toolFinished' }
  | { type: 'needHistory'; before: number }
  | { type: 'paneWeights'; value: string }
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

/** Request/response calls, answered by `reply`. */
export interface Calls {
  studyCatalog: { args: Record<string, never>; result: StudyDefinition[] };
  key: { args: { id: string; key: string }; result: boolean };
  /** `profile: false` skips the session profile; `bars` caps the history read (1,500 by default). */
  values: { args: { market: string; timeframe: string; basis: ProfileBasis; profile?: boolean; bars?: number }; result: IndicatorValues | null };
  /** The last `days` closed UTC days. */
  daily: { args: { market: string; days: number }; result: DailyBars };
  /** A picture of one chart as it stands, `wide` pixels across at most. */
  capture: { args: { id: string; wide: number }; result: Blob };
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
  | { t: 'hidden'; id: string; value: string }
  | { t: 'log'; id: string; value: boolean }
  /** A timeframe: each of its candles as a TPO profile of the loaded bars. Empty: candles. */
  | { t: 'tpo'; id: string; value: string }
  /** TPO options, `key=value;key=value`. */
  | { t: 'tpoSettings'; id: string; value: string }
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
  | { t: 'footprint'; id: string; flat: Float64Array; grid: number }
  | { t: 'footprintShown'; id: string; value: boolean }
  | { t: 'footprintOptions'; id: string; value: string }
  | { t: 'selected'; id: string; value: number | null }
  | { t: 'resetView'; id: string }
  | { t: 'autoScale'; id: string }
  | { t: 'range'; id: string; days: number | null }
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
  | { t: 'reply'; req: number; value?: unknown; error?: string };
