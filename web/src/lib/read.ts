/** What a gateway answers for a server it did not reach: Cloudflare's 52x and 530 among them. */
const UNREACHED: ReadonlySet<number> = new Set([502, 503, 504, 520, 521, 522, 523, 524, 530]);
/** Cloudflare gives up on a server only after 20 s; one that can answer has long since started to. */
const PATIENCE_MS = 8_000;
const PAUSES_MS: readonly number[] = [700, 2_000];

/**
 * A read that outlasts a bad moment on the way to the server: when the way failed,
 * rather than the server saying no, it asks again, and the next connection usually
 * opens. Writes never come through here: one landing late could undo a newer one.
 */
export async function read(path: string, headers: Record<string, string>, pauses = PAUSES_MS, patience = PATIENCE_MS): Promise<Response> {
  for (const pause of pauses) {
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), patience);
    try {
      const res = await fetch(path, { headers, signal: abort.signal });
      if (!UNREACHED.has(res.status)) return res;
      void res.body?.cancel();
    } catch {
      // No answer, or none in time: the next try.
    } finally {
      clearTimeout(timer);
    }
    await new Promise((done) => setTimeout(done, pause));
  }
  // The last try may take as long as it takes, and what it gets is the answer.
  return fetch(path, { headers });
}
