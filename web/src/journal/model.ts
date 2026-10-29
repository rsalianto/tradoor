/**
 * The trading journal's trades and what they add up to — the figures of
 * rsalianto's journal (components/apps/trading-journal/types.ts), over trades
 * the server keeps: every long or short position drawn on a chart, and those
 * added by hand. Price decides how each went until a status is set by hand.
 */
import { ApiError, isSignedOut, json } from '../lib/api';
import { authHeaders, SIGNED_OUT, startOver } from '../lib/owner';

export type Side = 'long' | 'short';
export type Status = 'planned' | 'open' | 'win' | 'loss' | 'closed' | 'breakeven' | 'cancelled';

/** Part taken off on the way to the target, part cut on the way to the stop, or more added. */
export type LegKind = 'take' | 'cut' | 'add';

/** A level drawn with a position: its price and the share of the entry's size it moves (half is 0.5). */
export interface Leg {
  kind: LegKind;
  price: number;
  share: number;
}

/** Coins into or out of a trade: at its entry, a leg, the stop, the target, or by hand. */
export interface Fill {
  kind: 'entry' | 'add' | 'take' | 'cut' | 'stop' | 'target' | 'hand';
  price: number;
  qty: number;
  at: number;
  /** The leg it filled, by its place in `legs`. */
  leg?: number;
}

export interface Trade {
  id: number;
  /** The position drawing it follows; null once that is deleted, or for one added by hand. */
  drawingId: number | null;
  /** Whether the server reads this market's minutes, so price can decide the trade. */
  tracked: boolean;
  market: string;
  side: Side;
  timeframe: string;
  startAt: number;
  entry: number;
  stop: number;
  target: number;
  /** What the position risks, in the quote. */
  risk: number;
  /** The coins that risk buys. */
  qty: number;
  status: Status;
  openedAt: number | null;
  closedAt: number | null;
  exit: number | null;
  fees: number;
  /** Price decides the status; false once one was set by hand. */
  auto: boolean;
  strategy: string;
  notes: string;
  createdAt: number;
  updatedAt: number;
  legs: Leg[];
  /** What price, or the hand, has filled so far; empty for a trade that kept none. */
  fills: Fill[];
}

export interface Review {
  week: string;
  answers: Partial<Record<AnswerKey, string>>;
  savedAt: number;
}

export interface NewTrade {
  market: string;
  side: Side;
  entry: number;
  stop: number;
  target: number;
  risk?: number;
  startAt?: number;
  status?: Status;
  exit?: number;
  closedAt?: number;
  strategy?: string;
  notes?: string;
}

export type TradePatch = Partial<Pick<Trade, 'strategy' | 'notes' | 'fees' | 'risk' | 'entry' | 'stop' | 'target'>> & { status?: 'closed' | 'breakeven' | 'cancelled'; exit?: number; closedAt?: number; auto?: true };

/** The list last read, and the tag the server gave it. */
let held: { tag: string; trades: Trade[] } | null = null;

/** Every trade, newest first: the array read last time while none has changed
 *  (the server answers the tag it gave 304), so nothing is drawn again. */
async function trades(): Promise<Trade[]> {
  const headers = authHeaders();
  if (held) headers['if-none-match'] = held.tag;
  const res = await fetch('/api/journal', { headers }).catch(() => {
    throw new ApiError(0, 'the tradoor server is not reachable');
  });
  if (res.status === 304 && held) return held.trades;
  const body = (await res.json().catch(() => null)) as { trades?: Trade[]; error?: unknown } | null;
  if (isSignedOut(res.status, body)) startOver(SIGNED_OUT);
  if (!res.ok || !body?.trades) throw new ApiError(res.status, typeof body?.error === 'string' ? body.error : `request failed (${res.status})`);
  const tag = res.headers.get('etag');
  held = tag ? { tag, trades: body.trades } : null;
  return body.trades;
}

export const journalApi = {
  trades,
  create: (t: NewTrade) => json<{ trade: Trade }>('/api/journal', { method: 'POST', body: t }).then((r) => r.trade),
  update: (id: number, patch: TradePatch) => json<{ trade: Trade }>(`/api/journal/${id}`, { method: 'PATCH', body: patch }).then((r) => r.trade),
  remove: (id: number) => json<{ id: number }>(`/api/journal/${id}`, { method: 'DELETE' }),
  reviews: () => json<{ reviews: Review[] }>('/api/journal/reviews').then((r) => r.reviews),
  saveReview: (week: string, answers: Review['answers']) => json<{ week: string; savedAt: number }>(`/api/journal/reviews/${week}`, { method: 'PUT', body: { answers } }),
};

