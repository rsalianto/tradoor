/**
 * The terminal layout: splits with draggable gutters, tab groups, and docking
 * by dragging a tab onto another group's centre or edge — or onto a tab
 * strip, between its tabs, which reorders them.
 */
import { createEffect, createSignal, For, lazy, Match, on, onCleanup, Show, Switch, type JSX } from 'solid-js';
import { chartControllers, controllersChanged, covered, dragging, mobileMenu, setDragging, setJournalOpen, setMobile, setMobileMenu, setOverlay, setSharedRail, setTrainingOpen } from '../app/ui';
import { DrawingRail } from '../panels/DrawingRail';
import { useDismiss } from '../lib/dismiss';
import { unread } from '../lib/alerts';
import { Icon } from '../lib/icons';
import { symbolOf } from '../lib/markets';
import { tfLabel } from '../lib/timeframes';
import { ChartPanel } from '../panels/ChartPanel';
import { WatchlistPanel } from '../panels/WatchlistPanel';
import { activate, addPanel, bareChart, LINK_COLORS, movePanel, newPanel, PANEL_TITLE, PANEL_TYPES, removePanel, setSizes, tabsOf, type DropZone, type LayoutNode, type Panel, type PanelType, type SplitNode, type TabsNode } from './model';
import { doc, focusedPanelId, focusPanel, marketOf, setPanelLink, targetChart, timeframeOf, update } from './store';

// Side panels, fetched when a layout shows one: the chart and watchlist come first.
const ValuesPanel = lazy(() => import('../panels/ValuesPanel').then((m) => ({ default: m.ValuesPanel })));
const AlertsPanel = lazy(() => import('../panels/AlertsPanel').then((m) => ({ default: m.AlertsPanel })));
const LiquidationsPanel = lazy(() => import('../panels/LiquidationsPanel').then((m) => ({ default: m.LiquidationsPanel })));
const FlowPanel = lazy(() => import('../panels/FlowPanel').then((m) => ({ default: m.FlowPanel })));
const SignalsPanel = lazy(() => import('../panels/SignalsPanel').then((m) => ({ default: m.SignalsPanel })));
const DomPanel = lazy(() => import('../panels/DomPanel').then((m) => ({ default: m.DomPanel })));
const TapePanel = lazy(() => import('../panels/TapePanel').then((m) => ({ default: m.TapePanel })));

/** Smallest a cell may be dragged to, in pixels. */
const MIN_CELL = 90;

export function panelTitle(p: Panel): string {
  if (p.type === 'watchlist') return 'Watchlist';
  if (p.type === 'alerts') return 'Alerts';
  // Flow and Signal list what the server follows, whatever chart they drive.
  if (p.type === 'flow') return 'Flow';
  if (p.type === 'signals') return 'Signal';
  // Every trade and the book now: no timeframe to them.
  if (p.type === 'dom' || p.type === 'tape') return `${PANEL_TITLE[p.type]} · ${symbolOf(marketOf(p))}`;
  const label = `${symbolOf(marketOf(p))} ${tfLabel(timeframeOf(p))}`;
  if (p.type === 'chart') return p.tpo ? `TPO · ${label}` : label;
  return `${PANEL_TITLE[p.type]} · ${label}`;
}

/** Phones and narrow windows: one chart and a menu, not splits. A phone on its
 *  side is wider than that but short, and touch: still one chart, not twenty
 *  too small to read. The same query is in styles.css. */
const MOBILE_QUERY = '(max-width: 760px), (max-height: 500px) and (pointer: coarse)';

function useMedia(query: string): () => boolean {
  const media = matchMedia(query);
  const [matches, setMatches] = createSignal(media.matches);
  const change = () => setMatches(media.matches);
  media.addEventListener('change', change);
  onCleanup(() => media.removeEventListener('change', change));
  return matches;
}

/** Charts on screen: a tab group shows the one panel it has open. */
const shownCharts = (node: LayoutNode): number =>
  node.kind === 'tabs' ? (doc.panels[node.active]?.type === 'chart' ? 1 : 0) : node.children.reduce((n, child) => n + shownCharts(child), 0);

