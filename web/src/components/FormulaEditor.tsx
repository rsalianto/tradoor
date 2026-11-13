/**
 * The Frost Script editor: the reader's formulas and the examples down one side,
 * the text on the other, checked as it is typed — a mistake is shown by its
 * line and column, and a formula that checks lists its settings and lines.
 * Saved in this browser; added to the chart it was opened from.
 */
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import type { FormulaDescription } from '../engine/protocol';
import { EXAMPLES, NEW_FORMULA } from '../lib/formula-sources';
import { setOverlay } from '../app/ui';
import { errorText } from '../lib/api';
import { canWrite, deleteFormula, describe, formulaOf, formulas, kindOf, loadCommunity, saveFormula, setEditing } from '../lib/formulas';
import { username } from '../lib/username';
import { UsernameClaim } from './Username';
import { splitList } from '../lib/study-style';
import { Icon } from '../lib/icons';
import { toast } from '../lib/toasts';
import { doc, marketOf, patchPanel, timeframeOf } from '../workspace/store';

const CHECK_MS = 250;
/** A description, in characters: at least, to share the script, and at most (bins/server scripts). */
const SHARED_ABOUT = 30;
const MAX_ABOUT = 1000;
/** What teaches an AI chat Frost Script, to turn a Pine Script v6 indicator into one: the same text the docs show. */
const AI_TEXT = '/frost-script-ai.txt';

