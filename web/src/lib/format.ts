/** Number and time formatting. Prices span 0.00000285 to 100,000 in one list,
 *  so precision comes from the value or the instrument, never a constant. */

export const MAX_PRICE_DECIMALS = 8;

/** Decimals a number actually carries, including below 1e-6 where JavaScript
 *  switches to exponent notation. */
export function decimalsOf(v: number): number {
  if (!Number.isFinite(v)) return 0;
  const s = String(Math.abs(v));
  const e = s.indexOf('e');
  const mantissa = e === -1 ? s : s.slice(0, e);
  const dot = mantissa.indexOf('.');
  const inMantissa = dot === -1 ? 0 : mantissa.length - dot - 1;
  return e === -1 ? inMantissa : Math.max(0, inMantissa - Number(s.slice(e + 1)));
}

function bucket(abs: number): number {
  if (abs === 0 || abs >= 1000) return 2;
  if (abs >= 1) return 4;
  if (abs >= 0.01) return 5;
  if (abs >= 0.0001) return 6;
  return MAX_PRICE_DECIMALS;
}

/** Decimals for one quoted price when the instrument's precision is not known.
 *  Only for traded prices: computed values carry float noise. */
export function priceDecimals(v: number | null | undefined): number {
  if (v === null || v === undefined || !Number.isFinite(v)) return 2;
  return Math.min(MAX_PRICE_DECIMALS, Math.max(bucket(Math.abs(v)), decimalsOf(v)));
}

/** Decimals for a price worked out or dragged rather than quoted, where the
 *  instrument's precision is not known: by its size alone, its own digits being
 *  an average's float tail (80311.69999999998) or where a pointer let go. */
export const roughDecimals = (v: number): number => (Number.isFinite(v) ? bucket(Math.abs(v)) : 2);

const formatters = new Map<number, Intl.NumberFormat>();

export function formatPrice(v: number | null | undefined, digits?: number): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  const d = digits ?? priceDecimals(v);
  let f = formatters.get(d);
  if (!f) {
    f = new Intl.NumberFormat('en-US', { minimumFractionDigits: d, maximumFractionDigits: d });
    formatters.set(d, f);
  }
  return f.format(v);
}

export function formatTurnover(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  const abs = Math.abs(v);
  if (abs >= 1e12) return `${(v / 1e12).toFixed(1)}T`;
  if (abs >= 1e9) return `${(v / 1e9).toFixed(1)}B`;
  if (abs >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (abs >= 1e3) return `${(v / 1e3).toFixed(0)}K`;
  return v.toFixed(0);
}

/** A volume as TradingView's watchlist writes it: 112.35K, 1.20M, 845.3. */
export function formatVolume(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  const abs = Math.abs(v);
  for (const [n, unit] of [[1e12, 'T'], [1e9, 'B'], [1e6, 'M'], [1e3, 'K']] as const) if (abs >= n) return `${(v / n).toFixed(2)}${unit}`;
  return abs >= 100 ? v.toFixed(1) : v.toFixed(2);
}

export function formatPct(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  return `${v > 0 ? '+' : ''}${v.toFixed(2)}%`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

export function formatAge(ms: number, now: number = Date.now()): string {
  const delta = now - ms;
  if (delta < 0) return 'upcoming';
  if (delta < MINUTE) return 'just now';
  if (delta < HOUR) return `${Math.floor(delta / MINUTE)}m ago`;
  if (delta < DAY) return `${Math.floor(delta / HOUR)}h ago`;
  const days = Math.floor(delta / DAY);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  return months < 12 ? `${months}mo ago` : `${Math.floor(months / 12)}y ago`;
}
