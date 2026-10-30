/**
 * One chart. The canvases belong to the engine worker; this component owns
 * the DOM around them — toolbar, drawing rail, legend, menus — and forwards
 * pointer input.
 */
import { batch, createEffect, createMemo, createSignal, For, type JSX, lazy, on, onCleanup, onMount, Show, untrack } from 'solid-js';
import { armedTool, chartControllers, controllersChanged, covered, drawingsChanged, magnet, mobile, setArmedTool, setControllersChanged, setMobileMenu, setOverlay, setShownPattern, sharedRail, shownPattern, type ShownPattern } from '../app/ui';
import { DrawingRail } from './DrawingRail';
import { call, debugClient, heardAt, holdChart, lastPrice, releaseChart, send } from '../engine/client';
import type { ChartEvent, ChartStatus, DrawingPoint, Legend, LiveStatus, PointerKind, StudyMeta, TpoHover } from '../engine/protocol';
import { api, catalog, errorText, type AlertStyle, type ChartKind, type Drawing, type Product } from '../lib/api';
import type { DrawingPatch } from '../components/DrawingSettings';
import { defaultStyle, visibleOn, type Style } from '../lib/drawing-style';
import { before, DrawingHistory, type DrawingChange, type DrawingRef, type Step } from '../lib/drawing-history';
import { captureChart } from '../lib/capture';
import { demoRate, forceFlatRenderer } from '../lib/demo';
import { flowMarks, watchFlowMarks } from '../lib/flow-marks';
import { fetchFootprint, flatten, foldMinutes, fromHours, recordedMarkets, type Levels, type Minute } from '../lib/footprint';
import { MinuteBook, toMinute } from '../lib/fp-minutes';
import { listenPrints } from '../lib/orderflow';
import { onRecorded, recordable, recordedFootprint, recorderSettings, recordStatus, want as wantRecorded } from '../lib/fp-recorder';
import { FOOTPRINT_EXTRAS, FOOTPRINT_MODES } from '../workspace/model';
import { useDismiss } from '../lib/dismiss';
import { decimalsOf, formatPrice, formatTurnover, MAX_PRICE_DECIMALS } from '../lib/format';
import type { LastPrice } from '../lib/last-price';
import { Icon } from '../lib/icons';
import { VenueTag } from '../lib/exchanges';
import { WatchlistToggle } from '../components/WatchlistToggle';
import { liquidationVenues } from '../lib/liquidation-venues';
import { formatClock, formatDateTime, formatFileTime, zoneLabel } from '../lib/timezone';
import { serializeTpo, tpoValue, withTpoValue } from '../lib/tpo-settings';
import { isRatio, symbolOf, venueLabel } from '../lib/markets';
import { alertState, createAlert, deleteAlert, digitsFor, editAlert, styleAlert, titleOf } from '../lib/alerts';
import { toast } from '../lib/toasts';
import { TIMEFRAMES, tfLabel, tfMs, tfName } from '../lib/timeframes';
import { TPO_SPLITS, tpoFits, type ChartPanelState } from '../workspace/model';
import { canSplit, mergeLeft, mergeRight, splitProfile, toggleExpanded } from '../lib/tpo-edits';
import { doc, marketOf, patchPanel, setPanelTimeframe, timeframeOf, toggleFavoriteTimeframe } from '../workspace/store';

// Drawing settings load when a drawing is first selected: nobody needs them to see a chart.
const DrawingSettings = lazy(() => import('../components/DrawingSettings').then((m) => ({ default: m.DrawingSettings })));
const OpenInterestSettings = lazy(() => import('../components/OpenInterestSettings').then((m) => ({ default: m.OpenInterestSettings })));
const AlertStyleSettings = lazy(() => import('../components/AlertStyleSettings').then((m) => ({ default: m.AlertStyleSettings })));
const DrawingToolbar = lazy(() => import('../components/DrawingToolbar').then((m) => ({ default: m.DrawingToolbar })));


/** What each click is for, in order — on a position tool the order is the meaning. */
/** `?debug`: a line of what the engine holds, for a phone with no console to read. */
const DEBUG = new URLSearchParams(location.search).has('debug');

const TOOL_STEPS: Record<string, string> = {
  path: 'Path: click each turn, then Enter or double-click to finish · right-click or esc cancels',
  brush: 'Brush: drag across the chart to draw',
  trendline: 'Trendline: click the first point, then the second · right-click or esc cancels',
  horizontal: 'Level: click the price',
  ray: 'Ray: click where the level starts',
  rect: 'Zone: click one corner, then the opposite corner · right-click or esc cancels',
  fib: 'Fib: click the 100% end — where the move started — then the 0% end · right-click or esc cancels',
  text: 'Note: click where the note goes',
  measure: 'Measure: click from, then to · right-click or esc cancels',
  channel: 'Channel: click both ends of the first line, then where the parallel line runs · shift keeps it level · right-click or esc cancels',
  long: 'Long: click where the entry goes',
  short: 'Short: click where the entry goes',
};

const LAYERS = [
  { id: 'ribbon', label: 'EMA 12/25 ribbon', candles: true },
  { id: 'ema200', label: 'EMA 200', candles: true },
  { id: 'volume', label: 'Volume', candles: true },
  { id: 'liquidations', label: 'Liquidations', candles: false },
];

const RANGES: { label: string; days: number | 'ytd' | null }[] = [
  { label: '1D', days: 1 },
  { label: '5D', days: 5 },
  { label: '1M', days: 30 },
  { label: '3M', days: 90 },
  { label: '6M', days: 180 },
  { label: 'YTD', days: 'ytd' },
  { label: '1Y', days: 365 },
  { label: 'All', days: null },
];

/** How long a finger stays put before it is a hold, and how far it may drift
 *  meanwhile — as far as the engine lets a press go before it pans. */
const HOLD_MS = 400;
const HOLD_SLOP = 5;

const LIVE: Record<LiveStatus, { label: string; tone: string; title: string }> = {
  off: { label: 'CLOSED BARS', tone: 'dim', title: 'The exchange streams no candle of this length; it is folded from shorter bars and refreshed every few minutes.' },
  connecting: { label: 'CONNECTING', tone: 'dim', title: 'Opening the exchange stream.' },
  live: { label: 'LIVE', tone: 'bull', title: 'Streaming from the exchange into the forming candle.' },
  stalled: { label: 'NO TICKS', tone: 'warn', title: 'Connected and subscribed, but the exchange has pushed nothing. Closed candles are unaffected.' },
  retrying: { label: 'RECONNECTING', tone: 'warn', title: 'The stream dropped; reconnecting. Bars that close meanwhile are fetched when it returns.' },
};

const splitList = (s: string): string[] =>
  s
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);

/** A pattern's side in the colours the chart gives a short and a long. */
const PATTERN_COLOR = { bear: '#f23645', bull: '#089981' } as const;
const FOUR_HOURS = 4 * 3_600_000;

/**
 * A pattern from the Signal panel as drawings: the line through its first two
 * drives carried a little past the third, and each drive ringed and numbered.
 */
function patternParts(p: ShownPattern): { kind: string; points: DrawingPoint[]; label: string | null; style: Style }[] {
  const [first, second, third] = p.drives;
  if (!first || !second || !third) return [];
  const color = PATTERN_COLOR[p.side];
  const slope = (second.price - first.price) / (second.at - first.at);
  const end = third.at + Math.max(2, Math.round((third.at - first.at) * 0.12 / FOUR_HOURS)) * FOUR_HOURS;
  return [
    { kind: 'trendline', points: [{ time: first.at, price: first.price }, { time: end, price: first.price + slope * (end - first.at) }], label: null, style: { color: '#d1d4dc', width: 1.5 } },
    ...p.drives.map((d, i) => ({ kind: 'circle', points: [{ time: d.at, price: d.price }], label: String(i + 1), style: { color, width: 2, textColor: color, fill: `${color}26` } })),
    // The trade, from where it was entered: take-profits green, the stop red.
    ...(p.levels && p.enteredAt
      ? [...p.levels.tp.map((price, i) => ({ price, label: `TP${i + 1}`, color: '#089981' })), { price: p.levels.sl, label: 'SL', color: '#f23645' }].map((l) => ({
          kind: 'ray',
          points: [{ time: p.enteredAt!, price: l.price }],
          label: l.label,
          style: { color: l.color, width: 1, dash: 'dashed', textColor: l.color, priceLabels: true },
        }))
      : []),
    // The limit entry, across the two days it waits.
    ...(p.limit
      ? [{ kind: 'trendline', points: [{ time: p.limit.from, price: p.limit.price }, { time: p.limit.until, price: p.limit.price }], label: 'Limit', style: { color: '#ffc247', width: 1.5, textColor: '#ffc247', priceLabels: true } }]
      : []),
  ];
}

/**
 * The training view's chart: its markets alone, on one timeframe, candles
 * only, its drawings its own — and the account's positions laid over them,
 * shown but not picked up.
 */
export interface TrainingChart {
  /** In place of the symbol and timeframes: its own coins, and its one timeframe. */
  toolbar: JSX.Element;
  overlay: () => { kind: string; points: DrawingPoint[]; label: string | null; style: Style }[];
  /** What to say over the candle under the pointer (or a finger), if anything. */
  hover: (time: number) => JSX.Element | null;
}

