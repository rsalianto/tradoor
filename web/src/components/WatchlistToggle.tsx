/**
 * The chart's own way into the watchlist: add the market it shows, or take it
 * out again, without going to the Watchlist panel.
 */
import { Icon } from '../lib/icons';
import { toast } from '../lib/toasts';
import { insertMarket } from '../lib/watchlist';
import type { WatchlistPanelState } from '../workspace/model';
import { doc, patchPanel } from '../workspace/store';

export function WatchlistToggle(props: { market: string }) {
  const lists = () => Object.values(doc.panels).filter((p): p is WatchlistPanelState => p.type === 'watchlist');
  const listed = () => lists().some((l) => l.markets.includes(props.market));
  const toggle = () => {
    const all = lists();
    if (all.length === 0) {
      toast({ tone: 'warn', title: 'No watchlist open', body: 'Open a Watchlist panel first.' });
      return;
    }
    if (!listed()) {
      patchPanel(all[0]!.id, { markets: insertMarket(all[0]!.markets, props.market) });
      return;
    }
    // Out of every watchlist it is in, its flag with it.
    for (const l of all.filter((l) => l.markets.includes(props.market))) {
      const flags = { ...l.flags };
      delete flags[props.market];
      patchPanel(l.id, { markets: l.markets.filter((m) => m !== props.market), flags });
    }
  };
  return (
    <button
      class="tb wl-toggle"
      classList={{ in: listed() }}
      aria-pressed={listed()}
      aria-label={listed() ? 'In your watchlist' : 'Add to your watchlist'}
      data-tip={listed() ? 'In your watchlist — click to remove it' : 'Add this market to your watchlist'}
      onClick={toggle}
    >
      <Icon name="star" size={13} fill={listed()} />
    </button>
  );
}
