/**
 * One drawing's settings, in the tabs TradingView puts them under. Every change
 * shows on the chart at once; Ok saves it, Cancel puts the drawing back.
 */
import { createSignal, For, type JSX, onCleanup, onMount, Show } from 'solid-js';
import type { DrawingPoint } from '../engine/protocol';
import type { Drawing } from '../lib/api';
import {
  isPosition,
  KIND_NAMES,
  parseLevels,
  resetDefaultStyle,
  saveDefaultStyle,
  serializeLevels,
  VISIBILITY_UNITS,
  withHidden,
  type Dash,
  type Level,
} from '../lib/drawing-options';
import { defaultStyle, factoryStyle, visibleOn, type Style } from '../lib/drawing-style';
import { useDismiss } from '../lib/dismiss';
import { ColorField } from './ColorField';
import { PositionTicket } from './PositionTicket';

export interface DrawingPatch {
  style: Style;
  label: string | null;
  points: DrawingPoint[];
}

type Tab = 'inputs' | 'style' | 'text' | 'coordinates' | 'visibility';

const LINE_KINDS = new Set(['trendline', 'ray', 'horizontal', 'path', 'brush']);

export function DrawingSettings(props: {
  drawing: Drawing;
  /** The instrument's price decimals: one tick is 10^-precision. */
  precision: number;
  onPreview(patch: DrawingPatch): void;
  /** Saves the drawing; resolves once the server has it. */
  onCommit(patch: DrawingPatch): Promise<void> | void;
  onCancel(): void;
}) {
  const kind = props.drawing.kind;
  const original: DrawingPatch = { style: { ...props.drawing.style }, label: props.drawing.label, points: props.drawing.points.map((p) => ({ ...p })) };
  const [style, setStyleState] = createSignal<Style>({ ...factoryStyle(kind), ...original.style });
  const [label, setLabelState] = createSignal<string | null>(original.label);
  const [points, setPointsState] = createSignal<DrawingPoint[]>(original.points);
  const tabs: Tab[] = isPosition(kind) ? ['inputs', 'style', 'visibility'] : kind === 'text' ? ['style', 'coordinates', 'visibility'] : ['style', 'text', 'coordinates', 'visibility'];
  const [tab, setTab] = createSignal<Tab>(tabs[0]!);
  const [templateMenu, setTemplateMenu] = createSignal<true | null>(null);
  useDismiss(templateMenu, () => setTemplateMenu(null));

  const patch = (): DrawingPatch => ({ style: style(), label: label(), points: points() });
  const preview = () => props.onPreview(patch());
  const setStyle = (next: Style) => {
    setStyleState(next);
    preview();
  };
  const set = (key: string, value: string | number | boolean) => setStyle({ ...style(), [key]: value });
  const merge = (values: Style) => setStyle({ ...style(), ...values });
  const setLabel = (value: string) => {
    setLabelState(value.trim() === '' ? null : value);
    preview();
  };
  const setPrice = (i: number, price: number) => {
    if (!Number.isFinite(price)) return;
    setPointsState((list) => list.map((p, j) => (j === i ? { ...p, price } : p)));
    preview();
  };
  const setTime = (i: number, time: number) => {
    if (!Number.isFinite(time)) return;
    setPointsState((list) => list.map((p, j) => (j === i ? { ...p, time } : p)));
    preview();
  };
  const str = (key: string, fallback = ''): string => {
    const v = style()[key];
    return v === undefined ? fallback : String(v);
  };
  const num = (key: string, fallback: number): number => {
    const v = Number(style()[key]);
    return Number.isFinite(v) ? v : fallback;
  };
  const flag = (key: string, fallback = false): boolean => {
    const v = style()[key];
    return v === undefined ? fallback : v === true || v === 1 || v === '1' || v === 'true';
  };

  const cancel = () => {
    props.onPreview(original);
    props.onCancel();
  };
  // A position's journal side, saved once the drawing is: its trade follows the drawing.
  let saveJournal: (() => Promise<void>) | undefined;
  const ok = () => {
    const after = saveJournal;
    void Promise.resolve(props.onCommit(patch()))
      .then(() => after?.())
      .catch(() => {});
  };

  // Drag the dialog by its header, kept inside the window.
  const [pos, setPos] = createSignal<{ x: number; y: number } | null>(null);
  let panel!: HTMLDivElement;
  const startDrag = (e: PointerEvent) => {
    if ((e.target as HTMLElement).closest('button')) return;
    const r = panel.getBoundingClientRect();
    const parent = panel.offsetParent?.getBoundingClientRect() ?? { left: 0, top: 0 };
    const dx = e.clientX - r.left;
    const dy = e.clientY - r.top;
    const move = (ev: PointerEvent) => setPos({ x: ev.clientX - dx - parent.left, y: Math.max(0, ev.clientY - dy - parent.top) });
    const up = () => {
      removeEventListener('pointermove', move);
      removeEventListener('pointerup', up);
    };
    addEventListener('pointermove', move);
    addEventListener('pointerup', up);
  };

  onMount(() => {
    // The dialog keeps the keyboard: a letter typed here must not open the symbol search.
    const onKey = (e: KeyboardEvent) => {
      e.stopPropagation();
      if (e.key === 'Escape') cancel();
      else if (e.key === 'Enter' && !(e.target instanceof HTMLTextAreaElement)) {
        // A field's change lands as it loses focus: first, or Enter saves what was there before the typing.
        if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) e.target.blur();
        ok();
      }
    };
    panel.addEventListener('keydown', onKey);
    onCleanup(() => panel.removeEventListener('keydown', onKey));
    queueMicrotask(() => panel.focus());
  });

  return (
    <div
      ref={panel}
      classList={{ dsettings: true, wide: isPosition(kind) }}
      tabIndex={-1}
      style={pos() ? { left: `${pos()!.x}px`, top: `${pos()!.y}px`, right: 'auto' } : {}}
      onContextMenu={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    >
      <header onPointerDown={startDrag}>
        <span class="title">{KIND_NAMES[kind] ?? kind}</span>
        <span class="spacer" />
        <button class="x" data-tip="Cancel (esc)" onClick={cancel}>
          ✕
        </button>
      </header>
      <nav class="dtabs">
        <For each={tabs}>
          {(t) => (
            <button classList={{ on: tab() === t }} onClick={() => setTab(t)}>
              {t[0]!.toUpperCase() + t.slice(1)}
            </button>
          )}
        </For>
      </nav>

      <div class="dbody">
        <Show when={tab() === 'inputs'}>
          <PositionTicket
            drawingId={props.drawing.id}
            kind={kind}
            style={style()}
            points={points()}
            precision={props.precision}
            set={set}
            merge={merge}
            setPrice={setPrice}
            num={num}
            str={str}
            register={(save) => (saveJournal = save)}
          />
        </Show>

        <Show when={tab() === 'style'}>
          <Show when={isPosition(kind)}>
            <Row label="Lines">
              <ColorField value={str('color', '#9598a1')} onChange={(v) => set('color', v)} />
            </Row>
            <Row label="Stop color">
              <ColorField value={str('stopLine', '#f2364566')} onChange={(v) => set('stopLine', v)} />
              <ColorField value={str('stopFill', '#f236451a')} onChange={(v) => set('stopFill', v)} title="Stop background" />
            </Row>
            <Row label="Target color">
              <ColorField value={str('profitLine', '#d1d4dc')} onChange={(v) => set('profitLine', v)} />
              <ColorField value={str('profitFill', '#d1d4dc1a')} onChange={(v) => set('profitFill', v)} title="Target background" />
            </Row>
            <Row label="Text">
              <ColorField value={str('textColor', '#ffffff')} onChange={(v) => set('textColor', v)} />
              <SizeSelect value={num('textSize', 11)} onChange={(v) => set('textSize', v)} />
            </Row>
            <Check label="Compact stats mode" checked={flag('compact')} onChange={(v) => set('compact', v)} />
            <Check label="Always show stats" checked={flag('alwaysStats')} onChange={(v) => set('alwaysStats', v)} />
          </Show>

          <Show when={kind === 'text'}>
            <Row label="Text">
              <ColorField value={str('textColor', '#2962ff')} onChange={(v) => set('textColor', v)} />
              <SizeSelect value={num('textSize', 14)} onChange={(v) => set('textSize', v)} />
            </Row>
            <textarea class="dtext" rows={4} value={label() ?? ''} onInput={(e) => setLabel(e.currentTarget.value)} />
            <Row label="Background">
              <input type="checkbox" checked={str('textBg') !== ''} onChange={(e) => set('textBg', e.currentTarget.checked ? '#2962ff33' : '')} />
              <Show when={str('textBg') !== ''}>
                <ColorField value={str('textBg')} onChange={(v) => set('textBg', v)} />
              </Show>
            </Row>
          </Show>

          <Show when={kind === 'channel' || kind === 'fib'}>
            <LevelsEditor
              kind={kind}
              raw={style().levels}
              mainColor={str('color', kind === 'channel' ? '#2962ff' : '#787b86')}
              onMainColor={(c) => set('color', c)}
              onChange={(levels) => set('levels', serializeLevels(levels))}
            />
            <Show when={kind === 'fib'}>
              <Row label="Trend line">
                <LineField color={str('color', '#787b86')} width={num('width', 1)} dash={str('dash', 'solid') as Dash} set={set} />
              </Row>
              <Check label="Labels" checked={flag('levelLabels', true)} onChange={(v) => set('levelLabels', v)} />
            </Show>
            <Show when={kind === 'channel'}>
              <Row label="Lines">
                <LineField color={str('color', '#2962ff')} width={num('width', 1)} dash={str('dash', 'solid') as Dash} set={set} />
              </Row>
            </Show>
            <ExtendRow flag={flag} set={set} />
            <Row label="Background">
              <input type="checkbox" checked={flag('fillOn', true)} onChange={(e) => set('fillOn', e.currentTarget.checked)} />
              <ColorField value={str('fill', '#2962ff1f')} onChange={(v) => set('fill', v)} />
            </Row>
          </Show>

          <Show when={kind === 'rect'}>
            <Row label="Border">
              <LineField color={str('color', '#9c27b0')} width={num('width', 1)} dash={str('dash', 'solid') as Dash} set={set} />
            </Row>
            <Row label="Background">
              <input type="checkbox" checked={flag('fillOn', true)} onChange={(e) => set('fillOn', e.currentTarget.checked)} />
              <ColorField value={str('fill', '#9c27b033')} onChange={(v) => set('fill', v)} />
            </Row>
            <Check label="Middle line" checked={flag('middle')} onChange={(v) => set('middle', v)} />
            <ExtendRow flag={flag} set={set} />
          </Show>

          <Show when={kind === 'measure'}>
            <Row label="Color">
              <ColorField value={str('color', '#2962ff')} onChange={(v) => set('color', v)} />
              <ColorField value={str('fill', '#2962ff33')} onChange={(v) => set('fill', v)} title="Background" />
            </Row>
          </Show>

          <Show when={LINE_KINDS.has(kind)}>
            <Row label="Line">
              <LineField color={str('color', '#2962ff')} width={num('width', 1.5)} dash={str('dash', 'solid') as Dash} set={set} />
              <Show when={kind === 'trendline'}>
                <ArrowToggle title="Left end" on={flag('leftArrow')} left onChange={(v) => set('leftArrow', v)} />
              </Show>
              <Show when={kind === 'trendline' || kind === 'path'}>
                <ArrowToggle title="Right end" on={flag('rightArrow')} onChange={(v) => set('rightArrow', v)} />
              </Show>
            </Row>
            <Show when={kind === 'trendline'}>
              <ExtendRow flag={flag} set={set} />
              <Check label="Middle point" checked={flag('middle')} onChange={(v) => set('middle', v)} />
            </Show>
            <Show when={kind === 'trendline' || kind === 'ray' || kind === 'horizontal'}>
              <Check label="Price labels" checked={flag('priceLabels')} onChange={(v) => set('priceLabels', v)} />
            </Show>
            <Show when={kind === 'trendline'}>
              <div class="dsection">Info</div>
              <Check label="Stats — price change, percent and bars" checked={flag('stats')} onChange={(v) => set('stats', v)} />
            </Show>
          </Show>
        </Show>

        <Show when={tab() === 'text'}>
          <Row label="Text">
            <ColorField value={str('textColor', str('color', '#2962ff'))} onChange={(v) => set('textColor', v)} />
            <SizeSelect value={num('textSize', 11)} onChange={(v) => set('textSize', v)} />
          </Row>
          <textarea class="dtext" rows={3} placeholder="Add text" value={label() ?? ''} onInput={(e) => setLabel(e.currentTarget.value)} />
        </Show>

        <Show when={tab() === 'coordinates'}>
          <For each={points()}>
            {(p, i) => (
              <Row label={`#${i() + 1}`}>
                <input class="dinput num" type="number" step="any" data-tip="Price" value={Number(p.price.toFixed(props.precision))} onChange={(e) => setPrice(i(), Number(e.currentTarget.value))} />
                <input
                  class="dinput"
                  type="datetime-local"
                  data-tip="Time, UTC"
                  value={new Date(p.time).toISOString().slice(0, 16)}
                  onChange={(e) => setTime(i(), Date.parse(`${e.currentTarget.value}:00Z`))}
                />
              </Row>
            )}
          </For>
          <p class="dnote">Times are UTC.</p>
        </Show>

        <Show when={tab() === 'visibility'}>
          <p class="dnote">Show this drawing on timeframes in:</p>
          <For each={VISIBILITY_UNITS}>
            {(u) => <Check label={u.label} checked={visibleOn(style(), `1${u.unit}`)} onChange={(v) => setStyle(withHidden(style(), u.unit, !v))} />}
          </For>
        </Show>
      </div>

      <footer>
        <div class="dtemplate">
          <button class="dbtn" onClick={() => setTemplateMenu((v) => (v ? null : true))} onPointerDown={(e) => e.stopPropagation()}>
            Template ▾
          </button>
          <Show when={templateMenu()}>
            <div class="menu up" onPointerDown={(e) => e.stopPropagation()}>
              <button
                onClick={() => {
                  saveDefaultStyle(kind, style());
                  setTemplateMenu(null);
                }}
              >
                Save as default
              </button>
              <button
                onClick={() => {
                  setStyle({ ...defaultStyle(kind), hideOn: style().hideOn ?? '' });
                  setTemplateMenu(null);
                }}
              >
                Apply default
              </button>
              <button
                onClick={() => {
                  resetDefaultStyle(kind);
                  setStyle(factoryStyle(kind));
                  setTemplateMenu(null);
                }}
              >
                Reset to factory
              </button>
            </div>
          </Show>
        </div>
        <span class="spacer" />
        <button class="dbtn" onClick={cancel}>
          Cancel
        </button>
        <button class="dbtn primary" onClick={ok}>
          Ok
        </button>
      </footer>
    </div>
  );
}

