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
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { api, errorText, type MessengerMessage } from '../lib/api';
import { Icon } from '../lib/icons';
import { session } from '../lib/owner';
import { noteOnline, online } from '../lib/presence';
import { username } from '../lib/username';

const EVERY_MS = 2500;
const MAX_CHARS = 300;
/** The ones a trader reaches for; any other can be typed or pasted. */
const EMOJI = ['👍', '👎', '😂', '🤣', '😅', '😊', '😎', '🤔', '😭', '😡', '🙏', '👏', '🔥', '🚀', '🌙', '💎', '🙌', '💪', '🤝', '👀', '❤️', '💯', '✅', '❌', '⚠️', '📈', '📉', '💰', '💸', '🐂', '🐻', '🐋', '🎯', '⏳', '☕', '🎉', '😴', '🤡', '💀', '🤑'];

const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

export function Messenger(props: { onClose(): void }) {
  const [messages, setMessages] = createSignal<MessengerMessage[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [text, setText] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [sending, setSending] = createSignal(false);
  const [emoji, setEmoji] = createSignal(false);
  /** The moderator's brake: only accounts with a username write while it is on. */
  const [accountsOnly, setAccountsOnly] = createSignal(false);
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
                  </div>
                  <div class="msgr-text">{m.text}</div>
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
          placeholder={held() ? 'Only accounts with a username can write for now' : as() ? `Write as ${as()} · no links` : 'Write as a guest · no links'}
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
      <Show when={!as()}>
        <p class="msgr-hint">{session() ? 'Choose a username in Settings to write as @name.' : 'Sign in with Telegram and choose a username to write as @name.'}</p>
      </Show>
    </section>
  );
}
