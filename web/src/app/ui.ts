/** Page-wide UI state that is not part of the saved workspace. */
import { createSignal } from 'solid-js';
import type { MagnetMode } from '../engine/protocol';
import { readPref, writePref } from '../lib/prefs';

export type Overlay =
  | { kind: 'symbol'; panelId: string | null; initial: string; /** Where in a watchlist to add it. */ at?: number }
  | { kind: 'indicators'; panelId: string }
  | { kind: 'tpoSettings'; panelId: string }
  | { kind: 'palette' }
  | { kind: 'settings' }
  | { kind: 'help' }
  | { kind: 'referrals' }
  | { kind: 'note'; initial?: string; save(label: string): void };

export const [overlay, setOverlay] = createSignal<Overlay | null>(null);
export const closeOverlay = (): void => void setOverlay(null);

/** What the keyboard and the command palette can ask of a mounted chart. */
export interface ChartController {
  key(key: string): Promise<boolean>;
  /** Arm a drawing tool, or disarm it when it is the one armed. */
  tool(kind: string): void;
  resetView(): void;
  showRange(days: number | null): void;
  /** Undo or redo the last drawing change; false when there was none. */
  undo(): boolean;
  redo(): boolean;
  /** Copy a picture of the chart, or save the file instead. */
  capture(save: boolean): void;
  /** For the drawing rail: why drawing is off — the server out of reach — or null. */
  offline(): string | null;
  canUndo(): boolean;
  canRedo(): boolean;
  /** The drawing selected, to delete; and how many there are on this market, to clear. */
  selected(): number | null;
  deleteSelected(): void;
  drawingCount(): number;
  clearDrawings(): void;
}

export const chartControllers = new Map<string, ChartController>();
/** A market whose drawings changed somewhere else — a position moved from the journal — for its charts to read again. */
export const [drawingsChanged, setDrawingsChanged] = createSignal<{ market: string } | null>(null, { equals: false });
/** Bumped as charts come and go, so a rail looking a chart up finds it. */
export const [controllersChanged, setControllersChanged] = createSignal(0);

/**
 * The drawing tool armed, one for every chart: a tool picked on the rail draws
 * on whichever chart is clicked next, and is put down once that drawing is done.
 */
export const [armedTool, setArmedTool] = createSignal('');
/** One rail at the workspace's left for its charts, once it shows more than one; each chart then keeps none of its own. */
export const [sharedRail, setSharedRail] = createSignal(false);

export type { MagnetMode };

/** Up and Down in a panel with a list of its own to walk, by the panel's id: true when it took the key. */
export const panelSteps = new Map<string, (step: number) => boolean>();

/** Alt + the key arms a drawing tool on the chart in use, as on TradingView.
 *  By `KeyboardEvent.code`: on a Mac, Alt changes the letter `key` reports. */
export const TOOL_KEYS: Record<string, string> = {
  KeyT: 'trendline',
  KeyH: 'horizontal',
  KeyJ: 'ray',
  KeyR: 'rect',
  KeyC: 'channel',
  KeyF: 'fib',
  KeyN: 'text',
  KeyM: 'measure',
  KeyL: 'long',
  KeyS: 'short',
  KeyB: 'brush',
  KeyP: 'path',
};
/** A phone, where the workspace is one chart and a menu (Layout's MobileShell).
 *  Set by the layout, read by the chart, whose toolbar carries the button. */
export const [mobile, setMobile] = createSignal(false);
/** Whether that menu is open. */
export const [mobileMenu, setMobileMenu] = createSignal(false);
/** Whether the trading journal covers the workspace. */
export const [journalOpen, setJournalOpen] = createSignal(false);
/** The training view over the workspace, and the id its chart goes by. */
export const [trainingOpen, setTrainingOpen] = createSignal(false);
export const TRAINING_CHART = 'training-chart';
/** The workspace is under one of them: unseen, its charts stop drawing and its panels are not laid out. */
export const covered = (): boolean => journalOpen() || trainingOpen();

/** Alt + this key turns the magnet through off, weak and strong. */
export const MAGNET_KEY = 'KeyG';
/** Alt + this key copies a picture of the chart; with shift, saves the file. */
export const CAPTURE_KEY = 'KeyK';

/** The drawing magnet, one setting for every chart, remembered by this browser. */
const MAGNETS: readonly MagnetMode[] = ['off', 'weak', 'strong'];
const storedMagnet = readPref('magnet');
export const [magnet, setMagnetSignal] = createSignal<MagnetMode>(MAGNETS.includes(storedMagnet as MagnetMode) ? (storedMagnet as MagnetMode) : 'off');
export function setMagnet(mode: MagnetMode): void {
  setMagnetSignal(mode);
  writePref('magnet', mode);
}
/** Off, weak, strong, off again. */
export const cycleMagnet = (): void => setMagnet(MAGNETS[(MAGNETS.indexOf(magnet()) + 1) % MAGNETS.length]!);

/** A pattern the Signal panel opened, drawn on the charts of its market until
 *  it is closed or kept (ChartPanel). Shown, not saved. */
export interface ShownPattern {
  market: string;
  side: 'bear' | 'bull';
  drives: { at: number; price: number }[];
  score: number;
  divergence: boolean;
  /** At the line: the third drive is price reaching it now, not yet a closed bar. */
  stage?: 'line';
  /** The trade, from the bar it was entered on: take-profits and the stop. */
  levels?: { tp: number[]; sl: number };
  enteredAt?: number;
  /** The length of bar it was found on, which the chart frames it at; four hours where not said. */
  timeframe?: string;
  /** The limit entry, from the bar it was set on to the end of its two days. */
  limit?: { price: number; from: number; until: number };
}
export const [shownPattern, setShownPattern] = createSignal<ShownPattern | null>(null);

/** The panel whose tab is being dragged, while it is. */
export const [dragging, setDragging] = createSignal<string | null>(null);

/** A bare-letter shortcut must never fire while someone is typing. */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.tagName === 'SELECT' || target.isContentEditable;
}
