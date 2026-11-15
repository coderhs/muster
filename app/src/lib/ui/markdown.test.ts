import { describe, expect, it } from "vitest";
import { inline, parse, plain, slug, target } from "./markdown";

const text = (s: string) => ({ t: "text", s });

describe("blocks", () => {
  it("reads headings, paragraphs and rules", () => {
    expect(parse("# Title\n\nOne\ntwo.\n\n---\n\nSetext\n======\n")).toEqual([
      { t: "heading", level: 1, id: "title", c: [text("Title")] },
      { t: "p", c: [text("One two.")] },
      { t: "hr" },
      { t: "heading", level: 1, id: "setext", c: [text("Setext")] },
    ]);
  });

  it("gives repeated headings their own anchors", () => {
    const ids = parse("## Methods\n## Methods\n## Card & board fields").map((b) => (b.t === "heading" ? b.id : ""));
    expect(ids).toEqual(["methods", "methods-1", "card--board-fields"]);
    expect(slug("Phase 4 and 5: resources")).toBe("phase-4-and-5-resources");
  });

  it("keeps fenced code as written", () => {
    expect(parse("```sh\nnpm test # not a heading\n\n  indented\n```\nafter")).toEqual([
      { t: "code", lang: "sh", s: "npm test # not a heading\n\n  indented" },
      { t: "p", c: [text("after")] },
    ]);
    // A fence that is never closed runs to the end.
    expect(parse("~~~\na")).toEqual([{ t: "code", lang: "", s: "a" }]);
  });

  it("reads lists with nesting, tasks and continued lines", () => {
    const [list] = parse("- one\n  still one\n- [x] done\n  - inner\n- [ ] open\n\n1. first");
    expect(list).toEqual({
      t: "list",
      ordered: false,
      start: 1,
      items: [
        { c: [{ t: "p", c: [text("one still one")] }] },
        {
          checked: true,
          c: [
            { t: "p", c: [text("done")] },
            { t: "list", ordered: false, start: 1, items: [{ c: [{ t: "p", c: [text("inner")] }] }] },
          ],
        },
        { checked: false, c: [{ t: "p", c: [text("open")] }] },
      ],
    });
    const ordered = parse("3. c\n4. d")[0];
    expect(ordered).toMatchObject({ t: "list", ordered: true, start: 3 });
    expect(ordered.t === "list" && ordered.items.length).toBe(2);
  });

  it("reads a paragraph and code inside a list item", () => {
    const [list] = parse("1. Run:\n\n   ```\n   make\n   ```\n\n   Then wait.\n2. Next");
    expect(list.t === "list" && list.items[0].c.map((b) => b.t)).toEqual(["p", "code", "p"]);
    expect(list.t === "list" && list.items[0].c[1]).toEqual({ t: "code", lang: "", s: "make" });
  });

  it("reads quotes", () => {
    expect(parse("> **Note**\n> text\n\nafter")).toEqual([
      { t: "quote", c: [{ t: "p", c: [{ t: "strong", c: [text("Note")] }, text(" text")] }] },
      { t: "p", c: [text("after")] },
    ]);
  });

  it("reads tables, with pipes inside code", () => {
    const [table] = parse("| Method | Result |\n|---|:-:|\n| `a\\|b` | `{x}` |\n| only |\n\nafter");
    expect(table).toEqual({
      t: "table",
      align: ["", "center"],
      head: [[text("Method")], [text("Result")]],
      rows: [
        [[{ t: "code", s: "a|b" }], [{ t: "code", s: "{x}" }]],
        [[text("only")], []],
      ],
    });
    // A line with a pipe and no rule under it is a paragraph.
    expect(parse("a | b")[0].t).toBe("p");
  });

  it("shows front matter as it is written and drops comments", () => {
    expect(parse("---\nname: x\n---\n<!-- hidden\nstill -->\nBody")).toEqual([
      { t: "code", lang: "yaml", s: "name: x" },
      { t: "p", c: [text("Body")] },
    ]);
  });
});

