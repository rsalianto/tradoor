/** The tradoor server: the market catalogue, drawings and workspaces. Candles
 *  do not come through here — they are files under /data and exchange REST. */
import type { DrawingPoint } from '../engine/protocol';
import type { Style } from './drawing-style';
import { listing, type Listing } from './catalog-wire';
import { authHeaders, SIGNED_OUT, startOver } from './owner';

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

async function request<T>(path: string, init: { method?: string; body?: unknown } = {}): Promise<{ status: number; body: T | null }> {
  const method = init.method ?? 'GET';
  // Drawings and layouts are this browser's own, or its account's: the server scopes them by this.
  const headers = authHeaders();
  if (init.body !== undefined) headers['content-type'] = 'application/json';
  let res: Response;
  try {
    res = await fetch(path, { method, headers, body: init.body === undefined ? undefined : JSON.stringify(init.body) });
  } catch {
    throw new ApiError(0, 'the tradoor server is not reachable');
  }
  const text = await res.text();
  if (!text) return { status: res.status, body: null };
  let body: T;
  try {
    body = JSON.parse(text) as T;
  } catch {
    // A dev proxy with nothing behind it answers with plain text.
    throw new ApiError(res.status, res.status >= 500 ? 'the tradoor server is not reachable' : `unexpected response (${res.status})`);
  }
  if (isSignedOut(res.status, body)) startOver(SIGNED_OUT);
  return { status: res.status, body };
}

/** The server's word that what this browser holds opens nothing any more (bins/server session). */
export const isSignedOut = (status: number, body: unknown): boolean => status === 401 && (body as { signedOut?: unknown } | null)?.signedOut === true;

/** A call answered with no body (204): only its status matters. */
async function send(path: string, init?: { method?: string; body?: unknown }): Promise<void> {
  const { status, body } = await request<{ error?: unknown }>(path, init);
  if (status >= 400) throw new ApiError(status, typeof body?.error === 'string' ? body.error : `request failed (${status})`);
}

/** A call answered with JSON; exported for the lazily loaded parts (the journal) to ask in the same way. */
export async function json<T>(path: string, init?: { method?: string; body?: unknown }): Promise<T> {
  const { status, body } = await request<T>(path, init);
  if (status >= 400 || body === null) {
    const error = (body as { error?: unknown } | null)?.error;
    throw new ApiError(status, typeof error === 'string' ? error : `request failed (${status})`);
  }
  return body;
}

export interface Product {
  market: string;
  venue: string;
  symbol: string;
  base: string;
  quote: string;
  kind: 'spot' | 'perp' | 'index';
  /** What it is, where the symbol does not say: an index's or a stock's name. */
  name?: string;
  priceDecimals: number | null;
  quoteVolume24h: number | null;
  lastPrice: number | null;
  changePct: number | null;
}

export interface VenueStatus {
  id: string;
  label: string;
  count: number;
  error: string | null;
}

export interface Catalog {
  updatedAt: number;
  venues: VenueStatus[];
  products: Product[];
  byMarket: Map<string, Product>;
}

let indexed: { of: Promise<Listing>; value: Promise<Catalog> } | null = null;

/** Every market on every venue, fetched once and searched locally. */
export function catalog(): Promise<Catalog> {
  const of = listing();
  if (indexed?.of !== of) indexed = { of, value: of.then((c) => ({ ...c, byMarket: new Map<string, Product>(c.products.map((p) => [p.market, p])) })) };
  return indexed.value;
}

export interface Drawing {
  id: number;
  market: string;
  timeframe: string;
  kind: string;
  points: DrawingPoint[];
  style: Style;
  label: string | null;
  createdAt: number;
}

/** A market's candle chart and its TPO chart keep separate drawings, and the training chart its own. */
export type ChartKind = 'candles' | 'tpo' | 'training';

const scope = (market: string, timeframe: string, chart: ChartKind): string => `market=${encodeURIComponent(market)}&timeframe=${encodeURIComponent(timeframe)}&chart=${chart}`;

