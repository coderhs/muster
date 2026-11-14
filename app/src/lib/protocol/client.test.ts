import { describe, expect, it, vi } from "vitest";
import { MusterClient, ProtocolError, type SocketLike } from "./client";
import { decodeTermFrame, encodeTermFrame } from "./frames";

class FakeSocket implements SocketLike {
  binaryType = "blob";
  readyState = 0;
  sent: any[] = [];
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  constructor(public url: string) {}
  send(data: string) {
    this.sent.push(JSON.parse(data));
  }
  close() {
    this.readyState = 3;
  }
  // test helpers
  open() {
    this.readyState = 1;
    this.onopen?.({});
  }
  reply(id: number, r: unknown) {
    this.onmessage?.({ data: JSON.stringify({ t: "res", id, ok: true, r }) });
  }
  fail(id: number, code: string, msg = "") {
    this.onmessage?.({ data: JSON.stringify({ t: "res", id, ok: false, e: { code, msg } }) });
  }
  event(m: string, p: unknown) {
    this.onmessage?.({ data: JSON.stringify({ t: "ev", m, p }) });
  }
  drop(code = 1006) {
    this.readyState = 3;
    this.onclose?.({ code });
  }
  last() {
    return this.sent[this.sent.length - 1];
  }
}

/** Manual timers so backoff is deterministic. */
function timers() {
  let seq = 0;
  const pending = new Map<number, { fn: () => void; ms: number }>();
  return {
    set: (fn: () => void, ms: number) => {
      pending.set(++seq, { fn, ms });
      return seq;
    },
    clear: (h: unknown) => void pending.delete(h as number),
    /** Run the earliest timer whose delay is below `maxMs` (skips request timeouts). */
    runRetry(maxMs = 10_000) {
      const e = [...pending.entries()].filter(([, t]) => t.ms <= maxMs).sort((a, b) => a[1].ms - b[1].ms)[0];
      if (!e) return null;
      pending.delete(e[0]);
      e[1].fn();
      return e[1].ms;
    },
    delays: () => [...pending.values()].map((t) => t.ms),
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

function setup(opts: { token?: () => string } = {}) {
  const sockets: FakeSocket[] = [];
  const t = timers();
  const resolve = vi.fn(async () => ({ url: "ws://127.0.0.1:9/ws", token: opts.token?.() ?? "tok" }));
  const client = new MusterClient({
    client: "desktop",
    resolve,
    socketFactory: (url) => {
      const s = new FakeSocket(url);
      sockets.push(s);
      return s;
    },
    setTimer: t.set,
    clearTimer: t.clear,
    random: () => 1,
    backoff: { initialMs: 100, maxMs: 1000 },
  });
  return { client, sockets, t, resolve };
}

async function connect(env: ReturnType<typeof setup>) {
  env.client.start();
  await flush();
  const s = env.sockets[env.sockets.length - 1];
  s.open();
  const hello = s.last();
  s.reply(hello.id, { protocol: 1, daemonVersion: "1.0" });
  await flush();
  return s;
}

describe("frames", () => {
  it("round-trips a terminal frame", () => {
    const f = encodeTermFrame("card-é", new Uint8Array([1, 2, 255]));
    expect(f[0]).toBe(1);
    const d = decodeTermFrame(f)!;
    expect(d.sid).toBe("card-é");
    expect([...d.data]).toEqual([1, 2, 255]);
  });
  it("rejects short or foreign frames", () => {
    expect(decodeTermFrame(new Uint8Array([2, 0, 0]))).toBeNull();
    expect(decodeTermFrame(new Uint8Array([1, 0, 9, 65]))).toBeNull();
  });
});

describe("MusterClient", () => {
  it("sends hello first, then becomes ready and fires onReady", async () => {
    const env = setup();
    const ready = vi.fn();
    env.client.onReady(ready);
    const s = await connect(env);
    expect(s.sent[0]).toMatchObject({ t: "req", m: "hello", p: { token: "tok", client: "desktop", protocol: 1 } });
    expect(s.binaryType).toBe("arraybuffer");
    expect(env.client.state).toEqual({ kind: "ready", daemonVersion: "1.0" });
    expect(ready).toHaveBeenCalledTimes(1);
  });

  it("correlates responses by id, including out of order", async () => {
    const env = setup();
    const s = await connect(env);
    const a = env.client.request("project.list", {});
    const b = env.client.request("agent.list", {});
    const [ra, rb] = [s.sent[1], s.sent[2]];
    expect(ra.id).not.toBe(rb.id);
    s.reply(rb.id, { agents: [{ id: "shell" }] });
    s.reply(ra.id, { projects: [] });
    expect(await a).toEqual({ projects: [] });
    expect(await b).toEqual({ agents: [{ id: "shell" }] });
  });

  it("rejects with the daemon's error code", async () => {
    const env = setup();
    const s = await connect(env);
    const p = env.client.request("card.start", { id: "x" });
    s.fail(s.last().id, "conflict", "already running");
    await expect(p).rejects.toMatchObject({ code: "conflict", message: "already running" });
  });

  it("refuses requests while not connected", async () => {
    const env = setup();
    await expect(env.client.request("project.list", {})).rejects.toBeInstanceOf(ProtocolError);
  });

  it("dispatches events and binary terminal frames by sid", async () => {
    const env = setup();
    const s = await connect(env);
    const ev = vi.fn();
    const term = vi.fn();
    const other = vi.fn();
    env.client.on("card.changed", ev);
    const off = env.client.onTerm("c1", term);
    env.client.onTerm("c2", other);
    s.event("card.changed", { removedId: "c9", projectId: "p" });
    s.onmessage!({ data: encodeTermFrame("c1", "hi").buffer });
    expect(ev).toHaveBeenCalledWith({ removedId: "c9", projectId: "p" });
    expect(new TextDecoder().decode(term.mock.calls[0][0])).toBe("hi");
    expect(other).not.toHaveBeenCalled();
    off();
    s.onmessage!({ data: encodeTermFrame("c1", "again").buffer });
    expect(term).toHaveBeenCalledTimes(1);
  });

  it("fails pending requests on disconnect and reconnects with growing backoff", async () => {
    const env = setup();
    const ready = vi.fn();
    env.client.onReady(ready);
    const s = await connect(env);
    const p = env.client.request("board.get", { projectId: "p" });
    s.drop();
    await expect(p).rejects.toMatchObject({ code: "unavailable" });
    expect(env.client.state.kind).toBe("waiting");

    // First retry fails to connect (socket closes before open): delay doubles.
    expect(env.t.runRetry()).toBe(100);
    await flush();
    env.sockets[1].drop();
    expect(env.t.delays()).toContain(200);
    expect(env.t.runRetry()).toBe(200);
    await flush();
    env.sockets[2].drop();
    expect(env.t.runRetry()).toBe(400);
    await flush();

    // Succeeds: ready again, onReady fires again so callers re-sync, backoff resets.
    const s4 = env.sockets[3];
    s4.open();
    s4.reply(s4.last().id, { protocol: 1, daemonVersion: "1.1" });
    await flush();
    expect(ready).toHaveBeenCalledTimes(2);
    expect(env.resolve).toHaveBeenCalledTimes(4);
    s4.drop();
    expect(env.t.runRetry()).toBe(100);
  });

  it("caps the backoff", async () => {
    const env = setup();
    env.client.start();
    for (let i = 0; i < 8; i++) {
      await flush();
      env.sockets[env.sockets.length - 1].drop();
      env.t.runRetry();
    }
    await flush();
    env.sockets[env.sockets.length - 1].drop();
    expect(Math.max(...env.t.delays().filter((d) => d <= 10_000))).toBe(1000);
  });

  it("passes an unauthorized hello back to resolve so the token can be re-read", async () => {
    let token = "old";
    const env = setup({ token: () => token });
    env.client.start();
    await flush();
    const s = env.sockets[0];
    s.open();
    s.fail(s.last().id, "unauthorized", "bad token");
    await flush();
    s.drop(4001);
    expect(env.client.state.kind).toBe("waiting");
    token = "new";
    env.t.runRetry();
    await flush();
    expect((env.resolve.mock.calls[1] as unknown[])[0]).toMatchObject({ code: "unauthorized" });
    const s2 = env.sockets[1];
    s2.open();
    expect(s2.last().p.token).toBe("new");
  });

  it("rejects a daemon speaking another protocol version", async () => {
    const env = setup();
    env.client.start();
    await flush();
    const s = env.sockets[0];
    s.open();
    s.reply(s.last().id, { protocol: 2, daemonVersion: "9" });
    await flush();
    expect(env.client.state).toMatchObject({ kind: "waiting" });
    expect((env.client.state as { error: string }).error).toMatch(/protocol 2/);
  });

  it("times out requests that never get an answer", async () => {
    const env = setup();
    const s = await connect(env);
    const p = env.client.request("project.list", {}, 50_000);
    expect(s.sent.length).toBe(2);
    // The request timer is the only one with that delay.
    env.t.runRetry(60_000);
    await expect(p).rejects.toMatchObject({ code: "unavailable" });
  });

  it("stop() closes the socket and does not reconnect", async () => {
    const env = setup();
    const s = await connect(env);
    env.client.stop();
    expect(s.readyState).toBe(3);
    expect(env.client.state.kind).toBe("stopped");
    expect(env.t.runRetry()).toBeNull();
  });
});
