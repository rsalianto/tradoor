/**
 * A Frost Script pasted from an AI chat, mended where the copy broke it.
 *
 * Some chats copy a code block without its line breaks: every line of the
 * script runs into the next — `"Range Start"input endTime = …`, `list()state`,
 * names fused where a line ended on one and the next began with another
 * (`startTimerangeEnd`) — while the indentation is left behind as runs of
 * spaces. Frost Script reads one statement a line, so such a script fails on
 * its first line. `restoreLines` puts the breaks back where the text says they
 * were; the editor offers it when a script reads as one that lost them.
 */

/** The words a statement begins with, and the ones that end one. */
const STARTS = ['input', 'state', 'fn', 'if', 'for', 'while', 'return', 'break', 'continue', 'pane', 'table'];
const KNOWN = ['na', 'true', 'false', 'and', 'or', 'not', 'else', 'to', 'by', 'from', 'min', 'max', 'step', ...STARTS];

/**
 * Whether a script reads as one whose line breaks were lost: statements joined
 * with nothing between them — a text, a bracket or a brace closed with the next
 * statement's word right after it — or indentation left inside a line.
 */
export function lostLines(src: string): boolean {
  let glued = 0;
  const ps = parts(src);
  ps.forEach((p, i) => {
    if (p.quoted) {
      // A text closed with a word right after it: `"Range Start"input`.
      if (p.text.length > 1 && /^[A-Za-z_]/.test(ps[i + 1]?.text ?? '')) glued++;
      return;
    }
    glued += (p.text.match(/[)\]}](?:input|state|fn|if|for|while|return)\b|\}(?=[A-Za-z_])|[^\s(\[{,] {2,}(?:input|state|fn|if|for|while|return|[A-Za-z_]\w* =)/g) ?? []).length;
  });
  return glued >= 3;
}

/** The text with its strings kept whole: each part says whether it is inside quotes. */
function parts(src: string): { text: string; quoted: boolean }[] {
  const out: { text: string; quoted: boolean }[] = [];
  let i = 0;
  while (i < src.length) {
    if (src[i] === '"') {
      let j = i + 1;
      while (j < src.length && src[j] !== '"' && src[j] !== '\n') j += src[j] === '\\' ? 2 : 1;
      out.push({ text: src.slice(i, Math.min(j + 1, src.length)), quoted: true });
      i = j + 1;
    } else {
      let j = i;
      while (j < src.length && src[j] !== '"') j++;
      out.push({ text: src.slice(i, j), quoted: false });
      i = j;
    }
  }
  return out;
}

/** Every name the script uses, and those it declares: set at a line's start, a setting, a state, a function or its parameters, a loop's. */
function names(src: string): { used: Set<string>; declared: Set<string> } {
  const used = new Set<string>(KNOWN);
  const declared = new Set<string>();
  for (const p of parts(src)) {
    if (p.quoted) continue;
    for (const m of p.text.matchAll(/(?<![\w.])([A-Za-z_]\w*)(?![\w])/g)) used.add(m[1]!);
    for (const m of p.text.matchAll(/^[ \t]*(?:input |state )?([A-Za-z_]\w*)[ \t]*=(?!=)/gm)) declared.add(m[1]!);
    for (const m of p.text.matchAll(/\bfn ([A-Za-z_]\w*)\(([^)]*)\)/g)) {
      declared.add(m[1]!);
      for (const a of m[2]!.split(',')) if (a.trim()) declared.add(a.trim());
    }
    for (const m of p.text.matchAll(/\bfor ([A-Za-z_]\w*) =/g)) declared.add(m[1]!);
  }
  return { used, declared };
}

/**
 * A fused word split where a line ended: `startTimerangeEnd` into `startTime` and
 * `rangeEnd`, `nastate` into `na` and `state`. Never one the script declares itself.
 */
function split(word: string, used: Set<string>, declared: Set<string>): [string, string] | null {
  if (declared.has(word) || KNOWN.includes(word)) return null;
  for (let k = word.length - 1; k >= 1; k--) {
    const a = word.slice(0, k);
    const b = word.slice(k);
    if (!used.has(a) || a === word) continue;
    if (STARTS.includes(b) || (b.length >= 3 && (declared.has(b) || used.has(b)) && b !== word)) return [a, b];
  }
  return null;
}

/** The breaks the text makes plain: indentation left as spaces, a close glued to the next statement. */
function plainBreaks(src: string): string {
  let out = '';
  const ps = parts(src);
  ps.forEach((p, idx) => {
    if (p.quoted) {
      out += p.text;
      // A text that ends a statement, the next one glued to it: `"Range Start"input`.
      const next = ps[idx + 1];
      if (next && !next.quoted && /^[A-Za-z_]/.test(next.text)) out += '\n';
      return;
    }
    let t = p.text;
    // Indentation left behind as a run of spaces: the line break before it.
    t = t.replace(/(\S) {2,}(?=\S)/g, (m, c: string) => `${c}\n${' '.repeat(m.length - 1)}`);
    // The same right after a text: `"L"  if beforeIdx > 0 {`.
    if (idx > 0) t = t.replace(/^ {2,}(?=\S)/, (m) => `\n${m}`);
    // A block's close, the next statement glued on: `}background(`, `}if`; not `} else`.
    t = t.replace(/\}(?! ?else\b)(?= ?[A-Za-z_])/g, '}\n');
    // A block's close right after its last statement: on a line of its own.
    t = t.replace(/(?<=[\w)\]])\}/g, '\n}');
    if (idx > 0) t = t.replace(/^\}/, '\n}');
    // A call's close, the next statement glued on: `list()state`, `pivotlow(…)pivotTime`.
    t = t.replace(/([)\]])(?=[A-Za-z_])/g, '$1\n');
    out += t;
  });
  // A brace opening a block keeps its first statement on the next line.
  return out.replace(/\{ *(?=[^\s}])/g, '{\n');
}

/** The script with its line breaks put back. */
export function restoreLines(src: string): string {
  const plain = plainBreaks(src);
  const { used, declared } = names(plain);
  let out = '';
  for (const p of parts(plain)) {
    if (p.quoted) {
      out += p.text;
      continue;
    }
    out += p.text.replace(/(?<![\w.])[A-Za-z_]\w*/g, (word) => {
      const s = split(word, used, declared);
      return s ? `${s[0]}\n${s[1]}` : word;
    });
  }
  return out
    .split('\n')
    .map((l) => l.replace(/\s+$/, ''))
    .filter((l, i, all) => l !== '' || all[i - 1] !== '')
    .join('\n');
}
