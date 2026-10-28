/**
 * The live workspace. Kept in localStorage so the page opens instantly and
 * offline, and on the server so it follows the reader to another browser.
 * Whichever copy changed last wins; writes carry the server version they were
 * based on, so two tabs cannot silently overwrite each other.
 */
import { batch, createSignal } from 'solid-js';
import { createStore, reconcile, unwrap } from 'solid-js/store';
import { api, ApiError } from '../lib/api';
import { identity } from '../lib/owner';
import { readJson, readPref, writePref } from '../lib/prefs';
import { defaultWorkspace, tabsOf, validate, type ChartPanelState, type LinkColor, type Panel, type SyncState, type WorkspaceDoc } from './model';
import { TIMEFRAMES } from '../lib/timeframes';

const LOCAL_KEY = 'workspace';
/** Whose the local copy is, written beside it. */
const LOCAL_OWNER_KEY = 'workspace-owner';
const SERVER_ID = 'default';

/**
 * Whether the copy in this browser is this browser's own to open with. Taking
 * another browser's id means asking for their layout, and theirs is on the
 * server: this one is a stranger's now, and opening with it would not only
 * show the wrong workspace but save it over the one that was asked for, since
 * whichever copy changed last wins. A copy from before the stamp existed has
 * no owner written beside it and is this browser's own.
 */
export const localIsOurs = (stamped: string | null, owner: string): boolean => stamped === null || stamped === owner;

const [doc, setDoc] = createStore<WorkspaceDoc>(
  (localIsOurs(readPref(LOCAL_OWNER_KEY), identity()) ? validate(readJson(LOCAL_KEY)) : null) ?? defaultWorkspace(),
);
export { doc };

/**
 * Charts outside the layout — the training view's — kept beside the workspace
 * rather than in it, which drops any panel its tree does not place. The panel
 * setters below reach them by id all the same; whoever holds one saves it.
 */
const [detached, setDetached] = createStore<Record<string, ChartPanelState>>({});
export { detached };

export function holdDetached(panel: ChartPanelState): void {
  setDetached(panel.id, panel);
}

export type SaveState = 'saved' | 'pending' | 'saving' | 'local' | 'conflict';
const [saveState, setSaveState] = createSignal<SaveState>('saved');
export { saveState };

const [focused, setFocused] = createSignal<string | null>(null);
const [lastChart, setLastChart] = createSignal<string | null>(null);
export const focusedPanelId = focused;

export function focusPanel(id: string): void {
  if (focused() !== id) setFocused(id);
  if (doc.panels[id]?.type === 'chart' && lastChart() !== id) setLastChart(id);
}

export function focusedPanel(): Panel | null {
  const id = focused();
  return (id && doc.panels[id]) || null;
}

/** The chart a watchlist that is not linked should drive: the last one used. */
export function targetChart(): ChartPanelState | null {
  const id = lastChart();
  const last = id ? doc.panels[id] : undefined;
  if (last?.type === 'chart') return last;
  const charts = Object.values(doc.panels).filter((p): p is ChartPanelState => p.type === 'chart');
  // Before one is used, the first on screen: a chart behind another tab has drawn nothing, not even a price.
  return charts.find((p) => tabsOf(doc.root, p.id)?.active === p.id) ?? charts[0] ?? null;
}

export const marketOf = (p: Panel): string => (p.link ? doc.links[p.link].market : p.market);
export const timeframeOf = (p: Panel): string => (p.link ? doc.links[p.link].timeframe : p.timeframe);

// ---------------------------------------------------------------- changes

let serverVersion = 0;
let localTimer: ReturnType<typeof setTimeout> | undefined;
let serverTimer: ReturnType<typeof setTimeout> | undefined;

function saveLocal(): void {
  clearTimeout(localTimer);
  writePref(LOCAL_KEY, JSON.stringify(unwrap(doc)));
  writePref(LOCAL_OWNER_KEY, identity());
}

function touched(): void {
  setDoc('savedAt', Date.now());
  clearTimeout(localTimer);
  localTimer = setTimeout(saveLocal, 300);
  clearTimeout(serverTimer);
  if (saveState() !== 'local') setSaveState('pending');
  serverTimer = setTimeout(() => void saveServer(), 1500);
}

/** Write the workspace this instant. It saves itself a second or so after every
 *  change; this only brings that forward, for a reader who reached for Ctrl+S. */
export function saveNow(): void {
  clearTimeout(localTimer);
  saveLocal();
  clearTimeout(serverTimer);
  void saveServer();
}

/** Structural changes: clone, mutate, reconcile. */
export function update(mutate: (draft: WorkspaceDoc) => void): void {
  const draft = structuredClone(unwrap(doc));
  mutate(draft);
  setDoc(reconcile(draft));
  touched();
}

export function patchPanel(id: string, patch: Partial<Omit<ChartPanelState, 'id' | 'type'>> | Record<string, unknown>): void {
  if (!doc.panels[id]) {
    if (detached[id]) (setDetached as (...args: unknown[]) => void)(id, patch);
    return;
  }
  (setDoc as (...args: unknown[]) => void)('panels', id, patch);
  touched();
}

