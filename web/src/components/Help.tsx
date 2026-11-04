import { For } from 'solid-js';

const GROUPS: { title: string; keys: [string, string][] }[] = [
  {
    title: 'Workspace',
    keys: [
      ['/  Ctrl K', 'Commands'],
      ['?', 'This help'],
      ['Ctrl S', 'Save the workspace now — it saves itself anyway'],
      ['drag a tab', 'Dock it: centre or edge of a group'],
      ['link icon on a tab', 'Link group — panels share a market'],
      ['middle-click tab', 'Close the panel'],
    ],
  },
  {
    title: 'Chart',
    keys: [
      ['a–z', 'Search for a symbol'],
      ['←  →', 'Shorter / longer timeframe'],
      ['↑  ↓', 'Previous / next market in the watchlist, or the Signal list'],
      ['wheel', 'Zoom time'],
      ['drag', 'Pan time; up or down moves price'],
      ['wheel or drag price axis', 'Scale price'],
      ['AUTO', 'Price follows the bars again'],
      ['Alt ↵', 'This chart alone over the layout, and back; or Maximize, under the chart'],
      ['double-click price axis', 'Auto scale that pane'],
      ['right-click', 'Chart menu'],
    ],
  },
  {
    title: 'Alerts',
    keys: [
      ['clock by the price axis', 'Add a crossing alert at the price under the pointer'],
      ['hover an alert line', 'Its name, with delete'],
      ['drag an alert line', 'Move it to another price'],
      ['Alerts panel', 'Every alert, Active or Stopped — Triggered: restart, edit, delete; the Log lists each trigger'],
    ],
  },
  {
    title: 'Watchlist',
    keys: [
      ['right-click', 'Flag a market, add a section or a symbol'],
      ['drag a row', 'Move it; a section moves with its markets'],
      ['click a section', 'Fold it shut or open'],
      ['double-click a section', 'Rename it'],
    ],
  },
  {
    title: 'Drawing',
    keys: [
      ['Ctrl Z', 'Undo the last drawing change'],
      ['Ctrl Shift Z  Ctrl Y', 'Redo'],
      ['esc', 'Cancel the tool, or deselect'],
      ['Delete', 'Remove the selected drawing'],
      ['Enter', 'Finish a path'],
      ['double-click', 'Finish a path'],
      ['double-click a drawing', 'Its settings: style, text, coordinates, visibility'],
      ['shift', 'Keep a line level while drawing or dragging it'],
      ['Alt T / H / J / R', 'Trendline, horizontal level, ray, zone'],
      ['Alt C / F / N / M', 'Parallel channel, fib, note, measure'],
      ['Alt L / S / B / P', 'Long, short, brush, path'],
      ['Alt G', 'Magnet: off, weak, strong'],
      ['Alt K', 'Copy a 4K picture of the chart — shift saves the file'],
      ['hold Ctrl', 'A weak magnet while drawing, when it is off'],
    ],
  },
  {
    title: 'Symbol search',
    keys: [
      ['↑  ↓', 'Move'],
      ['↵', 'Open'],
      ['tab', 'All / perp / spot'],
      ['bybit btc', 'Venue and kind words filter'],
      ['esc', 'Close'],
    ],
  },
];

export function Help(props: { onClose(): void }) {
  return (
    <div class="backdrop" onPointerDown={() => props.onClose()}>
      <div
        class="dialog"
        tabIndex={-1}
        ref={(el) => queueMicrotask(() => el.focus())}
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key !== 'Escape' && e.key !== '?') return;
          e.stopPropagation();
          props.onClose();
        }}
      >
        <header>
          <span class="title caps">Keyboard</span>
          <span class="spacer" />
          <button class="x" onClick={() => props.onClose()}>
            esc ✕
          </button>
        </header>
        <div class="keys">
          <For each={GROUPS}>
            {(g) => (
              <div>
                <h3 class="caps primary" style={{ margin: '0 0 6px', 'font-weight': '400' }}>
                  {g.title}
                </h3>
                <dl>
                  <For each={g.keys}>
                    {([key, label]) => (
                      <div>
                        <dd>{label}</dd>
                        <dt>
                          <kbd>{key}</kbd>
                        </dt>
                      </div>
                    )}
                  </For>
                </dl>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
