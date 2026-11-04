/**
 * The page's side of the engine worker. One worker serves every chart, so they
 * share WASM memory, loaded history and exchange sockets.
 */
import { createStore } from 'solid-js/store';
import { nextPrice, type LastPrice } from '../lib/last-price';
import type { CallName, Calls, ChartEvent, ChartStatus, FromWorker, Legend, ReplayState, StudyDefinition, StudyMeta, ToWorker } from './protocol';

export interface SlotHandlers {
  cursor(value: string): void;
  legend(value: Legend): void;
  studies(value: StudyMeta[]): void;
  event(event: ChartEvent): void;
  status(value: ChartStatus): void;
  replay(value: ReplayState): void;
}


let worker: Worker | null = null;
/** When the worker last said anything, for `?debug`'s line. */
export let heardAt = 0;
/** `?debug`: this module's own name, how many times it has run in the page, and the workers it made — more than one each is a second engine. */
const CLIENT = Math.random().toString(36).slice(2, 6);
const runs = globalThis as { __tradoorClients?: number };
runs.__tradoorClients = (runs.__tradoorClients ?? 0) + 1;
let workersMade = 0;
export const debugClient = (): string => `client ${CLIENT} ran ${runs.__tradoorClients}× made ${workersMade}`;
let failure: string | null = null;
const failureListeners = new Set<(message: string) => void>();
const slots = new Map<string, SlotHandlers>();
/** Engine slot → the chart panel it belongs to. */
const panelOf = new Map<string, string>();
const [prices, setPrices] = createStore<Record<string, LastPrice>>({});

/** A chart panel's last price, kept while the panel is off screen. */
export const lastPrice = (panelId: string): LastPrice | undefined => prices[panelId];
const pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>();
let nextRequest = 1;
let nextSlot = 1;

function fail(message: string): void {
  failure = message;
  for (const p of pending.values()) p.reject(new Error(message));
  pending.clear();
  for (const listener of failureListeners) listener(message);
}

function dispatch(msg: FromWorker): void {
  switch (msg.t) {
    case 'ready':
      return;
    case 'fatal':
      fail(msg.message);
      return;
    case 'reply': {
      const p = pending.get(msg.req);
      if (!p) return;
      pending.delete(msg.req);
      if (msg.error !== undefined) p.reject(new Error(msg.error));
      else p.resolve(msg.value);
      return;
    }
    case 'price': {
      const panel = panelOf.get(msg.id);
      if (panel) setPrices(panel, nextPrice(prices[panel], msg.market, msg.price, msg.precision));
      return;
    }
    default: {
      const h = slots.get(msg.id);
      if (!h) return;
      if (msg.t === 'cursor') h.cursor(msg.value);
      else if (msg.t === 'legend') h.legend(msg.value);
      else if (msg.t === 'studies') h.studies(msg.value);
      else if (msg.t === 'event') h.event(msg.event);
      else if (msg.t === 'replay') h.replay(msg.value);
      else h.status(msg.value);
    }
  }
}

function engine(): Worker {
  if (worker) return worker;
  worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module', name: 'tradoor-engine' });
  workersMade += 1;
  worker.onmessage = (e: MessageEvent<FromWorker>) => {
    heardAt = Date.now();
    dispatch(e.data);
  };
  worker.onerror = (e) => fail(e.message || 'the chart engine stopped');
  return worker;
}

/** Starts the chart engine — its worker and WebAssembly — before any chart
 *  asks for it, alongside the page's own first requests. */
export const warmEngine = (): void => void engine();

export function send(msg: ToWorker, transfer: Transferable[] = []): void {
  engine().postMessage(msg, transfer);
}

// Development only: `__tradoorSend({ t: 'loseGpu', id: 'chart-0', ms: 1500 })` in the console.
if (import.meta.env.DEV) (globalThis as { __tradoorSend?: typeof send }).__tradoorSend = send;

export function call<K extends CallName>(name: K, args: Calls[K]['args']): Promise<Calls[K]['result']> {
  if (failure) return Promise.reject(new Error(failure));
  const req = nextRequest++;
  return new Promise((resolve, reject) => {
    pending.set(req, { resolve: resolve as (value: unknown) => void, reject });
    send({ t: 'call', req, name, args });
  });
}

export interface HeldChart {
  /** The engine slot's id. */
  readonly id: string;
  readonly gl: HTMLCanvasElement;
  readonly text: HTMLCanvasElement;
  /** The liquidation feed, over the other two: a hit moving redraws it alone. */
  readonly feed: HTMLCanvasElement;
  /** Whether the canvases have been handed to the engine yet. */
  created: boolean;
}

const charts = new Map<string, HeldChart>();

/**
 * A chart panel's canvases and engine slot, kept for as long as the panel is in
 * the workspace rather than for as long as it is mounted. A browser gives a
 * page about sixteen WebGL contexts and frees a dropped one only when it gets
 * round to it, so a context per mount runs out after a few tab switches and the
 * oldest chart goes blank. One per panel never does — and a tab shown again
 * draws at once, with its bars still loaded. Pair with `releaseChart`.
 */
export function holdChart(panelId: string, handlers: SlotHandlers): HeldChart {
  let chart = charts.get(panelId);
  if (!chart) {
    chart = { id: `chart-${nextSlot++}`, gl: document.createElement('canvas'), text: document.createElement('canvas'), feed: document.createElement('canvas'), created: false };
    charts.set(panelId, chart);
    panelOf.set(chart.id, panelId);
  }
  slots.set(chart.id, handlers);
  return chart;
}

/** The panel's chart is off screen: kept, paused, while the panel still exists; freed once it does not. */
export function releaseChart(panelId: string, stillOpen: boolean): void {
  const chart = charts.get(panelId);
  if (!chart) return;
  slots.delete(chart.id);
  chart.gl.remove();
  chart.text.remove();
  chart.feed.remove();
  if (stillOpen) {
    if (chart.created) send({ t: 'visible', id: chart.id, value: false });
    return;
  }
  charts.delete(panelId);
  panelOf.delete(chart.id);
  setPrices(panelId, undefined!);
  if (chart.created) send({ t: 'destroy', id: chart.id });
}

export function onEngineFailure(listener: (message: string) => void): () => void {
  failureListeners.add(listener);
  if (failure) listener(failure);
  return () => void failureListeners.delete(listener);
}

let catalog: Promise<StudyDefinition[]> | null = null;

export function studyCatalog(): Promise<StudyDefinition[]> {
  catalog ??= call('studyCatalog', {});
  return catalog;
}
