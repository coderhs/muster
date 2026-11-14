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
