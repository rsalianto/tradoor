/**
 * The strategy tester's Strategy tab, laid out as the steps of a test, top to
 * bottom: start from a template; which way to trade; when to buy and when to
 * sell short, each rule a sentence; how a trade is closed and guarded; then the
 * money, the stretch tested, and the costs and finer controls folded away. Each
 * change runs the test again at once. Code and the library's scripts are the
 * other ways in, for a strategy written as Frost Script.
 */
import { createEffect, createMemo, createResource, createSignal, For, Index, on, Show, type JSX } from 'solid-js';
import type { FormulaDescription, FormulaInput } from '../engine/protocol';
import { EXAMPLES, isPerp, OP_LABEL, operand, OPERAND_INFO, operandLabel, rulesSource, venueFees } from '../lib/backtest';
import { DEFAULT_RULES, MAX_CODE, MAX_RULES, OPS, type BacktestSettings, type Condition, type ExitSetting, type Operand, type RuleSet, type Rules } from '../lib/backtest-state';
import { canWrite, community, describe, formulas, loadCommunity, setEditing, type Formula } from '../lib/formulas';
import { Icon } from '../lib/icons';
import { mendable, mendPaste, mendScript, type Mended } from '../lib/mend';
import type { TesterProps } from './StrategyTester';

const SOURCES = [
  { id: 'rules', label: 'Build with rules', tip: 'Pick conditions from lists: no code' },
  { id: 'code', label: 'Write code', tip: 'Write it in Frost Script, start from an example, or paste one from an AI chat' },
  { id: 'script', label: 'My scripts', tip: 'One of your saved scripts, or a public strategy from the Community' },
];

/** The values a rule compares, grouped as a trader thinks of them, each with a word on what it is. */
const GROUPS: { label: string; kinds: [string, string][] }[] = [
  { label: 'Price', kinds: [['close', 'Price (close)'], ['open', 'Open'], ['high', 'High'], ['low', 'Low'], ['hl2', 'Middle of the bar'], ['value', 'A number']] },
  { label: 'Averages', kinds: [['ema', 'EMA, exponential average'], ['sma', 'SMA, simple average'], ['wma', 'WMA, weighted average'], ['vwma', 'VWMA, volume-weighted average']] },
  { label: 'Momentum', kinds: [['rsi', 'RSI, 0 to 100'], ['stochk', 'Stochastic %K, 0 to 100'], ['stochd', 'Stochastic %D, 0 to 100'], ['mfi', 'MFI, 0 to 100'], ['cci', 'CCI, around 0'], ['roc', 'Rate of change %'], ['macd', 'MACD line'], ['macdsignal', 'MACD signal line'], ['macdhist', 'MACD histogram']] },
  { label: 'Volatility', kinds: [['bbupper', 'Bollinger upper band'], ['bbbasis', 'Bollinger middle'], ['bblower', 'Bollinger lower band'], ['atr', 'ATR, the average range']] },
  { label: 'Breakouts', kinds: [['highest', 'Highest high of the bars before'], ['lowest', 'Lowest low of the bars before']] },
  { label: 'Volume', kinds: [['volume', 'Volume'], ['volsma', 'Average volume']] },
];

/** What a value is measured in: comparing two of different scales means nothing. */
const SCALE: Record<string, string> = {
  close: 'price', open: 'price', high: 'price', low: 'price', hl2: 'price', sma: 'price', ema: 'price', wma: 'price', vwma: 'price', bbupper: 'price', bbbasis: 'price', bblower: 'price', highest: 'price', lowest: 'price',
  rsi: 'pct', stochk: 'pct', stochd: 'pct', mfi: 'pct',
  macd: 'macd', macdsignal: 'macd', macdhist: 'macd', cci: 'cci', roc: 'roc', atr: 'atr', volume: 'volume', volsma: 'volume',
};

