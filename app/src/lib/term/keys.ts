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