export function FormulaEditor(props: { id?: string; seed?: { name: string; source: string }; panelId?: string }) {
  const opened = props.id ? formulaOf(`fx-${props.id}`) : undefined;
  const [id, setId] = createSignal<string | undefined>(opened?.id);
  const [name, setName] = createSignal(opened?.name ?? props.seed?.name ?? 'My script');
  const [source, setSource] = createSignal(opened?.source ?? props.seed?.source ?? NEW_FORMULA);
  const [shared, setShared] = createSignal(opened?.public ?? false);
  const [about, setAbout] = createSignal(opened?.about ?? '');
  const [saved, setSaved] = createSignal(opened ? { name: opened.name, source: opened.source, shared: opened.public, about: opened.about ?? '' } : null);
  const [check, setCheck] = createSignal<FormulaDescription | null>(null);
  const [busy, setBusy] = createSignal(false);
  const dirty = () => saved()?.name !== name() || saved()?.source !== source() || (saved()?.shared ?? false) !== shared() || (saved()?.about ?? '') !== about().trim();
  /**
   * The chart it is shared from, as the showcase draws it: the one the editor
   * was opened from, its market and timeframe, and the script's settings there.
   */
  const preview = () => {
    const panel = props.panelId ? doc.panels[props.panelId] : undefined;
    if (panel?.type !== 'chart') return undefined;
    const kind = id() ? kindOf({ id: id()! }) : '';
    const on = splitList(panel.studies).find((s) => kind && s.split(':')[0] === kind);
    const inputs = on ? on.split(':').slice(1).map(Number).filter(Number.isFinite) : [];
    return { market: marketOf(panel), timeframe: timeframeOf(panel), inputs };
  };
  /** Characters a shared script's description still lacks; 0 for none. */
  const short = () => Math.max(0, SHARED_ABOUT - about().trim().length);
  /** The open script, as the library has it now: removed by the moderator, it says why. */
  const current = () => formulas().find((f) => f.id === id());
  const removed = () => current()?.removed === true;
  let area!: HTMLTextAreaElement;
  let gutter!: HTMLDivElement;

  // Checked a moment after typing stops, by the worker's formula module.
  createEffect(
    on(source, (text) => {
      const timer = setTimeout(() => void describe(text).then((d) => text === source() && setCheck(d)), CHECK_MS);
      onCleanup(() => clearTimeout(timer));
    }),
  );
  const lines = () => source().split('\n').length;
  const close = () => setEditing(null);

  const open = (f: { id?: string; name: string; source: string; public?: boolean; about?: string }) => {
    setId(f.id);
    setName(f.name);
    setSource(f.source);
    setShared(f.public ?? false);
    setAbout(f.about ?? '');
    setSaved(f.id ? { name: f.name, source: f.source, shared: f.public ?? false, about: f.about ?? '' } : null);
    area.scrollTop = 0;
  };

  /** Kept on the account's server copy; what went wrong said, and null. */
  const save = async (): Promise<string | null> => {
    if (!canWrite() || busy()) return null;
    setBusy(true);
    try {
      if (shared() && !username()) throw new Error('choose your username first: public scripts are shared under it');
      if (shared() && short() > 0) throw new Error(`describe it first, in ${SHARED_ABOUT} characters at least: what it shows, and how to read it`);
      const kept = await saveFormula({ id: id(), name: name(), source: source(), public: shared(), about: about(), preview: shared() ? preview() : undefined });
      setId(kept.id);
      setName(kept.name);
      setShared(kept.public);
      setAbout(kept.about);
      setSaved({ name: kept.name, source: kept.source, shared: kept.public, about: kept.about });
      // The Community shows it, or no longer does, at its next reading.
      if (kept.public || saved()?.shared) void loadCommunity();
      return kept.id;
    } catch (e) {
      toast({ tone: 'warn', title: 'Not saved', body: errorText(e) });
      return null;
    } finally {
      setBusy(false);
    }
  };

  const addToChart = async () => {
    const c = check();
    if (!c?.ok) {
      toast({ tone: 'warn', title: 'Not yet', body: 'The script has a mistake to put right first.' });
      return;
    }
    const kept = await save();
    const panel = props.panelId ? doc.panels[props.panelId] : undefined;
    if (!kept || panel?.type !== 'chart') return;
    const kind = kindOf({ id: kept });
    const studies = splitList(panel.studies);
    if (!studies.some((s) => s.split(':')[0] === kind)) patchPanel(panel.id, { studies: [...studies, kind].join(',') });
    close();
  };

  const remove = async () => {
    const gone = id();
    if (!gone || busy()) return;
    try {
      await deleteFormula(gone);
    } catch (e) {
      toast({ tone: 'warn', title: 'Not deleted', body: errorText(e) });
      return;
    }
    const kind = kindOf({ id: gone });
    // Off every chart as well: a formula no longer kept has nothing to draw.
    for (const panel of Object.values(doc.panels)) {
      if (panel?.type === 'chart' && splitList(panel.studies).some((s) => s.split(':')[0] === kind)) {
        patchPanel(panel.id, { studies: splitList(panel.studies).filter((s) => s.split(':')[0] !== kind).join(',') });
      }
    }
    open({ name: 'My script', source: NEW_FORMULA });
  };

  /** Where the mistake is: the cursor put there. */
  const jump = (line: number, col: number) => {
    const rows = source().split('\n');
    const at = rows.slice(0, Math.max(0, line - 1)).reduce((sum, r) => sum + r.length + 1, 0) + Math.max(0, col - 1);
    area.focus();
    area.setSelectionRange(at, at);
  };

  const onKey = (e: KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === 'Escape') close();
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      void save();
    }
  };
  const onTab = (e: KeyboardEvent) => {
    if (e.key !== 'Tab' || e.shiftKey) return;
    e.preventDefault();
    const [a, b] = [area.selectionStart, area.selectionEnd];
    setSource(source().slice(0, a) + '  ' + source().slice(b));
    queueMicrotask(() => area.setSelectionRange(a + 2, a + 2));
  };
  onMount(() => area.focus());

  // Read ahead, so a click copies at once: some browsers allow the clipboard only while the click is fresh.
  let aiText: Promise<string> | null = null;
  const readAiText = () => (aiText ??= fetch(AI_TEXT).then((r) => (r.ok ? r.text() : Promise.reject(new Error(`the instructions did not load (${r.status})`)))));
  let aiReady = '';
  onMount(() => void readAiText().then((t) => (aiReady = t), () => (aiText = null)));
  const copyAi = async () => {
    try {
      await navigator.clipboard.writeText(aiReady || (await readAiText()));
      toast({ tone: 'info', title: 'Copied the instructions for an AI', body: 'Paste them into Claude, ChatGPT, Gemini or another AI chat, then your Pine Script v6 indicator. Paste its answer here, and it is checked as usual.' });
    } catch (e) {
      aiText = null;
      toast({ tone: 'warn', title: 'Not copied', body: `${errorText(e)}. They are on the docs page as well.` });
    }
  };

  return (
    <div class="backdrop" onPointerDown={close}>
      <div class="dialog fx" tabIndex={-1} onPointerDown={(e) => e.stopPropagation()} onKeyDown={onKey}>
        <header>
          <span class="title caps fx-title">
            <Icon name="ice" size={13} /> Frost Script
          </span>
          <span class="spacer" />
          <button class="x" onClick={close}>
            esc ✕
          </button>
        </header>
        <div class="fx-body">
          <aside class="fx-list">
            <button class="fx-new" onClick={() => open({ name: 'My script', source: NEW_FORMULA })}>
              + New script
            </button>
            <div class="fx-head">Mine</div>
            <For each={formulas()} fallback={<p class="fx-none">None yet: start from an example below.</p>}>
              {(f) => (
                <button class="fx-item" classList={{ on: f.id === id() }} onClick={() => open(f)}>
                  {f.name}
                </button>
              )}
            </For>
            <div class="fx-head">Examples</div>
            <For each={EXAMPLES}>
              {(x) => (
                <button class="fx-item" data-tip="Opens a copy of your own" onClick={() => open({ name: x.name, source: x.source })}>
                  {x.name}
                </button>
              )}
            </For>
          </aside>
          <section class="fx-main">
            <Show when={!canWrite()}>
              <div class="fx-banner">
                <span>Sign in with Telegram to write and keep Frost Scripts. Until then, read the examples and the built-ins' sources.</span>
                <button
                  class="dbtn"
                  onClick={() => {
                    close();
                    setOverlay({ kind: 'settings' });
                  }}
                >
                  Sign in
                </button>
              </div>
            </Show>
            <Show when={removed()}>
              <div class="fx-banner warn">Removed by the moderator{current()?.why ? `: ${current()!.why}` : ''}. It runs for no one, and cannot be changed; you can still delete it.</div>
            </Show>
            <div class="fx-row">
              <input class="dinput fx-name" value={name()} maxLength={40} aria-label="Name" onInput={(e) => setName(e.currentTarget.value)} />
              <select
                class="box fx-vis"
                aria-label="Who sees it"
                disabled={!canWrite() || removed()}
                data-tip="Private: yours alone. Public: in the indicator picker's Community tab under your username, for anyone to add to a chart"
                onChange={(e) => setShared(e.currentTarget.value === 'public')}
              >
                <option value="private" selected={!shared()}>
                  Private
                </option>
                <option value="public" selected={shared()}>
                  Public
                </option>
              </select>
            </div>
            <Show when={canWrite()}>
              <textarea
                class="dinput fx-about"
                rows={2}
                maxLength={MAX_ABOUT}
                value={about()}
                disabled={removed()}
                aria-label="Description"
                placeholder={`What it shows, and how to read it. Needed to share it: ${SHARED_ABOUT} characters at least.`}
                onInput={(e) => setAbout(e.currentTarget.value)}
              />
            </Show>
            <Show when={canWrite() && shared()}>
              <div class="fx-share">
                <Show
                  when={username()}
                  fallback={
                    <div class="fx-claim">
                      <span>
                        Shared scripts go under your <b>username</b>, in place of your Telegram name. Choose it once: it can never be changed, and stays with this Telegram account.
                      </span>
                      <UsernameClaim button="dbtn" />
                    </div>
                  }
                >
                  {(u) => (
                    <Show when={short() === 0} fallback={<span class="warn">Describe it to share it: {short()} more characters. Say what it shows, and how to read it.</span>}>
                      <span>
                        {saved()?.shared ? 'Shared in the Community as' : 'Saved, it is shared in the Community as'} <b>@{u()}</b>, on every chart it is added to, and on the{' '}
                        <a href="/community.html" target="_blank" rel="noopener">
                          showcase ↗
                        </a>
                      </span>
                    </Show>
                  )}
                </Show>
              </div>
            </Show>
            <div class="fx-code">
              <div class="fx-gutter" ref={gutter} aria-hidden="true">
                <For each={Array.from({ length: lines() }, (_, i) => i + 1)}>{(n) => <span classList={{ bad: check()?.ok === false && (check() as { line: number }).line === n }}>{n}</span>}</For>
              </div>
              <textarea
                ref={area}
                class="fx-text"
                spellcheck={false}
                autocomplete="off"
                autocapitalize="off"
                aria-label="Script"
                value={source()}
                onInput={(e) => setSource(e.currentTarget.value)}
                onKeyDown={onTab}
                onScroll={() => (gutter.scrollTop = area.scrollTop)}
              />
            </div>
            <div class="fx-status">
              {/* Keyed: each new check is shown, not just the first. */}
              <Show when={check()} keyed fallback={<span class="dim">Checking…</span>}>
                {(r) => {
                  return r.ok ? (
                    <span class="ok">
                      ✓ Checks · {r.ownPane ? 'its own pane' : 'on price'} · {r.inputs.length} {r.inputs.length === 1 ? 'setting' : 'settings'} · {r.plots.length} {r.plots.length === 1 ? 'line' : 'lines'}
                      {r.marks.length > 0 ? ` · ${r.marks.length} ${r.marks.length === 1 ? 'mark' : 'marks'}` : ''}
                      {r.draws ? ' · shapes' : ''}
                    </span>
                  ) : (
                    <button class="warn fx-error" onClick={() => jump(r.line, r.col)}>
                      ▲ {r.line > 0 ? `Line ${r.line}, column ${r.col}: ` : ''}
                      {r.message}
                    </button>
                  );
                }}
              </Show>
            </div>
            <div class="fx-foot">
              <details class="fx-help">
                <summary>
                  How Frost Script works ·{' '}
                  <a href="/doc.html#frost-script" target="_blank" rel="noopener">
                    the full reference ↗
                  </a>
                </summary>
                <div>
                  <p>
                    Each line is worked out on every bar, oldest first. <code>x = …</code> names a value; <code>x[1]</code> is what it was a bar ago. <code>state x = 0</code> keeps its value from one bar to the next.
                  </p>
                  <p>
                    Bars: <code>open high low close volume hl2 hlc3 ohlc4 tr bar time</code>, and <code>day week month year</code>, which change as a new one begins. <code>na</code> is no value.
                  </p>
                  <p>
                    Remembering past bars, outside <code>if</code> and <code>for</code>: <code>sma ema rma stdev highest lowest sum rsi atr change crossover crossunder barssince valuewhen changed</code>. Anywhere:{' '}
                    <code>abs sqrt log exp pow round floor ceil min max clamp nz isna</code>.
                  </p>
                  <p>
                    Drawing: <code>plot(value, "Title", colour, width)</code>, <code>histogram</code>, <code>hline(80, "Upper")</code>, <code>fill("Upper", "Lower")</code>, <code>mark(condition, "Buy", below, green)</code>. <code>pane own</code> draws below the chart.
                    Settings: <code>input length = 14 min 2 max 500 "Length"</code>.
                  </p>
                  <p class="dim">A script reads candles and draws lines: it cannot reach the network, your account or the page. Loops go round 1,000 times at most, and one that asks too much stops with an error.</p>
                </div>
              </details>
              <button
                class="fx-ai"
                data-tip="Instructions that teach an AI chat (Claude, ChatGPT, Gemini…) Frost Script. Paste them, then a Pine Script v6 indicator, and it answers in Frost Script to paste here."
                onClick={() => void copyAi()}
              >
                Pine Script → Frost: copy AI instructions
              </button>
            </div>
          </section>
        </div>
        <footer>
          <Show when={id()}>
            <button class="dbtn" onClick={remove}>
              Delete
            </button>
          </Show>
          <span class="spacer" />
          <Show when={dirty()}>
            <span class="dim fx-unsaved">Not saved</span>
          </Show>
          <button class="dbtn" disabled={!canWrite() || removed() || busy()} onClick={() => void save()}>
            Save
          </button>
          <Show when={props.panelId}>
            <button class="dbtn primary" disabled={!canWrite() || removed() || busy()} onClick={() => void addToChart()}>
              Save and add to chart
            </button>
          </Show>
        </footer>
      </div>
    </div>
  );
}
