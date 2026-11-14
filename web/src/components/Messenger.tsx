/**
 * tradoor messenger: one room for everyone with tradoor open, beside the GitHub
 * mark. Anyone may write; a signed-in account with a username writes as
 * @username, anyone else as a guest. What the server keeps of it, and what it
 * refuses (links in any spelling, floods, the muted), is in bins/server
 * messenger. Every message is put on the page as text, never as markup.
 *
 * Loaded the first time it is opened; while open and on screen it asks for
 * what is new every few seconds, and nothing while it is shut.
 */
import { createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createStore } from 'solid-js/store';
import { api, catalog, errorText, type MessengerMessage, type Product } from '../lib/api';
import { dayOf, watchDay } from '../lib/day-feed';
import { formatPct, formatPrice } from '../lib/format';
import { Icon } from '../lib/icons';
import { setPanelMarket, targetChart } from '../workspace/store';
import { session } from '../lib/owner';
import { noteOnline, online } from '../lib/presence';
import { username } from '../lib/username';

const EVERY_MS = 2500;
const MAX_CHARS = 300;
/** The ones a trader reaches for; any other can be typed or pasted. */
const EMOJI = ['👍', '👎', '😂', '🤣', '😅', '😊', '😎', '🤔', '😭', '😡', '🙏', '👏', '🔥', '🚀', '🌙', '💎', '🙌', '💪', '🤝', '👀', '❤️', '💯', '✅', '❌', '⚠️', '📈', '📉', '💰', '💸', '🐂', '🐻', '🐋', '🎯', '⏳', '☕', '🎉', '😴', '🤡', '💀', '🤑'];

const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

// ---------------------------------------------------------------- $BTC, priced

/** `$BTC`, `$1000PEPE`: a tag, a letter first. */
const CASHTAG = /(\$[A-Za-z][A-Za-z0-9]{1,14})(?![A-Za-z0-9])/g;
/** Markets streamed at once for the chips; past this a chip shows the catalogue's price. */
const MOST_STREAMS = 12;

const [products] = createResource(() => catalog().catch(() => null));

/** The market a tag stands for: Binance's perpetual first, then its spot, another exchange's, or tradoor's own index. */
function marketOfTag(tag: string): Product | null {
  const c = products();
  if (!c) return null;
  const t = tag.toUpperCase();
  for (const m of [`binance-perp:${t}USDT`, `binance-spot:${t}USDT`, `bybit-perp:${t}USDT`, `okx-perp:${t}USDT`, `hyperliquid-perp:${t}`, `index:${t}`]) {
    const p = c.byMarket.get(m);
    if (p) return p;
  }
  return null;
}

/** The last price and the day's open of each market a chip shows, one stream each however many chips. */
const [quotes, setQuotes] = createStore<Record<string, { last: number; open: number }>>({});
const streams = new Map<string, { n: number; off: () => void }>();
function hold(market: string): void {
  const s = streams.get(market);
  if (s) {
    s.n += 1;
    return;
  }
  if (streams.size >= MOST_STREAMS) return;
  const known = dayOf(market);
  if (known) setQuotes(market, { last: known.close, open: known.open });
  streams.set(market, { n: 1, off: watchDay(market, (d) => setQuotes(market, { last: d.close, open: d.open })) });
}
function release(market: string): void {
  const s = streams.get(market);
  if (!s) return;
  s.n -= 1;
  if (s.n <= 0) {
    s.off();
    streams.delete(market);
  }
}

/** A `$TAG` that names a market: its price and the day's change, a click opens it on the chart. */
function Cashtag(props: { tag: string }) {
  const product = () => marketOfTag(props.tag.slice(1));
  return (
    <Show when={product()} fallback={<>{props.tag}</>} keyed>
      {(p) => {
        hold(p.market);
        onCleanup(() => release(p.market));
        const q = () => quotes[p.market];
        const last = () => q()?.last ?? p.lastPrice;
        const change = () => {
          const v = q();
          return v && v.open ? ((v.last - v.open) / v.open) * 100 : p.changePct;
        };
        return (
          <button
            class="cashtag"
            classList={{ bull: (change() ?? 0) > 0, bear: (change() ?? 0) < 0 }}
            data-tip={`${p.symbol} · open it on the chart`}
            onClick={() => {
              const chart = targetChart();
              if (chart) setPanelMarket(chart.id, p.market);
            }}
          >
            {props.tag.toUpperCase()}
            <span class="px">{formatPrice(last(), p.priceDecimals ?? undefined)}</span>
            <span class="pc">{formatPct(change())}</span>
          </button>
        );
      }}
    </Show>
  );
}

/** A message's text, its `$TAG`s priced and everything else as it was written. */
function Text(props: { text: string }) {
  const parts = () => props.text.split(CASHTAG);
  return (
    <For each={parts()}>{(part, i) => (i() % 2 === 1 ? <Cashtag tag={part} /> : <>{part}</>)}</For>
  );
}

