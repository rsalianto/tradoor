/**
 * Training: paper trading on Binance's BTC, ETH and SOL perpetuals at their
 * live prices, the account kept by the server (bins/server/src/training.rs) —
 * one position a coin, isolated margin, Binance's fees, liquidation and
 * funding. What the page adds: the account's worth at the price it sees, the
 * positions drawn on the chart, and the statistics, read with the journal's.
 */
import type { DrawingPoint } from '../engine/protocol';
import { json } from '../lib/api';
import type { Style } from '../lib/drawing-style';
import type { Trade } from '../journal/model';

export const TRAINING_MARKETS = ['binance-perp:BTCUSDT', 'binance-perp:ETHUSDT', 'binance-perp:SOLUSDT'] as const;
export const symbolOfMarket = (market: string): string => market.split(':')[1] ?? market;
export const marketOfSymbol = (symbol: string): string => `binance-perp:${symbol}`;
/** `BTC` for `BTCUSDT`. */
export const coinOf = (symbol: string): string => symbol.replace(/USDT$/, '');

export type Side = 'long' | 'short';

export interface Quote {
  last: number;
  mark: number;
  /** The rate the next settlement takes, and when that is. */
  funding: number;
  nextFunding: number;
  at: number;
}

export interface Position {
  symbol: string;
  side: Side;
  qty: number;
  entry: number;
  leverage: number;
  margin: number;
  tp: number | null;
  sl: number | null;
  liquidation: number;
  openedAt: number;
  /** What its partial closes made so far, and the fees and funding it has paid (negative: taken). */
  realized: number;
  fees: number;
  funding: number;
}

export interface Order {
  id: number;
  symbol: string;
  side: 'buy' | 'sell';
  price: number;
  qty: number;
  tp: number | null;
  sl: number | null;
  createdAt: number;
}

export interface State {
  run: number;
  wallet: number;
  startedAt: number;
  start: number;
  leverage: Record<string, number>;
  maxLeverage: number;
  maker: number;
  taker: number;
  mmr: Record<string, number>;
  positions: Position[];
  orders: Order[];
  quotes: Record<string, Quote>;
}

/** A position from its first fill to flat. */
export interface ClosedTrade {
  id: number;
  symbol: string;
  side: Side;
  qty: number;
  entry: number;
  exit: number;
  leverage: number;
  margin: number;
  pnl: number;
  fees: number;
  funding: number;
  /** What it made: the closes, less fees, with funding. */
  net: number;
  reason: 'tp' | 'sl' | 'close' | 'liquidation' | 'reversed';
  tp: number | null;
  sl: number | null;
  openedAt: number;
  closedAt: number;
}

export interface Run {
  run: number;
  trades: number;
  net: number;
  from: number | null;
  to: number | null;
}

export interface History {
  run: number;
  current: number;
  trades: ClosedTrade[];
  runs: Run[];
}

export interface NewOrder {
  symbol: string;
  side: 'buy' | 'sell';
  type: 'market' | 'limit';
  qty: number;
  price?: number;
  tp?: number;
  sl?: number;
}

export const trainingApi = {
  state: () => json<State>('/api/training'),
  order: (o: NewOrder) => json<{ order?: number; filled?: { price: number; qty: number }; closed?: boolean }>('/api/training/orders', { method: 'POST', body: o }),
  cancel: (id: number) => json<{ id: number }>(`/api/training/orders/${id}`, { method: 'DELETE' }),
  close: (symbol: string, qty?: number) => json<{ price: number; qty: number; closed: boolean }>(`/api/training/positions/${symbol}/close`, { method: 'POST', body: qty === undefined ? {} : { qty } }),
  reverse: (symbol: string) => json<{ price: number; qty: number; side: Side | null }>(`/api/training/positions/${symbol}/reverse`, { method: 'POST' }),
  exits: (symbol: string, exits: { tp?: number | null; sl?: number | null }) => json<{ tp: number | null; sl: number | null }>(`/api/training/positions/${symbol}`, { method: 'PATCH', body: exits }),
  leverage: (symbol: string, leverage: number) => json<{ leverage: number }>(`/api/training/leverage/${symbol}`, { method: 'PUT', body: { leverage } }),
  reset: () => json<{ run: number }>('/api/training/reset', { method: 'POST' }),
  history: (run?: number) => json<History>(`/api/training/trades${run === undefined ? '' : `?run=${run}`}`),
};

