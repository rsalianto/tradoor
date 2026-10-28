/**
 * Training, over the workspace: paper trading on Binance's BTC, ETH and SOL
 * perpetuals at their live prices (see ./model and bins/server/src/training.rs).
 * The one-minute chart, with indicators and drawings as anywhere, and the
 * account's trades drawn on it; the order ticket beside it — market or limit,
 * long or short, the coin's leverage, a take profit and stop loss — and under
 * it the positions with what they make now, the resting orders, the trades
 * closed, and the statistics of the run. Beside the chart, the DOM, which
 * trades too: a click on a bid or an ask rests an order there (panels/DomTrade).
 * Kept for an account: until the browser signs in with Telegram, it shows how.
 */
import { createEffect, createMemo, createSignal, For, lazy, onCleanup, onMount, Show, untrack } from 'solid-js';
import { createStore } from 'solid-js/store';
import { setTrainingOpen, TRAINING_CHART } from '../app/ui';
import { AccountCard } from '../components/Account';
import { lastPrice } from '../engine/client';
import { api, errorText, type AccountStatus } from '../lib/api';
import { Icon } from '../lib/icons';
import { readJson, readPref, writePref } from '../lib/prefs';
import { toast } from '../lib/toasts';
import { equityCurve, groupBy, money, stamp, stats, type Group } from '../journal/model';
import type { ChartPanel } from '../panels/ChartPanel';
import { domTrade } from '../panels/DomTrade';
import { DEFAULT_STUDIES, type ChartPanelState, type DomPanelState } from '../workspace/model';
import { detached, holdDetached, setPanelMarket } from '../workspace/store';
import {
  asJournal,
  candleOf,
  coinOf,
  coins,
  liquidationAt,
  marked,
  marketOfSymbol,
  percent,
  price as fmt,
  REASON_LABEL,
  roe,
  shapes,
  symbolOfMarket,
  TRAINING_MARKETS,
  trainingApi,
  unrealized,
  usd,
  worth,
  type ClosedTrade,
  type History,
  type Position,
  type State,
} from './model';

/** The chart's own settings, kept in this browser: its coin, indicators and view. */
const PANEL_KEY = 'trainingChart';
/** The DOM's, and whether it is shown. */
const DOM_KEY = 'trainingDom';
const DOM_SHOWN_KEY = 'trainingDomShown';

// Fetched when Training shows it, not with Training itself.
const DomPanel = lazy(() => import('../panels/DomPanel').then((m) => ({ default: m.DomPanel })));
/** How often the account is read while the view is in sight: fills and liquidations land within this. */
const REFRESH_MS = 1_000;
/** Price decimals per coin, as Binance quotes them. */
const DECIMALS: Record<string, number> = { BTCUSDT: 1, ETHUSDT: 2, SOLUSDT: 3 };
const SIZE_STEPS = [0.1, 0.25, 0.5, 1];

type Tab = 'positions' | 'orders' | 'history' | 'stats';

function startPanel(): ChartPanelState {
  const saved = (readJson(PANEL_KEY) ?? {}) as Partial<ChartPanelState>;
  const market = TRAINING_MARKETS.find((m) => m === saved.market) ?? TRAINING_MARKETS[0];
  return {
    id: TRAINING_CHART,
    type: 'chart',
    link: null,
    market,
    timeframe: '1m',
    studies: typeof saved.studies === 'string' ? saved.studies : DEFAULT_STUDIES,
    hidden: typeof saved.hidden === 'string' ? saved.hidden : '',
    log: saved.log === true,
    paneWeights: typeof saved.paneWeights === 'string' ? saved.paneWeights : '',
    tpo: '',
    footprint: false,
    footprintMode: 'split',
    footprintUnit: 'coins',
    footprintTicks: 0,
    footprintValueArea: true,
    footprintStats: true,
    footprintProfile: true,
    tpoSettings: {},
    legendShut: saved.legendShut === true,
  };
}

function startDom(market: string): DomPanelState {
  const saved = (readJson(DOM_KEY) ?? {}) as Partial<DomPanelState>;
  const ticks = Number(saved.ticks);
  const minUsd = Number(saved.minUsd);
  return {
    id: 'training-dom',
    type: 'dom',
    link: null,
    market,
    timeframe: '1m',
    ticks: Number.isInteger(ticks) && ticks >= 0 ? ticks : 0,
    minUsd: Number.isFinite(minUsd) && minUsd >= 0 ? minUsd : 0,
    unit: saved.unit === 'coins' ? 'coins' : 'usd',
    hidden: typeof saved.hidden === 'string' ? saved.hidden : '',
    resetTrades: Number.isInteger(saved.resetTrades) ? saved.resetTrades! : 0,
    resetProfile: Number.isInteger(saved.resetProfile) ? saved.resetProfile! : 0,
  };
}

