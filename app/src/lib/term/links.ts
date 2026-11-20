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

/**
 * File references in one logical line of the terminal, given as its rows (a long line
 * wraps onto several), with where each lies in cells. `firstRow` is the 0-based buffer
 * line of the first row. Rows with wide characters do not map one character to one
 * cell, so then only the row itself is searched.
 */
export function refsInRows<T extends { start: number; end: number }>(
  rows: string[],
  cols: number,
  firstRow: number,
  find: (text: string) => T[],
): { ref: T; range: CellRange }[] {
  const exact = rows.slice(0, -1).every((r) => r.length === cols);
  if (!exact && rows.length > 1) return [];
  const text = rows.join("");
  const at = (i: number) => ({ x: (i % cols) + 1, y: firstRow + Math.floor(i / cols) + 1 });
  return find(text).map((ref) => ({ ref, range: { start: at(ref.start), end: at(ref.end - 1) } }));
}