export function Layout() {
  const narrow = useMedia(MOBILE_QUERY);
  createEffect(() => setMobile(narrow()));
  onCleanup(() => setMobile(false));
  // Several charts share one rail at the left, acting on the chart in use.
  const shared = () => !narrow() && shownCharts(doc.root) > 1;
  createEffect(() => setSharedRail(shared()));
  onCleanup(() => setSharedRail(false));
  return (
    <div class="workspace" classList={{ mobile: narrow(), covered: covered() }}>
      <Show when={shared()}>
        <DrawingRail target={() => (controllersChanged(), chartControllers.get(targetChart()?.id ?? ''))} />
      </Show>
      <Show when={narrow()} fallback={<NodeView node={doc.root} />}>
        <MobileShell />
      </Show>
    </div>
  );
}

/**
 * A phone shows one chart and nothing else. The panels a wide screen keeps
 * beside it wait behind the menu button in the chart's toolbar, and open over
 * the chart until they are closed — the shape every phone charting app has,
 * for the reason all of them have it: splits and tab strips want a mouse and
 * a wide screen, and on a phone they leave no chart to read.
 *
 * The saved layout is untouched: turn the phone or widen the window and the
 * splits are back as they were. A panel the layout never had is added as a tab
 * beside the chart the first time it is asked for, which is where it belongs
 * when that window is wide again.
 */
function MobileShell() {
  const [sheet, setSheet] = createSignal<string | null>(null);
  const charts = (): Panel[] => Object.values(doc.panels).filter((p) => p.type === 'chart');
  const chart = (): Panel | null => targetChart() ?? charts()[0] ?? null;
  const open = () => {
    const id = sheet();
    return id ? doc.panels[id] : undefined;
  };

  // Choosing a market in the watchlist is what the sheet was opened for, so it
  // closes itself rather than leaving the chart behind it unseen.
  createEffect(on(() => { const c = chart(); return c && marketOf(c); }, () => setSheet(null), { defer: true }));

  const show = (type: PanelType) => {
    setMobileMenu(false);
    const held = Object.values(doc.panels).find((p) => p.type === type);
    if (held) {
      setSheet(held.id);
      return;
    }
    const from = chart();
    const panel = newPanel(type, from ? { link: from.link, market: marketOf(from), timeframe: timeframeOf(from) } : undefined);
    update((d) => addPanel(d, panel, from ? (tabsOf(d.root, from.id)?.id ?? null) : null, 'center'));
    setSheet(panel.id);
  };

  // A workspace with no chart has nowhere to put the menu button, which the
  // chart's toolbar carries: it gets a chart, and the phone its top row back.
  const addChart = () => {
    const panel = newPanel('chart');
    update((d) => addPanel(d, panel, null, 'center'));
    focusPanel(panel.id);
  };

  const overlay = (kind: 'palette' | 'settings' | 'help' | 'referrals') => () => {
    setMobileMenu(false);
    setOverlay({ kind });
  };

  return (
    <>
      {/* Keyed by id: a chart chosen from the menu is that chart, mounted afresh;
          the one left keeps its bars and lets go of its pictures (engine/worker). */}
      <Show
        when={chart()?.id}
        keyed
        fallback={
          <div class="mempty">
            <p>This workspace has no chart.</p>
            <button class="tb" onClick={addChart}>Open one</button>
          </div>
        }
      >
        {(id) => (
          <div class="panelbody" onPointerDown={() => focusPanel(id)}>
            <PanelView panel={doc.panels[id]!} />
          </div>
        )}
      </Show>

      <Show when={open()}>
        {(panel) => (
          <section class="msheet">
            <header class="mhead">
              <span class="grow">{panelTitle(panel())}</span>
              <button class="iconbtn" aria-label="Close" onClick={() => setSheet(null)}>
                <Icon name="close" size={15} />
              </button>
            </header>
            <div class="panelbody">
              <PanelView panel={panel()} />
            </div>
          </section>
        )}
      </Show>

      <Show when={mobileMenu()}>
        <div class="mscrim" onClick={() => setMobileMenu(false)} />
        <nav class="mdrawer">
          {/* Charts have their own section below: `tpo` is one of them, and
              asking for it here would make a new one every time. */}
          <For each={PANEL_TYPES.filter((t) => t !== 'chart' && t !== 'tpo')}>
            {(type) => (
              <button onClick={() => show(type)}>
                <span class="grow">{PANEL_TITLE[type]}</span>
                <Show when={type === 'alerts' && unread() > 0}>
                  <span class="al-badge">{unread()}</span>
                </Show>
              </button>
            )}
          </For>
          <Show when={charts().length > 1}>
            <hr />
            <div class="cap">Charts</div>
            <For each={charts()}>
              {(c) => (
                <button
                  classList={{ cur: c.id === chart()?.id }}
                  onClick={() => {
                    focusPanel(c.id);
                    setMobileMenu(false);
                  }}
                >
                  {panelTitle(c)}
                </button>
              )}
            </For>
          </Show>
          <hr />
          <button
            onClick={() => {
              setTrainingOpen(false);
              setJournalOpen(true);
              setMobileMenu(false);
            }}
          >
            Journal
          </button>
          <button
            onClick={() => {
              setJournalOpen(false);
              setTrainingOpen(true);
              setMobileMenu(false);
            }}
          >
            Training
          </button>
          <button onClick={overlay('palette')}>Commands</button>
          <button onClick={overlay('settings')}>Settings</button>
          <button onClick={overlay('help')}>Shortcuts</button>
          <hr />
          <button class="ref-item" onClick={overlay('referrals')}>
            <Icon name="gift" size={15} />
            <span class="grow">Partner exchanges</span>
          </button>
        </nav>
      </Show>
    </>
  );
}