/** What a value of each scale is set against, where the other side no longer fits: buying, then selling. */
function against(kind: string, buying: boolean): Operand {
  const v = (n: number): Operand => ({ kind: 'value', n, m: 0, k: 0 });
  switch (SCALE[kind]) {
    case 'price':
      return kind === 'close' ? operand('ema') : operand('close');
    case 'pct':
      return v(buying ? 30 : 70);
    case 'macd':
      return kind === 'macdsignal' ? operand('macd') : kind === 'macdhist' ? v(0) : operand('macdsignal');
    case 'cci':
      return v(buying ? -100 : 100);
    case 'roc':
      return v(0);
    case 'volume':
      return kind === 'volsma' ? operand('volume') : operand('volsma');
    default:
      return v(0);
  }
}

const fits = (a: Operand, b: Operand) => b.kind === 'value' || SCALE[a.kind] === SCALE[b.kind];

const op = (kind: string, n = 0, m = 0, k = 0): Operand => ({ ...operand(kind), ...(n ? { n } : {}), ...(m ? { m } : {}), ...(k ? { k } : {}) });
const rule = (a: Operand, o: string, b: Operand): Condition => ({ a, op: o, b });
const none = (): RuleSet => ({ all: true, list: [] });

/** Strategies to start from, written as rules a reader can then change. */
const TEMPLATES: { id: string; name: string; about: string; side: string; rules: () => Rules; settings: Partial<BacktestSettings> }[] = [
  {
    id: 'trend',
    name: 'Follow the trend',
    about: 'Buys when the fast average crosses above the slow one, sells short when it crosses below. Always in the market.',
    side: 'both',
    rules: () => DEFAULT_RULES(),
    settings: { stop: { kind: 'none', v: 2 }, target: { kind: 'none', v: 2 }, trail: { kind: 'none', v: 3 } },
  },
  {
    id: 'dip',
    name: 'Buy the dip',
    about: 'Buys when RSI climbs back above 30 after being oversold, sells when RSI reaches 70. A 3% stop under it.',
    side: 'long',
    rules: () => ({ long: { all: true, list: [rule(op('rsi', 14), 'xa', op('value', 30))] }, short: none(), exitLong: { all: true, list: [rule(op('rsi', 14), 'gt', op('value', 70))] }, exitShort: none() }),
    settings: { stop: { kind: 'pct', v: 3 }, target: { kind: 'none', v: 2 }, trail: { kind: 'none', v: 3 } },
  },
  {
    id: 'breakout',
    name: 'Breakout',
    about: 'Buys when the price breaks above the highest high of the last 20 bars, sells when it falls under the lowest low of the last 10.',
    side: 'long',
    rules: () => ({ long: { all: true, list: [rule(op('close'), 'xa', op('highest', 20))] }, short: none(), exitLong: { all: true, list: [rule(op('close'), 'xb', op('lowest', 10))] }, exitShort: none() }),
    settings: { stop: { kind: 'none', v: 2 }, target: { kind: 'none', v: 2 }, trail: { kind: 'none', v: 3 } },
  },
  {
    id: 'bounce',
    name: 'Bollinger bounce',
    about: 'Buys when the price comes back above the lower Bollinger band, sells at the middle line. A 2% stop.',
    side: 'long',
    rules: () => ({ long: { all: true, list: [rule(op('close'), 'xa', op('bblower', 20, 2))] }, short: none(), exitLong: { all: true, list: [rule(op('close'), 'xa', op('bbbasis', 20, 2))] }, exitShort: none() }),
    settings: { stop: { kind: 'pct', v: 2 }, target: { kind: 'none', v: 2 }, trail: { kind: 'none', v: 3 } },
  },
  {
    id: 'macd',
    name: 'MACD with the trend',
    about: 'MACD crossing its signal line, taken only on the side of the 200 EMA. A stop of 2 ATRs and a target twice as far.',
    side: 'both',
    rules: () => ({
      long: { all: true, list: [rule(op('macd', 12, 26, 9), 'xa', op('macdsignal', 12, 26, 9)), rule(op('close'), 'gt', op('ema', 200))] },
      short: { all: true, list: [rule(op('macd', 12, 26, 9), 'xb', op('macdsignal', 12, 26, 9)), rule(op('close'), 'lt', op('ema', 200))] },
      exitLong: none(),
      exitShort: none(),
    }),
    settings: { stop: { kind: 'atr', v: 2 }, target: { kind: 'r', v: 2 }, trail: { kind: 'none', v: 3 } },
  },
];

