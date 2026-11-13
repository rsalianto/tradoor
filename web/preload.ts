/**
 * Hints in the built page for what its first screen needs beyond the entry's
 * own imports, so the browser fetches them alongside it rather than once it
 * has run: the chart engine's worker and its WebAssembly, the other
 * exchanges' streams (the watchlist's indices and index perpetuals), and the
 * panels the default layout opens. Every deploy renames them, so a returning
 * visitor loads them afresh after each, and the wait is that of the slowest
 * rather than of them in a row.
 */
import type { HtmlTagDescriptor, Plugin } from 'vite';

const MODULES = /^assets\/(worker|streams-more|WatchlistPanel|ValuesPanel|LiquidationsPanel|common)-[\w-]+\.js$/;
const WASM = /^assets\/tradoor_wasm_bg-[\w-]+\.wasm$/;

export function preload(): Plugin {
  return {
    name: 'tradoor-preload',
    apply: 'build',
    transformIndexHtml: {
      order: 'post',
      handler(_html, ctx): HtmlTagDescriptor[] {
        const files = Object.keys(ctx.bundle ?? {});
        // The app's panels are the app's: the showcase loads only the engine.
        const app = ctx.path === '/index.html';
        return [
          ...files.filter((f) => app && MODULES.test(f)).map((f): HtmlTagDescriptor => ({ tag: 'link', attrs: { rel: 'modulepreload', href: `/${f}` }, injectTo: 'head' })),
          // Fetched by the worker, so only the browser's cache carries it over; a
          // preload the page itself never uses is warned of, a prefetch is not.
          // No `crossorigin`: Cloudflare refuses a prefetch that sends an Origin
          // (503, `cf-speculation-refused: prefetch refused: not eligible`).
          ...files.filter((f) => WASM.test(f)).map((f): HtmlTagDescriptor => ({ tag: 'link', attrs: { rel: 'prefetch', href: `/${f}`, as: 'fetch' }, injectTo: 'head' })),
        ];
      },
    },
  };
}