function NodeView(props: { node: LayoutNode }) {
  return (
    <Switch>
      <Match when={props.node.kind === 'split' && (props.node as SplitNode)}>{(node) => <SplitView node={node()} />}</Match>
      <Match when={props.node.kind === 'tabs' && (props.node as TabsNode)}>{(node) => <TabsView node={node()} />}</Match>
    </Switch>
  );
}

function SplitView(props: { node: SplitNode }) {
  let el!: HTMLDivElement;
  /** Sizes while a gutter is being dragged; committed once, on release. */
  const [live, setLive] = createSignal<number[] | null>(null);
  const [active, setActive] = createSignal(-1);
  const sizes = () => live() ?? props.node.sizes;

  const startDrag = (index: number, e: PointerEvent) => {
    e.preventDefault();
    const gutter = e.currentTarget as HTMLElement;
    gutter.setPointerCapture(e.pointerId);
    const row = props.node.dir === 'row';
    const rect = el.getBoundingClientRect();
    const total = row ? rect.width : rect.height;
    const start = row ? e.clientX : e.clientY;
    const initial = [...props.node.sizes];
    const pair = initial[index]! + initial[index + 1]!;
    const min = Math.min(pair / 2, MIN_CELL / total);
    setActive(index);
    const move = (ev: PointerEvent) => {
      const delta = ((row ? ev.clientX : ev.clientY) - start) / total;
      const a = Math.max(min, Math.min(pair - min, initial[index]! + delta));
      const next = [...initial];
      next[index] = a;
      next[index + 1] = pair - a;
      setLive(next);
    };
    const end = () => {
      gutter.removeEventListener('pointermove', move);
      gutter.removeEventListener('pointerup', end);
      gutter.removeEventListener('pointercancel', end);
      const final = live();
      setLive(null);
      setActive(-1);
      if (final) update((d) => setSizes(d, props.node.id, final));
    };
    gutter.addEventListener('pointermove', move);
    gutter.addEventListener('pointerup', end);
    gutter.addEventListener('pointercancel', end);
  };

  return (
    <div ref={el} class={`split ${props.node.dir}`}>
      <For each={props.node.children}>
        {(child, i) => (
          <>
            <Show when={i() > 0}>
              <div class="gutter" classList={{ active: active() === i() - 1 }} onPointerDown={(e) => startDrag(i() - 1, e)} />
            </Show>
            <div class="cell" style={{ flex: `${sizes()[i()] ?? 1} 1 0` }}>
              <NodeView node={child} />
            </div>
          </>
        )}
      </For>
    </div>
  );
}