/** One of the four totals: 1h, 4h, 12h or 24h, over every exchange. */
export interface LiquidationWindow {
  window: string;
  longUsd: number;
  shortUsd: number;
  longs: number;
  shorts: number;
}

export interface LiquidationRow {
  venue: string;
  time: number;
  price: number;
  qty: number;
  usd: number;
  long: boolean;
}

/** How the server's own socket to an exchange is. */
export interface CollectorVenue {
  venue: string;
  state: 'connecting' | 'connected' | 'retrying';
  since: number;
  error: string | null;
}

/** A ticker the server keeps a day of, or one it does not — with the ones it does. */
export type LiquidationStats =
  | { ticker: string; tracked: false; tickers: string[] }
  | { ticker: string; tracked: true; now: number; coverageFrom: number; windows: LiquidationWindow[]; venues: CollectorVenue[]; recent: LiquidationRow[] };

export type AlertCondition = 'crossing' | 'crossing_up' | 'crossing_down' | 'greater' | 'less';

export interface Alert {
  id: number;
  market: string;
  condition: AlertCondition;
  price: number;
  message: string | null;
  active: boolean;
  createdAt: number;
  triggeredAt: number | null;
  triggerPrice: number | null;
  /** What the market cost when the level was set: which way it is reached. */
  startPrice: number | null;
  /** How its line is drawn; what is left out is the chart's default. */
  style?: AlertStyle;
}

/** An alert line's colour (`#rrggbb[aa]`, opacity included), dash and width. */
export interface AlertStyle {
  color?: string;
  dash?: 'solid' | 'dashed' | 'dotted';
  width?: number;
}

/** One trigger, kept after its alert is edited or deleted. */
export interface AlertLog {
  id: number;
  alertId: number | null;
  market: string;
  condition: AlertCondition;
  price: number;
  message: string | null;
  time: number;
  value: number;
  startPrice: number | null;
}

/** One market's taker flow: what buyers took less what sellers took. */
export interface FlowMarket {
  market: string;
  price: number;
  /** Net flow over the last minute, and over longer stretches. */
  minute: number;
  m5: number;
  m15: number;
  h1: number;
  h4: number;
  h12: number;
  day: number;
  thresholdUsd: number;
  notify: boolean;
}

export interface FlowWatch {
  market: string;
  thresholdUsd: number;
  notify: boolean;
}

/** A minute whose flow passed the threshold. */
export interface FlowEvent {
  id: number;
  market: string;
  at: number;
  deltaUsd: number;
  windowMs: number;
  price: number;
}

/** A minute whose flow ran harder than the reader asks, to mark on a chart. */
export interface FlowMinute {
  at: number;
  deltaUsd: number;
  price: number;
}

/** A coin whose flow the server follows. */
export interface FlowCoin {
  coin: string;
  /** When it lapses; null for one recorded until it is stopped, as BTC always is. */
  until: number | null;
  /** One of those always recorded — BTC and ETH — that nobody stops. */
  fixed?: boolean;
  /** For one of this reader's own: false while the server has no room to record it. */
  recorded?: boolean;
}

/** How long a coin is recorded: a day, a week, or until it is stopped. */
export type FlowSpan = 'day' | 'week' | 'always';

export interface FlowState {
  /** Whether this server follows any market's flow. */
  watching: boolean;
  windowMs?: number;
  /** The coin shown: the one asked for, or BTC once that one has lapsed. */
  coin?: string;
  /** BTC and ETH, then this reader's own, in the order they were asked for. */
  coins?: FlowCoin[];
  /** Whether this browser is signed in, and so may follow coins of its own. */
  signedIn?: boolean;
  /** How many coins one reader may follow besides BTC and ETH. */
  perReader?: number;
  markets: FlowMarket[];
  events: FlowEvent[];
}

/** A drive's extreme, or where a pattern turned. */
export interface SignalPoint {
  at: number;
  price: number;
}

