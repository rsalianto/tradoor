/**
 * A long or short position's Inputs, laid out as rsalianto's trade form: the
 * strategy it was taken on, its prices and size, what those come to, the legs
 * drawn with it — partial take profits, partial cut losses, added entries — and,
 * for a browser signed in to keep a journal, its notes and a close by hand. The
 * prices, the size and the legs are the drawing's and show on the chart as they
 * are typed; the strategy, the notes and the close are the journal's trade's,
 * saved once the drawing is.
 */
import { createMemo, createSignal, For, Index, type JSX, onMount, Show } from 'solid-js';
import type { DrawingPoint } from '../engine/protocol';
import { api } from '../lib/api';
import type { Style } from '../lib/drawing-style';
import { Icon } from '../lib/icons';
import { cents, coinsFor, figures, figuresOf, fits, fullEntry, lossPerCoin, marginFor, MAX_LEGS, parseLegs, serializeLegs, type DrawnLeg } from '../journal/legs';
import { book, FILL_LABEL, journalApi, localInput, money, pnl, rMultiple, rText, stamp, STATUS_LABEL, type LegKind, type Trade, type TradePatch } from '../journal/model';
import { Readout } from '../journal/Readout';
import { StrategyPicker } from '../journal/strategies';
import { NumField } from './NumField';

const LEG_GROUPS: { kind: LegKind; title: string; tip: string }[] = [
  { kind: 'take', title: 'Partial take profit', tip: 'A partial take profit: part of the position off on the way to the target' },
  { kind: 'cut', title: 'Partial cut loss', tip: 'A partial cut loss: part of the position cut on the way to the stop' },
  { kind: 'add', title: 'Average entry', tip: 'An added entry: more bought (sold, for a short) at another price, averaging the entry' },
];
const WHERE: Record<LegKind, string> = {
  take: 'between the entry and the target',
  cut: 'between the entry and the stop',
  add: 'between the stop and the target',
};