export const REASON_LABEL: Record<ClosedTrade['reason'], string> = {
  tp: 'Take profit',
  sl: 'Stop loss',
  close: 'Closed',
  liquidation: 'Liquidated',
  reversed: 'Reversed',
};

const way = (side: Side): number => (side === 'long' ? 1 : -1);

/** What a position makes at `price`, still open. */
export const unrealized = (p: Position, price: number): number => (price - p.entry) * way(p.side) * p.qty;

/** That over its margin: Binance's return on equity. */
export const roe = (p: Position, price: number): number => (p.margin > 0 ? unrealized(p, price) / p.margin : 0);

/** Where a position of `qty` at `entry` would be liquidated, at `leverage` and the coin's maintenance rate. */
export function liquidationAt(side: Side, entry: number, leverage: number, mmr: number): number {
  const at = side === 'long' ? (entry * (1 - 1 / leverage)) / (1 - mmr) : (entry * (1 + 1 / leverage)) / (1 + mmr);
  return Math.max(0, at);
}

export interface Worth {
  /** Cash, with what the open positions make now. */
  equity: number;
  unrealized: number;
  /** The margin the positions hold and the orders would take. */
  used: number;
  /** What is free to put up. */
  available: number;
}

/** An account's worth at the prices given — as the server reckons what is free to put up. */
export function worth(s: State, priceOf: (symbol: string) => number | undefined): Worth {
  let open = 0;
  let used = 0;
  for (const p of s.positions) {
    const price = priceOf(p.symbol);
    if (price !== undefined) open += unrealized(p, price);
    used += p.margin;
  }
  for (const o of s.orders) {
    const held = s.positions.find((p) => p.symbol === o.symbol);
    const against = held && (held.side === 'long') !== (o.side === 'buy') ? held.qty : 0;
    used += (Math.max(0, o.qty - against) * o.price) / (s.leverage[o.symbol] ?? 20);
  }
  return { equity: s.wallet + open, unrealized: open, used, available: s.wallet - used };
}

/**
 * A closed training trade as the journal's statistics read one: its fees set
 * so the journal's own reckoning — exit less entry, less fees — comes to what
 * it netted, adds at other prices, partial closes and funding all in; its
 * margin as the risk, so an R is its return on margin; how it ended as the
 * strategy, to group by.
 */
export function asJournal(t: ClosedTrade): Trade {
  const gross = (t.exit - t.entry) * way(t.side) * t.qty;
  return {
    id: t.id,
    drawingId: null,
    tracked: false,
    leverage: 1,
    market: marketOfSymbol(t.symbol),
    side: t.side,
    timeframe: '1m',
    startAt: t.openedAt,
    entry: t.entry,
    stop: t.entry,
    target: t.exit,
    risk: t.margin,
    qty: t.qty,
    status: 'closed',
    openedAt: t.openedAt,
    closedAt: t.closedAt,
    exit: t.exit,
    fees: gross - t.net,
    auto: false,
    strategy: REASON_LABEL[t.reason],
    notes: '',
    createdAt: t.closedAt,
    updatedAt: t.closedAt,
    legs: [],
    fills: [],
  };
}

// ---------------------------------------------------------------- on the chart

export interface Shape {
  kind: string;
  points: DrawingPoint[];
  label: string | null;
  style: Style;
}

const BOX: Style = { profitLine: '#d1d4dc', stopLine: '#f2364566', profitFill: '#d1d4dc1a', stopFill: '#f236451a', riskUnit: 'cash', lot: 1, compact: true };
/** How far an open position's box runs past now. */
const AHEAD = 20 * 60_000;

/** A position box needs a stop on one side of the entry and a target on the other: what it has, or, missing one, the other mirrored. */
function levels(side: Side, entry: number, stop: number | null, target: number | null): { stop: number; target: number } | null {
  const dir = way(side);
  const s = stop !== null && (entry - stop) * dir > 0 ? stop : null;
  const t = target !== null && (target - entry) * dir > 0 ? target : null;
  if (s !== null && t !== null) return { stop: s, target: t };
  if (s !== null) return { stop: s, target: entry + (entry - s) };
  if (t !== null) return { stop: entry - (t - entry), target: t };
  return null;
}

