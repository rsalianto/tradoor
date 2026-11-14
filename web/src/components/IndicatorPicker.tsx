/**
 * The indicators, in three tabs: tradoor's built-ins (starred ones first, then by
 * what they are for), the Community's public Frost Scripts under their authors'
 * usernames, and the reader's own scripts. One search runs through all three, each
 * tab saying how many it matches. A click adds one with its usual settings and
 * leaves the list open for the next; one already on the chart opens its
 * settings. A study's identity is its parameters, so another of the same takes
 * other ones.
 */
import { createEffect, createMemo, createResource, createSignal, For, onMount, Show } from 'solid-js';
import { setOverlay, setStudyEditing } from '../app/ui';
import { studyCatalog } from '../engine/client';
import { api, errorText } from '../lib/api';
import { favourites, loadFavourites, toggleFavourite } from '../lib/favourites';
import { community, communityError, formulas, isFormulaKind, kindOf, loadCommunity, setEditing } from '../lib/formulas';
import { Icon } from '../lib/icons';
import { session } from '../lib/owner';
import { readPref, writePref } from '../lib/prefs';
import { anotherId, CATEGORIES, defaultId, entries, instanceLabel, searchEntries, type Entry } from '../lib/study-schema';
import { splitList, withLayer } from '../lib/study-style';
import { toast } from '../lib/toasts';
import type { ChartPanelState } from '../workspace/model';
import { patchPanel } from '../workspace/store';

/** What the engine keeps of a chart's list (`parse_studies`). */
const MOST_STUDIES = 12;

type Tab = 'built-in' | 'community' | 'mine';
const TABS: readonly { id: Tab; label: string }[] = [
  { id: 'built-in', label: 'Built-in' },
  { id: 'community', label: 'Community' },
  { id: 'mine', label: 'My scripts' },
];
/** A row: an entry, with its author's username for one from the Community, or whether one's own is public. */
type Listed = Entry & { by?: string; tag?: 'public' | 'private' };

