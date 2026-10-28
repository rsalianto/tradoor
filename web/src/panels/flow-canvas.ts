/**
 * What the DOM and tape panels share: a canvas that fills its box at the
 * screen's density and is drawn on animation frames, only when something
 * changed; the page's colours, read off its CSS; and sizes written short.
 * Both redraw many times a second, which a table of elements would not take.
 */
import { onCleanup } from 'solid-js';
import type { SizeUnit } from '../workspace/model';

export interface Canvas {
  /** Draw on the next frame, whatever `due` says. */
  redraw(): void;
}

/**
 * Draws `canvas` to fill `host` whenever `due()` says something changed, at
 * most every `every` milliseconds, and on every resize. Stops with the owner.
 */
export function useCanvas(host: HTMLElement, canvas: HTMLCanvasElement, draw: (ctx: CanvasRenderingContext2D, width: number, height: number) => void, due: () => boolean, every = 50): Canvas {
  let width = 0;
  let height = 0;
  let dpr = 1;
  let forced = true;
  let last = 0;
  const ctx = canvas.getContext('2d', { alpha: false });
  const observer = new ResizeObserver(() => {
    width = host.clientWidth;
    height = host.clientHeight;
    dpr = devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.round(width * dpr));
    canvas.height = Math.max(1, Math.round(height * dpr));
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    forced = true;
  });
  observer.observe(host);
  let frame = requestAnimationFrame(function loop(t) {
    frame = requestAnimationFrame(loop);
    if (!ctx || width === 0 || height === 0) return;
    if (!forced && (t - last < every || !due())) return;
    last = t;
    forced = false;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw(ctx, width, height);
  });
  onCleanup(() => {
    cancelAnimationFrame(frame);
    observer.disconnect();
  });
  return { redraw: () => (forced = true) };
}

export interface Palette {
  bg: string;
  surface: string;
  border: string;
  text: string;
  primary: string;
  bull: string;
  bear: string;
  warn: string;
  font: string;
}

/** The theme's colours; the mixed shades are left to `globalAlpha`, which a canvas takes as it is. */
export function palette(el: Element): Palette {
  const style = getComputedStyle(el);
  const v = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  return {
    bg: v('--bg', '#070907'),
    surface: v('--surface', '#0d100e'),
    border: v('--border', '#232823'),
    text: v('--text', '#dfe6e0'),
    primary: v('--primary', '#aafd35'),
    bull: v('--bull', '#aafd35'),
    bear: v('--bear', '#ff4d5f'),
    warn: v('--warn', '#ffc247'),
    font: v('--font', 'monospace'),
  };
}

/** A size in a few characters: `0.0042`, `12.5`, `3.4K`, `1.25M`. */
export function compact(v: number): string {
  const abs = Math.abs(v);
  if (abs >= 1e9) return `${(v / 1e9).toFixed(2)}B`;
  if (abs >= 1e7) return `${(v / 1e6).toFixed(1)}M`;
  if (abs >= 1e6) return `${(v / 1e6).toFixed(2)}M`;
  if (abs >= 1e4) return `${(v / 1e3).toFixed(0)}K`;
  if (abs >= 1e3) return `${(v / 1e3).toFixed(1)}K`;
  if (abs >= 100) return v.toFixed(0);
  if (abs >= 10) return v.toFixed(1);
  if (abs >= 1) return v.toFixed(2);
  if (abs === 0) return '0';
  return String(Number(v.toPrecision(2)));
}

/** Coins, or what they cost at `price`. */
export const sized = (qty: number, price: number, unit: SizeUnit): number => (unit === 'usd' ? qty * price : qty);

/** Decimals a price needs at this step: `0.5` one, `0.01` two, `5` none. */
export function stepDecimals(step: number): number {
  const s = String(Number(step.toPrecision(8)));
  const dot = s.indexOf('.');
  if (s.includes('e-')) return Number(s.split('e-')[1]);
  return dot === -1 ? 0 : s.length - dot - 1;
}

/** The filter's choices, in dollars. */
export const MIN_USD = [0, 1_000, 10_000, 50_000, 100_000, 500_000, 1_000_000] as const;

export const minLabel = (v: number): string => (v === 0 ? 'All sizes' : `≥ $${compact(v)}`);
