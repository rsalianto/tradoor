/**
 * A backtest's report as the formula module writes it (crates/frost-script-wasm
 * `fx_backtest`), read into what the strategy tester shows: kept apart so the
 * optimizer's workers read it without the chart's own modules.
 */
import { TRADE_SIZE, type BacktestResult } from './protocol';

/** The report's numbers read: trades, the equity a bar, and the bars decided as stopped. */
export function resultOf(r: Float64Array, time: Float64Array, close: Float64Array, market: string, timeframe: string): { result: BacktestResult & { ok: true }; trades: Float64Array; ambiguous: Float64Array } {
  const nt = r[0]!;
  const nb = r[1]!;
  const na = r[2]!;
  const trades = r.slice(8, 8 + nt * TRADE_SIZE);
  const equity = r.slice(8 + nt * TRADE_SIZE, 8 + nt * TRADE_SIZE + nb);
  const ambiguous = r.subarray(8 + nt * TRADE_SIZE + nb, 8 + nt * TRADE_SIZE + nb + na);
  const result: BacktestResult & { ok: true } = {
    ok: true,
    market,
    timeframe,
    time: time.slice(),
    close: close.slice(),
    trades,
    equity,
    ambiguous: na,
    magnified: r[3]!,
    inner: '',
    reading: false,
    skipped: r[4]!,
    held: r[5]!,
    next: r[6]!,
    nextLimit: r[7]!,
    funding: 'off',
    settlements: 0,
  };
  return { result, trades, ambiguous };
}

