// The single live terminal. It exists only while a card modal shows a session and is
// disposed on close, so at most one xterm instance is ever alive.

import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import type { MusterClient } from "../protocol/client";
import { errText } from "../protocol/client";
import { getTermPrefs, onTermPrefs } from "./prefs";
import { CTRL_ENTER, isCtrlEnter, isNewlineChord } from "./keys";
import { isOpenChord, pathsForPaste, refsInRows, webUrl } from "./links";
import { findRefs, type FileRef } from "../ui/fileRef";

const DARK: ITheme = {
  background: "#18181a",
  foreground: "#e6e6e8",
  cursor: "#e6e6e8",
  cursorAccent: "#18181a",
  selectionBackground: "#3a5a8c99",
  black: "#1c1c1e",
  red: "#ff6b6b",
  green: "#5fd38d",
  yellow: "#f2c55c",
  blue: "#5ea2ff",
  magenta: "#d58cf5",
  cyan: "#5fd3d3",
  white: "#d0d0d4",
  brightBlack: "#6c6c72",
  brightRed: "#ff8a8a",
  brightGreen: "#7ee2a5",
  brightYellow: "#ffd986",
  brightBlue: "#86baff",
  brightMagenta: "#e3a9fa",
  brightCyan: "#86e6e6",
  brightWhite: "#ffffff",
};

const LIGHT: ITheme = {
  background: "#ffffff",
  foreground: "#1d1d1f",
  cursor: "#1d1d1f",
  cursorAccent: "#ffffff",
  selectionBackground: "#b3d4ff",
  black: "#1d1d1f",
  red: "#c4302b",
  green: "#1f8a3b",
  yellow: "#9a6a00",
  blue: "#1d5fd1",
  magenta: "#9b3fc4",
  cyan: "#0f7f8a",
  white: "#bdbdc2",
  brightBlack: "#6e6e73",
  brightRed: "#e0443e",
  brightGreen: "#28a745",
  brightYellow: "#b58900",
  brightBlue: "#2f7af0",
  brightMagenta: "#b254de",
  brightCyan: "#169fae",
  brightWhite: "#f5f5f7",
};

const darkQuery = () => window.matchMedia?.("(prefers-color-scheme: dark)");
const isMac = () => /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

export interface TermCallbacks {
  onError?: (msg: string) => void;
  onSize?: (cols: number, rows: number) => void;
  /** Open a web link the user Cmd+clicked. Already checked to be http(s). */
  openUrl?: (url: string) => void;
  /** Show a file path the user Cmd+clicked (`src/a.rs:12`) in the file viewer. */
  openFile?: (ref: FileRef) => void;
  /**
   * Send Ctrl+Enter in the extended key encoding. Only for programs known to read it:
   * a plain shell would print it as text.
   */
  extendedCtrlEnter?: boolean;
  /** Paths of the files on the clipboard, when the host can tell. */
  clipboardFiles?: () => Promise<string[]>;
}

export interface TermSession {
  focus(): void;
  dispose(): void;
}

/**
 * Attach `cardId`'s session to a fresh terminal inside `host`. Re-attaches by itself
 * after a reconnect. Call `dispose()` to detach and free everything.
 */
