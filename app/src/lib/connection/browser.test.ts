import { describe, expect, it } from "vitest";
import { deviceName } from "./browser";

describe("deviceName", () => {
  it("names common devices", () => {
    expect(deviceName("Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X)", true)).toBe("iPhone");
    expect(deviceName("Mozilla/5.0 (iPad; CPU OS 19_0 like Mac OS X)", true)).toBe("iPad");
    // iPadOS Safari reports itself as a Mac; only the touch screen gives it away.
    expect(deviceName("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", true)).toBe("iPad");
    expect(deviceName("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", false)).toBe("Mac");
    expect(deviceName("Mozilla/5.0 (Linux; Android 15; SM-T733) Safari/537.36", true)).toBe("Android tablet");
    expect(deviceName("Mozilla/5.0 (Linux; Android 15; SM-S908U1) Mobile Safari/537.36", true)).toBe("Android phone");
    expect(deviceName("curl/8", false)).toBe("Browser");
  });
});
