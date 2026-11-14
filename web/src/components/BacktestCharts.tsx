/**
 * The strategy tester's Charts tab: the equity against holding the coin, its
 * drawdowns, the months, how the trades' results spread, how far each went
 * against it before it closed, and the same trades in a thousand other orders.
 * Plain SVG, drawn to the panel's width.
 */
import { createMemo, For, Show } from 'solid-js';
import { distribution, monteCarlo, monthly, pct, ratio, type Stats, type TradeRow } from '../lib/backtest';
import type { BacktestState } from '../lib/backtest-state';
import { formatAlertTime } from '../lib/timezone';
import type { Ok } from './StrategyTester';

const W = 1000;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** At most this many points a line: one per screen pixel is plenty. */
const POINTS = 1200;

function path(values: ArrayLike<number>, lo: number, hi: number, h: number): string {
  const n = values.length;
  if (n === 0 || !(hi > lo)) return '';
  const step = Math.max(1, Math.floor(n / POINTS));
  let d = '';
  for (let i = 0; i < n; i += step) {
    const v = values[i]!;
    if (!Number.isFinite(v)) continue;
    const x = (i / Math.max(1, n - 1)) * W;
    const y = h - ((v - lo) / (hi - lo)) * h;
    d += `${d ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`;
  }
  return d;
}

function Equity(props: { result: Ok; state: BacktestState }) {
  const H = 160;
  const data = createMemo(() => {
    const eq = props.result.equity;
    const close = props.result.close;
    const start = eq[0] ?? 0;
    const c0 = close[0] ?? 0;
    // Holding the coin with the same capital, to set beside it.
    const hold = Float64Array.from(close, (c) => (c0 > 0 ? (start * c) / c0 : Number.NaN));
    let lo = Infinity;
    let hi = -Infinity;
    for (const xs of [eq, hold]) for (const v of xs) if (Number.isFinite(v)) (lo = Math.min(lo, v)), (hi = Math.max(hi, v));
    const pad = (hi - lo) * 0.05 || 1;
    const dd = new Float64Array(eq.length);
    let peak = -Infinity;
    let worst = 0;
    for (let i = 0; i < eq.length; i++) {
      peak = Math.max(peak, eq[i]!);
      dd[i] = peak > 0 ? eq[i]! / peak - 1 : 0;
      worst = Math.min(worst, dd[i]!);
    }
    return { eq, hold, lo: lo - pad, hi: hi + pad, dd, worst: worst || -0.01 };
  });
  const mark = (t: number | null) => {
    if (t === null) return null;
    const time = props.result.time;
    const n = time.length;
    if (n < 2 || t < time[0]! || t > time[n - 1]!) return null;
    let i = 0;
    while (i < n && time[i]! < t) i++;
    return (i / (n - 1)) * W;
  };
  const split = () => (props.state.settings.split > 0 ? (props.state.settings.split / 100) * W : null);
  const fwd = () => mark(props.state.forwardFrom);
  const first = () => props.result.time[0] ?? 0;
  const last = () => props.result.time[props.result.time.length - 1] ?? 0;
  return (
    <figure class="bt-chart">
      <figcaption>
        Equity <span class="key eq" /> against holding the coin <span class="key hold" />
        <span class="dim num"> · {formatAlertTime(first())} to {formatAlertTime(last())}</span>
      </figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" class="bt-svg" role="img" aria-label="The equity over the test, and holding the coin">
        <path d={path(data().hold, data().lo, data().hi, H)} class="hold" vector-effect="non-scaling-stroke" />
        <path d={path(data().eq, data().lo, data().hi, H)} class="eq" vector-effect="non-scaling-stroke" />
        <Show when={split()}>{(x) => <line x1={x()} x2={x()} y1={0} y2={H} class="split" vector-effect="non-scaling-stroke" />}</Show>
        <Show when={fwd()}>{(x) => <line x1={x()} x2={x()} y1={0} y2={H} class="fwd" vector-effect="non-scaling-stroke" />}</Show>
      </svg>
      <div class="bt-axis num dim">
        <span>{Math.round(data().hi).toLocaleString('en-US')}</span>
        <span>{Math.round(data().lo).toLocaleString('en-US')}</span>
      </div>
      <figcaption>
        Drawdown <span class="dim num">· deepest {pct(data().worst)}</span>
        <Show when={split()}>
          <span class="dim"> · the dashed line: tuned on the left, kept back on the right</span>
        </Show>
        <Show when={fwd()}>
          <span class="dim"> · the dotted line: the forward test’s start</span>
        </Show>
      </figcaption>
      <svg viewBox={`0 0 ${W} 60`} preserveAspectRatio="none" class="bt-svg dd" role="img" aria-label="How far the equity was under its high">
        <path d={`${path(data().dd, data().worst, 0, 60)}L${W},0L0,0Z`} class="ddfill" />
      </svg>
    </figure>
  );
}