export function PositionTicket(props: {
  drawingId: number;
  kind: string;
  style: Style;
  points: DrawingPoint[];
  precision: number;
  set(key: string, v: string | number): void;
  merge(values: Style): void;
  setPrice(i: number, price: number): void;
  num(key: string, fallback: number): number;
  str(key: string, fallback?: string): string;
  /** Ok saves the drawing, then calls what this hands it. */
  register(save: () => Promise<void>): void;
}) {
  const tick = () => 10 ** -props.precision;
  const long = props.kind === 'long';
  const dir = long ? 1 : -1;
  const entry = () => props.points[0]?.price ?? 0;
  const stop = () => props.points[1]?.price ?? 0;
  const target = () => props.points[2]?.price ?? 0;
  const round = (v: number) => Number(v.toFixed(props.precision));
  const movePct = (price: number) => (entry() > 0 ? ((price - entry()) / entry()) * 100 : 0);
  const lot = () => props.num('lot', 1);
  const leverage = () => props.num('leverage', 1);
  const byMargin = () => props.str('sizeBy', 'risk') === 'margin';
  const inPercent = () => props.str('riskUnit', '%') === '%';
  const account = () => props.num('account', 1000);
  const riskAmount = () => (inPercent() ? (account() * props.num('risk', 1)) / 100 : props.num('risk', 10));
  const coins = () => {
    if (byMargin()) return coinsFor(props.num('margin', 100), entry(), leverage());
    const per = Math.abs(entry() - stop());
    return per > 0 ? riskAmount() / per : 0;
  };
  const qty = () => coins() / lot();
  const qtyText = (q: number) => {
    const d = props.str('qtyDecimals', '');
    return d === '' ? String(figuresOf(q)) : q.toFixed(Number(d));
  };
  const margin = () => marginFor(coins(), entry(), leverage());
  /** A risk setting that buys these coins, in the unit it keeps. */
  const riskFor = (c: number) => {
    const cash = c * Math.abs(entry() - stop());
    return Number((inPercent() ? (cash / account()) * 100 : cash).toFixed(8));
  };
  // Whichever of the four is typed, the size goes where it says, by the setting it is kept in.
  const setCoins = (c: number) => (byMargin() ? props.set('margin', marginFor(c, entry(), leverage())) : props.set('risk', riskFor(c)));
  const keepMargin = () => props.merge({ sizeBy: 'margin', margin: margin() });
  const keepRisk = () => props.merge({ sizeBy: 'risk', riskUnit: 'cash', risk: Number((coins() * Math.abs(entry() - stop())).toFixed(8)) });

  // The legs, as the drawing keeps them.
  const legs = createMemo(() => parseLegs(props.style.legs));
  const live = createMemo(() => legs().filter((l) => fits(l, entry(), stop(), target())));
  const setLegs = (list: DrawnLeg[]) => props.set('legs', serializeLegs(list));
  // A new leg starts past the last of its kind, apart from the others: a take halfway on to the
  // target, a cut two thirds on to the stop, an add a third of it. The first take or cut moves
  // half the size, later ones a quarter; an add doubles it.
  const addLeg = (kind: LegKind) => {
    const [end, part] = kind === 'take' ? [target(), 1 / 2] : kind === 'add' ? [stop(), 1 / 3] : [stop(), 2 / 3];
    const same = legs().filter((l) => l.kind === kind);
    const from = same.reduce((far, l) => (Math.abs(l.price - entry()) > Math.abs(far - entry()) ? l.price : far), entry());
    const percent = kind === 'add' ? 100 : same.length === 0 ? 50 : 25;
    setLegs([...legs(), { kind, price: round(from + (end - from) * part), percent }]);
  };
  const editLeg = (i: number, change: Partial<DrawnLeg>) => setLegs(legs().map((l, j) => (j === i ? { ...l, ...change } : l)));
  const removeLeg = (i: number) => setLegs(legs().filter((_, j) => j !== i));
  const made = createMemo(() => figures(entry(), stop(), target(), coins(), live(), dir));
  const hasAdds = () => live().some((l) => l.kind === 'add');

  // The journal's side: kept for a browser signed in with Telegram.
  const [signedIn, setSignedIn] = createSignal<boolean | null>(null);
  const [trade, setTrade] = createSignal<Trade | null>(null);
  const [strategy, setStrategy] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [exitPrice, setExitPrice] = createSignal('');
  const [exitAt, setExitAt] = createSignal(localInput(Date.now()));
  const mine = (list: Trade[]) => list.find((t) => t.drawingId === props.drawingId) ?? null;
  onMount(() => {
    void api
      .account()
      .then(async (a) => {
        setSignedIn(!!a.account);
        if (!a.account) return;
        const t = mine(await journalApi.trades());
        setTrade(t);
        setStrategy(t?.strategy ?? '');
        setNotes(t?.notes ?? '');
      })
      .catch(() => setSignedIn(false));
  });
  props.register(async () => {
    if (!signedIn()) return;
    // A position drawn before signing in has its trade once the drawing is saved.
    const t = trade() ?? mine(await journalApi.trades());
    if (!t) return;
    const patch: TradePatch = {};
    if (strategy() !== t.strategy) patch.strategy = strategy();
    if (notes() !== t.notes) patch.notes = notes();
    const x = Number(exitPrice());
    if (exitPrice().trim() !== '' && x > 0) Object.assign(patch, { status: 'closed', exit: x, closedAt: new Date(exitAt()).getTime() || Date.now() });
    if (Object.keys(patch).length > 0) await journalApi.update(t.id, patch);
  });
  const settled = () => {
    const t = trade();
    return t !== null && t.status !== 'planned' && t.status !== 'open';
  };

  const field = (label: string, input: JSX.Element, hint?: () => string, tone?: string) => (
    <label class="tfield">
      <span class="tlabel">{label}</span>
      <div class="trow">{input}</div>
      <Show when={hint?.()}>{(h) => <span class={`thint num ${tone ?? ''}`}>{h()}</span>}</Show>
    </label>
  );
  const pctText = (price: number) => `${movePct(price) > 0 ? '+' : ''}${movePct(price).toFixed(2)}%`;
  const rr = () => made().rr;
  const legName = (kind: LegKind) => (kind === 'take' ? 'TP' : kind === 'cut' ? 'SL' : 'Add');
  // The legs in one list: takes, then cuts, then adds, each numbered in its kind.
  const listed = createMemo(() =>
    LEG_GROUPS.flatMap((g) =>
      legs()
        .map((l, i) => ({ l, i }))
        .filter((x) => x.l.kind === g.kind)
        .map((x, n) => ({ ...x, n: n + 1, g })),
    ),
  );

  return (
    <div class="ticket">
      <Show when={signedIn() !== false} fallback={<p class="tnote">Strategy and notes go to the journal once you sign in with Telegram (Journal, in the top bar).</p>}>
        <StrategyPicker value={strategy()} onChange={setStrategy} />
      </Show>

      <div class="tgrid">
        {field('Entry', <NumField value={round(entry())} onValue={(v) => props.setPrice(0, v)} step={tick()} label="Entry price" />)}
        {field('Stop loss', <NumField value={round(stop())} onValue={(v) => props.setPrice(1, v)} step={tick()} label="Stop loss" />, () => pctText(stop()), 'bear')}
        {field('Take profit', <NumField value={round(target())} onValue={(v) => props.setPrice(2, v)} step={tick()} label="Take profit" />, () => `${pctText(target())} · ${rr().toFixed(2)}R`, 'bull')}
      </div>
      <div class="tsize">
        <div class="tsize-head">
          <span class="tlabel">When the stop moves, keep</span>
          <div class="tkeep">
            <button classList={{ on: byMargin() }} data-tip="The size stays; the risk is what the stop then costs" onClick={keepMargin}>
              Margin
            </button>
            <button classList={{ on: !byMargin() }} data-tip="The risk stays; the size is what it then buys" onClick={keepRisk}>
              Risk
            </button>
          </div>
        </div>
        <div class="tgrid four">
          {field('Margin $', <NumField value={cents(margin())} onValue={(v) => v > 0 && setCoins(coinsFor(v, entry(), leverage()))} label="Margin" />)}
          {field('Leverage ×', <NumField value={leverage()} onValue={(v) => v > 0 && props.set('leverage', v)} label="Leverage" />)}
          {field('Qty', <NumField value={Number(qtyText(qty()))} onValue={(v) => v > 0 && setCoins(v * lot())} label="Position size" />)}
          <Show
            when={byMargin()}
            fallback={field(
              'Risk',
              <>
                <NumField value={cents(props.num('risk', 1))} onValue={(v) => v > 0 && props.set('risk', v)} label="Risk" />
                <select class="dinput tunit" aria-label="Risk in" value={props.str('riskUnit', '%')} onChange={(e) => props.merge({ riskUnit: e.currentTarget.value, risk: e.currentTarget.value === '%' ? Number(((riskAmount() / account()) * 100).toFixed(8)) : Number(riskAmount().toFixed(8)) })}>
                  <option value="%">%</option>
                  <option value="cash">$</option>
                </select>
              </>,
              () => (inPercent() ? `= $${riskAmount().toFixed(2)}` : ''),
            )}
          >
            {field('Risk $', <NumField value={cents(made().loss)} onValue={(v) => v > 0 && lossPerCoin(entry(), stop(), live(), dir) > 0 && setCoins(v / lossPerCoin(entry(), stop(), live(), dir))} label="Risk" />, () => `${((made().loss / account()) * 100).toFixed(2)}% of account`)}
          </Show>
        </div>
        <Readout figures={made()} hasLegs={live().some((l) => l.kind !== 'take')} avgEntry={hasAdds() ? String(round(fullEntry(entry(), coins(), live()))) : null} />
      </div>
      <details class="tmore">
        <summary class="num">
          Account {account().toLocaleString('en-US')} · Lot {lot()} · Qty precision {props.str('qtyDecimals', '') === '' ? 'default' : props.str('qtyDecimals', '')}
        </summary>
        <div class="tgrid">
          {field('Account size', <NumField value={account()} onValue={(v) => v > 0 && props.set('account', v)} label="Account size" />)}
          {field('Lot size', <NumField value={lot()} onValue={(v) => v > 0 && props.set('lot', v)} label="Lot size" />)}
          {field(
            'Qty precision',
            <select class="dinput" value={props.str('qtyDecimals', '')} onChange={(e) => props.set('qtyDecimals', e.currentTarget.value)}>
              <option value="">Default</option>
              <For each={[0, 1, 2, 3, 4, 5, 6, 7, 8]}>{(d) => <option value={d}>{d}</option>}</For>
            </select>,
          )}
        </div>
      </details>

      <div class="tlegs">
        <div class="tlegs-head">
          <span class="tlabel tlegs-title">Partials &amp; averaging</span>
          <span class="grow" />
          <For each={LEG_GROUPS}>
            {(g) => (
              <button class={`tadd ${g.kind}`} data-tip={g.tip} disabled={legs().length >= MAX_LEGS} onClick={() => addLeg(g.kind)}>
                <Icon name="plus" size={10} />
                <span>{legName(g.kind)}</span>
              </button>
            )}
          </For>
        </div>
        {/* By place, not by item: each row keeps its fields, and the focus, as a price in it is typed. */}
        <Index each={listed()}>
          {(x) => {
            const ok = () => fits(x().l, entry(), stop(), target());
            const moved = () => (x().l.percent / 100) * coins();
            return (
              <div classList={{ tleg: true, bad: !ok() }} data-tip={ok() ? undefined : `Left out: a level here goes ${WHERE[x().l.kind]}`}>
                <span class={`tleg-name ${x().l.kind}`}>
                  {legName(x().l.kind)}
                  {x().n}
                </span>
                <NumField value={round(x().l.price)} onValue={(v) => editLeg(x().i, { price: v })} step={tick()} label={`${x().g.title} ${x().n} price`} />
                <span class="tpct">
                  <NumField value={x().l.percent} onValue={(v) => v > 0 && v <= 1000 && editLeg(x().i, { percent: v })} step={5} label={`${x().g.title} ${x().n} percent`} />%
                </span>
                <span class="thint num" data-tip={x().l.kind === 'add' ? 'Coins added' : 'Coins moved, and what they make against the entry'}>
                  {x().l.kind === 'add' ? `+${qtyText(moved() / lot())}` : money((x().l.price - entry()) * dir * moved())}
                </span>
                <button class="iconbtn" aria-label="Remove" onClick={() => removeLeg(x().i)}>
                  <Icon name="close" size={12} />
                </button>
              </div>
            );
          }}
        </Index>
      </div>

      <Show when={signedIn()}>
        <textarea class="dinput tnotes" rows={2} maxLength={4000} aria-label="Notes" placeholder="Notes: setup, emotions, lessons learned…" value={notes()} onInput={(e) => setNotes(e.currentTarget.value)} />
        <Show
          when={!settled()}
          fallback={
            <p class="tnote num">
              {STATUS_LABEL[trade()!.status]} {stamp(trade()!.closedAt)} · {money(pnl(trade()!))} · {rText(rMultiple(trade()!))}
            </p>
          }
        >
          <details class="tmore" open={exitPrice() !== ''}>
            <summary>Close by hand</summary>
            <div class="tgrid two">
              {field('Exit price', <input class="dinput num" type="number" inputmode="decimal" step={tick()} placeholder="—" value={exitPrice()} onInput={(e) => setExitPrice(e.currentTarget.value)} />)}
              {field('Exit date', <input class="dinput" type="datetime-local" value={exitAt()} onInput={(e) => setExitAt(e.currentTarget.value)} />)}
            </div>
          </details>
        </Show>
      </Show>

      <Show when={trade()}>
        {(t) => (
          <div class="tstatus num">
            <span class="tlabel">Journal</span>
            <span>{STATUS_LABEL[t().status]}</span>
            <Show when={t().status === 'planned'}>
              <span class="dim">waiting for the entry</span>
            </Show>
            <Show when={t().openedAt !== null}>
              <span class="dim">opened {stamp(t().openedAt)}</span>
            </Show>
            <For each={t().fills.filter((f) => f.kind !== 'entry')}>
              {(f) => (
                <span class="dim">
                  {FILL_LABEL[f.kind]} {qtyText(f.qty / lot())} @ {round(f.price)} · {stamp(f.at)}
                </span>
              )}
            </For>
            <Show when={t().status === 'open' && t().fills.length > 0}>
              <span class="dim">held {qtyText(book(t()).held / lot())}</span>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
