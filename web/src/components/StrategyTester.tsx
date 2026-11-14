/**
 * The strategy tester, under a chart: a strategy backtested over the chart's
 * bars in a worker of its own (lib/backtest-live), again as each bar closes, and
 * its report here — the figures, the trades, the charts, the optimizer and the
 * same strategy on other markets. Everything is worked out in this browser from
 * the exchange's bars; nothing about the strategy leaves it but a link the
 * reader chooses to share. Loaded the first time a chart opens it.
 */
import { createMemo, createSignal, For, Match, onCleanup, Show, Switch } from 'solid-js';
import type { PoolData } from '../engine/backtest-worker';
import type { BacktestResult, StrategyRun } from '../engine/protocol';
import { barOf, money, pct, ratio, REASONS, shareOf, span, statsOf, tradesCsv, tradesOf, venueFees, type Stats, type TradeRow } from '../lib/backtest';
import type { BacktestState } from '../lib/backtest-state';
import { Icon } from '../lib/icons';
import { symbolOf } from '../lib/markets';
import { formatAlertTime } from '../lib/timezone';
import { toast } from '../lib/toasts';
import { BacktestCharts } from './BacktestCharts';
import { BacktestLab } from './BacktestLab';
import { BacktestSetup } from './BacktestSetup';
import './strategy-tester.css';

export interface TesterProps {
  sid: string;
  state: BacktestState;
  result: BacktestResult | null;
  /** What the chart's worker runs: for the optimizer and the markets tab to run again. */
  run: StrategyRun | null;
  /** The bars the live backtest ran over, and what it read beside them. */
  data(): Omit<PoolData, 'source' | 'settings' | 'split'> | null;
  market: string;
  timeframe: string;
  bars: number;
  loading: boolean;
  exhausted: boolean;
  precision: number;
  onChange(patch: Partial<BacktestState>): void;
  onClose(): void;
  onShowSpan(from: number, to: number): void;
  onOpenMarket(market: string): void;
}

const TABS: { id: string; label: string }[] = [
  { id: 'summary', label: 'Summary' },
  { id: 'trades', label: 'Trades' },
  { id: 'charts', label: 'Charts' },
  { id: 'strategy', label: 'Strategy' },
  { id: 'optimize', label: 'Optimize' },
  { id: 'markets', label: 'Markets' },
];

export type Ok = BacktestResult & { ok: true };

const tone = (v: number) => (v > 0 ? 'bull' : v < 0 ? 'bear' : '');
const when = (ms: number) => (Number.isFinite(ms) ? formatAlertTime(ms) : '—');

