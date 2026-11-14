import { describe, expect, it } from "vitest";
import { CTRL_ENTER, isCtrlEnter, isNewlineChord } from "./keys";

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

describe("isCtrlEnter", () => {
  it("matches Ctrl+Enter only", () => {
    expect(isCtrlEnter(ev({ ctrlKey: true }))).toBe(true);
    expect(isCtrlEnter(ev({}))).toBe(false);
    expect(isCtrlEnter(ev({ ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(isCtrlEnter(ev({ ctrlKey: true, key: "a" }))).toBe(false);
  });
  it("is sent as key 13 with the Ctrl modifier", () => {
    expect(CTRL_ENTER).toBe("\x1b[13;5u");
  });
});