const RANGES: { days: number; label: string }[] = [
  { days: 0, label: 'What the chart has' },
  { days: 90, label: '3 months' },
  { days: 180, label: '6 months' },
  { days: 365, label: '1 year' },
  { days: 730, label: '2 years' },
  { days: 1825, label: '5 years' },
  { days: -1, label: 'Everything' },
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

/** A row of choices, one on. */
function Chips<T extends string | number>(props: { label: string; value: T; options: { value: T; label: string; tip?: string }[]; onChange(v: T): void }) {
  return (
    <div class="bt-chips" role="radiogroup" aria-label={props.label}>
      <For each={props.options}>
        {(o) => (
          <button role="radio" aria-checked={props.value === o.value} classList={{ on: props.value === o.value }} data-tip={o.tip} onClick={() => props.onChange(o.value)}>
            {o.label}
          </button>
        )}
      </For>
    </div>
  );
}

/** One step of the tab: a mark, a title, a line on what it is for, and its controls. */
function Step(props: { title: string; hint?: string; children: JSX.Element }) {
  return (
    <div class="bt-step">
      <div class="bt-step-head">
        <span class="bt-step-mark" aria-hidden="true" />
        <b>{props.title}</b>
        <Show when={props.hint}>
          <span class="dim">{props.hint}</span>
        </Show>
      </div>
      {props.children}
    </div>
  );
}

export function BacktestSetup(props: TesterProps) {
  const st = () => props.state;
  const set = (patch: Partial<BacktestSettings>) => props.onChange({ settings: { ...st().settings, ...patch } });
  return (
    <div class="bt-setup">
      <section class="bt-col">
        <div class="bt-seg" role="radiogroup" aria-label="How the strategy is given">
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
          <Step title="Which way it trades" hint="the code’s own orders, filtered here">
            <SideChips {...props} />
          </Step>
          <CodeEditor {...props} />
        </Show>
        <Show when={st().source === 'script'}>
          <LibraryPicker {...props} />
        </Show>
        <Protect {...props} />
      </section>
      <section class="bt-col">
        <Money {...props} set={set} />
        <Step title="Tested over" hint={`${props.bars.toLocaleString('en-US')} bars held${props.loading ? ', loading more…' : props.exhausted ? ', the exchange’s whole history' : ''}`}>
          <Chips
            label="The stretch tested"
            value={st().settings.since > 0 ? -2 : st().settings.days}
            options={[...RANGES.map((r) => ({ value: r.days, label: r.label })), { value: -2, label: 'From a date' }]}
            onChange={(v) => (v === -2 ? set({ since: Date.UTC(new Date().getUTCFullYear() - 1, 0, 1), days: 0 }) : set({ days: v, since: 0 }))}
          />
          <Show when={st().settings.since > 0}>
            <label class="bt-row">
              <span>From</span>
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
            </label>
          </Show>
          <p class="bt-hint dim">A longer stretch loads older bars, 100,000 at most: for a coin’s whole history, a daily or 4-hour chart holds it all.</p>
        </Step>
        <Realism {...props} set={set} />
      </section>
    </div>
  );
}

function SideChips(props: TesterProps) {
  const s = () => props.state.settings;
  return (
    <Chips
      label="Which way it trades"
      value={s().side}
      options={[
        { value: 'long', label: 'Buy only', tip: 'Long: makes money when the price rises' },
        { value: 'short', label: 'Sell short only', tip: 'Short: makes money when the price falls' },
        { value: 'both', label: 'Both ways', tip: 'Long on a buy signal, short on a sell signal, each closing the other' },
      ]}
      onChange={(side) => props.onChange({ settings: { ...s(), side } })}
    />
  );
}

// ---------------------------------------------------------------- rules

/** A value a rule compares, picked from groups, with the settings it takes. */
function OperandPicker(props: { value: Operand; label: string; onChange(o: Operand): void }) {
  const info = () => OPERAND_INFO[props.value.kind];
  return (
    <span class="bt-operand">
      <select value={props.value.kind} aria-label={props.label} onChange={(e) => props.onChange(operand(e.currentTarget.value))}>
        <For each={GROUPS}>
          {(g) => (
            <optgroup label={g.label}>
              <For each={g.kinds}>{([k, l]) => <option value={k}>{l}</option>}</For>
            </optgroup>
          )}
        </For>
      </select>
      <Show when={info()?.n}>
        <label class="bt-param" data-tip={info()?.n}>
          <Show when={props.value.kind !== 'value'}>
            <span class="dim">{(info()?.n ?? '').toLowerCase()}</span>
          </Show>
          <Num value={props.value.n} label={`${props.label}: ${info()?.n}`} step={props.value.kind === 'value' ? undefined : 1} min={props.value.kind === 'value' ? undefined : 1} max={props.value.kind === 'value' ? undefined : 5000} onChange={(n) => props.onChange({ ...props.value, n })} />
        </label>
      </Show>
      <Show when={info()?.m}>
        <label class="bt-param" data-tip={info()?.m}>
          <span class="dim">{(info()?.m ?? '').toLowerCase()}</span>
          <Num value={props.value.m} label={`${props.label}: ${info()?.m}`} min={props.value.kind.startsWith('bb') ? 0.1 : 1} max={5000} onChange={(m) => props.onChange({ ...props.value, m })} />
        </label>
      </Show>
      <Show when={info()?.k}>
        <label class="bt-param" data-tip={info()?.k}>
          <span class="dim">{(info()?.k ?? '').toLowerCase()}</span>
          <Num value={props.value.k} label={`${props.label}: ${info()?.k}`} min={1} max={5000} step={1} onChange={(k) => props.onChange({ ...props.value, k })} />
        </label>
      </Show>
    </span>
  );
}

/** A rule set as sentences: each condition a row, joined by and or or, and what they come to in words. */
function RuleSetEditor(props: { verb: string; buying: boolean; set: RuleSet; optional?: boolean; onChange(s: RuleSet): void }) {
  const change = (i: number, c: Condition) => props.onChange({ ...props.set, list: props.set.list.map((x, j) => (j === i ? c : x)) });
  const sentence = () =>
    props.set.list.map((c) => `${operandLabel(c.a)} ${OP_LABEL[c.op]} ${operandLabel(c.b)}`).join(props.set.all ? ', and ' : ', or ');
  return (
    <div class="bt-rules">
      <Index each={props.set.list}>
        {(c, i) => (
          <>
            <Show when={i > 0}>
              <button class="bt-join" data-tip={props.set.all ? 'Every condition must be true on the same bar · click for any one' : 'Any one condition is enough · click to need every one'} onClick={() => props.onChange({ ...props.set, all: !props.set.all })}>
                {props.set.all ? 'and' : 'or'}
              </button>
            </Show>
            <div class="bt-rule">
              <OperandPicker
                value={c().a}
                label="Value"
                onChange={(a) => change(i, { ...c(), a, b: fits(a, c().b) ? c().b : against(a.kind, props.buying) })}
              />
              <select value={c().op} aria-label="How they compare" onChange={(e) => change(i, { ...c(), op: e.currentTarget.value })}>
                <For each={OPS}>{(o) => <option value={o}>{OP_LABEL[o]}</option>}</For>
              </select>
              <OperandPicker value={c().b} label="Against" onChange={(b) => change(i, { ...c(), b })} />
              <button class="iconbtn" aria-label="Remove this condition" data-tip="Remove" onClick={() => props.onChange({ ...props.set, list: props.set.list.filter((_, j) => j !== i) })}>
                <Icon name="close" size={11} />
              </button>
            </div>
          </>
        )}
      </Index>
      <div class="bt-rules-foot">
        <Show when={props.set.list.length < MAX_RULES}>
          <button
            class="linkbtn"
            onClick={() => {
              const first = props.set.list[0];
              const c: Condition = first ? { a: operand('rsi'), op: props.buying ? 'lt' : 'gt', b: { kind: 'value', n: props.buying ? 30 : 70, m: 0, k: 0 } } : { a: operand('close'), op: 'xa', b: operand('ema') };
              props.onChange({ ...props.set, list: [...props.set.list, c] });
            }}
          >
            + {props.set.list.length === 0 ? (props.optional ? 'Add a rule' : 'Add a condition') : 'Another condition'}
          </button>
        </Show>
        <span class="dim bt-sentence">{props.set.list.length === 0 ? (props.optional ? '' : 'Never: add a condition') : `${props.verb} when ${sentence()}.`}</span>
      </div>
    </div>
  );
}

function RulesEditor(props: TesterProps) {
  const rules = () => props.state.rules;
  const side = () => props.state.settings.side;
  const put = (key: keyof Rules, s: RuleSet) => props.onChange({ rules: { ...rules(), [key]: s }, inputs: [] });
  const [exitsOpen, setExitsOpen] = createSignal(false);
  const exits = () => exitsOpen() || rules().exitLong.list.length > 0 || rules().exitShort.list.length > 0;
  return (
    <div class="bt-rules-all">
      <Step title="Start from a template" hint="then change the rules as you like">
        <div class="bt-templates">
          <For each={TEMPLATES}>
            {(t) => (
              <button
                class="bt-template"
                data-tip={t.about}
                onClick={() => props.onChange({ rules: t.rules(), inputs: [], settings: { ...props.state.settings, side: t.side, ...t.settings } })}
              >
                {t.name}
              </button>
            )}
          </For>
        </div>
      </Step>
      <Step title="Which way it trades">
        <SideChips {...props} />
      </Step>
      <Step title="When it trades" hint="read at each bar’s close, filled at the next open">
        <Show when={side() !== 'short'}>
          <p class="bt-say">Buy when</p>
          <RuleSetEditor verb="Buys" buying set={rules().long} onChange={(s) => put('long', s)} />
        </Show>
        <Show when={side() !== 'long'}>
          <p class="bt-say">Sell short when</p>
          <RuleSetEditor verb="Sells short" buying={false} set={rules().short} onChange={(s) => put('short', s)} />
        </Show>
        <Show
          when={exits()}
          fallback={
            <p class="bt-hint dim">
              {side() === 'both' ? 'A trade closes when the other side’s signal comes, or at a stop or target below.' : 'A trade closes at a stop or target below.'}{' '}
              <button class="linkbtn" onClick={() => setExitsOpen(true)}>
                Close it on a rule too
              </button>
            </p>
          }
        >
          <Show when={side() !== 'short'}>
            <p class="bt-say">Close a buy when</p>
            <RuleSetEditor verb="Closes a buy" buying={false} optional set={rules().exitLong} onChange={(s) => put('exitLong', s)} />
          </Show>
          <Show when={side() !== 'long'}>
            <p class="bt-say">Close a short when</p>
            <RuleSetEditor verb="Closes a short" buying optional set={rules().exitShort} onChange={(s) => put('exitShort', s)} />
          </Show>
        </Show>
        <button class="linkbtn" data-tip="The same strategy as Frost Script, to take further as code" onClick={() => props.onChange({ source: 'code', code: rulesSource(rules()), inputs: [] })}>
          Open these rules as code
        </button>
      </Step>
    </div>
  );
}

// ---------------------------------------------------------------- guarding a trade

/** One of a stop's, target's or trailing stop's ready choices, or one set by hand. */
function ExitChoice(props: { label: string; value: ExitSetting; presets: { value: ExitSetting; label: string; tip?: string }[]; kinds: [string, string][]; note?: string; onChange(v: ExitSetting): void }) {
  const same = (a: ExitSetting, b: ExitSetting) => a.kind === b.kind && (a.kind === 'none' || a.v === b.v);
  const picked = () => props.presets.findIndex((p) => same(p.value, props.value));
  const [custom, setCustom] = createSignal(false);
  const showCustom = () => custom() || picked() < 0;
  return (
    <div class="bt-exit">
      <span class="bt-exit-label">{props.label}</span>
      <div class="bt-chips">
        <For each={props.presets}>
          {(p, i) => (
            <button classList={{ on: !showCustom() && picked() === i() }} data-tip={p.tip} onClick={() => (setCustom(false), props.onChange(p.value))}>
              {p.label}
            </button>
          )}
        </For>
        <button classList={{ on: showCustom() }} onClick={() => setCustom(true)}>
          Other…
        </button>
      </div>
      <Show when={showCustom()}>
        <span class="bt-custom">
          <select value={props.value.kind} aria-label={`${props.label}: measured in`} onChange={(e) => props.onChange({ ...props.value, kind: e.currentTarget.value })}>
            <For each={props.kinds}>{([v, l]) => <option value={v}>{l}</option>}</For>
          </select>
          <Show when={props.value.kind !== 'none'}>
            <Num value={props.value.v} min={0} max={10000} label={props.label} onChange={(v) => props.onChange({ ...props.value, v })} />
          </Show>
        </span>
      </Show>
      <Show when={props.note}>
        <span class="warn bt-exit-note">{props.note}</span>
      </Show>
    </div>
  );
}

function Protect(props: TesterProps) {
  const s = () => props.state.settings;
  const set = (patch: Partial<BacktestSettings>) => props.onChange({ settings: { ...s(), ...patch } });
  const pct = (v: number): ExitSetting => ({ kind: 'pct', v });
  const off: ExitSetting = { kind: 'none', v: 0 };
  return (
    <Step title="Close the trade, and guard it" hint="whichever comes first">
      <ExitChoice
        label="Stop loss"
        value={s().stop}
        presets={[
          { value: off, label: 'None' },
          { value: pct(1), label: '1%' },
          { value: pct(2), label: '2%' },
          { value: pct(3), label: '3%' },
          { value: pct(5), label: '5%' },
          { value: { kind: 'atr', v: 2 }, label: '2 ATR', tip: 'Twice the average bar range: wider when the market is wild' },
        ]}
        kinds={[['none', 'None'], ['pct', '% from the entry'], ['atr', 'ATRs from the entry']]}
        onChange={(stop) => set({ stop })}
      />
      <ExitChoice
        label="Take profit"
        value={s().target}
        presets={[
          { value: off, label: 'None' },
          { value: { kind: 'r', v: 1 }, label: '1× the stop', tip: 'As far above the entry as the stop is below it' },
          { value: { kind: 'r', v: 2 }, label: '2× the stop', tip: 'Risk 1 to make 2' },
          { value: { kind: 'r', v: 3 }, label: '3× the stop', tip: 'Risk 1 to make 3' },
          { value: pct(5), label: '5%' },
          { value: pct(10), label: '10%' },
        ]}
        kinds={[['none', 'None'], ['pct', '% from the entry'], ['atr', 'ATRs from the entry'], ['r', '× the stop’s distance']]}
        note={s().target.kind === 'r' && s().stop.kind === 'none' ? 'A target as a multiple of the stop needs a stop loss.' : undefined}
        onChange={(target) => set({ target })}
      />
      <ExitChoice
        label="Trailing stop"
        value={s().trail}
        presets={[
          { value: off, label: 'None' },
          { value: pct(2), label: '2%', tip: 'Follows the best price, 2% behind it' },
          { value: pct(5), label: '5%' },
          { value: { kind: 'atr', v: 3 }, label: '3 ATR' },
        ]}
        kinds={[['none', 'None'], ['pct', '% behind the best price'], ['atr', 'ATRs behind the best price']]}
        onChange={(trail) => set({ trail })}
      />
    </Step>
  );
}

// ---------------------------------------------------------------- money

function Money(props: TesterProps & { set(p: Partial<BacktestSettings>): void }) {
  const s = () => props.state.settings;
  const per = () => (s().sizeKind === 'equity' ? (s().size === 100 ? 'all' : s().size === 50 ? 'half' : 'share') : s().sizeKind);
  // Roughly how far against a leveraged trade the margin is gone.
  const liquidation = () => (s().leverage > 1 ? Math.max(0, 100 / s().leverage - 0.5) : null);
  return (
    <Step title="Your money">
      <label class="bt-row">
        <span>Starting with</span>
        <Num value={s().capital} min={1} label="Starting balance" wide onChange={(capital) => props.set({ capital })} />
        <span class="dim">in the market’s quote currency</span>
      </label>
      <div class="bt-row">
        <span>Each trade uses</span>
        <select
          value={per()}
          aria-label="How much each trade uses"
          onChange={(e) => {
            const v = e.currentTarget.value;
            if (v === 'all') props.set({ sizeKind: 'equity', size: 100 });
            else if (v === 'half') props.set({ sizeKind: 'equity', size: 50 });
            else if (v === 'share') props.set({ sizeKind: 'equity', size: 25 });
            else if (v === 'fixed') props.set({ sizeKind: 'fixed', size: Math.round(s().capital / 10) });
            else props.set({ sizeKind: 'risk', size: 1 });
          }}
        >
          <option value="all">all of the balance</option>
          <option value="half">half of the balance</option>
          <option value="share">a share of the balance…</option>
          <option value="fixed">a fixed amount…</option>
          <option value="risk">as much as loses a set share at the stop…</option>
        </select>
        <Show when={per() === 'share' || per() === 'fixed' || per() === 'risk'}>
          <Num value={s().size} min={0} label="Order size" onChange={(size) => props.set({ size })} />
          <span class="dim">{per() === 'fixed' ? 'quote' : '%'}</span>
        </Show>
      </div>
      <Show when={per() === 'risk'}>
        <p class="bt-hint dim">Sized so that the stop loss, when hit, loses {s().size}% of the balance: it needs a stop loss.</p>
      </Show>
      <div class="bt-row">
        <span>Leverage</span>
        <Chips label="Leverage" value={[1, 2, 3, 5, 10, 20].includes(s().leverage) ? s().leverage : -1} options={[1, 2, 3, 5, 10, 20].map((v) => ({ value: v, label: `${v}×` })).concat([{ value: -1, label: 'Other' }])} onChange={(v) => props.set({ leverage: v === -1 ? 50 : v })} />
        <Show when={![1, 2, 3, 5, 10, 20].includes(s().leverage)}>
          <Num value={s().leverage} min={1} max={200} step={1} label="Leverage" onChange={(leverage) => props.set({ leverage })} />
        </Show>
      </div>
      <Show when={liquidation()}>
        {(l) => <p class="bt-hint warn">At {s().leverage}× a move of about {l().toFixed(1)}% against a trade loses its whole margin: the test counts that as liquidation.</p>}
      </Show>
    </Step>
  );
}

// ---------------------------------------------------------------- costs and finer controls

function Realism(props: TesterProps & { set(p: Partial<BacktestSettings>): void }) {
  const s = () => props.state.settings;
  const fees = () => (s().autoFees ? venueFees(props.market) : [s().taker, s().maker]);
  const summary = () =>
    [
      `fees ${fees()[0]}% / ${fees()[1]}%`,
      `slippage ${s().slippage}%`,
      s().fill === 'close' ? 'filled at the close' : 'filled at the next open',
      isPerp(props.market) ? (s().funding ? 'funding paid' : 'no funding') : null,
      s().magnify ? 'checked inside bars' : null,
    ]
      .filter(Boolean)
      .join(' · ');
  return (
    <>
      <details class="bt-fold">
        <summary>
          <b>Costs and realism</b> <span class="dim">{summary()}</span>
        </summary>
        <div class="bt-row">
          <span>Fees</span>
          <label class="bt-check">
            <input type="checkbox" checked={s().autoFees} onChange={(e) => props.set({ autoFees: e.currentTarget.checked })} />
            this exchange’s own ({venueFees(props.market)[0]}% a market order, {venueFees(props.market)[1]}% a limit one)
          </label>
        </div>
        <Show when={!s().autoFees}>
          <label class="bt-row">
            <span>My fees</span>
            <Num value={s().taker} min={0} max={5} label="Market order fee, percent" onChange={(taker) => props.set({ taker })} />
            <Num value={s().maker} min={-1} max={5} label="Limit order fee, percent" onChange={(maker) => props.set({ maker })} />
            <span class="dim">% market, limit</span>
          </label>
        </Show>
        <label class="bt-row" data-tip="What a market order loses to the order book: a fill a little worse than the price seen">
          <span>Slippage</span>
          <Num value={s().slippage} min={0} max={10} label="Slippage, percent" onChange={(slippage) => props.set({ slippage })} />
          <span class="dim">% on each market or stop fill</span>
        </label>
        <div class="bt-row">
          <span>Orders fill</span>
          <Chips
            label="Where orders fill"
            value={s().fill}
            options={[
              { value: 'open', label: 'At the next bar’s open', tip: 'As it would trade live: the signal is only known once its bar closes' },
              { value: 'close', label: 'At the signal bar’s close', tip: 'A little sooner than a live trade could' },
            ]}
            onChange={(fill) => props.set({ fill })}
          />
        </div>
        <Show when={isPerp(props.market)}>
          <label class="bt-row">
            <span>Funding</span>
            <label class="bt-check">
              <input type="checkbox" checked={s().funding} onChange={(e) => props.set({ funding: e.currentTarget.checked })} />
              paid and taken as the perpetual settles, from its exchange’s history
            </label>
          </label>
        </Show>
        <label class="bt-row">
          <span>Inside bars</span>
          <label class="bt-check">
            <input type="checkbox" checked={s().magnify} onChange={(e) => props.set({ magnify: e.currentTarget.checked })} />
            where one bar hit both the stop and the target, its 1-minute bars say which came first
          </label>
        </label>
      </details>
      <details class="bt-fold">
        <summary>
          <b>More controls</b> <span class="dim">ATR {s().atr} · {s().maxBars > 0 ? `held ${s().maxBars} bars at most` : 'held as long as it takes'}</span>
        </summary>
        <label class="bt-row">
          <span>ATR length</span>
          <Num value={s().atr} min={1} max={500} step={1} label="ATR length" onChange={(atr) => props.set({ atr: Math.round(atr) })} />
          <span class="dim">bars, for stops set in ATRs</span>
        </label>
        <label class="bt-row">
          <span>Hold at most</span>
          <Num value={s().maxBars} min={0} max={100000} step={1} label="Bars held at most" onChange={(maxBars) => props.set({ maxBars: Math.round(maxBars) })} />
          <span class="dim">bars · 0 for no limit</span>
        </label>
      </details>
    </>
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
