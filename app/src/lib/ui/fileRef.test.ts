import { describe, expect, it } from "vitest";
import { findRefs, parseRef } from "./fileRef";

const refs = (t: string, loose = false) => findRefs(t, loose).map(({ path, line, col }) => ({ path, line, col }));

describe("file references", () => {
  it("finds paths with lines in prose", () => {
    expect(refs("Deleting runs (crates/muster-core/src/store.rs:932), so it goes.")).toEqual([
      { path: "crates/muster-core/src/store.rs", line: 932, col: undefined },
    ]);
    expect(refs("see ./app/main.ts:12:4 and /Users/me/x.py (line 3)")).toEqual([
      { path: "./app/main.ts", line: 12, col: 4 },
      { path: "/Users/me/x.py", line: 3, col: undefined },
    ]);
    expect(refs("docs/AGENTS.md#L40-L52 and ~/notes/todo.txt")).toEqual([
      { path: "docs/AGENTS.md", line: 40, col: undefined },
      { path: "~/notes/todo.txt", line: undefined, col: undefined },
    ]);
  });

  it("keeps a bare name only when loose, a line is given, or the ending is known", () => {
    expect(refs("edited store.rs today")).toEqual([]);
    expect(refs("edited store.rs today", true)).toEqual([{ path: "store.rs", line: undefined, col: undefined }]);
    expect(refs("store.rs:10")).toEqual([{ path: "store.rs", line: 10, col: undefined }]);
    expect(refs("Makefile", true)).toEqual([{ path: "Makefile", line: undefined, col: undefined }]);
    expect(refs("see Guardfile:4 and Vagrantfile")).toEqual([{ path: "Guardfile", line: 4, col: undefined }]);
    expect(refs("Vagrantfile", true)).toEqual([{ path: "Vagrantfile", line: undefined, col: undefined }]);
    expect(refs("bin/rails:12 then bin/setup")).toEqual([{ path: "bin/rails", line: 12, col: undefined }]);
    expect(refs("localhost:3000, step:2, Note: profile and file", true)).toEqual([]);
  });

  it("leaves versions, URLs, domains and sentences alone", () => {
    expect(refs("version 1.2.3, e.g. this, v2.0", true)).toEqual([]);
    expect(refs("https://github.com/coderhs/muster/blob/main/a.rs", true)).toEqual([]);
    expect(refs("github.com/coderhs/x.rs", true)).toEqual([]);
    expect(refs("example.com is a site")).toEqual([]);
    expect(refs("ends here.")).toEqual([]);
  });

  it("reads inline code as one reference", () => {
    expect(parseRef("crates/musterd/src/files.rs:12")).toEqual({ path: "crates/musterd/src/files.rs", line: 12, col: undefined });
    expect(parseRef("CLAUDE.md")).toEqual({ path: "CLAUDE.md", line: undefined, col: undefined });
    expect(parseRef("cargo test -p musterd")).toBeNull();
    expect(parseRef("foo()")).toBeNull();
  });

  it("gives the matched span", () => {
    const [r] = findRefs("at src/a.rs:3 ok");
    expect([r.start, r.end]).toEqual([3, 13]);
  });
});
