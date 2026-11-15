import { describe, expect, it } from "vitest";
import type { DocEntry } from "../protocol/types";
import { newDocPath, starter, treeRows } from "./docsTree";

const docs = (...paths: string[]): DocEntry[] => paths.map((path) => ({ path, kind: "doc", size: 1, modifiedAt: 1 }));
const show = (rows: ReturnType<typeof treeRows>) =>
  rows.map((r) => `${"  ".repeat(r.depth)}${r.t === "folder" ? `${r.name}/ ${r.count}${r.open ? "" : " closed"}` : r.name}`);

describe("treeRows", () => {
  const all = docs("docs/wiki/Home.md", "CHANGELOG.md", "docs/PLAN.md", "README.md", "docs/wiki/Agents.md", "docs/api/v2/Auth.md", "docs/PROTOCOL.md");

  it("nests folders, folders before files, the README first", () => {
    expect(show(treeRows(all))).toEqual([
      "docs/ 5",
      "  api/ 1",
      "    v2/ 1",
      "      Auth.md",
      "  wiki/ 2",
      "    Agents.md",
      "    Home.md",
      "  PLAN.md",
      "  PROTOCOL.md",
      "README.md",
      "CHANGELOG.md",
    ]);
  });

  it("leaves out what is inside a closed folder", () => {
    expect(show(treeRows(all, new Set(["docs/wiki/", "docs/api/"])))).toEqual([
      "docs/ 5",
      "  api/ 1 closed",
      "  wiki/ 2 closed",
      "  PLAN.md",
      "  PROTOCOL.md",
      "README.md",
      "CHANGELOG.md",
    ]);
    expect(show(treeRows(all, new Set(["docs/"])))).toEqual(["docs/ 5 closed", "README.md", "CHANGELOG.md"]);
  });

  it("gives folders their path and files their entry", () => {
    const rows = treeRows(docs("docs/wiki/Home.md"));
    expect(rows[1]).toMatchObject({ t: "folder", path: "docs/wiki/", depth: 1 });
    expect(rows[2]).toMatchObject({ t: "file", depth: 2, doc: { path: "docs/wiki/Home.md" } });
    expect(treeRows([])).toEqual([]);
  });

  it("orders names as people read them", () => {
    expect(show(treeRows(docs("b.md", "A.md", "note10.md", "note2.md")))).toEqual(["A.md", "b.md", "note2.md", "note10.md"]);
  });
});

describe("newDocPath", () => {
  it("adds the extension and the folder", () => {
    expect(newDocPath("Setup", "docs/")).toEqual({ path: "docs/Setup.md" });
    expect(newDocPath("  guides/Deploy.md ", "docs/")).toEqual({ path: "docs/guides/Deploy.md" });
    expect(newDocPath("./NOTES.markdown")).toEqual({ path: "NOTES.markdown" });
    expect(newDocPath("/docs/a")).toEqual({ path: "docs/a.md" });
    expect(newDocPath("v1.2 notes")).toEqual({ path: "v1.2 notes.md" });
  });

  it("refuses names that are not a file in the project", () => {
    for (const bad of ["", "   ", "docs/", "../x", "a/../b", "a//b", ".md", "docs/.md"]) {
      expect(newDocPath(bad), bad).toHaveProperty("error");
    }
  });
});

describe("starter", () => {
  it("starts each kind of file usefully", () => {
    expect(starter("docs/release-notes.md", "doc", "muster", false)).toBe("# release notes\n\n");
    expect(starter("CLAUDE.md", "claude", "muster", true)).toBe("@AGENTS.md\n");
    expect(starter("CLAUDE.md", "claude", "muster", false)).toContain("# muster\n\n## Commands");
    expect(starter("AGENTS.md", "agents", "muster", true)).toContain("## Conventions");
  });
});
