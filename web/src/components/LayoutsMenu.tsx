/**
 * Layouts: the recommended ones to start from, the reader's own saved by name
 * (an account's, so they follow it to every device), and a layout as a JSON
 * file to keep or share. Opened from the top bar, and in Settings.
 *
 * Opening one replaces the layout in use, which the app saves as it goes, but
 * not the watchlists, which stay the reader's; a saved layout is a copy,
 * changed only by saving over it.
 */
import { createResource, For, Show } from 'solid-js';
import { setOverlay } from '../app/ui';
import { api, errorText } from '../lib/api';
import { Icon } from '../lib/icons';
import { session } from '../lib/owner';
import { formatDateTime } from '../lib/timezone';
import { toast } from '../lib/toasts';
import { validate, type WorkspaceDoc } from '../workspace/model';
import { applyLayout, keepWatchlists, LAYOUTS, watchlists } from '../workspace/presets';
import { doc, update } from '../workspace/store';

/** What a layout file or a saved layout brings: all of it but when it was saved — and the watchlists, which stay the reader's. */
function take(next: WorkspaceDoc): void {
  update((d) => {
    const lists = watchlists(d);
    Object.assign(d, { name: next.name, root: next.root, panels: next.panels, links: next.links, sync: next.sync, theme: next.theme, timeframes: next.timeframes });
    keepWatchlists(d, lists);
  });
}

const slug = (name: string): string => name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'layout';

export function LayoutsMenu(props: { onClose?(): void; inSettings?: boolean }) {
  const signedIn = () => session() !== null;
  const [saved, { refetch }] = createResource(signedIn, () => api.layouts());
  const done = () => props.onClose?.();

  const pick = (id: string) => {
    const preset = LAYOUTS.find((l) => l.id === id)!;
    if (!confirm(`Lay the workspace out as ${preset.name}? The layout in use is replaced; save it first to keep it.`)) return;
    update((d) => applyLayout(d, preset));
    done();
  };

  const open = async (id: string, name: string) => {
    if (!confirm(`Open ${name}? The layout in use is replaced; save it first to keep it.`)) return;
    try {
      const next = validate((await api.workspace(id)).doc);
      if (!next) throw new Error('it is not a layout this version reads');
      take(next);
      done();
    } catch (e) {
      toast({ tone: 'warn', title: `${name} not opened`, body: errorText(e) });
    }
  };

  const save = async () => {
    const name = prompt('Save this layout as', doc.name)?.trim();
    if (!name) return;
    const same = saved()?.layouts.find((l) => l.name === name);
    if (same && !confirm(`Save over ${name}?`)) return;
    try {
      update((d) => void (d.name = name));
      const id = same?.id ?? `l-${slug(name).slice(0, 40)}-${Math.random().toString(36).slice(2, 7)}`;
      await api.saveWorkspace(id, { name, doc: JSON.parse(JSON.stringify(doc)), version: same?.version ?? 0 });
      toast({ tone: 'info', title: `Saved ${name}`, body: 'Open it again from Layouts, on any device you sign in on.' });
      void refetch();
    } catch (e) {
      toast({ tone: 'warn', title: 'Layout not saved', body: errorText(e) });
    }
  };

  const remove = async (id: string, name: string) => {
    if (!confirm(`Delete ${name}? The layout in use stays as it is.`)) return;
    try {
      await api.deleteLayout(id);
      void refetch();
    } catch (e) {
      toast({ tone: 'warn', title: `${name} not deleted`, body: errorText(e) });
    }
  };

  const exportFile = () => {
    const blob = new Blob([JSON.stringify(doc, null, 2)], { type: 'application/json' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `tradoor-${slug(doc.name)}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1_000);
  };

  const importFile = (file: File | undefined) => {
    if (!file) return;
    void file.text().then((text) => {
      let next: WorkspaceDoc | null = null;
      try {
        next = validate(JSON.parse(text));
      } catch {
        next = null;
      }
      if (!next) {
        toast({ tone: 'warn', title: `${file.name} is not a tradoor layout`, body: 'Export one from Layouts to see what a layout file holds.' });
        return;
      }
      if (!confirm(`Open ${next.name} from ${file.name}? The layout in use is replaced; save it first to keep it.`)) return;
      take(next);
      done();
    });
  };

  let picker!: HTMLInputElement;
  return (
    <div class="layouts" classList={{ menu: !props.inSettings, 'layouts-menu': !props.inSettings }} onPointerDown={(e) => e.stopPropagation()}>
      <div class="cap">Recommended</div>
      <For each={LAYOUTS}>
        {(l) => (
          <button class="lo-row" onClick={() => pick(l.id)}>
            <span class="lo-name">{l.name}</span>
            <span class="lo-note">{l.note}</span>
          </button>
        )}
      </For>

      <div class="cap">Saved</div>
      <Show
        when={signedIn()}
        fallback={
          <button class="lo-row" onClick={() => (done(), setOverlay({ kind: 'settings' }))}>
            <span class="lo-note">Sign in with Telegram in Settings to save layouts of your own, on every device.</span>
          </button>
        }
      >
        <For each={saved()?.layouts ?? []} fallback={<div class="lo-empty">{saved.loading ? 'Loading…' : 'None yet: save the one in use below.'}</div>}>
          {(l) => (
            <div class="lo-saved">
              <button class="lo-row" onClick={() => void open(l.id, l.name)}>
                <span class="lo-name">{l.name}</span>
                <span class="lo-note">saved {formatDateTime(l.updatedAt).slice(0, 16)}</span>
              </button>
              <button class="x" aria-label={`Delete ${l.name}`} data-tip={`Delete ${l.name}`} onClick={() => void remove(l.id, l.name)}>
                <Icon name="close" size={10} stroke={2.4} />
              </button>
            </div>
          )}
        </For>
        <button class="lo-action" onClick={() => void save()}>
          <Icon name="plus" size={12} /> Save the layout in use
        </button>
      </Show>

      <div class="cap">File</div>
      <div class="lo-files">
        <button class="lo-action" data-tip="Download the layout in use as a JSON file" onClick={exportFile}>
          Export JSON
        </button>
        <button class="lo-action" data-tip="Open a layout from a JSON file" onClick={() => picker.click()}>
          Import JSON
        </button>
        <input ref={picker} type="file" accept="application/json,.json" hidden onChange={(e) => (importFile(e.currentTarget.files?.[0]), (e.currentTarget.value = ''))} />
      </div>
    </div>
  );
}
