/**
 * The account's username: what it goes by where others see it — the Frost
 * Scripts it shares — in place of its Telegram name, which it need not give
 * away. Chosen once, in Settings or on first sharing, and never changed: the
 * server binds it to the Telegram account for good (bins/server username).
 */
import { createSignal } from 'solid-js';
import { api } from './api';
import { identity, session } from './owner';
import { readJson, writePref } from './prefs';

const KEY = 'username';

function held(): string | null {
  const raw = readJson(KEY) as { who?: unknown; name?: unknown } | null;
  return session() && raw?.who === identity() && typeof raw.name === 'string' ? raw.name : null;
}

const [name, setName] = createSignal<string | null>(held());
/** The account's username, without its @; null until it has one, or signed out. */
export const username = name;

/** What the server says it is, kept for the next load. */
export function noteUsername(next: string | null): void {
  setName(next);
  writePref(KEY, JSON.stringify({ who: identity(), name: next }));
}

/** Chosen for good. Throws what the server said: taken, or tradoor's own. */
export async function claimUsername(raw: string): Promise<string> {
  const r = await api.claimUsername(raw);
  noteUsername(r.username);
  return r.username;
}

/** What is wrong with it before the server is asked, as the server's rule has it; null for nothing. */
export function usernameProblem(raw: string): string | null {
  const n = raw.trim().replace(/^@/, '');
  if (n.length < 3 || n.length > 20) return 'a username is 3 to 20 characters';
  if (!/^[A-Za-z0-9_]+$/.test(n)) return 'a username is letters a to z, digits and _ only';
  if (!/^[A-Za-z]/.test(n)) return 'a username starts with a letter';
  return null;
}