export function Messenger(props: { onClose(): void }) {
  const [messages, setMessages] = createSignal<MessengerMessage[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [text, setText] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [sending, setSending] = createSignal(false);
  const [emoji, setEmoji] = createSignal(false);
  /** The moderator's brake: only accounts with a username write while it is on. */
  const [accountsOnly, setAccountsOnly] = createSignal(false);
  /** Messages this tab reported, so each says so. */
  const [reported, setReported] = createSignal<ReadonlySet<number>>(new Set());
  const report = async (id: number) => {
    try {
      await api.reportMessage(id);
      setReported((r) => new Set([...r, id]));
    } catch (e) {
      setError(errorText(e));
    }
  };
  let list!: HTMLDivElement;
  let input!: HTMLTextAreaElement;
  let last = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  let asking = false;

  /** At the foot of the list, or near it: only then does a new message carry it down. */
  const atFoot = () => list.scrollHeight - list.scrollTop - list.clientHeight < 60;
  const toFoot = () => queueMicrotask(() => (list.scrollTop = list.scrollHeight));

  const take = (got: MessengerMessage[], gone: number[]) => {
    const down = !loaded() || atFoot();
    const have = new Set(messages().map((m) => m.id));
    const fresh = got.filter((m) => !have.has(m.id));
    const cut = new Set(gone);
    if (fresh.length === 0 && !messages().some((m) => cut.has(m.id))) return;
    setMessages((ms) => [...ms.filter((m) => !cut.has(m.id)), ...fresh].slice(-200));
    for (const m of fresh) last = Math.max(last, m.id);
    if (down) toFoot();
  };

  const ask = async () => {
    if (asking || document.visibilityState !== 'visible') return;
    asking = true;
    try {
      const r = await api.messenger(last);
      take(r.messages, r.gone);
      noteOnline(r.online);
      setAccountsOnly(r.accountsOnly === true);
      setLoaded(true);
    } catch {
      // Asked again on the next turn.
    } finally {
      asking = false;
    }
  };

  const post = async () => {
    const t = text().trim();
    if (!t || sending()) return;
    setSending(true);
    setError(null);
    try {
      const m = await api.sendMessage(t);
      setText('');
      setEmoji(false);
      take([m], []);
      toFoot();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSending(false);
      input.focus();
    }
  };

  const insert = (e: string) => {
    const at = input.selectionStart ?? text().length;
    const end = input.selectionEnd ?? at;
    const next = text().slice(0, at) + e + text().slice(end);
    if ([...next].length > MAX_CHARS) return;
    setText(next);
    queueMicrotask(() => {
      input.focus();
      input.setSelectionRange(at + e.length, at + e.length);
    });
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') props.onClose();
  };

  onMount(() => {
    void ask();
    timer = setInterval(() => void ask(), EVERY_MS);
    document.addEventListener('visibilitychange', ask);
    document.addEventListener('keydown', onKey);
    input.focus();
  });
  onCleanup(() => {
    clearInterval(timer);
    document.removeEventListener('visibilitychange', ask);
    document.removeEventListener('keydown', onKey);
  });

  const as = () => (session() && username() ? `@${username()}` : null);
  /** A guest, while only accounts may write. */
  const held = () => accountsOnly() && !as();

  return (
    <section class="msgr" role="dialog" aria-label="tradoor messenger" onPointerDown={(e) => e.stopPropagation()}>
      <header>
        <Icon name="chat" size={14} />
        <span class="title">tradoor messenger</span>
        <span class="live">
          <span class="dot" aria-hidden="true" />
          {online() || '·'} online
        </span>
        <button class="x" aria-label="Close tradoor messenger" data-tip="Close · Esc" onClick={() => props.onClose()}>
          ✕
        </button>
      </header>
      {/* Said always: the bait a link cannot carry is talk, and talk is answered here. */}
      <p class="msgr-safe">tradoor never asks for your keys, seed phrase or login. No one here is tradoor staff.</p>
      <div class="msgr-list" ref={list}>
        <Show when={loaded()} fallback={<p class="msgr-note">Loading…</p>}>
          <Show when={messages().length > 0} fallback={<p class="msgr-note">No messages yet. Say hello.</p>}>
            <For each={messages()}>
              {(m) => (
                <div class="msgr-row">
                  <div class="msgr-head">
                    <span class="who" classList={{ signed: m.signed }}>
                      {m.name}
                    </span>
                    <span class="at">{clock(m.at)}</span>
                    <button
                      class="msgr-report"
                      classList={{ done: reported().has(m.id) }}
                      disabled={reported().has(m.id)}
                      aria-label={reported().has(m.id) ? 'Reported' : `Report this message from ${m.name}`}
                      data-tip={reported().has(m.id) ? 'Reported to the moderator' : 'Report to the moderator: spam, a scam or abuse'}
                      onClick={() => void report(m.id)}
                    >
                      {reported().has(m.id) ? 'reported' : 'report'}
                    </button>
                  </div>
                  <div class="msgr-text">
                    <Text text={m.text} />
                  </div>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </div>
      <Show when={emoji()}>
        <div class="msgr-emoji" role="listbox" aria-label="Emoji">
          <For each={EMOJI}>
            {(e) => (
              <button role="option" aria-label={e} onClick={() => insert(e)}>
                {e}
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={error()}>{(e) => <p class="msgr-error">{e()}</p>}</Show>
      <footer>
        <button class="msgr-tool" classList={{ on: emoji() }} aria-label="Emoji" data-tip="Emoji" disabled={held()} onClick={() => setEmoji((v) => !v)}>
          <Icon name="smile" size={16} />
        </button>
        <textarea
          ref={input}
          rows={1}
          maxlength={MAX_CHARS * 2}
          disabled={held()}
          placeholder={held() ? 'Only accounts with a username can write for now' : `Write as ${as() ?? 'a guest'}`}
          value={text()}
          onInput={(e) => {
            setText(e.currentTarget.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
              e.preventDefault();
              void post();
            }
          }}
        />
        <button class="msgr-send" aria-label="Send" data-tip="Send · Enter" disabled={held() || sending() || !text().trim()} onClick={() => void post()}>
          <Icon name="send" size={15} />
        </button>
      </footer>
      <p class="msgr-hint">
        $BTC shows its live price · no links
        <Show when={!as()}>{session() ? ' · choose a username in Settings to write as @name' : ' · sign in with Telegram and choose a username to write as @name'}</Show>
      </p>
    </section>
  );
}
