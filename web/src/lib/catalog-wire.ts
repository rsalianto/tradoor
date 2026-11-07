/**
 * The catalogue as the server sends it (`/api/catalog`): each venue's markets
 * as rows of a few columns, a row stopping at its last value — a quarter of
 * the plain list to fetch and to parse. Read here into one object a market,
 * for the page's search and the engine's exchange names alike. Solid-free, so
 * the chart's worker reads it too.
 */
import { read } from './read';
import type { Product, VenueStatus } from './api';

/** A market as listed, with the exchange's own name for it where that is not ours. */
export type Listed = Product & { native?: string };

export interface Listing {
  updatedAt: number;
  venues: VenueStatus[];
  products: Listed[];
}

interface Wire {
  updatedAt: number;
  venues: VenueStatus[];
  columns: string[];
  markets: Record<string, unknown[][]>;
}

export function readWire(wire: Wire): Listing {
  const col = (name: string): number => wire.columns.indexOf(name);
  const [symbol, base, quote, decimals, volume, last, change, native, name] = [col('symbol'), col('base'), col('quote'), col('priceDecimals'), col('quoteVolume24h'), col('lastPrice'), col('changePct'), col('native'), col('name')] as const;
  const products: Listed[] = [];
  for (const [venue, rows] of Object.entries(wire.markets)) {
    const kind = venue === 'index' ? 'index' : venue.endsWith('perp') ? 'perp' : 'spot';
    for (const r of rows) {
      const at = <T,>(i: number): T | null => (r[i] ?? null) as T | null;
      const p: Listed = { market: `${venue}:${r[symbol]}`, venue, symbol: r[symbol] as string, base: r[base] as string, quote: r[quote] as string, kind, priceDecimals: at(decimals), quoteVolume24h: at(volume), lastPrice: at(last), changePct: at(change) };
      if (r[native] != null) p.native = r[native] as string;
      if (r[name] != null) p.name = r[name] as string;
      products.push(p);
    }
  }
  return { updatedAt: wire.updatedAt, venues: wire.venues, products };
}

let held: { at: number; value: Promise<Listing> } | null = null;

/** The catalogue, fetched and read once a half hour, shared by whoever asks in this page or worker. */
export function listing(): Promise<Listing> {
  if (held && Date.now() - held.at < 30 * 60_000) return held.value;
  const value = read('/api/catalog', {})
    .then((res) => {
      if (!res.ok) throw new Error(`the catalogue answered ${res.status}`);
      return res.json() as Promise<Wire>;
    })
    .then(readWire);
  held = { at: Date.now(), value };
  value.catch(() => {
    if (held?.value === value) held = null;
  });
  return value;
}
