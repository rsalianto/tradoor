/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import { TPO_SECTIONS } from './tpo-schema.ts';
import { DEFAULT_SESSIONS, parseSessions, serializeSessions, serializeTpo, TPO_DEFAULTS, tpoValue, withTpoValue } from './tpo-settings.ts';

Deno.test('the tabs are laid out as MMT lays them out', () => {
  assertEquals(
    TPO_SECTIONS.map((s) => s.title),
    ['General', 'Session', 'Point of Control', 'Value Area', 'Single Prints', 'Poor High Low', 'Initial Balance', 'Volume Profile'],
  );
});

Deno.test('a chart keeps only what differs from the defaults', () => {
  let s = withTpoValue({}, 'vp', '1');
  s = withTpoValue(s, 'vaPct', '68');
  assertEquals(s, { vp: '1', vaPct: '68' });
  assertEquals(withTpoValue(s, 'vaPct', '70'), { vp: '1' }, 'back to the default is no setting at all');
  assertEquals(tpoValue({}, 'highlight'), '1');
  assertEquals(tpoValue(s, 'vp'), '1');
});

Deno.test('the engine is sent known keys only', () => {
  assertEquals(serializeTpo({ vp: '1', poorColor: '#112233', letters: '1', bogus: 'x', vaColor: 'bad;value' }), 'vp=1;poorColor=#112233');
});

Deno.test('every field in the dialog has a default, and every default a field', () => {
  const fields = TPO_SECTIONS.flatMap((s) => s.fields.filter((f) => f.kind !== 'period').map((f) => f.key)).sort();
  assertEquals(fields, Object.keys(TPO_DEFAULTS).sort());
});

Deno.test('custom sessions read and write as the engine reads them', () => {
  const list = parseSessions(DEFAULT_SESSIONS);
  assertEquals(list.length, 4, "MMT's four");
  assertEquals(list[3], { start: 1200, end: 0, color: '#2aa12a' }, '20:00 to midnight');
  assertEquals(serializeSessions(list), DEFAULT_SESSIONS);
  assertEquals(parseSessions('0800-1630#FF0000,9999-0000#000000,junk'), [{ start: 480, end: 990, color: '#ff0000' }], 'what does not read is skipped');
});
