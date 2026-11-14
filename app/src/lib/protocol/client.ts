// Protocol client: one WebSocket, request/response correlation, events, binary
// terminal frames, and reconnect with backoff. Callers re-sync in `onReady`, which
// fires after every successful `hello` (first connect and each reconnect).

import { decodeTermFrame } from "./frames";
import {
  PROTOCOL_VERSION,
  type ClientKind,
  type EventName,
  type Events,
  type Frame,
  type Method,
  type Params,
  type Result,
} from "./types";

export class ProtocolError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message || code);
    this.name = "ProtocolError";
  }
}

export interface Endpoint {
  url: string;
  token: string;
}

/** Minimal surface of the browser WebSocket we rely on (lets tests inject a fake). */
export interface SocketLike {
  binaryType: string;
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: unknown) => void) | null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
}

export type ConnState =
  | { kind: "idle" }
  | { kind: "connecting"; attempt: number }
  | { kind: "ready"; daemonVersion: string }
  | { kind: "waiting"; attempt: number; retryInMs: number; error: string }
  | { kind: "stopped" };

export interface ClientOptions {
  /** Called before every connection attempt; may re-read port/token. */
  resolve: (lastError: ProtocolError | null) => Promise<Endpoint>;
  client: ClientKind;
  socketFactory?: (url: string) => SocketLike;
  requestTimeoutMs?: number;
  backoff?: { initialMs: number; maxMs: number };
  /** Injected for tests. */
  random?: () => number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (h: unknown) => void;
}

interface Pending {
  resolve: (r: unknown) => void;
  reject: (e: ProtocolError) => void;
  timer: unknown;
}

type Handler<T> = (p: T) => void;

const OPEN = 1;

export class MusterClient {
  private ws: SocketLike | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private evHandlers = new Map<string, Set<Handler<unknown>>>();
  private termHandlers = new Map<string, Set<Handler<Uint8Array>>>();
  private stateHandlers = new Set<Handler<ConnState>>();
  private readyHandlers = new Set<() => void>();
  private retryTimer: unknown = null;
  private attempt = 0;
  private lastError: ProtocolError | null = null;
  private running = false;
  private generation = 0;
  state: ConnState = { kind: "idle" };

  private readonly opt: Required<Omit<ClientOptions, "socketFactory">> & {
    socketFactory: (url: string) => SocketLike;
  };

  constructor(opts: ClientOptions) {
    this.opt = {
      requestTimeoutMs: 20_000,
      backoff: { initialMs: 250, maxMs: 5_000 },
      random: Math.random,
      setTimer: (fn, ms) => setTimeout(fn, ms),
      clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
      socketFactory: (url) => new WebSocket(url) as unknown as SocketLike,
      ...opts,
    };
  }

  get ready(): boolean {
    return this.state.kind === "ready";
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.attempt = 0;
    void this.connect();
  }

