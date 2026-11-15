import { describe, expect, it } from "vitest";
import { commitWhen, sameBuild } from "./about";

describe("sameBuild", () => {
  it("compares known commits", () => {
    expect(sameBuild("abc123456", "abc123456")).toBe(true);
    expect(sameBuild("abc123456", "def654321")).toBe(false);
    expect(sameBuild("abc123456", "abc123456+")).toBe(false);
  });
  it("is undecided when either side is unknown", () => {
    expect(sameBuild(undefined, "abc123456")).toBeNull();
    expect(sameBuild("abc123456", undefined)).toBeNull();
    expect(sameBuild("unknown", "unknown")).toBeNull();
  });
});

describe("commitWhen", () => {
  it("formats a date and tolerates a missing or bad one", () => {
    expect(commitWhen("2026-11-14T18:57:00+05:30")).toMatch(/2026/);
    expect(commitWhen("")).toBe("");
    expect(commitWhen("nonsense")).toBe("");
  });
});
