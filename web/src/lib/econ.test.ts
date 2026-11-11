/// <reference lib="deno.ns" />
import { assertEquals } from 'jsr:@std/assert@1';
import type { EconEvent } from './api.ts';
import { arrange, countdown, dayName, figure, isDue, nextOf, shiftDay, surprise } from './econ.ts';

const event = (at: number, title: string, more: Partial<EconEvent> = {}): EconEvent => ({ at, day: '2026-11-11', title, allDay: false, core: false, actual: null, forecast: null, previous: null, ...more });

Deno.test('a figure is read whatever it is written in', () => {
  assertEquals(figure('0.4%'), 0.4);
  assertEquals(figure('1,718K'), 1_718_000);
  assertEquals(figure('-1.2B'), -1_200_000_000);
  assertEquals(figure('$3.5'), 3.5);
  assertEquals(figure('n/a'), null);
  assertEquals(figure(null), null);
});

Deno.test('a release is above or below its forecast, by value', () => {
  assertEquals(surprise('0.4%', '0.3%'), 1);
  assertEquals(surprise('194K', '0.2M'), -1, 'in the same unit once read');
  assertEquals(surprise('0.3%', '0.3%'), 0);
  assertEquals(surprise('0.3%', null), 0);
  assertEquals(surprise(null, '0.3%'), 0);
});

Deno.test('a countdown says only as much as it needs', () => {
  assertEquals(countdown(5_000), '00:05');
  assertEquals(countdown(3_661_000), '01:01:01');
  assertEquals(countdown(90_061_000), '1d 01:01:01');
  assertEquals(countdown(-1), '00:00');
});

Deno.test('a release is due from its time until its figure, where one is coming', () => {
  const figures = event(1_000, 'CPI', { forecast: '0.3%' });
  assertEquals(isDue(figures, 999), false, 'not yet');
  assertEquals(isDue(figures, 2_000), true);
  assertEquals(isDue({ ...figures, actual: '0.4%' }, 2_000), false, 'it is in');
  assertEquals(isDue(figures, 1_000 + 21 * 60_000), false, 'and not for ever');
  assertEquals(isDue(event(1_000, 'Fed Chair Speaks'), 2_000), false, 'a speech has no figure');
});

Deno.test('what is next is everything at that time, the core releases first', () => {
  const list = [event(1, 'Old'), event(10, 'Retail Sales'), event(10, 'CPI', { core: true }), event(20, 'Later'), event(5, 'Vague', { allDay: true })];
  assertEquals(nextOf(list, 5).map((e) => e.title), ['CPI', 'Retail Sales']);
  assertEquals(nextOf(list, 30), []);
});

const dayOf = (e: EconEvent) => (e.at < 100 ? 'a' : 'b');
const shape = (rows: ReturnType<typeof arrange>) => rows.map((r) => (r.kind === 'day' ? `#${r.key}` : r.kind === 'now' ? 'now' : `${r.event.title}${r.past ? '.' : ''}`));

Deno.test('now stands between what is out and what is to come', () => {
  const list = [event(10, 'One'), event(20, 'Two'), event(110, 'Three')];
  assertEquals(shape(arrange(list, 15, dayOf)), ['#a', 'One.', 'now', 'Two', '#b', 'Three']);
});

Deno.test('now stands before the heading of a day it has not reached', () => {
  const list = [event(10, 'One'), event(110, 'Three')];
  assertEquals(shape(arrange(list, 50, dayOf)), ['#a', 'One.', 'now', '#b', 'Three']);
});

Deno.test('with everything out, now comes last, and with nothing yet, first', () => {
  const list = [event(10, 'One')];
  assertEquals(shape(arrange(list, 50, dayOf)), ['#a', 'One.', 'now']);
  assertEquals(shape(arrange(list, 5, dayOf)), ['now', '#a', 'One']);
  assertEquals(shape(arrange([], 5, dayOf)), ['now']);
});

Deno.test('a day is named and moved from its date alone', () => {
  assertEquals(dayName('2026-11-11'), 'Wed 11 Nov');
  assertEquals(shiftDay('2026-12-31', 1), '2027-01-01');
  assertEquals(shiftDay('2026-03-01', -1), '2026-02-28');
});
