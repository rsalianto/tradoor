/**
 * The indicators: starred ones first, then the rest by what they are for, one
 * short row each. A click adds one with its usual settings and leaves the list
 * open for the next; one already on the chart opens its settings. A study's
 * identity is its parameters, so another of the same takes other ones.
 */
import { createEffect, createMemo, createResource, createSignal, For, onMount, Show } from 'solid-js';
import { setOverlay, setStudyEditing } from '../app/ui';
import { studyCatalog } from '../engine/client';
import { errorText } from '../lib/api';
import { favourites, loadFavourites, toggleFavourite } from '../lib/favourites';
import { formulas, isFormulaKind, kindOf, setEditing } from '../lib/formulas';
import { Icon } from '../lib/icons';
import { session } from '../lib/owner';
import { anotherId, CATEGORIES, defaultId, entries, instanceLabel, searchEntries, type Entry } from '../lib/study-schema';
import { splitList, withLayer } from '../lib/study-style';
import { toast } from '../lib/toasts';
import type { ChartPanelState } from '../workspace/model';
import { patchPanel } from '../workspace/store';

/** What the engine keeps of a chart's list (`parse_studies`). */
const MOST_STUDIES = 12;

export function IndicatorPicker(props: { panel: ChartPanelState; onClose(): void }) {
  const [query, setQuery] = createSignal('');
  const [cur, setCur] = createSignal(0);
  const [defs] = createResource(() => studyCatalog());
  let input!: HTMLInputElement;
  let list!: HTMLDivElement;
  onMount(() => {
    input.focus();
    void loadFavourites();
  });

  const active = () => splitList(props.panel.studies);
  const hidden = () => splitList(props.panel.hidden);
  const removed = () => splitList(props.panel.removed);
  const on = (kind: string) => active().filter((id) => id.split(':')[0] === kind);
  /** The reader's own scripts, first: added as any indicator is, with the settings they write. */
  const mine = createMemo(() =>
    formulas().map((f): Entry => {
      const kind = kindOf(f);
      // One the moderator removed is listed, to open and delete, and not added.
      const about = f.removed ? `Removed by the moderator${f.why ? `: ${f.why}` : ''}.` : 'Your Frost Script: its settings and lines are the ones it writes. Edit opens it.';
      return { id: kind, name: f.name, short: 'Frost Script', category: 'Frost Script', keywords: 'frost script custom own formula code', about, where: f.removed ? 'removed' : 'script', def: f.removed ? undefined : { kind, short: f.name, group: 'price', params: [] } };
    }),
  );
  const all = createMemo(() => [...mine(), ...(defs.state === 'ready' ? entries(defs()!) : [])].filter((e) => !(props.panel.tpo && e.layer?.candles)));
  const write = (id?: string) => {
    props.onClose();
    setEditing({ id, panelId: props.panel.id });
  };

  // Each its own memo, so a star redraws its own section and the list stays where it is scrolled to.
  const filed = createMemo(() => CATEGORIES.map((title): { title: string; items: Entry[] } => ({ title, items: all().filter((e) => e.category === title) })).filter((s) => s.items.length > 0));
  const starred = createMemo(() => ({ title: 'Favourites', items: favourites().flatMap((id) => all().filter((e) => e.id === id)) }));
  /** What shows, in order: a search's matches alone, or favourites and then each category. */
  const sections = createMemo(() => (query().trim() ? [{ title: '', items: searchEntries(all(), query()) }] : [starred(), ...filed()]));
  const flat = createMemo(() => sections().flatMap((s) => s.items));
  const current = () => flat()[cur()];
  createEffect(() => {
    void query();
    setCur(0);
  });

  const settings = (id: string) => {
    props.onClose();
    setStudyEditing({ panelId: props.panel.id, id });
  };
  const add = (id: string): boolean => {
    if (active().length >= MOST_STUDIES) {
      toast({ tone: 'warn', title: `A chart holds ${MOST_STUDIES} indicators`, body: 'Remove one from the chart’s legend first.' });
      return false;
    }
    patchPanel(props.panel.id, { studies: [...active(), id].join(',') });
    return true;
  };
  const toggleLayer = (id: string) => patchPanel(props.panel.id, { hidden: hidden().includes(id) ? hidden().filter((h) => h !== id).join(',') : [...hidden(), id].join(',') });

  /** A click on a row, or Enter on the one highlighted. */
  const pick = (e: Entry | undefined) => {
    if (!e) return;
    if (e.layer) {
      if (removed().includes(e.id)) patchPanel(props.panel.id, withLayer(props.panel, e.id));
      else if (hidden().includes(e.id)) toggleLayer(e.id);
      else settings(e.id);
    } else if (on(e.id).length > 0) settings(on(e.id).at(-1)!);
    else if (e.def) add(defaultId(e.def));
    input.select();
  };
  const another = (e: Entry) => {
    const id = e.def && anotherId(e.def, active());
    if (id && add(id)) settings(id);
  };
  const move = (by: number) => {
    setCur(Math.max(0, Math.min(flat().length - 1, cur() + by)));
    queueMicrotask(() => list.querySelector('.ind.cur')?.scrollIntoView({ block: 'nearest' }));
  };

  const Row = (p: { entry: Entry; index: number }) => {
    const e = p.entry;
    const shown = () => (e.layer ? !hidden().includes(e.id) : on(e.id).length > 0);
    const starred = () => favourites().includes(e.id);
    return (
      <div class="ind" classList={{ cur: cur() === p.index, on: shown() }} role="option" aria-selected={cur() === p.index} onPointerMove={() => setCur(p.index)} onPointerDown={() => setCur(p.index)} onClick={() => pick(e)}>
        <button
          class="star"
          classList={{ on: starred(), none: isFormulaKind(e.id) }}
          disabled={isFormulaKind(e.id)}
          aria-pressed={starred()}
          aria-label={starred() ? `Take ${e.name} out of your favourites` : `Add ${e.name} to your favourites`}
          data-tip={starred() ? 'In your favourites' : 'Add to your favourites'}
          onClick={(ev) => {
            ev.stopPropagation();
            toggleFavourite(e.id);
            input.focus();
          }}
        >
          <Icon name="star" size={13} fill={starred()} />
        </button>
        <span class="iname">{e.name}</span>
        <Show when={isFormulaKind(e.id)}>
          <button
            class="inst"
            data-tip="Open it in the editor"
            onClick={(ev) => {
              ev.stopPropagation();
              write(e.id.slice(3));
            }}
          >
            Edit
          </button>
        </Show>
        <Show when={e.short !== e.name}>
          <span class="ishort">{e.short}</span>
        </Show>
        <span class="spacer" />
        {/* A script the moderator removed says so, on the chart or not. */}
        <Show when={!shown() || e.where === 'removed'}>
          <span class="where">{e.layer && !removed().includes(e.id) ? 'hidden' : e.where}</span>
        </Show>
        <Show
          when={e.layer}
          fallback={
            <For each={on(e.id)}>
              {(id) => (
                <button
                  class="inst"
                  data-tip="On this chart: open its settings"
                  onClick={(ev) => {
                    ev.stopPropagation();
                    settings(id);
                  }}
                >
                  {instanceLabel(id)}
                  <Icon name="settings" size={11} />
                </button>
              )}
            </For>
          }
        >
          <Show when={shown()}>
            <button
              class="inst"
              data-tip="On this chart: open its settings"
              onClick={(ev) => {
                ev.stopPropagation();
                settings(e.id);
              }}
            >
              On chart
              <Icon name="settings" size={11} />
            </button>
          </Show>
        </Show>
        <Show when={e.layer ? removed().includes(e.id) : on(e.id).length === 0 || (e.def && anotherId(e.def, active()))}>
          <button
            class="iadd"
            aria-label={on(e.id).length > 0 ? `Add another ${e.short}` : `Add ${e.short}`}
            data-tip={on(e.id).length > 0 ? 'Add another, with other settings' : 'Add to the chart'}
            onClick={(ev) => {
              ev.stopPropagation();
              if (on(e.id).length > 0) another(e);
              else pick(e);
            }}
          >
            <Icon name="plus" size={12} />
          </button>
        </Show>
      </div>
    );
  };

  return (
    <div class="backdrop" onPointerDown={() => props.onClose()}>
      <div
        class="dialog medium picker"
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation();
            props.onClose();
          } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            move(e.key === 'ArrowDown' ? 1 : -1);
          } else if (e.key === 'Enter') {
            e.preventDefault();
            pick(current());
          }
        }}
      >
        <header>
          <Icon name="search" size={12} />
          <input ref={input} class="q" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} placeholder="Search indicators" aria-label="Search indicators" />
          <button class="x" onClick={() => props.onClose()}>
            esc ✕
          </button>
        </header>
        <div class="list" ref={list} role="listbox">
          <Show when={defs.state !== 'errored'} fallback={<div class="empty warn">▲ {errorText(defs.error)}</div>}>
            <Show when={flat().length > 0 || !query().trim()} fallback={<div class="empty">Nothing matches “{query()}”.</div>}>
              <For each={sections()}>
                {(section, s) => (
                  <>
                    <Show when={section.title}>
                      <div class="ind-head">
                        <span>{section.title}</span>
                        <Show when={section.title === 'Favourites' && !session() && favourites().length > 0}>
                          <button
                            class="ind-hint"
                            data-tip="Without signing in, your favourites stay in this browser. Sign in with Telegram and they follow you to every device"
                            onClick={() => {
                              props.onClose();
                              setOverlay({ kind: 'settings' });
                            }}
                          >
                            This browser only · sign in to keep them
                          </button>
                        </Show>
                      </div>
                    </Show>
                    <Show when={section.title === 'Favourites' && section.items.length === 0}>
                      <div class="ind-none">
                        <Icon name="star" size={11} /> Star an indicator to keep it here
                      </div>
                    </Show>
                    <For each={section.items}>{(entry, i) => <Row entry={entry} index={sections().slice(0, s()).reduce((n, x) => n + x.items.length, 0) + i()} />}</For>
                  </>
                )}
              </For>
            </Show>
          </Show>
        </div>
        <footer class="ind-about">
          <span>{current()?.about ?? (defs.loading ? 'Loading…' : '')}</span>
          <button class="dbtn fx-write" data-tip="An indicator of your own, in a few lines of Frost Script" onClick={() => write()}>
            <Icon name="ice" size={12} /> New Frost Script
          </button>
        </footer>
      </div>
    </div>
  );
}
