/**
 * A chart's TPO settings, in the tabs Exocharts puts them under. Every change
 * applies at once; a value set back to its default is forgotten.
 */
import { createSignal, For, Match, Switch } from 'solid-js';
import { TPO_SECTIONS, type TpoField } from '../lib/tpo-schema';
import { tpoValue, withTpoValue } from '../lib/tpo-settings';
import type { ChartPanelState } from '../workspace/model';
import { doc, patchPanel } from '../workspace/store';

export function TpoSettings(props: { panel: ChartPanelState; onClose(): void }) {
  const [tab, setTab] = createSignal(0);
  // Unset, the blocks and the point of control take the theme's colours (Settings, Themes).
  const themed = (key: string) => (key === 'vaColor' ? doc.theme.tpo : key === 'pocColor' ? doc.theme.tpoPoc : undefined);
  const value = (key: string) => props.panel.tpoSettings?.[key] ?? themed(key) ?? tpoValue(props.panel.tpoSettings, key);
  const set = (key: string, v: string) => patchPanel(props.panel.id, { tpoSettings: withTpoValue(props.panel.tpoSettings, key, v) });

  const Control = (p: { field: TpoField }) => (
    <label class="swatch" data-tip={p.field.hint}>
      <Switch>
        <Match when={p.field.kind === 'bool'}>
          <input type="checkbox" checked={value(p.field.key) === '1'} onChange={(e) => set(p.field.key, e.currentTarget.checked ? '1' : '0')} />
          <span class="name">{p.field.label}</span>
        </Match>
        <Match when={p.field.kind === 'color'}>
          <input type="color" value={value(p.field.key)} onInput={(e) => set(p.field.key, e.currentTarget.value)} />
          <span class="name">{p.field.label}</span>
          <span class="num dim">{value(p.field.key)}</span>
        </Match>
        <Match when={p.field.kind === 'number' && p.field}>
          {(f) => {
            const field = f() as Extract<TpoField, { kind: 'number' }>;
            return (
              <>
                <span class="name">{field.label}</span>
                <input
                  class="box num"
                  type="number"
                  min={field.min}
                  max={field.max}
                  step={field.step ?? 1}
                  value={value(field.key)}
                  onChange={(e) => {
                    const n = Number(e.currentTarget.value);
                    if (Number.isFinite(n) && n >= field.min && n <= field.max) set(field.key, String(n));
                    else e.currentTarget.value = value(field.key);
                  }}
                />
              </>
            );
          }}
        </Match>
        <Match when={p.field.kind === 'choice' && p.field}>
          {(f) => {
            const field = f() as Extract<TpoField, { kind: 'choice' }>;
            return (
              <>
                <span class="name">{field.label}</span>
                <select class="box" onChange={(e) => set(field.key, e.currentTarget.value)}>
                  <For each={field.options}>
                    {(o) => (
                      <option value={o.value} selected={value(field.key) === o.value}>
                        {o.label}
                      </option>
                    )}
                  </For>
                </select>
              </>
            );
          }}
        </Match>
      </Switch>
    </label>
  );

  return (
    <div class="backdrop" onPointerDown={() => props.onClose()}>
      <div
        class="dialog medium"
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
          <button class="dim" data-tip="Every TPO option back to its default" onClick={() => patchPanel(props.panel.id, { tpoSettings: {} })}>
            reset
          </button>
          <button class="x" onClick={() => props.onClose()}>
            esc ✕
          </button>
        </header>
        <div class="tabs-row">
          <For each={TPO_SECTIONS}>
            {(section, i) => (
              <button class="tb" classList={{ on: tab() === i() }} onClick={() => setTab(i())}>
                {section.title}
              </button>
            )}
          </For>
        </div>
        <div class="form">
          <div class="tpo-fields">
            <For each={TPO_SECTIONS[tab()]!.fields}>{(field) => <Control field={field} />}</For>
          </div>
        </div>
      </div>
    </div>
  );
}