// ---------------------------------------------------------------- one trade

const DONE: ReadonlySet<Status> = new Set(['win', 'loss', 'closed', 'breakeven']);
/** Played out, with an exit to weigh it by. */
export const isClosed = (t: Trade): boolean => DONE.has(t.status) && t.exit !== null;
export const isLive = (t: Trade): boolean => t.status === 'planned' || t.status === 'open';
const way = (t: Trade): number => (t.side === 'long' ? 1 : -1);

/** A trade's coins in and out: what is held, at what average entry, and what those out made. */
export interface Book {
  bought: number;
  sold: number;
  held: number;
  avgEntry: number;
  /** What the coins out made against the average entry. */
  realized: number;
}

export function book(t: Pick<Trade, 'side' | 'fills'>): Book {
  let bought = 0;
  let cost = 0;
  let sold = 0;
  let proceeds = 0;
  for (const f of t.fills) {
    if (f.kind === 'entry' || f.kind === 'add') {
      bought += f.qty;
      cost += f.price * f.qty;
    } else {
      sold += f.qty;
      proceeds += f.price * f.qty;
    }
  }
  const avgEntry = bought > 0 ? cost / bought : 0;
  return { bought, sold, held: Math.max(0, bought - sold), avgEntry, realized: (proceeds - avgEntry * sold) * (t.side === 'long' ? 1 : -1) };
}

/** What the trade made, in the quote, fees off; null until it is closed. From its fills, or a
 *  trade that kept none, from its exit. */
export function pnl(t: Trade): number | null {
  if (!isClosed(t)) return null;
  if (t.fills.some((f) => f.kind !== 'entry' && f.kind !== 'add')) return book(t).realized - t.fees;
  return (t.exit! - t.entry) * way(t) * t.qty - t.fees;
}

/** How a trade stands, for its colour: won, lost, even, or still to play out. */
export type Outcome = 'win' | 'loss' | 'even' | 'open' | 'planned' | 'cancelled';

export function outcome(t: Trade): Outcome {
  if (t.status === 'planned' || t.status === 'open' || t.status === 'cancelled') return t.status;
  const p = pnl(t);
  return t.status === 'breakeven' || p === null || p === 0 ? 'even' : p > 0 ? 'win' : 'loss';
}

/** What it made in units of what it risked. */
export function rMultiple(t: Trade): number | null {
  const p = pnl(t);
  return p === null || !(t.risk > 0) ? null : p / t.risk;
}

/** The reward the plan aims for over what it risks. */
export const plannedRR = (t: Trade): number => Math.abs(t.target - t.entry) / Math.abs(t.entry - t.stop);

/** What it makes now, at `price`, while open: what was taken off, and what is held. */
export function openPnl(t: Trade, price: number): number {
  if (t.fills.length === 0) return (price - t.entry) * way(t) * t.qty;
  const b = book(t);
  return b.realized + (price - b.avgEntry) * way(t) * b.held;
}

/** When it counts: closed, else opened, else drawn. */
export const when = (t: Trade): number => t.closedAt ?? t.openedAt ?? t.startAt;

// ---------------------------------------------------------------- many trades

export interface Stats {
  total: number;
  closed: number;
  wins: number;
  losses: number;
  breakEven: number;
  open: number;
  planned: number;
  longs: number;
  shorts: number;
  totalPnl: number;
  totalR: number;
  avgR: number;
  winRate: number;
  grossProfit: number;
  grossLoss: number;
  avgWin: number;
  avgLoss: number;
  expectancy: number;
  /** Null while nothing has lost: there is no ratio to speak of. */
  profitFactor: number | null;
  best: number;
  worst: number;
  maxDrawdown: number;
  streak: number;
  streakKind: 'win' | 'loss' | null;
  /** Hours from opened to closed, on average. */
  avgHoldHours: number | null;
}

