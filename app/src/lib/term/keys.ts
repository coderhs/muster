// Key chords the terminal handles itself instead of passing to xterm.

type Mods = Pick<KeyboardEvent, "key" | "shiftKey" | "metaKey" | "ctrlKey" | "altKey">;

/**
 * Shift+Enter with no other modifier. It must be matched in every phase of the key
 * (keydown, keypress, keyup): refusing only the keydown lets xterm turn the keypress
 * into a plain Enter, which submits the prompt right after the newline.
 */
export function isNewlineChord(ev: Mods): boolean {
  return ev.key === "Enter" && ev.shiftKey && !ev.metaKey && !ev.ctrlKey && !ev.altKey;
}

/**
 * Ctrl+Enter with no other modifier. A terminal sends the same byte for it as for a
 * plain Enter, so a program that gives it its own meaning (Claude Code: send the typed
 * message now instead of queueing it) cannot tell them apart unless it is sent in the
 * extended form below. Matched in every key phase, like the newline chord.
 */
export function isCtrlEnter(ev: Mods): boolean {
  return ev.key === "Enter" && ev.ctrlKey && !ev.shiftKey && !ev.metaKey && !ev.altKey;
}

/** Ctrl+Enter in the "CSI u" key encoding: key 13 (Enter), modifier 5 (Ctrl). */
export const CTRL_ENTER = "\x1b[13;5u";
