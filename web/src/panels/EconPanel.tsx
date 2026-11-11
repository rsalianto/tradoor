/**
 * The US economic calendar: CPI, PPI, the FOMC and the other releases that move
 * markets, by the reader's clock. The next one stands pinned above with its
 * countdown; in the list a line marks now, between what is out and what is to come.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show, untrack } from 'solid-js';
import { api, errorText, type EconEvent } from '../lib/api';
import { Icon } from '../lib/icons';
import { arrange, countdown, dayName, isDue, nextOf, shiftDay, surprise } from '../lib/econ';
import { formatDateTime, formatHm, timeZone, zoneLabel } from '../lib/timezone';
import type { EconPanelState } from '../workspace/model';
import { patchPanel } from '../workspace/store';

const SETTLED_MS = 60_000;
const WAITING_MS = 15_000;
const FILLING_MS = 5_000;
/** Under this long, the countdown stands out. */
const SOON_MS = 3_600_000;
/** How long a figure just in is lit. */
const FRESH_MS = 3_000;
/** A refresh asks the server again at most this often from one panel; between, it only goes back to now. */
const REFRESH_MS = 15_000;
/** How long the server is given to read today before it is asked for the result. */
const READ_MS = 2_500;

const dateKey = (ms: number) => formatDateTime(ms).slice(0, 10);
// A release with no time sits at midnight New York, which can fall on another local date: its own date holds.
const dayOf = (e: EconEvent) => (e.allDay ? e.day : dateKey(e.at));
const keyOf = (e: EconEvent) => `${e.at}|${e.title}`;