export function ChartPanel(props: { panel: ChartPanelState; training?: TrainingChart }) {
  const id = props.panel.id;
  const market = () => marketOf(props.panel);
  const timeframe = () => timeframeOf(props.panel);
  /** The TPO period drawn now: chosen, and longer than the candles it stacks. */
  const split = () => (!props.training && props.panel.tpo && tpoFits(props.panel.tpo, timeframe()) ? props.panel.tpo : null);
  const tpoShown = () => split() !== null;
  /** Candles and TPO profiles keep separate drawings, and the training chart its own. */
  const chartKind = (): ChartKind => (props.training ? 'training' : tpoShown() ? 'tpo' : 'candles');

  const [legend, setLegend] = createSignal<Legend | null>(null);
  const [studies, setStudies] = createSignal<StudyMeta[]>([]);
  const [status, setStatus] = createSignal<ChartStatus>({ loading: true, error: null, bars: 0, source: null, live: 'off', relayed: false, exhausted: false });
  const [cursor, setCursor] = createSignal('crosshair');
  // The tool armed is every chart's: picked on a rail, it draws on whichever chart is clicked next.
  const tool = armedTool;
  const setTool = setArmedTool;
  const [autoScale, setAutoScale] = createSignal(true);
  const [selected, setSelected] = createSignal<number | null>(null);
  const [drawings, setDrawings] = createSignal<Drawing[]>([]);
  const [offline, setOffline] = createSignal<string | null>(null);
  const [drawError, setDrawError] = createSignal<string | null>(null);
  /** The study whose settings dialog is open. */
  const [studyEditing, setStudyEditing] = createSignal<string | null>(null);
  /** The drawing whose settings dialog is open. */
  const [editing, setEditing] = createSignal<number | null>(null);
  /** The chart's menu, and the TPO profile it was opened on, if any. */
  const [menu, setMenu] = createSignal<{ x: number; y: number; tpo?: TpoHover } | null>(null);
  /** The profile menu's prices, folded out. */
  const [tpoPrices, setTpoPrices] = createSignal(false);
  /** The alert a right-click landed on, and where its menu opens. */
  const [alertMenu, setAlertMenu] = createSignal<{ id: number; x: number; y: number } | null>(null);
  /** The alert whose line is being restyled, and the style shown meanwhile. */
  const [alertStyling, setAlertStyling] = createSignal<number | null>(null);
  const [alertDraft, setAlertDraft] = createSignal<{ id: number; style: AlertStyle } | null>(null);
  const [tfMenu, setTfMenu] = createSignal<number | null>(null);
  /** Under its button, or moved back inside the panel where it would run past the right edge — a phone's bar scrolls. */
  const fitMenu = (el: HTMLElement) =>
    queueMicrotask(() => {
      const room = (el.offsetParent as HTMLElement | null)?.clientWidth ?? 0;
      if (el.offsetLeft + el.offsetWidth > room - 4) el.style.left = `${Math.max(4, room - el.offsetWidth - 4)}px`;
    });
  /** The TPO quick access menu, open at this x in the toolbar. */
  const [tpoMenu, setTpoMenu] = createSignal<number | null>(null);
  useDismiss(tpoMenu, () => setTpoMenu(null));
  useDismiss(menu, () => setMenu(null));
  useDismiss(alertMenu, () => setAlertMenu(null));
  useDismiss(tfMenu, () => setTfMenu(null));

  let box!: HTMLDivElement;
  let tpoButton!: HTMLButtonElement;

  // The panel's canvases and engine slot outlive this mount: see holdChart.
  const chart = holdChart(id, {
    cursor: setCursor,
    legend: setLegend,
    studies: setStudies,
    status: setStatus,
    event: (e) => onEvent(e),
  });
  const sid = chart.id;
  const price = () => {
    const p = lastPrice(id);
    return p && p.market === market() ? p : null;
  };

  // ---------------------------------------------------------------- flow
  // Minutes whose taker flow ran hard, marked on the candles. They are the Flow
  // study's: without it nothing is asked for and nothing is drawn. The server
  // keeps two days; which minutes count is the size set in the flow panel.
  const flowOn = () => !isRatio(market()) && splitList(props.panel.studies).some((s) => s.split(':')[0] === 'flow');
  createEffect(() => {
    if (flowOn()) onCleanup(watchFlowMarks(market()));
  });
  createEffect(() => {
    const list = flowOn() ? flowMarks(market()) : [];
    send({ t: 'flow', id: sid, marks: list.map((m) => ({ at: m.at, price: m.price, usd: m.deltaUsd })) });
  });
  /** The row under the pointer, with the numbers that made it. */
  const hoveredCell = () => {
    const c = legend()?.footprint;
    return c ? { at: c.time, price: c.price, buy: c.buy, sell: c.sell, x: c.x, y: c.y } : null;
  };

  const hoveredFlow = () => {
    const at = legend()?.flow;
    const mark = at && flowMarks(market())[at.index];
    return mark ? { mark, x: at.x, y: at.y } : null;
  };

  // ---------------------------------------------------------------- footprint
  // Volume by price, in place of the candles. The server records eight markets
  // at a minute apiece; a chart of longer bars adds those minutes together.
  const [recorded, setRecorded] = createSignal<Map<string, number>>(new Map());
  void recordedMarkets().then(setRecorded);
  /** The footprint comes from this browser's own recordings, not the server's. */
  const browserFootprint = () => !recorded().has(market()) && recorderSettings().on && recordable(market());
  /** Whether this market's footprint can be asked for at all. */
  const footprintable = () => recorded().has(market()) || browserFootprint();
  // Every chart's market recorded here while it is open, so its footprint is there when asked for.
  createEffect(() => {
    if (props.training) return;
    onCleanup(wantRecorded(market()));
  });
  const footprintOn = () => !props.training && props.panel.footprint === true && footprintable() && !tpoShown();
  createEffect(() => send({ t: 'footprintShown', id: sid, value: footprintOn() }));
  // The mode and what else a column carries, as the engine reads them.
  createEffect(() =>
    send({
      t: 'footprintOptions',
      id: sid,
      value: [props.panel.footprintMode, props.panel.footprintValueArea, props.panel.footprintStats, props.panel.footprintProfile]
        .map((v) => (v === true ? '1' : v === false ? '0' : v))
        .join(';'),
    }),
  );
  /** The footprint is being fetched or read, and nothing of it is on the chart yet. */
  const [fpLoading, setFpLoading] = createSignal(false);
  /** The oldest bar this browser's recordings reach, once read. */
  const [fpSince, setFpSince] = createSignal<number | null>(null);
  /**
   * What the footprint is doing, said in the legend, and how far along a thin
   * line over the chart shows: a share done, `busy` while there is no telling, or none.
   */
  const fpNote = (): { text: string; tip: string; progress: number | 'busy' | null } | null => {
    if (!footprintOn() && !tpoFlow()) return null;
    const venue = venueLabel(market());
    if (!browserFootprint()) return fpLoading() ? { text: 'loading footprint…', tip: "The server's recorded footprint for these bars", progress: 'busy' } : null;
    const status = recordStatus(market());
    const filling = status?.filling;
    if (filling) {
      return {
        text: filling.done > 0 ? `filling from Binance · back to ${formatClock(filling.back).slice(0, 5)}` : 'filling from Binance…',
        tip: "The last day's hours no tab had this market open, from Binance's trade history: the newest first, paced to keep within its limits, as far as its calls reach",
        // Done when the day is in, or the calls are spent: whichever comes first.
        progress: Math.max(filling.done / filling.of, filling.calls),
      };
    }
    if (fpLoading()) return { text: 'reading footprint…', tip: 'What this browser recorded, from its own storage', progress: 'busy' };
    if (!status?.live) return { text: `footprint: connecting to ${venue}…`, tip: 'Trades come from the exchange itself, the same socket as the DOM and tape', progress: 'busy' };
    const since = fpSince();
    if (since === null) return { text: 'footprint: recording, the first cells come with the next trades', tip: 'Recorded in this browser from now on', progress: null };
    const at = Date.now() - since < 86_400_000 ? formatClock(since).slice(0, 5) : formatDateTime(since).slice(0, 16);
    const fills = /^Binance/.test(venue);
    return {
      text: fills ? `footprint from this browser, since ${at}` : `footprint since ${at} · ${venue} keeps no trade history for earlier bars`,
      tip: `Recorded in this browser while tradoor is open, kept ${recorderSettings().days} days: a bar with no cells is from before, or when it was closed.${fills ? " Binance's history fills the last day's gaps." : ''} Settings, General.`,
      progress: null,
    };
  };
  /** The line's share, held in an object: none done yet is still a line to show. */
  const fpProgress = () => {
    const share = fpNote()?.progress;
    return share === null || share === undefined ? null : { share };
  };
  /** A TPO chart's volume profiles split by who took the volume: the footprint's trades, where they are recorded. */
  const tpoFlow = () => tpoShown() && footprintable() && tpoValue(props.panel.tpoSettings, 'vp') === '1' && tpoValue(props.panel.tpoSettings, 'vpMode') !== 'total';
  createEffect(() => {
    const flow = tpoFlow();
    if (!footprintOn() && !flow) {
      send({ t: 'footprint', id: sid, flat: new Float64Array(), grid: 0, fitted: true });
      setFpLoading(false);
      return;
    }
    setFpLoading(true);
    setFpSince(null);
    // What was drawn is another market's or timeframe's: gone until this one's lands.
    send({ t: 'footprint', id: sid, flat: new Float64Array(), grid: 0, fitted: true });
    const market_ = market();
    const barMs = tfMs(timeframe());
    const settings = flow ? { ticks: 0, unit: 'coins' as const } : { ticks: props.panel.footprintTicks, unit: props.panel.footprintUnit === 'usd' ? ('usd' as const) : ('coins' as const) };
    if (browserFootprint()) {
      // This browser's recordings, every day kept, folded again as minutes are
      // written: the last bar alone, or all of it when a gap was filled.
      const from = Date.now() - recorderSettings().days * 86_400_000;
      let dropped = false;
      let by: number | undefined;
      let held = new Map<number, Levels>();
      let busy = false;
      let again: boolean | null = null;
      const load = async (full: boolean) => {
        if (busy) return void (again = (again ?? false) || full);
        busy = true;
        const last = Math.max(from, ...held.keys());
        const fp = await recordedFootprint(market_, barMs, full ? from : last, settings, full ? undefined : by);
        busy = false;
        if (dropped) return;
        if (fp) {
          if (full) {
            by = fp.by;
            held = new Map();
          }
          for (const bar of fp.bars) held.set(bar.time, bar);
          const bars = [...held.values()].sort((a, b) => a.time - b.time);
          send({ t: 'footprint', id: sid, flat: flatten(bars), grid: fp.grid, fitted: fp.fitted });
          setFpSince(bars.find((b) => b.levels.length > 0)?.time ?? null);
        } else send({ t: 'footprint', id: sid, flat: new Float64Array(), grid: 0, fitted: true });
        setFpLoading(false);
        if (again !== null) {
          const next = again;
          again = null;
          void load(next);
        }
      };
      void load(true);
      const off = onRecorded(market_, (full) => void load(full));
      onCleanup(() => {
        dropped = true;
        off();
      });
      return;
    }
    // What the chart can show of it: a screenful of bars back from now, since
    // a chart of fifteen-minute bars covers a day in one screen and a chart of
    // minutes an afternoon.
    const to = Date.now();
    // Hours and days come from the history the server keeps: four hundred bars of
    // it. Shorter bars are minutes, which it keeps a month, and a week is plenty —
    // for a footprint; TPO profiles reach back further, as far as the month goes.
    const hours = fromHours(barMs);
    const days = flow ? 30 : Math.min(7, Math.max(1, Math.round((barMs * 400) / 86_400_000)));
    const from = hours ? to - (flow ? 1_000 : 400) * barMs : to - days * 86_400_000;
    let dropped = false;
    // The month a TPO chart reads is asked for once; after that only its last
    // day, folded into what is held.
    let held = new Map<number, Levels>();
    let shown: Levels[] = [];
    let grid = 0;
    let fitted = true;
    // The bar under way, trade by trade: the server's finished minutes of it,
    // and what traded since off the exchange's own socket — the server itself
    // is asked only now and then, and hands out finished minutes alone.
    const tick = recorded().get(market_) ?? 0;
    const book = !flow && tick > 0 ? new MinuteBook(tick, Date.now()) : null;
    let barMinutes: Minute[] = [];
    let lastServer = 0;
    const draw = () => {
      if (dropped) return;
      let bars = shown;
      if (book && grid > 0) {
        const barStart = Math.floor(Date.now() / barMs) * barMs;
        const since = [...book.minutes].filter(([at]) => at > lastServer && at >= barStart).map(([at, levels]) => toMinute(at, levels, tick));
        const minutes = [...barMinutes.filter((m) => m.at >= barStart), ...since];
        const bar = minutes.length > 0 ? foldMinutes(minutes, tick, barMs, settings, Math.max(1, Math.round(grid / tick))).bars.find((b) => b.time === barStart) : undefined;
        if (bar) bars = [...shown.filter((b) => b.time !== barStart), bar].sort((a, b) => a.time - b.time);
      }
      send({ t: 'footprint', id: sid, flat: flatten(bars), grid, fitted });
    };
    const load = (span: number) =>
      fetchFootprint(market_, barMs, span === days ? from : Date.now() - span * 86_400_000, Date.now(), settings, span).then(
        (fp) => {
          if (dropped) return;
          let bars = fp.bars;
          if (flow) {
            held = new Map([...held, ...bars.map((b) => [b.time, b] as const)]);
            bars = [...held.values()].sort((a, b) => a.time - b.time);
          }
          shown = bars;
          grid = fp.grid;
          fitted = fp.fitted;
          draw();
          setFpLoading(false);
        },
        () => setFpLoading(false),
      );
    void load(days);
    const timer = setInterval(() => void load(flow && !hours ? 1 : days), flow || hours ? 60_000 : 20_000);
    let offPrints = () => {};
    let tailTimer: ReturnType<typeof setInterval> | undefined;
    if (book) {
      const tail = async () => {
        const barStart = Math.floor(Date.now() / barMs) * barMs;
        const answer = await api.footprint(market_, barStart, Date.now()).catch(() => null);
        if (!answer || dropped) return;
        barMinutes = answer.bars.map((b) => ({ at: b.at, levels: b.levels }));
        lastServer = Math.max(barStart - 1, ...barMinutes.map((m) => m.at));
        // What the server has now is the server's: the browser's own count of those minutes goes.
        book.take(lastServer + 1);
        draw();
      };
      let pending: ReturnType<typeof setTimeout> | undefined;
      offPrints = listenPrints(market_, {
        print(time, price, qty, buy) {
          if (book.add(time, price, qty, buy)) pending ??= setTimeout(() => ((pending = undefined), draw()), 500);
        },
        state() {},
      });
      void tail();
      tailTimer = setInterval(() => void tail(), 15_000);
      onCleanup(() => clearTimeout(pending));
    }
    onCleanup(() => {
      dropped = true;
      clearInterval(timer);
      clearInterval(tailTimer);
      offPrints();
    });
  });

  // ---------------------------------------------------------------- alerts
  // The market's active alerts, drawn by the engine as levels. The style is
  // copied out: an alert's own is a store proxy, which a worker cannot be sent.
  createEffect(() => {
    const draft = alertDraft();
    const list = alertState.alerts
      .filter((a) => a.active && a.market === market())
      .map((a) => ({ id: a.id, price: a.price, style: { ...(draft?.id === a.id ? draft.style : a.style) } }));
    send({ t: 'alerts', id: sid, list });
  });
  // The add-alert button and an alert's label stay while the pointer is on
  // them, though the chart below has lost it by then.
  const [overClock, setOverClock] = createSignal<{ y: number; price: number } | null>(null);
  const [overAlert, setOverAlert] = createSignal<{ id: number; y: number } | null>(null);
  const axisX = () => legend()?.axisX ?? 0;
  // Not over an alert's line: its own label is there, with its own delete.
  // A ratio is no market an alert can watch: no clock on its axis.
  const clock = () => (isRatio(market()) ? null : (overClock() ?? (tool() === '' && !legend()?.alert && !overAlert() ? (legend()?.cursor ?? null) : null)));
  const hoveredAlert = () => {
    const h = overAlert() ?? legend()?.alert ?? null;
    const a = h && alertState.alerts.find((x) => x.id === h.id && x.active);
    return a && h ? { alert: a, y: h.y } : null;
  };
  const addAlertAt = (level: number) => {
    const digits = digitsFor(market(), level);
    const rounded = Number(level.toFixed(digits));
    // The price it was set from says which way it is reached.
    createAlert(market(), 'crossing', rounded, null, price()?.price).then(
      (a) => toast({ tone: 'info', title: 'Alert created', body: titleOf(a) }, 4_000),
      (err) => toast({ tone: 'warn', title: 'Alert not created', body: errorText(err) }),
    );
  };

  const local = (e: MouseEvent) => {
    const r = box.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const [pointerX, setPointerX] = createSignal(0);
  const pointer = (kind: PointerKind, e: MouseEvent) => {
    const p = local(e);
    const touch = 'pointerType' in e && (e as PointerEvent).pointerType === 'touch';
    send({ t: 'pointer', id: sid, kind, x: p.x, y: p.y, button: e.button, shift: e.shiftKey, ctrl: e.ctrlKey || e.metaKey, touch });
  };

  // ---- hold: one finger kept still is the crosshair, as in TradingView's app.
  // A press pans a phone's chart, so the crosshair cannot be what a press
  // gives; the engine is told when a finger has stayed put, and decides what
  // a hold means where it is (the crosshair, an alert taken, a tool aiming).
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  let holdFrom: { x: number; y: number } | null = null;
  /** The finger lifted was a hold: its crosshair stays to be read, so the
   *  pointerleave that follows a lifted finger is not passed on. */
  let keepCrosshair = false;
  /** The last press was a finger's: the long press that would open the
   *  browser's menu is a hold here instead. */
  let fingerLast = false;
  const dropHold = () => {
    clearTimeout(holdTimer);
    holdTimer = undefined;
    holdFrom = null;
  };

  // ---- pinch: two fingers zoom time about the point between them, and carry
  // the chart along with that point. The engine hears it in the wheel's own
  // language — it knows nothing of fingers, and a pinch is a zoom with a
  // middle. `exp(-dy * 0.002)` is the factor a wheel zooms by, so a spread
  // that grew by `scale` is the `dy` that would have done the same.
  const fingers = new Map<number, { x: number; y: number }>();
  /** The spread and middle as last seen, while two fingers are down. */
  let pinch: { spread: number; x: number } | null = null;
  /** Fingers left over from a pinch: they lift before one of them pans again. */
  let spent = false;
  const between = () => {
    const [a, b] = [...fingers.values()];
    return a && b ? { spread: Math.hypot(a.x - b.x, a.y - b.y), x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } : null;
  };

  // The training chart's card over a marked candle: made again when the pointer
  // reaches another candle, not with each frame's legend.
  const hoverTime = createMemo(() => (props.training && legend()?.cursor ? legend()!.time : null));
  const trainingCard = createMemo(() => {
    const t = hoverTime();
    return t === null ? null : props.training!.hover(t);
  });
  // Under the journal or training a workspace chart keeps its market live and draws nothing until it is seen again.
  const paused = () => !props.training && covered();
  createEffect(on(paused, (p) => chart.created && send({ t: 'visible', id: sid, value: !p }), { defer: true }));
  onMount(() => {
    box.prepend(chart.gl, chart.text, chart.feed);
    const rect = box.getBoundingClientRect();
    if (chart.created) {
      send({ t: 'visible', id: sid, value: !paused() });
      send({ t: 'resize', id: sid, width: rect.width, height: rect.height, dpr: devicePixelRatio });
    } else {
      try {
        const gl = chart.gl.transferControlToOffscreen();
        const text = chart.text.transferControlToOffscreen();
        const feed = chart.feed.transferControlToOffscreen();
        send({ t: 'create', id: sid, gl, text, feed, width: rect.width, height: rect.height, dpr: devicePixelRatio, flat: forceFlatRenderer() }, [gl, text, feed]);
        chart.created = true;
        if (paused()) send({ t: 'visible', id: sid, value: false });
      } catch (e) {
        setStatus((s) => ({ ...s, loading: false, error: `this browser cannot draw off the main thread (${errorText(e)})` }));
      }
    }
    const observer = new ResizeObserver(() => {
      const r = box.getBoundingClientRect();
      send({ t: 'resize', id: sid, width: r.width, height: r.height, dpr: devicePixelRatio });
    });
    observer.observe(box);
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const p = local(e);
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? box.clientHeight : 1;
      send({ t: 'wheel', id: sid, x: p.x, y: p.y, dx: e.deltaX * unit, dy: e.deltaY * unit });
    };
    box.addEventListener('wheel', onWheel, { passive: false });
    // `?liqdemo=30` invents thirty liquidations a minute, for judging the feed.
    if (demoRate() > 0) send({ t: 'demoLiquidations', id: sid, perMinute: demoRate() });
    chartControllers.set(id, {
      key: (key) => call('key', { id: sid, key }),
      tool: (kind) => {
        if (offline() === null) setTool(tool() === kind ? '' : kind);
      },
      resetView: () => send({ t: 'resetView', id: sid }),
      showRange: (days) => send({ t: 'range', id: sid, days }),
      undo,
      redo,
      capture,
      offline,
      canUndo: () => (moved(), history.canUndo),
      canRedo: () => (moved(), history.canRedo),
      selected,
      deleteSelected: () => {
        const s = selected();
        if (s !== null) void deleteDrawing(s);
      },
      drawingCount: () => drawings().length,
      clearDrawings: () => void clearDrawings(),
    });
    setControllersChanged((n) => n + 1);
    onCleanup(() => {
      observer.disconnect();
      box.removeEventListener('wheel', onWheel);
      chartControllers.delete(id);
      setControllersChanged((n) => n + 1);
      // A tab switched away keeps its chart; a closed panel frees it.
      releaseChart(id, id in doc.panels);
    });
  });

  // The candles are the TPO brackets. A weekly split of 15-minute brackets is
  // 672 a profile, so a TPO chart opens with enough for a run of profiles.
  let marketSends = 0;
  const sendMarket = () => {
    marketSends += 1;
    const s = split();
    const perProfile = s ? Math.round(tfMs(s) / tfMs(timeframe())) : 0;
    send({ t: 'market', id: sid, market: market(), timeframe: timeframe(), bars: s ? Math.min(Math.max(perProfile * 8, 1_500), 6_000) : undefined });
  };
  createEffect(sendMarket);
  // Should that run be lost — an error anywhere in the same update ends the
  // rest of it — the chart would keep the last market's bars under this one's
  // name, as an iPhone showed. The engine says which market it holds: a chart
  // that does not follow is told again.
  const follow = setInterval(() => {
    const s = status();
    if (s.market && !s.loading && (s.market !== market() || s.timeframe !== timeframe())) untrack(sendMarket);
  }, 2_000);
  onCleanup(() => clearInterval(follow));
  const [tick, setTick] = createSignal(0);
  const [ping, setPing] = createSignal('ping…');
  if (DEBUG) {
    const timer = setInterval(() => setTick((n) => n + 1), 1_000);
    // Whether the worker hears this page at all: an answer, or none within a second and a half.
    const pinger = setInterval(() => {
      let answered = false;
      const sentAt = Date.now();
      call('ping', { id: sid }).then(
        (r) => {
          answered = true;
          setPing(`ping ${Date.now() - sentAt}ms · worker ${r.worker} up ${r.up}s · got ${r.got} · asked ${r.asked ?? '–'} · slots ${r.slots} · ${debugClient()} · canvases ${document.querySelectorAll('canvas').length}`);
        },
        (e) => setPing(`ping failed: ${errorText(e)}`),
      );
      setTimeout(() => !answered && setPing(`ping LOST (${new Date().toLocaleTimeString()})`), 1_500);
    }, 2_000);
    onCleanup(() => {
      clearInterval(timer);
      clearInterval(pinger);
    });
  }
  // The liquidation feed follows this ticker on every exchange that lists it.
  createEffect(() => {
    const m = market();
    let current = true;
    onCleanup(() => {
      current = false;
    });
    const post = (products: readonly Product[] | null) => {
      // A ratio has no ticker of its own whose liquidations could be followed.
      if (current) send({ t: 'liquidationMarkets', id: sid, markets: isRatio(m) ? [] : liquidationVenues(m, products).flatMap((v) => (v.market ? [v.market] : [])) });
    };
    catalog().then(
      (c) => post(c.products),
      () => post(null),
    );
  });
  createEffect(() => send({ t: 'studies', id: sid, value: props.panel.studies }));
  createEffect(() => send({ t: 'hidden', id: sid, value: props.panel.hidden }));
  createEffect(() => send({ t: 'log', id: sid, value: props.panel.log }));
  createEffect(() => send({ t: 'tpo', id: sid, value: split() ?? '' }));
  createEffect(() => send({ t: 'tpoSettings', id: sid, value: serializeTpo(props.panel.tpoSettings) }));
  createEffect(() => send({ t: 'tpoEdits', id: sid, value: props.panel.tpoEdits ?? '' }));
  const tpoEdit = (change: (raw: string, h: TpoHover) => string, h: TpoHover) => {
    patchPanel(id, { tpoEdits: change(props.panel.tpoEdits ?? '', h) });
    setMenu(null);
  };
  const tpoSet = (key: string, value: string) => patchPanel(id, { tpoSettings: withTpoValue(props.panel.tpoSettings, key, value) });
  const tpoOn = (key: string) => tpoValue(props.panel.tpoSettings, key) === '1';
  createEffect(() => send({ t: 'paneWeights', id: sid, value: props.panel.paneWeights }));
  createEffect(() => send({ t: 'tool', id: sid, value: offline() === null ? tool() : '', style: tool() ? defaultStyle(tool()) : {} }));
  createEffect(() => send({ t: 'magnet', id: sid, value: magnet() }));
  createEffect(() => send({ t: 'selected', id: sid, value: selected() }));
  // Every timeframe shows the market's drawings, bar those hidden on this one,
  // and a pattern the Signal panel opened here, locked so the pointer passes through.
  createEffect(() => {
    const tf = timeframe();
    const p = pattern();
    send({
      t: 'drawings',
      id: sid,
      list: [
        ...drawings()
          .filter((d) => visibleOn(d.style, tf))
          .map((d) => ({ id: d.id, kind: d.kind, points: d.points, label: d.label, style: d.style, timeframe: d.timeframe })),
        ...(p ? patternParts(p).map((d, i) => ({ id: -1_000_000 - i, ...d, style: { ...d.style, locked: true } })) : []),
        ...(props.training?.overlay() ?? []).map((d, i) => ({ id: -2_000_000 - i, ...d, style: { ...d.style, locked: true } })),
      ],
    });
  });

  // ---------------------------------------------------------------- a pattern shown

  /** The Signal panel's pattern, on this chart when it is on that market's candles. */
  const pattern = () => {
    const p = shownPattern();
    return p && p.market === market() && chartKind() === 'candles' ? p : null;
  };
  // Brought into view once its market's bars of its length are in: from a
  // little before the first drive to now.
  let framed: ShownPattern | null = null;
  createEffect(() => {
    const p = pattern();
    const s = status();
    if (!p || p === framed || s.loading || s.bars === 0 || s.market !== p.market || s.timeframe !== (p.timeframe ?? '4h')) return;
    framed = p;
    showRange((Date.now() - p.drives[0]!.at) / 86_400_000 + 2);
  });
  // Leaving its market closes it: coming back later is not asking for it again.
  createEffect(
    on(market, (m, prev) => {
      const p = untrack(shownPattern);
      if (p && prev === p.market && m !== p.market) setShownPattern(null);
    }, { defer: true }),
  );
  /** The pattern's line and circles saved as this chart's own drawings. */
  const keepPattern = (p: ShownPattern) => {
    setShownPattern(null);
    void (async () => {
      for (const part of patternParts(p)) await saveDrawing(part);
    })();
  };

  // ---------------------------------------------------------------- drawings

  let drawingsRequest = 0;
  async function loadDrawings(): Promise<void> {
    const req = ++drawingsRequest;
    try {
      const res = await api.drawings(market(), timeframe(), chartKind());
      if (req !== drawingsRequest) return;
      batch(() => {
        setDrawings(res.drawings);
        setOffline(null);
      });
    } catch (e) {
      if (req !== drawingsRequest) return;
      batch(() => {
        setDrawings([]);
        setOffline(errorText(e));
      });
    }
  }

  // A tool armed on one chart must not start drawing on the next. Drawings
  // belong to the market and the chart — candles or TPO — so a new timeframe
  // keeps them without a refetch, and switching to TPO reads its own.
  createEffect(
    on([market, timeframe, chartKind], ([m, , kind], prev) => {
      batch(() => {
        setTool('');
        setSelected(null);
        setDrawError(null);
        setEditing(null);
      });
      if (!prev || prev[0] !== m || prev[2] !== kind) {
        // Steps are about this chart's drawings.
        setDrawings([]);
        history.reset();
        void loadDrawings();
      }
    }),
  );
  // A position moved from the journal: read this market's drawings again.
  createEffect(
    on(
      drawingsChanged,
      (c) => {
        if (c && c.market === market() && chartKind() !== 'training') void loadDrawings();
      },
      { defer: true },
    ),
  );

  /* Every change is applied here first and sent afterwards, so a drawing appears
     under the pointer the moment it is placed rather than a round trip later. A
     rejected change is rolled back and the server's list is re-read. */
  let pending = 0;
  const onSameChart = (m: string, kind: ChartKind = chartKind()) => m === market() && kind === chartKind();
  // Read again by the undo and redo buttons whenever the history moves.
  const [moved, setMoved] = createSignal(0);
  const history = new DrawingHistory(() => setMoved((n) => n + 1));

  /** Save a drawing. `ref`: one undo is bringing back, which keeps its steps. */
  async function saveDrawing(input: { kind: string; points: DrawingPoint[]; label?: string | null; style: Style; timeframe?: string }, ref?: DrawingRef): Promise<void> {
    const m = market();
    const kind = chartKind();
    const tf = input.timeframe ?? timeframe();
    const temporary: Drawing = { id: --pending, market: m, timeframe: tf, kind: input.kind, points: input.points, style: input.style, label: input.label ?? null, createdAt: Date.now() };
    const r = ref ?? history.ref(temporary.id);
    history.rekey(r, temporary.id);
    if (!ref) history.record({ kind: 'create', ref: r, drawing: temporary });
    setDrawings((list) => [...list, temporary]);
    try {
      const { drawing } = await api.createDrawing(m, tf, kind, { kind: input.kind, points: input.points, label: input.label ?? undefined, style: input.style });
      setDrawError(null);
      const kept = onSameChart(m, kind) && drawings().some((d) => d.id === temporary.id);
      if (!kept) {
        // Undone, or the chart moved on, before the server answered.
        void api.deleteDrawing(drawing.id).catch(() => {});
        return;
      }
      history.rekey(r, drawing.id);
      setDrawings((list) => list.map((d) => (d.id === temporary.id ? drawing : d)));
    } catch (e) {
      setDrawError(errorText(e));
      history.forget(r);
      if (onSameChart(m, kind)) setDrawings((list) => list.filter((d) => d.id !== temporary.id));
    }
  }

  const createDrawing = (kind: string, points: DrawingPoint[], label?: string) => saveDrawing({ kind, points, label, style: defaultStyle(kind) });

  /** `previous`: what the change replaces, when the list already shows it (a dialog's preview). */
  async function changeDrawing(drawingId: number, patch: DrawingChange, record = true, previous?: DrawingChange): Promise<void> {
    const m = market();
    const kind = chartKind();
    const current = drawings().find((d) => d.id === drawingId);
    if (record && current) history.record({ kind: 'change', ref: history.ref(drawingId), before: previous ?? before(current, patch), after: patch });
    setDrawings((list) => list.map((d) => (d.id === drawingId ? { ...d, ...patch } : d)));
    if (drawingId < 0) return; // still being created; its own request carries the points
    try {
      const { drawing } = await api.updateDrawing(drawingId, patch);
      setDrawError(null);
      if (onSameChart(m, kind)) setDrawings((list) => list.map((d) => (d.id === drawingId ? drawing : d)));
    } catch (e) {
      setDrawError(errorText(e));
      if (onSameChart(m, kind)) void loadDrawings();
    }
  }

  async function deleteDrawing(drawingId: number, record = true): Promise<void> {
    const current = drawings().find((d) => d.id === drawingId);
    if (record && current) history.record({ kind: 'delete', ref: history.ref(drawingId), drawing: current });
    setDrawings((list) => list.filter((d) => d.id !== drawingId));
    if (selected() === drawingId) setSelected(null);
    if (editing() === drawingId) setEditing(null);
    if (drawingId < 0) return;
    try {
      await api.deleteDrawing(drawingId);
      setDrawError(null);
    } catch (e) {
      setDrawError(errorText(e));
      void loadDrawings();
    }
  }

  /** Put a removed drawing back as it was, under the ref its steps know it by. */
  const restore = (ref: DrawingRef, d: Drawing) => saveDrawing({ kind: d.kind, points: d.points, label: d.label, style: d.style, timeframe: d.timeframe }, ref);

  function replay(step: Step, back: boolean): void {
    switch (step.kind) {
      case 'create':
        if (back) void deleteDrawing(step.ref.id, false);
        else void restore(step.ref, step.drawing);
        break;
      case 'delete':
        if (back) void restore(step.ref, step.drawing);
        else void deleteDrawing(step.ref.id, false);
        break;
      case 'change':
        void changeDrawing(step.ref.id, back ? step.before : step.after, false);
        break;
      case 'clear':
        if (back) for (const item of step.items) void restore(item.ref, item.drawing);
        else void clearDrawings(false);
        break;
    }
  }

  /** A picture of this chart, to the clipboard — or to a file when `save`. */
  const capture = (save: boolean) => captureChart(sid, `tradoor ${symbolOf(market())} ${tfLabel(timeframe())} ${formatFileTime(Date.now())}.png`, save);

  const undo = () => {
    const step = history.takeUndo();
    if (step) replay(step, true);
    return step !== undefined;
  };
  const redo = () => {
    const step = history.takeRedo();
    if (step) replay(step, false);
    return step !== undefined;
  };

  const selectedDrawing = () => {
    const s = selected();
    const d = s === null ? undefined : drawings().find((x) => x.id === s);
    return d ?? null;
  };
  const restyle = (d: Drawing, patch: Style) => void changeDrawing(d.id, { style: { ...d.style, ...patch } });

  /** The settings dialog shows each change on the chart before it is saved. */
  const previewDrawing = (drawingId: number, patch: DrawingPatch) =>
    setDrawings((list) => list.map((d) => (d.id === drawingId ? { ...d, style: patch.style, label: patch.label, points: patch.points } : d)));

  function commitDrawing(d: Drawing, patch: DrawingPatch): Promise<void> {
    setEditing(null);
    // A note cannot lose its text: an emptied one keeps what it had.
    const label = d.kind === 'text' && patch.label === null ? d.label : patch.label;
    // `d` is the drawing as the dialog opened on it: what undo goes back to.
    return changeDrawing(d.id, { style: patch.style, points: patch.points, label }, true, { style: d.style, points: d.points, label: d.label });
  }

  async function clearDrawings(record = true): Promise<void> {
    const list = drawings();
    const n = list.length;
    if (n === 0 || (record && !confirm(`Remove all ${n} drawings on ${symbolOf(market())}, on every timeframe? Ctrl+Z brings them back.`))) return;
    if (record) history.record({ kind: 'clear', items: list.map((d) => ({ ref: history.ref(d.id), drawing: d })) });
    setDrawings([]);
    setSelected(null);
    try {
      await api.clearDrawings(market(), timeframe(), chartKind());
      setDrawError(null);
    } catch (e) {
      setDrawError(errorText(e));
      void loadDrawings();
    }
  }

  function onEvent(e: ChartEvent): void {
    switch (e.type) {
      case 'drawingCreated': {
        const points = e.points;
        if (e.kind === 'text') setOverlay({ kind: 'note', save: (label) => void createDrawing('text', points, label) });
        else void createDrawing(e.kind, points);
        break;
      }
      case 'selection':
        setSelected(e.id);
        break;
      case 'drawingChanged':
        void changeDrawing(e.id, { points: e.points });
        break;
      case 'editRequested':
        if (e.id >= 0) setEditing(e.id);
        break;
      case 'delete':
        if (editing() === e.id) setEditing(null);
        void deleteDrawing(e.id);
        break;
      case 'toolFinished':
        setTool('');
        break;
      case 'paneWeights':
        patchPanel(id, { paneWeights: e.value });
        break;
      case 'autoScale':
        setAutoScale(e.value);
        break;
      case 'alertMoved':
        editAlert(e.id, { price: e.price, atPrice: price()?.price }).catch((err) => toast({ tone: 'warn', title: 'Alert not moved', body: errorText(err) }));
        break;
      case 'needHistory':
        break;
    }
  }

  // ---------------------------------------------------------------- layers

  const hidden = () => new Set(splitList(props.panel.hidden));
  const studyCount = () => splitList(props.panel.studies).length;

  const toggleLayer = (layer: string) => {
    const next = hidden();
    if (next.has(layer)) next.delete(layer);
    else next.add(layer);
    patchPanel(id, { hidden: [...next].join(',') });
  };

  /** Swap a study for the same one with other parameters, keeping it shown or hidden. */
  const replaceStudy = (from: string, to: string) => {
    patchPanel(id, {
      studies: splitList(props.panel.studies)
        .map((s) => (s === from ? to : s))
        .filter((s, i, all) => all.indexOf(s) === i)
        .join(','),
      hidden: splitList(props.panel.hidden)
        .map((h) => (h === from ? to : h))
        .join(','),
    });
  };

  const removeStudy = (study: string) => {
    patchPanel(id, {
      studies: splitList(props.panel.studies)
        .filter((s) => s !== study)
        .join(','),
      // A removed study must not stay hidden, or adding it back would draw nothing.
      hidden: splitList(props.panel.hidden)
        .filter((h) => h !== study)
        .join(','),
    });
  };

  const showRange = (days: number | 'ytd' | null) => {
    const year = new Date().getUTCFullYear();
    send({ t: 'range', id: sid, days: days === 'ytd' ? (Date.now() - Date.UTC(year, 0, 1)) / 86_400_000 : days });
  };

  /** What the legend holds under its arrow: the layers drawn and the studies. */
  const folded = () => LAYERS.filter((layer) => !(tpoShown() && layer.candles)).length + studies().length;

  /** A finger off the glass; true when the chart should hear nothing of it. */
  const lift = (e: PointerEvent) => {
    fingers.delete(e.pointerId);
    if (fingers.size < 2) pinch = null;
    if (!spent) return false;
    if (fingers.size === 0) spent = false;
    return true;
  };

  /** The timeframe menu, under the button that asked for it. */
  const openTfMenu = (b: HTMLElement) => setTfMenu((v) => (v === null ? b.offsetLeft - (b.parentElement?.scrollLeft ?? 0) : null));

  return (
    <div class="panelcol chartcol" style={{ position: 'relative' }}>
      <div class="toolbar">
        {/* On a phone this row is the whole top of the app, so it carries the
            menu the rest of the workspace waits behind. */}
        <Show when={mobile()}>
          <button class="iconbtn burger" aria-label="Menu" onClick={() => setMobileMenu(true)}>
            <Icon name="menu" size={17} />
          </button>
        </Show>
        <Show when={!props.training} fallback={props.training!.toolbar}>
        <button class="symbol" data-tip="Change symbol — or just start typing" onClick={() => setOverlay({ kind: 'symbol', panelId: id, initial: '' })}>
          <span>{symbolOf(market())}</span>
          <Icon name="chevron" size={10} />
        </button>
        <VenueTag market={market()} name />
        <WatchlistToggle market={market()} />
        <span class="sep" />
        <For each={doc.timeframes}>
          {(tf) => (
            <button class="tb num tf-fav" classList={{ on: timeframe() === tf }} data-tip={tfName(tf)} onClick={() => setPanelTimeframe(id, tf)}>
              {tfLabel(tf)}
            </button>
          )}
        </For>
        <Show when={!doc.timeframes.includes(timeframe())}>
          <button class="tb num on tf-fav" data-tip={tfName(timeframe())}>
            {tfLabel(timeframe())}
          </button>
        </Show>
        {/* A narrow chart folds the buttons into the timeframe shown, which opens them all (styles.css). */}
        <button class="tb num on tf-now" data-tip={`${tfName(timeframe())} — every timeframe`} onPointerDown={(e) => e.stopPropagation()} onClick={(e) => openTfMenu(e.currentTarget)}>
          {tfLabel(timeframe())}
          <Icon name="chevron" size={10} />
        </button>
        <button class="tb tf-more" data-tip="Every timeframe — star one to keep it here" onPointerDown={(e) => e.stopPropagation()} onClick={(e) => openTfMenu(e.currentTarget)}>
          <Icon name="chevron" size={11} />
        </button>
        </Show>
        <span class="sep" />
        <button
          class="tb"
          data-tip="Copy a 4K picture of this chart · alt K — hold shift to save the file"
          aria-label="Copy a picture of this chart"
          onClick={(e) => capture(e.shiftKey)}
        >
          <Icon name="camera" size={13} />
        </button>
        <span class="sep" />
        <button class="tb" data-tip="Add an indicator" onClick={() => setOverlay({ kind: 'indicators', panelId: id })}>
          <Icon name="indicators" size={13} />
          <span>Indicators</span>
          <Show when={studyCount() > 0}>
            <span class="dim">{studyCount()}</span>
          </Show>
        </button>
        <Show when={!props.training}>
        <span class="sep" />
        <select
          data-tip="Candles, the footprint's own volume by price, or TPO profiles — the candles are the brackets, the split is how long a profile runs"
          onChange={(e) => patchPanel(id, { tpo: e.currentTarget.value === 'footprint' ? '' : e.currentTarget.value, footprint: e.currentTarget.value === 'footprint' })}
        >
          <option value="" selected={!split() && !footprintOn()}>
            Candles
          </option>
          <Show when={footprintable()}>
            <option value="footprint" selected={footprintOn()}>
              Footprint
            </option>
          </Show>
          <For each={TPO_SPLITS.filter((s) => tpoFits(s.value, timeframe()))}>
            {(s) => (
              <option value={s.value} selected={split() === s.value}>
                TPO · {s.label}
              </option>
            )}
          </For>
        </select>
        </Show>
        <Show when={footprintOn()}>
          <span class="sep" />
          <select
            class="tb"
            value={props.panel.footprintMode}
            data-tip="How a level reads: the two sides apart, their difference, or everything that traded there"
            onChange={(e) => patchPanel(id, { footprintMode: e.currentTarget.value })}
          >
            <For each={FOOTPRINT_MODES}>
              {(m) => (
                <option value={m.value} selected={props.panel.footprintMode === m.value}>
                  {m.label}
                </option>
              )}
            </For>
          </select>
          <select
            class="tb"
            value={props.panel.footprintUnit}
            data-tip="Counted in coins, or in dollars at the price each level traded at"
            onChange={(e) => patchPanel(id, { footprintUnit: e.currentTarget.value })}
          >
            <option value="coins" selected={props.panel.footprintUnit !== 'usd'}>
              Coins
            </option>
            <option value="usd" selected={props.panel.footprintUnit === 'usd'}>
              USD
            </option>
          </select>
          <For each={FOOTPRINT_EXTRAS}>
            {(extra) => (
              <button class="tb" classList={{ on: props.panel[extra.key] }} data-tip={extra.tip} onClick={() => patchPanel(id, { [extra.key]: !props.panel[extra.key] })}>
                {extra.label}
              </button>
            )}
          </For>
          <input
            class="box num"
            style={{ width: '58px' }}
            data-tip={`Ticks a row covers — empty fits the rows to the zoom, so every row carries its number. One tick is ${recorded().get(market()) ?? 1}`}
            value={props.panel.footprintTicks || ''}
            placeholder="auto"
            onChange={(e) => {
              const ticks = Number(e.currentTarget.value);
              patchPanel(id, { footprintTicks: Number.isFinite(ticks) && ticks >= 0 ? Math.min(1_000, Math.round(ticks)) : 0 });
            }}
          />
        </Show>
        <Show when={tpoShown()}>
          <button
            ref={tpoButton}
            class="tb"
            data-tip="TPO quick settings"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => setTpoMenu((v) => (v === null ? tpoButton.offsetLeft : null))}
          >
            <Icon name="chevron" size={11} />
          </button>
        </Show>
        <span class="spacer" />
        <LiveBadge market={market()} live={status().live} relayed={status().relayed} price={price()} />
      </div>

      <Show when={tpoMenu()} keyed>
        {(left) => (
          <div class="menu" style={{ left: `${Math.max(0, Math.min(left - 170, box.clientWidth - 200))}px`, top: '25px', 'min-width': '220px' }} onPointerDown={(e) => e.stopPropagation()}>
            {/* The options read most often, a click away; kept inside the panel. */}
            <For
              each={[
                { value: 'brightness', label: 'Brightness' },
                { value: 'solid', label: 'Solid colour' },
                { value: 'heatmap', label: 'Heat map' },
                { value: 'candle', label: 'Candle up/down' },
              ]}
            >
              {(mode) => (
                <button classList={{ cur: tpoValue(props.panel.tpoSettings, 'blockMode') === mode.value }} onClick={() => tpoSet('blockMode', mode.value)}>
                  {mode.label}
                </button>
              )}
            </For>
            <hr />
            <For
              each={[
                { key: 'poc', label: 'Point of control' },
                { key: 'vaRays', label: 'Value area rays' },
                { key: 'singlePrints', label: 'Single prints' },
                { key: 'poor', label: 'Poor high low' },
                { key: 'ib', label: 'Initial balance' },
                { key: 'vp', label: 'Volume profile' },
                { key: 'nakedPoc', label: 'Naked POC' },
                { key: 'highlight', label: 'Highlight start end' },
                { key: 'sessionHeader', label: 'Session header' },
              ]}
            >
              {(item) => (
                <button onClick={() => tpoSet(item.key, tpoOn(item.key) ? '0' : '1')}>
                  {item.label}
                  <span class={tpoOn(item.key) ? 'bull' : 'dim'}>{tpoOn(item.key) ? 'on' : 'off'}</span>
                </button>
              )}
            </For>
            <hr />
            <button
              onClick={() => {
                setTpoMenu(null);
                setOverlay({ kind: 'tpoSettings', panelId: id });
              }}
            >
              TPO settings…
            </button>
          </div>
        )}
      </Show>

      <Show when={tfMenu()} keyed>
        {(left) => (
          <div class="menu tf-menu" ref={fitMenu} style={{ left: `${left}px`, top: '25px' }} onPointerDown={(e) => e.stopPropagation()}>
            <For each={[0, 1, 2, 3, 4]}>
              {(group) => (
                <div class="group">
                  <For each={TIMEFRAMES.filter((t) => t.group === group)}>
                    {(t) => {
                      const favorite = () => doc.timeframes.includes(t.value);
                      return (
                        <span class="tf-pick">
                          <button
                            class="num"
                            classList={{ cur: t.value === timeframe() }}
                            data-tip={t.live ? t.name : `${t.name} — folded from shorter bars; closed bars only`}
                            onClick={() => {
                              setPanelTimeframe(id, t.value);
                              setTfMenu(null);
                            }}
                          >
                            {t.label}
                          </button>
                          {/* A favourite is a button in the chart's bar; the menu stays open to star more. */}
                          <button
                            class="fav"
                            classList={{ on: favorite() }}
                            aria-pressed={favorite()}
                            aria-label={`Favourite: ${t.name}`}
                            data-tip={favorite() ? 'In the bar — click to take it out' : 'Add to the bar'}
                            onClick={() => toggleFavoriteTimeframe(t.value)}
                          >
                            <Icon name="star" size={9} stroke={2} fill={favorite()} />
                          </button>
                        </span>
                      );
                    }}
                  </For>
                </div>
              )}
            </For>
          </div>
        )}
      </Show>

      <div class="chartwrap">
        {/* Its own rail when it is the only chart; several share one at the workspace's left. */}
        <Show when={props.training || !sharedRail()}>
          <DrawingRail target={() => (controllersChanged(), chartControllers.get(id))} />
        </Show>

        <div
          ref={box}
          class="canvasbox"
          style={{ cursor: cursor() }}
          onPointerDown={(e) => {
            if (e.button === 2) return;
            box.setPointerCapture(e.pointerId);
            fingerLast = e.pointerType === 'touch';
            dropHold();
            keepCrosshair = false;
            if (e.pointerType === 'touch') {
              fingers.set(e.pointerId, local(e));
              if (fingers.size === 1) {
                const at = local(e);
                holdFrom = at;
                holdTimer = setTimeout(() => {
                  holdTimer = undefined;
                  send({ t: 'pointer', id: sid, kind: 'hold', x: at.x, y: at.y, button: 0, touch: true });
                  keepCrosshair = true;
                  navigator.vibrate?.(8);
                }, HOLD_MS);
              }
              if (fingers.size >= 2) {
                // The first finger was panning or drawing: it is let go where
                // it stands rather than finishing whatever it began.
                if (!spent) pointer('leave', e);
                spent = true;
                const now = between();
                pinch = now && { spread: now.spread, x: now.x };
                return;
              }
              if (spent) return;
            }
            pointer('down', e);
          }}
          onPointerMove={(e) => {
            setPointerX(local(e).x);
            if (e.pointerType === 'touch') {
              if (!fingers.has(e.pointerId)) return;
              fingers.set(e.pointerId, local(e));
              // Moved before the hold came due: a pan, not a hold.
              const p = local(e);
              if (holdFrom && Math.hypot(p.x - holdFrom.x, p.y - holdFrom.y) > HOLD_SLOP) dropHold();
              const now = between();
              if (now && pinch) {
                if (pinch.spread > 0 && now.spread > 0) {
                  const dy = -Math.log(now.spread / pinch.spread) / 0.002;
                  if (Math.abs(dy) > 0.5) send({ t: 'wheel', id: sid, x: now.x, y: now.y, dx: 0, dy });
                }
                // A wheel carries one or the other: the engine pans when dx
                // leads and zooms when dy does, so the slide goes on its own.
                const slide = now.x - pinch.x;
                if (Math.abs(slide) >= 1) send({ t: 'wheel', id: sid, x: now.x, y: now.y, dx: -slide, dy: 0 });
                pinch = { spread: now.spread, x: now.x };
                return;
              }
              if (spent) return;
            }
            pointer('move', e);
          }}
          onPointerUp={(e) => {
            if (box.hasPointerCapture(e.pointerId)) box.releasePointerCapture(e.pointerId);
            dropHold();
            if (e.pointerType === 'touch' && lift(e)) return;
            if (e.button !== 2) pointer('up', e);
          }}
          onPointerCancel={(e) => {
            dropHold();
            keepCrosshair = false;
            if (e.pointerType !== 'touch' || !lift(e)) pointer('leave', e);
          }}
          onPointerLeave={(e) => {
            if (e.pointerType === 'touch' && keepCrosshair) {
              keepCrosshair = false;
              return;
            }
            pointer('leave', e);
          }}
          onDblClick={(e) => pointer('dblclick', e)}
          onContextMenu={(e) => {
            e.preventDefault();
            if (fingerLast) return;
            // While drawing, a right-click cancels it, and puts the tool down.
            if (tool() !== '') {
              setTool('');
              return;
            }
            const p = local(e);
            // On an alert's line or badge: that alert's own menu, not the chart's.
            const over = hoveredAlert();
            if (over) {
              setMenu(null);
              setAlertMenu({ id: over.alert.id, x: Math.max(0, Math.min(p.x, box.clientWidth - 200)), y: Math.max(0, Math.min(p.y, box.clientHeight - 80)) });
              return;
            }
            // On a TPO profile: MMT's menu for that profile, over the chart's own.
            const profile = tpoShown() ? (legend()?.tpo ?? undefined) : undefined;
            setMenu({ x: Math.max(0, Math.min(p.x, box.clientWidth - 220)), y: Math.max(0, Math.min(p.y, box.clientHeight - (profile ? 300 : 120))), tpo: profile });
          }}
        >
          {/* A hairline over the chart while the footprint is fetched or filled: how far, or that something is under way. */}
          <Show when={fpProgress()}>
            {(p) => <div class="fp-progress" classList={{ busy: p().share === 'busy' }} style={{ width: p().share === 'busy' ? undefined : `${Math.max(2, Math.round((p().share as number) * 100))}%` }} />}
          </Show>
          <Show when={DEBUG}>
            <div class="debug-line">
              {(tick(), `${sid} · panel ${market()} ${timeframe()} · engine ${status().market ?? '–'} ${status().timeframe ?? ''} ${status().loading ? 'loading' : 'ready'} ${status().bars} bars${status().error ? ` · ${status().error}` : ''} · sent ${marketSends} · worker ${((Date.now() - heardAt) / 1000).toFixed(1)}s ago · ${ping()}`)}
            </div>
          </Show>
          <Show when={tool() !== '' || selected() !== null || drawError()}>
            <div class="hint">
              {tool() ? TOOL_STEPS[tool()] : selected() !== null ? 'Drawing selected · double-click for settings · Delete removes it, esc deselects' : ''}
              <Show when={drawError()}>{(err) => <span class="warn"> ▲ {err()}</span>}</Show>
            </div>
          </Show>
          <Show when={pattern()}>
            {(p) => (
              <div class="pattern-bar" onPointerDown={(e) => e.stopPropagation()}>
                <span class={p().side === 'bear' ? 'bear' : 'bull'}>{p().side === 'bear' ? '▼' : '▲'} Three drives</span>
                <span class="dim">
                  {p().stage === 'line' ? 'at the line · not confirmed yet' : `score ${p().score}${p().divergence ? ' · RSI divergence' : ''}`}
                  {p().limit ? ` · limit ${formatPrice(p().limit!.price, digitsFor(p().market, p().limit!.price))}` : ''}
                </span>
                <button class="tb" data-tip="Save the line and the circles as drawings on this chart" onClick={() => keepPattern(p())}>
                  Keep drawings
                </button>
                <button class="iconbtn" aria-label="Close the pattern" data-tip="Close — nothing is saved" onClick={() => setShownPattern(null)}>
                  <Icon name="close" size={11} />
                </button>
              </div>
            )}
          </Show>
          <Show when={editing() === null && selectedDrawing()}>
            {(d) => <DrawingToolbar drawing={d()} onRestyle={(patch) => restyle(d(), patch)} onSettings={() => setEditing(d().id)} onDelete={() => void deleteDrawing(d().id)} />}
          </Show>
          <Show when={clock()}>
            {(c) => (
              <button
                class="alert-add"
                style={{ left: `${axisX() - 22}px`, top: `${c().y - 9}px` }}
                data-tip={`Add alert at ${formatPrice(c().price, digitsFor(market(), c().price))}`}
                onPointerEnter={() => setOverClock(c())}
                onPointerLeave={() => setOverClock(null)}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  addAlertAt(c().price);
                  setOverClock(null);
                }}
              >
                <Icon name="alarm" size={13} />
              </button>
            )}
          </Show>
          <Show when={hoveredCell()}>
            {(h) => (
              <div class="flow-label" classList={{ bull: h().buy > h().sell, bear: h().buy < h().sell }} style={{ left: `${h().x + 14}px`, top: `${h().y - 12}px` }}>
                <span class="dim">{formatPrice(h().price, digitsFor(market(), h().price))}</span>
                <b>{formatTurnover(h().buy)}</b>
                <span class="dim">bought</span>
                <b>{formatTurnover(h().sell)}</b>
                <span class="dim">sold</span>
                <span>{h().buy >= h().sell ? '+' : '-'}{formatTurnover(Math.abs(h().buy - h().sell))}</span>
                <span class="dim">{formatClock(h().at)}</span>
              </div>
            )}
          </Show>
          <Show when={trainingCard()}>
            {(card) => (
              <div class="trade-card" style={{ left: `${Math.min(pointerX() + 14, box.clientWidth - 250)}px`, top: `${legend()!.cursor!.y + 14}px` }}>
                {card()}
              </div>
            )}
          </Show>
          <Show when={hoveredFlow()}>
            {(h) => (
              <div class="flow-label" classList={{ bull: h().mark.deltaUsd > 0, bear: h().mark.deltaUsd < 0 }} style={{ left: `${h().x + 12}px`, top: `${h().y - 30}px` }}>
                <span class="dim">Flow</span>
                <b>
                  {h().mark.deltaUsd > 0 ? '+' : '-'}${formatTurnover(Math.abs(h().mark.deltaUsd))}
                </b>
                <span class="dim">{h().mark.deltaUsd > 0 ? 'net bought in a minute' : 'net sold in a minute'}</span>
                <span>{formatClock(h().mark.at)}</span>
                <span class="dim">{formatPrice(h().mark.price, digitsFor(market(), h().mark.price))}</span>
              </div>
            )}
          </Show>
          <Show when={hoveredAlert()}>
            {(h) => (
              <div
                class="alert-label"
                // In the line's own colour, whole: a faint line's label must still read.
                style={{ right: `${(box?.clientWidth ?? 0) - axisX() + 22}px`, top: `${h().y - 11}px`, '--al': h().alert.style?.color?.slice(0, 7) }}
                onPointerEnter={() => setOverAlert({ id: h().alert.id, y: h().y })}
                onPointerLeave={() => setOverAlert(null)}
                onPointerDown={(e) => e.stopPropagation()}
              >
                <span>{titleOf(h().alert)}</span>
                <button
                  data-tip="Line style — colour, opacity, thickness"
                  aria-label="Style this alert's line"
                  onClick={(e) => {
                    e.stopPropagation();
                    setAlertStyling(h().alert.id);
                  }}
                >
                  <Icon name="settings" size={12} />
                </button>
                <button
                  data-tip="Delete this alert"
                  aria-label="Delete this alert"
                  onClick={(e) => {
                    e.stopPropagation();
                    setOverAlert(null);
                    deleteAlert(h().alert.id).catch((err) => toast({ tone: 'warn', title: 'Alert not deleted', body: errorText(err) }));
                  }}
                >
                  <Icon name="trash" size={12} />
                </button>
              </div>
            )}
          </Show>
          <div class="legend">
            <div class="line">
              <span class="title">{symbolOf(market())}</span>
              <span class="dim">
                {tfLabel(timeframe())}
                {split() ? ` · TPO ${TPO_SPLITS.find((t) => t.value === split())?.label}` : ''} · {venueLabel(market())}
                <Show when={fpNote()}>
                  {(note) => (
                    <span class="dim" data-tip={note().tip}>
                      {` · ${note().text}`}
                    </span>
                  )}
                </Show>
              </span>
              <Show when={legend()}>
                {(l) => (
                  <>
                    <span class="num muted">{l().ohlc}</span>
                    <span class="num" classList={{ bull: l().up, bear: !l().up }}>
                      {l().change}
                    </span>
                    <span class="num dim">{l().volume}</span>
                  </>
                )}
              </Show>
            </div>
            <Show when={!props.panel.legendShut}>
              <For each={LAYERS.filter((layer) => !(tpoShown() && layer.candles))}>{(layer) => <LayerRow label={layer.label} off={hidden().has(layer.id)} onToggle={() => toggleLayer(layer.id)} />}</For>
            </Show>
            <For each={props.panel.legendShut ? [] : studies()}>
              {(s) => (
                <LayerRow
                  label={s.label}
                  off={s.hidden}
                  onToggle={() => toggleLayer(s.id)}
                  onRemove={() => removeStudy(s.id)}
                  onSettings={s.id.split(':')[0] === 'oi' ? () => setStudyEditing(s.id) : undefined}
                  values={s.hidden ? [] : s.lines.map((line) => ({ color: line.color, text: () => legend()?.values[line.id] ?? '—' }))}
                  stats={s.hidden ? [] : s.stats}
                />
              )}
            </For>
            {/* The list's own handle, under it: folded, the count is all that
                says what is down there. */}
            <Show when={folded() > 0}>
              <button
                class="fold"
                classList={{ shut: props.panel.legendShut }}
                data-tip={props.panel.legendShut ? `Show the ${folded()} layers and indicators` : 'Hide them'}
                aria-label={props.panel.legendShut ? `Show the ${folded()} layers and indicators` : 'Hide the layers and indicators'}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => patchPanel(id, { legendShut: !props.panel.legendShut })}
              >
                <Icon name="chevron" size={11} stroke={2.2} />
                <Show when={props.panel.legendShut}>
                  <span class="count num">{folded()}</span>
                </Show>
              </button>
            </Show>
          </div>
          <Show when={alertMenu()}>
            {(m) => (
              <div class="menu" style={{ left: `${m().x}px`, top: `${m().y}px` }} onPointerDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
                <button
                  onClick={() => {
                    setAlertStyling(m().id);
                    setAlertMenu(null);
                  }}
                >
                  Line style…<span class="dim">colour, opacity, thickness</span>
                </button>
                <hr />
                <button
                  onClick={() => {
                    const id = m().id;
                    setAlertMenu(null);
                    deleteAlert(id).catch((err) => toast({ tone: 'warn', title: 'Alert not deleted', body: errorText(err) }));
                  }}
                >
                  Delete alert
                </button>
              </div>
            )}
          </Show>
          <Show when={menu()}>
            {(m) => (
              <div class="menu" style={{ left: `${m().x}px`, top: `${m().y}px` }} onPointerDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
                <Show when={m().tpo}>
                  {(h) => (
                    <>
                      <button onClick={() => tpoEdit(toggleExpanded, h())}>
                        {h().expanded ? 'Collapse' : 'Expand'}
                        <span class="dim">{h().expanded ? 'back to its shape' : 'a column a bracket'}</span>
                      </button>
                      <button disabled={h().next === null} onClick={() => tpoEdit(mergeRight, h())}>
                        Merge right<span class="dim">→|</span>
                      </button>
                      <button disabled={h().prev === null} onClick={() => tpoEdit(mergeLeft, h())}>
                        Merge left<span class="dim">|←</span>
                      </button>
                      <button disabled={!canSplit(h())} onClick={() => tpoEdit(splitProfile, h())}>
                        Split profile<span class="dim">{h().merged ? 'into its sessions' : h().splitAt !== null ? 'at the pointer' : 'expand it first'}</span>
                      </button>
                      <button classList={{ cur: tpoPrices() }} onClick={() => setTpoPrices((v) => !v)}>
                        Prices<span class="dim">{tpoPrices() ? '▾' : '▸'}</span>
                      </button>
                      <Show when={tpoPrices()}>
                        <For each={h().prices}>
                          {([name, value]) => (
                            <button
                              class="sub"
                              data-tip="Copy it"
                              onClick={() => {
                                const text = formatPrice(value, digitsFor(market(), value));
                                void navigator.clipboard?.writeText(text).then(() => toast({ tone: 'info', title: `${name} copied`, body: text }, 2_500));
                                setMenu(null);
                              }}
                            >
                              {name}
                              <span class="num">{formatPrice(value, digitsFor(market(), value))}</span>
                            </button>
                          )}
                        </For>
                      </Show>
                      <button
                        onClick={() => {
                          patchPanel(id, { tpoEdits: '' });
                          send({ t: 'resetView', id: sid });
                          setMenu(null);
                        }}
                      >
                        Reset chart<span class="dim">every profile as it was</span>
                      </button>
                      <hr />
                    </>
                  )}
                </Show>
                <button
                  onClick={() => {
                    send({ t: 'resetView', id: sid });
                    setMenu(null);
                  }}
                >
                  Reset chart view<span class="dim">auto-scale</span>
                </button>
                <Show when={!autoScale()}>
                  <button
                    onClick={() => {
                      send({ t: 'autoScale', id: sid });
                      setMenu(null);
                    }}
                  >
                    Auto-scale price<span class="dim">follow the bars</span>
                  </button>
                </Show>
                <button
                  onClick={() => {
                    patchPanel(id, { log: !props.panel.log });
                    setMenu(null);
                  }}
                >
                  {props.panel.log ? 'Linear price scale' : 'Logarithmic price scale'}
                </button>
                <button
                  onClick={() => {
                    setMenu(null);
                    setOverlay({ kind: 'indicators', panelId: id });
                  }}
                >
                  Add indicator…
                </button>
                <hr />
                <button
                  disabled={offline() !== null}
                  onClick={() => {
                    setMenu(null);
                    setTool('horizontal');
                  }}
                >
                  Draw a level<span class="dim">tool</span>
                </button>
              </div>
            )}
          </Show>
        </div>
      </div>

      <Show when={studyEditing()} keyed>
        {(studyId) => <OpenInterestSettings id={studyId} onApply={(next) => replaceStudy(studyId, next)} onClose={() => setStudyEditing(null)} />}
      </Show>
      {/* Keyed on the id, not the alert: the list is reloaded now and then, and
          a fresh copy of the same alert must not start the dialog over. */}
      <Show when={alertStyling()} keyed>
        {(alertId) => (
          <Show when={untrack(() => alertState.alerts.find((x) => x.id === alertId))}>
            {(a) => (
              <AlertStyleSettings
                title={titleOf(a())}
                style={a().style}
                onPreview={(style) => setAlertDraft({ id: alertId, style })}
                onApply={(style) => {
                  setAlertStyling(null);
                  // The draft stays drawn until the saved style replaces it, so the
                  // line does not flick back to its old look for the round trip.
                  setAlertDraft({ id: alertId, style });
                  styleAlert(alertId, style)
                    .catch((err) => toast({ tone: 'warn', title: 'Alert style not saved', body: errorText(err) }))
                    .finally(() => setAlertDraft((d) => (d?.style === style ? null : d)));
                }}
                onCancel={() => {
                  setAlertStyling(null);
                  setAlertDraft(null);
                }}
              />
            )}
          </Show>
        )}
      </Show>

      {/* Keyed on the id alone: each previewed change must not remount the dialog. */}
      <Show when={editing()} keyed>
        {(editId) => {
          const d = untrack(() => drawings().find((x) => x.id === editId));
          return (
            <Show when={d}>
              {(drawing) => (
                <DrawingSettings
                  drawing={drawing()}
                  precision={untrack(() => price()?.precision ?? 2)}
                  onPreview={(patch) => previewDrawing(editId, patch)}
                  onCommit={(patch) => commitDrawing(drawing(), patch)}
                  onCancel={() => setEditing(null)}
                />
              )}
            </Show>
          );
        }}
      </Show>

      <div class="rangebar">
        <For each={RANGES}>
          {(r) => (
            <button class="tb num" data-tip={r.days === null ? 'Every loaded bar' : `Zoom to the last ${r.label}`} onClick={() => showRange(r.days)}>
              {r.label}
            </button>
          )}
        </For>
        <span class="spacer" />
        <Show when={status().error}>{(err) => <span class="warn" data-tip={err()}>▲ {err()} · </span>}</Show>
        <span class="dim num" data-tip={status().source === 'archive' ? 'Stored history, with the exchange filling the newest bars' : status().source === 'ratio' ? 'One market priced in the other: each bar of the first divided by the same bar of the second' : 'Candles from the exchange; this market is not archived here'}>
          {status().loading ? 'loading… · ' : ''}
          {status().bars.toLocaleString('en-US')} bars{status().source ? ` · ${status().source}` : ''} · {zoneLabel()}
        </span>
        <span class="sep" />
        <button
          class="tb"
          classList={{ on: autoScale() }}
          data-tip="Auto scale — the price axis follows the bars on screen. Dragging the chart up or down, or dragging the price axis, turns it off; click to turn it back on."
          onClick={() => send({ t: 'autoScale', id: sid })}
        >
          AUTO
        </button>
        <button class="tb" classList={{ on: !props.panel.log }} data-tip="Linear price axis" onClick={() => patchPanel(id, { log: false })}>
          LIN
        </button>
        <button class="tb" classList={{ on: props.panel.log }} data-tip="Logarithmic price axis — equal percentage moves take equal space" onClick={() => patchPanel(id, { log: true })}>
          LOG
        </button>
        <button class="tb" data-tip="Reset view — the last bars, every axis autoscaling" onClick={() => send({ t: 'resetView', id: sid })}>
          <Icon name="reset" size={11} />
        </button>
      </div>
    </div>
  );
}