export function StrategyTester(props: TesterProps) {
  const ok = (): Ok | null => (props.result?.ok ? props.result : null);
  const failed = () => (props.result && !props.result.ok ? props.result : null);
  const rows = createMemo(() => {
    const r = ok();
    return r ? tradesOf(r.trades) : [];
  });
  const stats = createMemo(() => {
    const r = ok();
    return r && r.equity.length > 0 ? statsOf(r, rows()) : null;
  });

  // ---------------------------------------------------------------- the height, dragged

  let startY = 0;
  let startH = 0;
  const drag = (e: PointerEvent) => {
    const h = Math.round(Math.min(window.innerHeight * 0.85, Math.max(140, startH + startY - e.clientY)));
    props.onChange({ height: h });
  };
  const stop = () => {
    window.removeEventListener('pointermove', drag);
    window.removeEventListener('pointerup', stop);
  };
  onCleanup(stop);

  const share = async () => {
    const s = props.state;
    const url = `${location.origin}/?market=${encodeURIComponent(props.market)}&tf=${encodeURIComponent(props.timeframe)}&bt=${shareOf(s)}`;
    try {
      await navigator.clipboard.writeText(url);
      toast({ tone: 'info', title: 'Link copied', body: 'Whoever opens it gets this strategy and account on this chart, and their own browser works the report out from the exchange’s bars.' });
    } catch {
      toast({ tone: 'warn', title: 'Not copied', body: 'This browser would not let the page write to the clipboard.' });
    }
  };

  const next = () => {
    const r = ok();
    if (!r || r.next === 0) return null;
    const at = Number.isFinite(r.nextLimit) && r.next !== 2 ? ` · limit ${r.nextLimit.toFixed(props.precision)}` : '';
    return r.next === 1 ? `Next open: long${at}` : r.next === -1 ? `Next open: short${at}` : 'Next open: exit';
  };

  return (
    <section class="bt" style={{ height: `${props.state.height}px` }} aria-label="Strategy tester" onKeyDown={(e) => e.stopPropagation()}>
      <div
        class="bt-grip"
        aria-hidden="true"
        onPointerDown={(e) => {
          e.preventDefault();
          startY = e.clientY;
          startH = props.state.height;
          window.addEventListener('pointermove', drag);
          window.addEventListener('pointerup', stop);
        }}
      />
      <header class="bt-head">
        <span class="bt-title">
          <Icon name="flask" size={13} />
          Strategy tester
        </span>
        <span class="bt-kpis num">
          <Show when={stats()} fallback={<span class="dim">{failed() ? '' : !props.run ? 'No strategy yet' : props.result === null ? 'Working it out…' : 'No bars yet'}</span>}>
            {(s) => (
              <>
                <span class={tone(s().net)}>{pct(s().netPct)}</span>
                <span data-tip="The deepest fall of the equity from a high">DD {pct(-s().maxDdPct)}</span>
                <span data-tip="What the winners made over what the losers lost">PF {ratio(s().profitFactor)}</span>
                <span>{s().trades} trades</span>
                <span class="bt-wide">Win {pct(s().winRate, false)}</span>
              </>
            )}
          </Show>
          <Show when={failed()}>{(f) => <span class="warn">▲ {f().line > 0 ? `Line ${f().line}: ` : ''}{f().message}</span>}</Show>
          <Show when={next()}>{(n) => <span class="badge">{n()}</span>}</Show>
        </span>
        <span class="spacer" />
        <button class="iconbtn" classList={{ on: props.state.draw }} aria-label={props.state.draw ? 'Hide the trades on the chart' : 'Show the trades on the chart'} data-tip={props.state.draw ? 'Trades drawn on the chart · click to hide' : 'Trades hidden · click to draw them'} onClick={() => props.onChange({ draw: !props.state.draw })}>
          <Icon name={props.state.draw ? 'eye' : 'eyeOff'} size={13} />
        </button>
        <button class="iconbtn" classList={{ on: props.state.alerts }} aria-label="Notify on the strategy's orders" data-tip={props.state.alerts ? 'A notice whenever a bar closes with an order, while tradoor is open · click to stop' : 'Tell me when a bar closes with an order, while tradoor is open'} onClick={() => void toggleAlerts(props)}>
          <Icon name="alarm" size={13} />
        </button>
        <button class="iconbtn" aria-label="Copy a link to this strategy" data-tip="Copy a link: this strategy and account, for anyone to test in their own browser" onClick={() => void share()}>
          <Icon name="link" size={13} />
        </button>
        <button class="iconbtn" aria-label="Close the strategy tester" data-tip="Close the tester — the strategy is kept" onClick={() => props.onClose()}>
          <Icon name="close" size={13} />
        </button>
      </header>
      <nav class="bt-tabs" role="tablist">
        <For each={TABS}>
          {(t) => (
            <button role="tab" aria-selected={props.state.tab === t.id} classList={{ on: props.state.tab === t.id }} onClick={() => props.onChange({ tab: t.id })}>
              {t.label}
            </button>
          )}
        </For>
      </nav>
      <div class="bt-body">
        <Switch>
          <Match when={props.state.tab === 'strategy'}>
            <BacktestSetup {...props} />
          </Match>
          <Match when={props.state.tab === 'optimize' || props.state.tab === 'markets'}>
            <BacktestLab {...props} tab={props.state.tab} />
          </Match>
          <Match when={!props.run}>
            <p class="bt-empty">
              Nothing to test yet: {props.state.source === 'script' ? 'choose one of the library’s strategies' : props.state.source === 'code' ? 'write a strategy, or start from an example' : 'add a condition to enter on'}.{' '}
              <button class="linkbtn" onClick={() => props.onChange({ tab: 'strategy' })}>
                Open the strategy
              </button>
            </p>
          </Match>
          <Match when={!ok()}>
            <p class="bt-empty">
              <Show when={failed()} fallback={props.result === null ? 'Working it out on the chart’s bars…' : 'No result yet.'}>
                {(f) => (
                  <>
                    The strategy does not run: {f().line > 0 ? `line ${f().line}, column ${f().col}: ` : ''}
                    {f().message}.{' '}
                    <button class="linkbtn" onClick={() => props.onChange({ tab: 'strategy' })}>
                      Open the strategy
                    </button>
                  </>
                )}
              </Show>
            </p>
          </Match>
          <Match when={props.state.tab === 'trades'}>
            <Trades rows={rows()} result={ok()!} precision={props.precision} market={props.market} onShow={props.onShowSpan} />
          </Match>
          <Match when={props.state.tab === 'charts'}>
            <BacktestCharts result={ok()!} rows={rows()} stats={stats()} state={props.state} />
          </Match>
          <Match when={true}>
            <Summary {...props} result={ok()!} rows={rows()} stats={stats()} />
          </Match>
        </Switch>
      </div>
    </section>
  );
}

