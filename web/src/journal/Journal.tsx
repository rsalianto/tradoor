/**
 * The trading journal, over the workspace: every long or short position drawn
 * on a chart lands here as a trade, and price decides how it went (see
 * bins/server/src/journal.rs). Three tabs, as rsalianto's journal has them:
 * the dashboard of what the trades add up to, their history — each opened to
 * set its strategy and notes or close it by hand — and the weekly review. It
 * is kept for an account: until the browser signs in with Telegram, it shows
 * how, and nothing is written down.
 */
import { createMemo, createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { Portal } from 'solid-js/web';
import { setDrawingsChanged, setJournalOpen } from '../app/ui';
import { AccountCard } from '../components/Account';
import { SymbolSearch } from '../components/SymbolSearch';
import { api, catalog, errorText, type AccountStatus } from '../lib/api';
import { formatPrice } from '../lib/format';
import { VenueTag } from '../lib/exchanges';
import { Icon } from '../lib/icons';
import { isMarket, symbolOf, venueLabel } from '../lib/markets';
import { focusPanel, marketOf, setPanelMarket, setPanelTimeframe, targetChart } from '../workspace/store';
import { StrategyPicker } from './strategies';
import {
  book,
  dailyPnl,
  FILL_LABEL,
  equityCurve,
  groupBy,
  inWeek,
  isLive,
  journalApi,
  lastWeeks,
  localDay,
  localInput,
  money,
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
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoaded(true);
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
  onMount(() => {
    void readAccount();
    // Asked again only while the page is in view and price still has a trade
    // to decide; back in view, at once.
    const timer = setInterval(() => {
      if (!document.hidden && trades().some((t) => t.auto && t.tracked && isLive(t))) void load();
    }, REFRESH_MS);
    const shown = () => {
      if (!document.hidden) void load();
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
      <Show when={error()}>{(e) => <div class="banner">▲ {e()}</div>}</Show>
      <div class="jr-body">
        <Show when={account()} fallback={<p class="jr-empty muted">Reading…</p>}>
          <Show when={signedIn()} fallback={<SignInFirst onSignedIn={() => void readAccount()} />}>
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
                <Dashboard trades={trades()} />
              </Match>
              <Match when={tab() === 'history' && trades().length > 0}>
                <History trades={trades()} price={price} onSelect={setSelected} />
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

function Dashboard(props: { trades: Trade[] }) {
  const [period, setPeriod] = createSignal('all');
  const list = createMemo(() => {
    const p = period();
    if (p === 'all') return props.trades;
    const from = Date.now() - Number(p) * 86_400_000;
    return props.trades.filter((t) => when(t) >= from);
  });
  const s = createMemo(() => stats(list()));
  return (
    <div class="jr-dash">
      <div class="chips jr-periods">
        <For each={PERIODS}>
          {([id, label]) => (
            <button classList={{ on: period() === id }} onClick={() => setPeriod(id)}>
              {label}
            </button>
          )}
        </For>
      </div>
      <div class="jr-cards">
        <Card label="Net P&L" value={money(s().totalPnl)} tone={tone(s().totalPnl)} sub={`${money(s().grossProfit)} won · ${money(-s().grossLoss)} lost`} />
        <Card label="Win rate" value={`${s().winRate.toFixed(1)}%`} sub={`${s().wins} won · ${s().losses} lost · ${s().breakEven} even`} />
        <Card label="Total R" value={rText(s().totalR)} tone={tone(s().totalR)} sub={`${rText(s().avgR)} a trade`} />
        <Card label="Profit factor" value={s().profitFactor === null ? '—' : s().profitFactor!.toFixed(2)} sub={`expectancy ${money(s().expectancy)}`} />
        <Card label="Avg win / loss" value={money(s().avgWin)} sub={money(-s().avgLoss)} />
        <Card label="Max drawdown" value={money(-s().maxDrawdown)} tone={s().maxDrawdown > 0 ? 'bear' : ''} sub={`best ${money(s().best)} · worst ${money(s().worst)}`} />
        <Card label="Streak" value={s().streak > 0 ? `${s().streak} ${s().streakKind === 'win' ? (s().streak > 1 ? 'wins' : 'win') : s().streak > 1 ? 'losses' : 'loss'}` : '—'} tone={s().streakKind === 'win' ? 'bull' : s().streakKind === 'loss' ? 'bear' : ''} sub={s().avgHoldHours === null ? undefined : `held ${hold(s().avgHoldHours!)} on average`} />
        <Card label="Trades" value={`${s().closed} closed`} sub={`${s().open} open · ${s().planned} planned · ${s().longs} long · ${s().shorts} short`} />
      </div>
      <div class="jr-grid">
        <section class="jr-box jr-wide">
          <h3>Equity</h3>
          <Equity trades={list()} />
        </section>
        <section class="jr-box">
          <h3>P&L calendar</h3>
          <Calendar trades={list()} />
        </section>
        <section class="jr-box">
          <h3>By pair</h3>
          <Groups groups={groupBy(list(), (t) => symbolOf(t.market))} />
        </section>
        <section class="jr-box">
          <h3>By side</h3>
          <Groups groups={groupBy(list(), (t) => (t.side === 'long' ? 'Long' : 'Short'))} />
        </section>
        <section class="jr-box">
          <h3>By strategy</h3>
          <Groups groups={groupBy(list(), (t) => t.strategy)} />
        </section>
      </div>
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

/** The running total of closed trades, as a line over zero. */
function Equity(props: { trades: Trade[] }) {
  const W = 600;
  const H = 150;
  const curve = createMemo(() => equityCurve(props.trades));
  const shape = createMemo(() => {
    const c = curve();
    if (c.length < 2) return null;
    const totals = [0, ...c.map((p) => p.total)];
    const lo = Math.min(...totals);
    const hi = Math.max(...totals);
    const span = hi - lo || 1;
    const y = (v: number) => H - 6 - ((v - lo) / span) * (H - 12);
    const x = (i: number) => (i / c.length) * W;
    const points = [`0,${y(0)}`, ...c.map((p, i) => `${x(i + 1)},${y(p.total)}`)].join(' ');
    return { points, zero: y(0), last: c[c.length - 1]!.total };
  });
  return (
    <Show when={shape()} fallback={<p class="dim">The curve draws itself once two trades have closed.</p>}>
      {(s) => (
        <svg class="jr-equity" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={`Equity: ${money(s().last)}`}>
          <line x1="0" x2={W} y1={s().zero} y2={s().zero} class="jr-zero" />
          <polyline points={s().points} class={s().last >= 0 ? 'jr-line bull' : 'jr-line bear'} vector-effect="non-scaling-stroke" />
        </svg>
      )}
    </Show>
  );
}

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

/** A month of days, each with what its closed trades made, and each week's R beside it. */
function Calendar(props: { trades: Trade[] }) {
  const today = new Date();
  const [month, setMonth] = createSignal({ y: today.getFullYear(), m: today.getMonth() });
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
      return { row, r: traded.reduce((sum, d) => sum + d.r, 0), trades: traded.reduce((sum, d) => sum + d.trades, 0), first: row.find((day) => day !== null)! };
    }),
  );
  const total = createMemo(() => cells().reduce((sum, day) => sum + (day ? (days().get(day)?.pnl ?? 0) : 0), 0));
  const biggest = createMemo(() => Math.max(1, ...cells().map((day) => Math.abs(day ? (days().get(day)?.pnl ?? 0) : 0))));
  const step = (n: number) => setMonth(({ y, m }) => ({ y: m + n < 0 ? y - 1 : m + n > 11 ? y + 1 : y, m: (m + n + 12) % 12 }));
  return (
    <div class="jr-cal">
      <div class="jr-cal-head">
        <button class="iconbtn" aria-label="Previous month" onClick={() => step(-1)}>
          ‹
        </button>
        <span class="grow">
          {MONTHS[month().m]} {month().y}
        </span>
        <span class={`num ${tone(total())}`}>{money(total())}</span>
        <button class="iconbtn" aria-label="Next month" onClick={() => step(1)}>
          ›
        </button>
      </div>
      <div class="jr-cal-grid">
        <For each={WEEKDAYS}>{(d) => <span class="dim jr-wd">{d}</span>}</For>
        <span class="dim jr-wd">Week</span>
        <For each={weeks()}>
          {(week) => (
            <>
              <For each={week.row}>
                {(day) => {
                  const d = () => (day ? days().get(day) : undefined);
                  const strength = () => Math.min(1, Math.abs(d()?.pnl ?? 0) / biggest());
                  return (
                    <div
                      class="jr-day"
                      classList={{ blank: !day, up: (d()?.pnl ?? 0) > 0, down: (d()?.pnl ?? 0) < 0 }}
                      style={{ '--s': String(0.12 + strength() * 0.5) }}
                      data-tip={d() ? `${day}: ${money(d()!.pnl)} from ${d()!.trades} trade${d()!.trades > 1 ? 's' : ''}` : undefined}
                    >
                      <Show when={day}>
                        <span class="dim">{Number(day!.slice(8))}</span>
                        <Show when={d()}>
                          <span class="num">{compactMoney(d()!.pnl)}</span>
                        </Show>
                      </Show>
                    </div>
                  );
                }}
              </For>
              <div class={`jr-week num ${week.trades > 0 ? tone(week.r) : 'dim'}`} data-tip={week.trades > 0 ? `Week of ${week.first}: ${rText(week.r)} from ${week.trades} trade${week.trades > 1 ? 's' : ''}` : undefined}>
                {week.trades > 0 ? rText(week.r) : '—'}
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

function Groups(props: { groups: Group[] }) {
  return (
    <Show when={props.groups.length > 0} fallback={<p class="dim">Nothing closed yet.</p>}>
      <table class="jr-table jr-groups">
        <thead>
          <tr>
            <th />
            <th>Trades</th>
            <th>Win</th>
            <th>R</th>
            <th>P&L</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.groups.slice(0, 12)}>
            {(g) => (
              <tr>
                <td>{g.key}</td>
                <td class="num">{g.trades}</td>
                <td class="num">{g.winRate.toFixed(0)}%</td>
                <td class={`num ${tone(g.totalR)}`}>{rText(g.totalR)}</td>
                <td class={`num ${tone(g.pnl)}`}>{money(g.pnl)}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </Show>
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
function History(props: { trades: Trade[]; price: (market: string, v: number | null) => string; onSelect: (id: number) => void }) {
  const [view, setView] = createSignal<View>('all');
  const [side, setSide] = createSignal<'all' | Side>('all');
  const [query, setQuery] = createSignal('');
  // The side and the search narrow everything, the counts too; the view picks among what is left.
  const pool = createMemo(() => {
    const q = query().trim().toUpperCase();
    return props.trades.filter((t) => (side() === 'all' || t.side === side()) && (!q || t.market.toUpperCase().includes(q) || t.strategy.toUpperCase().includes(q) || t.notes.toUpperCase().includes(q)));
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
function Row(props: { trade: Trade; price: (market: string, v: number | null) => string; onSelect: (id: number) => void }) {
  const t = () => props.trade;
  const done = () => !isLive(t());
  const p = (v: number | null) => props.price(t().market, v);
  return (
    <button class={`jh-row ${outcome(t())}`} onClick={() => props.onSelect(t().id)}>
      <span class="jh-time num dim">{clock(done() ? when(t()) : (t().openedAt ?? t().startAt))}</span>
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

/**
 * One trade, every part of it in one form: its plan — entry, stop, target,
 * risk, fees — who says how it went, its strategy and notes, saved together.
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
  const [risk, setRisk] = createSignal('');
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
    setRisk(String(Number(x.risk.toFixed(2))));
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

  // What changed, as the server takes it.
  const patch = (): TradePatch => {
    const x = t();
    const p: TradePatch = {};
    if (entry() !== plain(x.entry)) p.entry = Number(entry());
    if (stop() !== plain(x.stop)) p.stop = Number(stop());
    if (target() !== plain(x.target)) p.target = Number(target());
    if (Number(risk()) !== Number(x.risk.toFixed(2))) p.risk = Number(risk());
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
    if (!(Number(risk()) > 0)) return 'Risk needs an amount above 0.';
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
      if (t().drawingId !== null && (p.entry !== undefined || p.stop !== undefined || p.target !== undefined || p.risk !== undefined)) setDrawingsChanged({ market: t().market });
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
  const coin = () => symbolOf(t().market).replace(/(USDT|USDC|USD)$/, '') || 'units';
  const qtyOf = (q: number) => `${Number(q.toPrecision(4))} ${coin()}`;
  // The size the form's risk buys over its stop; with adds drawn, the trade's own is a little less.
  const size = () => {
    const per = Math.abs(Number(entry()) - Number(stop()));
    return per > 0 && Number(risk()) > 0 ? qtyOf(Number(risk()) / per) : '—';
  };
  const rr = () => {
    const per = Math.abs(Number(entry()) - Number(stop()));
    return per > 0 ? (Math.abs(Number(target()) - Number(entry())) / per).toFixed(2) : '—';
  };
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
    if (m === 'auto')
      return t().tracked
        ? `Open at the entry${t().legs.length > 0 ? ', its partials filled as price reaches them' : ''}, then closed at the target or the stop. Now: ${STATUS_LABEL[t().status]}.`
        : 'This market’s prices are not read here, so close it by hand when it is done.';
    if (m === 'closed') return 'Closed by hand at this price: price no longer decides it.';
    if (m === 'breakeven') return 'Out at the entry: no gain, no loss.';
    return 'Never taken: left out of every figure.';
  };
  const field = (label: string, value: () => string, set: (v: string) => void, hint?: () => string) => (
    <label class="jd-field">
      <span class="jd-label">{label}</span>
      <input class="jr-in num" inputmode="decimal" value={value()} onInput={(e) => set(e.currentTarget.value)} />
      {hint ? <span class="jd-hint dim num">{hint()}</span> : null}
    </label>
  );
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
          <div class={`jd-hero ${outcome(t())}`}>
            <div class="jd-result">
              <span class="jd-pnl num">{isLive(t()) || t().status === 'cancelled' ? STATUS_LABEL[t().status] : money(pnl(t()))}</span>
              <Show when={!isLive(t()) && t().status !== 'cancelled'}>
                <span class="jd-r num">{rText(rMultiple(t()))}</span>
              </Show>
            </div>
            <span class="dim num">{timeline()}</span>
          </div>

          <section class="jd-section">
            <h3>Plan</h3>
            <div class="jd-grid">
              {field('Entry', entry, setEntry)}
              {field('Stop loss', stop, setStop)}
              {field('Take profit', target, setTarget, () => `${rr()} RR`)}
              {field('Risk $', risk, setRisk, () => size())}
              {field('Fees $', fees, setFees)}
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
                <Show when={mode() === 'closed'}>{field('Exit price', exit, setExit)}</Show>
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

// ---------------------------------------------------------------- by hand

function AddTrade(props: { onClose: () => void; onAdd: (t: Trade) => void }) {
  const chart = targetChart();
  const [market, setMarket] = createSignal(chart ? marketOf(chart) : 'binance-perp:BTCUSDT');
  const [picking, setPicking] = createSignal(false);
  const [side, setSide] = createSignal<Side>('long');
  const [entry, setEntry] = createSignal('');
  const [stop, setStop] = createSignal('');
  const [target, setTarget] = createSignal('');
  const [risk, setRisk] = createSignal('10');
  const [at, setAt] = createSignal(localInput(Date.now()));
  const [closed, setClosed] = createSignal(false);
  const [exit, setExit] = createSignal('');
  const [closedAt, setClosedAt] = createSignal(localInput(Date.now()));
  const [strategy, setStrategy] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

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
      props.onAdd(
        await journalApi.create({
          market: market(),
          side: side(),
          entry: Number(entry()),
          stop: Number(stop()),
          target: Number(target()),
          risk: Number(risk()) || undefined,
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
          <div class="jr-row">
            <label class="jr-label">
              Entry
              <input class="jr-in num" inputmode="decimal" required value={entry()} onInput={(e) => setEntry(e.currentTarget.value)} />
            </label>
            <label class="jr-label">
              Stop
              <input class="jr-in num" inputmode="decimal" required value={stop()} onInput={(e) => setStop(e.currentTarget.value)} />
            </label>
            <label class="jr-label">
              Target
              <input class="jr-in num" inputmode="decimal" required value={target()} onInput={(e) => setTarget(e.currentTarget.value)} />
            </label>
          </div>
          <div class="jr-row">
            <label class="jr-label">
              Risk ($)
              <input class="jr-in num" inputmode="decimal" value={risk()} onInput={(e) => setRisk(e.currentTarget.value)} />
            </label>
            <label class="jr-label">
              Entered
              <input class="jr-in" type="datetime-local" value={at()} onInput={(e) => setAt(e.currentTarget.value)} />
            </label>
          </div>
          <label class="jr-check">
            <input type="checkbox" checked={closed()} onChange={(e) => setClosed(e.currentTarget.checked)} /> Already closed
          </label>
          <Show when={closed()}>
            <div class="jr-row">
              <label class="jr-label">
                Exit
                <input class="jr-in num" inputmode="decimal" required value={exit()} onInput={(e) => setExit(e.currentTarget.value)} />
              </label>
              <label class="jr-label">
                Closed
                <input class="jr-in" type="datetime-local" value={closedAt()} onInput={(e) => setClosedAt(e.currentTarget.value)} />
              </label>
            </div>
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
