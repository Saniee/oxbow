// A small evaluator for VS Code's `when` clause syntax, for the buttons the SlothGit tab draws itself (row and panel
// actions). VS Code evaluates the native right-click menus; this covers the subset package.json uses:
// `a && b`, `a || b`, `!a`, `a == b`, `a != b`, `a =~ /re/flags`, bare keys, and quoted or bare values.
// No parentheses: VS Code's own syntax has none either.

export type ContextValues = Record<string, unknown>;

/** True when `when` holds for `values`. An empty or missing clause always holds. */
export function evaluate(when: string | undefined, values: ContextValues): boolean {
  if (!when?.trim()) return true;
  return splitOutside(when, '||').some((alternative) => splitOutside(alternative, '&&').every((term) => test(term.trim(), values)));
}

function test(term: string, values: ContextValues): boolean {
  if (term.startsWith('!') && !term.startsWith('!=')) return !test(term.slice(1).trim(), values);
  const regex = /^([\w.-]+)\s*=~\s*\/(.*)\/([a-z]*)$/.exec(term);
  if (regex) {
    const value = values[regex[1]];
    return typeof value === 'string' && new RegExp(regex[2], regex[3]).test(value);
  }
  const compare = /^([\w.-]+)\s*(==|!=)\s*(.+)$/.exec(term);
  if (compare) {
    const equal = String(values[compare[1]] ?? '') === unquote(compare[3].trim());
    return compare[2] === '==' ? equal : !equal;
  }
  if (term === 'true') return true;
  if (term === 'false') return false;
  return truthy(values[term]);
}

function truthy(value: unknown): boolean {
  return !!value && !(Array.isArray(value) && value.length === 0);
}

function unquote(value: string): string {
  return /^'.*'$|^".*"$/.test(value) ? value.slice(1, -1) : value;
}

/** Splits on `separator`, except inside a /regex/ (whose `|` would otherwise look like `||`). */
function splitOutside(text: string, separator: string): string[] {
  const parts: string[] = [];
  let start = 0;
  let inRegex = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '\\' && inRegex) {
      i++;
      continue;
    }
    // A regex starts with the / right after =~ (and optional spaces) and ends at the next unescaped /.
    if (c === '/') {
      if (inRegex) inRegex = false;
      else if (/=~\s*$/.test(text.slice(start, i))) inRegex = true;
      continue;
    }
    if (!inRegex && text.startsWith(separator, i)) {
      parts.push(text.slice(start, i));
      start = i + separator.length;
      i += separator.length - 1;
    }
  }
  parts.push(text.slice(start));
  return parts;
}
