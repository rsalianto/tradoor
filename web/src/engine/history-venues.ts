/**
 * Candle loaders of the exchanges beyond Binance and Bybit, loaded with the
 * first market on one of them. Each returns `[time, open, high, low, close,
 * volume]` rows inside the range, oldest first.
 */
import type { MarketRef } from '../lib/markets';
import { intervalMs, type Range, type Row } from './history';
import { INTERVALS } from './intervals';

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

const num = (v: unknown): number => (typeof v === 'number' ? v : Number(v));
const inRange = (rows: Row[], range: Range): Row[] => rows.filter((r) => (range.start === undefined || r[0]! >= range.start) && (range.end === undefined || r[0]! <= range.end));

/** POSTed JSON; Hyperliquid's API takes nothing else and is not relayed. */
async function postJson(url: string, body: unknown): Promise<unknown> {
  for (let attempt = 0; ; attempt += 1) {
    const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
    if (res.ok) return res.json();
    if (!(res.status === 429 || res.status >= 500) || attempt >= 3) throw new Error(`${new URL(url).host} answered ${res.status}`);
    await sleep(1000 * 2 ** attempt);
  }
}

/** `[time, open, high, low, close, volume]` rows in the range, oldest first. */
export async function venueRows(ref: MarketRef, native: string, interval: string, range: Range, limit: number, getJson: (url: string) => Promise<unknown>): Promise<Row[]> {
  const name = INTERVALS[ref.venue]?.[interval] ?? interval;
  const ms = intervalMs(interval);
  const end = range.end ?? Date.now();
  switch (ref.venue) {
    case 'okx-spot':
    case 'okx-perp': {
      let url = `https://www.okx.com/api/v5/market/history-candles?instId=${encodeURIComponent(native)}&bar=${name}&limit=${limit}&after=${end + 1}`;
      if (range.start !== undefined) url += `&before=${range.start - 1}`;
      const body = (await getJson(url)) as { code?: string; msg?: string; data?: string[][] };
      if (body.code !== '0') throw new Error(body.msg || 'OKX refused the candle request');
      // Newest first. A swap's volume is in contracts; `volCcy` is the coins.
      const volume = ref.venue === 'okx-perp' ? 6 : 5;
      return (body.data ?? []).map((r) => [num(r[0]), num(r[1]), num(r[2]), num(r[3]), num(r[4]), num(r[volume])]).reverse();
    }
    case 'bitget-spot':
    case 'bitget-perp': {
      const path = ref.venue === 'bitget-perp' ? `mix/market/history-candles?productType=usdt-futures&` : 'spot/market/history-candles?';
      const url = `https://api.bitget.com/api/v2/${path}symbol=${native}&granularity=${name}&limit=${limit}&endTime=${end}`;
      const body = (await getJson(url)) as { code?: string; msg?: string; data?: string[][] };
      if (body.code !== '00000') throw new Error(body.msg || 'Bitget refused the candle request');
      return inRange((body.data ?? []).map((r) => [num(r[0]), num(r[1]), num(r[2]), num(r[3]), num(r[4]), num(r[5])]), range);
    }
    case 'hyperliquid-perp': {
      const startTime = range.start ?? end - limit * ms;
      const body = (await postJson('https://api.hyperliquid.xyz/info', { type: 'candleSnapshot', req: { coin: native, interval: name, startTime, endTime: end } })) as { t: number; o: string; h: string; l: string; c: string; v: string }[];
      if (!Array.isArray(body)) throw new Error('Hyperliquid sent an unexpected candle response');
      return inRange(body.map((k) => [k.t, num(k.o), num(k.h), num(k.l), num(k.c), num(k.v)]), range);
    }
    case 'kraken-spot': {
      // Only the latest 720 bars are served: paging back past them finds nothing.
      let url = `https://api.kraken.com/0/public/OHLC?pair=${encodeURIComponent(native)}&interval=${name}`;
      if (range.start !== undefined) url += `&since=${Math.floor(range.start / 1000) - 1}`;
      const body = (await getJson(url)) as { error?: string[]; result?: Record<string, unknown> };
      if (body.error?.length) throw new Error(body.error[0]);
      const rows = Object.entries(body.result ?? {}).find(([k]) => k !== 'last')?.[1] as (string | number)[][] | undefined;
      return inRange((rows ?? []).map((r) => [num(r[0]) * 1000, num(r[1]), num(r[2]), num(r[3]), num(r[4]), num(r[6])]), range);
    }
    case 'kraken-perp': {
      const from = range.start ?? end - limit * ms;
      const url = `https://futures.kraken.com/api/charts/v1/trade/${native}/${name}?from=${Math.floor(from / 1000)}&to=${Math.ceil(end / 1000)}`;
      const body = (await getJson(url)) as { candles?: { time: number; open: string; high: string; low: string; close: string; volume: string }[] };
      return inRange((body.candles ?? []).map((k) => [k.time, num(k.open), num(k.high), num(k.low), num(k.close), num(k.volume)]), range);
    }
    case 'lighter-perp': {
      const from = range.start ?? end - limit * ms;
      const url = `https://mainnet.zklighter.elliot.ai/api/v1/candles?market_id=${native}&resolution=${name}&start_timestamp=${from}&end_timestamp=${end}&count_back=${limit}`;
      const body = (await getJson(url)) as { code?: number; message?: string; c?: { t: number; o: number; h: number; l: number; c: number; v: number }[] };
      if (body.code !== 200) throw new Error(body.message || 'Lighter refused the candle request');
      return inRange((body.c ?? []).map((k) => [k.t, k.o, k.h, k.l, k.c, k.v]), range);
    }
    case 'index': {
      // Worked out by the tradoor server, and asked of it: never relayed. The
      // latest bars are asked for without an end, so every chart asks alike
      // and the server's kept answer serves them all.
      let url = `/api/index/candles?symbol=${ref.symbol}&interval=${interval}&limit=${limit}`;
      if (range.start !== undefined) url += `&start=${range.start}`;
      if (range.end !== undefined) url += `&end=${range.end}`;
      const res = await fetch(url);
      const body = (await res.json()) as Row[] | { error?: string };
      if (!Array.isArray(body)) throw new Error(body.error || `the index answered ${res.status}`);
      return body;
    }
    default: {
      // The newer exchanges' loaders are in a file of their own.
      const { loadVenues } = await import('./venue-load');
      return (await loadVenues()).rows(ref, native, interval, range, limit, getJson, ms);
    }
  }
}

