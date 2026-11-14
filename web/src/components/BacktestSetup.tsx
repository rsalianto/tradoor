/**
 * The strategy tester's Strategy tab: what is tested — rules picked from lists,
 * a script written here (from an example, or blank), or one of the library's —
 * and the account it is tested with: its size, costs, fills, stops and range.
 */
import { createEffect, createMemo, createResource, createSignal, For, Index, on, Show } from 'solid-js';
import type { FormulaDescription, FormulaInput } from '../engine/protocol';
import { EXAMPLES, isPerp, OP_LABEL, operand, OPERAND_INFO, rulesSource, venueFees } from '../lib/backtest';
import { MAX_CODE, MAX_RULES, OPERANDS, OPS, type BacktestSettings, type Condition, type ExitSetting, type Operand, type RuleSet, type Rules } from '../lib/backtest-state';
import { canWrite, community, describe, formulas, loadCommunity, setEditing, type Formula } from '../lib/formulas';
import { Icon } from '../lib/icons';
import { mendable, mendPaste, mendScript, type Mended } from '../lib/mend';
import type { TesterProps } from './StrategyTester';

const SOURCES = [
  { id: 'rules', label: 'Rules', tip: 'Pick conditions from lists: no code' },
  { id: 'code', label: 'Code', tip: 'Write it in Frost Script, or start from an example' },
  { id: 'script', label: 'Library', tip: 'One of your saved scripts, or a public one from the Community' },
];

const SETS: { key: keyof Rules; label: string }[] = [
  { key: 'long', label: 'Enter long when' },
  { key: 'short', label: 'Enter short when' },
  { key: 'exitLong', label: 'Exit a long when' },
  { key: 'exitShort', label: 'Exit a short when' },
];

const RANGES = [
  { days: 0, label: 'Every bar the chart holds' },
  { days: 30, label: 'The last 30 days' },
  { days: 90, label: 'The last 90 days' },
  { days: 180, label: 'The last 6 months' },
  { days: 365, label: 'The last year' },
  { days: 730, label: 'The last 2 years' },
  { days: 1095, label: 'The last 3 years' },
  { days: 1825, label: 'The last 5 years' },
  { days: -1, label: 'The market’s whole history' },
];

/** `2017-08-17`, UTC, for a date field. */
const dayOf = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/** A number field that keeps what is typed and gives a number back once it is one. */
function Num(props: { value: number; min?: number; max?: number; step?: number; label: string; wide?: boolean; onChange(v: number): void }) {
  return (
    <input
      class="box num bt-num"
      classList={{ wide: props.wide }}
      type="number"
      aria-label={props.label}
      value={props.value}
      min={props.min}
      max={props.max}
      step={props.step ?? 'any'}
      onChange={(e) => {
        const v = Number(e.currentTarget.value);
        if (!Number.isFinite(v)) return;
        const lo = props.min ?? -Infinity;
        const hi = props.max ?? Infinity;
        props.onChange(Math.min(hi, Math.max(lo, v)));
      }}
    />
  );
}

