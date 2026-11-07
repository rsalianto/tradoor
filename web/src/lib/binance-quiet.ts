/**
 * Binance counts an address's calls a minute at a time and answers 429 once
 * they are spent; asked again before the minute turns, it bans the address
 * (418) for longer each time. A host that said so is left alone until then by
 * the tab or worker that heard it, and the footprint's fill, which spends the
 * most, hears of it from every chart and stops.
 */
const until = new Map<string, number>();
let channel: BroadcastChannel | null | undefined;
const told = (): BroadcastChannel | null => (channel ??= typeof BroadcastChannel === 'function' ? new BroadcastChannel('tradoor-binance') : null);

export const isBinance = (host: string): boolean => /(^|\.)binance\.(com|vision)$/.test(host);

/** `host` answered `status` (429 or 418): when it may be asked again, told everywhere. */
export function throttled(host: string, status: number): number {
  const at = Math.ceil((Date.now() + 1) / 60_000) * 60_000 + (status === 418 ? 120_000 : 2_000);
  until.set(host, Math.max(until.get(host) ?? 0, at));
  told()?.postMessage({ host, until: at });
  return at;
}

/** Until when `host` is to be left alone; 0 when it is not. */
export const quietUntil = (host: string): number => {
  const at = until.get(host) ?? 0;
  return at > Date.now() ? at : 0;
};

/** What the charts were told, heard: the fill alone listens, so the chart worker carries no more than it needs. */
export function listen(): void {
  const c = told();
  if (c && !c.onmessage) c.onmessage = (e: MessageEvent<{ host: string; until: number }>) => until.set(e.data.host, Math.max(until.get(e.data.host) ?? 0, e.data.until));
}

/** What a chart waiting on Binance fails with: `until`, when it may ask again. */
export const throttledError = (at: number): Error & { until: number } => Object.assign(new Error('Binance is limiting calls from this network'), { until: at });
