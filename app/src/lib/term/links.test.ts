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
  const row = (text: string, cols: number, wrapped = false) => ({
    text: text.padEnd(cols),
    cells: [...text.padEnd(cols)].map((_, i) => i),
    wrapped,
  });

  it("maps a reference the terminal wrapped onto the next row to cells", async () => {
    const { refsAround } = await import("./links");
    const { findRefs } = await import("../ui/fileRef");
    // 10 columns: "see src/ab" + "c.rs:12 ok"
    const rows = [row("see src/ab", 10), row("c.rs:12 ok", 10, true)];
    const out = refsAround(rows, 1, 10, 4, (t) => findRefs(t, true));
    expect(out.map((o) => [o.ref.path, o.ref.line])).toEqual([["src/abc.rs", 12]]);
    expect(out[0].range).toEqual({ start: { x: 5, y: 5 }, end: { x: 7, y: 6 } });
  });

  it("joins a path an agent broke onto an indented next line", async () => {
    const { refsAround } = await import("./links");
    const { findRefs } = await import("../ui/fileRef");
    const rows = [
      row("⏺ Done.", 40),
      row("  Console script: /private/tmp/c04b/scra", 40),
      row("  tchpad/resend_failed_emails.rb", 40),
      row("  Next line of text", 40),
    ];
    for (const at of [1, 2]) {
      const out = refsAround(rows, at, 40, 0, (t) => findRefs(t, true));
      expect(out.map((o) => o.ref.path)).toEqual(["/private/tmp/c04b/scratchpad/resend_failed_emails.rb"]);
      expect(out[0].range).toEqual({ start: { x: 19, y: 2 }, end: { x: 32, y: 3 } });
    }
  });

  it("joins the real Claude Code output at 109 columns", async () => {
    const { refsAround } = await import("./links");
    const { findRefs } = await import("../ui/fileRef");
    const rows = [
      row("   Console script: /private/tmp/claude-501/-Users-user-work-demo-acme-store-front/c04b68ef-3453-49b8-9e0d-e7a", 109),
      row("   5c6cf8b8f/scratchpad/resend_failed_emails.rb", 109),
    ];
    const out = refsAround(rows, 1, 109, 0, (t) => findRefs(t, true));
    expect(out.map((o) => o.ref.path)).toEqual([
      "/private/tmp/claude-501/-Users-user-work-demo-acme-store-front/c04b68ef-3453-49b8-9e0d-e7a5c6cf8b8f/scratchpad/resend_failed_emails.rb",
    ]);
  });

  it("leaves short lines and lines not ending in a path apart", async () => {
    const { refsAround } = await import("./links");
    const { findRefs } = await import("../ui/fileRef");
    const find = (t: string) => findRefs(t, true);
    const short = [row("  Edited src/a.rs", 40), row("  Updated the tests", 40)];
    expect(refsAround(short, 0, 40, 0, find).map((o) => o.ref.path)).toEqual(["src/a.rs"]);
    const words = [row("  this sentence runs right up to the edge", 40), row("  src/b.rs:3", 40)];
    expect(refsAround(words, 1, 40, 0, find).map((o) => o.ref.path)).toEqual(["src/b.rs"]);
  });
});
