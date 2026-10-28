/**
 * The DOM: the order book by price, and what traded at each level. From the
 * left: the volume traded since the last clear, the bids, the market sells
 * that hit them, the price, the market buys that lifted the asks, the asks,
 * and what was pulled from a level (orange) or stacked onto it (green) beyond
 * what trades took. A row is a tick or a group of them. The last price stays
 * in the middle until the rows are scrolled; Center, or a double-click, brings
 * it back. Clear empties the volume profile, Bought and Sold, or Pull/Stack,
 * and each of the first two can start again on the clock by itself; a
 * right-click clears the levels above or below a row. The size filter judges
 * orders — the trades of one time and side — not single fills.
 * In Training it also trades, on paper (see DomTrade); elsewhere it only reads.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { exchangeOfVenue } from '../lib/exchanges';
import { useDismiss } from '../lib/dismiss';
import { formatPrice } from '../lib/format';
import { Icon } from '../lib/icons';
import { bestAsk, bestBid, holdFlow, releaseFlow, Tally, type Flow, type FlowState, type Where } from '../lib/orderflow';
import type { Order } from '../training/model';
import { symbolOf } from '../lib/markets';
import type { DomPanelState } from '../workspace/model';
import { doc, marketOf, patchPanel } from '../workspace/store';
import { MarketButton } from './common';
import { compact, MIN_USD, minLabel, palette, sized, stepDecimals, useCanvas, type Palette } from './flow-canvas';
import type { DomTrade } from './DomTrade';
import { message } from './TapePanel';

const ROW = 16;
const HEAD = 18;

type Col = 'profile' | 'bids' | 'sold' | 'price' | 'bought' | 'asks' | 'pulls';

const COLS: readonly { key: Col; title: string; weight: number }[] = [
  { key: 'profile', title: 'Volume', weight: 1.1 },
  { key: 'bids', title: 'Bids', weight: 1 },
  { key: 'sold', title: 'Sold', weight: 0.9 },
  { key: 'price', title: 'Price', weight: 1.3 },
  { key: 'bought', title: 'Bought', weight: 0.9 },
  { key: 'asks', title: 'Asks', weight: 1 },
  { key: 'pulls', title: 'Pull/Stack', weight: 1 },
];

/** Columns that can be left out, and what they are. */
const OPTIONAL: readonly { key: Col; label: string; tip: string }[] = [
  { key: 'profile', label: 'Volume', tip: 'Volume traded at each level since the profile was last cleared, split by side' },
  { key: 'pulls', label: 'Pull/Stack', tip: 'Size pulled from a level (orange) or stacked onto it (green) beyond what trades took, fading by half every 30 seconds' },
];

/** When Bought and Sold, and the volume profile, start again by themselves: minutes, on the clock. */
const RESET_TRADES: readonly { value: number; label: string }[] = [
  { value: 0, label: 'Off' },
  { value: 1, label: '1m' },
  { value: 5, label: '5m' },
  { value: 15, label: '15m' },
  { value: 60, label: '1h' },
];
const RESET_PROFILE: readonly { value: number; label: string }[] = [
  { value: 0, label: 'Off' },
  { value: 60, label: '1h' },
  { value: 240, label: '4h' },
  { value: 1440, label: 'Day' },
];

/** What Clear can empty. */
type Part = 'trades' | 'profile' | 'pulls';
const PARTS: readonly { part: Part | 'all'; label: string }[] = [
  { part: 'all', label: 'Everything' },
  { part: 'profile', label: 'Volume profile' },
  { part: 'trades', label: 'Bought & sold' },
  { part: 'pulls', label: 'Pull/Stack' },
];

/** Ticks a row can span. */
export const ROW_TICKS = [1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10_000] as const;

/** Rows about half a basis point tall: $5 at BTC's $80,000, five cents at $100. */
export function autoTicks(tick: number, price: number): number {
  const want = price * 0.00005;
  return ROW_TICKS.find((m) => m * tick >= want) ?? ROW_TICKS[ROW_TICKS.length - 1]!;
}

