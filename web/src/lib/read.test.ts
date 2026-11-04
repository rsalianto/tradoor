/// <reference lib="deno.ns" />
import { assertEquals, assertRejects } from 'jsr:@std/assert@1';
import { read } from './read.ts';

/** `fetch` answering from a list, one for each call, with how many calls it took. */
function answers(...list: (number | 'hang' | 'fail')[]) {
  const original = globalThis.fetch;
  const asked = { count: 0 };
  globalThis.fetch = ((_: string, init?: RequestInit) => {
    const next = list[asked.count++];
    if (next === 'fail') return Promise.reject(new TypeError('network'));
    if (next === 'hang') return new Promise((_, no) => init?.signal?.addEventListener('abort', () => no(new DOMException('aborted', 'AbortError'))));
    return Promise.resolve(new Response(next === 304 ? null : 'body', { status: next }));
  }) as typeof fetch;
  return { asked, restore: () => (globalThis.fetch = original) };
}

const quick = [0, 0];

Deno.test('a read answered at once is asked once', async () => {
  const f = answers(200);
  assertEquals((await read('/x', {}, quick)).status, 200);
  assertEquals(f.asked.count, 1);
  f.restore();
});

Deno.test("a gateway that did not reach the server is asked again", async () => {
  const f = answers(522, 200);
  assertEquals((await read('/x', {}, quick)).status, 200);
  assertEquals(f.asked.count, 2);
  f.restore();
});

Deno.test('no answer at all is asked again', async () => {
  const f = answers('fail', 'fail', 200);
  assertEquals((await read('/x', {}, quick)).status, 200);
  assertEquals(f.asked.count, 3);
  f.restore();
});

Deno.test('a connection that hangs is given up on, not waited out', async () => {
  const f = answers('hang', 200);
  assertEquals((await read('/x', {}, quick, 5)).status, 200);
  assertEquals(f.asked.count, 2);
  f.restore();
});

Deno.test("the server's own no is the answer, not asked again", async () => {
  for (const status of [304, 400, 401, 403, 404, 429, 500]) {
    const f = answers(status, 200);
    assertEquals((await read('/x', {}, quick)).status, status);
    assertEquals(f.asked.count, 1);
    f.restore();
  }
});

Deno.test('three tries at most, and the last one is what is told', async () => {
  const f = answers(522, 522, 522, 200);
  assertEquals((await read('/x', {}, quick)).status, 522);
  assertEquals(f.asked.count, 3);
  f.restore();

  const down = answers('fail', 'fail', 'fail', 200);
  await assertRejects(() => read('/x', {}, quick));
  assertEquals(down.asked.count, 3);
  down.restore();
});