export function BacktestSetup(props: TesterProps) {
  const st = () => props.state;
  const set = (patch: Partial<BacktestSettings>) => props.onChange({ settings: { ...st().settings, ...patch } });
  return (
    <div class="bt-setup">
      <section class="bt-col">
        <div class="bt-seg" role="radiogroup" aria-label="What is tested">
          <For each={SOURCES}>
            {(s) => (
              <button role="radio" aria-checked={st().source === s.id} classList={{ on: st().source === s.id }} data-tip={s.tip} onClick={() => props.onChange({ source: s.id, inputs: [] })}>
                {s.label}
              </button>
            )}
          </For>
        </div>
        <Show when={st().source === 'rules'}>
          <RulesEditor {...props} />
        </Show>
        <Show when={st().source === 'code'}>
          <CodeEditor {...props} />
        </Show>
        <Show when={st().source === 'script'}>
          <LibraryPicker {...props} />
        </Show>
      </section>
      <section class="bt-col bt-account">
        <h4>Account and orders</h4>
        <label class="bt-row">
          <span>Capital</span>
          <Num value={st().settings.capital} min={1} label="Starting capital" wide onChange={(capital) => set({ capital })} />
          <span class="dim">quote</span>
        </label>
        <label class="bt-row">
          <span>Order size</span>
          <Num value={st().settings.size} min={0} label="Order size" onChange={(size) => set({ size })} />
          <select value={st().settings.sizeKind} aria-label="What the size counts" onChange={(e) => set({ sizeKind: e.currentTarget.value })}>
            <option value="equity">% of equity</option>
            <option value="fixed">quote, as margin</option>
            <option value="risk">% risked at the stop</option>
          </select>
        </label>
        <label class="bt-row">
          <span>Leverage</span>
          <Num value={st().settings.leverage} min={1} max={200} step={1} label="Leverage" onChange={(leverage) => set({ leverage })} />
          <span class="dim">× · {st().settings.leverage > 1 || isPerp(props.market) ? 'liquidated at the first tier’s margin' : 'no liquidation'}</span>
        </label>
        <label class="bt-row">
          <span>Sides</span>
          <select value={st().settings.side} aria-label="Which sides it trades" onChange={(e) => set({ side: e.currentTarget.value })}>
            <option value="both">Long and short</option>
            <option value="long">Long only</option>
            <option value="short">Short only</option>
          </select>
        </label>
        <label class="bt-row">
          <span>Filled at</span>
          <select value={st().settings.fill} aria-label="Where orders fill" onChange={(e) => set({ fill: e.currentTarget.value })}>
            <option value="open">The next bar’s open</option>
            <option value="close">The close it is given at</option>
          </select>
        </label>
        <div class="bt-row">
          <span>Fees</span>
          <label class="bt-check">
            <input type="checkbox" checked={st().settings.autoFees} onChange={(e) => set({ autoFees: e.currentTarget.checked })} />
            the exchange’s ({venueFees(props.market)[0]}% / {venueFees(props.market)[1]}%)
          </label>
        </div>
        <Show when={!st().settings.autoFees}>
          <label class="bt-row">
            <span>Taker, maker</span>
            <Num value={st().settings.taker} min={0} max={5} label="Taker fee, percent" onChange={(taker) => set({ taker })} />
            <Num value={st().settings.maker} min={-1} max={5} label="Maker fee, percent" onChange={(maker) => set({ maker })} />
            <span class="dim">%</span>
          </label>
        </Show>
        <label class="bt-row">
          <span>Slippage</span>
          <Num value={st().settings.slippage} min={0} max={10} label="Slippage, percent" onChange={(slippage) => set({ slippage })} />
          <span class="dim">% a market or stop fill</span>
        </label>
        <ExitRow label="Stop loss" value={st().settings.stop} kinds={[['none', 'None'], ['pct', '% of entry'], ['atr', 'ATRs']]} onChange={(stop) => set({ stop })} />
        <ExitRow label="Take profit" value={st().settings.target} kinds={[['none', 'None'], ['pct', '% of entry'], ['atr', 'ATRs'], ['r', 'R (× the stop)']]} onChange={(target) => set({ target })} />
        <ExitRow label="Trailing stop" value={st().settings.trail} kinds={[['none', 'None'], ['pct', '% from the best'], ['atr', 'ATRs']]} onChange={(trail) => set({ trail })} />
        <label class="bt-row">
          <span>ATR length</span>
          <Num value={st().settings.atr} min={1} max={500} step={1} label="ATR length" onChange={(atr) => set({ atr: Math.round(atr) })} />
        </label>
        <label class="bt-row">
          <span>Hold at most</span>
          <Num value={st().settings.maxBars} min={0} max={100000} step={1} label="Bars held at most" onChange={(maxBars) => set({ maxBars: Math.round(maxBars) })} />
          <span class="dim">bars · 0 for no limit</span>
        </label>
        <label class="bt-row">
          <span>Tested over</span>
          <select
            value={st().settings.since > 0 ? 'since' : String(st().settings.days)}
            aria-label="The stretch tested"
            onChange={(e) => {
              const v = e.currentTarget.value;
              // From a date: a year back to start with, changed in the field beside it.
              if (v === 'since') set({ since: Date.UTC(new Date().getUTCFullYear() - 1, 0, 1), days: 0 });
              else set({ days: Number(v), since: 0 });
            }}
          >
            <For each={RANGES}>{(r) => <option value={String(r.days)}>{r.label}</option>}</For>
            <option value="since">From a date…</option>
          </select>
          <Show when={st().settings.since > 0}>
            <input
              class="box num bt-date"
              type="date"
              aria-label="Tested from this date, UTC"
              value={dayOf(st().settings.since)}
              min="2009-01-03"
              max={dayOf(Date.now())}
              onChange={(e) => {
                const t = Date.parse(`${e.currentTarget.value}T00:00:00Z`);
                if (Number.isFinite(t)) set({ since: t });
              }}
            />
          </Show>
        </label>
        <p class="bt-hint dim">
          {props.bars.toLocaleString('en-US')} bars held{props.loading ? ', loading more…' : props.exhausted ? ', the exchange’s whole history' : ''}. A longer stretch loads older bars behind them, 100,000 at most: for a coin’s whole history, a daily or 4-hour chart holds it all.
        </p>
        <label class="bt-row">
          <span>Funding</span>
          <label class="bt-check">
            <input type="checkbox" checked={st().settings.funding} onChange={(e) => set({ funding: e.currentTarget.checked })} />
            paid as a perpetual settles, from its exchange’s history
          </label>
        </label>
        <label class="bt-row">
          <span>Inside bars</span>
          <label class="bt-check">
            <input type="checkbox" checked={st().settings.magnify} onChange={(e) => set({ magnify: e.currentTarget.checked })} />
            where a bar hit both the stop and the target, its lower-timeframe bars say which came first
          </label>
        </label>
      </section>
    </div>
  );
}