function LayerRow(props: {
  label: string;
  off: boolean;
  onToggle(): void;
  onRemove?: () => void;
  onSettings?: () => void;
  values?: { color: string; text: () => string }[];
  stats?: { label: string; value: string }[];
}) {
  return (
    <div class="layer" classList={{ off: props.off }}>
      <span class="name">{props.label}</span>
      <For each={props.values ?? []}>
        {(v) => (
          <span class="val" style={{ color: v.color }}>
            {v.text()}
          </span>
        )}
      </For>
      <For each={props.stats ?? []}>
        {(s) => (
          <span class="dim">
            {s.label} <span class="val muted">{s.value}</span>
          </span>
        )}
      </For>
      {/* Only the buttons keep the press: a drag that starts on the legend text still pans. */}
      <span class="btns" onPointerDown={(e) => e.stopPropagation()}>
        <Show when={props.onSettings}>
          {(open) => (
            <button data-tip="Settings" onClick={() => open()()}>
              <Icon name="settings" size={13} />
            </button>
          )}
        </Show>
        <button data-tip={props.off ? 'Show' : 'Hide'} onClick={() => props.onToggle()}>
          <Icon name={props.off ? 'eyeOff' : 'eye'} size={13} />
        </button>
        <Show when={props.onRemove}>
          {(remove) => (
            <button data-tip="Remove" onClick={() => remove()()}>
              <Icon name="close" size={13} />
            </button>
          )}
        </Show>
      </span>
    </div>
  );
}