export function openTerminal(host: HTMLElement, client: MusterClient, cardId: string, cb: TermCallbacks = {}): TermSession {
  const mq = darkQuery();
  const prefs = getTermPrefs();
  // A plain click belongs to the program in the terminal; only Cmd+click opens a link.
  const openLink = (ev: MouseEvent, uri: string) => {
    if (!isOpenChord(ev, isMac())) return;
    const url = webUrl(uri);
    if (url) cb.openUrl?.(url);
  };
  const term = new Terminal({
    fontFamily: prefs.fontFamily,
    fontSize: prefs.fontSize,
    lineHeight: prefs.lineHeight,
    fontWeight: prefs.bold ? "bold" : "normal",
    fontWeightBold: "bold",
    cursorBlink: true,
    scrollback: 5000,
    allowProposedApi: false,
    macOptionClickForcesSelection: true,
    theme: mq?.matches === false ? LIGHT : DARK,
    // Links a program marks itself (OSC 8).
    linkHandler: { activate: (ev, uri) => openLink(ev, uri) },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  // URLs that appear as plain text.
  term.loadAddon(new WebLinksAddon(openLink));
  // File paths, such as `src/lib.rs:12` in an agent's reply or a compiler error.
  if (cb.openFile) {
    const openFile = cb.openFile;
    term.registerLinkProvider({
      provideLinks(y, callback) {
        const buf = term.buffer.active;
        let first = y - 1;
        while (first > 0 && buf.getLine(first)?.isWrapped) first--;
        let last = y - 1;
        while (last - first < 20 && buf.getLine(last + 1)?.isWrapped) last++;
        const rows: string[] = [];
        for (let i = first; i <= last; i++) rows.push(buf.getLine(i)?.translateToString(i === last) ?? "");
        const found = refsInRows(rows, term.cols, first, (t) => findRefs(t, true)).filter(
          (f) => f.range.start.y <= y && f.range.end.y >= y,
        );
        callback(
          found.length
            ? found.map(({ ref, range }) => ({
                range,
                text: ref.path,
                decorations: { pointerCursor: true, underline: true },
                activate: (ev: MouseEvent) => {
                  if (isOpenChord(ev, isMac())) openFile({ path: ref.path, line: ref.line, col: ref.col });
                },
              }))
            : undefined,
        );
      },
    });
  }
  term.open(host);
  // The GPU renderer keeps every cell on an exact grid and draws box and block
  // characters itself, as a native terminal does. Without WebGL the DOM renderer stays.
  try {
    const gl = new WebglAddon();
    gl.onContextLoss(() => gl.dispose());
    term.loadAddon(gl);
  } catch {
    /* DOM renderer */
  }

  let disposed = false;
  let attached = false;
  let attaching: Promise<void> | null = null;
  let requested = { cols: 0, rows: 0 };
  const offs: Array<() => void> = [];

  const send = (data: string, raw = false) => {
    if (!attached || disposed) return;
    client
      .request("term.write", raw ? { cardId, data, raw } : { cardId, data })
      .catch((e) => cb.onError?.(`Input not delivered: ${errText(e)}`));
  };

  const applySize = (cols: number, rows: number) => {
    if (cols > 0 && rows > 0 && (term.cols !== cols || term.rows !== rows)) term.resize(cols, rows);
    cb.onSize?.(cols, rows);
  };

  const proposed = () => {
    const d = fit.proposeDimensions();
    if (!d || !Number.isFinite(d.cols) || !Number.isFinite(d.rows)) return null;
    return { cols: Math.max(20, d.cols), rows: Math.max(5, d.rows) };
  };

  const attach = () => {
    if (disposed || !client.ready) return;
    const size = proposed() ?? { cols: 80, rows: 24 };
    attached = false;
    // The daemon repaints the full screen after attach, so start from a blank terminal.
    term.reset();
    requested = size;
    attaching = client
      .request("session.attach", { cardId, cols: size.cols, rows: size.rows })
      .then((r) => {
        if (disposed) return;
        attached = true;
        applySize(r.cols, r.rows);
      })
      .catch((e) => {
        if (!disposed) cb.onError?.(`Could not attach: ${errText(e)}`);
      })
      .finally(() => (attaching = null));
  };

  offs.push(client.onTerm(cardId, (bytes) => term.write(bytes)));
  offs.push(
    client.on("session.size", (p) => {
      if (p.cardId === cardId) applySize(p.cols, p.rows);
    }),
  );
  // Fires immediately when already connected, and again after each reconnect.
  offs.push(client.onReady(attach));
  offs.push(
    client.onState((s) => {
      if (s.kind !== "ready") attached = false;
    }),
  );

  const dataSub = term.onData((d) => send(d));
  const binSub = term.onBinary((d) => send(d));

  term.attachCustomKeyEventHandler((ev) => {
    if (isNewlineChord(ev)) {
      // Newline without submit, as agent CLIs expect it. The keypress that follows the
      // keydown must be refused too: xterm would otherwise send a plain Enter after
      // the newline, which submits the prompt.
      if (ev.type === "keydown") send("\x1b\r");
      ev.preventDefault();
      return false;
    }
    if (cb.extendedCtrlEnter && isCtrlEnter(ev)) {
      // Raw: tmux would read the sequence as a key and hand the program a plain Enter.
      if (ev.type === "keydown") send(CTRL_ENTER, true);
      ev.preventDefault();
      return false;
    }
    if (ev.type !== "keydown") return true;
    const mac = isMac();
    const cmd = mac ? ev.metaKey : ev.ctrlKey && ev.shiftKey;
    if (mac && ev.metaKey && !ev.ctrlKey) {
      switch (ev.key) {
        case "ArrowLeft":
          send("\x01");
          return false;
        case "ArrowRight":
          send("\x05");
          return false;
        case "Backspace":
          send("\x15");
          return false;
        case "k":
          term.clear();
          return false;
        case "a":
          term.selectAll();
          ev.preventDefault();
          return false;
      }
      // Every other Cmd chord (copy, paste, close, ...) belongs to the app, not the shell.
      return false;
    }
    if (mac && ev.altKey && !ev.metaKey && !ev.ctrlKey) {
      if (ev.key === "ArrowLeft") return send("\x1bb"), false;
      if (ev.key === "ArrowRight") return send("\x1bf"), false;
      if (ev.key === "Backspace") return send("\x17"), false;
    }
    // Modal shortcuts (switch Terminal/Chat, move focus) must reach the page.
    if (ev.ctrlKey && (ev.key === "Tab" || ev.key === "`")) return false;
    if (!mac && cmd && (ev.key === "C" || ev.key === "V")) return false;
    return true;
  });

  // A file copied in the Finder pastes as its path, so an agent can open it. The page
  // only gets the file's contents, not where it is; the host knows. Anything else on
  // the clipboard (text, a copied image) pastes as before.
  const onPaste = (ev: ClipboardEvent) => {
    if (!cb.clipboardFiles || !ev.clipboardData?.types.includes("Files")) return;
    const text = ev.clipboardData.getData("text/plain");
    ev.preventDefault();
    ev.stopPropagation();
    cb.clipboardFiles()
      .catch(() => [] as string[])
      .then((paths) => {
        if (!disposed) term.paste(paths.length ? pathsForPaste(paths) : text);
      });
  };
  host.addEventListener("paste", onPaste, true);

  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  const onResize = () => {
    if (resizeTimer) clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      resizeTimer = null;
      const size = proposed();
      if (!size || disposed) return;
      if (!attached) {
        if (!attaching) applySize(size.cols, size.rows);
        return;
      }
      if (size.cols === requested.cols && size.rows === requested.rows) return;
      requested = size;
      // Show the new size at once; the daemon's answer may clamp it if another client is smaller.
      applySize(size.cols, size.rows);
      client
        .request("term.resize", { cardId, cols: size.cols, rows: size.rows })
        .then((r) => !disposed && applySize(r.cols, r.rows))
        .catch(() => {});
    }, 60);
  };
  const ro = new ResizeObserver(onResize);
  ro.observe(host);

  // Font changes from Settings apply to the open terminal; the resize observer refits it.
  offs.push(
    onTermPrefs((p) => {
      term.options.fontFamily = p.fontFamily;
      term.options.fontSize = p.fontSize;
      term.options.lineHeight = p.lineHeight;
      term.options.fontWeight = p.bold ? "bold" : "normal";
      onResize();
    }),
  );

  const onScheme = () => (term.options.theme = mq?.matches === false ? LIGHT : DARK);
  mq?.addEventListener("change", onScheme);

  return {
    focus: () => term.focus(),
    dispose() {
      if (disposed) return;
      disposed = true;
      if (resizeTimer) clearTimeout(resizeTimer);
      ro.disconnect();
      host.removeEventListener("paste", onPaste, true);
      mq?.removeEventListener("change", onScheme);
      for (const off of offs) off();
      dataSub.dispose();
      binSub.dispose();
      if (client.ready) client.request("session.detach", { cardId }).catch(() => {});
      term.dispose();
    },
  };
}