  stop(): void {
    this.running = false;
    this.generation++;
    if (this.retryTimer) this.opt.clearTimer(this.retryTimer);
    this.retryTimer = null;
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      detach(ws);
      try {
        ws.close(1000, "client stop");
      } catch {
        /* already closed */
      }
    }
    this.failPending(new ProtocolError("unavailable", "client stopped"));
    this.setState({ kind: "stopped" });
  }

  /** Drop the current socket and reconnect immediately (e.g. after the user edits the token). */
  reconnectNow(): void {
    if (!this.running) return this.start();
    if (this.retryTimer) this.opt.clearTimer(this.retryTimer);
    this.retryTimer = null;
    this.attempt = 0;
    this.dropSocket(new ProtocolError("unavailable", "reconnecting"), false);
    void this.connect();
  }

  request<M extends Method>(m: M, p: Params<M>, timeoutMs?: number): Promise<Result<M>> {
    if (this.state.kind !== "ready" || !this.ws) {
      return Promise.reject(new ProtocolError("unavailable", "not connected to the daemon"));
    }
    return this.send(this.ws, m, p, timeoutMs) as Promise<Result<M>>;
  }

  on<E extends EventName>(event: E, fn: Handler<Events[E]>): () => void {
    return addTo(this.evHandlers, event, fn as Handler<unknown>);
  }

  onTerm(sid: string, fn: Handler<Uint8Array>): () => void {
    return addTo(this.termHandlers, sid, fn);
  }

  onState(fn: Handler<ConnState>): () => void {
    this.stateHandlers.add(fn);
    fn(this.state);
    return () => this.stateHandlers.delete(fn);
  }

  /** Fires after every successful hello. Use it to (re)fetch state and re-attach. */
  onReady(fn: () => void): () => void {
    this.readyHandlers.add(fn);
    if (this.state.kind === "ready") fn();
    return () => this.readyHandlers.delete(fn);
  }

  // ---- internals ----

  private setState(s: ConnState) {
    this.state = s;
    for (const fn of [...this.stateHandlers]) fn(s);
  }

  private async connect() {
    const gen = ++this.generation;
    this.attempt++;
    this.setState({ kind: "connecting", attempt: this.attempt });
    let ep: Endpoint;
    try {
      ep = await this.opt.resolve(this.lastError);
    } catch (e) {
      if (gen !== this.generation) return;
      return this.scheduleRetry(new ProtocolError("unavailable", errText(e)));
    }
    if (gen !== this.generation || !this.running) return;

    let ws: SocketLike;
    try {
      ws = this.opt.socketFactory(ep.url);
    } catch (e) {
      return this.scheduleRetry(new ProtocolError("unavailable", errText(e)));
    }
    ws.binaryType = "arraybuffer";
    this.ws = ws;

    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.send(ws, "hello", { token: ep.token, client: this.opt.client, protocol: PROTOCOL_VERSION })
        .then((r) => {
          if (this.ws !== ws) return;
          const res = r as Result<"hello">;
          if (res.protocol !== PROTOCOL_VERSION) {
            throw new ProtocolError("invalid", `daemon speaks protocol ${res.protocol}, client speaks ${PROTOCOL_VERSION}`);
          }
          this.attempt = 0;
          this.lastError = null;
          this.setState({ kind: "ready", daemonVersion: res.daemonVersion });
          for (const fn of [...this.readyHandlers]) {
            try {
              fn();
            } catch (e) {
              console.error("onReady handler failed", e);
            }
          }
        })
        .catch((e) => {
          if (this.ws !== ws) return;
          this.dropSocket(e instanceof ProtocolError ? e : new ProtocolError("internal", errText(e)), true);
        });
    };
    ws.onmessage = (ev) => {
      if (this.ws === ws) this.handleMessage(ev.data);
    };
    ws.onerror = () => {
      /* onclose follows and carries the reconnect */
    };
    ws.onclose = (ev) => {
      if (this.ws !== ws) return;
      const why = ev?.reason ? `${ev.reason}` : `connection closed${ev?.code ? ` (${ev.code})` : ""}`;
      this.dropSocket(this.lastErrorFromClose ?? new ProtocolError("unavailable", why), true);
    };
  }

  private lastErrorFromClose: ProtocolError | null = null;

  private dropSocket(err: ProtocolError, retry: boolean) {
    const ws = this.ws;
    this.ws = null;
    this.lastErrorFromClose = null;
    if (ws) {
      detach(ws);
      try {
        ws.close();
      } catch {
        /* ignore */
      }
    }
    this.failPending(err);
    if (retry && this.running) this.scheduleRetry(err);
  }

  private scheduleRetry(err: ProtocolError) {
    if (!this.running) return;
    this.lastError = err;
    const { initialMs, maxMs } = this.opt.backoff;
    const base = Math.min(maxMs, initialMs * 2 ** this.attempt);
    // Up to 25% jitter so several clients don't stampede a restarting daemon.
    const delay = Math.round(base * (0.75 + this.opt.random() * 0.25));
    this.setState({ kind: "waiting", attempt: this.attempt, retryInMs: delay, error: err.message });
    if (this.retryTimer) this.opt.clearTimer(this.retryTimer);
    this.retryTimer = this.opt.setTimer(() => {
      this.retryTimer = null;
      void this.connect();
    }, delay);
  }

  private send(ws: SocketLike, m: string, p: unknown, timeoutMs?: number): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = this.opt.setTimer(() => {
        if (this.pending.delete(id)) reject(new ProtocolError("unavailable", `${m}: no response from daemon`));
      }, timeoutMs ?? this.opt.requestTimeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try {
        if (ws.readyState !== OPEN) throw new Error("socket not open");
        ws.send(JSON.stringify({ t: "req", id, m, p }));
      } catch (e) {
        this.pending.delete(id);
        this.opt.clearTimer(timer);
        reject(new ProtocolError("unavailable", errText(e)));
      }
    });
  }

  private handleMessage(data: unknown) {
    if (typeof data !== "string") {
      const frame = data instanceof ArrayBuffer || data instanceof Uint8Array ? decodeTermFrame(data) : null;
      if (!frame) return;
      const hs = this.termHandlers.get(frame.sid);
      if (hs) for (const fn of [...hs]) fn(frame.data);
      return;
    }
    let f: Frame;
    try {
      f = JSON.parse(data);
    } catch {
      console.warn("muster: dropping malformed frame");
      return;
    }
    if (f.t === "res") {
      const p = this.pending.get(f.id);
      if (!p) return;
      this.pending.delete(f.id);
      this.opt.clearTimer(p.timer);
      if (f.ok) p.resolve(f.r ?? {});
      else {
        const err = new ProtocolError(f.e?.code ?? "internal", f.e?.msg ?? "");
        // An unauthorized reply means the daemon is about to close the socket;
        // remember why so the reconnect path can re-resolve the token.
        if (err.code === "unauthorized") this.lastErrorFromClose = err;
        p.reject(err);
      }
    } else if (f.t === "ev") {
      const hs = this.evHandlers.get(f.m);
      if (hs)
        for (const fn of [...hs]) {
          try {
            fn(f.p);
          } catch (e) {
            console.error(`muster: handler for ${f.m} failed`, e);
          }
        }
    }
  }

  private failPending(err: ProtocolError) {
    const all = [...this.pending.values()];
    this.pending.clear();
    for (const p of all) {
      this.opt.clearTimer(p.timer);
      p.reject(err);
    }
  }
}

function addTo<T>(map: Map<string, Set<Handler<T>>>, key: string, fn: Handler<T>): () => void {
  let s = map.get(key);
  if (!s) map.set(key, (s = new Set()));
  s.add(fn);
  return () => {
    const cur = map.get(key);
    if (!cur) return;
    cur.delete(fn);
    if (cur.size === 0) map.delete(key);
  };
}

function detach(ws: SocketLike) {
  ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
}

export function errText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
