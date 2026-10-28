/**
 * Time and sales: every trade of one market as it prints, newest on top,
 * green where a buyer took the offer and red where a seller hit the bid.
 * Trades sharing a time and side are one order — a market order sweeping
 * five levels prints five times — so they read as one row, and the size
 * filter judges the order. The bigger the order, the deeper its shade.
 * Hovering holds the tape still to read; trades keep coming underneath.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { exchangeOfVenue } from '../lib/exchanges';
import { formatPrice } from '../lib/format';
import { holdFlow, recentGroups, releaseFlow, type Flow, type FlowState, type Group } from '../lib/orderflow';
import { formatClock } from '../lib/timezone';
import type { TapePanelState } from '../workspace/model';
import { doc, marketOf, patchPanel } from '../workspace/store';
import { MarketButton } from './common';
import { compact, MIN_USD, minLabel, palette, stepDecimals, useCanvas, type Palette } from './flow-canvas';

const ROW = 16;
const HEAD = 18;

const STATE_LABEL: Partial<Record<FlowState, string>> = { connecting: 'Connecting', stalled: 'Stalled', retrying: 'Reconnecting' };

/** How deep a row is shaded: past ten, fifty and 250 times the filter (a thousand dollars at least). */
export function tier(usd: number, minUsd: number): number {
  const base = Math.max(minUsd, 1_000);
  return usd >= base * 250 ? 3 : usd >= base * 50 ? 2 : usd >= base * 10 ? 1 : 0;
}

