/**
 * The Community's showcase: every public Frost Script with its description and
 * its author, each drawn on the chart it was shared from by tradoor's own
 * engine — the worker and WebAssembly the app runs, on live data — so nothing
 * of a script is kept but its text. A card's picture is made one chart at a
 * time, taken and let go; the script opened stays live, to move about in.
 * Everything a reader wrote is put on the page as text, never as markup.
 *
 * It shares nothing with the app's own code but the worker: its own few lines
 * speak to the engine here, and the engine's own colours are tradoor's default
 * theme. Imported app modules would be split off into chunks the app then loads
 * on every visit.
 */
import type { Calls, ChartStatus, FromWorker, StudyMeta, ToWorker } from '../engine/protocol';
import './community.css';

interface Preview {
  market: string;
  timeframe: string;
  inputs: number[];
}
interface Listed {
  id: string;
  name: string;
  username: string;
  updated: number;
  created: number;
  about: string;
  more?: boolean;
  preview: Preview | null;
  /** One of tradoor's own, by its kind: a built-in indicator, written in Frost Script. */
  kind?: string;
}
interface Full extends Listed {
  source: string;
}

// ---------------------------------------------------------------- tradoor's own

/**
 * tradoor's own indicators, as Frost Script: the sources the app's Source tab
 * shows, read here under a query of the page's own, so the app's chunks stay
 * the app's. Each one's name and description are its first comment lines.
 */
const OWN_SOURCES = import.meta.glob<string>('../../../crates/frost-script/sources/*.frost', { query: '?raw&showcase', import: 'default', eager: true });
/** An example in the editor rather than one of the built-ins. */
const NOT_BUILT_IN = new Set(['supertrend']);
/** The timeframe each is best seen on; the rest on the hour. */
const OWN_TIMEFRAMES: Record<string, string> = { sessions: '15m', vwap: '15m', moon: '1d', ichimoku: '4h', swingmap: '4h' };

