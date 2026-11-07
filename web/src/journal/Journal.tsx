/**
 * The trading journal, over the workspace: every long or short position drawn
 * on a chart lands here as a trade, and price decides how it went (see
 * bins/server/src/journal.rs). Three tabs, as rsalianto's journal has them:
 * the dashboard of what the trades add up to, their history — each opened to
 * set its strategy and notes or close it by hand — and the weekly review. It
 * is kept for an account: until the browser signs in with Telegram, it shows
 * how, and nothing is written down.
 */
import { createMemo, createSignal, For, type JSX, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { Portal } from 'solid-js/web';
import { setDrawingsChanged, setJournalOpen } from '../app/ui';
import { AccountCard } from '../components/Account';
import { NumField } from '../components/NumField';
import { SymbolSearch } from '../components/SymbolSearch';
import { api, catalog, errorText, type AccountStatus } from '../lib/api';
import { readPref, writePref } from '../lib/prefs';
import { rememberSize } from '../lib/drawing-options';
import { defaultStyle } from '../lib/drawing-style';
import { formatPrice } from '../lib/format';
import { VenueTag } from '../lib/exchanges';
import { Icon } from '../lib/icons';
import { isMarket, symbolOf, venueLabel } from '../lib/markets';
import { focusPanel, marketOf, setPanelMarket, setPanelTimeframe, targetChart } from '../workspace/store';
import { smoothPath } from './curve';
import { cents, coinsFor, figures, figuresOf, fits, lossPerCoin, marginFor, type DrawnLeg } from './legs';
import { Readout } from './Readout';
import { StrategyPicker } from './strategies';
import {
  book,
  closedPnl,
  dailyPnl,
  FILL_LABEL,
  equityCurve,
  groupBy,
  inWeek,
  isClosed,
  isLive,
  journalApi,
  lastWeeks,
  localDay,
  localInput,
  money,
  niceTicks,
  outcome,
  plannedRR,
  pnl,
  QUESTIONS,
  rMultiple,
  rText,
  stamp,
  stats,
  STATUS_LABEL,
  weekOf,
  when,
  type AnswerKey,
  type Group,
  type Review,
  type Side,
  type Status,
  type Trade,
  type TradePatch,
} from './model';

type Tab = 'dashboard' | 'history' | 'review';
const TABS: [Tab, string][] = [
  ['dashboard', 'Dashboard'],
  ['history', 'History'],
  ['review', 'Review'],
];
/** The server weighs trades against price every minute; the page asks again twice in that. */
const REFRESH_MS = 30_000;

const tone = (v: number | null): string => (v === null || v === 0 ? '' : v > 0 ? 'bull' : 'bear');
/** Fills that put coins in; the rest take them out. */
const IN = new Set(['entry', 'add']);

export function Journal() {
  const [trades, setTrades] = createSignal<Trade[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [tab, setTab] = createSignal<Tab>('dashboard');
  const [focus, setFocus] = createSignal<Focus | null>(null);
  let body: HTMLDivElement | undefined;
  /** One row of the breakdown's trades, in History, from its top. */
  const openFocus = (f: Focus) => {
    setFocus(f);
    setTab('history');
    body?.scrollTo({ top: 0 });
  };
  const [selected, setSelected] = createSignal<number | null>(null);
  const [adding, setAdding] = createSignal(false);
  const [decimals, setDecimals] = createSignal(new Map<string, number>());
  const [account, setAccount] = createSignal<AccountStatus | null>(null);
  const signedIn = () => !!account()?.account;

  const load = async () => {
    if (!signedIn()) return;
    try {
      setTrades(await journalApi.trades());
      setError(null);
      // Only a list actually read can say there are no trades.
      setLoaded(true);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const readAccount = async () => {
    try {
      setAccount(await api.account());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
    await load();
  };
  /** Whatever is still to be read: the account while that is unknown, then the trades. */
  const refresh = () => (account() ? load() : readAccount());
  onMount(() => {
    void readAccount();
    // Asked again only while the page is in view, and either a first read has
    // yet to answer or price still has a trade to decide; back in view, at once.
    const timer = setInterval(() => {
      if (!document.hidden && (!account() || (signedIn() && !loaded()) || trades().some((t) => t.auto && t.tracked && isLive(t)))) void refresh();
    }, REFRESH_MS);
    const shown = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener('visibilitychange', shown);
    onCleanup(() => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', shown);
    });
    // Each market's tick, so a dragged anchor's 81,234.56789 reads as the exchange quotes it.
    void catalog()
      .then((c) => setDecimals(new Map(c.products.flatMap((p) => (p.priceDecimals === null ? [] : [[p.market, p.priceDecimals] as [string, number]])))))
      .catch(() => {});
  });
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    if (adding()) setAdding(false);
    else if (selected() !== null) setSelected(null);
    else setJournalOpen(false);
  };
  onMount(() => {
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  const price = (market: string, v: number | null) => formatPrice(v, decimals().get(market));
  const replace = (t: Trade) => setTrades((list) => list.map((x) => (x.id === t.id ? t : x)));
  const current = () => trades().find((t) => t.id === selected());
  const waiting = () => <p class="jr-empty muted">{error() ? 'Not read yet. Asking again shortly…' : 'Reading…'}</p>;

  return (
    <section class="journal">
      <header class="jr-head">
        <span class="jr-title">Journal</span>
        <Show when={signedIn()}>
          <nav class="chips jr-tabs">
            <For each={TABS}>
              {([id, label]) => (
                <button classList={{ on: tab() === id }} onClick={() => setTab(id)}>
                  {label}
                </button>
              )}
            </For>
          </nav>
        </Show>
        <span class="grow" />
        <Show when={signedIn()}>
          <button class="tb" data-tip="Add a trade by hand" onClick={() => setAdding(true)}>
            <Icon name="plus" size={12} />
            <span>Trade</span>
          </button>
        </Show>
        <button class="iconbtn" aria-label="Close the journal" data-tip="Back to the charts (Esc)" onClick={() => setJournalOpen(false)}>
          <Icon name="close" size={15} />
        </button>
      </header>
      <Show when={error()}>
        {(e) => (
          <div class="banner jr-fail">
            <span>▲ {e()}</span>
            <button class="tb" onClick={() => void refresh()}>
              Try again
            </button>
          </div>
        )}
      </Show>
      <div class="jr-body" ref={body}>
        <Show when={account()} fallback={waiting()}>
          <Show when={signedIn()} fallback={<SignInFirst onSignedIn={() => void readAccount()} />}>
            <Show when={!loaded() && tab() !== 'review'}>{waiting()}</Show>
            <Show when={loaded() && trades().length === 0 && tab() !== 'review'}>
              <div class="jr-empty">
                <p>No trades yet.</p>
                <p class="muted">
                  Draw a Long or Short position on a chart (Alt+L, Alt+S): it lands here, and price decides how it went — open at the entry, then the target or the stop. Or
                  add one by hand.
                </p>
              </div>
            </Show>
            <Switch>
              <Match when={tab() === 'dashboard' && trades().length > 0}>
                <Dashboard trades={trades()} price={price} onSelect={setSelected} onAll={() => setTab('history')} onFocus={openFocus} />
              </Match>
              <Match when={tab() === 'history' && trades().length > 0}>
                <History trades={trades()} price={price} onSelect={setSelected} focus={focus()} onUnfocus={() => setFocus(null)} />
              </Match>
              <Match when={tab() === 'review'}>
                <ReviewTab trades={trades()} />
              </Match>
            </Switch>
          </Show>
        </Show>
      </div>
      <Show when={current()}>
        {(t) => (
          <Editor
            trade={t()}
            price={price}
            onClose={() => setSelected(null)}
            onChange={replace}
            onRemove={(id) => {
              setTrades((list) => list.filter((x) => x.id !== id));
              setSelected(null);
            }}
          />
        )}
      </Show>
      <Show when={adding()}>
        <AddTrade
          onClose={() => setAdding(false)}
          onAdd={(t) => {
            setTrades((list) => [t, ...list]);
            setAdding(false);
          }}
        />
      </Show>
    </section>
  );
}

/** Before the journal opens, signing in: it is kept for a Telegram account. */
function SignInFirst(props: { onSignedIn(): void }) {
  return (
    <div class="jr-gate">
      <h2>Sign in with Telegram to keep a journal</h2>
      <p class="muted">
        Once you are signed in, every Long or Short position you draw on a chart lands here, and price decides how it went — open at the entry, then the target or the stop.
        The journal is kept with your account, so it follows you to any device you sign in on.
      </p>
      <AccountCard onSignedIn={props.onSignedIn} />
    </div>
  );
}

// ---------------------------------------------------------------- dashboard

const PERIODS: [string, string][] = [
  ['all', 'All'],
  ['7', '7D'],
  ['30', '30D'],
  ['90', '90D'],
];

/**
 * What the trades add up to, the one number first: the net, what stands behind
 * it, the curve that got there, then where it was made and the latest trades.
 * The period above scopes all of it.
 */
function Dashboard(props: { trades: Trade[]; price: (market: string, v: number | null) => string; onSelect: (id: number) => void; onAll: () => void; onFocus: (f: Focus) => void }) {
  const [period, setPeriod] = createSignal('all');
  const list = createMemo(() => {
    const p = period();
    if (p === 'all') return props.trades;
    const from = Date.now() - Number(p) * 86_400_000;
    return props.trades.filter((t) => when(t) >= from);
  });
  const s = createMemo(() => stats(list()));
  const latest = createMemo(() =>
    list()
      .filter((t) => isClosed(t))
      .sort((a, b) => when(b) - when(a))
      .slice(0, 6),
  );
  const share = (k: 'up' | 'down' | 'even') => (k === 'up' ? s().wins : k === 'down' ? s().losses : s().breakEven);
  const streak = () => (s().streak > 0 ? `${s().streak} ${s().streakKind === 'win' ? (s().streak > 1 ? 'wins' : 'win') : s().streak > 1 ? 'losses' : 'loss'}` : '—');
  return (
    <div class="jx">
      <div class="jx-bar">
        <div class="jh-sides">
          <For each={PERIODS}>
            {([id, label]) => (
              <button classList={{ on: period() === id }} onClick={() => setPeriod(id)}>
                {label}
              </button>
            )}
          </For>
        </div>
        <span class="dim num">
          {s().closed} closed · {s().open} open · {s().planned} planned
        </span>
      </div>
      <Show when={s().closed > 0} fallback={<p class="jh-none dim">Nothing closed in this period.</p>}>
        <section class="jx-summary">
          <div class="jx-hero">
            <span class="jx-cap">Net P&L</span>
            <span class={`jx-net ${tone(s().totalPnl)}`}>{money(s().totalPnl)}</span>
            <span class="muted num">
              {rText(s().totalR)} over {s().closed} trade{s().closed === 1 ? '' : 's'}
            </span>
          </div>
          <div class="jx-kpi" data-tip="Trades that made money, of those closed">
            <span class="jx-cap">Win rate</span>
            <span class="jx-value">{s().winRate.toFixed(0)}%</span>
            <div class="jx-meter" role="img" aria-label={`${s().wins} won, ${s().losses} lost, ${s().breakEven} even`}>
              <For each={(['up', 'down', 'even'] as const).filter((k) => share(k) > 0)}>{(k) => <i class={k} style={{ 'flex-grow': share(k) }} />}</For>
            </div>
            <span class="dim num">
              {s().wins} won · {s().losses} lost · {s().breakEven} even
            </span>
          </div>
          <div class="jx-kpi" data-tip="What the winners made for each dollar the losers lost; above 1 is making money">
            <span class="jx-cap">Profit factor</span>
            <span class="jx-value">{s().profitFactor === null ? '—' : s().profitFactor!.toFixed(2)}</span>
            <span class="dim num">
              {money(s().grossProfit)} won · {money(-s().grossLoss)} lost
            </span>
          </div>
          <div class="jx-kpi" data-tip="What a trade makes on average, in dollars and in what it risked">
            <span class="jx-cap">Average trade</span>
            <span class={`jx-value ${tone(s().expectancy)}`}>{money(s().expectancy)}</span>
            <span class="dim num">{rText(s().avgR)} a trade</span>
          </div>
          <div class="jx-kpi" data-tip="The deepest the running total has fallen from a high">
            <span class="jx-cap">Max drawdown</span>
            <span class={`jx-value ${s().maxDrawdown > 0 ? 'bear' : ''}`}>{money(-s().maxDrawdown)}</span>
            <span class="dim">from a high</span>
          </div>
        </section>

        <dl class="jx-facts num">
          <div>
            <dt>Best trade</dt>
            <dd class={tone(s().best)}>{money(s().best)}</dd>
          </div>
          <div>
            <dt>Worst trade</dt>
            <dd class={tone(s().worst)}>{money(s().worst)}</dd>
          </div>
          <div>
            <dt>Average win</dt>
            <dd>{money(s().avgWin)}</dd>
          </div>
          <div>
            <dt>Average loss</dt>
            <dd>{money(-s().avgLoss)}</dd>
          </div>
          <div>
            <dt>Streak now</dt>
            <dd class={s().streakKind === 'win' ? 'bull' : s().streakKind === 'loss' ? 'bear' : ''}>{streak()}</dd>
          </div>
          <div>
            <dt>Average hold</dt>
            <dd>{s().avgHoldHours === null ? '—' : hold(s().avgHoldHours!)}</dd>
          </div>
        </dl>

        <section class="jx-box">
          <h3>Equity</h3>
          <Equity trades={list()} />
        </section>

        <div class="jx-two">
          <section class="jx-box">
            <h3>P&L calendar</h3>
            <Calendar trades={list()} />
          </section>
          <section class="jx-box">
            <Breakdown trades={list()} onFocus={props.onFocus} />
          </section>
        </div>

        <section class="jx-box jx-latest">
          <header class="jx-head">
            <h3>Latest trades</h3>
            <button class="jx-link" onClick={props.onAll}>
              All trades
            </button>
          </header>
          <For each={latest()}>{(t) => <Row trade={t} price={props.price} onSelect={props.onSelect} dated />}</For>
        </section>
      </Show>
    </div>
  );
}

const hold = (hours: number): string => (hours < 1 ? `${Math.round(hours * 60)}m` : hours < 48 ? `${hours.toFixed(1)}h` : `${(hours / 24).toFixed(1)}d`);

function Card(props: { label: string; value: string; sub?: string; tone?: string }) {
  return (
    <div class="jr-card">
      <span class="cap">{props.label}</span>
      <span class={`jr-value num ${props.tone ?? ''}`}>{props.value}</span>
      <Show when={props.sub}>
        <span class="dim num">{props.sub}</span>
      </Show>
    </div>
  );
}

/** `$1.2K`, `−$85`: a figure on an axis, short. */
function axisMoney(v: number): string {
  const a = Math.abs(v);
  return `${v < 0 ? '−' : ''}$${a >= 1000 ? `${Number((a / 1000).toFixed(1))}K` : Number(a.toFixed(2))}`;
}

const dayMonth = (ms: number): string => {
  const d = new Date(ms);
  return `${MONTHS[d.getMonth()]!.slice(0, 3)} ${d.getDate()}`;
};

/** The running total of closed trades from nothing, over time, each point read under the pointer or with the arrow keys. */
function Equity(props: { trades: Trade[] }) {
  const curve = createMemo(() => equityCurve(props.trades));
  const shape = createMemo(() => {
    const c = curve();
    const first = c[0]!;
    const from = Math.min(first.trade.openedAt ?? first.trade.startAt, first.at);
    const to = c[c.length - 1]!.at;
    const ticks = niceTicks(Math.min(0, ...c.map((p) => p.total)), Math.max(0, ...c.map((p) => p.total)));
    const [lo, hi] = [ticks[0]!, ticks[ticks.length - 1]!];
    const y = (v: number) => (hi > lo ? 100 - ((v - lo) / (hi - lo)) * 100 : 50);
    // All in the one minute: evenly, for want of a span.
    const x = (at: number, i: number) => (to > from ? ((at - from) / (to - from)) * 100 : ((i + 1) / c.length) * 100);
    const points = c.map((p, i) => ({ x: x(p.at, i), y: y(p.total) }));
    const line = smoothPath([{ x: 0, y: y(0) }, ...points]);
    const labels = to - from < 36 * 3_600_000 ? (at: number) => stamp(at).slice(6) : dayMonth;
    return { zero: y(0), ticks: ticks.map((v) => ({ v, y: y(v) })), points, line, area: `${line}L100,${y(0)}L0,${y(0)}Z`, last: c[c.length - 1]!.total, days: [0, 1, 2, 3].map((i) => ({ x: (i / 3) * 100, label: labels(from + ((to - from) * i) / 3) })) };
  });
  const [hover, setHover] = createSignal<number | null>(null);
  const nearest = (e: PointerEvent & { currentTarget: HTMLElement }) => {
    const r = e.currentTarget.getBoundingClientRect();
    const at = ((e.clientX - r.left) / r.width) * 100;
    let best = 0;
    shape().points.forEach((p, i) => {
      if (Math.abs(p.x - at) < Math.abs(shape().points[best]!.x - at)) best = i;
    });
    setHover(best);
  };
  const keys = (e: KeyboardEvent) => {
    const n = shape().points.length;
    if (e.key === 'ArrowLeft') setHover((h) => Math.max(0, (h ?? n) - 1));
    else if (e.key === 'ArrowRight') setHover((h) => Math.min(n - 1, (h ?? -1) + 1));
    else return;
    e.preventDefault();
  };
  const at = () => {
    const i = hover();
    return i === null ? null : { ...shape().points[i]!, ...curve()[i]! };
  };
  return (
    <div class="jx-eq">
      <div class="jx-eq-y num">
        <For each={shape().ticks}>{(t) => <span style={{ top: `${t.y}%` }}>{axisMoney(t.v)}</span>}</For>
      </div>
      <div class="jx-eq-plot" tabIndex={0} role="img" aria-label={`Equity: ${money(shape().last)}. Arrow keys read each trade.`} onPointerMove={nearest} onPointerLeave={() => setHover(null)} onBlur={() => setHover(null)} onKeyDown={keys}>
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
          <For each={shape().ticks}>{(t) => <line x1="0" x2="100" y1={t.y} y2={t.y} class={t.v === 0 ? 'jx-zero' : 'jx-gridline'} vector-effect="non-scaling-stroke" />}</For>
          {/* The same curve twice, each cut at zero: green above it, red below. */}
          <clipPath id="jx-above">
            <rect x="-1" y="-1" width="102" height={shape().zero + 1} />
          </clipPath>
          <clipPath id="jx-below">
            <rect x="-1" y={shape().zero} width="102" height={101 - shape().zero} />
          </clipPath>
          <g clip-path="url(#jx-above)">
            <path d={shape().area} class="jx-area up" />
            <path d={shape().line} class="jx-line up" vector-effect="non-scaling-stroke" />
          </g>
          <g clip-path="url(#jx-below)">
            <path d={shape().area} class="jx-area down" />
            <path d={shape().line} class="jx-line down" vector-effect="non-scaling-stroke" />
          </g>
        </svg>
        <Show when={!at()}>
          <i class={`jx-dot ${shape().last >= 0 ? 'up' : 'down'}`} style={{ left: `${shape().points[shape().points.length - 1]!.x}%`, top: `${shape().points[shape().points.length - 1]!.y}%` }} />
        </Show>
        <Show when={at()}>
          {(p) => (
            <>
              <i class="jx-cross" style={{ left: `${p().x}%` }} />
              <i class={`jx-dot ${p().total >= 0 ? 'up' : 'down'}`} style={{ left: `${p().x}%`, top: `${p().y}%` }} />
              <div class="jx-tip num" classList={{ flip: p().x > 60 }} style={{ left: `${p().x}%` }}>
                <b class={tone(p().total)}>{money(p().total)}</b>
                <span class="dim">{stamp(p().at)}</span>
                <span class="muted">
                  {symbolOf(p().trade.market)} {p().trade.side === 'long' ? 'Long' : 'Short'} {money(pnl(p().trade))}
                </span>
              </div>
            </>
          )}
        </Show>
      </div>
      <div class="jx-eq-x num">
        <For each={shape().days}>{(d) => <span style={{ left: `${d.x}%` }}>{d.label}</span>}</For>
      </div>
    </div>
  );
}

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

const CAL_UNIT_KEY = 'journal-calendar-unit';

/** A month of days, each with what its closed trades made and each week's beside it, in money or in R. */
function Calendar(props: { trades: Trade[] }) {
  const [unit, setUnit] = createSignal<'usd' | 'r'>(readPref(CAL_UNIT_KEY) === 'r' ? 'r' : 'usd');
  const pick = (u: 'usd' | 'r') => {
    setUnit(u);
    writePref(CAL_UNIT_KEY, u);
  };
  /** What a day or a week shows: its money, or its R. */
  const measure = (d: { pnl: number; r: number }) => (unit() === 'r' ? d.r : d.pnl);
  const figure = (v: number, short: boolean) => (unit() === 'r' ? rText(v) : short ? compactMoney(v) : money(v));
  // It opens on the last month anything closed in, not on an empty one.
  const last = new Date(Math.max(0, ...props.trades.filter(isClosed).map(when)) || Date.now());
  const [month, setMonth] = createSignal({ y: last.getFullYear(), m: last.getMonth() });
  const days = createMemo(() => dailyPnl(props.trades));
  const cells = createMemo(() => {
    const { y, m } = month();
    const first = new Date(y, m, 1);
    const lead = (first.getDay() + 6) % 7;
    const count = new Date(y, m + 1, 0).getDate();
    const days = [...Array<null>(lead).fill(null), ...Array.from({ length: count }, (_, i) => localDay(new Date(y, m, i + 1).getTime()))];
    return [...days, ...Array<null>((7 - (days.length % 7)) % 7).fill(null)];
  });
  /** Monday to Sunday, a row each: the days of this month in it, and their R together. */
  const weeks = createMemo(() =>
    Array.from({ length: cells().length / 7 }, (_, i) => {
      const row = cells().slice(i * 7, i * 7 + 7);
      const traded = row.map((day) => (day ? days().get(day) : undefined)).filter((d) => d !== undefined);
      return { row, r: traded.reduce((sum, d) => sum + d.r, 0), pnl: traded.reduce((sum, d) => sum + d.pnl, 0), trades: traded.reduce((sum, d) => sum + d.trades, 0), first: row.find((day) => day !== null)! };
    }),
  );
  const inMonth = createMemo(() => cells().reduce((sum, day) => {
    const d = day ? days().get(day) : undefined;
    return d ? { pnl: sum.pnl + d.pnl, r: sum.r + d.r } : sum;
  }, { pnl: 0, r: 0 }));
  const total = () => measure(inMonth());
  // The shade is by what is shown: the day that made the most in R need not be the one that made the most money.
  const biggest = createMemo(() => Math.max(unit() === 'r' ? 0.01 : 1, ...cells().map((day) => Math.abs(day && days().get(day) ? measure(days().get(day)!) : 0))));
  const step = (n: number) => setMonth(({ y, m }) => ({ y: m + n < 0 ? y - 1 : m + n > 11 ? y + 1 : y, m: (m + n + 12) % 12 }));
  const now = localDay(Date.now());
  return (
    <div class="jx-cal">
      <div class="jx-cal-head">
        <button class="iconbtn" aria-label="Previous month" onClick={() => step(-1)}>
          ‹
        </button>
        <span class="jx-cal-month">
          {MONTHS[month().m]} {month().y}
        </span>
        <button class="iconbtn" aria-label="Next month" onClick={() => step(1)}>
          ›
        </button>
        <span class="grow" />
        <span class={`num ${tone(total())}`}>{figure(total(), false)}</span>
        <div class="jh-sides jx-unit">
          <button classList={{ on: unit() === 'usd' }} aria-pressed={unit() === 'usd'} data-tip="Each day in money" onClick={() => pick('usd')}>
            $
          </button>
          <button classList={{ on: unit() === 'r' }} aria-pressed={unit() === 'r'} data-tip="Each day in R: what it made against what it risked" onClick={() => pick('r')}>
            R
          </button>
        </div>
      </div>
      <div class="jx-cal-grid">
        <For each={WEEKDAYS}>{(d) => <span class="jx-wd">{d}</span>}</For>
        <span class="jx-wd">Week</span>
        <For each={weeks()}>
          {(week) => (
            <>
              <For each={week.row}>
                {(day) => {
                  const d = () => (day ? days().get(day) : undefined);
                  const strength = () => Math.min(1, Math.abs(d() ? measure(d()!) : 0) / biggest());
                  return (
                    <div
                      class="jx-day"
                      classList={{ blank: !day, up: (d() ? measure(d()!) : 0) > 0, down: (d() ? measure(d()!) : 0) < 0, today: day === now }}
                      style={{ '--s': String(0.1 + strength() * 0.22) }}
                      data-tip={d() ? `${day}: ${money(d()!.pnl)} · ${rText(d()!.r)} from ${d()!.trades} trade${d()!.trades > 1 ? 's' : ''}` : undefined}
                    >
                      <Show when={day}>
                        <span class="jx-date">{Number(day!.slice(8))}</span>
                        <Show when={d()}>
                          <span class={`num ${tone(measure(d()!))}`}>{figure(measure(d()!), true)}</span>
                        </Show>
                      </Show>
                    </div>
                  );
                }}
              </For>
              <div class={`jx-week num ${week.trades > 0 ? tone(measure(week)) : 'dim'}`} data-tip={week.trades > 0 ? `Week of ${week.first}: ${money(week.pnl)} · ${rText(week.r)} from ${week.trades} trade${week.trades > 1 ? 's' : ''}` : undefined}>
                {week.trades > 0 ? figure(measure(week), true) : '—'}
              </div>
            </>
          )}
        </For>
      </div>
    </div>
  );
}

/** `+1.2K`, `−85`: a calendar day's figure, short. */
function compactMoney(v: number): string {
  const sign = v > 0 ? '+' : v < 0 ? '−' : '';
  const a = Math.abs(v);
  return `${sign}${a >= 1000 ? `${(a / 1000).toFixed(1)}K` : a.toFixed(a >= 100 ? 0 : 1)}`;
}

type BreakdownId = 'pair' | 'side' | 'strategy';
/** History narrowed to one row of the breakdown: one pair's trades, one side's, one strategy's. */
interface Focus {
  by: BreakdownId;
  value: string;
}

const BREAKDOWNS: { id: BreakdownId; label: string; key: (t: Trade) => string }[] = [
  { id: 'pair', label: 'Pair', key: (t) => symbolOf(t.market) },
  { id: 'side', label: 'Side', key: (t) => (t.side === 'long' ? 'Long' : 'Short') },
  { id: 'strategy', label: 'Strategy', key: (t) => t.strategy || 'No strategy' },
];

/** Where it was made: closed trades by pair, side or strategy, each one's net as a bar from zero, each opening its trades. */
function Breakdown(props: { trades: Trade[]; onFocus: (f: Focus) => void }) {
  const [by, setBy] = createSignal(BREAKDOWNS[0]!);
  const groups = createMemo(() => groupBy(props.trades, by().key));
  const open = (key: string) => props.onFocus({ by: by().id, value: key });
  const scale = createMemo(() => {
    const neg = Math.max(0, ...groups().map((g) => -g.pnl));
    const pos = Math.max(0, ...groups().map((g) => g.pnl));
    return { neg, span: neg + pos || 1 };
  });
  return (
    <>
      <header class="jx-head">
        <h3>Where it was made</h3>
        <div class="jh-sides">
          <For each={BREAKDOWNS}>
            {(b) => (
              <button classList={{ on: by().id === b.id }} onClick={() => setBy(b)}>
                {b.label}
              </button>
            )}
          </For>
        </div>
      </header>
      <div class="jx-scroll">
      <table class="jx-table num">
        <thead>
          <tr>
            <th />
            <th>Trades</th>
            <th>Win</th>
            <th class="wide" />
            <th>P&L</th>
          </tr>
        </thead>
        <tbody>
          <For each={groups()}>
            {(g) => (
              <tr
                class="go"
                tabIndex={0}
                data-tip={`Every ${g.key} trade, in History`}
                onClick={() => open(g.key)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    open(g.key);
                  }
                }}
              >
                <td class="name">{g.key}</td>
                <td>{g.trades}</td>
                <td>{g.winRate.toFixed(0)}%</td>
                <td class="wide">
                  <div class="jx-track" style={{ '--zero': `${(scale().neg / scale().span) * 100}%` }}>
                    <i class={g.pnl >= 0 ? 'up' : 'down'} style={{ left: `${((scale().neg + Math.min(0, g.pnl)) / scale().span) * 100}%`, width: `${(Math.abs(g.pnl) / scale().span) * 100}%` }} />
                  </div>
                </td>
                <td class={tone(g.pnl)}>{money(g.pnl)}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
      </div>
    </>
  );
}

// ---------------------------------------------------------------- history

type View = 'all' | 'open' | 'win' | 'loss' | 'even';
const VIEWS: [View, string][] = [
  ['all', 'All'],
  ['open', 'Open'],
  ['win', 'Won'],
  ['loss', 'Lost'],
  ['even', 'Break-even'],
];
const WEEKDAY_NAMES = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

/** `Today`, `Yesterday`, else `Tuesday 28 October`, with the year when it is not this one. */
function dayName(day: string): string {
  if (day === localDay(Date.now())) return 'Today';
  if (day === localDay(Date.now() - 86_400_000)) return 'Yesterday';
  const [y, m, d] = day.split('-').map(Number) as [number, number, number];
  return `${WEEKDAY_NAMES[new Date(y, m - 1, d).getDay()]} ${d} ${MONTHS[m - 1]}${y === new Date().getFullYear() ? '' : ` ${y}`}`;
}

/** `14:05` in this browser's time. */
const clock = (ms: number): string => stamp(ms).slice(6);

/**
 * Every trade, as a statement reads: those still to play out on top, then the
 * rest by the day each closed — the latest first — with what the day made.
 */
function History(props: { trades: Trade[]; price: (market: string, v: number | null) => string; onSelect: (id: number) => void; focus: Focus | null; onUnfocus: () => void }) {
  const [view, setView] = createSignal<View>('all');
  const [side, setSide] = createSignal<'all' | Side>('all');
  const [query, setQuery] = createSignal('');
  const focused = createMemo(() => {
    const f = props.focus;
    const key = f && BREAKDOWNS.find((b) => b.id === f.by)?.key;
    return f && key ? { label: f.value, by: BREAKDOWNS.find((b) => b.id === f.by)!.label, test: (t: Trade) => key(t) === f.value } : null;
  });
  // The focus, the side and the search narrow everything, the counts too; the view picks among what is left.
  const pool = createMemo(() => {
    const q = query().trim().toUpperCase();
    const f = focused();
    return props.trades.filter((t) => (!f || f.test(t)) && (side() === 'all' || t.side === side()) && (!q || t.market.toUpperCase().includes(q) || t.strategy.toUpperCase().includes(q) || t.notes.toUpperCase().includes(q)));
  });
  const counts = createMemo(() => {
    const c: Record<View, number> = { all: 0, open: 0, win: 0, loss: 0, even: 0 };
    for (const t of pool()) {
      const o = outcome(t);
      c.all++;
      if (o === 'open' || o === 'planned') c.open++;
      else if (o !== 'cancelled') c[o]++;
    }
    return c;
  });
  const live = createMemo(() =>
    view() === 'all' || view() === 'open' ? pool().filter(isLive).sort((a, b) => (b.openedAt ?? b.startAt) - (a.openedAt ?? a.startAt)) : [],
  );
  const days = createMemo(() => {
    if (view() === 'open') return [];
    const byDay = new Map<string, Trade[]>();
    for (const t of pool()) {
      if (isLive(t) || (view() !== 'all' && outcome(t) !== view())) continue;
      const day = localDay(when(t));
      const list = byDay.get(day);
      if (list) list.push(t);
      else byDay.set(day, [t]);
    }
    return [...byDay.entries()]
      .sort(([a], [b]) => (a < b ? 1 : -1))
      .map(([day, list]) => ({ day, trades: list.sort((a, b) => when(b) - when(a)), pnl: list.reduce((s, t) => s + (pnl(t) ?? 0), 0), r: list.reduce((s, t) => s + (rMultiple(t) ?? 0), 0) }));
  });
  const many = (n: number) => `${n} trade${n === 1 ? '' : 's'}`;
  return (
    <div class="jh">
      <div class="jh-bar">
        <div class="jh-views">
          <For each={VIEWS}>
            {([id, label]) => (
              <button classList={{ on: view() === id, [id]: true }} onClick={() => setView(id)}>
                {label}
                <span class="jh-count num">{counts()[id]}</span>
              </button>
            )}
          </For>
        </div>
        <div class="jh-sides">
          <For each={['all', 'long', 'short'] as const}>
            {(id) => (
              <button classList={{ on: side() === id }} onClick={() => setSide(id)}>
                {id === 'all' ? 'Both' : id === 'long' ? 'Long' : 'Short'}
              </button>
            )}
          </For>
        </div>
        <Show when={focused()}>
          {(f) => (
            <button class="jh-focus" aria-label={`Show every trade, not only ${f().label}`} data-tip={`Only ${f().by.toLowerCase()} ${f().label}: show every trade`} onClick={() => props.onUnfocus()}>
              <span class="dim">{f().by}</span> {f().label}
              <Icon name="close" size={11} />
            </button>
          )}
        </Show>
        <label class="jh-search">
          <Icon name="search" size={12} />
          <input placeholder="Search market, strategy, notes" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
        </label>
      </div>
      <div class="jh-list">
        <Show when={live().length > 0}>
          <section class="jh-day">
            <header class="jh-dayhead">
              <span class="jh-dayname">Open &amp; planned</span>
              <span class="grow" />
              <span class="dim">{many(live().length)}</span>
            </header>
            <For each={live()}>{(t) => <Row trade={t} price={props.price} onSelect={props.onSelect} />}</For>
          </section>
        </Show>
        <For each={days()}>
          {(d) => (
            <section class="jh-day">
              <header class="jh-dayhead">
                <span class="jh-dayname">{dayName(d.day)}</span>
                <span class="grow" />
                <span class="dim">{many(d.trades.length)}</span>
                <span class={`num ${tone(d.r)}`}>{rText(d.r)}</span>
                <span class={`num jh-daypnl ${tone(d.pnl)}`}>{money(d.pnl)}</span>
              </header>
              <For each={d.trades}>{(t) => <Row trade={t} price={props.price} onSelect={props.onSelect} />}</For>
            </section>
          )}
        </For>
        <Show when={live().length === 0 && days().length === 0}>
          <p class="jh-none dim">No trades match.</p>
        </Show>
      </div>
    </div>
  );
}

/** One trade on a line: when, which way, where, in and out, how it went. */
function Row(props: { trade: Trade; price: (market: string, v: number | null) => string; onSelect: (id: number) => void; dated?: boolean }) {
  const t = () => props.trade;
  const done = () => !isLive(t());
  const p = (v: number | null) => props.price(t().market, v);
  return (
    <button class={`jh-row ${outcome(t())}`} onClick={() => props.onSelect(t().id)}>
      <span class="jh-time num dim">{(props.dated ? stamp : clock)(done() ? when(t()) : (t().openedAt ?? t().startAt))}</span>
      <span class={`jh-side ${t().side}`}>{t().side === 'long' ? 'Long' : 'Short'}</span>
      <span class="jh-market">
        <b>{symbolOf(t().market)}</b>
        <span class="dim">{venueLabel(t().market)}</span>
      </span>
      <span class="jh-prices num">
        <Show
          when={done()}
          fallback={
            <>
              {p(t().entry)} <span class="dim">SL</span> {p(t().stop)} <span class="dim">TP</span> {p(t().target)}
            </>
          }
        >
          {p(t().entry)} <span class="dim">→</span> {p(t().exit)}
        </Show>
      </span>
      <span class="jh-strategy dim">{t().strategy}</span>
      <Badge trade={t()} />
      <span class="jh-r num">{done() ? rText(rMultiple(t())) : <span class="dim">{plannedRR(t()).toFixed(2)} RR</span>}</span>
      <span class="jh-pnl num">{done() ? money(pnl(t())) : ''}</span>
    </button>
  );
}

/** A trade's status in its outcome's colour: won green, lost red, even orange. */
function Badge(props: { trade: Trade }) {
  const tip = () => (props.trade.auto ? (props.trade.tracked ? 'Price decides this one' : 'Kept by hand: this market’s prices are not read here') : 'Set by hand');
  return (
    <span class={`jh-badge ${outcome(props.trade)}`} data-tip={tip()}>
      {STATUS_LABEL[props.trade.status]}
    </span>
  );
}

// ---------------------------------------------------------------- one trade

/** Who says how a trade went: price, or the hand — closed at a price, evened or called off. */
type Mode = 'auto' | 'closed' | 'breakeven' | 'cancelled';
const modeOf = (t: Trade): Mode => (t.auto ? 'auto' : t.status === 'breakeven' ? 'breakeven' : t.status === 'cancelled' ? 'cancelled' : 'closed');
/** The minute a moment falls in: what a datetime-local input gives back. */
const minute = (ms: number): number => Math.floor(ms / 60_000) * 60_000;

/** `6h 07m`, `3d 4h`, `25m`. */
function held(ms: number): string {
  const m = Math.max(0, Math.round(ms / 60_000));
  if (m < 60) return `${m}m`;
  if (m < 1440) return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(m / 1440)}d ${Math.floor((m % 1440) / 60)}h`;
}

/** A price typed in a form, with what it comes to under it. */
function PriceField(props: { label: string; value: string; onInput: (v: string) => void; hint?: string; tone?: string; required?: boolean }) {
  return (
    <label class="jd-field">
      <span class="jd-label">{props.label}</span>
      <input class="jr-in num" inputmode="decimal" required={props.required} value={props.value} onInput={(e) => props.onInput(e.currentTarget.value)} />
      <Show when={props.hint}>
        <span class={`jd-hint num ${props.tone ?? 'dim'}`}>{props.hint}</span>
      </Show>
    </label>
  );
}

/** How far a price is from the entry. */
function fromEntry(price: string, entry: string): string | undefined {
  const [p, e] = [Number(price), Number(entry)];
  if (!(p > 0 && e > 0)) return undefined;
  const pct = ((p - e) / e) * 100;
  return `${pct > 0 ? '+' : pct < 0 ? '−' : ''}${Math.abs(pct).toFixed(2)}%`;
}

/**
 * A trade's size, in the form's own terms: its margin and leverage are what it
 * keeps, so a stop moved changes what it risks and not its size. Its coins or
 * its risk can be typed too, and the margin goes to what they take. What the
 * numbers come to is worked out under them as they change.
 */
function SizeFields(props: {
  entry: number;
  stop: number;
  target: number;
  dir: number;
  coin: string;
  margin: number;
  setMargin: (v: number) => void;
  leverage: number;
  setLeverage: (v: number) => void;
  legs?: DrawnLeg[];
  children?: JSX.Element;
}) {
  const live = () => (props.legs ?? []).filter((l) => fits(l, props.entry, props.stop, props.target));
  const coins = () => coinsFor(props.margin, props.entry, props.leverage);
  const per = () => lossPerCoin(props.entry, props.stop, live(), props.dir);
  const made = createMemo(() => figures(props.entry, props.stop, props.target, coins(), live(), props.dir));
  const number = (label: string, value: () => number, set: (v: number) => void) => (
    <label class="jd-field">
      <span class="jd-label">{label}</span>
      <NumField class="jr-in num" value={value()} onValue={(v) => v > 0 && set(v)} label={label} />
    </label>
  );
  return (
    <>
      <div class="jd-grid">
        {number('Margin $', () => cents(props.margin), props.setMargin)}
        {number('Leverage ×', () => props.leverage, props.setLeverage)}
        {number(`Qty ${props.coin}`, () => figuresOf(coins()), (v) => props.setMargin(marginFor(v, props.entry, props.leverage)))}
        {number('Risk $', () => cents(made().loss), (v) => per() > 0 && props.setMargin(marginFor(v / per(), props.entry, props.leverage)))}
        {props.children}
      </div>
      <Readout figures={made()} hasLegs={live().some((l) => l.kind !== 'take')} />
    </>
  );
}

/**
 * One trade, every part of it in one form: its plan, its size and what those
 * come to, who says how it went, its strategy and notes, saved together.
 */
function Editor(props: {
  trade: Trade;
  price: (market: string, v: number | null) => string;
  onClose: () => void;
  onChange: (t: Trade) => void;
  onRemove: (id: number) => void;
}) {
  const t = () => props.trade;
  const [entry, setEntry] = createSignal('');
  const [stop, setStop] = createSignal('');
  const [target, setTarget] = createSignal('');
  const [margin, setMargin] = createSignal(0);
  const [leverage, setLeverage] = createSignal(1);
  const [fees, setFees] = createSignal('');
  const [strategy, setStrategy] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [mode, setMode] = createSignal<Mode>('auto');
  const [exit, setExit] = createSignal('');
  const [closedAt, setClosedAt] = createSignal('');
  // The form from the trade: when it opens, and again from what the server kept once saved.
  // A price as typed, without the float's tail an average leaves: 80311.69999999998 is 80311.7.
  const plain = (v: number) => String(Number(v.toPrecision(12)));
  const fill = (x: Trade) => {
    setEntry(plain(x.entry));
    setStop(plain(x.stop));
    setTarget(plain(x.target));
    setLeverage(x.leverage > 0 ? x.leverage : 1);
    setMargin(marginFor(x.qty, x.entry, x.leverage > 0 ? x.leverage : 1));
    setFees(x.fees ? String(x.fees) : '');
    setStrategy(x.strategy);
    setNotes(x.notes);
    setMode(modeOf(x));
    setExit(x.exit === null ? '' : plain(x.exit));
    setClosedAt(localInput(x.closedAt ?? Date.now()));
  };
  fill(t());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [sure, setSure] = createSignal(false);

  const coins = () => coinsFor(margin(), Number(entry()), leverage());
  const dir = () => (t().side === 'long' ? 1 : -1);
  const legs = (): DrawnLeg[] => t().legs.map((l) => ({ kind: l.kind, price: l.price, percent: l.share * 100 }));
  const moved = () => entry() !== plain(t().entry) || stop() !== plain(t().stop) || target() !== plain(t().target);
  /** What the form comes to, before it is saved; null where only price can say. */
  const result = createMemo((): number | null => {
    const [x, m, feesNow] = [t(), mode(), Number(fees() || 0)];
    if (m === 'cancelled' || !(coins() > 0)) return null;
    if (m === 'auto') return isClosed(x) && !moved() && x.qty > 0 ? ((pnl(x)! + x.fees) * coins()) / x.qty - feesNow : null;
    if (m === 'closed' && !(Number(exit()) > 0)) return null;
    return closedPnl(x, { entry: entry() === plain(x.entry) ? x.entry : Number(entry()), qty: coins(), exit: m === 'closed' ? Number(exit()) : null, fees: feesNow });
  });
  const heroTone = () => {
    const r = result();
    return r === null ? outcome(t()) : r > 0 ? 'win' : r < 0 ? 'loss' : 'even';
  };
  const heroR = () => {
    const r = result();
    const risk = figures(Number(entry()), Number(stop()), Number(target()), coins(), legs().filter((l) => fits(l, Number(entry()), Number(stop()), Number(target()))), dir()).loss;
    return r === null ? rMultiple(t()) : risk > 0 ? r / risk : null;
  };

  // What changed, as the server takes it.
  const patch = (): TradePatch => {
    const x = t();
    const p: TradePatch = {};
    if (entry() !== plain(x.entry)) p.entry = Number(entry());
    if (stop() !== plain(x.stop)) p.stop = Number(stop());
    if (target() !== plain(x.target)) p.target = Number(target());
    // A margin rounds to the cent as it shows; only a real change is a new size.
    if (coins() > 0 && Math.abs(coins() - x.qty) > x.qty * 1e-9) p.qty = coins();
    if (leverage() !== x.leverage) p.leverage = leverage();
    if (Number(fees() || 0) !== x.fees) p.fees = Number(fees() || 0);
    if (strategy() !== x.strategy) p.strategy = strategy();
    if (notes() !== x.notes) p.notes = notes();
    const m = mode();
    const at = Date.parse(closedAt());
    const changed = x.auto || x.status !== m || at !== minute(x.closedAt ?? 0);
    if (m === 'auto') {
      if (!x.auto) p.auto = true;
    } else if (m === 'closed') {
      if (changed || exit() !== (x.exit === null ? '' : plain(x.exit))) Object.assign(p, { status: 'closed', exit: Number(exit()), closedAt: at });
    } else if (changed) Object.assign(p, { status: m, closedAt: at });
    return p;
  };
  const dirty = () => Object.keys(patch()).length > 0;
  const problem = (): string | null => {
    const [e, s, g] = [Number(entry()), Number(stop()), Number(target())];
    if (!(e > 0 && s > 0 && g > 0)) return 'Entry, stop and target need prices.';
    if (t().side === 'long' ? !(s < e && e < g) : !(g < e && e < s)) return t().side === 'long' ? 'A long’s stop goes below its entry, its target above.' : 'A short’s stop goes above its entry, its target below.';
    if (!(margin() > 0 && leverage() > 0)) return 'Margin and leverage need amounts above 0.';
    if (mode() === 'closed' && !(Number(exit()) > 0)) return 'Closed at a price needs the exit price.';
    if (mode() !== 'auto' && Number.isNaN(Date.parse(closedAt()))) return 'Say when it closed.';
    return null;
  };
  const act = async (run: () => Promise<Trade | void>) => {
    setBusy(true);
    setError(null);
    try {
      const done = await run();
      if (done) {
        props.onChange(done);
        fill(done);
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const save = () =>
    act(async () => {
      const p = patch();
      const done = await journalApi.update(t().id, p);
      // Its position on the chart moved, or was resized, with it.
      if (t().drawingId !== null && (p.entry !== undefined || p.stop !== undefined || p.target !== undefined || p.qty !== undefined || p.leverage !== undefined)) setDrawingsChanged({ market: t().market });
      return done;
    });
  const openChart = () => {
    const chart = targetChart();
    if (!chart) return;
    if (marketOf(chart) !== t().market) setPanelMarket(chart.id, t().market);
    if (t().timeframe) setPanelTimeframe(chart.id, t().timeframe);
    focusPanel(chart.id);
    setJournalOpen(false);
  };
  const coin = () => coinOf(t().market);
  const qtyOf = (q: number) => `${Number(q.toPrecision(4))} ${coin()}`;
  const timeline = () => {
    const x = t();
    const parts = [`Drawn ${stamp(x.startAt)}`];
    if (x.openedAt !== null) parts.push(`opened ${stamp(x.openedAt)}`);
    if (x.closedAt !== null && !isLive(x)) parts.push(`closed ${stamp(x.closedAt)}${x.openedAt !== null ? ` · held ${held(x.closedAt - x.openedAt)}` : ''}`);
    return parts.join(' · ');
  };
  const modes = (): [Mode, string][] => [
    ['auto', t().tracked ? 'Price decides' : 'Still open'],
    ['closed', 'Closed'],
    ['breakeven', 'Break-even'],
    ['cancelled', 'Cancelled'],
  ];
  const modeHint = () => {
    const m = mode();
    if (m === 'auto' && moved() && t().tracked) return 'Its prices changed, so price weighs it again from the start once saved.';
    if (m === 'auto')
      return t().tracked
        ? `Open at the entry${t().legs.length > 0 ? ', its partials filled as price reaches them' : ''}, then closed at the target or the stop. Now: ${STATUS_LABEL[t().status]}.`
        : 'This market’s prices are not read here, so close it by hand when it is done.';
    if (m === 'closed') return 'Closed by hand at this price: price no longer decides it.';
    if (m === 'breakeven') return 'Out at the entry: no gain, no loss.';
    return 'Never taken: left out of every figure.';
  };
  return (
    <>
      <div class="jr-scrim" onClick={props.onClose} />
      <aside class="jr-drawer">
        <header class="jr-head jd-head">
          <span class={`jh-side ${t().side}`}>{t().side === 'long' ? 'Long' : 'Short'}</span>
          <span class="jr-title">{symbolOf(t().market)}</span>
          <span class="dim">{venueLabel(t().market)}</span>
          <Badge trade={t()} />
          <span class="grow" />
          <button class="iconbtn" aria-label="Close" onClick={props.onClose}>
            <Icon name="close" size={15} />
          </button>
        </header>
        <div class="jr-drawer-body">
          <div class={`jd-hero ${heroTone()}`}>
            <div class="jd-result">
              <span class="jd-pnl num">{result() !== null ? money(result()) : isLive(t()) || t().status === 'cancelled' ? STATUS_LABEL[t().status] : money(pnl(t()))}</span>
              <Show when={result() !== null || (!isLive(t()) && t().status !== 'cancelled')}>
                <span class="jd-r num">{rText(heroR())}</span>
              </Show>
            </div>
            <span class="dim num">{timeline()}</span>
          </div>

          <section class="jd-section">
            <h3>Plan</h3>
            <div class="jd-grid">
              <PriceField label="Entry" value={entry()} onInput={setEntry} />
              <PriceField label="Stop loss" value={stop()} onInput={setStop} hint={fromEntry(stop(), entry())} tone="bear" />
              <PriceField label="Take profit" value={target()} onInput={setTarget} hint={fromEntry(target(), entry())} tone="bull" />
            </div>
            <p class="jd-hint dim">
              {t().drawingId !== null
                ? `Kept in step with its ${t().timeframe} position on the chart: change it here or there.`
                : t().timeframe
                  ? 'Its position was deleted from the chart; the trade keeps these prices.'
                  : 'Added by hand.'}
            </p>
          </section>

          <section class="jd-section">
            <h3>Size</h3>
            <SizeFields entry={Number(entry())} stop={Number(stop())} target={Number(target())} dir={dir()} coin={coin()} margin={margin()} setMargin={setMargin} leverage={leverage()} setLeverage={setLeverage} legs={legs()}>
              <PriceField label="Fees $" value={fees()} onInput={setFees} />
            </SizeFields>
          </section>

          <section class="jd-section">
            <h3>Outcome</h3>
            <div class="jd-modes">
              <For each={modes()}>
                {([id, label]) => (
                  <button classList={{ on: mode() === id, [id]: true }} onClick={() => setMode(id)}>
                    {label}
                  </button>
                )}
              </For>
            </div>
            <Show when={mode() !== 'auto'}>
              <div class="jd-grid">
                <Show when={mode() === 'closed'}>
                  <PriceField label="Exit price" value={exit()} onInput={setExit} />
                </Show>
                <label class="jd-field jd-wide">
                  <span class="jd-label">{mode() === 'cancelled' ? 'Called off' : 'Closed'}</span>
                  <input class="jr-in" type="datetime-local" value={closedAt()} onInput={(e) => setClosedAt(e.currentTarget.value)} />
                </label>
              </div>
            </Show>
            <p class="jd-hint dim">{modeHint()}</p>
          </section>

          <section class="jd-section">
            <StrategyPicker value={strategy()} onChange={setStrategy} />
            <label class="jd-field">
              <span class="jd-label">Notes</span>
              <textarea class="jr-in" rows={4} placeholder="Setup, emotions, lessons learned…" value={notes()} onInput={(e) => setNotes(e.currentTarget.value)} />
            </label>
          </section>

          <Show when={t().fills.length > 1 || t().legs.length > 0}>
            <details class="jd-section jd-fills">
              <summary>
                <h3>Fills</h3>
              </summary>
              <ol class="jr-fills num">
                <For each={t().fills}>
                  {(f) => {
                    const b = book(t());
                    const made = IN.has(f.kind) ? null : (f.price - b.avgEntry) * (t().side === 'long' ? 1 : -1) * f.qty;
                    return (
                      <li>
                        <span class="dim">{stamp(f.at)}</span>
                        <span>{FILL_LABEL[f.kind]}</span>
                        <span>
                          {IN.has(f.kind) ? '+' : '−'}
                          {qtyOf(f.qty)}
                        </span>
                        <span>@ {props.price(t().market, f.price)}</span>
                        <span class={tone(made)}>{made === null ? '' : money(made)}</span>
                      </li>
                    );
                  }}
                </For>
                <For each={t().legs.filter((_, i) => !t().fills.some((f) => f.leg === i))}>
                  {(l) => (
                    <li class="dim">
                      <span>{isLive(t()) ? 'waiting' : 'not reached'}</span>
                      <span>{l.kind === 'take' ? 'Take profit' : l.kind === 'cut' ? 'Cut loss' : 'Add'}</span>
                      <span>{Math.round(l.share * 100)}%</span>
                      <span>@ {props.price(t().market, l.price)}</span>
                      <span />
                    </li>
                  )}
                </For>
              </ol>
            </details>
          </Show>
        </div>
        <footer class="jd-foot">
          <Show when={(dirty() && problem()) || error()}>{(e) => <p class="bear jd-error">{e()}</p>}</Show>
          <div class="jd-actions">
            <button class="jd-save" disabled={busy() || !dirty() || problem() !== null} onClick={save}>
              {busy() ? 'Saving…' : 'Save'}
            </button>
            <button class="tb" onClick={openChart}>
              <Icon name="external" size={12} />
              <span>Open chart</span>
            </button>
            <span class="grow" />
            <button
              class="tb jr-delete"
              classList={{ sure: sure() }}
              disabled={busy()}
              onClick={() => (sure() ? act(async () => void (await journalApi.remove(t().id), props.onRemove(t().id))) : setSure(true))}
            >
              <Icon name="trash" size={12} />
              <span>{sure() ? 'Delete for good' : 'Delete'}</span>
            </button>
          </div>
        </footer>
      </aside>
    </>
  );
}

/** `BTC` of BTCUSDT: what a trade's size is counted in. */
const coinOf = (market: string): string => symbolOf(market).replace(/(USDT|USDC|USD)$/, '') || 'units';

// ---------------------------------------------------------------- by hand

function AddTrade(props: { onClose: () => void; onAdd: (t: Trade) => void }) {
  const chart = targetChart();
  const [market, setMarket] = createSignal(chart ? marketOf(chart) : 'binance-perp:BTCUSDT');
  const [picking, setPicking] = createSignal(false);
  const [side, setSide] = createSignal<Side>('long');
  const [entry, setEntry] = createSignal('');
  const [stop, setStop] = createSignal('');
  const [target, setTarget] = createSignal('');
  // As the last position was sized, or the factory's.
  const size = defaultStyle('long');
  const [margin, setMargin] = createSignal(Number(size.margin) || 100);
  const [leverage, setLeverage] = createSignal(Number(size.leverage) || 1);
  const [at, setAt] = createSignal(localInput(Date.now()));
  const [closed, setClosed] = createSignal(false);
  const [exit, setExit] = createSignal('');
  const [closedAt, setClosedAt] = createSignal(localInput(Date.now()));
  const [strategy, setStrategy] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const closedTo = () => (Number(exit()) - Number(entry())) * (side() === 'long' ? 1 : -1) * coinsFor(margin(), Number(entry()), leverage());

  const submit = async (e: Event) => {
    e.preventDefault();
    if (!isMarket(market())) {
      setError('Market is venue:SYMBOL, as a chart names it — binance-perp:BTCUSDT');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const status: Status = closed() ? 'closed' : 'planned';
      rememberSize({ margin: margin(), leverage: leverage() });
      props.onAdd(
        await journalApi.create({
          market: market(),
          side: side(),
          entry: Number(entry()),
          stop: Number(stop()),
          target: Number(target()),
          qty: coinsFor(margin(), Number(entry()), leverage()),
          leverage: leverage(),
          startAt: Date.parse(at()),
          status,
          ...(closed() ? { exit: Number(exit()), closedAt: Date.parse(closedAt()) } : {}),
          strategy: strategy(),
          notes: notes(),
        }),
      );
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <form class="dialog jr-form" onSubmit={submit}>
        <header class="jr-head">
          <span class="jr-title">Add a trade</span>
          <span class="grow" />
          <button type="button" class="iconbtn" aria-label="Close" onClick={props.onClose}>
            <Icon name="close" size={15} />
          </button>
        </header>
        <div class="jr-drawer-body">
          <div class="jr-label">
            Market
            <button type="button" class="jr-in jr-market" data-tip="Search every market, as the chart's symbol search does" onClick={() => setPicking(true)}>
              <span class="sym">{symbolOf(market())}</span>
              <VenueTag market={market()} name />
              <span class="grow" />
              <Icon name="search" size={12} />
            </button>
          </div>
          <Show when={picking()}>
            {/* In a portal: closing the search on a press outside must not land that click on this dialog's backdrop. */}
            <Portal>
              <SymbolSearch
                initial=""
                title="Market"
                current={market()}
                onClose={() => setPicking(false)}
                onSelect={(m) => {
                  setMarket(m);
                  setPicking(false);
                }}
              />
            </Portal>
          </Show>
          <div class="chips">
            <button type="button" classList={{ on: side() === 'long' }} onClick={() => setSide('long')}>
              Long
            </button>
            <button type="button" classList={{ on: side() === 'short' }} onClick={() => setSide('short')}>
              Short
            </button>
          </div>
          <div class="jd-grid">
            <PriceField label="Entry" value={entry()} onInput={setEntry} required />
            <PriceField label="Stop loss" value={stop()} onInput={setStop} hint={fromEntry(stop(), entry())} tone="bear" required />
            <PriceField label="Take profit" value={target()} onInput={setTarget} hint={fromEntry(target(), entry())} tone="bull" required />
          </div>
          <SizeFields entry={Number(entry())} stop={Number(stop())} target={Number(target())} dir={side() === 'long' ? 1 : -1} coin={coinOf(market())} margin={margin()} setMargin={setMargin} leverage={leverage()} setLeverage={setLeverage}>
            <label class="jd-field jd-wide">
              <span class="jd-label">Entered</span>
              <input class="jr-in" type="datetime-local" value={at()} onInput={(e) => setAt(e.currentTarget.value)} />
            </label>
          </SizeFields>
          <label class="jr-check">
            <input type="checkbox" checked={closed()} onChange={(e) => setClosed(e.currentTarget.checked)} /> Already closed
          </label>
          <Show when={closed()}>
            <div class="jd-grid">
              <PriceField label="Exit price" value={exit()} onInput={setExit} required />
              <label class="jd-field jd-wide">
                <span class="jd-label">Closed</span>
                <input class="jr-in" type="datetime-local" value={closedAt()} onInput={(e) => setClosedAt(e.currentTarget.value)} />
              </label>
            </div>
            <Show when={Number(exit()) > 0 && Number(entry()) > 0}>
              <p class="jd-hint num">
                Comes to <b class={tone(closedTo())}>{money(closedTo())}</b>
              </p>
            </Show>
          </Show>
          <StrategyPicker value={strategy()} onChange={setStrategy} />
          <label class="jr-label">
            Notes
            <textarea class="jr-in" rows={3} value={notes()} onInput={(e) => setNotes(e.currentTarget.value)} />
          </label>
          <p class="dim jr-note">{closed() ? 'Kept as you set it.' : 'Price decides it from when it was entered, where this market’s prices are read.'}</p>
          <Show when={error()}>{(e) => <p class="bear">{e()}</p>}</Show>
          <div class="jr-actions">
            <button class="tb on" type="submit" disabled={busy()}>
              Add to the journal
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------- review

function ReviewTab(props: { trades: Trade[] }) {
  const weeks = lastWeeks(12).reverse();
  const [week, setWeek] = createSignal(weekOf(Date.now()).key);
  const [reviews, setReviews] = createSignal(new Map<string, Review>());
  const [draft, setDraft] = createSignal<Review['answers']>({});
  const [state, setState] = createSignal<'idle' | 'saving' | 'saved'>('idle');
  const [error, setError] = createSignal<string | null>(null);
  onMount(() => {
    void journalApi
      .reviews()
      .then((list) => {
        setReviews(new Map(list.map((r) => [r.week, r])));
        setDraft({ ...(reviews().get(week())?.answers ?? {}) });
      })
      .catch((e) => setError(errorText(e)));
  });
  const pick = (key: string) => {
    setWeek(key);
    setDraft({ ...(reviews().get(key)?.answers ?? {}) });
    setState('idle');
  };
  const current = () => weeks.find((w) => w.key === week()) ?? weeks[0]!;
  const s = createMemo(() => stats(inWeek(props.trades, current())));
  const save = async () => {
    setState('saving');
    setError(null);
    try {
      const saved = await journalApi.saveReview(week(), draft());
      setReviews((m) => new Map(m).set(week(), { week: week(), answers: draft(), savedAt: saved.savedAt }));
      setState('saved');
    } catch (e) {
      setError(errorText(e));
      setState('idle');
    }
  };
  return (
    <div class="jr-review">
      <nav class="jr-weeks">
        <For each={weeks}>
          {(w) => (
            <button classList={{ cur: w.key === week() }} onClick={() => pick(w.key)}>
              <span class="grow">{w.key.replace('-W', ' · W')}</span>
              <span class="dim">{w.start.slice(5)}</span>
              <Show when={reviews().has(w.key)}>
                <span class="bull" aria-label="reviewed">
                  ✓
                </span>
              </Show>
            </button>
          )}
        </For>
      </nav>
      <div class="jr-review-body">
        <div class="jr-cards">
          <Card label={`${current().start} – ${current().end}`} value={money(s().totalPnl)} tone={tone(s().totalPnl)} sub={`${s().closed} closed · ${s().total} in all`} />
          <Card label="Win rate" value={`${s().winRate.toFixed(0)}%`} sub={`${s().wins} won · ${s().losses} lost`} />
          <Card label="Total R" value={rText(s().totalR)} tone={tone(s().totalR)} />
        </div>
        <For each={QUESTIONS}>
          {(q) => (
            <label class="jr-q">
              <span>
                <span class="badge">{q.badge}</span> {q.question}
              </span>
              <span class="dim">{q.prompt}</span>
              <textarea
                class="jr-in"
                rows={3}
                value={draft()[q.id as AnswerKey] ?? ''}
                onInput={(e) => {
                  const v = e.currentTarget.value;
                  setDraft((d) => ({ ...d, [q.id]: v }));
                  setState('idle');
                }}
              />
            </label>
          )}
        </For>
        <Show when={error()}>{(e) => <p class="bear">{e()}</p>}</Show>
        <div class="jr-actions">
          <button class="tb on" disabled={state() === 'saving'} onClick={() => void save()}>
            {state() === 'saving' ? 'Saving…' : 'Save the review'}
          </button>
          <Show when={state() === 'saved'}>
            <span class="dim">Saved.</span>
          </Show>
        </div>
      </div>
    </div>
  );
}