function ExitRow(props: { label: string; value: ExitSetting; kinds: [string, string][]; onChange(v: ExitSetting): void }) {
  return (
    <label class="bt-row">
      <span>{props.label}</span>
      <select value={props.value.kind} aria-label={`${props.label}: what it is measured in`} onChange={(e) => props.onChange({ ...props.value, kind: e.currentTarget.value })}>
        <For each={props.kinds}>{([v, l]) => <option value={v}>{l}</option>}</For>
      </select>
      <Show when={props.value.kind !== 'none'}>
        <Num value={props.value.v} min={0} max={10000} label={props.label} onChange={(v) => props.onChange({ ...props.value, v })} />
      </Show>
    </label>
  );
}

// ---------------------------------------------------------------- rules

function OperandPicker(props: { value: Operand; label: string; onChange(o: Operand): void }) {
  const info = () => OPERAND_INFO[props.value.kind];
  return (
    <span class="bt-operand">
      <select value={props.value.kind} aria-label={props.label} onChange={(e) => props.onChange(operand(e.currentTarget.value))}>
        <For each={OPERANDS}>{(k) => <option value={k}>{OPERAND_INFO[k]?.label ?? k}</option>}</For>
      </select>
      <Show when={info()?.n}>
        <Num value={props.value.n} label={`${props.label}: ${info()?.n}`} step={props.value.kind === 'value' ? undefined : 1} min={props.value.kind === 'value' ? undefined : 1} max={props.value.kind === 'value' ? undefined : 5000} onChange={(n) => props.onChange({ ...props.value, n })} />
      </Show>
      <Show when={info()?.m}>
        <Num value={props.value.m} label={`${props.label}: ${info()?.m}`} min={props.value.kind.startsWith('bb') ? 0.1 : 1} max={5000} onChange={(m) => props.onChange({ ...props.value, m })} />
      </Show>
      <Show when={info()?.k}>
        <Num value={props.value.k} label={`${props.label}: ${info()?.k}`} min={1} max={5000} step={1} onChange={(k) => props.onChange({ ...props.value, k })} />
      </Show>
    </span>
  );
}

