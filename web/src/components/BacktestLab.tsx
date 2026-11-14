/**
 * The strategy tester's Optimize and Markets tabs. The optimizer runs the
 * strategy once for each value, or each pair of values, of its settings on a
 * grid, in workers of their own (lib/optimizer), and can keep part of the bars
 * back to judge its pick on bars it never saw. Markets runs the same strategy,
 * as it stands, over other markets' bars of the chart's timeframe.
 */
import { batch, createMemo, createResource, createSignal, For, Index, onCleanup, Show } from 'solid-js';
import type { FormulaInput } from '../engine/protocol';
import { money, pct, ratio, rulesScript, settingsOf, statsOf, steps, tradesOf, withSettings, type Stats } from '../lib/backtest';
import { backtestMarket } from '../lib/backtest-live';
import { describe } from '../lib/formulas';
import { isChartMarket, isRatio, parseMarket, symbolOf } from '../lib/markets';
import { optimize, type Cell } from '../lib/optimizer';
import { doc } from '../workspace/store';
import type { TesterProps } from './StrategyTester';

const METRICS: { id: string; label: string; get(s: Stats): number; show(v: number): string }[] = [
  { id: 'net', label: 'Net profit', get: (s) => s.netPct, show: (v) => pct(v) },
  { id: 'sharpe', label: 'Sharpe', get: (s) => s.sharpe, show: (v) => ratio(v) },
  { id: 'pf', label: 'Profit factor', get: (s) => s.profitFactor, show: (v) => ratio(v) },
  { id: 'recovery', label: 'Net over drawdown', get: (s) => s.recovery, show: (v) => ratio(v) },
  { id: 'dd', label: 'Shallowest drawdown', get: (s) => -s.maxDdPct, show: (v) => pct(v) },
  { id: 'win', label: 'Win rate', get: (s) => s.winRate, show: (v) => pct(v, false) },
  { id: 'exp', label: 'Expectancy a trade', get: (s) => s.expectancyPct, show: (v) => pct(v) },
];

/** Fewer closed trades than this say little: shown, never picked. */
const FEW = 5;

const tone = (v: number) => (v > 0 ? 'bull' : v < 0 ? 'bear' : '');

export function BacktestLab(props: TesterProps & { tab: string }) {
  return <Show when={props.tab === 'optimize'} fallback={<Markets {...props} />}>{<Optimizer {...props} />}</Show>;
}

// ---------------------------------------------------------------- the optimizer

interface Axis {
  index: number;
  from: number;
  to: number;
  by: number;
}

function axisFor(inp: FormulaInput, value: number, index: number): Axis {
  const whole = inp.step >= 1 && Number.isInteger(value);
  const lo = Math.max(inp.min ?? -Infinity, whole ? Math.max(1, Math.round(value * 0.5)) : value * 0.5);
  const hi = Math.min(inp.max ?? Infinity, whole ? Math.round(value * 1.5) : value * 1.5);
  const by = whole ? Math.max(1, Math.round((hi - lo) / 9)) : Number(((hi - lo) / 9).toPrecision(2)) || inp.step;
  return { index, from: lo, to: hi, by };
}

