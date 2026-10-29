/**
 * The server's live liquidations for one ticker: every exchange it follows, as
 * server-sent events over one fetch. Read with fetch rather than EventSource so
 * the chart's worker can use it too. No Solid here, for the same reason.
 */

export interface ServerHit {
  venue: string;
  time: number;
  price: number;
  qty: number;
  usd: number;
  long: boolean;
}

export interface ServerVenue {
  venue: string;
  /** `connecting`, `connected` or `retrying`. */
  state: string;
  since: number;
  error: string | null;
}

export interface FeedHandlers {
  hit(hit: ServerHit): void;
  venues(venues: ServerVenue[]): void;
  /** The server does not follow this ticker, or cannot be reached: use the exchanges directly. */
  unavailable(): void;
}

const RETRY_MS = [1_000, 3_000, 5_000, 10_000, 30_000];

/** One server-sent event block: its `event` and `data` lines. */
export function parseEvent(block: string): { event: string; data: string } | null {
  let event = 'message';
  const data: string[] = [];
  for (const line of block.split('\n')) {
    if (line.startsWith('event:')) event = line.slice(6).trim();
    else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
  }
  return data.length > 0 ? { event, data: data.join('\n') } : null;
}

interface Shared {
  followers: Set<FeedHandlers>;
  stop: () => void;
  venues: ServerVenue[] | null;
  unavailable: boolean;
}

/** One stream a ticker, however many follow it here: four charts on BTC are one stream, not four. */
const shared = new Map<string, Shared>();

/**
 * Follow `ticker` until the returned function is called. Followers of one
 * ticker in this page (or this worker) share a stream: each held a connection,
 * and a browser keeps six to an address — a few charts on one ticker left the
 * page none to load with — while the server allows an address eight.
 */
export function followLiquidations(ticker: string, on: FeedHandlers): () => void {
  let feed = shared.get(ticker);
  if (!feed) {
    const f: Shared = { followers: new Set(), stop: () => {}, venues: null, unavailable: false };
    f.stop = openFeed(ticker, {
      hit: (h) => {
        for (const l of f.followers) l.hit(h);
      },
      venues: (v) => {
        f.venues = v;
        for (const l of f.followers) l.venues(v);
      },
      unavailable: () => {
        f.unavailable = true;
        for (const l of f.followers) l.unavailable();
      },
    });
    shared.set(ticker, f);
    feed = f;
  }
  const f = feed;
  f.followers.add(on);
  // One joining late hears what the rest already know.
  if (f.unavailable) queueMicrotask(() => f.followers.has(on) && on.unavailable());
  else if (f.venues) on.venues(f.venues);
  return () => {
    if (!f.followers.delete(on) || f.followers.size > 0) return;
    f.stop();
    if (shared.get(ticker) === f) shared.delete(ticker);
  };
}

function openFeed(ticker: string, on: FeedHandlers): () => void {
  let stopped = false;
  let controller: AbortController | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let attempt = 0;
  let everConnected = false;

  const run = async () => {
    controller = new AbortController();
    try {
      const res = await fetch(`/api/liquidations/${encodeURIComponent(ticker)}/live`, { signal: controller.signal, headers: { accept: 'text/event-stream' } });
      if (!res.ok || !res.body) {
        // Not followed: the direct streams take over for good.
        if (!everConnected) {
          on.unavailable();
          return;
        }
        throw new Error(`server answered ${res.status}`);
      }
      everConnected = true;
      attempt = 0;
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      let buffer = '';
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += value.replace(/\r\n/g, '\n');
        let end: number;
        while ((end = buffer.indexOf('\n\n')) !== -1) {
          const parsed = parseEvent(buffer.slice(0, end));
          buffer = buffer.slice(end + 2);
          if (!parsed) continue;
          try {
            if (parsed.event === 'hit') on.hit(JSON.parse(parsed.data) as ServerHit);
            else if (parsed.event === 'venues') on.venues(JSON.parse(parsed.data) as ServerVenue[]);
          } catch {
            // One bad event is skipped, not the feed.
          }
        }
      }
    } catch {
      if (stopped) return;
      if (!everConnected) {
        on.unavailable();
        return;
      }
    }
    if (stopped) return;
    timer = setTimeout(() => void run(), RETRY_MS[Math.min(attempt++, RETRY_MS.length - 1)]);
  };
  void run();
  return () => {
    stopped = true;
    clearTimeout(timer);
    controller?.abort();
  };
}
