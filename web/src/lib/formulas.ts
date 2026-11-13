/**
 * Frost Script: the reader's own indicators, written in tradoor's small language
 * (crates/formula). Only an account writes them — signed in with Telegram — and
 * they are kept on the server (bins/server scripts), so they follow it to every
 * device; this browser holds a copy of its account's to draw charts with at once.
 *
 * On a chart a script is a study like any other, its kind `fx-` and its id:
 * `fx-k3j9a2:14:2` is that script with its settings at 14 and 2. The worker's
 * formula module works it out over the chart's bars; nothing of it runs here.
 *
 * A script its author made public is in the Community, under their creator name,
 * for anyone to add, signed in or not: a chart that holds one reads its text by
 * its id. Made private again, or removed by the moderator, it draws no more.
 */
import { createSignal } from 'solid-js';
import { call } from '../engine/client';
import type { FormulaDescription, FormulaRun, StudyDefinition } from '../engine/protocol';
import { api, type CommunityScript, type Script, type ScriptPreview } from './api';
import { identity, session } from './owner';
import { readJson, writePref } from './prefs';
import { noteUsername, username } from './username';

/** A script of the reader's own, or, with `by` its author's username, someone else's public one. */
export type Formula = Script & { by?: string };

const KEY = 'frostScripts';

/** The copy held for whoever this browser is now: another account's is not shown. */
function held(): Formula[] {
  const raw = readJson(KEY) as { who?: unknown; scripts?: unknown } | null;
  if (!session() || raw?.who !== identity() || !Array.isArray(raw.scripts)) return [];
  return raw.scripts.filter((f): f is Formula => !!f && typeof f.id === 'string' && typeof f.name === 'string' && typeof f.source === 'string');
}

const [list, setList] = createSignal<Formula[]>(held());
export const formulas = list;
/** Whether this browser may write scripts: signed in with Telegram. */
export const canWrite = (): boolean => session() !== null;

function keep(next: Formula[]): void {
  setList(next);
  writePref(KEY, JSON.stringify({ who: identity(), scripts: next }));
}

let loading: Promise<void> | null = null;

/** The account's scripts as the server has them: once a page, after the copy held here is drawn. */
export function loadFormulas(): Promise<void> {
  if (!session()) return Promise.resolve();
  loading ??= api
    .scripts()
    .then((r) => {
      keep(r.scripts);
      noteUsername(r.username);
    })
    .catch(() => {
      // Out of reach: the copy held here stands, and the next page asks again.
      loading = null;
    });
  return loading;
}

/** Kept on the server, new or over the one of its id. Throws what the server said. */
export async function saveFormula(f: { id?: string; name: string; source: string; public: boolean; about: string; preview?: ScriptPreview }): Promise<Formula> {
  const body = { name: f.name.trim().slice(0, 40) || 'My script', source: f.source, public: f.public, about: f.about, preview: f.preview };
  const saved = f.id ? await api.updateScript(f.id, body) : await api.createScript(body);
  keep([saved, ...list().filter((x) => x.id !== saved.id)]);
  return saved;
}

export async function deleteFormula(id: string): Promise<void> {
  await api.deleteScript(id);
  keep(list().filter((f) => f.id !== id));
}

export const kindOf = (f: Pick<Formula, 'id'>): string => `fx-${f.id}`;
export const isFormulaKind = (kind: string): boolean => kind.startsWith('fx-');

// ---------------------------------------------------------------- the Community

const [listed, setListed] = createSignal<CommunityScript[] | null>(null);
/** Everyone's public scripts, the last changed first; null until read. */
export const community = listed;
const [communityError, setCommunityError] = createSignal<string | null>(null);
export { communityError };
let listedAt = 0;
let listing: Promise<void> | null = null;

/** The Community's list: read when first shown, and again once it is a minute old. */
export function loadCommunity(): Promise<void> {
  if (listing) return listing;
  if (listed() && Date.now() - listedAt < 60_000) return Promise.resolve();
  const read = api
    .community()
    .then((r) => {
      setListed(r.scripts);
      setCommunityError(null);
      listedAt = Date.now();
    })
    .catch((e: unknown) => {
      setCommunityError(e instanceof Error ? e.message : String(e));
    })
    .finally(() => {
      listing = null;
    });
  listing = read;
  return read;
}

/** Others' public scripts the charts hold, by id, read once a page: `gone` once private or removed. */
const [shared, setShared] = createSignal<ReadonlyMap<string, Formula | 'gone'>>(new Map());
const asked = new Set<string>();

