import { describe, expect, it } from "vitest";
import { formatReview, lineRef, parsePatch } from "./patch";

const PATCH = `diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,3 +1,4 @@ fn main
 one
-two
+TWO
 three
+four
\\ No newline at end of file
@@ -10,2 +11,2 @@
 ten
-eleven
+ELEVEN
`;

describe("parsePatch", () => {
  it("numbers old and new lines", () => {
    const p = parsePatch(PATCH);
    expect(p.hunks).toHaveLength(2);
    expect(p.hunks[0].header).toBe("fn main");
    expect(p.hunks[0].lines).toEqual([
      { kind: "ctx", old: 1, new: 1, text: "one" },
      { kind: "del", old: 2, text: "two" },
      { kind: "add", new: 2, text: "TWO" },
      { kind: "ctx", old: 3, new: 3, text: "three" },
      { kind: "add", new: 4, text: "four" },
    ]);
    expect(p.hunks[1].lines[2]).toEqual({ kind: "add", new: 12, text: "ELEVEN" });
    expect(lineRef("a.txt", p.hunks[0].lines[1])).toEqual({ path: "a.txt", side: "old", line: 2 });
  });

  it("keeps renames and spots binary files", () => {
    const p = parsePatch("diff --git a/x b/y\nsimilarity index 90%\nrename from x\nrename to y\n");
    expect(p.notes).toEqual(["similarity index 90%", "rename from x", "rename to y"]);
    expect(parsePatch("diff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\n").binary).toBe(true);
  });
});

describe("formatReview", () => {
  it("orders comments by file and line and quotes the line", () => {
    const text = formatReview(
      [
        { path: "b.rs", side: "new", line: 3, quote: "let x = 1;", text: "rename x" },
        { path: "a.rs", side: "old", line: 7, quote: "  removed()  ", text: "why was this\nremoved?" },
      ],
      "Looks close.",
    );
    expect(text).toBe(
      "Review of your changes. Please address each point, then summarise what you changed.\n\n" +
        "Looks close.\n\n" +
        "1. a.rs, removed line 7 of the old version\n   > removed()\n   why was this\n   removed?\n\n" +
        "2. b.rs:3\n   > let x = 1;\n   rename x",
    );
  });
});
