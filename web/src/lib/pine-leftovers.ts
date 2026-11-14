/**
 * What an AI conversion of a Pine Script v6 script tends to leave behind,
 * written as Frost Script: `ta.` and `math.` names, `color.new`, `:=`, `var`
 * and typed declarations, `input.int(…)`, blocks set off by indentation alone,
 * `=>` functions, `[a, b] =`, `switch`, `bgcolor`, `plotshape`, a strategy's
 * `strategy.entry` and the rest. Each change is said, and so is anything it
 * cannot carry over, for the reader to see before saving. Text inside quotes is
 * never touched. Loaded with the editor.
 */

export interface Mended {
  text: string;
  /** What was changed, a line each kind. */
  changes: string[];
  /** What could not be carried over, for the reader to see to. */
  left: string[];
}

// ---------------------------------------------------------------- texts kept aside

/** Texts in quotes, either kind, set aside as `\u0001n\u0001` while the rest is rewritten. */
function mask(src: string): { code: string; texts: string[] } {
  const texts: string[] = [];
  let code = '';
  let i = 0;
  while (i < src.length) {
    const c = src[i]!;
    if (c === '/' && src[i + 1] === '/') {
      // A comment runs to the line's end, quotes and all.
      const end = src.indexOf('\n', i);
      const j = end < 0 ? src.length : end;
      texts.push(src.slice(i, j));
      code += `\u0002${texts.length - 1}\u0002`;
      i = j;
      continue;
    }
    if (c === '"' || c === "'") {
      let j = i + 1;
      while (j < src.length && src[j] !== c && src[j] !== '\n') j += src[j] === '\\' ? 2 : 1;
      const body = src.slice(i + 1, Math.min(j, src.length));
      // Pine's single quotes are Frost Script's double ones.
      texts.push(`"${c === "'" ? body.replace(/"/g, '\\"') : body}"`);
      code += `\u0001${texts.length - 1}\u0001`;
      i = j + 1;
      continue;
    }
    code += c;
    i++;
  }
  return { code, texts };
}

const unmask = (code: string, texts: string[]): string => code.replace(/[\u0001\u0002](\d+)[\u0001\u0002]/g, (_, n: string) => texts[Number(n)] ?? '');
const textOf = (masked: string, texts: string[]): string | null => {
  const m = /^\s*\u0001(\d+)\u0001\s*$/.exec(masked);
  return m ? (texts[Number(m[1])] ?? null) : null;
};

// ---------------------------------------------------------------- calls

interface Call {
  start: number;
  /** Past the closing bracket. */
  end: number;
  args: string[];
  named: Map<string, string>;
}

/** The arguments of the call whose `(` is at `open`: split at its own commas, `name=value` apart. */
function callAt(code: string, nameStart: number, open: number): Call | null {
  let depth = 0;
  let from = open + 1;
  const pieces: string[] = [];
  for (let i = open; i < code.length; i++) {
    const c = code[i];
    if (c === '(' || c === '[') depth++;
    else if (c === ')' || c === ']') {
      depth--;
      if (depth === 0) {
        pieces.push(code.slice(from, i));
        const args: string[] = [];
        const named = new Map<string, string>();
        for (const p of pieces) {
          const m = /^\s*([A-Za-z_]\w*)\s*=(?!=)\s*([\s\S]*)$/.exec(p);
          if (m) named.set(m[1]!, m[2]!.trim());
          else if (p.trim() !== '') args.push(p.trim());
        }
        return { start: nameStart, end: i + 1, args, named };
      }
    } else if (c === ',' && depth === 1) {
      pieces.push(code.slice(from, i));
      from = i + 1;
    }
  }
  return null;
}