/**
 * Stream state and the last price. The decimals only ever grow, so the number
 * keeps its width as prints land on round prices, and it holds the colour of
 * the last change through unchanged prints.
 */
function LiveBadge(props: { market: string; live: LiveStatus; relayed: boolean; price: LastPrice | null }) {
  let digits = 0;
  createEffect(
    on(
      () => props.market,
      () => {
        digits = 0;
      },
    ),
  );
  // The same move the tab title's arrow shows.
  const direction = () => props.price?.direction ?? 0;
  const text = () => {
    const p = props.price;
    if (!p || !Number.isFinite(p.price)) return '';
    // The chart's own precision stands, even past the cap that keeps float noise out.
    digits = Math.max(p.precision, Math.min(MAX_PRICE_DECIMALS, Math.max(digits, decimalsOf(p.price))));
    return formatPrice(p.price, digits);
  };
  const state = () => LIVE[props.live];
  return (
    <span class="live" data-tip={props.relayed ? `${state().title} Through the tradoor server, because this network blocks the exchange.` : state().title}>
      <span class={`state ${state().tone}`} classList={{ pulse: props.live === 'connecting' || props.live === 'retrying' }}>
        {state().label}
      </span>
      {/* Which way it came, where that is not the exchange itself. */}
      <Show when={props.relayed}>
        <span class="state dim">RELAY</span>
      </Show>
      <span class="num" classList={{ bull: direction() > 0, bear: direction() < 0, muted: direction() === 0 }}>
        {text()}
      </span>
    </span>
  );
}
