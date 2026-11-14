import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import { compress } from './compress.ts';
import { preload } from './preload.ts';
import { site } from './site.ts';

declare const process: { env: Record<string, string | undefined> };

/** Where `tradoor-server` listens in development. The page asks for /api and
 *  /data on its own origin, exactly as it will behind nginx in production. */
const server = process.env.TRADOOR_API ?? 'http://127.0.0.1:8080';

export default defineConfig({
  plugins: [solid(), site(), preload(), compress()],
  server: {
    port: 5173,
    // ws: the exchange stream relay for networks that block the exchanges.
    proxy: { '/api': { target: server, ws: true }, '/data': server },
    // The built-in indicators' formulas live with the formula crate, and its tests read them too.
    fs: { allow: ['.', '../crates/frost-script/sources'] },
  },
  worker: {
    format: 'es',
    // One file, its lazy imports inlined. A chunk split off the worker imports
    // back from the worker's own file, and Safari runs that file a second time
    // in the same worker on the first such import: the copy takes over the
    // messages with no charts, while the first keeps drawing what it had — on
    // an iPhone, a chart that stopped answering after a Bitget market.
    rolldownOptions: { output: { codeSplitting: false } },
  },
  build: {
    target: 'es2022',
    modulePreload: { polyfill: false },
    assetsInlineLimit: 0,
    // The app, and the Community's showcase: a page of its own that draws scripts with the same engine.
    rolldownOptions: { input: { index: 'index.html', community: 'community.html' } },
  },
});