/** Notices on: the browser asked once for its permission, and a toast always. */
async function toggleAlerts(props: TesterProps): Promise<void> {
  const on = !props.state.alerts;
  props.onChange({ alerts: on });
  if (on && typeof Notification !== 'undefined' && Notification.permission === 'default') {
    try {
      await Notification.requestPermission();
    } catch {
      // A toast still says it.
    }
  }
  toast({ tone: 'info', title: on ? 'Strategy notices on' : 'Strategy notices off', body: on ? 'While tradoor is open, a notice each time a bar closes with an entry or an exit.' : 'No more notices from this strategy.' });
}

// ---------------------------------------------------------------- the summary

function Figure(props: { label: string; value: string; sub?: string; tone?: string; tip?: string }) {
  return (
    <div class="bt-fig" data-tip={props.tip}>
      <span class="l">{props.label}</span>
      <span class={`v num ${props.tone ?? ''}`}>{props.value}</span>
      <Show when={props.sub}>
        <span class="s num">{props.sub}</span>
      </Show>
    </div>
  );
}

function Figures(props: { s: Stats }) {
  const s = () => props.s;
  return (
    <div class="bt-figs">
      <Figure label="Net profit" value={money(s().net)} sub={pct(s().netPct)} tone={tone(s().net)} />
      <Figure label="Buy and hold" value={pct(s().buyHoldPct)} sub="the coin itself" tone={tone(s().buyHoldPct)} tip="What holding the coin from the first bar to the last made" />
      <Figure label="Max drawdown" value={pct(-s().maxDdPct)} sub={`${money(-s().maxDd)} · longest ${span(s().ddLongest)}`} tone="bear" tip="The deepest fall of the equity from a high, and the longest it stayed under one" />
      <Figure label="Profit factor" value={ratio(s().profitFactor)} sub={`${money(s().grossProfit)} / ${money(-s().grossLoss)}`} tip="What the winners made over what the losers lost: above 1 made money" />
      <Figure label="Win rate" value={pct(s().winRate, false)} sub={`${s().wins} won · ${s().losses} lost`} />
      <Figure label="Expectancy" value={money(s().expectancy)} sub={`${pct(s().expectancyPct)} a trade${Number.isFinite(s().avgR) ? ` · ${ratio(s().avgR)}R` : ''}`} tone={tone(s().expectancy)} tip="What a trade made on average, after its costs" />
      <Figure label="Sharpe" value={ratio(s().sharpe)} sub={`Sortino ${ratio(s().sortino)}`} tip="The equity's daily return over its spread, a year of 365 days; Sortino counts only the falls" />
      <Figure label="CAGR" value={pct(s().cagr)} sub={`recovery ${ratio(s().recovery)}`} tone={tone(s().cagr)} tip="The yearly growth the result comes to; recovery is the net profit over the deepest drawdown" />
      <Figure label="Trades" value={String(s().trades)} sub={`${s().longs} long ${pct(s().longWinRate, false)} · ${s().shorts} short ${pct(s().shortWinRate, false)}`} tip="Closed longs and shorts, and how many of each won" />
      <Figure label="Avg win / loss" value={`${money(s().avgWin)} / ${money(s().avgLoss)}`} sub={`payoff ${ratio(s().payoff)}`} />
      <Figure label="Largest" value={`${money(s().largestWin)} / ${money(s().largestLoss)}`} sub={`streaks ${s().winStreak} won · ${s().lossStreak} lost`} />
      <Figure label="In the market" value={pct(s().exposure, false)} sub={`${ratio(s().avgBars, 1)} bars a trade`} />
      <Figure label="Costs" value={money(-s().fees - s().funding)} sub={`fees ${money(-s().fees)} · funding ${money(-s().funding)}`} tone="bear" tip="Fees paid, and funding paid (or taken where it was positive for the position)" />
      <Show when={s().liquidations > 0}>
        <Figure label="Liquidated" value={String(s().liquidations)} sub="the whole margin lost each time" tone="bear" />
      </Show>
    </div>
  );
}

