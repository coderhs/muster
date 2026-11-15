// The one seam between the front end and its host. Everything else talks to the
// daemon over the WebSocket, so the same bundle runs in the Tauri window and,
// later, in a phone browser served by the daemon.

import type { Endpoint, ProtocolError } from "../protocol/client";

/** What this copy of the app is. */
export interface AppInfo {
  version: string;
  /** Short git commit, with "+" appended for a build with uncommitted changes. */
  commit: string;
  commitDate: string;
  /** A development build (Muster Dev). */
  dev: boolean;
}

export interface Platform {
  kind: "tauri" | "browser";
  /** Where to connect. Called before every connection attempt. */
  resolve(lastError: ProtocolError | null): Promise<Endpoint>;
  /** Native folder chooser, when the host has one. */
  pickFolder?: () => Promise<string | null>;
  /** Requests from the host to open a card (e.g. a click in the menu-bar item). */
  onOpenCard?: (cb: (cardId: string) => void) => void;
  /** Open a web link in the user's browser. */
  openUrl(url: string): Promise<void>;
  /** Paths of files on the clipboard (copied in the Finder), when the host can tell. */
  clipboardFiles?: () => Promise<string[]>;
  /** Version and build commit of the desktop app. */
  appInfo?: () => Promise<AppInfo>;
  /** Start the daemon on request (the desktop shell can; a browser cannot). */
  startDaemon?: () => Promise<string>;
  /** Desktop notifications for needs-you / done (posted by the shell, even with the window closed). */
  getNotify?: () => Promise<boolean>;
  setNotify?: (on: boolean) => Promise<void>;
  /** The strip around the display's notch that shows working and waiting cards. */
  getIsland?: () => Promise<{ enabled: boolean; available: boolean }>;
  setIsland?: (on: boolean) => Promise<void>;
  /** "unavailable" | "unknown" | "requesting" | "granted" | "denied" */
  notifyStatus?: () => Promise<string>;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function detectPlatform(): Promise<Platform> {
  if (isTauri()) {
    const { tauriPlatform } = await import("./tauri");
    return tauriPlatform();
  }
  const { browserPlatform } = await import("./browser");
  return browserPlatform();
}