function ownScripts(): Full[] {
  return Object.entries(OWN_SOURCES)
    .flatMap(([path, source]) => {
      const kind = path.slice(path.lastIndexOf('/') + 1).replace(/\.frost$/, '');
      if (NOT_BUILT_IN.has(kind)) return [];
      const head: string[] = [];
      for (const line of source.split('\n')) {
        if (!line.startsWith('//')) break;
        head.push(line.replace(/^\/\/\s?/, '').replaceAll('`', ''));
      }
      const [first = kind, ...rest] = head;
      const colon = first.indexOf(': ');
      const name = colon > 0 ? first.slice(0, colon) : first;
      const sub = colon > 0 ? first.slice(colon + 2) : '';
      const about = [sub && `${sub[0]!.toUpperCase()}${sub.slice(1)}.`, rest.join(' ')].filter(Boolean).join(' ');
      return [{ id: `tradoor${kind}`, kind, name, username: 'tradoor', updated: 0, created: 0, about, preview: { market: 'binance-perp:BTCUSDT', timeframe: OWN_TIMEFRAMES[kind] ?? '1h', inputs: [] }, source }];
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}
const OWN = ownScripts();

/** Where a script shared before previews were kept is drawn. */
const DEFAULT_PREVIEW: Preview = { market: 'binance-perp:BTCUSDT', timeframe: '1h', inputs: [] };
/**
 * A card's chart is drawn at the card's own size and the screen's pixel density:
 * its text at the size it is read at, and as many bars as fit there — a big chart
 * shrunk to a card is a smudge of tiny labels. Within these, whatever the card.
 */
const THUMB_MIN = { width: 280, height: 158 };
const DPR_MAX = 2;
/** Tradoor's own layers, left off: a preview shows candles and the script. */
const LAYERS_OFF = 'ribbon,ema200,volume,liquidations';
/** How long a chart is given to load before its card says so. */
const LOAD_MS = 25_000;
/** After the bars and the script are in, a moment for the frame to be drawn. */
const SETTLE_MS = 300;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const el = <K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

async function read<T>(path: string): Promise<T> {
  const r = await fetch(path, { headers: { accept: 'application/json' } });
  const body = (await r.json().catch(() => null)) as (T & { error?: string }) | null;
  if (!r.ok || !body) throw new Error(body?.error ?? `the server answered ${r.status}`);
  return body;
}

const scripts = new Map<string, Promise<Full>>(OWN.map((s) => [s.id, Promise.resolve(s)]));
const script = (id: string): Promise<Full> => {
  let got = scripts.get(id);
  if (!got) {
    got = read<Full>(`/api/community/${encodeURIComponent(id)}`);
    got.catch(() => scripts.delete(id));
    scripts.set(id, got);
  }
  return got;
};

const day = (ms: number) => new Date(ms).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
/** `binance-perp:BTCUSDT` as a reader says it: the symbol, then where. */
const marketName = (m: string) => {
  const [venue = '', symbol = m] = m.split(':');
  return { symbol, venue: venue.replace(/-/g, ' ') };
};

// ---------------------------------------------------------------- the engine

interface Slot {
  status(v: ChartStatus): void;
  studies(list: StudyMeta[]): void;
  cursor(value: string): void;
}
interface HeldChart {
  readonly id: string;
  readonly gl: HTMLCanvasElement;
  readonly text: HTMLCanvasElement;
  readonly feed: HTMLCanvasElement;
}

const worker = new Worker(new URL('../engine/worker.ts', import.meta.url), { type: 'module', name: 'tradoor-engine' });
const slots = new Map<string, Slot>();
const pending = new Map<number, { resolve(v: unknown): void; reject(e: Error): void }>();
let nextRequest = 1;
let broken: string | null = null;

const send = (msg: ToWorker, transfer: Transferable[] = []): void => worker.postMessage(msg, transfer);
worker.onmessage = (e: MessageEvent<FromWorker>) => {
  const m = e.data;
  if (m.t === 'reply') {
    const p = pending.get(m.req);
    pending.delete(m.req);
    if (m.error !== undefined) p?.reject(new Error(m.error));
    else p?.resolve(m.value);
  } else if (m.t === 'fatal') {
    broken = m.message;
    for (const p of pending.values()) p.reject(new Error(m.message));
    pending.clear();
    for (const s of slots.values()) s.status({ loading: false, error: m.message } as ChartStatus);
  } else if (m.t === 'status' || m.t === 'studies' || m.t === 'cursor') {
    const s = slots.get(m.id);
    if (m.t === 'status') s?.status(m.value);
    else if (m.t === 'studies') s?.studies(m.value);
    else s?.cursor(m.value);
  }
};
worker.onerror = (e) => {
  broken = e.message || 'the chart engine stopped';
  for (const s of slots.values()) s.status({ loading: false, error: broken } as ChartStatus);
};

function call<K extends keyof Calls>(name: K, args: Calls[K]['args']): Promise<Calls[K]['result']> {
  if (broken) return Promise.reject(new Error(broken));
  const req = nextRequest++;
  return new Promise((resolve, reject) => {
    pending.set(req, { resolve: resolve as (v: unknown) => void, reject });
    send({ t: 'call', req, name, args });
  });
}

let nextSlot = 1;
function holdChart(handlers: Slot): HeldChart {
  const chart = { id: `showcase-${nextSlot++}`, gl: document.createElement('canvas'), text: document.createElement('canvas'), feed: document.createElement('canvas') };
  slots.set(chart.id, handlers);
  return chart;
}
function releaseChart(chart: HeldChart): void {
  slots.delete(chart.id);
  chart.gl.remove();
  chart.text.remove();
  chart.feed.remove();
  send({ t: 'destroy', id: chart.id });
}

send({ t: 'timeZone', offsetMinutes: -new Date().getTimezoneOffset() });

/**
 * A chart of the script on its preview, made in the worker: `ready` once its bars
 * are in and the script has drawn, or fails with why. `flat` draws it in 2D, as a
 * picture needs no GPU of its own.
 */
function chartOf(s: Full, size: { width: number; height: number }, dpr: number, flat: boolean, box?: HTMLElement): { chart: HeldChart; ready: Promise<void>; studyId: string } {
  const p = s.preview ?? DEFAULT_PREVIEW;
  const studyId = [`fx-${s.id}`, ...p.inputs].join(':');
  let loaded = false;
  let drawn = false;
  let settle: (v: void) => void = () => {};
  let fail: (e: Error) => void = () => {};
  const ready = new Promise<void>((resolve, reject) => ((settle = resolve), (fail = reject)));
  const timer = setTimeout(() => (loaded ? settle() : fail(new Error('the chart did not load'))), LOAD_MS);
  const check = () => {
    if (loaded && drawn) {
      clearTimeout(timer);
      setTimeout(settle, SETTLE_MS);
    }
  };
  const chart = holdChart({
    cursor: (value) => {
      if (box) box.style.cursor = value;
    },
    studies: (list: StudyMeta[]) => {
      if (list.some((m) => m.id === studyId)) {
        drawn = true;
        check();
      }
    },
    status: (v: ChartStatus) => {
      if (v.error) {
        clearTimeout(timer);
        fail(new Error(v.error));
      } else if (!v.loading && v.bars > 0) {
        loaded = true;
        check();
      }
    },
  });
  if (box) box.prepend(chart.gl, chart.text, chart.feed);
  const [gl, text, feed] = [chart.gl.transferControlToOffscreen(), chart.text.transferControlToOffscreen(), chart.feed.transferControlToOffscreen()];
  send({ t: 'create', id: chart.id, gl, text, feed, width: size.width, height: size.height, dpr, flat }, [gl, text, feed]);
  send({ t: 'visible', id: chart.id, value: true });
  send({ t: 'hidden', id: chart.id, value: LAYERS_OFF });
  send({ t: 'market', id: chart.id, market: p.market, timeframe: p.timeframe });
  send({ t: 'studies', id: chart.id, value: studyId });
  send({ t: 'formulas', id: chart.id, value: [{ id: studyId, label: [s.name, ...p.inputs].join(' ') + ` · @${s.username}`, source: s.source, inputs: p.inputs }] });
  return { chart, ready, studyId };
}

/** Cards waiting for their picture, drawn one at a time. */
let line: Promise<void> = Promise.resolve();

function picture(card: HTMLElement, s: Listed): void {
  const canvas = card.querySelector('canvas')!;
  const wait = card.querySelector<HTMLElement>('.wait')!;
  line = line.then(async () => {
    try {
      const full = await script(s.id);
      const r = canvas.getBoundingClientRect();
      const size = { width: Math.max(THUMB_MIN.width, Math.round(r.width)), height: Math.max(THUMB_MIN.height, Math.round(r.height)) };
      const dpr = Math.min(DPR_MAX, Math.max(1, devicePixelRatio));
      const { chart, ready } = chartOf(full, size, dpr, true);
      try {
        await ready;
        // At its own size: the picture is the chart pixel for pixel.
        // No tradoor mark: on tradoor's own page, on every card, it would only repeat.
        const shot = await call('capture', { id: chart.id, wide: size.width * dpr, mark: false });
        canvas.width = shot.picture.width;
        canvas.height = shot.picture.height;
        canvas.getContext('2d')?.drawImage(shot.picture, 0, 0);
        shot.picture.close();
        wait.remove();
      } finally {
        releaseChart(chart);
      }
    } catch (e) {
      wait.textContent = `No chart: ${e instanceof Error ? e.message : String(e)}`;
      wait.classList.add('bad');
    }
  });
}

const seen = new IntersectionObserver(
  (entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      seen.unobserve(e.target);
      const s = listed.find((x) => x.id === (e.target as HTMLElement).dataset.id);
      if (s) picture(e.target as HTMLElement, s);
    }
  },
  { rootMargin: '200px' },
);

// ---------------------------------------------------------------- the list

let listed: Listed[] = [];
const grid = $<HTMLDivElement>('grid');
const find = $<HTMLInputElement>('find');
const order = $<HTMLSelectElement>('order');
const cards = new Map<string, HTMLElement>();

function card(s: Listed): HTMLElement {
  const c = el('article', 'card');
  c.dataset.id = s.id;
  c.tabIndex = 0;
  const thumb = el('div', 'thumb');
  const p = s.preview ?? DEFAULT_PREVIEW;
  const { symbol } = marketName(p.market);
  thumb.append(el('canvas'), el('span', 'where', `${symbol} · ${p.timeframe}`), el('span', 'wait', 'Drawing…'));
  const by = el('a', 'by', `@${s.username}`);
  by.href = `?q=${encodeURIComponent(`@${s.username}`)}`;
  by.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    find.value = `@${s.username}`;
    show();
  });
  const foot = el('div', 'meta');
  foot.append(by, el('span', 'when', s.kind ? 'built into tradoor' : day(s.updated)));
  c.append(thumb, el('h3', 'name', s.name), el('p', 'about', s.about + (s.more ? '…' : '')), foot);
  c.addEventListener('click', () => void open(s.id));
  c.addEventListener('keydown', (e) => e.key === 'Enter' && void open(s.id));
  return c;
}

