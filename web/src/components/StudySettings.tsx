/**
 * One indicator's settings, in the tabs TradingView puts them under, beside the
 * chart rather than over it: every change shows at once. Ok keeps them, Cancel
 * puts the chart back as it was, Defaults starts over. A study's identity is its
 * parameters, so a new length swaps it for the study of that length, its style
 * and its place in the legend kept.
 */
import { createEffect, createMemo, createResource, createSignal, For, Index, onCleanup, onMount, Show, untrack } from 'solid-js';
import { Portal } from 'solid-js/web';
import { studyCatalog } from '../engine/client';
import type { StudyMeta, StudyParam } from '../engine/protocol';
import { entries, LAYER_DEFS, NOTES, otherOwners, paramUi, studyNumbers, studyPlots, toHex, type NumberInput, type ParamUi, type StylePlot } from '../lib/study-schema';
import { dropOwner, numberOf, plotStyle, replaceStudy, splitList, withNumber, withPlot } from '../lib/study-style';
import { CHART_THEME } from '../lib/theme';
import type { ChartPanelState } from '../workspace/model';
import { doc, patchPanel } from '../workspace/store';
import { formulaDefinition, formulaLabels, formulaOf, isFormulaKind, setEditing } from '../lib/formulas';
import { SOURCES } from '../lib/formula-sources';
import { ColorField } from './ColorField';
import { NumField } from './NumField';

const WIDTHS = [1, 1.5, 2, 3, 4];
const PATTERNS = [
  ['0', 'Solid'],
  ['1', 'Dashed'],
  ['2', 'Dotted'],
] as const;
const DIALOG_WIDTH = 390;
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
const theme = (name: string) => doc.theme[name] ?? CHART_THEME.find((t) => t.name === name)?.value ?? '#000000';

