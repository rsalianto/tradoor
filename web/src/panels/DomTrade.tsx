/**
 * Paper trading from the DOM, in Training (training/): its account, its
 * Binance perpetuals, read and acted on by the Training view, which hands
 * them in. A click on a bid rests a buy there, on an ask a sell; a click on an
 * order of yours cancels it, and dragging it moves it. With a position open,
 * Shift-click a row for its stop loss and Ctrl-click for its take profit. The
 * bar under the DOM's toolbar sizes those orders and trades at the market.
 */
import { createEffect, createSignal, Show, type Accessor, type JSX } from 'solid-js';
import { formatPrice } from '../lib/format';
import { readJson, writePref } from '../lib/prefs';
import { coinOf, coins, trainingApi, unrealized, usd, type Order, type Position, type State } from '../training/model';

/** What a click orders, in USDT: kept in this browser. */
const SIZE_KEY = 'domTradeUsd';

export interface DomTrade {
  /** The account read. */
  live: Accessor<boolean>;
  orders(): Order[];
  position(): Position | undefined;
  limit(buy: boolean, price: number): void;
  cancel(order: Order): void;
  move(order: Order, price: number): void;
  exit(kind: 'tp' | 'sl', price: number): void;
  /** Size, market orders, and the position: under the DOM's toolbar. */
  bar(): JSX.Element;
}

export interface TradeAccount {
  symbol: Accessor<string>;
  state: Accessor<State | null>;
  busy: Accessor<boolean>;
  /** Does it, then reads the account again; a refusal shows on the ticket. */
  act(run: () => Promise<unknown>): Promise<boolean>;
}

export function domTrade(account: TradeAccount): DomTrade {
  const { symbol, state, busy, act } = account;
  const stored = Number(readJson(SIZE_KEY));
  const [size, setSize] = createSignal(Number.isFinite(stored) && stored > 0 ? stored : 1_000);
  createEffect(() => writePref(SIZE_KEY, String(size())));

  const orders = () => state()?.orders.filter((o) => o.symbol === symbol()) ?? [];
  const position = () => state()?.positions.find((p) => p.symbol === symbol());
  const last = () => state()?.quotes[symbol()]?.last ?? 0;
  const qtyAt = (price: number) => (price > 0 ? Number((size() / price).toPrecision(6)) : 0);
  const order = (buy: boolean, type: 'market' | 'limit', price: number) => {
    const q = qtyAt(price);
    if (q > 0 && !busy()) void act(() => trainingApi.order({ symbol: symbol(), side: buy ? 'buy' : 'sell', type, qty: q, ...(type === 'limit' ? { price } : {}) }));
  };

  return {
    live: () => state() !== null,
    orders,
    position,
    limit: (buy, price) => order(buy, 'limit', price),
    cancel: (o) => void act(() => trainingApi.cancel(o.id)),
    // No order is changed in place: cancelled, and placed again at the new price.
    move: (o, price) =>
      void act(async () => {
        await trainingApi.cancel(o.id);
        await trainingApi.order({ symbol: o.symbol, side: o.side, type: 'limit', qty: o.qty, price, ...(o.tp ? { tp: o.tp } : {}), ...(o.sl ? { sl: o.sl } : {}) });
      }),
    exit(kind, price) {
      const p = position();
      if (p) void act(() => trainingApi.exits(p.symbol, kind === 'tp' ? { tp: price, sl: p.sl } : { tp: p.tp, sl: price }));
    },
    bar: () => (
      <div class="dom-trade">
        <label class="dom-size" data-tip="What a click on the DOM orders, in USDT">
          <input class="dinput num" type="number" inputmode="decimal" min="0" value={size()} onInput={(e) => Number(e.currentTarget.value) > 0 && setSize(Number(e.currentTarget.value))} />
          <span class="dim">USDT</span>
        </label>
        <button class="tb dom-buy" disabled={busy()} data-tip="Buy at the market" onClick={() => order(true, 'market', last())}>
          Buy
        </button>
        <button class="tb dom-sell" disabled={busy()} data-tip="Sell at the market" onClick={() => order(false, 'market', last())}>
          Sell
        </button>
        <button class="tb" disabled={busy() || !position()} data-tip="Close the position at the market" onClick={() => void act(() => trainingApi.close(symbol()))}>
          Flat
        </button>
        <button class="tb" disabled={busy() || !position()} data-tip="Close it and open as much the other way, at the market" onClick={() => void act(() => trainingApi.reverse(symbol()))}>
          Reverse
        </button>
        <button
          class="tb"
          disabled={busy() || orders().length === 0}
          data-tip="Cancel this coin's orders"
          onClick={() =>
            void act(async () => {
              for (const o of orders()) await trainingApi.cancel(o.id);
            })
          }
        >
          Cancel all
        </button>
        <span class="spacer" />
        <Held position={position()} last={last()} />
      </div>
    ),
  };
}

/** The position, and what it makes at the last price. */
function Held(props: { position: Position | undefined; last: number }) {
  return (
    <Show when={props.position} fallback={<span class="dim" data-tip="Click your order to cancel it, drag it to move it; Shift-click a row for the stop loss, Ctrl-click for the take profit">Click a bid to buy, an ask to sell</span>}>
      {(p) => {
        const pnl = () => unrealized(p(), props.last);
        return (
          <span class="num">
            <span classList={{ bull: p().side === 'long', bear: p().side === 'short' }}>{p().side === 'long' ? 'Long' : 'Short'}</span> {coins(p().qty)} {coinOf(p().symbol)} <span class="dim">@</span> {formatPrice(p().entry)}{' '}
            <span classList={{ bull: pnl() > 0, bear: pnl() < 0 }}>{usd(pnl())}</span>
          </span>
        );
      }}
    </Show>
  );
}
