import { createSignal, onCleanup, Show } from 'solid-js';
import { api, errorText, type AccountStatus } from '../lib/api';
import { Icon } from '../lib/icons';
import { holdSession, startOver } from '../lib/owner';
import { noteUsername, username } from '../lib/username';
import { UsernameClaim } from './Username';
import { deviceZone, formatDateTime, LOCAL_ZONE, offsetMinutes, timeZone } from '../lib/timezone';

/** The clock the reader reads the page in, which their Telegram messages follow. */
export const clock = () => {
  const zone = timeZone() === LOCAL_ZONE ? deviceZone() : timeZone();
  return { zone, offsetMinutes: offsetMinutes(timeZone(), Date.now()) };
};

/**
 * Signing in with Telegram: everything stays keyed by the browser id it was
 * made under, and signing in gives a browser a session of the account that
 * holds it — its own, ended by signing out of it or of every browser. The link is minted before it is
 * needed, so the button is a plain link — one opened after waiting on the
 * server is a popup, and a popup is blocked — and while it is out the sign-in
 * is asked after until the reader says yes in the chat. Settings shows it, and
 * the journal, which is kept for an account, before it opens.
 */
export function AccountCard(props: { onSignedIn?: () => void }) {
  const [status, setStatus] = createSignal<AccountStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [invite, setInvite] = createSignal<{ url: string; code: string } | null>(null);
  const [waiting, setWaiting] = createSignal(false);
  const [note, setNote] = createSignal<string | null>(null);

  const mint = async () => {
    try {
      setInvite(await api.signIn(clock()));
    } catch {
      setInvite(null); // The bot may not have answered yet; the card says so.
    }
  };
  const load = async () => {
    try {
      const next = await api.account();
      setStatus(next);
      setError(null);
      if (next.account) noteUsername(next.account.username ?? null);
      if (!next.account && next.telegram && next.bot && !invite()) void mint();
    } catch (e) {
      setError(errorText(e));
    }
  };
  void load();

  const ask = async () => {
    const code = invite()?.code;
    if (!code || !waiting()) return;
    try {
      const got = await api.signedIn(code);
      if (got.state === 'done') {
        setWaiting(false);
        // The account's things load with the session, on the next load.
        holdSession(got.session);
      } else if (got.state === 'gone') {
        setWaiting(false);
        setNote('Not signed in: the link was declined or has expired. Try again.');
        setInvite(null);
        void mint();
      }
    } catch (e) {
      setError(errorText(e));
    }
  };
  const timer = setInterval(() => void ask(), 2_000);
  // A link is good for ten minutes: a fresh one before it lapses.
  const fresh = setInterval(() => {
    if (!waiting() && !status()?.account) void mint();
  }, 8 * 60_000);
  onCleanup(() => {
    clearInterval(timer);
    clearInterval(fresh);
  });

  const [leaving, setLeaving] = createSignal(false);
  const signOut = async () => {
    if (!confirm('Sign out of this browser? Everything stays with your account; this browser starts empty until you sign in again.')) return;
    setLeaving(true);
    // Ended on the server when it can be; dropped here whatever happens.
    await api.signOut().catch(() => undefined);
    startOver();
  };
  const signOutEverywhere = async () => {
    if (!confirm('Sign out of every browser signed in to this account, this one too? Each one has to sign in with Telegram again.')) return;
    setLeaving(true);
    try {
      await api.signOutEverywhere();
      startOver();
    } catch (e) {
      setLeaving(false);
      setError(errorText(e));
    }
  };

  return (
    <>
      <Show when={status()} fallback={<div class="dim">{error() ?? 'Reading…'}</div>}>
        {(s) => (
          <Show
            when={s().account}
            fallback={
              <div class="set-card">
                <div class="set-account">
                  <span class="avatar out">
                    <Icon name="user" size={18} />
                  </span>
                  <div class="who">
                    <b>Not signed in</b>
                    <span class="dim">This browser keeps its things under its own id only.</span>
                  </div>
                </div>
                <Show when={s().telegram} fallback={<div class="dim">This server has no Telegram bot set up, so signing in is not on offer here.</div>}>
                  <div class="set-actions">
                    <Show when={invite()} fallback={<span class="dim">{s().bot ? 'Making a link…' : 'The bot has not answered yet.'}</span>}>
                      {(link) => (
                        <a
                          class="btn primary big"
                          href={link().url}
                          target="_blank"
                          rel="noopener"
                          onClick={() => {
                            setNote(null);
                            setWaiting(true);
                          }}
                        >
                          <Icon name="send" size={13} />
                          <span>Sign in with Telegram</span>
                        </a>
                      )}
                    </Show>
                    <Show when={waiting()}>
                      <span class="warn">Waiting — press “Yes, sign in” in the Telegram chat.</span>
                    </Show>
                  </div>
                  <p class="note">
                    No password and no phone number: the tradoor bot asks you to confirm in Telegram, and your alerts then come to that chat. If you already have an account,
                    this browser switches to it; anything made here before signing in stays behind.
                  </p>
                </Show>
              </div>
            }
          >
            {(a) => (
              <div class="set-card">
                <div class="set-account">
                  <span class="avatar">{(a().name ?? '?').replace(/^@/, '').slice(0, 1).toUpperCase()}</span>
                  <div class="who">
                    <b>{a().name ?? 'Telegram account'}</b>
                    <span class="dim">Signed in with Telegram · since {formatDateTime(a().since).slice(0, 10)}</span>
                  </div>
                  <button class="btn" disabled={leaving()} onClick={() => void signOut()}>
                    Sign out of this browser
                  </button>
                </div>
                <div class="set-username">
                  <div class="set-username-head">
                    <b>Username</b>
                    <Show when={username()} fallback={<span class="dim">not chosen yet</span>}>
                      {(u) => (
                        <span class="uname-kept">
                          @{u()} <span class="dim">· yours for good</span>
                        </span>
                      )}
                    </Show>
                  </div>
                  <Show
                    when={username()}
                    fallback={
                      <>
                        <p class="note">
                          What you go by where others see you, such as the Frost Scripts you share — in place of your Telegram name, so you can stay anonymous. You choose it once: it can never be changed,
                          and stays with this Telegram account.
                        </p>
                        <UsernameClaim />
                      </>
                    }
                  >
                    <p class="note">Shown with the Frost Scripts you share, in place of your Telegram name. It is bound to this Telegram account and cannot be changed.</p>
                  </Show>
                </div>
                <div class="set-actions">
                  <span class="dim">
                    Signed in on {a().devices ?? 1} browser{(a().devices ?? 1) === 1 ? '' : 's'}.
                  </span>
                  <button class="btn" disabled={leaving()} onClick={() => void signOutEverywhere()}>
                    Sign out everywhere
                  </button>
                </div>
                <p class="note">
                  Sign in on any other browser or device to have the same drawings, journal, layout, watchlists and alerts there. Lost a phone? Sign out everywhere here, or send /signout to the bot
                  in Telegram.
                </p>
              </div>
            )}
          </Show>
        )}
      </Show>
      <Show when={error()}>{(e) => <div class="warn">▲ {e()}</div>}</Show>
      <Show when={note()}>{(n) => <div class="dim">{n()}</div>}</Show>
    </>
  );
}