const HINT: Record<DropZone, JSX.CSSProperties> = {
  center: { inset: '0' },
  left: { left: '0', top: '0', bottom: '0', width: '50%' },
  right: { right: '0', top: '0', bottom: '0', width: '50%' },
  top: { left: '0', right: '0', top: '0', height: '50%' },
  bottom: { left: '0', right: '0', bottom: '0', height: '50%' },
};

function zoneAt(e: DragEvent, el: HTMLElement): DropZone {
  const r = el.getBoundingClientRect();
  const x = (e.clientX - r.left) / r.width;
  const y = (e.clientY - r.top) / r.height;
  const edge = Math.min(x, 1 - x, y, 1 - y);
  if (edge > 0.22) return 'center';
  if (edge === x) return 'left';
  if (edge === 1 - x) return 'right';
  return edge === y ? 'top' : 'bottom';
}

function TabsView(props: { node: TabsNode }) {
  const [zone, setZone] = createSignal<DropZone | null>(null);
  /** Over the tab strip: the place among the tabs a tab let go would take, and where to mark it. */
  const [slot, setSlot] = createSignal<{ at: number; x: number } | null>(null);
  let list!: HTMLDivElement;
  const slotAt = (e: DragEvent) => {
    const box = list.getBoundingClientRect();
    const tabs = [...list.querySelectorAll<HTMLElement>(':scope > .tab')].map((t) => t.getBoundingClientRect());
    const at = tabs.findIndex((r) => e.clientX < r.left + r.width / 2);
    const edge = at === -1 ? (tabs.at(-1)?.right ?? box.left) : tabs[at]!.left;
    return { at: at === -1 ? tabs.length : at, x: edge - box.left + list.scrollLeft };
  };
  const [adding, setAdding] = createSignal(false);
  useDismiss(adding, () => setAdding(false));
  const active = () => doc.panels[props.node.active];
  const focused = () => props.node.panels.includes(focusedPanelId() ?? '');

  const drop = (where: DropZone, at?: number) => {
    const id = dragging();
    setZone(null);
    setSlot(null);
    setDragging(null);
    if (id) update((d) => movePanel(d, id, props.node.id, where, at));
  };

  const addHere = (type: PanelType, where: DropZone) => {
    setAdding(false);
    const from = active();
    // A new chart starts on its own, on the ticker it came from: many charts
    // are many tickers, and a link would turn them all with the first one
    // changed. Beside another it is bare. TPO and the side panels follow the
    // chart they came from.
    const at = from ? { link: type === 'chart' ? null : from.link, market: marketOf(from), timeframe: timeframeOf(from) } : undefined;
    const panel = type === 'chart' && Object.values(doc.panels).some((p) => p.type === 'chart') ? bareChart(at) : newPanel(type, at);
    update((d) => addPanel(d, panel, props.node.id, where));
    focusPanel(panel.id);
  };

  return (
    <div class="tabs" classList={{ focused: focused() }} onPointerDown={() => focusPanel(props.node.active)}>
      <div
        class="tabstrip"
        onDragOver={(e) => {
          if (!dragging()) return;
          e.preventDefault();
          setZone(null);
          setSlot(slotAt(e));
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setSlot(null);
        }}
        onDrop={(e) => {
          e.preventDefault();
          drop('center', slotAt(e).at);
        }}
      >
        <div class="tablist" ref={list}>
          <For each={props.node.panels}>{(id) => <Tab id={id} tabs={props.node} />}</For>
          <Show when={dragging() ? slot() : null}>{(s) => <div class="tabslot" style={{ left: `${s().x - 1}px` }} />}</Show>
        </div>
        <div class="tabtools">
          <button class="iconbtn" data-tip="Add a panel to this group" onPointerDown={(e) => e.stopPropagation()} onClick={() => setAdding((v) => !v)}>
            <Icon name="plus" size={12} />
          </button>
          <button class="iconbtn" data-tip="Split right" onClick={() => addHere(active()?.type ?? 'chart', 'right')}>
            <Icon name="splitRight" size={12} />
          </button>
          <button class="iconbtn" data-tip="Split down" onClick={() => addHere(active()?.type ?? 'chart', 'bottom')}>
            <Icon name="splitDown" size={12} />
          </button>
          <Show when={adding()}>
            <div class="menu" style={{ right: '0', top: '21px' }} onPointerDown={(e) => e.stopPropagation()}>
              <For each={PANEL_TYPES}>
                {(type) => (
                  <button onClick={() => addHere(type, 'center')}>
                    {PANEL_TITLE[type]}
                    <span class="dim">new tab</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      </div>
      <div class="panelbody">
        <Show when={active()} keyed>
          {(panel) => <PanelView panel={panel} />}
        </Show>
        <Show when={dragging()}>
          <div
            class="dropcover"
            onDragOver={(e) => {
              e.preventDefault();
              setZone(zoneAt(e, e.currentTarget));
            }}
            onDragLeave={() => setZone(null)}
            onDrop={(e) => {
              e.preventDefault();
              drop(zoneAt(e, e.currentTarget));
            }}
          >
            <Show when={zone()}>{(z) => <div class="drophint" style={HINT[z()]} />}</Show>
          </div>
        </Show>
      </div>
    </div>
  );
}

function Tab(props: { id: string; tabs: TabsNode }) {
  const panel = () => doc.panels[props.id];
  return (
    <Show when={panel()}>
      {(p) => (
        <div
          class="tab"
          classList={{ active: props.tabs.active === props.id }}
          draggable={true}
          data-tip={`${panelTitle(p())} — drag onto another group, or its edge, to dock it there`}
          onDragStart={(e) => {
            e.dataTransfer?.setData('text/plain', props.id);
            if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
            // Changing the page during dragstart cancels the drag in Chromium.
            requestAnimationFrame(() => setDragging(props.id));
          }}
          onDragEnd={() => setDragging(null)}
          onClick={() => {
            if (props.tabs.active !== props.id) update((d) => activate(d, props.tabs.id, props.id));
            focusPanel(props.id);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) update((d) => removePanel(d, props.id));
          }}
        >
          <LinkButton panel={p()} />
          <span class="label">{panelTitle(p())}</span>
          <Show when={p().type === 'alerts' && unread() > 0}>
            <span class="al-badge">{unread()}</span>
          </Show>
          <button
            class="close"
            data-tip="Close panel"
            onClick={(e) => {
              e.stopPropagation();
              update((d) => removePanel(d, props.id));
            }}
          >
            <Icon name="close" size={10} stroke={2.2} />
          </button>
        </div>
      )}
    </Show>
  );
}

/** Click to cycle through the link groups: panels in one group share a market. */
function LinkButton(props: { panel: Panel }) {
  const next = () => {
    const i = props.panel.link ? LINK_COLORS.indexOf(props.panel.link) : -1;
    return LINK_COLORS[i + 1] ?? null;
  };
  return (
    <button
      class="linkbtn"
      classList={{ linked: props.panel.link !== null }}
      style={props.panel.link ? { color: `var(--link-${props.panel.link})` } : undefined}
      data-tip={props.panel.link ? `Linked to the ${props.panel.link} group — click for the next group` : 'Not linked — click to join a group that shares one market'}
      onClick={(e) => {
        e.stopPropagation();
        setPanelLink(props.panel.id, next());
      }}
    >
      <Icon name="link" size={11} stroke={2.2} />
    </button>
  );
}

function PanelView(props: { panel: Panel }) {
  const p = props.panel;
  switch (p.type) {
    case 'chart':
      return <ChartPanel panel={p} />;
    case 'watchlist':
      return <WatchlistPanel panel={p} />;
    case 'values':
      return <ValuesPanel panel={p} />;
    case 'liquidations':
      return <LiquidationsPanel panel={p} />;
    case 'flow':
      return <FlowPanel panel={p} />;
    case 'signals':
      return <SignalsPanel panel={p} />;
    case 'alerts':
      return <AlertsPanel panel={p} />;
    case 'dom':
      return <DomPanel panel={p} />;
    case 'tape':
      return <TapePanel panel={p} />;
  }
}