/** A pattern the screener found, as its latest scan saw it. */
export interface Signal {
  id: number;
  market: string;
  side: 'bear' | 'bull';
  drives: SignalPoint[];
  /** The bar Stochastic RSI turned on; null while the pattern waits for it. */
  turnedAt: number | null;
  turnPrice: number | null;
  score: number;
  divergence: boolean;
  /** The close of the newest bar the scan read. */
  price: number;
  foundAt: number;
  /** Where the pattern points, where it is wrong, and where it was entered once it turned. */
  target: number | null;
  stop: number | null;
  entry: number | null;
  /** How it played out, once it has; and the move so far, the pattern's way. */
  outcome: SignalOutcome | null;
  ret: number | null;
  /** The close of the newest bar of the last scan that saw it. */
  lastSeen: number;
  /** The trade, once it turned and was entered. */
  levels: SignalLevels | null;
  /** Buyers weaker on the push into the third drive than into the second, and
   *  the third drive's bar sold more than bought; null where the exchange does
   *  not say who took each trade. */
  deltaDivergence: boolean | null;
  deltaAgainst: boolean | null;
  /** The push into each drive, where the exchange says who took each trade. */
  deltas: SignalPush[] | null;
  /** The limit entry the scan that first saw it set, once weighed. */
  limit: SignalLimit | null;
  /** The length of bar it was found on: four hours, or the day's or week's — the strong kind. */
  timeframe: '4h' | '1d' | '1w';
  /** Why it left the list, once it has. */
  endedWhy: 'outdone' | 'broken' | 'played' | 'waited' | 'lapsed' | 'hot' | 'unread' | 'replaced' | 'gone' | null;
}

/** A limit entry: half way back from the furthest price since the third drive
 *  toward it, set when a scan first saw the pattern and waiting two days. */
export interface SignalLimit {
  price: number;
  /** The bar the scan that set it read up to, and the end of its two days. */
  setAt: number;
  until: number | null;
  /** The trade at its entry. */
  levels: SignalLevels | null;
  /** Where it filled — at the limit, or better where price opened past it. */
  filledAt: number | null;
  entry: number | null;
  /** How the trade went, or `missed` when price never came back or reached TP1 first. */
  outcome: SignalOutcome | 'missed' | null;
  ret: number | null;
}

/** What takers did on the push into a drive: bought less sold, in dollars and
 *  as a part of all that traded. Positive is buyers, on either side. */
export interface SignalPush {
  usd: number;
  share: number;
}

/** How a trade closed: all three take-profits, two or one (the rest out at the
 *  entry or after four days), the stop before the first, or nothing in four days. */
export type SignalOutcome = 'tp3' | 'tp2' | 'tp1' | 'stop' | 'expired';

/** A trade's prices: take-profits a drop of 3, 6 and 9% from the entry (a rise
 *  for the bullish side), and the stop 3.5% past the third drive's extreme. */
export interface SignalLevels {
  tp: [number, number, number];
  sl: number;
}

/** A cut of the record: by score band — a limit's fifteen lower, set before
 *  the turn adds them — divergence, or both. */
export type SignalCut = 'all' | '70' | '50' | '0' | '55' | '40' | 'u40' | 'div' | 'ddiv' | '70ddiv' | '55ddiv' | 'dagainst';

/** One line of the record: a side, and a cut of it by score or divergence. */
export interface SignalGroup {
  side: 'bear' | 'bull';
  group: SignalCut;
  /** For limits: every one set, filled or not; `n` are those filled. */
  seen: number;
  n: number;
  /** Closed in profit. */
  win: number;
  /** How many reached each take-profit, each counting those that went further. */
  tp1: number;
  tp2: number;
  tp3: number;
  stop: number;
  expired: number;
  /** Average moves in percent, positive the pattern's way: to the exit, and at best and worst before it. */
  ret: number;
  best: number;
  worst: number;
}

/** A live pattern that turned, and how it went. */
export interface SignalPlayed {
  id: number;
  market: string;
  side: 'bear' | 'bull';
  drives: SignalPoint[];
  score: number;
  divergence: boolean;
  entryAt: number;
  entry: number | null;
  target: number | null;
  stop: number | null;
  outcome: SignalOutcome | null;
  outcomeAt: number | null;
  ret: number | null;
  best: number | null;
  worst: number | null;
}

