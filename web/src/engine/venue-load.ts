/** Fetches the newer exchanges' file (see venue-pack) once, for whoever asks first. */
// @ts-ignore: to Vite this is the built file's address; the tests' type check takes it for the module.
import url from './venue-pack.ts?worker&url';
import type { VenuePack } from './venue-pack';

let held: Promise<VenuePack> | null = null;

export function loadVenues(): Promise<VenuePack> {
  held ??= import(/* @vite-ignore */ url as string).then(() => (globalThis as { tradoorVenues?: VenuePack }).tradoorVenues as VenuePack);
  // Tried again next time: a dropped connection is no reason to give up on them.
  held.catch(() => {
    held = null;
  });
  return held;
}