export function stats(trades: readonly Trade[]): Stats {
  const closed = trades.filter(isClosed);
  const pnls = closed.map((t) => pnl(t)!);
  const wins = pnls.filter((p) => p > 0);
  const losses = pnls.filter((p) => p < 0);
  const rs = closed.map(rMultiple).filter((r): r is number => r !== null);
  const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);
  const grossProfit = sum(wins);
  const grossLoss = Math.abs(sum(losses));
  const totalPnl = sum(pnls);
  // The equity curve in the order trades closed: its deepest fall from a high.
  const byClose = [...closed].sort((a, b) => when(a) - when(b));
  let running = 0;
  let peak = 0;
  let maxDrawdown = 0;
  for (const t of byClose) {
    running += pnl(t)!;
    peak = Math.max(peak, running);
    maxDrawdown = Math.max(maxDrawdown, peak - running);
  }
  // The current run of wins or losses, back from the latest close.
  let streak = 0;
  let streakKind: 'win' | 'loss' | null = null;
  for (let i = byClose.length - 1; i >= 0; i--) {
    const p = pnl(byClose[i]!)!;
    const kind = p > 0 ? 'win' : p < 0 ? 'loss' : null;
    if (kind === null || (streakKind !== null && kind !== streakKind)) break;
    streakKind = kind;
    streak++;
  }
  const holds = closed.filter((t) => t.openedAt !== null && t.closedAt !== null).map((t) => (t.closedAt! - t.openedAt!) / 3_600_000);
  return {
    total: trades.length,
    closed: closed.length,
    wins: wins.length,
    losses: losses.length,
    breakEven: closed.length - wins.length - losses.length,
    open: trades.filter((t) => t.status === 'open').length,
    planned: trades.filter((t) => t.status === 'planned').length,
    longs: trades.filter((t) => t.side === 'long').length,
    shorts: trades.filter((t) => t.side === 'short').length,
    totalPnl,
    totalR: sum(rs),
    avgR: rs.length > 0 ? sum(rs) / rs.length : 0,
    winRate: closed.length > 0 ? (wins.length / closed.length) * 100 : 0,
    grossProfit,
    grossLoss,
    avgWin: wins.length > 0 ? grossProfit / wins.length : 0,
    avgLoss: losses.length > 0 ? grossLoss / losses.length : 0,
    expectancy: closed.length > 0 ? totalPnl / closed.length : 0,
    profitFactor: grossLoss > 0 ? grossProfit / grossLoss : null,
    best: pnls.length > 0 ? Math.max(...pnls) : 0,
    worst: pnls.length > 0 ? Math.min(...pnls) : 0,
    maxDrawdown,
    streak,
    streakKind,
    avgHoldHours: holds.length > 0 ? sum(holds) / holds.length : null,
  };
}

export interface Group {
  key: string;
  trades: number;
  wins: number;
  winRate: number;
  pnl: number;
  totalR: number;
}

/** Closed trades by some key of theirs — the pair, the side, the strategy — the most made first. */
export function groupBy(trades: readonly Trade[], key: (t: Trade) => string): Group[] {
  const out = new Map<string, Group>();
  for (const t of trades.filter(isClosed)) {
    const k = key(t) || '—';
    const g = out.get(k) ?? { key: k, trades: 0, wins: 0, winRate: 0, pnl: 0, totalR: 0 };
    g.trades++;
    g.pnl += pnl(t)!;
    g.totalR += rMultiple(t) ?? 0;
    if (pnl(t)! > 0) g.wins++;
    out.set(k, g);
  }
  return [...out.values()].map((g) => ({ ...g, winRate: (g.wins / g.trades) * 100 })).sort((a, b) => b.pnl - a.pnl);
}

