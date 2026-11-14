import { invoke } from "@tauri-apps/api/core";
import type { Platform } from "./index";

interface DaemonInfo {
  url: string;
  token: string;
  home: string;
}

export function tauriPlatform(): Platform {
  return {
    kind: "tauri",
    // Re-read daemon.json and token each time: a restarted daemon may have a new port.
    async resolve() {
      const info = await invoke<DaemonInfo>("daemon_info");
      return { url: info.url, token: info.token };
    },
    startDaemon: () => invoke<string>("start_daemon"),
    getNotify: () => invoke<boolean>("get_notify"),
    setNotify: (on) => invoke<void>("set_notify", { on }),
    notifyStatus: () => invoke<string>("notify_status"),
    async onOpenCard(cb) {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<string>("open-card", (e) => cb(e.payload));
      // A tray click that recreated the window leaves its card here.
      const pending = await invoke<string | null>("take_pending_open");
      if (pending) cb(pending);
    },
    async pickFolder() {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const r = await open({ directory: true, multiple: false, title: "Add project folder" });
      return typeof r === "string" ? r : null;
    },
  };
}
