// Links in the terminal open only on Cmd+click (Ctrl+click off macOS), and only web links.

/** The link as a normalized http(s) URL, or null when it must not be opened. */
export function webUrl(raw: string): string | null {
  try {
    const u = new URL(raw.trim());
    return u.protocol === "http:" || u.protocol === "https:" ? u.href : null;
  } catch {
    return null;
  }
}

export function isOpenChord(ev: Pick<MouseEvent, "metaKey" | "ctrlKey">, mac: boolean): boolean {
  return mac ? ev.metaKey : ev.ctrlKey;
}

/**
 * File paths as one line for a shell or an agent prompt: separated by spaces, with a
 * backslash before every character that would otherwise split or change a path.
 */
export function pathsForPaste(paths: string[]): string {
  // Only printable ASCII can be special to a shell; letters of other scripts pass.
  return paths.map((p) => p.replace(/[ -~]/g, (c) => (/[A-Za-z0-9_\-./:@%+,=]/.test(c) ? c : `\\${c}`))).join(" ");
}

export interface CellRange {
  /** 1-based columns and buffer lines, end inclusive, as xterm's link API wants them. */
  start: { x: number; y: number };
  end: { x: number; y: number };
}

/** One terminal row: its text, the 0-based cell of each character, and whether the
 * terminal wrapped it on from the row before. */
export interface Row {
  text: string;
  cells: number[];
  wrapped: boolean;
}

const PATH_END = /[\w/.\-~@+]$/;
const CONTINUES = /^( {0,8})[\w/.\-~@+]/;

/**
 * Whether `next` carries on a path that `prev` breaks off: agents (Claude Code among
 * them) wrap long lines themselves with a real line break and an indent, so the
 * terminal does not mark the row as wrapped. `prev` must end in a path running up to
 * the right edge, and `next` start (after a small indent) with more of it. Returns how
 * many characters of indent to skip, or null.
 */
export function continuesPath(prev: Row, next: Row, cols: number): number | null {
  const t = prev.text.trimEnd();
  if (!t || !PATH_END.test(t) || prev.cells[t.length - 1] < cols - 4) return null;
  const tail = t.slice(t.lastIndexOf(" ") + 1);
  if (!tail.includes("/")) return null;
  const m = CONTINUES.exec(next.text);
  return m ? m[1].length : null;
}

/**
 * File references in the logical line holding row `at` (a long line wraps onto several
 * rows, by the terminal or by the agent), with where each lies in cells. `rows` are
 * consecutive buffer rows, the first of them at 0-based buffer line `firstRow`.
 */
export function refsAround<T extends { start: number; end: number }>(
  rows: Row[],
  at: number,
  cols: number,
  firstRow: number,
  find: (text: string) => T[],
): { ref: T; range: CellRange }[] {
  const indent = rows.map((r, i) => (i === 0 ? null : r.wrapped ? 0 : continuesPath(rows[i - 1], r, cols)));
  let first = at;
  while (first > 0 && indent[first] !== null) first--;
  let last = at;
  while (last + 1 < rows.length && indent[last + 1] !== null) last++;
  let text = "";
  const where: { x: number; y: number }[] = [];
  for (let i = first; i <= last; i++) {
    const r = rows[i];
    const from = i === first ? 0 : (indent[i] ?? 0);
    const to = i < last && rows[i + 1].wrapped ? r.text.length : r.text.trimEnd().length;
    for (let k = from; k < to; k++) where.push({ x: r.cells[k] + 1, y: firstRow + i + 1 });
    text += r.text.slice(from, to);
  }
  return find(text).map((ref) => ({ ref, range: { start: where[ref.start], end: where[ref.end - 1] } }));
}
