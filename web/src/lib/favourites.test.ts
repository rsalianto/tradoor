/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { mergeStars, readHeld } from './favourites.ts';

Deno.test('stars from before signing in join the account’s, after them and once', () => {
  assertEquals(mergeStars(['rsi', 'vwap'], ['vwap', 'ema']), ['rsi', 'vwap', 'ema']);
  assertEquals(mergeStars([], ['ema']), ['ema']);
  assertEquals(mergeStars(['rsi'], []), ['rsi']);
});

Deno.test('a list kept in this browser is only trusted when it reads', () => {
  assertEquals(readHeld(null), null);
  assertEquals(readHeld({ who: 1, kinds: [] }), null);
  assertEquals(readHeld({ who: 'me', kinds: ['ema', 3] }), { who: 'me', kinds: ['ema'] });
});
