/**
 * While the page is on screen it tells the tradoor server so, once a minute:
 * the server's owner reads how many are here now on a status page of their own
 * (bins/server presence). The call carries the browser's id and nothing else,
 * does no database work there, and is not sent from a tab out of sight.
 */
import { createSignal } from 'solid-js';
import { api } from './api';

/** How many have tradoor on screen now, as the server last said: 0 until it has. */
const [online, setOnline] = createSignal(0);
export { online };
export const noteOnline = (n: number) => Number.isFinite(n) && n > 0 && setOnline(Math.round(n));

const EVERY_MS = 60_000;
/** A tab flicked back and forth says so once in this long. */
const LEAST_MS = 30_000;
let last = 0;

function here(): void {
  if (document.visibilityState !== 'visible' || Date.now() - last < LEAST_MS) return;
  last = Date.now();
  void api.here().then((r) => noteOnline(r.online), () => undefined);
}

export function tellPresence(): void {
  here();
  setInterval(here, EVERY_MS);
  document.addEventListener('visibilitychange', here);
}
