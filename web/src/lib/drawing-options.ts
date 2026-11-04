/** What the drawing dialog and toolbar need beyond a drawing's style: names,
 *  the level tables (the same defaults the engine draws with), templates and
 *  visibility. Loaded with them, not with the chart. */
import { writePref } from './prefs';
import { hiddenUnits, TEMPLATE_KEY, templates, type Style } from './drawing-style';

export interface Level {
  value: number;
  on: boolean;
  color: string;
  dash: Dash;
}

export type Dash = 'solid' | 'dashed' | 'dotted';

export const KIND_NAMES: Record<string, string> = {
  path: 'Path',
  brush: 'Brush',
  trendline: 'Trendline',
  horizontal: 'Horizontal line',
  ray: 'Horizontal ray',
  rect: 'Rectangle',
  fib: 'Fib retracement',
  text: 'Text',
  measure: 'Price range',
  long: 'Long position',
  short: 'Short position',
  channel: 'Parallel channel',
};

export const isPosition = (kind: string): boolean => kind === 'long' || kind === 'short';

const CHANNEL_LEVELS: Level[] = [
  { value: -0.25, on: false, color: '#2962ff', dash: 'solid' },
  { value: 0, on: true, color: '', dash: 'solid' },
  { value: 0.25, on: false, color: '#2962ff', dash: 'solid' },
  { value: 0.5, on: true, color: '#ff6d00', dash: 'dashed' },
  { value: 0.75, on: false, color: '#2962ff', dash: 'solid' },
  { value: 1, on: true, color: '', dash: 'solid' },
  { value: 1.25, on: false, color: '#2962ff', dash: 'solid' },
];

const FIB_LEVELS: Level[] = [
  { value: 0, on: true, color: '#787b86', dash: 'solid' },
  { value: 0.236, on: true, color: '#f23645', dash: 'solid' },
  { value: 0.382, on: true, color: '#ff9800', dash: 'solid' },
  { value: 0.5, on: true, color: '#4caf50', dash: 'solid' },
  { value: 0.618, on: true, color: '#089981', dash: 'solid' },
  { value: 0.786, on: true, color: '#00bcd4', dash: 'solid' },
  { value: 1, on: true, color: '#787b86', dash: 'solid' },
  { value: 1.618, on: false, color: '#2962ff', dash: 'solid' },
  { value: 2.618, on: false, color: '#f23645', dash: 'solid' },
  { value: 3.618, on: false, color: '#9c27b0', dash: 'solid' },
  { value: 4.236, on: false, color: '#e91e63', dash: 'solid' },
];

export const serializeLevels = (levels: Level[]): string => levels.map((l) => `${l.value},${l.on ? 1 : 0},${l.color},${l.dash}`).join('|');

export function parseLevels(raw: unknown, kind: string): Level[] {
  if (typeof raw !== 'string' || raw === '') return (kind === 'channel' ? CHANNEL_LEVELS : FIB_LEVELS).map((l) => ({ ...l }));
  return raw.split('|').flatMap((item) => {
    const [v, on, color, dash] = item.split(',');
    const value = Number(v);
    if (!Number.isFinite(value)) return [];
    return [{ value, on: on !== '0', color: color ?? '', dash: dash === 'dashed' || dash === 'dotted' ? dash : 'solid' }];
  });
}

export function saveDefaultStyle(kind: string, style: Style): void {
  const all = templates();
  // A position's legs are prices of its own, no other position's.
  const { legs: _legs, ...kept } = style;
  all[kind] = kept;
  writePref(TEMPLATE_KEY, JSON.stringify(all));
}

/** The next position starts at the size this one was set to, long or short. */
export function rememberSize(style: Style): void {
  const all = templates();
  const size = Object.fromEntries(['sizeBy', 'margin', 'leverage', 'account', 'risk', 'riskUnit'].filter((k) => style[k] !== undefined).map((k) => [k, style[k]!]));
  for (const kind of ['long', 'short']) all[kind] = { ...all[kind], ...size };
  writePref(TEMPLATE_KEY, JSON.stringify(all));
}

export function resetDefaultStyle(kind: string): void {
  const all = templates();
  delete all[kind];
  writePref(TEMPLATE_KEY, JSON.stringify(all));
}

/** Timeframe families a drawing can be hidden on, by the timeframe's unit letter. */
export const VISIBILITY_UNITS: { unit: string; label: string }[] = [
  { unit: 'm', label: 'Minutes' },
  { unit: 'h', label: 'Hours' },
  { unit: 'd', label: 'Days' },
  { unit: 'w', label: 'Weeks' },
  { unit: 'M', label: 'Months' },
];

export function withHidden(style: Style, unit: string, hidden: boolean): Style {
  const set = new Set(hiddenUnits(style));
  if (hidden) set.add(unit);
  else set.delete(unit);
  return { ...style, hideOn: [...set].join(',') };
}
