/** Bar lengths each exchange serves, shared by the REST loader and the streams. */
import type { Venue } from '../lib/markets';

/**
 * Each exchange's name for a bar length, where it has one. A length it lacks
 * is folded from a shorter one it has (see `restInterval`). Weekly bars are
 * taken only where they start on Monday, as the chart's weeks do — Kraken's
 * and Hyperliquid's start on Thursday, so theirs are folded from days.
 */
export const INTERVALS: Partial<Record<Venue, Record<string, string>>> = {
  'okx-spot': { '1m': '1m', '5m': '5m', '15m': '15m', '30m': '30m', '1h': '1H', '2h': '2H', '4h': '4H', '6h': '6Hutc', '12h': '12Hutc', '1d': '1Dutc', '1w': '1Wutc', '1M': '1Mutc' },
  'bitget-spot': { '1m': '1min', '5m': '5min', '15m': '15min', '30m': '30min', '1h': '1h', '4h': '4h', '6h': '6Hutc', '12h': '12Hutc', '1d': '1Dutc', '1w': '1Wutc', '1M': '1Mutc' },
  'bitget-perp': { '1m': '1m', '5m': '5m', '15m': '15m', '30m': '30m', '1h': '1H', '4h': '4H', '6h': '6Hutc', '12h': '12Hutc', '1d': '1Dutc', '1w': '1Wutc', '1M': '1Mutc' },
  'hyperliquid-perp': { '1m': '1m', '5m': '5m', '15m': '15m', '30m': '30m', '1h': '1h', '2h': '2h', '4h': '4h', '12h': '12h', '1d': '1d', '1M': '1M' },
  'kraken-spot': { '1m': '1', '5m': '5', '15m': '15', '30m': '30', '1h': '60', '4h': '240', '1d': '1440' },
  'kraken-perp': { '1m': '1m', '5m': '5m', '15m': '15m', '30m': '30m', '1h': '1h', '4h': '4h', '12h': '12h', '1d': '1d' },
  'lighter-perp': { '1m': '1m', '5m': '5m', '15m': '15m', '30m': '30m', '1h': '1h', '4h': '4h', '12h': '12h', '1d': '1d' },
};
INTERVALS['okx-perp'] = INTERVALS['okx-spot'];

/** What the newer exchanges lack, or start somewhere other than on the chart's boundaries (their names are in venue-pack). */
const LONG = ['6h', '12h', '1d', '1w', '1M'];
const LACKS: Record<string, string[]> = { gate: ['1M'], htx: ['2h', ...LONG], bitfinex: ['2h'], deribit: ['4h', ...LONG], coinbase: ['30m', '2h', '4h', '12h', '1w', '1M'], bitstamp: ['1w', '1M'] };

/** Shorter lengths a missing one is folded from, best first; each divides it on UTC boundaries. */
const DAY = ['4h', '2h', '1h'];
const FALLBACK: Record<string, string[]> = { '30m': ['15m'], '2h': ['1h'], '4h': ['2h', '1h'], '6h': ['2h', '1h'], '12h': ['6h', ...DAY], '1d': DAY, '1w': ['1d', ...DAY], '1M': ['1d', ...DAY] };

const serves = (venue: Venue, interval: string): boolean => {
  const names = INTERVALS[venue];
  return names ? !!names[interval] : !LACKS[venue.split('-')[0]!]?.includes(interval);
};

/** The length to ask `venue` for when the chart wants `interval`. */
export function restInterval(venue: Venue, interval: string): string {
  if (serves(venue, interval)) return interval;
  return FALLBACK[interval]?.find((i) => serves(venue, i)) ?? interval;
}

/** The lengths Bitget names by the UTC day, week or month. */
const UTC_NAMED = new Set(['6h', '12h', '1d', '1w', '1M']);
/**
 * Markets that refuse those names — Bitget's stock tokens ("rMU" and the
 * like) have no UTC day, week or month, and the days it does give them start
 * at 16:00 UTC — as found the first time one is asked (`history.ts`).
 */
const LACKS_UTC = new Set<string>();

export function lacksUtc(market: string): void {
  LACKS_UTC.add(market);
}

/** One of those markets: a stock token, whose candles also stop between its trading hours. */
export const lacksUtcOf = (market: string): boolean => LACKS_UTC.has(market);

/** As `restInterval`, for one market: one without UTC lengths reads them as 4h bars (1h for 6h), folded on UTC boundaries. */
export function marketInterval(market: string, venue: Venue, interval: string): string {
  if (LACKS_UTC.has(market) && UTC_NAMED.has(interval)) return interval === '6h' ? '1h' : '4h';
  return restInterval(venue, interval);
}
