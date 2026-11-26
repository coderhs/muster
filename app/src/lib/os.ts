// What differs between the desktop systems in the page: the shortcut modifier, how
// shortcuts are written, and what the computer is called.

export const isMac = () => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

type ChordEvent = Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "target">;

/**
 * Whether the app's shortcut modifier is held: Cmd on a Mac, Ctrl elsewhere. Elsewhere
 * a Ctrl chord typed into a terminal belongs to the program in it, not to the app.
 */
export function appMod(e: ChordEvent, mac = isMac()): boolean {
  if (mac) return e.metaKey && !e.ctrlKey;
  return e.ctrlKey && !e.metaKey && !(e.target as HTMLElement | null)?.closest?.(".xterm");
}

/** A shortcut written with Mac symbols (`⇧⌘↵`), as this system writes it. */
export function keyText(keys: string, mac = isMac()): string {
  if (mac) return keys;
  return keys
    .replaceAll("⌘-", "Ctrl-")
    .replaceAll("⇧⌘", "Ctrl+Shift+")
    .replaceAll("⌘", "Ctrl+")
    .replaceAll("⌃", "Ctrl+")
    .replaceAll("⇧", "Shift+")
    .replaceAll("⌥", "Alt+");
}

/** "this Mac" or "this computer". */
export const computer = () => (isMac() ? "Mac" : "computer");