/** Each call of `name(…)` rewritten by `f`, innermost first so arguments are rewritten too; null leaves it. */
function rewriteCalls(code: string, name: string, f: (c: Call) => string | null): string {
  const re = new RegExp(`(?<![\\w.])${name.replace(/\./g, '\\.')}\\s*\\(`, 'g');
  for (let guard = 0; guard < 500; guard++) {
    const hits = [...code.matchAll(re)];
    let done = false;
    // The last first: its arguments hold no call of the same name left to rewrite.
    for (const m of hits.reverse()) {
      const open = m.index! + m[0].length - 1;
      const call = callAt(code, m.index!, open);
      if (!call) continue;
      const out = f(call);
      if (out === null) continue;
      code = code.slice(0, call.start) + out + code.slice(call.end);
      done = true;
      break;
    }
    if (!done) return code;
  }
  return code;
}

/** Arguments by place or by name, as a Pine function's are given. */
const arg = (c: Call, i: number, ...names: string[]): string | undefined => {
  for (const n of names) if (c.named.has(n)) return c.named.get(n);
  return c.args[i];
};

// ---------------------------------------------------------------- words

const TYPES = 'int|float|bool|string|color|line|label|box|table|linefill|polyline|chart\\.point|array<[^>]*>|matrix<[^>]*>|map<[^>]*>';
const PLACE: Record<string, string> = {
  'location.abovebar': 'above',
  'location.top': 'above',
  'location.belowbar': 'below',
  'location.bottom': 'below',
  'location.absolute': 'below',
  'label.style_label_down': 'above',
  'label.style_label_up': 'below',
  'label.style_label_left': 'right',
  'label.style_label_right': 'left',
  'label.style_label_center': 'center',
  'label.style_none': 'center',
  'yloc.abovebar': 'above',
  'yloc.belowbar': 'below',
};
const WORDS: [RegExp, string][] = [
  [/\bbar_index\b/g, 'bar'],
  [/\blast_bar_index\b/g, 'lastbar'],
  [/\bbarstate\.islast(?:confirmedhistory)?\b/g, 'islast'],
  [/\bbarstate\.isfirst\b/g, 'isfirst'],
  [/\bbarstate\.(?:isconfirmed|isnew|ishistory)\b/g, 'true'],
  [/\bbarstate\.isrealtime\b/g, 'false'],
  [/\bsyminfo\.mintick\b/g, 'tick'],
  [/\bta\.tr\b(?!\s*\()/g, 'tr'],
  [/\bmath\.pi\b/g, 'pi'],
  [/\bmath\.e\b/g, 'exp(1)'],
  [/\bstrategy\.long\b/g, '1'],
  [/\bstrategy\.short\b/g, '-1'],
  [/\bstrategy\.position_avg_price\b/g, 'entryprice'],
  [/\bstrategy\.position_size\b/g, '(inlong ? 1 : inshort ? -1 : 0)'],
  [/\bstrategy\.opentrades\b(?!\.)/g, '(inlong or inshort ? 1 : 0)'],
  [/\b(?:hline|line|plot)\.style_(?:solid|line|linebr|area|areabr)\b/g, 'solid'],
  [/\b(?:hline|line)\.style_dashed\b/g, 'dashed'],
  [/\b(?:hline|line)\.style_dotted\b/g, 'dotted'],
  [/\bplot\.style_stepline(?:_diamond)?\b/g, 'steps'],
  [/\bplot\.style_(?:circles|cross)\b/g, 'dots'],
  [/\bline\.style_arrow_\w+\b/g, 'solid'],
  [/\bextend\.(none|left|right|both)\b/g, '$1'],
  [/\bsize\.(tiny|small|normal|large|huge|auto)\b/g, '$1'],
  [/\bposition\.(top|middle|bottom)_(left|center|right)\b/g, '"$1 $2"'],
];

/** The Pine functions that give several values, written as Frost Script functions where they are used. */
const TUPLES: Record<string, string> = {
  macd: `fn pine_macd(source, fast, slow, signal) {
  macdLine = ema(source, fast) - ema(source, slow)
  signalLine = ema(macdLine, signal)
  return macdLine, signalLine, macdLine - signalLine
}`,
  bb: `fn pine_bb(source, length, mult) {
  basis = sma(source, length)
  dev = mult * stdev(source, length)
  return basis, basis + dev, basis - dev
}`,
  kc: `fn pine_kc(source, length, mult) {
  basis = ema(source, length)
  span = ema(tr, length)
  return basis, basis + mult * span, basis - mult * span
}`,
  dmi: `fn pine_dmi(diLength, adxSmoothing) {
  up = change(high)
  down = -change(low)
  plusDM = isna(up) ? na : (up > down and up > 0 ? up : 0)
  minusDM = isna(down) ? na : (down > up and down > 0 ? down : 0)
  range = rma(tr, diLength)
  plus = 100 * rma(plusDM, diLength) / range
  minus = 100 * rma(minusDM, diLength) / range
  sum = plus + minus
  adx = 100 * rma(abs(plus - minus) / (sum == 0 ? 1 : sum), adxSmoothing)
  return plus, minus, adx
}`,
  supertrend: `fn pine_supertrend(factor, atrPeriod) {
  range = atr(atrPeriod)
  upper = hl2 + factor * range
  lower = hl2 - factor * range
  lower = lower > nz(lower[1]) or close[1] < nz(lower[1]) ? lower : nz(lower[1])
  upper = upper < nz(upper[1]) or close[1] > nz(upper[1]) ? upper : nz(upper[1])
  direction = 1
  if isna(range[1]) {
    direction = 1
  } else if trend[1] == upper[1] {
    direction = close > upper ? -1 : 1
  } else {
    direction = close < lower ? 1 : -1
  }
  trend = direction == -1 ? lower : upper
  return trend, direction
}`,
};

// ---------------------------------------------------------------- blocks

/** Pine's blocks, set off by indentation alone, given braces; `=>` functions and `switch` written as Frost Script's. */
function braces(code: string, changes: Set<string>): string {
  const lines = code.split('\n');
  const out: string[] = [];
  // Each open block: its line's indentation, and whether it is a function (its last line is its value).
  const open: { indent: number; fn: boolean; body: number }[] = [];
  const indentOf = (l: string) => l.length - l.trimStart().length;
  const blank = (l: string) => l.replace(/\u0002\d+\u0002/g, '').trim() === '';
  const close = (upTo: number, next: string | null) => {
    while (open.length > 0 && open[open.length - 1]!.indent >= upTo) {
      const b = open.pop()!;
      if (b.fn) {
        // A function gives back its last line's value, as Pine's does.
        for (let k = out.length - 1; k > b.body; k--) {
          if (blank(out[k]!)) continue;
          const l = out[k]!;
          const t = l.trim();
          const depthHere = indentOf(l);
          if (/^(return|if|for|while|\}|fn|state)\b/.test(t) || /^[A-Za-z_]\w*\s*=(?!=)/.test(t) || /^[A-Za-z_]\w*(\s*,\s*[A-Za-z_]\w*)+\s*=(?!=)/.test(t)) break;
          const value = t.startsWith('[') && t.endsWith(']') ? t.slice(1, -1) : t;
          out[k] = `${' '.repeat(depthHere)}return ${value}`;
          break;
        }
      }
      const pad = ' '.repeat(b.indent);
      // Before the blank lines the block ended on, not after them.
      let at = out.length;
      while (at > b.body + 1 && blank(out[at - 1]!)) at--;
      if (next !== null && b.indent === upTo && /^else\b/.test(next.trim())) {
        out.splice(at, 0, `${pad}} ${next.trim()}${next.trim().endsWith('{') ? '' : ' {'}`);
        open.push({ indent: b.indent, fn: false, body: at });
        return true;
      }
      out.splice(at, 0, `${pad}}`);
    }
    return false;
  };
  for (let i = 0; i < lines.length; i++) {
    let line = lines[i]!;
    if (blank(line)) {
      out.push(line);
      continue;
    }
    const indent = indentOf(line);
    if (open.length > 0 && indent <= open[open.length - 1]!.indent) {
      if (close(indent, line)) continue;
    }
    let t = line.trim();
    // A one-line function: name(a, b) => value.
    let m = /^([A-Za-z_]\w*)\s*\(([^()]*)\)\s*=>\s*(\S[\s\S]*)$/.exec(t);
    if (m) {
      out.push(`${' '.repeat(indent)}fn ${m[1]}(${params(m[2]!)}) {`, `${' '.repeat(indent + 2)}return ${m[3]}`, `${' '.repeat(indent)}}`);
      changes.add('=> functions written as fn … { return … }');
      continue;
    }
    // A function's first line: name(a, b) =>, its lines below.
    m = /^(?:method\s+)?([A-Za-z_]\w*)\s*\(([^()]*)\)\s*=>\s*$/.exec(t);
    if (m) {
      out.push(`${' '.repeat(indent)}fn ${m[1]}(${params(m[2]!)}) {`);
      open.push({ indent, fn: true, body: out.length - 1 });
      changes.add('=> functions written as fn … { return … }');
      continue;
    }
    // switch: each case a test, as a chain of ? : — for its value, or the function's.
    m = /^(?:([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\s*=\s*)?switch\b\s*(.*)$/.exec(t);
    if (m) {
      const subject = m[2]!.trim();
      const cases: [string | null, string][] = [];
      let j = i + 1;
      while (j < lines.length && (blank(lines[j]!) || indentOf(lines[j]!) > indent)) {
        const c = /^(.*?)=>\s*(.+)$/.exec(lines[j]!.trim());
        if (c) cases.push([c[1]!.trim() || null, c[2]!.trim()]);
        j++;
      }
      let expr = 'na';
      for (const [test, value] of [...cases].reverse()) {
        if (test === null) expr = value;
        else expr = `${subject ? `${subject} == ${test}` : test} ? ${value} : ${expr}`;
      }
      out.push(`${' '.repeat(indent)}${m[1] ? `${m[1]} = ` : ''}${expr}`);
      changes.add('switch written as a chain of ? :');
      i = j - 1;
      continue;
    }
    // A block of Pine's: if, else, for, while with no brace, its lines indented below.
    const nextLine = lines.slice(i + 1).find((l) => !blank(l));
    const opens = /^(if|else|for|while)\b/.test(t) && !t.endsWith('{') && nextLine !== undefined && indentOf(nextLine) > indent;
    if (opens) {
      if (/^else\b/.test(t) && out.length > 0) {
        // An else whose if was braced already: on the closing brace's line.
        out.push(`${' '.repeat(indent)}${t} {`);
      } else out.push(`${' '.repeat(indent)}${t} {`);
      open.push({ indent, fn: false, body: out.length - 1 });
      changes.add('blocks set off by indentation given braces');
      continue;
    }
    line = `${' '.repeat(indent)}${t}`;
    out.push(line);
  }
  close(0, null);
  return out.join('\n');
}

/** A function's parameters without their types: `int ms, float x` → `ms, x`. */
const params = (p: string) =>
  p
    .split(',')
    .map((x) => x.trim().replace(new RegExp(`^(?:(?:series|simple|const)\\s+)?(?:${TYPES})\\s+`), '').replace(/\s*=.*$/, ''))
    .filter(Boolean)
    .join(', ');

// ---------------------------------------------------------------- the pass

/** Pine Script's leftovers in a script, written as Frost Script. */
export function pineLeftovers(src: string): Mended {
  const changes = new Set<string>();
  const left = new Set<string>();
  let { code, texts } = mask(src.replace(/\r\n?/g, '\n').replace(/\t/g, '    '));
  const before = code;
  const text = (s: string | undefined) => (s === undefined ? null : textOf(s, texts));

  // The version line, a slash short or not.
  code = code.replace(/^\s*\/@version\s*=\s*\d+\s*$/gm, () => {
    changes.add('the version line taken out');
    return '';
  });
  // The script's own line: indicator(…), strategy(…).
  for (const name of ['indicator', 'strategy', 'study', 'library']) {
    code = rewriteCalls(code, name, (c) => {
      const lineStart = code.lastIndexOf('\n', c.start - 1) + 1;
      if (code.slice(lineStart, c.start).trim() !== '') return null;
      changes.add(`${name}(…) line taken out: a script's name is kept by the editor`);
      if (name === 'strategy') left.add('strategy(…)’s settings (capital, size, fees, pyramiding) are the strategy tester’s: set them there');
      return (arg(c, -1, 'overlay') ?? 'true').trim() === 'false' ? 'pane own' : '';
    });
  }

  // :=, +=, var and typed declarations.
  code = code.replace(/^(\s*)([A-Za-z_][\w.]*)\s*([+\-*/])=(?!=)\s*(.+)$/gm, (_, s: string, n: string, op: string, v: string) => {
    changes.add('x += y written as x = x + y');
    return `${s}${n} = ${n} ${op} (${v})`;
  });
  if (/:=/.test(code)) {
    code = code.replace(/:=/g, '=');
    changes.add(':= written as =');
  }
  code = code.replace(new RegExp(`^(\\s*)(?:(var|varip)\\s+)?(?:(?:series|simple|const)\\s+)?(?:${TYPES})\\s+([A-Za-z_]\\w*)\\s*=(?!=)`, 'gm'), (_, s: string, v: string | undefined, n: string) => {
    changes.add('types taken off declarations');
    return `${s}${v ? 'state ' : ''}${n} =`;
  });
  code = code.replace(/^(\s*)(?:var|varip)\s+([A-Za-z_]\w*)\s*=(?!=)/gm, (_, s: string, n: string) => {
    changes.add('var written as state');
    return `${s}state ${n} =`;
  });
  code = code.replace(/^(\s*)\[\s*([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\s*\]\s*=(?!=)/gm, (_, s: string, ns: string) => {
    changes.add('[a, b] = f() written as a, b = f()');
    return `${s}${ns} =`;
  });

  // Settings: name = input.int(14, "Length", minval = 1) and the like, a call that may run over several lines.
  const inputs = [...code.matchAll(/^([ \t]*)([A-Za-z_]\w*)\s*=\s*(input(?:\.\w+)?)\s*\(/gm)].reverse();
  for (const m of inputs) {
    const c = callAt(code, m.index!, m.index! + m[0].length - 1);
    if (!c) continue;
    const line = inputLine(m[1]!, m[2]!, m[3]!, c, text, left);
    if (line === null) continue;
    changes.add('input.int(…) and its kin written as input lines');
    code = code.slice(0, m.index!) + line + code.slice(c.end);
  }

  // Names under Pine's namespaces.
  code = rewriteCalls(code, 'color.new', (c) => `alpha(${c.args[0] ?? 'na'}, 1 - (${c.args[1] ?? '0'}) / 100)`);
  code = rewriteCalls(code, 'color.rgb', (c) => `rgb(${c.args.slice(0, 3).join(', ')}${c.args[3] ? `, 1 - (${c.args[3]}) / 100` : ''})`);
  code = rewriteCalls(code, 'color.from_gradient', (c) => `gradient(${c.args.join(', ')})`);
  if (/\bcolor\.\w/.test(code)) changes.add('color.new and colour names written as Frost Script’s');
  code = code.replace(/\bcolor\.(\w+)\b/g, '$1');
  code = rewriteCalls(code, 'str.tostring', (c) => {
    const x = c.args[0] ?? '0';
    const f = arg(c, 1, 'format');
    if (!f) return `str(${x})`;
    if (/format\.mintick/.test(f)) return `str(${x}, decimals)`;
    if (/format\.percent/.test(f)) return `str(round(${x}, 2)) + "%"`;
    const t = text(f);
    const places = t ? (t.split('.')[1] ?? '').replace(/[^#0]/g, '').length : 2;
    return places > 0 ? `str(round(${x}, ${places}))` : `str(round(${x}))`;
  });
  code = rewriteCalls(code, 'na', (c) => `isna(${c.args[0] ?? ''})`);
  code = rewriteCalls(code, 'math.round_to_mintick', (c) => `round((${c.args[0]}) / tick) * tick`);
  // Pine's casts: int() cuts toward zero, float() and bool() leave the number as it is.
  code = rewriteCalls(code, 'int', (c) => {
    changes.add('int(…) and float(…) casts written out');
    const x = c.args[0] ?? 'na';
    return `round((${x}) - 0.5 * sign(${x}))`;
  });
  for (const f of ['float', 'bool']) code = rewriteCalls(code, f, (c) => `(${c.args[0] ?? 'na'})`);
  for (const [name, fn] of Object.entries(TUPLES)) {
    if (new RegExp(`\\bta\\.${name}\\s*\\(`).test(code)) {
      code = code.replace(new RegExp(`\\bta\\.${name}\\s*\\(`, 'g'), `pine_${name}(`);
      code = `${fn}\n${code}`;
      changes.add(`ta.${name}(…) written as a function of the script’s`);
    }
  }
  if (/\b(?:ta|math)\.\w/.test(code)) changes.add('ta. and math. taken off the built-ins');
  code = code.replace(/\bta\.mom\b/g, 'change').replace(/\b(?:ta|math)\.(\w+)/g, '$1');
  // Pine's short forms: the source a high or low takes where none is given.
  code = rewriteCalls(code, 'pivothigh', (c) => (c.args.length === 2 ? `pivothigh(high, ${c.args.join(', ')})` : null));
  code = rewriteCalls(code, 'pivotlow', (c) => (c.args.length === 2 ? `pivotlow(low, ${c.args.join(', ')})` : null));
  for (const [f, s] of [['highest', 'high'], ['lowest', 'low'], ['highestbars', 'high'], ['lowestbars', 'low']] as const) {
    code = rewriteCalls(code, f, (c) => (c.args.length === 1 && c.named.size === 0 ? `${f}(${s}, ${c.args[0]})` : null));
  }
  for (const [re, to] of WORDS) {
    if (re.test(code)) changes.add('Pine’s names (bar_index, barstate, strategy.long, style names and the like) written as Frost Script’s');
    re.lastIndex = 0;
    code = code.replace(re, to);
  }

  // Lists: array.new…(…) and array.push(a, v).
  code = code.replace(/\barray\.(?:new(?:<[^>]*>|_\w+)?|from)\s*\(\s*\)/g, 'list()');
  code = rewriteCalls(code, 'array.new', () => 'list()');
  for (const t of ['float', 'int', 'bool', 'string', 'color', 'line', 'label', 'box']) code = rewriteCalls(code, `array.new_${t}`, () => 'list()');
  code = code.replace(/\barray\.new<[^>]*>\s*\([^)]*\)/g, 'list()');
  code = code.replace(/\barray\.(\w+)\s*\(\s*([A-Za-z_]\w*)\s*(,\s*)?/g, (_, op: string, a: string, comma: string | undefined) => {
    changes.add('array.push(a, v) written as a.push(v)');
    return `${a}.${op}(${comma ? '' : ''}`;
  });
  if (/\barray\.from\b/.test(code)) left.add('array.from(…): fill the list with push');

  // Drawing calls by Pine's names and arguments.
  code = rewriteCalls(code, 'bgcolor', (c) => {
    changes.add('bgcolor(…) written as background(…)');
    return `background(${arg(c, 0, 'color') ?? 'na'})`;
  });
  for (const f of ['plotshape', 'plotchar']) {
    code = rewriteCalls(code, f, (c) => {
      changes.add(`${f}(…) written as mark(…)`);
      const cond = arg(c, 0, 'series') ?? 'false';
      const label = text(arg(c, -1, 'text')) ?? text(arg(c, f === 'plotchar' ? 2 : -1, 'char')) ?? text(arg(c, 1, 'title')) ?? '""';
      const short = `"${label.slice(1, -1).slice(0, 12)}"`;
      const where = PLACE[(arg(c, 3, 'location') ?? '').trim()] ?? 'below';
      const colour = arg(c, f === 'plotchar' ? 4 : 4, 'color');
      return `mark(${cond}, ${short}, ${where === 'above' ? 'above' : 'below'}${colour ? `, ${colour}` : ''})`;
    });
  }
  code = rewriteCalls(code, 'hline', (c) => {
    const parts = [c.args[0] ?? '0', arg(c, 1, 'title') ?? '""'];
    const colour = arg(c, 2, 'color');
    const out = [`hline(${parts.join(', ')}`];
    if (colour) out.push(`, color = ${colour}`);
    const width = arg(c, 4, 'linewidth', 'width');
    if (width) out.push(`, width = ${width}`);
    const style = arg(c, 3, 'linestyle', 'style');
    if (style) out.push(`, style = ${style}`);
    return `${out.join('')})`;
  });
  code = rewriteCalls(code, 'plot', (c) => {
    if (!c.named.has('linewidth') && !c.named.has('display') && !c.named.has('style') && ![...c.named.keys()].some((k) => DROP.has(k))) return null;
    const style = c.named.get('style') ?? '';
    const histogram = /style_(?:histogram|columns)/.test(style);
    const out = [c.args[0] ?? arg(c, 0, 'series') ?? 'na', arg(c, 1, 'title') ?? null].filter((x): x is string => x !== null);
    const colour = /display\.none/.test(c.named.get('display') ?? '') ? 'na' : arg(c, 2, 'color');
    const named = [colour ? `color = ${colour}` : null, !histogram && c.named.get('linewidth') ? `width = ${c.named.get('linewidth')}` : null, !histogram && style && !/style_/.test(style) ? `style = ${style}` : null, c.named.get('offset') ? `offset = ${c.named.get('offset')}` : null].filter(Boolean);
    changes.add('plot(…)’s Pine arguments written as Frost Script’s');
    return `${histogram ? 'histogram' : 'plot'}(${[...out, ...named].join(', ')})`;
  });
  code = code.replace(/^\s*(?:alertcondition|alert)\s*\([\s\S]*?\)\s*$/gm, () => {
    changes.add('alertcondition(…) and alert(…) taken out: alerts are set on the chart');
    return '';
  });

  // A strategy's orders.
  code = rewriteCalls(code, 'strategy.entry', (c) => {
    changes.add('strategy.entry(…) written as long(…) or short(…)');
    const dir = (arg(c, 1, 'direction') ?? '1').trim();
    const when = arg(c, -1, 'when') ?? 'true';
    const limit = arg(c, -1, 'limit');
    const word = dir === '1' ? 'long' : dir === '-1' ? 'short' : null;
    if (!word) {
      left.add(`strategy.entry with a direction worked out (${dir}): written as long(…); check it`);
      return `long(${when}${limit ? `, ${limit}` : ''})`;
    }
    return `${word}(${when}${limit ? `, ${limit}` : ''})`;
  });
  code = rewriteCalls(code, 'strategy.order', (c) => {
    const dir = (arg(c, 1, 'direction') ?? '1').trim();
    return `${dir === '-1' ? 'short' : 'long'}(${arg(c, -1, 'when') ?? 'true'})`;
  });
  for (const f of ['strategy.close', 'strategy.close_all']) {
    code = rewriteCalls(code, f, (c) => {
      changes.add('strategy.close(…) written as exit(…)');
      return `exit(${arg(c, -1, 'when') ?? 'true'})`;
    });
  }
  code = rewriteCalls(code, 'strategy.exit', (c) => {
    changes.add('strategy.exit(…) written as stop(…) and target(…)');
    const out: string[] = [];
    const stop = c.named.get('stop');
    const limit = c.named.get('limit');
    const loss = c.named.get('loss');
    const profit = c.named.get('profit');
    if (stop) out.push(`stop(${stop})`);
    else if (loss) out.push(`stop(inlong ? entryprice - (${loss}) * tick : entryprice + (${loss}) * tick)`);
    if (limit) out.push(`target(${limit})`);
    else if (profit) out.push(`target(inlong ? entryprice + (${profit}) * tick : entryprice - (${profit}) * tick)`);
    if (c.named.has('trail_points') || c.named.has('trail_offset') || c.named.has('trail_price')) left.add('strategy.exit’s trailing stop: set the tester’s trailing stop instead');
    return out.join('\n') || 'stop(na)';
  });
  code = code.replace(/^\s*strategy\.cancel(?:_all)?\s*\([^)]*\)\s*$/gm, '');
  if (/\btimenow\b/.test(code)) {
    code = code.replace(/\btimenow\b/g, 'time');
    left.add('timenow: Frost Script has no clock, so each bar’s own time stands in for it: check what reads it');
  }
  for (const m of code.matchAll(/\bstrategy\.(\w+)/g)) left.add(`strategy.${m[1]}: not carried over`);
  for (const m of code.matchAll(/\b(request\.\w+|timeframe\.\w+|syminfo\.\w+|ticker\.\w+|polyline\.\w+|matrix\.\w+|map\.\w+|str\.(?!tostring)\w+)/g)) left.add(`${m[1]}: Frost Script reads this chart's own bars, with no word for it`);
  for (const m of code.matchAll(/\b(line|label|box|table|linefill)\.(new|cell|set_\w+|get_\w+|delete)\b/g)) left.add(`${m[1]}.${m[2]}: write it as Frost Script’s ${m[1] === 'table' ? 'table(…) and cell(…)' : `${m[1]}(…) and its set_ and get_ functions`}`);

  code = braces(code, changes);
  // Scientific numbers, which Frost Script writes out.
  code = code.replace(/(?<![\w.])(\d+(?:\.\d+)?)[eE]([+-]?\d+)\b/g, (_, a: string, e: string) => {
    changes.add('numbers like 1e-5 written out');
    return String(Number(`${a}e${e}`));
  });
  // Nothing of Pine's found: the script exactly as it was, spaces and all.
  if (code === before || changes.size === 0) return { text: src, changes: [], left: [...left] };
  return { text: unmask(code, texts).replace(/\n{3,}/g, '\n\n'), changes: [...changes], left: [...left] };
}

/** Pine's arguments with no Frost Script counterpart: dropped. */
const DROP = new Set(['trackprice', 'editable', 'show_last', 'precision', 'format', 'join', 'histbase', 'force_overlay', 'display']);

/** One of Pine's input calls as a Frost Script input line, or a plain value where Frost Script has no such setting. */
function inputLine(pad: string, name: string, kind: string, c: Call, text: (s: string | undefined) => string | null, left: Set<string>): string | null {
  const d = arg(c, 0, 'defval') ?? '0';
  const label = arg(c, 1, 'title');
  const title = label && text(label) ? ` ${text(label)}` : '';
  const lo = arg(c, -1, 'minval');
  const hi = arg(c, -1, 'maxval');
  const step = arg(c, -1, 'step');
  const options = arg(c, -1, 'options');
  if (['input.source', 'input.timeframe', 'input.symbol', 'input.session', 'input.text_area', 'input.price'].includes(kind)) {
    left.add(`${name}: ${kind} has no Frost Script setting; kept as its default (${d})`);
    return `${pad}${name} = ${d}`;
  }
  if (options) {
    const list = options.replace(/^\[|\]$/g, '').split(',').map((o) => o.trim()).filter(Boolean);
    if (list.length >= 2 && list.every((o) => text(o))) return `${pad}input ${name} = ${d} from ${list.join(', ')}${title}`;
  }
  if (kind === 'input.string') {
    left.add(`${name}: a text setting with no list of choices is kept as its default`);
    return `${pad}${name} = ${d}`;
  }
  if (kind === 'input.time') {
    // timestamp("2024-01-01 00:00 +0000") or timestamp(2024, 1, 1): a date.
    const inner = /timestamp\s*\(([^)]*)\)/.exec(d)?.[1] ?? '';
    const t = text(inner.trim());
    const ymd = t ? /(\d{4})-(\d{1,2})-(\d{1,2})(?:[ T](\d{1,2}):(\d{2}))?/.exec(t) : null;
    const date = ymd ? `date(${[ymd[1], Number(ymd[2]), Number(ymd[3]), ...(ymd[4] ? [Number(ymd[4]), Number(ymd[5])] : [])].join(', ')})` : inner && !t ? `date(${inner})` : d;
    return `${pad}input ${name} = ${date}${title}`;
  }
  const bounds = `${lo ? ` min ${lo}` : ''}${hi ? ` max ${hi}` : ''}${step ? ` step ${step}` : ''}`;
  return `${pad}input ${name} = ${d}${kind === 'input.bool' || kind === 'input.color' ? '' : bounds}${title}`;
}
