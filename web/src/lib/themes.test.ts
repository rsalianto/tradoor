import { assert, assertEquals } from 'jsr:@std/assert';
import { THEME_KEYS, THEMES, themeOf, withTheme } from './themes.ts';

Deno.test('every theme sets only colours a workspace has, as colours', () => {
  for (const t of THEMES) {
    for (const [k, v] of Object.entries(t.colors)) {
      assert(THEME_KEYS.includes(k), `${t.id}: ${k} is no theme key`);
      assert(k === 'ui.scheme' ? v === 'light' || v === 'dark' : /^#[0-9a-f]{6}$/i.test(v), `${t.id}: ${k} = ${v}`);
    }
  }
  assertEquals(THEMES[0]!.colors, {}, 'tradoor dark is the defaults themselves');
});

Deno.test('a light theme says so, for the browser’s own controls', () => {
  for (const t of THEMES) {
    const light = (t.colors['ui.bg'] ?? '#000000').replace('#', '');
    const bright = parseInt(light.slice(0, 2), 16) > 128;
    assertEquals(t.colors['ui.scheme'] === 'light', bright, t.id);
  }
});

Deno.test('a theme picked is that theme, until a colour of it changes', () => {
  for (const t of THEMES) assertEquals(themeOf(withTheme({}, t)), t.id);
  const tweaked = { ...withTheme({}, THEMES[1]!), bull: '#123456' };
  assertEquals(themeOf(tweaked), null, 'one colour changed by hand is a theme of your own');
});

Deno.test('picking a theme keeps the font and the time zone', () => {
  const before = { fontPx: '13', 'ui.timeZone': 'Asia/Jakarta', bull: '#111111', 'ui.scheme': 'light' };
  const after = withTheme(before, THEMES[1]!);
  assertEquals(after.fontPx, '13');
  assertEquals(after['ui.timeZone'], 'Asia/Jakarta');
  assertEquals(after.bull, THEMES[1]!.colors.bull);
  assertEquals(after['ui.scheme'], undefined, 'a dark theme leaves no light scheme behind');
});