function RuleSetEditor(props: { label: string; set: RuleSet; onChange(s: RuleSet): void }) {
  const change = (i: number, c: Condition) => props.onChange({ ...props.set, list: props.set.list.map((x, j) => (j === i ? c : x)) });
  return (
    <div class="bt-rules">
      <div class="bt-rules-head">
        <b>{props.label}</b>
        <Show when={props.set.list.length > 1}>
          <select value={props.set.all ? 'all' : 'any'} aria-label={`${props.label}: all or any`} onChange={(e) => props.onChange({ ...props.set, all: e.currentTarget.value === 'all' })}>
            <option value="all">all of these</option>
            <option value="any">any of these</option>
          </select>
        </Show>
        <Show when={props.set.list.length === 0}>
          <span class="dim">never</span>
        </Show>
      </div>
      <Index each={props.set.list}>
        {(c, i) => (
          <div class="bt-rule">
            <OperandPicker value={c().a} label="Value" onChange={(a) => change(i, { ...c(), a })} />
            <select value={c().op} aria-label="How they compare" onChange={(e) => change(i, { ...c(), op: e.currentTarget.value })}>
              <For each={OPS}>{(o) => <option value={o}>{OP_LABEL[o]}</option>}</For>
            </select>
            <OperandPicker value={c().b} label="Against" onChange={(b) => change(i, { ...c(), b })} />
            <button class="iconbtn" aria-label="Remove this condition" data-tip="Remove" onClick={() => props.onChange({ ...props.set, list: props.set.list.filter((_, j) => j !== i) })}>
              <Icon name="close" size={11} />
            </button>
          </div>
        )}
      </Index>
      <Show when={props.set.list.length < MAX_RULES}>
        <button class="linkbtn" onClick={() => props.onChange({ ...props.set, list: [...props.set.list, { a: operand('rsi'), op: 'lt', b: { kind: 'value', n: 30, m: 0, k: 0 } }] })}>
          + Add a condition
        </button>
      </Show>
    </div>
  );
}

function RulesEditor(props: TesterProps) {
  const rules = () => props.state.rules;
  return (
    <div class="bt-rules-all">
      <For each={SETS}>
        {(s) => <RuleSetEditor label={s.label} set={rules()[s.key]} onChange={(set) => props.onChange({ rules: { ...rules(), [s.key]: set }, inputs: [] })} />}
      </For>
      <p class="bt-hint dim">The other side’s entry also closes a position, reversing it; the stop, target and trailing stop are under Account and orders.</p>
      <button class="dbtn" data-tip="The same strategy as Frost Script, to change as code" onClick={() => props.onChange({ source: 'code', code: rulesSource(rules()), inputs: [] })}>
        Open as code
      </button>
    </div>
  );
}

// ---------------------------------------------------------------- code

/** A script's settings, as fields: numbers, tick boxes and choices; colours and moments as they are. */
export function InputFields(props: { inputs: FormulaInput[]; values: number[]; onChange(values: number[]): void }) {
  const value = (i: number) => props.values[i] ?? props.inputs[i]!.default;
  const put = (i: number, v: number) => {
    const next = props.inputs.map((inp, j) => (j === i ? v : (props.values[j] ?? inp.default)));
    props.onChange(next);
  };
  return (
    <Show when={props.inputs.length > 0}>
      <div class="bt-inputs">
        <Index each={props.inputs}>
          {(inp, i) => (
            <Show when={inp().kind !== 'colour'}>
              <label class="bt-row">
                <span>{inp().label}</span>
                <Show
                  when={inp().kind === 'bool'}
                  fallback={
                    <Show when={inp().kind === 'choice'} fallback={<Num value={value(i)} min={inp().min} max={inp().max} step={inp().step} label={inp().label} onChange={(v) => put(i, v)} />}>
                      <select value={String(value(i))} aria-label={inp().label} onChange={(e) => put(i, Number(e.currentTarget.value))}>
                        <For each={inp().options}>{(o, k) => <option value={String(k())}>{o}</option>}</For>
                      </select>
                    </Show>
                  }
                >
                  <input type="checkbox" checked={value(i) !== 0} aria-label={inp().label} onChange={(e) => put(i, e.currentTarget.checked ? 1 : 0)} />
                </Show>
              </label>
            </Show>
          )}
        </Index>
      </div>
    </Show>
  );
}

