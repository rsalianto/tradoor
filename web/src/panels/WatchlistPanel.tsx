/**
 * Markets worth a glance: last price, the day's change and volume, streamed.
 * Clicking a row drives the linked group, or the last chart used. The list never
 * reorders itself — knowing where each symbol sits is the point of a fixed
 * column — but it can be split into sections, flagged and dragged into order.
 *
 * Change and volume are the current daily candle's, as TradingView's are: since
 * 00:00 UTC, not a rolling 24 hours, which is what exchange tickers report.
 */
import { batch, createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { createStore } from 'solid-js/store';
import { mobile, setOverlay } from '../app/ui';
import { call } from '../engine/client';
import { dayOf, watchDay, type Day } from '../lib/day-feed';
import { catalog, type Catalog } from '../lib/api';
import { useDismiss } from '../lib/dismiss';
import { formatPct, formatPrice, formatVolume, priceDecimals } from '../lib/format';
import { Icon } from '../lib/icons';
import { ExchangeLogo, exchangeOfVenue, kindLabel } from '../lib/exchanges';
import { parseMarket, symbolOf, venueLabel } from '../lib/markets';
import { blockOf, FLAG_COLORS, FLAGS, freshName, isSection, move, SECTION_NAME_MAX, sectionEnd, sectionName, sectionStops, visible } from '../lib/watchlist';
import type { WatchlistPanelState } from '../workspace/model';
import { doc, marketOf, patchPanel, setLinkMarket, setPanelMarket, targetChart, timeframeOf } from '../workspace/store';

interface Row {
  last: number;
  /** The daily candle's, once one has arrived. */
  day: Day | null;
}

interface Drag {
  from: number;
  label: string;
  x: number;
  y: number;
  /** Where it would land, as an index into the list; null where it would not move. */
  to: number | null;
  lineY: number;
}

/** How long the pointer rests on a row before its market is fetched. */
const HOVER_MS = 120;

/** How far a press travels before it is a drag rather than a click. */
const DRAG_PX = 4;

export function WatchlistPanel(props: { panel: WatchlistPanelState }) {
  const id = props.panel.id;
  const [rows, setRows] = createStore<Record<string, Row>>({});
  const [products, setProducts] = createSignal<Catalog | null>(null);
  const [menu, setMenu] = createSignal<{ x: number; y: number; item: string | null } | null>(null);
  const [editing, setEditing] = createSignal<string | null>(null);
  const [drag, setDrag] = createSignal<Drag | null>(null);
  let list!: HTMLDivElement;

  useDismiss(menu, () => setMenu(null));
  catalog().then(setProducts, () => {});

  const items = () => props.panel.markets;
  const markets = () => items().filter((m) => !isSection(m));
  const collapsed = () => props.panel.collapsed ?? [];
  const flags = () => props.panel.flags ?? {};
  /** A section being dragged: every section shows folded meanwhile, so it
   *  moves among their headers as a market moves among rows. */
  const [folding, setFolding] = createSignal(false);
  const shown = () => visible(items(), folding() ? items().filter(isSection).map(sectionName) : collapsed()).map((i) => items()[i]!);

  // Seed the price from the catalogue so rows are not blank until the first tick.
  createEffect(() => {
    const c = products();
    if (!c) return;
    for (const m of markets()) {
      const p = c.byMarket.get(m);
      if (p?.lastPrice != null && !rows[m]) setRows(m, { last: p.lastPrice, day: null });
    }
  });

  // One daily-candle stream per market, kept while the market stays listed.
  const subs = new Map<string, () => void>();
  createEffect(() => {
    const want = new Set(markets());
    for (const [m, off] of subs) {
      if (!want.has(m)) {
        off();
        subs.delete(m);
      }
    }
    for (const m of want) {
      if (!subs.has(m)) {
        // What the page already knows, so a panel mounted afresh shows numbers
        // at once rather than dashes until the market trades again.
        const known = dayOf(m);
        if (known) setRows(m, { last: known.close, day: known });
        // Off a phone, the price follows every trade as a chart's does; a phone keeps to the lighter candle.
        subs.set(m, watchDay(m, (day) => setRows(m, { last: day.close, day }), !mobile()));
      }
    }
  });
  onCleanup(() => {
    for (const off of subs.values()) off();
    subs.clear();
  });

  /** A market under the pointer is fetched before it is clicked. */
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  const asked = new Set<string>();
  const warm = (market: string) => {
    clearTimeout(hoverTimer);
    const chart = targetChart();
    if (!chart || asked.has(market)) return;
    hoverTimer = setTimeout(() => {
      asked.add(market);
      void call('prefetch', { market, timeframe: timeframeOf(chart) }).catch(() => asked.delete(market));
    }, HOVER_MS);
  };
  onCleanup(() => clearTimeout(hoverTimer));

  const current = (): string | null => {
    if (props.panel.link) return doc.links[props.panel.link].market;
    const chart = targetChart();
    return chart ? marketOf(chart) : null;
  };

  const open = (market: string) => {
    if (props.panel.link) {
      setLinkMarket(props.panel.link, market);
      return;
    }
    const chart = targetChart();
    if (chart) setPanelMarket(chart.id, market);
  };

  const changePct = (r: Row | undefined): number | null => (r?.day ? ((r.last - r.day.open) / r.day.open) * 100 : null);
  const digits = (market: string, last: number) => products()?.byMarket.get(market)?.priceDecimals ?? priceDecimals(last);

  // ------------------------------------------------------------ editing the list

  const setFlag = (market: string, color: string | null) => {
    const next = { ...flags() };
    if (color && next[market] !== color) next[market] = color;
    else delete next[market];
    patchPanel(id, { flags: next });
  };

  const remove = (item: string) => {
    if (isSection(item)) {
      // The section goes; its symbols stay, in the section above.
      patchPanel(id, { markets: items().filter((m) => m !== item), collapsed: collapsed().filter((n) => n !== sectionName(item)) });
      return;
    }
    const next = { ...flags() };
    delete next[item];
    patchPanel(id, { markets: items().filter((m) => m !== item), flags: next });
  };

  /** A new section above `before` (the end by default), named at once. */
  const addSection = (before: string | null) => {
    const name = freshName(items());
    const list = [...items()];
    const at = before === null ? list.length : list.indexOf(before);
    list.splice(at === -1 ? list.length : at, 0, `#${name}`);
    patchPanel(id, { markets: list });
    setEditing(name);
  };

  const rename = (from: string, to: string) => {
    setEditing(null);
    const name = to.trim().slice(0, SECTION_NAME_MAX);
    if (!name || name === from || items().includes(`#${name}`)) return;
    patchPanel(id, {
      markets: items().map((m) => (m === `#${from}` ? `#${name}` : m)),
      collapsed: collapsed().map((n) => (n === from ? name : n)),
    });
  };

  const toggle = (name: string) => {
    const c = collapsed();
    patchPanel(id, { collapsed: c.includes(name) ? c.filter((n) => n !== name) : [...c, name] });
  };

  const addSymbol = (at: number) => setOverlay({ kind: 'symbol', panelId: id, initial: '', at });

  // ------------------------------------------------------------ dragging

  let press: { from: number; x: number; y: number; started: boolean } | null = null;
  let dragged = false;

  /** Where a drop at `y` (viewport) lands, and where the line for it goes in the list. */
  const dropAt = (from: number, y: number): { to: number | null; lineY: number } => {
    const box = list.getBoundingClientRect();
    const els = [...list.querySelectorAll<HTMLElement>('[data-item]')];
    const [start, end] = blockOf(items(), from);
    const bottom = els.length ? els[els.length - 1]!.getBoundingClientRect().bottom : box.top;
    const landing = (to: number, edge: number) => ({ to: to >= start && to <= end ? null : to, lineY: edge - box.top + list.scrollTop });
    // A section carries its markets, so it lands between whole sections, never
    // among another's rows. Every section is folded while it moves, so the
    // places it may land are rows on screen, chosen as a market's are: the
    // first whose middle is below the pointer.
    if (isSection(items()[from]!)) {
      for (const to of sectionStops(items())) {
        const r = els.find((el) => el.dataset.item === items()[to])?.getBoundingClientRect();
        if (r && y < r.top + r.height / 2) return landing(to, r.top);
      }
      return landing(items().length, bottom);
    }
    let to = items().length;
    let edge = bottom;
    for (const el of els) {
      const r = el.getBoundingClientRect();
      if (y < r.top + r.height / 2) {
        to = items().indexOf(el.dataset.item!);
        edge = r.top;
        break;
      }
    }
    return landing(to, edge);
  };

  const onMove = (e: PointerEvent) => {
    if (!press) return;
    if (!press.started) {
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_PX) return;
      press.started = true;
      setMenu(null);
      if (isSection(items()[press.from]!)) setFolding(true);
    }
    // Near an edge, the list scrolls to reach rows out of view.
    const box = list.getBoundingClientRect();
    if (e.clientY < box.top + 20) list.scrollTop -= 8;
    else if (e.clientY > box.bottom - 20) list.scrollTop += 8;
    const item = items()[press.from]!;
    setDrag({ from: press.from, label: isSection(item) ? sectionName(item).toUpperCase() : symbolOf(item), x: e.clientX, y: e.clientY, ...dropAt(press.from, e.clientY) });
  };

  const onUp = () => {
    removeEventListener('pointermove', onMove);
    removeEventListener('pointerup', onUp);
    removeEventListener('pointercancel', onUp);
    const d = drag();
    if (press?.started) {
      dragged = true;
      // The click this release makes is not a click on the row.
      setTimeout(() => (dragged = false));
    }
    press = null;
    batch(() => {
      setDrag(null);
      setFolding(false);
      if (d && d.to !== null) patchPanel(id, { markets: move(items(), d.from, d.to) });
    });
  };

  const onDown = (e: PointerEvent, item: string) => {
    if (e.button !== 0 || editing() !== null || (e.target as HTMLElement).closest('button, input')) return;
    press = { from: items().indexOf(item), x: e.clientX, y: e.clientY, started: false };
    addEventListener('pointermove', onMove);
    addEventListener('pointerup', onUp);
    addEventListener('pointercancel', onUp);
  };
  onCleanup(onUp);

  const openMenu = (e: MouseEvent, item: string | null) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: Math.min(e.clientX, innerWidth - 210), y: Math.min(e.clientY, innerHeight - 240), item });
  };

  const run = (f: () => void) => () => {
    setMenu(null);
    f();
  };

  /** Rows that would move with the drag under way. */
  const moving = (item: string): boolean => {
    const d = drag();
    if (!d) return false;
    const [start, end] = blockOf(items(), d.from);
    const i = items().indexOf(item);
    return i >= start && i < end;
  };

  const sectionCount = (item: string) => {
    const i = items().indexOf(item);
    return sectionEnd(items(), i) - i - 1;
  };

  return (
    <div class="panelcol wl">
      <div class="toolbar">
        <span class="caps" style={{ padding: '0 4px' }}>
          Watchlist
        </span>
        <span class="dim">{markets().length}</span>
        <span class="spacer" />
        <button class="iconbtn" data-tip="Add a section" onClick={() => addSection(null)}>
          <Icon name="section" size={13} />
        </button>
        <button class="iconbtn" data-tip="Add a market" onClick={() => addSymbol(items().length)}>
          <Icon name="plus" size={13} />
        </button>
      </div>
      <div class="wl-head">
        <span>Symbol</span>
        <span class="r">Last</span>
        <span class="r chg" data-tip="Change since the day's open, 00:00 UTC — as TradingView counts it">
          Chg%
        </span>
        <span class="r vol" data-tip="Volume since 00:00 UTC, in the base coin">
          Vol
        </span>
      </div>
      <div class="scroll wl-list" ref={list} onContextMenu={(e) => openMenu(e, null)}>
        <For each={shown()}>
          {(item) =>
            isSection(item) ? (
              <div
                class="wl-section"
                data-item={item}
                classList={{ dragging: moving(item) }}
                onPointerDown={(e) => onDown(e, item)}
                onClick={() => !dragged && editing() === null && toggle(sectionName(item))}
                onDblClick={() => setEditing(sectionName(item))}
                onContextMenu={(e) => openMenu(e, item)}
              >
                <span class="chev" classList={{ shut: collapsed().includes(sectionName(item)) }}>
                  <Icon name="chevron" size={11} stroke={2.2} />
                </span>
                <Show when={editing() === sectionName(item)} fallback={<span class="name">{sectionName(item)}</span>}>
                  <input
                    class="wl-rename"
                    value={sectionName(item)}
                    maxLength={SECTION_NAME_MAX}
                    ref={(el) =>
                      setTimeout(() => {
                        el.focus();
                        el.select();
                      })
                    }
                    onClick={(e) => e.stopPropagation()}
                    onBlur={(e) => rename(sectionName(item), e.currentTarget.value)}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === 'Enter') e.currentTarget.blur();
                      if (e.key === 'Escape') {
                        e.currentTarget.value = sectionName(item);
                        e.currentTarget.blur();
                      }
                    }}
                  />
                </Show>
                <span class="dim num">{sectionCount(item)}</span>
              </div>
            ) : (
              <div
                class="wl-row"
                data-item={item}
                classList={{ active: item === current(), dragging: moving(item) }}
                data-tip={`${symbolOf(item)} · ${venueLabel(item)}`}
                onPointerDown={(e) => onDown(e, item)}
                onPointerEnter={() => warm(item)}
                onPointerLeave={() => clearTimeout(hoverTimer)}
                onClick={() => !dragged && open(item)}
                onContextMenu={(e) => openMenu(e, item)}
              >
                <Show when={flags()[item]}>{(c) => <span class="flag" style={{ background: FLAG_COLORS[c()] }} />}</Show>
                <span class="sym">
                  <ExchangeLogo exchange={exchangeOfVenue(parseMarket(item)?.venue ?? 'binance')} size={13} />
                  <span class="name">{symbolOf(item)}</span>
                  <span class="kind">{kindLabel(item)}</span>
                </span>
                <span class="num r muted">{rows[item] ? formatPrice(rows[item]!.last, digits(item, rows[item]!.last)) : '—'}</span>
                <span class="num r chg" classList={{ bull: (changePct(rows[item]) ?? 0) > 0, bear: (changePct(rows[item]) ?? 0) < 0, dim: changePct(rows[item]) === null }}>
                  {formatPct(changePct(rows[item]))}
                </span>
                <span class="num r vol muted">{formatVolume(rows[item]?.day?.volume)}</span>
                <button
                  class="x"
                  data-tip="Remove from the watchlist"
                  onClick={(e) => {
                    e.stopPropagation();
                    remove(item);
                  }}
                >
                  <Icon name="close" size={10} stroke={2.4} />
                </button>
              </div>
            )
          }
        </For>
        <Show when={items().length === 0}>
          <div class="empty">Empty. Add a market with + above.</div>
        </Show>
        <Show when={drag()?.to != null}>
          <div class="wl-drop" style={{ top: `${Math.max(0, drag()!.lineY - 1)}px` }} />
        </Show>
      </div>
      <Show when={drag()}>
        {(d) => (
          <div class="wl-ghost" style={{ left: `${d().x + 12}px`, top: `${d().y - 10}px` }}>
            {d().label}
          </div>
        )}
      </Show>
      <Show when={menu()}>
        {(m) => {
          const item = m().item;
          const at = item === null ? items().length : items().indexOf(item);
          const anyFlag = Object.keys(flags()).length > 0;
          return (
            <div class="menu wl-menu" style={{ position: 'fixed', left: `${m().x}px`, top: `${m().y}px` }} onPointerDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
              <Show when={item !== null && !isSection(item)}>
                <div class="wl-flags">
                  <For each={FLAGS}>
                    {(c) => (
                      <button
                        class="swatch"
                        classList={{ on: flags()[item!] === c }}
                        style={{ '--flag': FLAG_COLORS[c] }}
                        data-tip={flags()[item!] === c ? 'Unflag' : `Flag ${c}`}
                        aria-label={`Flag ${c}`}
                        onClick={run(() => setFlag(item!, c))}
                      />
                    )}
                  </For>
                </div>
                <Show when={flags()[item!]}>
                  <button onClick={run(() => setFlag(item!, null))}>Unflag {symbolOf(item!)}</button>
                </Show>
              </Show>
              <Show when={anyFlag}>
                <button onClick={run(() => patchPanel(id, { flags: {} }))}>Unflag all symbols</button>
              </Show>
              <Show when={item !== null && !isSection(item) && anyFlag}>
                <hr />
              </Show>
              <Show when={item !== null && isSection(item)}>
                <button onClick={run(() => setEditing(sectionName(item!)))}>Rename section</button>
                <button onClick={run(() => toggle(sectionName(item!)))}>{collapsed().includes(sectionName(item!)) ? 'Expand section' : 'Collapse section'}</button>
              </Show>
              <button onClick={run(() => addSection(item))}>
                Add section<span class="dim">{item === null ? 'at the end' : 'above'}</span>
              </button>
              <button onClick={run(() => addSymbol(item !== null && isSection(item) ? sectionEnd(items(), at) : item === null ? at : at + 1))}>
                Add symbol…<span class="dim">{item !== null && isSection(item) ? 'to this section' : item === null ? '' : 'below'}</span>
              </button>
              <Show when={item !== null}>
                <hr />
                <button onClick={run(() => remove(item!))}>{isSection(item!) ? 'Remove section' : `Remove ${symbolOf(item!)}`}</button>
              </Show>
            </div>
          );
        }}
      </Show>
    </div>
  );
}