function box(side: Side, from: number, to: number, entry: number, stop: number, target: number, qty: number): Shape {
  return {
    kind: side,
    points: [
      { time: from, price: entry },
      { time: from, price: stop },
      { time: to, price: target },
    ],
    label: null,
    style: { ...BOX, risk: Math.abs(entry - stop) * qty },
  };
}

const MINUTE = 60_000;
/** The one-minute candle a moment falls in. */
export const candleOf = (ms: number): number => Math.floor(ms / MINUTE) * MINUTE;
const BULL = '#089981';
const BEAR = '#f23645';

/** A glyph on its own, centred on a price: a circle drawn with no ring and no fill, only its label. */
const glyph = (time: number, price: number, text: string, color: string, size: number): Shape => ({
  kind: 'circle',
  points: [{ time, price }],
  label: text,
  style: { color: '#00000000', width: 0, fillOn: false, textColor: color, textSize: size },
});

/**
 * What the chart shows of a coin's trading, clear and out of the candles' way,
 * as a strategy tester marks its trades: each trade of the run an arrow at its
 * entry — up for a long, down for a short — a thin dashed line to where it got
 * out, and a dot there, the line and dot green for a gain and red for a loss;
 * what it made, and the rest, come up over either candle (see `marked`);
 * the open position as a box running on, its stop the stop loss or else the
 * liquidation price; and each resting order as a line.
 */
export function shapes(s: State | null, trades: readonly ClosedTrade[], symbol: string, now: number): Shape[] {
  const out: Shape[] = [];
  for (const t of trades.filter((x) => x.symbol === symbol).slice(0, 60)) {
    const tone = t.net >= 0 ? BULL : BEAR;
    const entered = candleOf(t.openedAt);
    const closed = candleOf(t.closedAt);
    out.push({ kind: 'trendline', points: [{ time: entered, price: t.entry }, { time: closed, price: t.exit }], label: null, style: { color: `${tone}cc`, width: 1, dash: 'dashed' } });
    out.push(glyph(entered, t.entry, t.side === 'long' ? '▲' : '▼', t.side === 'long' ? BULL : BEAR, 14));
    out.push(glyph(closed, t.exit, '●', tone, 10));
  }
  const p = s?.positions.find((x) => x.symbol === symbol);
  if (p) {
    const at = levels(p.side, p.entry, p.sl ?? p.liquidation, p.tp);
    if (at) out.push(box(p.side, p.openedAt, now + AHEAD, p.entry, at.stop, at.target, p.qty));
  }
  for (const o of s?.orders.filter((x) => x.symbol === symbol) ?? []) {
    const label = `${o.side === 'buy' ? 'Buy' : 'Sell'} limit ${Number(o.qty.toPrecision(4))}`;
    out.push({ kind: 'trendline', points: [{ time: o.createdAt, price: o.price }, { time: now + AHEAD, price: o.price }], label, style: { color: '#ffc247', width: 1, dash: 'dashed', textColor: '#ffc247', priceLabels: true } });
  }
  return out;
}

/** The trades marked on or beside `candle` — entered or closed there — for the details over it. */
export function marked(trades: readonly ClosedTrade[], symbol: string, candle: number): ClosedTrade[] {
  const near = (ms: number) => Math.abs(candleOf(ms) - candle) <= MINUTE;
  return trades.filter((t) => t.symbol === symbol && (near(t.openedAt) || near(t.closedAt))).slice(0, 3);
}

/** `1,234.5` with the coin's decimals; `—` for nothing. */
export function price(v: number | null | undefined, decimals = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  return v.toLocaleString('en-US', { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
}

/** `$1,234.56`, a sign only when below nothing: an amount held, not a gain. */
export function usd(v: number): string {
  if (!Number.isFinite(v)) return '—';
  return `${v < 0 ? '−' : ''}$${Math.abs(v).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** `+12.34%`. */
export function percent(v: number): string {
  if (!Number.isFinite(v)) return '—';
  return `${v > 0 ? '+' : v < 0 ? '−' : ''}${Math.abs(v * 100).toFixed(2)}%`;
}

/** A coin amount, as few figures as tell it. */
export const coins = (v: number): string => String(Number(v.toPrecision(6)));