function CheckLine(props: { check: FormulaDescription | undefined }) {
  return (
    <Show when={props.check} fallback={<span class="dim">Checking…</span>}>
      {(c) => {
        const d = c();
        return d.ok ? (
          d.strategy ? (
            <span class="ok">✓ A strategy · {d.inputs.length} {d.inputs.length === 1 ? 'setting' : 'settings'}</span>
          ) : (
            <span class="warn">▲ It gives no orders: a strategy enters with long(condition) or short(condition).</span>
          )
        ) : (
          <span class="warn">
            ▲ {d.line > 0 ? `Line ${d.line}, column ${d.col}: ` : ''}
            {d.message}
          </span>
        );
      }}
    </Show>
  );
}

function CodeEditor(props: TesterProps) {
  const [text, setText] = createSignal(props.state.code);
  const [mended, setMended] = createSignal<Mended | null>(null);
  createEffect(on(() => props.state.code, (c) => c !== text() && setText(c), { defer: true }));
  let timer: ReturnType<typeof setTimeout> | undefined;
  const type = (v: string) => {
    setText(v);
    clearTimeout(timer);
    // Run again once typing stops, not on every key.
    timer = setTimeout(() => props.onChange({ code: v.slice(0, MAX_CODE) }), 500);
  };
  const [check] = createResource(() => props.state.code || null, (src) => describe(src));
  const inputs = () => {
    const c = check();
    return c?.ok ? c.inputs : [];
  };
  return (
    <div class="bt-code">
      <div class="bt-bar">
        <select
          aria-label="Start from an example"
          value=""
          onChange={(e) => {
            const ex = EXAMPLES.find((x) => x.id === e.currentTarget.value);
            if (ex) props.onChange({ code: ex.source, inputs: [] });
            e.currentTarget.value = '';
          }}
        >
          <option value="">Start from an example…</option>
          <For each={EXAMPLES}>{(x) => <option value={x.id}>{x.name}</option>}</For>
        </select>
        <span class="spacer" />
        <a class="linkbtn" href="/doc.html#strategies" target="_blank" rel="noopener">
          Strategy words ↗
        </a>
        <button class="dbtn" disabled={!canWrite() || !props.state.code} data-tip={canWrite() ? 'Save it to your scripts, in the script editor' : 'Sign in with Telegram to save scripts'} onClick={() => setEditing({ seed: { name: 'My strategy', source: props.state.code } })}>
          Save as a script
        </button>
      </div>
      <Show when={!props.state.code}>
        <p class="bt-hint dim">
          <code>long(condition)</code> and <code>short(condition)</code> enter, at the next open; <code>long(condition, price)</code> places a limit. <code>exit(condition)</code> closes, <code>stop(price)</code> and <code>target(price)</code> guard it. <code>inlong</code>, <code>inshort</code>, <code>entryprice</code> and <code>barsheld</code> read the position.
        </p>
      </Show>
      <textarea
        class="bt-src"
        spellcheck={false}
        value={text()}
        placeholder={'input length = 20\naverage = ema(close, length)\nlong(crossover(close, average))\nexit(crossunder(close, average))'}
        onInput={(e) => {
          // A paste from a chat that lost its line breaks: put back at once.
          if ((e as InputEvent).inputType === 'insertFromPaste') {
            const p = mendPaste(e.currentTarget.value);
            if (p.mended) {
              e.currentTarget.value = p.text;
              setMended({ text: p.text, changes: ['line breaks put back: the copy had lost them'], left: [] });
              type(p.text);
              return;
            }
          }
          setMended(null);
          type(e.currentTarget.value);
        }}
      />
      <Show when={!mended() && props.state.code && mendable(props.state.code)}>
        <div class="bt-bar">
          <span class="warn">This reads as a Pine Script conversion with Pine’s leftovers in it.</span>
          <button
            class="dbtn"
            onClick={() => {
              const m = mendScript(props.state.code);
              setMended(m);
              setText(m.text);
              props.onChange({ code: m.text.slice(0, MAX_CODE) });
            }}
          >
            Mend it
          </button>
        </div>
      </Show>
      <Show when={mended()}>
        {(m) => (
          <p class="bt-hint">
            Mended: {m().changes.join(' · ') || 'nothing to change'}.
            <For each={m().left}>{(l) => <span class="warn"> {l}.</span>}</For>
          </p>
        )}
      </Show>
      <div class="bt-check-line">
        <Show when={props.state.code}>
          <CheckLine check={check()} />
        </Show>
      </div>
      <InputFields inputs={inputs()} values={props.state.inputs} onChange={(values) => props.onChange({ inputs: values })} />
    </div>
  );
}

