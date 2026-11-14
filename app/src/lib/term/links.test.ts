import { describe, expect, it } from "vitest";
import { isOpenChord, webUrl } from "./links";

describe("webUrl", () => {
  it("accepts http and https", () => {
    expect(webUrl("https://example.com/a?b=1#c")).toBe("https://example.com/a?b=1#c");
    expect(webUrl(" http://localhost:3000 ")).toBe("http://localhost:3000/");
  });
  it("refuses every other scheme and non-URLs", () => {
    for (const s of ["file:///etc/passwd", "javascript:alert(1)", "ssh://host", "mailto:a@b.c", "not a url", ""]) {
      expect(webUrl(s)).toBeNull();
    }
  });
});

describe("isOpenChord", () => {
  it("needs Cmd on macOS and Ctrl elsewhere", () => {
    expect(isOpenChord({ metaKey: true, ctrlKey: false }, true)).toBe(true);
    expect(isOpenChord({ metaKey: false, ctrlKey: true }, true)).toBe(false);
    expect(isOpenChord({ metaKey: false, ctrlKey: true }, false)).toBe(true);
    expect(isOpenChord({ metaKey: false, ctrlKey: false }, false)).toBe(false);
  });
});
