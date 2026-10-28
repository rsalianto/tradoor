/**
 * The drawing tools down a chart's left edge: the tool armed, the magnet,
 * undo and redo, and the drawings' delete and clear. A workspace showing one
 * chart gives it its own; one showing several keeps a single rail at its left
 * for all of them, as TradingView does: the tool armed there draws on whichever
 * chart is clicked next, and the rest act on the chart in use.
 */
import { For, Show } from 'solid-js';
import { armedTool, cycleMagnet, magnet, setArmedTool, TOOL_KEYS, type ChartController, type MagnetMode } from '../app/ui';
import { Icon, type IconName } from '../lib/icons';

const TOOLS: { kind: string; icon: IconName; title: string }[] = [
  { kind: '', icon: 'pointer', title: 'Select and pan (Esc) — click a drawing to select it, then Delete to remove it' },
  { kind: 'path', icon: 'path', title: 'Path — click each turn, then Enter or double-click to finish' },
  { kind: 'trendline', icon: 'trendline', title: 'Trendline — click the two ends; anchored to time and price' },
  { kind: 'horizontal', icon: 'horizontal', title: 'Horizontal level — one click, drawn edge to edge' },
  { kind: 'ray', icon: 'ray', title: 'Ray — one click; the level starts where you put it' },
  { kind: 'rect', icon: 'rect', title: 'Zone — click opposite corners' },
  { kind: 'channel', icon: 'channel', title: 'Parallel channel — click the two ends of one line, then where the parallel line runs' },
  { kind: 'fib', icon: 'fib', title: 'Fib retracement — click the 100% end first, then the 0% end' },
  { kind: 'text', icon: 'text', title: 'Note — click where it goes, then type it' },
  { kind: 'measure', icon: 'measure', title: 'Measure — click from and to; reports the move, percent and bars' },
  { kind: 'long', icon: 'long', title: 'Long position — click the entry; drag its handles to set stop, target and width' },
  { kind: 'short', icon: 'short', title: 'Short position — click the entry; drag its handles to set stop, target and width' },
  { kind: 'brush', icon: 'brush', title: 'Brush — drag to draw freehand' },
];

/** The tool's name with its Alt shortcut: `Trendline (Alt+T) — click the two ends…`. */
const toolTip = (t: { kind: string; title: string }): string => {
  const key = Object.entries(TOOL_KEYS).find(([, kind]) => kind === t.kind)?.[0];
  if (!key) return t.title;
  const [name, ...rest] = t.title.split(' — ');
  return [`${name} (Alt+${key.slice(3)})`, ...rest].join(' — ');
};

const MAGNET_TIP: Record<MagnetMode, string> = {
  off: 'Magnet off (Alt+G) — click for a weak magnet, or hold Ctrl while drawing for one',
  weak: 'Weak magnet (Alt+G) — anchors near a bar take its open, high, low or close; elsewhere a whole tick. Click for strong',
  strong: 'Strong magnet (Alt+G) — anchors always take the nearest open, high, low or close. Click to turn it off',
};

/** `target`: the chart the undo, delete and clear act on — its own, or the one in use. */
export function DrawingRail(props: { target: () => ChartController | undefined }) {
  const offline = () => props.target()?.offline() ?? null;
  const count = () => props.target()?.drawingCount() ?? 0;
  return (
    <div class="rail">
      <For each={TOOLS}>
        {(t) => (
          <button
            classList={{ on: armedTool() === t.kind }}
            disabled={t.kind !== '' && offline() !== null}
            data-tip={t.kind !== '' && offline() ? `${t.title.split(' — ')[0]} — unavailable: drawings are saved on the tradoor server (${offline()})` : toolTip(t)}
            onClick={() => setArmedTool(armedTool() === t.kind ? '' : t.kind)}
          >
            <Icon name={t.icon} />
          </button>
        )}
      </For>
      <hr />
      <button class="magnet" classList={{ on: magnet() !== 'off', strong: magnet() === 'strong' }} data-tip={MAGNET_TIP[magnet()]} onClick={cycleMagnet}>
        <Icon name="magnet" />
      </button>
      <hr />
      <button disabled={!props.target()?.canUndo()} data-tip="Undo (Ctrl Z)" onClick={() => props.target()?.undo()}>
        <Icon name="undo" />
      </button>
      <button disabled={!props.target()?.canRedo()} data-tip="Redo (Ctrl Y)" onClick={() => props.target()?.redo()}>
        <Icon name="redo" />
      </button>
      <hr />
      <button
        class="del"
        disabled={props.target()?.selected() == null}
        data-tip={props.target()?.selected() == null ? 'Select a drawing on the chart first' : 'Delete the selected drawing (Delete)'}
        onClick={() => props.target()?.deleteSelected()}
      >
        <Icon name="trash" />
      </button>
      <button class="del" disabled={count() === 0} data-tip={`Remove all ${count()} drawings on this market, on every timeframe`} onClick={() => props.target()?.clearDrawings()}>
        <Icon name="eraser" />
      </button>
      <Show when={count() > 0}>
        <span class="count num">{count()}</span>
      </Show>
    </div>
  );
}
