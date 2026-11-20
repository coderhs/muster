import { describe, expect, it } from "vitest";
import { isOpenChord, pathsForPaste, webUrl } from "./links";

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

describe("pathsForPaste", () => {
  it("escapes spaces and shell characters, and joins with spaces", () => {
    expect(pathsForPaste(["/Users/me/Desktop/Screenshot 2026-11-14 at 6.53.33 PM.png"])).toBe(
      "/Users/me/Desktop/Screenshot\\ 2026-11-14\\ at\\ 6.53.33\\ PM.png",
    );
    expect(pathsForPaste(["/a/b.txt", "/c/it's (1).md"])).toBe("/a/b.txt /c/it\\'s\\ \\(1\\).md");
  });
  it("keeps letters of any script", () => {
    expect(pathsForPaste(["/tmp/മെനു.pdf"])).toBe("/tmp/മെനു.pdf");
  });
});

describe("file references across wrapped rows", () => {
  it("maps a reference that wraps onto the next row to cells", async () => {
    const { refsInRows } = await import("./links");
    const { findRefs } = await import("../ui/fileRef");
    // 10 columns: "see src/ab" + "c.rs:12 ok"
    const out = refsInRows(["see src/ab", "c.rs:12 ok"], 10, 4, (t) => findRefs(t, true));
    expect(out.map((o) => [o.ref.path, o.ref.line])).toEqual([["src/abc.rs", 12]]);
    expect(out[0].range).toEqual({ start: { x: 5, y: 5 }, end: { x: 7, y: 6 } });
  });
});
