import { createSignal, For, type JSX, onCleanup, Show } from 'solid-js';
import { api, errorText, type TelegramStatus } from '../lib/api';
import { OWNER_PATTERN, ownerId, session, switchOwner } from '../lib/owner';
import { ALL_EXCHANGES, ExchangeLogo } from '../lib/exchanges';
import { Icon, type IconName } from '../lib/icons';
import { deviceZone, LOCAL_ZONE, offsetLabel, offsetMinutes, TIME_ZONE_KEY, timeZone } from '../lib/timezone';
import { CHART_THEME, FONT_PX, UI_THEME } from '../lib/theme';
import { THEMES, themeOf, withTheme, type ThemePreset } from '../lib/themes';
import { LayoutsMenu } from './LayoutsMenu';
import { formatDateTime } from '../lib/timezone';
import { doc, resetWorkspace, setThemeValue, update } from '../workspace/store';
import { AccountCard, clock } from './Account';
import { forgetRecorded, KEEP_DAYS, recorderSettings, setRecorderSettings } from '../lib/fp-recorder';
import { usage } from '../lib/fp-store';
import { symbolOf, venueLabel } from '../lib/markets';

function Swatch(props: { name: string; label: string; fallback: string }) {
  const value = () => doc.theme[props.name] ?? props.fallback;
  return (
    <label class="swatch">
      <input type="color" value={value()} onInput={(e) => setThemeValue(props.name, e.currentTarget.value)} />
      <span class="name">{props.label}</span>
      <span class="num dim">{value()}</span>
      <Show when={doc.theme[props.name]}>
        <button
          class="dim"
          data-tip="Back to the default"
          onClick={(e) => {
            e.preventDefault();
            setThemeValue(props.name, null);
          }}
        >
          ↺
        </button>
      </Show>
    </label>
  );
}

/** Local first — the default — then UTC, then every named zone with its offset now. */
function zoneChoices(): { value: string; label: string }[] {
  const now = Date.now();
  const named = (typeof Intl.supportedValuesOf === 'function' ? Intl.supportedValuesOf('timeZone') : [])
    .filter((z) => z !== 'UTC')
    .map((z) => ({ zone: z, offset: offsetMinutes(z, now) }))
    .sort((a, b) => a.offset - b.offset || a.zone.localeCompare(b.zone));
  return [
    { value: LOCAL_ZONE, label: `Local — ${deviceZone()} (${offsetLabel(offsetMinutes(LOCAL_ZONE, now))})` },
    { value: 'UTC', label: 'UTC' },
    ...named.map((z) => ({ value: z.zone, label: `${z.zone.replaceAll('_', ' ')} (${offsetLabel(z.offset)})` })),
  ];
}

/** How long a message stays in the chat. Telegram refuses to take back one older than 48 hours. */
const CLEARING = [
  { value: '47', label: '2 days, then cleared' },
  { value: '24', label: '1 day, then cleared' },
  { value: '6', label: '6 hours, then cleared' },
  { value: 'keep', label: 'Keep every message' },
];

/** A labelled line of a card: what it is on the left, its control on the right. */
function Row(props: { label: string; tip?: string; children: JSX.Element }) {
  return (
    <div class="set-row">
      <span class="label" data-tip={props.tip}>
        {props.label}
      </span>
      <div class="grow">{props.children}</div>
    </div>
  );
}

function AccountSection() {
  return (
    <>
      <h2>Account</h2>
      <p class="lead">
        Your drawings, journal, layout, watchlists and alerts are kept on the server. Sign in with Telegram and they follow you — after clearing this browser's history,
        or on another device, sign in again to have them back.
      </p>
      <AccountCard />
    </>
  );
}

/**
 * Alerts on Telegram. A bot cannot open a conversation, so the reader does:
 * the link carries a one-use code that tells the server which browser the chat
 * belongs to. While it is open the status is read again, so the chat appears
 * here the moment the bot has it.
 */