/** Every chart on one ticker or one timeframe: a linked chart moves its group, the rest themselves. */
function stepCharts(key: 'market' | 'timeframe', value: string): void {
  batch(() => {
    for (const p of Object.values(doc.panels)) {
      if (p.type !== 'chart') continue;
      if (p.link) setDoc('links', p.link, key, value);
      setDoc('panels', p.id, key, value);
    }
  });
}

export function setPanelMarket(id: string, market: string): void {
  const p = doc.panels[id];
  if (!p && detached[id]) {
    if (detached[id].market !== market) setDetached(id, 'market', market);
    return;
  }
  if (!p || marketOf(p) === market) return;
  if (doc.sync.symbol && p.type === 'chart') {
    stepCharts('market', market);
    touched();
    return;
  }
  batch(() => {
    if (p.link) setDoc('links', p.link, 'market', market);
    setDoc('panels', id, 'market', market);
  });
  touched();
}

export function setPanelTimeframe(id: string, timeframe: string): void {
  const p = doc.panels[id];
  if (!p && detached[id]) {
    if (detached[id].timeframe !== timeframe) setDetached(id, 'timeframe', timeframe);
    return;
  }
  if (!p || timeframeOf(p) === timeframe) return;
  if (doc.sync.interval && p.type === 'chart') {
    stepCharts('timeframe', timeframe);
    touched();
    return;
  }
  batch(() => {
    if (p.link) setDoc('links', p.link, 'timeframe', timeframe);
    setDoc('panels', id, 'timeframe', timeframe);
  });
  touched();
}

/** A timeframe into the charts' bar of buttons, or back to their menu alone. */
export function toggleFavoriteTimeframe(timeframe: string): void {
  update((d) => {
    const on = d.timeframes.includes(timeframe);
    d.timeframes = TIMEFRAMES.map((t) => t.value).filter((tf) => (tf === timeframe ? !on : d.timeframes.includes(tf)));
  });
}

/** Joining a group adopts the group's market and timeframe. */
export function setPanelLink(id: string, color: LinkColor | null): void {
  const p = doc.panels[id];
  if (!p) return;
  batch(() => {
    if (color) {
      setDoc('panels', id, 'market', doc.links[color].market);
      setDoc('panels', id, 'timeframe', doc.links[color].timeframe);
    } else if (p.link) {
      setDoc('panels', id, 'market', doc.links[p.link].market);
      setDoc('panels', id, 'timeframe', doc.links[p.link].timeframe);
    }
    setDoc('panels', id, 'link', color);
  });
  touched();
}

export function setLinkMarket(color: LinkColor, market: string): void {
  if (doc.links[color].market === market) return;
  batch(() => {
    setDoc('links', color, 'market', market);
    if (doc.sync.symbol) stepCharts('market', market);
  });
  touched();
}

/** A sync switch: Symbol or Interval turned on brings every chart to the one in use's ticker or timeframe at once. */
export function setSync(key: keyof SyncState, on: boolean): void {
  batch(() => {
    setDoc('sync', key, on);
    const chart = targetChart();
    if (on && chart && key === 'symbol') stepCharts('market', marketOf(chart));
    if (on && chart && key === 'interval') stepCharts('timeframe', timeframeOf(chart));
  });
  touched();
}

export function setThemeValue(key: string, value: string | null): void {
  (setDoc as (...args: unknown[]) => void)('theme', key, value ?? undefined);
  touched();
}

export function resetWorkspace(): void {
  update((d) => Object.assign(d, defaultWorkspace()));
}

// ---------------------------------------------------------------- server

let saving = false;
let saveAgain = false;

function adopt(server: WorkspaceDoc): void {
  setDoc(reconcile(server));
  saveLocal();
  setSaveState('saved');
}

async function saveServer(): Promise<void> {
  if (saving) {
    saveAgain = true;
    return;
  }
  saving = true;
  setSaveState('saving');
  try {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const res = await api.saveWorkspace(SERVER_ID, { name: doc.name, doc: unwrap(doc), version: serverVersion });
      serverVersion = res.version;
      if (!res.conflict) {
        setSaveState('saved');
        break;
      }
      // Another tab saved first. The newer copy wins; ours is retried on top of its version.
      const server = validate(res.doc);
      if (server && server.savedAt > doc.savedAt) {
        adopt(server);
        break;
      }
      if (attempt === 1) setSaveState('conflict');
    }
  } catch {
    setSaveState('local');
  } finally {
    saving = false;
    if (saveAgain) {
      saveAgain = false;
      void saveServer();
    }
  }
}

export async function syncWithServer(): Promise<void> {
  try {
    const res = await api.workspace(SERVER_ID);
    serverVersion = res.version;
    const server = validate(res.doc);
    if (server && server.savedAt > doc.savedAt) adopt(server);
    else if (!server || server.savedAt < doc.savedAt) void saveServer();
    else setSaveState('saved');
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) {
      serverVersion = 0;
      void saveServer();
    } else {
      setSaveState('local');
    }
  }
}

addEventListener('pagehide', saveLocal);