/** `2026-10-19` in this browser's own day. */
export function localDay(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** Each local day's closed trades: what they made, in money and in R (a trade with no risk set adds none), and how many. */
export function dailyPnl(trades: readonly Trade[]): Map<string, { pnl: number; r: number; trades: number }> {
  const out = new Map<string, { pnl: number; r: number; trades: number }>();
  for (const t of trades.filter(isClosed)) {
    const day = localDay(when(t));
    const d = out.get(day) ?? { pnl: 0, r: 0, trades: 0 };
    d.pnl += pnl(t)!;
    d.r += rMultiple(t) ?? 0;
    d.trades++;
    out.set(day, d);
  }
  return out;
}

/** The running total of closed trades, in the order they closed. */
export function equityCurve(trades: readonly Trade[]): { at: number; total: number }[] {
  let total = 0;
  return trades
    .filter(isClosed)
    .sort((a, b) => when(a) - when(b))
    .map((t) => ({ at: when(t), total: (total += pnl(t)!) }));
}

// ---------------------------------------------------------------- weeks

export interface Week {
  key: string;
  /** Monday's and Sunday's local days. */
  start: string;
  end: string;
}

/** The ISO week a moment falls in, by this browser's days. */
export function weekOf(ms: number): Week {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  const monday = new Date(d);
  monday.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  const sunday = new Date(monday);
  sunday.setDate(monday.getDate() + 6);
  const thursday = new Date(monday);
  thursday.setDate(monday.getDate() + 3);
  const year = thursday.getFullYear();
  const n = Math.ceil(((thursday.getTime() - new Date(year, 0, 1).getTime()) / 86_400_000 + 1) / 7);
  return { key: `${year}-W${String(n).padStart(2, '0')}`, start: localDay(monday.getTime()), end: localDay(sunday.getTime()) };
}

/** The last `n` weeks, oldest first, this one last. */
export function lastWeeks(n: number, now = Date.now()): Week[] {
  return Array.from({ length: n }, (_, i) => weekOf(now - (n - 1 - i) * 7 * 86_400_000));
}

/** The trades of a week, by the day each counts on. */
export const inWeek = (trades: readonly Trade[], w: Week): Trade[] => trades.filter((t) => {
  const day = localDay(when(t));
  return day >= w.start && day <= w.end;
});

export type AnswerKey = 'q1' | 'q2' | 'q3' | 'q4' | 'q5' | 'q6' | 'q7' | 'q8' | 'q9';

/** rsalianto's weekly review, its nine questions as they are. */
export const QUESTIONS: { id: AnswerKey; badge: string; question: string; prompt: string }[] = [
  { id: 'q1', badge: 'Plan', question: 'What were my 2 main goals this week, and did I hit them?', prompt: 'If not — what specifically got in the way?' },
  { id: 'q2', badge: 'Win', question: 'What did I execute correctly this week — even if the trade lost?', prompt: 'Name one good habit or decision that followed my process.' },
  { id: 'q3', badge: 'Struggle', question: 'Where did I fumble, hesitate, or react emotionally?', prompt: "Is this the same pattern as last week? If yes, it's a habit — name it." },
  { id: 'q4', badge: 'Discipline', question: 'Did I break my trade plan, risk rules, or take more trades than planned?', prompt: 'What triggered it — boredom, FOMO, revenge, or something else?' },
  { id: 'q5', badge: 'Damage', question: 'What did those mistakes cost me — in dollars and in mental energy?', prompt: "Put a number on it. Vague regret doesn't change behavior." },
  { id: 'q6', badge: 'Risk', question: 'Did I respect my max loss limit and position sizing all week?', prompt: 'Did any single trade dictate the tone of my whole week? Should it have?' },
  { id: 'q7', badge: 'Market', question: 'Was there a move I missed, mistimed, or misread this week?', prompt: 'What does that tell me about my read on market conditions right now?' },
  { id: 'q8', badge: 'Conditions', question: "Was this week's market favorable or hostile for my trading style?", prompt: 'Did my results reflect my skill, or just the environment? Be honest.' },
  { id: 'q9', badge: 'Fix', question: 'What is the one concrete thing I will do differently on Monday?', prompt: 'One thing only. Be specific — not "trade better", but a precise rule.' },
];

export const STRATEGIES = ['Breakout', 'Trend Following', 'Mean Reversion', 'Support/Resistance', 'EMA Crossover', 'RSI Divergence', 'VWAP Bounce', 'News Play', 'Scalp', 'Swing', 'Three Drives'];

// ---------------------------------------------------------------- words and numbers

export const STATUS_LABEL: Record<Status, string> = {
  planned: 'Planned',
  open: 'Open',
  win: 'Target',
  loss: 'Stopped',
  closed: 'Closed',
  breakeven: 'Break-even',
  cancelled: 'Cancelled',
};

/** `+$1,234.50`, `−$12.00`. */
export function money(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '—';
  const sign = v > 0 ? '+' : v < 0 ? '−' : '';
  return `${sign}$${Math.abs(v).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** `+1.25R`. */
export function rText(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '—';
  return `${v > 0 ? '+' : v < 0 ? '−' : ''}${Math.abs(v).toFixed(2)}R`;
}

/** What each fill was. */
export const FILL_LABEL: Record<Fill['kind'], string> = {
  entry: 'Entry',
  add: 'Added',
  take: 'Took profit',
  cut: 'Cut loss',
  stop: 'Stopped',
  target: 'Target',
  hand: 'Closed by hand',
};

/** `2026-10-19T14:05` for a datetime-local input, in this browser's time. */
export function localInput(ms: number): string {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
}

/** `10-19 14:05` in this browser's own time. */
export function stamp(ms: number | null): string {
  if (ms === null) return '—';
  const d = new Date(ms);
  return `${localDay(ms).slice(5)} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}
