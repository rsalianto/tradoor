/** A market is `venue:SYMBOL` — the same spelling the server, the data
 *  directory and the drawings table use. BTCUSDT exists on many venues at many
 *  prices, so the venue is part of the identity. The symbol is this app's
 *  spelling; where an exchange names the market otherwise (`BTC-USDT-SWAP`,
 *  `BTC/USD`, a Lighter market number) the catalogue carries that name. */

export type Venue =
  | 'binance-spot'
  | 'binance-perp'
  | 'bybit-spot'
  | 'bybit-perp'
  | 'okx-spot'
  | 'okx-perp'
  | 'bitget-spot'
  | 'bitget-perp'
  | 'hyperliquid-perp'
  | 'kraken-spot'
  | 'kraken-perp'
  | 'lighter-perp'
  | 'gate-spot'
  | 'gate-perp'
  | 'aster-spot'
  | 'aster-perp'
  | 'htx-spot'
  | 'htx-perp'
  | 'bitfinex-spot'
  | 'bitfinex-perp'
  | 'deribit-perp'
  | 'coinbase-spot'
  | 'bitstamp-spot'
  | 'bitstamp-perp'
  /** Series the tradoor server works out: the crypto market cap and its parts, the VIX. */
  | 'index';

export const VENUES = [...['binance', 'bybit', 'okx', 'bitget', 'kraken', 'gate', 'aster', 'htx', 'bitfinex', 'bitstamp'].flatMap((e) => [`${e}-spot`, `${e}-perp`]), 'hyperliquid-perp', 'lighter-perp', 'deribit-perp', 'coinbase-spot', 'index'] as readonly Venue[];

const EXCHANGE_NAME: Record<string, string> = { binance: 'Binance', bybit: 'Bybit', okx: 'OKX', bitget: 'Bitget', hyperliquid: 'Hyperliquid', kraken: 'Kraken', lighter: 'Lighter', gate: 'Gate', aster: 'Aster', htx: 'HTX', bitfinex: 'Bitfinex', deribit: 'Deribit', coinbase: 'Coinbase', bitstamp: 'Bitstamp', index: 'Index' };

export interface MarketRef {
  venue: Venue;
  symbol: string;
}

const MARKET = /^([a-z]+(?:-(?:spot|perp))?):([A-Z0-9_]+)$/;
const KNOWN = new Set<string>(VENUES);

export function parseMarket(ref: string): MarketRef | null {
  const m = MARKET.exec(ref);
  return m && KNOWN.has(m[1]!) ? { venue: m[1] as Venue, symbol: m[2]! } : null;
}

export const isMarket = (v: unknown): v is string => typeof v === 'string' && parseMarket(v) !== null;

/**
 * One market priced in another — `bybit-perp:XPLUSDT/binance-spot:BTCUSDT`,
 * TradingView's `BYBIT:XPLUSDT.P/BINANCE:BTCUSDT` — drawn as the first's
 * candles divided by the second's. Its two markets, or null for anything else.
 */
export function ratioLegs(ref: string): [string, string] | null {
  const at = ref.indexOf('/');
  if (at < 0) return null;
  const [a, b] = [ref.slice(0, at), ref.slice(at + 1)];
  return a !== b && parseMarket(a) && parseMarket(b) ? [a, b] : null;
}

export const isRatio = (ref: string): boolean => ratioLegs(ref) !== null;

/** What a chart may show: one market, or one priced in another. */
export const isChartMarket = (v: unknown): v is string => typeof v === 'string' && (parseMarket(v) !== null || ratioLegs(v) !== null);

export const ratioOf = (a: string, b: string): string => `${a}/${b}`;

export function symbolOf(ref: string): string {
  const legs = ratioLegs(ref);
  if (legs) return `${symbolOf(legs[0])}/${symbolOf(legs[1])}`;
  const m = parseMarket(ref);
  if (!m) return ref;
  // An index is named as TradingView names it: `BTC_D` is BTC.D.
  return m.venue === 'index' ? m.symbol.replace(/_/g, '.') : m.symbol;
}

export function venueLabel(ref: string): string {
  const legs = ratioLegs(ref);
  if (legs) return `${venueLabel(legs[0])} / ${venueLabel(legs[1])}`;
  const m = parseMarket(ref);
  // `Binance` for spot, `Binance Perp` for the perpetuals.
  return m ? EXCHANGE_NAME[m.venue.split('-')[0]!] + (m.venue.endsWith('perp') ? ' Perp' : '') : '';
}

export const isPerp = (ref: string): boolean => parseMarket(ref)?.venue.endsWith('perp') ?? false;

export const DEFAULT_MARKET = 'binance-perp:BTCUSDT';