// ---------------------------------------------------------------- pieces

function Row(props: { label: string; children: JSX.Element }) {
  return (
    <div class="drow">
      <span class="dlabel">{props.label}</span>
      <div class="dctl">{props.children}</div>
    </div>
  );
}

function Check(props: { label: string; checked: boolean; onChange(v: boolean): void }) {
  return (
    <label class="dcheck">
      <input type="checkbox" checked={props.checked} onChange={(e) => props.onChange(e.currentTarget.checked)} />
      {props.label}
    </label>
  );
}

function SizeSelect(props: { value: number; onChange(v: number): void }) {
  return (
    <select class="dinput narrow" value={String(props.value)} onChange={(e) => props.onChange(Number(e.currentTarget.value))}>
      <For each={[8, 10, 11, 12, 14, 16, 20, 24, 28, 32, 40]}>{(s) => <option value={s}>{s}</option>}</For>
    </select>
  );
}

function ExtendRow(props: { flag(key: string, fallback?: boolean): boolean; set(key: string, v: boolean): void }) {
  const value = () => (props.flag('extendLeft') ? (props.flag('extendRight') ? 'both' : 'left') : props.flag('extendRight') ? 'right' : 'none');
  return (
    <Row label="Extend">
      <select
        class="dinput"
        value={value()}
        onChange={(e) => {
          const v = e.currentTarget.value;
          props.set('extendLeft', v === 'left' || v === 'both');
          props.set('extendRight', v === 'right' || v === 'both');
        }}
      >
        <option value="none">Don't extend</option>
        <option value="left">Extend left</option>
        <option value="right">Extend right</option>
        <option value="both">Extend both</option>
      </select>
    </Row>
  );
}