function Months(props: { result: Ok }) {
  const rows = createMemo(() => {
    const byYear = new Map<number, (number | null)[]>();
    for (const [y, m, r] of monthly(props.result)) {
      const row = byYear.get(y) ?? Array.from({ length: 12 }, () => null);
      row[m] = r;
      byYear.set(y, row);
    }
    return [...byYear.entries()].sort((a, b) => b[0] - a[0]);
  });
  const year = (row: (number | null)[]) => row.reduce<number>((acc, r) => (r === null || !Number.isFinite(r) ? acc : acc * (1 + r)), 1) - 1;
  const shade = (r: number | null) => {
    if (r === null || !Number.isFinite(r)) return undefined;
    const a = Math.min(1, Math.abs(r) / 0.2);
    return `color-mix(in oklab, var(${r >= 0 ? '--bull' : '--bear'}) ${Math.round(a * 45)}%, transparent)`;
  };
  return (
    <figure class="bt-chart">
      <figcaption>Returns by month, UTC</figcaption>
      <div class="bt-table-wrap">
        <table class="bt-months num">
          <thead>
            <tr>
              <th />
              <For each={MONTHS}>{(m) => <th>{m}</th>}</For>
              <th>Year</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {([y, row]) => (
                <tr>
                  <th>{y}</th>
                  <For each={row}>{(r) => <td style={{ background: shade(r) }}>{r === null ? '' : pct(r)}</td>}</For>
                  <td style={{ background: shade(year(row)) }}>
                    <b>{pct(year(row))}</b>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </figure>
  );
}

function Spread(props: { rows: TradeRow[] }) {
  const d = createMemo(() => distribution(props.rows));
  const most = () => Math.max(1, ...d().bins.map((b) => b.count));
  return (
    <figure class="bt-chart half">
      <figcaption>
        How the results spread <span class="dim">· in {d().unit === 'R' ? 'R, multiples of the first stop’s loss' : 'percent of the equity'}</span>
      </figcaption>
      <Show when={d().bins.length > 0} fallback={<p class="dim">No closed trades.</p>}>
        <svg viewBox="0 0 400 120" class="bt-svg" role="img" aria-label="How many trades ended at each result">
          <For each={d().bins}>
            {(b, i) => {
              const w = 400 / d().bins.length;
              const h = (b.count / most()) * 104;
              return (
                <rect x={i() * w + 1} width={Math.max(1, w - 2)} y={112 - h} height={h} class={b.to <= 0 ? 'neg' : b.from >= 0 ? 'pos' : 'mid'}>
                  <title>
                    {ratio(b.from)} to {ratio(b.to)}
                    {d().unit === 'R' ? 'R' : '%'}: {b.count}
                  </title>
                </rect>
              );
            }}
          </For>
        </svg>
        <div class="bt-axis row num dim">
          <span>
            {ratio(d().bins[0]!.from)}
            {d().unit}
          </span>
          <span>
            {ratio(d().bins[d().bins.length - 1]!.to)}
            {d().unit}
          </span>
        </div>
      </Show>
    </figure>
  );
}

function Excursion(props: { rows: TradeRow[] }) {
  const pts = createMemo(() => props.rows.filter((t) => !t.open).slice(-800));
  // Scaled to the 95th percentile, a far one held at the edge: one outlier would flatten the rest.
  const p95 = (xs: number[]) => {
    const s = xs.slice().sort((a, b) => a - b);
    return Math.max(0.001, s[Math.min(s.length - 1, Math.floor(s.length * 0.95))] ?? 0);
  };
  const most = createMemo(() => p95(pts().map((t) => t.mae)));
  const range = createMemo(() => p95(pts().map((t) => Math.abs(t.ret))));
  const clip = (v: number) => Math.max(-1, Math.min(1, v));
  return (
    <figure class="bt-chart half">
      <figcaption>
        How far each went against it <span class="dim">· across, before it closed; up, what it made. Winners far right say the stop sits too close.</span>
      </figcaption>
      <Show when={pts().length > 0} fallback={<p class="dim">No closed trades.</p>}>
        <svg viewBox="0 0 400 120" class="bt-svg" role="img" aria-label="Each trade's adverse excursion against its result">
          <line x1={0} x2={400} y1={60} y2={60} class="zero" />
          <For each={pts()}>{(t) => <circle cx={Math.min(1, t.mae / most()) * 392 + 4} cy={60 - clip(t.ret / range()) * 54} r={2.2} class={t.pnl > 0 ? 'pos' : 'neg'} />}</For>
        </svg>
        <div class="bt-axis row num dim">
          <span>0%</span>
          <span>{pct(-most())} against, and past</span>
        </div>
      </Show>
    </figure>
  );
}

function Shuffled(props: { rows: TradeRow[]; stats: Stats | null }) {
  const mc = createMemo(() => monteCarlo(props.rows));
  return (
    <figure class="bt-chart">
      <figcaption>
        The trades drawn again at random, 1,000 times <span class="dim">· the same edge, other luck</span>
      </figcaption>
      <Show when={mc()} fallback={<p class="dim">Five closed trades at least are needed.</p>}>
        {(m) => (
          <div class="bt-mc num">
            <span>
              Drawdown: half the runs reach <b class="bear">{pct(-m().dd[1])}</b>, one in twenty <b class="bear">{pct(-m().dd[2])}</b>
              {props.stats ? ` (the test’s own: ${pct(-props.stats.maxDdPct)})` : ''}
            </span>
            <span>
              Return: <b>{pct(m().ret[0])}</b> to <b>{pct(m().ret[2])}</b> for nine runs in ten, the middle one {pct(m().ret[1])} · {pct(m().losing, false)} of the runs lost money
            </span>
            <span class="dim">Plan for the deeper drawdown, not the one this stretch of history happened to give.</span>
          </div>
        )}
      </Show>
    </figure>
  );
}

export function BacktestCharts(props: { result: Ok; rows: TradeRow[]; stats: Stats | null; state: BacktestState }) {
  return (
    <div class="bt-charts">
      <Equity result={props.result} state={props.state} />
      <div class="bt-pair">
        <Spread rows={props.rows} />
        <Excursion rows={props.rows} />
      </div>
      <Shuffled rows={props.rows} stats={props.stats} />
      <Months result={props.result} />
    </div>
  );
}
