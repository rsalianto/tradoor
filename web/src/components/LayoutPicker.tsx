/**
 * The workspace's layout menu, opened from its name in the top bar: templates
 * of one chart to twenty, as TradingView has them, and what every chart keeps
 * in step. The side panels keep their place at the right; see
 * workspace/templates.
 */
import { For } from 'solid-js';
import type { SyncState } from '../workspace/model';
import { doc, setSync, update } from '../workspace/store';
import { applyTemplate, cellRects, surplus, TEMPLATES, type Template } from '../workspace/templates';

/** The template's cells, drawn small. */
function TemplateIcon(props: { template: Template }) {
  return (
    <svg viewBox="0 0 26 18" width="26" height="18" aria-hidden="true">
      <For each={cellRects(props.template.shape)}>
        {([x, y, w, h]) => <rect x={1 + x * 24} y={1 + y * 16} width={w * 24} height={h * 16} rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />}
      </For>
    </svg>
  );
}

const SYNCS: { key: keyof SyncState; label: string; tip: string }[] = [
  { key: 'symbol', label: 'Symbol', tip: 'Every chart on one ticker: change it on one and they all change' },
  { key: 'interval', label: 'Interval', tip: 'Every chart on one timeframe' },
];

export function LayoutPicker(props: { onClose(): void }) {
  const counts = [...new Set(TEMPLATES.map((t) => t.count))];
  const pick = (t: Template) => {
    const n = surplus(doc, t);
    if (n > 0 && !confirm(`This layout has room for ${t.count} chart${t.count > 1 ? 's' : ''}: the last ${n} will be closed. Their drawings stay with their tickers.`)) return;
    update((d) => void applyTemplate(d, t));
    props.onClose();
  };
  return (
    <div class="menu layout-menu" onPointerDown={(e) => e.stopPropagation()}>
      <div class="cap">Charts on screen</div>
      <For each={counts}>
        {(count) => (
          <div class="lm-row">
            <span class="lm-count num">{count}</span>
            <For each={TEMPLATES.filter((t) => t.count === count)}>
              {(t) => (
                <button class="lm-tpl" data-tip={t.label} aria-label={`${t.label}: ${t.count} chart${t.count > 1 ? 's' : ''}`} onClick={() => pick(t)}>
                  <TemplateIcon template={t} />
                </button>
              )}
            </For>
          </div>
        )}
      </For>
      <div class="cap lm-sync-cap">Keep in step</div>
      <For each={SYNCS}>
        {(s) => (
          <button class="lm-sync" role="switch" aria-checked={doc.sync[s.key]} data-tip={s.tip} onClick={() => setSync(s.key, !doc.sync[s.key])}>
            <span>{s.label}</span>
            <span class="switch" classList={{ on: doc.sync[s.key] }} />
          </button>
        )}
      </For>
    </div>
  );
}
