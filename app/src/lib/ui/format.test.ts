import { describe, expect, it } from "vitest";
import { bytes, cpu } from "./format";
import { linkKey } from "../state/app.svelte";

describe("format", () => {
  it("formats memory and CPU", () => {
    expect(bytes(512)).toBe("512 B");
    expect(bytes(180 * 2 ** 20)).toBe("180 MB");
    expect(bytes(1.25 * 2 ** 30)).toBe("1.3 GB");
    expect(bytes(12 * 2 ** 30)).toBe("12 GB");
    expect(cpu(0)).toBe("0%");
    expect(cpu(3.44)).toBe("3.4%");
    expect(cpu(54.2)).toBe("54%");
  });
  it("keys links the same in both directions", () => {
    expect(linkKey("b", "a")).toBe("a|b");
    expect(linkKey("a", "b")).toBe("a|b");
  });
});
