/**
 * Choosing the account's username, once: a field, then a word that it is for
 * good before it is kept. Settings shows it, and the Frost Script editor when a
 * script is first shared.
 */
import { createSignal, Show } from 'solid-js';
import { errorText } from '../lib/api';
import { claimUsername, usernameProblem } from '../lib/username';

export function UsernameClaim(props: { button?: string }) {
  const [draft, setDraft] = createSignal('');
  const [sure, setSure] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const name = () => draft().trim().replace(/^@/, '');
  const problem = () => (name() ? usernameProblem(name()) : null);
  const button = () => props.button ?? 'btn';

  const keep = async () => {
    setBusy(true);
    try {
      await claimUsername(name());
    } catch (e) {
      setError(errorText(e));
      setSure(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="uname">
      <Show
        when={sure()}
        fallback={
          <>
            <div class="uname-row">
              <span class="uname-at">@</span>
              <input
                class="uname-input"
                value={draft()}
                maxLength={21}
                placeholder="your_username"
                aria-label="Username"
                spellcheck={false}
                autocomplete="off"
                autocapitalize="off"
                onInput={(e) => {
                  setDraft(e.currentTarget.value);
                  setError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && name() && !problem()) setSure(true);
                }}
              />
              <button class={button()} disabled={!name() || !!problem()} onClick={() => setSure(true)}>
                Choose
              </button>
            </div>
            <Show when={problem() ?? error()}>{(m) => <span class="warn">▲ {m()}</span>}</Show>
          </>
        }
      >
        <div class="uname-sure">
          <span>
            Keep <b>@{name()}</b> for good? It can never be changed, and stays with this Telegram account.
          </span>
          <button class={`${button()} primary`} disabled={busy()} onClick={() => void keep()}>
            Yes, keep @{name()}
          </button>
          <button class={button()} disabled={busy()} onClick={() => setSure(false)}>
            Back
          </button>
        </div>
      </Show>
    </div>
  );
}
