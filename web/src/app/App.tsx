/**
 * The shell: a command bar, the workspace, a status line, and the dialogs.
 * Keyboard shortcuts go to the focused panel.
 */
import { createEffect, createSignal, For, lazy, onCleanup, onMount, Show } from 'solid-js';
import type { Command } from '../components/CommandPalette';
import { NoteInput } from '../components/NoteInput';
import { lastPrice, onEngineFailure, send } from '../engine/client';
import { api } from '../lib/api';
import { deviceZone, formatClock, LOCAL_ZONE, offsetMinutes, timeZone, zoneLabel } from '../lib/timezone';
import { MARK_ORDER, MARKS, markSpec } from '../lib/exchange-marks';
import { useDismiss } from '../lib/dismiss';
import { Icon } from '../lib/icons';
import { tabTitle } from '../lib/tab-title';
import { CHART_THEME, FONT_PX, UI_THEME } from '../lib/theme';
import { stepTimeframe, TIMEFRAMES } from '../lib/timeframes';
import { showAlertsPanel, startAlerts, unread } from '../lib/alerts';
import { Toasts } from '../components/Toasts';
import { toast } from '../lib/toasts';
import { takeNotice } from '../lib/owner';
import { insertMarket, isSection, visible } from '../lib/watchlist';
import { referralCard, startReferral } from '../lib/referral';
import { Layout, panelTitle } from '../workspace/Layout';
import { ChartPanel } from '../panels/ChartPanel';
import { addPanel, LINK_COLORS, newPanel, PANEL_TITLE, PANEL_TYPES, removePanel, tabsOf, TPO_SPLITS, type ChartPanelState, type DropZone, type Panel, type PanelType, type WatchlistPanelState } from '../workspace/model';
import {
  detached,
  doc,
  focusedPanel,
  focusPanel,
  marketOf,
  patchPanel,
  resetWorkspace,
  saveNow,
  saveState,
  setPanelLink,
  setPanelMarket,
  setLinkMarket,
  setPanelTimeframe,
  syncWithServer,
  targetChart,
  timeframeOf,
  update,
} from '../workspace/store';
import { CAPTURE_KEY, chartControllers, closeOverlay, cycleMagnet, isTypingTarget, journalOpen, MAGNET_KEY, mobileMenu, overlay, panelSteps, setJournalOpen, setOverlay, setTrainingOpen, TOOL_KEYS, TRAINING_CHART, trainingOpen } from './ui';

// Dialogs opened now and then load when first opened: nobody needs them to see a chart.
const SymbolSearch = lazy(() => import('../components/SymbolSearch').then((m) => ({ default: m.SymbolSearch })));
const Help = lazy(() => import('../components/Help').then((m) => ({ default: m.Help })));
const CommandPalette = lazy(() => import('../components/CommandPalette').then((m) => ({ default: m.CommandPalette })));
const IndicatorPicker = lazy(() => import('../components/IndicatorPicker').then((m) => ({ default: m.IndicatorPicker })));
const Settings = lazy(() => import('../components/Settings').then((m) => ({ default: m.Settings })));
const LayoutPicker = lazy(() => import('../components/LayoutPicker').then((m) => ({ default: m.LayoutPicker })));
const TpoSettings = lazy(() => import('../components/TpoSettings').then((m) => ({ default: m.TpoSettings })));
const ReferralDialog = lazy(() => import('../components/Referrals').then((m) => ({ default: m.ReferralDialog })));
const ReferralCard = lazy(() => import('../components/Referrals').then((m) => ({ default: m.ReferralCard })));
const Journal = lazy(() => import('../journal/Journal').then((m) => ({ default: m.Journal })));
const Training = lazy(() => import('../training/Training').then((m) => ({ default: m.Training })));

const splitList = (s: string): string[] => s.split(',').filter(Boolean);

function addNear(type: PanelType, zone: DropZone): void {
  const from = focusedPanel() ?? targetChart();
  const panel = newPanel(type, from ? { link: from.link, market: marketOf(from), timeframe: timeframeOf(from) } : undefined);
  update((d) => addPanel(d, panel, from ? (tabsOf(d.root, from.id)?.id ?? null) : null, zone));
  focusPanel(panel.id);
}

