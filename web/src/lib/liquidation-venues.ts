/**
 * One ticker's liquidations come from every exchange that lists it. BTCUSDT on
 * a Binance chart gathers Binance's BTCUSDT perpetual and Bybit's: the coin and
 * the quote decide, not the venue. Spot charts get their ticker's perpetuals,
 * since spot itself has no positions to close out.
 */
import type { Product } from './api';
import { parseMarket, type Venue } from './markets';

/** Perpetual venues the page streams liquidations from itself, when the
 *  server does not follow the ticker. The server's feed covers every other. */
export const PERP_VENUES: readonly Venue[] = ['binance-perp', 'bybit-perp'];

export interface LiquidationVenue {
  /** A chart venue, or any exchange the server's feed names. */
  venue: Venue | string;
  /** The ticker's perpetual there, or null when that exchange does not list it. */
  market: string | null;
}

/**
 * The coin behind a base, with the multiplier some venues put before a cheap
 * coin's name and others after it made the same: Binance's `1000SHIB` and
 * Bybit's `SHIB1000` are one coin. `1INCH` keeps its 1 — only powers of ten
 * of a hundred and up count as a multiplier.
 */
export function coinOf(base: string): string {
  const before = /^(10{2,})([A-Z].*)$/.exec(base);
  if (before) return `${before[2]}x${before[1]}`;
  const after = /^(.*[A-Z])(10{2,})$/.exec(base);
  return after ? `${after[1]}x${after[2]}` : base;
}

/**
 * The ticker the server keeps a coin's liquidations under. It gathers a coin
 * from every exchange whatever the quote, and names it with USDT: a Kraken or
 * Hyperliquid `BTCUSD` chart reads `BTCUSDT`.
 */
export const serverTicker = (symbol: string): string => `${symbol.replace(/(USDT|USDC|USD)$/, '') || symbol}USDT`;

/**
 * Where `market`'s ticker trades as a perpetual on each exchange. Without the
 * catalogue — the server down, the list not loaded — the same symbol is tried
 * everywhere: an exchange that does not list it simply never sends anything.
 */
export function liquidationVenues(market: string, products: readonly Product[] | null): LiquidationVenue[] {
  const ref = parseMarket(market);
  // An index has no positions anywhere to close out.
  if (!ref || ref.venue === 'index') return [];
  const own = products?.find((p) => p.market === market);
  if (!products || !own) return PERP_VENUES.map((venue) => ({ venue, market: `${venue}:${ref.symbol}` }));
  const coin = coinOf(own.base);
  return PERP_VENUES.map((venue) => {
    const listed = products.filter((p) => p.venue === venue && p.kind === 'perp' && p.quote === own.quote && coinOf(p.base) === coin);
    // Several contracts for one coin and quote: the one spelled like ours.
    const pick = listed.find((p) => p.symbol === ref.symbol) ?? listed[0];
    return { venue, market: pick?.market ?? null };
  });
}
