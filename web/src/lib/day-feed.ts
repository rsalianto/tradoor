/**
 * The current UTC day of a market, live: its daily candle from the exchange's
 * stream. What the watchlist and the values panel read change and volume from,
 * as TradingView does — since 00:00 UTC, not over a rolling 24 hours.
 */
import { createStore, unwrap } from 'solid-js/store';
import { StreamHub, type StreamListener } from '../engine/streams';
import { parseMarket } from './markets';
import { nativeOf } from './native';
import { readPref, writePref } from './prefs';

export interface Day {
  openTime: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
}

/** Tickers are few and slow, so the page keeps its own sockets off the engine thread. */
const hub = new StreamHub();

const DAY_MS = 86_400_000;
const SAVE_KEY = 'days';
/** Days saved this recently are shown on loading; after longer their close says too little. */
const SAVED_MS = 10 * 60_000;
/** Trades come in bursts: a market's figures are redrawn this often at most. */
const TRADE_MS = 250;

function saved(): Record<string, Day> {
  try {
    const { at, days: kept } = JSON.parse(readPref(SAVE_KEY) ?? '{}') as { at?: number; days?: Record<string, Day> };
    const now = Date.now();
    if (!kept || !(now - at! < SAVED_MS)) return {};
    return Object.fromEntries(Object.entries(kept).filter(([, d]) => now - d.openTime < DAY_MS));
  } catch {
    return {};
  }
}

/**
 * The last day seen for each market, kept for the page. A panel that mounts
 * afresh — a tab switched away from and back — reads these rather than showing
 * dashes until the next trade, which on a quiet market is a long wait.
 */
const [days, setDays] = createStore<Record<string, Day>>(saved());

// Kept for the reload that follows: the figures are there before any socket is.
for (const type of ['pagehide', 'visibilitychange']) addEventListener(type, () => writePref(SAVE_KEY, JSON.stringify({ at: Date.now(), days: unwrap(days) })));

export const dayOf = (market: string): Day | undefined => days[market];

/**
 * Calls `onDay` with each update of today's candle; returns the unsubscribe.
 * `trades`: its price follows every trade too, where the exchange's candle
 * lags them. Binance's comes about once a second, at times five seconds late.
 */
export function watchDay(market: string, onDay: (day: Day) => void, trades = false): () => void {
  const ref = parseMarket(market);
  if (!ref) return () => {};
  const offs: (() => void)[] = [];
  let live = true;
  let latest = 0;
  let day: Day | undefined = days[market];
  let tradeId = -1;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const show = () => {
    clearTimeout(timer);
    timer = undefined;
    setDays(market, day!);
    onDay(day!);
  };
  const listener: StreamListener = {
    message: (msg) => {
      if (msg.kind === 'trade') {
        // Already told, or past the day held: the next candle starts that one.
        if (!day || msg.lastTradeId <= tradeId || msg.time - day.openTime >= DAY_MS) return;
        tradeId = msg.lastTradeId;
        day = { ...day, close: msg.price, high: Math.max(day.high, msg.price), low: Math.min(day.low, msg.price) };
        timer ??= setTimeout(show, TRADE_MS);
        return;
      }
      // A late update of yesterday's candle must not replace today's.
      if (msg.kind !== 'kline' || !(msg.open > 0) || msg.openTime < latest) return;
      latest = msg.openTime;
      // A candle the trades have overtaken: its volume, not its older price.
      const close = day && day.openTime === msg.openTime && msg.lastTradeId < tradeId ? day.close : msg.close;
      day = { openTime: msg.openTime, open: msg.open, high: Math.max(msg.high, close), low: Math.min(msg.low, close), close, volume: msg.volume };
      show();
    },
    state: () => {},
  };
  // The exchange's own name for it, which may take the catalogue to know.
  void nativeOf(market).then((symbol) => {
    if (!live) return;
    offs.push(hub.subscribe(ref.venue, { kind: 'kline', symbol, interval: '1d' }, listener));
    if (trades && ref.venue.startsWith('binance')) offs.push(hub.subscribe(ref.venue, { kind: 'trades', symbol }, listener));
  }, () => {});
  return () => {
    live = false;
    clearTimeout(timer);
    for (const off of offs) off();
  };
}
