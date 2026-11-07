/**
 * Binance counts an address's calls a minute at a time and answers 429 once
 * they are spent; asked again before the minute turns, it bans the address
 * (418) for longer each time. A host that said so is left alone until then by
 * every tab and worker of this browser: the charts wait, and the footprint's
 * fill, which spends the most, stops.
 */
const until = new Map<string, number>();
const channel = typeof BroadcastChannel === 'function' ? new BroadcastChannel('tradoor-binance') : null;
channel?.addEventListener('message', (e: MessageEvent<{ host: string; until: number }>) => {
  until.set(e.data.host, Math.max(until.get(e.data.host) ?? 0, e.data.until));
});

export const isBinance = (host: string): boolean => /(^|\.)binance\.(com|vision)$/.test(host);

/** `host` answered `status` (429 or 418): when it may be asked again, told everywhere. */
export function throttled(host: string, status: number): number {
  const at = Math.ceil((Date.now() + 1) / 60_000) * 60_000 + (status === 418 ? 120_000 : 2_000);
  until.set(host, Math.max(until.get(host) ?? 0, at));
  channel?.postMessage({ host, until: at });
  return at;
}

/** Until when `host` is to be left alone; 0 when it is not. */
export const quietUntil = (host: string): number => {
  const at = until.get(host) ?? 0;
  return at > Date.now() ? at : 0;
};

/** A host left alone until `until`: what a chart says while it waits. */
export class Throttled extends Error {
  constructor(readonly until: number) {
    super('Binance is limiting calls from this network');
  }
}