const STATE_LABEL: Partial<Record<FlowState, string>> = { connecting: 'Connecting', stalled: 'Stalled', retrying: 'Reconnecting' };

interface Stats {
  volume: string;
  delta: number;
  deltaText: string;
  range: string;
  spread: string;
}

/**
 * `trade`: Training's account, to trade on from the rows; `patch`: where the
 * panel's settings go when it is not in the workspace (Training keeps its own).
 */
export function DomPanel(props: { panel: DomPanelState; trade?: DomTrade; patch?: (patch: Partial<DomPanelState>) => void }) {
  const id = props.panel.id;
  const market = () => marketOf(props.panel);
  const patch = (p: Partial<DomPanelState>) => (props.patch ? props.patch(p) : patchPanel(id, p));
  /** Trading: only where Training handed its account in. */
  const trade = {
    live: () => props.trade?.live() === true,
    orders: () => props.trade?.orders() ?? [],
    position: () => props.trade?.position(),
  };
  const [flow, setFlow] = createSignal<Flow | null>(null);
  const [state, setState] = createSignal<FlowState>('connecting');
  const [auto, setAuto] = createSignal(true);
  const [stats, setStats] = createSignal<Stats | null>(null);
  const [menu, setMenu] = createSignal<{ x: number; y: number; price: number; decimals: number } | null>(null);
  const [columnsOpen, setColumnsOpen] = createSignal(false);
  const [clearOpen, setClearOpen] = createSignal(false);
  const [tickNow, setTickNow] = createSignal(0);
  useDismiss(menu, () => setMenu(null));
  useDismiss(columnsOpen, () => setColumnsOpen(false));
  useDismiss(clearOpen, () => setClearOpen(false));

  createEffect(() => {
    const held = holdFlow(market());
    setFlow(held);
    setAuto(true);
    onCleanup(() => releaseFlow(held));
  });

  /** The interval each count was last read with, so turning one off brings back all that is held. */
  const every = new WeakMap<Tally, number>();
  /**
   * What traded, counted twice: once for Bought and Sold, once for the volume
   * profile and the totals under it, each cleared and started again on its own.
   * Kept with the market's flow, so a tab away and back loses nothing, and they
   * go when the flow does.
   */
  const tallies = (): { trades: Tally; profile: Tally } | null => {
    const f = flow();
    if (!f) return null;
    const get = (key: string, minutes: number) => {
      let t = f.tallies.get(key);
      if (!t) {
        t = new Tally(props.panel.minUsd, 0);
        f.tallies.set(key, t);
      }
      if (t.minUsd !== props.panel.minUsd) {
        t.minUsd = props.panel.minUsd;
        t.restart(f.prints);
      }
      if ((every.get(t) ?? 0) !== minutes) {
        every.set(t, minutes);
        if (minutes === 0) t.restart(f.prints, 0);
      }
      // On the clock: past an interval's turn, it counts from the turn.
      if (minutes > 0) {
        const turn = Math.floor(Date.now() / (minutes * 60_000)) * minutes * 60_000;
        if (t.since < turn) t.restart(f.prints, turn);
      }
      return t;
    };
    return { trades: get(id, props.panel.resetTrades), profile: get(`${id}:profile`, props.panel.resetProfile) };
  };

  const hidden = () => new Set(props.panel.hidden.split(',').filter(Boolean));
  const toggleColumn = (key: Col) => {
    const set = hidden();
    if (set.has(key)) set.delete(key);
    else set.add(key);
    patch({ hidden: [...set].join(',') });
  };
  const shown = () => COLS.filter((c) => !hidden().has(c.key));

  let host!: HTMLDivElement;
  let canvas!: HTMLCanvasElement;
  let colors: Palette | null = null;
  let seen = -1;
  /** The row bucket at the top; null until the price is first known. */
  let top: number | null = null;
  /** The market the rows were last drawn for: a new one is centered afresh. */
  let drawn = '';
  let hover = -1;
  /** An order of yours being dragged to another row, and the row it left. */
  let drag: { order: Order; from: number } | null = null;
  /** As last drawn, for the pointer: the row step and the columns' edges. */
  let layout = { step: 0, decimals: 2, rows: 0, cols: [] as { key: Col; x: number; w: number }[] };

  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number) => {
    const f = flow();
    const both = tallies();
    colors ??= palette(host);
    const c = colors;
    ctx.fillStyle = c.surface;
    ctx.fillRect(0, 0, w, h);
    ctx.textBaseline = 'middle';

    // The columns, by weight across the width.
    // Too narrow for every column: Pull/Stack goes first, then Volume.
    const list = shown().filter((col) => !(col.key === 'pulls' && w < 300) && !(col.key === 'profile' && w < 250));
    const total = list.reduce((s, col) => s + col.weight, 0);
    let x = 0;
    const cols = list.map((col) => {
      const cw = (w * col.weight) / total;
      const out = { key: col.key, x, w: cw };
      x += cw;
      return out;
    });
    ctx.font = `9px ${c.font}`;
    ctx.fillStyle = c.text;
    ctx.globalAlpha = 0.5;
    ctx.textAlign = 'center';
    const unit = props.panel.unit;
    for (const col of cols) {
      const title = COLS.find((k) => k.key === col.key)!.title.toUpperCase();
      ctx.fillText(col.key === 'price' || col.key === 'pulls' || unit === 'coins' ? title : `${title} $`, col.x + col.w / 2, HEAD / 2);
    }
    ctx.globalAlpha = 1;
    ctx.fillStyle = c.border;
    ctx.fillRect(0, HEAD - 1, w, 1);

    if (!f || !both) return;
    const { trades: t, profile } = both;
    setState(f.state.book);
    if (f.state.book === 'none') return message(ctx, c, w, h, 'An index has no order book.');
    const tick = f.tick();
    if (!f.book.synced || tick === 0) {
      const venue = f.venue ? exchangeOfVenue(f.venue).name : 'the exchange';
      return message(ctx, c, w, h, f.state.book === 'live' ? 'Loading the book…' : `Connecting to ${venue}…`);
    }
    setTickNow(tick);
    t.read(f.prints);
    profile.read(f.prints);

    const book = f.book;
    const bb = bestBid(book.bids);
    const ba = bestAsk(book.asks);
    const last = f.prints.last;
    const ref = Number.isFinite(last) ? last : (bb + ba) / 2;
    const step = tick * (props.panel.ticks || autoTicks(tick, ref));
    const decimals = stepDecimals(step);
    const rows = Math.max(1, Math.floor((h - HEAD) / ROW));
    const down = (p: number) => Math.floor(p / step + 1e-9);
    const up = (p: number) => Math.ceil(p / step - 1e-9);
    const refRow = down(ref);
    if (f.market !== drawn) {
      drawn = f.market;
      top = null;
    } else if (top !== null && layout.step > 0 && Math.abs(step - layout.step) > step * 1e-9) {
      // Another row size: the same price stays in the middle — a row number means another price now.
      top = Math.round(((top - Math.floor(layout.rows / 2)) * layout.step) / step) + Math.floor(rows / 2);
    }
    if (top === null || (auto() && (top - refRow < rows * 0.25 || top - refRow > rows * 0.75))) top = refRow + Math.floor(rows / 2);
    layout = { step, decimals, rows, cols };

    // Everything bucketed into the rows on screen: bids and sells round down, asks and buys up.
    const bids = new Float64Array(rows);
    const asks = new Float64Array(rows);
    const sold = new Float64Array(rows);
    const bought = new Float64Array(rows);
    const pulls = new Float64Array(rows);
    const profSold = new Float64Array(rows);
    const profBought = new Float64Array(rows);
    const into = (arr: Float64Array, bucket: number, v: number) => {
      const r = top! - bucket;
      if (r >= 0 && r < rows) arr[r] = arr[r]! + v;
    };
    for (const [p, q] of book.bids) into(bids, down(p), sized(q, p, unit));
    for (const [p, q] of book.asks) into(asks, up(p), sized(q, p, unit));
    for (const [p, q] of t.sold) into(sold, down(p), sized(q, p, unit));
    for (const [p, q] of t.bought) into(bought, up(p), sized(q, p, unit));
    if (cols.some((col) => col.key === 'profile')) {
      for (const [p, q] of profile.sold) into(profSold, down(p), sized(q, p, unit));
      for (const [p, q] of profile.bought) into(profBought, up(p), sized(q, p, unit));
    }
    const now = Date.now();
    if (cols.some((col) => col.key === 'pulls')) for (const p of book.pulls.keys()) into(pulls, p <= bb ? down(p) : up(p), sized(t.pull(book, p, now), p, unit));
    const most = (...arrs: Float64Array[]) => Math.max(1e-12, ...arrs.map((a) => a.reduce((m, v) => Math.max(m, Math.abs(v)), 0)));
    const bookMax = most(bids, asks);
    const tradeMax = most(sold, bought);
    const profileMax = Math.max(1e-12, ...profBought.map((v, i) => v + profSold[i]!));
    const pullMax = most(pulls);

    const lastRow = Number.isFinite(last) ? top - down(last) : -1;
    const lastBuy = f.prints.count > 0 && f.prints.buy[(f.prints.count - 1) % f.prints.size] === 1;
    const bidRow = top - down(bb);
    const askRow = top - up(ba);

    const bar = (col: { x: number; w: number }, y: number, share: number, color: string, alpha: number, fromRight: boolean) => {
      const bw = Math.max(1, Math.min(1, share) * (col.w - 2));
      ctx.globalAlpha = alpha;
      ctx.fillStyle = color;
      ctx.fillRect(fromRight ? col.x + col.w - 1 - bw : col.x + 1, y + 1, bw, ROW - 2);
      ctx.globalAlpha = 1;
    };
    const text = (s: string, col: { x: number; w: number }, y: number, align: CanvasTextAlign, color: string, strong: boolean, alpha = 1) => {
      ctx.font = `${strong ? '700 ' : ''}11px ${c.font}`;
      ctx.textAlign = align;
      ctx.fillStyle = color;
      ctx.globalAlpha = alpha;
      ctx.fillText(s, align === 'right' ? col.x + col.w - 5 : align === 'left' ? col.x + 5 : col.x + col.w / 2, y + ROW / 2 + 0.5);
      ctx.globalAlpha = 1;
    };

    for (let r = 0; r < rows; r++) {
      const y = HEAD + r * ROW;
      const price = (top - r) * step;
      // The spread: rows between the best bid and the best ask, shaded.
      if (r > askRow && r < bidRow) {
        ctx.globalAlpha = 0.04;
        ctx.fillStyle = c.text;
        ctx.fillRect(0, y, w, ROW);
        ctx.globalAlpha = 1;
      }
      if (r === hover) {
        ctx.globalAlpha = 0.07;
        ctx.fillStyle = c.text;
        ctx.fillRect(0, y, w, ROW);
        ctx.globalAlpha = 1;
      }
      for (const col of cols) {
        switch (col.key) {
          case 'profile': {
            const total = profBought[r]! + profSold[r]!;
            if (total <= 0) break;
            const share = total / profileMax;
            bar(col, y, (share * profSold[r]!) / total, c.bear, 0.45, false);
            bar({ x: col.x + (share * profSold[r]! * (col.w - 2)) / total, w: col.w }, y, (share * profBought[r]!) / total, c.bull, 0.45, false);
            if (share >= 0.999) text(compact(total), col, y, 'right', c.text, true);
            break;
          }
          case 'bids':
            if (bids[r]! > 0) {
              const share = bids[r]! / bookMax;
              bar(col, y, share, c.bull, 0.2, true);
              text(compact(bids[r]!), col, y, 'right', c.text, share >= 0.5, share >= 0.5 ? 1 : 0.8);
            }
            break;
          case 'asks':
            if (asks[r]! > 0) {
              const share = asks[r]! / bookMax;
              bar(col, y, share, c.bear, 0.2, false);
              text(compact(asks[r]!), col, y, 'left', c.text, share >= 0.5, share >= 0.5 ? 1 : 0.8);
            }
            break;
          case 'sold':
            if (sold[r]! > 0) {
              const share = sold[r]! / tradeMax;
              bar(col, y, share, c.bear, 0.35, true);
              text(compact(sold[r]!), col, y, 'right', c.bear, share >= 0.5);
            }
            break;
          case 'bought':
            if (bought[r]! > 0) {
              const share = bought[r]! / tradeMax;
              bar(col, y, share, c.bull, 0.35, false);
              text(compact(bought[r]!), col, y, 'left', c.bull, share >= 0.5);
            }
            break;
          case 'pulls': {
            const v = pulls[r]!;
            if (Math.abs(v) < bookMax * 1e-4) break;
            const color = v < 0 ? c.warn : c.bull;
            ctx.globalAlpha = 0.25 * Math.min(1, Math.abs(v) / pullMax);
            ctx.fillStyle = color;
            ctx.fillRect(col.x + 1, y + 1, col.w - 2, ROW - 2);
            ctx.globalAlpha = 1;
            text(`${v > 0 ? '+' : '−'}${compact(Math.abs(v))}`, col, y, 'center', color, Math.abs(v) >= pullMax * 0.5);
            break;
          }
          case 'price': {
            const isLast = r === lastRow;
            if (isLast) {
              ctx.fillStyle = lastBuy ? c.bull : c.bear;
              ctx.globalAlpha = 0.9;
              ctx.fillRect(col.x + 1, y, col.w - 2, ROW);
              ctx.globalAlpha = 1;
            }
            const side = r >= bidRow ? c.bull : r <= askRow ? c.bear : c.text;
            text(formatPrice(price, decimals), col, y, 'center', isLast ? c.surface : side, isLast, isLast ? 1 : 0.85);
            break;
          }
        }
      }
      ctx.globalAlpha = 0.25;
      ctx.fillStyle = c.border;
      ctx.fillRect(0, y + ROW - 1, w, 1);
      ctx.globalAlpha = 1;
    }
    // Your paper orders in the book's columns, and the position's entry, take profit and stop on the price.
    if (trade.live()) {
      const at = (key: Col) => cols.find((k) => k.key === key);
      for (const o of trade.orders()) {
        const buy = o.side === 'buy';
        const col = at(buy ? 'bids' : 'asks');
        const r = drag?.order.id === o.id && hover >= 0 ? hover : top - (buy ? down(o.price) : up(o.price));
        if (!col || r < 0 || r >= rows) continue;
        const label = `${buy ? 'B' : 'S'} ${compact(sized(o.qty, o.price, unit))}`;
        ctx.font = `700 10px ${c.font}`;
        const tw = ctx.measureText(label).width + 8;
        const tx = buy ? col.x + 2 : col.x + col.w - 2 - tw;
        const y = HEAD + r * ROW;
        ctx.fillStyle = c.primary;
        ctx.fillRect(tx, y + 2, tw, ROW - 4);
        ctx.fillStyle = c.surface;
        ctx.textAlign = 'left';
        ctx.fillText(label, tx + 4, y + ROW / 2 + 0.5);
      }
      const p = trade.position();
      const priceCol = at('price');
      if (p && priceCol) {
        const marks: [number | null, string, string][] = [
          [p.entry, p.side === 'long' ? 'L' : 'S', c.primary],
          [p.tp, 'TP', c.bull],
          [p.sl, 'SL', c.bear],
        ];
        for (const [v, label, color] of marks) {
          if (v === null) continue;
          const r = top - Math.round(v / step);
          if (r < 0 || r >= rows) continue;
          const y = HEAD + r * ROW;
          ctx.strokeStyle = color;
          ctx.lineWidth = 1;
          ctx.strokeRect(priceCol.x + 1.5, y + 0.5, priceCol.w - 3, ROW - 1);
          ctx.font = `700 9px ${c.font}`;
          ctx.fillStyle = color;
          ctx.textAlign = 'left';
          ctx.fillText(label, priceCol.x + 4, y + ROW / 2 + 0.5);
        }
      }
    }

    // The best bid and ask: a line under the one and over the other.
    ctx.fillStyle = c.border;
    for (const col of cols) if (col.key !== 'profile') ctx.fillRect(col.x + col.w - 1, HEAD, 1, h - HEAD);
    ctx.fillStyle = c.bull;
    if (bidRow >= 0 && bidRow < rows) ctx.fillRect(0, HEAD + bidRow * ROW, w, 1);
    ctx.fillStyle = c.bear;
    if (askRow >= 0 && askRow < rows) ctx.fillRect(0, HEAD + (askRow + 1) * ROW - 1, w, 1);

    // The totals go with the profile: what traded since it was last cleared.
    const delta = sized(profile.delta, ref, unit);
    const next: Stats = {
      volume: compact(sized(profile.volume, ref, unit)),
      delta,
      deltaText: `${delta > 0 ? '+' : delta < 0 ? '−' : ''}${compact(Math.abs(delta))}`,
      range: Number.isFinite(profile.high) ? `${formatPrice(profile.low, stepDecimals(tick))} – ${formatPrice(profile.high, stepDecimals(tick))} · ${Math.round((profile.high - profile.low) / tick)}t` : '—',
      spread: `${Math.max(0, Math.round((ba - bb) / tick))}t`,
    };
    const was = stats();
    if (!was || was.volume !== next.volume || was.deltaText !== next.deltaText || was.range !== next.range || was.spread !== next.spread) setStats(next);
  };

  let view: { redraw(): void } = { redraw: () => {} };
  onMount(() => {
    view = useCanvas(host, canvas, draw, () => {
      const f = flow();
      if (!f || f.version === seen) return false;
      seen = f.version;
      return true;
    });
    createEffect(() => {
      void [props.panel.minUsd, props.panel.unit, props.panel.ticks, props.panel.hidden, props.panel.resetTrades, props.panel.resetProfile, flow(), auto(), Object.values(doc.theme)];
      colors = null;
      view.redraw();
    });
    // Orders and the position are the account's, not the book's: drawn when they change.
    createEffect(() => {
      void [trade.live(), trade.orders(), trade.position()];
      view.redraw();
    });
  });

  const rowAt = (e: MouseEvent): number => {
    const rect = host.getBoundingClientRect();
    const y = e.clientY - rect.top - HEAD;
    return y < 0 ? -1 : Math.floor(y / ROW);
  };
  const colAt = (e: MouseEvent): Col | null => {
    const x = e.clientX - host.getBoundingClientRect().left;
    return layout.cols.find((col) => x >= col.x && x < col.x + col.w)?.key ?? null;
  };
  const priceAt = (r: number): number => ((top ?? 0) - r) * layout.step;
  /** Your order shown in a row of the book's columns. */
  const orderAt = (r: number, col: Col | null): Order | undefined => {
    if (top === null || layout.step === 0) return undefined;
    const step = layout.step;
    return trade.orders().find((o) => (o.side === 'buy' ? col === 'bids' && top! - Math.floor(o.price / step + 1e-9) === r : col === 'asks' && top! - Math.ceil(o.price / step - 1e-9) === r));
  };
  const center = () => {
    top = null;
    setAuto(true);
    view.redraw();
  };
  /** Empties a part, or all of them — everywhere, or above or below a price. */
  const clear = (part: Part | 'all', where: Where = 'all', price = 0) => {
    const f = flow();
    const both = tallies();
    if (f && both) {
      if (part === 'all' || part === 'trades') both.trades.clear(where, price);
      if (part === 'all' || part === 'profile') both.profile.clear(where, price);
      if (part === 'all' || part === 'pulls') both.trades.clearPulls(f.book, where, price);
    }
    setMenu(null);
    setClearOpen(false);
    view.redraw();
  };

  const rowChoices = () => {
    const tick = tickNow();
    return ROW_TICKS.map((m) => ({ value: m, label: tick > 0 ? formatPrice(m * tick, stepDecimals(m * tick)) : `${m} tick${m === 1 ? '' : 's'}` }));
  };

  return (
    <div class="panelcol flowpanel dom">
      <div class="toolbar">
        {/* Training picks the coin with its chart. */}
        <Show when={!props.trade} fallback={<span class="dom-coin">{symbolOf(market())}</span>}>
          <MarketButton panelId={id} market={market()} />
        </Show>
        <span class="spacer" />
        <Show when={STATE_LABEL[state()]}>{(label) => <span class={`conn ${state()}`}>{label()}</span>}</Show>
        <select data-tip="Price a row spans" onChange={(e) => patch({ ticks: Number(e.currentTarget.value) })}>
          <option value={0} selected={props.panel.ticks === 0}>
            Auto
          </option>
          <For each={rowChoices()}>
            {(c) => (
              <option value={c.value} selected={props.panel.ticks === c.value}>
                {c.label}
              </option>
            )}
          </For>
        </select>
        <select data-tip="Smallest order counted as traded: trades at one time and side count as one" onChange={(e) => patch({ minUsd: Number(e.currentTarget.value) })}>
          <For each={MIN_USD}>
            {(v) => (
              <option value={v} selected={props.panel.minUsd === v}>
                {minLabel(v)}
              </option>
            )}
          </For>
        </select>
        <button class="tb" data-tip="Sizes in dollars or in coins" onClick={() => patch({ unit: props.panel.unit === 'usd' ? 'coins' : 'usd' })}>
          {props.panel.unit === 'usd' ? 'USD' : 'Coins'}
        </button>
        <button class="tb dom-opt" classList={{ on: auto() }} data-tip="Keep the last price in the middle — or double-click the rows" onClick={center}>
          Center
        </button>
        <button class="tb dom-opt" classList={{ on: clearOpen() }} data-tip="Clear the volume profile, Bought and Sold, or Pull/Stack — right-click a row to clear above or below it" onClick={() => setClearOpen((v) => !v)}>
          Clear
        </button>
        {/* Kept however narrow: the intervals are only here. */}
        <button class="tb" classList={{ on: columnsOpen() }} aria-label="DOM options" data-tip="Columns, and when the volume profile and Bought and Sold start again" onClick={() => setColumnsOpen((v) => !v)}>
          <Icon name="settings" size={12} />
        </button>
      </div>
      <Show when={clearOpen()}>
        <div class="menu dom-menu" onPointerDown={(e) => e.stopPropagation()}>
          <For each={PARTS}>{(p) => <button onClick={() => clear(p.part)}>{p.label}</button>}</For>
        </div>
      </Show>
      <Show when={columnsOpen()}>
        <div class="menu dom-menu" onPointerDown={(e) => e.stopPropagation()}>
          <div class="cap">Columns</div>
          <For each={OPTIONAL}>
            {(o) => (
              <button data-tip={o.tip} classList={{ cur: !hidden().has(o.key) }} onClick={() => toggleColumn(o.key)}>
                <span>{o.label}</span>
                <span>{hidden().has(o.key) ? 'Off' : 'On'}</span>
              </button>
            )}
          </For>
          <Resets label="Volume profile starts again" tip="On the clock: 1h at the top of each hour, 4h at 00:00, 04:00… UTC, Day at 00:00 UTC" choices={RESET_PROFILE} value={props.panel.resetProfile} pick={(v) => patch({ resetProfile: v })} />
          <Resets label="Bought & sold start again" tip="On the clock: 5m at :00, :05, :10…" choices={RESET_TRADES} value={props.panel.resetTrades} pick={(v) => patch({ resetTrades: v })} />
        </div>
      </Show>
      {props.trade?.bar()}
      <div
        class="flowcanvas"
        ref={host}
        onWheel={(e) => {
          if (top === null) return;
          e.preventDefault();
          top += e.deltaY > 0 ? -2 : 2;
          setAuto(false);
          view.redraw();
        }}
        onPointerMove={(e) => {
          const r = rowAt(e);
          const col = colAt(e);
          host.style.cursor = trade.live() && (col === 'bids' || col === 'asks') ? 'pointer' : '';
          if (r !== hover) {
            hover = r;
            view.redraw();
          }
        }}
        onPointerDown={(e) => {
          if (e.button !== 0 || !trade.live()) return;
          const r = rowAt(e);
          const o = orderAt(r, colAt(e));
          if (!o) return;
          drag = { order: o, from: r };
          host.setPointerCapture(e.pointerId);
        }}
        onPointerUp={(e) => {
          if (e.button !== 0 || !trade.live() || top === null) return;
          const r = rowAt(e);
          const col = colAt(e);
          const d = drag;
          drag = null;
          if (d) {
            if (r === d.from) props.trade?.cancel(d.order);
            else if (r >= 0) props.trade?.move(d.order, priceAt(r));
            view.redraw();
            return;
          }
          if (r < 0) return;
          if (e.shiftKey) props.trade?.exit('sl', priceAt(r));
          else if (e.ctrlKey || e.metaKey) props.trade?.exit('tp', priceAt(r));
          else if (col === 'bids') props.trade?.limit(true, priceAt(r));
          else if (col === 'asks') props.trade?.limit(false, priceAt(r));
        }}
        onPointerLeave={() => {
          hover = -1;
          view.redraw();
        }}
        onDblClick={(e) => {
          // Twice on a bid or an ask is two orders, not a request to center.
          const col = colAt(e);
          if (!(trade.live() && (col === 'bids' || col === 'asks'))) center();
        }}
        onContextMenu={(e) => {
          const r = rowAt(e);
          if (r < 0 || top === null || layout.step === 0) return;
          e.preventDefault();
          const rect = host.getBoundingClientRect();
          setMenu({ x: Math.min(e.clientX - rect.left, rect.width - 200), y: e.clientY - rect.top, price: (top - r) * layout.step, decimals: layout.decimals });
        }}
      >
        <canvas ref={canvas} />
        <Show when={menu()}>
          {(m) => (
            <div class="menu" style={{ left: `${Math.max(0, m().x)}px`, top: `${m().y}px` }} onPointerDown={(e) => e.stopPropagation()}>
              <button onClick={center}>Center on the last price</button>
              <button onClick={() => clear('all', 'above', m().price)}>Clear above {formatPrice(m().price, m().decimals)}</button>
              <button onClick={() => clear('all', 'below', m().price)}>Clear below {formatPrice(m().price, m().decimals)}</button>
              {/* The toolbar's Clear, which a narrow DOM folds away. */}
              <For each={PARTS}>{(p) => <button onClick={() => clear(p.part)}>Clear {p.label.toLowerCase()}</button>}</For>
              {/* The toolbar's column switches, which a narrow DOM folds away. */}
              <For each={OPTIONAL}>
                {(o) => (
                  <button onClick={() => toggleColumn(o.key)}>
                    <span>{o.label} column</span>
                    <span class="dim">{hidden().has(o.key) ? 'Off' : 'On'}</span>
                  </button>
                )}
              </For>
            </div>
          )}
        </Show>
      </div>
      <Show when={stats()}>
        {(s) => (
          <div class="dom-foot num">
            <span data-tip="Volume traded since the profile was last cleared, of orders the filter lets through">
              <span class="caps">Vol</span> {s().volume}
            </span>
            <span data-tip="Bought less sold since the profile was last cleared">
              <span class="caps">Δ</span> <span classList={{ bull: s().delta > 0, bear: s().delta < 0 }}>{s().deltaText}</span>
            </span>
            <span data-tip="Lowest and highest trade since the profile was last cleared, and the ticks between">
              <span class="caps">Range</span> {s().range}
            </span>
            <span data-tip="Ticks between the best bid and the best ask">
              <span class="caps">Spread</span> {s().spread}
            </span>
          </div>
        )}
      </Show>
    </div>
  );
}

/** One count's interval: a row of choices under its name. */
function Resets(props: { label: string; tip: string; choices: readonly { value: number; label: string }[]; value: number; pick(v: number): void }) {
  return (
    <>
      <div class="cap" data-tip={props.tip}>
        {props.label}
      </div>
      <div class="group">
        <For each={props.choices}>
          {(c) => (
            <button classList={{ cur: props.value === c.value }} onClick={() => props.pick(c.value)}>
              {c.label}
            </button>
          )}
        </For>
      </div>
    </>
  );
}