function matches(s: Listed, q: string): boolean {
  if (!q) return true;
  const text = `${s.name} @${s.username} ${s.about}`.toLowerCase();
  return q.split(/\s+/).every((w) => text.includes(w));
}

function show(): void {
  const q = find.value.trim().toLowerCase();
  const list = listed.filter((s) => matches(s, q));
  if (order.value === 'name') list.sort((a, b) => a.name.localeCompare(b.name));
  grid.replaceChildren(
    ...list.map((s) => {
      let c = cards.get(s.id);
      if (!c) {
        c = card(s);
        cards.set(s.id, c);
        seen.observe(c);
      }
      return c;
    }),
  );
  if (list.length === 0) grid.append(el('p', 'empty', listed.length === 0 ? 'No public scripts yet. Write one in tradoor and make it public: it shows up here.' : `Nothing matches “${find.value.trim()}”.`));
  $('count').textContent = listed.length === 0 ? '' : list.length === listed.length ? `${listed.length} ${listed.length === 1 ? 'script' : 'scripts'}` : `${list.length} of ${listed.length}`;
  const url = new URL(location.href);
  if (q) url.searchParams.set('q', find.value.trim());
  else url.searchParams.delete('q');
  history.replaceState(history.state, '', url.pathname + url.search + url.hash);
}