/** The touches of one side: what they became, and target or breakout first. */
export interface SignalTouchGroup {
  side: 'bear' | 'bull';
  n: number;
  /** Closed in profit. */
  win: number;
  /** Became three drives within four days, and of those, turned. */
  pattern: number;
  turned: number;
  tp1: number;
  tp2: number;
  tp3: number;
  stop: number;
  expired: number;
  ret: number;
  best: number;
  worst: number;
}

/** A live touch of a line, and how it went. */
export interface SignalTouchPlayed {
  market: string;
  side: 'bear' | 'bull';
  drives: SignalPoint[];
  touchedAt: number;
  touchPrice: number;
  target: number | null;
  became: 'turned' | 'pattern' | 'none' | null;
  outcome: SignalOutcome | null;
  ret: number | null;
  best: number | null;
  worst: number | null;
}

/** One way of setting the take-profits and the stop, replayed: a kind, a side and a cut of it. */
export interface SignalPlanRow {
  plan: 'fixed' | 'fib' | 'fib-atr' | 'from-entry';
  kind: 'pattern' | 'touch' | 'limit';
  side: 'bear' | 'bull';
  group: SignalCut;
  n: number;
  win: number;
  tp1: number;
  tp2: number;
  tp3: number;
  stop: number;
  expired: number;
  ret: number;
  /** The average result in the first half of the replay, and in the second. */
  halves: [number, number];
}

export interface SignalStats {
  version: number;
  /** Bars a pattern is given to reach its target. */
  windowBars: number;
  /** The rules replayed over the history, bar by bar. */
  backtest: { at: number; version: number; markets: number; from: number; to: number; groups: SignalGroup[]; limits: SignalGroup[]; variants: SignalPlanRow[] | null } | null;
  live: { since: number | null; open: number; groups: SignalGroup[] };
  /** The day's and the week's patterns, live, apart from the four hours'. */
  higher: { since: number | null; open: number; groups: SignalGroup[] };
  /** Limit entries live: counted from the patterns first seen once they began. */
  limits: { since: number; open: number; groups: SignalGroup[] };
  recent: SignalPlayed[];
  /** Price at the line between scans: replayed, live, and the latest. */
  touches: {
    replayed: SignalTouchGroup[];
    live: { since: number | null; open: number; groups: SignalTouchGroup[] };
    recent: SignalTouchPlayed[];
  };
}

/** Price reaching the line of two drives between scans: a third drive as it happens. */
export interface SignalTouch {
  market: string;
  side: 'bear' | 'bull';
  /** The first two drives. */
  drives: SignalPoint[];
  touchedAt: number;
  touchPrice: number;
  /** The line's price when it was reached, and now. */
  line: number;
  lineNow: number;
  /** The trade entered at the touch. */
  levels: SignalLevels;
  /** The pushes into the two drives, and into the line so far once reached;
   *  and whether that last one is the weaker. Null without taker volume. */
  deltas: SignalPush[] | null;
  deltaDivergence: boolean | null;
  /** How the trade went, once it has: a touch whose trade ended is listed under Ended. */
  outcome: SignalOutcome | null;
  ret: number | null;
  outcomeAt: number | null;
}

export interface SignalsState {
  scannedAt: number | null;
  /** The close of the newest bar the latest scan read. */
  bar: number | null;
  markets: number;
  nextAt: number;
  timeframe: string;
  /** Whether this browser is told on Telegram when a pattern turns, and when price reaches a line. */
  notify: boolean;
  notifyTouch: boolean;
  notifyLimits: boolean;
  /** Whether the server has a bot, and this browser a chat linked to it. */
  telegram: boolean;
  linked: boolean;
  signals: Signal[];
  /** Patterns that had turned and left the list within the last day. */
  ended: Signal[];
  /** Setups whose line price has reached, their trade still open. */
  touches: SignalTouch[];
  /** Touches whose trade ended within the last day. */
  endedTouches: SignalTouch[];
}

/** A Telegram chat an alert is sent to. */
export interface TelegramChat {
  chatId: number;
  name: string | null;
  linkedAt: number;
  /** The clock messages to this chat are written in. */
  offsetMinutes: number;
  zone: string | null;
  /** Hours a message stays before the bot takes it back; null keeps them. */
  clearAfterHours: number | null;
}