export function EconPanel(props: { panel: EconPanelState }) {
  const [events, setEvents] = createSignal<EconEvent[] | null>(null);
  const [pending, setPending] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  const [fresh, setFresh] = createSignal<ReadonlySet<string>>(new Set());
  const [now, setNow] = createSignal(Date.now());
  const ticking = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(ticking));
  let list!: HTMLDivElement;

  const scope = () => props.panel.scope;
  const shown = createMemo(() => (scope() === 'core' ? (events() ?? []).filter((e) => e.core) : (events() ?? [])));
  /** How many are out: what moves the line, once a release's time passes, and nothing else. */
  const out = createMemo(() => shown().filter((e) => e.at <= now()).length);
  const today = createMemo(() => dateKey(now()));
  const rows = createMemo(() => {
    out();
    return arrange(shown(), untrack(now), dayOf);
  });
  const next = createMemo(() => {
    out();
    return nextOf(shown(), untrack(now));
  });
  const waiting = () => (events() ?? []).some((e) => isDue(e, now()));

  let answer = '';
  /** The reader has moved the list: what comes in no longer moves it for them, until they press refresh. */
  let moved = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let lit: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  const load = async (fresh = false) => {
    try {
      const got = await api.econ(fresh);
      if (!alive) return;
      setError(null);
      setPending(got.pending);
      const text = JSON.stringify(got.events);
      if (text === answer) return;
      answer = text;
      const before = new Map((events() ?? []).map((e) => [keyOf(e), e.actual]));
      const arrived = new Set(got.events.filter((e) => e.actual !== null && before.has(keyOf(e)) && before.get(keyOf(e)) === null).map(keyOf));
      setEvents(got.events);
      // Days still being read come in above it as well as below: the list stays on now until the reader moves it.
      if (!moved) toNow();
      if (arrived.size > 0) {
        setFresh(arrived);
        clearTimeout(lit);
        lit = setTimeout(() => setFresh(new Set<string>()), FRESH_MS);
      }
    } catch (e) {
      if (alive) setError(errorText(e));
    }
  };
  const poll = async () => {
    if (!document.hidden) await load();
    if (alive) timer = setTimeout(() => void poll(), pending() > 0 ? FILLING_MS : waiting() ? WAITING_MS : SETTLED_MS);
  };
  void poll();
  onCleanup(() => {
    alive = false;
    clearTimeout(timer);
    clearTimeout(lit);
  });

  /** The list scrolled to the line for now, a few rows of what is out above it. */
  const toNow = () =>
    queueMicrotask(() => {
      const line = list.querySelector<HTMLElement>('.ec-now');
      if (line) list.scrollTop = Math.max(0, line.offsetTop - 84);
    });
  // Opened on what is about to happen, not on last week; again when the list is cut down.
  let placedFor = '';
  createEffect(() => {
    const had = rows().length > 1;
    const key = scope();
    if (!had || placedFor === key) return;
    placedFor = key;
    toNow();
  });

  const [refreshing, setRefreshing] = createSignal(false);
  let refreshed = 0;
  /** Today read again, and back to now. Pressed again and again, it only goes back to now. */
  const refresh = async () => {
    moved = false;
    toNow();
    if (refreshing() || Date.now() - refreshed < REFRESH_MS) return;
    refreshed = Date.now();
    setRefreshing(true);
    await load(true);
    await new Promise((resolve) => setTimeout(resolve, READ_MS));
    if (alive) await load();
    setRefreshing(false);
    toNow();
  };

  const when = (e: EconEvent) => {
    const key = dayOf(e);
    const day = key === today() ? 'Today' : key === shiftDay(today(), 1) ? 'Tomorrow' : dayName(key);
    return `${day} ${formatHm(e.at)}`;
  };
  const dayHead = (key: string) => (key === today() ? 'Today' : key === shiftDay(today(), 1) ? 'Tomorrow' : key === shiftDay(today(), -1) ? 'Yesterday' : '');
  const tip = (e: EconEvent) => {
    const side = surprise(e.actual, e.forecast);
    const parts = [e.actual && `Actual ${e.actual}${side > 0 ? ', above forecast' : side < 0 ? ', below forecast' : ''}`, e.forecast && `Forecast ${e.forecast}`, e.previous && `Previous ${e.previous}`].filter(Boolean);
    return [e.title, ...parts].join(' | ');
  };
  const setScope = (value: 'all' | 'core') => patchPanel(props.panel.id, { scope: value });

  return (
    <div class="panelcol ec">
      <Show when={next()[0]}>
        {(first) => (
          <div class="ec-next" classList={{ core: first().core, soon: first().at - now() < SOON_MS }} data-tip={tip(first())}>
            <span class="ec-next-name">
              {first().title}
              <Show when={next().length > 1}>
                <span class="ec-more"> +{next().length - 1}</span>
              </Show>
            </span>
            <span class="ec-count num">{countdown(first().at - now())}</span>
            <span class="ec-sub">
              <span class="num">{when(first())}</span>
              <Show when={first().forecast}>{(v) => <span>Forecast <b class="num">{v()}</b></span>}</Show>
              <Show when={first().previous}>{(v) => <span>Previous <b class="num">{v()}</b></span>}</Show>
            </span>
          </div>
        )}
      </Show>
      <div class="toolbar">
        <button class="tb" classList={{ on: scope() === 'all' }} data-tip="CPI, PPI, the FOMC, jobs, retail sales, PMIs, PCE and GDP" onClick={() => setScope('all')}>
          All
        </button>
        <button class="tb" classList={{ on: scope() === 'core' }} data-tip="Only the three releases markets wait for" onClick={() => setScope('core')}>
          CPI, PPI, FOMC
        </button>
        <span class="spacer" />
        <Show when={pending() > 0 && events() !== null}>
          <span class="ec-note">Reading {pending()} more days</span>
        </Show>
        <button class="iconbtn ec-refresh" classList={{ busy: refreshing() }} data-tip="Refresh, and back to now" aria-label="Refresh, and back to now" onClick={() => void refresh()}>
          <Icon name="reset" size={13} />
        </button>
      </div>
      <div class="ec-list" ref={list} onWheel={() => (moved = true)} onPointerDown={() => (moved = true)} onTouchStart={() => (moved = true)}>
        <Show
          when={events() !== null}
          fallback={
            <Show when={error()} fallback={<div class="empty">Reading the calendar</div>}>
              {(message) => (
                <div class="empty warn">
                  Calendar not loaded: {message()}
                  <div>
                    <button class="tb" onClick={() => void load()}>
                      Try again
                    </button>
                  </div>
                </div>
              )}
            </Show>
          }
        >
          <div class="ec-cols">
            <span data-tip="Times in the zone set in Settings">{zoneLabel(timeZone())}</span>
            <span>Release</span>
            <span class="r">Actual</span>
            <span class="r ec-f">Fcst</span>
            <span class="r ec-p">Prev</span>
          </div>
          <For each={rows()}>
            {(row) => {
              if (row.kind === 'now') {
                return (
                  <div class="ec-now" data-tip="Now">
                    <span class="num">{formatHm(now())}</span>
                  </div>
                );
              }
              if (row.kind === 'day') {
                return (
                  <div class="ec-day">
                    <Show when={dayHead(row.key)}>
                      <b>{dayHead(row.key)}</b>
                    </Show>
                    {dayName(row.key)}
                  </div>
                );
              }
              const e = row.event;
              const side = surprise(e.actual, e.forecast);
              return (
                <div class="ec-row" classList={{ past: row.past, core: e.core, fresh: fresh().has(keyOf(e)) }} data-tip={tip(e)}>
                  <span class="num ec-t">{e.allDay ? '–' : formatHm(e.at)}</span>
                  <span class="ec-name">{e.title}</span>
                  <span class="num r ec-a" classList={{ above: side > 0, below: side < 0 }}>
                    <Show when={e.actual} fallback={<Show when={isDue(e, now())}><span class="ec-due">due</span></Show>}>
                      <Show when={side !== 0}>
                        <i>{side > 0 ? '▲' : '▼'}</i>
                      </Show>
                      {e.actual}
                    </Show>
                  </span>
                  <span class="num r ec-f">{e.forecast}</span>
                  <span class="num r ec-p">{e.previous}</span>
                </div>
              );
            }}
          </For>
          <Show when={shown().length === 0 && pending() === 0}>
            <div class="empty">No releases listed around today.</div>
          </Show>
        </Show>
      </div>
    </div>
  );
}