/** A picked market lands in the panel that asked: a watchlist gains a row, anything else switches. */
function pickMarket(panelId: string | null, market: string, at?: number): void {
  const panel = (panelId && doc.panels[panelId]) || targetChart();
  if (!panel) return;
  if (panel.type === 'watchlist') {
    if (at !== undefined) patchPanel(panel.id, { markets: insertMarket(panel.markets, market, at) });
    else if (!panel.markets.includes(market)) patchPanel(panel.id, { markets: [...panel.markets, market] });
  } else {
    setPanelMarket(panel.id, market);
  }
}

function commands(): Command[] {
  const focus = focusedPanel() ?? targetChart();
  const list: Command[] = [];
  if (focus && focus.type !== 'watchlist') {
    list.push({ group: 'Panel', label: 'Change symbol…', hint: 'a–z', run: () => setOverlay({ kind: 'symbol', panelId: focus.id, initial: '' }) });
    for (const tf of TIMEFRAMES) list.push({ group: 'Timeframe', label: tf.name, hint: tf.label, run: () => setPanelTimeframe(focus.id, tf.value) });
  }
  if (focus?.type === 'chart') {
    const chart = focus;
    list.push(
      { group: 'Chart', label: 'Add indicator…', run: () => setOverlay({ kind: 'indicators', panelId: chart.id }) },
      { group: 'Chart', label: chart.log ? 'Linear price scale' : 'Logarithmic price scale', run: () => patchPanel(chart.id, { log: !chart.log }) },
      { group: 'Chart', label: 'Reset chart view', run: () => chartControllers.get(chart.id)?.resetView() },
      { group: 'Chart', label: 'Show all loaded history', run: () => chartControllers.get(chart.id)?.showRange(null) },
      { group: 'Chart style', label: 'Candles', run: () => patchPanel(chart.id, { tpo: '' }) },
      { group: 'Chart style', label: 'TPO settings…', run: () => setOverlay({ kind: 'tpoSettings', panelId: chart.id }) },
      ...TPO_SPLITS.map((split) => ({ group: 'Chart style', label: `TPO — split by ${split.label.toLowerCase()}`, run: () => patchPanel(chart.id, { tpo: split.value }) })),
    );
  }
  const zones: [DropZone, string][] = [
    ['center', 'as a tab'],
    ['right', 'split right'],
    ['bottom', 'split down'],
  ];
  for (const type of PANEL_TYPES) for (const [zone, where] of zones) list.push({ group: 'New panel', label: `${PANEL_TITLE[type]} — ${where}`, run: () => addNear(type, zone) });
  if (focus) {
    list.push({ group: 'Link', label: 'Unlink this panel', run: () => setPanelLink(focus.id, null) });
    for (const color of LINK_COLORS) list.push({ group: 'Link', label: `Link this panel to ${color}`, run: () => setPanelLink(focus.id, color) });
    list.push({ group: 'Panel', label: `Close ${panelTitle(focus)}`, run: () => update((d) => removePanel(d, focus.id)) });
  }
  list.push(
    { group: 'App', label: 'Settings and colours…', run: () => setOverlay({ kind: 'settings' }) },
    { group: 'App', label: 'Keyboard shortcuts', hint: '?', run: () => setOverlay({ kind: 'help' }) },
    { group: 'App', label: 'Partner exchanges — Nado, Bybit and Binance', run: () => setOverlay({ kind: 'referrals' }) },
    {
      group: 'App',
      label: 'Reset workspace to default',
      run: () => {
        if (confirm('Replace this workspace with the default layout?')) resetWorkspace();
      },
    },
  );
  return list;
}

/**
 * Up and Down walk a watchlist, switching the chart to the next market in it:
 * the focused watchlist, else the one linked with this panel, else any.
 */
