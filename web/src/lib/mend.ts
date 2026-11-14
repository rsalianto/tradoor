/**
 * A script pasted from an AI chat, mended: its line breaks put back where the
 * copy lost them (lib/frost-repair), then Pine Script's leftovers written as
 * Frost Script (lib/pine-leftovers). What changed, and what could not be
 * carried over, are said with it. The script editor and the strategy tester's
 * code box both offer it.
 */
import { lostLines, restoreLines } from './frost-repair';
import { pineLeftovers, type Mended } from './pine-leftovers';

export type { Mended };

export function mendScript(src: string): Mended {
  let text = src;
  const changes: string[] = [];
  if (lostLines(text)) {
    text = restoreLines(text);
    changes.push('line breaks put back: the copy had lost them');
  }
  const m = pineLeftovers(text);
  return { text: m.text, changes: [...changes, ...m.changes], left: m.left };
}

/** Whether mending would change anything: what decides the editor offers it. */
export const mendable = (src: string): boolean => lostLines(src) || pineLeftovers(src).changes.length > 0;

/** A paste that reads as one whose line breaks were lost: mended at once, as nothing of the reader's is lost by it. */
export function mendPaste(src: string): { text: string; mended: boolean } {
  if (!lostLines(src)) return { text: src, mended: false };
  return { text: restoreLines(src), mended: true };
}