export function TapePanel(props: { panel: TapePanelState }) {
  const id = props.panel.id;
  const market = () => marketOf(props.panel);
  const [flow, setFlow] = createSignal<Flow | null>(null);
  const [state, setState] = createSignal<FlowState>('connecting');
  const [paused, setPaused] = createSignal(false);

  createEffect(() => {
    const held = holdFlow(market());
    setFlow(held);
    onCleanup(() => releaseFlow(held));
  });

  let host!: HTMLDivElement;
  let canvas!: HTMLCanvasElement;
  let colors: Palette | null = null;
  let seen = -1;
  let frozen: Group[] | null = null;

  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number) => {
    const f = flow();
    colors ??= palette(host);
    const c = colors;
    ctx.fillStyle = c.surface;
    ctx.fillRect(0, 0, w, h);
    ctx.font = `11px ${c.font}`;
    ctx.textBaseline = 'middle';
    if (!f) return;
    setState(f.state.prints);
    const rows = Math.max(0, Math.floor((h - HEAD) / ROW));
    const unit = props.panel.unit;
    const min = props.panel.minUsd;
    const groups = paused() && frozen ? frozen : recentGroups(f.prints, min, rows);
    if (!paused()) frozen = groups;
    const tick = f.tick();
    const decimals = tick > 0 ? stepDecimals(tick) : undefined;

    // Columns: time, price, size, and how many trades the order took. A
    // narrow tape drops the count, then the time, rather than overlap them.
    const xTime = 8;
    const counted = w >= 230;
    const xSize = w - (counted ? 30 : 6);
    const xPrice = Math.min(Math.max(w * 0.62, 150), xSize - 46);
    ctx.font = `11px ${c.font}`;
    const priceWidth = ctx.measureText(groups[0] ? formatPrice(groups[0].to, decimals) : '00,000.0').width;
    const timed = xTime + ctx.measureText('00:00:00').width + 8 <= xPrice - priceWidth;
    const xCount = w - 6;
    ctx.globalAlpha = 0.5;
    ctx.fillStyle = c.text;
    ctx.font = `9px ${c.font}`;
    ctx.textAlign = 'left';
    if (timed) ctx.fillText('TIME', xTime, HEAD / 2);
    ctx.textAlign = 'right';
    ctx.fillText('PRICE', xPrice, HEAD / 2);
    ctx.fillText(unit === 'usd' ? 'SIZE $' : 'SIZE', xSize, HEAD / 2);
    ctx.globalAlpha = 1;
    ctx.fillStyle = c.border;
    ctx.fillRect(0, HEAD - 1, w, 1);

    if (f.state.prints === 'none') return message(ctx, c, w, h, 'An index has no trades of its own.');
    if (groups.length === 0) {
      const venue = f.venue ? exchangeOfVenue(f.venue).name : 'the exchange';
      return message(ctx, c, w, h, f.state.prints === 'live' ? (min > 0 ? `No order of ${minLabel(min).slice(2)} or more yet.` : 'Waiting for the next trade.') : `Connecting to ${venue}…`);
    }

    groups.forEach((g, i) => {
      const y = HEAD + i * ROW;
      const side = g.buy ? c.bull : c.bear;
      const t = tier(g.usd, min);
      if (t > 0) {
        ctx.globalAlpha = [0, 0.1, 0.2, 0.36][t]!;
        ctx.fillStyle = side;
        ctx.fillRect(0, y, w, ROW - 1);
        ctx.globalAlpha = 1;
      }
      ctx.font = `${t >= 2 ? '700 ' : ''}11px ${c.font}`;
      ctx.fillStyle = c.text;
      ctx.globalAlpha = 0.55;
      ctx.textAlign = 'left';
      if (timed) ctx.fillText(formatClock(g.time), xTime, y + ROW / 2);
      ctx.globalAlpha = 1;
      ctx.fillStyle = side;
      ctx.textAlign = 'right';
      ctx.fillText(formatPrice(g.to, decimals), xPrice, y + ROW / 2);
      ctx.fillText(compact(unit === 'usd' ? g.usd : g.qty), xSize, y + ROW / 2);
      if (counted && g.trades > 1) {
        ctx.font = `9px ${c.font}`;
        ctx.fillStyle = c.text;
        ctx.globalAlpha = 0.4;
        ctx.fillText(String(g.trades), xCount, y + ROW / 2);
        ctx.globalAlpha = 1;
      }
    });
  };

  onMount(() => {
    const view = useCanvas(host, canvas, draw, () => {
      const f = flow();
      if (!f || paused() || f.version === seen) return false;
      seen = f.version;
      return true;
    });
    // A new filter, unit, market or theme: drawn at once, the colours read again.
    createEffect(() => {
      void [props.panel.minUsd, props.panel.unit, flow(), paused(), Object.values(doc.theme)];
      colors = null;
      view.redraw();
    });
  });

  return (
    <div class="panelcol flowpanel">
      <div class="toolbar">
        <MarketButton panelId={id} market={market()} />
        <span class="spacer" />
        <Show when={STATE_LABEL[state()]}>{(label) => <span class={`conn ${state()}`}>{label()}</span>}</Show>
        <Show when={paused()}>
          <span class="conn" data-tip="The pointer is over the tape: it holds still to be read">Paused</span>
        </Show>
        <select data-tip="Smallest order to show: trades at one time and side count as one" onChange={(e) => patchPanel(id, { minUsd: Number(e.currentTarget.value) })}>
          <For each={MIN_USD}>
            {(v) => (
              <option value={v} selected={props.panel.minUsd === v}>
                {minLabel(v)}
              </option>
            )}
          </For>
        </select>
        <button class="tb" data-tip="Sizes in dollars or in coins" onClick={() => patchPanel(id, { unit: props.panel.unit === 'usd' ? 'coins' : 'usd' })}>
          {props.panel.unit === 'usd' ? 'USD' : 'Coins'}
        </button>
      </div>
      <div class="flowcanvas" ref={host} onPointerEnter={() => setPaused(true)} onPointerLeave={() => setPaused(false)}>
        <canvas ref={canvas} />
      </div>
    </div>
  );
}

/** A line in the middle, in place of rows. */
export function message(ctx: CanvasRenderingContext2D, c: Palette, w: number, h: number, text: string): void {
  ctx.font = `11px ${c.font}`;
  ctx.fillStyle = c.text;
  ctx.globalAlpha = 0.4;
  ctx.textAlign = 'center';
  ctx.fillText(text, w / 2, Math.min(h / 2, 80));
  ctx.globalAlpha = 1;
}
