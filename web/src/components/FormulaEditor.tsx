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
import { canWrite, deleteFormula, describe, formulaOf, formulas, kindOf, saveFormula, setEditing } from '../lib/formulas';
import { splitList } from '../lib/study-style';
import { Icon } from '../lib/icons';
import { toast } from '../lib/toasts';
import { doc, patchPanel } from '../workspace/store';

const CHECK_MS = 250;

export function FormulaEditor(props: { id?: string; seed?: { name: string; source: string }; panelId?: string }) {
  const opened = props.id ? formulaOf(`fx-${props.id}`) : undefined;
  const [id, setId] = createSignal<string | undefined>(opened?.id);
  const [name, setName] = createSignal(opened?.name ?? props.seed?.name ?? 'My script');
  const [source, setSource] = createSignal(opened?.source ?? props.seed?.source ?? NEW_FORMULA);
  const [saved, setSaved] = createSignal(opened ? { name: opened.name, source: opened.source } : null);
  const [check, setCheck] = createSignal<FormulaDescription | null>(null);
  const [busy, setBusy] = createSignal(false);
  const dirty = () => saved()?.name !== name() || saved()?.source !== source();
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

  const open = (f: { id?: string; name: string; source: string }) => {
    setId(f.id);
    setName(f.name);
    setSource(f.source);
    setSaved(f.id ? { name: f.name, source: f.source } : null);
    area.scrollTop = 0;
  };

  /** Kept on the account's server copy; what went wrong said, and null. */
  const save = async (): Promise<string | null> => {
    if (!canWrite() || busy()) return null;
    setBusy(true);
    try {
      const kept = await saveFormula({ id: id(), name: name(), source: source() });
      setId(kept.id);
      setName(kept.name);
      setSaved({ name: kept.name, source: kept.source });
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
            <input class="dinput fx-name" value={name()} maxLength={40} aria-label="Name" onInput={(e) => setName(e.currentTarget.value)} />
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