function ArrowToggle(props: { title: string; on: boolean; left?: boolean; onChange(v: boolean): void }) {
  return (
    <button class="dbtn icon" classList={{ on: props.on }} data-tip={`${props.title}: ${props.on ? 'arrow' : 'normal'}`} onClick={() => props.onChange(!props.on)}>
      {props.left ? (props.on ? '←—' : '○—') : props.on ? '—→' : '—○'}
    </button>
  );
}

function LineField(props: { color: string; width: number; dash: Dash; set(key: string, v: string | number): void }) {
  return (
    <>
      <ColorField value={props.color} onChange={(v) => props.set('color', v)} />
      <select class="dinput narrow" data-tip="Thickness" value={String(props.width)} onChange={(e) => props.set('width', Number(e.currentTarget.value))}>
        <For each={[1, 1.5, 2, 3, 4]}>{(w) => <option value={w}>{w}px</option>}</For>
      </select>
      <DashSelect value={props.dash} onChange={(v) => props.set('dash', v)} />
    </>
  );
}

function DashSelect(props: { value: Dash; onChange(v: Dash): void }) {
  return (
    <select class="dinput narrow" data-tip="Line style" value={props.value} onChange={(e) => props.onChange(e.currentTarget.value as Dash)}>
      <option value="solid">───</option>
      <option value="dashed">- - -</option>
      <option value="dotted">·····</option>
    </select>
  );
}