export function StudySettings(props: { panel: ChartPanelState; target: string; meta(id: string): StudyMeta | undefined; anchor: HTMLElement; onClose(): void }) {
  const originalStyles = untrack(() => props.panel.studyStyles ?? {});
  const kind = props.target.split(':')[0]!;
  const layer = LAYER_DEFS.find((l) => l.id === kind);
  const formula = isFormulaKind(kind);
  // A formula's settings are the ones it writes, read by the formula module.
  const [defs] = createResource(() => (layer ? Promise.resolve([]) : formula ? formulaDefinition(kind).then((d) => (d ? [d] : [])) : studyCatalog()));
  const [labels] = createResource(() => (formula ? formulaLabels(kind) : Promise.resolve({} as Record<string, string>)));
  /** How it is worked out, as a formula: a built-in's own, or the formula itself. */
  const source = () => (formula ? formulaOf(kind)?.source : SOURCES[kind]?.source);
  const def = () => defs()?.find((d) => d.kind === kind);
  const about = createMemo(() => (layer ?? entries(defs() ?? []).find((e) => e.id === kind)));

  /** The study as it stands: its id moves as its parameters do. */
  const [current, setCurrent] = createSignal(props.target);
  const [problem, setProblem] = createSignal<string | null>(null);
  const styles = () => props.panel.studyStyles;
  const owner = (o?: string) => o ?? current();
  const patch = (next: Partial<ChartPanelState>) => patchPanel(props.panel.id, next);

  const values = (): number[] => {
    const given = current().split(':').slice(1).map(Number);
    return (def()?.params ?? []).map((p, i) => (Number.isFinite(given[i]) ? given[i]! : p.default));
  };
  const setValue = (i: number, param: StudyParam, ui: ParamUi, value: number) => {
    if (!(value >= param.min && value <= param.max)) {
      setProblem(`${ui.label} runs from ${param.min} to ${param.max}.`);
      return;
    }
    const next = values();
    next[i] = param.step >= 1 ? Math.round(value) : Math.round(value * 100) / 100;
    const id = [kind, ...next].join(':');
    if (id === current()) {
      setProblem(null);
    } else if (splitList(props.panel.studies).includes(id)) {
      setProblem('The same one is already on this chart.');
    } else {
      setProblem(null);
      patch(replaceStudy(props.panel, current(), id));
      setCurrent(id);
    }
  };

  /** Whole: where a new length lands, or a layer's own numbers and styles forgotten. */
  const defaults = () => {
    setProblem(null);
    const params = def()?.params ?? [];
    const id = params.length > 0 ? [kind, ...params.map((p) => p.default)].join(':') : current();
    const taken = id !== current() && splitList(props.panel.studies).includes(id);
    let next = dropOwner(props.panel.studyStyles, current());
    for (const other of otherOwners(kind)) next = dropOwner(next, other);
    if (taken) setProblem('The same one with its usual inputs is already on this chart, so the inputs stay.');
    if (id !== current() && !taken) {
      patch({ ...replaceStudy(props.panel, current(), id), studyStyles: next });
      setCurrent(id);
    } else {
      patch({ studyStyles: next });
    }
  };
  /** This one back as it was: the legend stays in use meanwhile, and what was done there stands. */
  const cancel = () => {
    const swapped = replaceStudy(props.panel, current(), props.target);
    const mine = (key: string) => [props.target, ...otherOwners(kind)].some((owner) => key.startsWith(`${owner}/`));
    let styles = dropOwner(swapped.studyStyles, props.target);
    for (const other of otherOwners(kind)) styles = dropOwner(styles, other);
    patch({ ...swapped, studyStyles: { ...styles, ...Object.fromEntries(Object.entries(originalStyles).filter(([key]) => mine(key))) } });
    props.onClose();
  };

  const numbers = (): readonly NumberInput[] => layer?.inputs ?? studyNumbers(kind);
  const hasInputs = () => numbers().length > 0 || (def()?.params.length ?? 0) > 0;
  const plots = createMemo((): readonly StylePlot[] => layer?.plots ?? studyPlots(current(), props.meta(current())?.lines ?? []));
  /** Keyed by plot, not by the row built from each new reading: a row made anew would shut the colour palette open on it. */
  const plotKeys = createMemo(() => plots().map((p) => p.key), undefined, { equals: (a, b) => a.length === b.length && a.every((k, i) => k === b[i]) });
  const [tab, setTab] = createSignal<'inputs' | 'style' | 'source'>('inputs');
  const showing = () => (tab() === 'source' && source() ? 'source' : tab() === 'inputs' && (hasInputs() || defs.loading) ? 'inputs' : 'style');
  const openSource = () => {
    const text = source();
    if (!text) return;
    props.onClose();
    const f = formula ? formulaOf(kind) : undefined;
    setEditing(f ? { id: f.id, panelId: props.panel.id } : { seed: { name: `My ${SOURCES[kind]?.name ?? 'script'}`, source: text }, panelId: props.panel.id });
  };

  // ---------------------------------------------------------------- where it stands

  const [at, setAt] = createSignal(
    untrack(() => {
      const r = props.anchor.getBoundingClientRect();
      return { left: clamp(r.right - DIALOG_WIDTH - 76, 8, Math.max(8, innerWidth - DIALOG_WIDTH - 8)), top: clamp(r.top + 34, 8, Math.max(8, innerHeight - 240)) };
    }),
  );
  const startDrag = (e: PointerEvent) => {
    if ((e.target as Element).closest('button')) return;
    const [x0, y0, from] = [e.clientX, e.clientY, at()];
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const onMove = (m: PointerEvent) => setAt({ left: clamp(from.left + m.clientX - x0, 8 - DIALOG_WIDTH + 80, innerWidth - 80), top: clamp(from.top + m.clientY - y0, 0, innerHeight - 40) });
    const onUp = () => {
      el.removeEventListener('pointermove', onMove);
      el.removeEventListener('pointerup', onUp);
    };
    el.addEventListener('pointermove', onMove);
    el.addEventListener('pointerup', onUp);
  };

  let box!: HTMLDivElement;
  onMount(() => {
    // The dialog keeps the keyboard: a letter typed here must not open the symbol search.
    const onKey = (e: KeyboardEvent) => {
      e.stopPropagation();
      if (e.key === 'Escape') cancel();
      else if (e.key === 'Enter' && e.target instanceof HTMLInputElement) props.onClose();
    };
    box.addEventListener('keydown', onKey);
    onCleanup(() => box.removeEventListener('keydown', onKey));
  });
  // Ready to type a new length in, but for a finger: a keyboard over the chart would hide the change.
  createEffect(() => {
    if (defs.state !== 'ready') return;
    // After the fields' own effects: a number field writes its value only while it has no focus.
    queueMicrotask(() => {
      const field = matchMedia('(pointer: coarse)').matches ? null : box.querySelector<HTMLInputElement>('input.num');
      if (field) field.focus();
      else box.focus();
    });
  });

  // ---------------------------------------------------------------- controls

  const Param = (p: { param: StudyParam; index: number }) => {
    const label = labels()?.[p.param.name];
    const ui = label ? { ...paramUi(kind, p.param), label } : paramUi(kind, p.param);
    const value = () => values()[p.index] ?? p.param.default;
    const set = (v: number) => setValue(p.index, p.param, ui, v);
    if (ui.bits) {
      const bits = ui.bits;
      return (
        <div class="dbits">
          <For each={bits}>
            {(name, i) => (
              <label class="dcheck">
                <input
                  type="checkbox"
                  checked={(value() & (1 << i())) !== 0}
                  onChange={(e) => {
                    const next = e.currentTarget.checked ? value() | (1 << i()) : value() & ~(1 << i());
                    // At least one: an empty sum is not a study.
                    if (next === 0) e.currentTarget.checked = true;
                    else set(next);
                  }}
                />
                {name}
              </label>
            )}
          </For>
        </div>
      );
    }
    return (
      <div class="drow" data-tip={ui.hint}>
        <span class="dlabel">{ui.label}</span>
        <div class="dctl">
          <Show
            when={ui.options}
            fallback={
              <>
                <Show when={ui.band !== undefined}>
                  <input type="checkbox" aria-label={`${ui.label} on`} checked={value() > 0} onChange={(e) => set(e.currentTarget.checked ? ui.band! : 0)} />
                </Show>
                <Show when={ui.band === undefined || value() > 0} fallback={<span class="dim">off</span>}>
                  {/* A 0 typed on the way to 0.5 must not turn the band off under the fingers. */}
                  <NumField class="dinput narrow num" value={value()} step={p.param.step} label={ui.label} onValue={(v) => (ui.band !== undefined && v <= 0 ? undefined : set(v))} />
                </Show>
              </>
            }
          >
            {(options) => (
              <select class="dinput" aria-label={ui.label} onChange={(e) => set(Number(e.currentTarget.value))}>
                <For each={ui.order ?? options().map((_, i) => i)}>
                  {(i) => (
                    <option value={i} selected={value() === i}>
                      {options()[i]}
                    </option>
                  )}
                </For>
              </select>
            )}
          </Show>
        </div>
      </div>
    );
  };

  const Num = (p: { input: NumberInput }) => {
    const i = p.input;
    return (
      <div class="drow" data-tip={i.hint}>
        <span class="dlabel">{i.label}</span>
        <div class="dctl">
          <NumField
            class="dinput narrow num"
            value={numberOf(styles(), owner(i.owner), i.key, i.default)}
            step={i.step ?? 1}
            label={i.label}
            onValue={(v) => {
              if (v >= i.min && v <= i.max) {
                setProblem(null);
                patch({ studyStyles: withNumber(styles(), owner(i.owner), i.key, Math.round(v), i.default) });
              } else {
                setProblem(`${i.label} runs from ${i.min} to ${i.max}.`);
              }
            }}
          />
        </div>
      </div>
    );
  };

  const Plot = (p: { plotKey: string }) => {
    const row = () => plots().find((x) => x.key === p.plotKey);
    const own = () => plotStyle(styles(), owner(row()?.owner), p.plotKey);
    const set = (patchStyle: Parameters<typeof withPlot>[3]) => patch({ studyStyles: withPlot(styles(), owner(row()?.owner), p.plotKey, patchStyle) });
    const color = () => {
      const r = row();
      if (own().color) return own().color;
      if (r?.theme) return `${theme(r.theme).slice(0, 7)}${r.alpha ?? ''}`;
      return toHex(r?.color ?? '#000000');
    };
    const width = () => own().width || String(row()?.width ?? 1);
    return (
      <Show when={row()}>
        {(r) => (
          <div class="dplot">
            <Show when={!r().noToggle} fallback={<span class="dplot-name" classList={{ indent: r().indent }}>{r().label}</span>}>
              <label class="dcheck dplot-name">
                <input type="checkbox" checked={!own().off} onChange={(e) => set({ off: !e.currentTarget.checked })} />
                {r().label}
              </label>
            </Show>
            <span class="spacer" />
            <Show when={!r().onlyToggle}>
              <ColorField value={color()} onChange={(c) => set({ color: c })} title={`${r().label} colour`} />
              <Show when={!r().noWidth}>
                <Show when={!r().noPattern}>
                  <select class="dinput dpattern" aria-label={`${r().label} line style`} onChange={(e) => set({ pattern: e.currentTarget.value })}>
                    <For each={PATTERNS}>
                      {([value, name]) => (
                        <option value={value} selected={(own().pattern || r().pattern || '0') === value}>
                          {name}
                        </option>
                      )}
                    </For>
                  </select>
                </Show>
                <select class="dinput dwidth" aria-label={`${r().label} width`} onChange={(e) => set({ width: e.currentTarget.value })}>
                  <For each={WIDTHS}>
                    {(w) => (
                      <option value={w} selected={width() === String(w)}>
                        {w} px
                      </option>
                    )}
                  </For>
                </select>
              </Show>
            </Show>
          </div>
        )}
      </Show>
    );
  };

  return (
    <Portal>
      <div
        ref={box}
        class="dsettings study-settings"
        tabIndex={-1}
        style={{ left: `${at().left}px`, top: `${at().top}px`, 'max-height': `calc(100dvh - ${at().top + 8}px)` }}
        onPointerDown={(e) => e.stopPropagation()}
        onWheel={(e) => e.stopPropagation()}
      >
        <header onPointerDown={startDrag}>
          <span class="title">{about()?.name ?? ''}</span>
          <span class="spacer" />
          <button class="x" data-tip="Cancel (esc)" aria-label="Cancel" onClick={cancel}>
            ✕
          </button>
        </header>
        <nav class="dtabs">
          <Show when={hasInputs()}>
            <button classList={{ on: showing() === 'inputs' }} onClick={() => setTab('inputs')}>
              Inputs
            </button>
          </Show>
          <button classList={{ on: showing() === 'style' }} onClick={() => setTab('style')}>
            Style
          </button>
          <Show when={source()}>
            <button classList={{ on: showing() === 'source' }} onClick={() => setTab('source')}>
              Source
            </button>
          </Show>
        </nav>
        <div class="dbody">
          <Show when={showing() === 'source'}>
            <p class="dnote">{formula ? 'Your script, as it is written.' : 'How tradoor works it out, written in Frost Script with its usual settings. Open a copy to make a script of your own.'}</p>
            <pre class="fx-source">{source()}</pre>
            <button class="dbtn" onClick={openSource}>
              {formula ? 'Edit the script' : 'Open a copy in Frost Script'}
            </button>
          </Show>
          <Show when={showing() !== 'source'}>
          <Show
            when={showing() === 'inputs'}
            fallback={
              <>
                <For each={plotKeys()} fallback={<p class="dnote">Nothing to style while it draws nothing.</p>}>
                  {(key) => <Plot plotKey={key} />}
                </For>
              </>
            }
          >
            <Index each={def()?.params ?? []}>{(param, i) => <Param param={param()} index={i} />}</Index>
            <For each={numbers()}>{(input) => <Num input={input} />}</For>
            <p class="dnote">{NOTES[kind] ?? about()?.about}</p>
          </Show>
          </Show>
          <Show when={problem()}>
            <p class="dnote warn">▲ {problem()}</p>
          </Show>
        </div>
        <footer>
          <button class="dbtn" data-tip="Its usual inputs and colours" onClick={defaults}>
            Defaults
          </button>
          <span class="spacer" />
          <button class="dbtn" onClick={cancel}>
            Cancel
          </button>
          <button class="dbtn primary" onClick={() => props.onClose()}>
            Ok
          </button>
        </footer>
      </div>
    </Portal>
  );
}
