/**
 * A chart's TPO settings, in the tabs MMT puts them under: the tabs down the
 * side, each one's fields beside them. Every change applies at once; a value
 * set back to its default is forgotten.
 */
import { createSignal, For, Index, Match, Show, Switch } from 'solid-js';
import { ColorField } from './ColorField';
import { TPO_SECTIONS, type TpoField } from '../lib/tpo-schema';
import { CHART_THEME } from '../lib/theme';
import { parseSessions, serializeSessions, themedTpoColor, TPO_DEFAULTS, tpoValue, withTpoValue, type TpoSession } from '../lib/tpo-settings';
import { TPO_SPLITS, tpoFits, type ChartPanelState } from '../workspace/model';
import { doc, patchPanel, timeframeOf } from '../workspace/store';

const pad = (n: number) => String(n).padStart(2, '0');
const clock = (m: number) => `${pad(Math.floor(m / 60) % 24)}:${pad(m % 60)}`;
const minutes = (v: string): number | null => {
  const m = /^(\d{1,2}):(\d{2})$/.exec(v.trim());
  return m && Number(m[1]) <= 23 && Number(m[2]) < 60 ? Number(m[1]) * 60 + Number(m[2]) : null;
};

export function TpoSettings(props: { panel: ChartPanelState; onClose(): void }) {
  const [tab, setTab] = createSignal(0);
  const value = (key: string) => tpoValue(props.panel.tpoSettings, key);
  // A colour left to the theme shows the theme's, as the chart draws it.
  const theme = (name: string) => doc.theme[name] ?? CHART_THEME.find((t) => t.name === name)?.value ?? '#000000';
  const shown = (key: string) => value(key) || themedTpoColor(key, theme);
  const set = (key: string, v: string) => patchPanel(props.panel.id, { tpoSettings: withTpoValue(props.panel.tpoSettings, key, v) });

  const Sessions = () => {
    const list = () => parseSessions(value('sessions'));
    const save = (next: TpoSession[]) => set('sessions', serializeSessions(next));
    const change = (i: number, patch: Partial<TpoSession>) => save(list().map((s, j) => (j === i ? { ...s, ...patch } : s)));
    return (
      <div class="tpo-sessions">
        <Index each={list()}>
          {(s, i) => (
            <div class="tpo-session">
              <span class="name">Session {i + 1}</span>
              <input
                class="box num"
                value={clock(s().start)}
                aria-label={`Session ${i + 1} start, UTC`}
                onChange={(e) => {
                  const m = minutes(e.currentTarget.value);
                  if (m === null) e.currentTarget.value = clock(s().start);
                  else change(i, { start: m });
                }}
              />
              <span class="dim">–</span>
              <input
                class="box num"
                value={clock(s().end)}
                aria-label={`Session ${i + 1} end, UTC`}
                onChange={(e) => {
                  const m = minutes(e.currentTarget.value);
                  if (m === null) e.currentTarget.value = clock(s().end);
                  else change(i, { end: m });
                }}
              />
              <ColorField value={s().color} onChange={(c) => change(i, { color: c.slice(0, 7) })} title={`Session ${i + 1} colour`} />
              <button class="dim" data-tip="Remove this session" aria-label={`Remove session ${i + 1}`} onClick={() => save(list().filter((_, j) => j !== i))}>
                ✕
              </button>
            </div>
          )}
        </Index>
        <Show when={list().length < 8}>
          <button
            class="btn"
            onClick={() => {
              const last = list().at(-1);
              const start = last ? last.end % 1440 : 0;
              save([...list(), { start, end: (start + 360) % 1440, color: '#9a9a9a' }]);
            }}
          >
            Add Session
          </button>
        </Show>
      </div>
    );
  };

  const Control = (p: { field: TpoField }) => (
    <div class="tpo-row" data-tip={p.field.hint}>
      <span class="name">{p.field.label}</span>
      <Switch>
        <Match when={p.field.kind === 'bool'}>
          <input type="checkbox" aria-label={p.field.label} checked={value(p.field.key) === '1'} onChange={(e) => set(p.field.key, e.currentTarget.checked ? '1' : '0')} />
        </Match>
        <Match when={p.field.kind === 'color'}>
          <ColorField value={shown(p.field.key)} onChange={(c) => set(p.field.key, c)} title={p.field.label} />
          <Show when={TPO_DEFAULTS[p.field.key] === '' && value(p.field.key) !== ''}>
            <button class="dim" data-tip="Back to the theme's colour" onClick={() => set(p.field.key, '')}>
              theme
            </button>
          </Show>
        </Match>
        <Match when={p.field.kind === 'ticks'}>
          <label class="tpo-auto">
            <input type="checkbox" checked={value('rowTicks') === '0'} onChange={(e) => set('rowTicks', e.currentTarget.checked ? '0' : '500')} />
            <span class="dim">auto</span>
          </label>
          <Show when={value('rowTicks') !== '0'}>
            <input
              class="box num"
              type="number"
              min={1}
              max={1_000_000}
              step={50}
              aria-label="Ticks per row"
              value={value('rowTicks')}
              onChange={(e) => {
                const n = Math.round(Number(e.currentTarget.value));
                if (Number.isFinite(n) && n >= 1 && n <= 1_000_000) set('rowTicks', String(n));
                else e.currentTarget.value = value('rowTicks');
              }}
            />
          </Show>
        </Match>
        <Match when={p.field.kind === 'sessions'}>
          <Sessions />
        </Match>
        <Match when={p.field.kind === 'period'}>
          <select class="box" aria-label="Session Period" onChange={(e) => patchPanel(props.panel.id, { tpo: e.currentTarget.value })}>
            <For each={TPO_SPLITS.filter((t) => tpoFits(t.value, timeframeOf(props.panel)))}>
              {(t) => (
                <option value={t.value} selected={props.panel.tpo === t.value}>
                  {t.label}
                </option>
              )}
            </For>
          </select>
        </Match>
        <Match when={p.field.kind === 'number' && p.field}>
          {(f) => {
            const field = f() as Extract<TpoField, { kind: 'number' }>;
            return (
              <input
                class="box num"
                type="number"
                min={field.min}
                max={field.max}
                step={field.step ?? 1}
                aria-label={field.label}
                value={value(field.key)}
                onChange={(e) => {
                  const n = Number(e.currentTarget.value);
                  if (Number.isFinite(n) && n >= field.min && n <= field.max) set(field.key, String(n));
                  else e.currentTarget.value = value(field.key);
                }}
              />
            );
          }}
        </Match>
        <Match when={p.field.kind === 'choice' && p.field}>
          {(f) => {
            const field = f() as Extract<TpoField, { kind: 'choice' }>;
            return (
              <select class="box" aria-label={field.label} onChange={(e) => set(field.key, e.currentTarget.value)}>
                <For each={field.options}>
                  {(o) => (
                    <option value={o.value} selected={value(field.key) === o.value}>
                      {o.label}
                    </option>
                  )}
                </For>
              </select>
            );
          }}
        </Match>
      </Switch>
    </div>
  );

  return (
    <div class="backdrop" onPointerDown={() => props.onClose()}>
      <div
        class="dialog tpo-dialog"
        tabIndex={-1}
        ref={(el) => queueMicrotask(() => el.focus())}
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key !== 'Escape') return;
          e.stopPropagation();
          props.onClose();
        }}
      >
        <header>
          <span class="title caps">TPO settings</span>
          <span class="spacer" />
          <button class="x" onClick={() => props.onClose()}>
            esc ✕
          </button>
        </header>
        <div class="set">
          <nav class="set-nav">
            <For each={TPO_SECTIONS}>
              {(section, i) => (
                <button classList={{ on: tab() === i() }} onClick={() => setTab(i())}>
                  {section.title}
                </button>
              )}
            </For>
          </nav>
          <div class="tpo-body">
            <For each={TPO_SECTIONS[tab()]!.fields}>{(field) => <Control field={field} />}</For>
          </div>
        </div>
        <footer>
          <button class="btn" data-tip="Every TPO option back to its default, colours to the theme's" onClick={() => patchPanel(props.panel.id, { tpoSettings: {} })}>
            Reset
          </button>
          <button class="btn primary" onClick={() => props.onClose()}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