function Summary(props: TesterProps & { result: Ok; rows: TradeRow[]; stats: Stats | null }) {
  const r = () => props.result;
  const fees = () => (props.state.settings.autoFees ? venueFees(props.market) : [props.state.settings.taker, props.state.settings.maker]);
  // The part tuned on and the part kept back, where the optimizer's split is set.
  const split = createMemo(() => {
    const at = props.state.settings.split;
    const n = r().equity.length;
    if (!(at > 0) || n < 20) return null;
    const bar = Math.floor((n * at) / 100);
    return { bar, train: statsOf(r(), props.rows, 0, bar), test: statsOf(r(), props.rows, bar), at: r().time[bar]! };
  });
  const forward = createMemo(() => {
    const from = props.state.forwardFrom;
    if (from === null) return null;
    const bar = barOf(r(), from);
    return { bar, s: bar < r().equity.length ? statsOf(r(), props.rows, bar) : null };
  });
  const first = () => r().time[0] ?? 0;
  const last = () => r().time[r().time.length - 1] ?? 0;
  return (
    <div class="bt-summary">
      <Show when={props.stats}>{(s) => <Figures s={s()} />}</Show>
      <Show when={split()}>
        {(p) => (
          <div class="bt-split">
            <span class="dim">Tuned on the bars to {when(p().at)}, the rest kept back:</span>
            <span class="num">
              tuned part <b class={tone(p().train.net)}>{pct(p().train.netPct)}</b> · {p().train.trades} trades · PF {ratio(p().train.profitFactor)}
            </span>
            <span class="num">
              kept back <b class={tone(p().test.net)}>{pct(p().test.netPct)}</b> · {p().test.trades} trades · PF {ratio(p().test.profitFactor)}
            </span>
          </div>
        )}
      </Show>
      <div class="bt-forward">
        <Show
          when={forward()}
          fallback={
            <>
              <span class="dim">Forward test: the trades from now on, as bars close, reported apart from the history.</span>
              <button class="dbtn" onClick={() => props.onChange({ forwardFrom: Date.now() })}>
                Start a forward test
              </button>
            </>
          }
        >
          {(f) => (
            <>
              <span class="num">
                Forward since {when(props.state.forwardFrom!)}:{' '}
                <Show when={f().s} fallback={<span class="dim">no bar closed since</span>}>
                  {(s) => (
                    <>
                      <b class={tone(s().net)}>{pct(s().netPct)}</b> · {s().trades} trades · win {pct(s().winRate, false)} · DD {pct(-s().maxDdPct)}
                    </>
                  )}
                </Show>
              </span>
              <button class="dbtn" onClick={() => props.onChange({ forwardFrom: null })}>
                Stop
              </button>
            </>
          )}
        </Show>
      </div>
      <ul class="bt-notes dim">
        <li>
          {r().time.length.toLocaleString('en-US')} bars of {props.timeframe}, {when(first())} to {when(last())}
          {props.state.settings.days > 0 ? ` · trades from the last ${props.state.settings.days} days` : ''}
          {props.loading ? ' · loading more…' : ''}
        </li>
        <li>
          {props.state.settings.fill === 'close' ? 'Filled at the close of the bar an order is given on' : 'Filled at the next bar’s open, never on the bar that gave the order'} · fees {fees()[0]}% taker, {fees()[1]}% maker{props.state.settings.autoFees ? ` (${symbolOf(props.market)}’s exchange, first tier)` : ''} · slippage {props.state.settings.slippage}%
        </li>
        <li>
          <Show when={r().inner} fallback={props.state.settings.magnify ? `No shorter timeframe to look inside ${props.timeframe} bars: a bar that reached both a stop and a target is taken as stopped` : 'A bar that reached both a stop and a target is taken as stopped'}>
            {r().magnified > 0 || r().ambiguous > 0 ? `${r().magnified} bars that reached both a stop and a target read from their ${r().inner} bars` : `A bar that reaches both a stop and a target is read from its ${r().inner} bars: none did`}
          </Show>
          <Show when={r().ambiguous > 0}>
            {' '}
            · <span class="warn">{r().ambiguous} taken as stopped</span>, the worse of the two{r().reading ? ' (reading their lower bars…)' : ''}
          </Show>
        </li>
        <li>
          {r().funding === 'paid'
            ? `Funding paid at ${r().settlements.toLocaleString('en-US')} settlements`
            : r().funding === 'reading'
              ? 'Reading the funding history…'
              : r().funding === 'none'
                ? 'Funding not counted: this exchange’s history is not read'
                : r().funding === 'failed'
                  ? 'Funding not counted: its history could not be read'
                  : r().funding === 'spot'
                    ? 'Spot: no funding'
                    : 'Funding off'}
          {r().skipped > 0 ? ` · ${r().skipped} orders not filled (no stop to size by, or no equity left)` : ''}
        </li>
        <li>Worked out in this browser from the exchange’s bars. A backtest is the past: costs, fills and the market itself differ live.</li>
      </ul>
    </div>
  );
}