// ---------------------------------------------------------------- the library

function LibraryPicker(props: TesterProps) {
  void loadCommunity();
  const all = createMemo<Formula[]>(() => {
    const own = formulas();
    const shared = (community() ?? []).filter((c) => !own.some((o) => o.id === c.id)).map((c): Formula => ({ id: c.id, name: c.name, source: c.source ?? '', checks: true, updated: c.updated, removed: false, why: null, public: true, about: c.about, preview: c.preview, by: c.username }));
    return [...own, ...shared].filter((f) => !f.removed && f.source);
  });
  // Which of them are strategies: each checked once.
  const [kinds] = createResource(all, async (list) => {
    const out = new Map<string, FormulaDescription>();
    for (const f of list) out.set(f.id, await describe(f.source));
    return out;
  });
  const strategies = () => all().filter((f) => {
    const d = kinds()?.get(f.id);
    return d?.ok && d.strategy;
  });
  const chosen = () => all().find((f) => `fx-${f.id}` === props.state.script);
  const d = () => {
    const f = chosen();
    const c = f ? kinds()?.get(f.id) : undefined;
    return c?.ok ? c : null;
  };
  return (
    <div class="bt-library">
      <Show when={kinds()} fallback={<p class="dim">Reading your scripts and the Community’s…</p>}>
        <Show when={strategies().length > 0} fallback={<p class="bt-hint dim">No strategy among your scripts or the Community’s yet: a script is one once it gives orders, with long(…) or short(…). Write one under Code.</p>}>
          <label class="bt-row">
            <span>Script</span>
            <select value={props.state.script} aria-label="The script tested" onChange={(e) => props.onChange({ script: e.currentTarget.value, inputs: [] })}>
              <option value="">Choose one…</option>
              <For each={strategies()}>{(f) => <option value={`fx-${f.id}`}>{f.name}{f.by ? ` · @${f.by}` : ''}</option>}</For>
            </select>
          </label>
          <Show when={chosen()}>
            {(f) => (
              <>
                <Show when={f().about}>
                  <p class="bt-hint dim">{f().about}</p>
                </Show>
                <InputFields inputs={d()?.inputs ?? []} values={props.state.inputs} onChange={(values) => props.onChange({ inputs: values })} />
                <button class="dbtn" onClick={() => props.onChange({ source: 'code', code: f().source, inputs: props.state.inputs })}>
                  Copy into the code editor
                </button>
              </>
            )}
          </Show>
        </Show>
      </Show>
    </div>
  );
}