function Optimizer(props: TesterProps) {
  const source = () => props.run?.source ?? '';
  const [desc] = createResource(() => source() || null, (s) => describe(s));
  const numbers = createMemo(() => {
    const d = desc();
    return d?.ok ? d.inputs.map((inp, index) => ({ inp, index })).filter((x) => x.inp.kind === 'number') : [];
  });
  const current = () => {
    const d = desc();
    if (!d?.ok) return [];
    return d.inputs.map((inp, i) => props.run?.inputs[i] ?? inp.default);
  };
  const [x, setX] = createSignal<Axis | null>(null);
  const [y, setY] = createSignal<Axis | null>(null);
  const [metric, setMetric] = createSignal('net');
  const [cells, setCells] = createSignal<Cell[]>([]);
  const [done, setDone] = createSignal(0);
  const [total, setTotal] = createSignal(0);
  const [running, setRunning] = createSignal(false);
  const [picked, setPicked] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [grid, setGrid] = createSignal<{ xs: number[]; ys: number[] | null; xi: number; yi: number | null }>({ xs: [], ys: null, xi: 0, yi: null });
  let abort: AbortController | null = null;
  onCleanup(() => abort?.abort());

  const pickAxis = (which: 'x' | 'y', index: number) => {
    const n = numbers().find((v) => v.index === index);
    const a = n ? axisFor(n.inp, current()[index] ?? n.inp.default, index) : null;
    (which === 'x' ? setX : setY)(a);
  };
  // The first one or two numbers, to start with.
  createMemo(() => {
    const list = numbers();
    if (!x() && list[0]) pickAxis('x', list[0].index);
  });
  const xs = () => (x() ? steps(x()!.from, x()!.to, x()!.by) : []);
  const ys = () => (y() ? steps(y()!.from, y()!.to, y()!.by) : null);
  const count = () => xs().length * (ys()?.length ?? 1);
  const m = () => METRICS.find((v) => v.id === metric())!;
  const scored = (c: Cell): Stats | null => (props.state.settings.split > 0 ? c.train : c.all);
  const score = (c: Cell): number => {
    const s = scored(c);
    return s && s.closed >= FEW ? m().get(s) : Number.NaN;
  };
  const best = createMemo(() => {
    let at = -1;
    let top = -Infinity;
    cells().forEach((c, i) => {
      const v = score(c);
      if (Number.isFinite(v) && v > top) {
        top = v;
        at = i;
      }
    });
    return at;
  });

  const run = async () => {
    const ax = x();
    if (!ax || !props.run) return;
    setError(null);
    const data = props.data();
    if (!data) {
      setError('The chart’s bars are not in yet.');
      return;
    }
    const n = data.bars.time.length;
    const split = props.state.settings.split > 0 ? Math.floor((n * props.state.settings.split) / 100) : 0;
    const base = current();
    const gx = xs();
    const gy = ys();
    const jobs: number[][] = [];
    for (const vy of gy ?? [null]) {
      for (const vx of gx) {
        const inputs = base.slice();
        inputs[ax.index] = vx;
        if (vy !== null && y()) inputs[y()!.index] = vy;
        jobs.push(inputs);
      }
    }
    abort?.abort();
    abort = new AbortController();
    batch(() => {
      setGrid({ xs: gx, ys: gy, xi: ax.index, yi: y()?.index ?? null });
      setCells(jobs.map((inputs) => ({ inputs, all: null, train: null, test: null })));
      setDone(0);
      setTotal(jobs.length);
      setPicked(null);
      setRunning(true);
    });
    const out = await optimize({ ...data, source: props.run.source, settings: props.run.settings, split }, jobs, (k, cell, index) => {
      setDone(k);
      setCells((list) => list.map((c, i) => (i === index ? { ...cell } : c)));
    }, abort.signal);
    batch(() => {
      setCells(out.map((c) => ({ ...c })));
      setRunning(false);
    });
  };

  const apply = (c: Cell) => {
    if (props.state.source === 'rules') {
      const { places } = rulesScript(props.state.rules);
      props.onChange({ rules: withSettings(props.state.rules, places, c.inputs), inputs: [] });
    } else props.onChange({ inputs: c.inputs });
  };

  const label = (index: number) => numbers().find((v) => v.index === index)?.inp.label ?? `Setting ${index + 1}`;
  const shade = (v: number, c: Cell) => {
    const s = scored(c);
    if (!Number.isFinite(v) || !s || s.closed < FEW) return undefined;
    const vals = cells().map(score).filter(Number.isFinite);
    const lo = Math.min(...vals);
    const hi = Math.max(...vals);
    const t = hi > lo ? (v - lo) / (hi - lo) : 0.5;
    return `color-mix(in oklab, var(${t >= 0.5 ? '--bull' : '--bear'}) ${Math.round(Math.abs(t - 0.5) * 2 * 50)}%, transparent)`;
  };

  return (
    <div class="bt-lab">
      <Show when={desc()} fallback={<p class="dim">Reading the strategy’s settings…</p>}>
        <Show when={numbers().length > 0} fallback={<p class="bt-hint dim">This strategy has no number settings to try. A script’s are its <code>input</code> lines; the rule builder makes one of each length and level.</p>}>
          <div class="bt-opt-form">
            <AxisRow title="Across" axis={x()} options={numbers()} onPick={(i) => pickAxis('x', i)} onChange={setX} />
            <AxisRow title="Down" axis={y()} options={numbers().filter((n) => n.index !== x()?.index)} optional onPick={(i) => (i < 0 ? setY(null) : pickAxis('y', i))} onChange={setY} />
            <label class="bt-row">
              <span>Best by</span>
              <select value={metric()} aria-label="What the best is" onChange={(e) => setMetric(e.currentTarget.value)}>
                <For each={METRICS}>{(v) => <option value={v.id}>{v.label}</option>}</For>
              </select>
            </label>
            <label class="bt-row">
              <span>Tune on</span>
              <input class="box num bt-num" type="number" min={0} max={95} step={5} value={props.state.settings.split} aria-label="Percent of the bars tuned on" onChange={(e) => props.onChange({ settings: { ...props.state.settings, split: Math.min(95, Math.max(0, Math.round(Number(e.currentTarget.value) || 0))) } })} />
              <span class="dim">% of the bars, the rest kept back to judge the pick · 0 tunes on all of them</span>
            </label>
            <div class="bt-row">
              <Show
                when={!running()}
                fallback={
                  <button class="dbtn" onClick={() => abort?.abort()}>
                    Stop · {done()} of {total()}
                  </button>
                }
              >
                <button class="dbtn primary" disabled={count() === 0 || !props.run} onClick={() => void run()}>
                  Run {count()} tests
                </button>
              </Show>
              <span class="dim">{count() > 300 ? 'a large grid: it takes a while on a long chart' : 'each over every bar the chart holds, several at once'}</span>
            </div>
            <Show when={error()}>{(e) => <p class="warn">{e()}</p>}</Show>
          </div>
          <Show when={cells().length > 0}>
            <Show when={running()}>
              <div class="bt-progress">
                <span style={{ width: `${(done() / Math.max(1, total())) * 100}%` }} />
              </div>
            </Show>
            <div class="bt-table-wrap">
              <table class="bt-heat num">
                <thead>
                  <tr>
                    <th class="dim">{grid().ys ? `${label(grid().yi!)} ↓ · ${label(grid().xi)} →` : label(grid().xi)}</th>
                    <Show when={grid().ys} fallback={<th>{m().label}{props.state.settings.split > 0 ? ', tuned part' : ''}</th>}>
                      <For each={grid().xs}>{(v) => <th>{v}</th>}</For>
                    </Show>
                  </tr>
                </thead>
                <tbody>
                  <Show
                    when={grid().ys}
                    fallback={
                      <For each={grid().xs}>
                        {(v, i) => {
                          const c = () => cells()[i()];
                          return (
                            <tr>
                              <th>{v}</th>
                              <td classList={{ best: best() === i(), sel: picked() === i(), few: !!c() && (scored(c()!)?.closed ?? 0) < FEW }} style={{ background: c() ? shade(score(c()!), c()!) : undefined }} onClick={() => setPicked(i())}>
                                {c()?.all ? m().show(score(c()!)) : c()?.error ? '!' : '…'}
                              </td>
                            </tr>
                          );
                        }}
                      </For>
                    }
                  >
                    {(gys) => (
                      <For each={gys()}>
                        {(vy, r) => (
                          <tr>
                            <th>{vy}</th>
                            <For each={grid().xs}>
                              {(_, col) => {
                                const k = () => r() * grid().xs.length + col();
                                const c = () => cells()[k()];
                                return (
                                  <td classList={{ best: best() === k(), sel: picked() === k(), few: !!c() && (scored(c()!)?.closed ?? 0) < FEW }} style={{ background: c() ? shade(score(c()!), c()!) : undefined }} onClick={() => setPicked(k())}>
                                    {c()?.all ? m().show(score(c()!)) : c()?.error ? '!' : '…'}
                                  </td>
                                );
                              }}
                            </For>
                          </tr>
                        )}
                      </For>
                    )}
                  </Show>
                </tbody>
              </table>
            </div>
            <Show when={picked() ?? (running() ? null : best() >= 0 ? best() : null)}>
              {(k) => {
                const c = () => cells()[k()];
                return (
                  <Show when={c()?.all}>
                    <div class="bt-pick num">
                      <span>
                        {grid().ys ? `${label(grid().xi)} ${c()!.inputs[grid().xi]} · ${label(grid().yi!)} ${c()!.inputs[grid().yi!]}` : `${label(grid().xi)} ${c()!.inputs[grid().xi]}`}
                        {k() === best() ? ' · the best' : ''}
                      </span>
                      <Show when={c()!.train} fallback={<PickLine name="All bars" s={c()!.all!} />}>
                        <PickLine name="Tuned part" s={c()!.train!} />
                        <PickLine name="Kept back" s={c()!.test!} />
                      </Show>
                      <button class="dbtn primary" onClick={() => apply(c()!)}>
                        Use these settings
                      </button>
                    </div>
                  </Show>
                );
              }}
            </Show>
            <p class="bt-hint dim">
              A broad area that does well is a pattern; a lone bright cell is most often luck. With part of the bars kept back, trust a pick that holds up there. Cells of fewer than {FEW} trades are faded and never the best.
            </p>
          </Show>
        </Show>
      </Show>
    </div>
  );
}