function TelegramSection() {
  const [status, setStatus] = createSignal<TelegramStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [note, setNote] = createSignal<string | null>(null);
  const [waiting, setWaiting] = createSignal(false);

  const load = async () => {
    try {
      const next = await api.telegram();
      // A code is good for fifteen minutes and spent once, so keep a fresh one
      // in hand: one when this opens, another after a chat takes the last.
      const linked = status()?.chats.length ?? 0;
      if (next.enabled && next.bot && (!invite() || next.chats.length !== linked)) void mint();
      setStatus(next);
      setError(null);
      // Their zone changed, or summer time moved it: the chats follow.
      const now = clock();
      if (next.chats.some((c) => c.offsetMinutes !== now.offsetMinutes || c.zone !== now.zone)) {
        await api.telegramClock(now);
        setStatus(await api.telegram());
      }
    } catch (e) {
      setError(errorText(e));
    }
  };
  void load();
  // While a link is out, watch for the chat to arrive; otherwise keep it fresh.
  const timer = setInterval(() => void load().then(() => setWaiting((w) => w && (status()?.chats.length ?? 0) === 0)), 3_000);
  onCleanup(() => clearInterval(timer));

  const [invite, setInvite] = createSignal<{ url: string; code: string } | null>(null);
  const mint = async () => {
    try {
      const { url, code } = await api.telegramLink(clock());
      setInvite({ url, code });
    } catch {
      setInvite(null); // The bot may not have answered yet; the status line says so.
    }
  };

  const act = async (what: () => Promise<unknown>, done: string) => {
    setError(null);
    try {
      await what();
      setNote(done);
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <>
      <h2>Telegram alerts</h2>
      <p class="lead">Price alerts and the signals you turn on fire on the server, so they reach these chats whether or not a tab is open.</p>
      <Show when={status()} fallback={<div class="dim">{error() ?? 'Reading…'}</div>}>
        {(s) => (
          <Show when={s().enabled} fallback={<div class="set-card dim">This server has no Telegram bot set up. Give it one with TRADOOR_TELEGRAM_TOKEN.</div>}>
            <div class="set-card">
              <h3>Chats</h3>
              <For each={s().chats} fallback={<div class="dim">No chat yet. Signing in with Telegram links yours, or connect one here.</div>}>
                {(chat) => (
                  <div class="set-row">
                    <Icon name="send" size={13} />
                    <span class="grow">{chat.name ?? `Chat ${chat.chatId}`}</span>
                    <span class="num dim">linked {formatDateTime(chat.linkedAt).slice(0, 16)}</span>
                    <button class="btn" onClick={() => void act(() => api.telegramUnlink(chat.chatId), 'Disconnected.')}>
                      Disconnect
                    </button>
                  </div>
                )}
              </For>
              <div class="set-actions">
                <Show when={invite()} fallback={<span class="dim">{s().bot ? 'Making a link…' : 'The bot has not answered yet.'}</span>}>
                  {(link) => (
                    <>
                      <a class="btn" href={link().url} target="_blank" rel="noopener" onClick={() => setWaiting(true)}>
                        {s().chats.length > 0 ? 'Connect another chat' : 'Connect Telegram'}
                      </a>
                      <span class="dim num" data-tip="Or send the bot /start with this code">
                        /start {link().code}
                      </span>
                    </>
                  )}
                </Show>
                <Show when={s().chats.length > 0}>
                  <button class="btn" onClick={() => void act(() => api.telegramTest(), 'Test message sent.')}>
                    Send a test
                  </button>
                </Show>
                <Show when={waiting()}>
                  <span class="warn">waiting for Telegram…</span>
                </Show>
              </div>
            </div>
            <Show when={s().chats.length > 0}>
              <div class="set-card">
                <h3>Messages</h3>
                <Row label="Messages stay" tip="The bot takes its own messages back when their time is up, so the chat does not fill with old alerts. Telegram only allows this within 48 hours of sending.">
                  <select class="box wide" onChange={(e) => void act(() => api.telegramClearing(e.currentTarget.value === 'keep' ? null : Number(e.currentTarget.value)), 'Saved.')}>
                    <For each={CLEARING}>
                      {(c) => (
                        <option value={c.value} selected={String(s().chats[0]?.clearAfterHours ?? 'keep') === c.value}>
                          {c.label}
                        </option>
                      )}
                    </For>
                  </select>
                </Row>
                <div class="dim">Times in them are written in {offsetLabel(clock().offsetMinutes)}, the zone set under General.</div>
              </div>
            </Show>
          </Show>
        )}
      </Show>
      <Show when={error()}>{(e) => <div class="warn">▲ {e()}</div>}</Show>
      <Show when={note() && !error()}>{(n) => <div class="dim">{n()}</div>}</Show>
    </>
  );
}

function GeneralSection() {
  return (
    <>
      <h2>General</h2>
      <p class="lead">The workspace's name and the clock every time on the page is written in.</p>
      <div class="set-card">
        <Row label="Workspace name">
          <input
            class="box wide"
            value={doc.name}
            maxLength={120}
            onChange={(e) => {
              const name = e.currentTarget.value.trim();
              if (name) update((d) => void (d.name = name));
            }}
          />
        </Row>
        <Row label="Time zone" tip="Every time on the page — axis, crosshair, tape, clock — and in Telegram messages. Daily and longer candles keep their exchange (UTC) dates.">
          <select class="box wide" onChange={(e) => setThemeValue(TIME_ZONE_KEY, e.currentTarget.value === LOCAL_ZONE ? null : e.currentTarget.value)}>
            <For each={zoneChoices()}>
              {(z) => (
                <option value={z.value} selected={timeZone() === z.value}>
                  {z.label}
                </option>
              )}
            </For>
          </select>
        </Row>
      </div>
      <BrowserFootprints />
    </>
  );
}

const megabytes = (bytes: number) => (bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : bytes > 0 ? `${Math.max(1, Math.round(bytes / 1e3))} KB` : 'nothing');

/** Footprints recorded in this browser: whether, for how long, and what they take — each deletable. */
function BrowserFootprints() {
  const [used, setUsed] = createSignal<{ market: string; minutes: number; bytes: number }[] | null>(null);
  const [site, setSite] = createSignal<{ usage: number; quota: number } | null>(null);
  const refresh = () => {
    void usage()
      .then(setUsed)
      .catch(() => setUsed([]));
    void navigator.storage
      ?.estimate()
      .then((e) => setSite({ usage: e.usage ?? 0, quota: e.quota ?? 0 }))
      .catch(() => {});
  };
  refresh();
  const total = () => (used() ?? []).reduce((sum, m) => sum + m.bytes, 0);
  const remove = (market?: string) => {
    const what = market ? `${symbolOf(market)} (${venueLabel(market)})'s footprint` : 'every footprint';
    if (!confirm(`Delete ${what} recorded in this browser? This cannot be undone; the server's own markets are not touched.`)) return;
    void forgetRecorded(market).then(refresh);
  };
  return (
    <>
      <h3 class="set-sub">Footprints in this browser</h3>
      <p class="lead">For every market the server does not record, the footprint is recorded here, in this browser alone, while a chart shows it. Nothing is sent to the server, and another device keeps its own.</p>
      <div class="set-card">
        <Row label="Record" tip="Off, footprints show only for the markets the server records">
          <label class="set-check">
            <input type="checkbox" checked={recorderSettings().on} onChange={(e) => setRecorderSettings({ on: e.currentTarget.checked })} />
            <span>{recorderSettings().on ? 'Recording the markets on your charts' : 'Not recording'}</span>
          </label>
        </Row>
        <Row label="Keep" tip="Older minutes are deleted on their own">
          <select class="box wide" onChange={(e) => setRecorderSettings({ days: Number(e.currentTarget.value) })}>
            <For each={KEEP_DAYS}>
              {(d) => (
                <option value={d} selected={recorderSettings().days === d}>
                  {d === 1 ? 'The last day' : `The last ${d} days`}
                </option>
              )}
            </For>
          </select>
        </Row>
        <Row label="Stored here">
          <div class="set-actions">
            <span class="num">{used() === null ? '…' : `${megabytes(total())} · ${used()!.length} market${used()!.length === 1 ? '' : 's'}`}</span>
            <Show when={site()}>{(s) => <span class="dim">of {megabytes(s().usage)} this site uses; the browser allows {megabytes(s().quota)}</span>}</Show>
            <Show when={(used() ?? []).length > 0}>
              <button class="btn danger" onClick={() => remove()}>
                Delete all
              </button>
            </Show>
          </div>
        </Row>
        <Show when={(used() ?? []).length > 0}>
          <div class="fp-used">
            <For each={used()!}>
              {(m) => (
                <div class="fp-used-row">
                  <span class="sym">
                    {symbolOf(m.market)} <span class="dim">{venueLabel(m.market)}</span>
                  </span>
                  <span class="num dim">{Math.round(m.minutes / 60)} h</span>
                  <span class="num">{megabytes(m.bytes)}</span>
                  <button class="dim" data-tip={`Delete ${symbolOf(m.market)}'s footprint`} aria-label={`Delete ${symbolOf(m.market)}'s footprint`} onClick={() => remove(m.market)}>
                    ✕
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </>
  );
}

/** The chart a theme draws, small: its background, a few candles each way, and the page's accent under it. */
function ThemePreview(props: { theme: ThemePreset }) {
  const color = (name: string): string => props.theme.colors[name] ?? [...CHART_THEME, ...UI_THEME].find((t) => t.name === name)?.value ?? '#000000';
  const candles = [6, 10, 7, 13, 9, 15, 12, 18];
  return (
    <span class="theme-preview" style={{ background: color('background'), 'border-color': color('ui.border') }}>
      <For each={candles}>{(h, i) => <i style={{ height: `${h * 3}%`, background: color(i() % 3 === 2 ? 'bear' : 'bull') }} />}</For>
      <b style={{ background: color('ui.primary') }} />
    </span>
  );
}

function ThemesSection() {
  const current = () => themeOf(doc.theme);
  const pick = (t: ThemePreset) => update((d) => void (d.theme = withTheme(d.theme, t)));
  return (
    <>
      <h2>Themes</h2>
      <p class="lead">A theme colours the chart and the page around it, and is kept with the workspace, so it follows your account too.</p>
      <div class="themes">
        <For each={THEMES}>
          {(t) => (
            <button class="theme-card" classList={{ on: current() === t.id }} aria-pressed={current() === t.id} onClick={() => pick(t)}>
              <ThemePreview theme={t} />
              <span class="theme-name">{t.name}</span>
              <span class="theme-note">{t.note}</span>
            </button>
          )}
        </For>
      </div>
      <h3 class="set-sub">{current() ? 'Fine-tune it' : 'Your own colours'}</h3>
      <p class="lead">Any colour changed here makes the theme your own; ↺ puts one back to the default.</p>
      <div class="set-card">
        <h3>Chart</h3>
        <div class="swatches">
          <For each={CHART_THEME}>{(t) => <Swatch name={t.name} label={t.label} fallback={t.value} />}</For>
        </div>
        <Row label="Axis font size (px)">
          <input
            class="box num"
            type="number"
            min={8}
            max={20}
            value={doc.theme[FONT_PX.name] ?? FONT_PX.value}
            onChange={(e) => {
              const px = Number(e.currentTarget.value);
              setThemeValue(FONT_PX.name, px >= 8 && px <= 20 ? String(px) : null);
            }}
          />
        </Row>
      </div>
      <div class="set-card">
        <h3>Page</h3>
        <div class="swatches">
          <For each={UI_THEME}>{(t) => <Swatch name={t.name} label={t.label} fallback={t.value} />}</For>
        </div>
      </div>
      <div class="set-actions">
        <button class="btn" onClick={() => pick(THEMES[0]!)}>
          Back to tradoor dark
        </button>
      </div>
    </>
  );
}

function LayoutsSection() {
  return (
    <>
      <h2>Layouts</h2>
      <p class="lead">Start from a recommended layout, keep your own by name, or carry one as a file. The same as Layouts in the top bar.</p>
      <div class="set-card">
        <LayoutsMenu inSettings />
      </div>
    </>
  );
}

function AdvancedSection(props: { onClose(): void }) {
  return (
    <>
      <h2>Advanced</h2>
      <p class="lead">What signing in does for you, by hand, and the switches to start over.</p>
      <div class="set-card">
        <h3>Browser id</h3>
        <OwnerRows />
      </div>
      <div class="set-card">
        <h3>Exchanges</h3>
        <div class="swatches one">
          <For each={ALL_EXCHANGES}>
            {(x) => (
              <span class="swatch">
                <ExchangeLogo exchange={x} size={16} />
                <span class="name">{x.name}</span>
                <span class={x.live ? 'bull' : 'dim'}>{x.live ? 'live' : 'coming later'}</span>
              </span>
            )}
          </For>
        </div>
      </div>
      <div class="set-card danger">
        <h3>Start over</h3>
        <div class="set-row">
          <span class="grow dim">Replace this workspace's panels and layout with the default. Drawings and alerts stay.</span>
          <button
            class="btn danger"
            onClick={() => {
              if (!confirm('Replace this workspace with the default layout?')) return;
              resetWorkspace();
              props.onClose();
            }}
          >
            Reset workspace
          </button>
        </div>
      </div>
    </>
  );
}

/** This browser's id, to copy — and another's, to take its drawings and layout over. A
 *  signed-in browser has a session instead, and nothing here to copy. */
function OwnerRows() {
  const [other, setOther] = createSignal('');
  const [copied, setCopied] = createSignal(false);
  if (session()) {
    return <p class="note">This browser is signed in: it opens the account with a session of its own, which signing out ends. There is no id to copy — sign in with Telegram on the other device instead.</p>;
  }
  return (
    <>
      <Row label="This browser's id">
        <div class="set-inline">
          <input class="box wide num" readOnly value={ownerId()} onFocus={(e) => e.currentTarget.select()} />
          <button class="btn" onClick={() => void navigator.clipboard?.writeText(ownerId()).then(() => setCopied(true))}>
            {copied() ? 'Copied' : 'Copy'}
          </button>
        </div>
      </Row>
      <Row label="Use another id">
        <div class="set-inline">
          <input class="box wide num" value={other()} placeholder="paste the id from your other device" onInput={(e) => setOther(e.currentTarget.value)} />
          <button
            class="btn"
            disabled={!OWNER_PATTERN.test(other().trim())}
            onClick={() => {
              if (confirm("Switch to that id's drawings and layout? This browser's stay under its own id.")) switchOwner(other().trim());
            }}
          >
            Use
          </button>
        </div>
      </Row>
      <p class="note">
        Everything is kept on the server under this id, and only this browser sends it. Keep it to yourself — whoever has it can change your things. An account's id does not open the account: sign in with Telegram
        for that.
      </p>
    </>
  );
}

const SECTIONS: { id: string; label: string; icon: IconName }[] = [
  { id: 'account', label: 'Account', icon: 'user' },
  { id: 'telegram', label: 'Telegram', icon: 'send' },
  { id: 'general', label: 'General', icon: 'settings' },
  { id: 'themes', label: 'Themes', icon: 'droplet' },
  { id: 'layouts', label: 'Layouts', icon: 'layout' },
  { id: 'advanced', label: 'Advanced', icon: 'command' },
];
/** The section open last, for the next time Settings opens. */
const [section, setSection] = createSignal('account');

export function Settings(props: { onClose(): void }) {
  return (
    <div class="backdrop" onPointerDown={() => props.onClose()}>
      <div
        class="dialog settings"
        tabIndex={-1}
        ref={(el) => queueMicrotask(() => el.focus())}
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key !== 'Escape') return;
          e.stopPropagation();
          props.onClose();
        }}
      >
        <header>
          <span class="title caps">Settings</span>
          <span class="spacer" />
          <button class="x" onClick={() => props.onClose()}>
            esc ✕
          </button>
        </header>
        <div class="set">
          <nav class="set-nav">
            <For each={SECTIONS}>
              {(s) => (
                <button classList={{ on: section() === s.id }} onClick={() => setSection(s.id)}>
                  <Icon name={s.icon} size={14} />
                  <span>{s.label}</span>
                </button>
              )}
            </For>
          </nav>
          <div class="set-body">
            <Show when={section() === 'account'}>
              <AccountSection />
            </Show>
            <Show when={section() === 'telegram'}>
              <TelegramSection />
            </Show>
            <Show when={section() === 'general'}>
              <GeneralSection />
            </Show>
            <Show when={section() === 'themes'}>
              <ThemesSection />
            </Show>
            <Show when={section() === 'layouts'}>
              <LayoutsSection />
            </Show>
            <Show when={section() === 'advanced'}>
              <AdvancedSection onClose={props.onClose} />
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
}
