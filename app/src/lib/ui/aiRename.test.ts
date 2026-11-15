import { describe, expect, it, vi } from "vitest";
import { aiRename, requestAiRename } from "./aiRename";
import type { AgentCapabilities } from "../protocol/types";

const base: AgentCapabilities = { statusSource: "hooks", approvals: true, questions: true, chat: true, context: true, usage: true, resume: true };
const claude = { capabilities: { ...base, aiRename: true } };

describe("aiRename", () => {
  it("is offered only for agents that can name their session", () => {
    expect(aiRename({ hasSession: true, status: "idle" }, { capabilities: base }).offered).toBe(false);
    expect(aiRename({ hasSession: true, status: "idle" }, { capabilities: null }).offered).toBe(false);
    expect(aiRename({ hasSession: true, status: "idle" }, null).offered).toBe(false);
    expect(aiRename({ hasSession: true, status: "idle" }, claude)).toEqual({ offered: true, blocked: null });
    expect(aiRename({ hasSession: true, status: "done" }, claude).blocked).toBeNull();
  });

  it("is blocked without a session or during a turn", () => {
    expect(aiRename({ hasSession: false, status: "exited" }, claude).blocked).toMatch(/Start the session/);
    expect(aiRename({ hasSession: false, status: "draft" }, claude).blocked).toMatch(/Start the session/);
    expect(aiRename({ hasSession: true, status: "running" }, claude).blocked).toMatch(/between turns/);
    expect(aiRename({ hasSession: true, status: "needs_you" }, claude).blocked).toMatch(/between turns/);
  });
});

describe("requestAiRename", () => {
  it("sends one request per card at a time and swallows failures", async () => {
    let finish!: () => void;
    const call = vi.fn(() => new Promise<void>((r) => (finish = r)));
    const first = requestAiRename({ call }, "a");
    await requestAiRename({ call }, "a");
    expect(call).toHaveBeenCalledTimes(1);
    finish();
    await first;
    const failing = vi.fn(() => Promise.reject(new Error("no")));
    await requestAiRename({ call: failing }, "a");
    expect(failing).toHaveBeenCalledTimes(1);
  });
});