// ---------------------------------------------------------------- the trades

const MOST_ROWS = 500;

function Trades(props: { rows: TradeRow[]; result: Ok; precision: number; market: string; onShow(from: number, to: number): void }) {
  const [asc, setAsc] = createSignal(false);
  const shown = createMemo(() => {
    const list = asc() ? props.rows : [...props.rows].reverse();
    return list.slice(0, MOST_ROWS);
  });
  const price = (v: number) => (Number.isFinite(v) ? v.toLocaleString('en-US', { minimumFractionDigits: props.precision, maximumFractionDigits: props.precision }) : '—');
  const csv = () => {
    const blob = new Blob([tradesCsv(props.rows, props.market)], { type: 'text/csv' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${symbolOf(props.market)} ${props.result.timeframe} backtest trades.csv`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };
  const barMs = () => (props.result.time.length > 1 ? props.result.time[1]! - props.result.time[0]! : 3_600_000);
  return (
    <div class="bt-trades">
      <div class="bt-bar">
        <span class="dim">
          {props.rows.length} trades{props.rows.length > MOST_ROWS ? ` · the ${asc() ? 'oldest' : 'newest'} ${MOST_ROWS} shown` : ''} · click one to see it on the chart
        </span>
        <span class="spacer" />
        <button class="dbtn" onClick={() => setAsc((v) => !v)}>
          {asc() ? 'Oldest first' : 'Newest first'}
        </button>
        <button class="dbtn" disabled={props.rows.length === 0} onClick={csv}>
          Download CSV
        </button>
      </div>
      <div class="bt-table-wrap">
        <table class="bt-table num">
          <thead>
            <tr>
              <th>#</th>
              <th>Side</th>
              <th>Entry</th>
              <th>Exit</th>
              <th>Out by</th>
              <th>P&amp;L</th>
              <th>%</th>
              <th>R</th>
              <th data-tip="How far it went against it, and for it">MAE / MFE</th>
              <th>Bars</th>
            </tr>
          </thead>
          <tbody>
            <For each={shown()}>
              {(t) => (
                <tr onClick={() => props.onShow(t.entryTime - 20 * barMs(), (t.open ? Date.now() : t.exitTime) + 20 * barMs())}>
                  <td class="dim">{t.n}</td>
                  <td class={t.long ? 'bull' : 'bear'}>{t.long ? 'Long' : 'Short'}</td>
                  <td>
                    <span class="dim">{when(t.entryTime)}</span> {price(t.entryPrice)}
                  </td>
                  <td>
                    <Show when={!t.open} fallback={<span class="dim">open · {price(t.exitPrice)}</span>}>
                      <span class="dim">{when(t.exitTime)}</span> {price(t.exitPrice)}
                    </Show>
                  </td>
                  <td class={t.reason === 6 ? 'bear' : 'dim'}>{REASONS[t.reason] ?? ''}</td>
                  <td class={tone(t.pnl)}>{money(t.pnl)}</td>
                  <td class={tone(t.pnl)}>{pct(t.ret)}</td>
                  <td class={tone(t.pnl)}>{Number.isFinite(t.r) ? ratio(t.r) : '—'}</td>
                  <td>
                    <span class="bear">{pct(-t.mae)}</span> / <span class="bull">{pct(t.mfe)}</span>
                  </td>
                  <td class="dim">{t.exitBar - t.entryBar + 1}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
