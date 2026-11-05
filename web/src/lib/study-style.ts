/**
 * How a chart's studies and layers draw, where the reader changed it. By
 * `owner/plot` — a study's id or a layer's, and one of its plots — either
 * `colour,width,off,pattern` or a number the owner takes. A value back at its default
 * is forgotten, so a chart left alone stores nothing. The engine reads the same
 * spelling (`Styles::parse` in tradoor-render).
 */
export type StudyStyles = Record<string, string>;

export interface PlotStyle {
  /** `#rrggbb` or `#rrggbbaa`; empty is the study's own. */
  color: string;
  /** Empty is the study's own. */
  width: string;
  off: boolean;
  /** `0` solid, `1` dashed, `2` dotted; empty is the study's own. */
  pattern: string;
}

export const splitList = (s: string): string[] =>
  s
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);

const at = (owner: string, plot: string): string => `${owner}/${plot}`;
const ownerOf = (key: string): string => key.slice(0, key.indexOf('/'));

export function plotStyle(styles: StudyStyles | undefined, owner: string, plot: string): PlotStyle {
  const [color = '', width = '', off = '', pattern = ''] = (styles?.[at(owner, plot)] ?? '').split(',');
  return { color, width, off: off === '1', pattern };
}

export function withPlot(styles: StudyStyles | undefined, owner: string, plot: string, patch: Partial<PlotStyle>): StudyStyles {
  const p = { ...plotStyle(styles, owner, plot), ...patch };
  const next = { ...styles };
  if (p.color || p.width || p.off || p.pattern) next[at(owner, plot)] = `${p.color},${p.width},${p.off ? '1' : ''}${p.pattern ? `,${p.pattern}` : ''}`;
  else delete next[at(owner, plot)];
  return next;
}

export function numberOf(styles: StudyStyles | undefined, owner: string, key: string, fallback: number): number {
  const raw = styles?.[at(owner, key)];
  return raw !== undefined && Number.isFinite(Number(raw)) ? Number(raw) : fallback;
}

export function withNumber(styles: StudyStyles | undefined, owner: string, key: string, value: number, fallback: number): StudyStyles {
  const next = { ...styles };
  if (value === fallback) delete next[at(owner, key)];
  else next[at(owner, key)] = String(value);
  return next;
}

/** What it was changed to, carried to the same study with other parameters. */
export function renameOwner(styles: StudyStyles | undefined, from: string, to: string): StudyStyles {
  return Object.fromEntries(Object.entries(styles ?? {}).map(([key, value]) => [ownerOf(key) === from ? `${to}${key.slice(from.length)}` : key, value]));
}

/** Everything of one owner's forgotten: it was removed, or put back to its defaults. */
export function dropOwner(styles: StudyStyles | undefined, owner: string): StudyStyles {
  return Object.fromEntries(Object.entries(styles ?? {}).filter(([key]) => ownerOf(key) !== owner));
}

/** What the engine is sent. */
export const serializeStyles = (styles: StudyStyles | undefined): string =>
  Object.entries(styles ?? {})
    .map(([key, value]) => `${key}=${value}`)
    .join(';');

const KEY = /^[a-z0-9.:]{1,40}\/[A-Za-z0-9]{1,20}$/;
const VALUE = /^[#0-9a-fA-F.,]{0,32}$/;

/** A stored chart's styles, with what cannot be one left out. */
export function readStyles(raw: unknown): StudyStyles {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return {};
  return Object.fromEntries(Object.entries(raw).filter(([key, value]) => KEY.test(key) && typeof value === 'string' && VALUE.test(value)).slice(0, 300)) as StudyStyles;
}

/** A study swapped for the same one with other parameters: shown or hidden as it was, and styled. */
export function replaceStudy(panel: { studies: string; hidden: string; studyStyles?: StudyStyles }, from: string, to: string): { studies: string; hidden: string; studyStyles: StudyStyles } {
  const swap = (list: string) => list.split(',').map((s) => (s === from ? to : s)).join(',');
  return { studies: swap(panel.studies), hidden: swap(panel.hidden), studyStyles: renameOwner(panel.studyStyles, from, to) };
}

const listed = (list: string, item: string): string => [...splitList(list).filter((x) => x !== item), item].join(',');
const unlisted = (list: string, item: string): string => splitList(list).filter((x) => x !== item).join(',');

/** A built-in layer taken off a chart: hidden, so nothing draws it, with no legend row, and its settings forgotten as a removed study's are. */
export function withoutLayer(panel: { hidden: string; removed: string; studyStyles?: StudyStyles }, layer: string): { hidden: string; removed: string; studyStyles: StudyStyles } {
  return { hidden: listed(panel.hidden, layer), removed: listed(panel.removed, layer), studyStyles: dropOwner(panel.studyStyles, layer) };
}

/** A built-in layer back on a chart, and drawn. */
export function withLayer(panel: { hidden: string; removed: string }, layer: string): { hidden: string; removed: string } {
  return { hidden: unlisted(panel.hidden, layer), removed: unlisted(panel.removed, layer) };
}