function PickLine(props: { name: string; s: Stats }) {
  return (
    <span>
      {props.name}: <b class={tone(props.s.net)}>{pct(props.s.netPct)}</b> · {props.s.trades} trades · win {pct(props.s.winRate, false)} · PF {ratio(props.s.profitFactor)} · DD {pct(-props.s.maxDdPct)} · Sharpe {ratio(props.s.sharpe)}
    </span>
  );
}

function AxisRow(props: { title: string; axis: Axis | null; options: { inp: FormulaInput; index: number }[]; optional?: boolean; onPick(index: number): void; onChange(a: Axis): void }) {
  const field = (key: 'from' | 'to' | 'by', label: string) => (
    <input class="box num bt-num" type="number" step="any" aria-label={`${props.title}: ${label}`} value={props.axis?.[key] ?? 0} onChange={(e) => props.axis && props.onChange({ ...props.axis, [key]: Number(e.currentTarget.value) })} />
  );
  return (
    <label class="bt-row">
      <span>{props.title}</span>
      <select value={props.axis ? String(props.axis.index) : '-1'} aria-label={`${props.title}: which setting`} onChange={(e) => props.onPick(Number(e.currentTarget.value))}>
        <Show when={props.optional}>
          <option value="-1">Nothing</option>
        </Show>
        <For each={props.options}>{(o) => <option value={String(o.index)}>{o.inp.label}</option>}</For>
      </select>
      <Show when={props.axis}>
        <span class="dim">from</span>
        {field('from', 'from')}
        <span class="dim">to</span>
        {field('to', 'to')}
        <span class="dim">by</span>
        {field('by', 'step')}
      </Show>
    </label>
  );
}