/**
 * `chart`: the chart panel, handed in by the page that has it already —
 * imported here, it would pull the whole chart into this lazily loaded view,
 * and the bundler would split the page's own modules apart to share them.
 */
export function Training(props: { chart: typeof ChartPanel }) {
  const [account, setAccount] = createSignal<AccountStatus | null>(null);
  const signedIn = () => !!account()?.account;
  const [state, setState] = createSignal<State | null>(null);
  const [history, setHistory] = createSignal<History | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [tab, setTab] = createSignal<Tab>('positions');
  const [now, setNow] = createSignal(Date.now());

  holdDetached(startPanel());
  const panel = () => detached[TRAINING_CHART]!;
  createEffect(() => {
    const p = panel();
    writePref(PANEL_KEY, JSON.stringify({ market: p.market, studies: p.studies, hidden: p.hidden, log: p.log, paneWeights: p.paneWeights, legendShut: p.legendShut }));
  });
  const symbol = () => symbolOfMarket(panel().market);
  const decimals = (s: string) => DECIMALS[s] ?? 2;

  // The DOM follows the chart's coin; its own settings are kept here, not in the workspace.
  const [dom, setDom] = createStore<DomPanelState>(startDom(panel().market));
  createEffect(() => setDom('market', panel().market));
  createEffect(() => writePref(DOM_KEY, JSON.stringify({ ticks: dom.ticks, minUsd: dom.minUsd, unit: dom.unit, hidden: dom.hidden, resetTrades: dom.resetTrades, resetProfile: dom.resetProfile })));
  const [domShown, setDomShown] = createSignal(readPref(DOM_SHOWN_KEY) !== '0');
  createEffect(() => writePref(DOM_SHOWN_KEY, domShown() ? '1' : '0'));

  /** The price a coin trades at now: the chart's own for its coin, the server's for the others. */
  const priceOf = (s: string): number | undefined => {
    const lp = lastPrice(TRAINING_CHART);
    if (lp && lp.market === marketOfSymbol(s) && Number.isFinite(lp.price)) return lp.price;
    return state()?.quotes[s]?.last;
  };
  const markOf = (s: string): number | undefined => state()?.quotes[s]?.mark || priceOf(s);
  const value = createMemo(() => {
    const s = state();
    // Read again as prices move: the chart's last price is a signal.
    void now();
    return s ? worth(s, priceOf) : null;
  });

  // ---------------------------------------------------------------- reading the account

  /** What closed since the trades were last read, told in a notice each. */
  let seen = 0;
  const loadHistory = async (announce: boolean) => {
    const h = await trainingApi.history();
    if (announce && seen > 0) {
      for (const t of h.trades.filter((x) => x.id > seen).reverse()) {
        toast({ title: `${coinOf(t.symbol)} ${t.side} — ${REASON_LABEL[t.reason].toLowerCase()}`, body: `${money(t.net)} at ${fmt(t.exit, decimals(t.symbol))}`, tone: t.net >= 0 ? 'info' : 'warn', way: t.net >= 0 }, 6_000);
      }
    }
    seen = Math.max(seen, ...h.trades.map((t) => t.id), 1);
    setHistory(h);
  };
  /** The positions and orders as a shape, so a change made by price shows. */
  const shape = (s: State | null) => (s ? [...s.positions.map((p) => `${p.symbol}:${p.side}:${p.qty}`), ...s.orders.map((o) => o.id)].join('|') : '');
  const load = async () => {
    if (!signedIn()) return;
    try {
      const next = await trainingApi.state();
      const changed = shape(untrack(state)) !== shape(next);
      setState(next);
      setError(null);
      if (changed || untrack(history) === null) await loadHistory(true);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const readAccount = async () => {
    try {
      setAccount(await api.account());
    } catch (e) {
      setError(errorText(e));
    }
    await load();
  };
  onMount(() => {
    void readAccount();
    const timer = setInterval(() => {
      setNow(Date.now());
      if (!document.hidden) void load();
    }, REFRESH_MS);
    onCleanup(() => clearInterval(timer));
  });

  /** Do it, then read the account again; a refusal shows where the ticket is. */
  const [busy, setBusy] = createSignal(false);
  const act = async (run: () => Promise<unknown>): Promise<boolean> => {
    setBusy(true);
    try {
      await run();
      setError(null);
      await load();
      return true;
    } catch (e) {
      setError(errorText(e));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const toolbar = (
    <>
      <For each={TRAINING_MARKETS}>
        {(m) => (
          <button class="tb" classList={{ on: panel().market === m }} onClick={() => setPanelMarket(TRAINING_CHART, m)}>
            <span>{coinOf(symbolOfMarket(m))}</span>
          </button>
        )}
      </For>
      <span class="dim tr-venue">Binance perp</span>
      <span class="sep" />
      <span class="tb num on">1m</span>
      <span class="sep" />
      <button class="tb tr-dom-toggle" classList={{ on: domShown() }} data-tip="The order book beside the chart: click a bid or an ask to rest an order there" onClick={() => setDomShown((v) => !v)}>
        DOM
      </button>
    </>
  );
  const trade = domTrade({ symbol, state, busy, act });
  const runTrades = () => (history()?.run === history()?.current ? (history()?.trades ?? []) : []);
  // Drawn again only when it changes — a trade, the position, an order, or the
  // minute its right edge reaches — not with every second's read of the account,
  // each of which would send the chart the whole list again.
  const minute = createMemo(() => Math.floor(now() / 60_000) * 60_000);
  const overlay = createMemo(() => shapes(state(), runTrades(), symbol(), minute()), [], { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) });
  /** The details over a marked candle: the trades entered or closed there, and the position opened there. */
  const hover = (time: number) => {
    const candle = candleOf(time);
    const list = marked(runTrades(), symbol(), candle);
    const open = state()?.positions.find((p) => p.symbol === symbol() && Math.abs(candleOf(p.openedAt) - candle) <= 60_000);
    if (list.length === 0 && !open) return null;
    const d = decimals(symbol());
    return (
      <>
        <Show when={open}>
          {(p) => {
            const made = () => unrealized(p(), priceOf(p().symbol) ?? p().entry);
            return (
              <div class="tc">
                <div class="tc-head">
                  <b class={p().side === 'long' ? 'bull' : 'bear'}>
                    {p().side === 'long' ? '▲' : '▼'} {coinOf(p().symbol)} {p().side} {p().leverage}×
                  </b>
                  <span class="dim">open</span>
                </div>
                <div>
                  <span class="dim">Entry</span> {fmt(p().entry, d)} <span class="dim">{stamp(p().openedAt)}</span>
                </div>
                <div>
                  <span class="dim">Size</span> {coins(p().qty)} {coinOf(p().symbol)} · <span class="dim">margin</span> {usd(p().margin)}
                </div>
                <div>
                  <span class="dim">Now</span> <b classList={{ bull: made() > 0, bear: made() < 0 }}>{money(made())}</b> ({percent(roe(p(), priceOf(p().symbol) ?? p().entry))})
                </div>
              </div>
            );
          }}
        </Show>
        <For each={list}>
          {(t) => (
            <div class="tc">
              <div class="tc-head">
                <b class={t.side === 'long' ? 'bull' : 'bear'}>
                  {t.side === 'long' ? '▲' : '▼'} {coinOf(t.symbol)} {t.side} {t.leverage}×
                </b>
                <span class="dim">{REASON_LABEL[t.reason]}</span>
              </div>
              <div>
                <span class="dim">Entry</span> {fmt(t.entry, d)} <span class="dim">{stamp(t.openedAt)}</span>
              </div>
              <div>
                <span class="dim">Exit</span> {fmt(t.exit, d)} <span class="dim">{stamp(t.closedAt)}</span>
              </div>
              <div>
                <span class="dim">Size</span> {coins(t.qty)} {coinOf(t.symbol)} · <span class="dim">margin</span> {usd(t.margin)}
              </div>
              <div>
                <span class="dim">Net</span> <b classList={{ bull: t.net > 0, bear: t.net < 0 }}>{money(t.net)}</b> ({percent(t.margin > 0 ? t.net / t.margin : 0)}) ·{' '}
                <span class="dim">fees</span> {usd(t.fees)}
              </div>
            </div>
          )}
        </For>
      </>
    );
  };

  // ---------------------------------------------------------------- reset

  const [sure, setSure] = createSignal(false);
  let sureTimer: ReturnType<typeof setTimeout> | undefined;
  const reset = () => {
    if (!sure()) {
      setSure(true);
      sureTimer = setTimeout(() => setSure(false), 4_000);
      return;
    }
    clearTimeout(sureTimer);
    setSure(false);
    void act(async () => {
      await trainingApi.reset();
      seen = 0;
      setHistory(null);
    });
  };
  onCleanup(() => clearTimeout(sureTimer));

  return (
    <section class="journal training">
      <header class="jr-head">
        <span class="jr-title">Training</span>
        <Show when={signedIn() && state() && value()}>
          {(_) => (
            <span class="tr-worth num">
              <span>
                <span class="dim">Equity</span> {usd(value()!.equity)}
              </span>
              <span classList={{ bull: value()!.unrealized > 0, bear: value()!.unrealized < 0 }}>
                <span class="dim">Unrealized</span> {money(value()!.unrealized)}
              </span>
              <span>
                <span class="dim">Available</span> {usd(value()!.available)}
              </span>
              <span class="dim">Run {state()!.run}</span>
            </span>
          )}
        </Show>
        <span class="grow" />
        <Show when={signedIn()}>
          <button class="tb" classList={{ 'jr-delete': true, sure: sure() }} disabled={busy()} data-tip="Start again at 100,000: open positions and orders go, this run's trades stay in the statistics" onClick={reset}>
            <Icon name="reset" size={12} />
            <span>{sure() ? 'Reset to 100,000?' : 'Reset'}</span>
          </button>
        </Show>
        <button class="iconbtn" aria-label="Close training" data-tip="Back to the charts" onClick={() => setTrainingOpen(false)}>
          <Icon name="close" size={15} />
        </button>
      </header>
      <Show when={account()} fallback={<p class="jr-empty muted">Reading…</p>}>
        <Show
          when={signedIn()}
          fallback={
            <div class="jr-body">
              <div class="jr-gate">
                <h2>Sign in with Telegram to train</h2>
                <p class="muted">
                  Training trades Binance's BTC, ETH and SOL perpetuals at their live prices, with 100,000 USDT to start and up to 100x: market and limit orders, long and short, a take profit and stop loss,
                  liquidation, fees and funding as Binance has them. Every trade is kept with your account, to look back on.
                </p>
                <AccountCard onSignedIn={() => void readAccount()} />
              </div>
            </div>
          }
        >
          <div class="tr-body" classList={{ 'with-dom': domShown() }}>
            <div class="tr-chart">
              <props.chart panel={panel()} training={{ toolbar, overlay, hover }} />
            </div>
            <Show when={domShown()}>
              <div class="tr-dom">
                <DomPanel panel={dom} trade={trade} patch={(p) => setDom(p)} />
              </div>
            </Show>
            <Ticket state={state()} symbol={symbol()} priceOf={priceOf} available={value()?.available ?? 0} decimals={decimals(symbol())} busy={busy()} act={act} error={error()} />
            <div class="tr-tabs">
              <nav class="chips jr-tabs">
                <button classList={{ on: tab() === 'positions' }} onClick={() => setTab('positions')}>
                  Positions <span class="dim">{state()?.positions.length ?? 0}</span>
                </button>
                <button classList={{ on: tab() === 'orders' }} onClick={() => setTab('orders')}>
                  Orders <span class="dim">{state()?.orders.length ?? 0}</span>
                </button>
                <button classList={{ on: tab() === 'history' }} onClick={() => setTab('history')}>
                  History
                </button>
                <button classList={{ on: tab() === 'stats' }} onClick={() => setTab('stats')}>
                  Statistics
                </button>
              </nav>
              <div class="tr-tab">
                <Show when={tab() === 'positions'}>
                  <Positions state={state()} priceOf={priceOf} markOf={markOf} decimals={decimals} busy={busy()} act={act} onPick={(s) => holdMarket(s)} />
                </Show>
                <Show when={tab() === 'orders'}>
                  <Orders state={state()} decimals={decimals} busy={busy()} act={act} />
                </Show>
                <Show when={tab() === 'history'}>
                  <Trades trades={history()?.trades ?? []} decimals={decimals} />
                </Show>
                <Show when={tab() === 'stats'}>
                  <Statistics history={history()} onRun={(run) => void trainingApi.history(run).then(setHistory, (e) => setError(errorText(e)))} />
                </Show>
              </div>
            </div>
          </div>
        </Show>
      </Show>
    </section>
  );

  /** Show a coin on the chart: a position's row picked. */
  function holdMarket(s: string) {
    setPanelMarket(TRAINING_CHART, marketOfSymbol(s));
  }
}

// ---------------------------------------------------------------- the order ticket

function Ticket(props: {
  state: State | null;
  symbol: string;
  priceOf: (s: string) => number | undefined;
  available: number;
  decimals: number;
  busy: boolean;
  act: (run: () => Promise<unknown>) => Promise<boolean>;
  error: string | null;
}) {
  const [type, setType] = createSignal<'market' | 'limit'>('market');
  const [limit, setLimit] = createSignal('');
  const [size, setSize] = createSignal('');
  const [exits, setExits] = createSignal(false);
  const [tp, setTp] = createSignal('');
  const [sl, setSl] = createSignal('');
  const last = () => props.priceOf(props.symbol);
  const leverage = () => props.state?.leverage[props.symbol] ?? 20;
  const held = () => props.state?.positions.find((p) => p.symbol === props.symbol);
  const locked = () => !!held() || !!props.state?.orders.some((o) => o.symbol === props.symbol);
  const [lev, setLev] = createSignal(20);
  createEffect(() => setLev(leverage()));
  // A new coin: its price in the limit box.
  createEffect(() => {
    const s = props.symbol;
    void s;
    setLimit('');
  });
  const at = () => (type() === 'limit' && Number(limit()) > 0 ? Number(limit()) : (last() ?? 0));
  const notional = () => Number(size()) || 0;
  const qty = () => (at() > 0 ? notional() / at() : 0);
  const margin = () => notional() / lev();
  const fee = () => notional() * (type() === 'market' ? (props.state?.taker ?? 0.0005) : (props.state?.maker ?? 0.0002));
  const mmr = () => props.state?.mmr[props.symbol] ?? 0.005;

  const place = (buy: boolean) => {
    const q = Number(qty().toPrecision(6));
    if (!(q > 0)) return;
    void props.act(() =>
      trainingApi.order({
        symbol: props.symbol,
        side: buy ? 'buy' : 'sell',
        type: type(),
        qty: q,
        ...(type() === 'limit' ? { price: Number(limit()) || at() } : {}),
        ...(exits() && Number(tp()) > 0 ? { tp: Number(tp()) } : {}),
        ...(exits() && Number(sl()) > 0 ? { sl: Number(sl()) } : {}),
      }),
    );
  };
  const setLeverage = (v: number) => {
    setLev(v);
    void props.act(() => trainingApi.leverage(props.symbol, v));
  };

  return (
    <aside class="tr-ticket">
      <div class="tr-price num">
        <span class="tr-coin">{coinOf(props.symbol)}USDT</span>
        <span>{fmt(last(), props.decimals)}</span>
      </div>
      <div class="chips tr-types">
        <button classList={{ on: type() === 'market' }} onClick={() => setType('market')}>
          Market
        </button>
        <button classList={{ on: type() === 'limit' }} onClick={() => setType('limit')}>
          Limit
        </button>
      </div>
      <label class="tfield">
        <span class="tlabel">
          Leverage <b class="num">{lev()}×</b>
        </span>
        <input
          type="range"
          min="1"
          max={props.state?.maxLeverage ?? 100}
          value={lev()}
          disabled={locked() || props.busy}
          data-tip={locked() ? 'Close the position and cancel its orders to change the leverage' : undefined}
          onInput={(e) => setLev(Number(e.currentTarget.value))}
          onChange={(e) => setLeverage(Number(e.currentTarget.value))}
        />
      </label>
      <Show when={type() === 'limit'}>
        <label class="tfield">
          <span class="tlabel">Price</span>
          <input class="dinput num" type="number" inputmode="decimal" step={10 ** -props.decimals} placeholder={fmt(last(), props.decimals).replace(/,/g, '')} value={limit()} onFocus={() => !limit() && setLimit(String(last() ?? ''))} onInput={(e) => setLimit(e.currentTarget.value)} />
        </label>
      </Show>
      <label class="tfield">
        <span class="tlabel">Size (USDT)</span>
        <input class="dinput num" type="number" inputmode="decimal" min="0" placeholder="0" value={size()} onInput={(e) => setSize(e.currentTarget.value)} />
        <span class="thint num">
          {coins(qty())} {coinOf(props.symbol)} · margin {usd(margin())}
        </span>
      </label>
      <div class="chips tr-steps">
        <For each={SIZE_STEPS}>
          {(f) => (
            <button data-tip={`${f * 100}% of what is free, at ${lev()}×`} onClick={() => setSize(String(Math.max(0, Math.floor(props.available * f * lev() * 0.999))))}>
              {f * 100}%
            </button>
          )}
        </For>
      </div>
      <label class="jr-check">
        <input type="checkbox" checked={exits()} onChange={(e) => setExits(e.currentTarget.checked)} />
        TP / SL
      </label>
      <Show when={exits()}>
        <div class="tgrid two">
          <label class="tfield">
            <span class="tlabel">Take profit</span>
            <input class="dinput num" type="number" inputmode="decimal" step={10 ** -props.decimals} value={tp()} onInput={(e) => setTp(e.currentTarget.value)} />
          </label>
          <label class="tfield">
            <span class="tlabel">Stop loss</span>
            <input class="dinput num" type="number" inputmode="decimal" step={10 ** -props.decimals} value={sl()} onInput={(e) => setSl(e.currentTarget.value)} />
          </label>
        </div>
      </Show>
      <div class="tr-sides">
        <button class="tr-buy" disabled={props.busy || !(notional() > 0)} onClick={() => place(true)}>
          <span>Buy / Long</span>
          <span class="num">liq {fmt(liquidationAt('long', at(), lev(), mmr()), props.decimals)}</span>
        </button>
        <button class="tr-sell" disabled={props.busy || !(notional() > 0)} onClick={() => place(false)}>
          <span>Sell / Short</span>
          <span class="num">liq {fmt(liquidationAt('short', at(), lev(), mmr()), props.decimals)}</span>
        </button>
      </div>
      <p class="tnote num">
        Available {usd(props.available)} · fee ≈ {usd(fee())}
        <Show when={props.state?.quotes[props.symbol]?.nextFunding}>
          {' '}
          · funding {percent(props.state!.quotes[props.symbol]!.funding)} at {stamp(props.state!.quotes[props.symbol]!.nextFunding).slice(6)}
        </Show>
      </p>
      <Show when={props.error}>{(e) => <p class="bear tnote">{e()}</p>}</Show>
    </aside>
  );
}

// ---------------------------------------------------------------- positions and orders

function Positions(props: {
  state: State | null;
  priceOf: (s: string) => number | undefined;
  markOf: (s: string) => number | undefined;
  decimals: (s: string) => number;
  busy: boolean;
  act: (run: () => Promise<unknown>) => Promise<boolean>;
  onPick: (symbol: string) => void;
}) {
  const [editing, setEditing] = createSignal<string | null>(null);
  const [tp, setTp] = createSignal('');
  const [sl, setSl] = createSignal('');
  const edit = (p: Position) => {
    setEditing(p.symbol);
    setTp(p.tp === null ? '' : String(p.tp));
    setSl(p.sl === null ? '' : String(p.sl));
  };
  const saveExits = (p: Position) =>
    void props.act(async () => {
      await trainingApi.exits(p.symbol, { tp: Number(tp()) > 0 ? Number(tp()) : null, sl: Number(sl()) > 0 ? Number(sl()) : null });
      setEditing(null);
    });
  return (
    <Show when={(props.state?.positions.length ?? 0) > 0} fallback={<p class="jr-empty muted">No open position. Pick a coin on the chart and place an order.</p>}>
      <div class="tr-table">
        <table class="jr-table num">
          <thead>
            <tr>
              <th>Coin</th>
              <th>Size</th>
              <th>Entry</th>
              <th>Mark</th>
              <th>Liq.</th>
              <th>Margin</th>
              <th>PnL (ROE)</th>
              <th>TP / SL</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {/* By coin, not by the position as read: the account is read every second, and a
                row made afresh each time would take the take-profit box being typed in with it. */}
            <For each={props.state!.positions.map((x) => x.symbol)}>
              {(symbol) => {
                const p = () => props.state!.positions.find((x) => x.symbol === symbol)!;
                const d = () => props.decimals(symbol);
                const now = () => props.priceOf(symbol) ?? p().entry;
                const made = () => unrealized(p(), now());
                return (
                  <Show when={props.state!.positions.some((x) => x.symbol === symbol)}>
                    <tr>
                      <td>
                        <button class="tr-coin-pick" onClick={() => props.onPick(symbol)}>
                          <b class={p().side === 'long' ? 'bull' : 'bear'}>{coinOf(symbol)}</b>{' '}
                          <span class="dim">
                            {p().side} {p().leverage}×
                          </span>
                        </button>
                      </td>
                      <td>
                        {coins(p().qty)} <span class="dim">{usd(p().qty * now())}</span>
                      </td>
                      <td>{fmt(p().entry, d())}</td>
                      <td>{fmt(props.markOf(symbol), d())}</td>
                      <td class="bear">{fmt(p().liquidation, d())}</td>
                      <td>{usd(p().margin)}</td>
                      <td classList={{ bull: made() > 0, bear: made() < 0 }}>
                        {money(made())} <span class="dim">({percent(roe(p(), now()))})</span>
                      </td>
                      <td>
                        <button class="tr-exits" data-tip="Set the take profit and stop loss" onClick={() => edit(p())}>
                          <span class="bull">{fmt(p().tp, d())}</span> / <span class="bear">{fmt(p().sl, d())}</span> <Icon name="pencil" size={10} />
                        </button>
                      </td>
                      <td class="tr-actions">
                        <button class="tb" disabled={props.busy} data-tip="Close half at the market" onClick={() => void props.act(() => trainingApi.close(symbol, Number((p().qty / 2).toPrecision(6))))}>
                          50%
                        </button>
                        <button class="tb" disabled={props.busy} data-tip={`Close it and open as much ${p().side === 'long' ? 'short' : 'long'} at the market, with no TP / SL`} onClick={() => void props.act(() => trainingApi.reverse(symbol))}>
                          Reverse
                        </button>
                        <button class="tb" disabled={props.busy} data-tip="Close it all at the market" onClick={() => void props.act(() => trainingApi.close(symbol))}>
                          Close
                        </button>
                      </td>
                    </tr>
                    <Show when={editing() === symbol}>
                      <tr class="tr-edit">
                        <td colSpan={9}>
                          <span class="dim">Take profit</span>
                          <input class="dinput num" type="number" inputmode="decimal" step={10 ** -d()} value={tp()} placeholder="none" onInput={(e) => setTp(e.currentTarget.value)} />
                          <span class="dim">Stop loss</span>
                          <input class="dinput num" type="number" inputmode="decimal" step={10 ** -d()} value={sl()} placeholder="none" onInput={(e) => setSl(e.currentTarget.value)} />
                          <button class="tb on" disabled={props.busy} onClick={() => saveExits(p())}>
                            Save
                          </button>
                          <button class="tb" onClick={() => setEditing(null)}>
                            Cancel
                          </button>
                        </td>
                      </tr>
                    </Show>
                  </Show>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

function Orders(props: { state: State | null; decimals: (s: string) => number; busy: boolean; act: (run: () => Promise<unknown>) => Promise<boolean> }) {
  return (
    <Show when={(props.state?.orders.length ?? 0) > 0} fallback={<p class="jr-empty muted">No resting order. A limit order waits here until the price reaches it.</p>}>
      <div class="tr-table">
        <table class="jr-table num">
          <thead>
            <tr>
              <th>Placed</th>
              <th>Coin</th>
              <th>Side</th>
              <th>Price</th>
              <th>Size</th>
              <th>TP / SL</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {/* By order: an order does not change while it waits, and a row made afresh each second is work for nothing. */}
            <For each={props.state!.orders.map((x) => x.id)}>
              {(id) => {
                const o = props.state!.orders.find((x) => x.id === id)!;
                return (
                  <tr>
                    <td>{stamp(o.createdAt)}</td>
                    <td>{coinOf(o.symbol)}</td>
                    <td class={o.side === 'buy' ? 'bull' : 'bear'}>{o.side === 'buy' ? 'Buy' : 'Sell'} limit</td>
                    <td>{fmt(o.price, props.decimals(o.symbol))}</td>
                    <td>
                      {coins(o.qty)} <span class="dim">{usd(o.qty * o.price)}</span>
                    </td>
                    <td>
                      <span class="bull">{fmt(o.tp, props.decimals(o.symbol))}</span> / <span class="bear">{fmt(o.sl, props.decimals(o.symbol))}</span>
                    </td>
                    <td class="tr-actions">
                      <button class="tb" disabled={props.busy} onClick={() => void props.act(() => trainingApi.cancel(o.id))}>
                        Cancel
                      </button>
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

function Trades(props: { trades: ClosedTrade[]; decimals: (s: string) => number }) {
  return (
    <Show when={props.trades.length > 0} fallback={<p class="jr-empty muted">No trade closed in this run yet.</p>}>
      <div class="tr-table">
        <table class="jr-table num">
          <thead>
            <tr>
              <th>Closed</th>
              <th>Coin</th>
              <th>Size</th>
              <th>Entry</th>
              <th>Exit</th>
              <th>How</th>
              <th>Fees</th>
              <th>Funding</th>
              <th>Net (on margin)</th>
            </tr>
          </thead>
          <tbody>
            <For each={props.trades}>
              {(t) => (
                <tr>
                  <td>{stamp(t.closedAt)}</td>
                  <td>
                    <b class={t.side === 'long' ? 'bull' : 'bear'}>{coinOf(t.symbol)}</b> <span class="dim">{t.side} {t.leverage}×</span>
                  </td>
                  <td>{coins(t.qty)}</td>
                  <td>{fmt(t.entry, props.decimals(t.symbol))}</td>
                  <td>{fmt(t.exit, props.decimals(t.symbol))}</td>
                  <td>{REASON_LABEL[t.reason]}</td>
                  <td>{money(-t.fees)}</td>
                  <td>{money(t.funding)}</td>
                  <td classList={{ bull: t.net > 0, bear: t.net < 0 }}>
                    {money(t.net)} <span class="dim">({percent(t.margin > 0 ? t.net / t.margin : 0)})</span>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

// ---------------------------------------------------------------- statistics

function Statistics(props: { history: History | null; onRun: (run: number) => void }) {
  const trades = () => (props.history?.trades ?? []).map(asJournal);
  const s = createMemo(() => stats(trades()));
  const curve = createMemo(() => equityCurve(trades()));
  const raw = () => props.history?.trades ?? [];
  const fees = () => raw().reduce((a, t) => a + t.fees, 0);
  const funding = () => raw().reduce((a, t) => a + t.funding, 0);
  const runs = () => {
    const h = props.history;
    if (!h) return [];
    const listed = h.runs.map((r) => r.run);
    return listed.includes(h.current) ? listed : [h.current, ...listed];
  };
  const cards = (): [string, string, string?][] => [
    ['Net P&L', money(s().totalPnl), s().totalPnl > 0 ? 'bull' : s().totalPnl < 0 ? 'bear' : ''],
    ['Win rate', `${s().winRate.toFixed(1)}%`, ''],
    ['Wins / losses', `${s().wins} / ${s().losses}`],
    ['Trades', String(s().closed)],
    ['Profit factor', s().profitFactor === null ? '—' : s().profitFactor!.toFixed(2)],
    ['Avg win', money(s().avgWin), 'bull'],
    ['Avg loss', money(-s().avgLoss), 'bear'],
    ['Best', money(s().best), 'bull'],
    ['Worst', money(s().worst), 'bear'],
    ['Max drawdown', money(-s().maxDrawdown), 'bear'],
    ['Streak', s().streak ? `${s().streak} ${s().streakKind === 'win' ? 'wins' : 'losses'}` : '—'],
    ['Avg hold', s().avgHoldHours === null ? '—' : s().avgHoldHours! < 1 ? `${Math.round(s().avgHoldHours! * 60)} min` : `${s().avgHoldHours!.toFixed(1)} h`],
    ['Fees paid', money(-fees()), 'bear'],
    ['Funding', money(funding())],
  ];
  const groups = (): [string, Group[]][] => [
    ['By coin', groupBy(trades(), (t) => coinOf(symbolOfMarket(t.market)))],
    ['By side', groupBy(trades(), (t) => t.side)],
    ['By how it ended', groupBy(trades(), (t) => t.strategy)],
  ];
  const path = () => {
    const pts = [{ at: 0, total: 0 }, ...curve()];
    const lo = Math.min(0, ...pts.map((p) => p.total));
    const hi = Math.max(0, ...pts.map((p) => p.total));
    const span = hi - lo || 1;
    return {
      line: pts.map((p, i) => `${((i / Math.max(1, pts.length - 1)) * 600).toFixed(1)},${(150 - ((p.total - lo) / span) * 140 - 5).toFixed(1)}`).join(' '),
      zero: (150 - ((0 - lo) / span) * 140 - 5).toFixed(1),
      up: (pts[pts.length - 1]?.total ?? 0) >= 0,
    };
  };
  return (
    <div class="tr-stats">
      <div class="tr-runs">
        <span class="dim">Run</span>
        <select class="dinput" value={props.history?.run ?? ''} onChange={(e) => props.onRun(Number(e.currentTarget.value))}>
          <For each={runs()}>
            {(r) => (
              <option value={r}>
                #{r}
                {r === props.history?.current ? ' (now)' : ''}
              </option>
            )}
          </For>
        </select>
      </div>
      <Show when={s().closed > 0} fallback={<p class="jr-empty muted">No trade closed in this run yet: the statistics fill in as trades close.</p>}>
        <div class="tr-cards num">
          <For each={cards()}>
            {([label, v, tone]) => (
              <div class="tr-card">
                <span class="dim">{label}</span>
                <b class={tone ?? ''}>{v}</b>
              </div>
            )}
          </For>
        </div>
        <svg class="tr-curve" viewBox="0 0 600 150" preserveAspectRatio="none" aria-label="Equity curve">
          <line x1="0" x2="600" y1={path().zero} y2={path().zero} class="tr-zero" />
          <polyline points={path().line} class={path().up ? 'bull' : 'bear'} fill="none" vector-effect="non-scaling-stroke" />
        </svg>
        <div class="tr-groups">
          <For each={groups()}>
            {([title, rows]) => (
              <table class="jr-table num">
                <thead>
                  <tr>
                    <th>{title}</th>
                    <th>Trades</th>
                    <th>Win rate</th>
                    <th>Net</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={rows}>
                    {(g) => (
                      <tr>
                        <td>{g.key}</td>
                        <td>{g.trades}</td>
                        <td>{g.winRate.toFixed(0)}%</td>
                        <td classList={{ bull: g.pnl > 0, bear: g.pnl < 0 }}>{money(g.pnl)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