export function IndicatorPicker(props: { panel: ChartPanelState; onClose(): void }) {
  const [query, setQuery] = createSignal('');
  const [cur, setCur] = createSignal(0);
  const [defs] = createResource(() => studyCatalog());
  // The tab last open, in this browser.
  const [tab, setTab] = createSignal<Tab>(TABS.find((t) => t.id === readPref('pickerTab'))?.id ?? 'built-in');
  let input!: HTMLInputElement;
  let list!: HTMLDivElement;
  onMount(() => {
    input.focus();
    void loadFavourites();
    void loadCommunity();
  });
  const choose = (t: Tab) => {
    setTab(t);
    writePref('pickerTab', t);
    setCur(0);
    list.scrollTop = 0;
    input.focus();
  };

  const active = () => splitList(props.panel.studies);
  const hidden = () => splitList(props.panel.hidden);
  const removed = () => splitList(props.panel.removed);
  const on = (kind: string) => active().filter((id) => id.split(':')[0] === kind);
  const builtins = createMemo(() => (defs.state === 'ready' ? entries(defs()!) : []).filter((e) => !(props.panel.tpo && e.layer?.candles)));
  /** The reader's own scripts: added as any indicator is, with the settings they write. */
  const mine = createMemo(() =>
    formulas().map((f): Listed => {
      const kind = kindOf(f);
      // One the moderator removed is listed, to open and delete, and not added.
      const about = f.removed
        ? `Removed by the moderator${f.why ? `: ${f.why}` : ''}.`
        : f.about
          ? `${f.public ? 'Public' : 'Private'}: ${f.about.replace(/\s+/g, ' ')}`
          : `Your Frost Script, ${f.public ? 'public: in the Community under your username' : 'private: yours alone'}. Edit opens it.`;
      return {
        id: kind,
        name: f.name,
        short: f.name,
        category: 'Frost Script',
        keywords: 'frost script custom own formula code mine',
        about,
        where: f.removed ? 'removed' : '',
        def: f.removed ? undefined : { kind, short: f.name, group: 'price', params: [] },
        tag: f.removed ? undefined : f.public ? 'public' : 'private',
      };
    }),
  );
  const own = createMemo(() => new Set(formulas().map((f) => f.id)));
  /** Everyone's public scripts, by their authors' usernames. */
  const shared = createMemo(() =>
    (community() ?? []).map((s): Listed => {
      const kind = `fx-${s.id}`;
      const about = `@${s.username}: ${s.about.replace(/\s+/g, ' ')}${s.more ? '…' : ''}`;
      return { id: kind, name: s.name, short: s.name, category: 'Frost Script', keywords: `${s.username} @${s.username} frost script community ${s.about}`, about, where: '', def: { kind, short: s.name, group: 'price', params: [] }, by: s.username };
    }),
  );
  const write = (id?: string) => {
    props.onClose();
    setEditing({ id, panelId: props.panel.id });
  };
  /** Someone's public script, as a copy of one's own in the editor: to read, or to change. */
  const copy = async (e: Listed) => {
    try {
      const s = await api.communityScript(e.id.slice(3));
      props.onClose();
      setEditing({ seed: { name: `My ${s.name}`.slice(0, 40), source: s.source }, panelId: props.panel.id });
    } catch (err) {
      toast({ tone: 'warn', title: 'Not opened', body: errorText(err) });
    }
  };

  const q = () => query().trim();
  // Each its own memo, so a star redraws its own section and the list stays where it is scrolled to.
  const filed = createMemo(() =>
    CATEGORIES.filter((c) => c !== 'Frost Script')
      .map((title): { title: string; items: Listed[] } => ({ title, items: builtins().filter((e) => e.category === title) }))
      .filter((s) => s.items.length > 0),
  );
  const starred = createMemo(() => ({ title: 'Favourites', items: favourites().flatMap((id) => builtins().filter((e) => e.id === id)) }));
  /** Each tab's matches for the search, for the tab itself and for the count on the others. */
  const found = createMemo((): Record<Tab, Listed[]> => ({ 'built-in': searchEntries(builtins(), q()), community: searchEntries(shared(), q()), mine: searchEntries(mine(), q()) }));
  /** What shows, in order: a search's matches alone, or the tab's sections. */
  const sections = createMemo((): { title: string; items: Listed[]; showcase?: boolean }[] => {
    const t = tab();
    if (q()) return [{ title: '', items: found()[t] }];
    if (t === 'built-in') return [starred(), ...filed()];
    if (t === 'community') return shared().length > 0 ? [{ title: 'The last changed first', items: shared(), showcase: true }] : [];
    return mine().length > 0 ? [{ title: '', items: mine() }] : [];
  });
  const flat = createMemo(() => sections().flatMap((s) => s.items));
  const current = () => flat()[cur()];
  createEffect(() => {
    void query();
    void tab();
    setCur(0);
  });
  /** What the footer says with nothing highlighted. */
  const blurb = () =>
    tab() === 'community'
      ? 'Frost Scripts their authors made public, under their usernames. Read one’s source before you trade on it.'
      : tab() === 'mine'
        ? 'Your own Frost Scripts. Each is private until you make it public in the editor.'
        : defs.loading
          ? 'Loading…'
          : '';

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

  const Row = (p: { entry: Listed; index: number }) => {
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
        <Show when={e.by}>
          <span class="iby">by @{e.by}</span>
        </Show>
        <Show when={e.tag}>
          <span class="itag" classList={{ public: e.tag === 'public' }} data-tip={e.tag === 'public' ? 'Listed in the Community' : 'Yours alone: not in the Community'}>
            {e.tag}
          </span>
        </Show>
        <Show when={isFormulaKind(e.id) && own().has(e.id.slice(3))}>
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
        <Show when={e.by && !own().has(e.id.slice(3))}>
          <button
            class="inst"
            data-tip="Open a copy of your own in the editor, to read or change"
            onClick={(ev) => {
              ev.stopPropagation();
              void copy(e);
            }}
          >
            Copy
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

  /** A tab with nothing to show: what it would hold, and where a search matches instead. */
  const Empty = () => {
    const elsewhere = () => TABS.filter((t) => t.id !== tab() && found()[t.id].length > 0);
    return (
      <div class="empty ind-empty">
        <Show
          when={q()}
          fallback={
            <Show
              when={tab() === 'community'}
              fallback={
                <Show when={session()} fallback={<SignIn text="Sign in with Telegram to write Frost Scripts of your own, and to make them public." />}>
                  <p>None yet. Write one with New Frost Script below, or open a copy of a built-in’s or a Community script’s.</p>
                </Show>
              }
            >
              <Show when={!communityError()} fallback={<p class="warn">▲ The Community did not load: {communityError()}</p>}>
                <p>{community() ? 'No public scripts yet. Make one of yours public in the editor, and it is listed here under your username.' : 'Loading the Community…'}</p>
                {/* The showcase is never empty: tradoor's own scripts are there, drawn live. */}
                <a class="ind-hint" href="/community.html" target="_blank" rel="noopener">
                  The showcase, with tradoor's own scripts ↗
                </a>
              </Show>
            </Show>
          }
        >
          <p>
            Nothing in {TABS.find((t) => t.id === tab())!.label} matches “{q()}”.
          </p>
          <For each={elsewhere()}>
            {(t) => (
              <button class="ind-hint" onClick={() => choose(t.id)}>
                {t.label} has {found()[t.id].length} →
              </button>
            )}
          </For>
        </Show>
      </div>
    );
  };
  const SignIn = (p: { text: string }) => (
    <>
      <p>{p.text}</p>
      <button
        class="dbtn"
        onClick={() => {
          props.onClose();
          setOverlay({ kind: 'settings' });
        }}
      >
        Sign in
      </button>
    </>
  );

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
        <nav class="ind-tabs" role="tablist" aria-label="Which indicators">
          <For each={TABS}>
            {(t) => (
              <button role="tab" aria-selected={tab() === t.id} classList={{ on: tab() === t.id }} onClick={() => choose(t.id)}>
                <Show when={t.id !== 'built-in'}>
                  <Icon name="ice" size={11} />
                </Show>
                {t.label}
                <Show when={q()}>
                  <span class="n">{found()[t.id].length}</span>
                </Show>
              </button>
            )}
          </For>
        </nav>
        <div class="list" ref={list} role="listbox">
          <Show when={tab() !== 'built-in' || defs.state !== 'errored'} fallback={<div class="empty warn">▲ {errorText(defs.error)}</div>}>
            <Show when={flat().length > 0 || (!q() && tab() === 'built-in')} fallback={<Empty />}>
              <For each={sections()}>
                {(section, s) => (
                  <>
                    <Show when={section.title}>
                      <div class="ind-head">
                        <span>{section.title}</span>
                        <Show when={section.showcase}>
                          <a class="ind-hint" href="/community.html" target="_blank" rel="noopener" data-tip="Every public script, with its description and source, on a page of its own to share">
                            The showcase ↗
                          </a>
                        </Show>
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
          <span>{current()?.about ?? blurb()}</span>
          <button class="dbtn fx-write" data-tip="An indicator of your own, in a few lines of Frost Script" onClick={() => write()}>
            <Icon name="ice" size={12} /> New Frost Script
          </button>
        </footer>
      </div>
    </div>
  );
}
