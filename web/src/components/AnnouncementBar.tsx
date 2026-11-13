/** The owner's announcement, a strip atop the app: written as text, its link opened apart. */
import { Show } from 'solid-js';
import { announcement, closeAnnouncement } from '../lib/announcement';
import { Icon } from '../lib/icons';

export function AnnouncementBar() {
  return (
    <Show when={announcement()} keyed>
      {(a) => (
        <div class="announce" classList={{ warn: a.tone === 'warn' }} role="status">
          <span class="announce-mark" aria-hidden="true">
            ●
          </span>
          <b>{a.title}</b>
          <Show when={a.body}>
            <span class="announce-body">{a.body}</span>
          </Show>
          <Show when={a.link}>
            {(href) => (
              <a href={href()} target="_blank" rel="noopener noreferrer">
                Read more <Icon name="external" size={10} stroke={2} />
              </a>
            )}
          </Show>
          <span class="spacer" />
          <button class="x" aria-label="Close the announcement" data-tip="Close" onClick={closeAnnouncement}>
            <Icon name="close" size={10} stroke={2.4} />
          </button>
        </div>
      )}
    </Show>
  );
}