function stepWatchlist(panel: Panel, step: number): boolean {
  const lists = Object.values(doc.panels).filter((p): p is WatchlistPanelState => p.type === 'watchlist' && p.markets.length > 0);
  const list = panel.type === 'watchlist' ? panel : (lists.find((w) => panel.link && w.link === panel.link) ?? lists.find((w) => !w.link) ?? lists[0]);
  if (!list || list.markets.length === 0) return false;
  // What the list drives: its link, the panel asked from, or the last chart.
  const target = panel.type === 'watchlist' ? targetChart() : panel;
  const current = list.link ? doc.links[list.link].market : target ? marketOf(target) : null;
  // Markets as shown: not section headers, nor what a folded section hides.
  const rows = visible(list.markets, list.collapsed).map((i) => list.markets[i]!).filter((m) => !isSection(m));
  if (rows.length === 0) return false;
  const at = current ? rows.indexOf(current) : -1;
  const next = rows[at === -1 ? (step > 0 ? 0 : rows.length - 1) : (at + step + rows.length) % rows.length]!;
  if (panel.type === 'watchlist' && list.link) setLinkMarket(list.link, next);
  else if (target) setPanelMarket(target.id, next);
  return true;
}

function onKeyDown(e: KeyboardEvent): void {
  // The journal over the charts keeps the keys to itself.
  if (journalOpen()) return;
  // The training view's chart takes the drawing keys; the workspace's wait behind it.
  const training = trainingOpen();
  const inUse = (chart: ChartPanelState | null) => chartControllers.get(training ? TRAINING_CHART : (chart?.id ?? ''));
  if (!training && (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
    e.preventDefault();
    setOverlay(overlay()?.kind === 'palette' ? null : { kind: 'palette' });
    return;
  }
  // Alt + a letter: a drawing tool on the chart in use, or the magnet.
  if (e.altKey && !e.ctrlKey && !e.metaKey && !overlay() && !isTypingTarget(e.target)) {
    // The physical key; some input sources report only the letter.
    const code = e.code || (/^[a-z]$/i.test(e.key) ? `Key${e.key.toUpperCase()}` : '');
    if (code === MAGNET_KEY) {
      e.preventDefault();
      cycleMagnet();
      return;
    }
    const kind = TOOL_KEYS[code];
    const focus = focusedPanel();
    const control = inUse(focus?.type === 'chart' ? focus : targetChart());
    // A picture of the chart in use: shift saves the file instead of copying.
    if (code === CAPTURE_KEY && control) {
      e.preventDefault();
      control.capture(e.shiftKey);
      return;
    }
    if (kind && control) {
      e.preventDefault();
      control.tool(kind);
      return;
    }
  }
  // Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y: undo and redo drawings on the chart in use.
  const mod = e.metaKey || e.ctrlKey;
  const k = e.key.toLowerCase();
  if (mod && !e.altKey && (k === 'z' || k === 'y') && !overlay() && !isTypingTarget(e.target)) {
    const focus = focusedPanel();
    const control = inUse(focus?.type === 'chart' ? focus : targetChart());
    if (control) {
      e.preventDefault();
      if (k === 'y' || e.shiftKey) control.redo();
      else control.undo();
    }
    return;
  }
  // Ctrl+S is the browser's "save this page", which is never what is meant
  // here and drops a half-page of HTML in the downloads. The workspace saves
  // itself; the key brings that forward, and the footer says it happened.
  if (mod && !e.altKey && k === 's' && !isTypingTarget(e.target)) {
    e.preventDefault();
    saveNow();
    return;
  }
  // Dialogs own the keyboard while open; modifiers belong to the browser.
  if (overlay() || e.metaKey || e.ctrlKey || e.altKey || isTypingTarget(e.target)) return;
  if (e.key === '?') {
    e.preventDefault();
    setOverlay({ kind: 'help' });
    return;
  }
  if (e.key === '/') {
    e.preventDefault();
    setOverlay({ kind: 'palette' });
    return;
  }
  const panel = training ? detached[TRAINING_CHART] : (focusedPanel() ?? targetChart());
  if (!panel) return;
  if (panel.type === 'chart' && (e.key === 'Escape' || e.key === 'Enter' || e.key === 'Delete' || e.key === 'Backspace')) {
    if (e.key === 'Delete' || e.key === 'Backspace') e.preventDefault();
    void chartControllers.get(panel.id)?.key(e.key);
    return;
  }
  // The rest — another symbol, timeframe or list row — is the workspace's.
  if (training) return;
  // A panel with a list of its own walks that (the Signal panel's patterns); any other, a watchlist.
  const step = e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0;
  if (step && (panelSteps.get(panel.id)?.(step) || stepWatchlist(panel, step))) {
    e.preventDefault();
    return;
  }
  if (panel.type !== 'watchlist' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
    e.preventDefault();
    setPanelTimeframe(panel.id, stepTimeframe(timeframeOf(panel), e.key === 'ArrowLeft' ? -1 : 1));
    return;
  }
  // Letters only: typing a ticker opens the search with that letter already in it.
  if (/^[a-z]$/i.test(e.key)) {
    e.preventDefault();
    setOverlay({ kind: 'symbol', panelId: panel.id, initial: e.key.toUpperCase() });
  }
}

function applyTheme(): void {
  // The marks travel from here so the worker does not carry their artwork.
  send({ t: 'marks', list: MARK_ORDER.map((id, index) => ({ index, box: MARKS[id].box, spec: markSpec(MARKS[id]) })) });
  for (const t of CHART_THEME) createEffect(() => send({ t: 'theme', key: t.name, value: doc.theme[t.name] ?? t.value }));
  // The zone's offset, again every minute: daylight saving moves it under a running page.
  createEffect(() => {
    const zone = timeZone();
    const push = () => send({ t: 'timeZone', offsetMinutes: offsetMinutes(zone) });
    push();
    const timer = setInterval(push, 60_000);
    onCleanup(() => clearInterval(timer));
  });
  createEffect(() => send({ t: 'theme', key: FONT_PX.name, value: doc.theme[FONT_PX.name] ?? FONT_PX.value }));
  for (const t of UI_THEME) {
    createEffect(() => {
      const value = doc.theme[t.name];
      if (value) document.documentElement.style.setProperty(t.css, value);
      else document.documentElement.style.removeProperty(t.css);
    });
  }
}

function Health() {
  const [state, setState] = createSignal<'checking' | 'ok' | 'db' | 'down'>('checking');
  const check = async () => {
    try {
      setState((await api.health()).ok ? 'ok' : 'db');
    } catch {
      setState('down');
    }
  };
  onMount(() => {
    void check();
    const timer = setInterval(() => void check(), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  const title = () =>
    ({
      checking: 'Checking the tradoor server…',
      ok: 'Server and database reachable: drawings and layouts are saved.',
      db: 'The server answers but its database does not: drawings cannot be saved.',
      down: 'The tradoor server is not reachable. Charts still load from the exchanges; drawings and layout sync are paused.',
    })[state()];
  return (
    <span class="item" data-tip={title()}>
      <span class="health" classList={{ bull: state() === 'ok', warn: state() === 'db', bear: state() === 'down', dim: state() === 'checking' }}>
        <Icon name="server" size={12} />
      </span>
      <span>{state() === 'down' ? 'OFFLINE' : state() === 'db' ? 'NO DB' : 'SERVER'}</span>
    </span>
  );
}

function Clock() {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  return (
    <span class="item num" data-tip={`Times are shown in ${timeZone() === LOCAL_ZONE ? `your local time (${deviceZone()})` : timeZone()} — change it in Settings`}>
      <span class="dim">{zoneLabel()}</span>
      <span>{formatClock(now())}</span>
    </span>
  );
}

const SAVE_TEXT = {
  saved: 'layout saved',
  pending: 'layout changed',
  saving: 'saving layout…',
  local: 'layout saved in this browser only',
  conflict: 'another tab changed this layout',
} as const;

function StatusBar() {
  return (
    <footer class="statusbar">
      <span class="caps">{focusedPanel() ? panelTitle(focusedPanel()!) : 'no panel focused'}</span>
      <For
        each={[
          ['a–z', 'symbol'],
          ['← →', 'timeframe'],
          ['/', 'commands'],
          ['?', 'help'],
        ]}
      >
        {([key, label]) => (
          <span class="hint-key">
            <kbd>{key}</kbd>
            {label}
          </span>
        )}
      </For>
      <span class="spacer" />
      <span classList={{ warn: saveState() === 'local' || saveState() === 'conflict' }}>{SAVE_TEXT[saveState()]}</span>
    </footer>
  );
}

function Overlays() {
  return (
    <Show when={overlay()} keyed>
      {(o) => {
        switch (o.kind) {
          case 'symbol': {
            const panel = o.panelId ? doc.panels[o.panelId] : undefined;
            return (
              <SymbolSearch
                initial={o.initial}
                title={panel?.type === 'watchlist' ? 'Add to watchlist' : 'Symbol'}
                current={panel && panel.type !== 'watchlist' ? marketOf(panel) : undefined}
                ratios={panel?.type !== 'watchlist'}
                timeframe={panel && panel.type !== 'watchlist' ? timeframeOf(panel) : undefined}
                onClose={closeOverlay}
                onSelect={(market) => {
                  closeOverlay();
                  pickMarket(o.panelId, market, o.at);
                }}
              />
            );
          }
          case 'indicators': {
            const panel = doc.panels[o.panelId] ?? detached[o.panelId];
            if (panel?.type !== 'chart') return null;
            return (
              <IndicatorPicker
                active={splitList(panel.studies)}
                onClose={closeOverlay}
                onAdd={(study) => {
                  closeOverlay();
                  const list = splitList(panel.studies);
                  if (!list.includes(study)) patchPanel(panel.id, { studies: [...list, study].join(',') });
                }}
              />
            );
          }
          case 'tpoSettings': {
            const panel = doc.panels[o.panelId];
            return panel?.type === 'chart' ? <TpoSettings panel={panel} onClose={closeOverlay} /> : null;
          }
          case 'palette':
            return <CommandPalette commands={commands()} onClose={closeOverlay} />;
          case 'settings':
            return <Settings onClose={closeOverlay} />;
          case 'help':
            return <Help onClose={closeOverlay} />;
          case 'referrals':
            return <ReferralDialog onClose={closeOverlay} />;
          case 'note':
            return <NoteInput initial={o.initial} onClose={closeOverlay} onSave={o.save} />;
        }
      }}
    </Show>
  );
}

/**
 * The browser's own menu — Reload, Save as, Inspect — is never what is wanted
 * over a chart or a list, and it buries the app's own menus. Panels that have
 * one have already shown it by the time this runs. Text fields keep theirs,
 * where Paste is the point, and so do links, for Copy link address.
 */
function onContextMenu(e: MouseEvent): void {
  if (isTypingTarget(e.target)) return;
  if (e.target instanceof HTMLElement && e.target.closest('a[href]')) return;
  e.preventDefault();
}

/** Search engines and link previews, which render the page but do not trade from it. */
const IS_CRAWLER = /bot\b|crawler|spider|slurp|facebookexternalhit|bingpreview|embedly|preview/i.test(navigator.userAgent);

export function App() {
  const [engineError, setEngineError] = createSignal<string | null>(null);
  const [layoutMenu, setLayoutMenu] = createSignal(false);
  useDismiss(layoutMenu, () => setLayoutMenu(false));
  applyTheme();
  // The chart last used names the browser tab, with its live price. A search
  // engine's crawler keeps the page's own title, which says what the site is.
  const pageTitle = document.title;
  if (!IS_CRAWLER) {
    createEffect(() => {
      const chart = targetChart();
      document.title = chart ? tabTitle(marketOf(chart), lastPrice(chart.id)) : pageTitle;
    });
  }
  onMount(() => {
    const chart = targetChart();
    if (chart) focusPanel(chart.id);
    // A link from Telegram opens its chart once the layout is in; new signals are told as they come.
    void syncWithServer().then(() => /[?&](signal|market)=/.test(location.search) && void import('../lib/deeplink').then((m) => m.followLink()));
    startAlerts();
    // Signed out by the server or another browser: said once, with the way back.
    const notice = takeNotice();
    if (notice) toast({ tone: 'info', title: 'Signed out', body: notice, action: () => setOverlay({ kind: 'settings' }) }, 30_000);
    if (!IS_CRAWLER) {
      startReferral();
      setTimeout(() => void import('../lib/signal-alerts').then((m) => m.watchSignals()), 4_000);
    }
    addEventListener('keydown', onKeyDown);
    addEventListener('contextmenu', onContextMenu);
    // An error nobody caught is said, not swallowed: in an update it also ends
    // the rest of that update, and a phone has no console to read it in.
    const told = new Set<string>();
    addEventListener('error', (e) => {
      // Another site's script says only "Script error.", or names its own address.
      if ((e.filename && !e.filename.startsWith(location.origin)) || /^Script error/.test(e.message) || told.has(e.message)) return;
      told.add(e.message);
      toast({ tone: 'warn', title: 'Something went wrong', body: e.message }, 20_000);
    });
    const off = onEngineFailure(setEngineError);
    onCleanup(() => {
      removeEventListener('keydown', onKeyDown);
      removeEventListener('contextmenu', onContextMenu);
      off();
    });
  });

  return (
    <div class="shell">
      <header class="topbar">
        <div class="brand">
          <svg viewBox="0 0 16 16" width="19" height="19" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.1">
            <rect x="1.8" y="3" width="7.6" height="10" rx="1" stroke-opacity="0.8" />
            <path d="M9.4 3 L13.6 1.2 L13.6 14.8 L9.4 13 Z" fill="currentColor" fill-opacity="0.32" stroke-linejoin="round" />
            <rect x="3.3" y="7.6" width="1.7" height="3.4" fill="currentColor" stroke="none" />
            <rect x="6.2" y="5.6" width="1.7" height="5.2" fill="currentColor" stroke="none" />
          </svg>
          <span>TRADOOR</span>
        </div>
        <button class="item" data-tip="Commands (/ or Ctrl K)" onClick={() => setOverlay({ kind: 'palette' })}>
          <Icon name="command" size={12} /> <span>Commands</span> <kbd>/</kbd>
        </button>
        <button class="item" classList={{ on: journalOpen() }} data-tip="Trading journal: every long and short position you draw" onClick={() => (setTrainingOpen(false), setJournalOpen(!journalOpen()))}>
          <Icon name="pencil" size={12} /> <span>Journal</span>
        </button>
        <button class="item" classList={{ on: trainingOpen() }} data-tip="Paper trading at live prices" onClick={() => (setJournalOpen(false), setTrainingOpen(!trainingOpen()))}>
          <Icon name="indicators" size={12} /> <span>Training</span>
        </button>
        <span class="layout-at">
          <button class="item" classList={{ on: layoutMenu() }} data-tip="Charts on screen: a layout of one to twenty" onPointerDown={(e) => e.stopPropagation()} onClick={() => setLayoutMenu(!layoutMenu())}>
            <Icon name="layout" size={12} /> <span>{doc.name}</span>
          </button>
          <Show when={layoutMenu()}>
            <LayoutPicker onClose={() => setLayoutMenu(false)} />
          </Show>
        </span>
        <span class="spacer" />
        <button class="item ref-btn" data-tip="Partner exchanges: Nado, Bybit and Binance" onClick={() => setOverlay({ kind: 'referrals' })}>
          <Icon name="gift" size={13} /> <span>Rewards</span>
        </button>
        <Health />
        <button class="item icon alerts-btn" data-tip="Price alerts" onClick={showAlertsPanel}>
          <Icon name="alarm" size={13} />
          <Show when={unread() > 0}>
            <span class="al-badge">{unread()}</span>
          </Show>
        </button>
        <Clock />
        <button class="item icon" data-tip="Settings and colours" onClick={() => setOverlay({ kind: 'settings' })}>
          <Icon name="settings" size={16} />
        </button>
        <button class="item icon" data-tip="Keyboard shortcuts (?)" onClick={() => setOverlay({ kind: 'help' })}>
          <span>?</span>
        </button>
      </header>
      <main style={{ display: 'flex', 'flex-direction': 'column', 'min-height': '0', overflow: 'hidden', position: 'relative' }}>
        <Show when={engineError()}>{(message) => <div class="banner">▲ Charts cannot be drawn: {message()}</div>}</Show>
        <Layout />
        <Show when={journalOpen()}>
          <Journal />
        </Show>
        <Show when={trainingOpen()}>
          <Training chart={ChartPanel} />
        </Show>
      </main>
      <StatusBar />
      <Overlays />
      {/* Offered once, and never over a dialog, the phone's menu or the journal. */}
      <Show when={referralCard() && !overlay() && !mobileMenu() && !journalOpen() && !trainingOpen()}>
        <ReferralCard />
      </Show>
      <Toasts />
    </div>
  );
}
