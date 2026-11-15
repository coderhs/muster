import { describe, expect, it } from "vitest";
import { idleChoices, idleLabel, isSleeping } from "./idle";

describe("idleLabel", () => {
  it("uses the largest whole unit", () => {
    expect(idleLabel(0)).toBe("Never");
    expect(idleLabel(60)).toBe("1 hour");
    expect(idleLabel(240)).toBe("4 hours");
    expect(idleLabel(1440)).toBe("1 day");
    expect(idleLabel(4320)).toBe("3 days");
    expect(idleLabel(90)).toBe("90 minutes");
  });
});

describe("idleChoices", () => {
  it("adds a value that is not on the list", () => {
    expect(idleChoices(240).length).toBe(7);
    expect(idleChoices(90).at(-1)).toEqual({ value: "90", label: "90 minutes" });
  });
});

describe("isSleeping", () => {
  it("is an exited card that Muster stopped", () => {
    expect(isSleeping({ status: "exited", idleStoppedAt: 1 })).toBe(true);
    expect(isSleeping({ status: "exited" })).toBe(false);
    expect(isSleeping({ status: "running", idleStoppedAt: 1 })).toBe(false);
  });
});
