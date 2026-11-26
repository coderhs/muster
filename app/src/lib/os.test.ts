import { describe, expect, it } from "vitest";
import { appMod, keyText } from "./os";

const press = (metaKey: boolean, ctrlKey: boolean, inTerminal = false) => ({
  metaKey,
  ctrlKey,
  target: { closest: () => (inTerminal ? {} : null) } as unknown as EventTarget,
});

describe("appMod", () => {
  it("is Cmd on a Mac, also in a terminal", () => {
    expect(appMod(press(true, false), true)).toBe(true);
    expect(appMod(press(true, false, true), true)).toBe(true);
    expect(appMod(press(false, true), true)).toBe(false);
  });

  it("is Ctrl elsewhere, except in a terminal", () => {
    expect(appMod(press(false, true), false)).toBe(true);
    expect(appMod(press(true, false), false)).toBe(false);
    expect(appMod(press(false, true, true), false)).toBe(false);
  });
});

describe("keyText", () => {
  it("keeps Mac symbols on a Mac and spells them out elsewhere", () => {
    expect(keyText("⇧⌘↵", true)).toBe("⇧⌘↵");
    expect(keyText("⇧⌘↵", false)).toBe("Ctrl+Shift+↵");
    expect(keyText("⌘F  or  ⌘K", false)).toBe("Ctrl+F  or  Ctrl+K");
    expect(keyText("⌘-click a path", false)).toBe("Ctrl-click a path");
    expect(keyText("⌃⇥", false)).toBe("Ctrl+⇥");
  });
});
