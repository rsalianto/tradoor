import { lazy as solidLazy, type Component } from 'solid-js';

/**
 * In place of a part of the page whose code could not be fetched. A browser holds
 * a failed import against the page for as long as it stays open, so asking again
 * from here would fail too: only loading the page anew can cure it.
 */
function NotLoaded() {
  return (
    <div class="not-loaded" role="alert">
      <span>This part of Tradoor did not load: the connection to the server dropped.</span>
      <button class="tb" onClick={() => location.reload()}>
        Reload the page
      </button>
    </div>
  );
}

// biome-ignore lint/suspicious/noExplicitAny: the same bound as Solid's own lazy.
export function lazy<T extends Component<any>>(load: () => Promise<{ default: T }>) {
  return solidLazy(() => load().catch(() => ({ default: NotLoaded as unknown as T })));
}
