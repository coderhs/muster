import { describe, expect, it } from "vitest";
import { isNewlineChord } from "./keys";

const ev = (over: Partial<Parameters<typeof isNewlineChord>[0]>) => ({
  key: "Enter",
  shiftKey: false,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  ...over,
});

describe("isNewlineChord", () => {
  it("matches Shift+Enter only", () => {
    expect(isNewlineChord(ev({ shiftKey: true }))).toBe(true);
    expect(isNewlineChord(ev({}))).toBe(false);
    expect(isNewlineChord(ev({ shiftKey: true, key: "a" }))).toBe(false);
  });

  it("leaves chords with other modifiers alone", () => {
    for (const mod of ["metaKey", "ctrlKey", "altKey"] as const) {
      expect(isNewlineChord(ev({ shiftKey: true, [mod]: true }))).toBe(false);
    }
  });
});
