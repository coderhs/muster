import { describe, expect, it } from "vitest";
import { inkOn, railLabels } from "./rail";

describe("railLabels", () => {
  it("uses one letter when the first letter is unique", () => {
    expect(railLabels(["muster", "Api", "web"])).toEqual(["M", "A", "W"]);
  });
  it("adds the second letter when the first is shared, ignoring case", () => {
    expect(railLabels(["Muster", "mobile", "api"])).toEqual(["Mu", "Mo", "A"]);
  });
  it("skips punctuation and copes with short or empty names", () => {
    expect(railLabels(["-x", "x", "  ", "@scope/pkg"])).toEqual(["X", "X", "?", "S"]);
  });
});

describe("inkOn", () => {
  it("picks dark ink on light colours and white on dark ones", () => {
    expect(inkOn("#ffd60a")).toBe("#1d1d1f");
    expect(inkOn("#0a84ff")).toBe("#ffffff");
    expect(inkOn("#fff")).toBe("#1d1d1f");
  });
});