// ---------------------------------------------------------------- markets

interface MarketRow {
  market: string;
  stats: Stats | null;
  error?: string;
}

function Markets(props: TesterProps) {
  const candidates = createMemo(() => {
    const seen = new Set<string>([props.market]);
    for (const p of Object.values(doc.panels)) {
      if (p.type === 'watchlist') for (const m of p.markets) if (isChartMarket(m) && !isRatio(m) && parseMarket(m)?.venue !== 'index') seen.add(m);
      if (p.type === 'chart' && isChartMarket(p.market) && !isRatio(p.market)) seen.add(p.market);
    }
    return [...seen].slice(0, 40);
  });
  const [chosen, setChosen] = createSignal<ReadonlySet<string>>(new Set(candidates()));
  const [rows, setRows] = createSignal<MarketRow[]>([]);
  const [running, setRunning] = createSignal(false);
  let stop = false;
  onCleanup(() => (stop = true));
  const bars = () => Math.min(20_000, Math.max(300, props.bars));
  const run = async () => {
    if (!props.run) return;
    const list = candidates().filter((m) => chosen().has(m));
    stop = false;
    setRunning(true);
    setRows(list.map((market) => ({ market, stats: null })));
    for (const market of list) {
      if (stop) break;
      let row: MarketRow;
      try {
        const r = await backtestMarket({ ...props.run, settings: settingsOf(props.state.settings, market, Date.now()), draw: false, magnify: false }, market, props.timeframe, bars());
        row = r.ok ? { market, stats: r.equity.length > 0 ? statsOf(r, tradesOf(r.trades)) : null } : { market, stats: null, error: r.message };
      } catch (e) {
        row = { market, stats: null, error: e instanceof Error ? e.message : String(e) };
      }
      setRows((rs) => rs.map((x) => (x.market === market ? row : x)));
    }
    setRunning(false);
  };
  const sorted = createMemo(() => [...rows()].sort((a, b) => (b.stats?.netPct ?? -Infinity) - (a.stats?.netPct ?? -Infinity)));
  return (
    <div class="bt-lab">
      <p class="bt-hint dim">
        The strategy as it stands, on other markets’ {props.timeframe} bars, {bars().toLocaleString('en-US')} of each at most, with each exchange’s own fees. A strategy that works on one coin alone is most often fitted to it.
      </p>
      <div class="bt-markets-pick">
        <Index each={candidates()}>
          {(m) => (
            <label class="bt-check">
              <input
                type="checkbox"
                checked={chosen().has(m())}
                onChange={(e) => {
                  const next = new Set(chosen());
                  if (e.currentTarget.checked) next.add(m());
                  else next.delete(m());
                  setChosen(next);
                }}
              />
              {symbolOf(m())} <span class="dim">{parseMarket(m())?.venue.replace('-perp', ' perp').replace('-spot', ' spot')}</span>
            </label>
          )}
        </Index>
      </div>
      <div class="bt-row">
        <Show
          when={!running()}
          fallback={
            <button class="dbtn" onClick={() => (stop = true)}>
              Stop
            </button>
          }
        >
          <button class="dbtn primary" disabled={chosen().size === 0 || !props.run} onClick={() => void run()}>
            Test on {chosen().size} markets
          </button>
        </Show>
        <span class="dim">Markets come from your watchlists and charts.</span>
      </div>
      <Show when={rows().length > 0}>
        <div class="bt-table-wrap">
          <table class="bt-table num">
            <thead>
              <tr>
                <th>Market</th>
                <th>Net</th>
                <th>Buy and hold</th>
                <th>Max DD</th>
                <th>Trades</th>
                <th>Win</th>
                <th>PF</th>
                <th>Sharpe</th>
                <th>Net $</th>
              </tr>
            </thead>
            <tbody>
              <For each={sorted()}>
                {(r) => (
                  <tr onClick={() => props.onOpenMarket(r.market)} data-tip="Open it on this chart">
                    <td>
                      {symbolOf(r.market)} <span class="dim">{parseMarket(r.market)?.venue}</span>
                    </td>
                    <Show when={r.stats} fallback={<td colSpan={8} class={r.error ? 'warn' : 'dim'}>{r.error ?? (running() ? '…' : 'no bars')}</td>}>
                      {(s) => (
                        <>
                          <td class={tone(s().net)}>{pct(s().netPct)}</td>
                          <td class={tone(s().buyHoldPct)}>{pct(s().buyHoldPct)}</td>
                          <td class="bear">{pct(-s().maxDdPct)}</td>
                          <td>{s().trades}</td>
                          <td>{pct(s().winRate, false)}</td>
                          <td>{ratio(s().profitFactor)}</td>
                          <td>{ratio(s().sharpe)}</td>
                          <td class={tone(s().net)}>{money(s().net)}</td>
                        </>
                      )}
                    </Show>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}
