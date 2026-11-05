/**
 * The indicators a reader starred. An account's are kept on the server, so they
 * follow it to every device; this browser holds a copy to show at once. Signed
 * out, the stars are this browser's alone, and signing in carries them into the
 * account, once.
 */
import { createSignal } from 'solid-js';
import { api, errorText } from './api';
import { identity, ownerId, session } from './owner';
import { readJson, writePref } from './prefs';
import { toast } from './toasts';

const KEY = 'indicatorFavourites';

interface Held {
  /** Whose: the browser's id, or a session's. */
  who: string;
  kinds: string[];
}

export function readHeld(raw: unknown): Held | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const { who, kinds } = raw as Partial<Held>;
  return typeof who === 'string' && Array.isArray(kinds) ? { who, kinds: kinds.filter((k): k is string => typeof k === 'string') } : null;
}

/** The server's list, then what was starred before signing in. */
export const mergeStars = (server: readonly string[], carried: readonly string[]): string[] => [...server, ...carried.filter((k) => !server.includes(k))];

const held = readHeld(readJson(KEY));
const [favourites, setFavourites] = createSignal<string[]>(held?.who === identity() ? held.kinds : []);
export { favourites };

function keep(kinds: string[]): void {
  setFavourites(kinds);
  writePref(KEY, JSON.stringify({ who: identity(), kinds } satisfies Held));
}

/** Changed since a read began: that read's answer is older than what is on screen. */
let changes = 0;

/** The account's stars as they are now, with what this browser starred before it signed in carried over. */
export async function loadFavourites(): Promise<void> {
  if (!session()) return;
  const began = changes;
  const before = readHeld(readJson(KEY));
  const carried = before?.who === ownerId() ? before.kinds : [];
  try {
    const { kinds } = await api.favourites();
    for (const kind of carried) if (!kinds.includes(kind)) await api.setFavourite(kind, true);
    if (changes === began) keep(mergeStars(kinds, carried));
  } catch {
    // Out of reach: the copy held here stands until the next time.
  }
}

export function toggleFavourite(kind: string): void {
  const was = favourites();
  const on = !was.includes(kind);
  changes += 1;
  keep(on ? [...was, kind] : was.filter((k) => k !== kind));
  if (!session()) return;
  api.setFavourite(kind, on).catch((e: unknown) => {
    keep(on ? favourites().filter((k) => k !== kind) : [...favourites(), kind]);
    toast({ tone: 'warn', title: 'Favourite not saved', body: errorText(e) });
  });
}