function LevelsEditor(props: { kind: string; raw: unknown; mainColor: string; onMainColor(c: string): void; onChange(levels: Level[]): void }) {
  const levels = () => parseLevels(props.raw, props.kind);
  const update = (i: number, patch: Partial<Level>) => props.onChange(levels().map((l, j) => (j === i ? { ...l, ...patch } : l)));
  // A channel's own two lines are always drawn.
  const fixed = (l: Level) => props.kind === 'channel' && (l.value === 0 || l.value === 1);
  return (
    <div class="dlevels">
      <For each={levels()}>
        {(l, i) => (
          <div class="dlevel" classList={{ off: !l.on && !fixed(l) }}>
            <span class="cb">
              <Show when={!fixed(l)}>
                <input type="checkbox" checked={l.on} onChange={(e) => update(i(), { on: e.currentTarget.checked })} />
              </Show>
            </span>
            <input
              class="dinput num"
              type="number"
              step="any"
              value={l.value}
              disabled={fixed(l)}
              onChange={(e) => {
                const v = Number(e.currentTarget.value);
                if (Number.isFinite(v)) update(i(), { value: v });
              }}
            />
            {/* A channel's own lines take the drawing's colour. */}
            <ColorField value={fixed(l) ? props.mainColor : l.color || props.mainColor} onChange={(c) => (fixed(l) ? props.onMainColor(c) : update(i(), { color: c }))} />
            <DashSelect value={l.dash} onChange={(d) => update(i(), { dash: d })} />
          </div>
        )}
      </For>
    </div>
  );
}
