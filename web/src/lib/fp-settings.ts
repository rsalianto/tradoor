/**
 * The browser footprint recorder's settings, and which markets it can record:
 * the few lines a chart needs to know whether to load the recorder at all
 * (lib/fp-kit), kept apart from it so the main script carries no more.
 */
import { createSignal } from 'solid-js';
import { isRatio, parseMarket } from './markets';

const SETTINGS_KEY = 'tradoor.footprint';

export interface RecorderSettings {
  /** Record in this browser at all. */
  on: boolean;
  /** Days kept: older minutes are let go. */
  days: number;
}

const DEFAULTS: RecorderSettings = { on: true, days: 7 };
export const KEEP_DAYS = [1, 3, 7, 14, 30] as const;

function loadSettings(): RecorderSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}') as Partial<RecorderSettings>;
    return { on: raw.on !== false, days: KEEP_DAYS.includes(raw.days as (typeof KEEP_DAYS)[number]) ? raw.days! : DEFAULTS.days };
  } catch {
    return { ...DEFAULTS };
  }
}

const [settings, setSettingsSignal] = createSignal<RecorderSettings>(loadSettings());
export const recorderSettings = settings;

/** The settings changed and kept; what the recorder does about it is its own (setRecorderSettings). */
export function storeRecorderSettings(next: RecorderSettings): void {
  setSettingsSignal(next);
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(next));
  } catch {
    // Kept for this page only.
  }
}

/** Whether a market's trades can be recorded here: an exchange's, not an index or a ratio. */
export const recordable = (market: string): boolean => {
  const venue = parseMarket(market)?.venue;
  return !!venue && venue !== 'index' && !isRatio(market);
};
