import { describe, expect, it } from "vitest";
import { bytesLabel, isImage, sizeChange } from "./image";

describe("isImage", () => {
  it("goes by the extension, like the daemon", () => {
    expect(isImage("public/Logo.PNG")).toBe(true);
    expect(isImage("icons/app.svg")).toBe(true);
    expect(isImage("a.jpeg")).toBe(true);
    expect(isImage("src/png")).toBe(false);
    expect(isImage("dir.png/notes.txt")).toBe(false);
    expect(isImage("README.md")).toBe(false);
  });
});

describe("sizes", () => {
  it("labels bytes", () => {
    expect(bytesLabel(812)).toBe("812 B");
    expect(bytesLabel(14 * 1024 + 200)).toBe("14.2 KB");
    expect(bytesLabel(3.1 * 1024 * 1024)).toBe("3.1 MB");
  });
  it("says how much a file grew or shrank", () => {
    expect(sizeChange(1000, 1120)).toBe("+12%");
    expect(sizeChange(1000, 600)).toBe("−40%");
    expect(sizeChange(1000, 2500)).toBe("×2.5");
    expect(sizeChange(4100, 94900)).toBe("×23");
    expect(sizeChange(1000, 1000)).toBe("");
    expect(sizeChange(undefined, 10)).toBe("");
    expect(sizeChange(10, undefined)).toBe("");
  });
});
