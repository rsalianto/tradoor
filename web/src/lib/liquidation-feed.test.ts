/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { followLiquidations, parseEvent } from './liquidation-feed.ts';

Deno.test('server-sent events are read block by block', () => {
  assertEquals(parseEvent('event: hit\ndata: {"venue":"okx-perp"}'), { event: 'hit', data: '{"venue":"okx-perp"}' });
  assertEquals(parseEvent('data: a\ndata: b'), { event: 'message', data: 'a\nb' });
  assertEquals(parseEvent(': keep-alive'), null);
});

Deno.test('followers of one ticker share one stream, closed when the last goes', async () => {
  const real = globalThis.fetch;
  let opened = 0;
  let aborted = 0;
  globalThis.fetch = ((_url: string, init?: RequestInit) => {
    opened++;
    init?.signal?.addEventListener('abort', () => aborted++);
    // A stream that never ends until aborted.
    return Promise.resolve(new Response(new ReadableStream({ start() {} }), { status: 200 }));
  }) as typeof fetch;
  try {
    const quiet = () => ({ hit: () => {}, venues: () => {}, unavailable: () => {} });
    const a = followLiquidations('SHAREUSDT', quiet());
    const b = followLiquidations('SHAREUSDT', quiet());
    const c = followLiquidations('OTHERUSDT', quiet());
    await new Promise((r) => setTimeout(r, 0));
    assertEquals(opened, 2, 'one stream a ticker');
    a();
    assertEquals(aborted, 0, 'still followed');
    b();
    c();
    assertEquals(aborted, 2);
  } finally {
    globalThis.fetch = real;
  }
});
