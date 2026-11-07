/// <reference lib="deno.ns" />
import { assert, assertEquals } from 'jsr:@std/assert@1';
import { isBinance, quietUntil, throttled } from './binance-quiet.ts';

Deno.test('a throttled Binance host is left alone until its minute turns, longer once it bans', () => {
  const now = Date.now();
  const next = Math.ceil((now + 1) / 60_000) * 60_000;
  const at = throttled('fapi.binance.com', 429);
  // Two seconds into the next minute: one more, were it to turn between the two clocks read.
  assert(at === next + 2_000 || at === next + 62_000, 'the next minute, two seconds in');
  assertEquals(quietUntil('fapi.binance.com'), at);
  assertEquals(quietUntil('api.binance.com'), 0);
  assert(throttled('api.binance.com', 418) >= now + 120_000);
});

Deno.test("Binance's hosts are told from others'", () => {
  for (const host of ['fapi.binance.com', 'api.binance.com', 'data-api.binance.vision']) assert(isBinance(host), host);
  for (const host of ['api.bybit.com', 'notbinance.com', 'binance.com.evil.io']) assert(!isBinance(host), host);
});
