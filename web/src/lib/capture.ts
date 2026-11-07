/**
 * A picture of a chart, copied to the clipboard in one press.
 *
 * The engine draws the chart again into canvases of its own, so the picture is
 * as large as asked for however small the panel is — 4K across by default. The
 * clipboard item is made while the click is still in hand, because Safari only
 * allows a write that starts in the gesture, and it takes the promise of the
 * picture rather than the picture itself. Where a browser will not write an
 * image at all, the file is offered instead.
 */
import { call } from '../engine/client';
import { errorText } from './api';
import { toast } from './toasts';

/** Across, in pixels: 4K, the width of a screenshot worth keeping. */
export const CAPTURE_WIDE = 3840;

function keep(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

const canWrite = (): boolean => typeof ClipboardItem === 'function' && typeof navigator.clipboard?.write === 'function';

/** What a picture says it is of, top left: see capture-title. */
export interface CaptureTitle {
  symbol: string;
  /** Its timeframe and exchange, as the legend has them. */
  detail: string;
  /** The bar's prices and its change, as the legend shows them; empty for none. */
  ohlc: string;
  change: string;
  up: boolean;
  /** When the picture was taken, or the replay's time. */
  time: string;
}

/**
 * Copy the chart in `slot`; `save` writes the file instead, as does a browser
 * that refuses the clipboard. `name` is what the file is called.
 */
export function captureChart(slot: string, name: string, title: CaptureTitle, save = false): void {
  const picture = Promise.all([call('capture', { id: slot, wide: CAPTURE_WIDE }), import('./capture-title')]).then(([shot, { titled }]) => titled(shot, title));
  const failed = (e: unknown) => toast({ tone: 'warn', title: 'Picture not taken', body: errorText(e) });
  if (save || !canWrite()) {
    picture.then((blob) => keep(blob, name), failed);
    return;
  }
  navigator.clipboard.write([new ClipboardItem({ 'image/png': picture })]).then(
    () => toast({ tone: 'info', title: 'Chart copied', body: 'Paste it anywhere; hold shift to save the file instead.' }, 4_000),
    // A browser that would not take it still gives the file.
    () => picture.then((blob) => keep(blob, name), failed),
  );
}