find.addEventListener('input', show);
order.addEventListener('change', show);

// ---------------------------------------------------------------- one opened

const dialog = $<HTMLDialogElement>('open');
const box = $<HTMLDivElement>('o-box');
let live: { chart: HeldChart; stop: () => void } | null = null;

function closeLive(): void {
  if (!live) return;
  live.stop();
  releaseChart(live.chart);
  live = null;
}

/** The live chart, moved about with a mouse or a finger as on tradoor. */
function steer(chart: HeldChart): () => void {
  const at = (e: PointerEvent | WheelEvent) => {
    const r = box.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const pointer = (kind: 'move' | 'down' | 'up' | 'leave') => (e: PointerEvent) => {
    if (kind === 'down') box.setPointerCapture(e.pointerId);
    const p = at(e);
    send({ t: 'pointer', id: chart.id, kind, x: p.x, y: p.y, button: e.button, touch: e.pointerType === 'touch' });
  };
  const wheel = (e: WheelEvent) => {
    e.preventDefault();
    const p = at(e);
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? box.clientHeight : 1;
    send({ t: 'wheel', id: chart.id, x: p.x, y: p.y, dx: e.deltaX * unit, dy: e.deltaY * unit });
  };
  const handlers = { pointerdown: pointer('down'), pointermove: pointer('move'), pointerup: pointer('up'), pointerleave: pointer('leave') };
  for (const [name, fn] of Object.entries(handlers)) box.addEventListener(name, fn as EventListener);
  box.addEventListener('wheel', wheel, { passive: false });
  const sized = new ResizeObserver(() => {
    const r = box.getBoundingClientRect();
    if (r.width > 0) send({ t: 'resize', id: chart.id, width: r.width, height: r.height, dpr: devicePixelRatio });
  });
  sized.observe(box);
  return () => {
    for (const [name, fn] of Object.entries(handlers)) box.removeEventListener(name, fn as EventListener);
    box.removeEventListener('wheel', wheel);
    sized.disconnect();
    box.replaceChildren();
  };
}

function tab(which: 'chart' | 'source'): void {
  for (const b of dialog.querySelectorAll<HTMLButtonElement>('.tabs button')) b.classList.toggle('on', b.dataset.tab === which);
  $('o-chart').hidden = which !== 'chart';
  $('o-source').hidden = which !== 'source';
}
for (const b of dialog.querySelectorAll<HTMLButtonElement>('.tabs button')) b.addEventListener('click', () => tab(b.dataset.tab as 'chart' | 'source'));

async function open(id: string): Promise<void> {
  closeLive();
  tab('chart');
  const wait = $('o-wait');
  wait.textContent = 'Drawing…';
  wait.hidden = false;
  if (!dialog.open) dialog.showModal();
  if (location.hash !== `#${id}`) history.replaceState(history.state, '', `${location.pathname}${location.search}#${id}`);
  let s: Full;
  try {
    s = await script(id);
  } catch (e) {
    $('o-name').textContent = 'Not found';
    $('o-about').textContent = `${e instanceof Error ? e.message : String(e)}.`;
    wait.textContent = '';
    return;
  }
  const p = s.preview ?? DEFAULT_PREVIEW;
  const { symbol, venue } = marketName(p.market);
  $('o-name').textContent = s.name;
  const by = $<HTMLAnchorElement>('o-by');
  by.textContent = `@${s.username}`;
  by.href = `?q=${encodeURIComponent(`@${s.username}`)}`;
  $('o-when').textContent = s.kind ? 'built into tradoor' : s.created === s.updated ? `shared ${day(s.created)}` : `changed ${day(s.updated)}`;
  $('o-market').textContent = `${symbol} · ${venue} · ${p.timeframe}${p.inputs.length ? ` · settings ${p.inputs.join(', ')}` : ''}`;
  $('o-about').textContent = s.about;
  $('o-source').textContent = s.source;
  $<HTMLAnchorElement>('o-use').href = s.kind ? `/?indicator=${encodeURIComponent(s.kind)}` : `/?script=${encodeURIComponent(s.id)}`;
  const r = box.getBoundingClientRect();
  const { chart, ready } = chartOf(s, { width: r.width || 800, height: r.height || 400 }, devicePixelRatio, false, box);
  live = { chart, stop: steer(chart) };
  ready.then(
    () => (wait.hidden = true),
    (e: unknown) => (wait.textContent = `No chart: ${e instanceof Error ? e.message : String(e)}`),
  );
}

function close(): void {
  closeLive();
  if (dialog.open) dialog.close();
  if (location.hash) history.replaceState(history.state, '', location.pathname + location.search);
}
$('o-close').addEventListener('click', close);
dialog.addEventListener('close', () => {
  closeLive();
  if (location.hash) history.replaceState(history.state, '', location.pathname + location.search);
});
dialog.addEventListener('click', (e) => e.target === dialog && close());
$<HTMLAnchorElement>('o-by').addEventListener('click', (e) => {
  e.preventDefault();
  find.value = $('o-by').textContent ?? '';
  close();
  show();
});
$('o-copy').addEventListener('click', async () => {
  const button = $('o-copy');
  try {
    await navigator.clipboard.writeText(`${location.origin}${location.pathname}#${location.hash.slice(1)}`);
    button.textContent = 'Copied';
  } catch {
    button.textContent = 'Copy it from the address bar';
  }
  setTimeout(() => (button.textContent = 'Copy its link'), 2000);
});

// ---------------------------------------------------------------- start

find.value = new URL(location.href).searchParams.get('q') ?? '';
// The community's first, the last changed first, then tradoor's own; without the server, tradoor's own alone.
const start = (shared: Listed[]) => {
  listed = [...shared, ...OWN];
  show();
  const id = location.hash.slice(1);
  if (/^[a-z0-9]{6,16}$/.test(id)) void open(id);
};
read<{ scripts: Listed[] }>('/api/community').then(
  (r) => start(r.scripts),
  (e: unknown) => {
    start([]);
    grid.prepend(el('p', 'empty bad', `The community's scripts did not load: ${e instanceof Error ? e.message : String(e)}.`));
  },
);
