/**
 * A perpetual's funding history, for a backtest to pay as a position is held:
 * when each settlement was and its rate, positive where longs pay. Read from
 * the exchange that runs the market, which keeps it: Binance, Bybit and
 * Hyperliquid for as far back as a chart goes, OKX for its last three months,
 * Bitget a page at a time. Another exchange's is not read, and the report says so.
 * Read by the page, for the strategy tester; through the tradoor server where
 * the network blocks the exchange, as the chart's candles are.
 */
import { parseMarket } from './markets';
import { nativeOf } from './native';

/** The exchange's answer, or the tradoor server's relay of it where the network will not reach the exchange. */
async function getJson(url: string): Promise<unknown> {
  let res: Response;
  try {
    res = await fetch(url);
  } catch {
    res = await fetch(`/api/relay/rest?url=${encodeURIComponent(url)}`);
  }
  if (!res.ok) throw new Error(`${new URL(url).host} answered ${res.status}`);
  return res.json();
}

export interface FundingRates {
  time: Float64Array;
  rate: Float64Array;
}

/** Pages read at most for one market: two years of Binance's, at three settlements a day. */
const MOST_PAGES = 30;

const num = (v: unknown): number => (typeof v === 'number' ? v : Number(v));

/** Whether funding is read for the market: a perpetual on one of the exchanges above. */
export function fundingKnown(market: string): boolean {
  const venue = parseMarket(market)?.venue;
  return venue === 'binance-perp' || venue === 'bybit-perp' || venue === 'okx-perp' || venue === 'hyperliquid-perp' || venue === 'bitget-perp';
}

async function postInfo(body: unknown): Promise<unknown> {
  const res = await fetch('https://api.hyperliquid.xyz/info', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (!res.ok) throw new Error(`api.hyperliquid.xyz answered ${res.status}`);
  return res.json();
}

/** Settlements from `from` to `to`, oldest first; null where the market's exchange is not read. */
export async function fundingRates(market: string, from: number, to: number): Promise<FundingRates | null> {
  const ref = parseMarket(market);
  if (!ref || !fundingKnown(market)) return null;
  const rows: [number, number][] = [];
  const add = (t: unknown, r: unknown) => {
    const time = num(t);
    const rate = num(r);
    if (Number.isFinite(time) && Number.isFinite(rate) && time >= from && time <= to) rows.push([time, rate]);
  };
  switch (ref.venue) {
    case 'binance-perp': {
      let start = from;
      for (let page = 0; page < MOST_PAGES && start <= to; page++) {
        const body = (await getJson(`https://fapi.binance.com/fapi/v1/fundingRate?symbol=${ref.symbol}&startTime=${start}&endTime=${to}&limit=1000`)) as { fundingTime: number; fundingRate: string }[];
        if (!Array.isArray(body) || body.length === 0) break;
        for (const f of body) add(f.fundingTime, f.fundingRate);
        start = num(body[body.length - 1]!.fundingTime) + 1;
        if (body.length < 1000) break;
      }
      break;
    }
    case 'bybit-perp': {
      let end = to;
      for (let page = 0; page < MOST_PAGES && end >= from; page++) {
        const body = (await getJson(`https://api.bybit.com/v5/market/funding/history?category=linear&symbol=${ref.symbol}&startTime=${from}&endTime=${end}&limit=200`)) as { retCode?: number; result?: { list?: { fundingRate: string; fundingRateTimestamp: string }[] } };
        const list = body.retCode === 0 ? (body.result?.list ?? []) : [];
        if (list.length === 0) break;
        for (const f of list) add(f.fundingRateTimestamp, f.fundingRate);
        end = Math.min(...list.map((f) => num(f.fundingRateTimestamp))) - 1;
        if (list.length < 200) break;
      }
      break;
    }
    case 'okx-perp': {
      const inst = await nativeOf(market);
      let after = to + 1;
      for (let page = 0; page < MOST_PAGES; page++) {
        const body = (await getJson(`https://www.okx.com/api/v5/public/funding-rate-history?instId=${encodeURIComponent(inst)}&after=${after}&limit=100`)) as { code?: string; data?: { fundingTime: string; realizedRate?: string; fundingRate: string }[] };
        const list = body.code === '0' ? (body.data ?? []) : [];
        if (list.length === 0) break;
        for (const f of list) add(f.fundingTime, f.realizedRate || f.fundingRate);
        after = Math.min(...list.map((f) => num(f.fundingTime)));
        if (after <= from || list.length < 100) break;
      }
      break;
    }
    case 'hyperliquid-perp': {
      const coin = await nativeOf(market);
      let start = from;
      for (let page = 0; page < MOST_PAGES * 3 && start <= to; page++) {
        const body = (await postInfo({ type: 'fundingHistory', coin, startTime: start, endTime: to })) as { time: number; fundingRate: string }[];
        if (!Array.isArray(body) || body.length === 0) break;
        for (const f of body) add(f.time, f.fundingRate);
        start = num(body[body.length - 1]!.time) + 1;
        if (body.length < 500) break;
      }
      break;
    }
    case 'bitget-perp': {
      const symbol = await nativeOf(market);
      for (let page = 1; page <= MOST_PAGES; page++) {
        const body = (await getJson(`https://api.bitget.com/api/v2/mix/market/history-fund-rate?symbol=${symbol}&productType=usdt-futures&pageSize=100&pageNo=${page}`)) as { code?: string; data?: { fundingRate: string; fundingTime: string }[] };
        const list = body.code === '00000' ? (body.data ?? []) : [];
        if (list.length === 0) break;
        for (const f of list) add(f.fundingTime, f.fundingRate);
        if (Math.min(...list.map((f) => num(f.fundingTime))) <= from || list.length < 100) break;
      }
      break;
    }
  }
  rows.sort((a, b) => a[0] - b[0]);
  // One a moment: pages that meet may both hold it.
  const kept = rows.filter((r, i) => i === 0 || r[0] !== rows[i - 1]![0]);
  return { time: Float64Array.from(kept, (r) => r[0]), rate: Float64Array.from(kept, (r) => r[1]) };
}