describe("inline", () => {
  it("reads emphasis, code and strikethrough", () => {
    expect(inline("a **b *c*** `d*e` ~~f~~")).toEqual([
      text("a "),
      { t: "strong", c: [text("b "), { t: "em", c: [text("c")] }] },
      text(" "),
      { t: "code", s: "d*e" },
      text(" "),
      { t: "del", c: [text("f")] },
    ]);
  });

  it("leaves names with underscores and lone marks alone", () => {
    expect(plain(inline("snake_case_name and 2 * 3 * 4 and a_b"))).toBe("snake_case_name and 2 * 3 * 4 and a_b");
    expect(inline("snake_case_name")).toEqual([text("snake_case_name")]);
    expect(inline("_yes_")).toEqual([{ t: "em", c: [text("yes")] }]);
  });

  it("reads links, bare addresses and references", () => {
    expect(inline('[the *plan*](docs/PLAN.md "title") and <https://a.dev>.')).toEqual([
      { t: "link", href: "docs/PLAN.md", c: [text("the "), { t: "em", c: [text("plan")] }] },
      text(" and "),
      { t: "link", href: "https://a.dev", c: [text("https://a.dev")] },
      text("."),
    ]);
    expect(inline("See https://a.dev/x_(y). Then")).toEqual([
      text("See "),
      { t: "link", href: "https://a.dev/x_(y)", c: [text("https://a.dev/x_(y)")] },
      text(". Then"),
    ]);
    expect(parse("[site][s] and [s]\n\n[s]: https://s.dev")).toEqual([
      {
        t: "p",
        c: [{ t: "link", href: "https://s.dev", c: [text("site")] }, text(" and "), { t: "link", href: "https://s.dev", c: [text("s")] }],
      },
    ]);
    expect(inline("[not a link] and arr[0]")).toEqual([text("[not a link] and arr[0]")]);
  });

  it("never passes HTML through", () => {
    expect(inline('<img src=x onerror="alert(1)">Hello <b>bold</b><br>next')).toEqual([text("Hello bold"), { t: "br" }, text("next")]);
    expect(inline("a < b and 1<2")).toEqual([text("a < b and 1<2")]);
    expect(inline("![logo](logo.png)")).toEqual([{ t: "link", href: "logo.png", c: [text("logo")] }]);
  });

  it("reads escapes and line breaks", () => {
    expect(inline("\\*not\\* \\d  \nnext\\\nlast")).toEqual([text("*not* \\d"), { t: "br" }, text("next"), { t: "br" }, text("last")]);
  });
});

describe("target", () => {
  it("resolves links from the document's folder", () => {
    expect(target("https://a.dev", "README.md")).toEqual({ kind: "web", url: "https://a.dev" });
    expect(target("docs/PLAN.md#goals", "README.md")).toEqual({ kind: "doc", path: "docs/PLAN.md", anchor: "goals" });
    expect(target("../PROTOCOL.md", "docs/wiki/Home.md")).toEqual({ kind: "doc", path: "docs/PROTOCOL.md", anchor: "" });
    expect(target("./Agents.md", "docs/wiki/Home.md")).toEqual({ kind: "doc", path: "docs/wiki/Agents.md", anchor: "" });
    expect(target("/README.md", "docs/wiki/Home.md")).toEqual({ kind: "doc", path: "README.md", anchor: "" });
    expect(target("My%20Notes.md", "docs/a.md")).toEqual({ kind: "doc", path: "docs/My Notes.md", anchor: "" });
    expect(target("#methods", "docs/a.md")).toEqual({ kind: "anchor", anchor: "methods" });
  });

  it("refuses what is neither the web nor the project", () => {
    expect(target("../../etc/passwd", "docs/a.md")).toEqual({ kind: "none" });
    expect(target("javascript:alert(1)", "a.md")).toEqual({ kind: "none" });
    expect(target("file:///etc/passwd", "a.md")).toEqual({ kind: "none" });
    expect(target("//evil.dev/x", "a.md")).toEqual({ kind: "none" });
    expect(target("", "a.md")).toEqual({ kind: "none" });
  });
});
