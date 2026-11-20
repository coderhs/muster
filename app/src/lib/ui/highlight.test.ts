import { describe, expect, it } from "vitest";
import { highlightLines, languageFor } from "./highlight";

describe("file viewer highlighting", () => {
  it("picks a language from the name", () => {
    expect(languageFor("crates/x/src/store.rs")).toBe("rust");
    expect(languageFor("app/Gemfile")).toBe("ruby");
    expect(languageFor("views/a.html.erb")).toBe("erb");
    expect(languageFor("notes.txt")).toBeNull();
  });

  it("keeps one entry per line, with spans closed on each line", () => {
    const src = "/* a\n   b */\nfn main() {}\n";
    const lines = highlightLines(src, "rust");
    expect(lines.length).toBe(4);
    for (const l of lines) expect((l.match(/<span/g) ?? []).length).toBe((l.match(/<\/span>/g) ?? []).length);
    expect(lines[1]).toContain("b */");
    expect(lines[1].startsWith("<span")).toBe(true);
  });

  it("escapes text that is not highlighted", () => {
    expect(highlightLines("<script>&</script>", null)).toEqual(["&lt;script&gt;&amp;&lt;/script&gt;"]);
    expect(highlightLines("let a = '<b>';", "typescript").join("")).not.toContain("<b>");
  });
});
