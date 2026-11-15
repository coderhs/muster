// The Docs tab's list of documents as folders within folders, and the names of new files.

import type { DocEntry, DocKind } from "../protocol/types";

export type TreeRow =
  /** `path` ends with `/`; `count` is every file below the folder. */
  | { t: "folder"; name: string; path: string; depth: number; count: number; open: boolean }
  | { t: "file"; name: string; doc: DocEntry; depth: number };

interface Folder {
  folders: Map<string, Folder>;
  files: DocEntry[];
  count: number;
}

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true }) || (a < b ? -1 : a > b ? 1 : 0);
const base = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const isReadme = (name: string) => /^readme\./i.test(name);

/**
 * The rows to show, top to bottom: in each folder its folders first, then its files,
 * each by name, with the README leading the top level. The rows inside a folder named
 * in `closed` (by its path) are left out.
 */
export function treeRows(docs: DocEntry[], closed: ReadonlySet<string> = new Set()): TreeRow[] {
  const root: Folder = { folders: new Map(), files: [], count: 0 };
  for (const doc of docs) {
    const parts = doc.path.split("/");
    let at = root;
    for (const part of parts.slice(0, -1)) {
      let next = at.folders.get(part);
      if (!next) at.folders.set(part, (next = { folders: new Map(), files: [], count: 0 }));
      next.count++;
      at = next;
    }
    at.files.push(doc);
  }
  const out: TreeRow[] = [];
  const walk = (f: Folder, prefix: string, depth: number) => {
    for (const name of [...f.folders.keys()].sort(byName)) {
      const path = `${prefix}${name}/`;
      const child = f.folders.get(name)!;
      const open = !closed.has(path);
      out.push({ t: "folder", name, path, depth, count: child.count, open });
      if (open) walk(child, path, depth + 1);
    }
    const files = [...f.files].sort((a, b) => {
      const [x, y] = [base(a.path), base(b.path)];
      if (!depth && isReadme(x) !== isReadme(y)) return isReadme(x) ? -1 : 1;
      return byName(x, y);
    });
    for (const doc of files) out.push({ t: "file", name: base(doc.path), doc, depth });
  };
  walk(root, "", 0);
  return out;
}

/**
 * The path for a new document from what was typed, inside `folder` (a path ending with
 * `/`, or empty for the top of the project). `.md` is added when no extension is given.
 * Returns the reason when the name cannot be used.
 */
export function newDocPath(typed: string, folder = ""): { path: string } | { error: string } {
  let name = typed.trim().replace(/\\/g, "/").replace(/^(\.?\/)+/, "");
  if (!name || name.endsWith("/")) return { error: "Give the file a name" };
  if (!/\.(md|markdown)$/i.test(name)) name += ".md";
  const path = `${folder}${name}`;
  const parts = path.split("/");
  if (parts.some((p) => !p || p === "." || p === ".." || p !== p.trim())) return { error: "That is not a path inside the project" };
  if (/^\.(md|markdown)$/i.test(parts[parts.length - 1])) return { error: "Give the file a name" };
  return { path };
}

/** What a new file starts with. */
export function starter(path: string, kind: DocKind, project: string, hasAgents: boolean): string {
  if (kind === "claude" && hasAgents) return "@AGENTS.md\n";
  if (kind === "doc") {
    const title = base(path).replace(/\.(md|markdown)$/i, "").replace(/[-_]+/g, " ").trim();
    return `# ${title}\n\n`;
  }
  return `# ${project}\n\n## Commands\n\n- Build: \n- Test: \n- Lint: \n\n## Conventions\n\n- \n\n## Do not\n\n- \n`;
}
