/**
 * TPO settings as a chart keeps them: only what differs from the defaults —
 * MMT's, with colours left to the theme — which are listed here and are the
 * engine's defaults too. The dialog's layout
 * — tabs, names, hints — is in tpo-schema, loaded with it.
 */

export type TpoSettings = Record<string, string>;

const flag = (on: boolean) => (on ? '1' : '0');

/** MMT's four sessions, in UTC: start–end and a colour each. */
export const DEFAULT_SESSIONS = '0000-0600#e3cd2a,0600-1200#c8202e,1200-2000#2f5bd8,2000-0000#2aa12a';

/** A colour left to the theme: worked out from its three TPO colours and its candles'. */
const THEMED = '';

export const TPO_DEFAULTS: Readonly<Record<string, string>> = {
  // General
  rowTicks: '0',
  profileColor: THEMED,
  blockMode: 'brightness',
  heatLow: '#d32f2f',
  heatHigh: '#ffd54f',
  highlight: flag(true),
  startColor: THEMED,
  endColor: THEMED,
  hideText: flag(false),
  spacing: '5',
  // Session
  sessions: DEFAULT_SESSIONS,
  sessionHeader: flag(false),
  // Point of control
  poc: flag(true),
  pocWidth: '2',
  pocColor: THEMED,
  pocRayColor: THEMED,
  nakedPoc: flag(false),
  // Value area
  vaPct: '70',
  vaColor: THEMED,
  vaRays: flag(true),
  vaRayColor: THEMED,
  nakedVa: flag(false),
  fillVa: flag(false),
  fillVaColor: THEMED,
  // Single prints
  singlePrints: flag(true),
  singlePrintColor: THEMED,
  singlePrintRayColor: THEMED,
  nakedSinglePrints: flag(false),
  // Poor high, poor low
  poor: flag(true),
  poorColor: THEMED,
  nakedPoor: flag(false),
  // Initial balance
  ib: flag(false),
  ibColor: THEMED,
  ibCandles: '2',
  // Volume profile
  vp: flag(false),
  vpBuy: THEMED,
  vpSell: THEMED,
  vpColor: THEMED,
  vpMode: 'standard',
  vpOrigin: 'right',
  vpWidth: '1',
  vpVaRays: flag(false),
  vpVaColor: THEMED,
  vpNakedVa: flag(false),
  vpVaPct: '70',
  vpLabels: flag(true),
  vpLabelColor: THEMED,
  vpPoc: flag(false),
  vpPocColor: THEMED,
  vpNakedPoc: flag(false),
  vpHideText: flag(false),
  vpBinFilter: '0',
};

const mix = (a: string, b: string, t: number): string => {
  const ch = (hex: string, i: number) => parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16) || 0;
  return `#${[0, 1, 2].map((i) => Math.round(ch(a, i) + (ch(b, i) - ch(a, i)) * t).toString(16).padStart(2, '0')).join('')}`;
};

/** The colour a TPO setting left to the theme is drawn in, as the engine works it out. */
export function themedTpoColor(key: string, theme: (name: string) => string): string {
  const tpo = theme('tpo').slice(0, 7);
  switch (key) {
    case 'profileColor':
    case 'vpVaColor':
    case 'vpPocColor':
      return theme('tpoOutside');
    case 'startColor':
    case 'vpBuy':
      return theme('bull');
    case 'endColor':
    case 'vpSell':
      return theme('bear');
    case 'vaColor':
    case 'vaRayColor':
      return tpo;
    case 'vpColor':
      return `${tpo}8c`;
    case 'fillVaColor':
      return `${tpo}22`;
    case 'singlePrintColor':
      return mix(theme('background').slice(0, 7), tpo, 0.22);
    case 'singlePrintRayColor':
      return `${tpo}5c`;
    default:
      return theme('tpoPoc');
  }
}

/** One custom session: minutes past midnight UTC, and its colour. */
export interface TpoSession {
  start: number;
  end: number;
  color: string;
}

const hhmm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, '0')}${String(m % 60).padStart(2, '0')}`;

/** `0000-0600#e3cd2a,…` as sessions; a part that does not read is skipped. */
export function parseSessions(raw: string): TpoSession[] {
  return raw.split(',').flatMap((part) => {
    const m = /^(\d{2})(\d{2})-(\d{2})(\d{2})#([0-9a-fA-F]{6})$/.exec(part.trim());
    if (!m) return [];
    const [start, end] = [Number(m[1]) * 60 + Number(m[2]), Number(m[3]) * 60 + Number(m[4])];
    return start <= 1440 && end <= 1440 && Number(m[2]) < 60 && Number(m[4]) < 60 ? [{ start, end, color: `#${m[5]!.toLowerCase()}` }] : [];
  });
}

export const serializeSessions = (list: readonly TpoSession[]): string => list.map((s) => `${hhmm(s.start)}-${hhmm(s.end)}${s.color.slice(0, 7)}`).join(',');

/** The value in force: the chart's own, or the default. */
export const tpoValue = (settings: TpoSettings | undefined, key: string): string => settings?.[key] ?? TPO_DEFAULTS[key] ?? '';

/** `settings` with `key` set, and dropped again once it equals the default. */
export function withTpoValue(settings: TpoSettings | undefined, key: string, value: string): TpoSettings {
  const next = { ...settings };
  if (value === TPO_DEFAULTS[key]) delete next[key];
  else next[key] = value;
  return next;
}

/** What the engine is sent: known keys only, `key=value;key=value`. */
export function serializeTpo(settings: TpoSettings | undefined): string {
  return Object.entries(settings ?? {})
    .filter(([key, value]) => key in TPO_DEFAULTS && !/[;=]/.test(value))
    .map(([key, value]) => `${key}=${value}`)
    .join(';');
}