function readShared(id: string): void {
  if (asked.has(id)) return;
  asked.add(id);
  api
    .communityScript(id)
    .then((s): Formula => ({ id: s.id, name: s.name, source: s.source, checks: true, updated: s.updated, removed: false, why: null, public: true, about: s.about, preview: s.preview, by: s.username }))
    .then(
      (f) => setShared((m) => new Map(m).set(id, f)),
      (e: unknown) => {
        // Not found: made private or removed. Out of reach: asked again on the next page.
        if ((e as { status?: number }).status === 404) setShared((m) => new Map(m).set(id, 'gone'));
        else asked.delete(id);
      },
    );
}

/** A script by its kind: the reader's own, or someone's public one, read the first time it is asked for. */
export function formulaOf(kind: string): Formula | undefined {
  const own = list().find((f) => kindOf(f) === kind);
  if (own || !isFormulaKind(kind)) return own;
  const id = kind.slice(3);
  const got = shared().get(id);
  if (got === undefined) readShared(id);
  return got === 'gone' ? undefined : got;
}

/**
 * Which of a script's settings are numbers, in their order: the legend names those alone,
 * as a tick, a colour, a choice or a moment reads as a raw number there.
 */
function numberSettings(source: string): boolean[] {
  return [...source.matchAll(/^[ \t]*input[ \t]+\w+[ \t]*=[ \t]*(\S)/gm)].map((m) => /[-+(.\d]/.test(m[1]!));
}

/** A chart's formulas from its studies, ready for the worker: one gone from the library is left out. */
export function formulaRuns(studies: string): FormulaRun[] {
  return studies
    .split(',')
    .filter((id) => isFormulaKind(id))
    .flatMap((id) => {
      const [kind, ...params] = id.split(':');
      const f = formulaOf(kind!);
      // One the moderator removed runs for no one, its author included.
      if (!f || f.removed) return [];
      // A shared script says whose it is, on every chart it is on.
      const author = f.by ?? (f.public ? username() : null);
      const numbers = numberSettings(f.source);
      const shown = params.filter((_, i) => numbers[i] ?? true);
      return [{ id, label: [f.name, ...shown].join(' ') + (author ? ` · @${author}` : ''), source: f.source, inputs: params.map(Number) }];
    });
}

/** Checked by the worker's formula module, one answer kept per text. */
const checked = new Map<string, Promise<FormulaDescription>>();
export function describe(source: string): Promise<FormulaDescription> {
  let answer = checked.get(source);
  if (!answer) {
    if (checked.size > 40) checked.clear();
    answer = call('formulaDescribe', { source }).catch((e): FormulaDescription => ({ ok: false, line: 0, col: 0, message: e instanceof Error ? e.message : String(e) }));
    checked.set(source, answer);
  }
  return answer;
}

/** As a study definition, for its settings: each setting a parameter. */
export async function formulaDefinition(kind: string): Promise<StudyDefinition | null> {
  const f = formulaOf(kind);
  if (!f) return null;
  const d = await describe(f.source);
  // Each setting's own range: a yes or no, a colour's four bytes, a choice's places, any moment.
  const range = (i: (typeof d & { ok: true })['inputs'][number]): [number, number] => {
    switch (i.kind) {
      case 'bool':
        return [0, 1];
      case 'colour':
        return [0, 4_294_967_295];
      case 'choice':
        return [0, Math.max(0, i.options.length - 1)];
      case 'time':
        return [-8.64e15, 8.64e15];
      default:
        return [i.min ?? -1e12, i.max ?? 1e12];
    }
  };
  const params = d.ok ? d.inputs.map((i) => ({ name: i.name, default: i.default, min: range(i)[0], max: range(i)[1], step: i.kind === 'number' ? i.step : 1, kind: i.kind, options: i.options })) : [];
  return { kind, short: f.name, group: d.ok && d.ownPane ? 'formula' : 'price', params };
}

/** Its settings' labels, as the formula writes them. */
export async function formulaLabels(kind: string): Promise<Record<string, string>> {
  const f = formulaOf(kind);
  const d = f ? await describe(f.source) : null;
  return d?.ok ? Object.fromEntries(d.inputs.map((i) => [i.name, i.label])) : {};
}

/** The editor, open on a formula of the library, a new one from `seed`, or a blank page; for a chart to add it to. */
export const [editing, setEditing] = createSignal<{ id?: string; seed?: { name: string; source: string }; panelId?: string } | null>(null);