/** The reader's clock, as the page reads it now. */
export interface Clock {
  offsetMinutes: number;
  zone: string;
}

/** The account this browser's id belongs to, and whether signing in is on offer here. */
export interface AccountStatus {
  telegram: boolean;
  bot: string | null;
  /** The account and how many browsers are signed in to it. */
  account: { name: string | null; since: number; devices?: number } | null;
}

/** A sign-in waiting on the reader's yes in Telegram, done with a session for this browser, or gone. */
export type SignIn = { state: 'waiting' } | { state: 'done'; session: string; name: string | null } | { state: 'gone' };

export interface TelegramStatus {
  /** Whether the server was given a bot token at all. */
  enabled: boolean;
  /** The bot's @name, once it has answered. */
  bot: string | null;
  /** The longest a message can be set to stay: Telegram's own limit. */
  maxClearHours?: number;
  chats: TelegramChat[];
}

export const api = {
  async health(): Promise<{ ok: boolean; db: boolean }> {
    const { body } = await request<{ ok: boolean; db: boolean }>('/api/health');
    return body ?? { ok: false, db: false };
  },
  drawings: (market: string, timeframe: string, chart: ChartKind) => json<{ count: number; drawings: Drawing[] }>(`/api/drawings?${scope(market, timeframe, chart)}`),
  createDrawing: (market: string, timeframe: string, chart: ChartKind, input: { kind: string; points: DrawingPoint[]; label?: string; style?: Style }) =>
    json<{ drawing: Drawing }>(`/api/drawings?${scope(market, timeframe, chart)}`, { method: 'POST', body: input }),
  updateDrawing: (id: number, patch: { points?: DrawingPoint[]; label?: string | null; style?: Style }) => json<{ drawing: Drawing }>(`/api/drawings/${id}`, { method: 'PATCH', body: patch }),
  deleteDrawing: (id: number) => json<{ id: number }>(`/api/drawings/${id}`, { method: 'DELETE' }),
  clearDrawings: (market: string, timeframe: string, chart: ChartKind) => json<{ deleted: number }>(`/api/drawings?${scope(market, timeframe, chart)}`, { method: 'DELETE' }),
  alerts: () => json<{ alerts: Alert[]; log: AlertLog[] }>('/api/alerts'),
  createAlert: (input: { market: string; condition: AlertCondition; price: number; message?: string | null; atPrice?: number; style?: AlertStyle }) => json<{ alert: Alert }>('/api/alerts', { method: 'POST', body: input }),
  updateAlert: (id: number, patch: { price?: number; condition?: AlertCondition; message?: string | null; active?: boolean; atPrice?: number; style?: AlertStyle }) => json<{ alert: Alert }>(`/api/alerts/${id}`, { method: 'PATCH', body: patch }),
  triggerAlert: (id: number, value: number, from?: number | null) => json<{ alert: Alert; log: AlertLog | null }>(`/api/alerts/${id}`, { method: 'PATCH', body: { triggered: { value, from } } }),
  deleteAlert: (id: number) => send(`/api/alerts/${id}`, { method: 'DELETE' }),
  clearAlerts: (which: 'stopped' | 'active' | 'all') => json<{ deleted: number }>(`/api/alerts?which=${which}`, { method: 'DELETE' }),
  restartAlerts: () => json<{ restarted: number }>('/api/alerts/restart', { method: 'POST' }),
  clearAlertLog: () => send('/api/alerts/log', { method: 'DELETE' }),
  account: () => json<AccountStatus>('/api/account'),
  signIn: (clock: Clock) => json<{ code: string; url: string; goodForMs: number }>('/api/account/signin', { method: 'POST', body: clock }),
  signedIn: (code: string) => json<SignIn>(`/api/account/signin/${encodeURIComponent(code)}`),
  signOut: () => send('/api/account/session', { method: 'DELETE' }),
  signOutEverywhere: () => json<{ signedOut: number }>('/api/account/sessions', { method: 'DELETE' }),
  telegram: () => json<TelegramStatus>('/api/telegram'),
  telegramLink: (clock: Clock) => json<{ code: string; url: string; goodForMs: number }>('/api/telegram/link', { method: 'POST', body: clock }),
  telegramClock: (clock: Clock) => json<{ chats: number }>('/api/telegram', { method: 'PATCH', body: clock }),
  telegramClearing: (clearAfterHours: number | null) => json<{ chats: number }>('/api/telegram', { method: 'PATCH', body: { clearAfterHours } }),
  telegramTest: () => json<{ sent: number }>('/api/telegram/test', { method: 'POST' }),
  telegramUnlink: (chatId: number) => send(`/api/telegram/${chatId}`, { method: 'DELETE' }),
  liquidations: (ticker: string) => json<LiquidationStats>(`/api/liquidations/${encodeURIComponent(ticker)}`),
  signals: () => json<SignalsState>('/api/signals'),
  signalStats: () => json<SignalStats>('/api/signals/stats'),
  signalsNotify: (patch: { notify?: boolean; touch?: boolean; limits?: boolean }) =>
    json<{ notify: boolean; touch: boolean; limits: boolean }>('/api/signals/notify', { method: 'PUT', body: patch }),
  flow: (coin: string) => json<FlowState>(`/api/flow?coin=${encodeURIComponent(coin)}`),
  flowFollow: (coin: string, span: FlowSpan = 'day') => json<FlowCoin & { markets: string[] }>(`/api/flow/coins/${encodeURIComponent(coin)}?for=${span}`, { method: 'PUT' }),
  flowUnfollow: (coin: string) => json<{ coin: string }>(`/api/flow/coins/${encodeURIComponent(coin)}`, { method: 'DELETE' }),
  flowMinutes: (market: string) => json<{ market: string; thresholdUsd: number; minutes: FlowMinute[] }>(`/api/flow/minutes?market=${encodeURIComponent(market)}`),
  flowWatch: (market: string, watch: { thresholdUsd: number; notify: boolean }) => json<FlowWatch>(`/api/flow/${encodeURIComponent(market)}`, { method: 'PUT', body: watch }),
  workspace: (id: string) => json<{ id: string; name: string; doc: unknown; version: number }>(`/api/workspaces/${id}`),
  /** The layouts this reader saved by name, newest first, and how many they may keep. */
  layouts: () => json<{ layouts: { id: string; name: string; version: number; updatedAt: number }[]; max: number }>('/api/workspaces'),
  deleteLayout: (id: string) => send(`/api/workspaces/${id}`, { method: 'DELETE' }),
  /** What this server records volume by price for, and at what level. */
  async footprintMarkets(): Promise<{ markets: { market: string; tick: number }[]; keptDays: number }> {
    const { body } = await request<{ markets: { market: string; tick: number }[]; keptDays: number }>('/api/footprint/markets');
    return body ?? { markets: [], keptDays: 0 };
  },

  /** The minutes recorded between `from` and `to`, each as its price levels. */
  async footprint(market: string, from: number, to: number): Promise<{ tick: number; bars: { at: number; trades: number; levels: [number, number, number][] }[] }> {
    const { status, body } = await request<{ tick: number; bars: { at: number; trades: number; levels: [number, number, number][] }[] }>(
      `/api/footprint?market=${encodeURIComponent(market)}&from=${Math.round(from)}&to=${Math.round(to)}`,
    );
    if (status >= 400 || !body) throw new ApiError(status, `no footprint for ${market}`);
    return body;
  },

  async saveWorkspace(id: string, body: { name: string; doc: unknown; version: number }): Promise<{ version: number; conflict: boolean; doc?: unknown }> {
    const res = await request<{ version: number; doc?: unknown; error?: string }>(`/api/workspaces/${id}`, { method: 'PUT', body });
    if (res.status === 409 && res.body) return { version: res.body.version, conflict: true, doc: res.body.doc };
    if (res.status >= 400 || !res.body) throw new ApiError(res.status, res.body?.error ?? `saving the workspace failed (${res.status})`);
    return { version: res.body.version, conflict: false };
  },
};
